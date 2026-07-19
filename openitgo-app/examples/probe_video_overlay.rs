//! Real-video compositing probe for the mpv-under-egui flip (Task 4):
//! the window has opaque top/bottom bars and a central transparent hole
//! with a semi-transparent green popup over it; the hole is filled by the
//! REAL `MpvNativeView` — the CAOpenGLLayer playing an
//! actual video via libmpv — instead of a red placeholder CALayer. The video
//! layer anchors below the winit view's CAMetalLayer inside
//! `MpvNativeView::new`; the egui surface is transparent, so the video shows
//! through the hole and the green egui popup must blend OVER the playing
//! video. A layer-tree dump after attach prints the video layer's sibling
//! index vs the CAMetalLayer's (video must be lower).
//! Verify with a screenshot: the hole shows the video, the popup blends
//! over it, the bars are fully opaque.
//! Usage: cargo run -p openitgo-app --example probe_video_overlay -- <video-file>

#[cfg(target_os = "macos")]
mod imp {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use openitgo_app::platform::macos::mpv_view::MpvNativeView;
    use openitgo_media::MpvPlayer;
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

    /// Hole margins in points.
    const BAR_MARGIN_PT: f64 = 60.0;

    struct ProbeApp {
        player: Option<MpvPlayer>,
        video: Option<MpvNativeView>,
        path: String,
        attach_retries: u32,
    }

    /// # Safety
    /// `obj` must be a live NSObject.
    unsafe fn class_name(obj: *mut AnyObject) -> String {
        unsafe {
            let name: *mut AnyObject = msg_send![obj, className];
            let c: *const std::os::raw::c_char = msg_send![name, UTF8String];
            std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
        }
    }

    /// # Safety
    /// `layer` must be a live CALayer.
    unsafe fn sublayers_of(layer: *mut AnyObject) -> Vec<*mut AnyObject> {
        unsafe {
            let subs: *mut AnyObject = msg_send![layer, sublayers];
            if subs.is_null() {
                return Vec::new();
            }
            let count: usize = msg_send![subs, count];
            (0..count)
                .map(|i| msg_send![subs, objectAtIndex: i])
                .collect()
        }
    }

    impl ProbeApp {
        fn hole_bounds(ctx: &egui::Context) -> wry::Rect {
            let screen = ctx.screen_rect();
            wry::Rect {
                position: wry::dpi::LogicalPosition::new(0.0, BAR_MARGIN_PT).into(),
                size: wry::dpi::LogicalSize::new(
                    screen.width(),
                    (screen.height() - 2.0 * BAR_MARGIN_PT as f32).max(0.0),
                )
                .into(),
            }
        }

        /// Dumps the superlayer's sibling list, tagging the video layer
        /// (OpenItGoMpvLayer) and the CAMetalLayer so their relative order
        /// is explicit.
        ///
        /// # Safety
        /// `view_layer` must be a live layer on the UI thread.
        unsafe fn dump_layer_tree(view_layer: *mut AnyObject) {
            unsafe {
                let sup: *mut AnyObject = msg_send![view_layer, superlayer];
                if sup.is_null() {
                    println!("[layer-diag] superlayer = nil");
                    return;
                }
                let ssubs = sublayers_of(sup);
                println!(
                    "[layer-diag] view.layer class={} ; superlayer class={} sublayers={}",
                    class_name(view_layer),
                    class_name(sup),
                    ssubs.len()
                );
                let mut video_idx = None;
                let mut metal_idx = None;
                for (i, s) in ssubs.iter().enumerate() {
                    let name = class_name(*s);
                    if name == "OpenItGoMpvLayer" {
                        video_idx = Some(i);
                    }
                    if *s == view_layer {
                        metal_idx = Some(i);
                    }
                    let tag = if name == "OpenItGoMpvLayer" {
                        "  ← VIDEO (mpv)"
                    } else if *s == view_layer {
                        "  ← view.layer (egui surface)"
                    } else {
                        ""
                    };
                    println!("[layer-diag]   super.sublayer[{i}] class={name}{tag}");
                }
                match (video_idx, metal_idx) {
                    (Some(v), Some(m)) => println!(
                        "[layer-diag] video index {} {} metal index {} (need video < metal)",
                        v,
                        if v < m { "<" } else { ">=" },
                        m
                    ),
                    _ => println!(
                        "[layer-diag] WARNING: video layer and metal layer are not siblings"
                    ),
                }
            }
        }

        /// Creates the player + native video layer on the first frame where
        /// the window layer tree is ready (superlayer non-nil).
        fn attach_video(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
            if self.video.is_some() {
                return;
            }
            if self.player.is_none() {
                let ctx2 = ctx.clone();
                self.player = Some(
                    MpvPlayer::new(Box::new(move || ctx2.request_repaint()))
                        .expect("mpv init failed"),
                );
            }
            let bounds = Self::hole_bounds(ctx);
            let player = self.player.as_ref().expect("player just created");
            match MpvNativeView::new(frame, bounds, player) {
                Ok(view) => {
                    self.video = Some(view);
                    player
                        .load_file(std::path::Path::new(&self.path))
                        .expect("loadfile failed");
                    println!("[probe] MpvNativeView attached below CAMetalLayer");
                    // SAFETY: the winit view is alive on the UI thread.
                    unsafe {
                        let RawWindowHandle::AppKit(h) = frame.window_handle().unwrap().as_raw()
                        else {
                            return;
                        };
                        let ns_view = h.ns_view.as_ptr() as *mut AnyObject;
                        let view_layer: *mut AnyObject = msg_send![ns_view, layer];
                        Self::dump_layer_tree(view_layer);
                    }
                }
                Err(e) => {
                    self.attach_retries += 1;
                    if self.attach_retries <= 5 || self.attach_retries.is_multiple_of(60) {
                        println!(
                            "[probe] attach failed (attempt {}): {e}",
                            self.attach_retries
                        );
                    }
                }
            }
        }
    }

    impl eframe::App for ProbeApp {
        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            [0.0, 0.0, 0.0, 0.0]
        }

        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.attach_video(ctx, frame);
            if let Some(view) = self.video.as_ref() {
                view.set_bounds(Self::hole_bounds(ctx));
            }
            let bar = egui::Frame::none().fill(egui::Color32::from_rgb(30, 60, 160));
            egui::TopBottomPanel::top("probe_top")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("opaque top bar (must fully cover the video)");
                });
            egui::TopBottomPanel::bottom("probe_bottom")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("opaque bottom bar");
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |_ui| {});
            // Semi-transparent popup inside the hole, like a dropdown menu.
            egui::Area::new(egui::Id::new("probe_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(120.0, 120.0))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgba_unmultiplied(0, 255, 0, 128))
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(220.0, 90.0));
                            ui.label("semi-transparent popup (must blend over the video)");
                        });
                });
            ctx.request_repaint();
        }
    }

    pub fn run(path: String) {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([700.0, 480.0])
            .with_transparent(true);
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            hardware_acceleration: eframe::HardwareAcceleration::Required,
            ..Default::default()
        };
        eframe::run_native(
            "OPENITGO_PROBE_VIDEO_OVERLAY",
            options,
            Box::new(|_cc| {
                Ok(Box::new(ProbeApp {
                    player: None,
                    video: None,
                    path,
                    attach_retries: 0,
                }))
            }),
        )
        .expect("eframe run_native failed");
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    {
        let path = std::env::args()
            .nth(1)
            .expect("usage: probe_video_overlay <video-file>");
        imp::run(path);
    }
    #[cfg(not(target_os = "macos"))]
    println!("probe_video_overlay 仅支持 macOS");
}
