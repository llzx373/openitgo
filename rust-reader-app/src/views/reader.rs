use crate::cache::PageCache;
use crate::loader::PageLoader;
use crate::timing;
use crate::widgets::progress_bar::{comic_progress_bar, page_at_x, ProgressBarResponse};
use crate::widgets::thumbnail_progress_bar::page_thumbnail_tooltip;
use rust_reader_core::layout;
use rust_reader_core::models::{Comic, FitMode, ReadingMode};
use rust_reader_core::state::{ReadingState, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 5.0;
const FALLBACK_PAGE_SIZE: egui::Vec2 = egui::Vec2::new(600.0, 800.0);

/// How long to pause preloads after the user turns a page. This prevents
/// rapid flips from getting stuck behind low-priority preload decode jobs.
const PRELOAD_COOLDOWN_AFTER_TURN: Duration = Duration::from_millis(100);
/// How long a page may stay in the pending state before we assume the result
/// was lost and allow a retry.
const PENDING_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct ReaderView {
    pub open: Option<OpenReader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDirection {
    Next,
    Prev,
}

#[derive(Debug, Clone, Copy)]
pub struct PageAnimation {
    pub from_page: usize,
    pub to_page: usize,
    pub direction: TurnDirection,
    pub start_time: std::time::Instant,
}

impl PageAnimation {
    pub const DURATION: std::time::Duration = std::time::Duration::from_millis(250);

    pub fn progress(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let t = (elapsed / Self::DURATION.as_secs_f32()).clamp(0.0, 1.0);
        // Ease-out cubic.
        1.0 - (1.0 - t).powi(3)
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= Self::DURATION
    }
}

#[derive(Debug, Clone)]
pub struct PageErrorRetry {
    pub count: u32,
    pub last_retry: Instant,
}

#[derive(Debug, Clone)]
pub struct ThumbnailError {
    pub retries: u32,
    pub last_retry: Instant,
}

pub struct OpenReader {
    pub comic: Comic,
    pub state: ReadingState,
    pub left_page: Option<usize>,
    pub right_page: Option<usize>,
    pub pending_fit: Option<FitMode>,
    pub current_epoch: u64,
    /// Full-resolution pages that have been requested but not yet loaded.
    pub pending_pages: HashMap<usize, Instant>,
    /// Thumbnail requests that are currently in flight.
    pub pending_thumbnails: HashSet<usize>,
    /// Next page index to request during the background thumbnail batch.
    pub thumbnail_batch_next: usize,
    pub page_errors: HashMap<usize, String>,
    pub page_error_retries: HashMap<usize, PageErrorRetry>,
    pub thumbnail_errors: HashMap<usize, ThumbnailError>,
    pub cache: PageCache,
    pub page_animation: Option<PageAnimation>,
    /// When the user last turned a page. Used to pause preloads briefly so
    /// rapid flips don't get stuck behind background preload decoding.
    pub last_page_turn: Instant,
    /// Vertical scroll offset for Webtoon mode, measured from the top of page 0.
    pub webtoon_scroll_offset: f32,
    /// The last `current_page` used to sync Webtoon scroll after keyboard nav.
    pub webtoon_last_page: usize,
    /// Last seen viewport size, used to re-apply the current fit on resize.
    pub last_available_size: Option<egui::Vec2>,
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

    pub fn request_fit(&mut self, fit: FitMode) {
        self.pending_fit = Some(fit);
    }

    fn mark_page_turn(&mut self) {
        self.last_page_turn = Instant::now();
    }

    pub fn first_page(&mut self) {
        let total = self.total_pages();
        if total > 0 {
            self.state.go_to_page(0, total);
            self.mark_page_turn();
        }
    }

    pub fn last_page(&mut self) {
        let total = self.total_pages();
        if total > 0 {
            self.state.go_to_page(total - 1, total);
            self.mark_page_turn();
        }
    }

    fn is_double_page(&self) -> bool {
        self.state.is_double_page()
    }

    fn can_animate_turn(&self) -> bool {
        !self.state.mode.is_webtoon() && !self.is_double_page()
    }

    pub fn next_page_with_animation(&mut self) {
        let from = self.state.current_page;
        let total = self.total_pages();
        self.state.next_page(total);
        let to = self.state.current_page;
        self.mark_page_turn();
        self.start_turn_animation(from, to);
    }

    pub fn prev_page_with_animation(&mut self) {
        let from = self.state.current_page;
        self.state.prev_page();
        let to = self.state.current_page;
        self.mark_page_turn();
        self.start_turn_animation(from, to);
    }

    fn start_turn_animation(&mut self, from: usize, to: usize) {
        if from == to || !self.can_animate_turn() {
            self.page_animation = None;
            return;
        }
        let direction = if to > from {
            TurnDirection::Next
        } else {
            TurnDirection::Prev
        };
        self.page_animation = Some(PageAnimation {
            from_page: from,
            to_page: to,
            direction,
            start_time: std::time::Instant::now(),
        });
    }

    /// Returns the page indices to display in left and right slots.
    ///
    /// In double-page mode, the first page (cover) is shown alone on the
    /// reading side; subsequent spreads use two pages.
    fn spread_pages(&self) -> (Option<usize>, Option<usize>) {
        let current = self.state.current_page;
        let total = self.total_pages();
        if total == 0 {
            return (None, None);
        }
        if !self.is_double_page() {
            return (Some(current), None);
        }
        if current == 0 {
            // Cover page shown alone on the reading side.
            match self.state.mode {
                ReadingMode::Ltr => (None, Some(0)),
                ReadingMode::Rtl => (Some(0), None),
                ReadingMode::Webtoon => (Some(current), None),
            }
        } else {
            let next = (current + 1).min(total - 1);
            match self.state.mode {
                ReadingMode::Ltr => (Some(current), Some(next)),
                ReadingMode::Rtl => (Some(next), Some(current)),
                ReadingMode::Webtoon => (Some(current), None),
            }
        }
    }

    fn spread_size(&self) -> Option<egui::Vec2> {
        let left_size = self.cache.get_original_size(self.left_page?)?;
        let right_size = self
            .right_page
            .and_then(|idx| self.cache.get_original_size(idx))
            .unwrap_or([0, 0]);
        Some(egui::vec2(
            (left_size[0] + right_size[0]) as f32,
            left_size[1].max(right_size[1]) as f32,
        ))
    }

    fn apply_pending_fit(&mut self, _ctx: &egui::Context, available: egui::Vec2) {
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
            FitMode::Width => available.x / spread_size.x,
            FitMode::Height => available.y / spread_size.y,
            FitMode::Page => (available.x / spread_size.x).min(available.y / spread_size.y),
            FitMode::Original => 1.0,
        };
        self.state.zoom = scale.clamp(MIN_ZOOM, MAX_ZOOM);
        self.state.fit_mode = fit;
        self.state.pan = Vec2::ZERO;
    }

    pub fn bump_epoch(&mut self, loader: &PageLoader) {
        self.current_epoch = loader.next_epoch();
        self.pending_pages.clear();
        self.pending_thumbnails.clear();
        self.thumbnail_batch_next = 0;
        self.page_errors.clear();
        self.left_page = None;
        self.right_page = None;
    }

    fn protected_page_indices(&self) -> Vec<usize> {
        let mut v = Vec::with_capacity(4);
        let (left, right) = self.spread_pages();
        if let Some(i) = left {
            v.push(i);
        }
        if let Some(i) = right {
            v.push(i);
        }
        if let Some(a) = self.page_animation {
            v.push(a.from_page);
            v.push(a.to_page);
        }
        v
    }

    pub fn update(&mut self, ctx: &egui::Context, loader: &PageLoader, cache_size_bytes: usize) {
        let _ = ctx;
        crate::timing::log_if_slow(
            "reader.update recv+insert",
            Duration::from_millis(5),
            || {
                while let Some(result) = loader.try_recv() {
                    if result.epoch != self.current_epoch {
                        timing::log(&format!(
                            "reader dropped stale result page {} epoch {} (current {})",
                            result.page_index, result.epoch, self.current_epoch
                        ));
                        continue;
                    }
                    timing::log(&format!(
                        "reader received result page {} thumbnail={}: {:?}",
                        result.page_index,
                        result.thumbnail,
                        result.image.as_ref().map(|_| "Ok").map_err(|e| e.as_str())
                    ));
                    if result.thumbnail {
                        self.pending_thumbnails.remove(&result.page_index);
                    } else {
                        self.pending_pages.remove(&result.page_index);
                    }

                    // Transient backpressure: the job was dropped before decoding.
                    // Don't record an error; the preload/thumbnail loop will retry.
                    if result.dropped {
                        continue;
                    }

                    match result.image {
                        Ok(image) => {
                            if result.thumbnail {
                                self.cache.insert_thumbnail(
                                    result.page_index,
                                    image,
                                    result.original_size,
                                );
                                self.thumbnail_errors.remove(&result.page_index);
                            } else {
                                let protected = self.protected_page_indices();
                                self.cache.insert_full(
                                    result.page_index,
                                    image,
                                    cache_size_bytes,
                                    &protected,
                                );
                                self.page_errors.remove(&result.page_index);
                                self.page_error_retries.remove(&result.page_index);
                            }
                        }
                        Err(err) => {
                            let now = Instant::now();
                            if result.thumbnail {
                                eprintln!(
                                    "failed to load thumbnail {}: {}",
                                    result.page_index, err
                                );
                                self.thumbnail_errors
                                    .entry(result.page_index)
                                    .and_modify(|e| {
                                        e.retries += 1;
                                        e.last_retry = now;
                                    })
                                    .or_insert(ThumbnailError {
                                        retries: 1,
                                        last_retry: now,
                                    });
                            } else {
                                eprintln!("failed to load page {}: {}", result.page_index, err);
                                self.page_errors.insert(result.page_index, err.clone());
                                self.page_error_retries
                                    .entry(result.page_index)
                                    .and_modify(|e| {
                                        e.count += 1;
                                        e.last_retry = now;
                                    })
                                    .or_insert(PageErrorRetry {
                                        count: 1,
                                        last_retry: now,
                                    });
                            }
                        }
                    }
                }

                // Time out any pending pages whose results never arrived, so the
                // UI can retry instead of staying stuck forever.
                let now = Instant::now();
                let timed_out: Vec<usize> = self
                    .pending_pages
                    .iter()
                    .filter(|(_, &since)| now.duration_since(since) >= PENDING_TIMEOUT)
                    .map(|(&page, _)| page)
                    .collect();
                for page in timed_out {
                    timing::log(&format!(
                        "reader pending page {} timed out, allowing retry",
                        page
                    ));
                    self.pending_pages.remove(&page);
                }
            },
        );
    }
}

impl ReaderView {
    /// Open a new comic, clearing any previous reader's cache first.
    pub fn open(
        &mut self,
        ctx: &egui::Context,
        comic: Comic,
        state: ReadingState,
        loader: &PageLoader,
    ) {
        let _ = ctx;
        timing::log(&format!(
            "ReaderView::open total_pages={}",
            comic.total_pages()
        ));
        self.clear_cache();
        let mut reader = OpenReader {
            comic,
            state,
            left_page: None,
            right_page: None,
            pending_fit: Some(state.fit_mode),
            current_epoch: 0,
            pending_pages: HashMap::new(),
            pending_thumbnails: HashSet::new(),
            thumbnail_batch_next: 0,
            page_errors: HashMap::new(),
            page_error_retries: HashMap::new(),
            thumbnail_errors: HashMap::new(),
            cache: PageCache::new(),
            page_animation: None,
            last_page_turn: Instant::now(),
            webtoon_scroll_offset: 0.0,
            webtoon_last_page: state.current_page,
            last_available_size: None,
        };
        reader.bump_epoch(loader);
        self.open = Some(reader);
    }

    /// Clear all cached images to free GPU memory, but keep the current reader
    /// open so the user can resume reading.
    pub fn clear_cache(&mut self) {
        if let Some(reader) = self.open.as_mut() {
            reader.cache.clear();
        }
    }

    /// Fully close the reader: clear cache and drop the open comic.
    pub fn close(&mut self) {
        self.clear_cache();
        self.open = None;
    }

    pub fn update(&mut self, ctx: &egui::Context, loader: &PageLoader, cache_size_mb: usize) {
        let budget = cache_size_mb * 1024 * 1024;
        if let Some(reader) = &mut self.open {
            reader.update(ctx, loader, budget);
        }
    }

    pub fn request_preloads(
        &mut self,
        loader: &PageLoader,
        cache_size_mb: usize,
        real_image_cache_pages: usize,
    ) {
        let Some(reader) = self.open.as_mut() else {
            return;
        };
        crate::timing::log_if_slow("reader.request_preloads", Duration::from_millis(5), || {
            let budget = cache_size_mb * 1024 * 1024;

            // Evict stale full pages so preloads can continue instead of giving up
            // when the cache is full.
            reader
                .cache
                .enforce_budget_with_protected(budget, &reader.protected_page_indices());

            // If the cache is already near capacity, stop preloading full images.
            // Otherwise the preload window (real_image_cache_pages each side) is
            // often larger than the budget can hold, causing a steady decode/evict
            // thrash that keeps CPU at 100% with no benefit.
            let free_budget = budget.saturating_sub(reader.cache.total_size_bytes());
            const MIN_FREE_BYTES_FOR_PRELOAD: usize = 64 * 1024 * 1024;
            if free_budget < MIN_FREE_BYTES_FOR_PRELOAD {
                return;
            }

            let current = reader.state.current_page;
            let total = reader.total_pages();
            if total == 0 {
                return;
            }

            // Background batch: generate thumbnails for every page. This is gated
            // so we never flood the low-priority queue in a single frame.
            const THUMBNAILS_PER_FRAME: usize = 32;
            let mut thumb_enqueued = 0;
            while reader.thumbnail_batch_next < total && thumb_enqueued < THUMBNAILS_PER_FRAME {
                let idx = reader.thumbnail_batch_next;
                reader.thumbnail_batch_next += 1;
                if reader.cache.contains_thumbnail(idx)
                    || reader.cache.contains_full(idx)
                    || reader.pending_thumbnails.contains(&idx)
                {
                    continue;
                }
                let Some(source) = reader.comic.page_source(idx).cloned() else {
                    continue;
                };
                if loader.request_thumbnail(reader.current_epoch, idx, source) {
                    reader.pending_thumbnails.insert(idx);
                    thumb_enqueued += 1;
                } else {
                    // Channel is full; retry next frame from the same index.
                    reader.thumbnail_batch_next = idx;
                    break;
                }
            }

            // Don't preload full-resolution pages until the current page is ready,
            // so preloads cannot delay the visible spread.
            if !reader.cache.contains_full(current) {
                return;
            }

            // During rapid page turns, pause full preloads briefly so decode workers
            // are available for the newly visible pages.
            if reader.last_page_turn.elapsed() < PRELOAD_COOLDOWN_AFTER_TURN {
                return;
            }

            // Cap the preload window by the remaining cache budget. Preloading more
            // pages than can fit causes a steady decode/evict thrash that keeps CPU
            // at 100% with no real benefit.
            let avg_full = reader.cache.average_full_size_bytes().max(8 * 1024 * 1024);
            let max_offset = (free_budget / avg_full)
                .min(real_image_cache_pages)
                .min(total.saturating_sub(1));
            if max_offset == 0 {
                return;
            }

            // Throttle preloads so the UI thread never blocks on a full low-priority
            // channel and so we don't starve the current page decode queue.
            let mut enqueued = 0;
            const MAX_PRELOADS_PER_FRAME: usize = 8;

            // Direction-aware asymmetric preloading: spend decode resources on the
            // side the user is most likely to turn to next first. For Ltr that is
            // forward (current + offset); for Rtl it is backward (current - offset).
            let mut try_preload = |idx: usize| -> bool {
                if idx >= total || idx == current {
                    return false;
                }
                if reader.cache.contains_full(idx) || reader.pending_pages.contains_key(&idx) {
                    return false;
                }
                let Some(source) = reader.comic.page_source(idx).cloned() else {
                    return false;
                };
                if loader.request_low(reader.current_epoch, idx, source) {
                    reader.pending_pages.insert(idx, Instant::now());
                    true
                } else {
                    false
                }
            };

            match reader.state.mode {
                ReadingMode::Ltr | ReadingMode::Webtoon => {
                    for offset in 1..=max_offset {
                        if enqueued >= MAX_PRELOADS_PER_FRAME {
                            break;
                        }
                        if try_preload(current.saturating_add(offset)) {
                            enqueued += 1;
                        }
                    }
                    for offset in 1..=max_offset {
                        if enqueued >= MAX_PRELOADS_PER_FRAME {
                            break;
                        }
                        if try_preload(current.saturating_sub(offset)) {
                            enqueued += 1;
                        }
                    }
                }
                ReadingMode::Rtl => {
                    for offset in 1..=max_offset {
                        if enqueued >= MAX_PRELOADS_PER_FRAME {
                            break;
                        }
                        if try_preload(current.saturating_sub(offset)) {
                            enqueued += 1;
                        }
                    }
                    for offset in 1..=max_offset {
                        if enqueued >= MAX_PRELOADS_PER_FRAME {
                            break;
                        }
                        if try_preload(current.saturating_add(offset)) {
                            enqueued += 1;
                        }
                    }
                }
            }
        });
    }

    /// Renders the current page or spread and returns the response covering the page area.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        loader: &PageLoader,
    ) -> Option<egui::Response> {
        crate::timing::log_if_slow("reader.ui", Duration::from_millis(5), || {
            self.ui_inner(ctx, ui, loader)
        })
    }

    fn ui_inner(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        loader: &PageLoader,
    ) -> Option<egui::Response> {
        let Some(reader) = &mut self.open else {
            ui.label("未打开漫画");
            return None;
        };

        let total_pages = reader.total_pages();
        if total_pages == 0 {
            ui.label("此漫画没有页面");
            return None;
        }

        if reader.state.mode.is_webtoon() {
            return self.render_webtoon(ctx, ui, loader);
        }

        if reader.page_animation.is_some() && reader.can_animate_turn() {
            return self.render_page_turn_animation(ctx, ui, loader);
        }

        let (left_idx, right_idx) = reader.spread_pages();
        let spread_changed = reader.left_page != left_idx || reader.right_page != right_idx;
        if spread_changed {
            timing::log(&format!(
                "reader spread changed: left={:?} right={:?} double_page={} current={}",
                left_idx,
                right_idx,
                reader.is_double_page(),
                reader.state.current_page
            ));
        }
        let left_texture = left_idx.and_then(|idx| reader.cache.get_texture(ctx, idx));
        let right_texture = right_idx.and_then(|idx| reader.cache.get_texture(ctx, idx));

        // Request full-resolution visible pages every frame until they are cached.
        // Also request a high-priority thumbnail if nothing is available yet, so
        // the user sees a low-res preview quickly.
        if let Some(idx) = left_idx {
            if !reader.cache.contains_full(idx) {
                request_page(loader, reader, idx);
            }
            if left_texture.is_none() && !reader.pending_thumbnails.contains(&idx) {
                request_page_thumbnail(loader, reader, idx);
            }
        }
        if let Some(idx) = right_idx {
            if !reader.cache.contains_full(idx) {
                request_page(loader, reader, idx);
            }
            if right_texture.is_none() && !reader.pending_thumbnails.contains(&idx) {
                request_page_thumbnail(loader, reader, idx);
            }
        }

        reader.left_page = left_idx;
        reader.right_page = right_idx;
        if spread_changed {
            reader.pending_fit = reader.pending_fit.or(Some(FitMode::Page));
        }

        let available = ui.available_rect_before_wrap();
        let available_size = available.size();
        if reader
            .last_available_size
            .map(|s| s != available_size)
            .unwrap_or(false)
        {
            reader.pending_fit = reader.pending_fit.or(Some(reader.state.fit_mode));
        }
        reader.last_available_size = Some(available_size);

        let left_size = left_idx
            .and_then(|idx| reader.cache.get_original_size(idx))
            .map(|s| egui::vec2(s[0] as f32, s[1] as f32))
            .unwrap_or(FALLBACK_PAGE_SIZE);
        let right_size = match right_idx {
            None => egui::Vec2::ZERO,
            Some(idx) => reader
                .cache
                .get_original_size(idx)
                .map(|s| egui::vec2(s[0] as f32, s[1] as f32))
                .unwrap_or(FALLBACK_PAGE_SIZE),
        };

        let any_loading = (left_idx.is_some() && left_texture.is_none())
            || (right_idx.is_some() && right_texture.is_none());
        if !any_loading {
            reader.apply_pending_fit(ctx, available.size());
        }

        let spread_size = egui::vec2(left_size.x + right_size.x, left_size.y.max(right_size.y));
        let scaled_spread = spread_size * reader.state.zoom;

        let max_pan_x = ((scaled_spread.x - available.width()) / 2.0).max(0.0);
        let max_pan_y = ((scaled_spread.y - available.height()) / 2.0).max(0.0);
        reader.state.pan.x = reader.state.pan.x.clamp(-max_pan_x, max_pan_x);
        reader.state.pan.y = reader.state.pan.y.clamp(-max_pan_y, max_pan_y);

        let center = available.center();
        let spread_top_left = egui::pos2(
            center.x - scaled_spread.x / 2.0 + reader.state.pan.x,
            center.y - scaled_spread.y / 2.0 + reader.state.pan.y,
        );

        // Render left page.
        let mut responses: Vec<egui::Response> = Vec::new();
        if let Some(idx) = left_idx {
            let left_rect =
                egui::Rect::from_min_size(spread_top_left, left_size * reader.state.zoom);
            responses.push(render_page_or_placeholder(
                ui,
                reader,
                loader,
                left_rect,
                idx,
                left_texture.as_ref(),
            ));
        }

        // Render right page if present.
        if let Some(idx) = right_idx {
            let right_top_left = if left_idx.is_some() {
                egui::pos2(
                    spread_top_left.x + left_size.x * reader.state.zoom,
                    spread_top_left.y,
                )
            } else {
                spread_top_left
            };
            let right_rect =
                egui::Rect::from_min_size(right_top_left, right_size * reader.state.zoom);
            responses.push(render_page_or_placeholder(
                ui,
                reader,
                loader,
                right_rect,
                idx,
                right_texture.as_ref(),
            ));
        }

        // Return a response that covers all visible pages.
        if responses.is_empty() {
            None
        } else {
            let mut combined = responses.remove(0);
            for r in responses {
                combined = combined.union(r);
            }
            Some(combined)
        }
    }

    fn render_webtoon(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        loader: &PageLoader,
    ) -> Option<egui::Response> {
        let reader = self.open.as_mut()?;
        let total = reader.total_pages();
        if total == 0 {
            return None;
        }

        let available = ui.available_rect_before_wrap();
        let viewport_size = Vec2::new(available.width(), available.height());
        let page_sizes: Vec<Vec2> = (0..total)
            .map(|idx| {
                reader
                    .cache
                    .get_original_size(idx)
                    .map(|s| Vec2::new(s[0] as f32, s[1] as f32))
                    .unwrap_or_else(|| Vec2::new(FALLBACK_PAGE_SIZE.x, FALLBACK_PAGE_SIZE.y))
            })
            .collect();

        let layouts = layout::compute_layout(
            ReadingMode::Webtoon,
            viewport_size,
            &page_sizes,
            reader.state.zoom,
        );
        let content_height = layouts
            .last()
            .map(|l| l.rect.min.y + l.rect.size.y)
            .unwrap_or(0.0);
        let max_offset = (content_height - available.height()).max(0.0);

        // Sync scroll offset when keyboard navigation changes the current page.
        if reader.webtoon_last_page != reader.state.current_page {
            if let Some(layout) = layouts.get(reader.state.current_page) {
                reader.webtoon_scroll_offset = layout.rect.min.y;
            }
            reader.webtoon_last_page = reader.state.current_page;
        }

        // Apply scroll wheel input.
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_delta != 0.0 {
            reader.webtoon_scroll_offset =
                (reader.webtoon_scroll_offset - scroll_delta * 3.0).clamp(0.0, max_offset);
        }
        reader.webtoon_scroll_offset = reader.webtoon_scroll_offset.clamp(0.0, max_offset);

        // Update current_page based on what is centered in the viewport.
        let center_y = reader.webtoon_scroll_offset + available.height() / 2.0;
        if let Some(layout) = layouts
            .iter()
            .find(|l| l.rect.min.y <= center_y && l.rect.min.y + l.rect.size.y > center_y)
        {
            reader.state.current_page = layout.page_index;
            reader.webtoon_last_page = layout.page_index;
        }

        let top = reader.webtoon_scroll_offset;
        let bottom = top + available.height();
        let visible_indices: Vec<usize> = layouts
            .iter()
            .filter(|l| {
                let page_top = l.rect.min.y;
                let page_bottom = page_top + l.rect.size.y;
                page_bottom > top && page_top < bottom
            })
            .map(|l| l.page_index)
            .collect();

        // Request thumbnails and full-resolution images for visible pages.
        for &idx in &visible_indices {
            if !reader.cache.contains_full(idx) {
                request_page(loader, reader, idx);
            }
            if reader.cache.get_texture(ctx, idx).is_none()
                && !reader.pending_thumbnails.contains(&idx)
            {
                request_page_thumbnail(loader, reader, idx);
            }
        }

        let mut combined_response: Option<egui::Response> = None;
        for idx in visible_indices {
            let layout = &layouts[idx];
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    available.min.x + layout.rect.min.x,
                    available.min.y + layout.rect.min.y - top,
                ),
                egui::vec2(layout.rect.size.x, layout.rect.size.y),
            );
            let texture = reader.cache.get_texture(ctx, idx);
            let response =
                render_page_or_placeholder(ui, reader, loader, rect, idx, texture.as_ref());
            combined_response = Some(match combined_response {
                Some(prev) => prev.union(response),
                None => response,
            });
        }

        combined_response
    }

    fn render_page_turn_animation(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        loader: &PageLoader,
    ) -> Option<egui::Response> {
        let reader = self.open.as_mut()?;
        let animation = reader.page_animation?;
        if animation.is_finished() {
            reader.page_animation = None;
            return None;
        }
        let progress = animation.progress();
        let from_idx = animation.from_page;
        let to_idx = animation.to_page;

        let from_texture = reader.cache.get_texture(ctx, from_idx);
        let to_texture = reader.cache.get_texture(ctx, to_idx);
        if !reader.cache.contains_full(from_idx) {
            request_page(loader, reader, from_idx);
        }
        if from_texture.is_none() && !reader.pending_thumbnails.contains(&from_idx) {
            request_page_thumbnail(loader, reader, from_idx);
        }
        if !reader.cache.contains_full(to_idx) {
            request_page(loader, reader, to_idx);
        }
        if to_texture.is_none() && !reader.pending_thumbnails.contains(&to_idx) {
            request_page_thumbnail(loader, reader, to_idx);
        }
        // Use cached original sizes for layout; fall back to placeholder sizes if
        // the metadata has not arrived yet.
        let from_size = reader
            .cache
            .get_original_size(from_idx)
            .map(|s| egui::vec2(s[0] as f32, s[1] as f32))
            .unwrap_or(FALLBACK_PAGE_SIZE);
        let to_size = reader
            .cache
            .get_original_size(to_idx)
            .map(|s| egui::vec2(s[0] as f32, s[1] as f32))
            .unwrap_or(FALLBACK_PAGE_SIZE);

        let available = ui.available_rect_before_wrap();
        let scale = (available.width() / to_size.x)
            .min(available.height() / to_size.y)
            .min(1.0);
        let to_scaled = to_size * scale;
        let from_scaled = from_size * scale;

        let center = available.center();
        let direction_sign = match animation.direction {
            TurnDirection::Next => -1.0,
            TurnDirection::Prev => 1.0,
        };
        let offset = direction_sign * progress * available.width();

        let from_rect = egui::Rect::from_min_size(
            egui::pos2(
                center.x - from_scaled.x / 2.0 + offset,
                center.y - from_scaled.y / 2.0,
            ),
            from_scaled,
        );
        let from_response = render_page_or_placeholder(
            ui,
            reader,
            loader,
            from_rect,
            from_idx,
            from_texture.as_ref(),
        );

        let to_rect = egui::Rect::from_min_size(
            egui::pos2(
                center.x - to_scaled.x / 2.0 + offset - direction_sign * available.width(),
                center.y - to_scaled.y / 2.0,
            ),
            to_scaled,
        );
        let to_response =
            render_page_or_placeholder(ui, reader, loader, to_rect, to_idx, to_texture.as_ref());

        Some(from_response.union(to_response))
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
                    reader.mark_page_turn();
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
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        hovered_page: Option<usize>,
    ) -> Option<egui::Response> {
        let reader = self.open.as_mut()?;
        let page_index = hovered_page?;
        let pointer_pos = ui.input(|i| i.pointer.hover_pos())?;
        Some(page_thumbnail_tooltip(
            ui,
            ctx,
            &mut reader.cache,
            page_index,
            pointer_pos,
        ))
    }
}

