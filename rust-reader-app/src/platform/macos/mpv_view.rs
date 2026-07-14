//! macOS native overlay hosting libmpv's CAOpenGLLayer, mirroring how the
//! ebook webview is overlaid on the egui window. Coordinates are top-left
//! logical points (winit's content view is flipped), same as wry child views.
//!
//! GL context lifecycle: `MpvNativeView` pre-builds one CGLPixelFormat and
//! one CGLContext. The same context is current for
//! `mpv_render_context_create`, every `drawInCGLContext` and the final
//! `mpv_render_context_free`, satisfying render.h's "same context" rule —
//! the app (wgpu/Metal) has no current GL context on the UI thread, and
//! creating the mpv render context without one segfaults. The layer's
//! `copyCGLPixelFormatForDisplayMask:`/`copyCGLContextForPixelFormat:` return
//! these pre-built objects with +1 retain (copy semantics).
//!
//! Teardown: the layer owns a `Box<LayerState>` (ivar `_rsState`, freed in
//! `dealloc`) holding `Mutex<Option<RenderContext>>`. Draws `try_lock` and
//! skip a frame on contention, so the CA render thread never blocks;
//! `Drop` takes the render context out under a blocking lock, which both
//! gates new draws (they see `None`) and waits out any in-flight draw
//! (render.h: only one mpv_render_* call at a time).
//!
//! Update drive: with MPV_RENDER_PARAM_ADVANCED_CONTROL=1 every update
//! callback must be answered by `mpv_render_context_update()` or the vo core
//! wedges (and free/commands/terminate then hang). CoreAnimation stops
//! scheduling draws for hidden windows, so the update callback hops to the
//! main thread's `rsDriveUpdate` selector, which answers `update()` under
//! the mutex with the CGL context current — draws stay optional.

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`;
// same allow as platform.rs's dock_open module.
#![allow(unexpected_cfgs)]
// The MediaView consumer lands in Task 7; until then the bin target (main.rs)
// sees this API as unused (the lib target exposes it via `pub mod platform`).
#![allow(dead_code)]

// `msg_send![this, bounds]` returns CGRect through plain objc_msgSend; the
// arm64 ABI handles struct returns uniformly, x86_64 would need
// objc_msgSend_stret. Fail the build instead of silently miscompiling.
#[cfg(not(target_arch = "aarch64"))]
compile_error!(
    "mpv_view assumes the arm64 objc_msgSend struct-return ABI (CGRect); Intel macOS needs stret handling"
);

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use rust_reader_media::render::RenderContext;
use std::ffi::c_void;
use std::sync::Mutex;

#[link(name = "OpenGL", kind = "framework")]
extern "C" {
    // Not exposed by the cgl crate (0.3.2), declared in <OpenGL/CGLContext.h>.
    fn CGLRetainContext(ctx: cgl::CGLContextObj) -> cgl::CGLContextObj;
    fn CGLReleaseContext(ctx: cgl::CGLContextObj);
}

/// Shared between the layer's draw callback and `MpvNativeView::drop`.
/// Owned by the layer via the `_rsState` ivar and freed in `dealloc`, so any
/// in-flight draw (the receiver stays alive for its whole method call) always
/// finds the pointee valid.
struct LayerState {
    /// Serializes mpv_render_* calls; `None` once teardown has taken the
    /// render context out.
    render: Mutex<Option<RenderContext>>,
    cgl_pf: cgl::CGLPixelFormatObj,
    cgl_ctx: cgl::CGLContextObj,
}

pub struct MpvNativeView {
    view: *mut Object,
    layer: *mut Object,
    state: *mut LayerState,
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
        decl.add_ivar::<usize>("_rsState");
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
            decl.add_method(
                sel!(rsDriveUpdate),
                drive_update as extern "C" fn(&Object, Sel),
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&Object, Sel));
        }
        decl.register()
    })
}

/// Reads the `_rsState` ivar. Returns `None` only after `dealloc` has run
/// (which cannot race a live method call on the layer).
fn state_from_ivar(this: &Object) -> Option<&LayerState> {
    // SAFETY: `this` is a RustReaderMpvLayer instance; the ivar was declared
    // with usize layout.
    let ptr: usize = unsafe { *this.get_ivar("_rsState") };
    if ptr == 0 {
        return None;
    }
    // SAFETY: ptr came from Box::into_raw in MpvNativeView::new and is freed
    // only in dealloc; the layer (method receiver) outlives this call, so the
    // pointee is valid for its duration.
    Some(unsafe { &*(ptr as *const LayerState) })
}

/// Runs `f` with `ctx` as the current CGL context, restoring the previous
/// one. The CGL lock serializes with CoreAnimation's own use of the same
/// context on its render thread.
fn with_current_context<R>(ctx: cgl::CGLContextObj, f: impl FnOnce() -> R) -> R {
    // SAFETY: ctx is a valid CGL context owned by us.
    unsafe {
        cgl::CGLLockContext(ctx);
        let prev = cgl::CGLGetCurrentContext();
        cgl::CGLSetCurrentContext(ctx);
        let r = f();
        cgl::CGLSetCurrentContext(prev);
        cgl::CGLUnlockContext(ctx);
        r
    }
}

fn create_pixel_format() -> Option<cgl::CGLPixelFormatObj> {
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
    if pf.is_null() {
        None
    } else {
        Some(pf)
    }
}

extern "C" fn copy_pixel_format(this: &Object, _sel: Sel, _mask: u32) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { cgl::CGLRetainPixelFormat(state.cgl_pf) },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn copy_context(this: &Object, _sel: Sel, _pf: *mut c_void) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { CGLRetainContext(state.cgl_ctx) },
        None => std::ptr::null_mut(),
    }
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
    let Some(state) = state_from_ivar(this) else {
        return;
    };
    // Never block the CA render thread: a contended lock means teardown is
    // freeing the render context — skip this frame.
    let Ok(guard) = state.render.try_lock() else {
        return;
    };
    let Some(render) = guard.as_ref() else {
        return;
    };
    // CoreAnimation has already made `_ctx` (our pre-built context, handed
    // out by copy_context) current on its render thread, as render.h requires.
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

/// Runs on the main thread (via performSelectorOnMainThread from the mpv
/// update callback). Answers every update callback with
/// `mpv_render_context_update()` — a hard requirement with advanced control,
/// independent of whether CoreAnimation schedules a draw (hidden windows).
extern "C" fn drive_update(this: &Object, _sel: Sel) {
    let Some(state) = state_from_ivar(this) else {
        return;
    };
    // Blocking lock: renders are short (BLOCK_FOR_TARGET_TIME=0), and every
    // callback must be answered to keep the vo core from wedging.
    let guard = state.render.lock().unwrap_or_else(|e| e.into_inner());
    let Some(render) = guard.as_ref() else {
        return;
    };
    let flags = with_current_context(state.cgl_ctx, || render.update());
    // MPV_RENDER_UPDATE_FRAME from render.h (bit values are ABI-stable).
    const MPV_RENDER_UPDATE_FRAME: u64 = 1;
    if flags & MPV_RENDER_UPDATE_FRAME != 0 {
        // SAFETY: `this` is a live layer (performSelector retained it);
        // setNeedsDisplay takes no arguments.
        unsafe {
            let () = msg_send![this, setNeedsDisplay];
        }
    }
}

extern "C" fn dealloc(this: &Object, _sel: Sel) {
    // SAFETY: `this` is a RustReaderMpvLayer; the ivar was declared as usize.
    let ptr: usize = unsafe { *this.get_ivar("_rsState") };
    if ptr != 0 {
        // SAFETY: ptr came from Box::into_raw in MpvNativeView::new; dealloc
        // runs at most once, so the box is reclaimed exactly once. No draw can
        // be in-flight: a method receiver stays alive for its whole call.
        let state = unsafe { Box::from_raw(ptr as *mut LayerState) };
        // Defensive: if the view was leaked without Drop (e.g. mem::forget),
        // free the render context here with the CGL context current.
        if let Ok(mut guard) = state.render.lock() {
            if let Some(render) = guard.take() {
                with_current_context(state.cgl_ctx, || drop(render));
            }
        }
        // SAFETY: balanced release of the base references created in new().
        unsafe {
            CGLReleaseContext(state.cgl_ctx);
            cgl::CGLReleasePixelFormat(state.cgl_pf);
        }
    }
    // SAFETY: forwards to CAOpenGLLayer's dealloc, required by ObjC rules.
    unsafe {
        let superclass = this
            .class()
            .superclass()
            .expect("CAOpenGLLayer superclass missing");
        msg_send![super(this, superclass), dealloc]
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
        // Pre-build the GL objects; the same CGL context backs the mpv render
        // context for its whole lifetime (create/render/free). The UI thread
        // normally has no current GL context, and mpv_render_context_create
        // requires one.
        let pf = create_pixel_format().ok_or("CGLChoosePixelFormat 失败".to_string())?;
        let mut ctx: cgl::CGLContextObj = std::ptr::null_mut();
        // SAFETY: pf is a valid pixel format object created above; ctx is a
        // valid out-pointer.
        unsafe {
            cgl::CGLCreateContext(pf, std::ptr::null_mut(), &mut ctx);
        }
        if ctx.is_null() {
            // SAFETY: balanced release of pf created above.
            unsafe { cgl::CGLReleasePixelFormat(pf) };
            return Err("CGLCreateContext 失败".to_string());
        }
        // SAFETY: ctx is current inside with_current_context (render.h
        // requirement); the render context is stored in LayerState and freed
        // before the player per MpvNativeView's lifetime contract.
        let render = with_current_context(ctx, || unsafe { RenderContext::new(player) });
        let render = match render {
            Ok(render) => render,
            Err(e) => {
                // SAFETY: balanced release of the objects created above.
                unsafe {
                    CGLReleaseContext(ctx);
                    cgl::CGLReleasePixelFormat(pf);
                }
                return Err(e.to_string());
            }
        };
        let state = Box::new(LayerState {
            render: Mutex::new(Some(render)),
            cgl_pf: pf,
            cgl_ctx: ctx,
        });
        let state_ptr = Box::into_raw(state);
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
            (*layer).set_ivar::<usize>("_rsState", state_ptr as usize);
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
        // SAFETY: state_ptr is a live Box<LayerState> (owned by the layer);
        // locking is uncontended here — the view was just attached and Drop
        // has not run.
        let mut guard = unsafe { &*state_ptr }
            .render
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(render) = guard.as_mut() {
            render.set_update_callback(move || {
                // mpv calls this from arbitrary threads; hop to the main
                // thread, where rsDriveUpdate answers update() and schedules
                // a draw. `layer` stays alive: the callback is unset (via
                // RenderContext::drop, in MpvNativeView::drop) before the
                // layer is released, and performSelectorOnMainThread retains
                // the receiver until delivery.
                let layer = layer_addr as *mut Object;
                // SAFETY: per the lifetime note above, layer is valid;
                // rsDriveUpdate takes no arguments and transfers nothing.
                unsafe {
                    let () = msg_send![
                        layer,
                        performSelectorOnMainThread: sel!(rsDriveUpdate)
                        withObject: std::ptr::null_mut::<Object>()
                        waitUntilDone: NO
                    ];
                }
            });
        }
        drop(guard);
        Ok(Self {
            view,
            layer,
            state: state_ptr,
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
        // SAFETY: self.view is a live NSView owned by us.
        unsafe {
            let () = msg_send![self.view, removeFromSuperview];
        }
        // Take the render context out under the lock: in-flight draws either
        // hold it (we wait them out) or will see None and skip. After this,
        // no draw can touch mpv again.
        //
        // SAFETY: self.state points at the layer-owned Box<LayerState>; the
        // layer is still alive (we release it below), so the pointee is valid.
        let render = {
            let state = unsafe { &*self.state };
            let mut guard = state.render.lock().unwrap_or_else(|e| e.into_inner());
            let render = guard.take();
            drop(guard);
            render
        };
        if let Some(render) = render {
            // Answer any still-pending update callback before free (harmless
            // if none), then free with the same CGL context current — both
            // render.h requirements.
            //
            // SAFETY: self.state is still valid (see above).
            let ctx = unsafe { &*self.state }.cgl_ctx;
            with_current_context(ctx, || {
                render.update();
                drop(render);
            });
        }
        // SAFETY: balanced release for the alloc/init retains. The layer's
        // dealloc reclaims the Box<LayerState> and the base CGL references;
        // the view also retained the layer via setLayer, keeping everything
        // valid until here.
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
