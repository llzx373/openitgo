//! 文件操作引擎：复制/移动/删除/压缩/分割/合并的后台执行 + 进度/取消/暂停/
//! 冲突处理；重命名/新建文件夹为瞬时操作，提供同步 helper。对齐 extract 约定：
//! 每任务一条后台线程、channel 上报进度、`Arc<AtomicBool>` 取消与暂停、
//! 取消清理半成品目标文件（已完整复制/移动的保留并计入进度）、
//! 单项失败记 `errors` 继续整批、结束汇总上报。
//! 进度含当前文件内进度（cur_done/cur_total_bytes，分块复制维护，
//! 节流 100ms）；暂停在块/项边界生效（200ms 轮询，期间可即时取消），
//! 压缩（阶段 AB 起）经 create_zip 的 paused 参数在条目/块边界生效。
//! 分割/合并（阶段 AD，`run_split`/`run_merge`）：COPY_CHUNK 分块流式读写，
//! 块边界响应暂停/取消；分割已存在 `.NNN` 分块整批不覆盖（冲突列 errors
//! 直接收工），取消/失败删当前半成品分块（已完成块保留）；合并预扫描全部
//! 分块可读才开工，取消/失败删半成品输出，同目录 `<name>.crc`（sfv 单行）
//! 存在时流式顺带 CRC32 校验（不一致记 errors、结果保留）。
//!
//! 任务队列（阶段 AA）：并发上限 `max_concurrent`（settings `fm_op_threads`，
//! 默认 2，0 = 不限）——超限任务进 `queued` FIFO 队列（不起线程、无进度），
//! 在途任务结束经 `pump_queue` 放行；排队任务取消 = 直接从队列移除（无半成品
//! 静默丢弃）。`set_max_concurrent` 运行时下发：在途任务不动，调高立即放行、
//! 调低只影响后续放行。
//!
//! 冲突语义：策略由 UI 层操作前一次性确定（见
//! `file_manager_dialog.rs`）。`ConflictMode::Ask`（「逐个询问」）执行期
//! 遇冲突经 `OpEvent::AskConflict` 向 UI 发问并阻塞等答（100ms 轮询，
//! 期间可被取消打断按 Cancel 收拢；回答通道断开同按 Cancel）；
//! `ConflictAnswer.apply_all` 把该选择记忆为后续同级冲突（文件级/目录级
//! 各自独立）的生效策略。Overwrite 覆盖/合并；Skip 跳过记 errors 汇总；
//! AutoRename 用 `name (1).ext` 递增（`resolve_conflict_name`，同 parser
//! extract 的 uniquify 语义）。目录↔目录冲突：非 Ask 模式恒合并不问，
//! Ask 模式问一次「合并/跳过」（合并 = 递归进入逐项处理），不整删目标
//! 目录；目录级 apply_all 不预决文件级策略（各问各的，各自记忆）。
//!
//! 移动：`fs::rename` 快速路径（同盘瞬间完成），失败（跨盘/占用）回退
//! 递归复制 + 成功后 `trash::delete` 源。
//! 删除：默认逐项 `trash::delete`（回收站）；`fm_delete_mode = "permanent"`
//! 或 Shift+Del 直删时逐项物理删除（`permanent_delete`）。单项失败记 errors 继续。
//! 符号链接：复制 = 复制链接目标内容（`fs::copy` 语义），删除 = 只删链接；
//! 预扫描不跟进符号链接目录（防环），递归复制同样不跟进（符号链接目录
//! 按文件处理，`File::open` 失败则记 errors 继续）。
//! Windows 长路径：manifest 未声明 longPathAware，文件系统调用统一经
//! `verbatim_path` 加 `\\?\` 前缀（见该函数注释）。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::views::file_manager_rows::wildcard_match;

/// 复制块大小（B）：手动分块复制以便块间响应取消/暂停并上报文件内进度。
const COPY_CHUNK: usize = 256 * 1024;
/// 文件内进度上报节流：距上次发送满此间隔才发（避免 channel 洪泛；
/// 文件结束经 copy_recursive 的逐项上报兜底）。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// 暂停等待的轮询间隔（期间 cancel 即时生效）。
const PAUSE_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
    Compress,
    Split,
    Merge,
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "复制",
            OpKind::Move => "移动",
            OpKind::Delete => "删除",
            OpKind::Compress => "压缩",
            OpKind::Split => "分割",
            OpKind::Merge => "合并",
        }
    }
}

/// 冲突处理策略：操作前由对话框一次性确定；Ask = 执行期逐个询问
/// （worker 发 `OpEvent::AskConflict` 阻塞等 `ConflictAnswer`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMode {
    Ask,
    Overwrite,
    Skip,
    AutoRename,
}

/// 执行期冲突询问（Ask 模式，worker → UI）。is_dir = 目录↔目录冲突
/// （UI 文案「合并/跳过」；其余形态都按文件冲突问）。元数据查询失败给 None。
#[derive(Debug, Clone)]
pub struct ConflictQuery {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub is_dir: bool,
    pub src_size: Option<u64>,
    pub src_mtime: Option<std::time::SystemTime>,
    pub dst_size: Option<u64>,
    pub dst_mtime: Option<std::time::SystemTime>,
}

/// 冲突问答的选择。目录冲突 UI 只给「合并(=Overwrite)/跳过」；
/// Cancel = 取消整个操作（无 apply_all）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    AutoRename,
    Cancel,
}

/// UI → worker 的回答：apply_all = 记忆为后续同级冲突的生效策略
/// （文件级/目录级各自记忆；Cancel 忽略 apply_all）。
#[derive(Debug, Clone, Copy)]
pub struct ConflictAnswer {
    pub action: ConflictAction,
    pub apply_all: bool,
}

/// 复制/移动高级选项（阶段 AB，CopyMoveDialog「高级」折叠区 + 校验
/// checkbox）：默认全关（= 阶段 AA 前行为）。
#[derive(Debug, Clone, Default)]
pub struct CopyOptions {
    /// 复制完成后校验：逐文件重读源与目标分块字节比对（`files_identical`），
    /// 不一致记 errors「校验失败」。Move 的同盘 rename 快速路径无复制动作，
    /// 不校验（原子改名不引入损坏）。
    pub verify: bool,
    /// 仅复制匹配：通配符（复用 `wildcard_match` 的分号多模式语法），
    /// 空 = 不过滤。只过滤文件，目录结构保留（目录本身不跳过）。
    pub filter_pattern: String,
    /// 仅复制最近 N 天修改：0 = 不过滤；mtime 缺失的文件放行（不丢数据）。
    pub filter_newer_days: u32,
}

impl CopyOptions {
    /// 过滤是否生效（Move 快速路径 rename 在过滤开启时必须禁用——
    /// rename 会整树搬走，无法按文件过滤）。
    pub fn filter_active(&self) -> bool {
        !self.filter_pattern.trim().is_empty() || self.filter_newer_days > 0
    }

    /// 见 `copy_filter_matches`。
    fn matches(&self, name: &str, is_dir: bool, mtime: Option<SystemTime>) -> bool {
        copy_filter_matches(
            name,
            is_dir,
            mtime,
            &self.filter_pattern,
            self.filter_newer_days,
        )
    }
}

/// 复制过滤判定（阶段 AB；预扫描 count_source 与执行 copy_recursive 共用，
/// 保证进度 total 与实际复制集合一致）：目录恒 true（结构保留、只过滤
/// 文件）；文件按通配符（空 = 过）与最近 N 天修改（0 = 过；mtime 缺失
/// 放行）判定。
pub fn copy_filter_matches(
    name: &str,
    is_dir: bool,
    mtime: Option<SystemTime>,
    pattern: &str,
    newer_than_days: u32,
) -> bool {
    if is_dir {
        return true;
    }
    let pat = pattern.trim();
    if !pat.is_empty() && !wildcard_match(pat, name) {
        return false;
    }
    if newer_than_days > 0 {
        let Some(mtime) = mtime else { return true };
        let cutoff =
            SystemTime::now().checked_sub(Duration::from_secs(newer_than_days as u64 * 86400));
        if let Some(cutoff) = cutoff {
            if mtime < cutoff {
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, Default)]
pub struct OpProgress {
    pub done_files: u64,
    pub total_files: u64,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current: PathBuf,
    /// 当前文件内进度（分块复制维护；进入新文件时 done 归零、total 设为
    /// 该文件大小）。cur_total_bytes=0 = 无文件内进度（目录/删除/rename
    /// 快速路径），UI 不显示当前文件条。
    pub cur_done_bytes: u64,
    pub cur_total_bytes: u64,
}

impl OpProgress {
    /// 进度比例：优先按字节，其次按项数，都没有则 0。
    pub fn fraction(&self) -> f32 {
        if self.total_bytes > 0 {
            (self.done_bytes as f64 / self.total_bytes as f64) as f32
        } else if self.total_files > 0 {
            self.done_files as f32 / self.total_files as f32
        } else {
            0.0
        }
    }
}

/// 冲突问答等待的轮询间隔（期间 cancel 即时生效，按 Cancel 收拢）。
const CONFLICT_POLL: Duration = Duration::from_millis(100);

/// 线程 → UI 的事件。
enum OpEvent {
    Progress(OpProgress),
    /// Ask 模式冲突询问：worker 阻塞等 `answer_tx` 回发 `ConflictAnswer`。
    AskConflict(ConflictQuery),
    Finished {
        cancelled: bool,
        fatal: Option<String>,
        errors: Vec<(PathBuf, String)>,
        /// 实际写入的顶层目标与对应源（阶段 AG 撤销用）：Copy = 每个成功
        /// 复制的 (目标, 源)（AutoRename 后为准）；Move = 源已移除的
        /// (目标, 源) 配对（源残留的不记——原位置仍被占用，撤销无意义）；
        /// Delete/Compress/Split/Merge 恒空。
        written: Vec<(PathBuf, PathBuf)>,
    },
}

pub struct FileOpTask {
    pub id: u64,
    pub kind: OpKind,
    pub progress: OpProgress,
    /// 源路径全集（任务面板摘要 + 重试重建用）。
    sources: Vec<PathBuf>,
    /// 工作线程目标（Copy/Move = dest_dir；Compress = dest_zip；Delete = None；
    /// 重试重建用）。
    dest: Option<PathBuf>,
    /// 冲突策略（重试重建用；Delete/Compress 恒 Skip）。
    conflict: ConflictMode,
    /// 永久删除档（重试重建用；仅 Delete 有意义）。
    delete_permanent: bool,
    /// 涉及的源目录（各 source 的 parent），完成后刷新用。
    src_dirs: Vec<PathBuf>,
    /// 目标目录（仅复制/移动），完成后刷新用。
    dest_dir: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    /// 暂停标志（与 cancel 同模式暴露给 UI；Copy/Move/Delete 在块/项边界
    /// 生效，Compress 经 create_zip 在条目/块边界生效，阶段 AB 起）。
    paused: Arc<AtomicBool>,
    rx: Receiver<OpEvent>,
    /// Ask 模式的回答回发端（worker 持 rx 阻塞等答；task 被移除时 drop，
    /// worker recv 出错按 Cancel 收拢，防悬挂）。
    answer_tx: Sender<ConflictAnswer>,
    /// 已到达、待 UI 取走的冲突询问（worker 逐一发问，恒最多一条）。
    pending_query: Option<ConflictQuery>,
}

/// 排队任务（阶段 AA；未起线程，参数全集保留供放行时起线程）。
struct QueuedTask {
    id: u64,
    kind: OpKind,
    sources: Vec<PathBuf>,
    dest: Option<PathBuf>,
    conflict: ConflictMode,
    delete_permanent: bool,
    /// 复制/移动高级选项（阶段 AB；Delete/Compress 为默认）。
    opts: CopyOptions,
    /// 分块字节数（阶段 AD；仅 Split 有意义，其余恒 0）。
    chunk_size: u64,
}

/// 任务面板行快照（阶段 AA；在途按提交序 + 排队按 FIFO 序）。
pub struct TaskSummary {
    pub id: u64,
    pub kind: OpKind,
    /// 首项源 + 总项数（UI 拼「首项 + 共 N 项」摘要）。
    pub first_source: Option<PathBuf>,
    pub source_count: usize,
    /// 目标目录（Copy/Move = dest_dir；Compress = zip 父目录；Delete = None）。
    pub dest_dir: Option<PathBuf>,
    pub progress: OpProgress,
    pub queued: bool,
    pub paused: bool,
}

/// 本帧完成的任务快照（poll 返回值携带，已从 manager 移除）。
pub struct FinishedOp {
    pub kind: OpKind,
    pub cancelled: bool,
    pub fatal: Option<String>,
    pub errors: Vec<(PathBuf, String)>,
    pub src_dirs: Vec<PathBuf>,
    pub dest_dir: Option<PathBuf>,
    /// 重试重建参数（阶段 AA 错误汇总窗「重试失败项」）：工作线程目标
    /// （Compress = dest_zip）/ 冲突策略 / 永久删除档；源从 errors 重建
    /// （errors 恒为源路径，retry_sources 过滤仍在项）。
    pub dest: Option<PathBuf>,
    pub conflict: ConflictMode,
    pub delete_permanent: bool,
    /// 实际写入的顶层目标与对应源（阶段 AG 撤销用；语义见 OpEvent::Finished）。
    pub written: Vec<(PathBuf, PathBuf)>,
}

/// 活动任务快照（状态栏进度条用）。
pub struct ActiveOp {
    pub id: u64,
    pub kind: OpKind,
    pub progress: OpProgress,
    /// 暂停态快照（UI 直接读，无需进进度消息）。
    pub paused: bool,
}

#[derive(Default)]
pub struct OpSummary {
    pub has_active: bool,
    pub active: Option<ActiveOp>,
    pub finished: Vec<FinishedOp>,
}

pub struct FileOpManager {
    tasks: Vec<FileOpTask>,
    /// 排队任务（FIFO；阶段 AA）：超过并发上限的任务在此等待放行。
    queued: VecDeque<QueuedTask>,
    next_id: u64,
    /// 并发上限（fm_op_threads）：0 = 不限。
    max_concurrent: usize,
}

impl Default for FileOpManager {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            queued: VecDeque::new(),
            next_id: 0,
            max_concurrent: 2,
        }
    }
}

impl FileOpManager {
    /// 后台复制 sources 到 dest_dir（逐项应用 conflict 策略）。
    /// 复制/移动不涉及永久删除（Move 回退路径的源清理恒走回收站）。
    pub fn start_copy(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
    ) -> u64 {
        self.start_copy_opts(sources, dest_dir, conflict, CopyOptions::default())
    }

