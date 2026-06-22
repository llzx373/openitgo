# Thumbnail-First Rendering Implementation Plan

> **Status:** Implemented and optimized. See commits `28e50f0`, `1171e87`, `59cdb43`.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the comic reader so that opening a file only loads
low-resolution (256px max) thumbnails for all pages in the background. The
visible spread renders with thumbnails first, then full-resolution images
replace them. Full-resolution images are preloaded within a configurable window
around the current page (default 50 pages each direction).

**Architecture:** Reuse the existing `PageLoader` with a new `thumbnail` flag on
requests/results. `PageCache` gains a separate thumbnail slot per page.
`ReaderView` requests thumbnails for all pages on open and full images only
inside the configured window. Rendering prefers the full image and falls back to
the thumbnail. Layout uses the cached `original_size` so that displaying a
256px thumbnail does not shrink the page layout. Dead/duplicate code identified
in the spec is removed first.

**Tech Stack:** Rust, egui/eframe, `image`, `crossbeam-channel`, existing
`PageLoader`/`PageCache`.

---

## File Structure

| File | Responsibility |
|---|---|
| `rust-reader-app/src/widgets/page_view.rs` | **Delete** — duplicated by `PageCache::get_texture`. |
| `rust-reader-app/src/widgets/mod.rs` | Remove `page_view` module declaration. |
| `rust-reader-app/src/widgets/progress_bar.rs` | Export `page_at_x`; remove duplicate in `reader.rs`. |
| `rust-reader-app/src/loader.rs` | Add thumbnail flag, thumbnail decode path, `original_size` on `LoadResult`. |
| `rust-reader-app/src/cache.rs` | Split cache entry into thumbnail + full image; add window pruning. |
| `rust-reader-app/src/views/reader.rs` | Request thumbnails/full images, render fallback, prune window. |
| `rust-reader-app/src/app.rs` | Pass `real_image_cache_pages` setting to reader. |
| `rust-reader-storage/src/models.rs` | Add `real_image_cache_pages` setting. |
| `rust-reader-app/src/views/settings.rs` | Add slider for `real_image_cache_pages`. |
| `rust-reader-app/examples/diagnose_hang.rs` | **Delete**. |
| `rust-reader-app/examples/rapid_flip.rs` | Remove unused `PageSource` import. |

---

## Task 1: Remove dead/duplicate code

**Files:**
- Delete: `rust-reader-app/src/widgets/page_view.rs`
- Modify: `rust-reader-app/src/widgets/mod.rs`
- Modify: `rust-reader-app/src/loader.rs`
- Modify: `rust-reader-app/src/cache.rs`
- Modify: `rust-reader-app/src/widgets/progress_bar.rs`
- Modify: `rust-reader-app/src/views/reader.rs`
- Delete: `rust-reader-app/examples/diagnose_hang.rs`
- Modify: `rust-reader-app/examples/rapid_flip.rs`

- [ ] **Step 1: Delete `widgets/page_view.rs` and its module declaration**

```bash
rm rust-reader-app/src/widgets/page_view.rs
```

Edit `rust-reader-app/src/widgets/mod.rs`:

```rust
pub mod progress_bar;
pub mod thumbnail_progress_bar;
```

- [ ] **Step 2: Remove `PageLoader::request` and make `LoadPriority` pub(crate)**

In `rust-reader-app/src/loader.rs`, replace:

```rust
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum LoadPriority {
    High,
    Low,
}
```

with:

```rust
#[derive(Clone, Copy)]
pub(crate) enum LoadPriority {
    High,
    Low,
}
```

Delete the `request` method entirely:

```rust
#[allow(dead_code)]
pub fn request(
    &self,
    priority: LoadPriority,
    epoch: Epoch,
    page_index: usize,
    source: PageSource,
) -> bool {
    match priority {
        LoadPriority::High => self.request_high(epoch, page_index, source),
        LoadPriority::Low => self.request_low(epoch, page_index, source),
    }
}
```

- [ ] **Step 3: Remove `CompressedFormat` enum and the `format` field**

