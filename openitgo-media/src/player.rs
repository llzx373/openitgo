//! libmpv handle lifecycle, command API, event pump and property observation.
//!
//! All unsafe FFI is contained in this file. State updates flow into a shared
//! `PlayerState`; every `time-pos`/`duration` change additionally fires the
//! repaint callback injected at construction time so the egui UI refreshes
//! immediately (same fix pattern as commit b071a7b).

use crate::apply::{self, AUDIO_DEVICES_REPLY_USERDATA, CHAPTER_LIST_REPLY_USERDATA};
use crate::args;
use crate::error::MediaError;
use crate::state::PlayerState;
use crate::tracks::RawTrack;
use libmpv_sys as mpv;
use std::ffi::CString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub struct MpvPlayer {
    handle: *mut mpv::mpv_handle,
    state: Arc<Mutex<PlayerState>>,
    /// Tells the event thread to leave its wait loop; set by Drop before
    /// joining the thread, so no `mpv_wait_event` call can race
    /// `mpv_terminate_destroy` freeing the handle (observed as intermittent
    /// SIGSEGV at address 0 inside libmpv during teardown).
    quit: Arc<AtomicBool>,
    event_thread: Option<JoinHandle<()>>,
}

// mpv handles are safe to command from any thread while the event loop owns
// waiting; libmpv documents concurrent command calls as safe.
unsafe impl Send for MpvPlayer {}
unsafe impl Sync for MpvPlayer {}

/// Send wrapper so the raw mpv handle can cross into the event thread.
struct SendHandle(*mut mpv::mpv_handle);

// SAFETY: libmpv documents that a handle may be waited on from a dedicated
// event thread while commands are issued concurrently from other threads.
unsafe impl Send for SendHandle {}

fn cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

