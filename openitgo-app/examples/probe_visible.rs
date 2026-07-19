//! Visible-window diagnostic probe: unlike probe_mpv_view (offscreen), this
//! shows a real on-screen window with the mpv overlay attached, so the actual
//! CoreAnimation draw + compositing path can be verified with a screenshot.
//! Usage: cargo run -p openitgo-app --example probe_visible -- <video-file> [seconds]

#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
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
        .expect("usage: probe_visible <video-file> [seconds]");
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    // Visible window + content view.
    // SAFETY: all messages go to valid AppKit objects on the main thread;
    // selectors match the receivers' classes.
    let (content_view, _window) = unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let () = msg_send![app, setActivationPolicy: 0i64]; // NSApplicationActivationPolicyRegular
        let rect = CGRect::new(CGPoint::new(200.0, 200.0), CGSize::new(640.0, 480.0));
        let window: Allocated<AnyObject> = msg_send![class!(NSWindow), alloc];
        let style: usize = (1 << 0) | (1 << 1) | (1 << 2); // titled|closable|miniaturizable
        let window: Option<Retained<AnyObject>> = msg_send![window,
            initWithContentRect: rect,
            styleMask: style,
            backing: 2usize, // NSBackingStoreBuffered
            defer: Bool::NO
        ];
        let window = Retained::into_raw(window.expect("NSWindow init failed"));
        let title: Allocated<AnyObject> = msg_send![class!(NSString), alloc];
        let title: Option<Retained<AnyObject>> =
            msg_send![title, initWithUTF8String: c"OPENITGO_PROBE_VISIBLE".as_ptr()];
        let title = title.expect("NSString init failed");
        let () = msg_send![window, setTitle: &*title];
        let () = msg_send![window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
        let () = msg_send![app, activateIgnoringOtherApps: Bool::YES];
        let content_view: *mut AnyObject = msg_send![window, contentView];
        (content_view, window)
    };
    assert!(!content_view.is_null(), "NSWindow contentView is null");

    struct Parent(*mut AnyObject);
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

    let player = MpvPlayer::new(Box::new(|| {})).expect("mpv init failed");
    let bounds = wry::Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(640.0, 480.0).into(),
    };
    let view = MpvNativeView::new(&parent, bounds, &player).expect("MpvNativeView::new failed");
    println!("visible probe: view created, loading {path}");

    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");
    let state = player.state();
    let iterations = seconds * 2;
    for _ in 0..iterations {
        // Pump the main runloop for ~500ms so CA commits and draws happen.
        // SAFETY: runUntilDate: on the main run loop from the main thread.
        unsafe {
            let run_loop: *mut AnyObject = msg_send![class!(NSRunLoop), mainRunLoop];
            let date: *mut AnyObject =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.5f64];
            let () = msg_send![run_loop, runUntilDate: date];
        }
        let s = state.lock().unwrap();
        println!(
            "pos={}ms dur={:?} video={} tracks={} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.has_video,
            s.tracks.len(),
            s.error
        );
    }
    println!("visible probe done");
    drop(view);
    drop(player);
    println!("teardown clean");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("probe_visible is macOS-only");
}