In `rust-reader-app/src/loader.rs`, delete:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressedFormat {
    Dxt5Srgb,
}
```

Change `LoadedImage::Compressed` to:

```rust
Compressed {
    data: Vec<u8>,
    original_size: [u32; 2],
    gpu_size: [u32; 2],
},
```

Update `size_bytes`:

```rust
pub fn size_bytes(&self) -> usize {
    match self {
        LoadedImage::Compressed { data, .. } => data.len(),
        LoadedImage::Color(img) => img.size[0] * img.size[1] * 4,
    }
}
```

Update `to_color_image`:

```rust
pub fn to_color_image(&self) -> Result<ColorImage, String> {
    match self {
        LoadedImage::Compressed {
            data,
            original_size,
            gpu_size,
            ..
        } => decompress_dxt5(data, *original_size, *gpu_size),
        LoadedImage::Color(img) => Ok(img.clone()),
    }
}
```

In `compress_dxt5`, remove `format: CompressedFormat::Dxt5Srgb,` from the
`LoadedImage::Compressed` construction.

In `rust-reader-app/src/cache.rs` test helper `make_compressed`, remove the
`format: crate::loader::CompressedFormat::Dxt5Srgb,` field.

- [ ] **Step 4: Remove duplicate `PageCache::enforce_budget`**

In `rust-reader-app/src/cache.rs`, delete:

```rust
/// Kept for tests; prefer [`Self::enforce_budget_with_protected`] in production.
#[allow(dead_code)]
pub fn enforce_budget(&mut self, max_size_bytes: usize) {
    self.enforce_budget_with_protected(max_size_bytes, &[]);
}
```

In the same file, update `test_cache_allows_oversized_single_texture`:

```rust
cache.enforce_budget_with_protected(budget, &[]);
```

- [ ] **Step 5: Delete diagnostic example and fix unused import**

```bash
rm rust-reader-app/examples/diagnose_hang.rs
```

In `rust-reader-app/examples/rapid_flip.rs`, remove:

```rust
use rust_reader_core::models::PageSource;
```

- [ ] **Step 6: Export `page_at_x` from `progress_bar.rs` and reuse it**

In `rust-reader-app/src/widgets/progress_bar.rs`, change:

```rust
fn page_at_x(x: f32, rect: egui::Rect, total_pages: usize) -> usize {
```

to:

```rust
pub fn page_at_x(x: f32, rect: egui::Rect, total_pages: usize) -> usize {
```

In `rust-reader-app/src/views/reader.rs`, update the import:

```rust
use crate::widgets::progress_bar::{comic_progress_bar, page_at_x, ProgressBarResponse};
```

Delete the local `page_at_x` function in `reader.rs`.

- [ ] **Step 7: Extract `texture_size_or_fallback` helper**

Add near the top of `rust-reader-app/src/views/reader.rs`:

```rust
fn texture_size_or_fallback(texture: Option<&egui::TextureHandle>) -> egui::Vec2 {
    texture
        .map(|t| {
            let size = t.size();
            egui::vec2(size[0] as f32, size[1] as f32)
        })
        .unwrap_or(FALLBACK_PAGE_SIZE)
}
```

Replace the two inline `from_texture...unwrap_or(FALLBACK_PAGE_SIZE)` blocks in
`ui_inner` and the two in `render_page_turn_animation` with calls to
`texture_size_or_fallback(...)`.

- [ ] **Step 8: Verify cleanup compiles and tests pass**

Run:

```bash
cargo fmt
cargo check -p rust-reader-app
cargo test -p rust-reader-app
```

Expected: all check/tests pass.

- [ ] **Step 9: Commit cleanup**

```bash
git add -A
git commit -m "refactor: remove dead/duplicate code before thumbnail refactor

- Delete widgets/page_view.rs (duplicated PageCache upload)
- Remove PageLoader::request and make LoadPriority private
- Remove CompressedFormat / LoadedImage::Compressed.format
- Remove duplicate PageCache::enforce_budget
- Delete diagnose_hang example and fix rapid_flip unused import
- Export page_at_x from progress_bar and reuse in reader
- Add texture_size_or_fallback helper"
```

---

## Task 2: Add `real_image_cache_pages` setting

**Files:**
- Modify: `rust-reader-storage/src/models.rs`
- Modify: `rust-reader-app/src/views/settings.rs`
- Test: `rust-reader-storage/src/models.rs`

- [ ] **Step 1: Add the setting field and default**

In `rust-reader-storage/src/models.rs`, add inside `Settings`:

```rust
pub real_image_cache_pages: u32,
```

Add to `Default for Settings`:

```rust
real_image_cache_pages: 50,
```

- [ ] **Step 2: Add a slider in the settings UI**

In `rust-reader-app/src/views/settings.rs`, after the decode-threads slider
section, add:

```rust
ui.horizontal(|ui| {
    ui.label("真实图片缓存页数:");
    ui.add(egui::Slider::new(&mut settings.real_image_cache_pages, 0..=200).text("页"));
    ui.label("（前后各 N 页）");
});
```

- [ ] **Step 3: Update the settings round-trip test**

In `rust-reader-storage/src/models.rs`, find
`test_settings_roundtrip_with_background_color` and add:

```rust
s.real_image_cache_pages = 75;
```

before serialization, and assert the deserialized value equals `75`.

- [ ] **Step 4: Run storage tests**

```bash
cargo test -p rust-reader-storage
```

Expected: PASS.

- [ ] **Step 5: Commit setting change**

```bash
git add rust-reader-storage/src/models.rs rust-reader-app/src/views/settings.rs
git commit -m "feat(settings): add real_image_cache_pages setting (default 50)"
```

---

## Task 3: Add thumbnail support to `PageLoader`

**Files:**
- Modify: `rust-reader-app/src/loader.rs`
- Test: `rust-reader-app/src/loader.rs`

- [ ] **Step 1: Add `thumbnail` flag to request/result/job types and update request methods**

In `rust-reader-app/src/loader.rs`, change:

```rust
pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub image: Result<LoadedImage, String>,
}

pub struct LoadRequest {
    pub epoch: Epoch,
    pub page_index: usize,
    pub source: PageSource,
}

struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}
```

to:

```rust
pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub thumbnail: bool,
    pub original_size: [u32; 2],
    pub image: Result<LoadedImage, String>,
}