impl MpvPlayer {
    pub fn new(repaint: Box<dyn Fn() + Send + Sync>) -> Result<Self, MediaError> {
        // SAFETY: mpv_create has no preconditions.
        let handle = unsafe { mpv::mpv_create() };
        if handle.is_null() {
            return Err(MediaError::Init("mpv_create 返回空句柄".to_string()));
        }
        for (k, v) in [
            ("vo", "libmpv"),
            ("keep-open", "yes"),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
        ] {
            let (k, v) = (cstring(k), cstring(v));
            // SAFETY: handle is a valid mpv handle; k/v are valid NUL-terminated
            // strings that outlive the call.
            let rc = unsafe { mpv::mpv_set_option_string(handle, k.as_ptr(), v.as_ptr()) };
            if rc < 0 {
                // SAFETY: handle is valid and owned by us; we abandon it on error.
                unsafe { mpv::mpv_terminate_destroy(handle) };
                return Err(MediaError::Init(format!("设置 {k:?} 失败: {rc}")));
            }
        }
        // SAFETY: handle is valid and not yet initialized.
        let rc = unsafe { mpv::mpv_initialize(handle) };
        if rc < 0 {
            // SAFETY: handle is valid and owned by us; we abandon it on error.
            unsafe { mpv::mpv_terminate_destroy(handle) };
            return Err(MediaError::Init(format!("mpv_initialize 失败: {rc}")));
        }
        // SAFETY: handle is initialized; all property names are valid
        // NUL-terminated static strings that outlive each call.
        unsafe {
            let level = if std::env::var_os("OPENITGO_MPV_LOG").is_some() {
                cstring("debug")
            } else {
                cstring("warn")
            };
            mpv::mpv_request_log_messages(handle, level.as_ptr());
            // Property observation: reply_userdata doubles as the property id.
            mpv::mpv_observe_property(
                handle,
                1,
                cstring("time-pos").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
            );
            mpv::mpv_observe_property(
                handle,
                2,
                cstring("duration").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
            );
            mpv::mpv_observe_property(
                handle,
                3,
                cstring("pause").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_FLAG,
            );
            mpv::mpv_observe_property(
                handle,
                4,
                cstring("volume").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
            );
            mpv::mpv_observe_property(
                handle,
                5,
                cstring("speed").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
            );
            mpv::mpv_observe_property(
                handle,
                6,
                cstring("track-list").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_NODE,
            );
            mpv::mpv_observe_property(
                handle,
                7,
                cstring("mute").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_FLAG,
            );
            mpv::mpv_observe_property(
                handle,
                8,
                cstring("sub-delay").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
            );
            mpv::mpv_observe_property(
                handle,
                9,
                cstring("chapter").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_INT64,
            );
        }
        // PlayerState::default() leaves volume/speed at 0.0; set sane initial
        // values matching mpv's own defaults (Task 4 review convention).
        let state = Arc::new(Mutex::new(PlayerState {
            volume: 100.0,
            speed: 1.0,
            ..Default::default()
        }));
        let mut player = Self {
            handle,
            state,
            quit: Arc::new(AtomicBool::new(false)),
            event_thread: None,
        };
        let event_thread = player.spawn_event_thread(repaint);
        player.event_thread = Some(event_thread);
        Ok(player)
    }

    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        self.state.clone()
    }

    pub(crate) fn handle(&self) -> *mut mpv::mpv_handle {
        self.handle
    }

    /// reply_userdata 0: the fire-and-forget reply event is ignored by
    /// event_loop. MUST stay async: a blocking mpv_command on the UI thread
    /// deadlocks against first-frame DR image allocation, which can only be
    /// serviced by the UI thread answering mpv_render_context_update()
    /// (docs/superpowers/reports/2026-07-17-bug-notes-archived.md 问题 A).
    fn command(&self, args: &[&str]) -> Result<(), MediaError> {
        let cargs: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut ptrs: Vec<*const std::ffi::c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: handle is valid; ptrs is a NULL-terminated array of valid
        // NUL-terminated strings. mpv copies the args while queueing, so they
        // only need to outlive this call.
        let rc = unsafe { mpv::mpv_command_async(self.handle, 0, ptrs.as_mut_ptr()) };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: args.join(" "),
            });
        }
        Ok(())
    }

    fn set_property_string(&self, name: &str, value: &str) -> Result<(), MediaError> {
        let (n, v) = (cstring(name), cstring(value));
        let mut vptr = v.as_ptr() as *mut std::ffi::c_char;
        // SAFETY: handle is valid; n/v are valid NUL-terminated strings. For
        // MPV_FORMAT_STRING, data points to a char*; client.h documents the
        // value is copied before the call returns. Async for the same
        // deadlock reason as command() — never block the UI thread on the
        // core dispatch queue.
        let rc = unsafe {
            mpv::mpv_set_property_async(
                self.handle,
                0,
                n.as_ptr(),
                mpv::mpv_format_MPV_FORMAT_STRING,
                &mut vptr as *mut _ as *mut std::ffi::c_void,
            )
        };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: format!("{name}={value}"),
            });
        }
        Ok(())
    }

    pub fn load_file(&self, path: &Path) -> Result<(), MediaError> {
        let s = path
            .to_str()
            .ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
        self.command(&["loadfile", s])
    }

    /// Stops playback and unloads the file; the vo core leaves its render
    /// wait, which lets a following mpv_render_context_free return promptly.
    pub fn stop(&self) -> Result<(), MediaError> {
        self.command(&["stop"])
    }

    pub fn cycle_pause(&self) -> Result<(), MediaError> {
        self.command(&["cycle", "pause"])
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), MediaError> {
        self.set_property_string("pause", args::yes_no(paused))
    }

    pub fn seek_rel_sec(&self, secs: f64) -> Result<(), MediaError> {
        self.command(&["seek", &format!("{secs}")])
    }

    /// Absolute seek. `exact` requests mpv's precise seek (decodes from the
    /// previous keyframe to the exact frame); without it mpv snaps to the
    /// nearest keyframe, which keeps interactive slider drags responsive.
    pub fn seek_abs_ms(&self, ms: u64, exact: bool) -> Result<(), MediaError> {
        let argv = args::seek_abs_args(ms, exact);
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.command(&refs)
    }

    pub fn set_volume(&self, volume: f64) -> Result<(), MediaError> {
        self.set_property_string("volume", &args::format_volume_arg(volume))
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), MediaError> {
        self.set_property_string("speed", &args::format_speed_arg(speed))
    }

    pub fn set_sub_track(&self, id: Option<i64>) -> Result<(), MediaError> {
        self.set_property_string("sid", &args::sid_arg(id))
    }

    /// Loads an external subtitle file and selects it immediately (`select`
    /// flag), so the new track shows without a manual sid switch.
    pub fn sub_add(&self, path: &Path) -> Result<(), MediaError> {
        let s = path
            .to_str()
            .ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
        self.command(&["sub-add", s, "select"])
    }

    /// Adjusts `sub-delay` by `delta` seconds (mpv `add` command).
    pub fn adjust_sub_delay(&self, delta: f64) -> Result<(), MediaError> {
        self.command(&["add", "sub-delay", &format!("{delta}")])
    }

    /// Resets `sub-delay` to 0 (the OSD/UI mirrors this from the observed
    /// property a frame later).
    pub fn reset_sub_delay(&self) -> Result<(), MediaError> {
        self.set_property_string("sub-delay", "0.0")
    }

    pub fn set_audio_track(&self, id: i64) -> Result<(), MediaError> {
        self.set_property_string("aid", &id.to_string())
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), MediaError> {
        self.set_property_string("mute", args::yes_no(muted))
    }

    /// Fires an async `audio-device-list` query; the reply is parsed on the
    /// event thread into `PlayerState::audio_devices` (None until the first
    /// reply lands). Async like every other call here — a blocking
    /// mpv_get_property on the UI thread deadlocks against first-frame DR
    /// image allocation (see command(),
    /// docs/superpowers/reports/2026-07-17-bug-notes-archived.md 问题 A).
    pub fn request_audio_devices(&self) -> Result<(), MediaError> {
        let name = cstring("audio-device-list");
        // SAFETY: handle is valid; name is a valid NUL-terminated string that
        // outlives the call. The reply arrives as MPV_EVENT_GET_PROPERTY_REPLY
        // carrying AUDIO_DEVICES_REPLY_USERDATA.
        let rc = unsafe {
            mpv::mpv_get_property_async(
                self.handle,
                AUDIO_DEVICES_REPLY_USERDATA,
                name.as_ptr(),
                mpv::mpv_format_MPV_FORMAT_NODE,
            )
        };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: "get audio-device-list".to_string(),
            });
        }
        Ok(())
    }

    /// `name` is an entry of `audio-device-list`; "auto" follows the system.
    pub fn set_audio_device(&self, name: &str) -> Result<(), MediaError> {
        self.set_property_string("audio-device", name)
    }

    /// `inf` loops the current file forever; `no` restores normal EOF
    /// behavior (used by the auto-next feature).
    pub fn set_loop_file(&self, enabled: bool) -> Result<(), MediaError> {
        self.set_property_string("loop-file", args::loop_file_arg(enabled))
    }

    /// Saves a screenshot of the current video frame; mpv picks the encoder
    /// from the file extension.
    pub fn screenshot_to_file(&self, path: &Path) -> Result<(), MediaError> {
        let s = path
            .to_str()
            .ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
        self.command(&["screenshot-to-file", s])
    }

    /// AB loop point A in seconds; `None` clears it (mpv `no`).
    pub fn set_ab_loop_a(&self, secs: Option<f64>) -> Result<(), MediaError> {
        self.set_property_string("ab-loop-a", &args::ab_loop_arg(secs))
    }

    /// AB loop point B in seconds; `None` clears it (mpv `no`).
    pub fn set_ab_loop_b(&self, secs: Option<f64>) -> Result<(), MediaError> {
        self.set_property_string("ab-loop-b", &args::ab_loop_arg(secs))
    }

    /// Relative chapter jump; mpv clamps at the first/last chapter.
    pub fn add_chapter(&self, delta: i64) -> Result<(), MediaError> {
        self.command(&["add", "chapter", &delta.to_string()])
    }

    /// Fires an async `chapter-list` query; the reply is parsed on the event
    /// thread into `PlayerState::chapters` (same pattern as
    /// `request_audio_devices`, distinct userdata).
    pub fn request_chapter_list(&self) -> Result<(), MediaError> {
        let name = cstring("chapter-list");
        // SAFETY: handle is valid; name is a valid NUL-terminated string that
        // outlives the call. The reply arrives as MPV_EVENT_GET_PROPERTY_REPLY
        // carrying CHAPTER_LIST_REPLY_USERDATA.
        let rc = unsafe {
            mpv::mpv_get_property_async(
                self.handle,
                CHAPTER_LIST_REPLY_USERDATA,
                name.as_ptr(),
                mpv::mpv_format_MPV_FORMAT_NODE,
            )
        };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: "get chapter-list".to_string(),
            });
        }
        Ok(())
    }

    fn spawn_event_thread(&self, repaint: Box<dyn Fn() + Send + Sync>) -> JoinHandle<()> {
        let handle = SendHandle(self.handle);
        let state = self.state.clone();
        let quit = self.quit.clone();
        std::thread::Builder::new()
            .name("mpv-events".to_string())
            .spawn(move || event_loop(handle, state, quit, repaint))
            .expect("failed to spawn mpv event thread")
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // Stop the event thread BEFORE destroying the handle:
        // mpv_terminate_destroy frees the client context, and a wait_event
        // call that races the free segfaults inside libmpv (observed:
        // intermittent SIGSEGV at address 0 on the mpv-events thread during
        // teardown). The quit flag plus the 50ms wait timeout guarantee the
        // thread leaves its loop; join makes the handle access over-with.
        self.quit.store(true, Ordering::Release);
        if let Some(thread) = self.event_thread.take() {
            let _ = thread.join();
        }
        // SAFETY: handle is valid and owned by us, and no other thread can
        // touch it now that the event thread is joined and the render
        // context was freed before the player (lifetime contract).
        unsafe { mpv::mpv_terminate_destroy(self.handle()) };
    }
}

