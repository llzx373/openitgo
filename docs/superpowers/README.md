# docs/superpowers/

本目录存放 OpenItGo（原 rustReader）历次重大功能的设计文档与实施计划。这些文档按时间顺序记录，部分内容可能已被后续重构覆盖。

| 文档 | 状态 | 说明 |
|---|---|---|
| `plans/2026-06-17-comic-reader-implementation-plan.md` | 已归档 | 初始实现计划，大量细节已被后续计划替代 |
| `specs/2026-06-17-comic-reader-design.md` | 已归档 | 2026-06-17 整体设计快照，后续已多次演进 |
| `plans/2026-06-17-async-preload-rar-pdf.md` | 已归档 | 异步加载/预加载/RAR/PDF 计划，架构已演进 |
| `plans/2026-06-17-library-search-sort.md` | 已实现 | 书架搜索/排序，与当前代码基本一致 |
| `specs/2026-06-17-library-search-sort-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-17-multi-threaded-decode-pool.md` | 已实现 | 多线程解码池，实现比计划更复杂（多 IO 线程 + 三队列解码） |
| `specs/2026-06-17-multi-threaded-decode-pool-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-17-archive-index-cache.md` | 已实现 | ZIP/RAR 索引与 raw cache |
| `specs/2026-06-17-archive-index-cache-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-17-gpu-texture-compression.md` | 部分实现 | 实现路径改为 wgpu + CPU 端 DXT5 解压，未使用 glow |
| `specs/2026-06-17-gpu-texture-compression-design.md` | 部分实现 | 对应设计文档 |
| `plans/2026-06-22-thumbnail-first-rendering.md` | 已实现 | 缩略图优先渲染，部分 API 与计划有差异 |
| `specs/2026-06-22-thumbnail-first-rendering-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-22-macos-dock-drop-open-plan.md` | 已实现 | macOS Dock 拖入打开（swizzle NSApplication delegate + 路径队列逐帧排空） |
| `specs/2026-06-22-macos-dock-drop-open-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-23-icons-toolbar-plan.md` | 已实现 | 应用图标与工具栏图标/文字混合显示模式 |
| `specs/2026-06-23-icons-toolbar-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-06-24-ebook-reader.md` | 已实现 | 电子书阅读完整功能已实现，包括目录面板、历史书签、书架混排与 reload 修复 |
| `plans/2026-06-24-ebook-spread-pagination.md` | 已实现 | 电子书单页/双页 spread 分页改造，消除横向漏边，3D 翻页、预加载、设置/resize 重测 |
| `specs/2026-06-24-ebook-spread-pagination-design.md` | 已实现 | 对应 spread 分页设计文档 |
| `plans/2026-06-26-migrate-ebook-to-css-columns.md` | 已实现 | 电子书分页从 JS 行盒测量迁移到 CSS columns |
| `plans/2026-06-26-ebook-page-bottom-mask.md` | 已实现 | 电子书跨页漏行动态遮罩（方案 B） |
| `reports/2026-06-26-css-columns-test-plan.md` | 部分执行（手动矩阵待走查，见 TODO #54） | CSS columns 分页器手动测试计划 |
| `plans/2026-07-14-media-playback.md` | 已实现 | 媒体播放（视频 + 音频，内嵌 libmpv） |
| `specs/2026-07-14-media-playback-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-07-15-media-player-ux.md` | 已实现 | 播放器体验补齐（全宽进度条/OSD/静音/滚轮音量/设备选择/音量倍速记忆） |
| `specs/2026-07-15-media-player-ux-design.md` | 已实现 | 对应设计文档 |
| `plans/2026-07-15-mpv-under-egui-overlay.md` | 已实现 | mpv 视频层下沉到 egui 之下（透明 backbuffer 合成） |
| `plans/2026-07-17-ebook-polish.md` | 已实现 | 电子书收尾（编码检测/EPUB 图片与字体/字体设置/搜索/快捷键，TODO 36–42） |
| `specs/2026-07-17-ebook-polish-design.md` | 已实现 | 对应设计文档 |
| `reports/2026-07-17-bug-notes-archived.md` | 已归档（全部修复） | 原 `docs/bug.md`：快速翻页加载死锁与媒体播放已知问题笔记 |

阅读建议：
- 想了解当前功能与交互，请优先查看根目录 `README.md`、`CHANGELOG.md`、`TODO.md`。
- 想了解某次重构的历史上下文，再查阅对应日期的计划/设计文档。