pub struct LoadRequest {
    pub epoch: Epoch,
    pub page_index: usize,
    pub thumbnail: bool,
    pub source: PageSource,
}

struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    thumbnail: bool,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}
```

Update `PageLoader::request_high` and `request_low` signatures:

```rust
pub fn request_high(
    &self,
    epoch: Epoch,
    page_index: usize,
    thumbnail: bool,
    source: PageSource,
) -> bool {
    self.high_sender
        .try_send(LoadRequest {
            epoch,
            page_index,
            thumbnail,
            source,
        })
        .is_ok()
}

pub fn request_low(
    &self,
    epoch: Epoch,
    page_index: usize,
    thumbnail: bool,
    source: PageSource,
) -> bool {
    self.low_sender
        .try_send(LoadRequest {
            epoch,
            page_index,
            thumbnail,
            source,
        })
        .is_ok()
}
```

- [ ] **Step 2: Add thumbnail resizing helper**

Add near the other image helpers:

```rust
pub(crate) const THUMBNAIL_MAX_DIMENSION: u32 = 256;

fn make_thumbnail(image: image::DynamicImage) -> Result<LoadedImage, String> {
    let (w, h) = (image.width(), image.height());
    let scale = (THUMBNAIL_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
    let new_w = (w as f32 * scale).ceil() as u32;
    let new_h = (h as f32 * scale).ceil() as u32;
    let thumb = image.resize(new_w, new_h, image::imageops::FilterType::Triangle);
    let rgba = thumb.to_rgba8();
    Ok(LoadedImage::Color(ColorImage::from_rgba_unmultiplied(
        [new_w as usize, new_h as usize],
        &rgba.into_raw(),
    )))
}
```

- [ ] **Step 3: Update `decode_image_bytes` for thumbnails**

Change the signature to return the original decoded size alongside the image:

```rust
fn decode_image_bytes(
    bytes: &[u8],
    format_hint: Option<&str>,
    compress: bool,
    thumbnail: bool,
) -> Result<(LoadedImage, [u32; 2]), String> {
```

At the end of the function, replace:

```rust
let result = dynamic_to_loaded_image(image, compress);
```

with:

```rust
let original_size = [image.width() as u32, image.height() as u32];
let result = if thumbnail {
    make_thumbnail(image)
} else {
    dynamic_to_loaded_image(image, compress)
};
result.map(|img| (img, original_size))
```

- [ ] **Step 4: Update PDF path for thumbnails**

Add a helper:

```rust
fn loaded_image_to_thumbnail(image: LoadedImage) -> Result<LoadedImage, String> {
    let color = image.to_color_image()?;
    let dynamic = image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(
            color.size[0] as u32,
            color.size[1] as u32,
            color.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), p.a()]).collect(),
        )
        .ok_or("failed to reconstruct image for thumbnail")?,
    );
    make_thumbnail(dynamic)
}
```

In `process_io_request`, for the `PageSource::PdfPage` branch, send:

```rust
let image = render_pdf_page(&document, page_number)?;
let original_size = image.original_size();
let image = if req.thumbnail {
    loaded_image_to_thumbnail(image)?
} else {
    image
};
let _ = result_sender.send(LoadResult {
    epoch: req.epoch,
    page_index: req.page_index,
    thumbnail: req.thumbnail,
    original_size,
    image: Ok(image),
});
```

- [ ] **Step 5: Update `process_io_request` non-PDF paths**

When building the `DecodeJob`, add `thumbnail: req.thumbnail`.

When sending an error result, set `thumbnail: req.thumbnail` and
`original_size: [0, 0]`.

When the decode queue is full and an error is sent, set the same fields.

- [ ] **Step 6: Update decode worker `process_job`**

Replace the body of `process_job` with:

```rust
fn process_job(job: DecodeJob, compress: bool, result_sender: &Sender<LoadResult>) {
    let (image, original_size) = match decode_image_bytes(
        &job.bytes,
        job.format_hint.as_deref(),
        compress,
        job.thumbnail,
    ) {
        Ok(pair) => (Ok(pair.0), pair.1),
        Err(e) => (Err(e), [0, 0]),
    };
    let _ = result_sender.send(LoadResult {
        epoch: job.epoch,
        page_index: job.page_index,
        thumbnail: job.thumbnail,
        original_size,
        image,
    });
}
```

- [ ] **Step 7: Update existing callers of `request_high` / `request_low`**

All full-image callers must pass `false` for `thumbnail` until Task 5 adds
thumbnail requests. Update:

- `rust-reader-app/src/views/reader.rs`:
  - `request_page(...)` → `loader.request_high(reader.current_epoch, page_index, false, source)`
  - `request_preloads(...)` → `loader.request_low(reader.current_epoch, idx, false, source)`
- `rust-reader-app/src/loader.rs` test module:
  - All `loader.request_high(...)` calls → add `false` before `PageSource`.
  - `test_decode_image_bytes_falls_back_for_wrong_extension` now returns a tuple:
    use `let (loaded, _size) = decode_image_bytes(...)?;`.
- `rust-reader-app/examples/rapid_flip.rs` and `flip_through.rs`:
  - All `loader.request_high(...)` calls → add `false` before `PageSource`.

- [ ] **Step 8: Add loader tests for thumbnails**

Add to the loader test module:

```rust
#[test]
fn test_loader_decodes_thumbnail_within_256px() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sample.png");
    let image = image::RgbaImage::from_pixel(800, 600, image::Rgba([255, 0, 0, 255]));
    image.save(&path).unwrap();

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();
    loader.request_high(
        epoch,
        0,
        true,
        PageSource::File(path),
    );

    let result = wait_for_result(&loader, epoch, 0, Duration::from_secs(5));
    assert!(result.thumbnail);
    let loaded = result.image.expect("thumbnail should decode");
    let [w, h] = loaded.original_size();
    assert!(w <= 256 && h <= 256, "thumbnail should fit in 256px");
    assert_eq!(result.original_size, [800, 600], "original_size should report full image dimensions");
}
```

- [ ] **Step 9: Verify loader compiles and tests pass**

```bash
cargo fmt
cargo check -p rust-reader-app
cargo test -p rust-reader-app loader
```

Expected: PASS.

- [ ] **Step 10: Commit loader thumbnail support**

```bash
git add rust-reader-app/src/loader.rs rust-reader-app/src/views/reader.rs rust-reader-app/examples/rapid_flip.rs rust-reader-app/examples/flip_through.rs
git commit -m "feat(loader): add thumbnail request/result path

