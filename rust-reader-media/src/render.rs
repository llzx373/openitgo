//! mpv OpenGL render context. Rendering happens inside a CAOpenGLLayer
//! (app side); this type only owns the mpv render context and its update
//! callback. Mirrors mpv's examples/libmpv/cocoa/cocoabasic.m.
//!
//! Threading rules (per libmpv render.h): the render context must be created
//! before the mpv handle is destroyed and freed before it; the update
//! callback may fire from arbitrary mpv threads and must not call any mpv
//! API — app code must hop to the render thread and call
//! `mpv_render_context_update`/`render` there.

use crate::error::MediaError;
use crate::player::MpvPlayer;
use libmpv_sys as mpv;
use std::ffi::c_void;

pub struct RenderContext {
    ctx: *mut mpv::mpv_render_context,
    // Owns the closure passed to mpv's update callback; the raw pointer given
    // to mpv aliases this box, so it must stay alive until the callback is
    // unset in Drop.
    update_cb: Option<Box<Box<dyn Fn() + Send + Sync>>>,
}

// The render context is only driven from the layer's draw callback (render
// thread) plus the update callback hop; ownership can move between threads.
unsafe impl Send for RenderContext {}

// SAFETY: called by libmpv with `name` being a valid NUL-terminated GL
// function name; dlsym on RTLD_DEFAULT is safe for any symbol name.
unsafe extern "C" fn get_proc_address(
    _ctx: *mut c_void,
    name: *const std::ffi::c_char,
) -> *mut c_void {
    libc::dlsym(libc::RTLD_DEFAULT, name)
}

extern "C" fn update_trampoline(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: `ctx` is the pointer we registered in `set_update_callback`,
    // pointing at a live `Box<dyn Fn() + Send + Sync>`; the callback is unset
    // before the box is dropped. libmpv forbids calling mpv APIs from this
    // callback — we only invoke user code that hops threads.
    let cb = unsafe { &*(ctx as *const Box<dyn Fn() + Send + Sync>) };
    cb();
}

impl RenderContext {
    /// Creates an mpv render context for the OpenGL backend.
    ///
    /// # Safety
    /// The caller must guarantee an OpenGL-capable environment
    /// (macOS CAOpenGLLayer with a current CGL context) and that the returned
    /// context is dropped before `player` is destroyed.
    pub unsafe fn new(player: &MpvPlayer) -> Result<Self, MediaError> {
        let mut init = mpv::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx: std::ptr::null_mut(),
            extra_exts: std::ptr::null(),
        };
        let api = c_api_type();
        let advanced: i32 = 1;
        let mut params = [
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_ADVANCED_CONTROL,
                data: &advanced as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let mut ctx: *mut mpv::mpv_render_context = std::ptr::null_mut();
        // SAFETY: player.handle() is a valid, initialized mpv handle; `params`
        // is a valid INVALID-terminated array whose data pointers stay valid
        // for the duration of the call (render.h only requires that).
        let rc = unsafe {
            mpv::mpv_render_context_create(&mut ctx, player.handle(), params.as_mut_ptr())
        };
        if rc < 0 || ctx.is_null() {
            return Err(MediaError::Init(format!(
                "mpv_render_context_create 失败: {rc}"
            )));
        }
        Ok(Self {
            ctx,
            update_cb: None,
        })
    }

    /// Registers `f` as the frame-available callback. mpv may invoke it from
    /// arbitrary threads; it must not call mpv APIs directly.
    pub fn set_update_callback<F: Fn() + Send + Sync + 'static>(&mut self, f: F) {
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(f));
        let ptr = Box::into_raw(boxed);
        // SAFETY: self.ctx is a valid render context; `ptr` stays valid until
        // the callback is reset (Drop) because we re-box it into `update_cb`
        // below.
        unsafe {
            mpv::mpv_render_context_set_update_callback(
                self.ctx,
                Some(update_trampoline),
                ptr as *mut c_void,
            );
        }
        // Reclaim the raw pointer into owned storage; dropping any previous
        // closure is safe because mpv no longer references it after the reset
        // above.
        self.update_cb = Some(unsafe { Box::from_raw(ptr) });
    }

    /// Must be called on the render thread after each update callback fired
    /// (a hard requirement because we create the context with
    /// MPV_RENDER_PARAM_ADVANCED_CONTROL=1; skipping it can stall the core).
    /// Returns the raw mpv_render_update_flag bitset.
    pub fn update(&self) -> u64 {
        // SAFETY: self.ctx is valid; the caller guarantees this runs on the
        // render thread with a current GL context and not inside the update
        // callback itself.
        unsafe { mpv::mpv_render_context_update(self.ctx) }
    }

    /// Renders into the currently bound framebuffer (CAOpenGLLayer FBO 0).
    /// Must be called on the render thread with the CGL context current.
    pub fn render(&self, width: i32, height: i32) {
        let mut fbo = mpv::mpv_opengl_fbo {
            fbo: 0,
            w: width,
            h: height,
            internal_format: 0,
        };
        let flip: i32 = 0; // CAOpenGLLayer is already upright.
        let block: i32 = 0; // Never block the layer's display callback.
        let mut params = [
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo as *mut _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: &flip as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: &block as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: self.ctx is valid; `params` is a valid INVALID-terminated
        // array living for the call; the caller guarantees a current GL
        // context on this thread.
        unsafe { mpv::mpv_render_context_render(self.ctx, params.as_mut_ptr()) };
    }

    /// Tells mpv a swap happened; call once after each `render` for timing.
    pub fn report_swap(&self) {
        // SAFETY: self.ctx is valid; report_swap is thread-safe with a
        // current context and ignored when no video is active.
        unsafe { mpv::mpv_render_context_report_swap(self.ctx) };
    }
}

fn c_api_type() -> *mut std::ffi::c_char {
    // Bindings expose this as `&'static [u8; 7]` (b"opengl\0"), not `&CStr`;
    // the cast drops const for the C API, which only reads it during create.
    mpv::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut std::ffi::c_char
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: self.ctx is a valid render context owned by us. Unsetting
        // the callback before free guarantees mpv cannot touch `update_cb`
        // afterwards; the context is freed before the player handle per the
        // caller's lifetime contract.
        unsafe {
            mpv::mpv_render_context_set_update_callback(self.ctx, None, std::ptr::null_mut());
            mpv::mpv_render_context_free(self.ctx);
        }
    }
}
