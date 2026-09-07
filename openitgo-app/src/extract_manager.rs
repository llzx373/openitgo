//! 后台解压任务管理：每个任务一条线程跑阻塞式 `archive::extract_archive`，
//! 进度经 crossbeam 通道汇总到 UI 线程；并发上限 4，超出的任务排队。

use crossbeam_channel::{Receiver, Sender};
use openitgo_parser::archive::{extract_archive, ExtractOptions, ExtractProgress};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 同时运行的解压任务上限，超出排队。
pub const MAX_CONCURRENT_EXTRACTS: usize = 4;

/// 引擎取消约定的 Failed 消息（extract.rs 模块头注释）。
const CANCELLED_MESSAGE: &str = "已取消";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractTaskStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl ExtractTaskStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    pub fn is_terminal(self) -> bool {
        !self.is_active()
    }
}

pub struct ExtractTask {
    pub id: u64,
    pub archive_path: PathBuf,
    pub output_dir: PathBuf,
    pub status: ExtractTaskStatus,
    pub total_entries: usize,
    pub total_bytes: u64,
    pub done_entries: usize,
    pub done_bytes: u64,
    /// 最近完成的条目名（进行中显示用）。
    pub current_entry: String,
    pub error: Option<String>,
    cancel: Arc<AtomicBool>,
}

impl ExtractTask {
    /// 进度比例：优先按字节，其次按条目数，都没有则 0（启动前）。
    pub fn fraction(&self) -> f32 {
        if self.total_bytes > 0 {
            (self.done_bytes as f64 / self.total_bytes as f64) as f32
        } else if self.total_entries > 0 {
            self.done_entries as f32 / self.total_entries as f32
        } else {
            0.0
        }
    }

    pub fn archive_name(&self) -> String {
        self.archive_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.archive_path.display().to_string())
    }
}

/// 排队中的解压请求（任务行已建，工作线程待启动）。
struct PendingExtract {
    id: u64,
    archive_path: PathBuf,
    output_dir: PathBuf,
    selection: Option<Vec<String>>,
    options: ExtractOptions,
}

/// `poll` 一帧的汇总：供 app 写 `error_message` / 决定重绘节奏。
#[derive(Debug, Default)]
pub struct ExtractSummary {
    /// 完成的包：(包路径, 输出目录)。
    pub finished: Vec<(PathBuf, PathBuf)>,
    /// 失败的包：(包路径, 错误消息)。
    pub failed: Vec<(PathBuf, String)>,
    /// 被取消的包路径。
    pub cancelled: Vec<PathBuf>,
    /// 仍有进行中或排队中的任务。
    pub has_active: bool,
}

pub struct ExtractManager {
    tasks: Vec<ExtractTask>,
    pending: VecDeque<PendingExtract>,
    tx: Sender<(u64, ExtractProgress)>,
    rx: Receiver<(u64, ExtractProgress)>,
    next_id: u64,
    /// 进度面板是否可见（关闭仅隐藏，不取消任务；新任务启动时重新展开）。
    pub panel_open: bool,
}

