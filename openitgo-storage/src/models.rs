use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 文件管理器常用目录书签分组（两栏共享；空分组保留，用户可能建空组备用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FmBookmarkGroup {
    /// 分组名（clamp 时空名修「未命名」）。
    pub name: String,
    /// 组内书签目录（clamp 时组内去重、去空串）。
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub double_page: bool,
    pub wide_page_threshold: f32,
    pub enable_page_animation: bool,
    pub compress_images: bool,
    pub decode_threads: u32,
    pub cache_size_mb: u32,
    pub real_image_cache_pages: u32,
    pub window_size: (f32, f32),
    /// Outer window position (top-left). `None` = let the OS place the window.
    #[serde(default)]
    pub window_pos: Option<(f32, f32)>,
    /// Whether the window was maximized when last saved (non-fullscreen).
    #[serde(default)]
    pub window_maximized: bool,
    pub show_toolbar: bool,
    pub show_statusbar: bool,
    pub invert_scroll: bool,
    /// 滚轮翻页触发阈值（pt）：累计滚动这么多距离翻一页。越小越灵敏。
    pub page_scroll_threshold: f32,
    /// 阅读界面工具栏 / 状态栏（进度条）半透明程度，1.0 = 不透明。
    #[serde(default = "default_chrome_opacity")]
    pub chrome_opacity: f32,
    pub background_color: [u8; 3],
    pub shortcuts: Shortcuts,
    pub library_sort: LibrarySort,
    pub toolbar_display_mode: ToolbarDisplayMode,
    pub ebook: EbookSettings,
    pub media_volume: f64,
    pub media_speed: f64,
    pub media_audio_device: String,
    /// 漫画翻到末页后再按「下一页」时的行为。
    #[serde(default)]
    pub comic_end_action: ComicEndAction,
    /// 媒体播放到结尾时的行为。
    #[serde(default)]
    pub media_end_action: MediaEndAction,
    /// 解压输出目录；空 = 压缩包同目录下的同名子目录。
    #[serde(default)]
    pub extract_dir: String,
    /// 解压并行线程数，0 = 自动。
    #[serde(default)]
    pub extract_threads: u32,
    /// 解压时同名文件是否覆盖（false = 自动改名 "name (1).ext"）。
    #[serde(default)]
    pub extract_overwrite: bool,
    /// 解压对话框的子目录策略："smart"（包内无单一顶层目录才建包名子目录）/
    /// "always"（总是建包名子目录）/ "never"（直接解压进目标文件夹）。
    #[serde(default = "default_extract_wrap")]
    pub extract_wrap: String,
    /// 解压完成后删除压缩包（移入回收站）。
    #[serde(default)]
    pub extract_delete_archive: bool,
    /// 解压完成后打开目标文件夹。
    #[serde(default)]
    pub extract_open_folder: bool,
    /// 文件管理器布局："dual"（双栏）| "single"（单栏+预览）。
    #[serde(default = "default_fm_layout")]
    pub fm_layout: String,
    /// 双栏模式左栏宽度比例。
    #[serde(default = "default_fm_dual_ratio")]
    pub fm_dual_ratio: f32,
    /// 单栏模式预览面板开关。
    #[serde(default = "default_true")]
    pub fm_preview_open: bool,
    /// 文件管理器排序键："name"|"size"|"mtime"。
    #[serde(default = "default_fm_sort_key")]
    pub fm_sort_key: String,
    /// 文件管理器视图模式："list"|"thumbs"（全局单值，恢复时两栏同用；
    /// 取舍同 fm_sort_key——持久化活动栏的模式）。
    #[serde(default = "default_fm_view_mode")]
    pub fm_view_mode: String,
    #[serde(default = "default_true")]
    pub fm_sort_asc: bool,
    /// 删除前确认（防误删）。
    #[serde(default = "default_true")]
    pub fm_confirm_delete: bool,
    /// 显示隐藏文件（`.` 开头 / Windows 隐藏属性）。
    #[serde(default = "default_true")]
    pub fm_show_hidden: bool,
    /// 左/右栏持久化目录；空 = 用户主目录。
    #[serde(default)]
    pub fm_dir_left: String,
    #[serde(default)]
    pub fm_dir_right: String,
    /// 旧版扁平常用目录书签：仅作读取兼容（阶段 N 起迁移进
    /// `fm_bookmark_groups`），clamp 迁移后清空，保存时不再写出。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fm_bookmarks: Vec<String>,
    /// 常用目录书签分组（两栏共享）。
    #[serde(default)]
    pub fm_bookmark_groups: Vec<FmBookmarkGroup>,
    /// 左/右栏标签页目录列表（活动标签 = 当前目录；只存目录路径）。
    /// 空列表（旧 settings 无此字段）= 单标签，恢复回退 fm_dir_left/right。
    #[serde(default)]
    pub fm_tabs_left: Vec<String>,
    #[serde(default)]
    pub fm_tabs_right: Vec<String>,
    /// 左/右栏活动标签索引（越界加载时 clamp）。
    #[serde(default)]
    pub fm_active_tab_left: usize,
    #[serde(default)]
    pub fm_active_tab_right: usize,
    /// 文件管理器大小/时间列宽与列块平移量（全局单值，恢复时两栏同用；
    /// 取舍同 fm_sort_key——持久化活动栏的值）。默认值同 panel.rs 的
    /// SIZE_COL_WIDTH/MTIME_COL_WIDTH/0（storage 不依赖 app，数值硬编码同步）。
    #[serde(default = "default_fm_col_size_width")]
    pub fm_col_size_width: f32,
    #[serde(default = "default_fm_col_mtime_width")]
    pub fm_col_mtime_width: f32,
    /// 列块平移量（≤0；0 = 列块贴右缘）。
    #[serde(default)]
    pub fm_col_shift: f32,
    /// 删除方式："trash"（移入回收站，默认）| "permanent"（永久删除）。
    #[serde(default = "default_fm_delete_mode")]
    pub fm_delete_mode: String,
    /// 空格行为："dir_size"（计算目录大小，默认）| "toggle_select"
    /// （TC 勾选语义：切换焦点项选中并下移；Insert 键无条件同为勾选下移）。
    #[serde(default = "default_fm_space_action")]
    pub fm_space_action: String,
    /// 目录恒排在文件前（false = 目录文件混排、统一排序）。
    #[serde(default = "default_true")]
    pub fm_dirs_first: bool,
    /// 栏间拖放复制前弹确认框（false = 松开直拷自动改名，按住 Shift = 移动）。
    #[serde(default = "default_true")]
    pub fm_drag_confirm: bool,
    /// 双击压缩包分发："archive"（Archive 视图，默认）| "comic"（作为漫画打开）
    /// | "ask"（鼠标处弹小菜单二选一）。
    #[serde(default = "default_fm_archive_open")]
    pub fm_archive_open: String,
    /// Esc 不清选中（true = Esc 链：关弹层 → 清 type-ahead → 清过滤，选中保留）。
    #[serde(default)]
    pub fm_esc_keep_selection: bool,
    /// 列表/网格空白区双击 = 回上级目录（默认 true，TC 可配惯例）。
    #[serde(default = "default_true")]
    pub fm_dblclick_blank_up: bool,
    /// 文件管理器显示系统真实图标（SHGetFileInfoW；false = 字体图标）。
    #[serde(default = "default_true")]
    pub fm_system_icons: bool,
    /// 保存的过滤方案（过滤框下拉列出；clamp 去空去重）。
    #[serde(default)]
    pub fm_saved_filters: Vec<String>,
    /// 过滤框渲染在焦点栏列表底部（false = 顶栏右侧）。
    #[serde(default)]
    pub fm_filter_bar_bottom: bool,
    /// 鼠标框选："right"（右键拖动，默认）| "left"（左键从空白区起拖）
    /// | "off"。
    #[serde(default = "default_fm_rubber_band")]
    pub fm_rubber_band: String,
}