    /// 带高级选项的复制（阶段 AB：CopyMoveDialog 校验/过滤）。
    pub fn start_copy_opts(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
        opts: CopyOptions,
    ) -> u64 {
        self.submit(
            OpKind::Copy,
            sources,
            Some(dest_dir),
            conflict,
            false,
            opts,
            0,
        )
    }

    /// 后台移动：同盘 `fs::rename` 快速路径，失败回退复制 + trash 源。
    pub fn start_move(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
    ) -> u64 {
        self.start_move_opts(sources, dest_dir, conflict, CopyOptions::default())
    }

    /// 带高级选项的移动（阶段 AB；过滤开启时 rename 快速路径自动禁用）。
    pub fn start_move_opts(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
        opts: CopyOptions,
    ) -> u64 {
        self.submit(
            OpKind::Move,
            sources,
            Some(dest_dir),
            conflict,
            false,
            opts,
            0,
        )
    }

    /// 后台删除：permanent=false 逐项移入回收站（trash::delete）；
    /// permanent=true（fm_delete_mode = "permanent" / Shift+Del 直删）
    /// 逐项物理删除（remove_file/remove_dir_all），失败记 errors 继续。
    pub fn start_delete(&mut self, sources: Vec<PathBuf>, permanent: bool) -> u64 {
        self.submit(
            OpKind::Delete,
            sources,
            None,
            ConflictMode::Skip,
            permanent,
            CopyOptions::default(),
            0,
        )
    }

    /// 后台压缩 sources 为 dest_zip（zip 引擎在 parser 侧，逐项进度桥接进
    /// OpProgress；dest_dir 记 dest_zip 的父目录使完成后栏刷新自动生效）。
    pub fn start_compress(&mut self, sources: Vec<PathBuf>, dest_zip: PathBuf) -> u64 {
        self.submit(
            OpKind::Compress,
            sources,
            Some(dest_zip),
            ConflictMode::Skip,
            false,
            CopyOptions::default(),
            0,
        )
    }

    /// 后台分割 source 为 dest_dir 下的 `name.NNN` 分块（阶段 AD）：
    /// 已存在分块不覆盖（执行前列出冲突报错）；取消删除当前半成品分块
    /// （已完成分块保留）。
    pub fn start_split(&mut self, source: PathBuf, dest_dir: PathBuf, chunk_size: u64) -> u64 {
        self.submit(
            OpKind::Split,
            vec![source],
            Some(dest_dir),
            ConflictMode::Skip,
            false,
            CopyOptions::default(),
            chunk_size,
        )
    }

    /// 后台合并 chunks（连续编号全集，由调用方经 `collect_chunks` 收集）
    /// 为 dest（阶段 AD）；同目录存在 `<name>.crc`（sfv 单行）时合并后
    /// 校验 CRC32（流式顺带计算，无额外 IO），不一致记 errors。
    pub fn start_merge(&mut self, chunks: Vec<PathBuf>, dest: PathBuf) -> u64 {
        self.submit(
            OpKind::Merge,
            chunks,
            Some(dest),
            ConflictMode::Skip,
            false,
            CopyOptions::default(),
            0,
        )
    }

    /// 提交任务：有空位立即起线程，否则进 FIFO 队列（阶段 AA）。
    #[allow(clippy::too_many_arguments)]
    fn submit(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        conflict: ConflictMode,
        delete_permanent: bool,
        opts: CopyOptions,
        chunk_size: u64,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        if self.at_capacity() {
            self.queued.push_back(QueuedTask {
                id,
                kind,
                sources,
                dest,
                conflict,
                delete_permanent,
                opts,
                chunk_size,
            });
        } else {
            self.launch(
                id,
                kind,
                sources,
                dest,
                conflict,
                delete_permanent,
                opts,
                chunk_size,
            );
        }
        id
    }

    /// 是否已达并发上限（0 = 不限）。
    fn at_capacity(&self) -> bool {
        self.max_concurrent > 0 && self.tasks.len() >= self.max_concurrent
    }

    /// 起线程跑一个任务（submit 直放 / pump_queue 放行共用）。
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &mut self,
        id: u64,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        conflict: ConflictMode,
        delete_permanent: bool,
        opts: CopyOptions,
        chunk_size: u64,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (answer_tx, answer_rx) = channel();
        let mut src_dirs: Vec<PathBuf> = sources
            .iter()
            .filter_map(|s| s.parent().map(Path::to_path_buf))
            .collect();
        src_dirs.sort();
        src_dirs.dedup();
        // Compress/Merge 的刷新目录 = 产出文件父目录（dest 为产出路径）；
        // Split 的 dest 本身就是目标目录。
        let dest_dir = match kind {
            OpKind::Compress | OpKind::Merge => dest
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            _ => dest.clone(),
        };
        let task = FileOpTask {
            id,
            kind,
            progress: OpProgress::default(),
            sources: sources.clone(),
            dest: dest.clone(),
            conflict,
            delete_permanent,
            src_dirs,
            dest_dir,
            cancel: cancel.clone(),
            paused: paused.clone(),
            rx,
            answer_tx,
            pending_query: None,
        };
        self.tasks.push(task);
        match kind {
            OpKind::Compress => {
                let dest_zip = dest.expect("Compress 必有 dest_zip");
                std::thread::spawn(move || {
                    run_compress(sources, dest_zip, cancel, paused, tx);
                });
            }
            OpKind::Split => {
                let dest_dir = dest.expect("Split 必有 dest_dir");
                std::thread::spawn(move || {
                    run_split(sources, dest_dir, chunk_size, cancel, paused, tx);
                });
            }
            OpKind::Merge => {
                let dest = dest.expect("Merge 必有 dest");
                std::thread::spawn(move || {
                    run_merge(sources, dest, cancel, paused, tx);
                });
            }
            _ => {
                // Ask 模式才需要问答通道（worker 阻塞等答）；其余模式不需要。
                let answer_rx = (conflict == ConflictMode::Ask).then_some(answer_rx);
                std::thread::spawn(move || {
                    run_op(
                        kind,
                        sources,
                        dest,
                        conflict,
                        cancel,
                        paused,
                        tx,
                        answer_rx,
                        delete_permanent,
                        opts,
                    );
                });
            }
        }
    }

    /// 放行排队任务（FIFO）：有空位才起线程。
    fn pump_queue(&mut self) {
        while !self.at_capacity() {
            let Some(q) = self.queued.pop_front() else {
                break;
            };
            self.launch(
                q.id,
                q.kind,
                q.sources,
                q.dest,
                q.conflict,
                q.delete_permanent,
                q.opts,
                q.chunk_size,
            );
        }
    }

    /// 运行时调整并发上限（fm_op_threads 每帧下发）：在途任务不动——
    /// 调高立即放行排队任务，调低只影响后续放行。
    pub fn set_max_concurrent(&mut self, n: usize) {
        if self.max_concurrent != n {
            self.max_concurrent = n;
            self.pump_queue();
        }
    }

    /// 在途任务数（不含排队）。
    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }

    /// 排队任务数。
    pub fn queued_count(&self) -> usize {
        self.queued.len()
    }

    /// 任务面板行快照（阶段 AA）：在途（提交序）+ 排队（FIFO 序）。
    pub fn task_summaries(&self) -> Vec<TaskSummary> {
        let mut out: Vec<TaskSummary> = self
            .tasks
            .iter()
            .map(|t| TaskSummary {
                id: t.id,
                kind: t.kind,
                first_source: t.sources.first().cloned(),
                source_count: t.sources.len(),
                dest_dir: t.dest_dir.clone(),
                progress: t.progress.clone(),
                queued: false,
                paused: t.paused.load(Ordering::Relaxed),
            })
            .collect();
        out.extend(self.queued.iter().map(|q| {
            TaskSummary {
                id: q.id,
                kind: q.kind,
                first_source: q.sources.first().cloned(),
                source_count: q.sources.len(),
                dest_dir: match q.kind {
                    OpKind::Compress | OpKind::Merge => q
                        .dest
                        .as_deref()
                        .and_then(Path::parent)
                        .map(Path::to_path_buf),
                    _ => q.dest.clone(),
                },
                progress: OpProgress::default(),
                queued: true,
                paused: false,
            }
        }));
        out
    }

    /// 取消任务：排队任务未起线程直接从队列移除（无半成品，静默）；
    /// 在途任务置旗标，工作线程在下一块/下一项停止并上报
    /// Finished(cancelled)。
    pub fn cancel(&mut self, id: u64) {
        if let Some(pos) = self.queued.iter().position(|t| t.id == id) {
            self.queued.remove(pos);
            return;
        }
        if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
            task.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 取出一个待答冲突询问（Ask 模式；worker 逐一发问且阻塞等答，全局
    /// 恒最多一条在途）。取走后由 `answer_conflict` 回发；任务已结束未答
    /// 时 worker 侧经 cancel/answer_tx 断开兜底按 Cancel 收拢，不会悬挂。
    pub fn take_pending_conflict(&mut self) -> Option<(u64, ConflictQuery)> {
        for task in &mut self.tasks {
            if let Some(q) = task.pending_query.take() {
                return Some((task.id, q));
            }
        }
        None
    }

    /// 回发冲突回答（发送失败 = worker 已退出，忽略）。
    pub fn answer_conflict(&mut self, id: u64, answer: ConflictAnswer) {
        if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
            let _ = task.answer_tx.send(answer);
        }
    }

    /// 任务是否仍在活动列表（冲突弹窗的任务结束检测用）。
    pub fn is_active(&self, id: u64) -> bool {
        self.tasks.iter().any(|t| t.id == id)
    }

    /// 暂停/继续任务（Copy/Move/Delete 在块/项边界生效；Compress 在
    /// 条目/块边界生效，阶段 AB 起；排队任务无旗标可置）。
    pub fn set_paused(&mut self, id: u64, paused: bool) {
        if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
            task.paused.store(paused, Ordering::Relaxed);
        }
    }

    /// 每帧调用：排空事件、移除已完成任务、返回汇总。
    pub fn poll(&mut self) -> OpSummary {
        let mut summary = OpSummary::default();
        let mut finished_ids = Vec::new();
        for task in &mut self.tasks {
            loop {
                match task.rx.try_recv() {
                    Ok(OpEvent::Progress(p)) => task.progress = p,
                    Ok(OpEvent::AskConflict(q)) => task.pending_query = Some(q),
                    Ok(OpEvent::Finished {
                        cancelled,
                        fatal,
                        errors,
                        written,
                    }) => {
                        finished_ids.push((task.id, cancelled, fatal, errors, written));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // 线程异常终止（panic 等）：按失败收尾，防任务卡死。
                        finished_ids.push((
                            task.id,
                            false,
                            Some("操作线程异常终止".to_string()),
                            Vec::new(),
                            Vec::new(),
                        ));
                        break;
                    }
                }
            }
        }
        for (id, cancelled, fatal, errors, written) in finished_ids {
            if let Some(pos) = self.tasks.iter().position(|t| t.id == id) {
                let task = self.tasks.remove(pos);
                summary.finished.push(FinishedOp {
                    kind: task.kind,
                    cancelled,
                    fatal,
                    errors,
                    src_dirs: task.src_dirs,
                    dest_dir: task.dest_dir,
                    dest: task.dest,
                    conflict: task.conflict,
                    delete_permanent: task.delete_permanent,
                    written,
                });
            }
        }
        // 在途任务结束腾出空位：FIFO 放行排队任务（阶段 AA）。
        self.pump_queue();
        summary.has_active = !self.tasks.is_empty();
        summary.active = self.tasks.first().map(|t| ActiveOp {
            id: t.id,
            kind: t.kind,
            progress: t.progress.clone(),
            paused: t.paused.load(Ordering::Relaxed),
        });
        summary
    }
}

/// 永久删除（fm_delete_mode = "permanent" / Shift+Del 直删）：符号链接与文件
/// remove_file、目录 remove_dir_all；错误交 errors 汇总（同 trash 路径语义）。
fn permanent_delete(path: &Path) -> Result<(), String> {
    let vp = verbatim_path(path);
    let meta = std::fs::symlink_metadata(&vp).map_err(|e| e.to_string())?;
    let r = if meta.is_dir() && !meta.file_type().is_symlink() {
        std::fs::remove_dir_all(&vp)
    } else {
        std::fs::remove_file(&vp)
    };
    r.map_err(|e| e.to_string())
}

/// 从失败项重建重试源列表（阶段 AA 错误汇总窗「重试失败项」）：
/// 源路径仍在才纳入（Copy/Move/Delete 的 errors 均为源路径），去重保序。
pub fn retry_sources(errors: &[(PathBuf, String)]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    errors
        .iter()
        .map(|(p, _)| p.clone())
        .filter(|p| p.exists() && seen.insert(p.clone()))
        .collect()
}

