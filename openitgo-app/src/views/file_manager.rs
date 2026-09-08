//! 双栏文件管理器视图（Total Commander 形态）：左右两个 `FsPanel` 各自独立
//! 浏览本地文件系统，Tab 切换焦点栏，单击/Ctrl/Shift 选择，双击分发打开
//! （目录 → 栏内进入；压缩包 → Archive 视图；其余 → open_path 分发）。
//! 单栏模式右半为预览面板（本阶段为占位，预览本体在阶段三）。
//!
//! 行渲染严格遵循 AGENTS.md 的列布局约定（与 archive.rs 明细列表同范式）：
//! 固定行高 + show_rows 虚拟化、item_spacing.y 归零、行内容 scope 内
//! interact_size.y 压回 ROW_HEIGHT-4、scope 后 advance_cursor_after_rect
//! 钉回行底、右两列 painter.text 右对齐直绘（禁止 RTL 嵌套）。

use crate::views::archive::{format_mtime, human_size};
use crate::views::file_manager_panel::{
    FocusMove, FsPanel, PanelLoadState, COL_RIGHT_PAD, ROW_HEIGHT,
};
use crate::views::file_manager_rows::{FsEntry, SortKey};
use egui_phosphor_icons::{icons, Icon};
use openitgo_parser::archive::archive_kind;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

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

pub struct FileManagerView {
    pub layout: PanelLayout,
    pub panels: [FsPanel; 2],
    /// 焦点栏（0/1）：键盘操作目标；鼠标点击某栏任意处即激活。
    pub active: usize,
    /// 切回双栏时恢复的比例（Single 期间 Dual.ratio 不可见）。
    saved_ratio: f32,
    /// 单栏模式预览面板宽度占比（拖动分隔条可调）。
    preview_ratio: f32,
}

/// 帧内意图：行内交互写入，帧尾统一触发回调（避免回调嵌套借用）。
#[derive(Default)]
struct FmIntents {
    back: bool,
    open_path: Option<PathBuf>,
    open_archive: Option<PathBuf>,
    open_as_comic: Option<PathBuf>,
}

pub struct FmCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    /// 打开分发（图片/电子书/媒体/漫画文件夹等，走 app 的 open_path）。
    pub on_open_path: &'a mut dyn FnMut(PathBuf),
    /// 双击压缩包：进 Archive 视图（open_archive_browser）。
    pub on_open_archive: &'a mut dyn FnMut(PathBuf),
    /// 右键「作为漫画打开」（目录与压缩包可用）。
    pub on_open_as_comic: &'a mut dyn FnMut(PathBuf),
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