fn default_fm_rubber_band() -> String {
    "right".to_string()
}

fn default_chrome_opacity() -> f32 {
    0.85
}

fn default_extract_wrap() -> String {
    "smart".to_string()
}

fn default_true() -> bool {
    true
}

fn default_fm_layout() -> String {
    "dual".to_string()
}

fn default_fm_dual_ratio() -> f32 {
    0.5
}

fn default_fm_sort_key() -> String {
    "name".to_string()
}

fn default_fm_view_mode() -> String {
    "list".to_string()
}

fn default_fm_delete_mode() -> String {
    "trash".to_string()
}

fn default_fm_space_action() -> String {
    "dir_size".to_string()
}

fn default_fm_archive_open() -> String {
    "archive".to_string()
}

/// 同 openitgo-app views/file_manager_panel.rs 的 SIZE_COL_WIDTH。
fn default_fm_col_size_width() -> f32 {
    90.0
}

/// 同 openitgo-app views/file_manager_panel.rs 的 MTIME_COL_WIDTH。
fn default_fm_col_mtime_width() -> f32 {
    110.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            default_mode: ReadingMode::default(),
            default_fit: FitMode::default(),
            double_page: false,
            wide_page_threshold: 1.4,
            enable_page_animation: true,
            compress_images: false,
            decode_threads: 0,
            cache_size_mb: 1024,
            real_image_cache_pages: 10,
            window_size: (1280.0, 800.0),
            window_pos: None,
            window_maximized: true,
            show_toolbar: true,
            show_statusbar: true,
            invert_scroll: false,
            page_scroll_threshold: 12.0,
            chrome_opacity: default_chrome_opacity(),
            background_color: [30, 30, 30],
            shortcuts: Shortcuts::default(),
            library_sort: LibrarySort::default(),
            toolbar_display_mode: ToolbarDisplayMode::default(),
            ebook: EbookSettings::default(),
            media_volume: 100.0,
            media_speed: 1.0,
            media_audio_device: String::new(),
            comic_end_action: ComicEndAction::default(),
            media_end_action: MediaEndAction::default(),
            extract_dir: String::new(),
            extract_threads: 0,
            extract_overwrite: false,
            extract_wrap: default_extract_wrap(),
            extract_delete_archive: false,
            extract_open_folder: false,
            fm_layout: default_fm_layout(),
            fm_dual_ratio: default_fm_dual_ratio(),
            fm_preview_open: true,
            fm_sort_key: default_fm_sort_key(),
            fm_view_mode: default_fm_view_mode(),
            fm_sort_asc: true,
            fm_confirm_delete: true,
            fm_show_hidden: true,
            fm_dir_left: String::new(),
            fm_dir_right: String::new(),
            fm_bookmarks: Vec::new(),
            fm_bookmark_groups: Vec::new(),
            fm_tabs_left: Vec::new(),
            fm_tabs_right: Vec::new(),
            fm_active_tab_left: 0,
            fm_active_tab_right: 0,
            fm_col_size_width: default_fm_col_size_width(),
            fm_col_mtime_width: default_fm_col_mtime_width(),
            fm_col_shift: 0.0,
            fm_delete_mode: default_fm_delete_mode(),
            fm_space_action: default_fm_space_action(),
            fm_dirs_first: true,
            fm_drag_confirm: true,
            fm_archive_open: default_fm_archive_open(),
            fm_esc_keep_selection: false,
            fm_dblclick_blank_up: true,
            fm_system_icons: true,
            fm_saved_filters: Vec::new(),
            fm_filter_bar_bottom: false,
            fm_rubber_band: default_fm_rubber_band(),
        }
    }
}

