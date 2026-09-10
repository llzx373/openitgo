//! 文件搜索器（Alt+F7，TC Search 简化版）：条件模型 + 后台遍历 worker +
//! 非模态 egui::Window 对话框（FileManagerView 持有）。匹配判定为纯函数
//! （内容检查以闭包注入），便于单测；结果可双击跳转焦点栏或整体
//! 「输送到焦点栏」（分支视图同款注入）。

use crate::views::archive::{format_mtime, human_size};
use crate::views::file_manager_rows::{wildcard_match, FsEntry};
use crate::views::preview_bytes::is_text_extension;
use egui_phosphor_icons::icons;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// 内容子串检索的文件大小上限（超过不读，防慢盘/大文件拖死 worker）。
const CONTENT_SEARCH_MAX_BYTES: u64 = 1024 * 1024;
/// 命中总数上限：超过即截断（Done.truncated 上报），防病态全盘点通配
/// 把 UI 侧命中列表撑爆。
const SEARCH_MAX_HITS: usize = 100_000;
/// 批量发送阈值：满 50 条或距上次发送 200ms 即发一批。
const BATCH_SIZE: usize = 50;
const BATCH_INTERVAL: Duration = Duration::from_millis(200);
/// 结果列表行高（pt）。
const ROW_H: f32 = 20.0;
/// 结果列表大小/时间列宽（pt）。
const SIZE_COL_W: f32 = 80.0;
const TIME_COL_W: f32 = 140.0;

/// 搜索条件。大小为字节（UI 侧从 MB 文本输入换算）。
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// `;` 分隔通配模式（空 = `*`），复用行模型的 wildcard_match。
    pub pattern: String,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// 修改时间在最近 N 天内。
    pub newer_than_days: Option<u32>,
    /// 递归子目录（不跟进符号链接目录，与遍历约定一致）。
    pub recursive: bool,
    /// 可选内容子串（不区分大小写）：仅 ≤1MB 且文本扩展名的文件生效。
    pub content: Option<String>,
}

/// 单条目匹配输入（遍历现场快照）。
pub struct SearchEntryMeta<'a> {
    pub name: &'a str,
    pub is_dir: bool,
    /// 文件大小；目录恒 0（大小条件对目录不生效，见 meta_matches）。
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

/// 名称/大小/时间条件（纯函数）。大小条件只对文件生效（目录无大小概念，
/// 按名/时间匹配）；newer_than_days 要求 mtime 已知，未来时间视为「最新」。
pub fn meta_matches(meta: &SearchEntryMeta, query: &SearchQuery, now: SystemTime) -> bool {
    let pattern = if query.pattern.trim().is_empty() {
        "*"
    } else {
        query.pattern.as_str()
    };
    if !wildcard_match(pattern, meta.name) {
        return false;
    }
    if !meta.is_dir {
        if let Some(min) = query.min_size {
            if meta.size < min {
                return false;
            }
        }
        if let Some(max) = query.max_size {
            if meta.size > max {
                return false;
            }
        }
    }
    if let Some(days) = query.newer_than_days {
        let limit = Duration::from_secs(u64::from(days) * 86_400);
        let within = match meta.mtime {
            Some(t) if t <= now => now.duration_since(t).map(|d| d <= limit).unwrap_or(false),
            // 未来时间（时钟漂移）按「最新」计。
            Some(_) => true,
            None => false,
        };
        if !within {
            return false;
        }
    }
    true
}

/// 完整匹配判定：meta 条件全过后，有内容条件时仅对 ≤1MB 文本扩展名文件
/// 惰性调用 `read_text`（返回解码后文本；读不出 = 不匹配），不区分大小写。
/// 目录在内容条件下恒不匹配（内容条件只对文件）。
pub fn entry_matches(
    meta: &SearchEntryMeta,
    query: &SearchQuery,
    now: SystemTime,
    read_text: &mut dyn FnMut() -> Option<String>,
) -> bool {
    if !meta_matches(meta, query, now) {
        return false;
    }
    let Some(needle) = &query.content else {
        return true;
    };
    if meta.is_dir || meta.size > CONTENT_SEARCH_MAX_BYTES || !is_searchable_text_name(meta.name) {
        return false;
    }
    let Some(text) = read_text() else {
        return false;
    };
    text.to_lowercase().contains(&needle.to_lowercase())
}

