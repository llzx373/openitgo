//! 同步目录对话框（阶段 AE，TC「同步目录」简化版）：双栏模式两栏目录不同
//! 时经顶栏/右键「同步目录…」打开非模态 egui::Window。打开即后台递归
//! 对比两棵树（遍历约定沿用：不跟进符号链接目录、单项失败跳过、可取消，
//! 进度 = 已扫描条目数），`diff_dir_trees` 纯函数分类 OnlyLeft/OnlyRight/
//! LeftNewer/RightNewer/Same（大小不同按 mtime 新者归类，mtime 相同大小
//! 不同归 LeftNewer；大小相同 mtime 差 ≤2s 视为 Same——FAT 精度惯例；
//! 目录只参与 OnlyX 判定）。结果列表虚拟化，「相同」默认不显示。执行
//! 经 `build_sync_plan` 纯函数产出 Copy（Overwrite）/Delete（回收站）
//! 任务交 file_ops 队列（进度/错误汇总天然生效）；嵌套 OnlyX 项只取最
//! 顶层（目录整体复制已覆盖后代）。

use crate::views::archive::{format_mtime, human_size};
use crate::views::file_ops::verbatim_path;
use egui_phosphor_icons::icons;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// 大小相同情况下 mtime 容差（FAT 2 秒精度惯例）。
const MTIME_TOLERANCE: Duration = Duration::from_secs(2);
/// 结果列表行高（pt）。
const ROW_H: f32 = 20.0;

/// 树条目元数据（相对路径 → 元数据的映射值；文件带大小/mtime，目录仅
/// is_dir 标记——目录不参与 Same/ newer 判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeMeta {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

/// 对比输入：相对路径（`/` 分隔）→ 元数据。
pub type TreeMap = HashMap<String, TreeMeta>;

/// 差异类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEntryKind {
    OnlyLeft,
    OnlyRight,
    LeftNewer,
    RightNewer,
    Same,
}

impl SyncEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            SyncEntryKind::OnlyLeft => "仅左",
            SyncEntryKind::OnlyRight => "仅右",
            SyncEntryKind::LeftNewer => "左新",
            SyncEntryKind::RightNewer => "右新",
            SyncEntryKind::Same => "相同",
        }
    }
}

/// 一条差异。size/mtime 为显示摘要（OnlyX = 存在侧，Newer = 较新侧，
/// Same = 左侧值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEntry {
    pub rel_path: String,
    pub is_dir: bool,
    pub kind: SyncEntryKind,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

/// mtime 比较：返回较新一侧（true = 左新，false = 右新/相同/缺失归右由
/// 调用方语境决定；此处相同归「不左新」）。
fn left_is_newer(l: Option<SystemTime>, r: Option<SystemTime>) -> bool {
    match (l, r) {
        (Some(a), Some(b)) => a > b,
        // 一侧缺失无法判定：不视为左新（归右/相同由调用方决定）。
        _ => false,
    }
}

