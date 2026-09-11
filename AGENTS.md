# OpenItGo Agent Instructions

## Project Overview

`OpenItGo` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
Supports ZIP/CBZ, RAR/CBR, PDF, and image folders; EPUB/TXT/MOBI/AZW3/Markdown ebooks via
an embedded `wry` webview; audio/video via embedded `libmpv`.

## Repository Layout

- `openitgo-core/` — shared models, reading-state machine, layout math.
- `openitgo-parser/` — archive/folder/PDF parsers, comic ID generation; `archive/` 是通用压缩包
  浏览/解压引擎（ZIP/RAR/7z/TAR 系）。Image-page listing uses `is_comic_image_name`
  (skips `._*` AppleDouble sidecars and `__MACOSX/`).
- `openitgo-storage/` — JSON persistence: settings, library, history, bookmarks,
  `comic_settings.json`, `reading_stats.json`, `password_book.json`.
- `openitgo-media/` — libmpv wrapper: commands, event pump, property observation, OpenGL
  render context, headless covers. `args.rs`/`apply.rs` 为 FFI-free 纯函数模块，CI 可测。
- `openitgo-app/` — egui application, cache, loader, UI views.
- `docs/` — audit reports, bug notes, implementation plans.

## Build & Test

Run the full verification pipeline before committing:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

- libmpv: macOS via Homebrew (`brew install mpv`); Windows from `vendor/mpv/`
  (`scripts/setup-windows.ps1` 下载 shinchiro build 并生成 `mpv.lib`，`openitgo-media/build.rs`
  链接并复制 `libmpv-2.dll`)。Packaged `.app` bundles embed libmpv.
- egui 0.35 requires rustc ≥ 1.92; local dev on 1.97.1, CI on unpinned stable.
- Windows 需 MSVC 环境（VS Build Tools + C++ workload）。`scripts/cargo-win.bat` 用
  `vcvars64.bat` 包裹 cargo。`unrar_sys` 需 `#[cfg(windows)] #[link(name = "advapi32")]`
  （在 `openitgo-parser/src/lib.rs` 与 `probe_rar_password` example 中）。

## Coding Conventions

- Use `cargo fmt`; keep clippy warnings at zero (`-D warnings`).
- Prefer minimal, focused changes; avoid unrelated refactoring.
- Update relevant tests when changing public interfaces.
- Keep UI text in Chinese unless it is a proper noun or technical identifier.

## Key Architectural Notes

### 通用

- **Comic IDs**: deterministic from file/folder path via `openitgo_parser::stable_comic_id`.
  Never use the filename alone.
- **History entries** store both `comic_id` and `path` for robust matching.
- **Settings**: validated on load/save, invalid values clamped and reported via `error_message`.
  解压相关：`extract_dir`、`extract_threads`（0=自动，≤32）、`extract_overwrite`
  （false=同名自动改名）、`extract_wrap`（"smart"/"always"/"never"）、
  `extract_delete_archive`/`extract_open_folder`。
  书架相关：`auto_add_to_library`（默认 true，打开漫画成功即自动入库，同路径去重）；
  漫画 tab 顶栏「清空书架」二次确认后移除漫画/视频/音频条目并清理封面与书签缩略图
  （不删源文件、不动历史）。
- **Per-comic reading settings**（`comic_settings.json`，keyed by comic_id）：打开时经
  `poll_opener` 覆盖全局默认（mode → double_page → fit → rotation，rotation 只收 90°
  步进，脏值回 0）；改动由 `ReaderApp::maybe_save_comic_settings`（`App::update` 末尾）
  diff 快照后落盘（快照 open/close 重置，保存失败也更新快照防每帧报错刷屏）。
- **Library covers**：异步生成存 `covers/`；源文件消失标记 deleted。书签缩略图在
  `covers/bookmarks/<comic_id>-p<page>.jpg`，随书签/书删除。
- **Window title**：`ReaderApp::sync_window_title` 每帧变化时设 `ViewportCommand::Title`——
  Reader: `{page} — {comic} - OpenItGo`；Ebook/Media/Loading: `{file} - OpenItGo`；
  Library/Settings: `OpenItGo`。
- **滚轮翻页（漫画）**：egui 0.35 移除了 `raw_scroll_delta`，`smooth_scroll_delta` 会把一次
  滚轮刻度摊到多帧。`render_reader` 用 `raw_wheel_delta_y(&i.events)` 汇总本帧增量，经
  `accumulate_page_turn` 累加：满阈值翻一页清零，单帧巨幅也只翻一页，余量跨帧保留。阈值 =
  `page_scroll_threshold`（默认 12pt）。Ctrl/Cmd+滚轮缩放共用同一事件和，±2.0 阈值。
  **改滚轮代码不要回退到 `smooth_scroll_delta`**（webtoon 连续滚动除外）。