fn event_loop(
    handle: SendHandle,
    state: Arc<Mutex<PlayerState>>,
    quit: Arc<AtomicBool>,
    repaint: Box<dyn Fn() + Send + Sync>,
) {
    let handle = handle.0;
    loop {
        if quit.load(Ordering::Acquire) {
            break;
        }
        // SAFETY: handle is valid for the lifetime of this loop — Drop sets
        // `quit` and joins this thread before calling mpv_terminate_destroy.
        // The 50ms timeout (instead of -1) lets the loop notice `quit`
        // while no mpv events arrive; expiry returns MPV_EVENT_NONE, which
        // the match below ignores.
        let event = unsafe { mpv::mpv_wait_event(handle, 0.05) };
        if event.is_null() {
            break;
        }
        // SAFETY: event is a valid pointer returned by mpv_wait_event.
        let event_id = unsafe { (*event).event_id };
        match event_id {
            mpv::mpv_event_id_MPV_EVENT_SHUTDOWN => break,
            mpv::mpv_event_id_MPV_EVENT_LOG_MESSAGE => {
                // Temporary diagnostics for the render-context free hang.
                if std::env::var_os("OPENITGO_MPV_LOG").is_some() {
                    // SAFETY: for MPV_EVENT_LOG_MESSAGE, data points to a
                    // valid mpv_event_log_message owned by mpv.
                    unsafe {
                        let m = (*event).data as *mut mpv::mpv_event_log_message;
                        if !m.is_null() {
                            let text = (*m).text;
                            if !text.is_null() {
                                eprint!(
                                    "[mpv] {}",
                                    std::ffi::CStr::from_ptr(text).to_string_lossy()
                                );
                            }
                        }
                    }
                }
            }
            mpv::mpv_event_id_MPV_EVENT_FILE_LOADED => {
                if let Ok(mut s) = state.lock() {
                    apply::apply_file_loaded(&mut s);
                }
                // Kick the async chapter-list query for the new file; the
                // reply lands as MPV_EVENT_GET_PROPERTY_REPLY with
                // CHAPTER_LIST_REPLY_USERDATA.
                let name = cstring("chapter-list");
                // SAFETY: handle is valid for the lifetime of this loop; name
                // is a valid NUL-terminated string that outlives the call.
                unsafe {
                    mpv::mpv_get_property_async(
                        handle,
                        CHAPTER_LIST_REPLY_USERDATA,
                        name.as_ptr(),
                        mpv::mpv_format_MPV_FORMAT_NODE,
                    );
                }
            }
            mpv::mpv_event_id_MPV_EVENT_END_FILE => {
                // SAFETY: for MPV_EVENT_END_FILE, event data points to a valid
                // mpv_event_end_file owned by mpv for the duration of the event.
                let reason = unsafe {
                    let data = (*event).data as *mut mpv::mpv_event_end_file;
                    if data.is_null() {
                        0
                    } else {
                        (*data).reason
                    }
                };
                let is_error = reason as u32 == mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR;
                if let Ok(mut s) = state.lock() {
                    apply::apply_end_file(&mut s, is_error);
                }
                repaint();
            }
            mpv::mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY => {
                // SAFETY: reply_userdata is a plain value field on the event.
                let userdata = unsafe { (*event).reply_userdata };
                match apply::classify_reply(userdata) {
                    apply::ReplyKind::AudioDevices => {
                        // SAFETY: for MPV_EVENT_GET_PROPERTY_REPLY, event data
                        // points to a valid mpv_event_property owned by mpv.
                        let prop = unsafe { (*event).data as *mut mpv::mpv_event_property };
                        if !prop.is_null() {
                            let (format, data) = unsafe { ((*prop).format, (*prop).data) };
                            if format == mpv::mpv_format_MPV_FORMAT_NODE && !data.is_null() {
                                // SAFETY: data points to a valid mpv_node owned by
                                // mpv for the duration of the event; we only read.
                                let raw =
                                    unsafe { read_audio_device_list(data as *mut mpv::mpv_node) };
                                if let Ok(mut s) = state.lock() {
                                    s.audio_devices =
                                        Some(crate::devices::parse_audio_devices(raw));
                                }
                                repaint();
                            }
                        }
                    }
                    apply::ReplyKind::ChapterList => {
                        // SAFETY: for MPV_EVENT_GET_PROPERTY_REPLY, event data
                        // points to a valid mpv_event_property owned by mpv.
                        let prop = unsafe { (*event).data as *mut mpv::mpv_event_property };
                        if !prop.is_null() {
                            let (format, data) = unsafe { ((*prop).format, (*prop).data) };
                            if format == mpv::mpv_format_MPV_FORMAT_NODE && !data.is_null() {
                                // SAFETY: data points to a valid mpv_node owned by
                                // mpv for the duration of the event; we only read.
                                let raw = unsafe { read_chapter_list(data as *mut mpv::mpv_node) };
                                if let Ok(mut s) = state.lock() {
                                    s.chapters = crate::chapters::parse_chapters(raw);
                                }
                                repaint();
                            }
                        }
                    }
                    apply::ReplyKind::Other => {}
                }
            }
            mpv::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                // SAFETY: event is valid; reply_userdata is a plain value field
                // on the event itself (not on mpv_event_property).
                let userdata = unsafe { (*event).reply_userdata };
                // SAFETY: for MPV_EVENT_PROPERTY_CHANGE, event data points to a
                // valid mpv_event_property owned by mpv for the duration of the
                // event.
                let (format, data) = unsafe {
                    let prop = (*event).data as *mut mpv::mpv_event_property;
                    if prop.is_null() {
                        continue;
                    }
                    ((*prop).format, (*prop).data)
                };
                let mut should_repaint = false;
                let mut kick_chapter_list = false;
                if let Ok(mut s) = state.lock() {
                    match userdata {
                        1 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                // SAFETY: format guarantees data points to an f64.
                                let secs = unsafe { *(data as *mut f64) };
                                apply::apply_time_pos(&mut s, secs);
                                should_repaint = true;
                            }
                        }
                        2 => {
                            let secs_opt =
                                if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                    // SAFETY: format guarantees data points to an f64.
                                    Some(unsafe { *(data as *mut f64) })
                                } else {
                                    None
                                };
                            apply::apply_duration(&mut s, secs_opt);
                            should_repaint = true;
                        }
                        3 => {
                            if format == mpv::mpv_format_MPV_FORMAT_FLAG && !data.is_null() {
                                // SAFETY: format guarantees data points to a c_int.
                                s.paused = unsafe { *(data as *mut i32) } != 0;
                                should_repaint = true;
                            }
                        }
                        4 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                // SAFETY: format guarantees data points to an f64.
                                s.volume = unsafe { *(data as *mut f64) };
                                should_repaint = true;
                            }
                        }
                        5 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                // SAFETY: format guarantees data points to an f64.
                                s.speed = unsafe { *(data as *mut f64) };
                                should_repaint = true;
                            }
                        }
                        6 => {
                            if format == mpv::mpv_format_MPV_FORMAT_NODE && !data.is_null() {
                                // SAFETY: format guarantees data points to a valid
                                // mpv_node owned by mpv for the duration of the
                                // event; we only read, never free it.
                                let raw = unsafe { read_track_list(data as *mut mpv::mpv_node) };
                                // Task 4 review convention: parsed tracks and raw
                                // tracks passed to has_real_video must come from
                                // this single track-list read (enforced inside
                                // apply_track_list).
                                apply::apply_track_list(&mut s, raw);
                                should_repaint = true;
                            }
                        }
                        7 => {
                            if format == mpv::mpv_format_MPV_FORMAT_FLAG && !data.is_null() {
                                // SAFETY: format guarantees data points to a c_int.
                                s.muted = unsafe { *(data as *mut i32) } != 0;
                                should_repaint = true;
                            }
                        }
                        8 if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() => {
                            // SAFETY: format guarantees data points to an f64.
                            s.sub_delay = unsafe { *(data as *mut f64) };
                            should_repaint = true;
                        }
                        9 if format == mpv::mpv_format_MPV_FORMAT_INT64 && !data.is_null() => {
                            // SAFETY: format guarantees data points to an i64.
                            s.chapter = Some(unsafe { *(data as *mut i64) });
                            should_repaint = true;
                            kick_chapter_list = true;
                        }
                        _ => {}
                    }
                }
                if kick_chapter_list {
                    // chapter 变化时刷新 chapter-list（spec：保持标题列表新鲜）。
                    let name = cstring("chapter-list");
                    // SAFETY: handle is valid for the lifetime of this loop;
                    // name is a valid NUL-terminated string outliving the call.
                    unsafe {
                        mpv::mpv_get_property_async(
                            handle,
                            CHAPTER_LIST_REPLY_USERDATA,
                            name.as_ptr(),
                            mpv::mpv_format_MPV_FORMAT_NODE,
                        );
                    }
                }
                if should_repaint {
                    repaint();
                }
            }
            _ => {}
        }
    }
}

