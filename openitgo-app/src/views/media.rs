use openitgo_media::tracks::{TrackInfo, TrackKind};
use openitgo_media::{MpvPlayer, PlayerState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OSD_DURATION: Duration = Duration::from_millis(1000);

struct Osd {
    text: String,
    until: Instant,
}

#[derive(Default)]
pub struct MediaView {
    pub open: Option<OpenMedia>,
    osd: Option<Osd>,
    pub scroll_acc: f32,
    /// Set once when the deferred startup device apply finds the saved
    /// device missing; read via take_startup_device_invalid.
    startup_device_invalid: bool,
    /// Guard so "auto-play next episode" fires at most once per opened
    /// media; reset in open().
    pub auto_next_fired: bool,
    /// OSD text to show once the next open succeeds (e.g. the auto-next
    /// notice); consumed by open(). It must wait for the new media: shown
    /// earlier it would paint on the old native view, which the swap destroys.
    pub pending_open_osd: Option<String>,
    /// 循环播放开关（mpv `loop-file` inf/no）；会话状态，不持久化。
    pub loop_file: bool,
    /// AB 循环状态机；随 open() 复位。
    pub ab_loop: AbLoop,
}

pub struct OpenMedia {
    pub path: PathBuf,
    pub title: String,
    // Field order matters for drop: `native` frees the mpv render context,
    // which render.h requires to happen before the player handle is
    // destroyed (`player` below). Struct fields drop in declaration order.
    native: crate::platform::macos::mpv_view::MpvNativeView,
    pub player: MpvPlayer,
    pub state: Arc<Mutex<PlayerState>>,
    pub last: PlayerState,
    pub pending_resume_ms: Option<u64>,
    /// Saved audio device to validate+apply once the async device-list
    /// reply lands in `last.audio_devices` (see sync_state).
    pending_startup_device: Option<String>,
}

impl MediaView {
    pub fn open(
        &mut self,
        ctx: &egui::Context,
        parent: &(impl wry::raw_window_handle::HasWindowHandle
              + wry::raw_window_handle::HasDisplayHandle),
        bounds: wry::Rect,
        path: PathBuf,
        resume_ms: Option<u64>,
    ) -> Result<(), String> {
        // Auto-next one-shot state resets/is consumed with every open; take
        // the pending OSD up front so a failed open cannot leak it into a
        // later, unrelated open.
        self.auto_next_fired = false;
        self.loop_file = false;
        self.ab_loop = AbLoop::None;
        let pending_osd = self.pending_open_osd.take();
        let ctx2 = ctx.clone();
        let player =
            MpvPlayer::new(Box::new(move || ctx2.request_repaint())).map_err(|e| e.to_string())?;
        let native = crate::platform::macos::mpv_view::MpvNativeView::new(parent, bounds, &player)?;
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("未知媒体")
            .to_string();
        player.load_file(&path).map_err(|e| e.to_string())?;
        let state = player.state();
        self.open = Some(OpenMedia {
            path,
            title,
            native,
            player,
            state,
            last: PlayerState::default(),
            pending_resume_ms: resume_ms,
            pending_startup_device: None,
        });
        if let Some(text) = pending_osd {
            self.show_osd(ctx, text);
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = None; // Drops native view, render context and player.
                          // The native view (and its CATextLayer OSD) is gone; drop the egui-side
                          // OSD state too, or the stale text leaks into the next opened media.
        self.osd = None;
    }

    pub fn update_bounds(&mut self, bounds: wry::Rect) {
        if let Some(open) = self.open.as_ref() {
            open.native.set_bounds(bounds);
        }
    }

    /// Copies the latest player state into `last` for UI reads; applies a
    /// pending resume once the duration is known, and the deferred startup
    /// audio device once the async device-list reply has landed.
    pub fn sync_state(&mut self) {
        let mut device_invalid = false;
        if let Some(open) = self.open.as_mut() {
            if let Ok(s) = open.state.lock() {
                open.last = s.clone();
            }
            if let Some(ms) = open.pending_resume_ms {
                if let Some(dur) = open.last.duration_ms {
                    if should_resume(ms, dur) {
                        let _ = open.player.seek_abs_ms(ms, true);
                    }
                    open.pending_resume_ms = None;
                }
            }
            if open.pending_startup_device.is_some() && open.last.audio_devices.is_some() {
                let saved = open.pending_startup_device.take().unwrap_or_default();
                let enumerated = open.last.audio_devices.clone().unwrap_or_default();
                let (target, saved_ok) = startup_device_target(&saved, &enumerated);
                if let Some(target) = target {
                    let _ = open.player.set_audio_device(target);
                }
                device_invalid = !saved_ok;
            }
        }
        if device_invalid {
            self.startup_device_invalid = true;
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.allocate_space(ui.available_size());
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let overlay = media_overlay(&open.last);
        let text = match &overlay {
            // The native view covers the panel and its CATextLayer shows the
            // OSD; nothing to paint here.
            MediaOverlay::None => return,
            // Decode error or audio-only file: the native view is parked at
            // zero size (see render_media), so paint the layer here.
            MediaOverlay::Error(msg) => msg.clone(),
            MediaOverlay::AudioOnly => open.title.clone(),
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(24.0),
            egui::Color32::WHITE,
        );
        // The parked native view's CATextLayer OSD is invisible (zero-size
        // frame), so mirror the active OSD text here with matching styling:
        // 20pt white text on 60% black, 8px radius, 16px top-right margin.
        if let Some(osd) = &self.osd {
            let font = egui::FontId::proportional(20.0);
            let galley = painter.layout_no_wrap(osd.text.clone(), font, egui::Color32::WHITE);
            let padding = egui::vec2(12.0, 5.0);
            let size = galley.size() + padding * 2.0;
            let bg = egui::Rect::from_min_size(
                egui::pos2(rect.max.x - 16.0 - size.x, rect.min.y + 16.0),
                size,
            );
            painter.rect_filled(bg, 8.0, egui::Color32::from_black_alpha(153));
            painter.galley(bg.min + padding, galley, egui::Color32::WHITE);
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.cycle_pause();
        }
    }

    pub fn seek_rel(&mut self, secs: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.seek_rel_sec(secs);
        }
    }

    pub fn seek_to_ratio(&mut self, ratio: f64) {
        if let Some(open) = self.open.as_ref() {
            if let Some(dur) = open.last.duration_ms {
                let target = clamp_seek((dur as f64 * ratio.clamp(0.0, 1.0)) as i64, dur);
                // Interactive slider drags fire this every frame, so use mpv's
                // keyframe-aligned seek; resume uses an exact seek instead.
                let _ = open.player.seek_abs_ms(target, false);
            }
        }
    }

    /// Exact seek for when the slider drag ends (release); interactive drags
    /// use the keyframe-aligned `seek_to_ratio` instead.
    pub fn seek_to_ratio_exact(&mut self, ratio: f64) {
        if let Some(open) = self.open.as_ref() {
            if let Some(dur) = open.last.duration_ms {
                let target = clamp_seek((dur as f64 * ratio.clamp(0.0, 1.0)) as i64, dur);
                let _ = open.player.seek_abs_ms(target, true);
            }
        }
    }

    /// Returns the new muted state for OSD text; None when nothing is open.
    pub fn toggle_mute(&mut self) -> Option<bool> {
        let open = self.open.as_ref()?;
        let muted = !open.last.muted;
        let _ = open.player.set_muted(muted);
        Some(muted)
    }

    pub fn set_volume(&mut self, v: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_volume(v);
        }
    }

    /// Returns the applied (clamped) target volume for persistence/OSD.
    pub fn adjust_volume(&mut self, delta: f64) -> Option<f64> {
        let open = self.open.as_ref()?;
        let target = (open.last.volume + delta).clamp(0.0, 100.0);
        let _ = open.player.set_volume(target);
        Some(target)
    }

    /// Returns the applied target speed for persistence/OSD.
    pub fn cycle_speed(&mut self) -> Option<f64> {
        let open = self.open.as_ref()?;
        let target = next_speed(open.last.speed);
        let _ = open.player.set_speed(target);
        Some(target)
    }

    pub fn set_speed(&mut self, speed: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_speed(speed);
        }
    }

    pub fn set_sub(&mut self, id: Option<i64>) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_sub_track(id);
        }
    }

    /// Loads an external subtitle file; mpv selects it immediately and the
    /// track-list observation picks it up automatically.
    pub fn sub_add(&mut self, path: &std::path::Path) -> Result<(), openitgo_media::MediaError> {
        let Some(open) = self.open.as_ref() else {
            return Ok(());
        };
        open.player.sub_add(path)
    }

    /// Returns the new sub-delay for the OSD, computed as
    /// `last.sub_delay + delta` (mpv's property reply re-syncs a frame later,
    /// so a one-frame error is acceptable).
    pub fn adjust_sub_delay(&mut self, delta: f64) -> Option<f64> {
        let open = self.open.as_ref()?;
        let _ = open.player.adjust_sub_delay(delta);
        Some(open.last.sub_delay + delta)
    }

    pub fn reset_sub_delay(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.reset_sub_delay();
        }
    }

    pub fn cycle_sub(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let subs: Vec<i64> = open
                .last
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Sub)
                .map(|t| t.id)
                .collect();
            if subs.is_empty() {
                return;
            }
            // Order: current -> next sub -> ... -> off -> first sub
            let next = match open.last.current_sub {
                None => Some(subs[0]),
                Some(cur) => {
                    let pos = subs.iter().position(|id| *id == cur);
                    match pos {
                        Some(i) if i + 1 < subs.len() => Some(subs[i + 1]),
                        _ => None,
                    }
                }
            };
            let _ = open.player.set_sub_track(next);
        }
    }

    pub fn set_audio(&mut self, id: i64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_audio_track(id);
        }
    }

    /// Fires the async device enumeration; the reply lands in
    /// `PlayerState::audio_devices` a few frames later.
    pub fn refresh_audio_devices(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.request_audio_devices();
        }
    }

    /// Applies persisted preferences after a successful open. Volume/speed
    /// are set (async) right away; the audio device can only be validated
    /// against the async enumeration once its reply lands, so it is deferred
    /// to `sync_state` via `pending_startup_device`. If the saved device
    /// turns out to be missing, "auto" is applied and
    /// `take_startup_device_invalid` reports it once.
    pub fn apply_startup_settings(&mut self, volume: f64, speed: f64, audio_device: &str) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let _ = open.player.set_volume(volume);
        let _ = open.player.set_speed(speed);
        if !audio_device.is_empty() {
            open.pending_startup_device = Some(audio_device.to_string());
        }
    }

    /// One-shot report: true once after the saved startup audio device was
    /// found missing and "auto" was applied instead (caller clears the
    /// persisted setting).
    pub fn take_startup_device_invalid(&mut self) -> bool {
        std::mem::take(&mut self.startup_device_invalid)
    }

    /// Empty `name` selects "auto" (follow the system default device).
    pub fn set_audio_device(&mut self, name: &str) -> Result<(), openitgo_media::MediaError> {
        let Some(open) = self.open.as_ref() else {
            return Ok(());
        };
        let name = if name.is_empty() { "auto" } else { name };
        open.player.set_audio_device(name)
    }

    /// 倍速微调 ±0.25；返回应用后的倍速供持久化/OSD。
    pub fn adjust_speed(&mut self, delta: f64) -> Option<f64> {
        let open = self.open.as_ref()?;
        let target = adjust_speed_value(open.last.speed, delta);
        let _ = open.player.set_speed(target);
        Some(target)
    }

    /// 切换循环播放；返回新状态供 OSD。
    pub fn toggle_loop_file(&mut self) -> Option<bool> {
        let open = self.open.as_ref()?;
        let next = !self.loop_file;
        let _ = open.player.set_loop_file(next);
        self.loop_file = next;
        Some(next)
    }

    /// 推进 AB 循环状态机并同步 mpv 点位；返回 OSD 文本。
    pub fn advance_ab_loop(&mut self) -> Option<String> {
        let open = self.open.as_ref()?;
        let next = ab_loop_advance(self.ab_loop, open.last.position_ms);
        match next {
            AbLoop::None => {
                let _ = open.player.set_ab_loop_a(None);
                let _ = open.player.set_ab_loop_b(None);
            }
            AbLoop::ASet(a) => {
                let _ = open.player.set_ab_loop_a(Some(a));
                let _ = open.player.set_ab_loop_b(None);
            }
            AbLoop::Both(_, b) => {
                let _ = open.player.set_ab_loop_b(Some(b));
            }
        }
        self.ab_loop = next;
        Some(ab_loop_osd_text(next))
    }

    /// 上一章/下一章；无章节返回 None（调用方禁用入口）。
    /// 返回乐观预计的章节标题供 OSD（mpv 会在边界处钳制）。
    pub fn chapter_step(&mut self, delta: i64) -> Option<String> {
        let open = self.open.as_ref()?;
        if open.last.chapters.is_empty() {
            return None;
        }
        let _ = open.player.add_chapter(delta);
        let target = open
            .last
            .chapter
            .map(|c| c + delta)
            .filter(|&t| t >= 0 && (t as usize) < open.last.chapters.len());
        Some(match target {
            Some(t) => open.last.chapters[t as usize].clone(),
            None => {
                if delta > 0 {
                    "下一章".to_string()
                } else {
                    "上一章".to_string()
                }
            }
        })
    }

    /// 截图到图片目录；返回完整路径。mpv 编码/写盘失败经事件泵
    /// error 反馈，本调用只保证命令已发出与目录存在。
    pub fn take_screenshot(&mut self) -> Result<PathBuf, String> {
        let Some(open) = self.open.as_ref() else {
            return Err("未打开媒体".to_string());
        };
        let dir = screenshot_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建截图目录: {e}"))?;
        let path = dir.join(screenshot_filename(
            &open.title,
            time::OffsetDateTime::now_utc(),
        ));
        open.player
            .screenshot_to_file(&path)
            .map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Shows an OSD message for ~1s. On video the CATextLayer sublayer fades
    /// in/out via CoreAnimation's implicit opacity animation; when the native
    /// view is parked (audio-only / error overlay), `ui` paints the stored
    /// text instead. We only track the text and expiry here.
    pub fn show_osd(&mut self, ctx: &egui::Context, text: String) {
        if let Some(open) = self.open.as_ref() {
            open.native.set_osd(&text);
            self.osd = Some(Osd {
                text,
                until: Instant::now() + OSD_DURATION,
            });
            ctx.request_repaint_after(OSD_DURATION);
        }
    }

    /// Hides an expired OSD; called once per frame from render_media.
    ///
    /// egui fires the frame scheduled by `request_repaint_after` slightly
    /// EARLY (it subtracts the predicted frame time) and only once, so an
    /// unexpired OSD must re-arm the timer here — otherwise an idle app
    /// (e.g. after EOF, when the mpv event pump stops repainting) never gets
    /// another frame and the OSD lingers until the next user input.
    pub fn tick_osd(&mut self, ctx: &egui::Context) {
        let Some(osd) = &self.osd else {
            return;
        };
        let now = Instant::now();
        if now >= osd.until {
            if let Some(open) = self.open.as_ref() {
                open.native.clear_osd();
            }
            self.osd = None;
        } else {
            ctx.request_repaint_after(osd.until.saturating_duration_since(now));
        }
    }
}