- Add thumbnail flag to LoadRequest, DecodeJob, LoadResult
- Update request_high/request_low signatures
- Add make_thumbnail helper (256px max, RGBA8)
- Downsample PDF results for thumbnails
- Carry original_size through LoadResult for layout"
```

---

## Task 4: Split `PageCache` into thumbnail + full image slots

**Files:**
- Modify: `rust-reader-app/src/cache.rs`
- Test: `rust-reader-app/src/cache.rs`

- [ ] **Step 1: Extend `CacheEntry`**

Change `CacheEntry` to:

```rust
struct CacheEntry {
    thumbnail: Option<LoadedImage>,
    thumbnail_handle: Option<TextureHandle>,
    image: Option<LoadedImage>,
    handle: Option<TextureHandle>,
    original_size: [u32; 2],
    size_bytes: usize,
    last_accessed: Instant,
}
```

- [ ] **Step 2: Update `insert` to preserve thumbnails and original size**

Replace `insert` with:

```rust
pub fn insert(
    &mut self,
    page_index: usize,
    image: LoadedImage,
    original_size: [u32; 2],
    max_size_bytes: usize,
    protected: &[usize],
) {
    let new_size = image.size_bytes();

    let (old_thumbnail, old_thumbnail_handle, old_size) =
        if let Some(old) = self.textures.get_mut(&page_index) {
            let size = old.size_bytes;
            self.total_size_bytes -= size;
            old.image = None;
            old.handle = None;
            old.size_bytes = 0;
            (
                old.thumbnail.take(),
                old.thumbnail_handle.take(),
                size,
            )
        } else {
            (None, None, 0)
        };

    let mut protected = protected.to_vec();
    protected.push(page_index);

    if new_size > max_size_bytes {
        while self.total_size_bytes > 0 {
            if !self.evict_lru_excluding(&protected) {
                break;
            }
        }
    } else {
        while self.total_size_bytes + new_size > max_size_bytes {
            if !self.evict_lru_excluding(&protected) {
                break;
            }
        }
    }

    let entry = self.textures.entry(page_index).or_insert(CacheEntry {
        thumbnail: None,
        thumbnail_handle: None,
        image: None,
        handle: None,
        original_size: [0, 0],
        size_bytes: 0,
        last_accessed: Instant::now(),
    });
    entry.image = Some(image);
    entry.handle = None;
    entry.original_size = original_size;
    entry.thumbnail = old_thumbnail;
    entry.thumbnail_handle = old_thumbnail_handle;
    entry.size_bytes = new_size;
    entry.last_accessed = Instant::now();
    self.total_size_bytes += new_size;
}
```

- [ ] **Step 3: Add thumbnail methods**

```rust
pub fn insert_thumbnail(
    &mut self,
    page_index: usize,
    thumbnail: LoadedImage,
    original_size: [u32; 2],
) {
    let entry = self.textures.entry(page_index).or_insert(CacheEntry {
        thumbnail: None,
        thumbnail_handle: None,
        image: None,
        handle: None,
        original_size: [0, 0],
        size_bytes: 0,
        last_accessed: Instant::now(),
    });
    entry.thumbnail = Some(thumbnail);
    entry.thumbnail_handle = None;
    entry.original_size = original_size;
    entry.last_accessed = Instant::now();
}

pub fn get_full_texture(&mut self, ctx: &Context, page_index: usize) -> Option<TextureHandle> {
    let entry = self.textures.get_mut(&page_index)?;
    entry.last_accessed = Instant::now();
    let image = entry.image.as_ref()?;
    if entry.handle.is_none() {
        let label = format!("page_{}", page_index);
        let color = image.to_color_image().ok()?;
        entry.handle = Some(ctx.load_texture(&label, color, egui::TextureOptions::LINEAR));
    }
    entry.handle.clone()
}