/// Walks an mpv NODE (track-list: list of maps) into FFI-free RawTrack records.
/// # Safety: `node` must point to a valid mpv_node owned by the mpv event.
unsafe fn read_track_list(node: *mut mpv::mpv_node) -> Vec<RawTrack> {
    let mut out = Vec::new();
    // SAFETY: per caller, node points to a valid mpv_node owned by mpv.
    if node.is_null() || unsafe { (*node).format } != mpv::mpv_format_MPV_FORMAT_NODE_ARRAY {
        return out;
    }
    // SAFETY: format is NODE_ARRAY, so the list union member is valid.
    let list = unsafe { (*node).u.list };
    if list.is_null() {
        return out;
    }
    // SAFETY: list points to a valid mpv_node_list owned by mpv; values has
    // num entries.
    let (num, values) = unsafe { ((*list).num, (*list).values) };
    for i in 0..num {
        // SAFETY: i < num, so values[i] is a valid mpv_node.
        let entry = unsafe { values.offset(i as isize) };
        // SAFETY: entry points to a valid mpv_node owned by mpv.
        if entry.is_null() || unsafe { (*entry).format } != mpv::mpv_format_MPV_FORMAT_NODE_MAP {
            continue;
        }
        // SAFETY: format is NODE_MAP, so the list union member is valid.
        let map = unsafe { (*entry).u.list };
        if map.is_null() {
            continue;
        }
        // SAFETY: map points to a valid mpv_node_list; keys and values each
        // have num entries.
        let (mnum, mvalues, mkeys) = unsafe { ((*map).num, (*map).values, (*map).keys) };
        let mut t = RawTrack {
            id: 0,
            kind: String::new(),
            selected: false,
            albumart: false,
            title: None,
            lang: None,
            codec: None,
        };
        for j in 0..mnum {
            // SAFETY: j < mnum, so keys[j] is a valid NUL-terminated string
            // owned by mpv.
            let key = unsafe {
                let k = mkeys.offset(j as isize);
                if k.is_null() || (*k).is_null() {
                    continue;
                }
                std::ffi::CStr::from_ptr(*k).to_string_lossy().into_owned()
            };
            // SAFETY: j < mnum, so values[j] is a valid mpv_node.
            let v = unsafe { mvalues.offset(j as isize) };
            if v.is_null() {
                continue;
            }
            // SAFETY: v points to a valid mpv_node owned by mpv.
            let fmt = unsafe { (*v).format };
            match key.as_str() {
                "id" if fmt == mpv::mpv_format_MPV_FORMAT_INT64 => {
                    // SAFETY: format guarantees the int64 union member is valid.
                    t.id = unsafe { (*v).u.int64 };
                }
                "type" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    // SAFETY: format guarantees v is a STRING node.
                    t.kind = unsafe { node_string(v) };
                }
                "selected" if fmt == mpv::mpv_format_MPV_FORMAT_FLAG => {
                    // SAFETY: format guarantees the flag union member is valid.
                    t.selected = unsafe { (*v).u.flag } != 0;
                }
                "albumart" if fmt == mpv::mpv_format_MPV_FORMAT_FLAG => {
                    // SAFETY: format guarantees the flag union member is valid.
                    t.albumart = unsafe { (*v).u.flag } != 0;
                }
                "title" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    // SAFETY: format guarantees v is a STRING node.
                    t.title = Some(unsafe { node_string(v) });
                }
                "lang" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    // SAFETY: format guarantees v is a STRING node.
                    t.lang = Some(unsafe { node_string(v) });
                }
                "codec" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    // SAFETY: format guarantees v is a STRING node.
                    t.codec = Some(unsafe { node_string(v) });
                }
                _ => {}
            }
        }
        if !t.kind.is_empty() {
            out.push(t);
        }
    }
    out
}