/// 目录树对比（纯函数，输出按 rel_path 字典序）。规则：
/// - 仅一侧存在 → OnlyLeft/OnlyRight（文件与目录都算）；
/// - 两侧类型不一致（dir↔file）→ LeftNewer（执行覆盖时由引擎报错兜底）；
/// - 都是目录 → Same（目录只在 OnlyX 参与复制）；
/// - 文件大小不同 → mtime 新者归类；mtime 相同或缺失 → LeftNewer；
/// - 大小相同：mtime 差 ≤2s 或一侧缺失 → Same，否则新者归类。
pub fn diff_dir_trees(left: &TreeMap, right: &TreeMap) -> Vec<SyncEntry> {
    let mut out: Vec<SyncEntry> = Vec::new();
    for (rel, l) in left {
        let entry = match right.get(rel) {
            None => SyncEntry {
                rel_path: rel.clone(),
                is_dir: l.is_dir,
                kind: SyncEntryKind::OnlyLeft,
                size: l.size,
                mtime: l.mtime,
            },
            Some(r) => {
                let kind = if l.is_dir || r.is_dir {
                    if l.is_dir && r.is_dir {
                        SyncEntryKind::Same
                    } else {
                        SyncEntryKind::LeftNewer
                    }
                } else if l.size != r.size {
                    if left_is_newer(l.mtime, r.mtime) {
                        SyncEntryKind::LeftNewer
                    } else if left_is_newer(r.mtime, l.mtime) {
                        SyncEntryKind::RightNewer
                    } else {
                        // mtime 相同/缺失但大小不同：归 LeftNewer。
                        SyncEntryKind::LeftNewer
                    }
                } else {
                    match (l.mtime, r.mtime) {
                        (Some(a), Some(b)) => {
                            let diff = a
                                .duration_since(b)
                                .unwrap_or_else(|_| b.duration_since(a).unwrap_or(Duration::ZERO));
                            if diff <= MTIME_TOLERANCE {
                                SyncEntryKind::Same
                            } else if a > b {
                                SyncEntryKind::LeftNewer
                            } else {
                                SyncEntryKind::RightNewer
                            }
                        }
                        _ => SyncEntryKind::Same,
                    }
                };
                let (size, mtime) =
                    if left_is_newer(r.mtime, l.mtime) && kind == SyncEntryKind::RightNewer {
                        (r.size, r.mtime)
                    } else {
                        (l.size, l.mtime)
                    };
                SyncEntry {
                    rel_path: rel.clone(),
                    is_dir: l.is_dir,
                    kind,
                    size,
                    mtime,
                }
            }
        };
        out.push(entry);
    }
    for (rel, r) in right {
        if !left.contains_key(rel) {
            out.push(SyncEntry {
                rel_path: rel.clone(),
                is_dir: r.is_dir,
                kind: SyncEntryKind::OnlyRight,
                size: r.size,
                mtime: r.mtime,
            });
        }
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

/// 递归收集目录树（worker 核心）：rel 路径 `/` 分隔；不跟进符号链接目录
/// （防环，链接本身不入图）；单项失败跳过；cancel 命中提前返回已收集
/// 部分（调用方据此放弃对比）。
fn collect_tree(root: &Path, cancel: &AtomicBool, scanned: &AtomicU64) -> TreeMap {
    let mut map = TreeMap::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return map;
        }
        let Ok(rd) = std::fs::read_dir(verbatim_path(&dir)) else {
            continue;
        };
        for item in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return map;
            }
            scanned.fetch_add(1, Ordering::Relaxed);
            let name = item.file_name().to_string_lossy().into_owned();
            let ft = match item.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let is_dir = ft.is_dir();
            let meta = item.metadata().ok();
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            map.insert(
                child_rel.clone(),
                TreeMeta {
                    is_dir,
                    size: meta
                        .as_ref()
                        .filter(|m| m.is_file())
                        .map(|m| m.len())
                        .unwrap_or(0),
                    mtime: meta.and_then(|m| m.modified().ok()),
                },
            );
            if is_dir {
                stack.push((item.path(), child_rel));
            }
        }
    }
    map
}

/// worker → UI 事件。
enum SyncCompareEvent {
    Done(Vec<SyncEntry>),
}

/// 后台对比任务：收集两树 + diff。Drop 即置取消。
pub struct SyncCompareTask {
    rx: crossbeam_channel::Receiver<SyncCompareEvent>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
}

impl SyncCompareTask {
    pub fn start(left: PathBuf, right: PathBuf) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let scanned = Arc::new(AtomicU64::new(0));
        let (cancel2, scanned2) = (cancel.clone(), scanned.clone());
        std::thread::Builder::new()
            .name("fm-sync-compare".to_string())
            .spawn(move || {
                let left_tree = collect_tree(&left, &cancel2, &scanned2);
                if cancel2.load(Ordering::Relaxed) {
                    return;
                }
                let right_tree = collect_tree(&right, &cancel2, &scanned2);
                if cancel2.load(Ordering::Relaxed) {
                    return;
                }
                let entries = diff_dir_trees(&left_tree, &right_tree);
                let _ = tx.send(SyncCompareEvent::Done(entries));
            })
            .ok();
        Self {
            rx,
            cancel,
            scanned,
        }
    }

    pub fn scanned(&self) -> u64 {
        self.scanned.load(Ordering::Relaxed)
    }

    fn drain(&self) -> Option<Vec<SyncEntry>> {
        let mut done = None;
        for ev in self.rx.try_iter() {
            match ev {
                SyncCompareEvent::Done(entries) => done = Some(entries),
            }
        }
        done
    }
}

impl Drop for SyncCompareTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 同步方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    LeftToRight,
    RightToLeft,
    Both,
}