impl Settings {
    /// Validate that all numeric fields are within sensible ranges. Returns an
    /// error message describing the first invalid field.
    pub fn validate(&self) -> Result<(), String> {
        if self.decode_threads > 64 {
            return Err(format!(
                "decode_threads must be <= 64, got {}",
                self.decode_threads
            ));
        }
        if !(100..=16384).contains(&self.cache_size_mb) {
            return Err(format!(
                "cache_size_mb must be between 100 and 16384, got {}",
                self.cache_size_mb
            ));
        }
        if !(1..=500).contains(&self.real_image_cache_pages) {
            return Err(format!(
                "real_image_cache_pages must be between 1 and 500, got {}",
                self.real_image_cache_pages
            ));
        }
        if self.wide_page_threshold < 1.0 || self.wide_page_threshold > 3.0 {
            return Err(format!(
                "wide_page_threshold must be between 1.0 and 3.0, got {}",
                self.wide_page_threshold
            ));
        }
        if !(1.0..=40.0).contains(&self.page_scroll_threshold) {
            return Err(format!(
                "page_scroll_threshold must be between 1.0 and 40.0, got {}",
                self.page_scroll_threshold
            ));
        }
        if !(0.2..=1.0).contains(&self.chrome_opacity) {
            return Err(format!(
                "chrome_opacity must be between 0.2 and 1.0, got {}",
                self.chrome_opacity
            ));
        }
        if self.window_size.0 < 400.0 || self.window_size.1 < 300.0 {
            return Err(format!(
                "window_size must be at least 400x300, got {:?}",
                self.window_size
            ));
        }
        if !(10..=72).contains(&self.ebook.font_size) {
            return Err(format!(
                "ebook.font_size must be between 10 and 72, got {}",
                self.ebook.font_size
            ));
        }
        if self.ebook.line_height < 1.0 || self.ebook.line_height > 3.0 {
            return Err(format!(
                "ebook.line_height must be between 1.0 and 3.0, got {}",
                self.ebook.line_height
            ));
        }
        if !(0..=200).contains(&self.ebook.margin_horizontal) {
            return Err(format!(
                "ebook.margin_horizontal must be between 0 and 200, got {}",
                self.ebook.margin_horizontal
            ));
        }
        if !(0..=200).contains(&self.ebook.margin_vertical) {
            return Err(format!(
                "ebook.margin_vertical must be between 0 and 200, got {}",
                self.ebook.margin_vertical
            ));
        }
        if self.ebook.font_family.trim().is_empty() {
            return Err("ebook.font_family must not be empty".to_string());
        }
        if !(0.0..=100.0).contains(&self.media_volume) {
            return Err(format!(
                "media_volume must be between 0 and 100, got {}",
                self.media_volume
            ));
        }
        if !(0.1..=16.0).contains(&self.media_speed) {
            return Err(format!(
                "media_speed must be between 0.1 and 16, got {}",
                self.media_speed
            ));
        }
        if self.extract_threads > 32 {
            return Err(format!(
                "extract_threads must be <= 32, got {}",
                self.extract_threads
            ));
        }
        if !matches!(self.extract_wrap.as_str(), "smart" | "always" | "never") {
            return Err(format!(
                "extract_wrap must be smart/always/never, got {}",
                self.extract_wrap
            ));
        }
        if !(0.2..=0.8).contains(&self.fm_dual_ratio) {
            return Err(format!(
                "fm_dual_ratio must be between 0.2 and 0.8, got {}",
                self.fm_dual_ratio
            ));
        }
        if !matches!(self.fm_layout.as_str(), "dual" | "single") {
            return Err(format!(
                "fm_layout must be dual/single, got {}",
                self.fm_layout
            ));
        }
        if !matches!(self.fm_sort_key.as_str(), "name" | "size" | "mtime" | "ext") {
            return Err(format!(
                "fm_sort_key must be name/size/mtime/ext, got {}",
                self.fm_sort_key
            ));
        }
        if !matches!(self.fm_view_mode.as_str(), "list" | "thumbs") {
            return Err(format!(
                "fm_view_mode must be list/thumbs, got {}",
                self.fm_view_mode
            ));
        }
        if !self.fm_tabs_left.is_empty() && self.fm_active_tab_left >= self.fm_tabs_left.len() {
            return Err(format!(
                "fm_active_tab_left {} out of range ({} tabs)",
                self.fm_active_tab_left,
                self.fm_tabs_left.len()
            ));
        }
        if !self.fm_tabs_right.is_empty() && self.fm_active_tab_right >= self.fm_tabs_right.len() {
            return Err(format!(
                "fm_active_tab_right {} out of range ({} tabs)",
                self.fm_active_tab_right,
                self.fm_tabs_right.len()
            ));
        }
        Ok(())
    }

    /// Clamp all numeric fields to their valid ranges. Used when repairing a
    /// settings file that failed validation.
    pub fn clamp(&mut self) {
        self.decode_threads = self.decode_threads.min(64);
        self.cache_size_mb = self.cache_size_mb.clamp(100, 16384);
        self.real_image_cache_pages = self.real_image_cache_pages.clamp(1, 500);
        self.wide_page_threshold = self.wide_page_threshold.clamp(1.0, 3.0);
        self.page_scroll_threshold = self.page_scroll_threshold.clamp(1.0, 40.0);
        self.chrome_opacity = self.chrome_opacity.clamp(0.2, 1.0);
        self.window_size.0 = self.window_size.0.clamp(400.0, 16384.0);
        self.window_size.1 = self.window_size.1.clamp(300.0, 16384.0);
        self.ebook.font_size = self.ebook.font_size.clamp(10, 72);
        self.ebook.line_height = self.ebook.line_height.clamp(1.0, 3.0);
        self.ebook.margin_horizontal = self.ebook.margin_horizontal.clamp(0, 200);
        self.ebook.margin_vertical = self.ebook.margin_vertical.clamp(0, 200);
        if self.ebook.font_family.trim().is_empty() {
            self.ebook.font_family = "system-ui".to_string();
        }
        self.media_volume = self.media_volume.clamp(0.0, 100.0);
        self.media_speed = self.media_speed.clamp(0.1, 16.0);
        self.extract_threads = self.extract_threads.min(32);
        if !matches!(self.extract_wrap.as_str(), "smart" | "always" | "never") {
            self.extract_wrap = default_extract_wrap();
        }
        self.fm_dual_ratio = self.fm_dual_ratio.clamp(0.2, 0.8);
        if self.fm_layout != "single" {
            self.fm_layout = default_fm_layout();
        }
        if !matches!(self.fm_sort_key.as_str(), "size" | "mtime" | "ext") {
            self.fm_sort_key = default_fm_sort_key();
        }
        if !matches!(self.fm_view_mode.as_str(), "list" | "thumbs") {
            self.fm_view_mode = default_fm_view_mode();
        }
        // 旧版扁平书签 → 分组迁移：非空即并入「常用」组（无此组则新建——
        // 组为空时即「迁移为单个分组」），迁移后清空旧字段（保存时
        // skip_serializing_if 空 Vec，不再写出）。
        if !self.fm_bookmarks.is_empty() {
            let legacy = std::mem::take(&mut self.fm_bookmarks);
            if let Some(g) = self
                .fm_bookmark_groups
                .iter_mut()
                .find(|g| g.name == "常用")
            {
                g.items.extend(legacy);
            } else {
                self.fm_bookmark_groups.push(FmBookmarkGroup {
                    name: "常用".to_string(),
                    items: legacy,
                });
            }
        }
        // 分组校验：空名修「未命名」；组内去空串 + 去重；空分组保留（备用）。
        for group in &mut self.fm_bookmark_groups {
            let name = group.name.trim();
            if name.is_empty() {
                group.name = "未命名".to_string();
            } else if name.len() != group.name.len() {
                group.name = name.to_string();
            }
            let mut seen = std::collections::HashSet::new();
            group
                .items
                .retain(|b| !b.trim().is_empty() && seen.insert(b.clone()));
        }
        self.fm_active_tab_left =
            clamp_active_tab(self.fm_tabs_left.len(), self.fm_active_tab_left);
        self.fm_active_tab_right =
            clamp_active_tab(self.fm_tabs_right.len(), self.fm_active_tab_right);
        // 列宽 clamp 同 panel.rs 的 COL_MIN_WIDTH/COL_MAX_WIDTH；col_shift
        // ≤0，下限取 -(两列宽之和)（panel.rs drag_column_sep 的 min_shift
        // 近似——运行时还会按栏宽再 clamp，此处只防脏值）。
        self.fm_col_size_width = self.fm_col_size_width.clamp(60.0, 400.0);
        self.fm_col_mtime_width = self.fm_col_mtime_width.clamp(60.0, 400.0);
        let min_shift = -(self.fm_col_size_width + self.fm_col_mtime_width);
        self.fm_col_shift = self.fm_col_shift.clamp(min_shift, 0.0);
        if self.fm_delete_mode != "permanent" {
            self.fm_delete_mode = default_fm_delete_mode();
        }
        if self.fm_space_action != "toggle_select" {
            self.fm_space_action = default_fm_space_action();
        }
        if !matches!(self.fm_archive_open.as_str(), "comic" | "ask") {
            self.fm_archive_open = default_fm_archive_open();
        }
        if !matches!(self.fm_rubber_band.as_str(), "left" | "off") {
            self.fm_rubber_band = default_fm_rubber_band();
        }
        // 保存的过滤方案：trim、去空、保序去重。
        let mut seen = std::collections::HashSet::new();
        let mut filters = Vec::with_capacity(self.fm_saved_filters.len());
        for f in std::mem::take(&mut self.fm_saved_filters) {
            let f = f.trim();
            if !f.is_empty() && seen.insert(f.to_string()) {
                filters.push(f.to_string());
            }
        }
        self.fm_saved_filters = filters;
    }
}

