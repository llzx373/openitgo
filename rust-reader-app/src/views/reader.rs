use crate::widgets::page_navigator::page_navigator;
use rust_reader_core::models::{Comic, PageSource, ReadingMode};
use rust_reader_core::state::{ReadingState, Vec2};

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 5.0;

#[derive(Default)]
pub struct ReaderView {
    pub open: Option<OpenReader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFit {
    /// Fit the image width to the available viewport width.
    Width,
    /// Fit the image height to the available viewport height.
    Height,
    /// Fit the whole image inside the available viewport (letterbox).
    Page,
}

pub struct OpenReader {
    pub comic: Comic,
    pub state: ReadingState,
    pub left_texture: Option<egui::TextureHandle>,
    pub left_page: Option<usize>,
    pub right_texture: Option<egui::TextureHandle>,
    pub right_page: Option<usize>,
    pub pending_fit: Option<QuickFit>,
}

impl OpenReader {
    pub fn total_pages(&self) -> usize {
        self.comic
            .volumes
            .first()
            .map(|v| v.pages.len())
            .unwrap_or(0)
    }

    pub fn zoom_in(&mut self) {
        self.state.zoom *= 1.1;
        self.state.zoom = self.state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn zoom_out(&mut self) {
        self.state.zoom *= 0.9;
        self.state.zoom = self.state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn request_fit(&mut self, fit: QuickFit) {
        self.pending_fit = Some(fit);
    }

    pub fn first_page(&mut self) {
        let total = self.total_pages();
        if total > 0 {
            self.state.go_to_page(0, total);
        }
    }

    pub fn last_page(&mut self) {
        let total = self.total_pages();
        if total > 0 {
            self.state.go_to_page(total - 1, total);
        }
    }

    fn is_double_page(&self) -> bool {
        self.state.double_page && !self.state.mode.is_webtoon()
    }

    /// Returns the page indices to display in left and right slots.
    fn spread_pages(&self) -> (usize, Option<usize>) {
        let current = self.state.current_page;
        let total = self.total_pages();
        if total == 0 {
            return (0, None);
        }
        let next = (current + 1).min(total - 1);
        if !self.is_double_page() || next == current {
            return (current, None);
        }
        match self.state.mode {
            ReadingMode::Ltr => (current, Some(next)),
            ReadingMode::Rtl => (next, Some(current)),
            ReadingMode::Webtoon => (current, None),
        }
    }

    fn spread_size(&self) -> Option<egui::Vec2> {
        let left_size = self.left_texture.as_ref()?.size_vec2();
        let right_size = self
            .right_texture
            .as_ref()
            .map(|t| t.size_vec2())
            .unwrap_or(egui::Vec2::ZERO);
        Some(egui::vec2(
            left_size.x + right_size.x,
            left_size.y.max(right_size.y),
        ))
    }

    fn apply_pending_fit(&mut self, available: egui::Vec2) {
        let Some(fit) = self.pending_fit.take() else {
            return;
        };
        let Some(spread_size) = self.spread_size() else {
            return;
        };
        if spread_size.x <= 0.0 || spread_size.y <= 0.0 {
            return;
        }

        let scale = match fit {
            QuickFit::Width => available.x / spread_size.x,
            QuickFit::Height => available.y / spread_size.y,
            QuickFit::Page => (available.x / spread_size.x).min(available.y / spread_size.y),
        };
        self.state.zoom = scale.clamp(MIN_ZOOM, MAX_ZOOM);
        self.state.pan = Vec2::ZERO;
    }
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState) {
        self.open = Some(OpenReader {
            comic,
            state,
            left_texture: None,
            left_page: None,
            right_texture: None,
            right_page: None,
            pending_fit: Some(QuickFit::Page),
        });
    }

    /// Renders the current page or spread and returns the response covering the page area.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<egui::Response> {
        let Some(reader) = &mut self.open else {
            ui.label("未打开漫画");
            return None;
        };

        let total_pages = reader.total_pages();
        if total_pages == 0 {
            ui.label("此漫画没有页面");
            return None;
        }

        let (left_idx, right_idx) = reader.spread_pages();
        if reader.left_page != Some(left_idx) {
            reader.left_texture = load_page_texture(ui.ctx(), &reader.comic, left_idx).ok();
            reader.left_page = Some(left_idx);
            reader.pending_fit = reader.pending_fit.or(Some(QuickFit::Page));
        }
        if reader.right_page != right_idx {
            reader.right_texture =
                right_idx.and_then(|idx| load_page_texture(ui.ctx(), &reader.comic, idx).ok());
            reader.right_page = right_idx;
            reader.pending_fit = reader.pending_fit.or(Some(QuickFit::Page));
        }

        let available = ui.available_rect_before_wrap();
        reader.apply_pending_fit(available.size());

        let left_texture = reader.left_texture.clone()?;
        let right_texture = reader.right_texture.clone();
        let left_size = left_texture.size_vec2();
        let right_size = right_texture
            .as_ref()
            .map(|t| t.size_vec2())
            .unwrap_or(egui::Vec2::ZERO);
        let spread_size = egui::vec2(left_size.x + right_size.x, left_size.y.max(right_size.y));
        let scaled_spread = spread_size * reader.state.zoom;

        let half_size = scaled_spread / 2.0;
        let max_pan_x = (available.width() / 2.0 + half_size.x).max(0.0);
        let max_pan_y = (available.height() / 2.0 + half_size.y).max(0.0);
        reader.state.pan.x = reader.state.pan.x.clamp(-max_pan_x, max_pan_x);
        reader.state.pan.y = reader.state.pan.y.clamp(-max_pan_y, max_pan_y);

        let center = available.center();
        let spread_top_left = egui::pos2(
            center.x - scaled_spread.x / 2.0 + reader.state.pan.x,
            center.y - scaled_spread.y / 2.0 + reader.state.pan.y,
        );

        // Render left page.
        let left_rect = egui::Rect::from_min_size(spread_top_left, left_size * reader.state.zoom);
        let left_response = ui.put(
            left_rect,
            egui::Image::new(&left_texture)
                .fit_to_exact_size(left_size * reader.state.zoom)
                .sense(egui::Sense::drag()),
        );
        if left_response.dragged() {
            let delta = left_response.drag_delta();
            reader.state.pan += Vec2::new(delta.x, delta.y);
        }

        // Render right page if present.
        if let Some(right_texture) = right_texture {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(spread_top_left.x + left_rect.width(), spread_top_left.y),
                right_size * reader.state.zoom,
            );
            let right_response = ui.put(
                right_rect,
                egui::Image::new(&right_texture)
                    .fit_to_exact_size(right_size * reader.state.zoom)
                    .sense(egui::Sense::drag()),
            );
            if right_response.dragged() {
                let delta = right_response.drag_delta();
                reader.state.pan += Vec2::new(delta.x, delta.y);
            }
        }

