//! 双栏文件管理器视图（Total Commander 形态）：左右两个 `FsPanel` 各自独立
//! 浏览本地文件系统，Tab 切换焦点栏，单击/Ctrl/Shift 选择，双击分发打开
//! （目录 → 栏内进入；压缩包 → Archive 视图；其余 → open_path 分发）。
//! 单栏模式右半为预览面板（图片/文本/元信息占位，选中即预览）；
//! 双栏模式 F3 对焦点文件弹临时预览窗，复用同一预览管线。
//!
//! 行渲染严格遵循 AGENTS.md 的列布局约定（与 archive.rs 明细列表同范式）：
//! 固定行高 + show_rows 虚拟化、item_spacing.y 归零、行内容 scope 内
//! interact_size.y 压回 ROW_HEIGHT-4、scope 后 advance_cursor_after_rect
//! 钉回行底、右两列 painter.text 右对齐直绘（禁止 RTL 嵌套）。

use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::archive::{format_mtime, human_size};
use crate::views::file_manager_dialog::{
    CompressDialog, CopyMoveDialog, DeleteDialog, FmDialog, FmDialogOutcome, NewDirDialog,
    RenameDialog, SelectGroupDialog,
};
use crate::views::file_manager_panel::{
    fallback_existing_dir, list_drives, FocusMove, FsPanel, PanelLoadState, COL_RIGHT_PAD,
    ROW_HEIGHT,
};
use crate::views::file_manager_rows::{FsEntry, SortKey};
use crate::views::file_ops::{
    create_dir, format_eta, rename_entry, suggest_folder_name, FileOpManager, FinishedOp, OpKind,
    OpSpeedMeter,
};
use crate::views::preview_bytes::{is_previewable_name, load_file_preview, PreviewData};
use egui_phosphor_icons::{icons, Icon};
use openitgo_parser::archive::archive_kind;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// 双栏/预览分隔条宽度（pt）。
const SPLITTER_WIDTH: f32 = 6.0;
/// 栏内容最小宽度（pt）：窗口过小时不再压缩。
const PANEL_MIN_WIDTH: f32 = 120.0;
/// 双栏比例/预览比例的取值范围（与 settings.fm_dual_ratio 的 clamp 一致）。
const RATIO_MIN: f32 = 0.2;
const RATIO_MAX: f32 = 0.8;
/// 表头「名称」文字的左缩进（pt）：与行内容对齐
/// （行 = 6pt shrink + 约 16pt 图标 + 6pt 间距）。
const NAME_HEADER_INDENT: f32 = 6.0 + 16.0 + 6.0;

/// 面板布局：双栏（ratio = 左栏宽占比）/ 单栏（右半为预览面板）。
pub enum PanelLayout {
    Dual { ratio: f32 },
    Single { preview_open: bool },
}

/// 文件管理器持久化快照（app 侧 diff 后写回 settings.fm_*）。
#[derive(Debug, Clone, PartialEq)]
pub struct FmStateSnapshot {
    /// "dual" | "single"。
    pub layout: String,
    /// 双栏左栏宽占比（Single 期间取切单栏前保存的比例）。
    pub ratio: f32,
    /// 单栏预览面板开关（Dual 期间取切双栏前保存的开关）。
    pub preview_open: bool,
    /// 排序键 "name"|"size"|"mtime"|"ext"：取活动栏（取舍：双栏各自排序可能不同，
    /// settings 只有单值，持久化活动栏的排序）。
    pub sort_key: String,
    pub sort_asc: bool,
    /// 两栏当前目录（字符串；空 = 用户主目录，跟随 resolve_fm_dir 语义）。
    pub dir_left: String,
    pub dir_right: String,
    /// 常用目录书签（两栏共享）。
    pub bookmarks: Vec<String>,
}

pub struct FileManagerView {
    pub layout: PanelLayout,
    pub panels: [FsPanel; 2],
    /// 焦点栏（0/1）：键盘操作目标；鼠标点击某栏任意处即激活。
    pub active: usize,
    /// 切回双栏时恢复的比例（Single 期间 Dual.ratio 不可见）。
    saved_ratio: f32,
    /// 切回单栏时恢复的预览开关（Dual 期间 Single.preview_open 不可见）。
    saved_preview_open: bool,
    /// 单栏模式预览面板宽度占比（拖动分隔条可调）。
    preview_ratio: f32,
    /// 当前预览目标文件（「选中即预览」：焦点行落到的文件）。
    preview_path: Option<PathBuf>,
    /// 预览目标的条目快照（元信息占位用；目录列举刷新后可能已失效）。
    preview_entry: Option<FsEntry>,
    /// 在途的后台预览读取。
    preview: Option<AsyncOpener<PreviewData>>,
    /// poll 拿到、待 ui() 上传为纹理的图片。
    pending_preview_image: Option<egui::ColorImage>,
    preview_tex: Option<egui::TextureHandle>,
    preview_text: Option<String>,
    preview_note: Option<String>,
    /// 图片预览「原始尺寸」模式（false = 适应宽度）。
    preview_full_size: bool,
    /// F3 临时预览弹窗开关（双栏模式）。
    preview_window_open: bool,
    /// 后台文件操作（复制/移动/删除）。
    ops: FileOpManager,
    /// 操作确认对话框；Some 时渲染模态窗口并屏蔽面板键盘。
    dialog: Option<FmDialog>,
    /// 应用内剪贴板（Ctrl+C/X 复制/剪切，Ctrl+V 粘贴到焦点栏）。
    clipboard: Vec<PathBuf>,
    clipboard_cut: bool,
    /// 盘符列表缓存与在途后台枚举（盘符下拉共用；慢速设备不卡 UI）。
    drives: Option<Vec<PathBuf>>,
    drives_rx: Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>,
    /// 删除前是否弹确认框（settings.fm_confirm_delete 快照，供右键菜单使用）。
    confirm_delete: bool,
    /// 常用目录书签（两栏共享，权威走快照写回 settings.fm_bookmarks）。
    bookmarks: Vec<PathBuf>,
    /// 「选择组」对话框上次使用的模式（会话内记忆，不落盘）。
    select_group_pattern: String,
    /// Alt+↓ 的一次性请求：下一帧焦点栏的历史下拉菜单开/关切换
    /// （弹层开关状态在 egui memory，键盘段无法直接触达）。
    history_menu_toggle: bool,
    /// 状态栏速度/ETA 估算器（任务 id + EMA 采样器；任务切换重置）。
    op_speed: Option<(u64, OpSpeedMeter)>,
    /// Ctrl+Q 对面栏快速预览（双栏；会话内状态，不落盘）：开启时非活动栏
    /// 整栏替换为预览面板，目标 = 活动栏焦点文件，焦点移动跟随。
    quickview_open: bool,
}

/// 帧内意图：行内交互写入，帧尾统一触发回调（避免回调嵌套借用）。
#[derive(Default)]
struct FmIntents {
    back: bool,
    open_path: Option<PathBuf>,
    open_archive: Option<PathBuf>,
    open_as_comic: Option<PathBuf>,
    /// 右键「解压到另一栏/当前目录…」：(压缩包路径, 目标目录)。
    extract: Option<(PathBuf, PathBuf)>,
    /// 文件操作汇总/错误（完成/取消/失败时上报给 app error_message）。
    op_error: Option<String>,
    /// 删除确认框「不再询问」勾选（false = 仍需确认）。
    confirm_delete_change: Option<bool>,
}

pub struct FmCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    /// 打开分发（图片/电子书/媒体/漫画文件夹等，走 app 的 open_path）。
    pub on_open_path: &'a mut dyn FnMut(PathBuf),
    /// 双击压缩包：进 Archive 视图（open_archive_browser）。
    pub on_open_archive: &'a mut dyn FnMut(PathBuf),
    /// 右键「作为漫画打开」（目录与压缩包可用）。
    pub on_open_as_comic: &'a mut dyn FnMut(PathBuf),
    /// 右键「解压到…」：(压缩包路径, 目标目录)；app 侧弹解压对话框。
    pub on_extract: &'a mut dyn FnMut(PathBuf, PathBuf),
    /// 文件操作完成/取消/失败的汇总消息。
    pub on_op_error: &'a mut dyn FnMut(String),
    /// 删除确认框「不再询问」勾选变化（写回 settings.fm_confirm_delete）。
    pub on_confirm_delete_change: &'a mut dyn FnMut(bool),
}

/// 名称/大小/修改时间三列的 x 坐标单一来源（表头 paint、行列分隔竖线、
/// 列宽拖拽共用），消除各自手算的漂移。名称列宽 = 剩余弹性。
#[derive(Debug, Clone, Copy)]
struct ColumnLayout {
    /// 名称列右缘（= 名称|大小分隔竖线 x、大小列左缘）。
    size_left: f32,
    /// 大小列右缘（= 大小|时间分隔竖线 x、时间列左缘），大小文字右锚点。
    mtime_left: f32,
    /// 时间文字右锚点（行右缘内 COL_RIGHT_PAD 处）。
    content_right: f32,
}

/// 由行/表头 rect 的右缘、列块平移量与两列宽度算出各列坐标。
/// shift ≤ 0：0 = 列块贴右缘（名称列吃满剩余宽度）；<0 = 列块整体左移。
fn column_layout(right: f32, shift: f32, size_w: f32, mtime_w: f32) -> ColumnLayout {
    let content_right = right - COL_RIGHT_PAD + shift;
    let mtime_left = content_right - mtime_w;
    let size_left = mtime_left - size_w;
    ColumnLayout {
        size_left,
        mtime_left,
        content_right,
    }
}

/// 栏间拖放复制的 payload：行 drag source 设置，经 egui 全局 dnd 状态
/// 跨栏传递（payload 与 widget Id 无关，栏间 push_id 隔离不影响）。
#[derive(Debug, Clone)]
struct FmDragPayload {
    sources: Vec<PathBuf>,
    src_panel: usize,
}

/// 行拖拽的源集合（Explorer 惯例）：被拖行已在选中集内 → 整个选中集
/// （排序保证确定性），否则仅被拖行自身。「..」上级行不可拖（调用处保证）。
fn drag_sources(selected: &HashSet<PathBuf>, row_path: &Path) -> Vec<PathBuf> {
    if selected.contains(row_path) {
        let mut sources: Vec<PathBuf> = selected.iter().cloned().collect();
        sources.sort();
        sources
    } else {
        vec![row_path.to_path_buf()]
    }
}

impl FileManagerView {
    /// 从 settings 恢复布局/排序（app 构造时调用）。
    pub fn new(
        layout: &str,
        ratio: f32,
        preview_open: bool,
        sort_key: &str,
        sort_asc: bool,
        bookmarks: &[String],
    ) -> Self {
        let layout = if layout == "single" {
            PanelLayout::Single { preview_open }
        } else {
            PanelLayout::Dual { ratio }
        };
        let sort = match sort_key {
            "size" => SortKey::Size,
            "mtime" => SortKey::Mtime,
            "ext" => SortKey::Ext,
            _ => SortKey::Name,
        };
        Self {
            layout,
            panels: [FsPanel::new(sort, sort_asc), FsPanel::new(sort, sort_asc)],
            active: 0,
            saved_ratio: ratio,
            saved_preview_open: preview_open,
            preview_ratio: 0.35,
            preview_path: None,
            preview_entry: None,
            preview: None,
            pending_preview_image: None,
            preview_tex: None,
            preview_text: None,
            preview_note: None,
            preview_full_size: false,
            preview_window_open: false,
            ops: FileOpManager::default(),
            dialog: None,
            clipboard: Vec::new(),
            clipboard_cut: false,
            drives: None,
            drives_rx: None,
            confirm_delete: true,
            bookmarks: bookmarks.iter().map(PathBuf::from).collect(),
            select_group_pattern: String::new(),
            history_menu_toggle: false,
            op_speed: None,
            quickview_open: false,
        }
    }

