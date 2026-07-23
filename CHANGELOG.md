# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Gallery Immersive UI（P0–P2）：全局 `theme` tokens + 自定义 Visuals/Style；书架分段 Tab / 圆角封面卡 / 标签 chip / 空状态；漫画·电子书·媒体工具栏统一 Phosphor 与半透明 chrome。
- Gallery Immersive UI（P3–P4）：设置页分段 Tab（外观/漫画/电子书/媒体/性能/快捷键）；电子书 Dark/Light 色值与壳层对齐（`EbookPalette`）；书架封面/标签 hover 与空历史·空书签插画。
- Settings：`chrome_opacity`（阅读栏透明度），漫画阅读区背景与工具栏 / 进度条 / 状态栏共用；相对窗口透明清屏透出桌面，不盖在正文像素上。
- Settings：`window_pos` / `window_maximized`，与既有 `window_size` 一起记忆窗口几何。

- 电子书阅读模式：支持 EPUB、TXT、MOBI/AZW3、Markdown 格式，基于 `wry` 内嵌 WebView 渲染。
- 电子书阅读布局：单页、双页、连续滚动，可设置字体、字号、行间距、页边距与主题（白天 / 夜晚 / 羊皮纸）。
- 自动按 EPUB 目录或 TXT/Markdown 标题分章；无章节标记时按字数切分虚拟章节。
- 电子书目录面板：工具栏/菜单栏"目录"打开左侧章节列表，点击跳转。
- 电子书 CSS Columns 分页器：实现单页/双页/滚动三种模式的 CSS `columns` 布局、`goToPage`/`nextPage`/`prevPage`/`getPageCount` 接口以及与 Rust 侧兼容的 `position` IPC。
- 电子书 CSS Columns 分页器（Phase 4）：移除旧 `measure` + 行盒测量 + `cloneNode` spread 分页器、3D `flipper` 翻页动画与 `window.ebookUseColumns` 功能开关；CSS columns 成为唯一分页路径，相关设置、模板与测试同步清理。
- 电子书 CSS Columns 分页器（Phase 5 部分）：新增布局结果缓存，避免相同章节与视口设置重复回流；resize 时若宽度与模式未变则跳过布局，仅高度变化时滚动模式直接跳过、分页模式仅重算总页数；新增相邻章节轻量预加载，JS 在章节加载后解析前后章到 inert `<template>`，Rust 侧暴露 `preload_chapter` 并在 `goto_chapter` 时触发。
- 电子书交互：支持滚轮（含水平方向与滚轮反转）、点击左右半边翻页；跨章节边界自动切换章节。
- 电子书设置/窗口自适应：字号、字体、边距变化或窗口 resize 后重新测量并保留当前字符偏移。
- 电子书连续滚动模式：完整章节渲染在 `#column-view` / `#column-content` 中并显示竖直滚动条。
- 电子书历史与书签：复用现有 `History` / `Bookmarks`，保存/恢复章节索引与字符偏移；支持添加/删除/跳转电子书书签。
- 书架混排电子书：`LibraryEntry` 增加 `media_type`，可按"全部 / 漫画 / 电子书"过滤，点击电子书条目进入 `View::Ebook`。
- 电子书打开流程测试：`is_ebook_file` 扩展名识别与 `open_path` 分发测试。
- 环境变量 `OPENITGO_OPEN`，启动时自动打开指定漫画或电子书（开发/测试用）。
- macOS: drag archives or folders onto the Dock icon to open them, even when the app is not running.
- macOS packaging script (`scripts/package-macos.sh`) that builds a signed `.app` bundle, plus a Zed task to run it.
- Menu bar with File / View / Read / Tools / Help menus, available even when the toolbar is hidden.
- Library grid uses a wrapping card layout that adapts to window width and supports vertical scrolling.
- Missing library covers are regenerated on demand; deleted-source entries show an overlay and can be removed in bulk.
- Colorful macOS-style app icon and runtime window icon.
- Phosphor icon font for the reader toolbar.
- Toolbar display mode setting: icon + text, icon only, or text only.
- VS Code 调试/任务配置：新增 `.vscode/launch.json`（Debug / Release / Attach）与 `.vscode/tasks.json`，与 Zed 配置对齐，方便在 VS Code 中运行、调试与打包。
- 媒体播放（内嵌 libmpv）：支持打开并播放视频（mp4/mkv/webm/avi/mov 等）与音频（mp3/flac/aac/m4a/ogg/wav/opus 等），经 `CAOpenGLLayer` 渲染。
- 媒体控制：播放/暂停、±5s/±10s 跳转、可拖进度条、0.5–2 倍速、音量、字幕轨切换/关闭、音轨切换与全屏；工具栏与进度条自动隐藏。
- 媒体书架集成：视频/音频文件与漫画、电子书同架展示、过滤与导入；封面由无头 mpv 截取（视频取 10% 帧，音频取专辑封面）。
- 媒体播放进度恢复：复用历史记录保存播放位置（毫秒），中途退出后自动续播，接近结尾时从头播放。
- macOS 打包脚本新增 `bundle_mpv`：将 libmpv 及其 Homebrew 依赖拷入 `.app` 的 `Contents/Frameworks` 并改写 `@rpath` 后逐个签名，打包产物无需安装 mpv。
- 媒体播放：两行式全宽进度条（悬停显示目标时间，拖动关键帧对齐、松手精确跳转）。
- 媒体播放：画面右上角 OSD 反馈（音量、静音、快进快退、倍速、输出设备切换），CATextLayer 原生叠加约 1 秒淡出。
- 媒体播放：静音（底栏按钮 + M 键，静音时音量滑块灰显）与滚轮音量（视频区滚动，25px 一格 ±5%）。
- 媒体播放：音频输出设备选择（工具栏下拉框，自动 + mpv 枚举设备），保存的设备不存在时回退自动。
- 媒体播放：音量、倍速与输出设备全局记忆（`media_volume` / `media_speed` / `media_audio_device` 设置项）。
- 电子书：全文搜索（工具栏搜索条 + Cmd/Ctrl+F，命中计数 `n/m`，Enter/Shift+Enter 前后跳转，设置变更/resize/换章重排后自动恢复高亮）。
- 电子书：EPUB 内嵌图片与内嵌字体显示——章节 HTML 的相对资源引用改写为 `ebook://reader/res/` 绝对 URL 并由自定义协议从包内取资源（MIME 取 manifest）；书籍 CSS 中的 `@font-face` 单独提取注入，不引入整份书籍样式。
- 电子书：字体设置下拉框（预设 + 自定义值保留），`ebook.font_family` 空值校验与钳制。
- 电子书：TXT/Markdown 自动识别编码——UTF-8（含 BOM）直通，GBK/GB18030/Big5 等经 chardetng + encoding_rs 转码。
- 电子书视图快捷键补全：Escape 返回书架（搜索条可见时优先关闭搜索）、PageUp/PageDown/Space 翻页、Cmd/Ctrl+F 唤起搜索。
- 媒体播放：播放到结尾自动续播同目录下一集（数字感知自然排序），OSD 提示集名；已是最后一集时给出一次性提示。
- 媒体播放：外部字幕加载（字幕菜单"加载外部字幕…"，支持 srt/ass/ssa/vtt）与字幕延迟调节（菜单 ±0.1s/重置，Z/X 快捷键，OSD 反馈当前延迟）。
- 阅读器：首页/末页快捷键（默认 Home/End，可在设置面板自定义，不随 RTL 翻转）。
- 漫画：每本书的阅读设置记忆——模式/双页/缩放按 `comic_id` 存入 `comic_settings.json`，重新打开时优先于全局默认恢复；无记录的书籍行为不变。
- 媒体播放：倍速微调（`[` / `]` 键 ±0.25，原四档与数字键直选保留）、循环播放开关、截图（保存至图片目录）、AB 循环（A 键设 A 点/B 点/取消）、mpv 章节导航（上一章/下一章，无章节禁用）；以上入口聚合于媒体视图的"播放"菜单。
- 阅读器：图片 90° 步进旋转（阅读菜单/工具栏），宽页检测与双页布局按旋转后宽高计算，角度随每本书记忆持久化。
- 漫画：加密 ZIP/RAR 密码支持——打开加密压缩包弹出密码输入框（AES 与传统 ZipCrypto 均可），密码仅会话内记忆不落盘，批量导入可跳过加密文件并汇总提示。
- 书架：条目标签——右键菜单"编辑标签…"，顶部标签过滤 chips（单选），搜索框同时匹配标签。
- 书架：阅读统计 tab——按书累计阅读时长（30s 粒度，存 `reading_stats.json`），显示总时长、条目数与每书时长排行。
- 书签：创建时生成页缩略图并在书签列表行首显示（回退：封面 → 占位色块），删除书签/书籍时联动清理缩略图文件。
- 帮助菜单：快捷键一览面板——可配置键位（当前生效值）与内置阅读/媒体键分区只读展示。
- 非 macOS 平台启动时支持 argv 文件关联打开：首个命令行参数经 `args_os` 读取（非 UTF-8 参数不再 panic），存在性检查过滤无效参数，`OPENITGO_OPEN` 环境变量仍优先（#59 轻量部分；非 macOS 媒体播放与打包脚本出范围，未做）。

