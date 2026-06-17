use crate::views::library::LibraryView;
use rust_reader_core::models::ReadingMode;
use rust_reader_storage::models::Settings;

#[allow(dead_code)]
pub enum View {
    Library,
    Reader,
    Settings,
}

#[allow(dead_code)]
pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
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
        }
    }
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                View::Library => {
                    self.library_view.ui(ui, &mut |_| {
                        // Reader opening implemented in Task 12
                    });
                }
                _ => {
                    ui.label("View not implemented yet");
                }
            }
        });
    }
}
