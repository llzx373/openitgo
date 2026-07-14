//! macOS native overlay hosting libmpv's CAOpenGLLayer, mirroring how the
//! ebook webview is overlaid on the egui window. Coordinates are top-left
//! logical points (winit's content view is flipped), same as wry child views.
//!
//! Ownership: `MpvNativeView` holds the `RenderContext` in a `Box` so its
//! address is stable; the CAOpenGLLayer subclass stores that raw pointer in
//! its `_rsRender` ivar. `Drop` zeroes the ivar and frees the mpv render
//! context before releasing the native objects, so no draw callback can ever
//! dereference freed state.

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`;
// same allow as platform.rs's dock_open module.
#![allow(unexpected_cfgs)]
// The MediaView consumer lands in Task 7; until then the bin target (main.rs)
// sees this API as unused (the lib target exposes it via `pub mod platform`).
#![allow(dead_code)]

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

pub struct MpvNativeView {
    view: *mut Object,
    layer: *mut Object,
    render: Option<Box<rust_reader_media::render::RenderContext>>,
}

// The raw NSView/CALayer pointers are only touched from the UI thread that
// owns this value; moving ownership between threads does not alias them.
unsafe impl Send for MpvNativeView {}

fn layer_class() -> &'static Class {
    use std::sync::OnceLock;
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let superclass = Class::get("CAOpenGLLayer").expect("CAOpenGLLayer missing");
        let mut decl =
            ClassDecl::new("RustReaderMpvLayer", superclass).expect("failed to declare layer");
        decl.add_ivar::<usize>("_rsRender");
        // SAFETY: each selector matches the CAOpenGLLayer delegate method
        // signature we register; the fn pointers use the C ABI and the types
        // are layout-compatible with the Objective-C declarations.
        unsafe {
            decl.add_method(
                sel!(copyCGLPixelFormatForDisplayMask:),
                copy_pixel_format as extern "C" fn(&Object, Sel, u32) -> *mut c_void,
            );
            decl.add_method(
                sel!(copyCGLContextForPixelFormat:),
                copy_context as extern "C" fn(&Object, Sel, *mut c_void) -> *mut c_void,
            );
            decl.add_method(
                sel!(canDrawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                can_draw
                    as extern "C" fn(
                        &Object,
                        Sel,
                        *mut c_void,
                        *mut c_void,
                        f64,
                        *const c_void,
                    ) -> BOOL,
            );
            decl.add_method(
                sel!(drawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                draw_in
                    as extern "C" fn(&Object, Sel, *mut c_void, *mut c_void, f64, *const c_void),
            );
        }
        decl.register()
    })
}

extern "C" fn copy_pixel_format(_this: &Object, _sel: Sel, _mask: u32) -> *mut c_void {
    use cgl::{
        kCGLPFAAccelerated, kCGLPFADoubleBuffer, kCGLPFANoRecovery, CGLChoosePixelFormat,
        CGLPixelFormatAttribute,
    };
    // Default (legacy) GL profile is enough for libmpv.
    let attrs: [CGLPixelFormatAttribute; 4] = [
        kCGLPFAAccelerated,
        kCGLPFANoRecovery,
        kCGLPFADoubleBuffer,
        0,
    ];
    let mut pf: cgl::CGLPixelFormatObj = std::ptr::null_mut();
    let mut npix: i32 = 0;
    // SAFETY: attrs is a valid 0-terminated attribute array; pf/npix are valid
    // out-pointers that outlive the call.
    unsafe {
        CGLChoosePixelFormat(attrs.as_ptr(), &mut pf, &mut npix);
    }
    pf
}

extern "C" fn copy_context(_this: &Object, _sel: Sel, pf: *mut c_void) -> *mut c_void {
    let mut ctx: cgl::CGLContextObj = std::ptr::null_mut();
    // SAFETY: pf is the pixel format object QuartzCore just handed us; ctx is
    // a valid out-pointer.
    unsafe {
        cgl::CGLCreateContext(pf, std::ptr::null_mut(), &mut ctx);
    }
    ctx
}

extern "C" fn can_draw(
    _this: &Object,
    _sel: Sel,
    _ctx: *mut c_void,
    _pf: *mut c_void,
    _t: f64,
    _ts: *const c_void,
) -> BOOL {
    YES
}

extern "C" fn draw_in(
    this: &Object,
    _sel: Sel,
    _ctx: *mut c_void,
    _pf: *mut c_void,
    _t: f64,
    _ts: *const c_void,
) {
    // SAFETY: `this` is a RustReaderMpvLayer instance; the ivar was declared
    // with usize layout. The pointer it holds aliases the Box in
    // MpvNativeView and is zeroed before that box is dropped, so a draw after
    // teardown is a no-op.
    let ptr: usize = unsafe { *this.get_ivar("_rsRender") };
    if ptr == 0 {
        return;
    }
    // SAFETY: per the ivar contract above, ptr references a live RenderContext
    // and we are on the layer's render thread with the CGL context current.
    let render = unsafe { &*(ptr as *const rust_reader_media::render::RenderContext) };
    // Advanced control makes update() a hard requirement after each update
    // callback; draw_in is the first render-thread call after the callback
    // hopped here via setNeedsDisplay.
    render.update();
    // SAFETY: `this` is a valid CALayer; `bounds`/`contentsScale` are plain
    // getters that don't transfer ownership.
    let bounds: core_graphics::geometry::CGRect = unsafe { msg_send![this, bounds] };
    let scale: f64 = unsafe { msg_send![this, contentsScale] };
    let w = (bounds.size.width * scale) as i32;
    let h = (bounds.size.height * scale) as i32;
    if w > 0 && h > 0 {
        render.render(w, h);
        render.report_swap();
    }
}

impl MpvNativeView {
    pub fn new<
        W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle,
    >(
        parent: &W,
        bounds: wry::Rect,
        player: &rust_reader_media::MpvPlayer,
    ) -> Result<Self, String> {
        use wry::raw_window_handle::RawWindowHandle;
        let handle = parent
            .window_handle()
            .map_err(|e| format!("无法获取窗口句柄: {e:?}"))?;
        let ns_view = match handle.as_raw() {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut Object,
            _ => return Err("媒体播放暂仅支持 macOS".to_string()),
        };
        // SAFETY: we are about to create a CAOpenGLLayer (OpenGL-capable
        // environment); the render context is stored in Self and dropped
        // before the player per MpvNativeView's lifetime contract.
        let render = unsafe { rust_reader_media::render::RenderContext::new(player) }
            .map_err(|e| e.to_string())?;
        let mut render = Box::new(render);
        // SAFETY: all Objective-C messages below run on the UI thread that
        // owns the parent window; every object is a valid, live instance
        // (freshly allocated or the window's content view), and selectors
        // match the receivers' classes.
        let (view, layer) = unsafe {
            let frame = make_frame(&bounds);
            let view: *mut Object = msg_send![class!(NSView), alloc];
            let view: *mut Object = msg_send![view, initWithFrame: frame];
            let () = msg_send![view, setWantsLayer: YES];
            let layer: *mut Object = msg_send![layer_class(), alloc];
            let layer: *mut Object = msg_send![layer, init];
            (*layer).set_ivar::<usize>("_rsRender", &mut *render as *mut _ as usize);
            let () = msg_send![layer, setAsynchronous: YES];
            let () = msg_send![layer, setNeedsDisplayOnBoundsChange: YES];
            // Retina: match the window's backing scale.
            let window: *mut Object = msg_send![ns_view, window];
            let scale: f64 = msg_send![window, backingScaleFactor];
            let () = msg_send![layer, setContentsScale: scale];
            let () = msg_send![view, setLayer: layer];
            let () = msg_send![ns_view, addSubview: view];
            (view, layer)
        };
        let layer_addr = layer as usize;
        render.set_update_callback(move || {
            // mpv calls this from arbitrary threads; hop to the main
            // thread. `layer` stays alive: the callback is unset (via
            // RenderContext::drop) before the layer is released.
            let layer = layer_addr as *mut Object;
            // SAFETY: per the lifetime note above, layer is valid;
            // setNeedsDisplay takes no arguments and transfers nothing.
            unsafe {
                let () = msg_send![
                    layer,
                    performSelectorOnMainThread: sel!(setNeedsDisplay)
                    withObject: std::ptr::null_mut::<Object>()
                    waitUntilDone: NO
                ];
            }
        });
        Ok(Self {
            view,
            layer,
            render: Some(render),
        })
    }

    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.view is a live NSView owned by us; setFrame: is a plain
        // setter that copies the rect.
        unsafe {
            let () = msg_send![self.view, setFrame: make_frame(&bounds)];
        }
    }

    pub fn remove_from_superview(&self) {
        // SAFETY: self.view is a live NSView owned by us.
        unsafe {
            let () = msg_send![self.view, removeFromSuperview];
        }
    }
}

impl Drop for MpvNativeView {
    fn drop(&mut self) {
        // SAFETY: view/layer are live objects owned by us (alloc/init +1);
        // zeroing the ivar first guarantees any in-flight draw callback
        // becomes a no-op before the render context is freed.
        unsafe {
            let () = msg_send![self.view, removeFromSuperview];
            (*self.layer).set_ivar::<usize>("_rsRender", 0);
        }
        self.render.take(); // frees the mpv render context before the layer dies
                            // SAFETY: balanced release for the alloc/init retains; the view also
                            // retained the layer via setLayer, which keeps it valid until here.
        unsafe {
            let () = msg_send![self.layer, release];
            let () = msg_send![self.view, release];
        }
    }
}

fn make_frame(bounds: &wry::Rect) -> core_graphics::geometry::CGRect {
    use wry::dpi::{Position, Size};
    let (x, y) = match bounds.position {
        Position::Logical(p) => (p.x, p.y),
        Position::Physical(p) => (p.x as f64, p.y as f64),
    };
    let (w, h) = match bounds.size {
        Size::Logical(s) => (s.width, s.height),
        Size::Physical(s) => (s.width as f64, s.height as f64),
    };
    core_graphics::geometry::CGRect::new(
        &core_graphics::geometry::CGPoint::new(x, y),
        &core_graphics::geometry::CGSize::new(w, h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wry::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

    #[test]
    fn make_frame_logical_passthrough() {
        let rect = wry::Rect {
            position: LogicalPosition::new(10.0, 20.0).into(),
            size: LogicalSize::new(640.0, 480.0).into(),
        };
        let frame = make_frame(&rect);
        assert_eq!(frame.origin.x, 10.0);
        assert_eq!(frame.origin.y, 20.0);
        assert_eq!(frame.size.width, 640.0);
        assert_eq!(frame.size.height, 480.0);
    }

    #[test]
    fn make_frame_physical_widens_to_f64() {
        let rect = wry::Rect {
            position: PhysicalPosition::new(3, 4).into(),
            size: PhysicalSize::new(100u32, 50u32).into(),
        };
        let frame = make_frame(&rect);
        assert_eq!(frame.origin.x, 3.0);
        assert_eq!(frame.origin.y, 4.0);
        assert_eq!(frame.size.width, 100.0);
        assert_eq!(frame.size.height, 50.0);
    }
}