fn request_page(loader: &PageLoader, reader: &mut OpenReader, page_index: usize) {
    let total = reader.total_pages();
    if page_index >= total {
        return;
    }
    if let Some(&since) = reader.pending_pages.get(&page_index) {
        if since.elapsed() < PENDING_TIMEOUT {
            return;
        }
        // Timed out: drop the stale pending entry so we can retry immediately.
        reader.pending_pages.remove(&page_index);
    }
    let Some(source) = reader.comic.page_source(page_index).cloned() else {
        return;
    };
    timing::log(&format!(
        "reader request_high page {} epoch {}",
        page_index, reader.current_epoch
    ));
    if loader.request_high(reader.current_epoch, page_index, source) {
        reader.pending_pages.insert(page_index, Instant::now());
        reader.page_errors.remove(&page_index);
    }
}

fn request_page_thumbnail(loader: &PageLoader, reader: &mut OpenReader, page_index: usize) {
    let total = reader.total_pages();
    if page_index >= total {
        return;
    }
    if reader.pending_thumbnails.contains(&page_index) {
        return;
    }
    let Some(source) = reader.comic.page_source(page_index).cloned() else {
        return;
    };
    timing::log(&format!(
        "reader request_thumbnail_high page {} epoch {}",
        page_index, reader.current_epoch
    ));
    if loader.request_thumbnail_high(reader.current_epoch, page_index, source) {
        reader.pending_thumbnails.insert(page_index);
    }
}