/// 工作线程入口：预扫描计数 → 逐项执行 → Finished 事件收尾。
/// answer_rx 仅 Ask 模式 Some（冲突问答的回答接收端）。
/// opts 仅 Copy/Move 有意义（阶段 AB 校验/过滤；预扫描与执行共用同一
/// 过滤判定，进度 total 按过滤后集合计算）。
#[allow(clippy::too_many_arguments)]
fn run_op(
    kind: OpKind,
    sources: Vec<PathBuf>,
    dest_dir: Option<PathBuf>,
    conflict: ConflictMode,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    tx: Sender<OpEvent>,
    answer_rx: Option<Receiver<ConflictAnswer>>,
    delete_permanent: bool,
    opts: CopyOptions,
) {
    let mut ctx = OpCtx {
        cancel: &cancel,
        paused: &paused,
        tx: &tx,
        answer_rx: answer_rx.as_ref(),
        opts: &opts,
        drop_source_unsafe: false,
        move_prune: kind == OpKind::Move && opts.filter_active(),
        remembered_file: None,
        remembered_dir: None,
        progress: OpProgress::default(),
        errors: Vec::new(),
        written: Vec::new(),
        cancelled: false,
    };
    // 预扫描：递归计数（文件+目录项数、文件字节，过滤后集合）；符号链接
    // 不跟进目录。
    let mut per_source = Vec::with_capacity(sources.len());
    for src in &sources {
        let (items, bytes) = count_source(src, &opts);
        ctx.progress.total_files += items;
        ctx.progress.total_bytes += bytes;
        per_source.push((items, bytes));
    }
    let mut fatal = None;
    match kind {
        OpKind::Compress | OpKind::Split | OpKind::Merge => {
            unreachable!("Compress/Split/Merge 各有专用 run 函数")
        }
        OpKind::Delete => {
            for (i, src) in sources.iter().enumerate() {
                if ctx.wait_if_paused() {
                    break;
                }
                ctx.progress.current = src.clone();
                let result = if delete_permanent {
                    permanent_delete(src)
                } else {
                    trash::delete(src).map_err(|e| e.to_string())
                };
                if let Err(e) = result {
                    ctx.errors.push((src.clone(), e));
                }
                let (items, bytes) = per_source[i];
                ctx.progress.done_files += items;
                ctx.progress.done_bytes += bytes;
                ctx.send_progress();
            }
        }
        OpKind::Copy | OpKind::Move => {
            let dest_dir = dest_dir.unwrap_or_default();
            for (i, src) in sources.iter().enumerate() {
                if ctx.wait_if_paused() {
                    break;
                }
                let Some(name) = src.file_name() else {
                    ctx.errors
                        .push((src.clone(), "无法确定名称（根目录不可作为源）".to_string()));
                    continue;
                };
                let dst = dest_dir.join(name);
                if *src == dst {
                    ctx.errors.push((src.clone(), "源与目标相同".to_string()));
                    continue;
                }
                ctx.progress.current = src.clone();
                let (items, bytes) = per_source[i];
                if kind == OpKind::Move && !opts.filter_active() {
                    // 快速路径：同盘 rename 瞬间完成（目标存在时 rename 在
                    // Windows 上会失败，落入回退路径由冲突策略处理）。
                    // 过滤开启时禁用——rename 整树搬走无法按文件过滤。
                    if !verbatim_path(&dst).exists()
                        && std::fs::rename(verbatim_path(src), verbatim_path(&dst)).is_ok()
                    {
                        ctx.written.push((dst, src.clone()));
                        ctx.progress.done_files += items;
                        ctx.progress.done_bytes += bytes;
                        ctx.send_progress();
                        continue;
                    }
                }
                // 冲突消解：目录↔目录非 Ask 恒合并；Ask 逐个询问（含目录
                // 「合并/跳过」）；Skip 记 errors 汇总；Cancel 收拢整批。
                let dst = match ctx.resolve_dst(src, &dst, conflict) {
                    Resolve::Proceed(d) => Some(d),
                    Resolve::Skip => {
                        ctx.errors
                            .push((src.clone(), "目标已存在，已跳过".to_string()));
                        ctx.progress.done_files += items;
                        ctx.progress.done_bytes += bytes;
                        ctx.send_progress();
                        None
                    }
                    Resolve::Cancelled => break,
                };
                if let Some(dst) = dst {
                    ctx.drop_source_unsafe = false;
                    let copied = copy_recursive(src, &dst, conflict, &mut ctx).is_ok();
                    // 撤销记账（阶段 AG）：Copy 成功即记 (目标, 源)；Move 仅
                    // 源已移除才记（源残留 = 原位置仍被占用，移回必冲突）。
                    if copied && kind == OpKind::Copy && !ctx.cancelled {
                        ctx.written.push((dst.clone(), src.clone()));
                    }
                    if copied && kind == OpKind::Move && !ctx.cancelled {
                        // 跨盘/占用回退：复制成功后清理源。过滤移动
                        // （move_prune）已逐文件 trash + 清空空目录，此处
                        // 只除根并说明残留；非过滤且发生过校验失败时不整删。
                        if ctx.drop_source_unsafe {
                            if ctx.move_prune {
                                let _ = std::fs::remove_dir(verbatim_path(src));
                                ctx.errors.push((
                                    src.clone(),
                                    "含被过滤/失败项：匹配项已搬走，其余保留在源".to_string(),
                                ));
                            } else {
                                ctx.errors.push((
                                    src.clone(),
                                    "存在未成功复制的项：源未删除（已搬走部分保留在目标）"
                                        .to_string(),
                                ));
                            }
                        } else if ctx.move_prune {
                            if std::fs::remove_dir(verbatim_path(src)).is_ok() {
                                ctx.written.push((dst.clone(), src.clone()));
                            }
                        } else if let Err(e) = trash::delete(src) {
                            ctx.errors
                                .push((src.clone(), format!("源移入回收站失败: {e}")));
                        } else {
                            ctx.written.push((dst.clone(), src.clone()));
                        }
                    }
                }
            }
        }
    }
    let _ = tx.send(OpEvent::Finished {
        cancelled: ctx.cancelled,
        fatal: fatal.take(),
        errors: ctx.errors,
        written: ctx.written,
    });
}

/// 压缩工作线程：调 parser 的 create_zip，ZipWriteProgress 经转发线程
/// 桥接成 OpProgress 快照流；取消/致命错误按 create_zip 约定收尾
/// （Err → fatal；cancel 置位 → cancelled；容错跳过项不逐项上报）。
/// 暂停（阶段 AB）：paused 透传 create_zip（条目/块边界 200ms 轮询）。
fn run_compress(
    sources: Vec<PathBuf>,
    dest_zip: PathBuf,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    tx: Sender<OpEvent>,
) {
    use openitgo_parser::archive::{create_zip, ZipWriteOptions, ZipWriteProgress};
    let (ztx, zrx) = crossbeam_channel::unbounded();
    let fwd_tx = tx.clone();
    let forwarder = std::thread::spawn(move || {
        let mut progress = OpProgress::default();
        while let Ok(ev) = zrx.recv() {
            match ev {
                ZipWriteProgress::Started {
                    total_files,
                    total_bytes,
                } => {
                    progress.total_files = total_files as u64;
                    progress.total_bytes = total_bytes;
                }
                ZipWriteProgress::EntryDone { name, bytes } => {
                    progress.done_files += 1;
                    progress.done_bytes += bytes;
                    progress.current = PathBuf::from(name);
                }
                // 结束态由下方 create_zip 返回值统一收尾，不重复上报。
                ZipWriteProgress::Finished { .. } | ZipWriteProgress::Failed(_) => {}
            }
            let _ = fwd_tx.send(OpEvent::Progress(progress.clone()));
        }
    });
    let result = create_zip(
        &sources,
        &dest_zip,
        &ZipWriteOptions::default(),
        ztx,
        cancel.clone(),
        Some(paused),
    );
    let _ = forwarder.join();
    let _ = tx.send(OpEvent::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
        fatal: result.err().map(|e| e.to_string()),
        errors: Vec::new(),
        written: Vec::new(),
    });
}

// ---------- 分割/合并（阶段 AD） ----------

/// 非复制类流式任务的占位选项（Split/Merge 无过滤/校验语义；String::new
/// 为 const fn，可静态构造）。
static NO_OPTS: CopyOptions = CopyOptions {
    verify: false,
    filter_pattern: String::new(),
    filter_newer_days: 0,
};

/// Split/Merge 工作线程共用的 OpCtx（无问答通道、无过滤选项）。
fn stream_ctx<'a>(
    cancel: &'a AtomicBool,
    paused: &'a AtomicBool,
    tx: &'a Sender<OpEvent>,
) -> OpCtx<'a> {
    OpCtx {
        cancel,
        paused,
        tx,
        answer_rx: None,
        opts: &NO_OPTS,
        drop_source_unsafe: false,
        move_prune: false,
        remembered_file: None,
        remembered_dir: None,
        progress: OpProgress::default(),
        errors: Vec::new(),
        written: Vec::new(),
        cancelled: false,
    }
}

/// 分割计划（阶段 AD；纯函数）：按 chunk_size 切 total_len，返回
/// (分块编号后缀 "001".."NNN", 各自字节数)——编号超 999 自然延伸
/// （"1000"…）。total_len == 0 → 单个空分块 "001"；chunk_size == 0 按 1
/// 防御（对话框已保证 >0）。
pub fn split_plan(chunk_size: u64, total_len: u64) -> Vec<(String, u64)> {
    let chunk = chunk_size.max(1);
    if total_len == 0 {
        return vec![("001".to_string(), 0)];
    }
    let count = total_len.div_ceil(chunk);
    (1..=count)
        .map(|i| {
            let bytes = if i < count {
                chunk
            } else {
                total_len - chunk * (count - 1)
            };
            (format!("{i:03}"), bytes)
        })
        .collect()
}

/// 分块名解析：`base.NNN`（恰好 3 位数字后缀）→ Some((base, 编号))。
pub(crate) fn parse_chunk_name(name: &str) -> Option<(&str, u32)> {
    let (base, suffix) = name.rsplit_once('.')?;
    if base.is_empty() || suffix.len() != 3 || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    suffix.parse().ok().map(|n| (base, n))
}

/// 「合并…」目标推导（阶段 AD；纯函数）：选中集为单个 `.001` 文件，或一组
/// 同目录同前缀 `.NNN` 分块（须含 .001）→ Some((目录, 前缀))。
pub fn merge_target_base(targets: &[PathBuf]) -> Option<(PathBuf, String)> {
    let mut base: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    let mut has_001 = false;
    for p in targets {
        if p.is_dir() {
            return None;
        }
        let name = p.file_name()?.to_string_lossy().into_owned();
        let (b, n) = parse_chunk_name(&name)?;
        if n == 1 {
            has_001 = true;
        }
        match &base {
            Some(prev) if prev != b => return None,
            None => base = Some(b.to_string()),
            _ => {}
        }
        let parent = p.parent()?.to_path_buf();
        match &dir {
            Some(prev) if *prev != parent => return None,
            None => dir = Some(parent),
            _ => {}
        }
    }
    // 单个文件时须恰为 .001（一组时 .001 必含其中）。
    if !has_001 {
        return None;
    }
    dir.zip(base)
}

/// 收集 dir 下 `base.NNN` 连续分块（阶段 AD；从 .001 起无缺号，重号去重）。
/// 缺号报错列出（前 5 个）；无任何分块亦报错。
pub fn collect_chunks(dir: &Path, base: &str) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(verbatim_path(dir)).map_err(|e| format!("无法列举目录: {e}"))?;
    let mut numbered: Vec<(u32, PathBuf)> = Vec::new();
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let Some((b, n)) = parse_chunk_name(&name) else {
            continue;
        };
        if b == base && n >= 1 && item.file_type().map(|t| t.is_file()).unwrap_or(false) {
            numbered.push((n, item.path()));
        }
    }
    if numbered.is_empty() {
        return Err(format!("未找到 {base}.001 起的分块"));
    }
    numbered.sort_by_key(|(n, _)| *n);
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for (n, p) in numbered {
        if seen.insert(n) {
            out.push(p);
        }
    }
    let max = *seen.iter().max().unwrap_or(&0);
    let missing: Vec<u32> = (1..=max).filter(|n| !seen.contains(n)).collect();
    if !missing.is_empty() {
        let list: Vec<String> = missing.iter().take(5).map(|n| format!(".{n:03}")).collect();
        let more = if missing.len() > 5 { " …" } else { "" };
        return Err(format!("分块缺失：{}{more}", list.join(" ")));
    }
    Ok(out)
}

/// 合并输出的同名 `.crc` 校验文件（sfv 单行，复用阶段 AC 解析）：条目
/// 文件名与 dest 名匹配（忽略大小写）时返回期望 CRC32 hex。
fn expected_crc_for(dest: &Path) -> Option<String> {
    use crate::views::file_manager_checksum::{parse_checksum_file, ChecksumAlgorithm};
    let crc_path = PathBuf::from(format!("{}.crc", dest.display()));
    let bytes = std::fs::read(verbatim_path(&crc_path)).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let name = dest.file_name()?.to_string_lossy();
    parse_checksum_file(&text)
        .into_iter()
        .find(|e| {
            e.algorithm == Some(ChecksumAlgorithm::Crc32) && e.filename.eq_ignore_ascii_case(&name)
        })
        .map(|e| e.hash)
}

/// 分割工作线程（阶段 AD）：`name.NNN` 逐块写出，COPY_CHUNK 分块读写
/// （块间响应暂停/取消，进度按字节推进）。已存在分块不覆盖——开始前
/// 一次性列出冲突进 errors 直接收工（不动任何文件）；取消/写失败删除
/// 当前半成品分块（已完成分块保留）；读源/写块 IO 错误 = fatal 终止
/// （残缺分块集无意义，不继续后续块）。
fn run_split(
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
    chunk_size: u64,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    tx: Sender<OpEvent>,
) {
    use std::io::{Read, Write};
    let mut ctx = stream_ctx(&cancel, &paused, &tx);
    let mut fatal: Option<String> = None;
    'done: {
        let Some(source) = sources.first() else {
            fatal = Some("无分割源".to_string());
            break 'done;
        };
        let meta = match std::fs::symlink_metadata(verbatim_path(source)) {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                fatal = Some("分割源不是文件".to_string());
                break 'done;
            }
            Err(e) => {
                fatal = Some(format!("无法读取分割源: {e}"));
                break 'done;
            }
        };
        let Some(name) = source.file_name() else {
            fatal = Some("无法确定分割源名称".to_string());
            break 'done;
        };
        let name = name.to_string_lossy().into_owned();
        let plan = split_plan(chunk_size, meta.len());
        let chunk_paths: Vec<PathBuf> = plan
            .iter()
            .map(|(suffix, _)| dest_dir.join(format!("{name}.{suffix}")))
            .collect();
        // 冲突预检：任一 .NNN 已存在则整批不动（分割不适用 AutoRename）。
        let mut has_conflict = false;
        for p in &chunk_paths {
            if verbatim_path(p).exists() {
                ctx.errors.push((
                    p.clone(),
                    "目标分块已存在（分割不覆盖，请先处理）".to_string(),
                ));
                has_conflict = true;
            }
        }
        if has_conflict {
            break 'done;
        }
        ctx.progress.total_files = plan.len() as u64;
        ctx.progress.total_bytes = meta.len();
        ctx.send_progress();
        let mut input = match std::fs::File::open(verbatim_path(source)) {
            Ok(f) => f,
            Err(e) => {
                fatal = Some(format!("无法打开分割源: {e}"));
                break 'done;
            }
        };
        let mut buf = vec![0u8; COPY_CHUNK];
        let mut last_sent = Instant::now();
        for ((_, bytes), chunk_path) in plan.iter().zip(&chunk_paths) {
            if ctx.wait_if_paused() {
                break;
            }
            ctx.progress.current = chunk_path.clone();
            let mut out = match std::fs::File::create(verbatim_path(chunk_path)) {
                Ok(f) => f,
                Err(e) => {
                    fatal = Some(format!("无法创建分块 {}: {e}", chunk_path.display()));
                    break;
                }
            };
            let mut remaining = *bytes;
            while remaining > 0 {
                if ctx.wait_if_paused() {
                    break;
                }
                let want = remaining.min(COPY_CHUNK as u64) as usize;
                match input.read(&mut buf[..want]) {
                    Ok(0) => {
                        fatal = Some("源文件读取提前结束（大小已变化）".to_string());
                        break;
                    }
                    Ok(n) => {
                        if let Err(e) = out.write_all(&buf[..n]) {
                            fatal = Some(format!("写入分块失败: {e}"));
                            break;
                        }
                        remaining -= n as u64;
                        ctx.progress.done_bytes += n as u64;
                        if last_sent.elapsed() >= PROGRESS_INTERVAL {
                            ctx.send_progress();
                            last_sent = Instant::now();
                        }
                    }
                    Err(e) => {
                        fatal = Some(format!("读取分割源失败: {e}"));
                        break;
                    }
                }
            }
            // 取消/失败：当前分块为半成品，删除；已完成分块保留。
            if ctx.cancelled || fatal.is_some() {
                drop(out);
                let _ = std::fs::remove_file(verbatim_path(chunk_path));
                break;
            }
            ctx.progress.done_files += 1;
            ctx.send_progress();
        }
    }
    let _ = tx.send(OpEvent::Finished {
        cancelled: ctx.cancelled,
        fatal,
        errors: ctx.errors,
        written: ctx.written,
    });
}