        Some(left_response)
    }

    pub fn render_page_navigator(&mut self, ui: &mut egui::Ui) {
        let Some(reader) = &mut self.open else {
            return;
        };
        let total_pages = reader.total_pages();
        if total_pages == 0 {
            return;
        }
        let current_page = reader.state.current_page;
        let comic = &reader.comic;
        let state = &mut reader.state;
        let left_page = &mut reader.left_page;
        let right_page = &mut reader.right_page;
        page_navigator(ui, comic, current_page, &mut |idx| {
            state.go_to_page(idx, total_pages);
            *left_page = None;
            *right_page = None;
        });
    }
}

fn load_page_texture(
    ctx: &egui::Context,
    comic: &Comic,
    page_index: usize,
) -> Result<egui::TextureHandle, String> {
    let volume = comic
        .volumes
        .first()
        .ok_or_else(|| "漫画没有卷".to_string())?;
    let page = volume
        .pages
        .get(page_index)
        .ok_or_else(|| "页面索引越界".to_string())?;

    let label = format!("page_{}", page_index);
    match &page.source {
        PageSource::File(path) => {
            crate::widgets::page_view::load_texture_from_path(ctx, path, &label)
        }
        PageSource::Bytes(bytes) => {
            crate::widgets::page_view::load_texture_from_bytes(ctx, bytes.as_ref(), &label)
        }
        PageSource::PdfRef { .. } => Err("PDF 页面暂不支持".to_string()),
    }
}
