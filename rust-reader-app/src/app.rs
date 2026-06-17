use crate::views::{library::LibraryView, reader::ReaderView};
use rust_reader_core::models::ReadingMode;
use rust_reader_core::state::ReadingState;
use rust_reader_storage::models::Settings;

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
    pub error_message: Option<String>,
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            settings: Settings {
                default_mode: ReadingMode::Ltr,
                ..Default::default()
            },
            library_view: LibraryView::default(),
            reader_view: ReaderView::default(),
            error_message: None,
        }
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if matches!(self.current_view, View::Reader) {
            if let Some(reader) = self.reader_view.open.as_mut() {
                let total = reader
                    .comic
                    .volumes
                    .first()
                    .map(|v| v.pages.len())
                    .unwrap_or(0);
                if total > 0 {
                    if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                        match reader.state.mode {
                            ReadingMode::Ltr => reader.state.next_page(total),
                            ReadingMode::Rtl => reader.state.prev_page(),
                            ReadingMode::Webtoon => reader.state.next_page(total),
                        }
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                        match reader.state.mode {
                            ReadingMode::Ltr => reader.state.prev_page(),
                            ReadingMode::Rtl => reader.state.next_page(total),
                            ReadingMode::Webtoon => reader.state.prev_page(),
                        }
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.current_view {
            View::Library => {
                let mut open_idx = None;
                self.library_view.ui(ui, &mut |idx| open_idx = Some(idx));
                if let Some(idx) = open_idx {
                    if let Some(entry) = self.library_view.entry_at(idx) {
                        match rust_reader_parser::parse(&entry.path) {
                            Ok(comic) => {
                                let state = ReadingState::new(
                                    self.settings.default_mode,
                                    comic.volumes[0].pages.len(),
                                );
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
                if let Some(err) = &self.error_message {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
            }
            View::Reader => {
                self.reader_view.ui(ui);
            }
            View::Settings => {
                ui.label("设置视图待实现");
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
}
