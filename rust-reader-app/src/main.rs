mod app;
mod views;
mod widgets;

use app::ReaderApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rustReader",
        options,
        Box::new(|cc| Ok(Box::new(ReaderApp::new(cc)))),
    )
}