/// 执行计划：Copy 任务组（sources → dest_dir）+ Delete 项（回收站）。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub copies: Vec<(Vec<PathBuf>, PathBuf)>,
    pub deletes: Vec<PathBuf>,
}

/// 嵌套去重：任一 `/` 前缀祖先也在集合内的项丢弃（目录整体复制/删除
/// 已覆盖后代）。
fn keep_top_level(paths: &BTreeSet<&str>) -> Vec<String> {
    paths
        .iter()
        .filter(|p| !p.match_indices('/').any(|(i, _)| paths.contains(&p[..i])))
        .map(|s| s.to_string())
        .collect()
}

/// 执行计划生成（纯函数）：LeftToRight = OnlyLeft+LeftNewer 复制到右
/// （Overwrite），delete_excess 时 OnlyRight 进删除集；RightToLeft 对称；
/// Both = OnlyLeft→右、OnlyRight→左、LeftNewer→右、RightNewer→左，无删除。
pub fn build_sync_plan(
    entries: &[SyncEntry],
    direction: SyncDirection,
    delete_excess: bool,
    left_dir: &Path,
    right_dir: &Path,
) -> SyncPlan {
    let mut only_left = BTreeSet::new();
    let mut only_right = BTreeSet::new();
    let mut left_newer = BTreeSet::new();
    let mut right_newer = BTreeSet::new();
    for e in entries {
        match e.kind {
            SyncEntryKind::OnlyLeft => {
                only_left.insert(e.rel_path.as_str());
            }
            SyncEntryKind::OnlyRight => {
                only_right.insert(e.rel_path.as_str());
            }
            SyncEntryKind::LeftNewer => {
                left_newer.insert(e.rel_path.as_str());
            }
            SyncEntryKind::RightNewer => {
                right_newer.insert(e.rel_path.as_str());
            }
            SyncEntryKind::Same => {}
        }
    }
    let to_right: Vec<PathBuf> = keep_top_level(&only_left.union(&left_newer).cloned().collect())
        .into_iter()
        .map(|rel| left_dir.join(rel))
        .collect();
    let to_left: Vec<PathBuf> = keep_top_level(&only_right.union(&right_newer).cloned().collect())
        .into_iter()
        .map(|rel| right_dir.join(rel))
        .collect();
    let excess_right: Vec<PathBuf> = keep_top_level(&only_right)
        .into_iter()
        .map(|rel| right_dir.join(rel))
        .collect();
    let excess_left: Vec<PathBuf> = keep_top_level(&only_left)
        .into_iter()
        .map(|rel| left_dir.join(rel))
        .collect();
    let mut plan = SyncPlan::default();
    match direction {
        SyncDirection::LeftToRight => {
            if !to_right.is_empty() {
                plan.copies.push((to_right, right_dir.to_path_buf()));
            }
            if delete_excess {
                plan.deletes = excess_right;
            }
        }
        SyncDirection::RightToLeft => {
            if !to_left.is_empty() {
                plan.copies.push((to_left, left_dir.to_path_buf()));
            }
            if delete_excess {
                plan.deletes = excess_left;
            }
        }
        SyncDirection::Both => {
            if !to_right.is_empty() {
                plan.copies.push((to_right, right_dir.to_path_buf()));
            }
            if !to_left.is_empty() {
                plan.copies.push((to_left, left_dir.to_path_buf()));
            }
        }
    }
    plan
}

/// 对话框动作（FileManagerView 消费）。
pub enum SyncUiAction {
    None,
    /// 「开始同步」确认：执行计划（Copy Overwrite 任务组 + Delete 回收站）。
    Run(SyncPlan),
}

/// 同步目录对话框状态（FileManagerView 持有；非模态 egui::Window）。
pub struct SyncDialog {
    open: bool,
    left_dir: PathBuf,
    right_dir: PathBuf,
    task: Option<SyncCompareTask>,
    entries: Vec<SyncEntry>,
    show_same: bool,
    direction: SyncDirection,
    delete_excess: bool,
    /// 已完成至少一次对比。
    compared: bool,
}

impl Default for SyncDialog {
    fn default() -> Self {
        Self {
            open: false,
            left_dir: PathBuf::new(),
            right_dir: PathBuf::new(),
            task: None,
            entries: Vec::new(),
            show_same: false,
            direction: SyncDirection::LeftToRight,
            delete_excess: false,
            compared: false,
        }
    }
}

