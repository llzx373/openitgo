use crate::models::{FitMode, ReadingMode};
use crate::state::Vec2;

/// A rectangle defined by its minimum-corner position and size.
pub struct Rect {
    pub min: Vec2,
    pub size: Vec2,
}

impl Rect {
    /// Creates a `Rect` from its minimum-corner position and size.
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self { min, size }
    }
}

/// The computed layout for a single page within a viewport.
pub struct PageLayout {
    pub rect: Rect,
    pub page_index: usize,
}

/// Computes the on-screen layout for a sequence of pages.
///
/// # Parameters
/// - `mode`: The reading direction (left-to-right, right-to-left, or vertical webtoon).
/// - `viewport_size`: The size of the viewport that contains the pages.
/// - `page_sizes`: The intrinsic sizes of each page.
/// - `zoom`: A scale factor applied after fitting each page to the viewport.
///
/// # Returns
/// A vector of `PageLayout` entries, one per provided page size, describing where
/// each page should be positioned and how large it should appear.
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

/// Scales a `size` to fit within `viewport` according to `fit_mode`.
///
/// # Parameters
/// - `size`: The intrinsic size to be scaled.
/// - `viewport`: The size of the container to fit within.
/// - `fit_mode`: The fitting strategy (`Original`, `Page`, `Height`, or `Width`).
///
/// # Returns
/// The scaled size. If `size` has a zero width or height, `Vec2::ZERO` is returned
/// to avoid division by zero.
pub fn scale_to_fit(size: Vec2, viewport: Vec2, fit_mode: FitMode) -> Vec2 {
    if size.x == 0.0 || size.y == 0.0 {
        return Vec2::ZERO;
    }
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
    fn test_scale_to_fit_width() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let result = scale_to_fit(size, viewport, FitMode::Width);
        assert!((result.x - 1920.0).abs() < 1e-3);
    }

    #[test]
    fn test_scale_to_fit_page() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let result = scale_to_fit(size, viewport, FitMode::Page);
        let scale = (viewport.x / size.x).min(viewport.y / size.y);
        assert_eq!(result, size * scale);
    }

    #[test]
    fn test_scale_to_fit_original() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let result = scale_to_fit(size, viewport, FitMode::Original);
        assert_eq!(result, size);
    }

    #[test]
    fn test_scale_to_fit_zero_size_returns_zero() {
        let viewport = Vec2::new(1920.0, 1080.0);
        assert_eq!(
            scale_to_fit(Vec2::ZERO, viewport, FitMode::Page),
            Vec2::ZERO
        );
        assert_eq!(
            scale_to_fit(Vec2::new(0.0, 100.0), viewport, FitMode::Width),
            Vec2::ZERO
        );
        assert_eq!(
            scale_to_fit(Vec2::new(100.0, 0.0), viewport, FitMode::Height),
            Vec2::ZERO
        );
    }

    #[test]
    fn test_webtoon_layout_stacks_vertically() {
        let sizes = vec![Vec2::new(800.0, 1200.0), Vec2::new(800.0, 1200.0)];
        let viewport = Vec2::new(1000.0, 600.0);
        let layouts = compute_layout(ReadingMode::Webtoon, viewport, &sizes, 1.0);
        assert_eq!(layouts.len(), 2);
        assert!(layouts[1].rect.min.y > layouts[0].rect.min.y);
    }

    #[test]
    fn test_compute_layout_ltr() {
        let sizes = vec![Vec2::new(800.0, 1200.0), Vec2::new(800.0, 1200.0)];
        let viewport = Vec2::new(1920.0, 1080.0);
        let layouts = compute_layout(ReadingMode::Ltr, viewport, &sizes, 1.0);
        assert_eq!(layouts.len(), 2);
        assert!(layouts[1].rect.min.x > layouts[0].rect.min.x);
        assert_eq!(layouts[0].page_index, 0);
        assert_eq!(layouts[1].page_index, 1);
    }

    #[test]
    fn test_compute_layout_rtl() {
        let sizes = vec![Vec2::new(800.0, 1200.0), Vec2::new(800.0, 1200.0)];
        let viewport = Vec2::new(1920.0, 1080.0);
        let layouts = compute_layout(ReadingMode::Rtl, viewport, &sizes, 1.0);
        assert_eq!(layouts.len(), 2);
        assert!(layouts[1].rect.min.x < layouts[0].rect.min.x);
        assert_eq!(layouts[0].page_index, 0);
        assert_eq!(layouts[1].page_index, 1);
    }

    #[test]
    fn test_compute_layout_zoom_doubles_sizes() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let layouts = compute_layout(ReadingMode::Ltr, viewport, &[size], 2.0);
        assert_eq!(layouts.len(), 1);
        let scaled = scale_to_fit(size, viewport, FitMode::Height);
        assert_eq!(layouts[0].rect.size, scaled * 2.0);
    }

    #[test]
    fn test_compute_layout_empty_page_sizes_returns_empty() {
        let viewport = Vec2::new(1920.0, 1080.0);
        let layouts = compute_layout(ReadingMode::Ltr, viewport, &[], 1.0);
        assert!(layouts.is_empty());
    }

    #[test]
    fn test_compute_layout_zero_viewport_returns_zero_sized_layouts() {
        let size = Vec2::new(800.0, 1200.0);
        let layouts = compute_layout(ReadingMode::Ltr, Vec2::ZERO, &[size], 1.0);
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].rect.size, Vec2::ZERO);
    }
}