/// 活动标签索引 clamp 到标签列表范围内（空列表 → 0）。
fn clamp_active_tab(tab_count: usize, active: usize) -> usize {
    if tab_count == 0 {
        0
    } else {
        active.min(tab_count - 1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Shortcuts {
    pub next_page: Vec<String>,
    pub prev_page: Vec<String>,
    pub first_page: Vec<String>,
    pub last_page: Vec<String>,
    pub page_down: Vec<String>,
    pub page_up: Vec<String>,
    pub fullscreen: Vec<String>,
    pub fit_page: Vec<String>,
    pub fit_width: Vec<String>,
    pub fit_height: Vec<String>,
    pub zoom_in: Vec<String>,
    pub zoom_out: Vec<String>,
    pub back_to_library: Vec<String>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            next_page: vec!["ArrowRight".to_string()],
            prev_page: vec!["ArrowLeft".to_string()],
            first_page: vec!["Home".to_string()],
            last_page: vec!["End".to_string()],
            page_down: vec!["PageDown".to_string(), "Space".to_string()],
            page_up: vec!["PageUp".to_string()],
            fullscreen: vec!["F11".to_string()],
            fit_page: vec!["Num0".to_string()],
            fit_width: vec!["W".to_string()],
            fit_height: vec!["H".to_string()],
            zoom_in: vec!["Plus".to_string(), "Equals".to_string()],
            zoom_out: vec!["Minus".to_string()],
            back_to_library: vec!["Escape".to_string()],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    LastRead,
    Title,
    Added,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarDisplayMode {
    #[default]
    IconAndText,
    IconOnly,
    TextOnly,
}

/// 漫画读到末页后再翻「下一页」时的行为。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComicEndAction {
    /// 停在末页（默认，与历史行为一致）。
    #[default]
    DoNothing,
    /// 回到本书第一页。
    WrapToFirst,
    /// 打开同级下一个漫画文件，或（当前为文件夹时）下一个兄弟文件夹。
    NextSibling,
}

/// 媒体播放到结尾时的行为。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaEndAction {
    /// 停在结尾，不自动续播。
    Stop,
    /// 自动打开同目录自然排序的下一个媒体文件（默认，与历史行为一致）。
    #[default]
    NextInDir,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    #[default]
    Comic,
    Ebook,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EbookTheme {
    #[default]
    Light,
    Dark,
    Sepia,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EbookSettings {
    pub reading_mode: EbookReadingMode,
    pub font_family: String,
    pub font_size: u32,
    pub line_height: f32,
    pub margin_horizontal: u32,
    pub margin_vertical: u32,
    pub theme: EbookTheme,
    pub enable_page_animation: bool,
    pub invert_scroll: bool,
}

impl Default for EbookSettings {
    fn default() -> Self {
        Self {
            reading_mode: EbookReadingMode::SinglePage,
            font_family: "system-ui".to_string(),
            font_size: 16,
            line_height: 1.6,
            margin_horizontal: 24,
            margin_vertical: 24,
            theme: EbookTheme::Light,
            enable_page_animation: false,
            invert_scroll: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64,
    pub media_type: MediaType,
    pub tags: Vec<String>,
    /// Total pages (comics) or chapters (ebooks). Media leaves `None`.
    #[serde(default)]
    pub page_count: Option<usize>,
}

impl Default for LibraryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            title: String::new(),
            path: PathBuf::new(),
            cover_path: None,
            added_at: 0,
            media_type: MediaType::Comic,
            tags: Vec::new(),
            page_count: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub entries: Vec<LibraryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryEntry {
    pub comic_id: String,
    pub path: std::path::PathBuf,
    pub volume_index: usize,
    pub page_index: usize,
    #[serde(default)]
    pub char_offset: Option<usize>,
    pub last_read_at: u64,
}

impl Default for HistoryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            path: std::path::PathBuf::new(),
            volume_index: 0,
            page_index: 0,
            char_offset: None,
            last_read_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub comic_id: String,
    pub volume_index: usize,
    pub page_index: usize,
    #[serde(default)]
    pub char_offset: Option<usize>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bookmarks {
    pub entries: Vec<Bookmark>,
}

/// 每本书记忆的阅读设置（模式/双页/缩放/旋转），打开时覆盖全局默认；
/// 以 comic_id 为 key 存于 comic_settings.json。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComicReadingSettings {
    pub mode: ReadingMode,
    pub double_page: bool,
    pub fit: FitMode,
    /// 90° 步进旋转（0/90/180/270）；旧文件无此字段时默认为 0。
    /// （用 u16 而非 u8：270 超出 u8 范围。）
    #[serde(default)]
    pub rotation: u16,
}

/// 每本书的累计阅读时长（自本功能启用起累计，不回填历史）。
/// 以 comic_id 为 key 存于 reading_stats.json。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadingStat {
    pub total_seconds: u64,
    pub first_read_at: u64,
    pub last_read_at: u64,
}

impl ReadingStat {
    /// 累加一次阅读增量；`now_ts` 为 unix 秒。首次累计时记录 first_read_at。
    pub fn accumulate(&mut self, seconds: u64, now_ts: u64) {
        if self.first_read_at == 0 {
            self.first_read_at = now_ts;
        }
        self.last_read_at = now_ts;
        self.total_seconds += seconds;
    }
}

/// 时长展示格式：不足一小时 `Y 分钟`，否则 `X 小时 Y 分`。
pub fn format_reading_duration(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    if hours > 0 {
        format!("{} 小时 {} 分", hours, minutes)
    } else {
        format!("{} 分钟", minutes)
    }
}

/// 密码字段的 base64 混淆序列化。
/// 注意：这只是防止浏览配置文件时一眼看到明文，并非加密——
/// base64 可逆且无密钥，任何拿到文件的人都能还原。
mod password_base64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(password: &str, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(password))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .map_err(serde::de::Error::custom)?;
        String::from_utf8(bytes).map_err(serde::de::Error::custom)
    }
}

/// 密码本单条记录。内存中 `password` 始终是明文，仅在 JSON 序列化时
/// 做 base64 混淆（见 `password_base64` 模块注释）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PasswordBookEntry {
    #[serde(with = "password_base64")]
    pub password: String,
    /// 是否为内置常见密码（内置条目可被用户删除，删除后不复活）。
    pub builtin: bool,
    pub note: String,
    pub use_count: u32,
    pub last_used_unix: u64,
}

/// 加密压缩包密码本，持久化于 password_book.json。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PasswordBook {
    pub entries: Vec<PasswordBookEntry>,
}