/// 内容检索的文本扩展名门槛（与预览的文本集合同源）。
fn is_searchable_text_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_text_extension)
}

/// 一条命中。name = 条目名；rel_dir = 相对搜索根的父目录（`/` 分隔，
/// 根下直属为空串）；path = 全路径。
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: PathBuf,
    pub rel_dir: String,
    pub name: String,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub is_dir: bool,
}

impl SearchHit {
    /// 相对搜索根的显示路径（`rel_dir/name`，根下直属 = name）。
    pub fn display_path(&self) -> String {
        if self.rel_dir.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.rel_dir, self.name)
        }
    }

    /// 「输送到焦点栏」的条目转换（分支视图同款：name 存显示名）。
    pub fn to_fs_entry(&self) -> FsEntry {
        FsEntry {
            name: self.display_path(),
            path: self.path.clone(),
            is_dir: self.is_dir,
            size: (!self.is_dir).then_some(self.size),
            mtime: self.mtime,
            is_symlink: false,
            is_hidden: crate::views::file_manager_rows::is_hidden_name(&self.name),
            is_readonly: false,
            is_system: false,
            rel_dir: self.rel_dir.clone(),
        }
    }
}

/// worker → UI 事件。
enum SearchEvent {
    Batch(Vec<SearchHit>),
    /// 遍历结束（truncated = 命中达上限截断；取消时不发）。
    Done {
        truncated: bool,
    },
}

/// 后台搜索任务：遍历线程 + 结果 channel + 已扫描计数 + 取消旗标。
/// Drop 即置取消（worker 在条目循环与发送失败处检查退出，线程自行收尾）。
pub struct SearchTask {
    rx: crossbeam_channel::Receiver<SearchEvent>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
}

impl SearchTask {
    pub fn start(root: PathBuf, query: SearchQuery) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let scanned = Arc::new(AtomicU64::new(0));
        let (cancel2, scanned2) = (cancel.clone(), scanned.clone());
        std::thread::Builder::new()
            .name("fm-search".to_string())
            .spawn(move || run_search(&root, &query, &tx, &cancel2, &scanned2))
            .ok();
        Self {
            rx,
            cancel,
            scanned,
        }
    }

    /// 已扫描条目数（进度行显示用）。
    pub fn scanned(&self) -> u64 {
        self.scanned.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 排空一批事件；返回 (新命中, 结束标记 Option<truncated>)。
    fn drain(&self, hits: &mut Vec<SearchHit>) -> Option<bool> {
        let mut done = None;
        for ev in self.rx.try_iter() {
            match ev {
                SearchEvent::Batch(batch) => hits.extend(batch),
                SearchEvent::Done { truncated } => done = Some(truncated),
            }
        }
        done
    }
}

impl Drop for SearchTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 遍历核心：迭代栈模式同分支视图的 collect_branch（符号链接目录不跟进
/// 防环、单项失败跳过；此处目录本身也参与匹配产出命中，且带取消/批量
/// 发送，无法直接复用，按同款模式实现）。发送失败（UI 已退出）即收工。
fn run_search(
    root: &Path,
    query: &SearchQuery,
    tx: &crossbeam_channel::Sender<SearchEvent>,
    cancel: &AtomicBool,
    scanned: &AtomicU64,
) {
    let now = SystemTime::now();
    let mut batch: Vec<SearchHit> = Vec::new();
    let mut last_flush = Instant::now();
    let mut total_hits = 0usize;
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    'outer: while let Some((dir, rel)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            scanned.fetch_add(1, Ordering::Relaxed);
            let item_path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let ft = item.file_type().ok();
            let is_symlink = ft.map(|t| t.is_symlink()).unwrap_or(false);
            let is_dir = !is_symlink && ft.map(|t| t.is_dir()).unwrap_or(false);
            let meta = item.metadata().ok();
            let size = meta
                .as_ref()
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .unwrap_or(0);
            let mtime = meta.and_then(|m| m.modified().ok());
            let smeta = SearchEntryMeta {
                name: &name,
                is_dir,
                size,
                mtime,
            };
            let matched = entry_matches(&smeta, query, now, &mut || read_search_text(&item_path));
            if matched {
                let hit = SearchHit {
                    path: item_path.clone(),
                    rel_dir: rel.clone(),
                    name: name.clone(),
                    size,
                    mtime,
                    is_dir,
                };
                batch.push(hit);
                total_hits += 1;
                if batch.len() >= BATCH_SIZE || last_flush.elapsed() >= BATCH_INTERVAL {
                    if tx
                        .send(SearchEvent::Batch(std::mem::take(&mut batch)))
                        .is_err()
                    {
                        return;
                    }
                    last_flush = Instant::now();
                }
                if total_hits >= SEARCH_MAX_HITS {
                    truncated = true;
                    break 'outer;
                }
            }
            if is_dir && query.recursive {
                let child_rel = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                stack.push((item_path, child_rel));
            }
        }
    }
    if !batch.is_empty() && tx.send(SearchEvent::Batch(batch)).is_err() {
        return;
    }
    let _ = tx.send(SearchEvent::Done { truncated });
}

