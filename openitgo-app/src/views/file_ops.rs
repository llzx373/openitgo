//! 文件操作引擎：复制/移动/删除/压缩的后台执行 + 进度/取消/暂停/冲突处理；
//! 重命名/新建文件夹为瞬时操作，提供同步 helper。对齐 extract 约定：
//! 每任务一条后台线程、channel 上报进度、`Arc<AtomicBool>` 取消与暂停、
//! 取消清理半成品目标文件（已完整复制/移动的保留并计入进度）、
//! 单项失败记 `errors` 继续整批、结束汇总上报。
//! 进度含当前文件内进度（cur_done/cur_total_bytes，分块复制维护，
//! 节流 100ms）；暂停在块/项边界生效（200ms 轮询，期间可即时取消），
//! Compress 不支持暂停。
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
//! 删除：逐项 `trash::delete`（回收站），单项失败记 errors 继续。
//! 符号链接：复制 = 复制链接目标内容（`fs::copy` 语义），删除 = 只删链接；
//! 预扫描不跟进符号链接目录（防环），递归复制同样不跟进（符号链接目录
//! 按文件处理，`File::open` 失败则记 errors 继续）。
//! Windows 长路径：manifest 未声明 longPathAware，文件系统调用统一经
//! `verbatim_path` 加 `\\?\` 前缀（见该函数注释）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "复制",
            OpKind::Move => "移动",
            OpKind::Delete => "删除",
            OpKind::Compress => "压缩",
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
    },
}

pub struct FileOpTask {
    pub id: u64,
    pub kind: OpKind,
    pub progress: OpProgress,
    /// 涉及的源目录（各 source 的 parent），完成后刷新用。
    src_dirs: Vec<PathBuf>,
    /// 目标目录（仅复制/移动），完成后刷新用。
    dest_dir: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    /// 暂停标志（与 cancel 同模式暴露给 UI；Copy/Move/Delete 在块/项边界
    /// 生效，Compress 不支持暂停）。
    paused: Arc<AtomicBool>,
    rx: Receiver<OpEvent>,
    /// Ask 模式的回答回发端（worker 持 rx 阻塞等答；task 被移除时 drop，
    /// worker recv 出错按 Cancel 收拢，防悬挂）。
    answer_tx: Sender<ConflictAnswer>,
    /// 已到达、待 UI 取走的冲突询问（worker 逐一发问，恒最多一条）。
    pending_query: Option<ConflictQuery>,
}

/// 本帧完成的任务快照（poll 返回值携带，已从 manager 移除）。
pub struct FinishedOp {
    pub kind: OpKind,
    pub cancelled: bool,
    pub fatal: Option<String>,
    pub errors: Vec<(PathBuf, String)>,
    pub src_dirs: Vec<PathBuf>,
    pub dest_dir: Option<PathBuf>,
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

#[derive(Default)]
pub struct FileOpManager {
    tasks: Vec<FileOpTask>,
    next_id: u64,
}

impl FileOpManager {
    /// 后台复制 sources 到 dest_dir（逐项应用 conflict 策略）。
    pub fn start_copy(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
    ) -> u64 {
        self.start_transfer(OpKind::Copy, sources, dest_dir, conflict)
    }

    /// 后台移动：同盘 `fs::rename` 快速路径，失败回退复制 + trash 源。
    pub fn start_move(
        &mut self,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
    ) -> u64 {
        self.start_transfer(OpKind::Move, sources, dest_dir, conflict)
    }

    /// 后台删除（逐项移入回收站）。
    pub fn start_delete(&mut self, sources: Vec<PathBuf>) -> u64 {
        self.spawn_task(OpKind::Delete, sources, None, ConflictMode::Skip)
    }

    /// 后台压缩 sources 为 dest_zip（zip 引擎在 parser 侧，逐项进度桥接进
    /// OpProgress；dest_dir 记 dest_zip 的父目录使完成后栏刷新自动生效）。
    pub fn start_compress(&mut self, sources: Vec<PathBuf>, dest_zip: PathBuf) -> u64 {
        let dest_dir = dest_zip.parent().map(Path::to_path_buf);
        let (id, cancel, _paused, tx, _answer_rx) =
            self.push_task(OpKind::Compress, &sources, dest_dir);
        std::thread::spawn(move || {
            run_compress(sources, dest_zip, cancel, tx);
        });
        id
    }

    fn start_transfer(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        conflict: ConflictMode,
    ) -> u64 {
        self.spawn_task(kind, sources, Some(dest_dir), conflict)
    }

    fn spawn_task(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest_dir: Option<PathBuf>,
        conflict: ConflictMode,
    ) -> u64 {
        let (id, cancel, paused, tx, answer_rx) = self.push_task(kind, &sources, dest_dir.clone());
        // Ask 模式才需要问答通道（worker 阻塞等答）；其余模式不需要。
        let answer_rx = (conflict == ConflictMode::Ask).then_some(answer_rx);
        std::thread::spawn(move || {
            run_op(
                kind, sources, dest_dir, conflict, cancel, paused, tx, answer_rx,
            );
        });
        id
    }

    /// 登记任务并返回 (id, cancel, paused, 事件发送端, 冲突回答接收端)，
    /// 由调用方自起工作线程。
    fn push_task(
        &mut self,
        kind: OpKind,
        sources: &[PathBuf],
        dest_dir: Option<PathBuf>,
    ) -> (
        u64,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Sender<OpEvent>,
        Receiver<ConflictAnswer>,
    ) {
        self.next_id += 1;
        let id = self.next_id;
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
        let task = FileOpTask {
            id,
            kind,
            progress: OpProgress::default(),
            src_dirs,
            dest_dir,
            cancel: cancel.clone(),
            paused: paused.clone(),
            rx,
            answer_tx,
            pending_query: None,
        };
        self.tasks.push(task);
        (id, cancel, paused, tx, answer_rx)
    }

