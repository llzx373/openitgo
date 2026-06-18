mod app;
mod cache;
mod fonts;
mod loader;
mod opener;
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
        Box::new(|cc| {
            fonts::load_cjk_font(&cc.egui_ctx);
            Ok(Box::new(ReaderApp::new(cc)))
        }),
    )
}
