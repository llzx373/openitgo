# OpenItGo Agent Instructions

## Project Overview

`OpenItGo` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
It supports ZIP/CBZ, RAR/CBR, PDF, and folders of images, plus EPUB, TXT, MOBI/AZW3, and
Markdown ebooks through an embedded `wry` webview renderer, and plays video/audio files
through an embedded `libmpv` backend.

## Repository Layout

- `openitgo-core/` — shared models, reading-state machine, and layout math.
- `openitgo-parser/` — archive/folder/PDF parsers and comic ID generation.
- `openitgo-storage/` — JSON persistence for settings, library, history, bookmarks,
  per-comic reading settings, and reading stats (`reading_stats.json`).
- `openitgo-media/` — libmpv wrapper: commands, event pump, property observation,
  OpenGL render context, and headless cover generation. `args.rs`/`apply.rs`
  为 FFI-free 纯函数模块（命令参数构造、事件状态迁移），ubuntu CI 可测。
- `openitgo-app/` — egui application, cache, loader, and UI views.
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
`openitgo-media` and to run the app from source; packaged `.app` bundles
already embed it.

## Coding Conventions

- Use `cargo fmt` for formatting.
- Keep `clippy` warnings at zero (`-D warnings`).
- Prefer minimal, focused changes; avoid unrelated refactoring.
- Update relevant tests when changing public interfaces.
- Keep UI text in Chinese unless it is a proper noun or technical identifier.

## Key Architectural Notes

- **Comic IDs** are generated deterministically from the file/folder path via
  `openitgo_parser::stable_comic_id`. Never use the filename alone.
- **加密压缩包密码**：`parse_with_password(path, Option<&str>)` 是带密码入口
  （`parse` 转调 None）；密码只存会话级 `ReaderApp.passwords`
  （`HashMap<PathBuf, String>`，不落盘），经 `PageLoader::passwords()` 共享给
  IO worker 用于 `by_index_decrypt` / `Archive::with_password` 解密读取；
  AsyncOpener 错误串用 `\u{1}` 前缀标记密码类错误供 poll_opener 识别。
  RAR 数据加密包（`rar -p`）列表可读，解析期靠首条目读探针分类
  （`MissingPassword`/`BadPassword`/CRC `BadData` → 密码错误）。
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
- **Per-comic reading settings** (`comic_settings.json`,
  `HashMap<String, ComicReadingSettings>` keyed by comic_id) remember each
  book's mode/double-page/fit/rotation. They override the global defaults only when a
  comic is opened (`poll_opener`: `set_mode` like the mode menu,
  `set_double_page`, then assign `fit_mode` so it flows through the same
  pending-fit path as `default_fit`, then assign `rotation` accepting only
  90° steps — dirty values fall back to 0). Changes from any source (menu, toolbar,
  shortcuts, double-click fit toggle) are caught by
  `ReaderApp::maybe_save_comic_settings` at the end of `App::update`, which
  diffs the open comic's `(mode, double_page, fit_mode, rotation)` against
  `last_saved_comic_settings` and writes on change; the snapshot is reset on
  open/close, and a failed save still updates it to avoid per-frame error
  spam. Global `settings.double_page` etc. keep updating as before — the
  per-book memory is an open-time override layer only.
- **Library covers** are generated asynchronously from the first page and saved
  to `covers/`. Missing covers are re-requested on demand, and entries whose
  source file no longer exist are marked as deleted.
  Bookmark thumbnails live in `covers/bookmarks/<comic_id>-p<page>.jpg`,
  generated on bookmark creation through the cover_loader channel and removed
  with the bookmark/book (comic bookmarks only; ebook bookmarks fall back to
  the cover in the bookmark list).