/// What the egui central panel paints over the native mpv view. Whenever
/// this is not `None`, render_media parks the native view at zero size so
/// the layer painted by `MediaView::ui` stays visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOverlay {
    /// Normal video playback: the native view covers the panel.
    None,
    /// Audio-only file: show the title placeholder.
    AudioOnly,
    /// Unrecoverable decode error: show the message instead of a black frame.
    Error(String),
}

pub fn media_overlay(last: &PlayerState) -> MediaOverlay {
    if let Some(err) = &last.error {
        MediaOverlay::Error(err.clone())
    } else if !last.has_video {
        MediaOverlay::AudioOnly
    } else {
        MediaOverlay::None
    }
}

/// Only resume when the saved position is not within the last 3 seconds,
/// so "reopen at the end" does not flash-finish the file.
pub fn should_resume(position_ms: u64, duration_ms: u64) -> bool {
    position_ms > 0 && position_ms + 3000 < duration_ms
}

/// Cycles 0.5 -> 1 -> 1.5 -> 2 -> 0.5; an unknown speed restarts the cycle.
pub fn next_speed(current: f64) -> f64 {
    const OPTIONS: [f64; 4] = [0.5, 1.0, 1.5, 2.0];
    for (i, s) in OPTIONS.iter().enumerate() {
        if (current - s).abs() < 0.01 {
            return OPTIONS[(i + 1) % OPTIONS.len()];
        }
    }
    OPTIONS[0]
}