impl PasswordBook {
    /// 内置常见漫画资源站默认密码表（kox.moe/mox.moe 等站点与常见弱密码）。
    pub fn builtin_defaults() -> Vec<PasswordBookEntry> {
        const BUILTIN: &[&str] = &[
            "123456", "1234", "12345", "password", "manga", "54188", "acg", "gumeng", "kox.moe",
            "mox.moe", "666666", "123123", "111111", "888888", "5201314", "sosg", "tlacg", "cy-cd",
            "acg12", "acgng",
        ];
        BUILTIN
            .iter()
            .map(|pw| PasswordBookEntry {
                password: pw.to_string(),
                builtin: true,
                ..Default::default()
            })
            .collect()
    }

    /// 候选密码列表：use_count 降序、再 last_used_unix 降序，去重。
    pub fn candidates(&self) -> Vec<String> {
        let mut entries: Vec<&PasswordBookEntry> = self
            .entries
            .iter()
            .filter(|e| !e.password.is_empty())
            .collect();
        entries.sort_by(|a, b| {
            b.use_count
                .cmp(&a.use_count)
                .then(b.last_used_unix.cmp(&a.last_used_unix))
        });
        let mut seen = std::collections::HashSet::new();
        entries
            .into_iter()
            .filter(|e| seen.insert(e.password.clone()))
            .map(|e| e.password.clone())
            .collect()
    }

