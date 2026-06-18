use crate::cache::PageCache;
use crate::loader::{Epoch, PageLoader};
use crate::widgets::page_view::upload_color_image;
use crate::widgets::progress_bar::{comic_progress_bar, ProgressBarResponse};
use crate::widgets::thumbnail_progress_bar::page_thumbnail_tooltip;
use rust_reader_core::models::{Comic, ReadingMode};
use rust_reader_core::state::{ReadingState, Vec2};
use std::collections::{HashMap, HashSet};

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
    pub current_epoch: Epoch,
    pub pending_pages: HashSet<usize>,
    pub page_errors: HashMap<usize, String>,
    pub cache: PageCache,
}

impl OpenReader {
    pub fn total_pages(&self) -> usize {
        self.comic.total_pages()
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

    pub fn bump_epoch(&mut self, loader: &PageLoader) {
        self.current_epoch = loader.next_epoch();
        self.pending_pages.clear();
        self.page_errors.clear();
        self.left_texture = None;
        self.right_texture = None;
        self.left_page = None;
        self.right_page = None;
    }

    pub fn update(&mut self, ctx: &egui::Context, loader: &PageLoader, cache_size_bytes: usize) {
        while let Some(result) = loader.try_recv() {
            if result.epoch != self.current_epoch {
                continue;
            }
            self.pending_pages.remove(&result.page_index);
            match result.image {
                Ok(image) => {
                    let texture =
                        upload_color_image(ctx, image, format!("page_{}", result.page_index));
                    self.cache
                        .insert(result.page_index, texture, cache_size_bytes);
                    if self.left_page == Some(result.page_index) {
                        self.left_texture = self.cache.get(result.page_index);
                    }
                    if self.right_page == Some(result.page_index) {
                        self.right_texture = self.cache.get(result.page_index);
                    }
                }
                Err(err) => {
                    eprintln!("failed to load page {}: {}", result.page_index, err);
                    self.page_errors.insert(result.page_index, err);
                }
            }
        }
    }
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState, loader: &PageLoader) {
        let mut reader = OpenReader {
            comic,
            state,
            left_texture: None,
            left_page: None,
            right_texture: None,
            right_page: None,
            pending_fit: Some(QuickFit::Page),
            current_epoch: 0,
            pending_pages: HashSet::new(),
            page_errors: HashMap::new(),
            cache: PageCache::new(),
        };
        reader.bump_epoch(loader);
        self.open = Some(reader);
    }

    pub fn update(&mut self, ctx: &egui::Context, loader: &PageLoader, cache_size_mb: u32) {
        let budget = cache_size_mb as usize * 1024 * 1024;
        if let Some(reader) = &mut self.open {
            reader.update(ctx, loader, budget);
        }
        self.enforce_cache_budget(cache_size_mb);
    }

    pub fn request_preloads(&mut self, loader: &PageLoader, cache_size_mb: u32) {
        let Some(reader) = self.open.as_mut() else {
            return;
        };
        let budget = cache_size_mb as usize * 1024 * 1024;
        reader.cache.enforce_budget(budget);
        if reader.cache.total_size_bytes() >= budget {
            return;
        }

        let current = reader.state.current_page;
        let total = reader.total_pages();
        if total == 0 {
            return;
        }

        for offset in 1..total {
            let candidates = [
                current.saturating_sub(offset),
                current.saturating_add(offset),
            ];
            for &idx in &candidates {
                if idx >= total {
                    continue;
                }
                if idx == current {
                    continue;
                }
                if reader.cache.contains(idx) || reader.pending_pages.contains(&idx) {
                    continue;
                }
                let Some(source) = reader.comic.page_source(idx).cloned() else {
                    continue;
                };
                reader.pending_pages.insert(idx);
                loader.request_low(reader.current_epoch, idx, source);
            }
        }
    }

    pub fn enforce_cache_budget(&mut self, cache_size_mb: u32) {
        let Some(reader) = self.open.as_mut() else {
            return;
        };
        let budget = cache_size_mb as usize * 1024 * 1024;
        reader.cache.enforce_budget(budget);
    }

    /// Renders the current page or spread and returns the response covering the page area.
    pub fn ui(&mut self, ui: &mut egui::Ui, loader: &PageLoader) -> Option<egui::Response> {
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
            reader.left_page = Some(left_idx);
            reader.left_texture = reader.cache.get(left_idx);
            if reader.left_texture.is_none() {
                request_page(loader, reader, left_idx);
            }
            reader.pending_fit = reader.pending_fit.or(Some(QuickFit::Page));
        }
        if reader.right_page != right_idx {
            reader.right_page = right_idx;
            reader.right_texture = right_idx.and_then(|idx| reader.cache.get(idx));
            if let Some(idx) = right_idx {
                if reader.right_texture.is_none() {
                    request_page(loader, reader, idx);
                }
            }
            reader.pending_fit = reader.pending_fit.or(Some(QuickFit::Page));
        }

        let available = ui.available_rect_before_wrap();