/// Clamps a seek target into `[0, duration_ms]`.
pub fn clamp_seek(position_ms: i64, duration_ms: u64) -> u64 {
    position_ms.clamp(0, duration_ms as i64) as u64
}

/// Maps a pointer x position on the seek bar to a time for the hover
/// tooltip; None when the duration is unknown or the bar has no width.
pub fn hover_time_at(
    pointer_x: f32,
    bar_rect: egui::Rect,
    duration_ms: Option<u64>,
) -> Option<u64> {
    let dur = duration_ms?;
    if bar_rect.width() <= 0.0 {
        return None;
    }
    let ratio = ((pointer_x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
    Some((dur as f64 * ratio as f64) as u64)
}

/// Picks the audio device to apply at startup plus whether the saved setting
/// is still valid. `None` means "leave the player on its current device".
/// An empty enumeration is treated as transient (driver busy, list not ready
/// yet): nothing is applied and the saved device is reported valid so the
/// caller does not clear it.
pub fn startup_device_target<'a>(
    saved: &'a str,
    enumerated: &[openitgo_media::AudioDevice],
) -> (Option<&'a str>, bool) {
    if saved.is_empty() || enumerated.is_empty() {
        return (None, true);
    }
    if enumerated.iter().any(|d| d.name == saved) {
        (Some(saved), true)
    } else {
        (Some("auto"), false)
    }
}

/// Caps a toolbar label at `max_chars` characters, appending "…" when
/// truncated, so long device names cannot blow out the toolbar width.
/// Counts chars (not bytes) so CJK labels truncate safely.
pub fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn volume_osd_text(volume: f64) -> String {
    format!("音量 {:.0}%", volume)
}

pub fn speed_osd_text(speed: f64) -> String {
    format!("{speed:.1}x")
}

pub fn mute_osd_text(muted: bool) -> &'static str {
    if muted {
        "静音"
    } else {
        "取消静音"
    }
}

