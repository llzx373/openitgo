//! 压缩包浏览视图：资源管理器式三栏布局（左栏目录树 / 中栏面包屑+当前
//! 目录文件列表 / 右栏预览面板）。后台列出全部条目（含目录与非图片），
//! 勾选后外抛解压意图；状态机 Idle → Listing → Ready / NeedPassword / Failed。

use crate::app::{PASSWORD_INCORRECT_MARKER, PASSWORD_REQUIRED_MARKER};
use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::archive_tree::{breadcrumb_paths, build_dir_rows, direct_children, TreeRow};
use egui_phosphor_icons::icons;
use openitgo_parser::archive::{list_entries, read_entry, ArchiveEntry};
use openitgo_parser::traits::ParseError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 预览读取上限：超过该大小的条目不读取（防爆内存）。
const PREVIEW_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 文本嗅探只取条目内容的前 256KB。
const TEXT_SNIFF_BYTES: usize = 256 * 1024;
/// 文本预览展示的字符数上限。
const TEXT_PREVIEW_MAX_CHARS: usize = 64_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArchiveViewState {
    #[default]
    Idle,
    Listing,
    Ready,
    /// 列表遇到 PasswordRequired/Incorrect；点击「输入密码」走密码对话框。
    NeedPassword,
    Failed(String),
}

/// 后台线程产出的预览内容。
#[derive(Debug, Clone)]
enum PreviewData {
    Image(egui::ColorImage),
    Text(String),
    Unsupported,
    /// 附带说明（过大/解码失败等）。
    Note(String),
}

pub struct ArchiveView {
    pub path: Option<PathBuf>,
    pub entries: Vec<ArchiveEntry>,
    /// 选中的条目名（仅文件条目可勾选）。
    pub selected: HashSet<String>,
    pub filter: String,
    pub state: ArchiveViewState,
    /// open_with_password 使用的密码；列表成功后由 app 取走记入密码本。
    pub tried_password: Option<String>,
    /// 带密码列目录仍遇密码错误：app 据此以 incorrect 复现密码对话框。
    pub password_failed: bool,
    listing: Option<AsyncOpener<Vec<ArchiveEntry>>>,
    /// 目录树中折叠的目录 full_path（`/` 分隔归一化路径）。
    pub collapsed: HashSet<String>,
    /// 中栏当前目录（`/` 分隔归一化路径）；None = 「全部文件」扁平模式。
    pub current_dir: Option<String>,
    /// 右侧预览面板开关。
    pub preview_open: bool,
    /// 当前预览目标条目名（单击文件条目设置）。
    preview_entry: Option<String>,
    /// 已为该条目发起过后台读取（避免每帧重复 spawn）。
    preview_requested: Option<String>,
    preview: Option<AsyncOpener<PreviewData>>,
    /// poll 拿到、待 ui() 上传为纹理的图片。
    pending_preview_image: Option<egui::ColorImage>,
    preview_tex: Option<egui::TextureHandle>,
    preview_text: Option<String>,
    preview_note: Option<String>,
    /// 会话密码（app 每帧从会话密码表写入），预览读取加密条目用。
    pub preview_password: Option<String>,
}

impl Default for ArchiveView {
    fn default() -> Self {
        Self {
            path: None,
            entries: Vec::new(),
            selected: HashSet::new(),
            filter: String::new(),
            state: ArchiveViewState::Idle,
            tried_password: None,
            password_failed: false,
            listing: None,
            collapsed: HashSet::new(),
            current_dir: None,
            preview_open: false,
            preview_entry: None,
            preview_requested: None,
            preview: None,
            pending_preview_image: None,
            preview_tex: None,
            preview_text: None,
            preview_note: None,
            preview_password: None,
        }
    }
}