/// 合并工作线程（阶段 AD）：chunks 按序流式拼合进 dest（COPY_CHUNK
/// 分块，块间响应暂停/取消）；同目录 `<name>.crc`（sfv 单行）存在时
/// 顺带计算 CRC32（无额外 IO）合并后比对，不一致记 errors（结果保留）。
/// 取消/IO 错误删除半成品 dest。
fn run_merge(
    chunks: Vec<PathBuf>,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    tx: Sender<OpEvent>,
) {
    use std::io::{Read, Write};
    let mut ctx = stream_ctx(&cancel, &paused, &tx);
    let mut fatal: Option<String> = None;
    'done: {
        if chunks.is_empty() {
            fatal = Some("无分块可合并".to_string());
            break 'done;
        }
        // 预扫描：全部分块可读才开工（缺/坏分块不产半成品输出）。
        let mut total_bytes = 0u64;
        for c in &chunks {
            match std::fs::symlink_metadata(verbatim_path(c)) {
                Ok(m) if m.is_file() => total_bytes += m.len(),
                _ => {
                    ctx.errors
                        .push((c.clone(), "分块不存在或不是文件".to_string()));
                }
            }
        }
        if !ctx.errors.is_empty() {
            break 'done;
        }
        let expected_crc = expected_crc_for(&dest);
        ctx.progress.total_files = chunks.len() as u64;
        ctx.progress.total_bytes = total_bytes;
        ctx.send_progress();
        let mut out = match std::fs::File::create(verbatim_path(&dest)) {
            Ok(f) => f,
            Err(e) => {
                fatal = Some(format!("无法创建目标文件 {}: {e}", dest.display()));
                break 'done;
            }
        };
        let mut crc = crc32fast::Hasher::new();
        let mut buf = vec![0u8; COPY_CHUNK];
        let mut last_sent = Instant::now();
        for chunk in &chunks {
            if ctx.wait_if_paused() {
                break;
            }
            ctx.progress.current = chunk.clone();
            let mut input = match std::fs::File::open(verbatim_path(chunk)) {
                Ok(f) => f,
                Err(e) => {
                    fatal = Some(format!("无法打开分块 {}: {e}", chunk.display()));
                    break;
                }
            };
            loop {
                if ctx.wait_if_paused() {
                    break;
                }
                match input.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Err(e) = out.write_all(&buf[..n]) {
                            fatal = Some(format!("写入目标失败: {e}"));
                            break;
                        }
                        crc.update(&buf[..n]);
                        ctx.progress.done_bytes += n as u64;
                        if last_sent.elapsed() >= PROGRESS_INTERVAL {
                            ctx.send_progress();
                            last_sent = Instant::now();
                        }
                    }
                    Err(e) => {
                        fatal = Some(format!("读取分块失败: {e}"));
                        break;
                    }
                }
            }
            if ctx.cancelled || fatal.is_some() {
                break;
            }
            ctx.progress.done_files += 1;
            ctx.send_progress();
        }
        drop(out);
        if ctx.cancelled || fatal.is_some() {
            let _ = std::fs::remove_file(verbatim_path(&dest));
            break 'done;
        }
        if let Some(expected) = expected_crc {
            let actual = format!("{:08x}", crc.finalize());
            if !actual.eq_ignore_ascii_case(&expected) {
                ctx.errors.push((
                    dest.clone(),
                    format!("CRC 校验不一致（期望 {expected}，实际 {actual}；结果已保留）"),
                ));
            }
        }
    }
    let _ = tx.send(OpEvent::Finished {
        cancelled: ctx.cancelled,
        fatal,
        errors: ctx.errors,
        written: ctx.written,
    });
}

struct OpCtx<'a> {
    cancel: &'a AtomicBool,
    paused: &'a AtomicBool,
    tx: &'a Sender<OpEvent>,
    /// Ask 模式的回答接收端（非 Ask 为 None）。
    answer_rx: Option<&'a Receiver<ConflictAnswer>>,
    /// 复制/移动高级选项（阶段 AB 校验/过滤；Delete 为默认）。
    opts: &'a CopyOptions,
    /// 本 source 内发生过过滤跳过或校验失败（阶段 AB）：Move 回退路径
    /// 据此放弃 trash 源（防部分搬运时整删源丢数据）。每 source 处理前
    /// 由 run_op 重置。
    drop_source_unsafe: bool,
    /// 过滤移动（阶段 AB，kind==Move 且过滤开启）：copy_recursive 内逐
    /// 文件 trash 已拷源文件、remove_dir 清空的源目录（TC 式部分搬运）；
    /// false = 回退路径成功后整源 trash（现状）。
    move_prune: bool,
    /// Ask 模式 apply_all 记忆：后续同级冲突的生效策略（文件级/目录级
    /// 各自独立——目录级「全部应用」不预决文件级策略）。
    remembered_file: Option<ConflictAction>,
    remembered_dir: Option<ConflictAction>,
    progress: OpProgress,
    errors: Vec<(PathBuf, String)>,
    /// 实际写入的顶层目标与对应源（阶段 AG 撤销；Copy = 成功复制项，
    /// Move = 源已移除项，AutoRename 后为准）。
    written: Vec<(PathBuf, PathBuf)>,
    cancelled: bool,
}

/// 冲突消解结果（resolve_dst 返回值）。
enum Resolve {
    /// 继续：Overwrite 原路径 / AutoRename 新路径 / 无冲突原路径 / 目录合并。
    Proceed(PathBuf),
    /// 跳过（调用方记 errors 汇总）。
    Skip,
    /// 取消整批（用户选 Cancel / 等待期间被 cancel / 回答通道断开）。
    Cancelled,
}

impl OpCtx<'_> {
    /// 取消检查：置位时标记 cancelled 并返回 true（调用方应立即收拢退出）。
    fn halt(&mut self) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            self.cancelled = true;
            true
        } else {
            false
        }
    }

    /// 块/项边界的暂停等待：暂停期间按 PAUSE_POLL 轮询（不发新进度，
    /// UI 经 paused 快照显示「已暂停」），cancel 即时生效；返回 true =
    /// 已取消（语义同 halt，调用方收拢退出）。
    fn wait_if_paused(&mut self) -> bool {
        while self.paused.load(Ordering::Relaxed) {
            if self.halt() {
                return true;
            }
            std::thread::sleep(PAUSE_POLL);
        }
        self.halt()
    }

    fn send_progress(&mut self) {
        let _ = self.tx.send(OpEvent::Progress(self.progress.clone()));
    }

    /// 冲突消解（Ask 模式含执行期问答）：目标不存在 → 原样；目录↔目录
    /// 非 Ask 恒合并（递归内部逐项处理，不整删目标目录）；Ask 模式逐个
    /// 询问（目录问「合并/跳过」，文件问「覆盖/跳过/自动改名/取消」，
    /// apply_all 记忆同级策略）；其余按模式静态消解。
    fn resolve_dst(&mut self, src: &Path, dst: &Path, mode: ConflictMode) -> Resolve {
        if !verbatim_path(dst).exists() {
            return Resolve::Proceed(dst.to_path_buf());
        }
        let dir_dir = verbatim_path(src).is_dir() && verbatim_path(dst).is_dir();
        match mode {
            ConflictMode::Overwrite => Resolve::Proceed(dst.to_path_buf()),
            ConflictMode::AutoRename | ConflictMode::Skip if dir_dir => {
                Resolve::Proceed(dst.to_path_buf())
            }
            ConflictMode::AutoRename => Resolve::Proceed(resolve_conflict_name(dst)),
            ConflictMode::Skip => Resolve::Skip,
            ConflictMode::Ask => self.resolve_ask(src, dst, dir_dir),
        }
    }

    /// Ask 模式问答：先看 apply_all 记忆，否则发 ConflictQuery 阻塞等答。
    fn resolve_ask(&mut self, src: &Path, dst: &Path, dir_dir: bool) -> Resolve {
        let remembered = if dir_dir {
            self.remembered_dir
        } else {
            self.remembered_file
        };
        let action = match remembered {
            Some(action) => action,
            None => {
                let Some(answer) = self.ask_conflict(src, dst, dir_dir) else {
                    self.cancelled = true;
                    return Resolve::Cancelled;
                };
                if answer.apply_all && answer.action != ConflictAction::Cancel {
                    if dir_dir {
                        self.remembered_dir = Some(answer.action);
                    } else {
                        self.remembered_file = Some(answer.action);
                    }
                }
                answer.action
            }
        };
        match action {
            ConflictAction::Overwrite => Resolve::Proceed(dst.to_path_buf()),
            ConflictAction::AutoRename => Resolve::Proceed(resolve_conflict_name(dst)),
            ConflictAction::Skip => Resolve::Skip,
            ConflictAction::Cancel => {
                self.cancelled = true;
                Resolve::Cancelled
            }
        }
    }

    /// 发问并阻塞等答：100ms 轮询 try_recv + cancel 检查（cancel 或通道
    /// 断开都按 None=Cancel 收拢）；无问答通道时按 Skip 兜底（不应发生：
    /// Ask 模式必配通道）。问答期间进度消息暂停（worker 阻塞），UI 侧
    /// 另有「等待确认…」展示。
    fn ask_conflict(&mut self, src: &Path, dst: &Path, is_dir: bool) -> Option<ConflictAnswer> {
        let Some(rx) = self.answer_rx else {
            return Some(ConflictAnswer {
                action: ConflictAction::Skip,
                apply_all: false,
            });
        };
        let stat = |p: &Path| {
            let m = std::fs::metadata(verbatim_path(p)).ok();
            let size = m.as_ref().filter(|m| m.is_file()).map(|m| m.len());
            let mtime = m.and_then(|m| m.modified().ok());
            (size, mtime)
        };
        let (src_size, src_mtime) = stat(src);
        let (dst_size, dst_mtime) = stat(dst);
        let query = ConflictQuery {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
            is_dir,
            src_size,
            src_mtime,
            dst_size,
            dst_mtime,
        };
        if self.tx.send(OpEvent::AskConflict(query)).is_err() {
            return None;
        }
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return None;
            }
            match rx.try_recv() {
                Ok(answer) => return Some(answer),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(CONFLICT_POLL);
                }
                // UI 侧 answer_tx 全部 drop（任务被移除等）：按 Cancel 收拢。
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

/// 预扫描单个 source：返回 (项数[文件+目录], 文件字节数)；
/// 符号链接按单项计（不跟进目录防环）；读取失败按 1 项 0 字节计。
/// 阶段 AB 过滤（Copy/Move；Delete 传默认不过滤）：不匹配的文件不计
/// （目录恒计入，结构保留）——与 copy_recursive 共用同一判定，进度
/// total 即过滤后集合。
fn count_source(path: &Path, opts: &CopyOptions) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(verbatim_path(path)) else {
        return (1, 0);
    };
    if meta.is_symlink() || !meta.is_dir() {
        // 顶层文件源同样参与过滤（与递归层一致，计数/执行不脱节）。
        if meta.is_file() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !opts.matches(&name, false, meta.modified().ok()) {
                return (0, 0);
            }
        }
        let bytes = if meta.is_file() { meta.len() } else { 0 };
        return (1, bytes);
    }
    let mut items = 1; // 目录本身
    let mut bytes = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(verbatim_path(&dir)) else {
            continue;
        };
        for item in rd.flatten() {
            let p = item.path();
            match item.file_type() {
                Ok(ft) if ft.is_symlink() || !ft.is_dir() => {
                    if ft.is_file() {
                        let meta = item.metadata().ok();
                        let name = item.file_name().to_string_lossy().to_string();
                        if !opts.matches(
                            &name,
                            false,
                            meta.as_ref().and_then(|m| m.modified().ok()),
                        ) {
                            continue; // 不匹配的文件：不计项数/字节
                        }
                        bytes += meta.map(|m| m.len()).unwrap_or(0);
                    } else if let Ok(m) = item.metadata() {
                        // 符号链接到文件等：沿用原语义计入目标大小。
                        if m.is_file() {
                            bytes += m.len();
                        }
                    }
                    items += 1;
                }
                Ok(_) => {
                    items += 1;
                    stack.push(p);
                }
                Err(_) => items += 1,
            }
        }
    }
    (items, bytes)
}