    /// 取消任务：工作线程在下一块/下一项停止并上报 Finished(cancelled)。
    pub fn cancel(&mut self, id: u64) {
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

    /// 暂停/继续任务（Copy/Move/Delete 在块/项边界生效；Compress 不响应）。
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
                    }) => {
                        finished_ids.push((task.id, cancelled, fatal, errors));
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
                        ));
                        break;
                    }
                }
            }
        }
        for (id, cancelled, fatal, errors) in finished_ids {
            if let Some(pos) = self.tasks.iter().position(|t| t.id == id) {
                let task = self.tasks.remove(pos);
                summary.finished.push(FinishedOp {
                    kind: task.kind,
                    cancelled,
                    fatal,
                    errors,
                    src_dirs: task.src_dirs,
                    dest_dir: task.dest_dir,
                });
            }
        }
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

/// 工作线程入口：预扫描计数 → 逐项执行 → Finished 事件收尾。
/// answer_rx 仅 Ask 模式 Some（冲突问答的回答接收端）。
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
) {
    let mut ctx = OpCtx {
        cancel: &cancel,
        paused: &paused,
        tx: &tx,
        answer_rx: answer_rx.as_ref(),
        remembered_file: None,
        remembered_dir: None,
        progress: OpProgress::default(),
        errors: Vec::new(),
        cancelled: false,
    };
    // 预扫描：递归计数（文件+目录项数、文件字节）；符号链接不跟进目录。
    let mut per_source = Vec::with_capacity(sources.len());
    for src in &sources {
        let (items, bytes) = count_source(src);
        ctx.progress.total_files += items;
        ctx.progress.total_bytes += bytes;
        per_source.push((items, bytes));
    }
    let mut fatal = None;
    match kind {
        OpKind::Compress => unreachable!("Compress 走 run_compress"),
        OpKind::Delete => {
            for (i, src) in sources.iter().enumerate() {
                if ctx.wait_if_paused() {
                    break;
                }
                ctx.progress.current = src.clone();
                if let Err(e) = trash::delete(src) {
                    ctx.errors.push((src.clone(), e.to_string()));
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
                if kind == OpKind::Move {
                    // 快速路径：同盘 rename 瞬间完成（目标存在时 rename 在
                    // Windows 上会失败，落入回退路径由冲突策略处理）。
                    if !verbatim_path(&dst).exists()
                        && std::fs::rename(verbatim_path(src), verbatim_path(&dst)).is_ok()
                    {
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
                    if copy_recursive(src, &dst, conflict, &mut ctx).is_ok()
                        && kind == OpKind::Move
                        && !ctx.cancelled
                    {
                        // 跨盘/占用回退：复制成功后源移入回收站。
                        if let Err(e) = trash::delete(src) {
                            ctx.errors
                                .push((src.clone(), format!("源移入回收站失败: {e}")));
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
    });
}

/// 压缩工作线程：调 parser 的 create_zip，ZipWriteProgress 经转发线程
/// 桥接成 OpProgress 快照流；取消/致命错误按 create_zip 约定收尾
/// （Err → fatal；cancel 置位 → cancelled；容错跳过项不逐项上报）。
fn run_compress(
    sources: Vec<PathBuf>,
    dest_zip: PathBuf,
    cancel: Arc<AtomicBool>,
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
    );
    let _ = forwarder.join();
    let _ = tx.send(OpEvent::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
        fatal: result.err().map(|e| e.to_string()),
        errors: Vec::new(),
    });
}

struct OpCtx<'a> {
    cancel: &'a AtomicBool,
    paused: &'a AtomicBool,
    tx: &'a Sender<OpEvent>,
    /// Ask 模式的回答接收端（非 Ask 为 None）。
    answer_rx: Option<&'a Receiver<ConflictAnswer>>,
    /// Ask 模式 apply_all 记忆：后续同级冲突的生效策略（文件级/目录级
    /// 各自独立——目录级「全部应用」不预决文件级策略）。
    remembered_file: Option<ConflictAction>,
    remembered_dir: Option<ConflictAction>,
    progress: OpProgress,
    errors: Vec<(PathBuf, String)>,
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
fn count_source(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(verbatim_path(path)) else {
        return (1, 0);
    };
    if meta.is_symlink() || !meta.is_dir() {
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
                    items += 1;
                    if let Ok(m) = item.metadata() {
                        if m.is_file() {
                            bytes += m.len();
                        }
                    }
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
/// 返回 Err 仅表示已取消（errors 里已记单项失败）。
fn copy_recursive(src: &Path, dst: &Path, mode: ConflictMode, ctx: &mut OpCtx) -> Result<(), ()> {
    if ctx.wait_if_paused() {
        return Err(());
    }
    let is_dir = std::fs::symlink_metadata(verbatim_path(src))
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
        Ok(())
    } else {
        match copy_file_chunks(src, dst, ctx) {
            Ok(bytes) => {
                ctx.progress.done_files += 1;
                ctx.progress.done_bytes += bytes;
                ctx.send_progress();
            }
            Err(CopyFail::Cancelled) => return Err(()),
            Err(CopyFail::Io(e)) => {
                ctx.errors.push((src.to_path_buf(), e));
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
        let (items, bytes) = count_source(&t.path().join("sub"));
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
        let mut ctx = OpCtx {
            cancel: &cancel,
            paused: &paused,
            tx: &tx,
            answer_rx: None,
            remembered_file: None,
            remembered_dir: None,
            progress: OpProgress::default(),
            errors: Vec::new(),
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
}
