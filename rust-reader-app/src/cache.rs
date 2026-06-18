use crate::widgets::page_view::TextureSlot;
use glow::HasContext;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::time::Instant;

struct CacheEntry {
    slot: TextureSlot,
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

    pub fn get(&mut self, page_index: usize) -> Option<TextureSlot> {
        let entry = self.textures.get_mut(&page_index)?;
        entry.last_accessed = Instant::now();
        Some(entry.slot.clone())
    }

    pub fn insert(
        &mut self,
        page_index: usize,
        slot: TextureSlot,
        max_size_bytes: usize,
        gl: Option<&glow::Context>,
        protected: &[usize],
    ) {
        let new_size = slot_size_bytes(&slot);

        if let Some(old) = self.textures.remove(&page_index) {
            self.total_size_bytes -= old.size_bytes;
            delete_native_texture(gl, old.slot);
        }

        if new_size > max_size_bytes {
            self.clear(gl);
        } else {
            while self.total_size_bytes + new_size > max_size_bytes {
                self.evict_lru_excluding(gl, protected);
            }
        }

        self.total_size_bytes += new_size;
        self.textures.insert(
            page_index,
            CacheEntry {
                slot,
                last_accessed: Instant::now(),
                size_bytes: new_size,
            },
        );
    }

    pub fn contains(&self, page_index: usize) -> bool {
        self.textures.contains_key(&page_index)
    }

    #[allow(dead_code)]
    pub fn enforce_budget(&mut self, max_size_bytes: usize, gl: Option<&glow::Context>) {
        self.enforce_budget_with_protected(max_size_bytes, gl, &[]);
    }

    pub fn enforce_budget_with_protected(
        &mut self,
        max_size_bytes: usize,
        gl: Option<&glow::Context>,
        protected: &[usize],
    ) {
        while self.total_size_bytes > max_size_bytes {
            self.evict_lru_excluding(gl, protected);
        }
    }

    pub fn clear(&mut self, gl: Option<&glow::Context>) {
        for (_, entry) in self.textures.drain() {
            delete_native_texture(gl, entry.slot);
            self.total_size_bytes -= entry.size_bytes;
        }
    }

    fn evict_lru_excluding(&mut self, gl: Option<&glow::Context>, protected: &[usize]) {
        let lru_key = self
            .textures
            .iter()
            .filter(|(&key, _)| !protected.contains(&key))
            .min_by(|(_, a), (_, b)| a.last_accessed.cmp(&b.last_accessed))
            .map(|(&key, _)| key);

        if let Some(key) = lru_key {
            if let Some(entry) = self.textures.remove(&key) {
                delete_native_texture(gl, entry.slot);
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

fn delete_native_texture(gl: Option<&glow::Context>, slot: TextureSlot) {
    if let TextureSlot::Native(id, _) = slot {
        if let Some(gl) = gl {
            if let egui::TextureId::User(native) = id {
                if let Some(non_zero) = NonZeroU32::new(native as u32) {
                    unsafe { gl.delete_texture(glow::NativeTexture(non_zero)) };
                }
            }
        }
    }
}

fn slot_size_bytes(slot: &TextureSlot) -> usize {
    match slot {
        TextureSlot::Managed(handle) => {
            let size = handle.size();
            size[0] * size[1] * 4
        }
        TextureSlot::Native(_, display_size) => {
            let gpu_w = display_size[0].div_ceil(4) * 4;
            let gpu_h = display_size[1].div_ceil(4) * 4;
            (gpu_w * gpu_h) as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_texture(ctx: &egui::Context, name: &str, width: usize, height: usize) -> TextureSlot {
        let image = egui::ColorImage::new([width, height], egui::Color32::WHITE);
        TextureSlot::Managed(ctx.load_texture(name, image, Default::default()))
    }

    #[test]
    fn test_cache_insert_and_get() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        assert!(!cache.contains(0));

        let handle = make_texture(&ctx, "page_0", 2, 2);
        cache.insert(0, handle.clone(), 1024, None, &[]);

        assert!(cache.contains(0));
        let retrieved = cache.get(0).expect("texture should be in cache");
        assert_eq!(retrieved.size(), handle.size());
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

        cache.insert(0, h0, budget, None, &[]);
        cache.insert(1, h1, budget, None, &[]);
        assert_eq!(cache.total_size_bytes(), 32);
        assert!(cache.contains(0));
        assert!(cache.contains(1));

        // Inserting a third page should evict the least-recently-used page (page 0).
        cache.insert(2, h2, budget, None, &[]);
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

        cache.insert(0, h0, budget, None, &[]);
        cache.insert(1, h1, budget, None, &[]);

        // Touch page 0 so page 1 becomes the LRU entry.
        let _ = cache.get(0);

        cache.insert(2, h2, budget, None, &[]);
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
        cache.insert(0, h0, budget, None, &[]);

        assert!(cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 16);

        // Enforcing the budget will evict the oversized texture because it is the only entry.
        cache.enforce_budget(budget, None);
        assert!(!cache.contains(0));
        assert_eq!(cache.total_size_bytes(), 0);
    }

    #[test]
    fn test_cache_protected_indices_are_not_evicted() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        let budget = 32;

        let h0 = make_texture(&ctx, "page_0", 2, 2);
        let h1 = make_texture(&ctx, "page_1", 2, 2);
        let h2 = make_texture(&ctx, "page_2", 2, 2);

        cache.insert(0, h0, budget, None, &[]);
        cache.insert(1, h1, budget, None, &[]);

        // Insert page 2 while protecting page 0. Page 1 is the only evictable entry.
        cache.insert(2, h2, budget, None, &[0]);
        assert!(cache.contains(0));
        assert!(!cache.contains(1));
        assert!(cache.contains(2));

        // Enforcing budget while protecting both remaining pages should not evict them.
        cache.enforce_budget_with_protected(budget, None, &[0, 2]);
        assert!(cache.contains(0));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_slot_size_bytes_managed() {
        let ctx = egui::Context::default();
        let slot = make_texture(&ctx, "size_test", 3, 5);
        // Managed RGBA8 size = width * height * 4.
        assert_eq!(slot_size_bytes(&slot), 3 * 5 * 4);
    }

    #[test]
    fn test_slot_size_bytes_native() {
        let slot = TextureSlot::Native(egui::TextureId::User(1), [5, 7]);
        let (gpu_w, gpu_h) = (8, 8);
        assert_eq!(slot_size_bytes(&slot), (gpu_w * gpu_h) as usize);
    }
}
