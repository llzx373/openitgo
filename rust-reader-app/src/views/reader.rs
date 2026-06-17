use crate::widgets::page_navigator::page_navigator;
use rust_reader_core::models::{Comic, PageSource};
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
    pub texture: Option<egui::TextureHandle>,
    pub texture_page: Option<usize>,
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

    fn apply_pending_fit(&mut self, available: egui::Vec2) {
        let Some(fit) = self.pending_fit.take() else {
            return;
        };
        let Some(texture) = &self.texture else {
            return;
        };
        let image_size = texture.size_vec2();
        if image_size.x <= 0.0 || image_size.y <= 0.0 {
            return;
        }

        let scale = match fit {
            QuickFit::Width => available.x / image_size.x,
            QuickFit::Height => available.y / image_size.y,
            QuickFit::Page => (available.x / image_size.x).min(available.y / image_size.y),
        };
        self.state.zoom = scale.clamp(MIN_ZOOM, MAX_ZOOM);
        self.state.pan = Vec2::ZERO;
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
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState) {
        self.open = Some(OpenReader {
            comic,
            state,
            texture: None,
            texture_page: None,
            pending_fit: Some(QuickFit::Page),
        });
    }

    /// Renders the current page and returns the response of the image widget.
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

        if reader.texture_page != Some(reader.state.current_page) {
            match load_page_texture(ui.ctx(), &reader.comic, reader.state.current_page) {
                Ok(texture) => {
                    reader.texture = Some(texture);
                    reader.texture_page = Some(reader.state.current_page);
                    // New page loaded: remember to auto-fit on first render.
                    if reader.pending_fit.is_none() {
                        reader.pending_fit = Some(QuickFit::Page);
                    }
                }
                Err(err) => {
                    reader.texture = None;
                    reader.texture_page = None;
                    ui.label(err);
                    return None;
                }
            }
        }

        let texture = reader.texture.clone();
        if let Some(texture) = texture {
            let available = ui.available_rect_before_wrap();
            reader.apply_pending_fit(available.size());

            let texture_size = texture.size_vec2();
            let scaled_size = texture_size * reader.state.zoom;
            let half_size = scaled_size / 2.0;
            let max_pan_x = (available.width() / 2.0 + half_size.x).max(0.0);
            let max_pan_y = (available.height() / 2.0 + half_size.y).max(0.0);
            reader.state.pan.x = reader.state.pan.x.clamp(-max_pan_x, max_pan_x);
            reader.state.pan.y = reader.state.pan.y.clamp(-max_pan_y, max_pan_y);
            let center = available.center();
            let top_left = egui::pos2(
                center.x - scaled_size.x / 2.0 + reader.state.pan.x,
                center.y - scaled_size.y / 2.0 + reader.state.pan.y,
            );
            let image_rect = egui::Rect::from_min_size(top_left, scaled_size);
            let response = ui.put(
                image_rect,
                egui::Image::new(&texture)
                    .fit_to_exact_size(scaled_size)
                    .sense(egui::Sense::drag()),
            );
            if response.dragged() {
                let delta = response.drag_delta();
                reader.state.pan += Vec2::new(delta.x, delta.y);
            }
            Some(response)
        } else {
            ui.label("无法加载页面");
            None
        }
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
        let texture_page = &mut reader.texture_page;
        page_navigator(ui, comic, current_page, &mut |idx| {
            state.go_to_page(idx, total_pages);
            *texture_page = None;
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
            crate::widgets::page_view::load_texture_from_bytes(ctx, bytes.as_slice(), &label)
        }
        PageSource::PdfRef { .. } => Err("PDF 页面暂不支持".to_string()),
    }
}