### Changed

- Library card click now triggers on the whole card, not just the cover.
- 媒体播放：视频层从 egui 之上的原生 NSView 改为 egui 透明 surface 之下的 CA 子层（透明 backbuffer 合成）；菜单栏菜单与字幕/音轨/输出下拉框现在直接悬浮在视频画面之上，打开菜单时视频不再黑屏让位。
- 项目更名为 OpenItGo：workspace 各 crate 由 `rust-reader-*` 更名为 `openitgo-*`，窗口标题、关于框、`.app` 包名、bundle id（`com.liu.openitgo`）与环境变量前缀（`OPENITGO_*`）同步更新；配置目录改为 `~/.config/openitgo`（开发阶段均为新用户，不提供旧 `rust-reader` 目录迁移）。
- CI：ubuntu job 安装 `libwebkit2gtk-4.1-dev` 修复 wry 构建；新增 macOS job（brew mpv + check/clippy/test）覆盖媒体路径。
- 清理：删除 `probe_overlay.rs` 诊断示例；`docs/bug.md` 归档至 `docs/superpowers/reports/2026-07-17-bug-notes-archived.md`；AGENTS.md 登记 5 个 profiling/smoke 示例；`docs/superpowers/README.md` 索引补全。
- 大章节分段加载评估完成（TODO 32.4 勾选归档）：实测 930KB / 8000 段样本首布局 328ms、resize 重排 211–545ms、内存线性增长，未达分段阈值，结论暂不实现（评估见 #55 与 `docs/superpowers/reports/2026-07-17-large-chapter-loading-eval.md`）。
- openitgo-media：命令参数构造与事件状态迁移抽为 FFI-free 纯函数模块（`args.rs` / `apply.rs`）并补单元测试，非 macOS 平台（ubuntu CI）亦可运行（#58）。
- README 平台措辞与实际对齐：完整支持 macOS；Windows/Linux 可编译运行，文件关联打开已支持，媒体播放暂仅 macOS。
- parser PDF 页数解析从 `pdf` 0.9 迁到 `pdf-syntax` 0.5，与渲染链（pdf-render）共用同一解析栈，消除双解析栈，净删 27 个依赖包（#57）。
- macOS 平台层（dock_open / mpv_view / probe 示例）从 `objc` 0.2 迁移到 `objc2` 0.6，删除 aarch64-only 编译守卫，解除 Intel macOS 编译限制（#57）。
- egui/eframe 0.29 → 0.35.0：图标库由 egui-phosphor 换为 egui_phosphor_icons 0.4；wgpu 22 → 29 连带升级；要求 rustc ≥ 1.92（#57）。

