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
        };
        state.clamp_page(total_pages);
        state
    }

    pub fn next_page(&mut self, total_pages: usize) {
        if total_pages == 0 {
            return;
        }
        let step = self.page_step();
        if self.current_page + step < total_pages {
            self.current_page += step;
            self.pan = Vec2::ZERO;
        } else {
            self.current_page = total_pages - 1;
            self.pan = Vec2::ZERO;
        }
    }

    pub fn prev_page(&mut self) {
        let step = self.page_step();
        if self.current_page >= step {
            self.current_page -= step;
        } else {
            self.current_page = 0;
        }
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

    pub fn toggle_double_page(&mut self, total_pages: usize) {
        self.set_double_page(!self.double_page, total_pages);
    }

    fn page_step(&self) -> usize {
        if self.double_page && !self.mode.is_webtoon() {
            2
        } else {
            1
        }
    }

    fn clamp_page(&mut self, total_pages: usize) {
        if total_pages == 0 {
            self.current_page = 0;
            return;
        }
        if self.current_page >= total_pages {
            self.current_page = total_pages - 1;
        }
        if self.double_page && !self.mode.is_webtoon() {
            // Align to the start of a spread.
            self.current_page = (self.current_page / 2) * 2;
        }
    }
}

fn default_fit_mode(mode: ReadingMode) -> FitMode {
    match mode {
        ReadingMode::Ltr | ReadingMode::Rtl => FitMode::Height,
        ReadingMode::Webtoon => FitMode::Width,
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
    fn test_double_page_navigation_steps_by_two() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 7);
        state.set_double_page(true, 7);
        assert_eq!(state.current_page, 0);

        state.next_page(7);
        assert_eq!(state.current_page, 2);

        state.next_page(7);
        assert_eq!(state.current_page, 4);

        state.next_page(7);
        assert_eq!(state.current_page, 6);

        // At the end, cannot advance further.
        state.next_page(7);
        assert_eq!(state.current_page, 6);

        state.prev_page();
        assert_eq!(state.current_page, 4);
    }

    #[test]
    fn test_double_page_rtl_steps_by_two() {
        let mut state = ReadingState::new(ReadingMode::Rtl, 7);
        state.set_double_page(true, 7);
        state.next_page(7);
        assert_eq!(state.current_page, 2);
        state.prev_page();
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_go_to_page_aligns_to_spread_start() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 10);
        state.set_double_page(true, 10);
        state.go_to_page(3, 10);
        assert_eq!(state.current_page, 2);
        state.go_to_page(5, 10);
        assert_eq!(state.current_page, 4);
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
}
