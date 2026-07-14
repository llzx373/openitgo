use rust_reader_media::tracks::{TrackInfo, TrackKind};
use rust_reader_media::{MpvPlayer, PlayerState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct MediaView {
    pub open: Option<OpenMedia>,
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
        });
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = None; // Drops native view, render context and player.
    }

    pub fn update_bounds(&mut self, bounds: wry::Rect) {
        if let Some(open) = self.open.as_ref() {
            open.native.set_bounds(bounds);
        }
    }

    /// Copies the latest player state into `last` for UI reads; applies a
    /// pending resume once the duration is known.
    pub fn sync_state(&mut self) {
        if let Some(open) = self.open.as_mut() {
            if let Ok(s) = open.state.lock() {
                open.last = s.clone();
            }
            if let Some(ms) = open.pending_resume_ms {
                if let Some(dur) = open.last.duration_ms {
                    if should_resume(ms, dur) {
                        let _ = open.player.seek_abs_ms(ms);
                    }
                    open.pending_resume_ms = None;
                }
            }
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.allocate_space(ui.available_size());
        let has_video = self.open.as_ref().map(|o| o.last.has_video).unwrap_or(true);
        if !has_video {
            // Audio-only file: the native view is parked at zero size (see
            // render_media), so paint a placeholder here.
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
            let title = self
                .open
                .as_ref()
                .map(|o| o.title.clone())
                .unwrap_or_default();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                title,
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
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
                let _ = open.player.seek_abs_ms(target);
            }
        }
    }

    pub fn set_volume(&mut self, v: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_volume(v);
        }
    }

    pub fn adjust_volume(&mut self, delta: f64) {
        let v = self.open.as_ref().map(|o| o.last.volume + delta);
        if let Some(v) = v {
            self.set_volume(v);
        }
    }

    pub fn cycle_speed(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_speed(next_speed(open.last.speed));
        }
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

/// Toolbar/dropdown label for a track: `#2 简中 [zh]`, or `#1 轨道 3`
/// when the track carries no title.
pub fn track_label(t: &TrackInfo, index: usize) -> String {
    let base = t.title.clone().unwrap_or_else(|| format!("轨道 {}", t.id));
    match &t.lang {
        Some(lang) => format!("#{} {} [{}]", index + 1, base, lang),
        None => format!("#{} {}", index + 1, base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn clamp_seek_bounds_to_duration() {
        assert_eq!(clamp_seek(-500, 10_000), 0);
        assert_eq!(clamp_seek(5_000, 10_000), 5_000);
        assert_eq!(clamp_seek(99_999, 10_000), 10_000);
    }

    #[test]
    fn track_label_prefers_title_and_lang() {
        use rust_reader_media::tracks::{TrackInfo, TrackKind};
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
}
