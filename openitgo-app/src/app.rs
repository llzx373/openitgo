use crate::loader::PageLoader;
use crate::opener::{AsyncOpener, OpenStatus};
use crate::shortcuts::is_shortcut_pressed;
use crate::timing;
use crate::views::settings::SettingsView;
use crate::views::{
    ebook::EbookView,
    library::{LibraryCallbacks, LibraryView},
    media::{media_overlay, MediaOverlay, MediaView},
    reader::ReaderView,
};
use egui_phosphor_icons::{icons, Icon};
use openitgo_core::ebook::Ebook;
use openitgo_core::models::{Comic, FitMode, PageSource, ReadingMode};
use openitgo_core::state::ReadingState;
use openitgo_storage::{
    json_store::JsonStore,
    models::{
        Bookmarks, ComicReadingSettings, EbookTheme, History, HistoryEntry, Library, MediaType,
        Settings, Theme, ToolbarDisplayMode,
    },
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn is_ebook_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "epub" | "mobi" | "azw" | "azw3" | "txt" | "md" | "markdown"
            )
        })
        .unwrap_or(false)
}

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "aac", "m4a", "ogg", "oga", "opus", "wav", "aiff", "ape", "wma",
];

fn is_media_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let ext = e.to_ascii_lowercase();
            matches!(
                ext.as_str(),
                // 视频
                "mp4"
                    | "m4v"
                    | "mkv"
                    | "webm"
                    | "avi"
                    | "mov"
                    | "wmv"
                    | "flv"
                    | "ts"
                    | "m2ts"
                    | "mpg"
                    | "mpeg"
                    | "3gp"
            ) || AUDIO_EXTS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

/// 数字感知、大小写不敏感的自然排序比较（"EP2" < "EP10"）。
/// 连续数字段按数值比较，其余字符按小写后的字典序逐字符比较。
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut s = String::new();
        while let Some(c) = it.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            s.push(*c);
            it.next();
        }
        s
    }

    let mut ca = a.chars().peekable();
    let mut cb = b.chars().peekable();
    loop {
        match (ca.peek().copied(), cb.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_digits(&mut ca);
                let nb = take_digits(&mut cb);
                // 去掉前导零后先比长度再比字典序，即数值比较（无溢出风险）。
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ca.next();
                cb.next();
            }
        }
    }
}

/// 返回同目录下按自然排序位于 current 之后的第一个媒体文件；
/// current 不在目录的媒体列表中或已是最后一个时返回 None。
fn next_media_in_dir(current: &Path) -> Option<PathBuf> {
    let dir = current.parent()?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_media_file(p))
        .collect();
    entries.sort_by(|a, b| {
        let an = a
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bn = b
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 自然比较相等（仅大小写/前导零差异）时用原始文件名兜底，保证排序确定。
        natural_cmp(&an, &bn).then_with(|| an.cmp(&bn))
    });
    let pos = entries.iter().position(|p| p == current)?;
    entries.get(pos + 1).cloned()
}

fn media_type_for_path(path: &std::path::Path) -> MediaType {
    if is_ebook_file(path) {
        MediaType::Ebook
    } else if is_media_file(path) {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some(ext) if AUDIO_EXTS.contains(&ext) => MediaType::Audio,
            _ => MediaType::Video,
        }
    } else {
        MediaType::Comic
    }
}

pub struct ReaderApp {
    pub current_view: View,
    pub last_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
    pub ebook_view: EbookView,
    pub media_view: MediaView,
    pub settings_view: SettingsView,
    pub store: JsonStore,
    pub history: History,
    pub bookmarks: Bookmarks,
    pub error_message: Option<String>,
    pub page_loader: PageLoader,
    pub cover_loader: PageLoader,
    pub opener: Option<AsyncOpener<Comic>>,
    pub ebook_opener: Option<AsyncOpener<Ebook>>,
    pub pending_media_open: Option<PathBuf>,
    /// Cover requests in flight: epoch -> (comic_id, comic_path).
    pub pending_covers: HashMap<crate::loader::Epoch, (String, PathBuf)>,
    /// Comic ids for which a cover generation has already been requested.
    pub requested_cover_ids: HashSet<String>,
    /// Media cover results from worker threads: (comic_id, cover_path).
    pub media_cover_tx: crossbeam_channel::Sender<(String, PathBuf)>,
    pub media_cover_rx: crossbeam_channel::Receiver<(String, PathBuf)>,
    /// The theme currently applied to egui, used to avoid redundant updates.
    pub current_theme: Theme,
    /// 每本书记忆的阅读设置（comic_id -> 模式/双页/缩放），打开漫画时覆盖全局默认。
    pub comic_settings: HashMap<String, ComicReadingSettings>,
    /// 上次写盘的每书阅读设置快照（含 comic_id）。每帧与当前打开漫画的
    /// 三元组对比，变更时 upsert 并写盘；打开/关闭漫画时重置。
    pub last_saved_comic_settings: Option<(String, ComicReadingSettings)>,
    /// 帮助菜单"快捷键一览"面板的显示状态。
    pub show_shortcuts: bool,
    /// 会话级压缩包密码缓存（不落盘）；key 为压缩包路径。
    pub passwords: HashMap<PathBuf, String>,
    /// 加密压缩包密码对话框状态；Some 时渲染模态窗口。
    pub password_dialog: Option<PasswordDialog>,
    /// 批量导入中等待输密码的文件队列（逐个弹同一对话框）。
    pub pending_password_imports: Vec<PathBuf>,
    /// 批量导入被用户取消的加密文件计数（汇总提示后清零）。
    pub skipped_encrypted_imports: usize,
    /// 最近一次 open_comic 的目标路径（供密码错误标记关联对话框）。
    pub opening_path: Option<PathBuf>,
    /// 每本书的累计阅读时长（comic_id -> ReadingStat），30s 粒度落盘。
    pub reading_stats: HashMap<String, openitgo_storage::models::ReadingStat>,
    /// 当前阅读会话：(comic_id, 上次结算时刻)。视图切换/退出时结算并重启。
    pub stats_session: Option<(String, std::time::Instant)>,
    /// 书签缩略图请求在途：epoch -> (comic_id, page_index)。
    pub pending_bookmark_thumbs: HashMap<crate::loader::Epoch, (String, usize)>,
}

impl Default for ReaderApp {
    fn default() -> Self {
        let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| PathBuf::from(".")));
        let (settings, settings_error) = load_settings_with_error(&store);
        let (comic_settings, comic_settings_error) = load_comic_settings_with_error(&store);
        let reading_stats = store.load_reading_stats().unwrap_or_default();
        let library = store.load_library().unwrap_or_default();
        let mut history = store.load_history().unwrap_or_default();
        let mut bookmarks = store.load_bookmarks().unwrap_or_default();
        let mut library_view = LibraryView::default();
        library_view.library = library;
        let covers_dir = store.dir().join("covers");
        library_view.covers_dir = Some(covers_dir.clone());
        if migrate_library_ids(
            &mut library_view.library,
            &mut history,
            &mut bookmarks,
            &covers_dir,
        ) {
            let _ = store.save_library(&library_view.library);
            let _ = store.save_history(&history);
            let _ = store.save_bookmarks(&bookmarks);
        }
        let page_loader = PageLoader::new_with_compress(
            settings.compress_images,
            settings.decode_threads as usize,
        );
        let cover_loader = PageLoader::new_with_compress(false, 1);
        let (media_cover_tx, media_cover_rx) = crossbeam_channel::unbounded();
        Self {
            current_view: View::Library,
            last_view: View::Library,
            settings,
            library_view,
            reader_view: ReaderView::default(),
            ebook_view: EbookView::default(),
            media_view: MediaView::default(),
            settings_view: SettingsView::default(),
            store,
            history,
            bookmarks,
            error_message: settings_error.or(comic_settings_error),
            page_loader,
            cover_loader,
            opener: None,
            ebook_opener: None,
            pending_media_open: None,
            pending_covers: HashMap::new(),
            requested_cover_ids: HashSet::new(),
            media_cover_tx,
            media_cover_rx,
            current_theme: Theme::System,
            comic_settings,
            last_saved_comic_settings: None,
            show_shortcuts: false,
            passwords: HashMap::new(),
            password_dialog: None,
            pending_password_imports: Vec::new(),
            skipped_encrypted_imports: 0,
            opening_path: None,
            reading_stats,
            stats_session: None,
            pending_bookmark_thumbs: HashMap::new(),
        }
    }
}

impl eframe::App for ReaderApp {
    /// Fully transparent: every view paints its own opaque panels, and in
    /// the media view the unpainted central area must let the video layer
    /// below the egui surface show through (see
    /// platform/macos/mpv_view.rs).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self) {
        self.record_reader_history();
        self.record_ebook_history();
        self.record_media_history();
        self.flush_reading_stats();
        self.reader_view.close();
        let _ = self.store.save_settings(&self.settings);
        let _ = self.store.save_library(&self.library_view.library);
        let _ = self.store.save_history(&self.history);
        let _ = self.store.save_bookmarks(&self.bookmarks);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if matches!(self.last_view, View::Reader) && !matches!(self.current_view, View::Reader) {
            self.record_reader_history();
            self.reader_view.clear_cache();
        }
        if matches!(self.last_view, View::Ebook) && !matches!(self.current_view, View::Ebook) {
            self.record_ebook_history();
            self.ebook_view.close();
        }
        if matches!(self.last_view, View::Media) && !matches!(self.current_view, View::Media) {
            self.record_media_history();
            self.media_view.close();
        }
        self.last_view = self.current_view.clone();

        if self.settings.theme != self.current_theme {
            self.apply_theme(&ctx);
            self.current_theme = self.settings.theme.clone();
        }

        self.handle_global_input(&ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.ctrl) {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.add_folder_to_library(path);
            }
        }
        self.handle_dropped_files(&ctx);
        #[cfg(target_os = "macos")]
        {
            let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
            self.handle_open_paths(dock_paths);
        }
        self.poll_opener(&ctx);
        self.poll_ebook_opener(&ctx, frame);
        self.poll_media_open(&ctx, frame);
        if self.media_view.take_startup_device_invalid() {
            // 保存的音频设备已拔出：已回退 auto，同步清除持久化设置。
            self.settings.media_audio_device.clear();
        }

        let cache_size_mb = self.settings.cache_size_mb as usize;
        self.page_loader.set_compress(self.settings.compress_images);
        self.reader_view
            .update(&ctx, &self.page_loader, cache_size_mb);
        self.reader_view.request_preloads(
            &self.page_loader,
            cache_size_mb,
            self.settings.real_image_cache_pages as usize,
        );
        self.poll_cover_results();
        self.poll_media_covers();

        self.render_menu_bar(ui);

        match self.current_view.clone() {
            View::Library => self.render_library(ui),
            View::Reader => self.render_reader(ui),
            View::Ebook => self.render_ebook(ui),
            View::Media => self.render_media(ui),
            View::Settings => self.render_settings(ui),
            View::Loading(path) => self.render_loading(ui, path),
        }
        self.render_shortcuts_window(&ctx);
        self.render_password_dialog(&ctx);
        self.maybe_save_comic_settings();
        self.tick_reading_stats();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Library,
    Reader,
    Ebook,
    Media,
    Settings,
    Loading(PathBuf),
}

enum BarEdge {
    Top,
    Bottom,
}

fn toolbar_button(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    match mode {
        ToolbarDisplayMode::IconOnly => ui.button(icon).on_hover_text(text),
        ToolbarDisplayMode::TextOnly => ui.button(text),
        ToolbarDisplayMode::IconAndText => ui.button(format!("{} {}", icon.as_str(), text)),
    }
}

fn toolbar_selectable(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    active: bool,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    let label = match mode {
        ToolbarDisplayMode::IconOnly => egui::WidgetText::from(icon),
        ToolbarDisplayMode::TextOnly => egui::WidgetText::from(text),
        ToolbarDisplayMode::IconAndText => {
            egui::WidgetText::from(format!("{} {}", icon.as_str(), text))
        }
    };
    ui.selectable_label(active, label).on_hover_text(text)
}

