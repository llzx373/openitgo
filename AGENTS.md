# rustReader Agent Instructions

## Project Overview

`rustReader` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
It supports ZIP/CBZ, RAR/CBR, PDF, and folders of images, plus EPUB, TXT, MOBI/AZW3, and
Markdown ebooks through an embedded `wry` webview renderer.

## Repository Layout

- `rust-reader-core/` — shared models, reading-state machine, and layout math.
- `rust-reader-parser/` — archive/folder/PDF parsers and comic ID generation.
- `rust-reader-storage/` — JSON persistence for settings, library, history, and bookmarks.
- `rust-reader-app/` — egui application, cache, loader, and UI views.
- `docs/` — audit reports, bug notes, and implementation plans.

## Build & Test

Run the full verification pipeline before committing:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Coding Conventions

- Use `cargo fmt` for formatting.
- Keep `clippy` warnings at zero (`-D warnings`).
- Prefer minimal, focused changes; avoid unrelated refactoring.
- Update relevant tests when changing public interfaces.
- Keep UI text in Chinese unless it is a proper noun or technical identifier.

## Key Architectural Notes

- **Comic IDs** are generated deterministically from the file/folder path via
  `rust_reader_parser::stable_comic_id`. Never use the filename alone.
- **PageLoader** runs IO and decode workers in background threads; results are
  sent back to the UI thread via channels. The app also maintains a separate
  `cover_loader` for library cover thumbnails.
- **PageCache** stores GPU textures and keeps `size_bytes` as an estimate of
  either CPU image memory or equivalent GPU memory. CPU-side `ColorImage` is
  released after upload when possible.
- **Settings** are validated on load/save; invalid values are clamped and the
  user is informed through `error_message`. Notable fields include
  `theme`, `default_mode`, `default_fit`, `double_page`,
  `wide_page_threshold`, `enable_page_animation`, `compress_images`,
  `decode_threads`, `cache_size_mb`, `real_image_cache_pages`,
  `show_toolbar`, `show_statusbar`, `invert_scroll`, and `library_sort`.
- **History entries** store both `comic_id` and `path` for robust matching.
- **Library covers** are generated asynchronously from the first page and saved
  to `covers/`. Missing covers are re-requested on demand, and entries whose
  source file no longer exist are marked as deleted.
- **EbookRenderer** hosts a `wry` child webview and serves a small HTML reader
  shell over the custom `ebook://` protocol. Chapter content is fetched via
  `ebook://reader?chapter=N` and rendered by `rust_reader_parser::html::render_chapter_html`.
  Pagination is handled by the embedded CSS `columns` paginator; the JS side uses
  a `sendIpc` helper that retries if the `window.ipc` bridge is not yet injected.
- **EbookRenderer position preservation** distinguishes settings changes from window
  resize: font/size/margin/theme changes re-layout and preserve the approximate
  character offset, while window resize debounces and preserves the scroll ratio
  in scroll mode or the current spread in paginated modes.

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