impl SyncDialog {
    /// 打开并开始对比（left/right = 两栏当前目录）。
    pub fn open_with(&mut self, left: PathBuf, right: PathBuf) {
        self.left_dir = left;
        self.right_dir = right;
        self.open = true;
        self.start_compare();
    }

    fn start_compare(&mut self) {
        self.task = Some(SyncCompareTask::start(
            self.left_dir.clone(),
            self.right_dir.clone(),
        ));
        self.entries.clear();
        self.compared = false;
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.left_dir, &mut self.right_dir);
        self.start_compare();
    }

    /// 渲染（open 时）；返回本帧动作。对比在途时主动重绘。
    pub fn ui(&mut self, ctx: &egui::Context) -> SyncUiAction {
        if !self.open {
            return SyncUiAction::None;
        }
        if let Some(task) = &self.task {
            if let Some(entries) = task.drain() {
                self.entries = entries;
                self.task = None;
                self.compared = true;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut open = self.open;
        let mut action = SyncUiAction::None;
        egui::Window::new("同步目录")
            .collapsible(false)
            .resizable(true)
            .default_size([680.0, 480.0])
            .open(&mut open)
            .show(ctx, |ui| {
                action = self.ui_body(ui);
            });
        self.open = open;
        if !open {
            self.task = None;
        }
        action
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) -> SyncUiAction {
        let mut action = SyncUiAction::None;
        // 两栏目录 + 交换。
        ui.horizontal(|ui| {
            ui.label("左：");
            ui.label(egui::RichText::new(self.left_dir.display().to_string()).strong());
            if ui.button((icons::SWAP, " 交换左右")).clicked() {
                self.swap();
            }
            ui.label("右：");
            ui.label(egui::RichText::new(self.right_dir.display().to_string()).strong());
        });
        // 方向与选项。
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.direction, SyncDirection::LeftToRight, "左 → 右");
            ui.radio_value(&mut self.direction, SyncDirection::RightToLeft, "右 → 左");
            ui.radio_value(&mut self.direction, SyncDirection::Both, "双向");
            ui.separator();
            ui.add_enabled(
                self.direction != SyncDirection::Both,
                egui::Checkbox::new(&mut self.delete_excess, "删除对侧多余项（回收站）"),
            );
            ui.separator();
            ui.checkbox(&mut self.show_same, "显示相同项");
        });
        // 状态行。
        let diff_count = self
            .entries
            .iter()
            .filter(|e| e.kind != SyncEntryKind::Same)
            .count();
        let status = if let Some(task) = &self.task {
            format!("对比中…已扫描 {} 项", task.scanned())
        } else if self.compared {
            format!(
                "对比完成：共 {} 项 · 差异 {diff_count} 项",
                self.entries.len()
            )
        } else {
            String::new()
        };
        ui.label(status);
        ui.separator();
        // 结果列表（虚拟化；Same 默认隐藏）。
        let visible: Vec<&SyncEntry> = self
            .entries
            .iter()
            .filter(|e| self.show_same || e.kind != SyncEntryKind::Same)
            .collect();
        let kind_color = |kind: SyncEntryKind, ui: &egui::Ui| match kind {
            SyncEntryKind::OnlyLeft | SyncEntryKind::OnlyRight => {
                egui::Color32::from_rgb(0x5f, 0xa8, 0xd3)
            }
            SyncEntryKind::LeftNewer | SyncEntryKind::RightNewer => {
                egui::Color32::from_rgb(0xd3, 0xa8, 0x5f)
            }
            SyncEntryKind::Same => ui.visuals().weak_text_color(),
        };
        let text_color = ui.visuals().text_color();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, visible.len(), |ui, range| {
                for row in range {
                    let Some(e) = visible.get(row) else {
                        continue;
                    };
                    let width = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::hover());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                    }
                    // 差异类型标记（左）+ 相对路径（中，裁剪）+ 大小/时间（右）。
                    let color = kind_color(e.kind, ui);
                    ui.painter().text(
                        rect.min + egui::vec2(4.0, ROW_H / 2.0),
                        egui::Align2::LEFT_CENTER,
                        e.kind.label(),
                        egui::FontId::proportional(12.0),
                        color,
                    );
                    let ts = e
                        .mtime
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    let right_text = if e.is_dir {
                        "〈目录〉".to_string()
                    } else {
                        format!("{} · {}", human_size(e.size), format_mtime(ts))
                    };
                    let name_rect = egui::Rect::from_min_max(
                        rect.min + egui::vec2(44.0, 0.0),
                        egui::pos2(rect.right() - 210.0, rect.max.y),
                    );
                    ui.painter().with_clip_rect(name_rect).text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &e.rel_path,
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    ui.painter().text(
                        egui::pos2(rect.right() - 4.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        right_text,
                        egui::FontId::proportional(12.0),
                        text_color,
                    );
                }
            });
        ui.add_space(4.0);
        // 执行。
        let plan = build_sync_plan(
            &self.entries,
            self.direction,
            self.delete_excess,
            &self.left_dir,
            &self.right_dir,
        );
        let plan_empty = plan.copies.is_empty() && plan.deletes.is_empty();
        // 摘要先于闭包算好（plan 在闭包内可能 move 进 action）。
        let copy_count: usize = plan.copies.iter().map(|(s, _)| s.len()).sum();
        let mut summary = format!("将复制 {copy_count} 项");
        if !plan.deletes.is_empty() {
            summary.push_str(&format!(" · 删除 {} 项", plan.deletes.len()));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.task.is_none() && self.compared && !plan_empty,
                    egui::Button::new("开始同步"),
                )
                .clicked()
            {
                action = SyncUiAction::Run(plan);
            }
            ui.label(egui::RichText::new(&summary).weak());
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_file(size: u64, secs: u64) -> TreeMeta {
        TreeMeta {
            is_dir: false,
            size,
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        }
    }

    fn meta_dir() -> TreeMeta {
        TreeMeta {
            is_dir: true,
            size: 0,
            mtime: None,
        }
    }

    fn kind_of(entries: &[SyncEntry], rel: &str) -> SyncEntryKind {
        entries
            .iter()
            .find(|e| e.rel_path == rel)
            .unwrap_or_else(|| panic!("缺条目 {rel}"))
            .kind
    }

    #[test]
    fn diff_classifies_all_kinds() {
        let mut left = TreeMap::new();
        let mut right = TreeMap::new();
        left.insert("only_l.txt".into(), meta_file(10, 100));
        right.insert("only_r.txt".into(), meta_file(20, 100));
        left.insert("newer_l.txt".into(), meta_file(10, 200));
        right.insert("newer_l.txt".into(), meta_file(10, 100));
        left.insert("newer_r.txt".into(), meta_file(10, 100));
        right.insert("newer_r.txt".into(), meta_file(10, 300));
        left.insert("same.txt".into(), meta_file(10, 100));
        right.insert("same.txt".into(), meta_file(10, 101)); // ≤2s 容差
        left.insert("dir".into(), meta_dir());
        right.insert("dir".into(), meta_dir());
        left.insert("dir_only".into(), meta_dir());
        let entries = diff_dir_trees(&left, &right);
        assert_eq!(kind_of(&entries, "only_l.txt"), SyncEntryKind::OnlyLeft);
        assert_eq!(kind_of(&entries, "only_r.txt"), SyncEntryKind::OnlyRight);
        assert_eq!(kind_of(&entries, "newer_l.txt"), SyncEntryKind::LeftNewer);
        assert_eq!(kind_of(&entries, "newer_r.txt"), SyncEntryKind::RightNewer);
        assert_eq!(kind_of(&entries, "same.txt"), SyncEntryKind::Same);
        assert_eq!(kind_of(&entries, "dir"), SyncEntryKind::Same);
        assert_eq!(kind_of(&entries, "dir_only"), SyncEntryKind::OnlyLeft);
        // 输出按 rel_path 排序。
        let rels: Vec<&str> = entries.iter().map(|e| e.rel_path.as_str()).collect();
        let mut sorted = rels.clone();
        sorted.sort();
        assert_eq!(rels, sorted);
    }

    #[test]
    fn diff_size_diff_with_equal_mtime_goes_left_newer() {
        let mut left = TreeMap::new();
        let mut right = TreeMap::new();
        left.insert("f.txt".into(), meta_file(10, 100));
        right.insert("f.txt".into(), meta_file(20, 100)); // mtime 相同、大小不同
        let entries = diff_dir_trees(&left, &right);
        assert_eq!(kind_of(&entries, "f.txt"), SyncEntryKind::LeftNewer);
        // 大小不同且右新 → RightNewer。
        left.insert("g.txt".into(), meta_file(10, 100));
        right.insert("g.txt".into(), meta_file(20, 200));
        let entries = diff_dir_trees(&left, &right);
        assert_eq!(kind_of(&entries, "g.txt"), SyncEntryKind::RightNewer);
    }

    #[test]
    fn diff_type_mismatch_goes_left_newer() {
        let mut left = TreeMap::new();
        let mut right = TreeMap::new();
        left.insert("x".into(), meta_dir());
        right.insert("x".into(), meta_file(1, 100));
        let entries = diff_dir_trees(&left, &right);
        assert_eq!(kind_of(&entries, "x"), SyncEntryKind::LeftNewer);
    }

    #[test]
    fn keep_top_level_drops_nested() {
        let paths: BTreeSet<&str> = ["a", "a/b", "a/b/c.txt", "d.txt", "e", "e/f.txt"]
            .into_iter()
            .collect();
        let top = keep_top_level(&paths);
        assert_eq!(
            top,
            vec!["a".to_string(), "d.txt".to_string(), "e".to_string()]
        );
    }

    #[test]
    fn build_plan_directions_and_excess() {
        let left_dir = Path::new("/L");
        let right_dir = Path::new("/R");
        let entries = vec![
            SyncEntry {
                rel_path: "ol.txt".into(),
                is_dir: false,
                kind: SyncEntryKind::OnlyLeft,
                size: 1,
                mtime: None,
            },
            SyncEntry {
                rel_path: "or.txt".into(),
                is_dir: false,
                kind: SyncEntryKind::OnlyRight,
                size: 1,
                mtime: None,
            },
            SyncEntry {
                rel_path: "ln.txt".into(),
                is_dir: false,
                kind: SyncEntryKind::LeftNewer,
                size: 1,
                mtime: None,
            },
            SyncEntry {
                rel_path: "rn.txt".into(),
                is_dir: false,
                kind: SyncEntryKind::RightNewer,
                size: 1,
                mtime: None,
            },
        ];
        // 左 → 右（无删除）。
        let plan = build_sync_plan(
            &entries,
            SyncDirection::LeftToRight,
            false,
            left_dir,
            right_dir,
        );
        assert_eq!(plan.copies.len(), 1);
        let (srcs, dest) = &plan.copies[0];
        assert_eq!(dest, &right_dir.to_path_buf());
        assert!(srcs.contains(&left_dir.join("ol.txt")));
        assert!(srcs.contains(&left_dir.join("ln.txt")));
        assert!(!srcs.contains(&right_dir.join("rn.txt")));
        assert!(plan.deletes.is_empty());
        // 左 → 右（删除多余）。
        let plan = build_sync_plan(
            &entries,
            SyncDirection::LeftToRight,
            true,
            left_dir,
            right_dir,
        );
        assert_eq!(plan.deletes, vec![right_dir.join("or.txt")]);
        // 右 → 左（删除多余）。
        let plan = build_sync_plan(
            &entries,
            SyncDirection::RightToLeft,
            true,
            left_dir,
            right_dir,
        );
        let (srcs, dest) = &plan.copies[0];
        assert_eq!(dest, &left_dir.to_path_buf());
        assert!(srcs.contains(&right_dir.join("or.txt")));
        assert!(srcs.contains(&right_dir.join("rn.txt")));
        assert_eq!(plan.deletes, vec![left_dir.join("ol.txt")]);
        // 双向：两组复制、无删除。
        let plan = build_sync_plan(&entries, SyncDirection::Both, true, left_dir, right_dir);
        assert_eq!(plan.copies.len(), 2);
        assert!(plan.deletes.is_empty());
    }

    #[test]
    fn collect_tree_skips_and_counts() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("sub/deep")).unwrap();
        std::fs::write(t.path().join("a.txt"), b"aa").unwrap();
        std::fs::write(t.path().join("sub/b.txt"), b"bbb").unwrap();
        let cancel = AtomicBool::new(false);
        let scanned = AtomicU64::new(0);
        let map = collect_tree(t.path(), &cancel, &scanned);
        assert!(map.get("a.txt").is_some_and(|m| !m.is_dir && m.size == 2));
        assert!(map.get("sub").is_some_and(|m| m.is_dir));
        assert!(map.get("sub/b.txt").is_some_and(|m| m.size == 3));
        assert!(map.get("sub/deep").is_some_and(|m| m.is_dir));
        assert_eq!(map.len(), 4);
        assert!(scanned.load(Ordering::Relaxed) >= 4);
    }
}