/// 启动时要打开的路径：优先 `OPENITGO_OPEN` 环境变量，其次第一个命令行参数
/// （Windows/Linux 文件关联经 argv 传入；macOS 走 Apple Event，argv 无路径，
/// 偶发的 `-psn_*` 等系统参数会被 `exists()` 检查天然过滤）。
fn initial_open_path(
    env_open: Option<std::path::PathBuf>,
    arg1: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    [env_open, arg1].into_iter().flatten().find(|p| p.exists())
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        let env_open = std::env::var("OPENITGO_OPEN")
            .ok()
            .map(std::path::PathBuf::from);
        let arg1 = std::env::args_os().nth(1).map(std::path::PathBuf::from);
        if let Some(path) = initial_open_path(env_open, arg1) {
            app.open_path(path);
        }
        app
    }

    fn poll_opener(&mut self, ctx: &egui::Context) {
        let Some(mut opener) = self.opener.take() else {
            return;
        };
        match opener.poll() {
            OpenStatus::Loading => {
                self.opener = Some(opener);
            }
            OpenStatus::Ready(result) => match result {
                Ok(comic) => {
                    timing::log(&format!(
                        "poll_opener comic ready: {} pages",
                        comic.total_pages()
                    ));
                    let total = comic.volumes.first().map(|v| v.pages.len()).unwrap_or(0);
                    let mut state = ReadingState::new(self.settings.default_mode, total);
                    state.set_double_page(self.settings.double_page, total);
                    state.fit_mode = self.settings.default_fit;
                    // 每本书记忆的阅读设置（模式/双页/缩放/旋转）优先于全局默认；
                    // 应用方式与模式菜单/双页开关一致，fit 走 default_fit 同一后续路径。
                    if let Some(saved) = self.comic_settings.get(&comic.id).copied() {
                        state.set_mode(saved.mode, total);
                        state.set_double_page(saved.double_page, total);
                        state.fit_mode = saved.fit;
                        // 只接受 90° 步进值，脏数据按 0 处理。
                        state.rotation = match saved.rotation {
                            90 | 180 | 270 => saved.rotation,
                            _ => 0,
                        };
                    }
                    self.last_saved_comic_settings =
                        Some(comic_reading_settings_snapshot(&comic.id, &state));
                    if let Some(h) = self
                        .history
                        .entries
                        .iter()
                        .find(|h| history_matches(h, &comic.id, &comic.path))
                    {
                        state.go_to_page(h.page_index, total);
                    }
                    self.reader_view.open(
                        ctx,
                        comic,
                        state,
                        &self.page_loader,
                        self.settings.wide_page_threshold,
                        self.settings.enable_page_animation,
                    );
                    self.current_view = View::Reader;
                    self.error_message = None;
                }
                Err(e) => match password_prompt_kind(&e) {
                    Some(kind) => {
                        if let Some(path) = self.opening_path.take() {
                            self.password_dialog = Some(PasswordDialog::new(path, kind));
                        }
                        self.current_view = View::Library;
                    }
                    None => {
                        self.error_message = Some(format!("无法打开漫画: {}", e));
                        self.current_view = View::Library;
                    }
                },
            },
        }
    }

    fn render_library(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &self.error_message {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            let mut open_idx = None;
            let mut open_path: Option<PathBuf> = None;
            let mut add_requested = false;
            let mut request_cover_idx: Option<usize> = None;
            let mut remove_missing = false;
            let mut delete_bookmark_idx: Option<usize> = None;
            let mut update_bookmark: Option<(usize, Option<String>)> = None;
            let mut update_title: Option<(usize, String)> = None;
            let mut update_tags: Option<(usize, Vec<String>)> = None;
            let mut delete_library_idx: Option<usize> = None;
            let mut clear_history = false;
            let mut delete_history_idx: Option<usize> = None;
            self.library_view.ui(
                ui,
                &self.history,
                &self.bookmarks,
                &mut self.settings.library_sort,
                &self.reading_stats,
                LibraryCallbacks {
                    on_open_library: &mut |idx| open_idx = Some(idx),
                    on_open_path: &mut |path| open_path = Some(path),
                    on_add: &mut || add_requested = true,
                    on_request_cover: &mut |idx| request_cover_idx = Some(idx),
                    on_remove_missing: &mut || remove_missing = true,
                    on_delete_bookmark: &mut |idx| delete_bookmark_idx = Some(idx),
                    on_update_bookmark: &mut |idx, note| update_bookmark = Some((idx, note)),
                    on_update_title: &mut |idx, title| update_title = Some((idx, title)),
                    on_update_tags: &mut |idx, tags| update_tags = Some((idx, tags)),
                    on_delete_library: &mut |idx| delete_library_idx = Some(idx),
                    on_clear_history: &mut || clear_history = true,
                    on_delete_history: &mut |idx| delete_history_idx = Some(idx),
                },
            );
            if add_requested {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.add_folder_to_library(path);
                }
            }
            if let Some(idx) = open_idx {
                if let Some(entry) = self.library_view.entry_at(idx).cloned() {
                    self.open_path(entry.path);
                }
            }
            if let Some(path) = open_path {
                self.open_path(path);
            }
            if let Some(idx) = request_cover_idx {
                self.request_cover_for_library_entry(idx);
            }
            if remove_missing {
                self.remove_missing_library_entries();
            }
            if let Some((idx, title)) = update_title {
                if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
                    entry.title = title;
                }
            }
            if let Some((idx, tags)) = update_tags {
                if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
                    entry.tags = tags;
                }
            }
            if let Some(idx) = delete_library_idx {
                if idx < self.library_view.library.entries.len() {
                    let removed = self.library_view.library.entries.remove(idx);
                    remove_bookmark_thumbs(&self.covers_dir(), &removed.comic_id, None);
                }
            }
            if let Some((idx, note)) = update_bookmark {
                if let Some(entry) = self.bookmarks.entries.get_mut(idx) {
                    entry.note = note;
                }
            }
            if let Some(idx) = delete_bookmark_idx {
                if idx < self.bookmarks.entries.len() {
                    let removed = self.bookmarks.entries.remove(idx);
                    if removed.char_offset.is_none() {
                        // 漫画书签才有缩略图（电子书书签不生成）
                        remove_bookmark_thumbs(
                            &self.covers_dir(),
                            &removed.comic_id,
                            Some(removed.page_index),
                        );
                    }
                }
            }
            if clear_history {
                self.history.entries.clear();
            }
            if let Some(idx) = delete_history_idx {
                self.history.entries.remove(idx);
            }
        });
    }

    fn render_reader(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(reader) = self.reader_view.open.as_ref() else {
            self.current_view = View::Library;
            return;
        };
        let total_pages = reader.total_pages();
        let current_page = reader.state.current_page;
        let mode = reader.state.mode;
        let zoom = reader.state.zoom;

        let (fullscreen, mouse_pos, screen_size) = ctx.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or(false),
                i.pointer.latest_pos(),
                i.content_rect().size(),
            )
        });
        if Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        ) {
            self.render_reader_toolbar(ui, total_pages, current_page, mode, zoom);
        }
        let (progress_bar_rect, hovered_page) = if Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        ) {
            self.render_reader_statusbar(ui)
        } else {
            (None, None)
        };

        let bg = self.settings.background_color;
        let frame = egui::Frame::central_panel(&ctx.global_style())
            .fill(egui::Color32::from_rgb(bg[0], bg[1], bg[2]));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let response = self.reader_view.ui(&ctx, ui, &self.page_loader);

            // Floating thumbnail tooltip above the cursor when hovering the progress bar.
            if progress_bar_rect.is_some() {
                self.reader_view
                    .render_progress_thumbnail(&ctx, ui, hovered_page);
            }

            // Right-click context menu on the page area.
            if let Some(response) = response {
                // Click left/right halves to turn page (disabled in webtoon mode).
                if mode != ReadingMode::Webtoon && response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let center_x = response.rect.center().x;
                        let rtl = mode == ReadingMode::Rtl;
                        if pos.x < center_x {
                            if rtl {
                                self.reader_next_page();
                            } else {
                                self.reader_prev_page();
                            }
                        } else if rtl {
                            self.reader_prev_page();
                        } else {
                            self.reader_next_page();
                        }
                    }
                }
                response.context_menu(|ui| {
                    self.context_menu_items(ui);
                });
            }

            // Scroll wheel navigation.
            let mut scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if self.settings.invert_scroll {
                scroll = -scroll;
            }
            let modifiers = ui.input(|i| i.modifiers);
            let ctrl_or_cmd = modifiers.command || modifiers.ctrl;
            if ctrl_or_cmd {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    if scroll > 2.0 {
                        reader.zoom_in();
                    } else if scroll < -2.0 {
                        reader.zoom_out();
                    }
                }
            } else if mode != ReadingMode::Webtoon {
                if scroll > 2.0 {
                    self.reader_page_down();
                } else if scroll < -2.0 {
                    self.reader_page_up();
                }
            }

            // Double-click toggles between Original (100%) and Page fit.
            if ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    let new_fit = if reader.state.fit_mode == FitMode::Original {
                        FitMode::Page
                    } else {
                        FitMode::Original
                    };
                    reader.request_fit(new_fit);
                }
            }
        });
    }

    fn render_ebook(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.ebook_view.open.is_none() {
            self.current_view = View::Library;
            return;
        }

        let (fullscreen, mouse_pos, screen_size) = ctx.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or(false),
                i.pointer.latest_pos(),
                i.content_rect().size(),
            )
        });
        let show_toolbar = Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        );
        let show_statusbar = Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        );
        if show_toolbar {
            self.render_ebook_toolbar(ui);
        }
        self.render_ebook_search_bar(ui);
        if show_statusbar {
            self.render_ebook_statusbar(ui);
        }

        // Render the TOC side panel before the central panel so that the
        // WebView bounds shrink to avoid covering the panel.
        if self.ebook_view.show_toc {
            if let Some((chapter, fragment)) = self.ebook_view.render_toc(ui) {
                self.ebook_view.goto_toc(chapter, fragment);
            }
        }

        // 菜单/弹层打开时隐藏 wry webview（停放方案）：egui 弹层无法穿透
        // 原生 webview，隐藏期间正文区以当前主题的阅读背景色填充，
        // 关闭即恢复。menu_overlay_open 与媒体视图共用同一判定，互不影响。
        let menu_open = menu_overlay_open(&ctx);
        self.ebook_view.set_webview_hidden(menu_open);

        let frame = if menu_open {
            egui::Frame::central_panel(&ctx.global_style()).fill(
                crate::views::ebook::ebook_theme_bg(self.settings.ebook.theme),
            )
        } else {
            egui::Frame::central_panel(&ctx.global_style())
        };
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let rect = ui.max_rect();
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(rect.min.x, rect.min.y).into(),
                size: wry::dpi::LogicalSize::new(rect.width(), rect.height()).into(),
            };
            self.ebook_view.update_bounds(bounds);
            self.ebook_view.ui(&ctx, ui);
        });
    }

    /// 播放正常结束（ended 且无 error）时自动续播同目录自然排序的下一集，
    /// 每个打开的媒体只触发一次（auto_next_fired 守卫，open() 时复位）。
    /// 有下一集时经 open_media 走正常打开流程（含历史续播的默认行为），
    /// OSD 通过 pending_open_osd 由 MediaView::open 在新媒体就绪后显示。
    fn maybe_auto_next_media(&mut self, ctx: &egui::Context) {
        let should_fire = match self.media_view.open.as_ref() {
            Some(open) => {
                !self.media_view.auto_next_fired && open.last.ended && open.last.error.is_none()
            }
            None => false,
        };
        if !should_fire {
            return;
        }
        self.media_view.auto_next_fired = true;
        let Some(current) = self.media_view.open.as_ref().map(|o| o.path.clone()) else {
            return;
        };
        match next_media_in_dir(&current) {
            Some(next) => {
                let title = next
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("未知媒体")
                    .to_string();
                self.media_view.pending_open_osd = Some(format!("自动播放下一集：{title}"));
                self.open_media(next);
            }
            None => {
                self.media_view.show_osd(ctx, "已是最后一集".to_string());
            }
        }
    }

    fn render_media(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.media_view.open.is_none() {
            self.current_view = View::Library;
            return;
        }
        self.media_view.sync_state();
        self.maybe_auto_next_media(&ctx);
        self.media_view.tick_osd(&ctx);

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        let screen_size = ctx.content_rect().size();
        // While a menu/dropdown is open, keep the toolbar up (otherwise the
        // dropdown self-dismisses in fullscreen when the pointer leaves the
        // top edge).
        let menu_open = menu_overlay_open(&ctx);
        let show_toolbar = Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        ) || (menu_open && self.settings.show_toolbar);
        let show_seekbar = Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        );
        if show_toolbar {
            self.render_media_toolbar(ui);
        }
        if show_seekbar {
            self.render_media_seekbar(ui);
        }

        // Transparent frame: the video layer composites below the egui
        // surface (Task 4), so the central area must stay unpainted for the
        // video to show through. Audio-only/error states still get an opaque
        // black fill from MediaView::ui.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                // Scroll-wheel volume over the video area. Skipped while an egui
                // popup (字幕/音轨/输出 dropdown) is open under the pointer, so
                // scrolling the popup list does not also change the volume.
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 && ui.rect_contains_pointer(rect) && !ctx.is_pointer_over_egui() {
                    let (acc, steps) =
                        crate::views::media::accumulate_scroll(self.media_view.scroll_acc, scroll);
                    self.media_view.scroll_acc = acc;
                    if steps != 0 {
                        self.adjust_media_volume(&ctx, steps as f64 * 5.0);
                    }
                }
                let overlay = self
                    .media_view
                    .open
                    .as_ref()
                    .map(|o| media_overlay(&o.last))
                    .unwrap_or(MediaOverlay::None);
                // Audio-only or decode error: park the native layer at zero
                // size so the egui placeholder painted by MediaView::ui shows
                // instead of the video. Menus need no parking: the egui
                // surface composites above the video layer now.
                let bounds = if matches!(overlay, MediaOverlay::None) {
                    wry::Rect {
                        position: wry::dpi::LogicalPosition::new(rect.min.x, rect.min.y).into(),
                        size: wry::dpi::LogicalSize::new(rect.width(), rect.height()).into(),
                    }
                } else {
                    wry::Rect {
                        position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
                        size: wry::dpi::LogicalSize::new(0.0, 0.0).into(),
                    }
                };
                self.media_view.update_bounds(bounds);
                self.media_view.ui(&ctx, ui);
            });
    }

    fn render_media_toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (title, tracks, current_sub, current_audio, speed, paused) = self
            .media_view
            .open
            .as_ref()
            .map(|o| {
                (
                    o.title.clone(),
                    o.last.tracks.clone(),
                    o.last.current_sub,
                    o.last.current_audio,
                    o.last.speed,
                    o.last.paused,
                )
            })
            .unwrap_or_default();
        let devices: Vec<(String, String)> = self
            .media_view
            .open
            .as_ref()
            .and_then(|o| o.last.audio_devices.as_ref())
            .map(|ds| ds.iter().map(|d| (d.name.clone(), d.label())).collect())
            .unwrap_or_default();
        let current_device = self.settings.media_audio_device.clone();
        egui::Panel::top("media_toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("书架").clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();
                if ui.button(if paused { "播放" } else { "暂停" }).clicked() {
                    self.media_view.toggle_pause();
                }
                if ui.button("-10s").clicked() {
                    self.seek_media_rel(&ctx, -10.0);
                }
                if ui.button("+10s").clicked() {
                    self.seek_media_rel(&ctx, 10.0);
                }
                ui.separator();
                if ui.button(format!("{:.1}x", speed)).clicked() {
                    if let Some(target) = self.media_view.cycle_speed() {
                        self.settings.media_speed = target;
                        self.media_view
                            .show_osd(&ctx, crate::views::media::speed_osd_text(target));
                    }
                }
                ui.separator();
                let subs: Vec<(i64, String)> = tracks
                    .iter()
                    .filter(|t| t.kind == openitgo_media::TrackKind::Sub)
                    .enumerate()
                    .map(|(i, t)| (t.id, crate::views::media::track_label(t, i)))
                    .collect();
                egui::ComboBox::from_label("字幕")
                    .selected_text(
                        current_sub
                            .and_then(|id| subs.iter().find(|(sid, _)| *sid == id))
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| "关闭".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current_sub.is_none(), "关闭").clicked() {
                            self.media_view.set_sub(None);
                        }
                        for (id, label) in &subs {
                            if ui
                                .selectable_label(current_sub == Some(*id), label)
                                .clicked()
                            {
                                self.media_view.set_sub(Some(*id));
                            }
                        }
                        ui.separator();
                        if ui.button("加载外部字幕…").clicked() {
                            self.load_external_subtitle(&ctx);
                        }
                        ui.separator();
                        if ui.button("字幕延迟 -0.1s").clicked() {
                            self.adjust_media_sub_delay(&ctx, -0.1);
                        }
                        if ui.button("字幕延迟 +0.1s").clicked() {
                            self.adjust_media_sub_delay(&ctx, 0.1);
                        }
                        if ui.button("重置字幕延迟").clicked() {
                            self.reset_media_sub_delay(&ctx);
                        }
                    });
                let audios: Vec<(i64, String)> = tracks
                    .iter()
                    .filter(|t| t.kind == openitgo_media::TrackKind::Audio)
                    .enumerate()
                    .map(|(i, t)| (t.id, crate::views::media::track_label(t, i)))
                    .collect();
                if audios.len() > 1 {
                    egui::ComboBox::from_label("音轨")
                        .selected_text(
                            current_audio
                                .and_then(|id| audios.iter().find(|(aid, _)| *aid == id))
                                .map(|(_, l)| l.clone())
                                .unwrap_or_else(|| "-".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for (id, label) in &audios {
                                if ui
                                    .selectable_label(current_audio == Some(*id), label)
                                    .clicked()
                                {
                                    self.media_view.set_audio(*id);
                                }
                            }
                        });
                }
                ui.separator();
                let current_label = devices
                    .iter()
                    .find(|(n, _)| *n == current_device)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_else(|| "自动".to_string());
                egui::ComboBox::from_label("输出")
                    // 选中项截断显示，避免长设备名撑爆工具栏；下拉列表保留全名
                    .selected_text(crate::views::media::truncate_label(&current_label, 20))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current_device.is_empty(), "自动")
                            .clicked()
                        {
                            self.set_media_audio_device(&ctx, String::new(), "自动".to_string());
                        }
                        for (name, label) in &devices {
                            if ui
                                .selectable_label(current_device == *name, label)
                                .clicked()
                            {
                                self.set_media_audio_device(&ctx, name.clone(), label.clone());
                            }
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("全屏").clicked() {
                        self.toggle_fullscreen(&ctx);
                    }
                    ui.label(title);
                });
            });
        });
    }

    /// 媒体视图菜单：倍速微调/循环播放/截图/AB 循环/章节导航。
    fn render_media_menu(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if ui.button("倍速 -0.25").clicked() {
            self.adjust_media_speed(ctx, -0.25);
            ui.close();
        }
        if ui.button("倍速 +0.25").clicked() {
            self.adjust_media_speed(ctx, 0.25);
            ui.close();
        }
        ui.separator();
        let loop_on = self.media_view.loop_file;
        if ui.selectable_label(loop_on, "循环播放").clicked() {
            self.toggle_media_loop(ctx);
            ui.close();
        }
        if ui.button("截图").clicked() {
            self.media_screenshot(ctx);
            ui.close();
        }
        ui.separator();
        let ab_label = match self.media_view.ab_loop {
            crate::views::media::AbLoop::None => "设置 A 点",
            crate::views::media::AbLoop::ASet(_) => "设置 B 点",
            crate::views::media::AbLoop::Both(..) => "取消 AB 循环",
        };
        if ui.button(ab_label).clicked() {
            self.media_ab_advance(ctx);
            ui.close();
        }
        ui.separator();
        let has_chapters = self
            .media_view
            .open
            .as_ref()
            .map(|o| !o.last.chapters.is_empty())
            .unwrap_or(false);
        if ui
            .add_enabled(has_chapters, egui::Button::new("上一章"))
            .clicked()
        {
            self.media_chapter_step(ctx, -1);
            ui.close();
        }
        if ui
            .add_enabled(has_chapters, egui::Button::new("下一章"))
            .clicked()
        {
            self.media_chapter_step(ctx, 1);
            ui.close();
        }
    }

    fn adjust_media_speed(&mut self, ctx: &egui::Context, delta: f64) {
        if let Some(v) = self.media_view.adjust_speed(delta) {
            self.settings.media_speed = v;
            self.media_view
                .show_osd(ctx, crate::views::media::speed_fine_osd_text(v));
        }
    }

    fn toggle_media_loop(&mut self, ctx: &egui::Context) {
        if let Some(on) = self.media_view.toggle_loop_file() {
            self.media_view.show_osd(
                ctx,
                if on {
                    "循环播放 开".to_string()
                } else {
                    "循环播放 关".to_string()
                },
            );
        }
    }

    fn media_screenshot(&mut self, ctx: &egui::Context) {
        match self.media_view.take_screenshot() {
            Ok(path) => self
                .media_view
                .show_osd(ctx, format!("已保存截图：{}", path.display())),
            Err(e) => {
                self.error_message = Some(format!("截图失败: {e}"));
            }
        }
    }

    fn media_ab_advance(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.media_view.advance_ab_loop() {
            self.media_view.show_osd(ctx, text);
        }
    }

    fn media_chapter_step(&mut self, ctx: &egui::Context, delta: i64) {
        if let Some(text) = self.media_view.chapter_step(delta) {
            self.media_view.show_osd(ctx, text);
        }
    }

    fn set_media_volume(&mut self, ctx: &egui::Context, v: f64) {
        let v = v.clamp(0.0, 100.0);
        self.media_view.set_volume(v);
        self.settings.media_volume = v;
        self.media_view
            .show_osd(ctx, crate::views::media::volume_osd_text(v));
    }

    fn toggle_media_mute(&mut self, ctx: &egui::Context) {
        if let Some(muted) = self.media_view.toggle_mute() {
            self.media_view
                .show_osd(ctx, crate::views::media::mute_osd_text(muted).to_string());
        }
    }

    fn adjust_media_volume(&mut self, ctx: &egui::Context, delta: f64) {
        if let Some(v) = self.media_view.adjust_volume(delta) {
            self.settings.media_volume = v;
            self.media_view
                .show_osd(ctx, crate::views::media::volume_osd_text(v));
        }
    }

    fn seek_media_rel(&mut self, ctx: &egui::Context, secs: f64) {
        self.media_view.seek_rel(secs);
        self.media_view.show_osd(ctx, format!("{:+}s", secs as i32));
    }

    fn set_media_speed(&mut self, ctx: &egui::Context, speed: f64) {
        self.media_view.set_speed(speed);
        self.settings.media_speed = speed;
        self.media_view
            .show_osd(ctx, crate::views::media::speed_osd_text(speed));
    }

    fn set_media_audio_device(&mut self, ctx: &egui::Context, name: String, label: String) {
        match self.media_view.set_audio_device(&name) {
            Ok(()) => {
                self.settings.media_audio_device = name;
                self.media_view.show_osd(ctx, format!("输出: {label}"));
            }
            Err(e) => {
                self.error_message = Some(format!("无法切换音频输出设备: {e}"));
            }
        }
    }

    fn load_external_subtitle(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("字幕", &["srt", "ass", "ssa", "vtt"])
            .pick_file()
        else {
            return;
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("未知字幕")
            .to_string();
        match self.media_view.sub_add(&path) {
            Ok(()) => self.media_view.show_osd(ctx, format!("已加载字幕：{name}")),
            Err(e) => {
                self.error_message = Some(format!("无法加载字幕: {e}"));
            }
        }
    }

    fn adjust_media_sub_delay(&mut self, ctx: &egui::Context, delta: f64) {
        if let Some(v) = self.media_view.adjust_sub_delay(delta) {
            self.media_view.show_osd(
                ctx,
                format!("字幕延迟 {}s", crate::views::media::format_sub_delay(v)),
            );
        }
    }

    fn reset_media_sub_delay(&mut self, ctx: &egui::Context) {
        self.media_view.reset_sub_delay();
        self.media_view.show_osd(
            ctx,
            format!("字幕延迟 {}s", crate::views::media::format_sub_delay(0.0)),
        );
    }

    fn render_media_seekbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (pos, dur, volume, muted) = self
            .media_view
            .open
            .as_ref()
            .map(|o| {
                (
                    o.last.position_ms,
                    o.last.duration_ms,
                    o.last.volume,
                    o.last.muted,
                )
            })
            .unwrap_or((0, None, 100.0, false));
        egui::Panel::bottom("media_seekbar").show(ui, |ui| {
            ui.vertical(|ui| {
                // Row 1: full-width seek bar with a hover-time tooltip.
                // egui 0.35 Slider still allocates spacing().slider_width
                // (100px) and ignores add_sized (egui-0.35.0
                // widgets/slider.rs:652-653), so override the width in a
                // scoped Ui — without leaking it to the row-2 volume slider.
                match dur {
                    Some(d) if d > 0 => {
                        let mut ratio = pos as f32 / d as f32;
                        let slider = egui::Slider::new(&mut ratio, 0.0..=1.0).show_value(false);
                        let width = ui.available_width();
                        let response = ui
                            .scope(|ui| {
                                ui.spacing_mut().slider_width = width;
                                ui.add(slider)
                            })
                            .inner;
                        let hover_text = response.hover_pos().and_then(|hover| {
                            crate::views::media::hover_time_at(hover.x, response.rect, dur)
                                .map(openitgo_media::time::format_time_ms)
                        });
                        if response.drag_stopped() {
                            self.media_view.seek_to_ratio_exact(ratio as f64);
                        } else if response.changed() {
                            self.media_view.seek_to_ratio(ratio as f64);
                        }
                        if let Some(text) = hover_text {
                            // Consumes the response, so it goes last.
                            response.on_hover_text(text);
                        }
                    }
                    _ => {
                        let width = ui.available_width();
                        ui.scope(|ui| {
                            ui.spacing_mut().slider_width = width;
                            ui.add_enabled(
                                false,
                                egui::Slider::new(&mut 0.0f32, 0.0..=1.0).show_value(false),
                            )
                        });
                    }
                }
                // Row 2: time display on the left; mute + volume on the right.
                ui.horizontal(|ui| {
                    let dur_text = dur
                        .map(openitgo_media::time::format_time_ms)
                        .unwrap_or_else(|| "--:--".to_string());
                    ui.label(format!(
                        "{} / {}",
                        openitgo_media::time::format_time_ms(pos),
                        dur_text
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut vol = volume as f32;
                        if ui
                            .add_enabled(
                                !muted,
                                egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false),
                            )
                            .changed()
                        {
                            self.set_media_volume(&ctx, vol as f64);
                        }
                        if ui.button(if muted { "取消静音" } else { "静音" }).clicked() {
                            self.toggle_media_mute(&ctx);
                        }
                    });
                });
            });
        });
    }

    fn render_reader_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        total_pages: usize,
        current_page: usize,
        mode: ReadingMode,
        zoom: f32,
    ) {
        let ctx = ui.ctx().clone();
        let display_mode = self.settings.toolbar_display_mode;
        egui::Panel::top("reader_toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if toolbar_button(ui, icons::HOUSE, "书架", display_mode).clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();

                let modes = [
                    (ReadingMode::Ltr, icons::ARROW_RIGHT, "国漫"),
                    (ReadingMode::Rtl, icons::ARROW_LEFT, "日漫"),
                    (ReadingMode::Webtoon, icons::ARROW_DOWN, "韩漫"),
                ];
                for (m, icon, label) in modes {
                    if toolbar_selectable(ui, icon, label, mode == m, display_mode).clicked() {
                        if let Some(reader) = self.reader_view.open.as_mut() {
                            reader.state.set_mode(m, total_pages);
                        }
                    }
                }
                ui.separator();

                if mode != ReadingMode::Webtoon {
                    let double_page = self
                        .reader_view
                        .open
                        .as_ref()
                        .map(|r| r.state.double_page)
                        .unwrap_or(self.settings.double_page);
                    if toolbar_selectable(ui, icons::BOOK_OPEN, "双页", double_page, display_mode)
                        .on_hover_text("切换到双页模式")
                        .clicked()
                    {
                        let new_double = !double_page;
                        self.settings.double_page = new_double;
                        if let Some(reader) = self.reader_view.open.as_mut() {
                            reader.state.set_double_page(new_double, total_pages);
                            reader.pending_fit = Some(FitMode::Page);
                        }
                    }
                    ui.separator();
                }

                if toolbar_button(ui, icons::MINUS, "", display_mode).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_out();
                    }
                }
                ui.label(format!("{:.0}%", zoom * 100.0));
                if toolbar_button(ui, icons::PLUS, "", display_mode).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_in();
                    }
                }
                if toolbar_button(
                    ui,
                    icons::ARROWS_OUT_LINE_HORIZONTAL,
                    "适应宽度",
                    display_mode,
                )
                .clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Width);
                    }
                }
                if toolbar_button(
                    ui,
                    icons::ARROWS_OUT_LINE_VERTICAL,
                    "适应高度",
                    display_mode,
                )
                .clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Height);
                    }
                }
                if toolbar_button(ui, icons::FRAME_CORNERS, "自动适应", display_mode).clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Page);
                    }
                }
                if toolbar_button(ui, icons::ARROW_CLOCKWISE, "旋转", display_mode)
                    .on_hover_text("顺时针旋转 90°")
                    .clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.rotate_cw();
                    }
                }
                ui.separator();

                if toolbar_button(ui, icons::CARET_LEFT, "上一页", display_mode).clicked() {
                    self.reader_prev_page();
                }
                let mut displayed_page = current_page + 1;
                let page_response = ui.add(
                    egui::DragValue::new(&mut displayed_page)
                        .speed(1.0)
                        .range(1..=total_pages.max(1))
                        .update_while_editing(false),
                );
                if page_response.lost_focus() {
                    let target = displayed_page
                        .saturating_sub(1)
                        .min(total_pages.saturating_sub(1));
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        if target != reader.state.current_page {
                            reader.state.go_to_page(target, total_pages);
                            reader.mark_page_turn();
                        }
                    }
                }
                ui.label(format!("/ {}", total_pages));
                if toolbar_button(ui, icons::CARET_RIGHT, "下一页", display_mode).clicked() {
                    self.reader_next_page();
                }
                ui.separator();

                if toolbar_button(ui, icons::BOOKMARK, "添加书签", display_mode).clicked() {
                    self.add_bookmark(current_page);
                }
                if toolbar_button(ui, icons::ARROWS_OUT_SIMPLE, "全屏", display_mode).clicked() {
                    self.toggle_fullscreen(&ctx);
                }
                if toolbar_button(ui, icons::GEAR, "设置", display_mode).clicked() {
                    self.current_view = View::Settings;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(icons::X).on_hover_text("隐藏工具栏").clicked() {
                        self.settings.show_toolbar = false;
                    }
                });
            });
        });
    }

    fn render_reader_statusbar(
        &mut self,
        ui: &mut egui::Ui,
    ) -> (Option<egui::Rect>, Option<usize>) {
        let mut progress_rect = None;
        let mut hovered_page = None;
        egui::Panel::bottom("reader_statusbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let bar_response = self.reader_view.render_progress_bar(ui);
                hovered_page = bar_response.hovered_page;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").on_hover_text("隐藏状态栏").clicked() {
                        self.settings.show_statusbar = false;
                    }
                });
            });
            progress_rect = Some(ui.min_rect());
        });
        (progress_rect, hovered_page)
    }

    fn render_ebook_toolbar(&mut self, ui: &mut egui::Ui) {
        self.ebook_view.sync_position();
        let (total, current, current_spread, total_spreads) = self
            .ebook_view
            .open
            .as_ref()
            .map(|e| {
                (
                    e.ebook.total_chapters(),
                    e.current_chapter,
                    e.current_spread,
                    e.renderer.current_spread_count(),
                )
            })
            .unwrap_or((0, 0, 0, 0));

        egui::Panel::top("ebook_toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("书架").clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();
                if ui.button("目录").clicked() {
                    self.ebook_view.toggle_toc();
                }
                if ui.button("搜索").clicked() {
                    self.ebook_view.toggle_search();
                }
                if ui.button("上一章").clicked() {
                    self.ebook_view.prev_chapter();
                }
                if ui.button("下一章").clicked() {
                    self.ebook_view.next_chapter();
                }
                if ui.button("上一页").clicked() {
                    self.ebook_view.prev_page();
                }
                if ui.button("下一页").clicked() {
                    self.ebook_view.next_page();
                }
                ui.separator();
                if ui.button("添加书签").clicked() {
                    self.add_ebook_bookmark();
                }
                ui.separator();
                if ui.button("A-").clicked() {
                    self.settings.ebook.font_size =
                        self.settings.ebook.font_size.saturating_sub(1).max(10);
                    self.ebook_view.apply_settings(&self.settings.ebook);
                }
                if ui.button("A+").clicked() {
                    self.settings.ebook.font_size = (self.settings.ebook.font_size + 1).min(72);
                    self.ebook_view.apply_settings(&self.settings.ebook);
                }
                if ui.button("主题").clicked() {
                    self.settings.ebook.theme = match self.settings.ebook.theme {
                        EbookTheme::Light => EbookTheme::Dark,
                        EbookTheme::Dark => EbookTheme::Sepia,
                        EbookTheme::Sepia => EbookTheme::Light,
                    };
                    self.ebook_view.apply_settings(&self.settings.ebook);
                }
                let (_, page_label) =
                    Self::ebook_status_text(current, total, current_spread, total_spreads, None);
                ui.label(page_label);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("设置").clicked() {
                        self.current_view = View::Settings;
                    }
                });
            });
        });
    }

    fn render_ebook_search_bar(&mut self, ui: &mut egui::Ui) {
        if !self.ebook_view.search_visible() {
            return;
        }
        egui::Panel::top("ebook_search_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("搜索:");
                let mut submitted = None::<bool>; // Some(true)=下一个, Some(false)=上一个
                let mut close_requested = false;
                if let Some(open) = self.ebook_view.open.as_mut() {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut open.search.query)
                            .id(egui::Id::new("ebook_search_input"))
                            .desired_width(240.0),
                    );
                    if open.search.take_focus_request() {
                        response.request_focus();
                    }
                    if response.changed() {
                        let q = open.search.query.clone();
                        open.renderer.find_text(&q);
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submitted = Some(!ui.input(|i| i.modifiers.shift));
                    }
                    let (count, active) = open.renderer.search_state();
                    let label = if count == 0 {
                        "0/0".to_string()
                    } else {
                        format!("{}/{}", active.max(0) as usize + 1, count)
                    };
                    ui.label(label);
                    if ui
                        .add_enabled(count > 0, egui::Button::new("上一个"))
                        .clicked()
                    {
                        open.renderer.find_prev();
                    }
                    if ui
                        .add_enabled(count > 0, egui::Button::new("下一个"))
                        .clicked()
                    {
                        open.renderer.find_next();
                    }
                    if ui.button("✕").clicked() {
                        close_requested = true;
                    }
                }
                if close_requested {
                    self.ebook_view.close_search();
                }
                match submitted {
                    Some(true) => self.ebook_view.find_next(),
                    Some(false) => self.ebook_view.find_prev(),
                    None => {}
                }
            });
        });
    }

    /// Returns the status-bar text for an ebook: chapter title and spread progress.
    fn ebook_status_text(
        current_chapter: usize,
        total_chapters: usize,
        current_spread: usize,
        total_spreads: usize,
        title: Option<&str>,
    ) -> (String, String) {
        let title = title.unwrap_or("无标题").to_string();
        let chapter = if total_chapters > 0 {
            format!("第 {} / {} 章", current_chapter + 1, total_chapters)
        } else {
            "无章节".to_string()
        };
        let progress = if total_spreads > 0 {
            format!(
                "第 {} / {} 章 · 第 {} / {} 页",
                current_chapter + 1,
                total_chapters,
                current_spread + 1,
                total_spreads
            )
        } else {
            chapter
        };
        (title, progress)
    }

    fn render_ebook_statusbar(&mut self, ui: &mut egui::Ui) {
        self.ebook_view.sync_position();
        let (title, progress) = self
            .ebook_view
            .open
            .as_ref()
            .map(|e| {
                let title = e
                    .ebook
                    .chapters
                    .get(e.current_chapter)
                    .and_then(|c| c.title.as_deref());
                Self::ebook_status_text(
                    e.current_chapter,
                    e.ebook.total_chapters(),
                    e.current_spread,
                    e.renderer.current_spread_count(),
                    title,
                )
            })
            .unwrap_or_else(|| ("".to_string(), "".to_string()));

        egui::Panel::bottom("ebook_statusbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(progress);
                });
            });
        });
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        let from_ebook = self.ebook_view.open.is_some();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("← 返回").clicked() {
                    self.current_view = if self.reader_view.open.is_some() {
                        View::Reader
                    } else if self.ebook_view.open.is_some() {
                        View::Ebook
                    } else {
                        View::Library
                    };
                }
            });
            ui.separator();
            if from_ebook {
                self.settings_view.ebook_ui(ui, &mut self.settings);
            } else {
                self.settings_view.ui(ui, &mut self.settings);
            }
        });
        if from_ebook {
            self.ebook_view.apply_settings(&self.settings.ebook);
        }
    }

    fn render_loading(&mut self, ui: &mut egui::Ui, path: PathBuf) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.vertical_centered(|ui| {
                        // Static icon avoids the continuous repaint that egui's
                        // animated spinner triggers.
                        ui.label(egui::RichText::new("⏳").size(24.0));
                        ui.label("正在打开漫画...");
                        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                            ui.label(egui::RichText::new(name).size(14.0).strong());
                        }
                        ui.add_space(16.0);
                        if ui.button("取消").clicked() {
                            self.opener = None;
                            self.ebook_opener = None;
                            self.current_view = View::Library;
                        }
                        if let Some(err) = &self.error_message {
                            ui.colored_label(ui.visuals().error_fg_color, err);
                        }
                    });
                },
            );
        });
    }

    fn render_menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("文件", |ui| {
                    if ui.button("打开文件夹").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.add_folder_to_library(path);
                        }
                        ui.close();
                    }
                    ui.menu_button("打开最近", |ui| {
                        let recent: Vec<_> = self
                            .history
                            .entries
                            .iter()
                            .filter_map(|h| {
                                self.library_view
                                    .find_by_id(&h.comic_id)
                                    .map(|e| (e.title.clone(), e.path.clone()))
                            })
                            .collect();
                        if recent.is_empty() {
                            ui.weak("暂无记录");
                        } else {
                            for (title, path) in recent {
                                if ui.button(&title).clicked() {
                                    self.open_path(path);
                                    ui.close();
                                }
                            }
                        }
                    });
                    ui.separator();
                    let can_back = matches!(self.current_view, View::Reader | View::Settings);
                    if ui
                        .add_enabled(can_back, egui::Button::new("返回书架"))
                        .clicked()
                    {
                        self.current_view = View::Library;
                        ui.close();
                    }
                });

                ui.menu_button("视图", |ui| {
                    let toolbar_label = if self.settings.show_toolbar {
                        "隐藏工具栏"
                    } else {
                        "显示工具栏"
                    };
                    if ui.button(toolbar_label).clicked() {
                        self.settings.show_toolbar = !self.settings.show_toolbar;
                        ui.close();
                    }
                    let statusbar_label = if self.settings.show_statusbar {
                        "隐藏状态栏"
                    } else {
                        "显示状态栏"
                    };
                    if ui.button(statusbar_label).clicked() {
                        self.settings.show_statusbar = !self.settings.show_statusbar;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("全屏").clicked() {
                        self.toggle_fullscreen(&ctx);
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("主题", |ui| {
                        use openitgo_storage::models::Theme;
                        for (theme, label) in [
                            (Theme::System, "跟随系统"),
                            (Theme::Light, "浅色"),
                            (Theme::Dark, "深色"),
                        ] {
                            if ui
                                .selectable_label(self.settings.theme == theme, label)
                                .clicked()
                            {
                                self.settings.theme = theme;
                                ui.close();
                            }
                        }
                    });
                });

                let is_reader = matches!(self.current_view, View::Reader);
                let is_ebook = matches!(self.current_view, View::Ebook);
                ui.add_enabled_ui(is_reader || is_ebook, |ui| {
                    ui.menu_button("阅读", |ui| {
                        if is_ebook {
                            self.render_ebook_menu(ui);
                        } else {
                            self.render_reader_menu(ui);
                        }
                    });
                });

                let is_media = matches!(self.current_view, View::Media);
                ui.add_enabled_ui(is_media, |ui| {
                    ui.menu_button("播放", |ui| {
                        self.render_media_menu(&ctx, ui);
                    });
                });

                ui.menu_button("工具", |ui| {
                    if ui.button("设置").clicked() {
                        self.current_view = View::Settings;
                        ui.close();
                    }
                });

                ui.menu_button("帮助", |ui| {
                    if ui.button("快捷键一览").clicked() {
                        self.show_shortcuts = true;
                        ui.close();
                    }
                    if ui.button("关于 OpenItGo").clicked() {
                        self.error_message = Some(format!(
                            "OpenItGo v{}\n一个用 Rust 写的漫画阅读器。",
                            env!("CARGO_PKG_VERSION")
                        ));
                        ui.close();
                    }
                });
            });
        });
    }

    /// 帮助菜单的只读快捷键一览面板（编辑入口在 设置 → 快捷键）。
    fn render_shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = self.show_shortcuts;
        egui::Window::new("快捷键一览")
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("可自定义快捷键（修改入口：设置 → 快捷键）");
                egui::Grid::new("shortcuts_configurable_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (action, keys) in
                            crate::shortcuts::configurable_shortcut_rows(&self.settings.shortcuts)
                        {
                            ui.label(action);
                            ui.label(keys);
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label("内置快捷键（阅读器）");
                egui::Grid::new("shortcuts_reader_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, action) in crate::shortcuts::hardcoded_reader_rows() {
                            ui.label(keys);
                            ui.label(action);
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label("内置快捷键（媒体）");
                egui::Grid::new("shortcuts_media_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, action) in crate::shortcuts::hardcoded_media_rows() {
                            ui.label(keys);
                            ui.label(action);
                            ui.end_row();
                        }
                    });
            });
        self.show_shortcuts = open;
    }

    /// 加密压缩包密码对话框：确认后缓存密码（会话级）并重试打开/导入；
    /// 取消则放弃打开，导入流程中则跳过该文件。
    fn render_password_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.password_dialog.as_mut() else {
            return;
        };
        let name = dialog
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("压缩包")
            .to_string();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("输入密码")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("「{name}」已加密，请输入密码："));
                if dialog.incorrect {
                    ui.colored_label(ui.visuals().error_fg_color, "密码错误，请重试。");
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut dialog.input)
                        .password(true)
                        .desired_width(260.0),
                );
                if !response.has_focus() {
                    response.request_focus();
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });
        if confirm {
            let dialog = self.password_dialog.take().unwrap();
            let pw = dialog.input.trim().to_string();
            if !pw.is_empty() {
                self.passwords.insert(dialog.path.clone(), pw);
                self.sync_passwords_to_loaders();
            }
            if self.pending_password_imports.contains(&dialog.path) {
                self.retry_password_import(dialog.path);
            } else {
                self.open_path(dialog.path);
            }
        } else if cancel {
            let dialog = self.password_dialog.take().unwrap();
            if self.pending_password_imports.contains(&dialog.path) {
                self.skip_password_import(dialog.path);
            }
        }
    }

    fn sync_passwords_to_loaders(&self) {
        for loader in [&self.page_loader, &self.cover_loader] {
            *loader.passwords().write().unwrap() = self.passwords.clone();
        }
    }

    fn retry_password_import(&mut self, path: PathBuf) {
        self.pending_password_imports.retain(|p| p != &path);
        self.add_file_to_library(path);
        self.advance_password_import_queue();
    }

    fn skip_password_import(&mut self, path: PathBuf) {
        self.pending_password_imports.retain(|p| p != &path);
        self.skipped_encrypted_imports += 1;
        self.advance_password_import_queue();
    }

    fn advance_password_import_queue(&mut self) {
        if let Some(next) = self.pending_password_imports.first().cloned() {
            // retry_password_import 里 add_file_to_library 可能刚用
            // PasswordIncorrect 打开过对话框——不要覆盖它。
            if self.password_dialog.is_none() {
                self.password_dialog =
                    Some(PasswordDialog::new(next, PasswordPromptKind::Required));
            }
        } else if self.skipped_encrypted_imports > 0 {
            self.error_message = Some(format!(
                "已跳过 {} 个加密压缩包",
                self.skipped_encrypted_imports
            ));
            self.skipped_encrypted_imports = 0;
        }
    }

    fn render_reader_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("上一页").clicked() {
            self.reader_prev_page();
            ui.close();
        }
        if ui.button("下一页").clicked() {
            self.reader_next_page();
            ui.close();
        }
        ui.separator();
        if ui.button("首页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
            ui.close();
        }
        if ui.button("末页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
            ui.close();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            if let Some(reader) = self.reader_view.open.as_ref() {
                self.add_bookmark(reader.state.current_page);
            }
            ui.close();
        }
        ui.separator();
        ui.menu_button("模式", |ui| {
            let mode = self
                .reader_view
                .open
                .as_ref()
                .map(|r| r.state.mode)
                .unwrap_or(self.settings.default_mode);
            let total_pages = self
                .reader_view
                .open
                .as_ref()
                .map(|r| r.total_pages())
                .unwrap_or(0);
            for (m, label) in [
                (ReadingMode::Ltr, "国漫（从左到右）"),
                (ReadingMode::Rtl, "日漫（从右到左）"),
                (ReadingMode::Webtoon, "韩漫（条漫）"),
            ] {
                if ui.selectable_label(mode == m, label).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.state.set_mode(m, total_pages);
                    }
                    ui.close();
                }
            }
        });
        let mode = self
            .reader_view
            .open
            .as_ref()
            .map(|r| r.state.mode)
            .unwrap_or(self.settings.default_mode);
        let double_page = self
            .reader_view
            .open
            .as_ref()
            .map(|r| r.state.double_page)
            .unwrap_or(self.settings.double_page);
        if ui
            .add_enabled(
                !mode.is_webtoon(),
                egui::Button::new(if double_page {
                    "关闭双页"
                } else {
                    "双页"
                }),
            )
            .clicked()
        {
            let new_double = !double_page;
            self.settings.double_page = new_double;
            if let Some(reader) = self.reader_view.open.as_mut() {
                let total = reader.total_pages();
                reader.state.set_double_page(new_double, total);
                reader.pending_fit = Some(FitMode::Page);
            }
            ui.close();
        }
        let rotation = self
            .reader_view
            .open
            .as_ref()
            .map(|r| r.state.rotation)
            .unwrap_or(0);
        if ui
            .button(format!("旋转 90°（当前 {}°）", rotation))
            .clicked()
        {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.rotate_cw();
            }
            ui.close();
        }
        ui.separator();
        ui.menu_button("缩放", |ui| {
            if ui.button("放大").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.zoom_in();
                }
                ui.close();
            }
            if ui.button("缩小").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.zoom_out();
                }
                ui.close();
            }
            ui.separator();
            if ui.button("适应宽度").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Width);
                }
                ui.close();
            }
            if ui.button("适应高度").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Height);
                }
                ui.close();
            }
            if ui.button("自动适应").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Page);
                }
                ui.close();
            }
        });
    }

    fn render_ebook_bookmarks(&mut self, ui: &mut egui::Ui) {
        let Some(open) = self.ebook_view.open.as_ref() else {
            return;
        };
        let comic_id = open.ebook.id.clone();
        let chapters: Vec<Option<String>> = open
            .ebook
            .chapters
            .iter()
            .map(|c| c.title.clone())
            .collect();
        let entries: Vec<(usize, usize, Option<String>)> = self
            .bookmarks
            .entries
            .iter()
            .enumerate()
            .filter(|(_, b)| b.comic_id == comic_id)
            .map(|(idx, b)| (idx, b.page_index, b.note.clone()))
            .collect();
        if entries.is_empty() {
            return;
        }
        ui.menu_button("书签", |ui| {
            for (idx, chapter, note) in entries {
                let title = chapters
                    .get(chapter)
                    .cloned()
                    .flatten()
                    .unwrap_or_else(|| format!("第 {} 章", chapter + 1));
                let label = note.unwrap_or(title);
                if ui.button(label).clicked() {
                    self.ebook_view.goto_chapter(chapter);
                    ui.close();
                }
                if ui.small_button("删除").clicked() {
                    self.bookmarks.entries.remove(idx);
                    ui.close();
                }
            }
        });
    }

    fn render_ebook_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("上一章").clicked() {
            self.ebook_view.prev_chapter();
            ui.close();
        }
        if ui.button("下一章").clicked() {
            self.ebook_view.next_chapter();
            ui.close();
        }
        if ui.button("上一页").clicked() {
            self.ebook_view.prev_page();
            ui.close();
        }
        if ui.button("下一页").clicked() {
            self.ebook_view.next_page();
            ui.close();
        }
        ui.separator();
        if ui.button("目录").clicked() {
            self.ebook_view.toggle_toc();
            ui.close();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            self.add_ebook_bookmark();
            ui.close();
        }
        self.render_ebook_bookmarks(ui);
        ui.separator();
        if ui.button("增大字体").clicked() {
            self.settings.ebook.font_size = (self.settings.ebook.font_size + 1).min(72);
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close();
        }
        if ui.button("减小字体").clicked() {
            self.settings.ebook.font_size = self.settings.ebook.font_size.saturating_sub(1).max(10);
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close();
        }
        ui.separator();
        if ui.button("切换主题").clicked() {
            self.settings.ebook.theme = match self.settings.ebook.theme {
                EbookTheme::Light => EbookTheme::Dark,
                EbookTheme::Dark => EbookTheme::Sepia,
                EbookTheme::Sepia => EbookTheme::Light,
            };
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close();
        }
    }

    fn context_menu_items(&mut self, ui: &mut egui::Ui) {
        if ui.button("下一页").clicked() {
            self.reader_next_page();
            ui.close();
        }
        if ui.button("上一页").clicked() {
            self.reader_prev_page();
            ui.close();
        }
        ui.separator();
        if ui.button("首页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
            ui.close();
        }
        if ui.button("末页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
            ui.close();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            if let Some(reader) = self.reader_view.open.as_ref() {
                self.add_bookmark(reader.state.current_page);
            }
            ui.close();
        }
        if ui.button("全屏").clicked() {
            self.toggle_fullscreen(ui.ctx());
            ui.close();
        }
        ui.separator();
        let toolbar_label = if self.settings.show_toolbar {
            "隐藏工具栏"
        } else {
            "显示工具栏"
        };
        if ui.button(toolbar_label).clicked() {
            self.settings.show_toolbar = !self.settings.show_toolbar;
            ui.close();
        }
        let statusbar_label = if self.settings.show_statusbar {
            "隐藏状态栏"
        } else {
            "显示状态栏"
        };
        if ui.button(statusbar_label).clicked() {
            self.settings.show_statusbar = !self.settings.show_statusbar;
            ui.close();
        }
        ui.separator();
        if ui.button("返回书架").clicked() {
            self.current_view = View::Library;
            ui.close();
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let preference = match self.settings.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        };
        ctx.set_theme(preference);
    }

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.fullscreen) {
            self.toggle_fullscreen(ctx);
        }

        match self.current_view {
            View::Reader => {
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.back_to_library) {
                    if ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
                        self.toggle_fullscreen(ctx);
                    } else {
                        self.current_view = View::Library;
                    }
                }

                let rtl = self
                    .reader_view
                    .open
                    .as_ref()
                    .map(|r| r.state.mode == ReadingMode::Rtl)
                    .unwrap_or(false);
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                    if rtl {
                        self.reader_prev_page();
                    } else {
                        self.reader_next_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                    if rtl {
                        self.reader_next_page();
                    } else {
                        self.reader_prev_page();
                    }
                }
                // 首页/末页不随 RTL 翻转：首页永远是第一页
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.first_page) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.first_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.last_page) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.last_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_down) {
                    self.reader_page_down();
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_up) {
                    self.reader_page_up();
                }

                // Mouse side buttons: Extra1 is typically "back" and Extra2 is "forward".
                let extra1_pressed =
                    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1));
                let extra2_pressed =
                    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2));
                if extra1_pressed {
                    if rtl {
                        self.reader_next_page();
                    } else {
                        self.reader_prev_page();
                    }
                }
                if extra2_pressed {
                    if rtl {
                        self.reader_prev_page();
                    } else {
                        self.reader_next_page();
                    }
                }

                if is_shortcut_pressed(ctx, &self.settings.shortcuts.zoom_in) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_in();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.zoom_out) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_out();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_page) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Page);
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_width) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Width);
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_height) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Height);
                    }
                }
            }
            View::Ebook => {
                if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.command) {
                    self.ebook_view.toggle_search();
                }
                // 文本框聚焦时不响应翻页类全局键，避免与输入冲突（如搜索框
                // 里按 Space）。
                if !ctx.egui_wants_keyboard_input() {
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                        self.ebook_view.next_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                        self.ebook_view.prev_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_down) {
                        self.ebook_view.next_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_up) {
                        self.ebook_view.prev_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.back_to_library) {
                    if self.ebook_view.search_visible() {
                        self.ebook_view.close_search();
                    } else {
                        self.current_view = View::Library;
                    }
                }
            }
            View::Media => {
                if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                    self.media_view.toggle_pause();
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    self.seek_media_rel(ctx, 5.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    self.seek_media_rel(ctx, -5.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::J)) {
                    self.seek_media_rel(ctx, -10.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::L)) {
                    self.seek_media_rel(ctx, 10.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    self.adjust_media_volume(ctx, 5.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    self.adjust_media_volume(ctx, -5.0);
                }
                for (key, speed) in [
                    (egui::Key::Num1, 0.5),
                    (egui::Key::Num2, 1.0),
                    (egui::Key::Num3, 1.5),
                    (egui::Key::Num4, 2.0),
                ] {
                    if ctx.input(|i| i.key_pressed(key)) {
                        self.set_media_speed(ctx, speed);
                    }
                }
                if ctx.input(|i| i.key_pressed(egui::Key::V)) {
                    self.media_view.cycle_sub();
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Z)) {
                    self.adjust_media_sub_delay(ctx, -0.1);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                    self.adjust_media_sub_delay(ctx, 0.1);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::M)) {
                    self.toggle_media_mute(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::F)) {
                    self.toggle_fullscreen(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::OpenBracket)) {
                    self.adjust_media_speed(ctx, -0.25);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::CloseBracket)) {
                    self.adjust_media_speed(ctx, 0.25);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::A)) {
                    self.media_ab_advance(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    if fullscreen {
                        self.toggle_fullscreen(ctx);
                    } else {
                        self.current_view = View::Library;
                    }
                }
            }
            _ => {}
        }
    }

    fn reader_next_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.next_page_with_animation();
        }
    }

    fn reader_prev_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.prev_page_with_animation();
        }
    }

    fn reader_page_down(&mut self) {
        self.reader_next_page();
    }

    fn reader_page_up(&mut self) {
        self.reader_prev_page();
    }

    fn toggle_fullscreen(&self, ctx: &egui::Context) {
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
    }

    fn should_show_bar(
        show_setting: bool,
        fullscreen: bool,
        mouse_pos: Option<egui::Pos2>,
        screen_size: egui::Vec2,
        edge: BarEdge,
    ) -> bool {
        if !show_setting {
            return false;
        }
        if !fullscreen {
            return true;
        }
        const THRESHOLD: f32 = 20.0;
        match edge {
            BarEdge::Top => mouse_pos.map(|p| p.y <= THRESHOLD).unwrap_or(false),
            BarEdge::Bottom => mouse_pos
                .map(|p| p.y >= screen_size.y - THRESHOLD)
                .unwrap_or(false),
        }
    }

    /// 每帧检测当前打开漫画的阅读设置（模式/双页/缩放）与上次写盘值是否不同：
    /// 不同则 upsert 到每书记忆表并立即写盘（用户手势频率，直接写即可）。
    /// 集中在帧尾做快照对比，可以捕获菜单、工具栏、快捷键、双击切 fit 等
    /// 所有修改来源。保存失败时快照照样更新，避免每帧重复轰炸错误信息。
    fn maybe_save_comic_settings(&mut self) {
        let Some(reader) = self.reader_view.open.as_ref() else {
            self.last_saved_comic_settings = None;
            return;
        };
        let snapshot = comic_reading_settings_snapshot(&reader.comic.id, &reader.state);
        if self.last_saved_comic_settings.as_ref() == Some(&snapshot) {
            return;
        }
        self.comic_settings.insert(snapshot.0.clone(), snapshot.1);
        if let Err(e) = self.store.save_comic_settings(&self.comic_settings) {
            self.error_message = Some(format!("无法保存阅读设置: {}", e));
        }
        self.last_saved_comic_settings = Some(snapshot);
    }

    /// 当前打开中的读物 id（漫画/电子书/媒体都算），无则 None。
    fn current_reading_id(&self) -> Option<String> {
        match self.current_view {
            View::Reader => self.reader_view.open.as_ref().map(|r| r.comic.id.clone()),
            View::Ebook => self.ebook_view.open.as_ref().map(|e| e.ebook.id.clone()),
            View::Media => self
                .media_view
                .open
                .as_ref()
                .map(|o| openitgo_parser::stable_comic_id(&o.path)),
            _ => None,
        }
    }

    /// 每帧调用：维护阅读会话，满 30s 或换书/合书时结算增量并落盘。
    /// 退出丢失最后 <30s 的增量，可接受（spec 风险表）。
    fn tick_reading_stats(&mut self) {
        const STATS_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        let current = self.current_reading_id();
        match (&self.stats_session, current.clone()) {
            (Some((id, since)), Some(cur)) if *id == cur => {
                if since.elapsed() >= STATS_FLUSH_INTERVAL {
                    self.flush_reading_stats();
                }
            }
            (Some(_), _) => {
                self.flush_reading_stats();
                self.stats_session = current.map(|id| (id, std::time::Instant::now()));
            }
            (None, Some(cur)) => {
                self.stats_session = Some((cur, std::time::Instant::now()));
            }
            (None, None) => {}
        }
    }

    /// 把当前会话经过的时间计入 reading_stats 并落盘，然后以原 id 重启会话。
    fn flush_reading_stats(&mut self) {
        let Some((id, since)) = self.stats_session.take() else {
            return;
        };
        let seconds = since.elapsed().as_secs();
        if seconds > 0 {
            let now_ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            self.reading_stats
                .entry(id.clone())
                .or_default()
                .accumulate(seconds, now_ts);
            if let Err(e) = self.store.save_reading_stats(&self.reading_stats) {
                self.error_message = Some(format!("无法保存阅读统计: {}", e));
            }
        }
        self.stats_session = Some((id, std::time::Instant::now()));
    }

    fn record_reader_history(&mut self) {
        if let Some(reader) = self.reader_view.open.as_ref() {
            let comic_id = reader.comic.id.clone();
            let path = reader.comic.path.clone();
            let page_index = reader.state.current_page;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| history_matches(h, &comic_id, &path))
            {
                entry.comic_id = comic_id;
                entry.path = path;
                entry.page_index = page_index;
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id,
                    path,
                    volume_index: 0,
                    page_index,
                    char_offset: None,
                    last_read_at: now,
                });
            }
        }
    }

    fn record_ebook_history(&mut self) {
        self.ebook_view.sync_position();
        if let Some(open) = self.ebook_view.open.as_ref() {
            let ebook_id = open.ebook.id.clone();
            let path = open.ebook.path.clone();
            let chapter = open.current_chapter;
            let char_offset = open.renderer.current_position().1;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| history_matches(h, &ebook_id, &path))
            {
                entry.comic_id = ebook_id;
                entry.path = path;
                entry.page_index = chapter;
                entry.char_offset = Some(char_offset);
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id: ebook_id,
                    path,
                    volume_index: 0,
                    page_index: chapter,
                    char_offset: Some(char_offset),
                    last_read_at: now,
                });
            }
        }
    }

    fn covers_dir(&self) -> PathBuf {
        self.store.dir().join("covers")
    }

    /// Media playback progress: `char_offset` holds the position in
    /// milliseconds and `page_index` stays 0 (mirrors the ebook pattern).
    fn record_media_history(&mut self) {
        if let Some(open) = self.media_view.open.as_ref() {
            let media_id = openitgo_parser::stable_comic_id(&open.path);
            let path = open.path.clone();
            let position_ms = open.last.position_ms;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| history_matches(h, &media_id, &path))
            {
                entry.comic_id = media_id;
                entry.path = path;
                entry.page_index = 0;
                entry.char_offset = Some(position_ms as usize);
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id: media_id,
                    path,
                    volume_index: 0,
                    page_index: 0,
                    char_offset: Some(position_ms as usize),
                    last_read_at: now,
                });
            }
        }
    }

    fn request_cover(&mut self, comic_id: &str, path: &Path, source: PageSource) {
        if !self.requested_cover_ids.insert(comic_id.to_string()) {
            return;
        }
        let epoch = self.cover_loader.next_epoch();
        if self.cover_loader.request_thumbnail_high(epoch, 0, source) {
            self.pending_covers
                .insert(epoch, (comic_id.to_string(), path.to_path_buf()));
        } else {
            self.requested_cover_ids.remove(comic_id);
        }
    }

    fn poll_cover_results(&mut self) {
        while let Some(result) = self.cover_loader.try_recv() {
            if !result.thumbnail || result.dropped {
                continue;
            }
            if let Some((comic_id, page_index)) = self.pending_bookmark_thumbs.remove(&result.epoch)
            {
                if let Ok(crate::loader::LoadedImage::Color(img)) = result.image {
                    save_bookmark_thumb(&self.covers_dir(), &comic_id, page_index, &img);
                }
                continue;
            }
            let (comic_id, _path) = match self.pending_covers.remove(&result.epoch) {
                Some(v) => v,
                None => continue,
            };
            let image = match result.image {
                Ok(crate::loader::LoadedImage::Color(img)) => img,
                _ => continue,
            };
            if let Some(cover_path) = save_cover_image(&self.covers_dir(), &comic_id, &image) {
                if let Some(entry) = self
                    .library_view
                    .library
                    .entries
                    .iter_mut()
                    .find(|e| e.comic_id == comic_id)
                {
                    entry.cover_path = Some(cover_path);
                    let _ = self.store.save_library(&self.library_view.library);
                }
            }
        }
    }

    fn poll_media_covers(&mut self) {
        let mut updated = false;
        while let Ok((id, path)) = self.media_cover_rx.try_recv() {
            if let Some(entry) = self
                .library_view
                .library
                .entries
                .iter_mut()
                .find(|e| e.comic_id == id)
            {
                entry.cover_path = Some(path);
                updated = true;
            }
        }
        // Same persistence timing as the comic cover path: save after a
        // drained batch so cover_path survives restarts.
        if updated {
            let _ = self.store.save_library(&self.library_view.library);
        }
    }

    fn request_media_cover(&mut self, idx: usize) {
        let Some(entry) = self.library_view.library.entries.get(idx) else {
            return;
        };
        let input = entry.path.clone();
        let id = entry.comic_id.clone();
        if !input.exists() {
            return;
        }
        // The id stays in the set even when generation fails, so the library
        // view's per-frame cover requests do not retry in a loop.
        if !self.requested_cover_ids.insert(id.clone()) {
            return;
        }
        let covers_dir = self.covers_dir();
        std::fs::create_dir_all(&covers_dir).ok();
        let out = openitgo_media::cover::cover_output_path(&covers_dir, &id);
        if out.exists() {
            // 封面已在磁盘上（上次生成过）：直接回填路径，无需重新生成。
            if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
                entry.cover_path = Some(out);
                let _ = self.store.save_library(&self.library_view.library);
            }
            return;
        }
        let tx = self.media_cover_tx.clone();
        std::thread::spawn(move || {
            // mpv grabs a full-resolution PNG; shrink it to the comic-cover
            // thumbnail size and store JPEG so disk usage and GPU textures
            // stay small (4K videos would otherwise upload 4K textures).
            let tmp_png = covers_dir.join(format!(".{id}.cover-tmp.png"));
            let ok = openitgo_media::cover::generate_cover(
                &input,
                &tmp_png,
                std::time::Duration::from_secs(15),
            )
            .is_ok()
                && shrink_media_cover(&tmp_png, &out).is_ok();
            std::fs::remove_file(&tmp_png).ok();
            if ok {
                let _ = tx.send((id, out));
            } else {
                // 生成或缩放失败时清除可能残缺的 .jpg，
                // 避免回填分支把坏文件当成有效封面。
                std::fs::remove_file(&out).ok();
            }
        });
    }

    fn add_folder_to_library(&mut self, path: std::path::PathBuf) {
        if path.is_file() {
            self.add_file_to_library(path);
            return;
        }

        if let Ok(comic) = openitgo_parser::parse(&path) {
            self.add_comic_to_library(comic, &path);
            return;
        }

        // If the selected path is not a comic itself, recursively scan it for
        // supported archives/folders and ebooks and import everything found.
        let mut found = 0;
        for entry in walk_supported_files(&path) {
            self.add_file_to_library(entry);
            found += 1;
        }
        if found == 0 {
            self.error_message = Some(format!("无法添加文件或文件夹: {}", path.display()));
        }
    }

    fn add_comic_to_library(
        &mut self,
        comic: openitgo_core::models::Comic,
        path: &std::path::Path,
    ) {
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let comic_id = comic.id.clone();
        if !self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            if let Some(page) = comic.volumes.first().and_then(|v| v.pages.first()) {
                self.request_cover(&comic_id, path, page.source.clone());
            }
            self.library_view
                .library
                .entries
                .push(openitgo_storage::models::LibraryEntry {
                    comic_id,
                    title: comic.title.clone(),
                    path: path.to_path_buf(),
                    cover_path: None,
                    added_at,
                    media_type: media_type_for_path(path),
                    tags: Vec::new(),
                });
        }
    }

    fn add_ebook_to_library(&mut self, path: std::path::PathBuf) {
        let Ok(ebook) = openitgo_parser::parse_ebook(&path) else {
            return;
        };
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let media_type = media_type_for_path(&path);
        if !self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            self.library_view
                .library
                .entries
                .push(openitgo_storage::models::LibraryEntry {
                    comic_id: ebook.id,
                    title: ebook.title,
                    path,
                    cover_path: None,
                    added_at,
                    media_type,
                    tags: Vec::new(),
                });
        }
    }

    fn add_media_to_library(&mut self, path: std::path::PathBuf) {
        if self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            return;
        }
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("未知媒体")
            .to_string();
        let media_type = media_type_for_path(&path);
        self.library_view
            .library
            .entries
            .push(openitgo_storage::models::LibraryEntry {
                comic_id: openitgo_parser::stable_comic_id(&path),
                title,
                path,
                cover_path: None,
                added_at,
                media_type,
                tags: Vec::new(),
            });
    }

    fn add_file_to_library(&mut self, path: std::path::PathBuf) {
        if is_ebook_file(&path) {
            self.add_ebook_to_library(path);
        } else if is_media_file(&path) {
            self.add_media_to_library(path);
        } else {
            let password = self.passwords.get(&path).cloned();
            match openitgo_parser::parse_with_password(&path, password.as_deref()) {
                Ok(comic) => self.add_comic_to_library(comic, &path),
                Err(e)
                    if matches!(
                        e,
                        openitgo_parser::traits::ParseError::PasswordRequired
                            | openitgo_parser::traits::ParseError::PasswordIncorrect
                    ) =>
                {
                    let kind =
                        if matches!(e, openitgo_parser::traits::ParseError::PasswordIncorrect) {
                            PasswordPromptKind::Incorrect
                        } else {
                            PasswordPromptKind::Required
                        };
                    if !self.pending_password_imports.contains(&path) {
                        self.pending_password_imports.push(path.clone());
                    }
                    if self.password_dialog.is_none() {
                        self.password_dialog = Some(PasswordDialog::new(path, kind));
                    }
                }
                Err(_) => {}
            }
        }
    }

    fn add_bookmark(&mut self, page_index: usize) {
        if let Some(reader) = self.reader_view.open.as_ref() {
            let comic_id = reader.comic.id.clone();
            let exists = self.bookmarks.entries.iter().any(|b| {
                b.comic_id == comic_id && b.volume_index == 0 && b.page_index == page_index
            });
            if !exists {
                self.bookmarks
                    .entries
                    .push(openitgo_storage::models::Bookmark {
                        comic_id: comic_id.clone(),
                        volume_index: 0,
                        page_index,
                        char_offset: None,
                        note: None,
                    });
                // 生成书签页缩略图（走封面同款缩略图通道，结果在
                // poll_cover_results 里落盘）。电子书书签走 add_ebook_bookmark，
                // 不经过这里，天然跳过。
                if let Some(page) = reader
                    .comic
                    .volumes
                    .first()
                    .and_then(|v| v.pages.get(page_index))
                {
                    let epoch = self.cover_loader.next_epoch();
                    if self.cover_loader.request_thumbnail_high(
                        epoch,
                        page_index,
                        page.source.clone(),
                    ) {
                        self.pending_bookmark_thumbs
                            .insert(epoch, (comic_id, page_index));
                    }
                }
            }
        }
    }

    fn add_ebook_bookmark(&mut self) {
        self.ebook_view.sync_position();
        if let Some(open) = self.ebook_view.open.as_ref() {
            let comic_id = open.ebook.id.clone();
            let page_index = open.current_chapter;
            let char_offset = open.renderer.current_position().1;
            let exists = self.bookmarks.entries.iter().any(|b| {
                b.comic_id == comic_id && b.volume_index == 0 && b.page_index == page_index
            });
            if !exists {
                self.bookmarks
                    .entries
                    .push(openitgo_storage::models::Bookmark {
                        comic_id,
                        volume_index: 0,
                        page_index,
                        char_offset: Some(char_offset),
                        note: None,
                    });
            }
        }
    }

    fn request_cover_for_library_entry(&mut self, idx: usize) {
        let Some(entry) = self.library_view.library.entries.get(idx) else {
            return;
        };
        if matches!(entry.media_type, MediaType::Video | MediaType::Audio) {
            self.request_media_cover(idx);
            return;
        }
        if !entry.path.exists() {
            return;
        }
        if !self.requested_cover_ids.insert(entry.comic_id.clone()) {
            return;
        }
        let password = self.passwords.get(&entry.path).cloned();
        let comic = match openitgo_parser::parse_with_password(&entry.path, password.as_deref()) {
            Ok(c) => c,
            Err(_) => {
                self.requested_cover_ids.remove(&entry.comic_id);
                return;
            }
        };
        let Some(page) = comic.volumes.first().and_then(|v| v.pages.first()) else {
            self.requested_cover_ids.remove(&entry.comic_id);
            return;
        };
        let epoch = self.cover_loader.next_epoch();
        if !self
            .cover_loader
            .request_thumbnail_high(epoch, 0, page.source.clone())
        {
            self.requested_cover_ids.remove(&entry.comic_id);
            return;
        }
        self.pending_covers
            .insert(epoch, (entry.comic_id.clone(), entry.path.clone()));
    }

    fn remove_missing_library_entries(&mut self) {
        let before = self.library_view.library.entries.len();
        self.library_view
            .library
            .entries
            .retain(|e| e.path.exists());
        let removed = before.saturating_sub(self.library_view.library.entries.len());
        if removed > 0 {
            timing::log(&format!("removed {} missing library entries", removed));
        }
    }

    fn open_comic(&mut self, path: std::path::PathBuf) {
        timing::log(&format!("open_comic {:?}", path));
        let password = self.passwords.get(&path).cloned();
        self.opener = Some(AsyncOpener::open(path.clone(), move |p| {
            openitgo_parser::parse_with_password(p, password.as_deref()).map_err(|e| match e {
                openitgo_parser::traits::ParseError::PasswordRequired => {
                    PASSWORD_REQUIRED_MARKER.to_string()
                }
                openitgo_parser::traits::ParseError::PasswordIncorrect => {
                    PASSWORD_INCORRECT_MARKER.to_string()
                }
                other => other.to_string(),
            })
        }));
        self.opening_path = Some(path.clone());
        self.current_view = View::Loading(path);
        self.error_message = None;
    }

    fn open_ebook(&mut self, path: std::path::PathBuf) {
        timing::log(&format!("open_ebook {:?}", path));
        self.ebook_opener = Some(AsyncOpener::open(path.clone(), |p| {
            openitgo_parser::parse_ebook(p).map_err(|e| e.to_string())
        }));
        self.current_view = View::Loading(path);
        self.error_message = None;
    }

    fn open_path(&mut self, path: std::path::PathBuf) {
        if is_ebook_file(&path) {
            self.open_ebook(path);
        } else if is_media_file(&path) {
            self.open_media(path);
        } else {
            self.open_comic(path);
        }
    }

    fn open_media(&mut self, path: std::path::PathBuf) {
        timing::log(&format!("open_media {:?}", path));
        self.pending_media_open = Some(path);
    }

    fn poll_media_open(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(path) = self.pending_media_open.take() else {
            return;
        };
        let screen = ctx.content_rect();
        let bounds = wry::Rect {
            position: wry::dpi::LogicalPosition::new(screen.min.x, screen.min.y).into(),
            size: wry::dpi::LogicalSize::new(screen.width(), screen.height()).into(),
        };
        let media_id = openitgo_parser::stable_comic_id(&path);
        let resume_ms = self
            .history
            .entries
            .iter()
            .find(|h| history_matches(h, &media_id, &path))
            .and_then(|h| h.char_offset.map(|ms| ms as u64));
        match self.media_view.open(ctx, frame, bounds, path, resume_ms) {
            Ok(()) => {
                self.media_view.refresh_audio_devices();
                self.media_view.apply_startup_settings(
                    self.settings.media_volume,
                    self.settings.media_speed,
                    &self.settings.media_audio_device,
                );
                self.current_view = View::Media;
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("无法打开媒体文件: {}", e));
                self.current_view = View::Library;
            }
        }
    }

    fn poll_ebook_opener(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(mut opener) = self.ebook_opener.take() else {
            return;
        };
        match opener.poll() {
            OpenStatus::Loading => self.ebook_opener = Some(opener),
            OpenStatus::Ready(result) => match result {
                Ok(ebook) => {
                    let screen = ctx.content_rect();
                    let bounds = wry::Rect {
                        position: wry::dpi::LogicalPosition::new(screen.min.x, screen.min.y).into(),
                        size: wry::dpi::LogicalSize::new(screen.width(), screen.height()).into(),
                    };
                    match self.ebook_view.open(
                        ctx,
                        frame,
                        bounds,
                        ebook.clone(),
                        &self.settings.ebook,
                    ) {
                        Ok(()) => {
                            if let Some(h) = self
                                .history
                                .entries
                                .iter()
                                .find(|h| history_matches(h, &ebook.id, &ebook.path))
                            {
                                let chapter = h.page_index;
                                let offset = h.char_offset.unwrap_or(0);
                                self.ebook_view.goto_chapter(chapter);
                                if let Some(o) = self.ebook_view.open.as_mut() {
                                    o.renderer.goto_chapter(chapter, offset);
                                }
                            }
                            self.current_view = View::Ebook;
                            self.error_message = None;
                        }
                        Err(e) => {
                            self.error_message = Some(format!("无法创建阅读器: {}", e));
                            self.current_view = View::Library;
                        }
                    }
                }
                Err(e) => {
                    self.error_message = Some(format!("无法打开电子书: {}", e));
                    self.current_view = View::Library;
                }
            },
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<_> = ctx
            .input(|i| i.raw.dropped_files.clone())
            .into_iter()
            .filter_map(|f| f.path)
            .collect();
        self.handle_open_paths(paths);
    }

    fn handle_open_paths(&mut self, paths: Vec<PathBuf>) {
        for path in &paths {
            self.add_folder_to_library(path.clone());
        }
        if let Some(path) = paths.first() {
            let password = self.passwords.get(path).cloned();
            let probe = openitgo_parser::parse_with_password(path, password.as_deref());
            let openable = probe.is_ok()
                || matches!(
                    probe,
                    Err(openitgo_parser::traits::ParseError::PasswordRequired)
                );
            if is_ebook_file(path) || is_media_file(path) || openable {
                self.open_path(path.clone());
            }
        }
    }
}