/// 自动改名：`name (1).ext`、`name (2).ext`…（同 parser extract 的
/// uniquify 语义）；目录与无扩展名文件同样适用。
pub fn resolve_conflict_name(path: &Path) -> PathBuf {
    if !verbatim_path(path).exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for i in 1..1000u32 {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !verbatim_path(&candidate).exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// 目录总大小（字节）：递归累加文件 len；不跟进符号链接（防环，链接本身
/// 不计入）；单项 read_dir/file_type/metadata 失败跳过不计；cancel 置位
/// 提前返回已累加值。文件管理器「计算大小」的后台 worker 用。
pub(crate) fn dir_size(path: &Path, cancel: &AtomicBool) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return total;
        }
        let Ok(rd) = std::fs::read_dir(verbatim_path(&dir)) else {
            continue;
        };
        for item in rd.flatten() {
            let Ok(ft) = item.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(item.path());
            } else if let Ok(m) = item.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Windows 长路径（>MAX_PATH=260）支持：本程序 manifest 未声明
/// longPathAware，Win32 文件 API 默认拒绝超长路径，需加 `\\?\`
/// （verbatim）前缀。仅转换超过 240 字符（留余量）的绝对路径——普通路径
/// 原样返回，避免 verbatim 前缀经 `read_dir` 传播进错误信息；已是
/// verbatim 前缀的原样返回；UNC `\\server\share` 转 `\\?\UNC\server\share`。
/// 非 Windows 恒等。
#[cfg(windows)]
pub(crate) fn verbatim_path(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    if !p.is_absolute()
        || s.starts_with(r"\\?\")
        || s.starts_with(r"\\.\")
        || s.chars().count() <= 240
    {
        return p.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    PathBuf::from(format!(r"\\?\{s}"))
}

#[cfg(not(windows))]
pub(crate) fn verbatim_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// 递归复制 src → dst（dst 已按顶层冲突策略消解）：目录建目录并合并进入，
/// 文件分块复制（块间响应取消/暂停，取消时删除半成品目标文件）。
/// 阶段 AB：文件先过 opts 过滤（不匹配跳过并置 drop_source_unsafe，目录
/// 恒保留）；opts.verify 时写完后重读双端分块比对（verify_copied_file）。
/// 返回 Err 仅表示已取消（errors 里已记单项失败）。
fn copy_recursive(src: &Path, dst: &Path, mode: ConflictMode, ctx: &mut OpCtx) -> Result<(), ()> {
    if ctx.wait_if_paused() {
        return Err(());
    }
    let meta = std::fs::symlink_metadata(verbatim_path(src)).ok();
    let is_dir = meta
        .as_ref()
        .map(|m| m.is_dir() && !m.is_symlink())
        .unwrap_or(false);
    if is_dir {
        // 进入目录：无文件内进度，清零隐藏状态栏当前文件条。
        ctx.progress.cur_done_bytes = 0;
        ctx.progress.cur_total_bytes = 0;
        if let Err(e) = std::fs::create_dir_all(verbatim_path(dst)) {
            ctx.errors
                .push((dst.to_path_buf(), format!("无法创建目录: {e}")));
        }
        ctx.progress.done_files += 1;
        ctx.send_progress();
        let Ok(rd) = std::fs::read_dir(verbatim_path(src)) else {
            ctx.errors
                .push((src.to_path_buf(), "无法读取目录内容".to_string()));
            return Ok(());
        };
        for item in rd.flatten() {
            let child_src = item.path();
            let child_dst = dst.join(item.file_name());
            // 递归内部逐项冲突消解（目录↔目录合并在 resolve_dst 内判定）。
            let child_dst = match ctx.resolve_dst(&child_src, &child_dst, mode) {
                Resolve::Proceed(d) => d,
                Resolve::Skip => {
                    ctx.errors
                        .push((child_src.clone(), "目标已存在，已跳过".to_string()));
                    continue;
                }
                Resolve::Cancelled => return Err(()),
            };
            copy_recursive(&child_src, &child_dst, mode, ctx)?;
        }
        // 过滤移动（阶段 AB）：移除已清空的源目录（非空 = 有残留，保留）。
        if ctx.move_prune {
            let _ = std::fs::remove_dir(verbatim_path(src));
        }
        Ok(())
    } else {
        // 阶段 AB 过滤：不匹配的文件跳过（与 count_source 同一判定，进度
        // total 即过滤后集合——跳过即完成，不动进度）；目录结构保留。
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        if !ctx.opts.matches(&name, false, mtime) {
            ctx.drop_source_unsafe = true; // Move：源不可整删
            return Ok(());
        }
        match copy_file_chunks(src, dst, ctx) {
            Ok(bytes) => {
                // 复制后校验（阶段 AB）：重读双端分块比对，不一致记 errors
                // 并标 drop_source_unsafe（Move 不删源）。
                let mut unsafe_to_prune = false;
                if ctx.opts.verify {
                    match verify_copied_file(src, dst, ctx) {
                        Ok(()) => {}
                        Err(CopyFail::Cancelled) => return Err(()),
                        Err(CopyFail::Io(e)) => {
                            ctx.errors.push((src.to_path_buf(), e));
                            ctx.drop_source_unsafe = true;
                            unsafe_to_prune = true;
                        }
                    }
                }
                ctx.progress.done_files += 1;
                ctx.progress.done_bytes += bytes;
                ctx.send_progress();
                // 过滤移动（阶段 AB）：逐文件 trash 已拷源文件；校验失败/
                // 删源失败的保留在源。
                if ctx.move_prune && !unsafe_to_prune {
                    if let Err(e) = trash::delete(src) {
                        ctx.errors
                            .push((src.to_path_buf(), format!("源文件删除失败: {e}")));
                        ctx.drop_source_unsafe = true;
                    }
                }
            }
            Err(CopyFail::Cancelled) => return Err(()),
            Err(CopyFail::Io(e)) => {
                ctx.errors.push((src.to_path_buf(), e));
                // 移动语义：复制失败的源文件保留 → 源不可整删。
                ctx.drop_source_unsafe = true;
            }
        }
        Ok(())
    }
}

enum CopyFail {
    Cancelled,
    Io(String),
}

/// 分块复制文件：维护当前文件内进度（cur_done/cur_total_bytes），每块后
/// 检查暂停/取消并按 PROGRESS_INTERVAL 节流上报（文件结束的上报由
/// copy_recursive 的逐项进度兜底）；取消时删除半成品目标文件。
fn copy_file_chunks(src: &Path, dst: &Path, ctx: &mut OpCtx) -> Result<u64, CopyFail> {
    use std::io::{Read, Write};
    if ctx.wait_if_paused() {
        return Err(CopyFail::Cancelled);
    }
    let mut reader = std::fs::File::open(verbatim_path(src))
        .map_err(|e| CopyFail::Io(format!("无法读取: {e}")))?;
    let mut writer = std::fs::File::create(verbatim_path(dst))
        .map_err(|e| CopyFail::Io(format!("无法创建目标: {e}")))?;
    // 进入新文件：文件内进度归零并设总量（元数据失败按 0 = 不显示）。
    ctx.progress.cur_done_bytes = 0;
    ctx.progress.cur_total_bytes = reader.metadata().map(|m| m.len()).unwrap_or(0);
    let mut last_send = Instant::now();
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut written = 0u64;
    loop {
        if ctx.wait_if_paused() {
            drop(writer);
            let _ = std::fs::remove_file(verbatim_path(dst));
            return Err(CopyFail::Cancelled);
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = writer.write_all(&buf[..n]) {
                    drop(writer);
                    let _ = std::fs::remove_file(verbatim_path(dst));
                    return Err(CopyFail::Io(format!("写入失败: {e}")));
                }
                written += n as u64;
                ctx.progress.cur_done_bytes = written;
                if last_send.elapsed() >= PROGRESS_INTERVAL {
                    ctx.send_progress();
                    last_send = Instant::now();
                }
            }
            Err(e) => {
                drop(writer);
                let _ = std::fs::remove_file(verbatim_path(dst));
                return Err(CopyFail::Io(format!("读取失败: {e}")));
            }
        }
    }
    Ok(written)
}

enum VerifyFail {
    Cancelled,
    Io(String),
}

/// 分块字节比对（复制后校验核心）：长度快查 + 256KB 分块读双端比较；
/// 每块后查 cancel（校验不删目标——文件已写完，取消只中止校验）；
/// on_chunk 回调已校验字节数（驱动 cur_done_bytes 第二轮进度）。
fn files_identical(
    a: &Path,
    b: &Path,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(u64),
) -> Result<(), VerifyFail> {
    use std::io::Read;
    let len_a = std::fs::metadata(verbatim_path(a))
        .map_err(|e| VerifyFail::Io(format!("无法读取源: {e}")))?
        .len();
    let len_b = std::fs::metadata(verbatim_path(b))
        .map_err(|e| VerifyFail::Io(format!("无法读取目标: {e}")))?
        .len();
    if len_a != len_b {
        return Err(VerifyFail::Io(format!("长度不一致（{len_a} ≠ {len_b}）")));
    }
    let mut ra = std::fs::File::open(verbatim_path(a))
        .map_err(|e| VerifyFail::Io(format!("无法读取源: {e}")))?;
    let mut rb = std::fs::File::open(verbatim_path(b))
        .map_err(|e| VerifyFail::Io(format!("无法读取目标: {e}")))?;
    let mut buf_a = vec![0u8; COPY_CHUNK];
    let mut buf_b = vec![0u8; COPY_CHUNK];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(VerifyFail::Cancelled);
        }
        let na = ra
            .read(&mut buf_a)
            .map_err(|e| VerifyFail::Io(format!("读取源失败: {e}")))?;
        let nb = rb
            .read(&mut buf_b)
            .map_err(|e| VerifyFail::Io(format!("读取目标失败: {e}")))?;
        if na == 0 && nb == 0 {
            break;
        }
        if na != nb || buf_a[..na] != buf_b[..nb] {
            return Err(VerifyFail::Io(format!("内容不一致（偏移 {done} 附近）")));
        }
        done += na as u64;
        on_chunk(done);
    }
    Ok(())
}

/// 复制后校验（阶段 AB）：文件内进度归零走第二轮（cur_total 不变 =
/// 文件大小，不新增字段）；不一致/读取失败 → CopyFail::Io（「校验失败」
/// 前缀由调用方记 errors 时带出）；校验期暂停/取消在块边界生效。
fn verify_copied_file(src: &Path, dst: &Path, ctx: &mut OpCtx) -> Result<(), CopyFail> {
    ctx.progress.cur_done_bytes = 0;
    ctx.send_progress();
    if ctx.wait_if_paused() {
        return Err(CopyFail::Cancelled);
    }
    let mut cancelled = false;
    let mut last_send = Instant::now();
    let cancel = ctx.cancel;
    let result = files_identical(src, dst, cancel, &mut |done| {
        ctx.progress.cur_done_bytes = done;
        if ctx.wait_if_paused() {
            cancelled = true;
        }
        if last_send.elapsed() >= PROGRESS_INTERVAL {
            ctx.send_progress();
            last_send = Instant::now();
        }
    });
    if cancelled {
        return Err(CopyFail::Cancelled);
    }
    match result {
        Ok(()) => Ok(()),
        Err(VerifyFail::Cancelled) => Err(CopyFail::Cancelled),
        Err(VerifyFail::Io(e)) => Err(CopyFail::Io(format!("校验失败: {e}"))),
    }
}

/// 条目名校验（重命名/新建文件夹共用）：非空、非 . 或 ..、不含 Windows
/// 非法字符 `\\/:*?"<>|`。
pub fn validate_entry_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name == "." || name == ".." {
        return Err("非法名称".to_string());
    }
    if let Some(c) = name.chars().find(|c| "\\/:*?\"<>|".contains(*c)) {
        return Err(format!("名称不能包含字符「{c}」"));
    }
    Ok(())
}

/// 重命名（瞬时操作）：同目录 fs::rename；重名/非法名校验失败返回 Err。
pub fn rename_entry(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_entry_name(new_name)?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let new_path = parent.join(new_name.trim());
    if new_path == path {
        return Ok(new_path);
    }
    if verbatim_path(&new_path).exists() {
        return Err("已存在同名文件或文件夹".to_string());
    }
    std::fs::rename(verbatim_path(path), verbatim_path(&new_path))
        .map_err(|e| format!("重命名失败: {e}"))?;
    Ok(new_path)
}

/// 新建文件夹（瞬时操作）：重名/非法名校验失败返回 Err。
pub fn create_dir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_entry_name(name)?;
    let path = parent.join(name.trim());
    if verbatim_path(&path).exists() {
        return Err("已存在同名文件或文件夹".to_string());
    }
    std::fs::create_dir(verbatim_path(&path)).map_err(|e| format!("无法创建文件夹: {e}"))?;
    Ok(path)
}

/// 新建文件夹的默认名建议：「新建文件夹」，重名时「新建文件夹 (2)」递增。
pub fn suggest_folder_name(parent: &Path) -> String {
    let base = "新建文件夹";
    if !verbatim_path(&parent.join(base)).exists() {
        return base.to_string();
    }
    for i in 2..1000u32 {
        let candidate = format!("{base} ({i})");
        if !verbatim_path(&parent.join(&candidate)).exists() {
            return candidate;
        }
    }
    base.to_string()
}