/// 内容检索读取：全文件 ≤1MB（调用方已按扩展名/大小门槛过滤），
/// 经 decode_text_guess 解码（非 UTF-8 中文文本同预览管线）。
fn read_search_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    openitgo_parser::archive::decode_text_guess(&bytes)
}

/// 对话框动作（FileManagerView 消费）。
pub enum SearchUiAction {
    None,
    /// 双击/Enter/右键「打开所在目录」：焦点栏导航到所在目录并选中该项。
    Reveal(PathBuf),
    /// 「输送到焦点栏」：全部命中注入焦点栏（分支视图同款展示）。
    FeedToPanel,
}

/// 搜索对话框状态（FileManagerView 持有；非模态 egui::Window）。
pub struct SearchDialog {
    pub open: bool,
    /// 搜索根 = 打开对话框时焦点栏当前目录。
    root: PathBuf,
    // 条件输入（会话内记忆，不落盘）。
    pattern: String,
    min_size_mb: String,
    max_size_mb: String,
    newer_days: String,
    recursive: bool,
    content: String,
    // 结果与在途任务。
    hits: Vec<SearchHit>,
    selected: Option<usize>,
    task: Option<SearchTask>,
    /// 搜索已结束（None = 未搜过/搜索中；Some = truncated）。
    finished: Option<bool>,
}

impl Default for SearchDialog {
    fn default() -> Self {
        Self {
            open: false,
            root: PathBuf::new(),
            pattern: String::new(),
            min_size_mb: String::new(),
            max_size_mb: String::new(),
            newer_days: String::new(),
            recursive: true,
            content: String::new(),
            hits: Vec::new(),
            selected: None,
            task: None,
            finished: None,
        }
    }
}

impl SearchDialog {
    /// 打开对话框（搜索根 = 焦点栏当前目录）；条件与上次结果保留。
    pub fn open_with(&mut self, root: PathBuf) {
        self.root = root;
        self.open = true;
    }

    /// 关闭对话框并取消在途搜索（跳转/输送/外部关窗共用）。
    pub fn close(&mut self) {
        self.open = false;
        self.task = None;
    }

    /// MB 文本输入 → 字节（空/非法 = None）。
    fn parse_mb(text: &str) -> Option<u64> {
        let v: f64 = text.trim().parse().ok()?;
        (v >= 0.0).then_some((v * 1024.0 * 1024.0) as u64)
    }

    fn build_query(&self) -> SearchQuery {
        SearchQuery {
            pattern: self.pattern.clone(),
            min_size: Self::parse_mb(&self.min_size_mb),
            max_size: Self::parse_mb(&self.max_size_mb),
            newer_than_days: self.newer_days.trim().parse().ok(),
            recursive: self.recursive,
            content: (!self.content.trim().is_empty()).then(|| self.content.clone()),
        }
    }