    /// 采集当前状态快照（持久化写回用）。目录取面板当前 dir；
    /// 排序取活动栏（settings 只有单值，见 FmStateSnapshot.sort_key 注释）。
    pub fn snapshot(&self) -> FmStateSnapshot {
        let (layout, ratio, preview_open) = match &self.layout {
            PanelLayout::Dual { ratio } => ("dual", *ratio, self.saved_preview_open),
            PanelLayout::Single { preview_open } => ("single", self.saved_ratio, *preview_open),
        };
        let panel = &self.panels[self.active];
        let sort_key = match panel.sort_key {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Mtime => "mtime",
            SortKey::Ext => "ext",
        };
        FmStateSnapshot {
            layout: layout.to_string(),
            ratio,
            preview_open,
            sort_key: sort_key.to_string(),
            sort_asc: panel.sort_asc,
            dir_left: self.panels[0].dir.display().to_string(),
            dir_right: self.panels[1].dir.display().to_string(),
            bookmarks: self
                .bookmarks
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }
    }

    /// 添加书签（两栏共享）；已在列表中时 no-op 返回 false。
    pub fn add_bookmark(&mut self, dir: &Path) -> bool {
        if self.bookmarks.iter().any(|b| b == dir) {
            return false;
        }
        self.bookmarks.push(dir.to_path_buf());
        true
    }

