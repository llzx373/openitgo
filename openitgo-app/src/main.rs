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
    // 最大化启动（Windows）：不传 with_maximized——winit 创建期
    // set_maximized 内部的 ShowWindow(SW_MAXIMIZE) 会强制显示尚未绘制的
    // 窗口，紧随的 SW_HIDE 藏回之前 DWM 已把全屏黑窗合成上屏（启动闪黑），
    // 且该序列先于任何应用代码，无法拦截。改为按保存尺寸创建普通隐藏窗口，
    // 由 ReaderApp::new 经 platform::startup_cloak 在 DWM cloak 遮蔽下写入
    // 最大化 show state，首帧几何验证通过后才解除遮蔽（app.rs
    // maybe_uncloak_startup_window）。还原矩形仍由 platform::restore_rect
    // 写成保存值；winit 侧的 Maximized 标志由 maybe_validate_window_geometry
    // 补发同步（cloaked 下无闪烁）。
    if restored.maximized {
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_inner_size([w, h]);
        }
        #[cfg(not(target_os = "windows"))]
        {
            // 非 Windows 维持创建期最大化：egui-winit 创建后若再按
            // inner_size 调 winit request_inner_size，会异步撤销创建期
            // 最大化（SW_RESTORE），紧随的 set_maximized 因 winit 内部
            // 标志未同步成为空操作；不传 inner_size 则整链跳过。
            viewport = viewport.with_maximized(true);
        }
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
