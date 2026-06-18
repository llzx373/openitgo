use crate::loader::PageLoader;
use crate::views::{
    library::LibraryView,
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
}

impl Default for ReaderApp {
    fn default() -> Self {
        let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| PathBuf::from(".")));
        let settings = store.load_settings().unwrap_or_default();
        let library = store.load_library().unwrap_or_default();
        let history = store.load_history().unwrap_or_default();
        let bookmarks = store.load_bookmarks().unwrap_or_default();
        let library_view = LibraryView { library };
        Self {
            current_view: View::Library,
            settings,
            library_view,
            reader_view: ReaderView::default(),
            settings_view: SettingsView,
            store,
            history,
            bookmarks,
            error_message: None,
            page_loader: PageLoader::default(),
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

        self.reader_view
            .update(ctx, &self.page_loader, self.settings.cache_size_mb);
        self.reader_view
            .request_preloads(&self.page_loader, self.settings.cache_size_mb);

        match self.current_view {
            View::Library => self.render_library(ctx),
            View::Reader => self.render_reader(ctx),
            View::Settings => self.render_settings(ctx),
        }
    }
}

pub enum View {
    Library,
    Reader,
    Settings,
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn render_library(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error_message {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            let mut open_idx = None;
            let mut add_requested = false;
            self.library_view
                .ui(ui, &mut |idx| open_idx = Some(idx), &mut || {
                    add_requested = true
                });
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

        self.render_reader_toolbar(ctx, total_pages, current_page, mode, zoom);
        self.render_reader_statusbar(ctx, total_pages, current_page, mode, zoom);

        egui::CentralPanel::default().show(ctx, |ui| {
            let response = self.reader_view.ui(ui, &self.page_loader);

            // Right-click context menu on the page area.
            if let Some(response) = response {
                response.context_menu(|ui| {
                    self.context_menu_items(ui);
                });
            }

            // Scroll wheel navigation.
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
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
            });
        });
    }

    fn render_reader_statusbar(
        &mut self,
        ctx: &egui::Context,
        total_pages: usize,
        current_page: usize,
        mode: ReadingMode,
        zoom: f32,
    ) {
        egui::TopBottomPanel::bottom("reader_statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("页面: {}/{}", current_page + 1, total_pages));
                ui.separator();
                ui.label(format!(
                    "模式: {}",
                    match mode {
                        ReadingMode::Ltr => "国漫（左→右）",
                        ReadingMode::Rtl => "日漫（右→左）",
                        ReadingMode::Webtoon => "韩漫（上→下）",
                    }
                ));
                ui.separator();
                ui.label(format!("缩放: {:.0}%", zoom * 100.0));
                ui.separator();
                let double_page = self
                    .reader_view
                    .open
                    .as_ref()
                    .map(|r| r.state.double_page)
                    .unwrap_or(false);
                ui.label(if double_page { "双页" } else { "单页" });
                ui.separator();
                ui.label("快捷键: ← → 翻页 | +/- 缩放 | F11 全屏 | Esc 返回书架");
            });
            ui.add_space(4.0);
            self.reader_view.render_page_navigator(ui);
        });
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
        if ui.button("返回书架").clicked() {
            self.current_view = View::Library;
            ui.close_menu();
        }
    }

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        let is_reader = matches!(self.current_view, View::Reader);

        if ctx
            .input(|i| i.key_pressed(egui::Key::F11) || (i.key_pressed(egui::Key::F) && is_reader))
        {
            self.toggle_fullscreen(ctx);
        }

        if !is_reader {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
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
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            if rtl {
                self.reader_prev_page();
            } else {
                self.reader_next_page();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            if rtl {
                self.reader_next_page();
            } else {
                self.reader_prev_page();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::Space)) {
            self.reader_page_down();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::PageUp)) {
            self.reader_page_up();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Home)) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::End)) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.zoom_in();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.zoom_out();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.request_fit(QuickFit::Page);
            }
        }
    }

    fn reader_next_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            let total = reader.total_pages();
            reader.state.next_page(total);
        }
    }

    fn reader_prev_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.state.prev_page();
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
                let entry = rust_reader_storage::models::LibraryEntry {
                    comic_id: comic.id.clone(),
                    title: comic.title.clone(),
                    path: path.clone(),
                    cover_path: None,
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
        match rust_reader_parser::parse(&path) {
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
            }
        }
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
        self.library_view
            .library
            .entries
            .push(rust_reader_storage::models::LibraryEntry {
                comic_id,
                title,
                path: path.to_path_buf(),
                cover_path: None,
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
