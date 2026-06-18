use crate::loader::PageLoader;
use crate::opener::{ComicOpener, OpenStatus};
use crate::shortcuts::is_shortcut_pressed;
use crate::views::{
    library::{LibraryCallbacks, LibraryView},
    reader::{QuickFit, ReaderView},
    settings::SettingsView,
};
use rust_reader_core::models::ReadingMode;
use rust_reader_core::state::ReadingState;
use rust_reader_storage::{
    json_store::JsonStore,
    models::{Bookmarks, History, HistoryEntry, Settings},
};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
    pub settings_view: SettingsView,
    pub store: JsonStore,
    pub history: History,
    pub bookmarks: Bookmarks,
    pub error_message: Option<String>,
    pub page_loader: PageLoader,
    pub opener: Option<ComicOpener>,
}

impl Default for ReaderApp {
    fn default() -> Self {
        let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| PathBuf::from(".")));
        let settings = store.load_settings().unwrap_or_default();
        let library = store.load_library().unwrap_or_default();
        let history = store.load_history().unwrap_or_default();
        let bookmarks = store.load_bookmarks().unwrap_or_default();
        let mut library_view = LibraryView::default();
        library_view.library = library;
        Self {
            current_view: View::Library,
            settings,
            library_view,
            reader_view: ReaderView::default(),
            settings_view: SettingsView::default(),
            store,
            history,
            bookmarks,
            error_message: None,
            page_loader: PageLoader::default(),
            opener: None,
        }
    }
}

impl eframe::App for ReaderApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.record_reader_history();
        let _ = self.store.save_settings(&self.settings);
        let _ = self.store.save_library(&self.library_view.library);
        let _ = self.store.save_history(&self.history);
        let _ = self.store.save_bookmarks(&self.bookmarks);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !matches!(self.current_view, View::Reader) {
            self.record_reader_history();
            self.reader_view.open = None;
        }

        self.handle_global_input(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.ctrl) {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.add_folder_to_library(path);
            }
        }
        self.handle_dropped_files(ctx);
        self.poll_opener();

        self.reader_view
            .update(ctx, &self.page_loader, self.settings.cache_size_mb);
        self.reader_view
            .request_preloads(&self.page_loader, self.settings.cache_size_mb);

        match self.current_view.clone() {
            View::Library => self.render_library(ctx),
            View::Reader => self.render_reader(ctx),
            View::Settings => self.render_settings(ctx),
            View::Loading(path) => self.render_loading(ctx, path),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Library,
    Reader,
    Settings,
    Loading(PathBuf),
}