fn load_settings_with_error(store: &JsonStore) -> (Settings, Option<String>) {
    match store.load_settings() {
        Ok(s) => (s, None),
        Err(err) => {
            let (s, e) = *err;
            (s, Some(e.to_string()))
        }
    }
}

fn load_comic_settings_with_error(
    store: &JsonStore,
) -> (HashMap<String, ComicReadingSettings>, Option<String>) {
    match store.load_comic_settings() {
        Ok(m) => (m, None),
        Err(e) => (HashMap::new(), Some(e.to_string())),
    }
}

/// AsyncOpener 只携带 String 错误：用不可见前缀标记密码类错误，
/// poll_opener 据此弹密码对话框而不是普通错误。
const PASSWORD_REQUIRED_MARKER: &str = "\u{1}password-required";
const PASSWORD_INCORRECT_MARKER: &str = "\u{1}password-incorrect";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordPromptKind {
    Required,
    Incorrect,
}

fn password_prompt_kind(err: &str) -> Option<PasswordPromptKind> {
    if err == PASSWORD_REQUIRED_MARKER {
        Some(PasswordPromptKind::Required)
    } else if err == PASSWORD_INCORRECT_MARKER {
        Some(PasswordPromptKind::Incorrect)
    } else {
        None
    }
}

/// 加密压缩包密码输入对话框状态（会话内有效，不落盘）。
pub struct PasswordDialog {
    path: PathBuf,
    input: String,
    incorrect: bool,
}

