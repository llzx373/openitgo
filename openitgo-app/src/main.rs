// release 构建为 GUI 子系统，避免启动时附带控制台窗口；debug 保留控制台以查看日志
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cache;
mod ebook_renderer;
mod extract_dialog;
mod extract_manager;
mod loader;
mod opener;
mod platform;
mod shortcuts;
mod temp_open;
mod theme;
mod timing;
mod views;
mod webp_thumb;
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

    let mut viewport = egui::ViewportBuilder::default().with_clamp_size_to_monitor_size(true);
    // 最大化时不传 inner_size：egui-winit 创建窗口后会按 inner_size 调
    // winit request_inner_size，它异步 ShowWindow(SW_RESTORE) 撤销创建期
    // 最大化，且紧随的 set_maximized 因 winit 内部标志尚未同步成为空操作。
    // 不传 inner_size 则该调用被跳过，创建期最大化存活——窗口创建出来即
    // 最大化，全程无 resize / surface 重配置 / 黑帧。还原矩形由 ReaderApp
    // 确认最大化后经 platform::restore_rect 写成保存值；罕见丢失时由
    // maybe_validate_window_geometry 首帧补发兜底。
    if restored.maximized {
        viewport = viewport.with_maximized(true);
    } else {
        viewport = viewport.with_inner_size([w, h]);
    }
    // Transparent backbuffer (macOS only): egui-wgpu then picks
    // CompositeAlphaMode::PreMultiplied and the CAMetalLayer becomes
    // non-opaque, so the video layer below the egui surface (Task 4)
    // shows through unpainted regions. Windows hosts video in an HWND
    // child window and keeps an opaque surface.
    #[cfg(target_os = "macos")]
    {
        viewport = viewport.with_transparent(true);
    }
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