/// 字幕延迟 OSD 数值：`0.0` / `+0.1` / `-0.2`，保留一位小数；零不带符号。
pub fn format_sub_delay(v: f64) -> String {
    if v == 0.0 {
        "0.0".to_string()
    } else {
        format!("{v:+.1}")
    }
}

/// 倍速微调步进（[ / ] 键与菜单项）：±0.25，clamp 到 mpv 允许的 0.1–16，
/// 并去掉浮点累加尾差。
pub fn adjust_speed_value(current: f64, delta: f64) -> f64 {
    (((current + delta) * 100.0).round() / 100.0).clamp(0.1, 16.0)
}

/// 微调倍速 OSD：`倍速 1.25x`（两位小数内去掉尾随零）。
pub fn speed_fine_osd_text(speed: f64) -> String {
    let rounded = (speed * 100.0).round() / 100.0;
    let mut s = format!("{rounded:.2}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    format!("倍速 {s}x")
}

/// AB 循环状态机（mpv `ab-loop-a`/`ab-loop-b` 属性，秒）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AbLoop {
    #[default]
    None,
    ASet(f64),
    Both(f64, f64),
}

/// 按一次 A 键/菜单项的状态迁移：`position_ms` 为当前播放位置。
/// 已设 A 且当前位置在 A 之前时重设 A（mpv 不允许 B < A）。
pub fn ab_loop_advance(current: AbLoop, position_ms: u64) -> AbLoop {
    let secs = position_ms as f64 / 1000.0;
    match current {
        AbLoop::None => AbLoop::ASet(secs),
        AbLoop::ASet(a) => {
            if secs > a {
                AbLoop::Both(a, secs)
            } else {
                AbLoop::ASet(secs)
            }
        }
        AbLoop::Both(..) => AbLoop::None,
    }
}