/// Walks an audio-device-list NODE (list of maps with string `name` /
/// `description` keys) into FFI-free records.
/// # Safety: `node` must point to a valid mpv_node owned by the caller.
unsafe fn read_audio_device_list(node: *mut mpv::mpv_node) -> Vec<crate::devices::RawAudioDevice> {
    let mut out = Vec::new();
    // SAFETY: per caller, node points to a valid mpv_node owned by us.
    if node.is_null() || unsafe { (*node).format } != mpv::mpv_format_MPV_FORMAT_NODE_ARRAY {
        return out;
    }
    // SAFETY: format is NODE_ARRAY, so the list union member is valid.
    let list = unsafe { (*node).u.list };
    if list.is_null() {
        return out;
    }
    // SAFETY: values has num entries.
    let (num, values) = unsafe { ((*list).num, (*list).values) };
    for i in 0..num {
        // SAFETY: i < num, so values[i] is a valid mpv_node.
        let entry = unsafe { values.offset(i as isize) };
        if entry.is_null() || unsafe { (*entry).format } != mpv::mpv_format_MPV_FORMAT_NODE_MAP {
            continue;
        }
        // SAFETY: format is NODE_MAP, so the list union member is valid.
        let map = unsafe { (*entry).u.list };
        if map.is_null() {
            continue;
        }
        // SAFETY: keys and values each have num entries.
        let (mnum, mvalues, mkeys) = unsafe { ((*map).num, (*map).values, (*map).keys) };
        let mut device = crate::devices::RawAudioDevice {
            name: String::new(),
            description: None,
        };
        for j in 0..mnum {
            // SAFETY: j < mnum, so keys[j] is a valid NUL-terminated string
            // owned by the node.
            let key = unsafe {
                let k = mkeys.offset(j as isize);
                if k.is_null() || (*k).is_null() {
                    continue;
                }
                std::ffi::CStr::from_ptr(*k).to_string_lossy().into_owned()
            };
            // SAFETY: j < mnum, so values[j] is a valid mpv_node.
            let v = unsafe { mvalues.offset(j as isize) };
            if v.is_null() || unsafe { (*v).format } != mpv::mpv_format_MPV_FORMAT_STRING {
                continue;
            }
            match key.as_str() {
                // SAFETY: format checked STRING above; node_string copies it.
                "name" => device.name = unsafe { node_string(v) },
                "description" => device.description = Some(unsafe { node_string(v) }),
                _ => {}
            }
        }
        if !device.name.is_empty() {
            out.push(device);
        }
    }
    out
}