- **Wide page in double-page mode**：aspect ≥ `wide_page_threshold` → `spread_pages` 返回
  `(Some(current), None)` 居中；空左槽必须用零尺寸（不是 `FALLBACK_PAGE_SIZE`），否则
  spread 获得幻影宽度偏离中心（LTR 单封面同理）。
- **Comic end action**：`settings.comic_end_action` —— 请求下一页但不前进时：
  `DoNothing`（默认）/`WrapToFirst`/`NextSibling`（经 `next_comic_sibling` 自然序；
  最后一个时 `error_message` = `已是最后一个漫画`）。
- **Startup open (non-macOS)**：`initial_open_path` 从 `OPENITGO_OPEN` 环境变量（优先）或
  `argv[1]` 取路径经 `open_path` 打开。`open_path` 分发顺序：ebook → media → 图片
  （`is_image_file`）→ zip/cbz/rar/cbr（`open_archive_auto` 启发式分流）→ 纯压缩包
  （`open_archive_browser`）→ comic。图片分支 `open_image_as_comic` 把父目录作为漫画打开
  并置一次性 `pending_open_options`：`poll_opener` 在每书设置与历史恢复之后消费——定位该图
  （`find_image_page_index`）并强制单页；应用后同步 `last_saved_comic_settings` 快照，防止
  「单页」被误存为长期每书设置。

### 压缩包引擎（parser 侧）

- **密码**：入口 `parse_with_password(path, Option<&str>)`；会话级 `ReaderApp.passwords`
  （`HashMap<PathBuf, String>`，不落盘）经 `PageLoader::passwords()` 共享给 IO worker；
  AsyncOpener 错误串用 `\u{1}` 前缀标记密码类错误。RAR 数据加密包列表可读，解析期靠首条目
  读探针分类（`MissingPassword`/`BadPassword`/CRC `BadData` → 密码错误）。**密码本**
  （`password_book.json`）：内置常见资源站密码表 + 验证成功的用户密码自动收录
  （`record_success` 只在成功路径调用）；`candidates()` 按 use_count/last_used 排序；JSON
  中密码字段 base64 混淆。`poll_opener` 先 `start_password_probe` 后台静默尝试候选（每路径
  每会话一次），全灭才弹密码框。设置页「压缩包」tab 管理密码本。
- `archive_kind()` 按扩展名分发（含双后缀 tar.gz/tar.xz/tar.zst/tar.bz2 等）；
  `list_entries()` 列全量条目；`extract_archive()` 带 `ExtractProgress` channel 与
  `Arc<AtomicBool>` 取消（取消约定：发 `Failed("已取消")` 返回 Ok，半成品删除）。ZIP 条目级
  并行（每 worker 独立句柄 + crossbeam）；RAR/7z/TAR 单线程流式，批量时 app 侧包级并行
  （`ExtractManager`，上限 4）。条目名统一路径穿越防护。密码错误分类：ZIP `InvalidPassword`、
  RAR `classify_rar_error` + CRC `BadData`、7z `classify_sevenz_io_error` 归一为
  `PasswordIncorrect`。`classify_archive()` 按图片占比 ≥80% 分类 Comic/Files。ZIP 条目名经
  `decode_zip_entry_name`（`archive/encoding.rs`）重解码（UTF-8 → SJIS → chardetng → 有序
  兜底），此后按名查条目只能索引扫描（`find_zip_index`）。`read_comment()` 仅 zip 非 None；
  `needs_wrapper_dir()` 为智能解压目录判定。
- **zip 写出**：`archive/zip_write.rs` 的 `create_zip(sources, dest, opts, progress, cancel,
  paused)`（FM「压缩为 zip」用）——条目名 = 相对各 source 父目录（多 source 各自 basename
  为根），目录写 `name/` 条目，文件 256KB 分块写入（块间查暂停/取消），符号链接跳过，
  进度/取消/半成品删除约定与 extract 一致（`ZipWriteProgress`）。

### Archive 视图（app 侧）

- `View::Archive` + `views/archive.rs` 资源管理器式三栏（目录树 / 面包屑+明细列表 / 预览）。
  行模型与树逻辑在 `views/archive_tree.rs` 纯函数（`list_rows`/`build_dir_rows` 等；
  `current_dir: Option<String>` 的 None = 根目录，`flat_all: bool` = 「全部文件」扁平模式；
  行模型经 `rows_cache` 避免每帧重算）。
- **选择模型**：`selected: HashSet<String>` 只存文件条目名，目录行选中态派生自后代统计；
  `click_row(key, ctrl, shift)` 与 `move_focus` 实现 Explorer 式语义；焦点揭示用最小滚动
  （`min_scroll_to_reveal`，勿回退绝对置顶）。「..」上级行恒居行首、不可选。**选中即预览**：
  焦点落到文件行经 `set_preview_target` 更新预览目标；可预览类型（`is_previewable_name`
  门槛，`load_preview` 内容嗅探最终裁决）顺带自动打开预览面板。
