//! Manual smoke test for RenderContext (macOS, no GUI window required):
//! creates an offscreen CGL context, attaches an mpv render context before
//! loading the file, and prints state changes. The key signal: with a render
//! context attached, vo=libmpv selects the video track, so `has_video`
//! flips to true for video files (it was false in the Task 5 probe).
//! Usage: cargo run -p rust-reader-media --example probe_render -- <media-file>

#[cfg(target_os = "macos")]
fn main() {
    use rust_reader_media::player::MpvPlayer;
    use rust_reader_media::render::RenderContext;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_render <media-file>");

    // Offscreen GL: mpv_render_context_create requires a current GL context.
    // SAFETY: attrs is a valid 0-terminated attribute array; all out-pointers
    // are valid and outlive their calls.
    let ctx = unsafe {
        let attrs: [cgl::CGLPixelFormatAttribute; 4] = [
            cgl::kCGLPFAAccelerated,
            cgl::kCGLPFANoRecovery,
            cgl::kCGLPFADoubleBuffer,
            0,
        ];
        let mut pf: cgl::CGLPixelFormatObj = std::ptr::null_mut();
        let mut npix = 0;
        cgl::CGLChoosePixelFormat(attrs.as_ptr(), &mut pf, &mut npix);
        assert!(!pf.is_null(), "CGLChoosePixelFormat failed");
        let mut ctx: cgl::CGLContextObj = std::ptr::null_mut();
        cgl::CGLCreateContext(pf, std::ptr::null_mut(), &mut ctx);
        assert!(!ctx.is_null(), "CGLCreateContext failed");
        cgl::CGLSetCurrentContext(ctx);
        ctx
    };

    let updates = Arc::new(AtomicUsize::new(0));
    let updates2 = updates.clone();
    let player = MpvPlayer::new(Box::new(|| {})).expect("mpv init failed");
    // SAFETY: a current CGL context exists (above); `render` is dropped before
    // `player` at the end of main.
    let mut render = unsafe { RenderContext::new(&player) }.expect("render context failed");
    render.set_update_callback(move || {
        updates2.fetch_add(1, Ordering::SeqCst);
    });
    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");

    let state = player.state();
    let mut saw_video = false;
    for _ in 0..50 {
        // Advanced control requires update() on the render thread after each
        // update callback; poll it here (no layer to draw into offscreen).
        let flags = render.update();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let s = state.lock().unwrap();
        saw_video |= s.has_video;
        println!(
            "pos={}ms dur={:?} video={} tracks={} updates={} flags={:#x} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.has_video,
            s.tracks.len(),
            updates.load(Ordering::SeqCst),
            flags,
            s.error
        );
    }
    println!("probe_render done: has_video_seen={saw_video}");
    drop(render);
    drop(player);
    // SAFETY: ctx is the context created above and still current.
    unsafe {
        cgl::CGLSetCurrentContext(std::ptr::null_mut());
        cgl::CGLDestroyContext(ctx);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("probe_render is macOS-only");
}
