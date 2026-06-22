use crate::loader::LoadedImage;
use crate::timing;
use egui::{Context, TextureHandle};
use std::collections::HashMap;
use std::time::Instant;

struct CacheEntry {
    image: LoadedImage,
    handle: Option<TextureHandle>,
    last_accessed: Instant,
    size_bytes: usize,
}

pub struct PageCache {
    textures: HashMap<usize, CacheEntry>,
    total_size_bytes: usize,
}

impl PageCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            total_size_bytes: 0,
        }
    }

    pub fn total_size_bytes(&self) -> usize {
        self.total_size_bytes
    }

    pub fn contains(&self, page_index: usize) -> bool {
        self.textures.contains_key(&page_index)
    }

    pub fn get_texture(&mut self, ctx: &Context, page_index: usize) -> Option<TextureHandle> {
        let entry = self.textures.get_mut(&page_index)?;
        entry.last_accessed = Instant::now();
        if entry.handle.is_none() {
            timing::log(&format!("cache decompress page {}", page_index));
            let label = format!("page_{}", page_index);
            let color = timing::time("cache decompress+upload", || {
                entry.image.to_color_image().ok()
            })?;
            entry.handle = Some(ctx.load_texture(&label, color, egui::TextureOptions::LINEAR));
            timing::log(&format!("cache upload page {} done", page_index));
        }
        entry.handle.clone()
    }

    pub fn insert(
        &mut self,
        page_index: usize,
        image: LoadedImage,
        max_size_bytes: usize,
        protected: &[usize],
    ) {
        let new_size = image.size_bytes();
        timing::log(&format!(
            "cache insert page {} size {} budget {}",
            page_index, new_size, max_size_bytes
        ));

        if let Some(old) = self.textures.remove(&page_index) {
            self.total_size_bytes -= old.size_bytes;
        }

        if new_size > max_size_bytes {
            while self.total_size_bytes > 0 {
                if !self.evict_lru_excluding(protected) {
                    break;
                }
            }
        } else {
            while self.total_size_bytes + new_size > max_size_bytes {
                if !self.evict_lru_excluding(protected) {
                    break;
                }
            }
        }

        self.total_size_bytes += new_size;
        self.textures.insert(
            page_index,
            CacheEntry {
                image,
                handle: None,
                last_accessed: Instant::now(),
                size_bytes: new_size,
            },
        );
    }

    pub fn enforce_budget_with_protected(&mut self, max_size_bytes: usize, protected: &[usize]) {
        while self.total_size_bytes > max_size_bytes {
            if !self.evict_lru_excluding(protected) {
                break;
            }
        }
    }

    /// Kept for tests; prefer [`Self::enforce_budget_with_protected`] in production.
    #[allow(dead_code)]
    pub fn enforce_budget(&mut self, max_size_bytes: usize) {
        self.enforce_budget_with_protected(max_size_bytes, &[]);
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        self.total_size_bytes = 0;
    }

    fn evict_lru_excluding(&mut self, protected: &[usize]) -> bool {
        let lru_key = self
            .textures
            .iter()
            .filter(|(k, _)| !protected.contains(k))
            .min_by(|(_, a), (_, b)| a.last_accessed.cmp(&b.last_accessed))
            .map(|(&key, _)| key);

        if let Some(key) = lru_key {
            if let Some(entry) = self.textures.remove(&key) {
                self.total_size_bytes -= entry.size_bytes;
            }
            true
        } else {
            false
        }
    }
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

    fn make_image(width: usize, height: usize) -> LoadedImage {
        LoadedImage::Color(ColorImage::new([width, height], egui::Color32::WHITE))
    }

    fn make_compressed(width: u32, height: u32) -> LoadedImage {
        let (gpu_w, gpu_h) = crate::loader::dxt5_padded_size(width, height);
        let block_count = (gpu_w / 4) * (gpu_h / 4);
        LoadedImage::Compressed {
            data: vec![0u8; (block_count * 16) as usize],
            original_size: [width, height],
            gpu_size: [gpu_w, gpu_h],
            format: crate::loader::CompressedFormat::Dxt5Srgb,
        }
    }

    #[test]
    fn test_cache_insert_and_get() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        assert!(!cache.contains(0));

        let image = make_image(2, 2);
        cache.insert(0, image, 1024, &[]);

        assert!(cache.contains(0));
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

        cache.insert(0, make_image(2, 2), budget, &[]);
        cache.insert(1, make_image(2, 2), budget, &[]);
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(cache.contains(0));
        assert!(cache.contains(1));

        // Inserting a third page should evict the least-recently-used page (page 0).
        cache.insert(2, make_image(2, 2), budget, &[]);
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(!cache.contains(0));
        assert!(cache.contains(1));
        assert!(cache.contains(2));

        // A texture is uploaded lazily, so budget tests don't depend on egui.
        assert!(cache.get_texture(&ctx, 1).is_some());
        assert!(cache.get_texture(&ctx, 2).is_some());
    }

    #[test]
    fn test_cache_get_updates_recency() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        cache.insert(0, make_image(2, 2), budget, &[]);
        cache.insert(1, make_image(2, 2), budget, &[]);

        // Touch page 0 so page 1 becomes the LRU entry.
        let _ = cache.get_texture(&ctx, 0);

        cache.insert(2, make_image(2, 2), budget, &[]);
        assert!(cache.contains(0));
        assert!(!cache.contains(1));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_cache_allows_oversized_single_texture() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 8;

        cache.insert(0, make_image(2, 2), budget, &[]); // 16 bytes, exceeds budget

        assert!(cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 16);

        // Enforcing the budget will evict the oversized texture because it is the only entry.
        cache.enforce_budget(budget);
        assert!(!cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 0);

        assert!(cache.get_texture(&ctx, 0).is_none());
    }

    #[test]
    fn test_cache_protected_indices_are_not_evicted() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        cache.insert(0, make_image(2, 2), budget, &[]);
        cache.insert(1, make_image(2, 2), budget, &[]);

        // Insert page 2 while protecting page 0. Page 1 is the only evictable entry.
        cache.insert(2, make_image(2, 2), budget, &[0]);
        assert!(cache.contains(0));
        assert!(!cache.contains(1));
        assert!(cache.contains(2));

        assert!(cache.get_texture(&ctx, 0).is_some());
        assert!(cache.get_texture(&ctx, 2).is_some());
    }

    #[test]
    fn test_cache_insert_allows_over_budget_when_all_protected() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        // 2x2 RGBA8 = 16 bytes each; budget can only hold one.
        let budget = 16;

        cache.insert(0, make_image(2, 2), budget, &[]);
        // Insert page 1 while page 0 is protected. Since page 0 cannot be
        // evicted, the budget must be exceeded rather than looping forever.
        cache.insert(1, make_image(2, 2), budget, &[0]);

        assert!(cache.contains(0));
        assert!(cache.contains(1));
        assert!(cache.total_size_bytes() > budget);

        assert!(cache.get_texture(&ctx, 0).is_some());
        assert!(cache.get_texture(&ctx, 1).is_some());
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