/// AB 循环 OSD 文本。一小时内分钟补零到两位（`mm:ss`，与进度条的 `m:ss`
/// 格式刻意不同）；更长的时间回退媒体的 `h:mm:ss`。
pub fn ab_loop_osd_text(state: AbLoop) -> String {
    // 一小时内分钟补零到两位（`mm:ss`）；更长的时间回退媒体的 `h:mm:ss`。
    let fmt = |secs: f64| {
        let ms = (secs * 1000.0) as u64;
        if ms >= 3_600_000 {
            openitgo_media::time::format_time_ms(ms)
        } else {
            let total_secs = ms / 1000;
            format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
        }
    };
    match state {
        AbLoop::None => "已取消 AB 循环".to_string(),
        AbLoop::ASet(a) => format!("A 点 {}", fmt(a)),
        AbLoop::Both(a, b) => format!("AB 循环 {} - {}", fmt(a), fmt(b)),
    }
}

/// 截图保存目录：系统图片目录下 `OpenItGo/`；取不到时退回配置目录的
/// `screenshots/`（与 JsonStore 同根）。
pub fn screenshot_dir() -> PathBuf {
    if let Some(pictures) = dirs::picture_dir() {
        return pictures.join("OpenItGo");
    }
    openitgo_storage::json_store::JsonStore::default_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("screenshots")
}

