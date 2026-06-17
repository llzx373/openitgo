use crate::views::{library::LibraryView, reader::ReaderView};
use rust_reader_core::models::ReadingMode;
use rust_reader_core::state::ReadingState;
use rust_reader_storage::models::Settings;

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
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
        }
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| match self.current_view {
            View::Library => {
                let mut open_idx = None;
                self.library_view.ui(ui, &mut |idx| open_idx = Some(idx));
                if let Some(idx) = open_idx {
                    if let Some(entry) = self.library_view.library.entries.get(idx) {
                        if let Ok(comic) = rust_reader_parser::parse(&entry.path) {
                            let state = ReadingState::new(
                                self.settings.default_mode,
                                comic.volumes[0].pages.len(),
                            );
                            self.reader_view.open(comic, state);
                            self.current_view = View::Reader;
                        }
                    }
                }
            }
            View::Reader => {
                self.reader_view.ui(ui, ctx);
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