pub struct ArchiveCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    /// 「作为漫画打开」：仅当包本身是支持的漫画格式（zip/cbz/rar/cbr）时展示。
    pub on_open_as_comic: &'a mut dyn FnMut(),
    /// 双击图片条目：以漫画打开并定位到该页。
    pub on_open_entry_as_comic: &'a mut dyn FnMut(String),
    pub on_extract_all: &'a mut dyn FnMut(),
    pub on_extract_selected: &'a mut dyn FnMut(Vec<String>),
    /// NeedPassword 状态下点击「输入密码」。
    pub on_need_password: &'a mut dyn FnMut(),
}

impl ArchiveView {
    /// 清空条目相关状态（选择/过滤/折叠/当前目录/预览），供 open* 系列复用。
    fn clear_entries_state(&mut self) {
        self.entries.clear();
        self.selected.clear();
        self.filter.clear();
        self.collapsed.clear();
        self.current_dir = None;
        self.preview_entry = None;
        self.preview_requested = None;
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
    }

    /// 后台列出条目（不带密码；加密包落入 NeedPassword 状态）。
    pub fn open(&mut self, path: PathBuf) {
        self.path = Some(path.clone());
        self.clear_entries_state();
        self.state = ArchiveViewState::Listing;
        self.tried_password = None;
        self.password_failed = false;
        self.listing = Some(AsyncOpener::open(path, |p| {
            list_entries(p, None).map_err(|e| match e {
                ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                other => other.to_string(),
            })
        }));
    }

    /// 带密码重跑列目录（密码对话框确认后调用）。
    pub fn open_with_password(&mut self, path: PathBuf, password: String) {
        self.path = Some(path.clone());
        self.clear_entries_state();
        self.state = ArchiveViewState::Listing;
        self.password_failed = false;
        self.tried_password = Some(password.clone());
        self.listing = Some(AsyncOpener::open(path, move |p| {
            list_entries(p, Some(&password)).map_err(|e| match e {
                ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                other => other.to_string(),
            })
        }));
    }

    /// 直接以已列好的条目进入 Ready（启发式分流复用列目录结果，不重复列出）。
    pub fn open_with_entries(&mut self, path: PathBuf, entries: Vec<ArchiveEntry>) {
        self.path = Some(path);
        self.clear_entries_state();
        self.entries = entries;
        self.state = ArchiveViewState::Ready;
        self.tried_password = None;
        self.password_failed = false;
        self.listing = None;
    }

    /// 每帧排空列表与预览结果；视图不处于前台时结果留在通道里，
    /// 下次 open 直接替换。
    pub fn poll(&mut self) {
        if let Some(mut listing) = self.listing.take() {
            match listing.poll() {
                OpenStatus::Loading => self.listing = Some(listing),
                OpenStatus::Ready(result) => self.apply_listing_result(result),
            }
        }
        self.poll_preview();
    }

    fn apply_listing_result(&mut self, result: Result<Vec<ArchiveEntry>, String>) {
        match result {
            Ok(entries) => {
                self.entries = entries;
                self.state = ArchiveViewState::Ready;
                self.password_failed = false;
            }
            Err(e) if e == PASSWORD_REQUIRED_MARKER || e == PASSWORD_INCORRECT_MARKER => {
                // 带密码尝试后仍失败 = 密码错误，对话框以 incorrect 复现。
                self.password_failed = self.tried_password.is_some();
                self.state = ArchiveViewState::NeedPassword;
            }
            Err(e) => self.state = ArchiveViewState::Failed(e),
        }
    }

    /// 预览目标变化时发起后台读取，并排空预览结果。
    fn poll_preview(&mut self) {
        if self.preview_open && self.preview_entry != self.preview_requested {
            self.start_preview();
        }
        let Some(mut preview) = self.preview.take() else {
            return;
        };
        match preview.poll() {
            OpenStatus::Loading => self.preview = Some(preview),
            OpenStatus::Ready(result) => self.apply_preview_result(result),
        }
    }