/// 截图文件名 `<标题>-<yyyyMMdd-HHmmss>.png`：非文件名字符替换为 `_`，
/// 最长 40 字符。时间戳用 UTC——time 的 `local-offset` feature 未启用，
/// 避免多线程下的 unsoundness；文件名只需可读且基本唯一。
pub fn screenshot_filename(title: &str, dt: time::OffsetDateTime) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!(
        "{}-{:04}{:02}{:02}-{:02}{:02}{:02}.png",
        safe,
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// Toolbar/dropdown label for a track: `#2 简中 [zh]`, or `#1 轨道 3`
/// when the track carries no title.
pub fn track_label(t: &TrackInfo, index: usize) -> String {
    let base = t.title.clone().unwrap_or_else(|| format!("轨道 {}", t.id));
    match &t.lang {
        Some(lang) => format!("#{} {} [{}]", index + 1, base, lang),
        None => format!("#{} {}", index + 1, base),
    }
}

/// Pixels of scroll delta that make one 5% volume step.
pub const SCROLL_VOLUME_STEP_PX: f32 = 25.0;

/// Accumulates scroll delta into volume steps. Returns the new accumulator
/// and how many 5% steps to apply (sign = direction). The remainder is kept,
/// so reverse scrolling cancels partial steps and smooth trackpad scrolling
/// cannot jump the volume in one event.
pub fn accumulate_scroll(acc: f32, delta: f32) -> (f32, i32) {
    let acc = acc + delta;
    let steps = (acc / SCROLL_VOLUME_STEP_PX).trunc() as i32;
    (acc - steps as f32 * SCROLL_VOLUME_STEP_PX, steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_overlay_prefers_error_over_audio_placeholder() {
        let mut last = PlayerState::default();
        assert_eq!(media_overlay(&last), MediaOverlay::AudioOnly);
        last.has_video = true;
        assert_eq!(media_overlay(&last), MediaOverlay::None);
        last.has_video = false;
        last.error = Some("无法播放该文件".to_string());
        assert_eq!(
            media_overlay(&last),
            MediaOverlay::Error("无法播放该文件".to_string())
        );
    }

    #[test]
    fn should_resume_skips_positions_near_the_end() {
        assert!(!should_resume(0, 100_000));
        assert!(should_resume(50_000, 100_000));
        assert!(!should_resume(98_000, 100_000));
        assert!(!should_resume(100_000, 100_000));
    }

    #[test]
    fn next_speed_cycles_through_options() {
        assert_eq!(next_speed(0.5), 1.0);
        assert_eq!(next_speed(1.0), 1.5);
        assert_eq!(next_speed(1.5), 2.0);
        assert_eq!(next_speed(2.0), 0.5);
        assert_eq!(next_speed(1.25), 0.5); // unknown -> restart cycle
    }

    #[test]
    fn hover_time_maps_pointer_to_ratio_of_duration() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(200.0, 16.0));
        assert_eq!(hover_time_at(100.0, rect, Some(60_000)), Some(0));
        assert_eq!(hover_time_at(200.0, rect, Some(60_000)), Some(30_000));
        assert_eq!(hover_time_at(300.0, rect, Some(60_000)), Some(60_000));
        // 指针越界时 clamp
        assert_eq!(hover_time_at(400.0, rect, Some(60_000)), Some(60_000));
        assert_eq!(hover_time_at(200.0, rect, None), None);
        // 进度条宽度为 0 时无法映射
        let zero = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(0.0, 16.0));
        assert_eq!(hover_time_at(100.0, zero, Some(60_000)), None);
    }

    #[test]
    fn clamp_seek_bounds_to_duration() {
        assert_eq!(clamp_seek(-500, 10_000), 0);
        assert_eq!(clamp_seek(5_000, 10_000), 5_000);
        assert_eq!(clamp_seek(99_999, 10_000), 10_000);
    }

    #[test]
    fn startup_device_target_keeps_saved_device_on_empty_enumeration() {
        use openitgo_media::AudioDevice;
        let dev = |name: &str| AudioDevice {
            name: name.into(),
            description: None,
        };
        // 未保存设备：不动播放器，设置有效
        assert_eq!(startup_device_target("", &[dev("a")]), (None, true));
        // 枚举瞬时失败（空列表）：不动播放器，且不得清除保存的设备
        assert_eq!(startup_device_target("a", &[]), (None, true));
        // 保存的设备仍在：应用它
        assert_eq!(
            startup_device_target("a", &[dev("a"), dev("b")]),
            (Some("a"), true)
        );
        // 枚举成功但设备已拔出：回退 auto，调用方清除设置
        assert_eq!(
            startup_device_target("gone", &[dev("a")]),
            (Some("auto"), false)
        );
    }

    #[test]
    fn truncate_label_caps_length_with_ellipsis() {
        assert_eq!(truncate_label("自动", 20), "自动");
        assert_eq!(
            truncate_label("MacBook Pro 扬声器", 20),
            "MacBook Pro 扬声器"
        );
        // ASCII：超过 20 字符截断为 19 + 省略号
        assert_eq!(
            truncate_label("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 20),
            "ABCDEFGHIJKLMNOPQRS…"
        );
        // 多字节字符按字符数截断，不会切在 UTF-8 边界中间
        let cjk = "一二三四五六七八九十一二三四五六七八九十一";
        assert_eq!(
            truncate_label(cjk, 20),
            "一二三四五六七八九十一二三四五六七八九…"
        );
    }

    #[test]
    fn osd_texts_format_values() {
        assert_eq!(volume_osd_text(75.0), "音量 75%");
        assert_eq!(speed_osd_text(1.5), "1.5x");
        assert_eq!(mute_osd_text(true), "静音");
        assert_eq!(mute_osd_text(false), "取消静音");
    }

    #[test]
    fn format_sub_delay_signs_and_rounds_to_one_decimal() {
        // 0 不带符号
        assert_eq!(format_sub_delay(0.0), "0.0");
        // 正值带 +
        assert_eq!(format_sub_delay(0.1), "+0.1");
        assert_eq!(format_sub_delay(1.0), "+1.0");
        // 负值带 -
        assert_eq!(format_sub_delay(-0.2), "-0.2");
        assert_eq!(format_sub_delay(-1.5), "-1.5");
        // 保留一位小数（累加浮点误差被舍掉）
        assert_eq!(format_sub_delay(0.30000000000000004), "+0.3");
        assert_eq!(format_sub_delay(-0.05), "-0.1");
    }

    #[test]
    fn tick_osd_keeps_unexpired_osd_and_clears_expired() {
        let ctx = egui::Context::default();
        // 未到期：保留（tick 会为剩余时间重新预约一帧，见 tick_osd 注释）。
        let mut view = MediaView {
            osd: Some(Osd {
                text: "x".into(),
                until: Instant::now() + Duration::from_secs(60),
            }),
            ..Default::default()
        };
        view.tick_osd(&ctx);
        assert!(view.osd.is_some());
        // 已到期：清除；open 为 None 时仅清 egui 侧状态，不触碰原生层。
        view.osd = Some(Osd {
            text: "x".into(),
            until: Instant::now() - Duration::from_millis(1),
        });
        view.tick_osd(&ctx);
        assert!(view.osd.is_none());
    }

    #[test]
    fn close_clears_osd_state() {
        // close 丢弃原生层后若不清 egui 侧 OSD，残留文本会漏进下次打开的媒体。
        let mut view = MediaView {
            osd: Some(Osd {
                text: "x".into(),
                until: Instant::now() + Duration::from_secs(60),
            }),
            ..Default::default()
        };
        view.close();
        assert!(view.osd.is_none());
    }

    #[test]
    fn accumulate_scroll_steps_and_keeps_remainder() {
        let (acc, steps) = accumulate_scroll(0.0, 30.0);
        assert_eq!(steps, 1);
        assert!((acc - 5.0).abs() < 1e-6);
        let (acc, steps) = accumulate_scroll(acc, 20.0);
        assert_eq!(steps, 1);
        assert!((acc - 0.0).abs() < 1e-6);
        // 反向滚动抵消累计值
        let (acc, steps) = accumulate_scroll(10.0, -10.0);
        assert_eq!(steps, 0);
        assert!((acc - 0.0).abs() < 1e-6);
        let (_acc, steps) = accumulate_scroll(0.0, -60.0);
        assert_eq!(steps, -2);
    }

    #[test]
    fn track_label_prefers_title_and_lang() {
        use openitgo_media::tracks::{TrackInfo, TrackKind};
        let t = TrackInfo {
            id: 3,
            kind: TrackKind::Sub,
            title: Some("简中".into()),
            lang: Some("zh".into()),
            codec: None,
            selected: false,
        };
        assert_eq!(track_label(&t, 1), "#2 简中 [zh]");
        let t2 = TrackInfo {
            title: None,
            lang: None,
            ..t.clone()
        };
        assert_eq!(track_label(&t2, 0), "#1 轨道 3");
    }

    #[test]
    fn adjust_speed_value_steps_and_clamps() {
        assert!((adjust_speed_value(1.0, 0.25) - 1.25).abs() < 1e-9);
        assert!((adjust_speed_value(1.0, -0.25) - 0.75).abs() < 1e-9);
        // 浮点尾差被去掉
        assert!((adjust_speed_value(1.25, 0.25) - 1.5).abs() < 1e-9);
        // clamp 到 mpv 允许范围（与 settings.media_speed 校验一致）
        assert!((adjust_speed_value(16.0, 0.25) - 16.0).abs() < 1e-9);
        assert!((adjust_speed_value(0.1, -0.25) - 0.1).abs() < 1e-9);
    }

    #[test]
    fn speed_fine_osd_text_trims_trailing_zeros() {
        assert_eq!(speed_fine_osd_text(1.25), "倍速 1.25x");
        assert_eq!(speed_fine_osd_text(1.5), "倍速 1.5x");
        assert_eq!(speed_fine_osd_text(2.0), "倍速 2x");
        assert_eq!(speed_fine_osd_text(0.1), "倍速 0.1x");
    }

    #[test]
    fn ab_loop_advance_state_machine() {
        // 未设置 -> 设 A（当前秒）
        assert_eq!(ab_loop_advance(AbLoop::None, 83_000), AbLoop::ASet(83.0));
        // 已设 A，B 在 A 之后 -> A/B 齐全
        assert_eq!(
            ab_loop_advance(AbLoop::ASet(83.0), 95_000),
            AbLoop::Both(83.0, 95.0)
        );
        // 已设 A，当前位置在 A 之前 -> 重设 A 到当前位置
        assert_eq!(
            ab_loop_advance(AbLoop::ASet(83.0), 10_000),
            AbLoop::ASet(10.0)
        );
        // A/B 齐全 -> 取消
        assert_eq!(
            ab_loop_advance(AbLoop::Both(83.0, 95.0), 100_000),
            AbLoop::None
        );
    }

    #[test]
    fn ab_loop_osd_text_formats_states() {
        assert_eq!(ab_loop_osd_text(AbLoop::None), "已取消 AB 循环");
        assert_eq!(ab_loop_osd_text(AbLoop::ASet(83.0)), "A 点 01:23");
        assert_eq!(
            ab_loop_osd_text(AbLoop::Both(83.0, 95.0)),
            "AB 循环 01:23 - 01:35"
        );
    }

    #[test]
    fn screenshot_filename_sanitizes_title_and_stamps_utc() {
        let dt = time::Date::from_calendar_date(2026, time::Month::July, 17)
            .unwrap()
            .with_hms(12, 30, 45)
            .unwrap()
            .assume_utc();
        assert_eq!(
            screenshot_filename("Episode 01", dt),
            "Episode_01-20260717-123045.png"
        );
        assert_eq!(
            screenshot_filename("第 1 集/最终话", dt),
            "第_1_集_最终话-20260717-123045.png"
        );
    }
}
