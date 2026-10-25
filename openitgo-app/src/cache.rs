use crate::loader::LoadedImage;
use crate::timing;
use eframe::wgpu;
use egui::{Context, TextureHandle};
use std::sync::Arc;

const WGPU_RENDER_STATE_KEY: &str = "openitgo_wgpu_render_state";

#[allow(dead_code)]
pub fn wgpu_options_with_bc3() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_setup {
        let base_descriptor = create.device_descriptor.clone();
        create.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = base_descriptor(adapter);
            if adapter
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
            {
                descriptor
                    .required_features
                    .insert(wgpu::Features::TEXTURE_COMPRESSION_BC);
            }
            descriptor
        });
    }
    options
}

pub fn install_wgpu_render_state(
    ctx: &Context,
    render_state: Option<eframe::egui_wgpu::RenderState>,
) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new(WGPU_RENDER_STATE_KEY), render_state);
    });
}

fn wgpu_render_state(ctx: &Context) -> Option<eframe::egui_wgpu::RenderState> {
    ctx.data(|data| {
        data.get_temp(egui::Id::new(WGPU_RENDER_STATE_KEY))
            .unwrap_or_default()
    })
}
fn upload_bc3_texture(
    ctx: &Context,
    page_index: usize,
    data: &[u8],
    gpu_size: [u32; 2],
) -> Option<(TextureHandle, wgpu::Texture)> {
    let render_state = wgpu_render_state(ctx)?;
    if !render_state
        .device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        return None;
    }
    let texture = render_state
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("page_{page_index}_bc3")),
            size: wgpu::Extent3d {
                width: gpu_size[0],
                height: gpu_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bc3RgbaUnorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    render_state.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(gpu_size[0] * 4),
            rows_per_image: Some(gpu_size[1]),
        },
        wgpu::Extent3d {
            width: gpu_size[0],
            height: gpu_size[1],
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let handle = ctx.load_texture(
        format!("page_{page_index}_bc3"),
        egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
        egui::TextureOptions::LINEAR,
    );
    let id = handle.id();
    render_state.renderer.write().update_texture(
        &render_state.device,
        &render_state.queue,
        id,
        &eframe::epaint::ImageDelta::full(
            egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
            egui::TextureOptions::LINEAR,
        ),
    );
    render_state
        .renderer
        .write()
        .update_egui_texture_from_wgpu_texture(
            &render_state.device,
            &view,
            wgpu::FilterMode::Linear,
            id,
        );
    Some((handle, texture))
}

use std::collections::HashMap;
use std::time::Instant;

const THUMBNAIL_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;

struct CacheEntry {
    /// The original page dimensions, used for layout before any full image is ready.
    original_size: [u32; 2],
    thumbnail: Option<LoadedImage>,
    thumbnail_handle: Option<TextureHandle>,
    /// The full-resolution image (counts toward the memory budget).
    image: Option<LoadedImage>,
    handle: Option<TextureHandle>,
    native_texture: Option<wgpu::Texture>,
    last_accessed: Instant,
    size_bytes: usize,
    thumbnail_size_bytes: usize,
}

impl CacheEntry {
    fn empty(original_size: [u32; 2]) -> Self {
        Self {
            original_size,
            thumbnail: None,
            thumbnail_handle: None,
            image: None,
            handle: None,
            native_texture: None,
            last_accessed: Instant::now(),
            size_bytes: 0,
            thumbnail_size_bytes: 0,
        }
    }
}

pub struct PageCache {
    textures: HashMap<usize, CacheEntry>,
    total_size_bytes: usize,
    thumbnail_total_size_bytes: usize,
    dimensions_version: u64,
}