/// # Safety: `v` must be a valid mpv_node with format MPV_FORMAT_STRING.
unsafe fn node_string(v: *mut mpv::mpv_node) -> String {
    // SAFETY: per caller, the string union member is valid for a STRING node.
    let p = unsafe { (*v).u.string };
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: p is a valid NUL-terminated string owned by mpv; we copy it.
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Walks a chapter-list NODE (list of maps with an optional string `title`
/// key) into FFI-free records.
/// # Safety: `node` must point to a valid mpv_node owned by the caller.
unsafe fn read_chapter_list(node: *mut mpv::mpv_node) -> Vec<crate::chapters::RawChapter> {
    let mut out = Vec::new();
    // SAFETY: per caller, node points to a valid mpv_node owned by us.
    if node.is_null() || unsafe { (*node).format } != mpv::mpv_format_MPV_FORMAT_NODE_ARRAY {
        return out;
    }
    // SAFETY: format is NODE_ARRAY, so the list union member is valid.
    let list = unsafe { (*node).u.list };
    if list.is_null() {
        return out;
    }
    // SAFETY: list points to a valid mpv_node_list owned by mpv; values has
    // num entries.
    let (num, values) = unsafe { ((*list).num, (*list).values) };
    for i in 0..num {
        // SAFETY: i < num, so values[i] is a valid mpv_node.
        let entry = unsafe { values.offset(i as isize) };
        // SAFETY: entry points to a valid mpv_node owned by mpv.
        if entry.is_null() || unsafe { (*entry).format } != mpv::mpv_format_MPV_FORMAT_NODE_MAP {
            continue;
        }
        // SAFETY: format is NODE_MAP, so the list union member is valid.
        let map = unsafe { (*entry).u.list };
        if map.is_null() {
            continue;
        }
        // SAFETY: map points to a valid mpv_node_list; keys and values each
        // have num entries.
        let (mnum, mvalues, mkeys) = unsafe { ((*map).num, (*map).values, (*map).keys) };
        let mut chapter = crate::chapters::RawChapter { title: None };
        for j in 0..mnum {
            // SAFETY: j < mnum, so keys[j] is a valid NUL-terminated string
            // owned by the node.
            let key = unsafe {
                let k = mkeys.offset(j as isize);
                if k.is_null() || (*k).is_null() {
                    continue;
                }
                std::ffi::CStr::from_ptr(*k).to_string_lossy().into_owned()
            };
            // SAFETY: j < mnum, so values[j] is a valid mpv_node.
            let v = unsafe { mvalues.offset(j as isize) };
            if v.is_null() || unsafe { (*v).format } != mpv::mpv_format_MPV_FORMAT_STRING {
                continue;
            }
            if key == "title" {
                // SAFETY: format checked STRING above; node_string copies it.
                chapter.title = Some(unsafe { node_string(v) });
            }
        }
        out.push(chapter);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstring_passes_through_normal_strings() {
        assert_eq!(cstring("hello").to_str().unwrap(), "hello");
    }

    #[test]
    fn cstring_falls_back_to_empty_on_interior_nul() {
        assert_eq!(cstring("a\0b").to_str().unwrap(), "");
    }
}
