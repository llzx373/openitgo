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
| `plans/2026-06-24-ebook-reader.md` | 已实现 | 电子书阅读完整功能已实现，包括目录面板、历史书签、书架混排与 reload 修复 |
| `plans/2026-06-24-ebook-spread-pagination.md` | 已实现 | 电子书单页/双页 spread 分页改造，消除横向漏边，3D 翻页、预加载、设置/resize 重测 |
| `specs/2026-06-24-ebook-spread-pagination-design.md` | 已实现 | 对应 spread 分页设计文档 |

阅读建议：
- 想了解当前功能与交互，请优先查看根目录 `README.md`、`CHANGELOG.md`、`TODO.md`。
- 想了解某次重构的历史上下文，再查阅对应日期的计划/设计文档。
