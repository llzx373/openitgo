# Thumbnail-First Rendering Design

> **Status:** Implemented and optimized. See commits `28e50f0`, `1171e87`, `59cdb43`.

## Goal

Refactor the comic reader rendering pipeline so that opening a file only loads
low-resolution thumbnails for all pages in the background. The visible spread
is first rendered with thumbnails, and full-resolution images are loaded
afterwards to replace them. Full-resolution images are preloaded within a
configurable window around the current page (default 50 pages in each
direction).

## Context

The current implementation (`rust-reader-app/src/views/reader.rs`) requests a
single full-resolution image for each visible page every frame. While
preloading exists, there is no separate thumbnail tier: pages either show the
full image or a static "loading" placeholder. This leads to visible stalls when
opening large archives or flipping quickly through a comic.

## Design Decisions

| Question | Decision |
|---|---|
| Thumbnail max dimension | 256px |
| Thumbnail format | RGBA8 `ColorImage` (no DXT5) |
| Thumbnail retention | Keep all thumbnails for the current comic in memory |
| Thumbnail generation | Batch all pages on open; visible pages get high priority |
| Real-image cache | Configurable `real_image_cache_pages`, default 10 each direction |
| Real-image preloading | Only inside `[current - N, current + N]` |
| Loader approach | Reuse `PageLoader`, add `thumbnail` flag to requests/results |

## Architecture

### Data Flow

```text
Open comic
  │
  ▼
Request thumbnails for ALL pages
  (visible pages → High, others → Low)
  │
  ▼
IO workers read bytes / PDF renders page
  │
  ▼
Decode workers downsample to 256px → LoadedImage::Color
  │
  ▼
ReaderView::update stores thumbnail in PageCache
  │
  ▼
ReaderView::ui renders thumbnail while full image loads
  │
  ▼
Request full images in [current - N, current + N]
  │
  ▼
Full images replace thumbnails on screen
```

### Loader Changes

Files: `rust-reader-app/src/loader.rs`

- Add `thumbnail: bool` to `LoadRequest`, `DecodeJob`, and `LoadResult`.
- In the decode worker:
  - If `thumbnail` is true, decode the source image, downsample it so the
    longest side is at most 256px, and return `LoadedImage::Color(thumbnail)`.
  - If `thumbnail` is false, keep the existing behavior (decode + optional
    DXT5 compression).
- For PDF sources, render the page using the existing `render_pdf_page` path,
  then downsample the resulting image to 256px when generating a thumbnail.

### Cache Changes

Files: `rust-reader-app/src/cache.rs`

`CacheEntry` is extended to hold both a thumbnail and a full image:

```rust
struct CacheEntry {
    thumbnail: Option<LoadedImage>,
    thumbnail_handle: Option<TextureHandle>,
    image: Option<LoadedImage>,
    handle: Option<TextureHandle>,
    size_bytes: usize,
    last_accessed: Instant,
}
```

- `size_bytes` continues to track only the full image size; thumbnails are
  retained unconditionally, so they do not participate in LRU budget eviction.
- New methods:
  - `insert_thumbnail(page_index, thumbnail)` — stores and lazily uploads a
    thumbnail texture.
  - `get_full_texture(ctx, page_index)` — returns the full image texture only.
  - `get_thumbnail_texture(ctx, page_index)` — returns the thumbnail texture
    only.
  - `prune_full_images_outside_window(current, window, protected)` — removes
    full images for pages outside `[current - window, current + window]`,
    leaving thumbnails intact.

### ReaderView Changes

Files: `rust-reader-app/src/views/reader.rs`

- Add to `OpenReader`:
  - `thumbnail_requests_sent: HashSet<usize>` to avoid duplicate thumbnail
    requests.
- On `open` / `bump_epoch`:
  - Clear `thumbnail_requests_sent`.
- New helpers:
  - `request_thumbnail(loader, reader, page_index, priority)` — sends a
    thumbnail `LoadRequest` and records the page in `thumbnail_requests_sent`.
  - `request_all_thumbnails(reader, loader)` — iterates all pages, sending high
    priority requests for visible pages and low priority requests for the rest.
  - `request_full_image_window(reader, loader, cache_pages)` — requests full
    images for pages inside `[current - cache_pages, current + cache_pages]`.
    Visible pages use high priority; the rest use low priority.
- `ui_inner` rendering:
  1. For each visible page slot, call `request_thumbnail(..., High)` and the
     existing `request_page` (full image, high).
  2. Render using `cache.get_full_texture()` if available; otherwise fall back
     to `cache.get_thumbnail_texture()`; otherwise show the loading/error
     placeholder.
