# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 电子书阅读模式：支持 EPUB、TXT、MOBI/AZW3、Markdown 格式，基于 `wry` 内嵌 WebView 渲染。
- 电子书阅读布局：单页、双页、连续滚动，可设置字体、字号、行间距、页边距与主题（白天 / 夜晚 / 羊皮纸）。
- 自动按 EPUB 目录或 TXT/Markdown 标题分章；无章节标记时按字数切分虚拟章节。
- 环境变量 `RUST_READER_OPEN`，启动时自动打开指定漫画或电子书（开发/测试用）。
- macOS: drag archives or folders onto the Dock icon to open them, even when the app is not running.
- macOS packaging script (`scripts/package-macos.sh`) that builds a signed `.app` bundle, plus a Zed task to run it.
- Menu bar with File / View / Read / Tools / Help menus, available even when the toolbar is hidden.
- Library grid uses a wrapping card layout that adapts to window width and supports vertical scrolling.
- Missing library covers are regenerated on demand; deleted-source entries show an overlay and can be removed in bulk.
- Colorful macOS-style app icon and runtime window icon.
- Phosphor icon font for the reader toolbar.
- Toolbar display mode setting: icon + text, icon only, or text only.

### Changed

- Library card click now triggers on the whole card, not just the cover.

### Fixed

- 电子书：修复自定义协议 `ebook://reader?chapter=N` 的解析顺序，章节请求不再被错误地当作阅读器壳页面返回。
- 电子书：修复单页/双页 CSS 模式类名，使 `body.paginated` / `body.double` 选择器正确生效。
- macOS: 修复应用未运行时通过 Finder / Dock 打开压缩包报 “rustReader cannot open files in the “Comic Archive” format” 的错误。通过 swizzle `-[NSApplication setDelegate:]` 在 winit 设置 delegate 前注入 `application:openURLs:` / `application:openFiles:` / `application:openFile:` 实现。
- macOS: 修复应用图标在 Dock/Finder 中显示为带白色方角的问题。`generate_icons.py` 现在会使用 macOS 圆角遮罩生成带透明四角的 PNG 与 `.icns`。

## [0.1.0] - 2026-06-23

### Added

- Initial desktop comic reader implementation.
- Comic library with cover thumbnails, search, and sorting.
- Reading modes: LTR (国漫), RTL (日漫), and Webtoon (韩漫).
- Double-page / spread layout with wide-page detection and configurable threshold.
- Page-turn animation with an on/off switch.
- Mouse side-button navigation (forward/back).
- Instant page-number jump via toolbar input.
- Bookmarks with editable notes and history management.
- Recursive import of ZIP/CBZ/RAR/CBR/PDF archives and image folders.
- Settings persistence with atomic writes, backups, and validation.
- History entries store both `comic_id` and `path` for robust matching.
- GPU texture upload releases CPU-side `ColorImage` to reduce RAM use.
- Concurrent raw-bytes cache for archive entries using `RwLock`.
- Protected page indices in `PageCache` now use `HashSet`.
- Cache budget accounting keeps a consistent total after GPU upload.
- Thumbnail previews in the progress bar keep original aspect ratio.
- Empty library state shows a clear call-to-action.
- Settings load failures are reported to the user instead of silently falling back.