- **明细列表行布局**：行距 = `ROW_HEIGHT`（22pt；列表把 `item_spacing.y` 归零），`show_rows`
  虚拟化假定同一 pitch——行内容 scope 里必须把 `interact_size.y` 局部压回 `ROW_HEIGHT - 4.0`
  （`ui.horizontal` 初始行高取全局 28pt 工具栏值，不压会撑爆行），且 scope 结束后要
  `advance_cursor_after_rect(rect)` 钉回行底。
- **列绘制**：`col_shift`（≤0，列块整体平移）+ 列宽字段；`drag_column_sep` = Explorer 语义
  （分隔线右侧列保持宽度随鼠标平移，左侧列吸收变化；撞限同步停住）。大小/压缩后/时间三列用
  `column_layout` 锚点 `painter.text` 右对齐直绘——**不要改回 `with_layout(right_to_left)`**：
  egui 0.35 RTL 嵌套会把文字画到格子右缘之外（被 clip），且不读 col_shift。
- **入口/分流**：`open_path` 对纯压缩包（7z/tar 系）直接进 Archive 视图；zip/cbz/rar/cbr 经
  `open_archive_auto` + `poll_archive_router` 启发式分流（Comic → 漫画链路，Files → Archive
  视图，列目录错误含密码错误回落漫画链路）；显式动作跳过启发式。双击图片条目 →
  `open_entry_as_comic`（同漫画已开则直接跳页）；双击其他条目 → `temp_open::open_entry_external`
  （提取到 temp_dir 后 `cmd /c start`，启动时 `clean_stale` 回收 24h 前目录）。
- **拖出解压（Windows only）**：`DragOutState` 状态机 → `platform::drag_out::do_drag_drop`
  （OLE HDROP，COPY-only，模态）；松开未进 DoDragDrop = 取消并清理暂存目录。
- **多卷 RAR 归一**：`open_path` 最前面经 `normalize_rar_volume_path` 把 partN.rar/.r00/.r01
  改写为首卷，保证历史/密码表 key 稳定。
- **解压对话框**（`extract_dialog.rs`）：统一入口 `ExtractDialogState`，三档 wrap
  （smart/always/never）、完成后删除压缩包（`trash` 移回收站）/打开目标文件夹；选项写回
  settings，`poll_extracts` 在成功时执行删除/打开（失败/取消不执行）。

### 文件管理器（View::FileManager，Total Commander 形态）

结构：`views/file_manager.rs` 视图主体（`FileManagerView`）；`file_manager_panel.rs` 单栏
`FsPanel`；`file_manager_rows.rs` 纯函数行模型（`list_rows`/`natural_cmp`/`SortKey`）；
`file_ops.rs` 文件操作引擎（`OpKind::Compress` 桥接 parser `create_zip`）；
`file_manager_dialog.rs` 确认对话框（对齐 `extract_dialog.rs` 模式）。

**核心约定**：

- **选择模型与 Archive 的差异**：`selected: HashSet<PathBuf>` **含目录**；焦点是 UI 行索引
  usize（0 = 「..」上级行，盘符根禁用上级）。
