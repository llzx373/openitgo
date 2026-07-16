//! macOS-only probe for MpvNativeView, simulating the real app environment:
//! the creating thread (like the wgpu/Metal UI thread) has NO current CGL
//! context. Verifies that view + mpv render context creation still succeeds
//! (a pre-fix version segfaulted inside mpv_render_context_create here) and
//! that `has_video` flips to true once a video file loads.
//! Usage: cargo run -p openitgo-app --example probe_mpv_view -- <video-file>

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`.
#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
fn main() {
    use objc::runtime::{Object, NO};
    use objc::{class, msg_send, sel, sel_impl};
    use openitgo_app::platform::macos::mpv_view::MpvNativeView;
    use openitgo_media::MpvPlayer;
    use std::ffi::c_void;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::raw_window_handle::{
        AppKitDisplayHandle, AppKitWindowHandle, DisplayHandle, HandleError, RawDisplayHandle,
        RawWindowHandle, WindowHandle,
    };

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_mpv_view <video-file>");

    // Offscreen window + content view; no event loop is needed for creation.
    // SAFETY: all messages go to valid AppKit objects on the main thread;
    // selectors match the receivers' classes.
    let (content_view, _window) = unsafe {
        let _app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let rect = core_graphics::geometry::CGRect::new(
            &core_graphics::geometry::CGPoint::new(0.0, 0.0),
            &core_graphics::geometry::CGSize::new(640.0, 480.0),
        );
        let window: *mut Object = msg_send![class!(NSWindow), alloc];
        let style: usize = 1 << 1; // NSWindowStyleMaskClosable
        let window: *mut Object = msg_send![window,
            initWithContentRect: rect
            styleMask: style
            backing: 2usize // NSBackingStoreBuffered
            defer: NO
        ];
        let content_view: *mut Object = msg_send![window, contentView];
        // No orderFront: the window stays offscreen on purpose — the
        // rsDriveUpdate selector answers mpv's update callbacks on the main
        // run loop independently of CoreAnimation draws, so playback health
        // must not depend on visibility.
        (content_view, window)
    };
    assert!(!content_view.is_null(), "NSWindow contentView is null");

    struct Parent(*mut Object);
    impl wry::raw_window_handle::HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let view = std::ptr::NonNull::new(self.0 as *mut c_void)
                .expect("content view pointer is non-null");
            // SAFETY: the handle borrows a live NSView that outlives it.
            Ok(unsafe {
                WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(view)))
            })
        }
    }
    impl wry::raw_window_handle::HasDisplayHandle for Parent {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            // SAFETY: the AppKit display handle carries no pointers.
            Ok(unsafe {
                DisplayHandle::borrow_raw(RawDisplayHandle::AppKit(AppKitDisplayHandle::new()))
            })
        }
    }
    let parent = Parent(content_view);

    // The crux: this thread must have no current CGL context, exactly like
    // the app's wgpu/Metal UI thread.
    // SAFETY: plain getter, no preconditions.
    assert!(
        unsafe { cgl::CGLGetCurrentContext() }.is_null(),
        "probe must start with no current CGL context"
    );

    let player = MpvPlayer::new(Box::new(|| {})).expect("mpv init failed");
    let bounds = wry::Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(640.0, 480.0).into(),
    };
    let view = MpvNativeView::new(&parent, bounds, &player).expect("MpvNativeView::new failed");
    println!("MpvNativeView created with no pre-set current CGL context");
    // SAFETY: plain getter, no preconditions.
    assert!(
        unsafe { cgl::CGLGetCurrentContext() }.is_null(),
        "current CGL context must be restored after creation"
    );

    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");
    let state = player.state();
    let mut saw_video = false;
    for _ in 0..50 {
        // Pump the main runloop for ~100ms so the queued setNeedsDisplay
        // messages and CA commits run and the layer actually draws (each
        // draw calls mpv update()/render() on the CA render thread).
        // SAFETY: runUntilDate: on the main run loop from the main thread;
        // NSDate factory returns an autoreleased object.
        unsafe {
            let run_loop: *mut Object = msg_send![class!(NSRunLoop), mainRunLoop];
            let date: *mut Object = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.1f64];
            let () = msg_send![run_loop, runUntilDate: date];
        }
        let s = state.lock().unwrap();
        saw_video |= s.has_video;
        println!(
            "pos={}ms dur={:?} video={} tracks={} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.has_video,
            s.tracks.len(),
            s.error
        );
    }
    println!("probe_mpv_view done: has_video_seen={saw_video}");
    // Teardown order: view (frees the render context) before player.
    drop(view);
    drop(player);
    println!("teardown clean");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("probe_mpv_view is macOS-only");
}