fn error_retry_backoff(count: u32) -> Duration {
    let seconds = 2u32.pow(count.min(5)).min(30);
    Duration::from_secs(seconds as u64)
}

fn should_retry_page_error(reader: &OpenReader, page_index: usize) -> bool {
    match reader.page_error_retries.get(&page_index) {
        Some(retry) => retry.last_retry.elapsed() >= error_retry_backoff(retry.count),
        None => true,
    }
}

fn should_retry_thumbnail_error(reader: &OpenReader, page_index: usize) -> bool {
    match reader.thumbnail_errors.get(&page_index) {
        Some(err) => err.last_retry.elapsed() >= error_retry_backoff(err.retries),
        None => true,
    }
}

fn render_page_or_placeholder(
    ui: &mut egui::Ui,
    reader: &mut OpenReader,
    loader: &PageLoader,
    rect: egui::Rect,
    page_index: usize,
    texture: Option<&egui::TextureHandle>,
) -> egui::Response {
    // If the full-resolution page is already cached, always prefer it over a
    // thumbnail that may have been passed in from an earlier frame.
    let full_texture = if reader.cache.contains_full(page_index) {
        reader.cache.get_texture(ui.ctx(), page_index)
    } else {
        None
    };
    let texture = full_texture.as_ref().or(texture);

    if let Some(texture) = texture {
        let response = ui.put(
            rect,
            egui::Image::new(texture)
                .fit_to_exact_size(rect.size())
                .sense(egui::Sense::click_and_drag()),
        );
        if response.dragged() {
            let delta = response.drag_delta();
            reader.state.pan += Vec2::new(delta.x, delta.y);
        }
        response
    } else if let Some(err) = reader.page_errors.get(&page_index).cloned() {
        let can_retry = should_retry_page_error(reader, page_index);
        render_error_placeholder(ui, rect, &err, || {
            if can_retry {
                request_page(loader, reader, page_index);
            }
        })
    } else if reader.thumbnail_errors.contains_key(&page_index) {
        let can_retry = should_retry_thumbnail_error(reader, page_index);
        render_thumbnail_error_placeholder(ui, rect, || {
            if can_retry {
                request_page_thumbnail(loader, reader, page_index);
                if let Some(err) = reader.thumbnail_errors.get_mut(&page_index) {
                    err.retries += 1;
                    err.last_retry = Instant::now();
                }
            }
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

fn render_thumbnail_error_placeholder(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    mut retry: impl FnMut(),
) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                ui.colored_label(ui.visuals().error_fg_color, "缩略图加载失败");
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
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                // Use a static icon instead of ui.spinner() to avoid forcing a
                // continuous repaint while waiting for the decode thread.
                ui.label(egui::RichText::new("⏳").size(24.0));
                ui.label("加载中...");
            },
        );
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_core::models::{Comic, Page, PageSource, Volume};
    use rust_reader_core::state::ReadingState;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    fn dummy_reader() -> OpenReader {
        OpenReader {
            comic: Comic {
                id: "test".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp/test"),
                volumes: vec![Volume {
                    title: "Vol 1".to_string(),
                    pages: (0..10)
                        .map(|i| Page {
                            index: i,
                            source: PageSource::File(PathBuf::from(format!("page{}.png", i))),
                        })
                        .collect(),
                }],
            },
            state: ReadingState::new(ReadingMode::Ltr, 10),
            left_page: None,
            right_page: None,
            pending_fit: None,
            current_epoch: 0,
            pending_pages: HashMap::new(),
            pending_thumbnails: HashSet::new(),
            thumbnail_batch_next: 0,
            page_errors: HashMap::new(),
            page_error_retries: HashMap::new(),
            thumbnail_errors: HashMap::new(),
            cache: PageCache::new(),
            page_animation: None,
            last_page_turn: Instant::now(),
            webtoon_scroll_offset: 0.0,
            webtoon_last_page: 0,
            last_available_size: None,
        }
    }

    #[test]
    fn test_page_animation_progress_clamps_and_eases() {
        let animation = PageAnimation {
            from_page: 0,
            to_page: 1,
            direction: TurnDirection::Next,
            start_time: std::time::Instant::now(),
        };
        assert!(
            animation.progress() < 0.1,
            "progress should start near zero"
        );

        let animation = PageAnimation {
            start_time: std::time::Instant::now() - PageAnimation::DURATION,
            ..animation
        };
        assert!((animation.progress() - 1.0).abs() < 0.001);
        assert!(animation.is_finished());
    }

    #[test]
    fn test_next_page_starts_animation_in_single_page_mode() {
        let mut reader = dummy_reader();
        reader.state.set_double_page(false, 10);
        reader.next_page_with_animation();
        assert_eq!(reader.state.current_page, 1);
        assert!(reader.page_animation.is_some());
        let anim = reader.page_animation.unwrap();
        assert_eq!(anim.from_page, 0);
        assert_eq!(anim.to_page, 1);
        assert_eq!(anim.direction, TurnDirection::Next);
    }

    #[test]
    fn test_next_page_does_not_animate_in_double_page_mode() {
        let mut reader = dummy_reader();
        reader.state.set_double_page(true, 10);
        reader.next_page_with_animation();
        // Cover page is alone; next anchor is page 1.
        assert_eq!(reader.state.current_page, 1);
        assert!(reader.page_animation.is_none());
    }

    #[test]
    fn test_animation_clears_after_duration() {
        let mut reader = dummy_reader();
        reader.next_page_with_animation();
        thread::sleep(PageAnimation::DURATION + Duration::from_millis(10));
        assert!(reader.page_animation.as_ref().unwrap().is_finished());
    }

    #[test]
    fn test_spread_pages_cover_alone_in_double_page_ltr() {
        let mut reader = dummy_reader();
        reader.state.mode = ReadingMode::Ltr;
        reader.state.set_double_page(true, 10);
        assert_eq!(reader.spread_pages(), (None, Some(0)));

        reader.state.current_page = 1;
        assert_eq!(reader.spread_pages(), (Some(1), Some(2)));
    }

    #[test]
    fn test_spread_pages_cover_alone_in_double_page_rtl() {
        let mut reader = dummy_reader();
        reader.state.mode = ReadingMode::Rtl;
        reader.state.set_double_page(true, 10);
        assert_eq!(reader.spread_pages(), (Some(0), None));

        reader.state.current_page = 1;
        assert_eq!(reader.spread_pages(), (Some(2), Some(1)));
    }

    #[test]
    fn test_pending_page_timeout_allows_retry() {
        let mut reader = dummy_reader();
        let loader = PageLoader::new();
        // Simulate a pending entry whose result never arrived.
        reader
            .pending_pages
            .insert(0, Instant::now() - PENDING_TIMEOUT - Duration::from_secs(1));

        request_page(&loader, &mut reader, 0);

        let since = reader
            .pending_pages
            .get(&0)
            .expect("page should be pending again after retry");
        assert!(
            since.elapsed() < Duration::from_secs(1),
            "pending timestamp should be fresh"
        );
    }
}