/// Alias kept for callers that only need the full-resolution texture.
pub fn get_texture(&mut self, ctx: &Context, page_index: usize) -> Option<TextureHandle> {
    self.get_full_texture(ctx, page_index)
}

pub(crate) fn get_full_texture_internal(&self, page_index: usize) -> Option<&LoadedImage> {
    self.textures.get(&page_index).and_then(|e| e.image.as_ref())
}

pub fn get_thumbnail_texture(
    &mut self,
    ctx: &Context,
    page_index: usize,
) -> Option<TextureHandle> {
    let entry = self.textures.get_mut(&page_index)?;
    entry.last_accessed = Instant::now();
    let thumb = entry.thumbnail.as_ref()?;
    if entry.thumbnail_handle.is_none() {
        let label = format!("page_thumb_{}", page_index);
        let color = thumb.to_color_image().ok()?;
        entry.thumbnail_handle =
            Some(ctx.load_texture(&label, color, egui::TextureOptions::LINEAR));
    }
    entry.thumbnail_handle.clone()
}

pub fn original_size(&self, page_index: usize) -> Option<[u32; 2]> {
    self.textures
        .get(&page_index)
        .map(|e| e.original_size)
        .filter(|s| s[0] > 0 && s[1] > 0)
}
```

- [ ] **Step 4: Add window pruning**

```rust
pub fn prune_full_images_outside_window(
    &mut self,
    current: usize,
    window: usize,
    protected: &[usize],
) {
    let total = self.textures.len();
    if total == 0 {
        return;
    }
    let start = current.saturating_sub(window);
    let end = current.saturating_add(window);
    let to_remove: Vec<usize> = self
        .textures
        .iter()
        .filter(|(&idx, e)| e.image.is_some())
        .filter(|(&idx, _)| idx < start || idx > end)
        .filter(|(&idx, _)| !protected.contains(&idx))
        .map(|(&idx, _)| idx)
        .collect();
    for idx in to_remove {
        if let Some(entry) = self.textures.get_mut(&idx) {
            if let Some(image) = entry.image.take() {
                self.total_size_bytes -= image.size_bytes();
            }
            entry.handle = None;
            entry.size_bytes = 0;
        }
    }
}
```

- [ ] **Step 5: Update cache tests**

Adjust existing tests that call `cache.insert` to pass `original_size`:

```rust
cache.insert(0, image, [2, 2], budget, &[]);
```

Add new tests:

```rust
#[test]
fn test_cache_preserves_thumbnail_when_inserting_full() {
    let ctx = egui::Context::default();
    let mut cache = PageCache::new();
    cache.insert_thumbnail(0, make_image(2, 2), [100, 100]);
    cache.insert(0, make_image(4, 4), [4, 4], 1024, &[]);
    assert!(cache.get_full_texture(&ctx, 0).is_some());
    assert!(cache.get_thumbnail_texture(&ctx, 0).is_some());
}

#[test]
fn test_cache_prune_full_outside_window_keeps_thumbnail() {
    let ctx = egui::Context::default();
    let mut cache = PageCache::new();
    for i in 0..5 {
        cache.insert(i, make_image(4, 4), [4, 4], 1024, &[]);
        cache.insert_thumbnail(i, make_image(2, 2), [4, 4]);
    }
    cache.prune_full_images_outside_window(2, 1, &[]);
    assert!(cache.get_full_texture(&ctx, 0).is_none());
    assert!(cache.get_thumbnail_texture(&ctx, 0).is_some());
    assert!(cache.get_full_texture(&ctx, 2).is_some());
}
```

- [ ] **Step 6: Run cache tests**

```bash
cargo test -p rust-reader-app cache
```

Expected: PASS.

- [ ] **Step 7: Commit cache split**

```bash
git add rust-reader-app/src/cache.rs
git commit -m "feat(cache): split cache into thumbnail and full-image slots

- CacheEntry now holds thumbnail + full image + original_size
- insert preserves existing thumbnail
- Add get_full_texture / get_thumbnail_texture / original_size
- Add prune_full_images_outside_window to bound full-image memory"
```

---

## Task 5: Wire thumbnail-first behavior into `ReaderView`

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: Ensure `request_high` / `request_low` signatures are ready**

These were updated in Task 3. Verify that `rust-reader-app/src/views/reader.rs`
callers already pass `false` for full-image requests; thumbnail calls added in
this task will pass `true`.

- [ ] **Step 2: Add thumbnail tracking to `OpenReader`**

In `rust-reader-app/src/views/reader.rs`, update imports:

```rust
use std::collections::{HashMap, HashSet};
```

Add to `OpenReader`:

```rust
thumbnail_requests_sent: HashSet<usize>,
```

Initialize it in `ReaderView::open`:

```rust
thumbnail_requests_sent: HashSet::new(),
```

Clear it in `bump_epoch`:

```rust
self.thumbnail_requests_sent.clear();
```

Update the `dummy_reader` test helper to include:

```rust
thumbnail_requests_sent: HashSet::new(),
```

- [ ] **Step 3: Add request helpers**

```rust
fn request_thumbnail(
    loader: &PageLoader,
    reader: &mut OpenReader,
    page_index: usize,
    priority: crate::loader::LoadPriority,
) {
    let total = reader.total_pages();
    if page_index >= total {
        return;
    }
    if reader.thumbnail_requests_sent.contains(&page_index) {
        return;
    }
    let Some(source) = reader.comic.page_source(page_index).cloned() else {
        return;
    };
    let sent = match priority {
        crate::loader::LoadPriority::High => {
            loader.request_high(reader.current_epoch, page_index, true, source)
        }
        crate::loader::LoadPriority::Low => {
            loader.request_low(reader.current_epoch, page_index, true, source)
        }
    };
    if sent {
        reader.thumbnail_requests_sent.insert(page_index);
    }
}