    /// 为 preview_entry 发起后台读取；目标变化时丢弃旧纹理与旧结果。
    fn start_preview(&mut self) {
        self.preview_requested = self.preview_entry.clone();
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        let (Some(path), Some(name)) = (self.path.clone(), self.preview_entry.clone()) else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| !e.is_dir && e.name == name) else {
            return;
        };
        let size = entry.size;
        let password = self.preview_password.clone();
        self.preview = Some(AsyncOpener::open(path, move |p| {
            load_preview(p, &name, size, password.as_deref())
        }));
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

    /// 目录的全部后代文件条目名（原始名；`/` 与 `\\` 都算边界，
    /// 先剥掉目录路径尾部斜杠再加边界匹配，`a` 不会误中 `ab/`）。
    fn descendant_file_names(&self, dir_full_path: &str) -> Vec<String> {
        let dir = dir_full_path.trim_end_matches(['/', '\\']);
        let prefix_slash = format!("{dir}/");
        let prefix_backslash = format!("{dir}\\");
        self.entries
            .iter()
            .filter(|e| {
                !e.is_dir
                    && (e.name.starts_with(&prefix_slash) || e.name.starts_with(&prefix_backslash))
            })
            .map(|e| e.name.clone())
            .collect()
    }

    /// 目录勾选态：有后代文件且全部被选中。
    fn dir_all_selected(&self, dir_full_path: &str) -> bool {
        let names = self.descendant_file_names(dir_full_path);
        !names.is_empty() && names.iter().all(|n| self.selected.contains(n))
    }

    /// 目录勾选级联：勾选/取消勾选其全部后代文件条目。
    fn cascade_set(&mut self, dir_full_path: &str, checked: bool) {
        for name in self.descendant_file_names(dir_full_path) {
            if checked {
                self.selected.insert(name);
            } else {
                self.selected.remove(&name);
            }
        }
    }

    /// 当前可见文件条目索引（中栏文件列表与「全选」共用）：
    /// 过滤激活时忽略 current_dir 全包子串匹配（大小写不敏感）；
    /// 否则 current_dir None = 全部文件，Some(dir) = dir 的直接子文件。
    fn visible_file_indices(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        if !needle.is_empty() {
            return self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.is_dir && e.name.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect();
        }
        direct_children(&self.entries, self.current_dir.as_deref()).0
    }