        let left_texture = reader.left_texture.clone();
        let right_texture = reader.right_texture.clone();
        let right_idx = reader.right_page;
        const FALLBACK_PAGE_SIZE: egui::Vec2 = egui::Vec2::new(600.0, 800.0);
        let left_size = left_texture
            .as_ref()
            .map(|t| t.size_vec2())
            .unwrap_or(FALLBACK_PAGE_SIZE);
        let right_size = match (right_idx, right_texture.as_ref()) {
            (None, _) => egui::Vec2::ZERO,
            (Some(_), None) => FALLBACK_PAGE_SIZE,
            (Some(_), Some(t)) => t.size_vec2(),
        };

        let any_loading =
            left_texture.is_none() || (right_idx.is_some() && right_texture.is_none());
        if !any_loading {
            reader.apply_pending_fit(available.size());
        }

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
        let left_response = render_page_or_placeholder(
            ui,
            reader,
            loader,
            left_rect,
            left_idx,
            left_texture.as_ref(),
        );

        // Render right page if present.
        let mut right_response: Option<egui::Response> = None;
        if let Some(idx) = right_idx {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(spread_top_left.x + left_rect.width(), spread_top_left.y),
                right_size * reader.state.zoom,
            );
            let response = render_page_or_placeholder(
                ui,
                reader,
                loader,
                right_rect,
                idx,
                right_texture.as_ref(),
            );
            right_response = Some(response);
        }

        // Return a response that covers both pages so context menu and drag work everywhere.
        match right_response {
            Some(right) => Some(left_response.union(right)),
            None => Some(left_response),
        }
    }

    pub fn render_progress_bar(&mut self, ui: &mut egui::Ui) -> ProgressBarResponse {
        let Some(reader) = &mut self.open else {
            return ProgressBarResponse {
                response: ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
                hovered_page: None,
            };
        };
        let total_pages = reader.total_pages();
        let current_page = reader.state.current_page;

        let ProgressBarResponse {
            response,
            hovered_page,
        } = comic_progress_bar(ui, current_page, total_pages);

        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let target = page_at_x(pos.x, response.rect, total_pages);
                if target != current_page {
                    reader.state.go_to_page(target, total_pages);
                    reader.left_page = None;
                    reader.right_page = None;
                }
            }
        }

        ProgressBarResponse {
            response,
            hovered_page,
        }
    }

    pub fn render_progress_thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        hovered_page: Option<usize>,
    ) -> Option<egui::Response> {
        let reader = self.open.as_mut()?;
        let page_index = hovered_page?;
        let pointer_pos = ui.input(|i| i.pointer.hover_pos())?;
        Some(page_thumbnail_tooltip(
            ui,
            &mut reader.cache,
            page_index,
            pointer_pos,
        ))
    }
}

fn page_at_x(x: f32, rect: egui::Rect, total_pages: usize) -> usize {
    if rect.width() <= 0.0 || total_pages == 0 {
        return 0;
    }
    let ratio = ((x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
    let page = (ratio * total_pages as f32).floor() as usize;
    page.min(total_pages - 1)
}

fn request_page(loader: &PageLoader, reader: &mut OpenReader, page_index: usize) {
    let total = reader.total_pages();
    if page_index >= total {
        return;
    }
    if reader.pending_pages.contains(&page_index) {
        return;
    }
    let Some(source) = reader.comic.page_source(page_index).cloned() else {
        return;
    };
    reader.pending_pages.insert(page_index);
    reader.page_errors.remove(&page_index);
    loader.request_high(reader.current_epoch, page_index, source);
}

fn render_page_or_placeholder(
    ui: &mut egui::Ui,
    reader: &mut OpenReader,
    loader: &PageLoader,
    rect: egui::Rect,
    page_index: usize,
    texture: Option<&egui::TextureHandle>,
) -> egui::Response {
    if let Some(texture) = texture {
        let response = ui.put(
            rect,
            egui::Image::new(texture)
                .fit_to_exact_size(rect.size())
                .sense(egui::Sense::drag()),
        );
        if response.dragged() {
            let delta = response.drag_delta();
            reader.state.pan += Vec2::new(delta.x, delta.y);
        }
        response
    } else if let Some(err) = reader.page_errors.get(&page_index).cloned() {
        render_error_placeholder(ui, rect, &err, || {
            request_page(loader, reader, page_index);
        })
    } else {
        render_loading_placeholder(ui, rect)
    }
}

fn render_error_placeholder(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    error: &str,
    mut retry: impl FnMut(),
) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                ui.colored_label(ui.visuals().error_fg_color, "加载失败");
                let short = if error.len() > 80 {
                    format!("{}...", &error[..80])
                } else {
                    error.to_string()
                };
                ui.label(egui::RichText::new(short).size(12.0));
                ui.label(egui::RichText::new("点击重试").size(12.0));
            },
        );
    });
    if response.clicked() {
        retry();
    }
    response
}

fn render_loading_placeholder(ui: &mut egui::Ui, rect: egui::Rect) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::drag());
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                ui.spinner();
                ui.label("加载中...");
            },
        );
    });
    response
}
