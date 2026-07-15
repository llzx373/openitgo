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
//!
//! OSD: a CATextLayer sublayer of the CAOpenGLLayer shows transient text
//! (volume, mute, seeks) at the top-right of the video. egui cannot paint
//! over the native view (that is why overlays park it at zero size), so the
//! OSD lives in the native layer tree; CoreAnimation's implicit opacity
//! animation provides the fade.

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`;
// same allow as platform.rs's dock_open module.
#![allow(unexpected_cfgs)]

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
    // Declared in <OpenGL/gl.h>; used to learn which framebuffer
    // CoreAnimation bound for the current draw.
    fn glGetIntegerv(pname: u32, params: *mut i32);
}

/// From <OpenGL/gl.h>; CoreAnimation binds its own drawable FBO (typically
/// 1/2, alternating — not 0) before calling drawInCGLContext.
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

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
    osd_layer: *mut Object,
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
    // out by copy_context) current on its render thread, as render.h requires,
    // and bound its own drawable framebuffer — which is NOT FBO 0 (observed:
    // 1/2, alternating). mpv must target exactly that FBO or the layer's
    // drawable stays untouched and composites as fully transparent.
    //
    // SAFETY: `this` is a valid CALayer; `bounds`/`contentsScale` are plain
    // getters that don't transfer ownership. glGetIntegerv is safe with a
    // current context.
    let bounds: core_graphics::geometry::CGRect = unsafe { msg_send![this, bounds] };
    let scale: f64 = unsafe { msg_send![this, contentsScale] };
    let w = (bounds.size.width * scale) as i32;
    let h = (bounds.size.height * scale) as i32;
    let mut fbo: i32 = 0;
    unsafe { glGetIntegerv(GL_FRAMEBUFFER_BINDING, &mut fbo) };
    if w > 0 && h > 0 {
        render.render(fbo, w, h);
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
        let (view, layer, osd_layer) = unsafe {
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
            // OSD text layer: a sublayer of the CAOpenGLLayer, hidden until
            // the first set_osd. Retained by us (+1), released in Drop.
            let osd_layer: *mut Object = msg_send![class!(CATextLayer), alloc];
            let osd_layer: *mut Object = msg_send![osd_layer, init];
            let () = msg_send![osd_layer, setFontSize: 20.0f64];
            let fg: *mut Object = msg_send![class!(NSColor), colorWithRed: 1.0f64 green: 1.0f64 blue: 1.0f64 alpha: 1.0f64];
            let fg_cg: *mut c_void = msg_send![fg, CGColor];
            let () = msg_send![osd_layer, setForegroundColor: fg_cg];
            let bg: *mut Object = msg_send![class!(NSColor), colorWithRed: 0.0f64 green: 0.0f64 blue: 0.0f64 alpha: 0.6f64];
            let bg_cg: *mut c_void = msg_send![bg, CGColor];
            let () = msg_send![osd_layer, setBackgroundColor: bg_cg];
            let () = msg_send![osd_layer, setCornerRadius: 8.0f64];
            let () = msg_send![osd_layer, setContentsScale: scale];
            let () = msg_send![osd_layer, setOpacity: 0.0f32];
            let () = msg_send![layer, addSublayer: osd_layer];
            (view, layer, osd_layer)
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
            osd_layer,
            state: state_ptr,
        })
    }

    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.view/osd_layer are live objects owned by us.
        unsafe {
            let () = msg_send![self.view, setFrame: make_frame(&bounds)];
            let frame: core_graphics::geometry::CGRect = msg_send![self.view, frame];
            let cur: core_graphics::geometry::CGRect = msg_send![self.osd_layer, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(frame.size.width, frame.size.height, cur.size.width, cur.size.height)
            ];
        }
    }

    /// Shows `text` at the top-right of the video. The CATextLayer fades in
    /// via CoreAnimation's implicit opacity animation.
    pub fn set_osd(&self, text: &str) {
        let ctext = std::ffi::CString::new(text)
            .unwrap_or_else(|_| std::ffi::CString::new("").expect("empty CString"));
        // SAFETY: all objects are live instances owned by us, messaged on the
        // UI thread; selectors match the receivers' classes.
        unsafe {
            let ns: *mut Object = msg_send![class!(NSString), alloc];
            let ns: *mut Object = msg_send![ns, initWithUTF8String: ctext.as_ptr()];
            let () = msg_send![self.osd_layer, setString: ns];
            let () = msg_send![ns, release];
            let text: core_graphics::geometry::CGSize =
                msg_send![self.osd_layer, preferredFrameSize];
            let frame: core_graphics::geometry::CGRect = msg_send![self.view, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(
                    frame.size.width,
                    frame.size.height,
                    text.width + 24.0,
                    text.height + 10.0,
                )
            ];
            let () = msg_send![self.osd_layer, setOpacity: 1.0f32];
        }
    }

    /// Fades the OSD out (implicit animation); harmless when already hidden.
    pub fn clear_osd(&self) {
        // SAFETY: self.osd_layer is a live CATextLayer owned by us.
        unsafe {
            let () = msg_send![self.osd_layer, setOpacity: 0.0f32];
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
        // SAFETY: balanced release for the alloc/init retain in new(). The
        // superlayer also retains it until its own dealloc.
        unsafe {
            let () = msg_send![self.osd_layer, release];
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

/// Top-right anchor in the layer's bottom-left-origin coordinate space (our
/// NSView is not flipped — the reason the video needs FLIP_Y). `text_w` is
/// clamped so long device names cannot run off the left edge.
fn osd_frame(
    view_w: f64,
    view_h: f64,
    text_w: f64,
    text_h: f64,
) -> core_graphics::geometry::CGRect {
    const MARGIN: f64 = 16.0;
    let w = text_w.min((view_w - 2.0 * MARGIN).max(0.0));
    let x = (view_w - w - MARGIN).max(MARGIN);
    let y = (view_h - text_h - MARGIN).max(MARGIN);
    core_graphics::geometry::CGRect::new(
        &core_graphics::geometry::CGPoint::new(x, y),
        &core_graphics::geometry::CGSize::new(w, text_h),
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

    #[test]
    fn osd_frame_anchors_top_right() {
        let frame = osd_frame(800.0, 600.0, 120.0, 30.0);
        assert_eq!(frame.size.width, 120.0);
        assert_eq!(frame.size.height, 30.0);
        assert_eq!(frame.origin.x, 800.0 - 120.0 - 16.0);
        assert_eq!(frame.origin.y, 600.0 - 30.0 - 16.0);
    }

    #[test]
    fn osd_frame_clamps_oversized_text_to_view() {
        // Text wider than the view: clamp to view width minus margins and
        // pin the origin to the left margin so the OSD stays on screen.
        let frame = osd_frame(200.0, 100.0, 1000.0, 30.0);
        assert_eq!(frame.size.width, 200.0 - 2.0 * 16.0);
        assert_eq!(frame.origin.x, 16.0);
        assert_eq!(frame.origin.y, 100.0 - 30.0 - 16.0);
    }

    #[test]
    fn osd_frame_collapses_to_zero_width_on_tiny_view() {
        // Parked (0x0) native view: width clamps to 0, origin to the margin.
        let frame = osd_frame(0.0, 0.0, 100.0, 30.0);
        assert_eq!(frame.size.width, 0.0);
        assert_eq!(frame.origin.x, 16.0);
        assert_eq!(frame.origin.y, 16.0);
    }
}