- **file_ops 约定**：每任务一条后台线程 + channel 进度 + `Arc<AtomicBool>` 取消；删除逐项
  `trash::delete`（回收站）；`ConflictMode::Ask` 执行期逐个问——worker 经
  `OpEvent::AskConflict` 阻塞等 `ConflictAnswer`（100ms 轮询，cancel 或断连按 Cancel）；
  「本次操作全部应用」记忆为后续同级冲突策略（文件级/目录级各自独立）；目录↔目录非 Ask
  恒合并；AutoRename `name (1).ext`；移动 = `fs::rename` 快速路径，失败回退递归复制 +
  trash 源；取消清理半成品；不跟进符号链接目录（防环）；长路径经 `verbatim_path` 加
  `\\?\` 前缀（>240 字符才加）。
- **队列（阶段 AA）**：并发上限 `max_concurrent`（settings `fm_op_threads`，默认 2，0=不限，
  ≤8）——超上限进 `queued` FIFO（不起线程、无进度），排队取消 = 静默出队。
  `task_summaries()` 任务面板快照；`FinishedOp` 带 dest/conflict/delete_permanent 供
  「重试失败项」（`retry_sources` 从 errors 过滤仍在的源）。
- **持久化模式**：`FileManagerView::snapshot()` 采集，`maybe_save_fm_state`（`App::update`
  末尾）diff 快照后写回 settings——**不自行落盘**，退出时 `on_exit` 统一 `save_settings`；
  快照在离开 FM 视图时重置。设置页改动经 `apply_layout_settings`/`set_button_bar`/
  `set_tab_groups` 等同步休眠视图（否则快照写回覆盖设置页改动）。
- **栏间 Id 隔离**：每栏包 `ui.push_id(("fm_panel", idx), …)`——不加盐两栏同位置控件共享
  持久状态（滚动串扰、列宽联动）。
- **行内导航停笔**：`render_list` 行循环每行先查 `state == Ready`——行内双击/右键「打开」
  触发 navigate/refresh 当场清空 entries，继续按旧 rows 快照渲染会越界 panic；entries 索引
  一律 `get` 防御。`FsPanel::poll()` 非 Loading 状态必须原样返回（`mem::take` 会打回
  Ready → 每 3 帧重列闪烁）。

**fm_* settings 一览**：布局/显示——`fm_layout`/`fm_dual_ratio`/`fm_preview_open`/
`fm_view_mode`("list"|"brief"|"thumbs")/`fm_columns`（自定义列，阶段 V 的权威；
`fm_col_size_width`/`fm_col_mtime_width`/`fm_col_shift` 为兼容回写）/`fm_system_icons`（S）。
排序/目录——`fm_sort_key`/`fm_sort_asc`/`fm_dirs_first`/`fm_dir_left`/`fm_dir_right`。
标签/书签——`fm_tabs_left`/`fm_tabs_right`/`fm_active_tab_left`/`fm_active_tab_right`/
`fm_tab_groups`（AI）/`fm_bookmark_groups`（`FmBookmarkGroup{name, items}` 两栏共享；旧扁平
`fm_bookmarks` 仅读取兼容，clamp 时并入「常用」组。书签项目录/文件均可：点击目录
经 `fallback_existing_dir` 导航，点击文件经 `pending_bookmark_open` → `open_path`
直接打开——`bookmark_jump_target` 纯函数分流；目录切换在 `start_listing` 清空过滤器，
防残留过滤串导致列表空白，refresh/标签恢复快照不受影响）。
行为包（O）——`fm_confirm_delete`/`fm_show_hidden`/`fm_delete_mode`/`fm_space_action`/
`fm_drag_confirm`/`fm_archive_open`/`fm_esc_keep_selection`/`fm_dblclick_blank_up`。
其他——`fm_saved_filters`/`fm_filter_bar_bottom`（T）/`fm_rubber_band`（U）/
`fm_command_bar`（X）/`fm_button_bar`（Y）/`fm_watch_recursive`（Z）/`fm_op_threads`（AA）。
行为类设置统一 `FmBehaviorOptions` 由 `fm_behavior_options(&settings)` 每帧经
`view.ui(ui, callbacks, options)` 下发（不进 FmStateSnapshot）；面板级 show_hidden/
dirs_first 逐栏下发，RowsKey 自动失效。

**功能与实现要点**（按阶段标记；细节看代码与 git 历史）：

- 盘符切换：面包屑最左 `list_drives()`（Windows 枚举 A–Z；Unix 为 `/` + `/Volumes/*`），
  首次点开菜单才起一次性后台枚举、结果缓存。
- 目录大小：右键「计算大小」/空格 → `FsPanel::request_dir_sizes`（单 worker 重启式，结果在
  `poll()` 的 Loading 检查之前排空进 `dir_sizes`；navigate/refresh 取消清空）。核心
  `file_ops::dir_size`（不跟进符号链接、cancel 提前返回已累加值）；命中时弱一档颜色显示。
- 名称语义着色（`entry_name_rich_text`，固定规则）：目录 = accent 色、符号链接 = 斜体+弱档、
  隐藏 = 弱档 0.6；选中行统一回退 `text_color()`；颜色全经 visuals 派生不硬编码。
- 栏间拖放：行 `click_and_drag` + `dnd_set_drag_payload(FmDragPayload)`；落点判定**不用**
  `dnd_drop_zone`，由 `poll_inter_panel_dnd` 手写；松开走确认复制链路（`fm_drag_confirm`=
  false 时直拷 AutoRename，Shift=移动）。面包屑段/标签也是落点（`*_drop_rects` 帧首 clear，
  在 Dual 早退之前判定，单栏也接收）。拖出 MOVE 语义（Z）：`do_drag_drop(files, allow_move)`，
  进 OLE 模态**前**快照 Shift，MOVE 成功 → 源走 `start_delete`（Archive 视图维持 COPY-only）。
- FS watch：Ready 后 `ensure_watch` 建 `notify` watcher（`fm_watch_recursive` 可开递归，
  选项变化自动重建）；回调发信号 + `request_repaint()`；`poll()` 在 Loading 检查前
  `poll_watch`，去抖 300ms 后 `refresh()`（保留选中）。
- 文件搜索（Alt+F7，`file_manager_search.rs`）：`SearchQuery`（`wildcard_match`、空=`*`；
  大小/天数过滤；content 仅 ≤1MB 文本扩展名）；批量回发，上限 10 万截断。非模态窗口；
  Enter = `reveal_path`；「输送到焦点栏」= `inject_entries_branch`。纯 F7 新建文件夹需排 Alt。
- 阶段 A–I（TC 对齐）：`*` 反选 / `+` 选择组 / `-` 取消选择（`wildcard_match` `;` 多模式）；
  可打印字符进 type-ahead（空格保留给目录大小）；`SortKey::Ext`；状态栏「已选 N 项 · 合计 X」；
  Alt+↓ 目录历史；Ctrl+U 交换两栏 / Ctrl+→← 目录互带 / Ctrl+\ 回根（仅双栏）；
  `OpSpeedMeter` EMA 速度/ETA + `set_paused` 暂停；Ctrl+Q 快览（非活动栏整栏替换为预览）；
  Ctrl+B 分支视图（`toggle_branch_view`）；Ctrl+M 批量重命名（`plan_renames` 纯函数）；
  栏内标签页 Ctrl+T/Ctrl+W/Ctrl(+Shift)+Tab；缩略图 `ThumbCache`（视图级共享，网格只是
  渲染层，行索引/选择/过滤与列表完全共用；可见范围变化 bump 代次，worker 丢弃过期请求）；
  系统剪贴板互通（CF_HDROP + 剪切标志；macOS 实现为 raw msg_send!，**本机只编 Windows，
  依赖 CI/真机验证**）；外部拖入经 `panel_rect_at` 命中落点栏。
- 新建文本文件（Shift+F4）：`suggest_text_file_name`（重名 `(2)` 递增）+ `create_text_file`
  （`create_new(true)` 防竞态覆盖）。
- Shell 动词（Windows only，`platform::shell_verbs`）：Alt+Enter 属性、右键「打开方式…」、
  F4 `edit_file`（edit 失败回退 openas）、Ctrl+Shift+Enter `run_as_admin`（UAC）。
- 面包屑路径编辑：铅笔按钮 → TextEdit（Enter/Esc 在编辑 UI 内自测——egui_wants_keyboard_input
  会屏蔽面板全局键）；Enter 校验 `is_dir()` 后 `navigate_to(fallback_existing_dir)` 兜底。
- 键位（P）：**egui `key_pressed` 不认修饰键**——纯 Enter 打开分支已排 command/alt，纯 F5
  排 Alt，否则同帧双触发。双击空白回上级（`fm_dblclick_blank_up`）：`dblclick_hits_blank`
  纯函数，接线在 render_list/render_grid 帧尾 `blank_dblclick_up`。
- 状态栏（Q）：驱动器剩余空间 `platform::drive_info`（Windows `GetDiskFreeSpaceExW` 调用者
  可用口径；unix statvfs，以 `[target.'cfg(unix)'] libc` 复用）；30s 会话缓存（失败 None
  同样缓存）。过滤激活指示：弱色 `过滤: xxx`。
- 预览器（R）：FM 三处共用 `draw_preview_content(ui, in_popup)`。加载器返回 `PreviewOutcome
  { data, bytes }`（原始字节是 HEX/重解码数据源，超 64MB 为 None）；所有类型都读取，二进制落
  HEX。HEX = `format_hex_line` + show_rows 虚拟化（不物化全表）。编码手动切换：
  `decode_text_with`/`decode_preview_text`/`preview_redecode`（用已读字节重解码不重读文件）。
  文本内搜索：**无搜索词维持 TextEdit 只读多行，有搜索词才切行级虚拟化**。图片旋转 = Mesh
  UV 角点轮换（不写文件）。F3 弹窗最大化走全屏 Area 自绘标题行。
- 系统真实图标（S，Windows only）：`platform/windows/file_icons.rs` `extract_icon`
  （`SHGetFileInfoW`，目录/普通扩展名用伪属性免读盘；HICON→RGBA 手工转换，资源 guard 清理）。
  缓存 `SysIconCache`（key = `SysIconKind` + 尺寸档，容量 256 插入序逐出）；`fm_system_icons`
  默认 true；Miss/Failed 回退字体图标；图片缩略图优先。
- 过滤（T）：`filter_matches`——含 `*`/`?` 走 `wildcard_match`（`;` 多模式），否则子串。
  `render_filter_bar` 两处调用点（顶栏右侧 / `fm_filter_bar_bottom` 时焦点栏底部）。
  `saved_filters` 入快照写回；`filter_history` 会话内。**Ctrl+S** 聚焦过滤框（必须在
  `egui_wants_keyboard_input` 检查之前）；聚焦时 Esc = 清空 + `filter_esc_handled` 防同帧
  双消费。
- 选择（U）：**鼠标框选**（`fm_rubber_band` = "right"默认/"left"/"off"）——位移超 6pt 才
  active；拖动中只画半透明矩形；松开应用：**无修饰 = 替换、Shift = 追加、Ctrl = 切换，
  anchor/focus 不动**；命中纯函数 `rows_in_rect`/`cells_in_rect`。**保存/恢复选择集**
  （会话内）：右键「选择」子菜单；恢复 = 替换式按路径匹配当前栏可见项。
- 显示模式与自定义列（V）：`PanelViewMode` 加 `Brief`（简表，行主序多列，列宽
  `brief_col_width` clamp 120..=300；与网格共用线性行号键盘步进/空白双击/框选）。
  `ColumnKind{Name, Ext, Size, Mtime, Attr, Comment}`；**columns[0] 恒 Name 且弹性宽度**；
  固定列 ≥1（列头右键勾选增删）；`column_layout(right, shift, columns)` 从右往左排固定列；
  `drag_column_sep` 泛化。`SortKey::Unsorted`（read_dir 物理序）与 `SortKey::Attr`。
  持久化 `fm_columns`（"kind:width"；`parse_columns`/`serialize_columns` 并回写 legacy
  列宽字段）。`PanelTabSnapshot` 加排序/视图/列（仅会话内——标签持久化只存目录+锁定/标题）。
- 文件注释（W）：`views/fm_comments.rs` 读写 descript.ion（每行 `文件名 注释`）；
  `read_comments` 经 `decode_text_guess` 探测编码；`write_comment`（空 = 删除该条，UTF-8
  无 BOM）。列举就绪时读入；按**文件名**匹配（分支视图子目录项不递归读）。编辑 = 右键/
  Ctrl+Shift+Z（**Ctrl+Z 是撤销，勿占**）。
- 命令行输入条（X）：`fm_command_bar` 默认开——FM 底部，`{dir}>` 前缀 + 单行 TextEdit；
  Enter 经 `spawn_shell_command` 后台执行（current_dir = 焦点栏目录）；历史 ↑/↓（cap 32，
  会话内）。**egui 0.35 焦点锁滤波三坑**：① 单行框必须 `.return_key(None)` 自处理 Enter；
  ② 已聚焦时**不得**重复 `request_focus`（重置方向键滤波，下帧 ↑/↓ 被当漫游键移焦）；
  ③ Esc 在帧首收走焦点，分支必须认 `lost_focus() || has_focus()`（过滤框同此）。执行分支
  排修饰键（Ctrl+Enter 粘贴文件名与执行同帧）。快捷键（wants_keyboard_input 检查之前）：
  Ctrl+P 追加路径、Ctrl+Enter 追加文件名。
- 自定义按钮栏（Y）：`fm_button_bar: Vec<FmButton>`——**空列表不渲染不占垂直空间**；点击经
  `spawn_shell_command`；占位符 `%P`/`%N`/`%p`（`expand_button_command` 纯函数）。右键
  编辑/删除；写回同 fm_bookmark_groups 模式（视图副本 + 快照 diff + `set_button_bar` 同步）。
- 任务面板 + 错误汇总（AA）：顶栏「任务」/点状态栏进度区开 `render_task_panel`（排队任务
  显示「排队中」）；状态栏只显示第一个在途 + `（+N 在途 +M 排队）`。errors 非空留存
  `OpErrorReport` → `render_op_error_report` 虚拟化列失败项 + 「重试失败项」（不带
  verify/filter 高级选项）。
- 复制校验/过滤/压缩暂停（AB）：`CopyOptions{verify, filter_pattern, filter_newer_days}`；
  `copy_filter_matches` 预扫描与执行共用（目录恒保留）；verify 经 `files_identical` 第二轮
  进度。**过滤 + Move**：禁 rename 快速路径；`ctx.move_prune` 逐文件 trash 已拷源；
  `ctx.drop_source_unsafe` 阻止整删源（顺带修了预存 bug）。**压缩暂停**：`create_zip` 增
  `paused: Option<Arc<AtomicBool>>`。
- 校验和 + 属性/时间戳（AC）：`ChecksumDialog`——单次过同算 CRC32/SHA-1/SHA-256（**MD5
  无依赖不引入**，条目标「不支持」）；验证 `parse_checksum_file`（md5sum 族/sfv）。
  `AttrTimestampDialog`——**时间戳先于属性位写入**（只读文件 SetFileTime 被拒）；
  `platform::file_attr`（Windows 仅改 READONLY/HIDDEN/SYSTEM/ARCHIVE；unix 仅 readonly）；
  时间戳走 `filetime` crate；即时生效不落盘。
- 分割/合并（AD）：`OpKind::Split`/`Merge`；`split_plan` 纯函数（`name.001 … NNN`）；已存在
  分块整批不覆盖；取消/失败删半成品。合并 `collect_chunks` 要求 .001 起连续；同目录 `.crc`
  存在时顺带 CRC32 校验。Split/Merge 不支持「重试失败项」。
- 同步目录（AE）：`file_manager_sync.rs` `SyncDialog`（双栏且目录不同才可用）；
  `diff_dir_trees` 纯函数（大小相同 mtime 差 ≤2s 视为 Same——FAT 精度）；`build_sync_plan`
  产 Copy(Overwrite)/Delete(回收站) 任务组；`keep_top_level` 去嵌套。
- 比较内容（AF）：`file_manager_compare.rs` `CompareDialog`（选中恰 2 个非目录，或双栏
  「比较两栏焦点文件」）；`binary_compare` 快比 + `text_diff`（自实现 LCS u16 DP，行数乘积
  >4001×4001 回退）；单 ScrollArea 左右对照（天然联动滚动）。
- 撤销（AG）：`file_manager_undo.rs` `UndoStack` 会话内上限 32。可撤销：Copy（撤销 = 目标集
  Delete 进回收站）、Move、Rename/MultiRename（批量逆序回改防互换命名相撞）、NewFile/NewDir；
  Delete/覆盖写/Compress/Split/Merge/属性不可撤销——不入栈也**不清空栈**。记账点：Copy/Move
  在 `on_op_finished` 按 `FinishedOp.written`（实际写入的顶层对，AutoRename 后为准）。
  执行 `plan_undo` 纯函数 + `precheck`（Trash 目标缺失跳过；RenameBack **改回位置被占用**
  跳过，防覆盖第三方文件）。入口 Ctrl+Z + 右键动态项「撤销 {label}」。
- 树形面板（AH）：`file_manager_tree.rs` `DirTree`——双栏下 Alt+F10 把**非活动栏**整栏替换
  为目录树（与 Ctrl+Q 互斥；关闭后原栏原样恢复）。懒加载 + `children` 缓存 + `expanded`；
  行模型纯函数 `visible_rows`/`move_cursor`/`parent_in_rows`。双击/Enter = **活动栏**
  navigate_to（树驱动文件栏）；`tree_focused` 决定键盘归属。
- 标签增强（AI）：锁定（`PanelTabSnapshot.locked`，锁定时 navigate 自动 `new_tab_to`）、
  重命名（`custom_title` 优先于 basename）、标签组（`fm_tab_groups`，应用经
  `fallback_existing_dir` 回退）。持久化：`FmTabEntry` untagged（旧纯目录字符串兼容读入，
  写出恒 Full）。
- **全局「← 返回」**：`ReaderApp.previous_view: Option<View>` **单层**回退落点（非栈）——
  仅 FM 打开动作真正离开 FM 时记录；进 Library/Settings 即清空；消费后即 None。
  FileManagerView 常驻内存，返回时原样恢复。

### PageLoader / 缓存 / 电子书 / 媒体

- **PageLoader**：后台 IO + decode worker threads + channels；独立 `cover_loader` 负责库封面。
  **进度条悬停缩略图**：全尺寸解码且 compress=false 时顺产生成 256px 缩略图；悬停时
  `request_page_thumbnail` 高优先级 + 按方向低优先预取 8 页；批量经
  `THUMBNAIL_BATCH_MAX_INFLIGHT=2` 节流，翻页冷却期暂停；webp 走 `webp_thumb.rs`，失败回退
  image crate。
- **PageCache**：GPU textures；`size_bytes` 为 CPU/GPU 内存估算；上传后尽快释放 CPU 侧
  `ColorImage`。
- **EbookRenderer**：wry child webview + 自定义 `ebook://` 协议；章节经
  `openitgo_parser::html::render_chapter_html` 渲染；分页用 CSS `columns`。Pagination
  transforms 只能加在 `#column-content` 上（`#column-view` 是事件容器不可平移）。**WebView2
  (Windows) 约束**：页面内绝对 `ebook://...` 请求不会被拦截、直接失败——JS 与章节 HTML 一律
  用相对 URL（`?chapter=N`、`/res/...`）。**目录树**：`EbookChapter.level` 为嵌套深度，经
  `views/ebook_toc.rs::toc_rows` 纯函数展开为可折叠树。**markdown 渲染**：fenced code 经
  syntect 高亮；相对图片改写 `/file/` 根相对 URL（canonicalize 必须在书目录子树内）。**菜单
  停放（#52）**：菜单打开时 `menu_overlay_open(ctx)` 驱动 `set_webview_hidden`。**位置保持**：
  字号/边距/主题按字符偏移，resize 防抖后按滚动比例（scroll）或 spread（分页）。
- **Media (macOS)**：mpv 渲染进 CAOpenGLLayer 插到 wgpu CAMetalLayer 之下；app 以透明
  backbuffer 运行；裸 layer 几何变化必须走禁用 actions 的 `CATransaction`；
  `drawInCGLContext` 里必须查询 `GL_FRAMEBUFFER_BINDING` 传给 `RenderContext::render`，
  `FLIP_Y=1`，否则合成全透明。播放进度存 `HistoryEntry.char_offset`（毫秒）。
- **Media (Windows)**：mpv 经 `wid` 渲染进 `WS_CHILD` HWND 子窗口；`wid` 必须在
  `mpv_initialize` 前设置，故两段式 open：`PendingVideoView::create(parent)` →
  `MpvPlayer::new_with_wid` → `finish(bounds, &player)`。HWND 恒在 wgpu 表面之上，
  `render_media` 在 `menu_overlay_open(ctx)` 时零尺寸 bounds 停放视频窗口（同 #52）。
- **MpvPlayer command rule**：UI 线程的 mpv 命令/属性调用必须用异步 API——阻塞调用会把 UI
  线程停在 mpv dispatch 队列上，与首帧 DR 分配形成循环等待，冻结窗口。
- **MpvPlayer observe/userdata 分配**：观察 id 1-9（9 = `chapter`），userdata 100
  （`audio-device-list`）/ 101（`chapter-list`），常量在 `apply.rs`；下一可用：观察 id 10、
  userdata 102。
- **MpvPlayer teardown**：`Drop` 先设 quit flag 并 join `mpv-events` 线程，再
  `mpv_terminate_destroy`——颠倒顺序会因 `mpv_wait_event` 与 handle free 竞争而 segfault。
- **Media OSD**：macOS 走 CATextLayer；原生视图停放时由 egui painter 画同文。`show_osd` 存
  文本 + 1s 过期；`tick_osd` 未过期时重挂 `request_repaint_after`（egui 定时帧只触发一次，
  不重挂则空闲时 OSD 永远不清）；`close()` 清 OSD 防泄漏。
- **Media preferences**：volume/speed/audio-device 全局持久化；音频设备延迟到异步
  `audio-device-list` 回复后校验，失效回退 "auto" 并经 `take_startup_device_invalid` 上报。
- **Media auto-next**：`maybe_auto_next_media`（`media_end_action` = `NextInDir`）；`ended`
  且无 `error` 时打开同目录下一个媒体（`next_media_in_dir` 自然序），每个媒体只触发一次；
  OSD 存 `pending_open_osd` 由下一次 `open` 显示；播放错误不触发；最后一集弹 `已是最后一集`。
- **Media menus**：媒体 seek bar 需 scoped `ui.spacing_mut().slider_width` 覆盖。
- **文件关联（Windows only）**：`platform/windows/file_assoc.rs` 注册到
  `HKCU\Software\Classes`（winreg）；覆盖前备份，取消时仅在本程序 ProgID 才删除并恢复备份；
  完成后 `SHChangeNotify`。UserChoice 优先——UI 提供 `open_default_apps_settings()` 引导。
- **图标字体（egui_phosphor_icons）**：`fonts::setup_fonts` 追加 phosphor + 系统 CJK（同时挂
  Proportional 与 Monospace，缺挂会中文豆腐块；非 UTF-8 文本经 `decode_text_guess` 识别）。
  **字体未注册时 epaint 0.35 直接 panic**——任何构造 `ReaderApp` 的 example 必须先调
  `openitgo_app::fonts::setup_fonts`；带 UI 的 example 还需每帧 `request_repaint_after`。
- **Packaging**：`scripts/package-macos.sh` 签名前跑 `bundle_mpv`，把 libmpv 及 Homebrew 依赖
  复制进 `Contents/Frameworks` 并改写 install names 为 `@rpath`。
- **Dock open (macOS)**：`platform::macos::dock_open` swizzle NSApplication delegate 入
  `OPEN_QUEUE`，每帧排空；回调必须 `set_wake_context` + `request_repaint()` 唤醒——删掉会
  复现 "dock 打开的文件卡住" bug。macOS 平台层基于 **objc2 0.6**——**不要重新引入 `objc`
  0.2**。

## Diagnostic Examples

带 UI 的用法均为 `cargo run -p openitgo-app --example <name> -- <路径>`：

- 漫画：`flip_through`（全页遍历冒烟）、`rapid_flip`（80ms 连续翻页压力）、`profile_open`
  （打开 10 秒后快照退出）、`profile_view`（持续运行每 10 秒快照）、`ui_smoke`（当前页入缓存
  即退出，30 秒超时）。
- 媒体：`media_smoke`（播放推进即退出）、`probe_visible`/`probe_mpv_view`/`probe_video_overlay`；
  `openitgo-media/examples/{probe,probe_render,probe_cover}.rs`（无头，`probe_cover` 跨平台）。
- 电子书：`probe_ebook_menu`（菜单停放 #52 验证）。文件管理器：`fm_smoke`。
- `OPENITGO_MPV_LOG=1` 开启 mpv debug 日志（stderr）。

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