    fn start_search(&mut self) {
        // 重开搜索先取消旧任务（Drop 置旗标，旧线程随发送失败/旗标退出）。
        self.task = None;
        self.hits.clear();
        self.selected = None;
        self.finished = None;
        let query = self.build_query();
        self.task = Some(SearchTask::start(self.root.clone(), query));
    }

    fn stop_search(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
        self.task = None;
        self.finished = Some(false);
    }

    /// 渲染（open 时）；返回本帧动作。搜索在途时主动重绘排空结果。
    pub fn ui(&mut self, ctx: &egui::Context) -> SearchUiAction {
        if !self.open {
            return SearchUiAction::None;
        }
        // 排空 worker 事件。
        if let Some(task) = &self.task {
            let done = task.drain(&mut self.hits);
            if let Some(truncated) = done {
                self.finished = Some(truncated);
                self.task = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut open = self.open;
        let mut action = SearchUiAction::None;
        egui::Window::new(format!("搜索 — {}", self.root.display()))
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 480.0])
            .open(&mut open)
            .show(ctx, |ui| {
                action = self.ui_body(ui);
            });
        self.open = open;
        if !open {
            // 关闭即取消在途搜索。
            self.task = None;
        }
        action
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) -> SearchUiAction {
        let mut action = SearchUiAction::None;
        // 条件区。
        egui::Grid::new("fm-search-conds")
            .num_columns(4)
            .show(ui, |ui| {
                ui.label("模式:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.pattern)
                        .hint_text("*.jpg;EP*")
                        .desired_width(200.0),
                );
                ui.label("内容包含:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.content)
                        .hint_text("空 = 不搜内容")
                        .desired_width(160.0),
                );
                ui.end_row();
                ui.label("大小(MB):");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.min_size_mb).desired_width(50.0));
                    ui.label("—");
                    ui.add(egui::TextEdit::singleline(&mut self.max_size_mb).desired_width(50.0));
                });
                ui.label("最近几天修改:");
                ui.add(egui::TextEdit::singleline(&mut self.newer_days).desired_width(50.0));
                ui.end_row();
                ui.checkbox(&mut self.recursive, "包含子目录");
                ui.end_row();
            });
        ui.add_space(4.0);
        // 开始/停止 + 状态行。
        ui.horizontal(|ui| {
            let searching = self.task.is_some();
            if ui
                .add_enabled(!searching, egui::Button::new("开始"))
                .clicked()
            {
                self.start_search();
            }
            if ui
                .add_enabled(searching, egui::Button::new("停止"))
                .clicked()
            {
                self.stop_search();
            }
            ui.separator();
            let scanned = self.task.as_ref().map(|t| t.scanned()).unwrap_or(0);
            let mut status = format!("已扫描 {scanned} 项 · 命中 {} 项", self.hits.len());
            if let Some(truncated) = self.finished {
                status.push_str(if truncated {
                    " · 已截断"
                } else {
                    " · 完成"
                });
            } else if searching {
                status.push_str(" · 搜索中…");
            }
            ui.label(status);
        });
        ui.add_space(4.0);
        // 结果列表（虚拟化；单击选中，双击/Enter 跳转，右键「打开所在目录」）。
        let row_count = self.hits.len();
        let mut reveal: Option<PathBuf> = None;
        let enter =
            ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().egui_wants_keyboard_input();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, row_count, |ui, range| {
                for row in range {
                    let Some(hit) = self.hits.get(row) else {
                        continue;
                    };
                    let width = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::click());
                    let is_sel = self.selected == Some(row);
                    if is_sel {
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                    }
                    let text_color = ui.visuals().text_color();
                    let right = rect.right() - 4.0;
                    let time_right = right;
                    let size_right = time_right - TIME_COL_W;
                    // 名称列（裁剪防画进右侧列）。
                    let name_rect = egui::Rect::from_min_max(
                        rect.min + egui::vec2(4.0, 0.0),
                        egui::pos2(size_right - SIZE_COL_W - 4.0, rect.max.y),
                    );
                    let icon = if hit.is_dir {
                        icons::FOLDER
                    } else {
                        icons::FILE
                    };
                    let display = format!("{} {}", icon.as_str(), hit.display_path());
                    ui.painter().with_clip_rect(name_rect).text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        display,
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    let size_text = if hit.is_dir {
                        String::new()
                    } else {
                        human_size(hit.size)
                    };
                    ui.painter().text(
                        egui::pos2(size_right, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        size_text,
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    let ts = hit
                        .mtime
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    ui.painter().text(
                        egui::pos2(time_right, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        format_mtime(ts),
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    if resp.clicked() {
                        self.selected = Some(row);
                    }
                    if resp.double_clicked() {
                        reveal = Some(hit.path.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.button((icons::FOLDER, " 打开所在目录")).clicked() {
                            reveal = Some(hit.path.clone());
                            ui.close();
                        }
                    });
                }
            });
        if enter {
            if let Some(row) = self.selected {
                if let Some(hit) = self.hits.get(row) {
                    reveal = Some(hit.path.clone());
                }
            }
        }
        if let Some(path) = reveal {
            action = SearchUiAction::Reveal(path);
        }
        // 底部「输送到焦点栏」。
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.hits.is_empty(),
                    egui::Button::new((icons::ARROW_SQUARE_OUT, " 输送到焦点栏")),
                )
                .clicked()
            {
                action = SearchUiAction::FeedToPanel;
            }
        });
        action
    }

    /// 「输送到焦点栏」的条目集（分支视图同款：name = 显示名）。
    pub fn feed_entries(&self) -> Vec<FsEntry> {
        self.hits.iter().map(SearchHit::to_fs_entry).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> SearchQuery {
        SearchQuery {
            recursive: true,
            ..Default::default()
        }
    }

    fn file(name: &'static str, size: u64) -> SearchEntryMeta<'static> {
        SearchEntryMeta {
            name,
            is_dir: false,
            size,
            mtime: None,
        }
    }

    fn no_text() -> Option<String> {
        panic!("read_text 不应被调用")
    }

    #[test]
    fn meta_matches_pattern_empty_is_star() {
        let q = query();
        assert!(meta_matches(
            &file("anything.xyz", 1),
            &q,
            SystemTime::now()
        ));
        let q2 = SearchQuery {
            pattern: "*.jpg;EP*".into(),
            ..query()
        };
        assert!(meta_matches(&file("a.JPG", 1), &q2, SystemTime::now()));
        assert!(meta_matches(&file("EP2.txt", 1), &q2, SystemTime::now()));
        assert!(!meta_matches(&file("b.png", 1), &q2, SystemTime::now()));
    }

    #[test]
    fn meta_matches_size_range_files_only() {
        let q = SearchQuery {
            min_size: Some(100),
            max_size: Some(200),
            ..query()
        };
        let now = SystemTime::now();
        assert!(meta_matches(&file("a.bin", 150), &q, now));
        assert!(meta_matches(&file("a.bin", 100), &q, now));
        assert!(meta_matches(&file("a.bin", 200), &q, now));
        assert!(!meta_matches(&file("a.bin", 99), &q, now));
        assert!(!meta_matches(&file("a.bin", 201), &q, now));
        // 目录不受大小条件约束。
        let dir = SearchEntryMeta {
            name: "d",
            is_dir: true,
            size: 0,
            mtime: None,
        };
        assert!(meta_matches(&dir, &q, now));
    }

    #[test]
    fn meta_matches_newer_than_days() {
        let now = SystemTime::now();
        let q = SearchQuery {
            newer_than_days: Some(7),
            ..query()
        };
        let recent = SearchEntryMeta {
            mtime: Some(now - Duration::from_secs(3 * 86_400)),
            ..file("a", 1)
        };
        let old = SearchEntryMeta {
            mtime: Some(now - Duration::from_secs(30 * 86_400)),
            ..file("a", 1)
        };
        assert!(meta_matches(&recent, &q, now));
        assert!(!meta_matches(&old, &q, now));
        // mtime 未知不匹配时间条件；未来时间按最新计。
        assert!(!meta_matches(&file("a", 1), &q, now));
        let future = SearchEntryMeta {
            mtime: Some(now + Duration::from_secs(86_400)),
            ..file("a", 1)
        };
        assert!(meta_matches(&future, &q, now));
    }

    #[test]
    fn entry_matches_content_gating() {
        let q = SearchQuery {
            content: Some("Hello".into()),
            ..query()
        };
        let now = SystemTime::now();
        // 目录/非文本扩展名/超 1MB：不调用 read_text 且恒不匹配。
        let dir = SearchEntryMeta {
            name: "d.txt",
            is_dir: true,
            size: 1,
            mtime: None,
        };
        assert!(!entry_matches(&dir, &q, now, &mut no_text));
        assert!(!entry_matches(&file("a.bin", 1), &q, now, &mut no_text));
        let big = file("a.txt", CONTENT_SEARCH_MAX_BYTES + 1);
        assert!(!entry_matches(&big, &q, now, &mut no_text));
        // 文本文件：不区分大小写；读不出 = 不匹配。
        let hit = file("a.txt", 10);
        assert!(entry_matches(&hit, &q, now, &mut || Some(
            "say HELLO world".into()
        )));
        assert!(!entry_matches(&hit, &q, now, &mut || Some(
            "nothing".into()
        )));
        assert!(!entry_matches(&hit, &q, now, &mut || None));
        // 无内容条件时不调用 read_text。
        let q2 = query();
        assert!(entry_matches(&hit, &q2, now, &mut no_text));
    }

    #[test]
    fn hit_display_path_and_fs_entry() {
        let top = SearchHit {
            path: PathBuf::from("/r/a.txt"),
            rel_dir: String::new(),
            name: "a.txt".into(),
            size: 5,
            mtime: None,
            is_dir: false,
        };
        assert_eq!(top.display_path(), "a.txt");
        let nested = SearchHit {
            path: PathBuf::from("/r/sub/dir/b.txt"),
            rel_dir: "sub/dir".into(),
            name: "b.txt".into(),
            size: 7,
            mtime: None,
            is_dir: false,
        };
        assert_eq!(nested.display_path(), "sub/dir/b.txt");
        let e = nested.to_fs_entry();
        assert_eq!(e.name, "sub/dir/b.txt");
        assert_eq!(e.rel_dir, "sub/dir");
        assert_eq!(e.size, Some(7));
        assert!(!e.is_dir);
        let dir_hit = SearchHit {
            is_dir: true,
            ..nested.clone()
        };
        assert_eq!(dir_hit.to_fs_entry().size, None);
    }

    #[test]
    fn worker_finds_and_reports() {
        let tmp = std::env::temp_dir().join(format!("fm-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("ep1.txt"), "hello search").unwrap();
        std::fs::write(tmp.join("sub/ep2.txt"), "nothing here").unwrap();
        std::fs::write(tmp.join("sub/skip.bin"), "hello bin").unwrap();
        let q = SearchQuery {
            pattern: "ep*".into(),
            content: Some("hello".into()),
            ..query()
        };
        let task = SearchTask::start(tmp.clone(), q);
        let mut hits = Vec::new();
        let mut done = None;
        for _ in 0..200 {
            done = task.drain(&mut hits);
            if done.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(done, Some(false));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "ep1.txt");
        assert_eq!(hits[0].rel_dir, "");
        assert!(task.scanned() >= 4);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn worker_non_recursive_and_dir_hits() {
        let tmp = std::env::temp_dir().join(format!("fm-search-nr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("matchdir/deep")).unwrap();
        std::fs::write(tmp.join("matchdir/deep/match.txt"), "x").unwrap();
        std::fs::write(tmp.join("match.txt"), "x").unwrap();
        let q = SearchQuery {
            pattern: "match*".into(),
            recursive: false,
            ..Default::default()
        };
        let task = SearchTask::start(tmp.clone(), q);
        let mut hits = Vec::new();
        let mut done = None;
        for _ in 0..200 {
            done = task.drain(&mut hits);
            if done.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(done, Some(false));
        // 非递归：根下 match.txt + matchdir 目录命中；deep/match.txt 不出现。
        let names: Vec<String> = hits.iter().map(|h| h.display_path()).collect();
        assert!(names.contains(&"match.txt".to_string()), "{names:?}");
        assert!(names.contains(&"matchdir".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("deep")), "{names:?}");
        assert!(hits.iter().any(|h| h.is_dir && h.name == "matchdir"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