### Fixed

- 漫画首页缩放：双页 LTR 封面仅右页时 `spread_size` 不再因 `left_page?` 失败；`available` 过小或尺寸晚到时保留/重挂 `pending_fit`，翻页后按当前 `fit_mode` 适应。
- 窗口几何：启动恢复上次大小/位置/最大化；屏外时回退默认尺寸并居中；运行中节流写入 settings，退出再 flush。

- 隐藏缺陷批次（TODO #62–#71）：`stable_comic_id` 改用 blake3 并启动迁移；历史/书签离开视图与变更时及时落盘（阅读中 30s 节流）；媒体换片写进度且自动续播强制从头；双页末页不再跳封面；Webtoon 清除双页标志并重置滚轮累加器；`SharedRawCache` 重复插入账本；PDF 文档缓存 256MiB LRU；密码路径 canonicalize 与空密码不重试；加密包首帧尺寸探测带密码；批量导入非密码错误汇总提示。
- 书架卡片显示真实阅读进度（`LibraryEntry.page_count`，打开/导入时写入）。

- 修复非 macOS 平台编译失败：`player_stub` 补齐 `request_audio_devices` / `sub_add` / `adjust_sub_delay` / `reset_sub_delay`（其中 `request_audio_devices` 为既有缺口），ubuntu CI 由此可用。
- 电子书：修复菜单栏菜单/弹层被 wry webview 遮盖不可见的问题——菜单/弹层打开时临时 `set_visible(false)` 隐藏 webview 并以当前阅读主题背景色填充，关闭即恢复（可见性变更按状态去重，不逐帧 IPC）。
- 电子书排版：修复宽表格溢出（td/th 强制折行）与 pre 块溢出（横向滚动）；超高图片经注入 CSS 约束缩放至视口内；剥离 EPUB 内联 `column-*` / `position: fixed|absolute` 样式声明（数量经日志可观测），避免与 CSS columns 分页器冲突。