impl PasswordDialog {
    fn new(path: PathBuf, kind: PasswordPromptKind) -> Self {
        Self {
            path,
            input: String::new(),
            incorrect: matches!(kind, PasswordPromptKind::Incorrect),
        }
    }
}

/// 当前打开漫画阅读设置（模式/双页/缩放/旋转）的快照，用于与上次写盘值对比。
fn comic_reading_settings_snapshot(
    comic_id: &str,
    state: &ReadingState,
) -> (String, ComicReadingSettings) {
    (
        comic_id.to_string(),
        ComicReadingSettings {
            mode: state.mode,
            double_page: state.double_page,
            fit: state.fit_mode,
            rotation: state.rotation % 360,
        },
    )
}

fn history_matches(entry: &HistoryEntry, comic_id: &str, path: &std::path::Path) -> bool {
    if entry.comic_id == comic_id {
        return true;
    }
    !entry.path.as_os_str().is_empty() && entry.path == path
}

/// Recursively walk `root` and return paths that look like supported comic
/// files, ebook files, media files, or folders containing images.
fn walk_supported_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if is_supported_comic_file(&path) || is_ebook_file(&path) || is_media_file(&path)
            {
                result.push(path);
            }
        }
    }
    result.sort();
    result
}

fn is_supported_comic_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "zip" | "cbz" | "rar" | "cbr" | "pdf"
            )
        })
        .unwrap_or(false)
}