fn request_all_thumbnails(loader: &PageLoader, reader: &mut OpenReader) {
    let total = reader.total_pages();
    let (left, right) = reader.spread_pages();
    let visible: HashSet<usize> = [left, right].iter().filter_map(|&x| x).collect();
    for idx in 0..total {
        let priority = if visible.contains(&idx) {
            crate::loader::LoadPriority::High
        } else {
            crate::loader::LoadPriority::Low
        };
        request_thumbnail(loader, reader, idx, priority);
    }
}

const MAX_THUMBNAILS_PER_FRAME: usize = 32;

/// Retry any thumbnail requests that failed to send on the initial batch.
fn continue_thumbnail_batch(loader: &PageLoader, reader: &mut OpenReader) {
    let total = reader.total_pages();
    let (left, right) = reader.spread_pages();
    let visible: HashSet<usize> = [left, right].iter().filter_map(|&x| x).collect();
    let mut enqueued = 0;
    for idx in 0..total {
        if enqueued >= MAX_THUMBNAILS_PER_FRAME {
            break;
        }
        if reader.thumbnail_requests_sent.contains(&idx) {
            continue;
        }
        let priority = if visible.contains(&idx) {
            crate::loader::LoadPriority::High
        } else {
            crate::loader::LoadPriority::Low
        };
        let before = reader.thumbnail_requests_sent.len();
        request_thumbnail(loader, reader, idx, priority);
        if reader.thumbnail_requests_sent.len() > before {
            enqueued += 1;
        }
    }
}
```

`LoadPriority` is already `pub(crate)` from Task 1.

- [ ] **Step 4: Replace `request_preloads` with full-image window preloading**

Rename or rewrite `request_preloads` to:

```rust
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

        // Keep pushing thumbnail requests until every page has one in flight.
        continue_thumbnail_batch(loader, reader);

        reader
            .cache
            .enforce_budget_with_protected(budget, &reader.protected_page_indices());

        let current = reader.state.current_page;
        let total = reader.total_pages();
        if total == 0 {
            return;
        }

        // Prune full images outside the configured window.
        reader.cache.prune_full_images_outside_window(
            current,
            real_image_cache_pages,
            &reader.protected_page_indices(),
        );

        if reader.last_page_turn.elapsed() < PRELOAD_COOLDOWN_AFTER_TURN {
            return;
        }

        let (left, right) = reader.spread_pages();
        let visible: HashSet<usize> = [left, right].iter().filter_map(|&x| x).collect();

        let start = current.saturating_sub(real_image_cache_pages);
        let end = (current + real_image_cache_pages).min(total - 1);
        let mut enqueued = 0;
        const MAX_FULL_PRELOADS_PER_FRAME: usize = 16;
        for idx in start..=end {
            if enqueued >= MAX_FULL_PRELOADS_PER_FRAME {
                break;
            }
            if idx >= total {
                continue;
            }
            if reader.cache.get_full_texture_internal(idx).is_some()
                || reader.pending_pages.contains_key(&idx)
            {
                continue;
            }
            let Some(source) = reader.comic.page_source(idx).cloned() else {
                continue;
            };
            let sent = if visible.contains(&idx) {
                loader.request_high(reader.current_epoch, idx, false, source)
            } else {
                loader.request_low(reader.current_epoch, idx, false, source)
            };
            if sent {
                reader.pending_pages.insert(idx, Instant::now());
                enqueued += 1;
            }
        }
    });
}
```

Add a private helper in `PageCache` to check for a full image without a `ctx`:

```rust
fn get_full_texture_internal(&self, page_index: usize) -> Option<&LoadedImage> {
    self.textures.get(&page_index).and_then(|e| e.image.as_ref())
}
```

- [ ] **Step 5: Request all thumbnails on open**

In `ReaderView::open`, after `reader.bump_epoch(loader);`, add:

```rust
request_all_thumbnails(loader, &mut reader);
```

- [ ] **Step 6: Update `ui_inner` to use thumbnail fallback**

In `ui_inner`, replace the visible-page request block with:

```rust
if let Some(idx) = left_idx {
    if reader.cache.get_full_texture_internal(idx).is_none() {
        request_page(loader, reader, idx);
    }
    request_thumbnail(loader, reader, idx, crate::loader::LoadPriority::High);
}
if let Some(idx) = right_idx {
    if reader.cache.get_full_texture_internal(idx).is_none() {
        request_page(loader, reader, idx);
    }
    request_thumbnail(loader, reader, idx, crate::loader::LoadPriority::High);
}
```

Then compute textures:

```rust
let left_full = left_idx.and_then(|idx| reader.cache.get_full_texture(ctx, idx));
let right_full = right_idx.and_then(|idx| reader.cache.get_full_texture(ctx, idx));
let left_thumb = left_full
    .is_none()
    .then(|| left_idx.and_then(|idx| reader.cache.get_thumbnail_texture(ctx, idx)))
    .flatten();
