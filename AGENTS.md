# rustReader Agent Instructions

## Project Overview

`rustReader` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
It supports ZIP/CBZ, RAR/CBR, PDF, and folders of images, plus EPUB, TXT, MOBI/AZW3, and
Markdown ebooks through an embedded `wry` webview renderer, and plays video/audio files
through an embedded `libmpv` backend.

## Repository Layout

- `rust-reader-core/` — shared models, reading-state machine, and layout math.
- `rust-reader-parser/` — archive/folder/PDF parsers and comic ID generation.
- `rust-reader-storage/` — JSON persistence for settings, library, history, and bookmarks.
- `rust-reader-media/` — libmpv wrapper: commands, event pump, property observation,
  OpenGL render context, and headless cover generation.
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

Media playback requires libmpv from Homebrew (`brew install mpv`) to build
`rust-reader-media` and to run the app from source; packaged `.app` bundles
already embed it.

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
  `show_toolbar`, `show_statusbar`, `invert_scroll`, `library_sort`,
  `media_volume`, `media_speed`, and `media_audio_device`.
- **History entries** store both `comic_id` and `path` for robust matching.
- **Library covers** are generated asynchronously from the first page and saved
  to `covers/`. Missing covers are re-requested on demand, and entries whose
  source file no longer exist are marked as deleted.
- **EbookRenderer** hosts a `wry` child webview and serves a small HTML reader
  shell over the custom `ebook://` protocol. Chapter content is fetched via
  `ebook://reader?chapter=N` and rendered by `rust_reader_parser::html::render_chapter_html`.
  Pagination is handled by the embedded CSS `columns` paginator; the JS side uses
  a `sendIpc` helper that retries if the `window.ipc` bridge is not yet injected.
  Pagination transforms must be applied to `#column-content` inside `#column-view`;
  `#column-view` itself is the click/wheel event container and must not be translated.
- **EbookRenderer position preservation** distinguishes settings changes from window
  resize: font/size/margin/theme changes re-layout and preserve the approximate
  character offset, while window resize debounces and preserves the scroll ratio
  in scroll mode or the current spread in paginated modes.
- **Media playback** renders mpv video through a CAOpenGLLayer inserted into
  the superlayer of the winit view's CAMetalLayer, anchored BELOW it via
  `insertSublayer:below:` (the view's layer IS wgpu's CAMetalLayer — wgpu-hal
  adopts it as the main layer) (`rust-reader-app/src/platform/macos/mpv_view.rs`).
  The app runs with a
  transparent backbuffer (`with_transparent(true)` + `clear_color` returning
  zero alpha) and the media view's CentralPanel uses a transparent frame, so
  the video shows through the unpainted central area while egui menus,
  dropdowns and popups composite above it. Hit-testing is unaffected (the
  egui NSView still receives all events). Bare-layer geometry changes must go
  through a `CATransaction` with disabled actions; the OSD opacity fade
  relies on implicit animation and must stay outside such transactions.
  Playback progress is persisted in `HistoryEntry.char_offset` (milliseconds).
  Inside `drawInCGLContext`, CoreAnimation binds its own drawable FBO (observed:
  1/2, alternating — never 0); the draw must query `GL_FRAMEBUFFER_BINDING` and
  pass it to `RenderContext::render`, because rendering to FBO 0 leaves the
  layer's drawable untouched and composites fully transparent. `FLIP_Y` must be
  1 for this drawable. Audio output defaults to the system device (`auto`) and
  can be switched at runtime (see Media preferences below).
- **Media OSD**: transient feedback (volume, mute, seeks, speed, device
  switches) renders in a CATextLayer sublayer of the CAOpenGLLayer
  (`MpvNativeView::set_osd/clear_osd`) — the CATextLayer lives inside the
  video layer below egui and shows through the transparent central area.
  When the native view is parked at zero size (audio-only or
  decode-error overlay), `MediaView::ui` paints the same text top-right with
  the egui painter instead. `MediaView::show_osd` stores the text plus the 1s
  expiry (`Osd`); `tick_osd` clears both paths. CoreAnimation's implicit
  opacity animation provides the native fade.
- **Media menus/popups**: with the video layer below the transparent egui
  surface, egui overlays (menu-bar menus, the 字幕/音轨/输出 dropdowns)
  naturally render above the video. `menu_overlay_open(ctx)` (visible
  `Order::Middle`/`Order::Foreground` areas) is still used to keep the media
  toolbar from auto-hiding in fullscreen while a menu is open. The media
  seek bar needs a scoped `ui.spacing_mut().slider_width` override: egui 0.29
  `Slider` always allocates `slider_width` (100px) and ignores `add_sized`.
  The diagnostic examples `probe_overlay.rs` (transparent-compositing proof)
  and `probe_visible.rs` (real video compositing) verify the layering.
- **Media preferences**: volume/speed/audio-device are persisted globally in
  `Settings` and applied by `MediaView::apply_startup_settings` after open;
  a missing saved device falls back to "auto".
- **MpvPlayer teardown** order matters: `Drop` sets a quit flag and joins the
  `mpv-events` thread (50ms `mpv_wait_event` timeout) *before*
  `mpv_terminate_destroy` — a `mpv_wait_event` call racing the handle free
  segfaults inside libmpv.
- **Media diagnostic examples**: `rust-reader-app/examples/probe_visible.rs`
  (visible window, real CA compositing, screenshot-verifiable),
  `probe_mpv_view.rs` (offscreen overlay),
  `probe_overlay.rs` (transparent-compositing proof for the
  video-below-egui layering), `probe_video_overlay.rs` (real video layer
  compositing verification), and
  `rust-reader-media/examples/{probe,probe_render}.rs` (headless player/render
  context). `RUST_READER_MPV_LOG=1` enables mpv debug logs on stderr.
- **Packaging**: `scripts/package-macos.sh` runs `bundle_mpv` before signing,
  copying libmpv and its Homebrew dependencies into `Contents/Frameworks` and
  rewriting their install names to `@rpath`, so the bundled app runs without a
  Homebrew mpv installation.

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
