//! Window geometry persistence helpers (size / position / maximized).
//!
//! Validation is pure so unit tests do not need a real display; the app
//! supplies monitor rectangles from egui/winit when available.

/// Default restored size when saved geometry is missing or invalid.
pub const DEFAULT_WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);
pub const MIN_WINDOW_SIZE: (f32, f32) = (400.0, 300.0);

/// Axis-aligned monitor / work-area rectangle in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorRect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl MonitorRect {
    pub fn from_min_size(min: (f32, f32), size: (f32, f32)) -> Self {
        Self {
            min_x: min.0,
            min_y: min.1,
            max_x: min.0 + size.0,
            max_y: min.1 + size.1,
        }
    }

    pub fn intersection_area(self, other: MonitorRect) -> f32 {
        let x0 = self.min_x.max(other.min_x);
        let y0 = self.min_y.max(other.min_y);
        let x1 = self.max_x.min(other.max_x);
        let y1 = self.max_y.min(other.max_y);
        let w = (x1 - x0).max(0.0);
        let h = (y1 - y0).max(0.0);
        w * h
    }
}

/// Geometry we are willing to apply at startup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RestoredGeometry {
    pub size: (f32, f32),
    pub pos: Option<(f32, f32)>,
    pub maximized: bool,
}

/// Decide what to restore. If `monitors` is empty, size/maximized still apply
/// and a saved position is kept optimistically (the OS usually clamps; the app
/// may re-validate once viewport monitor info is available).
pub fn resolve_startup_geometry(
    size: (f32, f32),
    pos: Option<(f32, f32)>,
    maximized: bool,
    monitors: &[MonitorRect],
) -> RestoredGeometry {
    let size = sanitize_size(size);
    if maximized {
        // Maximized: still pass a sane restore size for the un-maximize path,
        // but skip position (OS places the maximized frame).
        return RestoredGeometry {
            size,
            pos: None,
            maximized: true,
        };
    }

    let Some(pos) = pos else {
        return RestoredGeometry {
            size,
            pos: None,
            maximized: false,
        };
    };

    if monitors.is_empty() {
        // No monitor list yet (typical at process start). Keep the saved
        // position optimistically; the OS usually clamps, and the app may
        // re-validate once viewport info is available.
        return RestoredGeometry {
            size,
            pos: Some(pos),
            maximized: false,
        };
    }

    let window = MonitorRect::from_min_size(pos, size);
    let min_visible = 100.0 * 100.0;
    let visible = monitors
        .iter()
        .map(|m| m.intersection_area(window))
        .fold(0.0_f32, f32::max);
    if visible < min_visible {
        // Off-screen after a monitor change — reset to defaults.
        return RestoredGeometry {
            size: DEFAULT_WINDOW_SIZE,
            pos: None,
            maximized: false,
        };
    }

    RestoredGeometry {
        size,
        pos: Some(pos),
        maximized: false,
    }
}

fn sanitize_size(size: (f32, f32)) -> (f32, f32) {
    (
        size.0.clamp(MIN_WINDOW_SIZE.0, 16384.0),
        size.1.clamp(MIN_WINDOW_SIZE.1, 16384.0),
    )
}

/// Snapshot used when persisting geometry from the live viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveGeometry {
    pub inner_size: (f32, f32),
    pub outer_pos: Option<(f32, f32)>,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// Merge live viewport into persisted fields.
/// - Fullscreen: leave stored geometry unchanged (caller should skip).
/// - Maximized: set flag true; keep previous size/pos (restore size).
/// - Normal: update size + pos + maximized=false.
pub fn merge_live_into_settings(
    prev_size: (f32, f32),
    prev_pos: Option<(f32, f32)>,
    live: LiveGeometry,
) -> Option<RestoredGeometry> {
    if live.fullscreen {
        return None;
    }
    if live.maximized {
        return Some(RestoredGeometry {
            size: sanitize_size(prev_size),
            pos: prev_pos,
            maximized: true,
        });
    }
    Some(RestoredGeometry {
        size: sanitize_size(live.inner_size),
        pos: live.outer_pos,
        maximized: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_monitors_keeps_position_optimistically() {
        let g = resolve_startup_geometry((900.0, 700.0), Some((100.0, 80.0)), false, &[]);
        assert_eq!(g.pos, Some((100.0, 80.0)));
        assert_eq!(g.size, (900.0, 700.0));
        assert!(!g.maximized);
    }

    #[test]
    fn offscreen_geometry_resets_to_default() {
        let monitors = [MonitorRect::from_min_size((0.0, 0.0), (1920.0, 1080.0))];
        let g = resolve_startup_geometry((800.0, 600.0), Some((5000.0, 5000.0)), false, &monitors);
        assert_eq!(g.size, DEFAULT_WINDOW_SIZE);
        assert_eq!(g.pos, None);
        assert!(!g.maximized);
    }

    #[test]
    fn onscreen_geometry_kept() {
        let monitors = [MonitorRect::from_min_size((0.0, 0.0), (1920.0, 1080.0))];
        let g = resolve_startup_geometry((900.0, 700.0), Some((100.0, 80.0)), false, &monitors);
        assert_eq!(g.size, (900.0, 700.0));
        assert_eq!(g.pos, Some((100.0, 80.0)));
        assert!(!g.maximized);
    }

    #[test]
    fn maximized_drops_position() {
        let monitors = [MonitorRect::from_min_size((0.0, 0.0), (1920.0, 1080.0))];
        let g = resolve_startup_geometry((900.0, 700.0), Some((100.0, 80.0)), true, &monitors);
        assert!(g.maximized);
        assert_eq!(g.pos, None);
        assert_eq!(g.size, (900.0, 700.0));
    }

    #[test]
    fn merge_maximized_preserves_restore_size() {
        let live = LiveGeometry {
            inner_size: (1920.0, 1080.0),
            outer_pos: Some((0.0, 0.0)),
            maximized: true,
            fullscreen: false,
        };
        let merged = merge_live_into_settings((1100.0, 720.0), Some((40.0, 50.0)), live).unwrap();
        assert_eq!(merged.size, (1100.0, 720.0));
        assert_eq!(merged.pos, Some((40.0, 50.0)));
        assert!(merged.maximized);
    }

    #[test]
    fn merge_normal_updates_size_and_pos() {
        let live = LiveGeometry {
            inner_size: (1024.0, 768.0),
            outer_pos: Some((12.0, 34.0)),
            maximized: false,
            fullscreen: false,
        };
        let merged = merge_live_into_settings((800.0, 600.0), None, live).unwrap();
        assert_eq!(merged.size, (1024.0, 768.0));
        assert_eq!(merged.pos, Some((12.0, 34.0)));
        assert!(!merged.maximized);
    }

    #[test]
    fn merge_fullscreen_skips() {
        let live = LiveGeometry {
            inner_size: (1920.0, 1080.0),
            outer_pos: Some((0.0, 0.0)),
            maximized: false,
            fullscreen: true,
        };
        assert!(merge_live_into_settings((800.0, 600.0), None, live).is_none());
    }
}
