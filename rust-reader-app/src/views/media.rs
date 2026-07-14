use rust_reader_media::{MpvPlayer, PlayerState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct MediaView {
    pub open: Option<OpenMedia>,
}

pub struct OpenMedia {
    // `path`/`title` are part of the planned interface: Task 8 renders `title`
    // in the control bar and Task 9 records history from `path`/`title`.
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
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
        ui.allocate_space(ui.available_size());
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
}

/// Only resume when the saved position is not within the last 3 seconds,
/// so "reopen at the end" does not flash-finish the file.
pub fn should_resume(position_ms: u64, duration_ms: u64) -> bool {
    position_ms > 0 && position_ms + 3000 < duration_ms
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
}