impl FileManagerView {
    /// 从 settings 恢复布局/排序（app 构造时调用）。
    pub fn new(layout: &str, ratio: f32, sort_key: &str, sort_asc: bool) -> Self {
        let layout = if layout == "single" {
            PanelLayout::Single { preview_open: true }
        } else {
            PanelLayout::Dual { ratio }
        };
        let sort = match sort_key {
            "size" => SortKey::Size,
            "mtime" => SortKey::Mtime,
            _ => SortKey::Name,
        };
        Self {
            layout,
            panels: [FsPanel::new(sort, sort_asc), FsPanel::new(sort, sort_asc)],
            active: 0,
            saved_ratio: ratio,
            preview_ratio: 0.35,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, callbacks: FmCallbacks<'_>) {
        let FmCallbacks {
            on_back,
            on_open_path,
            on_open_archive,
            on_open_as_comic,
        } = callbacks;
        let mut intents = FmIntents::default();

        // 每帧排空两栏的列举结果；Loading 期间主动重绘（egui 空闲不重绘）。
        let mut loading = false;
        for panel in &mut self.panels {
            loading |= panel.poll();
        }
        if loading {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        self.render_top_bar(ui, &mut intents);
        ui.separator();

        // 底栏：当前栏选中/条目统计。
        egui::Panel::bottom("fm_status_bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let panel = &self.panels[self.active];
                let total = panel.entries.len();
                let selected = panel.selected.len();
                ui.label(format!("共 {total} 项"));
                if selected > 0 {
                    ui.separator();
                    ui.label(format!("已选 {selected} 项"));
                }
                ui.separator();
                ui.label(egui::RichText::new("Tab 切换栏 · 双击打开 · 右键菜单").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(panel.dir.display().to_string()).weak());
                });
            });
            ui.add_space(4.0);
        });

        // 中央：双栏 + 可拖分隔条 / 单栏 + 预览占位。
        let panel_rects = self.render_panels(ui, &mut intents);

        // 鼠标点击某栏任意处即激活该栏。
        let (pressed, pos) = ui.ctx().input(|i| {
            (
                i.pointer.primary_pressed() || i.pointer.secondary_pressed(),
                i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
            )
        });
        if pressed {
            if let Some(pos) = pos {
                for (idx, rect) in panel_rects {
                    if rect.contains(pos) {
                        self.active = idx;
                    }
                }
            }
        }

        self.handle_keyboard(ui, &mut intents);

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
                self.layout = PanelLayout::Single { preview_open: true };
            }
            if let PanelLayout::Single { preview_open } = &mut self.layout {
                ui.separator();
                if ui
                    .add(egui::Button::new((icons::EYE, " 预览")).selected(*preview_open))
                    .on_hover_text("切换预览面板（预览本体在下一阶段）")
                    .clicked()
                {
                    *preview_open = !*preview_open;
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
                        |ui| self.render_panel(ui, 0, intents),
                    );
                    rects.push((0, left.response.rect));
                    if let Some(delta) = render_splitter(ui, height) {
                        ratio = (ratio + delta / total_w).clamp(RATIO_MIN, RATIO_MAX);
                    }
                    let right_w = ui.available_width();
                    let right = ui.allocate_ui_with_layout(
                        egui::vec2(right_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.render_panel(ui, 1, intents),
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
                            |ui| self.render_panel(ui, 0, intents),
                        );
                        rects.push((0, panel.response.rect));
                        if let Some(delta) = render_splitter(ui, height) {
                            preview_ratio = (preview_ratio - delta / total_w).clamp(RATIO_MIN, 0.6);
                        }
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(60.0);
                                    ui.label(
                                        egui::RichText::new(icons::EYE.as_str()).size(28.0).weak(),
                                    );
                                    ui.label(egui::RichText::new("预览（下一阶段）").weak());
                                });
                            },
                        );
                    });
                    self.preview_ratio = preview_ratio;
                } else {
                    let panel = ui.allocate_ui_with_layout(
                        egui::vec2(total_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.render_panel(ui, 0, intents),
                    );
                    rects.push((0, panel.response.rect));
                }
                rects
            }
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

    /// 面包屑：路径分段可点击跳回；焦点栏铺淡底色高亮。
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
        let cols: [(egui::Rect, SortKey, &str, f32, egui::Align2); 3] = [
            (
                name_rect,
                SortKey::Name,
                "名称",
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
            if response.hovered() {
                painter.rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
            }
            let arrow = if panel.sort_key == key {
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
        let entry: Option<FsEntry> = if is_parent {
            None
        } else {
            rows.get(row - 1).map(|&i| panel.entries[i].clone())
        };
        let selected = entry
            .as_ref()
            .is_some_and(|e| panel.selected.contains(&e.path));
        let focused = panel.focus == Some(row);
        let at_root = panel.is_root();
        let parent_enabled = !at_root;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
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
        // （目录行与「..」行大小列留空）。
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
                        ui.colored_label(text_color, icons::ARROW_UP.as_str());
                        ui.colored_label(text_color, "..");
                    }
                    Some(e) => {
                        ui.label(egui::RichText::new(entry_icon(e).as_str()).weak());
                        ui.add(egui::Label::new(&e.name).truncate());
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
                ui.separator();
                if ui.button((icons::COPY, " 复制路径")).clicked() {
                    ui.ctx().copy_text(e.path.display().to_string());
                    ui.close();
                }
                if ui.button((icons::ARROW_CLOCKWISE, " 刷新")).clicked() {
                    self.panels[idx].refresh();
                    ui.close();
                }
                // TODO(阶段四): 复制/移动/删除/重命名/新建文件夹 菜单项。
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
        let Some(&entry_idx) = rows.get(row - 1) else {
            return;
        };
        let entry = self.panels[idx].entries[entry_idx].clone();
        if entry.is_dir {
            self.panels[idx].navigate_to(entry.path.clone());
        } else if archive_kind(&entry.path).is_some() {
            intents.open_archive = Some(entry.path);
        } else {
            intents.open_path = Some(entry.path);
        }
    }

    /// 键盘导航（Explorer/TC 式）：Tab 切换焦点栏、↑/↓ 移动焦点并单选、
    /// Shift+↑/↓ 从 anchor 扩选、Ctrl+↑/↓ 只移焦点、Home/End 跳首/末行、
    /// PgUp/PgDn 整页步进、Enter 打开焦点行、Backspace 上级、Ctrl+A 全选可见、
    /// Ctrl+R 刷新、Alt+←/→ 导航历史、Esc 清过滤或清空选中。
    /// 过滤框等文本输入占用键盘时不处理。
    /// TODO(阶段四): F2 重命名 / F5 复制 / F6 移动 / F7 新建文件夹 / F8 删除。
    fn handle_keyboard(&mut self, ui: &egui::Ui, intents: &mut FmIntents) {
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
        if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.panels[active].parent_dir();
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::A)) {
            self.panels[active].select_all_visible();
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
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            let panel = &mut self.panels[active];
            if !panel.filter.is_empty() {
                panel.filter.clear();
            } else {
                panel.clear_selection();
            }
        }
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
    fn min_scroll_only_when_out_of_view() {
        // 已在视口内：不动
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 150.0), 100.0);
        // 上方越界：贴顶
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 50.0), 50.0);
        // 下方越界：贴底
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 280.0), 102.0);
    }
}