/// comic_id 的文件系统安全形式（非字母数字/-/下划线一律替换为 _）。
fn sanitize_id_component(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn cover_filename(comic_id: &str) -> String {
    let safe = sanitize_id_component(comic_id);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    comic_id.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{}_{:016x}.jpg", safe, hash)
}

fn cover_path_for_comic_id(covers_dir: &Path, comic_id: &str) -> PathBuf {
    covers_dir.join(cover_filename(comic_id))
}

fn save_cover_image(
    covers_dir: &Path,
    comic_id: &str,
    image: &egui::ColorImage,
) -> Option<PathBuf> {
    std::fs::create_dir_all(covers_dir).ok()?;
    let path = cover_path_for_comic_id(covers_dir, comic_id);
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
        .collect();
    let img = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, rgba)?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    rgb.save_with_format(&path, image::ImageFormat::Jpeg).ok()?;
    Some(path)
}

/// 书签缩略图目录：covers/bookmarks/。
fn bookmark_thumb_dir(covers_dir: &Path) -> PathBuf {
    covers_dir.join("bookmarks")
}

/// 书签缩略图路径：covers/bookmarks/<comic_id>-p<page>.jpg。
fn bookmark_thumb_path(covers_dir: &Path, comic_id: &str, page_index: usize) -> PathBuf {
    bookmark_thumb_dir(covers_dir).join(format!(
        "{}-p{}.jpg",
        sanitize_id_component(comic_id),
        page_index
    ))
}