    /// 移除书签；不在列表中返回 false。
    pub fn remove_bookmark(&mut self, dir: &Path) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b != dir);
        self.bookmarks.len() != before
    }

    /// 设置页改动默认布局/双栏比例时同步到本视图（此时视图休眠）。
    /// 不同步的话，maybe_save_fm_state 会在下次进入视图时把旧布局写回
    /// settings，覆盖用户在设置页的改动。
    pub fn apply_layout_settings(&mut self, layout: &str, ratio: f32) {
        let ratio = ratio.clamp(RATIO_MIN, RATIO_MAX);
        if layout == "single" {
            if let PanelLayout::Dual { ratio: r } = self.layout {
                self.saved_ratio = r;
            }
            // 单栏只渲染 panels[0]：焦点栏内容保留到左栏（同顶栏切换逻辑）。
            if self.active == 1 {
                self.panels.swap(0, 1);
                self.active = 0;
            }
            self.preview_window_open = false;
            // 单栏无对面栏概念，快览关闭。
            self.quickview_open = false;
            self.layout = PanelLayout::Single {
                preview_open: self.saved_preview_open,
            };
        } else {
            self.saved_ratio = ratio;
            self.layout = PanelLayout::Dual { ratio };
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        callbacks: FmCallbacks<'_>,
        confirm_delete: bool,
        show_hidden: bool,
    ) {
        self.confirm_delete = confirm_delete;
        // show_hidden 权威在 settings（同 confirm_delete），每帧下发；
        // 休眠期间设置页改动在下次进入时经此同步，rows_cache 按
        // RowsKey 自动失效，无需重新 read_dir。
        for panel in &mut self.panels {
            panel.show_hidden = show_hidden;
            // FS watch 回调唤醒 UI 用（首帧注入，后续 no-op）。
            panel.set_wake_ctx(ui.ctx().clone());
        }
        let FmCallbacks {
            on_back,
            on_open_path,
            on_open_archive,
            on_open_as_comic,
            on_extract,
            on_op_error,
            on_confirm_delete_change,
        } = callbacks;
        let mut intents = FmIntents::default();

        // 每帧排空两栏的列举结果；Loading 期间主动重绘（egui 空闲不重绘）。
        let mut loading = false;
        let mut drop_preview = false;
        for panel in &mut self.panels {
            let was_loading = matches!(panel.state, PanelLoadState::Loading(_));
            loading |= panel.poll();
            // 目录大小计算在途：主动重绘排空结果（egui 空闲不重绘）。
            loading |= panel.dir_sizes_in_flight();
            // FS watch 事件待去抖：主动重绘推进去抖窗口直到触发 refresh。
            loading |= panel.watch_refresh_pending();
            // 列举完成（navigate/refresh）：预览目标属于该栏且已消失则清预览。
            if was_loading && !matches!(panel.state, PanelLoadState::Loading(_)) {
                if let Some(target) = &self.preview_path {
                    if target.parent() == Some(panel.dir.as_path())
                        && !panel.entries.iter().any(|e| &e.path == target)
                    {
                        drop_preview = true;
                    }
                }
            }
        }
        if drop_preview {
            self.clear_preview();
        }
        if loading {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        self.poll_preview(ui.ctx());
        // 文件操作：每帧排空进度/完成事件；活动任务期间主动重绘。
        let op_summary = self.ops.poll();
        if op_summary.has_active {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        for finished in op_summary.finished {
            self.on_op_finished(finished, &mut intents);
        }
        let active_op = op_summary.active;
        // 速度/ETA 采样（任务切换重置采样器；暂停期间 done 不变，
        // EMA 自然衰减归零、速率显示消失）。采样间隔由 meter 内部节流。
        match (&active_op, &mut self.op_speed) {
            (Some(op), Some((id, meter))) if *id == op.id => {
                meter.sample(Instant::now(), op.progress.done_bytes);
            }
            (Some(op), _) => {
                let mut meter = OpSpeedMeter::new();
                meter.sample(Instant::now(), op.progress.done_bytes);
                self.op_speed = Some((op.id, meter));
            }
            (None, _) => self.op_speed = None,
        }

        self.render_top_bar(ui, &mut intents);
        ui.separator();

        // 底栏：当前栏选中/条目统计 + 操作进度。
        let mut cancel_op: Option<u64> = None;
        let mut toggle_pause: Option<(u64, bool)> = None;
        egui::Panel::bottom("fm_status_bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let panel = &self.panels[self.active];
                let total = panel.entries.len();
                let selected = panel.selected.len();
                ui.label(format!("共 {total} 项"));
                if selected > 0 {
                    // 选中集总大小：文件直接求和；目录仅计入「计算大小」已缓存
                    // 的值，未命中不触发计算（0 字节选中集不显示，避免误导）。
                    let total_size: u64 = panel
                        .entries
                        .iter()
                        .filter(|e| panel.selected.contains(&e.path))
                        .map(|e| {
                            if e.is_dir {
                                panel.dir_sizes.get(&e.path).copied().unwrap_or(0)
                            } else {
                                e.size.unwrap_or(0)
                            }
                        })
                        .sum();
                    ui.separator();
                    let label = if total_size > 0 {
                        format!("已选 {selected} 项 · 合计 {}", human_size(total_size))
                    } else {
                        format!("已选 {selected} 项")
                    };
                    ui.label(label);
                }
                // type-ahead 缓冲（type-to-select）：弱色显示，到期自动消失
                // （主动重绘推进过期判定）。
                if let Some(buf) = panel.type_ahead_buffer() {
                    ui.separator();
                    ui.label(egui::RichText::new(format!("定位: {buf}")).weak());
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                // 操作进行中提示文本让位（状态栏宽度有限）。
                if active_op.is_none() {
                    ui.separator();
                    ui.label(egui::RichText::new("Tab 切换栏 · 双击打开 · 右键菜单").weak());
                }
                // 活动文件操作：总进度条 + 当前文件进度 + 速度/ETA +
                // 暂停/继续 + 取消（进度区加宽，提示文本相应让位）。
                if let Some(op) = &active_op {
                    ui.separator();
                    let fraction = op.progress.fraction();
                    let pct = (fraction * 100.0).round() as u32;
                    let bar_text = if op.paused {
                        format!("{} {pct}%（已暂停）", op.kind.verb())
                    } else {
                        format!("{} {pct}%", op.kind.verb())
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(200.0)
                            .text(bar_text),
                    );
                    // 当前文件内进度（分块复制维护；cur_total=0 不显示）。
                    if op.progress.cur_total_bytes > 0 {
                        let cur_pct = (op.progress.cur_done_bytes as f64
                            / op.progress.cur_total_bytes as f64
                            * 100.0)
                            .round() as u32;
                        ui.label(egui::RichText::new(format!("当前文件 {cur_pct}%")).weak());
                    }
                    // 速度 / ETA（EMA 平滑；速度 0 或 ETA<2s 不显示剩余时间）。
                    if let Some((_, meter)) = &self.op_speed {
                        if let Some(bps) = meter.speed_bps() {
                            if bps > 0.0 {
                                let mut text = format!("{}/s", human_size(bps as u64));
                                if let Some(eta) =
                                    meter.eta_secs(op.progress.total_bytes, op.progress.done_bytes)
                                {
                                    if eta >= 2.0 {
                                        text.push_str(&format!(" · 剩余 {}", format_eta(eta)));
                                    }
                                }
                                ui.label(egui::RichText::new(text).weak());
                            }
                        }
                    }
                    let current = op
                        .progress
                        .current
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !current.is_empty() {
                        ui.label(egui::RichText::new(current).weak())
                            .on_hover_text(op.progress.current.display().to_string());
                    }
                    // 暂停/继续（压缩不支持暂停，禁用并说明）。
                    let pause_label = if op.paused { "继续" } else { "暂停" };
                    if ui
                        .add_enabled(
                            op.kind != OpKind::Compress,
                            egui::Button::new(pause_label).small(),
                        )
                        .on_hover_text(if op.kind == OpKind::Compress {
                            "压缩不支持暂停"
                        } else {
                            ""
                        })
                        .clicked()
                    {
                        toggle_pause = Some((op.id, !op.paused));
                    }
                    if ui.small_button("取消").clicked() {
                        cancel_op = Some(op.id);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(panel.dir.display().to_string()).weak());
                });
            });
            ui.add_space(4.0);
        });
        if let Some(id) = cancel_op {
            self.ops.cancel(id);
        }
        if let Some((id, paused)) = toggle_pause {
            self.ops.set_paused(id, paused);
        }

        // 中央：双栏 + 可拖分隔条 / 单栏 + 预览占位。
        let panel_rects = self.render_panels(ui, &mut intents);

        // 鼠标点击某栏任意处即激活该栏（快览面板不产生栏切换：
        // 快览恒停在「对面」，点击它激活会把两栏语义搞乱）。
        let (pressed, pos) = ui.ctx().input(|i| {
            (
                i.pointer.primary_pressed() || i.pointer.secondary_pressed(),
                i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
            )
        });
        if pressed {
            if let Some(pos) = pos {
                for (idx, rect) in &panel_rects {
                    if self.quickview_open && *idx != self.active {
                        continue;
                    }
                    if rect.contains(pos) {
                        self.active = *idx;
                    }
                }
            }
        }

        // 栏间拖放复制：悬停高亮落点栏 + 松开弹「复制到…」确认框 + 拖动徽标。
        self.poll_inter_panel_dnd(ui, &panel_rects);

        self.handle_keyboard(ui, &mut intents, confirm_delete);
        // 「选中即预览」跟随焦点行（键盘/鼠标改动焦点之后统一同步）。
        self.sync_preview_target();
        // F3 临时预览弹窗（双栏模式）。
        self.render_preview_window(ui.ctx());
        // 文件操作确认对话框（复制/移动/删除/重命名/新建文件夹）。
        self.render_dialog(ui.ctx(), &mut intents);

        // 帧尾统一外抛回调。
        if intents.back {
            on_back();
        }
        if let Some(path) = intents.open_archive {
            on_open_archive(path);
        }
        if let Some(path) = intents.open_path {
            on_open_path(path);
        }
        if let Some(path) = intents.open_as_comic {
            on_open_as_comic(path);
        }
        if let Some((archive, dest)) = intents.extract {
            on_extract(archive, dest);
        }
        if let Some(msg) = intents.op_error {
            on_op_error(msg);
        }
        if let Some(confirm) = intents.confirm_delete_change {
            on_confirm_delete_change(confirm);
        }
    }

    /// 顶栏：返回书架 / 单双栏切换（单栏时含预览开关）/ 右侧过滤框
    /// （过滤作用于焦点栏）。
    fn render_top_bar(&mut self, ui: &mut egui::Ui, intents: &mut FmIntents) {
        ui.horizontal(|ui| {
            if ui
                .button((icons::HOUSE, " 书架"))
                .on_hover_text("返回书架")
                .clicked()
            {
                intents.back = true;
            }
            ui.separator();
            let dual = matches!(self.layout, PanelLayout::Dual { .. });
            if ui
                .add(egui::Button::new((icons::COLUMNS, " 双栏")).selected(dual))
                .on_hover_text("双栏模式")
                .clicked()
                && !dual
            {
                self.layout = PanelLayout::Dual {
                    ratio: self.saved_ratio,
                };
            }
            if ui
                .add(egui::Button::new((icons::SQUARE_HALF, " 单栏")).selected(!dual))
                .on_hover_text("单栏模式（右侧为预览面板）")
                .clicked()
                && dual
            {
                if let PanelLayout::Dual { ratio } = self.layout {
                    self.saved_ratio = ratio;
                }
                // 保留焦点栏内容到左栏（单栏只渲染 panels[0]）。
                if self.active == 1 {
                    self.panels.swap(0, 1);
                }
                self.active = 0;
                self.preview_window_open = false;
                // 单栏无对面栏概念，快览关闭。
                self.quickview_open = false;
                self.layout = PanelLayout::Single {
                    preview_open: self.saved_preview_open,
                };
            }
            if matches!(self.layout, PanelLayout::Dual { .. })
                && ui
                    .add(egui::Button::new((icons::EYE, " 快览")).selected(self.quickview_open))
                    .on_hover_text("对面栏快速预览（Ctrl+Q）")
                    .clicked()
            {
                self.quickview_open = !self.quickview_open;
                if !self.quickview_open {
                    // 关闭即清预览目标，避免后台继续读取。
                    self.clear_preview();
                }
            }
            if let PanelLayout::Single { preview_open } = &mut self.layout {
                ui.separator();
                if ui
                    .add(egui::Button::new((icons::EYE, " 预览")).selected(*preview_open))
                    .on_hover_text("切换预览面板")
                    .clicked()
                {
                    *preview_open = !*preview_open;
                    self.saved_preview_open = *preview_open;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.panels[self.active].filter)
                        .hint_text("过滤（焦点栏）")
                        .desired_width(160.0),
                );
            });
        });
    }

    /// 中央面板区；返回各栏的 rect（点击激活用）。
    fn render_panels(
        &mut self,
        ui: &mut egui::Ui,
        intents: &mut FmIntents,
    ) -> Vec<(usize, egui::Rect)> {
        let total_w = ui.available_width();
        let height = ui.available_height();
        match self.layout {
            PanelLayout::Dual { ratio } => {
                let mut ratio = ratio;
                let left_w = ((total_w - SPLITTER_WIDTH) * ratio).clamp(
                    PANEL_MIN_WIDTH,
                    (total_w - SPLITTER_WIDTH - PANEL_MIN_WIDTH).max(PANEL_MIN_WIDTH),
                );
                let mut rects = Vec::with_capacity(2);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let left = ui.allocate_ui_with_layout(
                        egui::vec2(left_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        // push_id 按栏隔离 widget Id 子树：否则两栏同位置控件的
                        // auto-id 相同（ScrollArea 滚动状态、列宽拖拽、列头点击
                        // 的持久状态被跨栏共享，滚左栏右栏跟着动）。
                        |ui| {
                            ui.push_id(("fm_panel", 0), |ui| {
                                self.render_panel_or_quickview(ui, 0, intents)
                            });
                        },
                    );
                    rects.push((0, left.response.rect));
                    // total_w 为 0（窗口被压扁的退化帧）时除法产生 NaN，
                    // NaN ratio 会传染进后续布局计算。
                    if let Some(delta) = render_splitter(ui, height) {
                        if total_w > 0.0 {
                            ratio = (ratio + delta / total_w).clamp(RATIO_MIN, RATIO_MAX);
                        }
                    }
                    let right_w = ui.available_width();
                    let right = ui.allocate_ui_with_layout(
                        egui::vec2(right_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.push_id(("fm_panel", 1), |ui| {
                                self.render_panel_or_quickview(ui, 1, intents)
                            });
                        },
                    );
                    rects.push((1, right.response.rect));
                });
                self.saved_ratio = ratio;
                self.layout = PanelLayout::Dual { ratio };
                rects
            }
            PanelLayout::Single { preview_open } => {
                let mut rects = Vec::with_capacity(1);
                if preview_open {
                    let mut preview_ratio = self.preview_ratio;
                    let preview_w = (total_w - SPLITTER_WIDTH) * preview_ratio;
                    let panel_w = (total_w - SPLITTER_WIDTH - preview_w).max(PANEL_MIN_WIDTH);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let panel = ui.allocate_ui_with_layout(
                            egui::vec2(panel_w, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.push_id(("fm_panel", 0), |ui| self.render_panel(ui, 0, intents));
                            },
                        );
                        rects.push((0, panel.response.rect));
                        if let Some(delta) = render_splitter(ui, height) {
                            if total_w > 0.0 {
                                preview_ratio =
                                    (preview_ratio - delta / total_w).clamp(RATIO_MIN, 0.6);
                            }
                        }
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.draw_preview_content(ui),
                        );
                    });
                    self.preview_ratio = preview_ratio;
                } else {
                    let panel = ui.allocate_ui_with_layout(
                        egui::vec2(total_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.push_id(("fm_panel", 0), |ui| self.render_panel(ui, 0, intents));
                        },
                    );
                    rects.push((0, panel.response.rect));
                }
                rects
            }
        }
    }

    /// 双栏一侧内容：Ctrl+Q 快览开启且本侧为非活动栏时整栏替换为预览面板
    /// （被替换栏对象不列举不渲染，state/selected/滚动原样保留，关闭快览
    /// 后原样恢复）；否则渲染常规栏。
    fn render_panel_or_quickview(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        intents: &mut FmIntents,
    ) {
        if self.quickview_open && self.active != idx {
            self.draw_preview_content(ui);
        } else {
            self.render_panel(ui, idx, intents);
        }
    }

    /// 单栏内容：面包屑 → 状态（加载/失败）→ 列头 → 虚拟化明细列表。
    fn render_panel(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        self.render_breadcrumb(ui, idx);
        // 先抽出状态快照，避免 match 借用与臂内 &mut self 冲突。
        enum Phase {
            Idle,
            Loading,
            Failed(String),
            Ready,
        }
        let phase = match &self.panels[idx].state {
            PanelLoadState::Idle => Phase::Idle,
            PanelLoadState::Loading(_) => Phase::Loading,
            PanelLoadState::Failed(e) => Phase::Failed(e.clone()),
            PanelLoadState::Ready => Phase::Ready,
        };
        match phase {
            Phase::Idle => {
                ui.label(egui::RichText::new("正在初始化…").weak());
            }
            Phase::Loading => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new("正在读取目录…").weak());
                });
            }
            Phase::Failed(message) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new(icons::WARNING.as_str()).size(24.0));
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, message);
                    if ui.button((icons::ARROW_CLOCKWISE, " 重试")).clicked() {
                        self.panels[idx].refresh();
                    }
                });
            }
            Phase::Ready => {
                self.render_column_header(ui, idx);
                self.render_list(ui, idx, intents);
            }
        }
    }

    /// 面包屑：盘符下拉（最左）+ 路径分段可点击跳回；焦点栏铺淡底色高亮。
    fn render_breadcrumb(&mut self, ui: &mut egui::Ui, idx: usize) {
        let active = self.active == idx;
        let fill = if active {
            ui.visuals().faint_bg_color
        } else {
            egui::Color32::TRANSPARENT
        };
        egui::Frame::new()
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(4, 2))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let back_tip = self.panels[idx]
                        .back_target()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "后退".to_string());
                    if ui
                        .add_enabled(
                            self.panels[idx].can_go_back(),
                            egui::Button::new(icons::CARET_LEFT.as_str()).frame(false),
                        )
                        .on_hover_text(back_tip)
                        .clicked()
                    {
                        self.panels[idx].go_back();
                    }
                    let forward_tip = self.panels[idx]
                        .forward_target()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "前进".to_string());
                    if ui
                        .add_enabled(
                            self.panels[idx].can_go_forward(),
                            egui::Button::new(icons::CARET_RIGHT.as_str()).frame(false),
                        )
                        .on_hover_text(forward_tip)
                        .clicked()
                    {
                        self.panels[idx].go_forward();
                    }
                    self.render_history_button(ui, idx);
                    self.render_drive_switcher(ui, idx);
                    self.render_bookmarks_button(ui, idx);
                    ui.separator();
                    let dir = self.panels[idx].dir.clone();
                    let segments = breadcrumb_segments(&dir);
                    if segments.is_empty() {
                        ui.label(egui::RichText::new("…").weak());
                        return;
                    }
                    let last = segments.len() - 1;
                    let mut jump: Option<PathBuf> = None;
                    for (i, (path, label)) in segments.iter().enumerate() {
                        if i > 0 {
                            ui.label(egui::RichText::new("›").weak());
                        }
                        let text = if i == last {
                            egui::RichText::new(label).strong()
                        } else {
                            egui::RichText::new(label)
                        };
                        if ui
                            .selectable_label(i == last, text)
                            .on_hover_text(path.display().to_string())
                            .clicked()
                            && i != last
                        {
                            jump = Some(path.clone());
                        }
                    }
                    if let Some(path) = jump {
                        self.panels[idx].navigate_to(path);
                    }
                });
            });
    }

    /// 历史下拉按钮（‹ › 旁）：菜单列出目录历史（新→旧，当前项打勾 ✓，
    /// 悬停显示完整路径），点击直跳；Alt+↓ 经 history_menu_toggle 一次性
    /// 请求切换焦点栏菜单的开/关（与点击共用同一 memory 弹层状态）。
    fn render_history_button(&mut self, ui: &mut egui::Ui, idx: usize) {
        let response = ui
            .add(egui::Button::new(icons::CLOCK_COUNTER_CLOCKWISE.as_str()).frame(false))
            .on_hover_text("目录历史（Alt+↓）");
        let kb_toggle = self.history_menu_toggle && self.active == idx;
        if kb_toggle {
            self.history_menu_toggle = false;
        }
        let set = (response.clicked() || kb_toggle).then_some(egui::SetOpenCommand::Toggle);
        egui::Popup::menu(&response)
            .id(egui::Id::new(("fm_history_menu", idx)))
            .open_memory(set)
            .show(|ui| {
                let history = self.panels[idx].history_list();
                if history.is_empty() {
                    ui.label(egui::RichText::new("（无历史）").weak());
                    return;
                }
                ui.set_min_width(320.0);
                let mut jump: Option<usize> = None;
                let count = history.len();
                for (i, (path, is_current)) in history.iter().enumerate() {
                    let display = path.display().to_string();
                    let text = if *is_current {
                        format!("✓ {display}")
                    } else {
                        display.clone()
                    };
                    if ui
                        .selectable_label(*is_current, text)
                        .on_hover_text(&display)
                        .clicked()
                    {
                        // 菜单序 i（新→旧）→ 历史 pos = count - i（1 起）。
                        jump = Some(count - i);
                        ui.close();
                    }
                }
                if let Some(pos) = jump {
                    self.panels[idx].navigate_history_to(pos);
                }
            });
    }

    /// 面包屑最左的盘符下拉：显示该栏当前盘符（如 `C:\`），点开列出可用
    /// 盘符/卷，点击后该栏 navigate_to 盘符根（两栏各自独立）。盘符枚举
    /// 首次打开时后台线程执行（慢速/断开设备不卡 UI），结果缓存于视图。
    fn render_drive_switcher(&mut self, ui: &mut egui::Ui, idx: usize) {
        // 每帧尝试接收后台枚举结果；在途时主动重绘直到就绪。
        if let Some(rx) = &self.drives_rx {
            if let Ok(list) = rx.try_recv() {
                self.drives = Some(list);
                self.drives_rx = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        }
        let label = format!(
            "{} {}",
            icons::HARD_DRIVES.as_str(),
            drive_label(&self.panels[idx].dir)
        );
        ui.menu_button(label, |ui| {
            if self.drives.is_none() {
                // 首次打开：起后台线程枚举（仅一次，结果缓存）。
                if self.drives_rx.is_none() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(list_drives());
                    });
                    self.drives_rx = Some(rx);
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                ui.label(egui::RichText::new("正在扫描盘符…").weak());
                return;
            }
            let drives = self.drives.clone().unwrap_or_default();
            let current = self.panels[idx].dir.clone();
            for d in drives {
                let is_current = current.starts_with(&d) && d.as_os_str() != "/";
                if ui
                    .selectable_label(is_current, d.display().to_string())
                    .clicked()
                {
                    self.panels[idx].navigate_to(d);
                    ui.close();
                }
            }
        });
    }

    /// 面包屑上的书签菜单（两栏共享一份）：「添加当前目录」+ 书签列表
    /// （点击跳转，目录不存在逐级回退最近存在祖先；✕ 移除，菜单不收起）。
    fn render_bookmarks_button(&mut self, ui: &mut egui::Ui, idx: usize) {
        let button = egui::Button::new(icons::STAR.as_str()).frame(false);
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        egui::containers::menu::MenuButton::from_button(button)
            .config(config)
            .ui(ui, |ui| {
                ui.set_min_width(280.0);
                let dir = self.panels[idx].dir.clone();
                let already = self.bookmarks.iter().any(|b| b == &dir);
                if ui
                    .add_enabled(!already, egui::Button::new("添加当前目录"))
                    .clicked()
                {
                    self.add_bookmark(&dir);
                    ui.close();
                }
                ui.separator();
                if self.bookmarks.is_empty() {
                    ui.label(egui::RichText::new("（无书签）").weak());
                    return;
                }
                let mut jump: Option<PathBuf> = None;
                let mut remove: Option<PathBuf> = None;
                for bm in &self.bookmarks {
                    let tip = bm.display().to_string();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(icons::X.as_str()).frame(false).small())
                            .on_hover_text("移除书签")
                            .clicked()
                        {
                            remove = Some(bm.clone());
                        }
                        if ui
                            .add(
                                egui::Label::new(tip.as_str())
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(&tip)
                            .clicked()
                        {
                            jump = Some(bm.clone());
                        }
                    });
                }
                if let Some(bm) = remove {
                    self.remove_bookmark(&bm);
                }
                if let Some(bm) = jump {
                    let target = fallback_existing_dir(bm);
                    self.panels[idx].navigate_to(target);
                    ui.close();
                }
            })
            .0
            .on_hover_text("常用目录书签");
    }

    /// 列头：名称 / 大小 / 修改时间，整列格可点击切换排序键与升降序，
    /// 当前键显示 ▲/▼。列坐标取自 column_layout（与行内容/竖线同一来源）。
    /// 两条分隔竖线各带 6pt 拖拽热区（后注册于列点击格，拖拽优先）。
    fn render_column_header(&mut self, ui: &mut egui::Ui, idx: usize) {
        let (header_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::hover(),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(
            header_rect,
            0.0,
            ui.visuals().widgets.noninteractive.bg_fill,
        );
        let panel = &mut self.panels[idx];
        let layout = column_layout(
            header_rect.right(),
            panel.col_shift,
            panel.col_width_size,
            panel.col_width_mtime,
        );
        let cy = header_rect.center().y;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let name_rect = egui::Rect::from_min_max(
            header_rect.min,
            egui::pos2(layout.size_left, header_rect.max.y),
        );
        let size_rect = egui::Rect::from_min_max(
            egui::pos2(layout.size_left, header_rect.min.y),
            egui::pos2(layout.mtime_left, header_rect.max.y),
        );
        let mtime_rect = egui::Rect::from_min_max(
            egui::pos2(layout.mtime_left, header_rect.min.y),
            header_rect.max,
        );
        // (列 rect, 排序键, 标题, 文字锚点 x, 对齐方式)
        // 扩展名排序时名称列头显示「扩展名」（扩展名没有独立列，TC 同款约定）。
        let name_label = if panel.sort_key == SortKey::Ext {
            "扩展名"
        } else {
            "名称"
        };
        let cols: [(egui::Rect, SortKey, &str, f32, egui::Align2); 3] = [
            (
                name_rect,
                SortKey::Name,
                name_label,
                header_rect.left() + NAME_HEADER_INDENT,
                egui::Align2::LEFT_CENTER,
            ),
            (
                size_rect,
                SortKey::Size,
                "大小",
                layout.mtime_left,
                egui::Align2::RIGHT_CENTER,
            ),
            (
                mtime_rect,
                SortKey::Mtime,
                "修改时间",
                layout.content_right,
                egui::Align2::RIGHT_CENTER,
            ),
        ];
        let mut clicked: Option<SortKey> = None;
        let mut texts: Vec<(egui::Pos2, egui::Align2, String)> = Vec::with_capacity(3);
        for (rect, key, label, anchor_x, align) in cols {
            let response = ui.interact(
                rect,
                ui.id().with(("fm-header", label)),
                egui::Sense::click(),
            );
            if response.clicked() {
                clicked = Some(key);
            }
            // 名称列头右键：排序键/升降序菜单（整格左击逻辑不变）。
            if key == SortKey::Name {
                response.context_menu(|ui| {
                    for (k, menu_label) in [
                        (SortKey::Name, "名称"),
                        (SortKey::Ext, "扩展名"),
                        (SortKey::Size, "大小"),
                        (SortKey::Mtime, "修改时间"),
                    ] {
                        if ui
                            .selectable_label(panel.sort_key == k, menu_label)
                            .clicked()
                        {
                            panel.toggle_sort(k);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(panel.sort_asc, "升序").clicked() {
                        panel.sort_asc = true;
                        ui.close();
                    }
                    if ui.selectable_label(!panel.sort_asc, "降序").clicked() {
                        panel.sort_asc = false;
                        ui.close();
                    }
                });
            }
            if response.hovered() {
                painter.rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
            }
            // 扩展名排序的箭头画在名称列（扩展名借用名称列头，见 name_label）。
            let arrow = if panel.sort_key == key
                || (key == SortKey::Name && panel.sort_key == SortKey::Ext)
            {
                if panel.sort_asc {
                    " ▲"
                } else {
                    " ▼"
                }
            } else {
                ""
            };
            texts.push((egui::pos2(anchor_x, cy), align, format!("{label}{arrow}")));
        }
        // 列宽拖拽热区：两条分隔竖线各 ±3pt，后注册于列点击格使拖拽优先。
        let sep_xs = [layout.size_left, layout.mtime_left];
        let mut hovered_sep: Option<usize> = None;
        for (i, &x) in sep_xs.iter().enumerate() {
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(x - 3.0, header_rect.top()),
                egui::pos2(x + 3.0, header_rect.bottom()),
            );
            let response = ui.interact(
                drag_rect,
                ui.id().with(("fm-col-sep", i)),
                egui::Sense::drag(),
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                hovered_sep = Some(i);
            }
            if response.dragged() {
                let dx = response.drag_motion().x;
                if dx != 0.0 {
                    panel.drag_column_sep(i, dx, header_rect);
                }
            }
        }
        let line_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        paint_column_separators(&painter, header_rect, line_color, &layout);
        if let Some(i) = hovered_sep {
            let strong = ui.visuals().widgets.active.bg_stroke.color;
            painter.vline(
                sep_xs[i],
                header_rect.y_range(),
                egui::Stroke::new(1.0, strong),
            );
        }
        painter.hline(
            header_rect.x_range(),
            header_rect.bottom(),
            egui::Stroke::new(1.0, line_color),
        );
        for (pos, align, text) in texts {
            painter.text(pos, align, text, font_id.clone(), ui.visuals().text_color());
        }
        if let Some(key) = clicked {
            panel.toggle_sort(key);
        }
    }

    /// 明细列表：行 0 = 「..」上级行（恒居行首，根目录禁用），其余来自
    /// `FsPanel::rows()`（目录优先 + 排序 + 过滤）。固定行高 + show_rows 虚拟化；
    /// 键盘移动焦点按 Explorer 最小滚动语义揭示。
    fn render_list(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        let rows = self.panels[idx].rows();
        let row_count = rows.len() + 1;
        // 行间不留缝（Explorer/WinRAR 式紧密列表）：item_spacing.y 归零后
        // 行距 = ROW_HEIGHT。注意 show_rows 在调用时捕获本 ui 的 item_spacing
        // 计算 pitch，行内 advance_cursor_after_rect 也读同一值，三处天然一致。
        ui.spacing_mut().item_spacing.y = 0.0;
        let row_pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let panel = &mut self.panels[idx];
        panel.last_row_pitch = row_pitch;
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        if panel.focus_scroll_pending {
            panel.focus_scroll_pending = false;
            if panel.last_viewport_height > 0.0 {
                if let Some(focus) = panel.focus {
                    if focus < row_count {
                        let new_offset = min_scroll_to_reveal(
                            panel.last_scroll_offset,
                            panel.last_viewport_height,
                            focus as f32 * row_pitch,
                        );
                        if (new_offset - panel.last_scroll_offset).abs() > 0.01 {
                            area = area.vertical_scroll_offset(new_offset);
                        }
                    }
                }
            }
        }
        let active = self.active == idx;
        let output = area.show_rows(ui, ROW_HEIGHT, row_count, |ui, range| {
            for row in range {
                // 行内交互（双击目录/「..」、右键「打开」）可触发 navigate_to /
                // refresh：entries 当场清空、rows 快照即刻失效，继续按旧行索引
                // 渲染剩余行会越界 panic（双击目录闪退的根因）。状态离开 Ready
                // 就停笔，下一帧用新行模型整帧重画。
                if !matches!(self.panels[idx].state, PanelLoadState::Ready) {
                    break;
                }
                self.render_row(ui, idx, &rows, row, active, intents);
            }
        });
        let panel = &mut self.panels[idx];
        panel.last_scroll_offset = output.state.offset.y;
        panel.last_viewport_height = output.inner_rect.height();
    }

    /// 明细列表一行：整行 allocate 交互 + 斑马纹/高亮/焦点描边 + 列分隔竖线，
    /// 名称列截断、大小/时间列右对齐固定宽。焦点描边仅焦点栏强显示。
    fn render_row(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        rows: &[usize],
        row: usize,
        active: bool,
        intents: &mut FmIntents,
    ) {
        let panel = &mut self.panels[idx];
        let is_parent = row == 0;
        // rows 可能是导航前的旧快照（同帧行内双击已清空 entries）：用 get
        // 防御，越界行本帧按「..」样式渲染，下一帧即被新行模型替换。
        let entry: Option<FsEntry> = if is_parent {
            None
        } else {
            rows.get(row - 1)
                .and_then(|&i| panel.entries.get(i).cloned())
        };
        let selected = entry
            .as_ref()
            .is_some_and(|e| panel.selected.contains(&e.path));
        let dir_size = entry
            .as_ref()
            .filter(|e| e.is_dir)
            .and_then(|e| panel.dir_sizes.get(&e.path).copied());
        let focused = panel.focus == Some(row);
        let at_root = panel.is_root();
        let parent_enabled = !at_root;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        // 斑马纹（全局行号，滚动时条纹不闪动）先铺底，再叠加选中/悬停高亮。
        if row.is_multiple_of(2) {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        }
        if selected {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        if focused {
            // 焦点行描边：活动栏强色，非活动栏弱化（对齐 Archive 视图焦点语义）。
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1.0, ui.visuals().weak_text_color())
            };
            ui.painter()
                .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
        }
        let layout = column_layout(
            rect.right(),
            panel.col_shift,
            panel.col_width_size,
            panel.col_width_mtime,
        );
        paint_column_separators(
            ui.painter(),
            rect,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
            &layout,
        );

        // 行内容：图标 + 名称；右侧固定宽的大小/修改时间列
        // （目录行大小列仅在「计算大小」已算出时显示，未命中留空）。
        let content = rect.shrink2(egui::vec2(6.0, 2.0));
        // 名称列在列块左缘前截断（跟随 col_shift），不与大小列文字叠字。
        let name_right = (layout.size_left - 6.0).max(content.min.x + 20.0);
        let name_rect =
            egui::Rect::from_min_max(content.min, egui::pos2(name_right, content.max.y));
        ui.scope_builder(egui::UiBuilder::new().max_rect(name_rect), |ui| {
            // ui.horizontal 的初始行高取 interact_size.y（全局 28pt，为工具栏
            // 按钮而设），不压回会撑爆 18pt 的内容区：文字随之下沉约 5pt 贴到
            // 条纹下缘，且 scope 结束时列表竖向光标被多推 8pt，实际行距与
            // show_rows 假定的 22pt 逐行漂移。
            ui.spacing_mut().interact_size.y = ROW_HEIGHT - 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match &entry {
                    None => {
                        let text_color = if parent_enabled {
                            ui.visuals().weak_text_color()
                        } else {
                            ui.visuals().widgets.noninteractive.fg_stroke.color
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(icons::ARROW_UP.as_str()).color(text_color),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            egui::Label::new(egui::RichText::new("..").color(text_color))
                                .selectable(false),
                        );
                    }
                    Some(e) => {
                        ui.add(
                            egui::Label::new(egui::RichText::new(entry_icon(e).as_str()).weak())
                                .selectable(false),
                        );
                        ui.add(egui::Label::new(&e.name).truncate().selectable(false));
                    }
                }
                // 右对齐列：与表头/竖线共用 column_layout 锚点直接绘制，天然跟随
                // col_shift（拖分隔线时一起动）且无间距误差。不走 right_to_left
                // 布局——egui 0.35 RTL 嵌套会把文字画到格子右缘之外且不读 col_shift。
                if let Some(e) = &entry {
                    let painter = ui.painter();
                    let font_id = egui::TextStyle::Body.resolve(ui.style());
                    let col = ui.visuals().weak_text_color();
                    let cy = rect.center().y;
                    if !e.is_dir {
                        if let Some(size) = e.size {
                            painter.text(
                                egui::pos2(layout.mtime_left, cy),
                                egui::Align2::RIGHT_CENTER,
                                human_size(size),
                                font_id.clone(),
                                col,
                            );
                        }
                    } else if let Some(size) = dir_size {
                        // 目录大小比文件大小再弱一档，与精确文件大小区分。
                        painter.text(
                            egui::pos2(layout.mtime_left, cy),
                            egui::Align2::RIGHT_CENTER,
                            human_size(size),
                            font_id.clone(),
                            col.gamma_multiply(0.6),
                        );
                    }
                    painter.text(
                        egui::pos2(layout.content_right, cy),
                        egui::Align2::RIGHT_CENTER,
                        format_mtime(system_time_to_unix(e.mtime)),
                        font_id,
                        col,
                    );
                }
            });
        });
        // scope_dyn 结束时会用「内容区底 + item_spacing」改写列表竖向光标
        // （内容区比行高矮 4pt，会回退光标、行距偏离 show_rows 的假定），
        // 钉回「行底 + item_spacing」。
        ui.advance_cursor_after_rect(rect);

        // 行 = 栏间拖放的 drag source（「..」上级行不可拖）：拖动开始即设置
        // payload，落点栏判定与 drop 生效在 poll_inter_panel_dnd。
        // 拖拽与单击互斥由 egui 保证（拖动超过阈值后不产生 clicked）。
        if let Some(e) = &entry {
            let sources = drag_sources(&self.panels[idx].selected, &e.path);
            response.dnd_set_drag_payload(FmDragPayload {
                sources,
                src_panel: idx,
            });
        }

        let mods = ui.input(|i| i.modifiers);
        if response.clicked() {
            self.panels[idx].click_row(row, mods.command, mods.shift);
        }
        if response.double_clicked() {
            if is_parent {
                if parent_enabled {
                    self.panels[idx].parent_dir();
                }
            } else {
                self.open_ui_row(idx, rows, row, intents);
            }
        }
        // 「..」行无右键菜单。
        if !is_parent {
            response.context_menu(|ui| {
                // Explorer 惯例：右键未选中的行先把它单选。
                let Some(e) = &entry else { return };
                if !self.panels[idx].selected.contains(&e.path) {
                    self.panels[idx].click_row(row, false, false);
                }
                if ui.button((icons::ARROW_SQUARE_OUT, " 打开")).clicked() {
                    self.open_ui_row(idx, rows, row, intents);
                    ui.close();
                }
                let comic_openable = e.is_dir || archive_kind(&e.path).is_some();
                if comic_openable && ui.button((icons::BOOK_OPEN, " 作为漫画打开")).clicked()
                {
                    intents.open_as_comic = Some(e.path.clone());
                    ui.close();
                }
                let targets = self.op_targets(idx);
                let extract_src = match targets.as_slice() {
                    [p] if archive_kind(p).is_some() => Some(p.clone()),
                    _ => None,
                };
                let other_dir = match self.layout {
                    PanelLayout::Dual { .. } => Some(self.panels[1 - idx].dir.clone()),
                    PanelLayout::Single { .. } => None,
                };
                let (extract_label, extract_dest) = match other_dir {
                    Some(d) if d != self.panels[idx].dir => ("另一栏", d),
                    _ => ("当前目录", self.panels[idx].dir.clone()),
                };
                if ui
                    .add_enabled(
                        extract_src.is_some(),
                        egui::Button::new((icons::EXPORT, format!(" 解压到{extract_label}…"))),
                    )
                    .clicked()
                {
                    if let Some(src) = extract_src {
                        intents.extract = Some((src, extract_dest));
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button((icons::COPY, " 复制路径")).clicked() {
                    ui.ctx().copy_text(e.path.display().to_string());
                    ui.close();
                }
                if ui.button((icons::ARROW_CLOCKWISE, " 刷新")).clicked() {
                    self.panels[idx].refresh();
                    ui.close();
                }
                let dir_targets: Vec<PathBuf> = self.panels[idx]
                    .entries
                    .iter()
                    .filter(|e| e.is_dir && targets.contains(&e.path))
                    .map(|e| e.path.clone())
                    .collect();
                if ui
                    .add_enabled(
                        !dir_targets.is_empty(),
                        egui::Button::new((icons::GAUGE, " 计算大小")),
                    )
                    .clicked()
                {
                    self.panels[idx].request_dir_sizes(dir_targets);
                    ui.close();
                }
                ui.separator();
                if ui.button((icons::PENCIL_SIMPLE, " 重命名")).clicked() {
                    self.dialog = Some(FmDialog::Rename(RenameDialog::new(e.path.clone())));
                    ui.close();
                }
                let dual = matches!(self.layout, PanelLayout::Dual { .. });
                let dest_label = if dual { "另一栏" } else { "当前目录" };
                if ui
                    .button((icons::COPY, format!(" 复制到{dest_label}…")))
                    .clicked()
                {
                    let targets = self.op_targets(idx);
                    if !targets.is_empty() {
                        self.open_copy_move_dialog(OpKind::Copy, targets, idx);
                    }
                    ui.close();
                }
                if ui
                    .button((icons::EXPORT, format!(" 移动到{dest_label}…")))
                    .clicked()
                {
                    let targets = self.op_targets(idx);
                    if !targets.is_empty() {
                        self.open_copy_move_dialog(OpKind::Move, targets, idx);
                    }
                    ui.close();
                }
                if ui.button((icons::PACKAGE, " 压缩为 zip…")).clicked() {
                    let targets = self.op_targets(idx);
                    if !targets.is_empty() {
                        self.open_compress_dialog(targets, idx);
                    }
                    ui.close();
                }
                if ui.button((icons::FOLDER_PLUS, " 新建文件夹")).clicked() {
                    let parent = self.panels[idx].dir.clone();
                    let suggested = suggest_folder_name(&parent);
                    self.dialog = Some(FmDialog::NewDir(NewDirDialog::new(parent, suggested)));
                    ui.close();
                }
                ui.separator();
                if ui.button((icons::TRASH, " 删除")).clicked() {
                    let targets = self.op_targets(idx);
                    if !targets.is_empty() {
                        if self.confirm_delete {
                            self.dialog = Some(FmDialog::Delete(DeleteDialog::new(targets)));
                        } else {
                            self.start_delete(targets);
                        }
                    }
                    ui.close();
                }
            });
        }
        // 悬停信息提示（被截断名称的完整信息）：全路径 + 大小；「..」= 上级目录提示。
        let tip = match &entry {
            None => "上级目录".to_string(),
            Some(e) => {
                let mut tip = e.path.display().to_string();
                if !e.is_dir {
                    if let Some(size) = e.size {
                        tip.push_str(&format!("\n大小: {}", human_size(size)));
                    }
                }
                tip
            }
        };
        response.on_hover_text(tip);
    }

    /// 打开一行的默认动作（双击/Enter/右键「打开」共用）：「..」= 上级；
    /// 目录 = 栏内进入；压缩包 = Archive 视图；其余 = open_path 分发。
    fn open_ui_row(&mut self, idx: usize, rows: &[usize], row: usize, intents: &mut FmIntents) {
        if row == 0 {
            self.panels[idx].parent_dir();
            return;
        }
        // rows 与 entries 之间存在失配窗口（同帧前面的行已触发导航），用 get 防御。
        let Some(entry) = rows
            .get(row - 1)
            .and_then(|&entry_idx| self.panels[idx].entries.get(entry_idx).cloned())
        else {
            return;
        };
        if entry.is_dir {
            self.panels[idx].navigate_to(entry.path.clone());
        } else if archive_kind(&entry.path).is_some() {
            intents.open_archive = Some(entry.path);
        } else {
            intents.open_path = Some(entry.path);
        }
    }

    /// 操作目标：selected 非空用选中集（按栏内顺序），否则用焦点项。
    fn op_targets(&mut self, idx: usize) -> Vec<PathBuf> {
        let panel = &mut self.panels[idx];
        if panel.selected.is_empty() {
            return panel
                .focused_entry()
                .map(|e| vec![e.path])
                .unwrap_or_default();
        }
        panel
            .entries
            .iter()
            .filter(|e| panel.selected.contains(&e.path))
            .map(|e| e.path.clone())
            .collect()
    }

    /// 复制/移动确认框：目标默认非焦点栏目录（单栏模式 = 本栏目录）。
    fn open_copy_move_dialog(&mut self, kind: OpKind, sources: Vec<PathBuf>, from_panel: usize) {
        let dest = match self.layout {
            PanelLayout::Dual { .. } => self.panels[1 - from_panel].dir.clone(),
            PanelLayout::Single { .. } => self.panels[from_panel].dir.clone(),
        };
        self.dialog = Some(FmDialog::CopyMove(CopyMoveDialog::new(
            kind, sources, &dest,
        )));
    }

    /// 压缩确认框：目标目录默认值与复制/移动一致（非焦点栏/本栏目录）。
    fn open_compress_dialog(&mut self, sources: Vec<PathBuf>, from_panel: usize) {
        let dest = match self.layout {
            PanelLayout::Dual { .. } => self.panels[1 - from_panel].dir.clone(),
            PanelLayout::Single { .. } => self.panels[from_panel].dir.clone(),
        };
        self.dialog = Some(FmDialog::Compress(CompressDialog::new(sources, &dest)));
    }

    /// 栏间拖放复制（仅双栏接收）：行 payload 经 egui 全局 dnd 状态传递，
    /// 拖动中画「N 项」光标徽标；指针悬停另一栏（目录不同）时整栏高亮，
    /// 松开弹出既有「复制到…」确认框（dest = 目标栏当前目录，经
    /// open_copy_move_dialog 的既有 dest 计算）。拖到源栏自身或两栏
    /// 同目录时忽略（不高亮、不响应 drop）。
    fn poll_inter_panel_dnd(&mut self, ui: &egui::Ui, panel_rects: &[(usize, egui::Rect)]) {
        let ctx = ui.ctx();
        let Some(payload) = egui::DragAndDrop::payload::<FmDragPayload>(ctx) else {
            return;
        };
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        // 光标跟随徽标（「N 项」）；松开帧 payload 仍在但按键已抬，徽标消失。
        if pointer_down {
            if let Some(pos) = ctx.pointer_interact_pos() {
                egui::Area::new(egui::Id::new("fm-dnd-badge"))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .fixed_pos(pos + egui::vec2(14.0, 14.0))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(format!("{} 项", payload.sources.len()));
                        });
                    });
            }
        }
        if !matches!(self.layout, PanelLayout::Dual { .. }) {
            return;
        }
        let released = ctx.input(|i| i.pointer.primary_released());
        let layer = ui.layer_id();
        let mut drop_sources: Option<Vec<PathBuf>> = None;
        for &(idx, rect) in panel_rects {
            // 快览面板（被替换的非活动栏）不作为落点（不高亮、不响应 drop）。
            if idx == payload.src_panel
                || (self.quickview_open && idx != self.active)
                || !ctx.rect_contains_pointer(layer, rect)
            {
                continue;
            }
            if self.panels[idx].dir == self.panels[payload.src_panel].dir {
                continue;
            }
            if pointer_down {
                // 高亮落点栏：active 栏面包屑同款淡底 + 选中色描边。
                let painter = ui.painter();
                painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
            } else if released {
                drop_sources = Some(payload.sources.clone());
            }
        }
        if let Some(sources) = drop_sources {
            egui::DragAndDrop::clear_payload(ctx);
            // 落点恒为 1-src_panel，dest 计算与 F5/菜单「复制到另一栏…」一致。
            self.open_copy_move_dialog(OpKind::Copy, sources, payload.src_panel);
        }
    }

    /// 删除（确认框已把关或 fm_confirm_delete=false）：预览目标在被删项中
    /// 先清预览，然后起后台任务。
    fn start_delete(&mut self, sources: Vec<PathBuf>) {
        if let Some(tp) = &self.preview_path {
            if sources.contains(tp) {
                self.clear_preview();
            }
        }
        self.ops.start_delete(sources);
    }

    /// 渲染操作确认对话框；Some(outcome) 时统一执行。
    fn render_dialog(&mut self, ctx: &egui::Context, intents: &mut FmIntents) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        match dialog.ui(ctx) {
            None => self.dialog = Some(dialog),
            Some(outcome) => self.apply_dialog_outcome(outcome, intents),
        }
    }

    fn apply_dialog_outcome(&mut self, outcome: FmDialogOutcome, intents: &mut FmIntents) {
        match outcome {
            FmDialogOutcome::Cancelled => {}
            FmDialogOutcome::ConfirmCopyMove {
                kind,
                sources,
                dest,
                conflict,
            } => {
                match kind {
                    OpKind::Copy => self.ops.start_copy(sources, dest, conflict),
                    OpKind::Move => self.ops.start_move(sources, dest, conflict),
                    OpKind::Delete | OpKind::Compress => unreachable!("各有专用路径"),
                };
            }
            FmDialogOutcome::ConfirmCompress { sources, dest_zip } => {
                self.ops.start_compress(sources, dest_zip);
            }
            FmDialogOutcome::ConfirmDelete {
                sources,
                dont_ask_again,
            } => {
                if dont_ask_again {
                    intents.confirm_delete_change = Some(false);
                }
                self.start_delete(sources);
            }
            FmDialogOutcome::ConfirmRename { path, new_name } => {
                match rename_entry(&path, &new_name) {
                    Ok(new_path) => self.refresh_panel_of(&new_path),
                    Err(e) => intents.op_error = Some(e),
                }
            }
            FmDialogOutcome::ConfirmNewDir { parent, name } => match create_dir(&parent, &name) {
                Ok(new_path) => self.refresh_panel_of(&new_path),
                Err(e) => intents.op_error = Some(e),
            },
            FmDialogOutcome::SelectGroup {
                pattern,
                select,
                files_only,
            } => {
                // 记住上次输入（会话内），作用于焦点栏当前可见行。
                self.select_group_pattern = pattern.clone();
                self.panels[self.active].apply_pattern_selection(&pattern, select, files_only);
            }
        }
    }

    /// 重命名/新建文件夹完成：刷新所在栏并选中新条目（refresh 保留选中，
    /// 列举完成后新路径仍在选中集内）。
    fn refresh_panel_of(&mut self, new_path: &Path) {
        for panel in &mut self.panels {
            if new_path.parent() == Some(panel.dir.as_path()) {
                panel.selected.insert(new_path.to_path_buf());
                panel.refresh();
            }
        }
    }

    /// 解压完成刷新落点栏：栏目录 == 输出目录（直接解压进目标文件夹）或
    /// 输出目录的父目录（智能/强制建包名子目录）时刷新，保留选中。
    pub fn refresh_extract_dest(&mut self, output_dir: &Path) {
        for panel in &mut self.panels {
            if panel.dir == output_dir || output_dir.parent() == Some(panel.dir.as_path()) {
                panel.refresh();
            }
        }
    }

    /// 操作完成：刷新涉及的两栏（目标栏 + 源栏），汇总错误经回调上报。
    fn on_op_finished(&mut self, op: FinishedOp, intents: &mut FmIntents) {
        for panel in &mut self.panels {
            let involved =
                op.dest_dir.as_ref() == Some(&panel.dir) || op.src_dirs.contains(&panel.dir);
            if involved {
                panel.refresh();
            }
        }
        let verb = op.kind.verb();
        let mut parts = Vec::new();
        if op.cancelled {
            parts.push(format!("{verb}已取消（已处理部分保留）"));
        }
        if let Some(fatal) = &op.fatal {
            parts.push(format!("{verb}失败: {fatal}"));
        }
        if !op.errors.is_empty() {
            let first: Vec<String> = op
                .errors
                .iter()
                .take(3)
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect();
            parts.push(format!(
                "{verb}完成，{} 项失败:\n{}",
                op.errors.len(),
                first.join("\n")
            ));
        }
        if !parts.is_empty() {
            intents.op_error = Some(parts.join("\n"));
        }
    }

    /// 键盘导航（Explorer/TC 式）：Tab 切换焦点栏、↑/↓ 移动焦点并单选、
    /// Shift+↑/↓ 从 anchor 扩选、Ctrl+↑/↓ 只移焦点、Home/End 跳首/末行、
    /// PgUp/PgDn 整页步进、Enter 打开焦点行、空格计算焦点目录大小、
    /// Backspace 上级、Ctrl+A 全选可见、Ctrl+R 刷新、Alt+←/→ 导航历史、
    /// Alt+↓ 历史下拉开关、可打印字符 type-ahead 定位、`*` 反选、
    /// `+`/`-` 弹「选择组」对话框、Ctrl+U 交换两栏、Ctrl+←/→ 栏间目录
    /// 同步、Ctrl+\ 回根目录、Ctrl+Q 对面栏快速预览（单栏 = 预览开关）、
    /// Esc 分级清 type-ahead 缓冲→过滤→选中。
    /// 过滤框等文本输入占用键盘时不处理。
    /// 文件操作键：F2 重命名 / F5 复制 / F6 移动 / F7 新建文件夹 /
    /// F8(Delete) 删除（confirm_delete 时先弹确认框）；Ctrl+C/X/V 剪贴板。
    fn handle_keyboard(&mut self, ui: &egui::Ui, intents: &mut FmIntents, confirm_delete: bool) {
        // 对话框打开时屏蔽面板键盘（输入归对话框）。
        if self.dialog.is_some() {
            return;
        }
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        // Tab 切换焦点栏（仅双栏；Shift/Ctrl+Tab 不拦，留给系统/输入焦点）。
        if matches!(self.layout, PanelLayout::Dual { .. })
            && !mods.shift
            && !mods.command
            && ui.input(|i| i.key_pressed(egui::Key::Tab))
        {
            self.active = 1 - self.active;
        }
        // Shift 优先于 Ctrl（Explorer：Shift+方向 = 扩选，Ctrl+方向 = 只移焦点）。
        let focus_mode = if mods.shift {
            FocusMove::Extend
        } else if mods.command {
            FocusMove::FocusOnly
        } else {
            FocusMove::Select
        };
        let active = self.active;
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.panels[active].go_back();
        }
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.panels[active].go_forward();
        }
        // Alt+↓：开/关焦点栏的目录历史下拉（一次性请求，面包屑渲染时消费）。
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.history_menu_toggle = true;
        }
        // 鼠标侧键：Extra1 = 后退，Extra2 = 前进（仅本视图；漫画阅读器侧
        // 键翻页在 app.rs 的 View::Reader 分支处理，不冲突）。
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.panels[active].go_back();
        }
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
            self.panels[active].go_forward();
        }
        // 应用内剪贴板：Ctrl+C 复制 / Ctrl+X 剪切 / Ctrl+V 粘贴（经确认框）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::C)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.clipboard = targets;
                self.clipboard_cut = false;
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::X)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.clipboard = targets;
                self.clipboard_cut = true;
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::V)) && !self.clipboard.is_empty() {
            let kind = if self.clipboard_cut {
                OpKind::Move
            } else {
                OpKind::Copy
            };
            self.open_copy_move_dialog(kind, self.clipboard.clone(), active);
        }
        // 文件操作：F2 重命名 / F5 复制 / F6 移动 / F7 新建文件夹 / F8(Del) 删除。
        if ui.input(|i| i.key_pressed(egui::Key::F2)) {
            if let Some(entry) = self.panels[active].focused_entry() {
                self.dialog = Some(FmDialog::Rename(RenameDialog::new(entry.path)));
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Copy, targets, active);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F6)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Move, targets, active);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F7)) {
            let parent = self.panels[active].dir.clone();
            let suggested = suggest_folder_name(&parent);
            self.dialog = Some(FmDialog::NewDir(NewDirDialog::new(parent, suggested)));
        }
        if ui.input(|i| i.key_pressed(egui::Key::F8) || i.key_pressed(egui::Key::Delete)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                if confirm_delete {
                    self.dialog = Some(FmDialog::Delete(DeleteDialog::new(targets)));
                } else {
                    self.start_delete(targets);
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.panels[active].parent_dir();
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::A)) {
            self.panels[active].select_all_visible();
        }
        // `*` 反选 / `+`「选择组」对话框 / `-` 同框预置取消选择（TC 语义），
        // 其余可打印字符进 type-ahead（type-to-select）缓冲。
        // egui 0.35 的 Key 枚举没有小键盘乘/加/减键，主键盘 `*` 又是 Shift+8，
        // 统一用 Event::Text 捕获——文本事件只在无控件占用键盘时产生
        // （上面 egui_wants_keyboard_input 已挡掉过滤框/对话框输入）。
        // 空格保留给「计算焦点目录大小」，不进 type-ahead。
        let (star, plus, minus, type_chars) = ui.input(|i| {
            let (mut star, mut plus, mut minus) = (false, false, false);
            let mut chars = Vec::new();
            for e in &i.events {
                if let egui::Event::Text(t) = e {
                    for c in t.chars() {
                        match c {
                            '*' => star = true,
                            '+' => plus = true,
                            '-' => minus = true,
                            ' ' => {}
                            c if !c.is_control() => chars.push(c),
                            _ => {}
                        }
                    }
                }
            }
            (star, plus, minus, chars)
        });
        if star {
            self.panels[active].invert_selection();
        }
        if plus {
            self.dialog = Some(FmDialog::SelectGroup(SelectGroupDialog::new(
                self.select_group_pattern.clone(),
                false,
            )));
        }
        if minus {
            self.dialog = Some(FmDialog::SelectGroup(SelectGroupDialog::new(
                self.select_group_pattern.clone(),
                true,
            )));
        }
        // type-to-select：命中即移动焦点并单选（Select 语义同 ↑/↓，
        // 经 focus_row 置最小滚动揭示）；Ctrl/Alt 组合键不产出定位字符。
        if !mods.command && !mods.alt {
            let panel = &mut self.panels[active];
            for c in type_chars {
                if let Some(row) = panel.type_ahead_push(c) {
                    panel.focus_row(row, FocusMove::Select);
                }
            }
        }
        // 栏间快捷键（仅双栏）：Ctrl+U 交换两栏（watcher/loader 随结构体走，
        // active 不变；快照 diff 自然写回 dir_left/dir_right）。
        let dual = matches!(self.layout, PanelLayout::Dual { .. });
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::U)) {
            self.panels.swap(0, 1);
        }
        // Ctrl+→：另一栏跳到本栏焦点目录（焦点非目录则跳本栏当前目录）；
        // Ctrl+←：反向（本栏 ← 另一栏焦点目录/当前目录）。
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            let target = match self.panels[active].focused_entry() {
                Some(e) if e.is_dir => e.path,
                _ => self.panels[active].dir.clone(),
            };
            self.panels[1 - active].navigate_to(target);
        }
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            let other = 1 - active;
            let target = match self.panels[other].focused_entry() {
                Some(e) if e.is_dir => e.path,
                _ => self.panels[other].dir.clone(),
            };
            self.panels[active].navigate_to(target);
        }
        // Ctrl+\：本栏回根目录（同面包屑段点击根段）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::Backslash)) {
            let panel_dir = self.panels[active].dir.clone();
            if !panel_dir.as_os_str().is_empty() {
                if let Some(root) = panel_dir.ancestors().last() {
                    self.panels[active].navigate_to(root.to_path_buf());
                }
            }
        }
        // Ctrl+Q：双栏 = 开关对面栏快速预览（快览面板恒渲染非活动栏位置，
        // 目标 = 活动栏焦点文件）；单栏 = 切换预览面板（同顶栏「预览」）。
        // Q 是字母键，但 type-ahead 捕获有 !mods.command 门控，不会抢键。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::Q)) {
            match &mut self.layout {
                PanelLayout::Dual { .. } => {
                    self.quickview_open = !self.quickview_open;
                    if !self.quickview_open {
                        // 关闭即清预览目标，避免后台继续读取。
                        self.clear_preview();
                    }
                }
                PanelLayout::Single { preview_open } => {
                    *preview_open = !*preview_open;
                    self.saved_preview_open = *preview_open;
                }
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::R)) {
            self.panels[active].refresh();
        }
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.panels[active].move_focus(1, focus_mode);
        }
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.panels[active].move_focus(-1, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Home)) {
            self.panels[active].move_focus_edge(false, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::End)) {
            self.panels[active].move_focus_edge(true, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
            let step = self.panels[active].page_step();
            self.panels[active].move_focus(step, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
            let step = self.panels[active].page_step();
            self.panels[active].move_focus(-step, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let rows = self.panels[active].rows();
            if let Some(row) = self.panels[active].focus {
                self.open_ui_row(active, &rows, row, intents);
            }
        }
        // 空格：计算焦点目录大小（文件/「..」/无焦点忽略）。
        if !mods.command
            && !mods.shift
            && !mods.alt
            && ui.input(|i| i.key_pressed(egui::Key::Space))
        {
            if let Some(entry) = self.panels[active].focused_entry() {
                if entry.is_dir {
                    self.panels[active].request_dir_sizes(vec![entry.path]);
                }
            }
        }
        // F3：双栏模式对焦点文件弹临时预览窗（再按 F3 / Esc / 关闭按钮关窗）。
        if ui.input(|i| i.key_pressed(egui::Key::F3)) {
            if self.preview_window_open {
                self.preview_window_open = false;
            } else if matches!(self.layout, PanelLayout::Dual { .. }) {
                if let Some(entry) = self.panels[active].focused_entry() {
                    if !entry.is_dir {
                        self.set_preview_target(entry);
                        self.preview_window_open = true;
                    }
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.preview_window_open {
                self.preview_window_open = false;
            } else {
                let panel = &mut self.panels[active];
                if panel.type_ahead_buffer().is_some() {
                    panel.clear_type_ahead();
                } else if !panel.filter.is_empty() {
                    panel.filter.clear();
                } else {
                    panel.clear_selection();
                }
            }
        }
    }

    /// 「选中即预览」：焦点栏焦点行落到文件时更新预览目标（可预览类型后台
    /// 加载，不可预览类型直接显示元信息占位，不读内容）；焦点不在文件上
    /// （目录/「..」/无焦点）时清空预览——预览面板/快览恒在，内容切占位。
    /// 仅单栏预览开 / F3 弹窗开 / 双栏快览开时跟随焦点，其余情况不发起
    /// 后台读取。
    fn sync_preview_target(&mut self) {
        let follows = matches!(self.layout, PanelLayout::Single { preview_open: true })
            || self.preview_window_open
            || (matches!(self.layout, PanelLayout::Dual { .. }) && self.quickview_open);
        if !follows {
            return;
        }
        match self.panels[self.active].focused_entry() {
            Some(entry) if !entry.is_dir => {
                if self.preview_path.as_ref() != Some(&entry.path) {
                    self.set_preview_target(entry);
                }
            }
            _ => self.clear_preview(),
        }
    }

    /// 设置预览目标并发起后台读取（不可预览类型直接置元信息占位说明）。
    fn set_preview_target(&mut self, entry: FsEntry) {
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        self.preview_full_size = false;
        if is_previewable_name(&entry.name) {
            let path = entry.path.clone();
            self.preview = Some(AsyncOpener::open(path, load_file_preview));
        } else {
            self.preview_note = Some("不支持预览该类型".to_string());
        }
        self.preview_path = Some(entry.path.clone());
        self.preview_entry = Some(entry);
    }

    /// 清空预览目标与全部预览内容（焦点离开文件 / 目标消失时）。
    fn clear_preview(&mut self) {
        self.preview_path = None;
        self.preview_entry = None;
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        self.preview_full_size = false;
    }

    /// 每帧排空预览后台读取；在途时主动重绘（egui 空闲不重绘）。
    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(mut preview) = self.preview.take() else {
            return;
        };
        match preview.poll() {
            OpenStatus::Loading => {
                self.preview = Some(preview);
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            OpenStatus::Ready(result) => self.apply_preview_result(result),
        }
    }

    fn apply_preview_result(&mut self, result: Result<PreviewData, String>) {
        match result {
            Ok(PreviewData::Image(img)) => self.pending_preview_image = Some(img),
            Ok(PreviewData::Text(text)) => self.preview_text = Some(text),
            Ok(PreviewData::Unsupported) => {
                self.preview_note = Some("不支持预览该类型".to_string());
            }
            Ok(PreviewData::Note(note)) => self.preview_note = Some(note),
            Err(e) => self.preview_note = Some(e),
        }
    }

    /// F3 临时预览弹窗（双栏模式）：与单栏预览面板共用绘制代码。
    fn render_preview_window(&mut self, ctx: &egui::Context) {
        if !self.preview_window_open {
            return;
        }
        let title = self
            .preview_entry
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "预览".to_string());
        let mut open = true;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_size([420.0, 520.0])
            .open(&mut open)
            .show(ctx, |ui| {
                self.draw_preview_content(ui);
            });
        if !open {
            self.preview_window_open = false;
        }
    }

    /// 预览内容绘制（单栏预览面板与 F3 弹窗共用）：名称/大小/时间头部、
    /// 图片适应宽度/原始尺寸切换、文本 Monospace 只读可选中、
    /// 不支持类型显示元信息占位。
    fn draw_preview_content(&mut self, ui: &mut egui::Ui) {
        // poll 收到的 ColorImage 在此（有 ctx）惰性上传为纹理。
        if let Some(img) = self.pending_preview_image.take() {
            self.preview_tex = Some(ui.ctx().load_texture(
                "fm-preview",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        let Some(entry) = self.preview_entry.clone() else {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("选中文件以预览").weak());
            return;
        };
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&entry.name).strong());
        let mut meta = entry.size.map(human_size).unwrap_or_default();
        let mtime = format_mtime(system_time_to_unix(entry.mtime));
        if !mtime.is_empty() {
            if !meta.is_empty() {
                meta.push_str(" · ");
            }
            meta.push_str(&mtime);
        }
        if !meta.is_empty() {
            ui.label(egui::RichText::new(meta).weak());
        }
        // 图片预览的「适应宽度 / 原始尺寸」切换。
        if self.preview_tex.is_some() {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!self.preview_full_size, "适应宽度")
                    .clicked()
                {
                    self.preview_full_size = false;
                }
                if ui
                    .selectable_label(self.preview_full_size, "原始尺寸")
                    .clicked()
                {
                    self.preview_full_size = true;
                }
            });
        }
        ui.separator();
        // 原始尺寸模式：按纹理原始大小显示，独立双向滚动区。
        if self.preview_full_size {
            if let Some(tex) = &self.preview_tex {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.image(egui::load::SizedTexture::new(tex.id(), tex.size_vec2()));
                    });
                return;
            }
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(tex) = &self.preview_tex {
                    let width = ui.available_width();
                    let size = tex.size_vec2();
                    let height = if size.x > 0.0 {
                        width * size.y / size.x
                    } else {
                        width
                    };
                    ui.image(egui::load::SizedTexture::new(
                        tex.id(),
                        egui::vec2(width, height),
                    ));
                } else if let Some(text) = &self.preview_text {
                    // 只读 &str 缓冲（TextBuffer for &str 拒绝修改）：可选中复制。
                    let mut text = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                }
                if let Some(note) = &self.preview_note {
                    ui.label(egui::RichText::new(note).weak());
                }
                if self.preview.is_some()
                    && self.preview_tex.is_none()
                    && self.preview_text.is_none()
                    && self.preview_note.is_none()
                {
                    ui.label(egui::RichText::new("正在读取…").weak());
                }
            });
    }
}