- `update` dispatch:
  - If `result.thumbnail` is true, call `cache.insert_thumbnail(...)`.
  - Otherwise call the existing `cache.insert(...)` for the full image.

### Settings Changes

Files:
- `rust-reader-storage/src/models.rs`
- `rust-reader-app/src/views/settings.rs`

Add a new setting:

```rust
pub real_image_cache_pages: u32, // default 10
```

- Update `Default for Settings`.
- Add a slider in the settings UI (range 0–200).
- Update storage round-trip tests.

### Memory and Cancellation

- Thumbnails for the current comic are kept in memory for the entire reading
  session.
- Full images are bounded by the configurable window: at most
  `2 * real_image_cache_pages + 1` full images are retained, plus any
  currently protected visible/animating pages.
- When the user opens a new comic or jumps to a different page range,
  `bump_epoch` increments the loader epoch. Results from the old epoch are
  discarded, preventing stale thumbnail/full-image writes.
- Thumbnail requests do not have a separate timeout. If the loader queue is
  full and returns an error result, the page is removed from
  `thumbnail_requests_sent` so the next frame can retry.

## Error Handling

- If a thumbnail fails to load, the page falls back to the existing error
  placeholder; retrying the thumbnail works the same way as retrying a full
  image.
- If a full image fails but its thumbnail is available, the thumbnail remains
  on screen with an optional small error indicator (out of scope for the
  initial implementation; keep the existing placeholder behavior).

## Testing

- Loader:
  - Decoding a thumbnail returns an image whose longest side is ≤ 256px.
  - Thumbnail results carry `thumbnail: true`.
- Cache:
  - `insert_thumbnail` + `get_thumbnail_texture` works independently of the
    full image.
  - `prune_full_images_outside_window` removes only full images outside the
    window and preserves thumbnails.
- Reader:
  - `request_all_thumbnails` sends exactly one request per page.
  - `request_full_image_window` only requests pages inside the configured
    window.

## Code Cleanup

The refactor is a good opportunity to remove duplicate and dead code that
would otherwise need to be updated for the new thumbnail/full-image split.

### Safe Removals

| File | Item | Action |
|---|---|---|
| `rust-reader-app/src/widgets/page_view.rs` | entire module | Remove; `PageCache::get_texture` already handles decompress + upload. Remove its declaration from `widgets/mod.rs`. |
| `rust-reader-app/src/loader.rs` | `PageLoader::request` | Remove; only `request_high` / `request_low` are used. |
| `rust-reader-app/src/loader.rs` | `LoadPriority` visibility | Make private once `request` is gone. |
| `rust-reader-app/src/loader.rs` | `CompressedFormat` enum + `LoadedImage::Compressed.format` | Remove; only `Dxt5Srgb` exists and the field is never read. Update `make_compressed` test helper. |
| `rust-reader-app/src/cache.rs` | `PageCache::enforce_budget` | Remove; it duplicates `enforce_budget_with_protected(max, &[])`. Update the one test that uses it. |
| `rust-reader-app/examples/diagnose_hang.rs` | entire example | Remove; it was a diagnostic artifact for the now-fixed rapid page-turn hang. |
| `rust-reader-app/examples/rapid_flip.rs` | unused `PageSource` import | Remove. |

### Consolidations

| Files | Duplication | Action |
|---|---|---|
| `rust-reader-app/src/views/reader.rs` and `rust-reader-app/src/widgets/progress_bar.rs` | identical `page_at_x` helper | Export one copy from `widgets/progress_bar.rs` and reuse it in `reader.rs`. |
| `rust-reader-app/src/views/reader.rs` | texture-size/fallback computed twice | Extract `texture_size_or_fallback(Option<&TextureHandle>) -> Vec2`. |

### Optional / Bigger Cleanup (out of scope unless explicitly requested)

- `rust-reader-storage/src/models.rs` `default_fit` and `theme` settings are
  stored and exposed in the UI but not wired to behavior.
- `rust-reader-core/src/layout.rs` is exposed but never imported by the app.
- `rust-reader-app/examples/rapid_flip.rs` and `flip_through.rs` overlap heavily.

## Out of Scope

- Persistent on-disk thumbnail cache.
- Separate thumbnail loader thread pool.
- Animated placeholder or progress indicator during thumbnail generation.
- Displaying a distinct error overlay on top of a thumbnail when the full
  image fails.
- Fully wiring or removing the unrelated `default_fit` / `theme` settings.
