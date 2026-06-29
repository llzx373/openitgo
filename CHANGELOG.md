# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 电子书阅读模式：支持 EPUB、TXT、MOBI/AZW3、Markdown 格式，基于 `wry` 内嵌 WebView 渲染。
- 电子书阅读布局：单页、双页、连续滚动，可设置字体、字号、行间距、页边距与主题（白天 / 夜晚 / 羊皮纸）。
- 自动按 EPUB 目录或 TXT/Markdown 标题分章；无章节标记时按字数切分虚拟章节。
- 电子书目录面板：工具栏/菜单栏"目录"打开左侧章节列表，点击跳转。
- 电子书 CSS Columns 分页器（Phase 1）：新增电子书 CSS Columns 分页器函数组（Phase 1）与 `window.ebookUseColumns` 功能开关，默认关闭；实现单页/双页/滚动三种模式的 CSS `columns` 布局、`columnGoToPage`/`columnNext`/`columnPrev`/`columnGetPageCount` 接口以及与 Rust 侧兼容的 `position` IPC；旧行盒分页器保持完整可回退。
- 电子书 spread 分页：单页/双页模式改为 JS 真实排版后切分 spread，每次只渲染当前 spread，配合 ±1 预加载与 3D 翻页动画，消除横向 column 布局的漏边问题。
- 电子书交互：支持滚轮（含水平方向与滚轮反转）、点击左右半边翻页；跨章节边界自动切换章节。
- 电子书设置/窗口自适应：字号、字体、边距变化或窗口 resize 后重新测量并保留当前字符偏移。
- 电子书连续滚动模式：完整章节渲染在 `#spread` 中并显示竖直滚动条。
- 电子书历史与书签：复用现有 `History` / `Bookmarks`，保存/恢复章节索引与字符偏移；支持添加/删除/跳转电子书书签。
- 书架混排电子书：`LibraryEntry` 增加 `media_type`，可按"全部 / 漫画 / 电子书"过滤，点击电子书条目进入 `View::Ebook`。
- 电子书打开流程测试：`is_ebook_file` 扩展名识别与 `open_path` 分发测试。
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

- 电子书：修复 spread 分页时跨页首行顶部被截断的问题；根因是克隆节点丢失了 `measure` 的顶部 padding，导致非首页内容向上偏移 `margin-v`，把换页处的那一行藏到了可视区上方。
- 电子书：修复 spread 分页时页面底部文字被截断的问题；根因是旧算法用固定 buffer 从内容区顶部向上切分，对于字形较大的字体（如中文）会切到上一行的内容区。新算法按完整行盒（line box）边界切分，确保上一行整行进入当前页，下一行整行进入下一页。
- 电子书：在单页/双页 spread 渲染区域四周增加 4px 安全空白区，并让分页目标高度减少对应尺寸，使轻微超出 line box 的字形或亚像素渲染仍能被看到，进一步避免截断。
- 电子书：修正 `pageHeight()` 把 `measure` 的 padding 算进页面高度的问题，并将第一页起点对齐到内容区顶部，使页面四周的 `#spread` 边距真正对称显示。
- 电子书：分页算法改为激进策略——只要目标页的最后一行有可能被截断，就连上一行一起放到下一页，进一步避免偶发的底部截断。
- 电子书：将分页切分点取整到整数像素，避免 CSS 高度取整后导致下一页首行在上一页底部露出一个像素条，从而消除相邻页重复行的问题。
- 电子书：为分页逻辑增加 `showError` 错误展示与 `try/catch` 保护，避免分页异常时直接变成白页；同时修复双页模式下左页可能为空的 fallback。
- 电子书：修复双页模式下测量容器宽度与页面列宽不一致导致的排版错位。
- 电子书：修复自定义协议 `ebook://reader?chapter=N` 的解析顺序，章节请求不再被错误地当作阅读器壳页面返回。
- 电子书：修复单页/双页 CSS 模式类名，使 `body.paginated` / `body.double` 选择器正确生效。
- 电子书：修复打开 EPUB 后 WebView 重复 reload 的问题。清理 EPUB 章节 HTML 中的 `<base>` / `<script>` / `<link>` 并禁用 `<a>` 导航；JS 拦截点击与 `beforeunload`；对未知 `ebook://` 资源请求返回空 200 而非 404。
- 电子书：修复单页/双页模式下横向翻页逐渐出现的左侧漏边问题，彻底移除 `column-width` 横向列布局。
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