/// 竖向分隔条：拖动返回本帧的水平位移（px），hover/拖拽时换光标。
fn render_splitter(ui: &mut egui::Ui, height: f32) -> Option<f32> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SPLITTER_WIDTH, height), egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let color = if response.dragged() || response.hovered() {
        ui.visuals().widgets.active.bg_stroke.color
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0, color),
    );
    if response.dragged() {
        let dx = response.drag_motion().x;
        (dx != 0.0).then_some(dx)
    } else {
        None
    }
}

/// 栏当前目录的根展示名（Windows `C:\`；Unix `/`）；空路径回退 `/`。
/// 复用面包屑首段（Windows Prefix+Root 已合并为 `C:\`）。
fn drive_label(dir: &Path) -> String {
    breadcrumb_segments(dir)
        .first()
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| "/".to_string())
}

/// 面包屑分段：根（`C:\` 或 `/`）+ 各级目录名，每段带其完整路径。
fn breadcrumb_segments(path: &Path) -> Vec<(PathBuf, String)> {
    let mut segments: Vec<(PathBuf, String)> = Vec::new();
    let mut cur = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(prefix) => {
                cur.push(prefix.as_os_str());
                segments.push((
                    cur.clone(),
                    prefix.as_os_str().to_string_lossy().into_owned(),
                ));
            }
            Component::RootDir => {
                cur.push(comp.as_os_str());
                match segments.last_mut() {
                    // Windows：`C:` + 根 → 显示 `C:\`。
                    Some(last) => {
                        last.0 = cur.clone();
                        last.1.push('\\');
                    }
                    None => segments.push((cur.clone(), "/".to_string())),
                }
            }
            Component::Normal(name) => {
                cur.push(name);
                segments.push((cur.clone(), name.to_string_lossy().into_owned()));
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    segments
}

/// 明细列表行的类型图标（按扩展名，大小写不敏感）；目录行恒用 FOLDER。
fn entry_icon(entry: &FsEntry) -> Icon {
    if entry.is_dir {
        return icons::FOLDER;
    }
    if archive_kind(&entry.path).is_some() {
        return icons::FILE_ARCHIVE;
    }
    let ext = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        e if openitgo_parser::traits::is_image_extension(e) => icons::FILE_IMAGE,
        "txt" | "md" | "log" => icons::FILE_TEXT,
        "pdf" => icons::FILE_PDF,
        "mp4" | "mkv" | "avi" | "mov" | "webm" => icons::FILE_VIDEO,
        "mp3" | "flac" | "aac" | "ogg" | "wav" => icons::FILE_AUDIO,
        _ => icons::FILE,
    }
}