    /// 记录一次密码命中：已有条目 use_count+1 并刷新 last_used_unix；
    /// 没有则追加用户条目（builtin=false，note 为空）。
    pub fn record_success(&mut self, pw: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.password == pw) {
            entry.use_count = entry.use_count.saturating_add(1);
            entry.last_used_unix = now;
        } else {
            self.entries.push(PasswordBookEntry {
                password: pw.to_string(),
                builtin: false,
                note: String::new(),
                use_count: 1,
                last_used_unix: now,
            });
        }
    }

    pub fn remove(&mut self, pw: &str) {
        self.entries.retain(|e| e.password != pw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_deserialize_missing_extract_fields() {
        // 旧版 settings.json 无解压相关字段 → 取默认值
        let json = r#"{"theme":"Dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.extract_dir, "");
        assert_eq!(s.extract_threads, 0);
        assert!(!s.extract_overwrite);
        assert_eq!(s.extract_wrap, "smart");
        assert!(!s.extract_delete_archive);
        assert!(!s.extract_open_folder);
    }

    #[test]
    fn test_settings_extract_wrap_validate_and_clamp() {
        assert!(Settings::default().validate().is_ok());
        for wrap in ["smart", "always", "never"] {
            let s = Settings {
                extract_wrap: wrap.to_string(),
                ..Default::default()
            };
            assert!(s.validate().is_ok());
        }
        let mut s = Settings {
            extract_wrap: "bogus".to_string(),
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert_eq!(s.extract_wrap, "smart");
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_settings_extract_threads_validate_and_clamp() {
        assert!(Settings::default().validate().is_ok());
        let mut s = Settings {
            extract_threads: 33,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert_eq!(s.extract_threads, 32);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_comic_reading_settings_serde_roundtrip() {
        let s = ComicReadingSettings {
            mode: ReadingMode::Rtl,
            double_page: true,
            fit: FitMode::Page,
            rotation: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: ComicReadingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_comic_reading_settings_deserializes_missing_rotation_as_zero() {
        // 旧版 comic_settings.json 不含 rotation 字段（枚举按变体名序列化）
        let json = r#"{"mode":"Ltr","double_page":true,"fit":"Page"}"#;
        let s: ComicReadingSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.rotation, 0);
        assert!(s.double_page);
    }

    #[test]
    fn test_comic_reading_settings_rotation_roundtrip() {
        let s = ComicReadingSettings {
            mode: ReadingMode::Ltr,
            double_page: false,
            fit: FitMode::Width,
            rotation: 270,
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: ComicReadingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_library_entry_deserializes_missing_tags_as_empty() {
        // 旧版 library.json 不含 tags 字段
        let json =
            r#"{"comic_id":"id","title":"Test","path":"/tmp","cover_path":null,"added_at":0}"#;
        let entry: LibraryEntry = serde_json::from_str(json).unwrap();
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn test_library_entry_tags_roundtrip() {
        let entry = LibraryEntry {
            tags: vec!["热血".to_string(), "连载中".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&entry).unwrap();
        let loaded: LibraryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.tags, vec!["热血", "连载中"]);
    }

    #[test]
    fn library_entry_page_count_roundtrip_json() {
        let entry = LibraryEntry {
            page_count: Some(12),
            ..Default::default()
        };
        let json = serde_json::to_string(&entry).unwrap();
        let loaded: LibraryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_count, Some(12));

        // 旧版 library.json 无 page_count → None，不炸
        let old =
            r#"{"comic_id":"id","title":"Test","path":"/tmp","cover_path":null,"added_at":0}"#;
        let legacy: LibraryEntry = serde_json::from_str(old).unwrap();
        assert_eq!(legacy.page_count, None);
    }

    #[test]
    fn test_settings_default() {
        let s = Settings::default();
        assert!(matches!(s.theme, Theme::System));
        assert_eq!(s.cache_size_mb, 1024);
        assert!(s.show_toolbar);
        assert!(s.show_statusbar);
        assert!(!s.invert_scroll);
        assert_eq!(s.background_color, [30, 30, 30]);
        assert!((s.wide_page_threshold - 1.4).abs() < f32::EPSILON);
        assert!((s.page_scroll_threshold - 12.0).abs() < f32::EPSILON);
        assert!((s.chrome_opacity - 0.85).abs() < f32::EPSILON);
        assert_eq!(s.comic_end_action, ComicEndAction::DoNothing);
        assert_eq!(s.media_end_action, MediaEndAction::NextInDir);
    }

    #[test]
    fn test_settings_deserialize_missing_end_actions() {
        let json = r#"{"theme":"Dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.comic_end_action, ComicEndAction::DoNothing);
        assert_eq!(s.media_end_action, MediaEndAction::NextInDir);
    }

    #[test]
    fn test_settings_chrome_opacity_validate_and_clamp() {
        let mut s = Settings {
            chrome_opacity: 0.05,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert!((s.chrome_opacity - 0.2).abs() < f32::EPSILON);
        let mut s = Settings {
            chrome_opacity: 1.5,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert!((s.chrome_opacity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_settings_deserialize_missing_chrome_opacity() {
        let json = r#"{"theme":"Dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!((s.chrome_opacity - 0.85).abs() < f32::EPSILON);
    }

    #[test]
    fn test_settings_page_scroll_threshold_validate_and_clamp() {
        let s = Settings::default();
        assert!(s.validate().is_ok());
        let mut s = Settings {
            page_scroll_threshold: 0.5,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert!((s.page_scroll_threshold - 1.0).abs() < f32::EPSILON);
        assert!(s.validate().is_ok());
        let mut s = Settings {
            page_scroll_threshold: 100.0,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.clamp();
        assert!((s.page_scroll_threshold - 40.0).abs() < f32::EPSILON);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_settings_deserialize_missing_page_scroll_threshold() {
        // 旧版 settings.json 无此字段 → 取默认值
        let json = r#"{"theme":"Dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!((s.page_scroll_threshold - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_settings_roundtrip_with_background_color() {
        let s = Settings {
            background_color: [12, 34, 56],
            library_sort: LibrarySort::Title,
            toolbar_display_mode: ToolbarDisplayMode::IconOnly,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.toolbar_display_mode, ToolbarDisplayMode::IconOnly);
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_fm_show_hidden_roundtrip_and_default() {
        let s = Settings {
            fm_show_hidden: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert!(!loaded.fm_show_hidden);
        assert_eq!(s, loaded);

        let loaded: Settings = serde_json::from_str("{}").unwrap();
        assert!(loaded.fm_show_hidden);
    }

    #[test]
    fn test_fm_bookmark_groups_migration_and_sanitize() {
        // 旧版扁平书签 → 单个分组「常用」；旧字段清空且保存不再写出。
        let json = r#"{"fm_bookmarks": ["C:\\a", "D:\\b"]}"#;
        let mut loaded: Settings = serde_json::from_str(json).unwrap();
        loaded.clamp();
        assert!(loaded.fm_bookmarks.is_empty());
        assert_eq!(loaded.fm_bookmark_groups.len(), 1);
        assert_eq!(loaded.fm_bookmark_groups[0].name, "常用");
        assert_eq!(loaded.fm_bookmark_groups[0].items, ["C:\\a", "D:\\b"]);
        let out = serde_json::to_string(&loaded).unwrap();
        assert!(!out.contains("fm_bookmarks"));

        // 全新 settings：无分组。
        let loaded: Settings = serde_json::from_str("{}").unwrap();
        assert!(loaded.fm_bookmark_groups.is_empty());

        // 校验：空名修「未命名」、组内去重去空、空分组保留。
        let mut s = Settings {
            fm_bookmark_groups: vec![
                FmBookmarkGroup {
                    name: "  ".to_string(),
                    items: vec![
                        "C:\\a".to_string(),
                        "  ".to_string(),
                        "C:\\a".to_string(),
                        "D:\\b".to_string(),
                    ],
                },
                FmBookmarkGroup {
                    name: "空组".to_string(),
                    items: Vec::new(),
                },
            ],
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_bookmark_groups[0].name, "未命名");
        assert_eq!(s.fm_bookmark_groups[0].items, ["C:\\a", "D:\\b"]);
        assert_eq!(s.fm_bookmark_groups[1].name, "空组");
        assert!(s.fm_bookmark_groups[1].items.is_empty());

        // 新字段 roundtrip。
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_fm_col_widths_default_and_clamp() {
        // 旧 settings 无字段：serde default = 现状默认列宽/零平移。
        let loaded: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(loaded.fm_col_size_width, 90.0);
        assert_eq!(loaded.fm_col_mtime_width, 110.0);
        assert_eq!(loaded.fm_col_shift, 0.0);

        let mut s = Settings {
            fm_col_size_width: 10.0,
            fm_col_mtime_width: 9999.0,
            fm_col_shift: -9999.0,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_col_size_width, 60.0);
        assert_eq!(s.fm_col_mtime_width, 400.0);
        // 下限 = -(两列宽之和)（clamp 后的 60+400）。
        assert_eq!(s.fm_col_shift, -460.0);

        let mut s = Settings {
            fm_col_shift: 50.0,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_col_shift, 0.0, "col_shift ≤ 0");
    }

    #[test]
    fn test_fm_behavior_options_default_roundtrip_and_clamp() {
        // 旧 settings 无字段：serde default = 现状行为。
        let loaded: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(loaded.fm_delete_mode, "trash");
        assert_eq!(loaded.fm_space_action, "dir_size");
        assert!(loaded.fm_dirs_first);
        assert!(loaded.fm_drag_confirm);
        assert_eq!(loaded.fm_archive_open, "archive");
        assert!(!loaded.fm_esc_keep_selection);
        assert!(loaded.fm_dblclick_blank_up);
        assert!(loaded.fm_system_icons);
        assert!(loaded.fm_saved_filters.is_empty());
        assert!(!loaded.fm_filter_bar_bottom);
        assert_eq!(loaded.fm_rubber_band, "right");

        // 非默认值 roundtrip。
        let s = Settings {
            fm_delete_mode: "permanent".to_string(),
            fm_space_action: "toggle_select".to_string(),
            fm_dirs_first: false,
            fm_drag_confirm: false,
            fm_archive_open: "ask".to_string(),
            fm_esc_keep_selection: true,
            fm_dblclick_blank_up: false,
            fm_system_icons: false,
            fm_saved_filters: vec!["*.zip".to_string(), "漫画".to_string()],
            fm_filter_bar_bottom: true,
            fm_rubber_band: "left".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);

        // 脏值 clamp 回默认。
        let mut s = Settings {
            fm_delete_mode: "shred".to_string(),
            fm_space_action: "launch".to_string(),
            fm_archive_open: "hack".to_string(),
            fm_rubber_band: "middle".to_string(),
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_delete_mode, "trash");
        assert_eq!(s.fm_space_action, "dir_size");
        assert_eq!(s.fm_archive_open, "archive");
        assert_eq!(s.fm_rubber_band, "right");
        // 保存的过滤方案：trim + 去空 + 保序去重。
        let mut s = Settings {
            fm_saved_filters: vec![
                "  *.zip ".to_string(),
                "".to_string(),
                "*.zip".to_string(),
                "漫画".to_string(),
                "   ".to_string(),
            ],
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_saved_filters, ["*.zip", "漫画"]);
        // 合法非默认值保留。
        let mut s = Settings {
            fm_delete_mode: "permanent".to_string(),
            fm_archive_open: "comic".to_string(),
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.fm_delete_mode, "permanent");
        assert_eq!(s.fm_archive_open, "comic");
    }

    #[test]
    fn test_library_serialize() {
        let lib = Library {
            entries: vec![LibraryEntry {
                comic_id: "id".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp"),
                cover_path: None,
                added_at: 0,
                media_type: MediaType::Comic,
                tags: Vec::new(),
                page_count: None,
            }],
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(json.contains("Test"));
    }

    #[test]
    fn test_library_entry_default_media_type_is_comic() {
        let entry = LibraryEntry::default();
        assert_eq!(entry.media_type, MediaType::Comic);
    }

    #[test]
    fn test_library_entry_deserializes_missing_media_type_as_comic() {
        let json =
            r#"{"comic_id":"id","title":"Test","path":"/tmp","cover_path":null,"added_at":0}"#;
        let entry: LibraryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.media_type, MediaType::Comic);
    }

    #[test]
    fn test_media_type_video_audio_roundtrip() {
        let v = serde_json::to_string(&MediaType::Video).unwrap();
        let a = serde_json::to_string(&MediaType::Audio).unwrap();
        assert_eq!(v, "\"video\"");
        assert_eq!(a, "\"audio\"");
        assert_eq!(
            serde_json::from_str::<MediaType>(&v).unwrap(),
            MediaType::Video
        );
        assert_eq!(
            serde_json::from_str::<MediaType>(&a).unwrap(),
            MediaType::Audio
        );
    }

    #[test]
    fn test_library_entry_deserializes_media_type_video() {
        let json = r#"{"comic_id":"id","title":"T","path":"/tmp/v.mp4","cover_path":null,"added_at":0,"media_type":"video"}"#;
        let entry: LibraryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.media_type, MediaType::Video);
    }

    #[test]
    fn test_ebook_settings_default() {
        let s = EbookSettings::default();
        assert_eq!(s.reading_mode, EbookReadingMode::SinglePage);
        assert_eq!(s.font_family, "system-ui");
        assert_eq!(s.font_size, 16);
        assert!((s.line_height - 1.6).abs() < f32::EPSILON);
        assert_eq!(s.margin_horizontal, 24);
        assert_eq!(s.margin_vertical, 24);
        assert_eq!(s.theme, EbookTheme::Light);
        assert!(!s.enable_page_animation);
        assert!(!s.invert_scroll);
    }

    #[test]
    fn test_history_entry_defaults() {
        let h = HistoryEntry::default();
        assert_eq!(h.volume_index, 0);
        assert_eq!(h.page_index, 0);
        assert_eq!(h.char_offset, None);
    }

    #[test]
    fn test_history_entry_roundtrip_with_char_offset() {
        let h = HistoryEntry {
            comic_id: "ebook1".to_string(),
            path: PathBuf::from("/tmp/book.epub"),
            volume_index: 0,
            page_index: 2,
            char_offset: Some(1500),
            last_read_at: 12345,
        };
        let json = serde_json::to_string(&h).unwrap();
        let loaded: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_index, 2);
        assert_eq!(loaded.char_offset, Some(1500));
        assert_eq!(h, loaded);
    }

    #[test]
    fn test_history_entry_deserializes_missing_char_offset_as_none() {
        let json =
            r#"{"comic_id":"id","path":"/tmp","volume_index":0,"page_index":1,"last_read_at":0}"#;
        let h: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(h.char_offset, None);
    }

    #[test]
    fn test_bookmark_defaults_and_roundtrip() {
        let b = Bookmark {
            comic_id: "ebook1".to_string(),
            volume_index: 0,
            page_index: 3,
            char_offset: Some(1200),
            note: Some("note".to_string()),
        };
        let json = serde_json::to_string(&b).unwrap();
        let loaded: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_index, 3);
        assert_eq!(loaded.char_offset, Some(1200));
        assert_eq!(b, loaded);
    }

    #[test]
    fn test_bookmark_deserializes_missing_char_offset_as_none() {
        let json = r#"{"comic_id":"id","volume_index":0,"page_index":1,"note":null}"#;
        let b: Bookmark = serde_json::from_str(json).unwrap();
        assert_eq!(b.char_offset, None);
    }

    #[test]
    fn test_settings_validate_rejects_bad_ebook_margins() {
        let mut s = Settings::default();
        s.ebook.margin_horizontal = 250;
        assert!(s.validate().is_err());
        s.ebook.margin_horizontal = 24;
        s.ebook.margin_vertical = 250;
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_settings_clamp_ebook_margins() {
        let mut s = Settings::default();
        s.ebook.margin_horizontal = 300;
        s.ebook.margin_vertical = 400;
        s.clamp();
        assert_eq!(s.ebook.margin_horizontal, 200);
        assert_eq!(s.ebook.margin_vertical, 200);
    }

    #[test]
    fn test_settings_validate_rejects_empty_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = "   ".to_string();
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_settings_clamp_restores_default_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = String::new();
        s.clamp();
        assert_eq!(s.ebook.font_family, "system-ui");
    }

    #[test]
    fn test_default_shortcuts_cover_ebook_actions() {
        let s = Shortcuts::default();
        assert!(s.back_to_library.contains(&"Escape".to_string()));
        assert!(s.page_down.contains(&"PageDown".to_string()));
        assert!(s.page_up.contains(&"PageUp".to_string()));
    }

    #[test]
    fn test_shortcuts_default_first_last_page() {
        let s = Shortcuts::default();
        assert_eq!(s.first_page, vec!["Home".to_string()]);
        assert_eq!(s.last_page, vec!["End".to_string()]);
    }

    #[test]
    fn test_shortcuts_deserialize_missing_first_last_page_uses_defaults() {
        // 旧版 settings.json 不含 first_page/last_page 字段，应落到新默认值
        let json = r#"{"next_page":["ArrowRight"],"prev_page":["ArrowLeft"]}"#;
        let s: Shortcuts = serde_json::from_str(json).unwrap();
        assert_eq!(s.next_page, vec!["ArrowRight".to_string()]);
        assert_eq!(s.first_page, vec!["Home".to_string()]);
        assert_eq!(s.last_page, vec!["End".to_string()]);
    }

    #[test]
    fn test_reading_stat_accumulate_sets_first_and_last() {
        let mut stat = ReadingStat::default();
        stat.accumulate(30, 1_000);
        assert_eq!(stat.total_seconds, 30);
        assert_eq!(stat.first_read_at, 1_000);
        assert_eq!(stat.last_read_at, 1_000);
        stat.accumulate(45, 2_000);
        assert_eq!(stat.total_seconds, 75);
        assert_eq!(stat.first_read_at, 1_000); // 首次不变
        assert_eq!(stat.last_read_at, 2_000);
    }

    #[test]
    fn test_format_reading_duration() {
        assert_eq!(format_reading_duration(0), "0 分钟");
        assert_eq!(format_reading_duration(59), "0 分钟");
        assert_eq!(format_reading_duration(60), "1 分钟");
        assert_eq!(format_reading_duration(3_599), "59 分钟");
        assert_eq!(format_reading_duration(3_600), "1 小时 0 分");
        assert_eq!(format_reading_duration(5_460), "1 小时 31 分");
    }

    #[test]
    fn test_password_book_entry_base64_roundtrip() {
        // 含空格与 unicode 的密码、中文 note，往返一致
        let entry = PasswordBookEntry {
            password: "密 码🔒 pass".to_string(),
            builtin: false,
            note: "资源站默认密码".to_string(),
            use_count: 3,
            last_used_unix: 1_700_000_000,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let loaded: PasswordBookEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, loaded);
    }

    #[test]
    fn test_password_book_serialized_json_hides_plaintext() {
        // base64 混淆：序列化后的 JSON 中不出现明文密码
        let book = PasswordBook {
            entries: vec![PasswordBookEntry {
                password: "super-secret-pw".to_string(),
                ..Default::default()
            }],
        };
        let json = serde_json::to_string(&book).unwrap();
        assert!(!json.contains("super-secret-pw"));
        let loaded: PasswordBook = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.entries[0].password, "super-secret-pw");
    }

    #[test]
    fn test_password_book_deserializes_missing_fields() {
        // 旧文件/手编文件缺字段 → 默认填充
        let json = r#"{"entries":[{"password":"MTIzNDU2"}]}"#;
        let book: PasswordBook = serde_json::from_str(json).unwrap();
        assert_eq!(book.entries[0].password, "123456");
        assert!(!book.entries[0].builtin);
        assert_eq!(book.entries[0].use_count, 0);
    }

    #[test]
    fn test_password_book_builtin_defaults_all_builtin() {
        let defaults = PasswordBook::builtin_defaults();
        assert!(!defaults.is_empty());
        assert!(defaults.iter().all(|e| e.builtin));
        assert!(defaults.iter().any(|e| e.password == "123456"));
        // 无重复
        let mut seen = std::collections::HashSet::new();
        assert!(defaults.iter().all(|e| seen.insert(e.password.as_str())));
    }

    #[test]
    fn test_password_book_candidates_sort_and_dedup() {
        let entry = |pw: &str, use_count: u32, last_used_unix: u64| PasswordBookEntry {
            password: pw.to_string(),
            use_count,
            last_used_unix,
            ..Default::default()
        };
        let book = PasswordBook {
            entries: vec![
                entry("a", 1, 100),
                entry("b", 5, 50),  // use_count 最高 → 第一
                entry("c", 1, 200), // 与 a 同 use_count，last_used 更新 → 排 a 前
                entry("b", 9, 999), // 重复密码 → 去重后只出现一次
                entry("", 99, 999), // 空密码忽略
            ],
        };
        let candidates = book.candidates();
        assert_eq!(candidates, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_password_book_record_success_new_and_existing() {
        let mut book = PasswordBook::default();
        // 新密码：追加用户条目
        book.record_success("mypw");
        assert_eq!(book.entries.len(), 1);
        let e = &book.entries[0];
        assert_eq!(e.password, "mypw");
        assert!(!e.builtin);
        assert_eq!(e.use_count, 1);
        assert!(e.last_used_unix > 0);

        // 命中已有条目（含内置条目）：计数+1、刷新时间，不新增
        let mut book = PasswordBook {
            entries: PasswordBook::builtin_defaults(),
        };
        let len = book.entries.len();
        book.record_success("123456");
        assert_eq!(book.entries.len(), len);
        let e = book
            .entries
            .iter()
            .find(|e| e.password == "123456")
            .unwrap();
        assert_eq!(e.use_count, 1);
        assert!(e.builtin);
        assert!(e.last_used_unix > 0);
    }

    #[test]
    fn test_password_book_remove() {
        let mut book = PasswordBook {
            entries: PasswordBook::builtin_defaults(),
        };
        assert!(book.entries.iter().any(|e| e.password == "123456"));
        book.remove("123456");
        assert!(!book.entries.iter().any(|e| e.password == "123456"));
        // 删除不存在的密码不炸
        book.remove("no-such-pw");
    }
}