enum BarEdge {
    Top,
    Bottom,
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn poll_opener(&mut self) {
        let Some(mut opener) = self.opener.take() else {
            return;
        };
        match opener.poll() {
            OpenStatus::Loading => {
                self.opener = Some(opener);
            }
            OpenStatus::Ready(result) => match result {
                Ok(comic) => {
                    let total = comic.volumes.first().map(|v| v.pages.len()).unwrap_or(0);
                    let mut state = ReadingState::new(self.settings.default_mode, total);
                    state.set_double_page(self.settings.double_page, total);
                    if let Some(h) = self.history.entries.iter().find(|h| h.comic_id == comic.id) {
                        state.go_to_page(h.page_index, total);
                    }
                    self.reader_view.open(comic, state, &self.page_loader);
                    self.current_view = View::Reader;
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("无法打开漫画: {}", e));
                    self.current_view = View::Library;
                }
            },
        }
    }

    fn render_library(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error_message {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            let mut open_idx = None;
            let mut open_path: Option<PathBuf> = None;
            let mut add_requested = false;
            let mut delete_bookmark_idx: Option<usize> = None;
            let mut update_title: Option<(usize, String)> = None;
            let mut delete_library_idx: Option<usize> = None;
            self.library_view.ui(
                ui,
                &self.history,
                &self.bookmarks,
                &mut self.settings.library_sort,
                LibraryCallbacks {
                    on_open_library: &mut |idx| open_idx = Some(idx),
                    on_open_path: &mut |path| open_path = Some(path),
                    on_add: &mut || add_requested = true,
                    on_delete_bookmark: &mut |idx| delete_bookmark_idx = Some(idx),
                    on_update_title: &mut |idx, title| update_title = Some((idx, title)),
                    on_delete_library: &mut |idx| delete_library_idx = Some(idx),
                },
            );
            if add_requested {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.add_folder_to_library(path);
                }
            }
            if let Some(idx) = open_idx {
                if let Some(entry) = self.library_view.entry_at(idx).cloned() {
                    self.open_comic(entry.path);
                }
            }
            if let Some(path) = open_path {
                self.open_comic(path);
            }
            if let Some((idx, title)) = update_title {
                if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
                    entry.title = title;
                }
            }
            if let Some(idx) = delete_library_idx {
                self.library_view.library.entries.remove(idx);
            }
            if let Some(idx) = delete_bookmark_idx {
                self.bookmarks.entries.remove(idx);
            }
        });
    }

    fn render_reader(&mut self, ctx: &egui::Context) {
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
                i.screen_rect().size(),
            )
        });
        if Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        ) {
            self.render_reader_toolbar(ctx, total_pages, current_page, mode, zoom);
        }
        let (progress_bar_rect, hovered_page) = if Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        ) {
            self.render_reader_statusbar(ctx)
        } else {
            (None, None)
        };

        let bg = self.settings.background_color;
        let frame = egui::Frame::central_panel(&ctx.style())
            .fill(egui::Color32::from_rgb(bg[0], bg[1], bg[2]));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let response = self.reader_view.ui(ui, &self.page_loader);

            // Floating thumbnail tooltip above the cursor when hovering the progress bar.
            if progress_bar_rect.is_some() {
                self.reader_view.render_progress_thumbnail(ui, hovered_page);
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
            let mut scroll = ui.input(|i| i.raw_scroll_delta.y);
            if self.settings.invert_scroll {
                scroll = -scroll;
            }
            if scroll > 2.0 {
                self.reader_page_down();
            } else if scroll < -2.0 {
                self.reader_page_up();
            }

            // Double-click toggles fullscreen.
            if ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) {
                self.toggle_fullscreen(ctx);
            }
        });
    }

    fn render_reader_toolbar(
        &mut self,
        ctx: &egui::Context,
        total_pages: usize,
        current_page: usize,
        mode: ReadingMode,
        zoom: f32,
    ) {
        egui::TopBottomPanel::top("reader_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("← 书架").clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();

                let modes = [
                    (ReadingMode::Ltr, "国漫"),
                    (ReadingMode::Rtl, "日漫"),
                    (ReadingMode::Webtoon, "韩漫"),
                ];
                for (m, label) in modes {
                    if ui.selectable_label(mode == m, label).clicked() {
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
                    if ui
                        .selectable_label(double_page, "双页")
                        .on_hover_text("切换到双页模式")
                        .clicked()
                    {
                        let new_double = !double_page;
                        self.settings.double_page = new_double;
                        if let Some(reader) = self.reader_view.open.as_mut() {
                            reader.state.set_double_page(new_double, total_pages);
                            reader.pending_fit = Some(QuickFit::Page);
                        }
                    }
                    ui.separator();
                }

                if ui.button("-").clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_out();
                    }
                }
                ui.label(format!("{:.0}%", zoom * 100.0));
                if ui.button("+").clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_in();
                    }
                }
                if ui.button("适应宽度").clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(QuickFit::Width);
                    }
                }
                if ui.button("适应高度").clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(QuickFit::Height);
                    }
                }
                if ui.button("自动适应").clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(QuickFit::Page);
                    }
                }
                ui.separator();

                if ui.button("上一页").clicked() {
                    self.reader_prev_page();
                }
                let mut displayed_page = current_page + 1;
                ui.add(
                    egui::DragValue::new(&mut displayed_page)
                        .speed(1.0)
                        .range(1..=total_pages.max(1)),
                );
                ui.label(format!("/ {}", total_pages));
                if ui.button("下一页").clicked() {
                    self.reader_next_page();
                }
                ui.separator();

                if ui.button("添加书签").clicked() {
                    self.add_bookmark(current_page);
                }
                if ui.button("全屏").clicked() {
                    self.toggle_fullscreen(ctx);
                }
                if ui.button("设置").clicked() {
                    self.current_view = View::Settings;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").on_hover_text("隐藏工具栏").clicked() {
                        self.settings.show_toolbar = false;
                    }
                });
            });
        });
    }

    fn render_reader_statusbar(
        &mut self,
        ctx: &egui::Context,
    ) -> (Option<egui::Rect>, Option<usize>) {
        let mut progress_rect = None;
        let mut hovered_page = None;
        egui::TopBottomPanel::bottom("reader_statusbar").show(ctx, |ui| {
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

    fn render_settings(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("← 返回").clicked() {
                    self.current_view = if self.reader_view.open.is_some() {
                        View::Reader
                    } else {
                        View::Library
                    };
                }
            });
            ui.separator();
            self.settings_view.ui(ui, &mut self.settings);
        });
    }

    fn render_loading(&mut self, ctx: &egui::Context, path: PathBuf) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.label("正在打开漫画...");
                        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                            ui.label(egui::RichText::new(name).size(14.0).strong());
                        }
                        ui.add_space(16.0);
                        if ui.button("取消").clicked() {
                            self.opener = None;
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

    fn context_menu_items(&mut self, ui: &mut egui::Ui) {
        if ui.button("下一页").clicked() {
            self.reader_next_page();
            ui.close_menu();
        }
        if ui.button("上一页").clicked() {
            self.reader_prev_page();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("首页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
            ui.close_menu();
        }
        if ui.button("末页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            if let Some(reader) = self.reader_view.open.as_ref() {
                self.add_bookmark(reader.state.current_page);
            }
            ui.close_menu();
        }
        if ui.button("全屏").clicked() {
            self.toggle_fullscreen(ui.ctx());
            ui.close_menu();
        }
        ui.separator();
        let toolbar_label = if self.settings.show_toolbar {
            "隐藏工具栏"
        } else {
            "显示工具栏"
        };
        if ui.button(toolbar_label).clicked() {
            self.settings.show_toolbar = !self.settings.show_toolbar;
            ui.close_menu();
        }
        let statusbar_label = if self.settings.show_statusbar {
            "隐藏状态栏"
        } else {
            "显示状态栏"
        };
        if ui.button(statusbar_label).clicked() {
            self.settings.show_statusbar = !self.settings.show_statusbar;
            ui.close_menu();
        }
        ui.separator();
        if ui.button("返回书架").clicked() {
            self.current_view = View::Library;
            ui.close_menu();
        }
    }

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        let is_reader = matches!(self.current_view, View::Reader);
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.fullscreen) {
            self.toggle_fullscreen(ctx);
        }

        if !is_reader {
            return;
        }

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
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_down) {
            self.reader_page_down();
        }
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_up) {
            self.reader_page_up();
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
                reader.request_fit(QuickFit::Page);
            }
        }
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_width) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.request_fit(QuickFit::Width);
            }
        }
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_height) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.request_fit(QuickFit::Height);
            }
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

    fn record_reader_history(&mut self) {
        if let Some(reader) = self.reader_view.open.as_ref() {
            let comic_id = reader.comic.id.clone();
            let page_index = reader.state.current_page;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| h.comic_id == comic_id)
            {
                entry.page_index = page_index;
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id,
                    volume_index: 0,
                    page_index,
                    last_read_at: now,
                });
            }
        }
    }

    fn add_folder_to_library(&mut self, path: std::path::PathBuf) {
        match rust_reader_parser::parse(&path) {
            Ok(comic) => {
                let added_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let entry = rust_reader_storage::models::LibraryEntry {
                    comic_id: comic.id.clone(),
                    title: comic.title.clone(),
                    path: path.clone(),
                    cover_path: None,
                    added_at,
                };
                if !self
                    .library_view
                    .library
                    .entries
                    .iter()
                    .any(|e| e.path == path)
                {
                    self.library_view.library.entries.push(entry);
                }
            }
            Err(e) => {
                self.error_message = Some(format!("无法添加漫画: {}", e));
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
                    .push(rust_reader_storage::models::Bookmark {
                        comic_id,
                        volume_index: 0,
                        page_index,
                        note: None,
                    });
            }
        }
    }

    fn open_comic(&mut self, path: std::path::PathBuf) {
        self.opener = Some(ComicOpener::open(path.clone(), |p| {
            rust_reader_parser::parse(p).map_err(|e| e.to_string())
        }));
        self.current_view = View::Loading(path);
        self.error_message = None;
    }

    fn ensure_in_library(&mut self, path: &std::path::Path) {
        if self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            return;
        }
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();
        let comic_id = title.clone();
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.library_view
            .library
            .entries
            .push(rust_reader_storage::models::LibraryEntry {
                comic_id,
                title,
                path: path.to_path_buf(),
                cover_path: None,
                added_at,
            });
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped_files.first() {
            if let Some(path) = &file.path {
                self.ensure_in_library(path);
                self.open_comic(path.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_core::models::{Comic, Page, PageSource, Volume};
    use rust_reader_core::state::ReadingState;
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
            let settings = store.load_settings().unwrap_or_default();
            let library = store.load_library().unwrap_or_default();
            let history = store.load_history().unwrap_or_default();
            let bookmarks = store.load_bookmarks().unwrap_or_default();
            let mut library_view = LibraryView::default();
            library_view.library = library;
            Self {
                current_view: View::Library,
                settings,
                library_view,
                reader_view: ReaderView::default(),
                settings_view: SettingsView::default(),
                store,
                history,
                bookmarks,
                error_message: None,
                page_loader: PageLoader::default(),
                opener: None,
            }
        }
    }

    #[test]
    fn test_record_reader_history_creates_entry() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic = dummy_comic();
        app.reader_view.open(
            comic.clone(),
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
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

        let comic_id = tmp_dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        app.history.entries.push(HistoryEntry {
            comic_id: comic_id.clone(),
            volume_index: 0,
            page_index: 1,
            last_read_at: 0,
        });

        app.open_comic(tmp_dir.path().to_path_buf());
        for _ in 0..100 {
            app.poll_opener();
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
    fn test_history_roundtrip_via_storage() {
        let (mut app1, store_tmp) = app_with_temp_store();
        // Use a stable folder name so the parsed comic id matches the saved history.
        let comic_dir = store_tmp.path().join("test-comic");
        std::fs::create_dir(&comic_dir).unwrap();
        for i in 0..10 {
            write_dummy_image(&comic_dir, &format!("page{}.png", i));
        }

        app1.open_comic(comic_dir.clone());
        for _ in 0..100 {
            app1.poll_opener();
            if app1.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app1.current_view, View::Reader);
        app1.reader_view.open.as_mut().unwrap().state.current_page = 6;
        app1.record_reader_history();
        app1.store.save_history(&app1.history).unwrap();

        let mut app2 = ReaderApp::with_store_dir(store_tmp.path());
        app2.open_comic(comic_dir);
        for _ in 0..100 {
            app2.poll_opener();
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
        assert_eq!(reader.comic.id, "test-comic");
        assert_eq!(reader.state.current_page, 6);
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
}
