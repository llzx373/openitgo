use egui::TextureHandle;
use std::collections::HashMap;
use std::time::Instant;

struct CacheEntry {
    texture: TextureHandle,
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

    pub fn get(&mut self, page_index: usize) -> Option<TextureHandle> {
        let entry = self.textures.get_mut(&page_index)?;
        entry.last_accessed = Instant::now();
        Some(entry.texture.clone())
    }

    pub fn insert(&mut self, page_index: usize, texture: TextureHandle, max_size_bytes: usize) {
        let new_size = texture_size_bytes(&texture);

        // Remove any existing entry for this page so the size accounting stays correct.
        if let Some(old) = self.textures.remove(&page_index) {
            self.total_size_bytes -= old.size_bytes;
        }

        // A single page is allowed to exceed the global budget. If it does,
        // clear everything else before inserting it.
        if new_size > max_size_bytes {
            self.textures.clear();
            self.total_size_bytes = 0;
        } else {
            while self.total_size_bytes + new_size > max_size_bytes {
                self.evict_lru();
            }
        }

        self.total_size_bytes += new_size;
        self.textures.insert(
            page_index,
            CacheEntry {
                texture,
                last_accessed: Instant::now(),
                size_bytes: new_size,
            },
        );
    }

    pub fn contains(&self, page_index: usize) -> bool {
        self.textures.contains_key(&page_index)
    }

    pub fn enforce_budget(&mut self, max_size_bytes: usize) {
        while self.total_size_bytes > max_size_bytes {
            self.evict_lru();
        }
    }

    fn evict_lru(&mut self) {
        let lru_key = self
            .textures
            .iter()
            .min_by(|(_, a), (_, b)| a.last_accessed.cmp(&b.last_accessed))
            .map(|(&key, _)| key);

        if let Some(key) = lru_key {
            if let Some(entry) = self.textures.remove(&key) {
                self.total_size_bytes -= entry.size_bytes;
            }
        }
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new()
    }
}

fn texture_size_bytes(texture: &egui::TextureHandle) -> usize {
    let size = texture.size_vec2();
    (size.x * size.y * 4.0) as usize // assume RGBA8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_texture(ctx: &egui::Context, name: &str, width: usize, height: usize) -> TextureHandle {
        let image = egui::ColorImage::new([width, height], egui::Color32::WHITE);
        ctx.load_texture(name, image, Default::default())
    }

    #[test]
    fn test_cache_insert_and_get() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        assert!(!cache.contains(0));

        let handle = make_texture(&ctx, "page_0", 2, 2);
        cache.insert(0, handle.clone(), 1024);

        assert!(cache.contains(0));
        let retrieved = cache.get(0).expect("texture should be in cache");
        assert_eq!(retrieved.id(), handle.id());
    }

    #[test]
    fn test_cache_respects_budget_and_evicts_lru() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        // 2x2 RGBA8 = 16 bytes each.
        let budget = 32;

        let h0 = make_texture(&ctx, "page_0", 2, 2);
        let h1 = make_texture(&ctx, "page_1", 2, 2);
        let h2 = make_texture(&ctx, "page_2", 2, 2);

        cache.insert(0, h0, budget);
        cache.insert(1, h1, budget);
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(cache.contains(0));
        assert!(cache.contains(1));

        // Inserting a third page should evict the least-recently-used page (page 0).
        cache.insert(2, h2, budget);
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(!cache.contains(0));
        assert!(cache.contains(1));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_cache_get_updates_recency() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        let h0 = make_texture(&ctx, "page_0", 2, 2);
        let h1 = make_texture(&ctx, "page_1", 2, 2);
        let h2 = make_texture(&ctx, "page_2", 2, 2);

        cache.insert(0, h0, budget);
        cache.insert(1, h1, budget);

        // Touch page 0 so page 1 becomes the LRU entry.
        let _ = cache.get(0);

        cache.insert(2, h2, budget);
        assert!(cache.contains(0));
        assert!(!cache.contains(1));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_cache_allows_oversized_single_texture() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 8;

        let h0 = make_texture(&ctx, "page_0", 2, 2); // 16 bytes, exceeds budget
        cache.insert(0, h0, budget);

        assert!(cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 16);

        // Enforcing the budget will evict the oversized texture because it is the only entry.
        cache.enforce_budget(budget);
        assert!(!cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 0);
    }
}
