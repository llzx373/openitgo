use crate::models::{FitMode, ReadingMode};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadingState {
    pub mode: ReadingMode,
    pub current_page: usize,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit_mode: FitMode,
    pub double_page: bool,
    /// 90° 步进的显示旋转（0/90/180/270），随每书阅读设置持久化。
    pub rotation: u16,
}

impl ReadingState {
    pub fn new(mode: ReadingMode, total_pages: usize) -> Self {
        let mut state = Self {
            mode,
            current_page: 0,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_mode: default_fit_mode(mode),
            double_page: false,
            rotation: 0,
        };
        state.clamp_page(total_pages);
        state
    }

    pub fn next_page(&mut self, total_pages: usize) {
        if total_pages == 0 {
            return;
        }
        self.current_page = self.next_anchor(total_pages);
        self.pan = Vec2::ZERO;
    }

    pub fn prev_page(&mut self) {
        self.current_page = self.prev_anchor();
        self.pan = Vec2::ZERO;
    }

    pub fn go_to_page(&mut self, page: usize, total_pages: usize) {
        self.current_page = page;
        self.pan = Vec2::ZERO;
        self.clamp_page(total_pages);
    }

    pub fn set_mode(&mut self, mode: ReadingMode, total_pages: usize) {
        self.mode = mode;
        self.fit_mode = default_fit_mode(mode);
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.clamp_page(total_pages);
    }

    pub fn set_double_page(&mut self, double_page: bool, total_pages: usize) {
        self.double_page = double_page && !self.mode.is_webtoon();
        self.pan = Vec2::ZERO;
        self.clamp_page(total_pages);
    }

    /// 顺时针旋转 90°（360° 取模回 0）。旋转不重置页码/缩放，
    /// 重新适配由调用方挂 pending_fit（与双页开关同一路径）。
    pub fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
    }

    pub fn toggle_double_page(&mut self, total_pages: usize) {
        self.set_double_page(!self.double_page, total_pages);
    }

    /// True when the reader is currently in double-page mode.
    pub fn is_double_page(&self) -> bool {
        self.double_page && !self.mode.is_webtoon()
    }

    /// Returns the last valid spread anchor for the given total page count.
    ///
    /// In double-page mode with cover handling, the anchors are `0, 1, 3, 5, ...`.
    pub fn last_anchor(&self, total_pages: usize) -> usize {
        match total_pages {
            0 | 1 => 0,
            n => n - 1 - (n % 2),
        }
    }

    fn next_anchor(&self, total_pages: usize) -> usize {
        if !self.is_double_page() || total_pages <= 1 {
            return (self.current_page + 1).min(total_pages.saturating_sub(1));
        }
        let last = self.last_anchor(total_pages);
        if self.current_page == 0 {
            return 1.min(last);
        }
        (self.current_page + 2).min(last)
    }

    fn prev_anchor(&self) -> usize {
        if !self.is_double_page() {
            return self.current_page.saturating_sub(1);
        }
        if self.current_page <= 1 {
            return 0;
        }
        self.current_page - 2
    }

    fn clamp_page(&mut self, total_pages: usize) {
        if total_pages == 0 {
            self.current_page = 0;
            return;
        }
        if self.current_page >= total_pages {
            self.current_page = total_pages - 1;
        }
        if !self.is_double_page() {
            return;
        }
        let last = self.last_anchor(total_pages);
        if self.current_page == 0 || self.current_page > last {
            self.current_page = 0;
            return;
        }
        // Align to the nearest odd-numbered anchor.
        if self.current_page.is_multiple_of(2) {
            self.current_page = (self.current_page - 1).min(last);
        }
    }

    /// Move forward by one logical spread, respecting wide-page boundaries.
    /// `is_wide` should return true for pages that must be shown alone.
    pub fn next_spread(&mut self, total_pages: usize, is_wide: impl Fn(usize) -> bool) {
        if !self.is_double_page() || total_pages <= 1 {
            self.next_page(total_pages);
            return;
        }
        let last = self.last_anchor(total_pages);
        if self.current_page == 0 {
            self.current_page = 1.min(last);
        } else if is_wide(self.current_page) {
            self.current_page = (self.current_page + 1).min(last);
        } else {
            let candidate = self.current_page + 2;
            if candidate <= last {
                self.current_page = candidate;
            } else if self.current_page < last {
                self.current_page = last;
            }
        }
        self.pan = Vec2::ZERO;
    }

    /// Move backward by one logical spread, respecting wide-page boundaries.
    pub fn prev_spread(&mut self, is_wide: impl Fn(usize) -> bool) {
        if !self.is_double_page() {
            self.prev_page();
            return;
        }
        if self.current_page == 0 {
            return;
        }
        if is_wide(self.current_page) {
            self.current_page = self.current_page.saturating_sub(1);
        } else if self.current_page <= 1 {
            self.current_page = 0;
        } else if is_wide(self.current_page - 1) {
            self.current_page -= 1;
        } else {
            self.current_page = self.current_page.saturating_sub(2);
        }
        self.pan = Vec2::ZERO;
    }
}

fn default_fit_mode(mode: ReadingMode) -> FitMode {
    match mode {
        ReadingMode::Ltr | ReadingMode::Rtl => FitMode::Height,
        ReadingMode::Webtoon => FitMode::Width,
    }
}