impl PageCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            total_size_bytes: 0,
            thumbnail_total_size_bytes: 0,
            dimensions_version: 0,
        }
    }

    pub fn dimensions_version(&self) -> u64 {
        self.dimensions_version
    }

    #[allow(dead_code)]
    pub fn total_size_bytes(&self) -> usize {
        self.total_size_bytes
    }

    #[allow(dead_code)]
    pub fn thumbnail_total_size_bytes(&self) -> usize {
        self.thumbnail_total_size_bytes
    }

    #[allow(dead_code)]
    pub fn native_texture(&self, page_index: usize) -> Option<(&wgpu::Texture, [u32; 2])> {
        let entry = self.textures.get(&page_index)?;
        let texture = entry.native_texture.as_ref()?;
        let LoadedImage::Compressed { gpu_size, .. } = entry.image.as_ref()? else {
            return None;
        };
        Some((texture, *gpu_size))
    }

    pub fn texture_uv(&self, page_index: usize) -> egui::Rect {
        let Some(entry) = self.textures.get(&page_index) else {
            return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
        };
        if entry.native_texture.is_none() {
            return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
        }
        let Some(LoadedImage::Compressed {
            original_size,
            gpu_size,
            ..
        }) = entry.image.as_ref()
        else {
            return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
        };
        bc3_texture_uv(*original_size, *gpu_size)
    }

    pub fn contains_full(&self, page_index: usize) -> bool {
        self.textures
            .get(&page_index)
            .map(|e| e.image.is_some() || e.handle.is_some())
            .unwrap_or(false)
    }

    /// Average byte size of the currently cached full-resolution images.
    /// Returns 0 if no full image is cached yet.
    pub fn average_full_size_bytes(&self) -> usize {
        let (count, bytes) = self
            .textures
            .values()
            .filter(|e| e.image.is_some() || e.handle.is_some())
            .map(|e| e.size_bytes)
            .fold((0usize, 0usize), |(c, b), s| (c + 1, b + s));
        bytes.checked_div(count).unwrap_or(0)
    }

    pub fn contains_thumbnail(&self, page_index: usize) -> bool {
        self.textures
            .get(&page_index)
            .map(|e| e.thumbnail.is_some() || e.thumbnail_handle.is_some())
            .unwrap_or(false)
    }

    pub fn get_original_size(&self, page_index: usize) -> Option<[u32; 2]> {
        self.textures.get(&page_index).map(|e| e.original_size)
    }

    /// Seed the original dimensions for a page before any image data has been
    /// decoded. This lets the reader compute the correct fit zoom on the first
    /// frame after opening a comic.
    pub fn insert_dimensions(&mut self, page_index: usize, dimensions: [u32; 2]) {
        let entry = self
            .textures
            .entry(page_index)
            .or_insert_with(|| CacheEntry::empty(dimensions));
        entry.original_size = dimensions;
        self.dimensions_version += 1;
    }

    /// Returns the full texture if available, otherwise the thumbnail texture.
    pub fn get_texture(&mut self, ctx: &Context, page_index: usize) -> Option<TextureHandle> {
        let entry = self.textures.get_mut(&page_index)?;
        entry.last_accessed = Instant::now();

        if let Some(handle) = entry.handle.as_ref() {
            return Some(handle.clone());
        }
        if let Some(image) = entry.image.as_ref() {
            timing::log(&format!("cache upload full page {}", page_index));
            if let LoadedImage::Compressed { data, gpu_size, .. } = image {
                if let Some((handle, texture)) =
                    upload_bc3_texture(ctx, page_index, data, *gpu_size)
                {
                    entry.handle = Some(handle.clone());
                    entry.native_texture = Some(texture);
                    return Some(handle);
                }
            }
            let is_color = matches!(image, LoadedImage::Color(_));
            let color = timing::time("cache full decompress+upload", || {
                image.to_color_image().ok()
            })?;
            let handle = ctx.load_texture(
                format!("page_{}", page_index),
                color,
                egui::TextureOptions::LINEAR,
            );
            entry.handle = Some(handle.clone());
            // Release the CPU-side ColorImage once it has been uploaded to the
            // GPU. size_bytes is retained as an estimate of GPU texture memory,
            // so the total budget stays consistent. Compressed images are kept
            // so they can be re-uploaded if the texture handle is later evicted.
            if is_color {
                entry.image = None;
            }
            return Some(handle);
        }

        if let Some(handle) = entry.thumbnail_handle.as_ref() {
            return Some(handle.clone());
        }
        if let Some(image) = entry.thumbnail.as_ref() {
            timing::log(&format!("cache upload thumbnail page {}", page_index));
            let color = timing::time("cache thumbnail upload", || image.to_color_image().ok())?;
            let handle = ctx.load_texture(
                format!("page_{}_thumb", page_index),
                color,
                egui::TextureOptions::LINEAR,
            );
            entry.thumbnail_handle = Some(handle.clone());
            entry.thumbnail = None;
            return Some(handle);
        }

        None
    }

    pub fn insert_full(
        &mut self,
        page_index: usize,
        image: LoadedImage,
        // Preferred layout size (typically file-header dims from LoadResult).
        // When the bitmap was capped at MAX_IMAGE_DIMENSION, this keeps
        // fit/zoom calibrated to the true page size.
        layout_size: [u32; 2],
        max_size_bytes: usize,
        protected: &std::collections::HashSet<usize>,
    ) {
        let new_size = image.size_bytes();
        let decoded_size = image.original_size();
        timing::log(&format!(
            "cache insert full page {} size {} budget {} layout {:?} decoded {:?}",
            page_index, new_size, max_size_bytes, layout_size, decoded_size
        ));

        // Prefer an already-seeded / thumbnail-reported size over a smaller
        // downsampled decode so pending fit zoom stays stable across the
        // thumb→full transition.
        let existing_layout = self.textures.get(&page_index).map(|e| e.original_size);
        let original_size = prefer_layout_size(existing_layout, layout_size, decoded_size);

        // Remove any existing full-resolution data for this page first so the
        // budget check accounts for the replacement. size_bytes tracks either
        // CPU image memory or the equivalent GPU texture estimate, so it is
        // always subtracted here.
        if let Some(entry) = self.textures.get_mut(&page_index) {
            self.total_size_bytes -= entry.size_bytes;
            entry.image = None;
            entry.handle = None;
            entry.native_texture = None;
            entry.size_bytes = 0;
        }

        // Evict other full-resolution pages until there is room for the new one.
        if new_size > max_size_bytes {
            while self.total_size_bytes > 0 {
                if !self.evict_lru_full_excluding(protected) {
                    break;
                }
            }
        } else {
            while self.total_size_bytes + new_size > max_size_bytes {
                if !self.evict_lru_full_excluding(protected) {
                    break;
                }
            }
        }

        let entry = self
            .textures
            .entry(page_index)
            .or_insert_with(|| CacheEntry::empty(original_size));
        entry.original_size = original_size;
        entry.image = Some(image);
        entry.size_bytes = new_size;
        entry.last_accessed = Instant::now();
        self.total_size_bytes += new_size;
        self.dimensions_version += 1;
    }

    pub fn insert_thumbnail(
        &mut self,
        page_index: usize,
        image: LoadedImage,
        original_size: [u32; 2],
    ) {
        self.insert_thumbnail_impl(page_index, image, original_size, THUMBNAIL_CACHE_MAX_BYTES);
    }

    #[cfg(test)]
    fn insert_thumbnail_with_budget(
        &mut self,
        page_index: usize,
        image: LoadedImage,
        original_size: [u32; 2],
        max_size_bytes: usize,
    ) {
        self.insert_thumbnail_impl(page_index, image, original_size, max_size_bytes);
    }

    fn insert_thumbnail_impl(
        &mut self,
        page_index: usize,
        image: LoadedImage,
        original_size: [u32; 2],
        max_size_bytes: usize,
    ) {
        timing::log(&format!("cache insert thumbnail page {}", page_index));
        let new_size = image.size_bytes();
        if let Some(entry) = self.textures.get_mut(&page_index) {
            self.thumbnail_total_size_bytes -= entry.thumbnail_size_bytes;
            entry.thumbnail = None;
            entry.thumbnail_handle = None;
            entry.thumbnail_size_bytes = 0;
        }
        while self.thumbnail_total_size_bytes + new_size > max_size_bytes
            && self.evict_lru_thumbnail_excluding(page_index)
        {}
        let entry = self
            .textures
            .entry(page_index)
            .or_insert_with(|| CacheEntry::empty(original_size));
        entry.thumbnail = Some(image);
        entry.thumbnail_handle = None;
        entry.thumbnail_size_bytes = new_size;
        entry.original_size = original_size;
        entry.last_accessed = Instant::now();
        self.thumbnail_total_size_bytes += new_size;
        self.dimensions_version += 1;
    }

    pub fn enforce_budget_with_protected(
        &mut self,
        max_size_bytes: usize,
        protected: &std::collections::HashSet<usize>,
    ) {
        while self.total_size_bytes > max_size_bytes {
            if !self.evict_lru_full_excluding(protected) {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        self.total_size_bytes = 0;
        self.thumbnail_total_size_bytes = 0;
        self.dimensions_version += 1;
    }

    fn evict_lru_thumbnail_excluding(&mut self, excluded: usize) -> bool {
        let lru_key = self
            .textures
            .iter()
            .filter(|(key, entry)| {
                **key != excluded && (entry.thumbnail.is_some() || entry.thumbnail_handle.is_some())
            })
            .min_by(|(_, a), (_, b)| a.last_accessed.cmp(&b.last_accessed))
            .map(|(&key, _)| key);
        if let Some(key) = lru_key {
            if let Some(entry) = self.textures.get_mut(&key) {
                self.thumbnail_total_size_bytes -= entry.thumbnail_size_bytes;
                entry.thumbnail = None;
                entry.thumbnail_handle = None;
                entry.thumbnail_size_bytes = 0;
                if entry.image.is_none() && entry.handle.is_none() {
                    self.textures.remove(&key);
                }
            }
            true
        } else {
            false
        }
    }

    fn evict_lru_full_excluding(&mut self, protected: &std::collections::HashSet<usize>) -> bool {
        let lru_key = self
            .textures
            .iter()
            .filter(|(k, e)| !protected.contains(k) && (e.image.is_some() || e.handle.is_some()))
            .min_by(|(_, a), (_, b)| a.last_accessed.cmp(&b.last_accessed))
            .map(|(&key, _)| key);

        if let Some(key) = lru_key {
            if let Some(entry) = self.textures.get_mut(&key) {
                self.total_size_bytes -= entry.size_bytes;
                entry.image = None;
                entry.handle = None;
                entry.native_texture = None;
                entry.size_bytes = 0;
                if entry.thumbnail.is_none() && entry.thumbnail_handle.is_none() {
                    self.textures.remove(&key);
                }
            }
            true
        } else {
            false
        }
    }
}

fn bc3_texture_uv(original_size: [u32; 2], gpu_size: [u32; 2]) -> egui::Rect {
    if original_size.contains(&0) || gpu_size.contains(&0) {
        return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
    }
    egui::Rect::from_min_max(
        egui::Pos2::ZERO,
        egui::Pos2::new(
            original_size[0] as f32 / gpu_size[0] as f32,
            original_size[1] as f32 / gpu_size[1] as f32,
        ),
    )
}

/// Pick the layout size used for fit/zoom.
///
/// Prefers an already-known header/seed size, then an explicit `layout_size`
/// from the loader (file header), and only falls back to the decoded bitmap
/// size. Never replaces a larger known size with a smaller downsample so
/// `apply_pending_fit` stays calibrated across the thumb→full transition.
fn prefer_layout_size(
    existing: Option<[u32; 2]>,
    layout_size: [u32; 2],
    decoded_size: [u32; 2],
) -> [u32; 2] {
    fn area(s: [u32; 2]) -> u64 {
        s[0] as u64 * s[1] as u64
    }
    fn valid(s: [u32; 2]) -> bool {
        s[0] > 0 && s[1] > 0
    }

    let mut best = if valid(decoded_size) {
        decoded_size
    } else {
        [1, 1]
    };
    if valid(layout_size) && area(layout_size) >= area(best) {
        best = layout_size;
    }
    if let Some(existing) = existing {
        if valid(existing) && area(existing) >= area(best) {
            best = existing;
        }
    }
    best
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ColorImage;
    use std::collections::HashSet;

    fn make_image(width: usize, height: usize) -> LoadedImage {
        LoadedImage::Color(ColorImage::filled([width, height], egui::Color32::WHITE))
    }

    fn make_compressed(width: u32, height: u32) -> LoadedImage {
        let (gpu_w, gpu_h) = crate::loader::dxt5_padded_size(width, height);
        let block_count = (gpu_w / 4) * (gpu_h / 4);
        LoadedImage::Compressed {
            data: vec![0u8; (block_count * 16) as usize],
            original_size: [width, height],
            gpu_size: [gpu_w, gpu_h],
        }
    }

    #[test]
    fn test_cache_insert_and_get() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        assert!(!cache.contains_full(0));

        let image = make_image(2, 2);
        cache.insert_full(0, image, [2, 2], 1024, &HashSet::new());

        assert!(cache.contains_full(0));
        let handle = cache
            .get_texture(&ctx, 0)
            .expect("texture should be in cache");
        assert_eq!(handle.size(), [2, 2]);
    }

    #[test]
    fn test_cache_respects_budget_and_evicts_lru() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        // 2x2 RGBA8 = 16 bytes each.
        let budget = 32;

        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new());
        cache.insert_full(1, make_image(2, 2), [2, 2], budget, &HashSet::new());
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(cache.contains_full(0));
        assert!(cache.contains_full(1));

        // Inserting a third page should evict the least-recently-used full image (page 0).
        cache.insert_full(2, make_image(2, 2), [2, 2], budget, &HashSet::new());
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(!cache.contains_full(0));
        assert!(cache.contains_full(1));
        assert!(cache.contains_full(2));

        assert!(cache.get_texture(&ctx, 1).is_some());
        assert!(cache.get_texture(&ctx, 2).is_some());
    }

    #[test]
    fn test_cache_get_updates_recency() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new());
        cache.insert_full(1, make_image(2, 2), [2, 2], budget, &HashSet::new());

        // Touch page 0 so page 1 becomes the LRU entry.
        let _ = cache.get_texture(&ctx, 0);

        cache.insert_full(2, make_image(2, 2), [2, 2], budget, &HashSet::new());
        assert!(cache.contains_full(0));
        assert!(!cache.contains_full(1));
        assert!(cache.contains_full(2));
    }

    #[test]
    fn test_cache_allows_oversized_single_texture() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 8;

        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new()); // 16 bytes, exceeds budget

        assert!(cache.contains_full(0));
        assert_eq!(cache.total_size_bytes(), 16);

        // Enforcing the budget will evict the oversized texture because it is the only entry.
        cache.enforce_budget_with_protected(budget, &HashSet::new());
        assert!(!cache.contains_full(0));
        assert_eq!(cache.total_size_bytes(), 0);

        assert!(cache.get_texture(&ctx, 0).is_none());
    }

    #[test]
    fn test_cache_protected_indices_are_not_evicted() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new());
        cache.insert_full(1, make_image(2, 2), [2, 2], budget, &HashSet::new());

        // Insert page 2 while protecting page 0. Page 1 is the only evictable entry.
        cache.insert_full(2, make_image(2, 2), [2, 2], budget, &HashSet::from([0]));
        assert!(cache.contains_full(0));
        assert!(!cache.contains_full(1));
        assert!(cache.contains_full(2));

        assert!(cache.get_texture(&ctx, 0).is_some());
        assert!(cache.get_texture(&ctx, 2).is_some());
    }

    #[test]
    fn test_cache_insert_allows_over_budget_when_all_protected() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        // 2x2 RGBA8 = 16 bytes each; budget can only hold one.
        let budget = 16;

        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new());
        // Insert page 1 while page 0 is protected. Since page 0 cannot be
        // evicted, the budget must be exceeded rather than looping forever.
        cache.insert_full(1, make_image(2, 2), [2, 2], budget, &HashSet::from([0]));

        assert!(cache.contains_full(0));
        assert!(cache.contains_full(1));
        assert!(cache.total_size_bytes() > budget);

        assert!(cache.get_texture(&ctx, 0).is_some());
        assert!(cache.get_texture(&ctx, 1).is_some());
    }

    #[test]
    fn test_cache_thumbnails_are_retained_when_full_is_evicted() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 8;

        cache.insert_thumbnail(0, make_image(2, 2), [2, 2]);
        cache.insert_full(0, make_image(2, 2), [2, 2], budget, &HashSet::new());

        // Evict the full image; the thumbnail should remain.
        cache.enforce_budget_with_protected(budget, &HashSet::new());
        assert!(!cache.contains_full(0));
        assert!(cache.contains_thumbnail(0));
        assert_eq!(cache.get_original_size(0), Some([2, 2]));
        assert!(cache.get_texture(&ctx, 0).is_some());
    }

    #[test]
    fn test_thumbnail_upload_releases_cpu_and_survives_full_eviction() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        cache.insert_thumbnail(0, make_image(2, 2), [2, 2]);
        assert!(cache.get_texture(&ctx, 0).is_some());
        let entry = cache.textures.get(&0).unwrap();
        assert!(entry.thumbnail.is_none());
        assert!(entry.thumbnail_handle.is_some());
        assert!(cache.contains_thumbnail(0));

        cache.insert_full(0, make_image(2, 2), [2, 2], 8, &HashSet::new());
        cache.enforce_budget_with_protected(8, &HashSet::new());
        assert!(!cache.contains_full(0));
        assert!(cache.contains_thumbnail(0));
        assert!(cache.get_texture(&ctx, 0).is_some());
        assert_eq!(cache.thumbnail_total_size_bytes(), 16);
    }

    #[test]
    fn test_thumbnail_budget_evicts_lru_and_reinsert_keeps_accounting() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        cache.insert_thumbnail_with_budget(0, make_image(2, 2), [2, 2], 32);
        cache.insert_thumbnail_with_budget(1, make_image(2, 2), [2, 2], 32);
        assert!(cache.get_texture(&ctx, 0).is_some());
        cache.insert_thumbnail_with_budget(2, make_image(2, 2), [2, 2], 32);

        assert!(cache.contains_thumbnail(0));
        assert!(!cache.contains_thumbnail(1));
        assert!(cache.contains_thumbnail(2));
        assert_eq!(cache.thumbnail_total_size_bytes(), 32);

        cache.insert_thumbnail_with_budget(2, make_image(2, 2), [2, 2], 32);
        assert_eq!(cache.thumbnail_total_size_bytes(), 32);
    }

    #[test]
    fn test_cache_insert_dimensions_seeds_original_size() {
        let mut cache = PageCache::new();
        cache.insert_dimensions(0, [123, 456]);
        assert_eq!(cache.get_original_size(0), Some([123, 456]));
        assert!(!cache.contains_full(0));
        assert!(!cache.contains_thumbnail(0));
    }

    #[test]
    fn test_insert_full_preserves_larger_seeded_layout_size() {
        let mut cache = PageCache::new();
        // Header/seed says 4500×2815; GPU decode was capped to 4096×2562.
        cache.insert_dimensions(0, [4500, 2815]);
        cache.insert_full(
            0,
            make_image(64, 40), // stand-in for downsampled bitmap
            [4096, 2562],
            1024 * 1024,
            &HashSet::new(),
        );
        assert_eq!(cache.get_original_size(0), Some([4500, 2815]));
        assert!(cache.contains_full(0));
    }

    #[test]
    fn test_insert_full_uses_layout_size_over_decoded_when_no_seed() {
        let mut cache = PageCache::new();
        cache.insert_full(
            0,
            make_image(64, 40),
            [4500, 2815],
            1024 * 1024,
            &HashSet::new(),
        );
        assert_eq!(cache.get_original_size(0), Some([4500, 2815]));
    }

    #[test]
    fn test_prefer_layout_size_never_shrinks() {
        assert_eq!(
            prefer_layout_size(Some([4500, 2815]), [4096, 2562], [160, 100]),
            [4500, 2815]
        );
        assert_eq!(
            prefer_layout_size(None, [4500, 2815], [4096, 2562]),
            [4500, 2815]
        );
        assert_eq!(prefer_layout_size(None, [0, 0], [4096, 2562]), [4096, 2562]);
    }

    #[test]
    fn test_bc3_uv_excludes_padding() {
        assert_eq!(
            bc3_texture_uv([5, 7], [8, 8]),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(5.0 / 8.0, 7.0 / 8.0))
        );
        assert_eq!(
            bc3_texture_uv([8, 8], [8, 8]),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0))
        );
    }

    #[test]
    fn test_compressed_cpu_fallback_uses_full_uv() {
        let mut cache = PageCache::new();
        cache.insert_full(0, make_compressed(5, 7), [5, 7], 1024, &HashSet::new());
        assert_eq!(
            cache.texture_uv(0),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0))
        );
    }

    #[test]
    fn test_slot_size_bytes_managed() {
        let image = make_image(3, 5);
        assert_eq!(image.size_bytes(), 3 * 5 * 4);
    }

    #[test]
    fn test_slot_size_bytes_compressed() {
        let image = make_compressed(5, 7);
        let (gpu_w, gpu_h) = crate::loader::dxt5_padded_size(5, 7);
        let block_count = (gpu_w / 4) * (gpu_h / 4);
        assert_eq!(image.size_bytes(), (block_count * 16) as usize);
    }
}
