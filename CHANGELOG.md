# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

- macOS: 修复应用未运行时通过 Finder / Dock 打开压缩包报 “rustReader cannot open files in the “Comic Archive” format” 的错误。通过 swizzle `-[NSApplication setDelegate:]` 在 winit 设置 delegate 前注入 `application:openURLs:` / `application:openFiles:` / `application:openFile:` 实现。

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
