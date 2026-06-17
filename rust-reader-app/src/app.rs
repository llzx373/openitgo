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
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            settings: Settings {
                default_mode: ReadingMode::Ltr,
                ..Default::default()
            },
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
            ui.heading("rustReader");
            ui.label("App skeleton loaded");
        });
    }
}