- 电子书：修复 calibre 风格 EPUB（如《朱颜血》）封面/简介等章节渲染报 "No pages found" 的问题；根因是 NCX 中的 href 带 `#fragment` 或未归一化的 `../`（相对 OPF 目录），zip 精确匹配查找失败，查找前新增归一化（去 fragment、解析 `.`/`..`）。

- 电子书：修复 spread 分页时跨页首行顶部被截断的问题；根因是克隆节点丢失了 `measure` 的顶部 padding，导致非首页内容向上偏移 `margin-v`，把换页处的那一行藏到了可视区上方。
- 电子书：修复 spread 分页时页面底部文字被截断的问题；根因是旧算法用固定 buffer 从内容区顶部向上切分，对于字形较大的字体（如中文）会切到上一行的内容区。新算法按完整行盒（line box）边界切分，确保上一行整行进入当前页，下一行整行进入下一页。
- 电子书：在单页/双页 spread 渲染区域四周增加 4px 安全空白区，并让分页目标高度减少对应尺寸，使轻微超出 line box 的字形或亚像素渲染仍能被看到，进一步避免截断。
- 电子书（旧 spread 分页器已移除的历史修复）：修正 `pageHeight()` 把 `measure` 的 padding 算进页面高度的问题，并将第一页起点对齐到内容区顶部，使页面四周的 `#spread` 边距真正对称显示。
- 电子书：分页算法改为激进策略——只要目标页的最后一行有可能被截断，就连上一行一起放到下一页，进一步避免偶发的底部截断。
- 电子书：将分页切分点取整到整数像素，避免 CSS 高度取整后导致下一页首行在上一页底部露出一个像素条，从而消除相邻页重复行的问题。
- 电子书：为分页逻辑增加 `showError` 错误展示与 `try/catch` 保护，避免分页异常时直接变成白页；同时修复双页模式下左页可能为空的 fallback。
- 电子书：修复双页模式下测量容器宽度与页面列宽不一致导致的排版错位。
- 电子书：修复自定义协议 `ebook://reader?chapter=N` 的解析顺序，章节请求不再被错误地当作阅读器壳页面返回。
- 电子书：修复单页/双页 CSS 模式类名，使 `body.paginated` / `body.double` 选择器正确生效。
- 电子书：修复打开 EPUB 后 WebView 重复 reload 的问题。清理 EPUB 章节 HTML 中的 `<base>` / `<script>` / `<link>` 并禁用 `<a>` 导航；JS 拦截点击与 `beforeunload`；对未知 `ebook://` 资源请求返回空 200 而非 404。
- 电子书：修复单页/双页模式下横向翻页逐渐出现的左侧漏边问题，彻底移除 `column-width` 横向列布局。
- 电子书：分页位移 `transform` 改作用于内层 `#column-content`，不再移动作为 click/wheel 事件监听容器的 `#column-view`，修复每章第一页之后无法点击/滚轮翻页的问题。
- 电子书：布局缓存 key 增加 `--font` 变量，避免字体变更后仍使用旧排版结果。
- 电子书：加载章节时清理超出相邻窗口的预加载 `<template>` 节点，避免 DOM 无限增长。
- 电子书：CSS columns 分页 review 修复——`showError` 使用独立 `#ebook-error-layer` 覆盖层并在渲染成功后隐藏；连续滚动模式在 `applySettings` 中以 `maxScroll()` 作为分母保留滚动比例；目录目标解析区分 fragment/path，对特殊 id 使用 `CSS.escape()`，Rust 侧对 fragment 做 URL 解码；`jump_to_toc` 在注入 JS 前同步 `current_chapter`。
- macOS: 修复应用未运行时通过 Finder / Dock 打开压缩包报 “OpenItGo cannot open files in the “Comic Archive” format” 的错误。通过 swizzle `-[NSApplication setDelegate:]` 在 winit 设置 delegate 前注入 `application:openURLs:` / `application:openFiles:` / `application:openFile:` 实现。
- macOS: 修复应用图标在 Dock/Finder 中显示为带白色方角的问题。`generate_icons.py` 现在会使用 macOS 圆角遮罩生成带透明四角的 PNG 与 `.icns`。
- 电子书：修复工具栏/状态栏不随 WebView 阅读位置上报实时刷新的问题；`EbookRenderer` 在处理 IPC position 消息时调用 `egui::Context::request_repaint()`。
- 媒体播放：修复视频有进度无画面的问题；根因是 CAOpenGLLayer 在 `drawInCGLContext` 前绑定的是自己的 drawable FBO（实测 1/2 交替，并非 0），mpv 一直渲染到 FBO 0，layer 的 drawable 从未被写入而完全透明。现在渲染前查询 `GL_FRAMEBUFFER_BINDING` 并传入，同时 `FLIP_Y` 修正为 1（画面不再上下颠倒）。
- 媒体播放：修复退出媒体播放时间歇性段错误（SIGSEGV）；根因是 `MpvPlayer::drop` 调 `mpv_terminate_destroy` 释放 handle 时，事件线程可能仍阻塞在 `mpv_wait_event` 中。现在 `Drop` 先置 quit 标志并 join 事件线程，再销毁 handle。
- 媒体播放：修复进度条实际只有 100px 宽的问题；根因是 egui 0.29 的 `Slider` 固定按 `spacing().slider_width` 分配宽度，`add_sized` 对其无效。现在在作用域内将 `slider_width` 覆盖为可用宽度，进度条占满整行（悬停时间映射也随之正确），第二行音量滑块不受影响。
- 媒体播放：修复菜单栏菜单与字幕/音轨/输出下拉框被视频画面遮盖的问题；根因是原生视频视图位于整个 egui 图层之上，任何 egui 弹层都会被遮挡。现在 `menu_overlay_open` 检测到菜单/弹层打开时把原生视频视图临时停放为零尺寸，并在全屏下有弹层打开时保持媒体工具栏不自动隐藏。

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