/// 删除书签缩略图：`page_index` 为 Some 时只删该页，None 删整本书的全部
/// 书签缩略图（删除书籍时用）。返回删除的文件数；目录缺失视为 0。
fn remove_bookmark_thumbs(covers_dir: &Path, comic_id: &str, page_index: Option<usize>) -> usize {
    match page_index {
        Some(page) => {
            let path = bookmark_thumb_path(covers_dir, comic_id, page);
            std::fs::remove_file(path).ok().map(|_| 1).unwrap_or(0)
        }
        None => {
            let prefix = format!("{}-p", sanitize_id_component(comic_id));
            let Ok(entries) = std::fs::read_dir(bookmark_thumb_dir(covers_dir)) else {
                return 0;
            };
            let mut removed = 0;
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name.starts_with(&prefix)
                    && name.ends_with(".jpg")
                    && std::fs::remove_file(entry.path()).is_ok()
                {
                    removed += 1;
                }
            }
            removed
        }
    }
}

/// 把解码后的页缩略图落盘为 JPEG（与 save_cover_image 同一格式约定）。
fn save_bookmark_thumb(
    covers_dir: &Path,
    comic_id: &str,
    page_index: usize,
    image: &egui::ColorImage,
) -> Option<PathBuf> {
    let dir = bookmark_thumb_dir(covers_dir);
    std::fs::create_dir_all(&dir).ok()?;
    let path = bookmark_thumb_path(covers_dir, comic_id, page_index);
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
        .collect();
    let img = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, rgba)?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    rgb.save_with_format(&path, image::ImageFormat::Jpeg).ok()?;
    Some(path)
}