let right_thumb = right_full
    .is_none()
    .then(|| right_idx.and_then(|idx| reader.cache.get_thumbnail_texture(ctx, idx)))
    .flatten();
```

Compute layout size from the cached `original_size` when available, otherwise
fall back to the texture size:

```rust
fn page_display_size(reader: &OpenReader, idx: usize, texture: Option<&egui::TextureHandle>) -> egui::Vec2 {
    reader
        .cache
        .original_size(idx)
        .map(|s| egui::vec2(s[0] as f32, s[1] as f32))
        .unwrap_or_else(|| texture_size_or_fallback(texture))
}

let left_size = left_idx
    .map(|idx| page_display_size(reader, idx, left_full.as_ref().or(left_thumb.as_ref())))
    .unwrap_or(FALLBACK_PAGE_SIZE);
let right_size = match right_idx {
    None => egui::Vec2::ZERO,
    Some(idx) => page_display_size(reader, idx, right_full.as_ref().or(right_thumb.as_ref())),
};

let any_loading = (left_idx.is_some() && left_full.is_none())
    || (right_idx.is_some() && right_full.is_none());
if !any_loading {
    reader.apply_pending_fit(ctx, available.size());
}
```

When calling `render_page_or_placeholder`, pass the full texture if present,
otherwise the thumbnail:

```rust
responses.push(render_page_or_placeholder(
    ui,
    reader,
    loader,
    left_rect,
    idx,
    left_full.as_ref().or(left_thumb.as_ref()),
));
```

Do the same for the right page.

- [ ] **Step 7: Update `render_page_turn_animation` similarly**

After fetching `from_texture` / `to_texture`, request thumbnails and full images:

```rust
if from_texture.is_none() {
    if reader.cache.get_full_texture_internal(from_idx).is_none() {
        request_page(loader, reader, from_idx);
    }
    request_thumbnail(loader, reader, from_idx, crate::loader::LoadPriority::High);
}
if to_texture.is_none() {
    if reader.cache.get_full_texture_internal(to_idx).is_none() {
        request_page(loader, reader, to_idx);
    }
    request_thumbnail(loader, reader, to_idx, crate::loader::LoadPriority::High);
}
```

(Assume `from_texture` / `to_texture` are renamed to the best available texture
`from_display` / `to_display`, computed as
`get_full_texture(...).or(get_thumbnail_texture(...))`.)

Use `page_display_size(reader, idx, display_texture.as_ref())` for both sizes,
then pass `display_texture.as_ref()` to `render_page_or_placeholder`.

- [ ] **Step 8: Update `ReaderView::update` dispatch and add prune**

Change the wrapper signature in `ReaderView`:

```rust
pub fn update(
    &mut self,
    ctx: &egui::Context,
    loader: &PageLoader,
    cache_size_mb: usize,
    real_image_cache_pages: usize,
) {
    let budget = cache_size_mb * 1024 * 1024;
    if let Some(reader) = &mut self.open {
        reader.update(ctx, loader, budget, real_image_cache_pages);
    }
}
```

Change the `OpenReader::update` signature to accept `real_image_cache_pages`:

```rust
pub fn update(
    &mut self,
    ctx: &egui::Context,
    loader: &PageLoader,
    cache_size_bytes: usize,
    real_image_cache_pages: usize,
)
```

Inside the result-handling loop, dispatch by `result.thumbnail`:

```rust
let cache_size_bytes = cache_size_mb * 1024 * 1024;
while let Some(result) = loader.try_recv() {
    if result.epoch != self.current_epoch {
        continue;
    }
    // ... existing logging ...
    if result.thumbnail {
        match result.image {
            Ok(thumb) => {
                self.cache
                    .insert_thumbnail(result.page_index, thumb, result.original_size);
            }
            Err(err) => {
                eprintln!("failed to load thumbnail page {}: {}", result.page_index, err);
                self.thumbnail_requests_sent.remove(&result.page_index);
            }
        }
    } else {
        self.pending_pages.remove(&result.page_index);
        match result.image {
            Ok(image) => {
                let protected = self.protected_page_indices();
                self.cache.insert(
                    result.page_index,
                    image,
                    result.original_size,
                    cache_size_bytes,
                    &protected,
                );
            }
            Err(err) => {
                eprintln!("failed to load page {}: {}", result.page_index, err);
                self.page_errors.insert(result.page_index, err);
            }
        }
    }
}

