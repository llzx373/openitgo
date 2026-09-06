//! Windows video hosting: mpv renders into a child HWND via the `wid` option.
//!
//! Unlike macOS (CAOpenGLLayer anchored below the transparent egui surface),
//! an HWND child window always composites ABOVE the wgpu surface, so egui
//! menus/popups cannot overlap the video. `render_media` therefore parks the
//! window (zero-size bounds → hidden) while `menu_overlay_open`, mirroring
//! the ebook webview parking (#52).
//!
//! OSD is delegated to mpv's own `show-text` (the CATextLayer approach has no
//! HWND equivalent that stays inside the video); the egui painter fallback in
//! MediaView still covers the parked states.
//!
//! Construction is two-phase because mpv requires `wid` before
//! `mpv_initialize`: `PendingVideoView::create` makes the HWND, media.rs then
//! builds the player with `new_with_wid`, and `finish` wraps the HWND.

use std::ffi::CString;
use std::sync::OnceLock;
use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetWindowPos, ShowWindow,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_ERASEBKGND, WNDCLASSW, WS_CHILD,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Null-terminated UTF-16 "OpenItGoMpvVideo".
const VIDEO_CLASS: [u16; 17] = [
    0x4f, 0x70, 0x65, 0x6e, 0x49, 0x74, 0x47, 0x6f, 0x4d, 0x70, 0x76, 0x56, 0x69, 0x64, 0x65, 0x6f,
    0,
];

/// mpv paints the whole client area; suppressing the background erase avoids
/// a white flash while (re)positioning.
unsafe extern "system" fn video_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_ERASEBKGND {
        return 1;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Registers the video window class once. Errors are reported on the first
/// call; later calls reuse the stored result.
fn ensure_class() -> Result<(), String> {
    static RESULT: OnceLock<Result<(), String>> = OnceLock::new();
    RESULT.get_or_init(register_class).clone()
}

fn register_class() -> Result<(), String> {
    // SAFETY: no preconditions; returns the exe module handle.
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    if hinst.is_null() {
        return Err("GetModuleHandleW 失败".to_string());
    }
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(video_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: VIDEO_CLASS.as_ptr(),
    };
    // SAFETY: `class` is a valid stack struct whose name pointer is static.
    // A repeat registration fails with ERROR_CLASS_ALREADY_EXISTS, which is
    // fine — the class exists either way.
    unsafe { RegisterClassW(&class) };
    Ok(())
}

fn parent_hwnd<W: HasWindowHandle>(parent: &W) -> Result<HWND, String> {
    let handle = parent
        .window_handle()
        .map_err(|e| format!("无法获取窗口句柄: {e:?}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Ok(h.hwnd.get() as HWND),
        _ => Err("媒体播放暂仅支持 Win32 窗口".to_string()),
    }
}

/// Logical (egui points) → physical pixels for `SetWindowPos`.
fn dpi_scale(hwnd: HWND) -> f64 {
    // SAFETY: hwnd is a live window; GetDpiForWindow has no side effects.
    // Returns 0 for an invalid hwnd; fall back to no scaling.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

/// Holds the child HWND between creation and player construction.
pub struct PendingVideoView {
    hwnd: HWND,
}

impl PendingVideoView {
    pub fn create<W: HasWindowHandle>(parent: &W) -> Result<Self, String> {
        let parent = parent_hwnd(parent)?;
        ensure_class()?;
        // SAFETY: all pointers are valid/static; parent is a live window on
        // the UI thread. Created at 0x0; set_bounds positions it.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                VIDEO_CLASS.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_CLIPSIBLINGS | WS_VISIBLE,
                0,
                0,
                0,
                0,
                parent,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err("创建视频子窗口失败".to_string());
        }
        Ok(Self { hwnd })
    }

    /// mpv `wid` option value (must be set before `mpv_initialize`).
    pub fn wid(&self) -> Option<i64> {
        Some(self.hwnd as i64)
    }

    pub fn finish<W: HasWindowHandle>(
        self,
        _parent: &W,
        bounds: wry::Rect,
        player: &openitgo_media::MpvPlayer,
    ) -> Result<MpvNativeView, String> {
        let view = MpvNativeView {
            hwnd: self.hwnd,
            mpv: player.handle(),
        };
        view.set_bounds(bounds);
        Ok(view)
    }
}

pub struct MpvNativeView {
    hwnd: HWND,
    /// Borrowed from the MpvPlayer field that outlives this view (`OpenMedia`
    /// drops `native` before `player`).
    mpv: *mut libmpv_sys::mpv_handle,
}

// Created and used on the UI thread only; the raw handles are not shared.
unsafe impl Send for MpvNativeView {}

impl MpvNativeView {
    pub fn set_bounds(&self, bounds: wry::Rect) {
        // render_media always constructs Rect from logical (egui) sizes, so
        // any scale factor passes the logical variant through unchanged.
        let size: wry::dpi::LogicalSize<f64> = bounds.size.to_logical(1.0);
        let pos: wry::dpi::LogicalPosition<f64> = bounds.position.to_logical(1.0);
        if size.width <= 0.0 || size.height <= 0.0 {
            // Parked (audio-only / error overlay / menu parking): hide rather
            // than zero-size to avoid a stray 1px sliver.
            // SAFETY: hwnd is a live window owned by this view.
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            return;
        }
        let scale = dpi_scale(self.hwnd);
        let (x, y) = (
            (pos.x * scale).round() as i32,
            (pos.y * scale).round() as i32,
        );
        let (w, h) = (
            (size.width * scale).round() as i32,
            (size.height * scale).round() as i32,
        );
        // SAFETY: hwnd is a live window; SetWindowPos/ShowWindow are
        // single-threaded UI calls.
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x,
                y,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(self.hwnd, SW_SHOW);
        }
    }

    /// Delegates OSD rendering to mpv (`show-text`, 1s). Stays inside the
    /// video HWND where egui cannot paint.
    pub fn set_osd(&self, text: &str) {
        self.show_text(text, "1000");
    }

    pub fn clear_osd(&self) {
        self.show_text("", "1");
    }

    fn show_text(&self, text: &str, duration_ms: &str) {
        let cmd = CString::new("show-text").unwrap();
        let text = CString::new(text).unwrap_or_else(|_| CString::new("").unwrap());
        let dur = CString::new(duration_ms).unwrap();
        let mut args = [cmd.as_ptr(), text.as_ptr(), dur.as_ptr(), std::ptr::null()];
        // SAFETY: `mpv` is a live player handle that outlives this view;
        // args point to valid NUL-terminated strings that outlive the call.
        // Async per the UI-thread command rule (AGENTS.md).
        unsafe {
            libmpv_sys::mpv_command_async(self.mpv, 0, args.as_mut_ptr());
        }
    }
}

impl Drop for MpvNativeView {
    fn drop(&mut self) {
        // SAFETY: hwnd was created by PendingVideoView::create and is
        // destroyed exactly once here.
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