/// 90° 步进旋转后的有效尺寸：90°/270° 宽高互换；其余值（含非法值）原样返回。
pub fn rotate_size(size: [u32; 2], rotation: u16) -> [u32; 2] {
    match rotation % 360 {
        90 | 270 => [size[1], size[0]],
        _ => size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReadingMode;

    #[test]
    fn test_next_page_stops_at_end() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 3);
        state.next_page(3);
        state.next_page(3);
        state.next_page(3);
        assert_eq!(state.current_page, 2);
    }

    #[test]
    fn test_prev_page_stops_at_start() {
        let mut state = ReadingState::new(ReadingMode::Rtl, 3);
        state.prev_page();
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_go_to_page_rejects_out_of_bounds() {
        let mut state = ReadingState::new(ReadingMode::Webtoon, 5);
        state.go_to_page(10, 5);
        assert_eq!(state.current_page, 4);
        state.go_to_page(2, 5);
        assert_eq!(state.current_page, 2);
    }

    #[test]
    fn test_set_mode_resets_zoom_fit_pan_and_clamps_page() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 5);
        state.current_page = 4;
        state.zoom = 2.5;
        state.pan = Vec2 { x: 10.0, y: 20.0 };

        state.set_mode(ReadingMode::Webtoon, 3);

        assert_eq!(state.mode, ReadingMode::Webtoon);
        assert_eq!(state.fit_mode, FitMode::Width);
        assert_eq!(state.zoom, 1.0);
        assert_eq!(state.pan, Vec2::ZERO);
        assert_eq!(state.current_page, 2);
    }

    #[test]
    fn test_double_page_navigation_with_cover_page() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 7);
        state.set_double_page(true, 7);
        assert_eq!(state.current_page, 0);

        // Cover is alone, next spread starts at page 1.
        state.next_page(7);
        assert_eq!(state.current_page, 1);

        state.next_page(7);
        assert_eq!(state.current_page, 3);

        state.next_page(7);
        assert_eq!(state.current_page, 5);

        // At the end, cannot advance further.
        state.next_page(7);
        assert_eq!(state.current_page, 5);

        state.prev_page();
        assert_eq!(state.current_page, 3);

        state.prev_page();
        assert_eq!(state.current_page, 1);

        state.prev_page();
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_double_page_rtl_navigation_with_cover_page() {
        let mut state = ReadingState::new(ReadingMode::Rtl, 7);
        state.set_double_page(true, 7);
        state.next_page(7);
        assert_eq!(state.current_page, 1);
        state.prev_page();
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_go_to_page_aligns_to_spread_anchor() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 10);
        state.set_double_page(true, 10);
        state.go_to_page(3, 10);
        assert_eq!(state.current_page, 3);
        state.go_to_page(4, 10);
        assert_eq!(state.current_page, 3);
        state.go_to_page(5, 10);
        assert_eq!(state.current_page, 5);
    }

    #[test]
    fn test_webtoon_ignores_double_page() {
        let mut state = ReadingState::new(ReadingMode::Webtoon, 10);
        state.set_double_page(true, 10);
        assert!(!state.double_page);
        state.next_page(10);
        assert_eq!(state.current_page, 1);
    }

    #[test]
    fn test_default_fit_mode_for_each_reading_mode() {
        assert_eq!(default_fit_mode(ReadingMode::Ltr), FitMode::Height);
        assert_eq!(default_fit_mode(ReadingMode::Rtl), FitMode::Height);
        assert_eq!(default_fit_mode(ReadingMode::Webtoon), FitMode::Width);
    }

    #[test]
    fn test_new_with_zero_total_pages() {
        let state = ReadingState::new(ReadingMode::Ltr, 0);
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_new_starts_at_first_page() {
        // ReadingState::new always starts at page 0, so this primarily
        // ensures the constructor respects total_pages when it is non-zero.
        let state = ReadingState::new(ReadingMode::Ltr, 3);
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_rotate_cw_steps_and_wraps() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 10);
        assert_eq!(state.rotation, 0);
        state.rotate_cw();
        assert_eq!(state.rotation, 90);
        state.rotate_cw();
        assert_eq!(state.rotation, 180);
        state.rotate_cw();
        assert_eq!(state.rotation, 270);
        state.rotate_cw();
        assert_eq!(state.rotation, 0);
    }

    #[test]
    fn test_rotate_size_swaps_on_quarter_turns() {
        assert_eq!(rotate_size([800, 1200], 0), [800, 1200]);
        assert_eq!(rotate_size([800, 1200], 90), [1200, 800]);
        assert_eq!(rotate_size([800, 1200], 180), [800, 1200]);
        assert_eq!(rotate_size([800, 1200], 270), [1200, 800]);
        // 防御：非 90 倍数的值按不旋转处理
        assert_eq!(rotate_size([800, 1200], 45), [800, 1200]);
        assert_eq!(rotate_size([800, 1200], 450), [1200, 800]);
    }

    #[test]
    fn test_rotation_does_not_affect_page_navigation() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 10);
        state.rotate_cw();
        state.next_page(10);
        assert_eq!(state.current_page, 1);
        assert_eq!(state.rotation, 90);
    }
}