self.cache.prune_full_images_outside_window(
    self.state.current_page,
    real_image_cache_pages,
    &self.protected_page_indices(),
);
```

- [ ] **Step 9: Update `render_progress_thumbnail` to use thumbnail texture**

In `rust-reader-app/src/widgets/thumbnail_progress_bar.rs`, replace the body
that calls `cache.get_texture(...)` with:

```rust
if let Some(handle) = cache.get_thumbnail_texture(ctx, page_index) {
    ui.put(
        rect,
        egui::Image::new(&handle).fit_to_exact_size(rect.size()),
    );
} else {
    // existing page-number fallback
}
```

If `get_texture` was the only caller and the alias is no longer used elsewhere,
the alias can be removed in a later cleanup.

- [ ] **Step 10: Update `app.rs` to pass the new setting**

In `rust-reader-app/src/app.rs`, change:

```rust
self.reader_view
    .update(ctx, &self.page_loader, cache_size_mb);
self.reader_view
    .request_preloads(&self.page_loader, cache_size_mb);
```

to:

```rust
let real_image_cache_pages = self.settings.real_image_cache_pages as usize;
self.reader_view
    .update(ctx, &self.page_loader, cache_size_mb, real_image_cache_pages);
self.reader_view
    .request_preloads(&self.page_loader, cache_size_mb, real_image_cache_pages);
```

- [ ] **Step 11: Add reader tests**

Add to the reader test module:

```rust
#[test]
fn test_request_all_thumbnails_sends_one_per_page() {
    let loader = PageLoader::new();
    let mut reader = dummy_reader();
    reader.bump_epoch(&loader);
    request_all_thumbnails(&loader, &mut reader);
    assert_eq!(reader.thumbnail_requests_sent.len(), 10);
}

#[test]
fn test_full_image_window_only_requests_inside_range() {
    let loader = PageLoader::new();
    let mut reader = dummy_reader();
    reader.bump_epoch(&loader);
    reader.state.current_page = 5;
    let cache_pages = 2;

    // Request visible page full image and thumbnail.
    request_page(&loader, &mut reader, 5);
    request_thumbnail(&loader, &mut reader, 5, crate::loader::LoadPriority::High);

    // Simulate the windowed full-image preload.
    let current = reader.state.current_page;
    let total = reader.total_pages();
    let start = current.saturating_sub(cache_pages);
    let end = (current + cache_pages).min(total - 1);
    for idx in start..=end {
        if idx == current || reader.cache.get_full_texture_internal(idx).is_some() {
            continue;
        }
        if let Some(source) = reader.comic.page_source(idx).cloned() {
            loader.request_low(reader.current_epoch, idx, false, source);
            reader.pending_pages.insert(idx, Instant::now());
        }
    }

    for idx in 0..total {
        let in_window = idx >= start && idx <= end;
        if reader.pending_pages.contains_key(&idx) {
            assert!(in_window, "page {} is outside the configured window", idx);
        }
    }
}
```

- [ ] **Step 12: Run reader tests**

```bash
cargo test -p rust-reader-app reader
```

Expected: PASS.

- [ ] **Step 13: Commit ReaderView wiring**

```bash
git add rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs rust-reader-app/src/loader.rs
# include any other changed files
git commit -m "feat(reader): thumbnail-first rendering and windowed full-image preload

- OpenReader tracks thumbnail_requests_sent and requests all on open
- Visible pages request thumbnail + full image each frame
- Rendering falls back from full texture to thumbnail texture
- Replace old preloader with [current-N, current+N] window
- app.rs passes real_image_cache_pages setting"
```

---

## Task 6: Full verification and final commit

- [ ] **Step 1: Format and lint**

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
```

Expected: no warnings.

- [ ] **Step 2: Run full test suite**

```bash
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 3: Smoke test manually (if possible)**

Run:

```bash
cargo run -p rust-reader-app
```

Open a comic and verify:

1. The visible pages show a low-resolution image immediately (thumbnail).
2. After a short delay the image sharpens (full image replacement).
3. Flipping pages remains responsive.

- [ ] **Step 4: Update TODO.md**

Add or mark a new TODO item for thumbnail-first rendering as done, e.g.:

```markdown
- [x] 24. 缩略图优先渲染 + 真实图片窗口预读
```

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat: thumbnail-first rendering with configurable full-image window

- Load thumbnails for all pages on open (visible pages high priority)
- Render thumbnails first, then replace with full-resolution images
- Preload full images only within [current-N, current+N] window
- Add real_image_cache_pages setting (default 50 pages each direction)
- Remove dead/duplicate code identified during design"
```

---

## Self-Review Checklist

- **Spec coverage:** Every section of `docs/superpowers/specs/2026-06-22-thumbnail-first-rendering-design.md` maps to one or more tasks above.
- **Placeholder scan:** No TBD, TODO, or "implement later" steps.
- **Type consistency:** `LoadResult`, `LoadRequest`, `DecodeJob` all carry `thumbnail` and `original_size`; `PageCache::insert` and `insert_thumbnail` accept `original_size`; `ReaderView::update`/`request_preloads` accept `real_image_cache_pages`.
- **Scope:** The plan does not add disk thumbnail cache, native DXT5 thumbnails, or error overlays on thumbnails — those remain out of scope per the spec.