impl Default for ExtractManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtractManager {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            tasks: Vec::new(),
            pending: VecDeque::new(),
            tx,
            rx,
            next_id: 0,
            panel_open: false,
        }
    }

    pub fn tasks(&self) -> &[ExtractTask] {
        &self.tasks
    }

    pub fn has_active(&self) -> bool {
        self.tasks.iter().any(|t| t.status.is_active())
    }

    /// 启动一个解压任务，返回任务 id；超过并发上限时排队。
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        archive_path: PathBuf,
        output_dir: PathBuf,
        selection: Option<Vec<String>>,
        password: Option<String>,
        threads: usize,
        overwrite: bool,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let options = ExtractOptions {
            password,
            threads,
            overwrite,
        };
        let running = self
            .tasks
            .iter()
            .filter(|t| t.status == ExtractTaskStatus::Running)
            .count();
        let status = if running < MAX_CONCURRENT_EXTRACTS {
            ExtractTaskStatus::Running
        } else {
            ExtractTaskStatus::Queued
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.tasks.push(ExtractTask {
            id,
            archive_path: archive_path.clone(),
            output_dir: output_dir.clone(),
            status,
            total_entries: 0,
            total_bytes: 0,
            done_entries: 0,
            done_bytes: 0,
            current_entry: String::new(),
            error: None,
            cancel: cancel.clone(),
        });
        if status == ExtractTaskStatus::Running {
            spawn_worker(
                &self.tx,
                id,
                archive_path,
                output_dir,
                selection,
                options,
                cancel,
            );
        } else {
            self.pending.push_back(PendingExtract {
                id,
                archive_path,
                output_dir,
                selection,
                options,
            });
        }
        self.panel_open = true;
        id
    }

    /// 取消任务：运行中置 cancel flag（引擎经 Failed("已取消") 收尾），
    /// 排队中直接从队列移除并标记已取消。
    pub fn cancel(&mut self, id: u64) {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        match task.status {
            ExtractTaskStatus::Queued => {
                task.status = ExtractTaskStatus::Cancelled;
                self.pending.retain(|p| p.id != id);
            }
            ExtractTaskStatus::Running => {
                task.cancel.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// 排空进度事件、更新任务状态、提拔排队任务，返回本帧汇总。
    pub fn poll(&mut self) -> ExtractSummary {
        let mut summary = ExtractSummary::default();
        while let Ok((id, event)) = self.rx.try_recv() {
            let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
                continue;
            };
            // 忽略已终态任务的迟到事件（如 worker 兜底补发的重复 Failed）。
            if task.status.is_terminal() {
                continue;
            }
            match event {
                ExtractProgress::Started {
                    total_entries,
                    total_bytes,
                } => {
                    task.total_entries = total_entries;
                    task.total_bytes = total_bytes;
                }
                ExtractProgress::EntryDone { name, bytes } => {
                    task.done_entries += 1;
                    task.done_bytes += bytes;
                    task.current_entry = name;
                }
                ExtractProgress::Finished { output_dir, .. } => {
                    task.status = ExtractTaskStatus::Done;
                    summary
                        .finished
                        .push((task.archive_path.clone(), output_dir));
                }
                ExtractProgress::Failed(message) => {
                    if message == CANCELLED_MESSAGE {
                        task.status = ExtractTaskStatus::Cancelled;
                        summary.cancelled.push(task.archive_path.clone());
                    } else {
                        task.status = ExtractTaskStatus::Failed;
                        task.error = Some(message.clone());
                        summary.failed.push((task.archive_path.clone(), message));
                    }
                }
            }
        }
        self.promote_pending();
        summary.has_active = self.has_active();
        if !summary.has_active {
            // 全部结束：终态已计入汇总，清掉避免任务列表无限累积。
            self.tasks.retain(|t| !t.status.is_terminal());
        }
        summary
    }

    /// 有空位时按 FIFO 启动排队任务。
    fn promote_pending(&mut self) {
        loop {
            let running = self
                .tasks
                .iter()
                .filter(|t| t.status == ExtractTaskStatus::Running)
                .count();
            if running >= MAX_CONCURRENT_EXTRACTS {
                break;
            }
            let Some(p) = self.pending.pop_front() else {
                break;
            };
            let Some(task) = self.tasks.iter_mut().find(|t| t.id == p.id) else {
                continue;
            };
            task.status = ExtractTaskStatus::Running;
            let cancel = task.cancel.clone();
            spawn_worker(
                &self.tx,
                p.id,
                p.archive_path,
                p.output_dir,
                p.selection,
                p.options,
                cancel,
            );
        }
    }
}

/// 起工作线程跑阻塞式解压；引擎对部分早期错误（如 ZIP 无密码）只返回
/// Err 不发 Failed 事件，这里兜底补发，保证任务必然收敛到终态。
/// 引擎只认 `Sender<ExtractProgress>`（不带任务 id），因此每任务一条私有
/// 通道 + 一条转发线程贴上 id 送入管理器的汇总通道。
fn spawn_worker(
    tx: &Sender<(u64, ExtractProgress)>,
    id: u64,
    archive_path: PathBuf,
    output_dir: PathBuf,
    selection: Option<Vec<String>>,
    options: ExtractOptions,
    cancel: Arc<AtomicBool>,
) {
    let tx = tx.clone();
    let (task_tx, task_rx) = crossbeam_channel::unbounded::<ExtractProgress>();
    // 转发线程：extract_archive 返回且 worker 内的 sender 全部释放后，
    // recv 出错即退出。
    std::thread::spawn(move || {
        while let Ok(event) = task_rx.recv() {
            if tx.send((id, event)).is_err() {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let result = extract_archive(
            &archive_path,
            &output_dir,
            selection.as_deref(),
            &options,
            task_tx.clone(),
            cancel,
        );
        if let Err(e) = result {
            let _ = task_tx.send(ExtractProgress::Failed(e.to_string()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> ExtractManager {
        ExtractManager::new()
    }

    /// 手工塞一个指定状态的任务（不经 start，避免起真实工作线程）。
    fn push_task(m: &mut ExtractManager, id: u64, status: ExtractTaskStatus) {
        m.tasks.push(ExtractTask {
            id,
            archive_path: PathBuf::from(format!("pack{id}.zip")),
            output_dir: PathBuf::from("out"),
            status,
            total_entries: 0,
            total_bytes: 0,
            done_entries: 0,
            done_bytes: 0,
            current_entry: String::new(),
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        m.next_id = m.next_id.max(id);
    }

    #[test]
    fn start_queues_beyond_concurrency_limit() {
        let mut m = manager();
        for id in 1..=MAX_CONCURRENT_EXTRACTS as u64 {
            push_task(&mut m, id, ExtractTaskStatus::Running);
        }
        // 已满员：新任务排队，不起线程。
        let id = m.start(
            PathBuf::from("new.zip"),
            PathBuf::from("out"),
            None,
            None,
            0,
            false,
        );
        let task = m.tasks.iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, ExtractTaskStatus::Queued);
        assert_eq!(m.pending.len(), 1);
        assert!(m.panel_open);
    }

    #[test]
    fn poll_aggregates_finished_and_cleans_terminal_tasks() {
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        m.tx.send((
            1,
            ExtractProgress::Started {
                total_entries: 2,
                total_bytes: 100,
            },
        ))
        .unwrap();
        m.tx.send((
            1,
            ExtractProgress::EntryDone {
                name: "a.txt".into(),
                bytes: 60,
            },
        ))
        .unwrap();
        let summary = m.poll();
        // 任务仍在进行：终态前不清理。
        assert!(summary.has_active);
        let task = &m.tasks[0];
        assert_eq!(task.done_bytes, 60);
        assert!((task.fraction() - 0.6).abs() < 1e-6);

        m.tx.send((
            1,
            ExtractProgress::Finished {
                output_dir: PathBuf::from("out"),
                written: 100,
            },
        ))
        .unwrap();
        let summary = m.poll();
        assert!(!summary.has_active);
        assert_eq!(
            summary.finished,
            vec![(PathBuf::from("pack1.zip"), PathBuf::from("out"))]
        );
        assert!(m.tasks.is_empty(), "终态任务应在无活动时清理");
    }

    #[test]
    fn poll_classifies_failed_and_cancelled() {
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        push_task(&mut m, 2, ExtractTaskStatus::Running);
        m.tx.send((1, ExtractProgress::Failed("坏包".into())))
            .unwrap();
        m.tx.send((2, ExtractProgress::Failed(CANCELLED_MESSAGE.into())))
            .unwrap();
        let summary = m.poll();
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].1, "坏包");
        assert_eq!(summary.cancelled, vec![PathBuf::from("pack2.zip")]);
        assert!(!summary.has_active);
    }

    #[test]
    fn poll_ignores_late_events_for_terminal_tasks() {
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        m.tx.send((
            1,
            ExtractProgress::Finished {
                output_dir: PathBuf::from("out"),
                written: 0,
            },
        ))
        .unwrap();
        // worker 兜底补发的重复 Failed 不得覆盖 Done 或重复汇报。
        m.tx.send((1, ExtractProgress::Failed("重复".into())))
            .unwrap();
        let summary = m.poll();
        assert_eq!(summary.finished.len(), 1);
        assert!(summary.failed.is_empty());
    }

    #[test]
    fn cancel_queued_task_removes_from_pending() {
        let mut m = manager();
        for id in 1..=MAX_CONCURRENT_EXTRACTS as u64 {
            push_task(&mut m, id, ExtractTaskStatus::Running);
        }
        let id = m.start(
            PathBuf::from("new.zip"),
            PathBuf::from("out"),
            None,
            None,
            0,
            false,
        );
        m.cancel(id);
        let task = m.tasks.iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, ExtractTaskStatus::Cancelled);
        assert!(m.pending.is_empty());
    }

    #[test]
    fn cancel_running_task_sets_flag() {
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        m.cancel(1);
        assert!(m.tasks[0].cancel.load(Ordering::Relaxed));
        // 状态由后续 Failed("已取消") 事件收敛，不在 cancel 里改。
        assert_eq!(m.tasks[0].status, ExtractTaskStatus::Running);
    }

    #[test]
    fn promote_pending_starts_queued_tasks_in_fifo_order() {
        // 输出目录放 tempdir：worker 会对不存在的包路径失败，但不会污染工作目录。
        let tmp = tempfile::tempdir().unwrap();
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        // 直接塞两个排队任务（不存在的包路径：worker 会失败，但不影响状态断言）。
        for (id, name) in [(2u64, "b.zip"), (3u64, "c.zip")] {
            m.tasks.push(ExtractTask {
                id,
                archive_path: PathBuf::from(name),
                output_dir: tmp.path().join(format!("out{id}")),
                status: ExtractTaskStatus::Queued,
                total_entries: 0,
                total_bytes: 0,
                done_entries: 0,
                done_bytes: 0,
                current_entry: String::new(),
                error: None,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            m.pending.push_back(PendingExtract {
                id,
                archive_path: PathBuf::from(name),
                output_dir: tmp.path().join(format!("out{id}")),
                selection: None,
                options: ExtractOptions {
                    password: None,
                    threads: 0,
                    overwrite: false,
                },
            });
        }
        m.promote_pending();
        // 1 个在跑 → 再提拔 3 个空位，两个排队任务都转为 Running。
        assert!(m.pending.is_empty());
        assert_eq!(m.tasks[1].status, ExtractTaskStatus::Running);
        assert_eq!(m.tasks[2].status, ExtractTaskStatus::Running);
    }

    #[test]
    fn promote_pending_respects_concurrency_limit() {
        let mut m = manager();
        for id in 1..=MAX_CONCURRENT_EXTRACTS as u64 {
            push_task(&mut m, id, ExtractTaskStatus::Running);
        }
        m.pending.push_back(PendingExtract {
            id: 99,
            archive_path: PathBuf::from("x.zip"),
            output_dir: PathBuf::from("out"),
            selection: None,
            options: ExtractOptions {
                password: None,
                threads: 0,
                overwrite: false,
            },
        });
        push_task(&mut m, 99, ExtractTaskStatus::Queued);
        m.promote_pending();
        // 满员：不提拔。
        assert_eq!(m.pending.len(), 1);
        assert_eq!(
            m.tasks.iter().find(|t| t.id == 99).unwrap().status,
            ExtractTaskStatus::Queued
        );
    }

    #[test]
    fn task_fraction_prefers_bytes() {
        let mut m = manager();
        push_task(&mut m, 1, ExtractTaskStatus::Running);
        let task = &mut m.tasks[0];
        assert_eq!(task.fraction(), 0.0);
        task.total_entries = 10;
        task.done_entries = 5;
        assert!((task.fraction() - 0.5).abs() < 1e-6);
        task.total_bytes = 200;
        task.done_bytes = 50;
        assert!((task.fraction() - 0.25).abs() < 1e-6);
    }
}