- **EbookRenderer** hosts a `wry` child webview and serves a small HTML reader
  shell over the custom `ebook://` protocol. Chapter content is fetched via
  `ebook://reader?chapter=N` and rendered by `openitgo_parser::html::render_chapter_html`.
  Pagination is handled by the embedded CSS `columns` paginator; the JS side uses
  a `sendIpc` helper that retries if the `window.ipc` bridge is not yet injected.
  Pagination transforms must be applied to `#column-content` inside `#column-view`;
  `#column-view` itself is the click/wheel event container and must not be translated.
  The custom-protocol callback's Request URI is the full absolute URL: in
  `ebook://reader/res/...` the `reader` part is the host in `http::Uri` semantics
  and never appears in `uri().path()`, so resource discriminators must live in
  the path (`/res/<archive-path>`) or query (`?chapter=N`) — never expect a host
  segment to show up in the path.
  **EbookRenderer menu parking（#52）**：egui 弹层无法穿透原生 webview，
  菜单/浮层打开时 `render_ebook` 用 `menu_overlay_open(ctx)`（与媒体视图
  同一判定）驱动 `EbookView::set_webview_hidden` 调 wry `set_visible(false)`，
  正文区以 `ebook_theme_bg(theme)` 填充；状态去重（`visibility_transition`）
  避免每帧重复 IPC，关闭即恢复。诊断探针：
  `cargo run -p openitgo-app --example probe_ebook_menu -- <epub路径>`。
- **EbookRenderer position preservation** distinguishes settings changes from window
  resize: font/size/margin/theme changes re-layout and preserve the approximate
  character offset, while window resize debounces and preserves the scroll ratio
  in scroll mode or the current spread in paginated modes.
- **Media playback** renders mpv video through a CAOpenGLLayer inserted into
  the superlayer of the winit view's CAMetalLayer, anchored BELOW it via
  `insertSublayer:below:` (the view's layer IS wgpu's CAMetalLayer — wgpu-hal
  adopts it as the main layer) (`openitgo-app/src/platform/macos/mpv_view.rs`).
  The app runs with a
  transparent backbuffer (`with_transparent(true)` + `clear_color` returning
  zero alpha) and the media view's CentralPanel uses a transparent frame, so
  the video shows through the unpainted central area while egui menus,
  dropdowns and popups composite above it. Hit-testing is unaffected (the
  egui NSView still receives all events). The egui control bars are
  repainted by the mpv event-pump thread calling
  `egui::Context::request_repaint()`. Bare-layer geometry changes must go
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
  expiry (`Osd`); `tick_osd` clears both paths and, while unexpired, re-arms
  `request_repaint_after` for the remaining time — egui fires the scheduled
  frame slightly early (predicted frame time is subtracted) and only once,
  so without the re-arm an idle app (e.g. after EOF) never clears the OSD.
  `MediaView::close()` clears the OSD state so it cannot leak into the next
  opened media. CoreAnimation's implicit
  opacity animation provides the native fade.
- **Media menus/popups**: with the video layer below the transparent egui
  surface, egui overlays (menu-bar menus, the 字幕/音轨/输出 dropdowns)
  naturally render above the video. `menu_overlay_open(ctx)` (visible
  `Order::Middle`/`Order::Foreground` areas) is still used to keep the media
  toolbar from auto-hiding in fullscreen while a menu is open. The media
  seek bar needs a scoped `ui.spacing_mut().slider_width` override: egui 0.29
  `Slider` always allocates `slider_width` (100px) and ignores `add_sized`.
  The diagnostic examples `probe_visible.rs` (bare window without an egui
  surface — exercises the index-0 fallback) and `probe_video_overlay.rs`
  (real video compositing below the transparent egui surface) verify the
  layering.
- **Media preferences**: volume/speed/audio-device are persisted globally in
  `Settings` and applied by `MediaView::apply_startup_settings` after open.
  Volume/speed are set immediately; the audio device is deferred
  (`pending_startup_device`) until the async `audio-device-list` reply lands
  in `PlayerState::audio_devices`, then validated in `sync_state` —
  a missing saved device falls back to "auto" and is reported once via
  `take_startup_device_invalid` (the app clears the stale setting).
- **Media auto-next (自动续播)**: `ReaderApp::maybe_auto_next_media` runs in
  `render_media` right after `sync_state`; when `open.last.ended` is true
  with no `error`, it opens the next media file in the same directory
  exactly once per opened media (`MediaView::auto_next_fired`, reset in
  `MediaView::open`). The successor comes from `next_media_in_dir`
  (`app.rs`), sorted by the numeric-aware, case-insensitive `natural_cmp`
  ("EP2" < "EP10"). The "自动播放下一集" OSD is stashed in
  `MediaView::pending_open_osd` and shown by `MediaView::open` after the new
  media is up — showing it before the swap would paint on the old native
  view, which the swap destroys. Playback errors (`error` non-empty) never
  trigger auto-next; at the last episode a one-shot `已是最后一集` OSD shows
  instead.
