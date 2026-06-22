mod app;
mod cache;
mod fonts;
mod loader;
mod opener;
mod platform;
mod shortcuts;
mod timing;
mod views;
mod widgets;

use app::ReaderApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        // Force the wgpu/Metal backend on macOS instead of falling back to glow/OpenGL.
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
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