/// Scales a full-size media cover grab down to the comic-cover thumbnail
/// size and writes it as JPEG, so media covers cost the same disk space and
/// GPU texture memory as comic covers.
fn shrink_media_cover(input: &Path, output: &Path) -> Result<PathBuf, String> {
    let img = image::open(input).map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(
        crate::loader::THUMBNAIL_MAX_DIMENSION,
        crate::loader::THUMBNAIL_MAX_DIMENSION,
    );
    thumb
        .to_rgb8()
        .save_with_format(output, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    Ok(output.to_path_buf())
}

fn migrate_library_ids(
    library: &mut Library,
    history: &mut History,
    bookmarks: &mut Bookmarks,
    covers_dir: &Path,
) -> bool {
    let mut id_map = HashMap::new();
    for entry in &mut library.entries {
        let expected = openitgo_parser::stable_comic_id(&entry.path);
        if entry.comic_id != expected {
            id_map.insert(entry.comic_id.clone(), expected.clone());
            entry.comic_id = expected;
        }
    }
    if id_map.is_empty() {
        return false;
    }
    for h in &mut history.entries {
        if let Some(new) = id_map.get(&h.comic_id) {
            h.comic_id = new.clone();
            if let Some(entry) = library.entries.iter().find(|e| e.comic_id == *new) {
                h.path = entry.path.clone();
            }
        }
    }
    for b in &mut bookmarks.entries {
        if let Some(new) = id_map.get(&b.comic_id) {
            b.comic_id = new.clone();
        }
    }
    let _ = std::fs::create_dir_all(covers_dir);
    for (old_id, new_id) in &id_map {
        let old_path = cover_path_for_comic_id(covers_dir, old_id);
        let new_path = cover_path_for_comic_id(covers_dir, new_id);
        if old_path.exists() && !new_path.exists() && std::fs::rename(&old_path, &new_path).is_ok()
        {
            for entry in &mut library.entries {
                if entry.comic_id == *new_id && entry.cover_path.as_deref() == Some(&*old_path) {
                    entry.cover_path = Some(new_path.clone());
                }
            }
        }
    }
    true
}

/// True while any egui overlay (menu, dropdown popup, floating window) is
/// visible. The video layer sits below the transparent egui surface, so
/// overlays composite above the video on their own — this is only used to
/// keep the media toolbar from auto-hiding in fullscreen while a menu is
/// open. Tooltips are ignored: they are small, follow the pointer, and must
/// not affect the toolbar.
fn menu_overlay_open(ctx: &egui::Context) -> bool {
    ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .iter()
            .any(|layer| matches!(layer.order, egui::Order::Middle | egui::Order::Foreground))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openitgo_core::models::{Comic, Page, PageSource, Volume};
    use openitgo_core::state::ReadingState;
    use std::path::Path;

    fn dummy_comic() -> Comic {
        Comic {
            id: "test-comic".to_string(),
            title: "Test Comic".to_string(),
            path: PathBuf::from("/tmp/test-comic"),
            volumes: vec![Volume {
                title: "Vol 1".to_string(),
                pages: (0..10)
                    .map(|i| Page {
                        index: i,
                        source: PageSource::File(PathBuf::from(format!("page{}.png", i))),
                    })
                    .collect(),
            }],
        }
    }

    fn app_with_temp_store() -> (ReaderApp, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let app = ReaderApp::with_store_dir(tmp.path());
        (app, tmp)
    }

    fn write_dummy_image(dir: &Path, name: &str) {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        img.save(dir.join(name)).unwrap();
    }

    impl ReaderApp {
        fn with_store_dir(dir: &Path) -> Self {
            let store = JsonStore::new(dir);
            let (settings, _settings_error) = load_settings_with_error(&store);
            let (comic_settings, _) = load_comic_settings_with_error(&store);
            let library = store.load_library().unwrap_or_default();
            let history = store.load_history().unwrap_or_default();
            let bookmarks = store.load_bookmarks().unwrap_or_default();
            let mut library_view = LibraryView::default();
            library_view.library = library;
            library_view.covers_dir = Some(dir.join("covers"));
            let page_loader = PageLoader::new_with_compress(
                settings.compress_images,
                settings.decode_threads as usize,
            );
            let cover_loader = PageLoader::new_with_compress(false, 1);
            let (media_cover_tx, media_cover_rx) = crossbeam_channel::unbounded();
            Self {
                current_view: View::Library,
                last_view: View::Library,
                settings,
                library_view,
                reader_view: ReaderView::default(),
                ebook_view: EbookView::default(),
                media_view: MediaView::default(),
                settings_view: SettingsView::default(),
                store,
                history,
                bookmarks,
                error_message: None,
                page_loader,
                cover_loader,
                opener: None,
                ebook_opener: None,
                pending_media_open: None,
                pending_covers: HashMap::new(),
                requested_cover_ids: HashSet::new(),
                media_cover_tx,
                media_cover_rx,
                current_theme: Theme::System,
                comic_settings,
                last_saved_comic_settings: None,
                show_shortcuts: false,
                passwords: HashMap::new(),
                password_dialog: None,
                pending_password_imports: Vec::new(),
                skipped_encrypted_imports: 0,
                opening_path: None,
                reading_stats: HashMap::new(),
                stats_session: None,
                pending_bookmark_thumbs: HashMap::new(),
            }
        }
    }

    #[test]
    fn test_password_prompt_kind_matches_markers() {
        assert!(matches!(
            password_prompt_kind(PASSWORD_REQUIRED_MARKER),
            Some(PasswordPromptKind::Required)
        ));
        assert!(matches!(
            password_prompt_kind(PASSWORD_INCORRECT_MARKER),
            Some(PasswordPromptKind::Incorrect)
        ));
        assert!(password_prompt_kind("无法打开漫画: io error").is_none());
    }

    #[test]
    fn test_sync_passwords_to_loaders_copies_map() {
        let (mut app, _tmp) = app_with_temp_store();
        app.passwords
            .insert(std::path::PathBuf::from("/tmp/enc.cbz"), "pw".to_string());
        app.sync_passwords_to_loaders();
        let page_pw = app.page_loader.passwords();
        assert_eq!(
            page_pw
                .read()
                .unwrap()
                .get(std::path::Path::new("/tmp/enc.cbz")),
            Some(&"pw".to_string())
        );
        let cover_pw = app.cover_loader.passwords();
        assert_eq!(
            cover_pw
                .read()
                .unwrap()
                .get(std::path::Path::new("/tmp/enc.cbz")),
            Some(&"pw".to_string())
        );
    }

    #[test]
    fn test_record_reader_history_creates_entry() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic = dummy_comic();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            comic.clone(),
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        app.reader_view.open.as_mut().unwrap().state.current_page = 5;
        app.record_reader_history();
        let entry = app
            .history
            .entries
            .iter()
            .find(|h| h.comic_id == comic.id)
            .expect("history entry should exist");
        assert_eq!(entry.page_index, 5);
    }

    #[test]
    fn test_poll_opener_restores_page_from_history() {
        let (mut app, _tmp) = app_with_temp_store();
        let tmp_dir = tempfile::tempdir().unwrap();
        write_dummy_image(tmp_dir.path(), "page0.png");
        write_dummy_image(tmp_dir.path(), "page1.png");

        let comic_id = openitgo_parser::stable_comic_id(tmp_dir.path());
        app.history.entries.push(HistoryEntry {
            comic_id: comic_id.clone(),
            path: tmp_dir.path().to_path_buf(),
            volume_index: 0,
            page_index: 1,
            char_offset: None,
            last_read_at: 0,
        });

        app.open_comic(tmp_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app.poll_opener(&ctx);
            if app.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.current_view, View::Reader);
        let reader = app
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.comic.id, comic_id);
        assert_eq!(reader.state.current_page, 1);
    }

    #[test]
    fn test_comic_reading_settings_snapshot_captures_triple() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 10);
        state.set_double_page(true, 10);
        state.fit_mode = FitMode::Page;
        let snapshot = comic_reading_settings_snapshot("comic-1", &state);
        assert_eq!(
            snapshot,
            (
                "comic-1".to_string(),
                ComicReadingSettings {
                    mode: ReadingMode::Ltr,
                    double_page: true,
                    fit: FitMode::Page,
                    rotation: 0,
                }
            )
        );
    }

    #[test]
    fn test_poll_opener_applies_saved_comic_settings() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic_dir = tempfile::tempdir().unwrap();
        write_dummy_image(comic_dir.path(), "page0.png");
        write_dummy_image(comic_dir.path(), "page1.png");
        let comic_id = openitgo_parser::stable_comic_id(comic_dir.path());
        app.comic_settings.insert(
            comic_id,
            ComicReadingSettings {
                mode: ReadingMode::Rtl,
                double_page: true,
                fit: FitMode::Page,
                rotation: 0,
            },
        );

        app.open_comic(comic_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app.poll_opener(&ctx);
            if app.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.current_view, View::Reader);
        let reader = app
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.state.mode, ReadingMode::Rtl);
        assert!(reader.state.double_page);
        assert_eq!(reader.state.fit_mode, FitMode::Page);
        // 打开即记录快照：未做改动时不应立刻触发写盘。
        assert_eq!(
            app.last_saved_comic_settings,
            Some(comic_reading_settings_snapshot(
                &reader.comic.id,
                &reader.state
            ))
        );
    }

    #[test]
    fn test_poll_opener_without_saved_settings_uses_global_defaults() {
        let (mut app, _tmp) = app_with_temp_store();
        app.settings.default_mode = ReadingMode::Webtoon;
        app.settings.double_page = true;
        app.settings.default_fit = FitMode::Original;
        let comic_dir = tempfile::tempdir().unwrap();
        write_dummy_image(comic_dir.path(), "page0.png");
        write_dummy_image(comic_dir.path(), "page1.png");

        app.open_comic(comic_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app.poll_opener(&ctx);
            if app.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.current_view, View::Reader);
        let reader = app
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.state.mode, ReadingMode::Webtoon);
        // Webtoon 下双页被抑制，与现状一致。
        assert!(!reader.state.double_page);
        assert_eq!(reader.state.fit_mode, FitMode::Original);
    }

    #[test]
    fn test_maybe_save_comic_settings_writes_on_change() {
        let (mut app, _tmp) = app_with_temp_store();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            dummy_comic(),
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        // 模拟 poll_opener 打开时记录的快照。
        let initial = {
            let reader = app.reader_view.open.as_ref().unwrap();
            comic_reading_settings_snapshot(&reader.comic.id, &reader.state)
        };
        app.last_saved_comic_settings = Some(initial);

        // 用户改了双页和 fit（来源不限：菜单/工具栏/快捷键/双击）。
        {
            let reader = app.reader_view.open.as_mut().unwrap();
            reader.state.set_double_page(true, 10);
            reader.state.fit_mode = FitMode::Width;
        }
        app.maybe_save_comic_settings();

        let saved = app.store.load_comic_settings().unwrap();
        assert_eq!(
            saved.get("test-comic"),
            Some(&ComicReadingSettings {
                mode: ReadingMode::Ltr,
                double_page: true,
                fit: FitMode::Width,
                rotation: 0,
            })
        );
        // 快照已更新：无新变化时再次调用不会重写文件。
        std::fs::remove_file(app.store.dir().join("comic_settings.json")).unwrap();
        app.maybe_save_comic_settings();
        assert!(!app.store.dir().join("comic_settings.json").exists());
    }

    #[test]
    fn test_maybe_save_comic_settings_unchanged_writes_nothing() {
        let (mut app, _tmp) = app_with_temp_store();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            dummy_comic(),
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        let initial = {
            let reader = app.reader_view.open.as_ref().unwrap();
            comic_reading_settings_snapshot(&reader.comic.id, &reader.state)
        };
        app.last_saved_comic_settings = Some(initial);

        app.maybe_save_comic_settings();

        assert!(!app.store.dir().join("comic_settings.json").exists());
    }

    #[test]
    fn test_maybe_save_comic_settings_resets_snapshot_when_reader_closed() {
        let (mut app, _tmp) = app_with_temp_store();
        app.last_saved_comic_settings = Some((
            "test-comic".to_string(),
            ComicReadingSettings {
                mode: ReadingMode::Ltr,
                double_page: false,
                fit: FitMode::Height,
                rotation: 0,
            },
        ));

        app.maybe_save_comic_settings();

        assert_eq!(app.last_saved_comic_settings, None);
    }

    #[test]
    fn test_poll_opener_applies_saved_rotation() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic_dir = tempfile::tempdir().unwrap();
        write_dummy_image(comic_dir.path(), "page0.png");
        write_dummy_image(comic_dir.path(), "page1.png");
        let comic_id = openitgo_parser::stable_comic_id(comic_dir.path());
        app.comic_settings.insert(
            comic_id,
            ComicReadingSettings {
                mode: ReadingMode::Ltr,
                double_page: false,
                fit: FitMode::Page,
                rotation: 90,
            },
        );

        app.open_comic(comic_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app.poll_opener(&ctx);
            if app.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.current_view, View::Reader);
        let reader = app
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.state.rotation, 90);
    }

    #[test]
    fn test_maybe_save_comic_settings_persists_rotation_change() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic = dummy_comic();
        let comic_id = comic.id.clone();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            comic,
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        app.last_saved_comic_settings = Some(comic_reading_settings_snapshot(
            &comic_id,
            &app.reader_view.open.as_ref().unwrap().state,
        ));

        app.reader_view.open.as_mut().unwrap().rotate_cw();
        app.maybe_save_comic_settings();

        let saved = app.comic_settings.get(&comic_id).expect("should be saved");
        assert_eq!(saved.rotation, 90);
    }

    #[test]
    fn test_comic_settings_persist_across_reopen() {
        let (mut app1, store_tmp) = app_with_temp_store();
        let comic_dir = store_tmp.path().join("test-comic");
        std::fs::create_dir(&comic_dir).unwrap();
        for i in 0..4 {
            write_dummy_image(&comic_dir, &format!("page{}.png", i));
        }
        let comic_id = openitgo_parser::stable_comic_id(&comic_dir);

        app1.open_comic(comic_dir.clone());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app1.poll_opener(&ctx);
            if app1.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app1.current_view, View::Reader);
        // 用户修改三项设置，帧尾 diff 检测写盘。
        {
            let reader = app1.reader_view.open.as_mut().unwrap();
            let total = reader.total_pages();
            reader.state.set_mode(ReadingMode::Rtl, total);
            reader.state.set_double_page(true, total);
            reader.state.fit_mode = FitMode::Page;
        }
        app1.maybe_save_comic_settings();

        // 模拟重开：新 App 实例从同一 store 目录加载并应用记忆设置。
        let mut app2 = ReaderApp::with_store_dir(store_tmp.path());
        assert!(app2.comic_settings.contains_key(&comic_id));
        app2.open_comic(comic_dir);
        for _ in 0..100 {
            app2.poll_opener(&ctx);
            if app2.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app2.current_view, View::Reader);
        let reader = app2
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.state.mode, ReadingMode::Rtl);
        assert!(reader.state.double_page);
        assert_eq!(reader.state.fit_mode, FitMode::Page);
    }

    #[test]
    fn test_history_roundtrip_via_storage() {
        let (mut app1, store_tmp) = app_with_temp_store();
        // Use a stable folder name so the parsed comic id matches the saved history.
        let comic_dir = store_tmp.path().join("test-comic");
        std::fs::create_dir(&comic_dir).unwrap();
        for i in 0..10 {
            write_dummy_image(&comic_dir, &format!("page{}.png", i));
        }

        app1.open_comic(comic_dir.clone());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app1.poll_opener(&ctx);
            if app1.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app1.current_view, View::Reader);
        app1.reader_view.open.as_mut().unwrap().state.current_page = 6;
        app1.record_reader_history();
        app1.store.save_history(&app1.history).unwrap();

        let expected_id = openitgo_parser::stable_comic_id(&comic_dir);
        let mut app2 = ReaderApp::with_store_dir(store_tmp.path());
        app2.open_comic(comic_dir);
        for _ in 0..100 {
            app2.poll_opener(&ctx);
            if app2.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let reader = app2
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.comic.id, expected_id);
        assert_eq!(reader.state.current_page, 6);
    }

    #[test]
    fn test_is_ebook_file_recognizes_extensions() {
        assert!(is_ebook_file(Path::new("book.epub")));
        assert!(is_ebook_file(Path::new("book.mobi")));
        assert!(is_ebook_file(Path::new("book.azw3")));
        assert!(is_ebook_file(Path::new("notes.md")));
        assert!(is_ebook_file(Path::new("notes.markdown")));
        assert!(is_ebook_file(Path::new("book.TXT")));
        assert!(!is_ebook_file(Path::new("book.pdf")));
        assert!(!is_ebook_file(Path::new("book")));
    }

    #[test]
    fn test_is_media_file_recognizes_video_and_audio() {
        assert!(is_media_file(Path::new("a.mp4")));
        assert!(is_media_file(Path::new("a.MKV")));
        assert!(is_media_file(Path::new("a.webm")));
        assert!(is_media_file(Path::new("a.mp3")));
        assert!(is_media_file(Path::new("a.flac")));
        assert!(!is_media_file(Path::new("a.epub")));
        assert!(!is_media_file(Path::new("a.cbz")));
        assert!(!is_media_file(Path::new("a")));
    }

    #[test]
    fn test_media_type_for_path_classifies_media() {
        assert_eq!(media_type_for_path(Path::new("a.mp4")), MediaType::Video);
        assert_eq!(media_type_for_path(Path::new("a.mkv")), MediaType::Video);
        assert_eq!(media_type_for_path(Path::new("a.mp3")), MediaType::Audio);
        assert_eq!(media_type_for_path(Path::new("a.flac")), MediaType::Audio);
        assert_eq!(media_type_for_path(Path::new("a.epub")), MediaType::Ebook);
        assert_eq!(media_type_for_path(Path::new("a.cbz")), MediaType::Comic);
    }

    #[test]
    fn test_add_media_to_library_uses_stable_id_and_media_type() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let video = tmp_dir.path().join("clip.mp4");
        std::fs::write(&video, b"fake").unwrap();
        let (mut app, _tmp) = app_with_temp_store();
        app.add_media_to_library(video.clone());
        let entry = &app.library_view.library.entries[0];
        assert_eq!(entry.media_type, MediaType::Video);
        assert_eq!(entry.title, "clip");
        assert_eq!(entry.comic_id, openitgo_parser::stable_comic_id(&video));
    }

    #[test]
    fn test_open_path_dispatches_to_ebook_opener() {
        let (mut app, _tmp) = app_with_temp_store();
        app.open_path(PathBuf::from("/tmp/fake.epub"));
        assert!(matches!(app.current_view, View::Loading(_)));
        assert!(app.ebook_opener.is_some());
        assert!(app.opener.is_none());
    }

    #[test]
    fn test_open_path_dispatches_to_comic_opener() {
        let (mut app, _tmp) = app_with_temp_store();
        app.open_path(PathBuf::from("/tmp/fake.cbz"));
        assert!(matches!(app.current_view, View::Loading(_)));
        assert!(app.opener.is_some());
        assert!(app.ebook_opener.is_none());
    }

    #[test]
    fn test_ebook_status_text_includes_spread() {
        let (title, progress) = ReaderApp::ebook_status_text(0, 3, 2, 10, Some("第一章"));
        assert_eq!(title, "第一章");
        assert!(
            progress.contains("第 3 / 10 页"),
            "progress should show spread: {}",
            progress
        );
    }

    #[test]
    fn test_ebook_status_text_formats_progress() {
        let (title, progress) = ReaderApp::ebook_status_text(0, 3, 2, 10, Some("第一章"));
        assert_eq!(title, "第一章");
        assert_eq!(progress, "第 1 / 3 章 · 第 3 / 10 页");

        let (title, progress) = ReaderApp::ebook_status_text(2, 3, 9, 10, None);
        assert_eq!(title, "无标题");
        assert_eq!(progress, "第 3 / 3 章 · 第 10 / 10 页");
    }

    #[test]
    fn test_ebook_status_text_handles_empty_book() {
        let (title, progress) = ReaderApp::ebook_status_text(0, 0, 0, 0, None);
        assert_eq!(title, "无标题");
        assert_eq!(progress, "无章节");
    }

    #[test]
    fn test_history_matches_by_comic_id() {
        let h = HistoryEntry {
            comic_id: "abc".to_string(),
            path: PathBuf::from("/old/path"),
            ..Default::default()
        };
        assert!(history_matches(&h, "abc", Path::new("/new/path")));
    }

    #[test]
    fn test_history_matches_by_path_when_comic_id_differs() {
        let h = HistoryEntry {
            comic_id: "abc".to_string(),
            path: PathBuf::from("/book.epub"),
            ..Default::default()
        };
        assert!(history_matches(&h, "def", Path::new("/book.epub")));
        assert!(!history_matches(&h, "def", Path::new("/other.epub")));
    }

    #[test]
    fn test_history_matches_empty_path_falls_back_to_false() {
        let h = HistoryEntry {
            comic_id: "abc".to_string(),
            path: PathBuf::new(),
            ..Default::default()
        };
        assert!(!history_matches(&h, "def", Path::new("/book.epub")));
    }

    #[test]
    fn test_media_history_entry_contract() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let video = tmp_dir.path().join("clip.mp4");
        std::fs::write(&video, b"fake").unwrap();
        let (mut app, _tmp) = app_with_temp_store();
        let media_id = openitgo_parser::stable_comic_id(&video);
        app.history.entries.push(HistoryEntry {
            comic_id: media_id.clone(),
            path: video.clone(),
            volume_index: 0,
            page_index: 0,
            char_offset: Some(42_000),
            last_read_at: 1,
        });
        let entry = app
            .history
            .entries
            .iter()
            .find(|h| history_matches(h, &media_id, &video))
            .unwrap();
        assert_eq!(entry.char_offset, Some(42_000));
        assert_eq!(entry.page_index, 0);
    }

    #[test]
    fn test_poll_media_covers_sets_cover_path() {
        let (mut app, _tmp) = app_with_temp_store();
        let video = PathBuf::from("/tmp/clip.mp4");
        let id = openitgo_parser::stable_comic_id(&video);
        app.library_view
            .library
            .entries
            .push(openitgo_storage::models::LibraryEntry {
                comic_id: id.clone(),
                title: "clip".to_string(),
                path: video,
                cover_path: None,
                added_at: 0,
                media_type: MediaType::Video,
                tags: Vec::new(),
            });
        let cover = PathBuf::from("/tmp/covers/clip.jpg");
        app.media_cover_tx.send((id, cover.clone())).unwrap();
        app.poll_media_covers();
        assert_eq!(
            app.library_view.library.entries[0].cover_path.as_deref(),
            Some(cover.as_path())
        );
        let saved = app.store.load_library().unwrap();
        assert_eq!(
            saved.entries[0].cover_path.as_deref(),
            Some(cover.as_path()),
            "cover_path should be persisted to library.json"
        );
    }

    #[test]
    fn test_request_media_cover_backfills_existing_cover() {
        let (mut app, tmp) = app_with_temp_store();
        let video = tmp.path().join("clip.mp4");
        std::fs::write(&video, b"fake").unwrap();
        let id = openitgo_parser::stable_comic_id(&video);
        app.library_view
            .library
            .entries
            .push(openitgo_storage::models::LibraryEntry {
                comic_id: id.clone(),
                title: "clip".to_string(),
                path: video,
                cover_path: None,
                added_at: 0,
                media_type: MediaType::Video,
                tags: Vec::new(),
            });
        let covers_dir = tmp.path().join("covers");
        std::fs::create_dir_all(&covers_dir).unwrap();
        let cover = openitgo_media::cover::cover_output_path(&covers_dir, &id);
        std::fs::write(&cover, b"jpeg").unwrap();
        app.request_media_cover(0);
        assert_eq!(
            app.library_view.library.entries[0].cover_path.as_deref(),
            Some(cover.as_path())
        );
        assert!(app.requested_cover_ids.contains(&id));
        let saved = app.store.load_library().unwrap();
        assert_eq!(
            saved.entries[0].cover_path.as_deref(),
            Some(cover.as_path()),
            "backfilled cover_path should be persisted to library.json"
        );
    }

    #[test]
    fn test_shrink_media_cover_writes_jpeg_thumbnail() {
        let tmp = tempfile::tempdir().unwrap();
        let png = tmp.path().join("grab.png");
        let img = image::RgbaImage::from_pixel(1024, 512, image::Rgba([10, 20, 30, 255]));
        img.save(&png).unwrap();
        let out = tmp.path().join("cover.jpg");
        shrink_media_cover(&png, &out).unwrap();
        let thumb = image::open(&out).unwrap();
        assert!(
            thumb.width() <= crate::loader::THUMBNAIL_MAX_DIMENSION
                && thumb.height() <= crate::loader::THUMBNAIL_MAX_DIMENSION,
            "thumbnail should fit within THUMBNAIL_MAX_DIMENSION"
        );
        assert_eq!(
            (thumb.width(), thumb.height()),
            (256, 128),
            "aspect ratio should be preserved"
        );
        assert!(
            matches!(thumb, image::DynamicImage::ImageRgb8(_)),
            "cover should be stored as JPEG"
        );
    }

    fn should_show_bar(
        show_setting: bool,
        fullscreen: bool,
        mouse_pos: Option<egui::Pos2>,
        screen_size: egui::Vec2,
        edge: BarEdge,
    ) -> bool {
        ReaderApp::should_show_bar(show_setting, fullscreen, mouse_pos, screen_size, edge)
    }

    #[test]
    fn test_bar_hidden_when_setting_off() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(!should_show_bar(
            false,
            false,
            Some(egui::pos2(0.0, 0.0)),
            screen,
            BarEdge::Top
        ));
        assert!(!should_show_bar(
            false,
            true,
            Some(egui::pos2(0.0, 0.0)),
            screen,
            BarEdge::Top
        ));
    }

    #[test]
    fn test_bar_shown_when_not_fullscreen_and_setting_on() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(true, false, None, screen, BarEdge::Top));
        assert!(should_show_bar(
            true,
            false,
            Some(egui::pos2(500.0, 500.0)),
            screen,
            BarEdge::Bottom
        ));
    }

    #[test]
    fn test_top_bar_shown_in_fullscreen_near_top_edge() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 10.0)),
            screen,
            BarEdge::Top
        ));
        assert!(!should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 100.0)),
            screen,
            BarEdge::Top
        ));
    }

    #[test]
    fn test_bottom_bar_shown_in_fullscreen_near_bottom_edge() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 1070.0)),
            screen,
            BarEdge::Bottom
        ));
        assert!(!should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 500.0)),
            screen,
            BarEdge::Bottom
        ));
    }

    fn run_frame_with_area(ctx: &egui::Context, order: egui::Order) {
        let _ = ctx.run_ui(Default::default(), |ctx| {
            egui::Area::new(egui::Id::new("test_overlay"))
                .order(order)
                .show(ctx, |ui| {
                    ui.label("overlay");
                });
        });
    }

    #[test]
    fn test_menu_overlay_open_detects_popup_like_areas() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |_| {});
        assert!(!menu_overlay_open(&ctx), "no areas -> no overlay");

        run_frame_with_area(&ctx, egui::Order::Foreground);
        assert!(menu_overlay_open(&ctx), "dropdown/menu areas count");

        run_frame_with_area(&ctx, egui::Order::Middle);
        assert!(menu_overlay_open(&ctx), "floating windows count");
    }

    #[test]
    fn test_menu_overlay_open_ignores_tooltips() {
        let ctx = egui::Context::default();
        run_frame_with_area(&ctx, egui::Order::Tooltip);
        assert!(
            !menu_overlay_open(&ctx),
            "hover tooltips must not hide the video"
        );
    }

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("EP2", "EP10"), Less);
        assert_eq!(natural_cmp("EP10", "EP2"), Greater);
        assert_eq!(natural_cmp("EP10", "EP10"), Equal);
        // 同前缀数字段：整段数字按数值比较，而不是逐字符
        assert_eq!(natural_cmp("EP2x", "EP10a"), Less);
        assert_eq!(natural_cmp("file9.mkv", "file10.mkv"), Less);
        // 前导零不影响数值比较
        assert_eq!(natural_cmp("EP02", "EP2"), Equal);
    }

    #[test]
    fn natural_cmp_is_case_insensitive() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("ep2", "EP2"), Equal);
        assert_eq!(natural_cmp("ABC", "abd"), Less);
        assert_eq!(natural_cmp("a", "B"), Less);
    }

    #[test]
    fn natural_cmp_non_digit_parts_compare_lexicographically() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("abc", "abd"), Less);
        // 前缀相同则短串在前
        assert_eq!(natural_cmp("abc", "ab"), Greater);
        assert_eq!(natural_cmp("", ""), Equal);
        assert_eq!(natural_cmp("", "a"), Less);
    }

    #[test]
    fn next_media_in_dir_follows_natural_order_and_skips_non_media() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for name in ["EP1.mkv", "EP2.mkv", "EP10.mkv", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        // 扩展名像媒体的目录不算媒体文件
        std::fs::create_dir(dir.join("EP3.mkv")).unwrap();

        assert_eq!(
            next_media_in_dir(&dir.join("EP1.mkv")),
            Some(dir.join("EP2.mkv"))
        );
        assert_eq!(
            next_media_in_dir(&dir.join("EP2.mkv")),
            Some(dir.join("EP10.mkv"))
        );
        // 已是最后一集
        assert_eq!(next_media_in_dir(&dir.join("EP10.mkv")), None);
        // current 不存在于目录中
        assert_eq!(next_media_in_dir(&dir.join("EP9.mkv")), None);
        // current 本身不是媒体文件
        assert_eq!(next_media_in_dir(&dir.join("notes.txt")), None);
    }

    #[test]
    fn next_media_in_dir_filters_audio_and_video() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for name in ["a.mp3", "b.flac", "c.mkv", "d.cbz", "e.pdf"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(
            next_media_in_dir(&dir.join("a.mp3")),
            Some(dir.join("b.flac"))
        );
        assert_eq!(
            next_media_in_dir(&dir.join("b.flac")),
            Some(dir.join("c.mkv"))
        );
        assert_eq!(next_media_in_dir(&dir.join("c.mkv")), None);
    }

    #[test]
    fn test_flush_reading_stats_credits_elapsed_seconds() {
        let (mut app, _tmp) = app_with_temp_store();
        app.stats_session = Some((
            "comic-1".to_string(),
            std::time::Instant::now() - std::time::Duration::from_secs(40),
        ));
        app.flush_reading_stats();
        let stat = app.reading_stats.get("comic-1").expect("stat should exist");
        assert!(stat.total_seconds >= 39, "got {}", stat.total_seconds);
        assert_ne!(stat.first_read_at, 0);
        assert_eq!(stat.first_read_at, stat.last_read_at);
        // flush 后会话以原 id 重启（仍在读），不会丢书
        assert!(matches!(&app.stats_session, Some((id, _)) if id == "comic-1"));
    }

    #[test]
    fn test_current_reading_id_tracks_open_views() {
        let (mut app, _tmp) = app_with_temp_store();
        assert_eq!(app.current_reading_id(), None);
        let comic = dummy_comic();
        let comic_id = comic.id.clone();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            comic,
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        app.current_view = View::Reader;
        assert_eq!(app.current_reading_id(), Some(comic_id));
    }

    #[test]
    fn test_bookmark_thumb_path_layout() {
        let dir = std::path::Path::new("/tmp/covers");
        assert_eq!(
            bookmark_thumb_path(dir, "abc123", 7),
            std::path::PathBuf::from("/tmp/covers/bookmarks/abc123-p7.jpg")
        );
        // 特殊字符清洗
        assert_eq!(
            bookmark_thumb_path(dir, "a/b\\c:d", 0),
            std::path::PathBuf::from("/tmp/covers/bookmarks/a_b_c_d-p0.jpg")
        );
    }

    #[test]
    fn test_remove_bookmark_thumbs_single_page() {
        let tmp = tempfile::tempdir().unwrap();
        let covers = tmp.path();
        let thumbs = bookmark_thumb_dir(covers);
        std::fs::create_dir_all(&thumbs).unwrap();
        let p1 = bookmark_thumb_path(covers, "id1", 1);
        let p2 = bookmark_thumb_path(covers, "id1", 2);
        let other = bookmark_thumb_path(covers, "id2", 1);
        for p in [&p1, &p2, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        assert_eq!(remove_bookmark_thumbs(covers, "id1", Some(1)), 1);
        assert!(!p1.exists());
        assert!(p2.exists());
        assert!(other.exists());
    }

    #[test]
    fn test_remove_bookmark_thumbs_whole_book() {
        let tmp = tempfile::tempdir().unwrap();
        let covers = tmp.path();
        let thumbs = bookmark_thumb_dir(covers);
        std::fs::create_dir_all(&thumbs).unwrap();
        let p1 = bookmark_thumb_path(covers, "id1", 1);
        let p2 = bookmark_thumb_path(covers, "id1", 2);
        let other = bookmark_thumb_path(covers, "id2", 1);
        for p in [&p1, &p2, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        assert_eq!(remove_bookmark_thumbs(covers, "id1", None), 2);
        assert!(!p1.exists());
        assert!(!p2.exists());
        assert!(other.exists());
        // 目录不存在时不报错
        assert_eq!(remove_bookmark_thumbs(covers, "nope", None), 0);
    }

    #[test]
    fn initial_open_path_prefers_env_over_argv() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join("env.cbz");
        let arg_file = dir.path().join("arg.cbz");
        std::fs::write(&env_file, b"x").unwrap();
        std::fs::write(&arg_file, b"x").unwrap();
        assert_eq!(
            initial_open_path(Some(env_file.clone()), Some(arg_file)),
            Some(env_file)
        );
    }

    #[test]
    fn initial_open_path_falls_back_to_argv_and_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let arg_file = dir.path().join("arg.cbz");
        std::fs::write(&arg_file, b"x").unwrap();
        assert_eq!(
            initial_open_path(None, Some(arg_file.clone())),
            Some(arg_file.clone())
        );
        // env 设置了但不存在 → 回退到 argv
        assert_eq!(
            initial_open_path(
                Some(std::path::PathBuf::from("/nonexistent/env.cbz")),
                Some(arg_file.clone()),
            ),
            Some(arg_file)
        );
        assert_eq!(
            initial_open_path(None, Some(std::path::PathBuf::from("/nonexistent/xx.cbz"))),
            None
        );
        assert_eq!(initial_open_path(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn initial_open_path_accepts_non_utf8_argv() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("comic.cbz");
        std::fs::write(&file, b"x").unwrap();
        // 非 UTF-8 字节序列构成的 argv 不再 panic（回归：env::args() 会 panic）
        let raw = std::ffi::OsString::from_vec(b"\xff\xfe.cbz".to_vec());
        assert_eq!(initial_open_path(None, Some(file.clone())), Some(file));
        assert_eq!(
            initial_open_path(None, Some(std::path::PathBuf::from(raw))),
            None
        );
    }
}
