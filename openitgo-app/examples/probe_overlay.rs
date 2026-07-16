//! Overlay-compositing probe for the mpv-under-egui architecture, round 2:
//! same transparent-eframe experiment as round 1 (opaque top/bottom bars,
//! central transparent hole, semi-transparent green popup over the hole,
//! programmatic resize after 2s), but the red CALayer simulating the video
//! layer is inserted as a *sibling below* the winit view's CAMetalLayer via
//! `insertSublayer:below:` on the metal layer's superlayer. Round 1 proved
//! the view's layer IS the CAMetalLayer, so `insertSublayer:atIndex:0` had
//! landed *inside* the metal layer, above all egui content. The red layer's
//! frame is recomputed whenever the metal layer's frame changes (inside a
//! CATransaction with disabled actions) so the hole tracks window resizes.
//! Verify with a screenshot: the hole shows red, the popup blends over red,
//! the bars are fully opaque.
//! Usage: cargo run -p openitgo-app --example probe_overlay

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`.
#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
mod imp {
    use core_foundation::base::TCFType;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use objc::runtime::{Object, BOOL, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

    /// Hole margins in points, identical to round 1.
    const BAR_MARGIN_PT: f64 = 60.0;

    struct ProbeApp {
        /// Probe layer, retained at +1 between `alloc`/`init` and `on_exit`;
        /// null until successfully inserted below the metal layer.
        red_layer: *mut Object,
        /// Metal layer frame (superlayer coordinates) the red frame was last
        /// synced to: (x, y, width, height) in points.
        last_metal_frame: Option<(f64, f64, f64, f64)>,
        attach_retries: u32,
        start: std::time::Instant,
        resized: bool,
        dumped_post_resize: bool,
    }

    /// Central "hole" rectangle in superlayer coordinates, derived from the
    /// metal layer's frame the same way round 1 derived it from the layer
    /// bounds: full width, 60pt clear at top and bottom.
    fn hole_rect(metal_frame: CGRect) -> CGRect {
        CGRect::new(
            &CGPoint::new(metal_frame.origin.x, metal_frame.origin.y + BAR_MARGIN_PT),
            &CGSize::new(
                metal_frame.size.width,
                metal_frame.size.height - 2.0 * BAR_MARGIN_PT,
            ),
        )
    }

    fn frame_tuple(r: CGRect) -> (f64, f64, f64, f64) {
        (r.origin.x, r.origin.y, r.size.width, r.size.height)
    }

    /// # Safety
    /// `obj` must be a live NSObject.
    unsafe fn class_name(obj: *mut Object) -> String {
        unsafe {
            let name: *mut Object = msg_send![obj, className];
            let c: *const std::os::raw::c_char = msg_send![name, UTF8String];
            std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
        }
    }

    /// # Safety
    /// `layer` must be a live CALayer.
    unsafe fn sublayers_of(layer: *mut Object) -> Vec<*mut Object> {
        unsafe {
            let subs: *mut Object = msg_send![layer, sublayers];
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
        /// Dumps the layer tree in the same `[layer-diag]` format as round 1,
        /// tagging the red probe layer and the CAMetalLayer among the
        /// superlayer's siblings so their relative order is explicit.
        ///
        /// # Safety
        /// `view_layer` and `red` must be live layers on the UI thread.
        unsafe fn dump_layer_tree(view_layer: *mut Object, red: *mut Object) {
            unsafe {
                let vsubs = sublayers_of(view_layer);
                println!(
                    "[layer-diag] view.layer class={} sublayers={}",
                    class_name(view_layer),
                    vsubs.len()
                );
                for (i, s) in vsubs.iter().enumerate() {
                    let tag = if *s == red { "  ← RED (probe)" } else { "" };
                    println!(
                        "[layer-diag]   sublayer[{}] class={}{}",
                        i,
                        class_name(*s),
                        tag
                    );
                }
                let sup: *mut Object = msg_send![view_layer, superlayer];
                if sup.is_null() {
                    println!("[layer-diag] superlayer = nil");
                    return;
                }
                let ssubs = sublayers_of(sup);
                println!(
                    "[layer-diag] superlayer class={} sublayers={}",
                    class_name(sup),
                    ssubs.len()
                );
                let mut red_idx = None;
                let mut metal_idx = None;
                for (i, s) in ssubs.iter().enumerate() {
                    if *s == red {
                        red_idx = Some(i);
                    }
                    if *s == view_layer {
                        metal_idx = Some(i);
                    }
                    let tag = if *s == red {
                        "  ← RED (probe)"
                    } else if *s == view_layer {
                        "  ← view.layer (egui surface)"
                    } else {
                        ""
                    };
                    println!(
                        "[layer-diag]   super.sublayer[{}] class={}{}",
                        i,
                        class_name(*s),
                        tag
                    );
                }
                match (red_idx, metal_idx) {
                    (Some(r), Some(m)) => println!(
                        "[layer-diag] red index {} {} metal index {} (need red < metal)",
                        r,
                        if r < m { "<" } else { ">=" },
                        m
                    ),
                    _ => {
                        println!("[layer-diag] WARNING: red layer and metal layer are not siblings")
                    }
                }
            }
        }

        /// Returns the winit view's layer (the CAMetalLayer), or null when the
        /// window handle is unavailable yet.
        fn ns_view_layer(frame: &eframe::Frame) -> *mut Object {
            let Ok(handle) = frame.window_handle() else {
                return std::ptr::null_mut();
            };
            let RawWindowHandle::AppKit(h) = handle.as_raw() else {
                return std::ptr::null_mut();
            };
            let ns_view = h.ns_view.as_ptr() as *mut Object;
            // SAFETY: the winit view is alive on the UI thread.
            unsafe {
                let () = msg_send![ns_view, setWantsLayer: YES];
                msg_send![ns_view, layer]
            }
        }

        fn attach_red_layer(&mut self, frame: &eframe::Frame) {
            if !self.red_layer.is_null() {
                return;
            }
            let view_layer = Self::ns_view_layer(frame);
            if view_layer.is_null() {
                return;
            }
            // SAFETY: all messages run on the UI thread against live AppKit
            // objects (the winit view and freshly allocated layers); selectors
            // match their receivers' classes.
            unsafe {
                // Make the round-1 premise explicit so a future wgpu/winit
                // upgrade that changes the layer structure is noticed early.
                let is_metal: BOOL = msg_send![view_layer, isKindOfClass: class!(CAMetalLayer)];
                println!(
                    "[probe] view.layer class={} isKindOfClass CAMetalLayer = {}",
                    class_name(view_layer),
                    is_metal == YES
                );
                let superlayer: *mut Object = msg_send![view_layer, superlayer];
                if superlayer.is_null() {
                    // The view is not hooked into the window layer tree yet.
                    // Do NOT fall back to the round-1 insertion (it landed
                    // inside the metal layer); retry on a later frame.
                    self.attach_retries += 1;
                    if self.attach_retries <= 5 || self.attach_retries.is_multiple_of(60) {
                        println!(
                            "[probe] superlayer is nil, retrying next frame (attempt {})",
                            self.attach_retries
                        );
                    }
                    return;
                }
                let red: *mut Object = msg_send![class!(CALayer), alloc];
                let red: *mut Object = msg_send![red, init];
                // core-graphics 0.24's `CGColor::rgb` returns `Self` (not
                // Result), and `CGColor` does not implement objc::Encode, so
                // the raw CGColorRef is passed as `*mut c_void` like
                // mpv_view.rs's setBackgroundColor.
                let cg = core_graphics::color::CGColor::rgb(1.0, 0.0, 0.0, 1.0);
                let cg_ptr = cg.as_concrete_TypeRef() as *mut std::ffi::c_void;
                let () = msg_send![red, setBackgroundColor: cg_ptr];
                let metal_frame: CGRect = msg_send![view_layer, frame];
                let () = msg_send![class!(CATransaction), begin];
                let () = msg_send![class!(CATransaction), setDisableActions: YES];
                let () = msg_send![red, setFrame: hole_rect(metal_frame)];
                // Round-2 fix: sibling insertion *below* the CAMetalLayer.
                let () = msg_send![superlayer, insertSublayer: red below: view_layer];
                let () = msg_send![class!(CATransaction), commit];
                println!(
                    "[probe] red layer inserted into superlayer (class={}) below view.layer",
                    class_name(superlayer)
                );
                self.red_layer = red; // kept at +1, released in on_exit
                self.last_metal_frame = Some(frame_tuple(metal_frame));
                Self::dump_layer_tree(view_layer, red);
            }
        }

        /// Keeps the red layer's frame following the metal layer's frame
        /// (a plain CALayer has no autoresizing). CATransaction with disabled
        /// actions avoids implicit-animation jitter/ghosting during resizes.
        fn sync_red_frame(&mut self, frame: &eframe::Frame) {
            if self.red_layer.is_null() {
                return;
            }
            let view_layer = Self::ns_view_layer(frame);
            if view_layer.is_null() {
                return;
            }
            // SAFETY: UI thread, live objects.
            unsafe {
                let metal_frame: CGRect = msg_send![view_layer, frame];
                if self.last_metal_frame == Some(frame_tuple(metal_frame)) {
                    return;
                }
                let () = msg_send![class!(CATransaction), begin];
                let () = msg_send![class!(CATransaction), setDisableActions: YES];
                let () = msg_send![self.red_layer, setFrame: hole_rect(metal_frame)];
                let () = msg_send![class!(CATransaction), commit];
                println!(
                    "[probe] metal frame {:?} -> {:?}; red frame updated to {:?}",
                    self.last_metal_frame,
                    frame_tuple(metal_frame),
                    frame_tuple(hole_rect(metal_frame))
                );
                self.last_metal_frame = Some(frame_tuple(metal_frame));
                if self.resized && !self.dumped_post_resize {
                    self.dumped_post_resize = true;
                    println!("[probe] post-resize layer tree:");
                    Self::dump_layer_tree(view_layer, self.red_layer);
                }
            }
        }
    }

    impl eframe::App for ProbeApp {
        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            [0.0, 0.0, 0.0, 0.0]
        }

        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.attach_red_layer(frame);
            self.sync_red_frame(frame);
            if !self.resized && self.start.elapsed() > std::time::Duration::from_secs(2) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(900.0, 640.0)));
                self.resized = true;
            }
            let bar = egui::Frame::none().fill(egui::Color32::from_rgb(30, 60, 160));
            egui::TopBottomPanel::top("probe_top")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("不透明顶栏（应完全遮挡下层红色）");
                });
            egui::TopBottomPanel::bottom("probe_bottom")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("不透明底栏");
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
                            ui.label("半透明弹层（应混色在红色之上）");
                        });
                });
            ctx.request_repaint();
        }

        fn on_exit(&mut self) {
            if self.red_layer.is_null() {
                return;
            }
            // SAFETY: balances the alloc/init retain; the superlayer holds its
            // own retain on the layer while it is inserted.
            unsafe {
                let () = msg_send![self.red_layer, release];
            }
            self.red_layer = std::ptr::null_mut();
        }
    }

    pub fn run() {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([700.0, 480.0])
            .with_transparent(true);
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            hardware_acceleration: eframe::HardwareAcceleration::Required,
            ..Default::default()
        };
        let start = std::time::Instant::now();
        eframe::run_native(
            "OPENITGO_PROBE_OVERLAY",
            options,
            Box::new(|_cc| {
                Ok(Box::new(ProbeApp {
                    red_layer: std::ptr::null_mut(),
                    last_metal_frame: None,
                    attach_retries: 0,
                    start,
                    resized: false,
                    dumped_post_resize: false,
                }))
            }),
        )
        .expect("eframe run_native failed");
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    imp::run();
    #[cfg(not(target_os = "macos"))]
    println!("probe_overlay 仅支持 macOS");
}
