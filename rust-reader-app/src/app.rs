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
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        self.handle_reader_input(ctx);

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