- **MpvPlayer command rule**: every mpv command/property call made from the
  UI thread MUST use the async libmpv APIs (`mpv_command_async`,
  `mpv_set_property_async`, `mpv_get_property_async` — see
  `openitgo-media/src/player.rs`). A blocking call (`mpv_command`,
  `mpv_get_property`, ...) parks the UI thread on mpv's core dispatch queue,
  which can itself be waiting for first-frame DR image allocation — and that
  allocation can only be serviced by the UI thread answering
  `mpv_render_context_update()`. The resulting circular wait froze the
  window on media re-open (docs/superpowers/reports/2026-07-17-bug-notes-archived.md
  问题 A). The `audio-device-list`
  reply is parsed on the event thread into `PlayerState::audio_devices`.
- **MpvPlayer observe/userdata 分配**：属性观察 id 1-9（9 = `chapter`），
  异步查询 userdata 100（`audio-device-list`）/ 101（`chapter-list`），
  常量为 `apply.rs` 的 `AUDIO_DEVICES_REPLY_USERDATA`/`CHAPTER_LIST_REPLY_USERDATA`；
  下一可用观察 id 10、userdata 102。`chapter-list` 在 FILE_LOADED 与
  需要时经 `request_chapter_list` 拉取，解析入 `PlayerState.chapters`。
- **MpvPlayer teardown** order matters: `Drop` sets a quit flag and joins the
  `mpv-events` thread (50ms `mpv_wait_event` timeout) *before*
  `mpv_terminate_destroy` — a `mpv_wait_event` call racing the handle free
  segfaults inside libmpv.
- **Media diagnostic examples**: `openitgo-app/examples/probe_visible.rs`
  (visible window, real CA compositing, screenshot-verifiable),
  `probe_mpv_view.rs` (offscreen overlay), `probe_video_overlay.rs` (real
  video layer compositing verification), and
  `openitgo-media/examples/{probe,probe_render}.rs` (headless player/render
  context). `OPENITGO_MPV_LOG=1` enables mpv debug logs on stderr.
- **Reader diagnostic examples** (`openitgo-app/examples/`，用法均为
  `cargo run -p openitgo-app --example <name> -- <漫画路径>`)：
  - `flip_through.rs`：逐页顺序请求整本漫画，报告每页成功/错误/超时（PageLoader 全页遍历冒烟）。
  - `rapid_flip.rs`：以 80ms 间隔连续请求全部页面，统计加载延迟与错误（快速翻页压力回归）。
  - `profile_open.rs`：带 UI 打开漫画，10 秒后打印缓存快照（总页数/缩略图/全尺寸页数）并自动退出（打开性能剖析）。
  - `profile_view.rs`：同 `profile_open.rs` 但持续运行，每 10 秒打印一次缓存快照（浏览期缓存观察）。
  - `ui_smoke.rs`：带 UI 打开漫画，当前页进入缓存即自动退出（30 秒超时），UI 启动冒烟。
- **Packaging**: `scripts/package-macos.sh` runs `bundle_mpv` before signing,
  copying libmpv and its Homebrew dependencies into `Contents/Frameworks` and
  rewriting their install names to `@rpath`, so the bundled app runs without a
  Homebrew mpv installation.
- **Dock open (macOS)**: `platform::macos::dock_open` swizzles the
  NSApplication delegate to queue files from `application:openURLs:` /
  `application:openFiles:` / `application:openFile:` into `OPEN_QUEUE`, drained
  once per frame in `App::update`. An idle egui app does not repaint, so the
  callbacks must wake the event loop: `set_wake_context` (registered from the
  app creator in `main.rs`) stores an `egui::Context` that `enqueue_paths`
  calls `request_repaint()` on after every enqueue. Removing the wake
  re-introduces the "dock-opened files stall until the next repaint" bug.
  The macOS platform layer (`dock_open`, `mpv_view`, probe examples) is built
  on `objc2` 0.6 + `objc2-core-foundation` (CGRect `Encode` impls) — do not
  reintroduce the unmaintained `objc` 0.2 crate. `msg_send!` picks the
  correct `objc_msgSend` variant (incl. stret) from the return type's
  encoding, so the layer code is arch-neutral (aarch64 and x86_64); wgpu-hal
  still pulls in `objc` 0.2 transitively via `metal`, which is expected.

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
