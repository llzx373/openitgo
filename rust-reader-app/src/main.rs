mod app;
mod cache;
#[allow(dead_code)]
mod ebook_renderer;
mod fonts;
mod loader;
mod opener;
mod platform;
mod shortcuts;
mod timing;
mod views;
mod widgets;

use app::ReaderApp;

fn load_app_icon() -> Option<std::sync::Arc<egui::IconData>> {
    let bytes = include_bytes!("../../assets/icon/1024x1024.png");
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some(std::sync::Arc::new(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }))
}

fn main() -> eframe::Result<()> {
    #[cfg(target_os = "macos")]
    crate::platform::macos::dock_open::install_dock_open_handler_early();

    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]);
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        // Force the wgpu/Metal backend on macOS instead of falling back to glow/OpenGL.
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        ..Default::default()
    };
    eframe::run_native(
        "rustReader",
        options,
        Box::new(|cc| {
            fonts::setup_fonts(&cc.egui_ctx);
            #[cfg(target_os = "macos")]
            crate::platform::macos::dock_open::install_dock_open_handler();
            Ok(Box::new(ReaderApp::new(cc)))
        }),
    )
}
