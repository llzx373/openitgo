use egui::TextureHandle;
use std::collections::HashMap;

pub struct PageCache {
    textures: HashMap<usize, TextureHandle>,
}

impl PageCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }

    pub fn get(&self, page_index: usize) -> Option<TextureHandle> {
        self.textures.get(&page_index).cloned()
    }

    pub fn insert(&mut self, page_index: usize, texture: TextureHandle) {
        self.textures.insert(page_index, texture);
    }

    pub fn contains(&self, page_index: usize) -> bool {
        self.textures.contains_key(&page_index)
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(usize) -> bool,
    {
        self.textures.retain(|&page_index, _texture| f(page_index));
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

    #[test]
    fn test_cache_insert_and_get() {
        let ctx = egui::Context::default();
        let mut cache = PageCache::new();
        assert!(!cache.contains(0));

        let image = egui::ColorImage::example();
        let handle = ctx.load_texture("page_0", image, Default::default());
        cache.insert(0, handle.clone());

        assert!(cache.contains(0));
        let retrieved = cache.get(0).expect("texture should be in cache");
        assert_eq!(retrieved.id(), handle.id());
    }
}
