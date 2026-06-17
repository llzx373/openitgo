use crate::views::{library::LibraryView, reader::ReaderView, settings::SettingsView};
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

        let toggle = ctx.input(|i| {
            i.key_pressed(egui::Key::F11)
                .then(|| !i.viewport().fullscreen.unwrap_or(false))
        });
        if let Some(fullscreen) = toggle {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
        }
        self.handle_reader_input(ctx);

        egui::CentralPanel::default().show(ctx, |ui| match self.current_view {
            View::Library => {
                if let Some(err) = &self.error_message {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                let mut open_idx = None;
                self.library_view.ui(ui, &mut |idx| open_idx = Some(idx));
                if let Some(idx) = open_idx {
                    if let Some(entry) = self.library_view.entry_at(idx).cloned() {
                        match rust_reader_parser::parse(&entry.path) {
                            Ok(comic) => {
                                let total =
                                    comic.volumes.first().map(|v| v.pages.len()).unwrap_or(0);
                                let mut state =
                                    ReadingState::new(self.settings.default_mode, total);
                                if let Some(h) = self
                                    .history
                                    .entries
                                    .iter()
                                    .find(|h| h.comic_id == entry.comic_id)
                                {
                                    state.go_to_page(h.page_index, total);
                                }
                                self.reader_view.open(comic, state);
                                self.current_view = View::Reader;
                                self.error_message = None;
                            }
                            Err(e) => {
                                self.error_message = Some(format!("无法打开漫画: {}", e));
                            }
                        }
                    }
                }
            }
            View::Reader => {
                let mut bookmark_page = None;
                self.reader_view
                    .ui(ui, &mut |page| bookmark_page = Some(page));
                if let Some(page) = bookmark_page {
                    self.add_bookmark(page);
                }
            }
            View::Settings => {
                self.settings_view.ui(ui, &mut self.settings);
            }
        });
    }
}

#[allow(dead_code)]
pub enum View {
    Library,
    Reader,
    Settings,
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
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

    fn handle_reader_input(&mut self, ctx: &egui::Context) {
        if !matches!(self.current_view, View::Reader) {
            return;
        }
        let Some(reader) = self.reader_view.open.as_mut() else {
            return;
        };
        let total = reader.total_pages();
        if total == 0 {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            match reader.state.mode {
                ReadingMode::Ltr | ReadingMode::Webtoon => reader.state.next_page(total),
                ReadingMode::Rtl => reader.state.prev_page(),
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            match reader.state.mode {
                ReadingMode::Ltr | ReadingMode::Webtoon => reader.state.prev_page(),
                ReadingMode::Rtl => reader.state.next_page(total),
            }
        }
    }
}
