//! macOS mpv video layer: a CAOpenGLLayer inserted into the superlayer of
//! the winit view's CAMetalLayer, anchored BELOW it via
//! `insertSublayer:below:` (the view's layer IS wgpu's CAMetalLayer —
//! wgpu-hal adopts it as the main layer; verified by Task 1 layer dumps).
//! The egui surface is non-opaque (transparent backbuffer, see main.rs),
//! so the video shows through the unpainted central area and egui
//! menus/popups composite above the video. Coordinates are top-left
//! logical (make_frame passes them through; the superlayer is
//! geometryFlipped, confirmed empirically in Task 1). The OSD is a
//! CATextLayer sublayer of the video layer and is visible through the
//! same hole.
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
//! OSD: a CATextLayer sublayer of the video layer shows transient text
//! (volume, mute, seeks) at its top-right, composited below the egui
//! surface like the rest of the layer. CoreAnimation's implicit opacity
//! animation provides the fade, so `setOpacity` calls must stay outside
//! the disabled-actions transactions used for geometry changes.

// objc2's msg_send! picks the correct objc_msgSend variant (incl. stret)
// from the return type's Encode signature, so this compiles on both
// aarch64 and x86_64 — the old objc 0.2 code was aarch64-only.

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Ivar, Sel};
use objc2::{class, ffi, msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use openitgo_media::render::RenderContext;
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

/// `layer` is retained +1 by our alloc/init in `new` and additionally by the
/// superlayer while inserted (that retain is dropped by
/// `removeFromSuperlayer` in `Drop`). `osd_layer` is retained +1 by us and
/// by `layer` as its sublayer. `state` points at the layer-owned
/// `Box<LayerState>` (freed in the layer's `dealloc`).
pub struct MpvNativeView {
    layer: *mut AnyObject,
    osd_layer: *mut AnyObject,
    state: *mut LayerState,
}

// The raw CALayer pointers are only touched from the UI thread that
// owns this value; moving ownership between threads does not alias them.
unsafe impl Send for MpvNativeView {}

fn layer_class() -> &'static AnyClass {
    use std::sync::OnceLock;
    static CLS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLS.get_or_init(|| {
        let superclass = AnyClass::get(c"CAOpenGLLayer").expect("CAOpenGLLayer missing");
        let mut builder =
            ClassBuilder::new(c"OpenItGoMpvLayer", superclass).expect("failed to declare layer");
        builder.add_ivar::<usize>(c"_rsState");
        // SAFETY: each selector matches the CAOpenGLLayer delegate method
        // signature we register; the fn pointers use the C ABI and the types
        // are layout-compatible with the Objective-C declarations. The `_`
        // placeholders let inference pick a concrete receiver lifetime (the
        // fully-written `&AnyObject` form is "not general enough" for
        // MethodImplementation's HRTB).
        unsafe {
            builder.add_method(
                sel!(copyCGLPixelFormatForDisplayMask:),
                copy_pixel_format as extern "C" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(copyCGLContextForPixelFormat:),
                copy_context as extern "C" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(canDrawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                can_draw as extern "C" fn(_, _, _, _, _, _) -> _,
            );
            builder.add_method(
                sel!(drawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                draw_in as extern "C" fn(_, _, _, _, _, _),
            );
            builder.add_method(sel!(rsDriveUpdate), drive_update as extern "C" fn(_, _));
            builder.add_method(sel!(dealloc), dealloc as extern "C" fn(_, _));
        }
        builder.register()
    })
}

/// The `_rsState` ivar descriptor, cached alongside the class.
fn rs_state_ivar() -> &'static Ivar {
    use std::sync::OnceLock;
    static IVAR: OnceLock<&'static Ivar> = OnceLock::new();
    IVAR.get_or_init(|| {
        layer_class()
            .instance_variable(c"_rsState")
            .expect("_rsState ivar missing")
    })
}

/// Reads the `_rsState` ivar. Returns `None` only after `dealloc` has run
/// (which cannot race a live method call on the layer).
fn state_from_ivar(this: &AnyObject) -> Option<&LayerState> {
    // SAFETY: `this` is a OpenItGoMpvLayer instance; the ivar was declared
    // with usize layout on this exact class.
    let ptr: usize = unsafe { *rs_state_ivar().load::<usize>(this) };
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

extern "C" fn copy_pixel_format(this: &AnyObject, _sel: Sel, _mask: u32) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { cgl::CGLRetainPixelFormat(state.cgl_pf) },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn copy_context(this: &AnyObject, _sel: Sel, _pf: *mut c_void) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { CGLRetainContext(state.cgl_ctx) },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn can_draw(
    _this: &AnyObject,
    _sel: Sel,
    _ctx: *mut c_void,
    _pf: *mut c_void,
    _t: f64,
    _ts: *const c_void,
) -> Bool {
    Bool::YES
}

extern "C" fn draw_in(
    this: &AnyObject,
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
    let bounds: CGRect = unsafe { msg_send![this, bounds] };
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
extern "C" fn drive_update(this: &AnyObject, _sel: Sel) {
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

extern "C" fn dealloc(this: &AnyObject, _sel: Sel) {
    // SAFETY: `this` is a OpenItGoMpvLayer; the ivar was declared as usize.
    let ptr: usize = unsafe { *rs_state_ivar().load::<usize>(this) };
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
        let () = msg_send![super(this, superclass), dealloc];
    }
}

impl MpvNativeView {
    pub fn new<
        W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle,
    >(
        parent: &W,
        bounds: wry::Rect,
        player: &openitgo_media::MpvPlayer,
    ) -> Result<Self, String> {
        use wry::raw_window_handle::RawWindowHandle;
        let handle = parent
            .window_handle()
            .map_err(|e| format!("无法获取窗口句柄: {e:?}"))?;
        let ns_view = match handle.as_raw() {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut AnyObject,
            _ => return Err("媒体播放暂仅支持 macOS".to_string()),
        };
        // The winit view's layer IS wgpu's CAMetalLayer (wgpu-hal adopts it
        // as the view's main layer — Task 1 layer-dump verified), so "index 0
        // of the view layer" would put the video INSIDE the metal layer,
        // where sublayers always composite above the parent's content (Task
        // 1 round 1 failure). Anchor the video layer BELOW the metal layer
        // in its superlayer instead. Probe windows without wgpu have a plain
        // CALayer after setWantsLayer: fall back to index 0 there. These
        // checks run before any GL/layer/state allocation below so the Err
        // paths stay leak-free.
        //
        // SAFETY: ns_view is the parent window's live view, messaged on the
        // UI thread that owns it; selectors match the receivers' classes.
        let (metal_layer, parent_layer, is_metal) = unsafe {
            let () = msg_send![ns_view, setWantsLayer: Bool::YES];
            let view_layer: *mut AnyObject = msg_send![ns_view, layer];
            if view_layer.is_null() {
                return Err("父 view 尚无 layer（setWantsLayer 未生效），请重试".to_string());
            }
            let is_metal: bool = msg_send![view_layer, isKindOfClass: class!(CAMetalLayer)];
            let parent_layer: *mut AnyObject = if is_metal {
                let parent: *mut AnyObject = msg_send![view_layer, superlayer];
                if parent.is_null() {
                    return Err(
                        "winit view 尚未挂入窗口层树（superlayer 为 nil），请重试".to_string()
                    );
                }
                parent
            } else {
                // Below-egui anchoring needs the CAMetalLayer sibling; the
                // index-0 fallback (expected only in probe windows without
                // wgpu) stacks the video ABOVE the egui surface, so log it.
                let cls: *const i8 = msg_send![view_layer, className];
                let cls = std::ffi::CStr::from_ptr(cls).to_string_lossy();
                eprintln!(
                    "[mpv_view] view layer is {cls}, not CAMetalLayer; layer structure may \
                     have changed — using insertSublayer:atIndex:0 fallback (video will \
                     composite above egui)"
                );
                view_layer
            };
            (view_layer, parent_layer, is_metal)
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
        // (freshly allocated, or the layer-tree anchors validated above),
        // and selectors match the receivers' classes.
        let (layer, osd_layer) = unsafe {
            let layer: Allocated<AnyObject> = msg_send![layer_class(), alloc];
            let layer: Option<Retained<AnyObject>> = msg_send![layer, init];
            let layer = Retained::into_raw(layer.expect("OpenItGoMpvLayer init failed"));
            *rs_state_ivar().load_ptr::<usize>(&*layer) = state_ptr as usize;
            let () = msg_send![layer, setAsynchronous: Bool::YES];
            let () = msg_send![layer, setNeedsDisplayOnBoundsChange: Bool::YES];
            // Retina: match the window's backing scale.
            let window: *mut AnyObject = msg_send![ns_view, window];
            let scale: f64 = msg_send![window, backingScaleFactor];
            let () = msg_send![layer, setContentsScale: scale];
            // Bare layers animate geometry changes implicitly; the video
            // must track egui layout exactly, so every frame change goes
            // through a disabled-actions transaction (also in
            // set_bounds/set_osd). The OSD opacity fade lives on osd_layer
            // and is unaffected.
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: Bool::YES];
            let () = msg_send![layer, setFrame: make_frame(&bounds)];
            let () = if is_metal {
                msg_send![parent_layer, insertSublayer: layer, below: metal_layer]
            } else {
                msg_send![parent_layer, insertSublayer: layer, atIndex: 0u32]
            };
            let () = msg_send![class!(CATransaction), commit];
            // OSD text layer: a sublayer of the CAOpenGLLayer, hidden until
            // the first set_osd. Retained by us (+1), released in Drop.
            let osd_layer: Allocated<AnyObject> = msg_send![class!(CATextLayer), alloc];
            let osd_layer: Option<Retained<AnyObject>> = msg_send![osd_layer, init];
            let osd_layer = Retained::into_raw(osd_layer.expect("CATextLayer init failed"));
            let () = msg_send![osd_layer, setFontSize: 20.0f64];
            let fg: *mut AnyObject = msg_send![class!(NSColor), colorWithRed: 1.0f64, green: 1.0f64, blue: 1.0f64, alpha: 1.0f64];
            let fg_cg: *mut c_void = msg_send![fg, CGColor];
            let () = msg_send![osd_layer, setForegroundColor: fg_cg];
            let bg: *mut AnyObject = msg_send![class!(NSColor), colorWithRed: 0.0f64, green: 0.0f64, blue: 0.0f64, alpha: 0.6f64];
            let bg_cg: *mut c_void = msg_send![bg, CGColor];
            let () = msg_send![osd_layer, setBackgroundColor: bg_cg];
            let () = msg_send![osd_layer, setCornerRadius: 8.0f64];
            let () = msg_send![osd_layer, setContentsScale: scale];
            let () = msg_send![osd_layer, setOpacity: 0.0f32];
            let () = msg_send![layer, addSublayer: osd_layer];
            (layer, osd_layer)
        };
        let layer_addr = layer as usize;
        // SAFETY: state_ptr is a live Box<LayerState> (owned by the layer);
        // locking is uncontended here — the layer was just inserted and Drop
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
                let layer = layer_addr as *mut AnyObject;
                // SAFETY: per the lifetime note above, layer is valid;
                // rsDriveUpdate takes no arguments and transfers nothing.
                unsafe {
                    let () = msg_send![
                        layer,
                        performSelectorOnMainThread: sel!(rsDriveUpdate),
                        withObject: std::ptr::null_mut::<AnyObject>(),
                        waitUntilDone: Bool::NO
                    ];
                }
            });
        }
        drop(guard);
        Ok(Self {
            layer,
            osd_layer,
            state: state_ptr,
        })
    }

    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.layer/osd_layer are live objects owned by us. Bare
        // layers animate geometry changes implicitly, so every frame change
        // goes through a disabled-actions transaction (see new()).
        unsafe {
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: Bool::YES];
            let () = msg_send![self.layer, setFrame: make_frame(&bounds)];
            let lbounds: CGRect = msg_send![self.layer, bounds];
            let cur: CGRect = msg_send![self.osd_layer, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(lbounds.size.width, lbounds.size.height, cur.size.width, cur.size.height)
            ];
            let () = msg_send![class!(CATransaction), commit];
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
            let ns: Allocated<AnyObject> = msg_send![class!(NSString), alloc];
            let ns: Option<Retained<AnyObject>> = msg_send![ns, initWithUTF8String: ctext.as_ptr()];
            let ns = ns.expect("NSString initWithUTF8String failed");
            let () = msg_send![self.osd_layer, setString: &*ns];
            // `ns` drops (releases) here; the layer retains its string copy.
            let text: CGSize = msg_send![self.osd_layer, preferredFrameSize];
            let lbounds: CGRect = msg_send![self.layer, bounds];
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: Bool::YES];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(
                    lbounds.size.width,
                    lbounds.size.height,
                    text.width + 24.0,
                    text.height + 10.0,
                )
            ];
            let () = msg_send![class!(CATransaction), commit];
            // Outside the transaction: the opacity fade needs the implicit
            // animation that the transaction suppresses.
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
        // SAFETY: self.layer is a live CAOpenGLLayer owned by us; removing it
        // from the superlayer drops the superlayer's retain, leaving our
        // alloc/init retain (+1) balanced by the release below.
        unsafe {
            let () = msg_send![self.layer, removeFromSuperlayer];
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
        // video layer also retained it as a sublayer until the video layer's
        // dealloc below.
        unsafe {
            ffi::objc_release(self.osd_layer);
        }
        // SAFETY: balanced release for the alloc/init retain in new(). The
        // layer's dealloc reclaims the Box<LayerState> and the base CGL
        // references; the superlayer's retain was already dropped by the
        // removeFromSuperlayer above.
        unsafe {
            ffi::objc_release(self.layer);
        }
    }
}

fn make_frame(bounds: &wry::Rect) -> CGRect {
    use wry::dpi::{Position, Size};
    let (x, y) = match bounds.position {
        Position::Logical(p) => (p.x, p.y),
        Position::Physical(p) => (p.x as f64, p.y as f64),
    };
    let (w, h) = match bounds.size {
        Size::Logical(s) => (s.width, s.height),
        Size::Physical(s) => (s.width as f64, s.height as f64),
    };
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

/// Top-right anchor in the layer's bottom-left-origin coordinate space (the
/// bare CAOpenGLLayer has default, unflipped geometry — the reason the video
/// needs FLIP_Y). `text_w` is clamped so long device names cannot run off
/// the left edge.
fn osd_frame(view_w: f64, view_h: f64, text_w: f64, text_h: f64) -> CGRect {
    const MARGIN: f64 = 16.0;
    let w = text_w.min((view_w - 2.0 * MARGIN).max(0.0));
    let x = (view_w - w - MARGIN).max(MARGIN);
    let y = (view_h - text_h - MARGIN).max(MARGIN);
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, text_h))
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
