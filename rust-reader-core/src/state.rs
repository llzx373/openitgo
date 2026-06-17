use crate::models::{FitMode, ReadingMode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadingState {
    pub mode: ReadingMode,
    pub current_page: usize,
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub fit_mode: FitMode,
}

impl ReadingState {
    pub fn new(mode: ReadingMode, _total_pages: usize) -> Self {
        Self {
            mode,
            current_page: 0,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            fit_mode: default_fit_mode(mode),
        }
    }

    pub fn next_page(&mut self, total_pages: usize) {
        if self.current_page + 1 < total_pages {
            self.current_page += 1;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn prev_page(&mut self) {
        if self.current_page > 0 {
            self.current_page -= 1;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn go_to_page(&mut self, page: usize, total_pages: usize) {
        if page < total_pages {
            self.current_page = page;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn set_mode(&mut self, mode: ReadingMode, total_pages: usize) {
        self.mode = mode;
        self.fit_mode = default_fit_mode(mode);
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        if self.current_page >= total_pages && total_pages > 0 {
            self.current_page = total_pages - 1;
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
    fn test_go_to_page_clamps() {
        let mut state = ReadingState::new(ReadingMode::Webtoon, 5);
        state.go_to_page(10, 5);
        assert_eq!(state.current_page, 0);
        state.go_to_page(2, 5);
        assert_eq!(state.current_page, 2);
    }
}
