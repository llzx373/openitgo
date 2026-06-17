use crate::models::{FitMode, ReadingMode};
use crate::state::Vec2;

pub struct Rect {
    pub min: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self { min, size }
    }
}

pub struct PageLayout {
    pub rect: Rect,
    pub page_index: usize,
}

pub fn compute_layout(
    mode: ReadingMode,
    viewport_size: Vec2,
    page_sizes: &[Vec2],
    zoom: f32,
) -> Vec<PageLayout> {
    let mut layouts = Vec::new();
    match mode {
        ReadingMode::Ltr | ReadingMode::Rtl => {
            let mut cursor = 0.0;
            let direction = if matches!(mode, ReadingMode::Ltr) {
                1.0
            } else {
                -1.0
            };
            for (idx, &size) in page_sizes.iter().enumerate() {
                let scaled = scale_to_fit(size, viewport_size, FitMode::Height) * zoom;
                let x = if direction > 0.0 {
                    cursor
                } else {
                    viewport_size.x - cursor - scaled.x
                };
                layouts.push(PageLayout {
                    rect: Rect::from_min_size(
                        Vec2::new(x, (viewport_size.y - scaled.y) / 2.0),
                        scaled,
                    ),
                    page_index: idx,
                });
                cursor += scaled.x;
            }
        }
        ReadingMode::Webtoon => {
            let mut cursor = 0.0;
            for (idx, &size) in page_sizes.iter().enumerate() {
                let scaled = scale_to_fit(size, viewport_size, FitMode::Width) * zoom;
                layouts.push(PageLayout {
                    rect: Rect::from_min_size(
                        Vec2::new((viewport_size.x - scaled.x) / 2.0, cursor),
                        scaled,
                    ),
                    page_index: idx,
                });
                cursor += scaled.y;
            }
        }
    }
    layouts
}

pub fn scale_to_fit(size: Vec2, viewport: Vec2, fit_mode: FitMode) -> Vec2 {
    match fit_mode {
        FitMode::Original => size,
        FitMode::Page => {
            let scale = (viewport.x / size.x).min(viewport.y / size.y);
            size * scale
        }
        FitMode::Height => {
            let scale = viewport.y / size.y;
            size * scale
        }
        FitMode::Width => {
            let scale = viewport.x / size.x;
            size * scale
        }
    }
}

impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_to_fit_height() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let result = scale_to_fit(size, viewport, FitMode::Height);
        assert_eq!(result.y, 1080.0);
    }

    #[test]
    fn test_webtoon_layout_stacks_vertically() {
        let sizes = vec![Vec2::new(800.0, 1200.0), Vec2::new(800.0, 1200.0)];
        let viewport = Vec2::new(1000.0, 600.0);
        let layouts = compute_layout(ReadingMode::Webtoon, viewport, &sizes, 1.0);
        assert_eq!(layouts.len(), 2);
        assert!(layouts[1].rect.min.y > layouts[0].rect.min.y);
    }
}
