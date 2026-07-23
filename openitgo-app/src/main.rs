mod app;
mod cache;
mod ebook_renderer;
mod loader;
mod opener;
mod platform;
mod shortcuts;
mod timing;
mod views;
mod widgets;
mod window_geometry;

use app::ReaderApp;
use openitgo_storage::json_store::JsonStore;
use window_geometry::{resolve_startup_geometry, DEFAULT_WINDOW_SIZE};

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

    let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| ".".into()));
    let settings = store
        .load_settings()
        .unwrap_or_else(|_| openitgo_storage::models::Settings::default());
    // At process start we do not yet have a reliable monitor list from egui;
    // pass empty monitors and keep the saved position optimistically. ReaderApp
    // re-validates against live monitor size on the first frame.
    let restored = resolve_startup_geometry(
        settings.window_size,
        settings.window_pos,
        settings.window_maximized,
        &[],
    );
    let (w, h) = if restored.size.0 > 0.0 && restored.size.1 > 0.0 {
        restored.size
    } else {
        DEFAULT_WINDOW_SIZE
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_maximized(restored.maximized)
        // Transparent backbuffer: egui-wgpu then picks
        // CompositeAlphaMode::PreMultiplied and the CAMetalLayer becomes
        // non-opaque, so the video layer below the egui surface (Task 4)
        // shows through unpainted regions.
        .with_transparent(true)
        .with_clamp_size_to_monitor_size(true);
    if let Some((x, y)) = restored.pos {
        viewport = viewport.with_position([x, y]);
    }
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        // Force the wgpu/Metal backend on macOS instead of falling back to glow/OpenGL.
        // eframe 0.35 removed `hardware_acceleration`; the wgpu adapter default
        // power preference is already HighPerformance (see egui-wgpu setup.rs).
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "OpenItGo",
        options,
        Box::new(|cc| {
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            #[cfg(target_os = "macos")]
            {
                crate::platform::macos::dock_open::install_dock_open_handler();
                crate::platform::macos::dock_open::set_wake_context(cc.egui_ctx.clone());
            }
            Ok(Box::new(ReaderApp::new(cc)))
        }),
    )
}