    /// 选中文件条目数与总大小（解压后字节）。
    fn selected_stats(&self) -> (usize, u64) {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && self.selected.contains(&e.name))
            .fold((0, 0), |(n, bytes), e| (n + 1, bytes + e.size))
    }

    /// 全选：选中当前可见的全部文件条目。
    fn select_all_visible_files(&mut self) {
        for idx in self.visible_file_indices() {
            self.selected.insert(self.entries[idx].name.clone());
        }
    }

    /// 选中的条目名，按包内顺序排列。
    fn selected_names_in_order(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, callbacks: ArchiveCallbacks<'_>) {
        let ArchiveCallbacks {
            on_back,
            on_open_as_comic,
            on_open_entry_as_comic,
            on_extract_all,
            on_extract_selected,
            on_need_password,
        } = callbacks;

        ui.horizontal(|ui| {
            if ui
                .button((icons::HOUSE, " 书架"))
                .on_hover_text("返回书架")
                .clicked()
            {
                on_back();
            }
            ui.separator();
            if self
                .path
                .as_deref()
                .is_some_and(crate::app::is_supported_comic_file)
                && ui
                    .button((icons::BOOK_OPEN, " 作为漫画打开"))
                    .on_hover_text("用漫画阅读器打开该压缩包")
                    .clicked()
            {
                on_open_as_comic();
            }
            if self.state == ArchiveViewState::Ready {
                ui.separator();
                if ui
                    .add(egui::Button::new((icons::EYE, " 预览")).selected(self.preview_open))
                    .on_hover_text("切换右侧预览面板")
                    .clicked()
                {
                    self.preview_open = !self.preview_open;
                }
            }
            if let Some(path) = &self.path {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.label(egui::RichText::new(name).strong())
                    .on_hover_text(path.display().to_string());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("过滤条目")
                        .desired_width(160.0),
                );
                if self.state == ArchiveViewState::Ready {
                    if ui.button("清空选中").clicked() {
                        self.selected.clear();
                    }
                    if ui.button("全选").clicked() {
                        self.select_all_visible_files();
                    }
                }
            });
        });
        ui.separator();

        let ready = self.state == ArchiveViewState::Ready;
        if ready {
            egui::Panel::bottom("archive_bottom_bar").show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let file_count = self.entries.iter().filter(|e| !e.is_dir).count();
                    let (selected_count, selected_bytes) = self.selected_stats();
                    ui.label(format!("共 {file_count} 个文件"));
                    ui.separator();
                    ui.label(format!(
                        "已选 {selected_count} 项 · {}",
                        human_size(selected_bytes)
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button((icons::EXPORT, " 解压全部"))
                            .on_hover_text("解压到包同目录的同名子目录")
                            .clicked()
                        {
                            on_extract_all();
                        }
                        let selected_button =
                            egui::Button::new(format!("解压选中 ({selected_count})"));
                        if ui
                            .add_enabled(selected_count > 0, selected_button)
                            .clicked()
                        {
                            on_extract_selected(self.selected_names_in_order());
                        }
                    });
                });
                ui.add_space(4.0);
            });
            if self.preview_open {
                egui::Panel::right("archive_preview")
                    .default_size(280.0)
                    .show(ui, |ui| {
                        self.render_preview(ui);
                    });
            }
            egui::Panel::left("archive_tree_pane")
                .default_size(180.0)
                .show(ui, |ui| {
                    self.render_dir_pane(ui);
                });
            // 预览在途时主动重绘以排空后台读取结果。
            if self.preview.is_some() {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        }

        match &self.state {
            ArchiveViewState::Idle => {}
            ArchiveViewState::Listing => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("⏳").size(24.0));
                    ui.label("正在读取压缩包…");
                });
            }
            ArchiveViewState::NeedPassword => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::LOCK.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("该压缩包已加密，需要密码").size(16.0));
                    ui.label(egui::RichText::new("输入密码后可浏览条目并解压").weak());
                    ui.add_space(8.0);
                    if ui.button((icons::KEY, " 输入密码")).clicked() {
                        on_need_password();
                    }
                });
            }
            ArchiveViewState::Failed(message) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::WARNING.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("无法读取压缩包: {message}"),
                    );
                });
            }
            ArchiveViewState::Ready => {
                self.render_breadcrumb(ui);
                self.render_file_list(ui, on_open_entry_as_comic);
            }
        }
    }

    /// 左栏目录树：特殊根节点「全部文件」+ 仅目录节点（折叠三角 /
    /// 级联勾选 / 单击设为当前目录 / 当前目录高亮）。
    fn render_dir_pane(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let all_selected = self.current_dir.is_none();
                if ui
                    .selectable_label(all_selected, (icons::FILES, " 全部文件"))
                    .clicked()
                {
                    self.current_dir = None;
                }
                let rows = build_dir_rows(&self.entries, &self.collapsed);
                for row in rows {
                    ui.horizontal(|ui| {
                        ui.add_space(row.depth as f32 * 16.0);
                        self.dir_pane_row(ui, &row);
                    });
                }
            });
    }

    /// 目录树行：折叠三角 + 级联勾选框 + 可单击的目录名（高亮当前目录）。
    fn dir_pane_row(&mut self, ui: &mut egui::Ui, row: &TreeRow) {
        if row.has_children {
            let is_collapsed = self.collapsed.contains(&row.full_path);
            let triangle = if is_collapsed { "▸" } else { "▾" };
            if ui
                .add(egui::Button::new(triangle).frame(false))
                .on_hover_text("展开/折叠")
                .clicked()
            {
                if is_collapsed {
                    self.collapsed.remove(&row.full_path);
                } else {
                    self.collapsed.insert(row.full_path.clone());
                }
            }
        } else {
            ui.add_space(16.0);
        }
        let mut now = self.dir_all_selected(&row.full_path);
        if ui.checkbox(&mut now, "").clicked() {
            self.cascade_set(&row.full_path, now);
        }
        let is_current = self.current_dir.as_deref() == Some(row.full_path.as_str());
        if ui
            .selectable_label(
                is_current,
                format!("{} {}", icons::FOLDER.as_str(), row.name),
            )
            .clicked()
        {
            self.current_dir = Some(row.full_path.clone());
        }
    }

    /// 面包屑：「全部文件 / dir1 / dir2」，每段可点击跳回；
    /// 仅 current_dir 非 None 时显示路径段。
    fn render_breadcrumb(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(self.current_dir.is_none(), "全部文件")
                .clicked()
            {
                self.current_dir = None;
            }
            if let Some(dir) = self.current_dir.clone() {
                for path in breadcrumb_paths(&dir) {
                    let label = path.rsplit('/').next().unwrap_or(&path).to_string();
                    ui.label(egui::RichText::new("/").weak());
                    let is_current = path == dir;
                    if ui.selectable_label(is_current, label).clicked() {
                        self.current_dir = Some(path);
                    }
                }
            }
        });
        ui.separator();
    }

    /// 中栏文件列表：过滤激活时全包匹配（忽略 current_dir）；
    /// current_dir None = 全部文件扁平展示；Some(dir) = 子目录行
    /// （不可勾选，双击进入）+ 直接子文件行。
    fn render_file_list(
        &mut self,
        ui: &mut egui::Ui,
        on_open_entry_as_comic: &mut dyn FnMut(String),
    ) {
        let filter_active = !self.filter.trim().is_empty();
        let (files, subdirs) = if filter_active {
            (self.visible_file_indices(), Vec::new())
        } else {
            direct_children(&self.entries, self.current_dir.as_deref())
        };
        if files.is_empty() && subdirs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(egui::RichText::new("没有匹配的条目").weak());
            });
            return;
        }
        let openable_comic = self
            .path
            .as_deref()
            .is_some_and(crate::app::is_supported_comic_file);
        let flat = filter_active || self.current_dir.is_none();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for subdir in subdirs {
                    ui.horizontal(|ui| {
                        // 子目录行不可勾选，占位对齐文件行。
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new(icons::FOLDER.as_str()).weak());
                        let label = ui.add(egui::Label::new(&subdir).sense(egui::Sense::click()));
                        if label.double_clicked() {
                            if let Some(dir) = &self.current_dir {
                                self.current_dir = Some(format!("{dir}/{subdir}"));
                            }
                        }
                        label.on_hover_text("双击进入该目录");
                    });
                }
                for idx in files {
                    let name = self.entries[idx].name.clone();
                    // 目录模式下展示 basename，扁平/过滤模式展示完整路径名。
                    let display = if flat {
                        name.clone()
                    } else {
                        name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string()
                    };
                    ui.horizontal(|ui| {
                        self.file_entry_row(
                            ui,
                            idx,
                            &display,
                            openable_comic,
                            on_open_entry_as_comic,
                        );
                    });
                }
            });
    }

    /// 文件条目行：勾选框、可单击/双击的文件名、大小。
    /// `display` 为展示名（目录模式下为 basename），选择/预览/解压身份恒用
    /// 原始 entry.name。
    fn file_entry_row(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        display: &str,
        openable_comic: bool,
        on_open_entry_as_comic: &mut dyn FnMut(String),
    ) {
        let entry = &self.entries[idx];
        let name = entry.name.clone();
        let mut now = self.selected.contains(&name);
        if ui.checkbox(&mut now, "").clicked() {
            if now {
                self.selected.insert(name.clone());
            } else {
                self.selected.remove(&name);
            }
        }
        ui.label(icons::FILE);
        let label = ui.add(egui::Label::new(display).sense(egui::Sense::click()));
        if label.clicked() {
            self.preview_entry = Some(name.clone());
        }
        if openable_comic
            && label.double_clicked()
            && openitgo_parser::traits::is_comic_image_name(&name)
        {
            on_open_entry_as_comic(name.clone());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(human_size(entry.size)).weak());
            if let Some(compressed) = entry.compressed_size {
                ui.label(egui::RichText::new(format!("压缩 {}", human_size(compressed))).weak());
            }
        });
    }

    /// 右侧预览面板内容：条目名/大小 + 图片纹理 / 只读文本 / 说明。
    fn render_preview(&mut self, ui: &mut egui::Ui) {
        // poll 收到的 ColorImage 在此（有 ctx）惰性上传为纹理。
        if let Some(img) = self.pending_preview_image.take() {
            self.preview_tex = Some(ui.ctx().load_texture(
                "archive-preview",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Some(name) = self.preview_entry.clone() else {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("单击条目以预览").weak());
                    return;
                };
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&name).strong());
                if let Some(entry) = self.entries.iter().find(|e| e.name == name) {
                    ui.label(egui::RichText::new(human_size(entry.size)).weak());
                }
                ui.separator();
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
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
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

/// 后台读取并分类预览内容：图片解码、UTF-8 文本（嗅探前 256KB）、
/// 其余不支持；超过 64MB 的条目不读取。
fn load_preview(
    path: &Path,
    entry_name: &str,
    size: u64,
    password: Option<&str>,
) -> Result<PreviewData, String> {
    if size > PREVIEW_MAX_BYTES {
        return Ok(PreviewData::Note("文件过大，不预览".to_string()));
    }
    let bytes = read_entry(path, entry_name, password).map_err(|e| e.to_string())?;
    Ok(classify_preview_bytes(entry_name, &bytes))
}

/// 按扩展名与内容嗅探分类已读出的字节（纯函数，便于单测）。
fn classify_preview_bytes(name: &str, bytes: &[u8]) -> PreviewData {
    let is_image = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(openitgo_parser::traits::is_image_extension);
    if is_image {
        return match image::load_from_memory(bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                PreviewData::Image(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
            }
            Err(e) => PreviewData::Note(format!("无法解码图片: {e}")),
        };
    }
    let prefix = &bytes[..bytes.len().min(TEXT_SNIFF_BYTES)];
    let text = match std::str::from_utf8(prefix) {
        Ok(s) => Some(s),
        Err(e) if e.valid_up_to() > 0 => std::str::from_utf8(&prefix[..e.valid_up_to()]).ok(),
        Err(_) => None,
    };
    match text {
        Some(s) if !s.contains('\0') => {
            let mut truncated: String = s.chars().take(TEXT_PREVIEW_MAX_CHARS).collect();
            if s.chars().count() > TEXT_PREVIEW_MAX_CHARS {
                truncated.push_str("\n…（内容过长，已截断）");
            }
            PreviewData::Text(truncated)
        }
        _ => PreviewData::Unsupported,
    }
}

/// 人类可读大小：B / KB / MB / GB。
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size,
            compressed_size: None,
        }
    }

    #[test]
    fn apply_listing_result_state_transitions() {
        let mut view = ArchiveView::default();
        view.apply_listing_result(Ok(vec![entry("a.txt", false, 1)]));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert_eq!(view.entries.len(), 1);

        view.apply_listing_result(Err(PASSWORD_REQUIRED_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);

        view.apply_listing_result(Err("IO error: x".to_string()));
        assert_eq!(
            view.state,
            ArchiveViewState::Failed("IO error: x".to_string())
        );
    }

    #[test]
    fn apply_listing_result_tracks_password_attempt_outcome() {
        let mut view = ArchiveView::default();
        // 无密码尝试时的密码错误：不触发 incorrect 复现。
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(!view.password_failed);
        // 带密码尝试后仍失败：password_failed 置位，tried_password 保留。
        view.tried_password = Some("pw".to_string());
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(view.password_failed);
        // 带密码尝试成功：Ready 且 password_failed 清除。
        view.apply_listing_result(Ok(vec![entry("a.txt", false, 1)]));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert!(!view.password_failed);
        assert_eq!(view.tried_password.as_deref(), Some("pw"));
    }

    #[test]
    fn visible_file_indices_respects_filter_and_current_dir() {
        let mut view = ArchiveView {
            entries: vec![
                entry("Dir/", true, 0),
                entry("Page01.PNG", false, 10),
                entry("notes.md", false, 5),
                entry("Dir/inner.png", false, 3),
            ],
            ..Default::default()
        };
        // 默认「全部文件」扁平模式：全部文件条目，目录条目不参与。
        assert_eq!(view.visible_file_indices(), vec![1, 2, 3]);
        view.filter = "png".to_string();
        assert_eq!(view.visible_file_indices(), vec![1, 3]);
        view.filter = " PAGE ".to_string();
        assert_eq!(view.visible_file_indices(), vec![1]);
        // 进入目录：仅直接子文件。
        view.filter.clear();
        view.current_dir = Some("Dir".to_string());
        assert_eq!(view.visible_file_indices(), vec![3]);
        // 过滤激活时忽略 current_dir，全包搜索。
        view.filter = "page".to_string();
        assert_eq!(view.visible_file_indices(), vec![1]);
    }

    #[test]
    fn select_all_visible_files_skips_directories() {
        let mut view = ArchiveView {
            entries: vec![
                entry("dir/", true, 0),
                entry("dir/b.png", false, 9),
                entry("a.txt", false, 7),
            ],
            ..Default::default()
        };
        view.select_all_visible_files();
        assert_eq!(view.selected.len(), 2);
        assert!(!view.selected.contains("dir/"));
        // 过滤后全选只选可见文件。
        view.selected.clear();
        view.filter = "png".to_string();
        view.select_all_visible_files();
        assert_eq!(view.selected.len(), 1);
        assert!(view.selected.contains("dir/b.png"));
    }

    #[test]
    fn selected_stats_and_order() {
        let mut view = ArchiveView {
            entries: vec![
                entry("z.png", false, 3),
                entry("dir/", true, 0),
                entry("a.png", false, 4),
            ],
            ..Default::default()
        };
        view.selected.insert("a.png".to_string());
        view.selected.insert("z.png".to_string());
        assert_eq!(view.selected_stats(), (2, 7));
        // 按包内顺序而非插入顺序。
        assert_eq!(view.selected_names_in_order(), vec!["z.png", "a.png"]);
    }

    #[test]
    fn human_size_formats_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn cascade_set_selects_descendants_with_boundary() {
        let mut view = ArchiveView {
            entries: vec![
                entry("a/b.png", false, 1),
                entry("a/sub/c.png", false, 1),
                entry("ab/d.png", false, 1),
                entry("a.txt", false, 1),
            ],
            ..Default::default()
        };
        view.cascade_set("a", true);
        assert!(view.selected.contains("a/b.png"));
        assert!(view.selected.contains("a/sub/c.png"));
        // 边界：`a` 不得误中 `ab/` 与同名文件 `a.txt`。
        assert!(!view.selected.contains("ab/d.png"));
        assert!(!view.selected.contains("a.txt"));
        assert!(view.dir_all_selected("a"));
        view.cascade_set("a", false);
        assert!(view.selected.is_empty());
        assert!(!view.dir_all_selected("a"));
    }

    #[test]
    fn cascade_set_accepts_backslash_and_trailing_slash_paths() {
        let mut view = ArchiveView {
            entries: vec![entry("a\\b.png", false, 1), entry("a/c.png", false, 1)],
            ..Default::default()
        };
        // 尾部斜杠剥掉后照常匹配；反斜杠条目也命中。
        view.cascade_set("a/", true);
        assert_eq!(view.selected.len(), 2);
    }

    #[test]
    fn open_with_entries_sets_ready_and_resets_state() {
        let mut view = ArchiveView {
            selected: HashSet::from(["old.png".to_string()]),
            filter: "old".to_string(),
            collapsed: HashSet::from(["d".to_string()]),
            current_dir: Some("d".to_string()),
            preview_entry: Some("old.png".to_string()),
            preview_text: Some("txt".to_string()),
            tried_password: Some("pw".to_string()),
            password_failed: true,
            state: ArchiveViewState::Failed("x".to_string()),
            ..Default::default()
        };
        view.open_with_entries(
            PathBuf::from("/tmp/pack.zip"),
            vec![entry("a.png", false, 1)],
        );
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert_eq!(view.entries.len(), 1);
        assert!(view.selected.is_empty());
        assert!(view.filter.is_empty());
        assert!(view.collapsed.is_empty());
        assert_eq!(view.current_dir, None);
        assert_eq!(view.preview_entry, None);
        assert_eq!(view.preview_text, None);
        assert_eq!(view.tried_password, None);
        assert!(!view.password_failed);
        assert!(view.listing.is_none());
        assert_eq!(view.path.as_deref(), Some(Path::new("/tmp/pack.zip")));
    }

    #[test]
    fn poll_preview_spawns_only_for_open_panel_and_new_target() {
        let mut view = ArchiveView {
            path: Some(PathBuf::from("/tmp/pack.zip")),
            entries: vec![entry("a.txt", false, 1)],
            state: ArchiveViewState::Ready,
            preview_entry: Some("a.txt".to_string()),
            ..Default::default()
        };
        // 面板关闭时不发起读取。
        view.poll();
        assert!(view.preview.is_none());
        assert_eq!(view.preview_requested, None);
        // 面板打开后发起读取并记录目标。
        view.preview_open = true;
        view.poll();
        assert!(view.preview.is_some());
        assert_eq!(view.preview_requested.as_deref(), Some("a.txt"));
    }

    #[test]
    fn classify_preview_bytes_decodes_image() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        match classify_preview_bytes("p.png", &buf.into_inner()) {
            PreviewData::Image(ci) => assert_eq!(ci.size, [2, 2]),
            other => panic!("expected Image, got {other:?}"),
        }
        // 扩展名是图片但内容不可解码 → 说明。
        assert!(matches!(
            classify_preview_bytes("p.png", b"not a png"),
            PreviewData::Note(_)
        ));
    }

    #[test]
    fn classify_preview_bytes_text_and_unsupported() {
        match classify_preview_bytes("notes.txt", "你好 world".as_bytes()) {
            PreviewData::Text(t) => assert_eq!(t, "你好 world"),
            other => panic!("expected Text, got {other:?}"),
        }
        // 含 NUL → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", b"ab\0cd"),
            PreviewData::Unsupported
        ));
        // 起始即非法 UTF-8 → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", &[0xFF, 0xFE, 0x00]),
            PreviewData::Unsupported
        ));
        // 前段是合法 UTF-8、后段非法 → 取合法前缀。
        match classify_preview_bytes("a.txt", &[b'a', b'b', 0xFF]) {
            PreviewData::Text(t) => assert_eq!(t, "ab"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_preview_bytes_truncates_long_text() {
        let long = "x".repeat(TEXT_PREVIEW_MAX_CHARS + 10_000);
        match classify_preview_bytes("a.txt", long.as_bytes()) {
            PreviewData::Text(t) => {
                assert!(t.ends_with("（内容过长，已截断）"));
                assert!(t.chars().count() < long.chars().count());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }
}
