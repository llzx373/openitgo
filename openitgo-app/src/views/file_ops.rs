//! 文件操作引擎：复制/移动/删除/压缩的后台执行 + 进度/取消/冲突处理；
//! 重命名/新建文件夹为瞬时操作，提供同步 helper。对齐 extract 约定：
//! 每任务一条后台线程、channel 上报进度、`Arc<AtomicBool>` 取消、
//! 取消清理半成品目标文件（已完整复制/移动的保留并计入进度）、
//! 单项失败记 `errors` 继续整批、结束汇总上报。
//!
//! 冲突语义（一期）：策略由 UI 层操作前一次性确定（见
//! `file_manager_dialog.rs`），执行期不再询问；执行时遇「扫描后新出现的
//! 冲突」且模式为 Ask/Skip 时按 Skip 记入 errors 汇总。AutoRename 用
//! `name (1).ext` 递增（`resolve_conflict_name`，同 parser extract 的
//! uniquify 语义）。目录冲突：两边都是目录 → 合并进入（递归内部按文件
//! 逐项应用策略），不整删目标目录。
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

/// 复制块大小（B）：手动分块复制以便块间响应取消并清理半成品。
const COPY_CHUNK: usize = 128 * 1024;

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

/// 冲突处理策略：操作前由对话框一次性确定；执行期 Ask 按 Skip 处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMode {
    Ask,
    Overwrite,
    Skip,
    AutoRename,
}

#[derive(Debug, Clone, Default)]
pub struct OpProgress {
    pub done_files: u64,
    pub total_files: u64,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current: PathBuf,
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

/// 线程 → UI 的事件。
enum OpEvent {
    Progress(OpProgress),
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
    rx: Receiver<OpEvent>,
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
        let (id, cancel, tx) = self.push_task(OpKind::Compress, &sources, dest_dir);
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
        let (id, cancel, tx) = self.push_task(kind, &sources, dest_dir.clone());
        std::thread::spawn(move || {
            run_op(kind, sources, dest_dir, conflict, cancel, tx);
        });
        id
    }

    /// 登记任务并返回 (id, cancel, 事件发送端)，由调用方自起工作线程。
    fn push_task(
        &mut self,
        kind: OpKind,
        sources: &[PathBuf],
        dest_dir: Option<PathBuf>,
    ) -> (u64, Arc<AtomicBool>, Sender<OpEvent>) {
        self.next_id += 1;
        let id = self.next_id;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
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
            rx,
        };
        self.tasks.push(task);
        (id, cancel, tx)
    }

    /// 取消任务：工作线程在下一块/下一项停止并上报 Finished(cancelled)。
    pub fn cancel(&mut self, id: u64) {
        if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
            task.cancel.store(true, Ordering::Relaxed);
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
        });
        summary
    }
}

/// 工作线程入口：预扫描计数 → 逐项执行 → Finished 事件收尾。
fn run_op(
    kind: OpKind,
    sources: Vec<PathBuf>,
    dest_dir: Option<PathBuf>,
    conflict: ConflictMode,
    cancel: Arc<AtomicBool>,
    tx: Sender<OpEvent>,
) {
    let mut ctx = OpCtx {
        cancel: &cancel,
        tx: &tx,
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
                if ctx.halt() {
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
                if ctx.halt() {
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
                // 两边都是目录：不应用文件冲突策略，合并进入（递归内部
                // 按文件逐项应用）。否则按顶层策略消解冲突。
                let dst = if verbatim_path(src).is_dir() && verbatim_path(&dst).is_dir() {
                    Some(dst)
                } else {
                    match resolve_conflict(&dst, conflict) {
                        Some(d) => Some(d),
                        None => {
                            ctx.errors
                                .push((src.clone(), "目标已存在，已跳过".to_string()));
                            ctx.progress.done_files += items;
                            ctx.progress.done_bytes += bytes;
                            ctx.send_progress();
                            None
                        }
                    }
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
    tx: &'a Sender<OpEvent>,
    progress: OpProgress,
    errors: Vec<(PathBuf, String)>,
    cancelled: bool,
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

    fn send_progress(&mut self) {
        let _ = self.tx.send(OpEvent::Progress(self.progress.clone()));
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

/// 执行期冲突消解：目标不存在 → 原样；否则按策略：
/// Overwrite → 原样（复制时覆盖/合并）；AutoRename → `name (1).ext` 递增；
/// Ask/Skip（含扫描后新出现的冲突）→ None（跳过并记汇总）。
fn resolve_conflict(dst: &Path, mode: ConflictMode) -> Option<PathBuf> {
    if !verbatim_path(dst).exists() {
        return Some(dst.to_path_buf());
    }
    match mode {
        ConflictMode::Overwrite => Some(dst.to_path_buf()),
        ConflictMode::AutoRename => Some(resolve_conflict_name(dst)),
        ConflictMode::Ask | ConflictMode::Skip => None,
    }
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
fn verbatim_path(p: &Path) -> PathBuf {
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
fn verbatim_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// 递归复制 src → dst（dst 已按顶层冲突策略消解）：目录建目录并合并进入，
/// 文件分块复制（块间响应取消，取消时删除半成品目标文件）。
/// 返回 Err 仅表示已取消（errors 里已记单项失败）。
fn copy_recursive(src: &Path, dst: &Path, mode: ConflictMode, ctx: &mut OpCtx) -> Result<(), ()> {
    if ctx.halt() {
        return Err(());
    }
    let is_dir = std::fs::symlink_metadata(verbatim_path(src))
        .map(|m| m.is_dir() && !m.is_symlink())
        .unwrap_or(false);
    if is_dir {
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
            // 目录合并：两边都是目录时不应用文件冲突策略，直接递归。
            let child_dst =
                if verbatim_path(&child_src).is_dir() && verbatim_path(&child_dst).is_dir() {
                    child_dst
                } else {
                    match resolve_conflict(&child_dst, mode) {
                        Some(d) => d,
                        None => {
                            ctx.errors
                                .push((child_src.clone(), "目标已存在，已跳过".to_string()));
                            continue;
                        }
                    }
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

/// 分块复制文件：每块后检查取消；取消时删除半成品目标文件。
fn copy_file_chunks(src: &Path, dst: &Path, ctx: &mut OpCtx) -> Result<u64, CopyFail> {
    use std::io::{Read, Write};
    if ctx.halt() {
        return Err(CopyFail::Cancelled);
    }
    let mut reader = std::fs::File::open(verbatim_path(src))
        .map_err(|e| CopyFail::Io(format!("无法读取: {e}")))?;
    let mut writer = std::fs::File::create(verbatim_path(dst))
        .map_err(|e| CopyFail::Io(format!("无法创建目标: {e}")))?;
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut written = 0u64;
    loop {
        if ctx.halt() {
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
        run_op(kind, sources, dest, mode, cancel, tx);
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
        let (tx, _rx) = channel();
        let mut ctx = OpCtx {
            cancel: &cancel,
            tx: &tx,
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
}