/// 名称列与「大小」「修改时间」列之间的淡竖线（表头与数据行共用；
/// 坐标取自 column_layout，与表头列区间一致）。
fn paint_column_separators(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    layout: &ColumnLayout,
) {
    let stroke = egui::Stroke::new(1.0, color);
    for x in [layout.size_left, layout.mtime_left] {
        painter.vline(x, rect.y_range(), stroke);
    }
}

/// 键盘焦点的最小滚动（Explorer 语义）：焦点行已在视口内则偏移不变；
/// 上方越界贴顶（offset = 行顶），下方越界贴底（offset = 行底 - 视口高）。
fn min_scroll_to_reveal(offset: f32, viewport_h: f32, row_top: f32) -> f32 {
    let row_bottom = row_top + ROW_HEIGHT;
    if row_top < offset {
        row_top
    } else if row_bottom > offset + viewport_h {
        row_bottom - viewport_h
    } else {
        offset
    }
}

/// SystemTime → Unix 秒（本地时间格式化在 archive::format_mtime 里做）。
fn system_time_to_unix(t: Option<std::time::SystemTime>) -> Option<i64> {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumb_segments_windows_style() {
        // 不依赖真实文件系统，纯路径解析（Windows 前缀在非 Windows 上
        // 走 Normal 分支，此处只验证分段语义不验证盘符）。
        let segs = breadcrumb_segments(Path::new("/a/b/c"));
        let labels: Vec<&str> = segs.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["/", "a", "b", "c"]);
        assert_eq!(segs[2].0, PathBuf::from("/a/b"));
    }

    #[test]
    fn drive_label_takes_root_segment() {
        assert_eq!(drive_label(Path::new("/a/b")), "/");
        assert_eq!(drive_label(Path::new("")), "/");
        #[cfg(windows)]
        assert_eq!(drive_label(Path::new(r"C:\foo\bar")), r"C:\");
    }

    #[test]
    fn min_scroll_only_when_out_of_view() {
        // 已在视口内：不动
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 150.0), 100.0);
        // 上方越界：贴顶
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 50.0), 50.0);
        // 下方越界：贴底
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 280.0), 102.0);
    }

    #[test]
    fn drag_sources_selected_set_or_single_row() {
        let a = PathBuf::from("/x/a");
        let b = PathBuf::from("/x/b");
        let c = PathBuf::from("/x/c");
        let selected = HashSet::from([b.clone(), a.clone()]);
        // 拖选中行 → 整个选中集（排序后确定）。
        assert_eq!(drag_sources(&selected, &a), vec![a.clone(), b.clone()]);
        // 拖未选中行 → 仅该行自身。
        assert_eq!(drag_sources(&selected, &c), vec![c.clone()]);
    }

    #[test]
    fn bookmark_add_dedup_remove_and_snapshot() {
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        let a = PathBuf::from("/a");
        let b = PathBuf::from("/b");

        assert!(view.add_bookmark(&a));
        assert!(view.add_bookmark(&b));
        // 去重：重复添加 no-op。
        assert!(!view.add_bookmark(&a));
        assert_eq!(
            view.snapshot().bookmarks,
            ["/a".to_string(), "/b".to_string()]
        );

        assert!(view.remove_bookmark(&a));
        assert!(!view.remove_bookmark(&a));
        assert_eq!(view.snapshot().bookmarks, ["/b".to_string()]);
    }

    #[test]
    fn snapshot_restores_bookmarks_from_constructor() {
        let saved = vec!["/a".to_string(), "/b".to_string()];
        let view = FileManagerView::new("dual", 0.5, false, "name", true, &saved);
        assert_eq!(view.snapshot().bookmarks, saved);
    }

    // ---- 无头 egui 测试基座：注入输入事件驱动 FileManagerView 真实渲染帧 ----

    /// 跑一帧真实渲染（含行交互/意图分发），events 为本帧注入的输入。
    fn headless_frame(
        ctx: &egui::Context,
        view: &mut FileManagerView,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            view.ui(
                ui,
                FmCallbacks {
                    on_back: &mut || {},
                    on_open_path: &mut |_| {},
                    on_open_archive: &mut |_| {},
                    on_open_as_comic: &mut |_| {},
                    on_extract: &mut |_, _| {},
                    on_op_error: &mut |_| {},
                    on_confirm_delete_change: &mut |_| {},
                },
                false,
                true,
            );
        });
    }

    fn primary_click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = egui::PointerButton::Primary;
        vec![
            egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    /// navigate 并 poll 到 Ready（AsyncOpener 后台线程）。
    fn navigate_ready(panel: &mut FsPanel, dir: &Path) {
        panel.navigate_to(dir.to_path_buf());
        for _ in 0..400 {
            if !panel.poll() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("目录列举未在限时内完成");
    }

    /// epaint 0.35 对未注册的字体族直接 panic：FM 行/按钮用磷图标字体
    /// （fonts.rs 在 lib crate root，bin 测试目标够不到，此处内联最小版）。
    fn setup_test_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor_icons::add_fonts(&mut fonts);
        ctx.set_fonts(fonts);
    }

    /// 回归（双击目录闪退）：双击目录行同帧 navigate_to 清空 entries，
    /// render_list 的循环若继续按导航前 rows 快照渲染后续行，render_row
    /// 里 entries[i] 越界 panic。修复后状态离开 Ready 即停笔。
    #[test]
    fn double_click_dir_mid_frame_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        for d in ["d1", "d2", "d3"] {
            std::fs::create_dir(tmp.path().join(d)).unwrap();
        }
        for f in ["f1", "f2", "f3"] {
            std::fs::write(tmp.path().join(f), b"x").unwrap();
        }
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧

        // 自上而下单击扫描，定位 UI 行 1（第一个目录 d1）的 y。
        // y 从 95 起跳过顶栏/面包屑/列头（面包屑单击会导航、污染扫描）。
        let mut row1_pos = None;
        let mut y = 95.0;
        while y < 500.0 {
            t += 1.0; // 间隔超过 max_double_click_delay，避免连击计数干扰
            let pos = egui::pos2(200.0, y);
            headless_frame(&ctx, &mut view, t, primary_click_events(pos));
            if view.panels[0].focus == Some(1) {
                row1_pos = Some(pos);
                break;
            }
            y += 5.0;
        }
        let pos = row1_pos.expect("未能定位到行 1（d1）");
        // 扫描期间的点击可能改动选中态；重置回被测目录（布局不变，y 仍有效）。
        navigate_ready(&mut view.panels[0], tmp.path());
        headless_frame(&ctx, &mut view, t, vec![]);

        // 双击 d1 → navigate_to 同帧清 entries：修复前下一行渲染即越界 panic。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        t += 0.1;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        for _ in 0..20 {
            t += 1.0;
            view.panels[0].poll();
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert!(view.panels[0].dir.ends_with("d1"));
        assert!(matches!(view.panels[0].state, PanelLoadState::Ready));
    }

    /// 回归（双栏滚动串扰）：两栏 ScrollArea 曾共享 auto-id，滚动状态
    /// 互相跟随。修复后每栏 push_id 隔离，滚左栏右栏不动、反之亦然。
    #[test]
    fn dual_panels_scroll_independently() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..200 {
            std::fs::write(tmp.path().join(format!("f{i:03}")), b"x").unwrap();
        }
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        navigate_ready(&mut view.panels[1], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧

        let wheel_down = |pos: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -5.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ]
        };

        // 滚左栏：右栏不应跟随。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, wheel_down(egui::pos2(300.0, 400.0)));
        let left_after = view.panels[0].last_scroll_offset;
        assert!(left_after > 0.0, "滚轮应滚动左栏");
        assert_eq!(
            view.panels[1].last_scroll_offset, 0.0,
            "右栏不应跟随左栏滚动"
        );

        // 滚右栏：左栏保持不动。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, wheel_down(egui::pos2(980.0, 400.0)));
        assert!(view.panels[1].last_scroll_offset > 0.0, "滚轮应滚动右栏");
        assert_eq!(
            view.panels[0].last_scroll_offset, left_after,
            "左栏不应跟随右栏滚动"
        );
    }

    /// 栏间拖放：从左栏拖行到右栏松开 → payload 设置/传递 → 弹既有
    /// 「复制到…」确认框（不直拷）；拖到源栏自身松开则无事发生。
    #[test]
    fn drag_row_to_other_panel_opens_copy_dialog() {
        let tmp_left = tempfile::tempdir().unwrap();
        let tmp_right = tempfile::tempdir().unwrap();
        std::fs::write(tmp_left.path().join("a.txt"), b"x").unwrap();
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp_left.path());
        navigate_ready(&mut view.panels[1], tmp_right.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.txt");
        assert_eq!(view.active, 0, "行定位应落在左栏");

        let button = egui::PointerButton::Primary;
        let mods = egui::Modifiers::NONE;
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: mods,
        };
        // 按下后移动超过拖拽阈值（6pt）→ drag_started → payload 设置。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![press(pos, true)]);
        t += 0.1;
        let mid = egui::pos2(pos.x + 30.0, pos.y + 10.0);
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(mid)]);
        assert!(
            egui::DragAndDrop::has_payload_of_type::<FmDragPayload>(&ctx),
            "拖动行应设置 dnd payload"
        );

        // 拖到右栏松开 → 弹复制确认框，payload 被取走。
        t += 0.1;
        let drop_pos = egui::pos2(1000.0, 400.0);
        headless_frame(
            &ctx,
            &mut view,
            t,
            vec![egui::Event::PointerMoved(drop_pos), press(drop_pos, false)],
        );
        assert!(
            matches!(view.dialog, Some(FmDialog::CopyMove(_))),
            "drop 到另一栏应弹「复制到…」确认框"
        );
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));

        // 关对话框；再拖到源栏自身松开 → 不弹框。
        view.dialog = None;
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![press(pos, true)]);
        t += 0.1;
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(mid)]);
        t += 0.1;
        headless_frame(&ctx, &mut view, t, vec![press(mid, false)]);
        assert!(view.dialog.is_none(), "拖回源栏不应弹框");
    }

    // ---- 预览链路回归（查看预览崩溃）----

    fn key_events(key: egui::Key) -> Vec<egui::Event> {
        let mods = egui::Modifiers::NONE;
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: mods,
            },
        ]
    }

    /// 2x2 红 PNG 字节。
    fn test_png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 单击扫描定位「指定文件名的焦点行」（跳过顶栏/面包屑/列头）。
    fn locate_file_row(
        ctx: &egui::Context,
        view: &mut FileManagerView,
        t: &mut f64,
        file_name: &str,
    ) -> egui::Pos2 {
        let mut y = 95.0;
        while y < 500.0 {
            *t += 1.0;
            let pos = egui::pos2(200.0, y);
            headless_frame(ctx, view, *t, primary_click_events(pos));
            let hit = view.panels[view.active]
                .focused_entry()
                .is_some_and(|e| e.name == file_name);
            if hit {
                return pos;
            }
            y += 5.0;
        }
        panic!("未能定位到文件行 {file_name}");
    }

    /// 单栏预览：选中图片行 → 后台加载 → 纹理上传渲染若干帧；
    /// 再切「原始尺寸」模式渲染（独立双向滚动区路径）。
    #[test]
    fn preview_image_load_and_render() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "hello").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");

        // 单击图片行 → 焦点 → sync_preview_target 发起后台读取。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        assert!(view.preview.is_some(), "应发起预览读取");
        // 排空加载并渲染（含纹理上传帧）。
        for _ in 0..30 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
            if view.preview_tex.is_some() {
                break;
            }
        }
        assert!(view.preview_tex.is_some(), "图片预览应已上传纹理");
        // 原始尺寸模式（ScrollArea::both 路径）。
        view.preview_full_size = true;
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
    }

    /// 竞态：预览在途时导航离开（clear_preview 先行），结果晚到不应 panic。
    #[test]
    fn preview_navigate_away_while_loading() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.txt"), "x").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        assert!(view.preview.is_some());
        // 加载在途时直接导航进子目录（焦点清空 → clear_preview）。
        view.panels[0].navigate_to(sub.clone());
        for _ in 0..30 {
            t += 1.0;
            view.panels[0].poll();
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert_eq!(view.panels[0].dir, sub);
        assert!(matches!(view.panels[0].state, PanelLoadState::Ready));
    }

    /// 损坏图片（扩展名 png、内容垃圾）→ Note 路径渲染。
    #[test]
    fn preview_corrupt_image_note() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), b"not a png at all").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        for _ in 0..30 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
            if view.preview_note.is_some() {
                break;
            }
        }
        assert!(view.preview_note.is_some(), "损坏图片应走 Note 说明");
    }

    /// F3 弹窗（双栏）：打开 → 移动焦点换内容 → Esc 关闭，全程渲染。
    #[test]
    fn preview_f3_window_open_switch_close() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("c.bin"), [0xFF, 0xFE, 0x00]).unwrap();
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        navigate_ready(&mut view.panels[1], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        // 焦点落到 a.png 后开 F3 弹窗。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::F3));
        assert!(view.preview_window_open, "F3 应打开预览弹窗");
        // 加载 + 渲染若干帧（弹窗与面板同帧共存）。
        for _ in 0..10 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        // ↓ 移焦点到 b.txt（弹窗内容切换），再 ↓ 到 c.bin（不支持类型）。
        for _ in 0..2 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, key_events(egui::Key::ArrowDown));
            for _ in 0..5 {
                t += 1.0;
                headless_frame(&ctx, &mut view, t, vec![]);
            }
        }
        // Esc 关弹窗。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::Escape));
        assert!(!view.preview_window_open, "Esc 应关闭弹窗");
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
    }
}