/// 新建文本文件（瞬时操作）：create_new 防竞态覆盖已存在文件；
/// 重名/非法名校验失败返回 Err。
pub fn create_text_file(parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_entry_name(name)?;
    let path = parent.join(name.trim());
    if verbatim_path(&path).exists() {
        return Err("已存在同名文件或文件夹".to_string());
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(verbatim_path(&path))
        .map_err(|e| format!("无法创建文件: {e}"))?;
    Ok(path)
}

/// 新建文本文件的默认名建议：「新建文本文件.txt」，重名时
/// 「新建文本文件 (2).txt」递增（扩展名固定在末尾）。
pub fn suggest_text_file_name(parent: &Path) -> String {
    let base = "新建文本文件";
    let ext = ".txt";
    let candidate = format!("{base}{ext}");
    if !verbatim_path(&parent.join(&candidate)).exists() {
        return candidate;
    }
    for i in 2..1000u32 {
        let candidate = format!("{base} ({i}){ext}");
        if !verbatim_path(&parent.join(&candidate)).exists() {
            return candidate;
        }
    }
    format!("{base}{ext}")
}

/// 状态栏速度/ETA 估算：对 done_bytes 的增量做指数滑动平均（EMA）。
/// 纯函数式采样（now 由调用方传入），与 egui 无关，可单测。
pub struct OpSpeedMeter {
    /// 上次采样（时间, done_bytes）。
    last: Option<(Instant, u64)>,
    /// 平滑速度（B/s）。
    ema_bps: Option<f64>,
}

impl OpSpeedMeter {
    /// 采样最小间隔：过密的样本直接忽略（UI 每帧调用，内部节流）。
    pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
    /// EMA 平滑系数。
    const ALPHA: f64 = 0.3;

    pub fn new() -> Self {
        Self {
            last: None,
            ema_bps: None,
        }
    }

    /// 记录一次采样，返回当前平滑速度（B/s）。首样本只记基准返回 None；
    /// 距上次 < SAMPLE_INTERVAL 忽略（返回既有平滑值）。done 倒退
    /// （新任务复用等）按 0 速度计。
    pub fn sample(&mut self, now: Instant, done_bytes: u64) -> Option<f64> {
        match self.last {
            None => {
                self.last = Some((now, done_bytes));
                None
            }
            Some((t, b)) => {
                let dt = now.duration_since(t);
                if dt < Self::SAMPLE_INTERVAL {
                    return self.ema_bps;
                }
                let inst = done_bytes.saturating_sub(b) as f64 / dt.as_secs_f64();
                self.ema_bps = Some(match self.ema_bps {
                    Some(ema) => Self::ALPHA * inst + (1.0 - Self::ALPHA) * ema,
                    None => inst,
                });
                self.last = Some((now, done_bytes));
                self.ema_bps
            }
        }
    }

    /// 当前平滑速度（B/s）；尚无有效样本为 None。
    pub fn speed_bps(&self) -> Option<f64> {
        self.ema_bps
    }

    /// ETA（秒）：速度未知/为 0 或已完成时 None。
    pub fn eta_secs(&self, total_bytes: u64, done_bytes: u64) -> Option<f64> {
        let speed = self.ema_bps?;
        if speed <= 0.0 || done_bytes >= total_bytes {
            return None;
        }
        Some((total_bytes - done_bytes) as f64 / speed)
    }
}

impl Default for OpSpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// ETA 显示格式：「~45s」「~3m05s」「~2h07m」。
pub fn format_eta(secs: f64) -> String {
    let s = secs.round().max(0.0) as u64;
    if s < 60 {
        format!("~{s}s")
    } else if s < 3600 {
        format!("~{}m{:02}s", s / 60, s % 60)
    } else {
        format!("~{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 唯一临时子目录；Drop 时清理。
    struct TempTree(PathBuf);
    impl TempTree {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "openitgo-fileops-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn count_source_recurses_and_skips_nothing() {
        let t = TempTree::new("count");
        write_file(&t.path().join("a.txt"), b"1234");
        write_file(&t.path().join("sub/b.txt"), b"123456");
        write_file(&t.path().join("sub/deep/c.txt"), b"12");
        let (items, bytes) = count_source(&t.path().join("sub"), &CopyOptions::default());
        // sub + deep + b.txt + c.txt
        assert_eq!(items, 4);
        assert_eq!(bytes, 8);
    }

    #[test]
    fn dir_size_sums_nested_files() {
        let t = TempTree::new("dirsize");
        write_file(&t.path().join("a.txt"), b"1234");
        write_file(&t.path().join("sub/b.txt"), b"123456");
        write_file(&t.path().join("sub/deep/c.txt"), b"12");
        let cancel = AtomicBool::new(false);
        assert_eq!(dir_size(t.path(), &cancel), 12);
        assert_eq!(dir_size(&t.path().join("sub"), &cancel), 8);
    }

    #[test]
    fn dir_size_cancel_returns_partial_early() {
        let t = TempTree::new("dirsize-cancel");
        write_file(&t.path().join("a.txt"), b"1234");
        let cancel = AtomicBool::new(true);
        assert_eq!(dir_size(t.path(), &cancel), 0);
    }

    /// 符号链接目录不跟进（防环）：链接指向的内容不计入。
    #[cfg(unix)]
    #[test]
    fn dir_size_does_not_follow_symlinks() {
        let t = TempTree::new("dirsize-symlink");
        write_file(&t.path().join("real/x.txt"), b"1234");
        std::os::unix::fs::symlink(t.path().join("real"), t.path().join("link")).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(dir_size(t.path(), &cancel), 4);
    }

    #[test]
    fn resolve_conflict_name_increments() {
        let t = TempTree::new("rename");
        let a = t.path().join("a.txt");
        write_file(&a, b"orig");
        let renamed = resolve_conflict_name(&a);
        assert_eq!(renamed, t.path().join("a (1).txt"));
        write_file(&renamed, b"second");
        assert_eq!(resolve_conflict_name(&a), t.path().join("a (2).txt"));
        // 不冲突时原样返回
        let free = t.path().join("free.txt");
        assert_eq!(resolve_conflict_name(&free), free);
        // 目录同样适用
        let dir = t.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(resolve_conflict_name(&dir), t.path().join("d (1)"));
    }

    /// 直接驱动工作线程函数（同步），便于断言。
    fn run_copy_sync(sources: Vec<PathBuf>, dest: PathBuf, mode: ConflictMode) -> FinishedLike {
        run_op_sync(
            OpKind::Copy,
            sources,
            Some(dest),
            mode,
            Arc::new(AtomicBool::new(false)),
        )
    }

    struct FinishedLike {
        cancelled: bool,
        errors: Vec<(PathBuf, String)>,
    }

    fn run_op_sync(
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        mode: ConflictMode,
        cancel: Arc<AtomicBool>,
    ) -> FinishedLike {
        run_op_sync_opts(kind, sources, dest, mode, cancel, CopyOptions::default())
    }

    /// 带高级选项的同步驱动（阶段 AB 校验/过滤测试用）。
    fn run_op_sync_opts(
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        mode: ConflictMode,
        cancel: Arc<AtomicBool>,
        opts: CopyOptions,
    ) -> FinishedLike {
        let (tx, rx) = channel();
        run_op(
            kind,
            sources,
            dest,
            mode,
            cancel,
            Arc::new(AtomicBool::new(false)),
            tx,
            None,
            false,
            opts,
        );
        let mut finished = None;
        while let Ok(ev) = rx.try_recv() {
            if let OpEvent::Finished {
                cancelled, errors, ..
            } = ev
            {
                finished = Some(FinishedLike { cancelled, errors });
            }
        }
        finished.expect("worker must finish")
    }

    #[test]
    fn copy_recursive_with_auto_rename() {
        let t = TempTree::new("copy");
        let src_dir = t.path().join("src");
        write_file(&src_dir.join("a.txt"), b"new content");
        write_file(&src_dir.join("sub/b.txt"), b"sub file");
        let dest = t.path().join("dest");
        // 预置冲突：目标已有同名（不同内容）文件与同名目录。
        write_file(&dest.join("src/a.txt"), b"old content");
        std::fs::create_dir_all(dest.join("src/sub")).unwrap();

        let r = run_copy_sync(
            vec![src_dir.clone()],
            dest.clone(),
            ConflictMode::AutoRename,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!r.cancelled);
        // 目录合并：sub/b.txt 落进已有目录；冲突文件改名。
        assert_eq!(
            std::fs::read(dest.join("src/sub/b.txt")).unwrap(),
            b"sub file"
        );
        assert_eq!(
            std::fs::read(dest.join("src/a (1).txt")).unwrap(),
            b"new content"
        );
        // 原文件不动
        assert_eq!(
            std::fs::read(dest.join("src/a.txt")).unwrap(),
            b"old content"
        );
        // 源保留
        assert!(src_dir.join("a.txt").exists());
    }

    #[test]
    fn copy_skip_mode_records_conflict() {
        let t = TempTree::new("skip");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let r = run_copy_sync(vec![src], dest.clone(), ConflictMode::Skip);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");
    }

    fn answer(action: ConflictAction, apply_all: bool) -> ConflictAnswer {
        ConflictAnswer { action, apply_all }
    }

    /// Ask 模式问答测试驱动：worker 线程跑 run_op，主线程收 AskConflict
    /// 按脚本回发（脚本外再来询问 = panic），收 Finished 收尾；
    /// 返回 (结果, 实际收到的全部询问)。
    fn run_ask_sync(
        sources: Vec<PathBuf>,
        dest: PathBuf,
        answers: Vec<ConflictAnswer>,
    ) -> (FinishedLike, Vec<ConflictQuery>) {
        let (tx, rx) = channel();
        let (atx, arx) = channel();
        let handle = std::thread::spawn(move || {
            run_op(
                OpKind::Copy,
                sources,
                Some(dest),
                ConflictMode::Ask,
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                tx,
                Some(arx),
                false,
                CopyOptions::default(),
            );
        });
        let mut queries = Vec::new();
        let mut answers = answers.into_iter();
        let mut finished = None;
        // worker 退出后 tx 断开，recv 出错收尾。
        while let Ok(ev) = rx.recv() {
            match ev {
                OpEvent::AskConflict(q) => {
                    let ans = answers.next().expect("询问数超脚本");
                    atx.send(ans).unwrap();
                    queries.push(q);
                }
                OpEvent::Finished {
                    cancelled, errors, ..
                } => {
                    finished = Some(FinishedLike { cancelled, errors });
                }
                OpEvent::Progress(_) => {}
            }
        }
        handle.join().unwrap();
        (finished.expect("worker must finish"), queries)
    }

    #[test]
    fn ask_conflict_overwrite_skip_rename_cancel() {
        // 覆盖
        let t = TempTree::new("ask-ow");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let (r, qs) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::Overwrite, false)],
        );
        assert!(!r.cancelled && r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(qs.len(), 1);
        assert!(!qs[0].is_dir);
        assert_eq!(qs[0].src_size, Some(3));
        assert_eq!(qs[0].dst_size, Some(3));
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"new");

        // 跳过
        let t = TempTree::new("ask-skip");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let (r, _) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::Skip, false)],
        );
        assert!(!r.cancelled);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");

        // 自动改名
        let t = TempTree::new("ask-rn");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let (r, _) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::AutoRename, false)],
        );
        assert!(!r.cancelled && r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(dest.join("a (1).txt")).unwrap(), b"new");

        // 取消操作：整批收拢，目标不动。
        let t = TempTree::new("ask-cancel");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let (r, _) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::Cancel, false)],
        );
        assert!(r.cancelled);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");
    }

    #[test]
    fn ask_apply_all_remembers_per_level() {
        // 文件级 apply_all：两个冲突文件只问一次，第二个自动覆盖。
        let t = TempTree::new("ask-all");
        let src = t.path().join("s");
        write_file(&src.join("a.txt"), b"new-a");
        write_file(&src.join("b.txt"), b"new-b");
        let dest = t.path().join("dest");
        write_file(&dest.join("s/a.txt"), b"old-a");
        write_file(&dest.join("s/b.txt"), b"old-b");
        let (r, qs) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![
                answer(ConflictAction::Overwrite, true), // 目录冲突（合并，全部应用）
                answer(ConflictAction::Overwrite, true), // 首个文件冲突（覆盖，全部应用）
            ],
        );
        assert!(!r.cancelled && r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(qs.len(), 2, "目录问一次 + 文件只问首个: {qs:?}");
        assert!(qs[0].is_dir);
        assert!(!qs[1].is_dir);
        assert_eq!(std::fs::read(dest.join("s/a.txt")).unwrap(), b"new-a");
        assert_eq!(std::fs::read(dest.join("s/b.txt")).unwrap(), b"new-b");

        // 目录级 apply_all 不预决文件级：目录「合并+全部」后文件仍逐个问
        // （此处文件首问答「跳过+全部」→ 第二个文件免问自动跳过）。
        let t = TempTree::new("ask-levels");
        let src = t.path().join("s");
        write_file(&src.join("a.txt"), b"new-a");
        write_file(&src.join("b.txt"), b"new-b");
        let dest = t.path().join("dest");
        write_file(&dest.join("s/a.txt"), b"old-a");
        write_file(&dest.join("s/b.txt"), b"old-b");
        let (r, qs) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![
                answer(ConflictAction::Overwrite, true), // 目录：合并，全部应用
                answer(ConflictAction::Skip, true),      // 首个文件：跳过，全部应用
            ],
        );
        assert!(!r.cancelled);
        assert_eq!(
            qs.len(),
            2,
            "目录记忆不预决文件，文件记忆免问第二个: {qs:?}"
        );
        assert_eq!(r.errors.len(), 2, "两个文件都跳过记汇总: {:?}", r.errors);
        assert_eq!(std::fs::read(dest.join("s/a.txt")).unwrap(), b"old-a");
        assert_eq!(std::fs::read(dest.join("s/b.txt")).unwrap(), b"old-b");
    }

    #[test]
    fn ask_dir_conflict_merge_or_skip() {
        // 合并：递归进入逐项处理，目标已有内容保留。
        let t = TempTree::new("ask-merge");
        let src = t.path().join("s");
        write_file(&src.join("f.txt"), b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("s/other.txt"), b"keep");
        let (r, qs) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::Overwrite, false)],
        );
        assert!(!r.cancelled && r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(qs.len(), 1);
        assert!(qs[0].is_dir);
        assert_eq!(qs[0].src_size, None, "目录不给大小");
        assert_eq!(std::fs::read(dest.join("s/f.txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(dest.join("s/other.txt")).unwrap(), b"keep");

        // 跳过：整目录不进，记汇总。
        let t = TempTree::new("ask-dirskip");
        let src = t.path().join("s");
        write_file(&src.join("f.txt"), b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("s/other.txt"), b"keep");
        let (r, _) = run_ask_sync(
            vec![src],
            dest.clone(),
            vec![answer(ConflictAction::Skip, false)],
        );
        assert!(!r.cancelled);
        assert_eq!(r.errors.len(), 1);
        assert!(!dest.join("s/f.txt").exists());
        assert_eq!(std::fs::read(dest.join("s/other.txt")).unwrap(), b"keep");
    }

    #[test]
    fn ask_wait_interrupted_by_cancel_flag() {
        // 等待回答期间置 cancel：按 Cancel 收拢（worker 不悬挂）。
        let t = TempTree::new("ask-waitcancel");
        let src = t.path().join("a.txt");
        write_file(&src, b"new");
        let dest = t.path().join("dest");
        write_file(&dest.join("a.txt"), b"old");
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let dest2 = dest.clone();
        let (tx, rx) = channel();
        let (_atx, arx) = channel::<ConflictAnswer>();
        let handle = std::thread::spawn(move || {
            run_op(
                OpKind::Copy,
                vec![src],
                Some(dest2),
                ConflictMode::Ask,
                c2,
                Arc::new(AtomicBool::new(false)),
                tx,
                Some(arx),
                false,
                CopyOptions::default(),
            );
        });
        // 收到询问后不答，直接置 cancel。
        loop {
            match rx.recv() {
                Ok(OpEvent::AskConflict(_)) => break,
                Ok(_) => {}
                Err(_) => panic!("worker 退出前未发问"),
            }
        }
        cancel.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let cancelled = rx
            .try_iter()
            .any(|ev| matches!(ev, OpEvent::Finished { cancelled, .. } if cancelled));
        assert!(cancelled);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");
    }

    #[test]
    fn move_same_disk_uses_rename_fast_path() {
        let t = TempTree::new("move");
        let src = t.path().join("m.txt");
        write_file(&src, b"data");
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let r = run_op_sync(
            OpKind::Move,
            vec![src.clone()],
            Some(dest.clone()),
            ConflictMode::AutoRename,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!src.exists());
        assert_eq!(std::fs::read(dest.join("m.txt")).unwrap(), b"data");
    }

    #[test]
    fn cancel_before_start_cleans_partial_and_marks_cancelled() {
        let t = TempTree::new("cancel");
        let src = t.path().join("big.bin");
        write_file(&src, &vec![7u8; COPY_CHUNK * 4]);
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        // 预置的取消标志：第一块前就停，半成品目标不得残留。
        let cancel = Arc::new(AtomicBool::new(true));
        let r = run_op_sync(
            OpKind::Copy,
            vec![src],
            Some(dest.clone()),
            ConflictMode::Overwrite,
            cancel,
        );
        assert!(r.cancelled);
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn copy_file_chunks_cancel_before_write_keeps_dst_absent() {
        let t = TempTree::new("partial");
        let src = t.path().join("f.bin");
        write_file(&src, &vec![1u8; COPY_CHUNK * 2]);
        let dst = t.path().join("out.bin");
        // 取消在写入前生效：目标从未创建（不得残留半成品）。
        let cancel = AtomicBool::new(true);
        let paused = AtomicBool::new(false);
        let (tx, _rx) = channel();
        let opts = CopyOptions::default();
        let mut ctx = OpCtx {
            cancel: &cancel,
            paused: &paused,
            tx: &tx,
            answer_rx: None,
            opts: &opts,
            drop_source_unsafe: false,
            move_prune: false,
            remembered_file: None,
            remembered_dir: None,
            progress: OpProgress::default(),
            errors: Vec::new(),
            written: Vec::new(),
            cancelled: false,
        };
        let r = copy_file_chunks(&src, &dst, &mut ctx);
        assert!(matches!(r, Err(CopyFail::Cancelled)));
        assert!(!dst.exists());
        assert!(src.exists());
    }

    #[test]
    fn rename_and_create_dir_validation() {
        let t = TempTree::new("rename-helper");
        let f = t.path().join("old.txt");
        write_file(&f, b"x");
        // 非法字符
        assert!(rename_entry(&f, "a/b").is_err());
        assert!(rename_entry(&f, "").is_err());
        // 正常改名
        let new = rename_entry(&f, "new.txt").unwrap();
        assert!(new.exists() && !f.exists());
        // 重名冲突
        write_file(&t.path().join("taken.txt"), b"y");
        assert!(rename_entry(&new, "taken.txt").is_err());
        // 新建文件夹
        let d = create_dir(t.path(), "新建文件夹").unwrap();
        assert!(d.is_dir());
        assert!(create_dir(t.path(), "新建文件夹").is_err());
        assert_eq!(suggest_folder_name(t.path()), "新建文件夹 (2)");
    }

    #[test]
    fn create_text_file_and_suggest() {
        let t = TempTree::new("new-text-file");
        // 默认建议名 + 创建为空文件
        assert_eq!(suggest_text_file_name(t.path()), "新建文本文件.txt");
        let f = create_text_file(t.path(), "新建文本文件.txt").unwrap();
        assert!(f.is_file());
        assert_eq!(std::fs::metadata(&f).unwrap().len(), 0);
        // 重名 → 拒绝 + 建议名递增（扩展名保持末尾）
        assert!(create_text_file(t.path(), "新建文本文件.txt").is_err());
        assert_eq!(suggest_text_file_name(t.path()), "新建文本文件 (2).txt");
        // 非法字符
        assert!(create_text_file(t.path(), "a/b.txt").is_err());
    }

    /// 删除走回收站：Windows 本机实跑（CI Linux 无回收站/gio）。
    #[cfg(windows)]
    #[test]
    fn delete_moves_to_recycle_bin() {
        let t = TempTree::new("trash");
        let f = t.path().join("victim.txt");
        write_file(&f, b"bye");
        let r = run_op_sync(
            OpKind::Delete,
            vec![f.clone()],
            None,
            ConflictMode::Skip,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!f.exists());
    }

    /// 永久删除分派（fm_delete_mode = "permanent"）：文件与目录物理删除。
    #[test]
    fn delete_permanent_removes_files_and_dirs() {
        let t = TempTree::new("permanent-delete");
        let f = t.path().join("victim.txt");
        write_file(&f, b"bye");
        write_file(&t.path().join("dir/sub.txt"), b"nested");
        let dir = t.path().join("dir");
        let mut mgr = FileOpManager::default();
        mgr.start_delete(vec![f.clone(), dir.clone()], true);
        let mut finished = None;
        for _ in 0..200 {
            let summary = mgr.poll();
            if let Some(fin) = summary.finished.into_iter().next() {
                finished = Some(fin);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let fin = finished.expect("op should finish");
        assert!(fin.errors.is_empty(), "{:?}", fin.errors);
        assert!(!f.exists());
        assert!(!dir.exists());
        // 单测语义：单项失败不中断整批（不存在的源记 errors）。
        let mut mgr = FileOpManager::default();
        mgr.start_delete(vec![t.path().join("nonexistent")], true);
        let mut finished = None;
        for _ in 0..200 {
            let summary = mgr.poll();
            if let Some(fin) = summary.finished.into_iter().next() {
                finished = Some(fin);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(finished.expect("op should finish").errors.len(), 1);
    }

    #[test]
    fn manager_poll_lifecycle() {
        let t = TempTree::new("manager");
        let src = t.path().join("x.txt");
        write_file(&src, b"data");
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let mut mgr = FileOpManager::default();
        mgr.start_copy(vec![src], dest.clone(), ConflictMode::AutoRename);
        let mut finished = None;
        for _ in 0..200 {
            let summary = mgr.poll();
            if let Some(f) = summary.finished.into_iter().next() {
                finished = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = finished.expect("op should finish");
        assert_eq!(f.kind, OpKind::Copy);
        assert!(f.errors.is_empty());
        assert!(!mgr.poll().has_active);
        assert_eq!(std::fs::read(dest.join("x.txt")).unwrap(), b"data");
    }

    #[test]
    fn manager_compress_lifecycle() {
        let t = TempTree::new("compress");
        let src_dir = t.path().join("src");
        write_file(&src_dir.join("a.txt"), b"alpha");
        write_file(&src_dir.join("sub/b.txt"), b"beta");
        let dest_zip = t.path().join("out").join("src.zip");
        std::fs::create_dir_all(dest_zip.parent().unwrap()).unwrap();

        let mut mgr = FileOpManager::default();
        mgr.start_compress(vec![src_dir], dest_zip.clone());
        let mut finished = None;
        for _ in 0..200 {
            let summary = mgr.poll();
            if let Some(f) = summary.finished.into_iter().next() {
                finished = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = finished.expect("op should finish");
        assert_eq!(f.kind, OpKind::Compress);
        assert!(!f.cancelled);
        assert!(f.fatal.is_none());
        assert!(f.errors.is_empty());
        // dest_dir = zip 父目录，on_op_finished 的栏刷新匹配依赖这一点。
        assert_eq!(f.dest_dir, dest_zip.parent().map(Path::to_path_buf));
        assert!(!mgr.poll().has_active);
        let entries = openitgo_parser::archive::list_entries(&dest_zip, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src/", "src/a.txt", "src/sub/", "src/sub/b.txt"]);
    }

    /// Windows 长路径：manifest 未声明 longPathAware，>260 字符的路径必须
    /// 经 verbatim_path 加 `\\?\` 前缀才能读写。本测试用 verbatim 前缀
    /// 搭好深层目录树，再用普通（未加前缀）路径驱动引擎验证读/写两侧。
    #[cfg(windows)]
    #[test]
    fn copy_handles_paths_longer_than_260_chars() {
        fn vp(p: &Path) -> PathBuf {
            PathBuf::from(format!(r"\\?\{}", p.display()))
        }
        let t = TempTree::new("longpath");
        // 24 × 12 字符/层 + temp 根 ≈ 330 字符，远超 MAX_PATH。
        let mut deep = t.path().to_path_buf();
        for _ in 0..24 {
            deep = deep.join("deep-dir-x");
        }
        assert!(deep.display().to_string().chars().count() > 260);
        std::fs::create_dir_all(vp(&deep)).unwrap();
        std::fs::write(vp(&deep.join("f.txt")), b"long").unwrap();

        // 读侧：深源 → 浅目标（count_source 递归 + File::open 都过长路径）。
        let shallow_dest = t.path().join("out");
        std::fs::create_dir_all(&shallow_dest).unwrap();
        let r = run_copy_sync(
            vec![deep.clone()],
            shallow_dest.clone(),
            ConflictMode::AutoRename,
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let copied = shallow_dest.join("deep-dir-x/f.txt");
        assert_eq!(std::fs::read(&copied).unwrap(), b"long");

        // 写侧：浅源文件 → 深目标（File::create 过长路径）。
        let shallow_src = t.path().join("s.txt");
        write_file(&shallow_src, b"shallow");
        let deep_dest = deep.join("dest");
        std::fs::create_dir_all(vp(&deep_dest)).unwrap();
        let r = run_copy_sync(
            vec![shallow_src],
            deep_dest.clone(),
            ConflictMode::AutoRename,
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        assert_eq!(
            std::fs::read(vp(&deep_dest.join("s.txt"))).unwrap(),
            b"shallow"
        );
    }

    #[test]
    fn copy_reports_cur_file_progress() {
        let t = TempTree::new("curprog");
        let src = t.path().join("f.bin");
        write_file(&src, &vec![5u8; COPY_CHUNK + 100]);
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let (tx, rx) = channel();
        run_op(
            OpKind::Copy,
            vec![src.clone()],
            Some(dest.clone()),
            ConflictMode::Overwrite,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            tx,
            None,
            false,
            CopyOptions::default(),
        );
        let progresses: Vec<OpProgress> = rx
            .try_iter()
            .filter_map(|ev| match ev {
                OpEvent::Progress(p) => Some(p),
                _ => None,
            })
            .collect();
        let size = (COPY_CHUNK + 100) as u64;
        let last = progresses.last().expect("progress events");
        // 文件结束时文件内进度满格，总进度同步。
        assert_eq!(last.cur_total_bytes, size);
        assert_eq!(last.cur_done_bytes, size);
        assert_eq!(last.done_bytes, size);
    }

    #[test]
    fn paused_worker_waits_then_cancel_exits_immediately() {
        let t = TempTree::new("pause");
        let src = t.path().join("big.bin");
        write_file(&src, &vec![3u8; COPY_CHUNK * 4]);
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel();
        let (c2, p2, src2, dest2) = (cancel.clone(), paused.clone(), src.clone(), dest.clone());
        let handle = std::thread::spawn(move || {
            run_op(
                OpKind::Copy,
                vec![src2],
                Some(dest2),
                ConflictMode::Overwrite,
                c2,
                p2,
                tx,
                None,
                false,
                CopyOptions::default(),
            );
        });
        // 暂停期间：不写目标、不发 Finished（暂停中不发新进度）。
        std::thread::sleep(Duration::from_millis(500));
        assert!(!dest.join("big.bin").exists());
        assert!(rx.try_iter().all(|ev| matches!(ev, OpEvent::Progress(_))));
        // 暂停中取消即时生效：半成品不残留，cancelled 上报。
        cancel.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let cancelled = rx.try_iter().any(|ev| {
            matches!(
                ev,
                OpEvent::Finished {
                    cancelled: true,
                    ..
                }
            )
        });
        assert!(cancelled);
        assert!(!dest.join("big.bin").exists());
        assert!(src.exists());
    }

    #[test]
    fn speed_meter_ema_and_eta() {
        let t0 = Instant::now();
        let mut m = OpSpeedMeter::new();
        // 首样本只记基准，不产生速度。
        assert_eq!(m.sample(t0, 0), None);
        assert_eq!(m.speed_bps(), None);
        // 过密样本忽略（返回既有平滑值 None）。
        assert_eq!(m.sample(t0 + Duration::from_millis(100), 500), None);
        // 满间隔：1MB / 1s = 1MB/s。
        let s = m.sample(t0 + Duration::from_secs(1), 1_000_000).unwrap();
        assert!((s - 1_000_000.0).abs() < 1.0);
        // EMA：第二样本 3MB/s → 0.3*3 + 0.7*1 = 1.6MB/s。
        let s = m.sample(t0 + Duration::from_secs(2), 4_000_000).unwrap();
        assert!((s - 1_600_000.0).abs() < 1.0);
        // ETA：剩余 3.2MB / 1.6MB/s = 2s。
        let eta = m.eta_secs(7_200_000, 4_000_000).unwrap();
        assert!((eta - 2.0).abs() < 0.01);
        // 已完成 → None。
        assert_eq!(m.eta_secs(4_000_000, 4_000_000), None);
    }

    #[test]
    fn format_eta_ranges() {
        assert_eq!(format_eta(45.0), "~45s");
        assert_eq!(format_eta(1.9), "~2s");
        assert_eq!(format_eta(185.0), "~3m05s");
        assert_eq!(format_eta(7620.0), "~2h07m");
        assert_eq!(format_eta(-1.0), "~0s");
    }

    // ---- 阶段 AA：任务队列 + 重试重建 ----

    /// 轮询直到 pred 满足（超时 panic）；返回最后一次 poll 的 summary 留给
    /// 调用方继续断言的场景不适用——这里只驱动事件排空。
    fn poll_until(mgr: &mut FileOpManager, mut pred: impl FnMut(&mut FileOpManager) -> bool) {
        for _ in 0..500 {
            mgr.poll();
            if pred(mgr) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("条件未在限时内满足");
    }

    /// 队列化：超过并发上限的任务排队（Queued、不起线程）；在途结束按
    /// FIFO 放行；排队取消 = 直接移除；set_max_concurrent 调高立即放行。
    /// 确定性阻塞用 Ask 冲突：worker 发问后阻塞等答，任务钉在在途。
    #[test]
    fn queue_fifo_release_and_cancel() {
        let t = TempTree::new("queue");
        let src = t.path().join("a.txt");
        write_file(&src, b"data");
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        // 冲突：dest 已有同名文件 → Ask 模式 worker 发问并阻塞。
        write_file(&dest.join("a.txt"), b"old");
        let src2 = t.path().join("b.txt");
        write_file(&src2, b"data2");

        let mut mgr = FileOpManager::default();
        mgr.set_max_concurrent(1);
        let id1 = mgr.start_copy(vec![src.clone()], dest.clone(), ConflictMode::Ask);
        // 等 id1 发出冲突询问（= 已起线程且阻塞等答）。
        poll_until(&mut mgr, |m| m.take_pending_conflict().is_some());
        assert!(mgr.is_active(id1));

        // 超限 → id2 排队：summaries 两行（在途 + 排队），排队行无进度。
        let id2 = mgr.start_copy(vec![src2.clone()], dest.clone(), ConflictMode::AutoRename);
        assert_eq!(mgr.active_count(), 1);
        assert_eq!(mgr.queued_count(), 1);
        let sums = mgr.task_summaries();
        assert_eq!(sums.len(), 2);
        assert_eq!(sums[0].id, id1);
        assert!(!sums[0].queued);
        assert_eq!(sums[0].first_source.as_deref(), Some(src.as_path()));
        assert_eq!(sums[0].source_count, 1);
        assert_eq!(sums[0].dest_dir.as_deref(), Some(dest.as_path()));
        assert_eq!(sums[1].id, id2);
        assert!(sums[1].queued);
        assert!(!sums[1].paused);

        // 排队取消 = 静默移除（无 Finished 事件）。
        mgr.cancel(id2);
        assert_eq!(mgr.queued_count(), 0);

        // 再放一个排队任务；取消 id1（冲突等答被打断按 Cancel 收拢）→
        // poll 收 Finished(cancelled) 后 FIFO 放行 id3。
        let id3 = mgr.start_delete(vec![t.path().join("nothing.bin")], true);
        assert_eq!(mgr.queued_count(), 1);
        mgr.cancel(id1);
        poll_until(&mut mgr, |m| m.is_active(id3));
        assert_eq!(mgr.queued_count(), 0);
        assert!(mgr.is_active(id3));
        // id3 = 删除不存在项 → 单错收尾；排空。
        poll_until(&mut mgr, |m| !m.is_active(id3));

        // set_max_concurrent 调高立即放行：上限 1 时起 Ask 任务占坑，
        // 再提交一个排队，调到 2 后排队任务立刻起线程。
        mgr.set_max_concurrent(1);
        let id4 = mgr.start_copy(vec![src.clone()], dest.clone(), ConflictMode::Ask);
        poll_until(&mut mgr, |m| m.take_pending_conflict().is_some());
        let id5 = mgr.start_copy(vec![src2.clone()], dest.clone(), ConflictMode::AutoRename);
        assert_eq!(mgr.queued_count(), 1);
        mgr.set_max_concurrent(2);
        assert_eq!(mgr.queued_count(), 0);
        assert!(mgr.is_active(id5));
        mgr.cancel(id4);
        poll_until(&mut mgr, |m| !m.is_active(id4) && !m.is_active(id5));
    }

    /// 重试重建（错误汇总窗「重试失败项」）：只保留仍存在的源、去重保序。
    #[test]
    fn retry_sources_filters_missing_and_dedups() {
        let t = TempTree::new("retry");
        let keep = t.path().join("keep.txt");
        write_file(&keep, b"x");
        let gone = t.path().join("gone.txt");
        let errors = vec![
            (keep.clone(), "占用".to_string()),
            (gone, "不存在".to_string()),
            (keep.clone(), "重复项".to_string()),
        ];
        assert_eq!(retry_sources(&errors), vec![keep]);
        assert!(retry_sources(&[]).is_empty());
    }

    // ---- 阶段 AB：复制后校验 / 高级过滤 ----

    /// 分块字节比对：一致 / 内容不一致 / 长度不一致。
    #[test]
    fn files_identical_compares_content_and_length() {
        let t = TempTree::new("verify");
        let a = t.path().join("a.bin");
        let b = t.path().join("b.bin");
        let c = t.path().join("c.bin");
        write_file(&a, b"same-content-123");
        write_file(&b, b"same-content-123");
        write_file(&c, b"same-content-124"); // 尾字节不同
        let cancel = AtomicBool::new(false);
        assert!(files_identical(&a, &b, &cancel, &mut |_| {}).is_ok());
        assert!(matches!(
            files_identical(&a, &c, &cancel, &mut |_| {}),
            Err(VerifyFail::Io(_))
        ));
        // 长度不一致（快查路径）。
        write_file(&c, b"shorter");
        assert!(matches!(
            files_identical(&a, &c, &cancel, &mut |_| {}),
            Err(VerifyFail::Io(_))
        ));
        // cancel 置位 → Cancelled。
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            files_identical(&a, &b, &cancel, &mut |_| {}),
            Err(VerifyFail::Cancelled)
        ));
    }

    /// 过滤判定：目录恒过；通配符；最近 N 天；mtime 缺失放行。
    #[test]
    fn copy_filter_matches_rules() {
        let now = SystemTime::now();
        let old = now - Duration::from_secs(10 * 86400);
        // 目录恒 true（结构保留）。
        assert!(copy_filter_matches("anything", true, Some(old), "*.txt", 3));
        // 通配符（空 = 不过滤；复用分号多模式）。
        assert!(copy_filter_matches("a.txt", false, None, "", 0));
        assert!(copy_filter_matches("a.txt", false, None, "*.txt", 0));
        assert!(!copy_filter_matches("a.png", false, None, "*.txt", 0));
        assert!(copy_filter_matches("a.png", false, None, "*.txt;*.png", 0));
        // 最近 N 天（0 = 不过滤；旧文件被滤掉；mtime 缺失放行）。
        assert!(copy_filter_matches("a.txt", false, Some(now), "", 3));
        assert!(!copy_filter_matches("a.txt", false, Some(old), "", 3));
        assert!(copy_filter_matches("a.txt", false, Some(old), "", 0));
        assert!(copy_filter_matches("a.txt", false, None, "", 3));
        // 双条件 AND。
        assert!(copy_filter_matches("a.txt", false, Some(now), "*.txt", 3));
        assert!(!copy_filter_matches("a.txt", false, Some(old), "*.txt", 3));
        assert!(!copy_filter_matches("a.png", false, Some(now), "*.txt", 3));
    }

    /// 过滤集成：目录结构保留、只拷匹配文件、进度 total 即过滤后集合；
    /// Move + 过滤不整删源。
    #[test]
    fn copy_with_filter_keeps_dirs_and_skips_files() {
        let t = TempTree::new("filter");
        let src = t.path().join("src");
        write_file(&src.join("keep.txt"), b"aaa");
        write_file(&src.join("skip.png"), b"png");
        write_file(&src.join("sub/inner.txt"), b"bbb");
        write_file(&src.join("sub/inner.log"), b"log");
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let opts = CopyOptions {
            filter_pattern: "*.txt".to_string(),
            ..Default::default()
        };
        let r = run_op_sync_opts(
            OpKind::Copy,
            vec![src.clone()],
            Some(dest.clone()),
            ConflictMode::AutoRename,
            Arc::new(AtomicBool::new(false)),
            opts,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(dest.join("src/keep.txt").exists());
        assert!(dest.join("src/sub/inner.txt").exists());
        assert!(!dest.join("src/skip.png").exists(), "不匹配文件应跳过");
        assert!(!dest.join("src/sub/inner.log").exists());
        assert!(dest.join("src/sub").is_dir(), "目录结构应保留");

        // Move + 过滤：匹配项搬走，源不整删（errors 带说明）。
        let dest2 = t.path().join("dest2");
        std::fs::create_dir_all(&dest2).unwrap();
        let opts = CopyOptions {
            filter_pattern: "*.txt".to_string(),
            ..Default::default()
        };
        let r = run_op_sync_opts(
            OpKind::Move,
            vec![src.clone()],
            Some(dest2.clone()),
            ConflictMode::AutoRename,
            Arc::new(AtomicBool::new(false)),
            opts,
        );
        assert_eq!(r.errors.len(), 1, "源未删除应有说明: {:?}", r.errors);
        assert!(dest2.join("src/keep.txt").exists());
        assert!(src.join("skip.png").exists(), "被过滤项应留在源");
        assert!(src.join("sub/inner.log").exists());
        assert!(!src.join("keep.txt").exists(), "匹配项已搬走");

        // 顶层文件源被过滤：什么都不拷（且无错误）。
        let lone = t.path().join("lone.png");
        write_file(&lone, b"x");
        let r = run_op_sync_opts(
            OpKind::Copy,
            vec![lone],
            Some(dest.clone()),
            ConflictMode::AutoRename,
            Arc::new(AtomicBool::new(false)),
            CopyOptions {
                filter_pattern: "*.txt".to_string(),
                ..Default::default()
            },
        );
        assert!(r.errors.is_empty());
        assert!(!dest.join("lone.png").exists());
    }

    /// 校验集成：verify = true 的复制正常通过（内容一致无 errors）。
    #[test]
    fn copy_with_verify_passes() {
        let t = TempTree::new("verify-copy");
        let src = t.path().join("src");
        write_file(&src.join("a.txt"), b"verify me");
        write_file(&src.join("sub/b.txt"), b"nested verify");
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let r = run_op_sync_opts(
            OpKind::Copy,
            vec![src],
            Some(dest.clone()),
            ConflictMode::AutoRename,
            Arc::new(AtomicBool::new(false)),
            CopyOptions {
                verify: true,
                ..Default::default()
            },
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(std::fs::read(dest.join("src/a.txt")).unwrap(), b"verify me");
        assert_eq!(
            std::fs::read(dest.join("src/sub/b.txt")).unwrap(),
            b"nested verify"
        );
    }

    // ---------- 阶段 AD：分割/合并 ----------

    /// 同步驱动分割/合并工作线程（直接调用，断言 Finished）。
    fn run_split_sync(
        source: PathBuf,
        dest_dir: PathBuf,
        chunk_size: u64,
        cancel: Arc<AtomicBool>,
    ) -> FinishedLike {
        let (tx, rx) = channel();
        run_split(
            vec![source],
            dest_dir,
            chunk_size,
            cancel,
            Arc::new(AtomicBool::new(false)),
            tx,
        );
        let mut finished = None;
        while let Ok(ev) = rx.try_recv() {
            if let OpEvent::Finished {
                cancelled, errors, ..
            } = ev
            {
                finished = Some(FinishedLike { cancelled, errors });
            }
        }
        finished.expect("run_split 必有 Finished")
    }

    fn run_merge_sync(
        chunks: Vec<PathBuf>,
        dest: PathBuf,
        cancel: Arc<AtomicBool>,
    ) -> FinishedLike {
        let (tx, rx) = channel();
        run_merge(chunks, dest, cancel, Arc::new(AtomicBool::new(false)), tx);
        let mut finished = None;
        while let Ok(ev) = rx.try_recv() {
            if let OpEvent::Finished {
                cancelled, errors, ..
            } = ev
            {
                finished = Some(FinishedLike { cancelled, errors });
            }
        }
        finished.expect("run_merge 必有 Finished")
    }

    #[test]
    fn split_plan_sizes_and_names() {
        // 整除 + 余数。
        let plan = split_plan(4, 10);
        assert_eq!(
            plan,
            vec![
                ("001".to_string(), 4),
                ("002".to_string(), 4),
                ("003".to_string(), 2)
            ]
        );
        // 单块（chunk >= total）。
        assert_eq!(split_plan(100, 5), vec![("001".to_string(), 5)]);
        // 空文件：单个空分块。
        assert_eq!(split_plan(4, 0), vec![("001".to_string(), 0)]);
        // chunk_size 0 防御按 1。
        assert_eq!(split_plan(0, 3).len(), 3);
        // 字节总数守恒。
        let plan = split_plan(1024, 1_000_000);
        assert_eq!(plan.iter().map(|(_, b)| b).sum::<u64>(), 1_000_000);
    }

    #[test]
    fn parse_chunk_name_rules() {
        assert_eq!(parse_chunk_name("a.001"), Some(("a", 1)));
        assert_eq!(parse_chunk_name("a.b.mkv.010"), Some(("a.b.mkv", 10)));
        assert_eq!(parse_chunk_name("a.1"), None);
        assert_eq!(parse_chunk_name("a.001x"), None);
        assert_eq!(parse_chunk_name(".001"), None);
        assert_eq!(parse_chunk_name("noext"), None);
    }

    #[test]
    fn merge_target_base_rules() {
        let dir = std::env::temp_dir();
        let p = |n: &str| dir.join(n);
        // 单个 .001。
        let (d, b) = merge_target_base(&[p("a.001")]).expect("single .001");
        assert_eq!((d, b), (dir.clone(), "a".to_string()));
        // 一组同前缀分块。
        let (d, b) = merge_target_base(&[p("a.002"), p("a.001"), p("a.003")]).expect("group");
        assert_eq!((d, b), (dir.clone(), "a".to_string()));
        // 单个非 .001 分块不行。
        assert!(merge_target_base(&[p("a.002")]).is_none());
        // 混合前缀不行。
        assert!(merge_target_base(&[p("a.001"), p("b.002")]).is_none());
        // 非分块名不行。
        assert!(merge_target_base(&[p("a.zip")]).is_none());
        // 一组缺 .001 不行。
        assert!(merge_target_base(&[p("a.002"), p("a.003")]).is_none());
    }

    #[test]
    fn collect_chunks_continuous_and_missing() {
        let t = tempfile::tempdir().unwrap();
        for n in ["a.001", "a.002", "a.003"] {
            std::fs::write(t.path().join(n), b"x").unwrap();
        }
        std::fs::write(t.path().join("b.001"), b"x").unwrap();
        let chunks = collect_chunks(t.path(), "a").expect("continuous");
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].ends_with("a.001"));
        assert!(chunks[2].ends_with("a.003"));
        // 缺号报错并列出。
        std::fs::remove_file(t.path().join("a.002")).unwrap();
        let err = collect_chunks(t.path(), "a").unwrap_err();
        assert!(err.contains(".002"), "{err}");
        // 无前缀分块报错。
        assert!(collect_chunks(t.path(), "zzz").is_err());
    }

    #[test]
    fn split_merge_roundtrip_with_crc() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("data.bin");
        let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let out_dir = t.path().join("out");
        std::fs::create_dir(&out_dir).unwrap();

        // 分割（chunk 256KB+ 边界外小一点，跨多块）。
        let r = run_split_sync(
            src.clone(),
            out_dir.clone(),
            200_000,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let chunks = collect_chunks(&out_dir, "data.bin").expect("chunks");
        assert_eq!(chunks.len(), 4); // ceil(700000/200000)
        let sizes: Vec<u64> = chunks
            .iter()
            .map(|c| std::fs::metadata(c).unwrap().len())
            .collect();
        assert_eq!(sizes, vec![200_000, 200_000, 200_000, 100_000]);

        // 冲突预检：再分割同目标 → 列出全部冲突、不改动。
        let before = std::fs::read(out_dir.join("data.bin.001")).unwrap();
        let r = run_split_sync(
            src.clone(),
            out_dir.clone(),
            200_000,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(r.errors.len(), 4, "{:?}", r.errors);
        assert_eq!(std::fs::read(out_dir.join("data.bin.001")).unwrap(), before);

        // 写 .crc（sfv 单行：文件名 + 空格 + crc32 hex）。
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&data);
        let crc = format!("{:08x}", hasher.finalize());
        std::fs::write(out_dir.join("data.bin.crc"), format!("data.bin {crc}\n")).unwrap();

        // 合并 + CRC 校验通过。
        let dest = out_dir.join("data.bin");
        let r = run_merge_sync(
            chunks.clone(),
            dest.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(std::fs::read(&dest).unwrap(), data);

        // CRC 不一致 → errors 报告（结果保留）。
        std::fs::write(out_dir.join("data.bin.crc"), "data.bin 00000000\n").unwrap();
        std::fs::remove_file(&dest).unwrap();
        let r = run_merge_sync(chunks, dest.clone(), Arc::new(AtomicBool::new(false)));
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].1.contains("校验不一致"), "{:?}", r.errors);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[test]
    fn split_cancel_removes_partial_chunk() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("big.bin");
        std::fs::write(&src, vec![7u8; 600_000]).unwrap();
        let cancel = Arc::new(AtomicBool::new(true)); // 预置取消：第一块即停
        let r = run_split_sync(src, t.path().to_path_buf(), 100_000, cancel);
        assert!(r.cancelled);
        // 半成品分块已删除（无任何 .NNN 产出残留）。
        assert!(collect_chunks(t.path(), "big.bin").is_err());
    }
}
