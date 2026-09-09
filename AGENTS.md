# OpenItGo Agent Instructions

## Project Overview

`OpenItGo` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
Supports ZIP/CBZ, RAR/CBR, PDF, and image folders; EPUB/TXT/MOBI/AZW3/Markdown ebooks via
an embedded `wry` webview; audio/video via embedded `libmpv`.

## Repository Layout

- `openitgo-core/` — shared models, reading-state machine, layout math.
- `openitgo-parser/` — archive/folder/PDF parsers and comic ID generation; `archive/`
  是通用压缩包浏览/解压引擎（ZIP/RAR/7z/TAR 系）。Image-page listing uses
  `is_comic_image_name` (skips macOS `._*` AppleDouble sidecars and `__MACOSX/` trees).
- `openitgo-storage/` — JSON persistence: settings, library, history, bookmarks,
  per-comic reading settings (`comic_settings.json`), stats (`reading_stats.json`),
  password book (`password_book.json`).
- `openitgo-media/` — libmpv wrapper: commands, event pump, property observation, OpenGL
  render context, headless cover generation. `args.rs`/`apply.rs` 为 FFI-free 纯函数模块，
  ubuntu CI 可测。
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
  (`scripts/setup-windows.ps1` 下载 shinchiro build 并用 `lib.exe` 生成 `mpv.lib`，
  `openitgo-media/build.rs` 链接并复制 `libmpv-2.dll` 到产物旁）。Packaged `.app`
  bundles embed libmpv.
- egui 0.35 requires rustc ≥ 1.92; local dev on 1.97.1, CI on unpinned stable.
- Windows 需 MSVC 环境（VS Build Tools + C++ workload）。`scripts/cargo-win.bat` 用
  `vcvars64.bat` 包裹 cargo 并设 crates.io 代理。`unrar_sys` 的 UnRAR C++ 未声明
  advapi32 链接，需 `#[cfg(windows)] #[link(name = "advapi32")]`（在
  `openitgo-parser/src/lib.rs` 与 `probe_rar_password` example 中）。

## Coding Conventions

- Use `cargo fmt`; keep clippy warnings at zero (`-D warnings`).
- Prefer minimal, focused changes; avoid unrelated refactoring.
- Update relevant tests when changing public interfaces.
- Keep UI text in Chinese unless it is a proper noun or technical identifier.

## Key Architectural Notes

- **Comic IDs**: deterministic from file/folder path via `openitgo_parser::stable_comic_id`.
  Never use the filename alone.
- **加密压缩包密码**：入口 `parse_with_password(path, Option<&str>)`（`parse` 转调
  None）；会话级 `ReaderApp.passwords`（`HashMap<PathBuf, String>`，不落盘）经
  `PageLoader::passwords()` 共享给 IO worker 做解密读取；AsyncOpener 错误串用
  `\u{1}` 前缀标记密码类错误供 `poll_opener` 识别。RAR 数据加密包（`rar -p`）列表
  可读，解析期靠首条目读探针分类（`MissingPassword`/`BadPassword`/CRC `BadData` →
  密码错误）。**密码本**（`password_book.json`，有意变更"密码不落盘"旧 spec）：
  `PasswordBook` = 内置常见资源站密码表 + 验证成功的用户密码自动收录
  （`record_success`，只在成功路径调用，密码框确认时不 record）；`candidates()`
  按 use_count/last_used 排序；JSON 中密码字段 base64 混淆（仅防明文浏览）。
  `poll_opener` 先经 `start_password_probe` 后台静默尝试候选（每路径每会话一次），
  全灭才弹密码框。设置页「压缩包」tab 管理密码本。
- **压缩包引擎（parser 侧）**：`archive_kind()` 按扩展名分发（含双后缀
  tar.gz/tar.xz/tar.zst/tar.bz2 等），`list_entries()` 列全量条目，
  `extract_archive()` 带 `ExtractProgress` channel 事件与 `Arc<AtomicBool>` 取消
  （取消约定：发 `Failed("已取消")` 返回 Ok，半成品删除）。ZIP 条目级并行（每
  worker 独立句柄 + crossbeam）；RAR/7z/TAR 单线程流式，批量时 app 侧包级并行
  （`ExtractManager`，上限 4）。条目名统一路径穿越防护。密码错误分类：ZIP
  `InvalidPassword`、RAR `classify_rar_error` + CRC `BadData`、7z 错密码表现为
  CRC 失败经 `classify_sevenz_io_error` 归一为 `PasswordIncorrect`。
  `classify_archive()` 按图片占比 ≥80% 分类 Comic/Files。ZIP 条目名经
  `decode_zip_entry_name`（`archive/encoding.rs`）重解码（UTF-8 → SJIS → chardetng
  → 有序兜底），此后按名查条目只能索引扫描（`find_zip_index`）。`read_comment()`
  仅 zip 非 None；`needs_wrapper_dir()` 为智能解压目录判定；
  `ExtractProgress::Started.total_bytes` 为 `Option<u64>`（仅 ZIP Some）。
  **zip 写出**：`archive/zip_write.rs` 的 `create_zip(sources, dest, opts, progress,
  cancel)`（FM「压缩为 zip」用）——条目名 = 相对各 source 父目录（多 source 各自
  basename 为根，'/' 分隔），目录写 `name/` 条目，文件流式 `io::copy`，符号链接
  跳过，进度/取消/半成品删除约定与 extract 一致（`ZipWriteProgress`）。
- **Archive 视图（app 侧）**：`View::Archive` + `views/archive.rs` 资源管理器式
  三栏（目录树 / 面包屑+明细列表 / 预览），操作手感对齐 Explorer/WinRAR；行模型与
  树逻辑在 `views/archive_tree.rs` 纯函数（`list_rows`/`build_dir_rows`/
  `direct_children`/`build_dir_stats` 等；`current_dir: Option<String>` 的 None =
  根目录，`flat_all: bool` 单独表示「全部文件」扁平模式；行模型经 `rows_cache`
  避免每帧重算）。**选择模型**：`selected: HashSet<String>` 只存文件条目名，目录行
  选中态派生自后代统计；`click_row(key, ctrl, shift)` 与 `move_focus` 实现
  Explorer 式单选/Ctrl/Shift/键盘语义；焦点揭示用最小滚动（`min_scroll_to_reveal`，
  勿回退绝对置顶）。「..」上级行（`ListRow::Parent`）恒居行首、不可选。
  **选中即预览**：单击/键盘焦点落到文件行经 `set_preview_target` 更新预览目标；
  可预览类型（图片或常见文本扩展名，`is_previewable_name` 门槛，最终能否预览由
  `load_preview` 内容嗅探决定）顺带自动打开预览面板，不可预览类型不动面板开关。
  **明细列表行布局**：行距 = `ROW_HEIGHT`（22pt；列表把 `item_spacing.y`
  归零，行间不留缝），`show_rows` 虚拟化假定同一 pitch——行内容 scope 里必须把
  `interact_size.y` 局部压回 `ROW_HEIGHT - 4.0`（`ui.horizontal` 初始行高取
  全局 `interact_size.y` = 28pt 工具栏值，不压会撑爆行、文字下沉贴条纹下缘、
  行距漂移），且 scope 结束后要 `advance_cursor_after_rect(rect)` 钉回行底
  （scope_dyn 会用内容区底改写竖向光标）。**列拖动**：`col_shift`（≤0，列块
  整体平移，0 = 贴右缘）+ 三列宽字段；`drag_column_sep` = Explorer 语义
  （分隔线右侧列保持宽度随鼠标平移，左侧列吸收宽度变化；名称列弹性由
  col_shift 吸收；撞限同步停住）。**列内容绘制**：行内大小/压缩后/时间三列用
  `column_layout` 锚点（含 col_shift）`painter.text` 右对齐直绘，与表头/竖线同一
  坐标源——**不要改回 `with_layout(right_to_left)`**：egui 0.35 外层 horizontal +
  RTL 嵌套（格子内再 RTL）会把文字画到格子右缘之外（右移一格宽，被滚动区 clip），
  且该布局不读 col_shift。名称列在 `layout.size_left` 前截断。
  **入口/分流**：`open_path` 对纯压缩包（7z/tar 系）直接进 Archive 视图；
  zip/cbz/rar/cbr 经 `open_archive_auto` + `poll_archive_router` 启发式分流
  （Comic → 漫画链路，Files → Archive 视图，列目录错误含密码错误回落漫画链路）；
  显式动作跳过启发式（顶栏「作为漫画打开」→ `open_comic`）；双击图片条目 →
  `open_entry_as_comic`（同漫画已开则直接跳页，否则 `open_comic` +
  `PendingOpenOptions.start_entry`）；双击其他条目 → `temp_open::open_entry_external`
  （提取到 temp_dir 后 `cmd /c start`，启动时 `clean_stale` 回收 24h 前目录）。
  **拖出解压（Windows only）**：`DragOutState` 状态机 Potential →（位移 >40pt 或
  出窗）Extracting →（出窗）`platform::drag_out::do_drag_drop`（OLE HDROP，
  COPY-only，模态）；松开未进 DoDragDrop = 取消并清理暂存目录。
  **多卷 RAR 归一**：`open_path` 最前面经 `normalize_rar_volume_path` 把
  partN.rar/.r00/.r01 改写为首卷，保证历史/密码表 key 稳定。**智能解压目录**：
  `resolve_extract_output`（`needs_wrapper_dir` 判定是否建包名子目录）。
  **解压对话框**（`extract_dialog.rs`）：统一入口 `ExtractDialogState`，三档 wrap
  （smart/always/never）、完成后删除压缩包（`trash` 移回收站）/打开目标文件夹；
  选项写回 settings，`poll_extracts` 按 `ExtractSummary.finished` 在成功时执行
  删除/打开（失败/取消不执行）。
- **文件管理器（app 侧）**：`View::FileManager` 双栏文件管理器（Total Commander
  形态；入口 = 书架顶栏「文件管理器」+ 文件菜单）。`views/file_manager.rs` 视图主体
  （`FileManagerView`：单/双栏布局与分隔条、顶栏/状态栏、键盘与右键接线、
  选中即预览）；`file_manager_panel.rs` 单栏面板 `FsPanel`（导航历史/选择/排序/
  过滤/异步列举）；`file_manager_rows.rs` 纯函数行模型（`list_rows`/`natural_cmp`/
  `SortKey`）；`file_ops.rs` 文件操作引擎（`OpKind::Compress` 桥接 parser
  `create_zip`，dest_dir 记 zip 父目录使完成后栏刷新自动生效）；
  `file_manager_dialog.rs` 确认对话框
  （复制/移动/删除/重命名/新建文件夹/新建文本文件/压缩为 zip，对齐
  `extract_dialog.rs` 模式）。
  **选择模型与 Archive 的差异**：`selected: HashSet<PathBuf>` **含目录**
  （Archive 只存文件条目名、目录选中态派生自后代统计）；焦点是 UI 行索引
  usize（0 = 「..」上级行，盘符根禁用上级）。**file_ops 约定**：每任务一条后台
  线程 + channel 进度 + `Arc<AtomicBool>` 取消；删除逐项 `trash::delete`（回收站）；
  冲突策略由 UI 层操作前一次性确定；`ConflictMode::Ask`（确认框「逐个询问」档）
  执行期真正逐个问——worker 经 `OpEvent::AskConflict` 发 `ConflictQuery`（含
  双方大小/时间快照）并阻塞等 `ConflictAnswer`（100ms 轮询，期间 cancel 旗标
  或回答通道断开都按 Cancel 收拢）；manager 侧 `take_pending_conflict()`/
  `answer_conflict()`，`FileManagerView.pending_conflict` 弹「同名冲突」窗
  （目录冲突只给「合并/跳过」；「本次操作全部应用」把选择记忆为后续同级冲突
  策略——文件级/目录级各自独立，目录级不预决文件级；Cancel 无 apply_all）；
  Skip 记 errors 汇总；目录↔目录冲突非 Ask 恒合并（不整删目标目录），
  AutoRename 用 `name (1).ext` 递增；移动 = `fs::rename` 快速路径，失败回退递归
  复制 + trash 源；取消清理半成品目标文件（写入前取消不动已存在目标）；预扫描与
  递归复制均不跟进符号链接目录（防环）；Windows 长路径统一经 `verbatim_path`
  加 `\\?\` 前缀（>240 字符才加；manifest 未声明 longPathAware）。**fm_* settings**：
  `fm_layout`/`fm_dual_ratio`/`fm_preview_open`/`fm_sort_key`/`fm_sort_asc`/
  `fm_dir_left`/`fm_dir_right`/`fm_confirm_delete`/`fm_show_hidden`（隐藏 =
  `.` 开头或 Windows FILE_ATTRIBUTE_HIDDEN，行模型过滤不进快照，权威在
  settings，与 confirm_delete 同走 ui() 每帧下发）/`fm_bookmarks`（常用目录
  书签，两栏共享；面包屑星标菜单管理，点击跳转经 `fallback_existing_dir`
  逐级回退最近存在祖先，与启动恢复 fm_dir_* 共用）/`fm_col_size_width`/
  `fm_col_mtime_width`/`fm_col_shift`（大小/时间列宽与列块平移，全局单值
  取活动栏——同 fm_sort_key 先例；sanitize clamp 列宽 60..=400、shift
  ≤0 且 ≥ -(两列宽之和)，默认值同 panel.rs SIZE_COL_WIDTH/MTIME_COL_WIDTH/0
  在 storage 侧硬编码同步）；`FileManagerView::snapshot()`
  采集，`maybe_save_fm_state`（`App::update` 末尾）diff 快照后写回 settings——
  **不自行落盘**，退出时 `on_exit` 统一 `save_settings`（排序只持久化活动栏，
  取舍见 `FmStateSnapshot` 注释）；快照在离开 FileManager 视图时重置。设置页
  「文件管理器」tab 改默认布局/比例经 `apply_layout_settings` 同步到休眠视图
  （否则下次进入时快照写回会覆盖设置页改动）。压缩包双击进 Archive 视图
  （设计决策 3，不做面板内浏览）。**栏间 Id 隔离**：每栏内容包在
  `ui.push_id(("fm_panel", idx), …)` 里——`allocate_ui_with_layout` 的子 ui
  与父同 id_stack，不加盐两栏同位置控件共享持久状态（ScrollArea 滚动
  串扰、列宽分隔条拖动联动）。**行内导航停笔**：`render_list` 的行循环每行
  先查 `state == Ready`——行内双击/右键「打开」触发 `navigate_to`/`refresh`
  当场清空 entries，继续按导航前 rows 快照渲染后续行会越界 panic（双击目录
  闪退）；`render_row`/`open_ui_row` 的 entries 索引一律 `get` 防御。
  `FsPanel::poll()` 在非 Loading 状态必须原样返回（`mem::take` 会把 state
  先换成 Idle，曾被它每帧打回 Ready → app 恢复逻辑每 3 帧重列目录 → 闪烁）。
  **盘符切换**：面包屑最左 `list_drives()`（panel.rs，Windows 枚举 A–Z 取
  `read_dir` 可列出者；Unix 为 `/` + `/Volumes/*`），首次点开菜单才起一次性
  后台线程枚举、结果缓存进 `drives`（在途时 `request_repaint_after` 轮询）。
  **目录大小计算**：右键「计算大小」（选中集含目录时可用）/ 空格（焦点目录）
  触发 `FsPanel::request_dir_sizes`——过滤已算出/已请求项，已有在途任务时
  取消并以「剩余 pending + 新增」重启单 worker 线程，结果经 channel 在
  `poll()`（Loading 检查之前）排空进 `dir_sizes`，在途时 `dir_sizes_in_flight()`
  驱动 `request_repaint_after`；navigate/refresh 取消在途并清空。worker 核心
  是 `file_ops::dir_size(path, cancel)`：递归累加文件 len，不跟进符号链接
  （防环且链接不计）、单项失败跳过、cancel 提前返回已累加值、`verbatim_path`
  包长路径。目录行大小列命中 `dir_sizes` 时以比文件大小弱一档的颜色显示，
  未命中留空。
  **名称语义着色**（明细行与网格共用 `entry_name_rich_text`，固定规则不做
  用户配色）：目录 = accent 色（`hyperlink_color`；egui 无字重支持，strong()
  仅为更强颜色，故以 accent 色承担目录强调）、符号链接 = 斜体 + 弱一档、
  隐藏条目 = 弱档 0.6（同 dir_sizes 先例）、普通文件不变；选中行颜色统一
  回退 `text_color()`（选中底色上保对比，斜体保留）；「..」行维持现状。
  颜色全经 visuals 派生不硬编码。网格名称渲染从 `painter.layout` 改为
  `WidgetText::into_galley`（RichText 携带颜色/斜体进 galley）。
  **栏间拖放复制**：文件/目录行（除「..」）改 `click_and_drag` 并
  `dnd_set_drag_payload(FmDragPayload{sources, src_panel})`，payload 经 egui
  全局 dnd 状态跨栏传递；`drag_sources` 纯函数——拖选中行带整个选中集，
  否则仅该行。落点判定**不用** `dnd_drop_zone`，由
  `FileManagerView::poll_inter_panel_dnd` 手写：拖动中经 `DragAndDrop::payload`
  取 payload 画「N 项」光标徽标（Area+Foreground），仅双栏接收，悬停另一栏
  （目录不同）整栏淡底+选中色描边高亮；松开（payload 仍在、primary_released）
  `DragAndDrop::clear_payload` 后调 `open_copy_move_dialog(Copy)` 走既有确认
  复制链路（对齐 F19，非无确认直拷）；拖回源栏或两栏同目录忽略。Esc 取消由
  egui dnd 插件内建处理（清全局 payload，poll 自然返回）。
  **FS watch 自动刷新**：列举就绪（Ready）后 `ensure_watch` 为当前目录
  建非递归 `notify::RecommendedWatcher`（同路径已监听则跳过；失败静默
  降级 None）；回调发 crossbeam 信号 + `wake_ctx.request_repaint()` 唤醒
  UI（ctx 由 FileManagerView 首帧 ui() 经 `set_wake_ctx` 注入）。`poll()`
  在 Loading 检查之前 `poll_watch` 排空事件，去抖 300ms
  （`watch_debounce_ready` 纯函数）后调 `refresh()`（保留选中，同
  Ctrl+R）；去抖窗口内 `watch_refresh_pending()` 并入 ui() 的在途重绘
  判定。`start_listing` 丢弃旧 watcher，refresh 路径复用。
  **文件搜索（Alt+F7，TC Search 简化版）**：`file_manager_search.rs`——
  条件模型 `SearchQuery`（pattern 复用 `wildcard_match`、空 = `*`；大小区间
  字节、只对文件生效；`newer_than_days`；recursive 默认开；content 仅
  ≤1MB 文本扩展名文件、不区分大小写，文本集合与预览共用
  `preview_bytes::is_text_extension`）；匹配判定纯函数
  `meta_matches`/`entry_matches`（内容检查以 `read_text` 闭包惰性注入）。
  `SearchTask` worker 走 collect_branch 同款迭代栈（目录也参与名称匹配、
  符号链接目录不跟进），结果按 50 条/200ms 批量回发，命中上限 10 万截断，
  Drop/关窗即取消。对话框为非模态 `egui::Window`（FileManagerView 持有，
  搜索根 = 打开时焦点栏目录；入口 Alt+F7 / 顶栏「搜索」/ 右键「搜索…」；
  纯 F7 新建文件夹需排 Alt 防同键双触发）。双击/Enter/右键「打开所在目录」
  = 关闭对话框 + `FsPanel::reveal_path`（同目录就绪当场 `apply_reveal`，
  否则导航/refresh 后 `pending_reveal` 在 poll Ready 应用）；「输送到焦点栏」
  = `FsPanel::inject_entries_branch`（命中集直接注入 + branch_view=true，
  同分支视图约定 name 存显示名，退出条件与分支视图一致 navigate/refresh）。
  **新建文本文件（Shift+F4 / 右键 FILE_PLUS）**：`file_ops::suggest_text_file_name`
  （「新建文本文件.txt」重名 `(2)` 递增，扩展名固定末尾）+ `create_text_file`
  （`create_new(true)` 防竞态覆盖）；确认经 `FmDialogOutcome::ConfirmNewFile` →
  `refresh_panel_of` 创建后选中（同新建文件夹）。
  **Shell 动词（Windows only）**：`platform::shell_verbs`（非 Windows stub
  `is_supported()` false，菜单项隐藏）——`ShellExecuteExW` +
  `SEE_MASK_INVOKEIDLIST` 调 `properties`/`openas` 动词（Explorer 右键同款
  系统对话框，Shell 自管无需父窗口）；入口 = Alt+Enter（焦点项属性）+
  右键「打开方式…」（仅文件行，「打开」下方）/ 菜单末尾「属性」(INFO)。
  **面包屑路径编辑**：铅笔按钮 → TextEdit 替换分段（`breadcrumb_edit`
  会话态，Enter/Esc 在编辑 UI 内自测——egui_wants_keyboard_input 会屏蔽
  面板全局键）；Enter 校验 `is_dir()` 后 `navigate_to(fallback_existing_dir)`
  兜底，无效红字「路径不存在」保持编辑（文本变化即清），Esc 还原。
  **全局「← 返回」**：顶栏按钮 + `ReaderApp.previous_view: Option<View>`
  **单层**回退落点（非栈）——仅 `render_file_manager` 打开动作使视图真的离开
  FM 时记录 `Some(FileManager)`（打开失败留在 FM 不记）；`sync_previous_view`
  进 Library/Settings 即清空（顶层目的地不回退）；消费后即 None，再按兜底回
  书架。FileManagerView 常驻内存，返回时目录/选中/滚动原样恢复。
- **PageLoader**: background IO + decode worker threads, results via channels；独立的
  `cover_loader` 负责库封面。**进度条悬停缩略图**：① 全尺寸解码且 compress=false 时
  顺产生成 256px 缩略图；② 悬停时 `request_page_thumbnail` 高优先级 + 按方向低优先
  预取 8 页；③ 全本批量经 `THUMBNAIL_BATCH_MAX_INFLIGHT=2` 节流，翻页冷却期暂停，
  未完成时 `request_repaint_after(100ms)` 排空。webp 缩略图走 `webp_thumb.rs` 的
  libwebp 缩放解码，失败自动回退 image crate。
- **PageCache**: GPU textures; `size_bytes` 为 CPU/GPU 内存估算；上传后尽快释放
  CPU 侧 `ColorImage`。
- **Settings**: validated on load/save, invalid values clamped and reported via
  `error_message`。Notable fields: `theme`, `default_mode`, `default_fit`,
  `double_page`, `wide_page_threshold`, `enable_page_animation`, `compress_images`,
  `decode_threads`, `cache_size_mb`, `real_image_cache_pages`, `show_toolbar`,
  `show_statusbar`, `invert_scroll`, `library_sort`, `page_scroll_threshold`,
  `media_volume`, `media_speed`, `media_audio_device`, `comic_end_action`,
  `media_end_action`。解压相关：`extract_dir`、`extract_threads`（0=自动，≤32）、
  `extract_overwrite`（false=同名自动改名）、`extract_wrap`（"smart"/"always"/"never"）、
  `extract_delete_archive`/`extract_open_folder`（默认 false）。
- **History entries** store both `comic_id` and `path` for robust matching.
- **Per-comic reading settings** (`comic_settings.json`, keyed by comic_id)：打开时
  经 `poll_opener` 覆盖全局默认（`set_mode` → `set_double_page` → `fit_mode` →
  `rotation`，rotation 只收 90° 步进，脏值回 0）；任何来源的改动由
  `ReaderApp::maybe_save_comic_settings`（`App::update` 末尾）diff
  `last_saved_comic_settings` 快照后落盘（快照在 open/close 重置，保存失败也更新
  快照防每帧报错刷屏）。全局 settings 照常更新——每书设置只是打开时的覆盖层。
- **Library covers**: 从首页异步生成存 `covers/`；缺失时按需重新请求；源文件消失
  标记 deleted。书签缩略图在 `covers/bookmarks/<comic_id>-p<page>.jpg`，随书签/书
  删除（仅漫画书签；电子书书签回退封面）。
- **EbookRenderer**: `wry` child webview + 自定义 `ebook://` 协议；章节内容经
  `ebook://reader?chapter=N` 由 `openitgo_parser::html::render_chapter_html` 渲染；
  分页用内嵌 CSS `columns`。Pagination transforms 只能加在 `#column-view` 内的
  `#column-content` 上，`#column-view` 本身是事件容器不可平移。自定义协议回调的
  URI 是绝对 URL：`ebook://reader/res/...` 中 `reader` 是 host，不会出现在
  `uri().path()`——资源判别符必须放在 path/query。**WebView2 (Windows) 约束**：
  wry 用 `http://ebook.*` workaround 拦截自定义协议，页面内绝对 `ebook://...`
  请求（fetch/img/字体）不会被拦截、直接失败（fetch 报 TypeError: Failed to
  fetch）——JS 与章节 HTML 内一律用相对 URL（`?chapter=N`、`/res/...`），由
  当前 origin 解析后在各平台都能命中协议回调。**目录树**：`EbookChapter.level`
  为嵌套深度（markdown 经 `chapters::split_markdown` 按 pulldown-cmark 标题事件
  分章并归一化层级——parse 与 render 共用此入口；EPUB 为 navpoint 深度；txt/mobi
  恒 0），目录面板经 `views/ebook_toc.rs::toc_rows` 纯函数展开为缩进+可折叠树，
  折叠态存 `OpenEbook.toc_collapsed`（会话内，不落盘）。**markdown 渲染**：
  fenced code block 经 syntect（`default-fancy`，base16-ocean.dark）Rust 侧高亮；
  相对路径图片改写为 `/file/` 根相对 URL，handler 经 `read_text_resource` 从 md
  同目录提供（canonicalize 后必须在书目录子树内，防穿越）。**菜单停放（#52）**：egui 弹层
  无法穿透原生 webview，菜单打开时 `render_ebook` 用 `menu_overlay_open(ctx)` 驱动
  `EbookView::set_webview_hidden`（wry `set_visible(false)`，状态去重避免每帧
  IPC）。**位置保持**：字号/边距/主题变化按字符偏移保持，窗口 resize 防抖后按滚动
  比例（scroll 模式）或当前 spread（分页模式）保持。
- **Media playback (macOS)**：mpv 渲染进 CAOpenGLLayer，插到 wgpu CAMetalLayer 之下
  （`insertSublayer:below:`；`openitgo-app/src/platform/macos/mpv_view.rs`）。app 以
  透明 backbuffer 运行（`with_transparent(true)` + 零 alpha `clear_color`），媒体
  CentralPanel 用透明 frame，视频从未绘制区域透出，egui 菜单/弹层在上层合成。
  letterbox 用 mpv `background=color` + `background-color`（`#AARRGGBB`，来自
  `settings.background_color` + `chrome_opacity`）。裸 layer 几何变化必须走禁用
  actions 的 `CATransaction`（OSD 透明度渐变依赖隐式动画，须留在事务外）。
  `drawInCGLContext` 里 CA 绑定自己的 FBO（恒非 0）——必须查询
  `GL_FRAMEBUFFER_BINDING` 传给 `RenderContext::render`，`FLIP_Y=1`，否则合成全
  透明。播放进度存 `HistoryEntry.char_offset`（毫秒）。
- **Media playback (Windows)**：mpv 经 `wid` 渲染进 `WS_CHILD` HWND 子窗口
  （`platform/windows/mpv_view.rs`），输出/硬解/letterbox 全由 mpv 自管（无透明
  合成；`with_transparent` 等均为 macOS-only）。`wid` 必须在 `mpv_initialize` 前
  设置，故 `MediaView::open` 两段式：`PendingVideoView::create(parent)` 先建 HWND →
  `MpvPlayer::new_with_wid` → `finish(bounds, &player)`；统一入口
  `crate::platform::video`。HWND 恒在 wgpu 表面之上，`render_media` 在
  `menu_overlay_open(ctx)` 时零尺寸 bounds 停放视频窗口（与电子书 #52 同模式）。
  OSD 委托 mpv 自绘 `show-text`（`mpv_command_async`）；坐标逻辑值，`set_bounds`
  内按 `GetDpiForWindow` 换算物理像素。
- **文件关联（Windows only）**：`platform/windows/file_assoc.rs`（其他平台 stub，
  统一入口 `crate::platform::file_assoc`）注册到 `HKCU\Software\Classes`（winreg，
  无需管理员）：三个 ProgID（`OpenItGo.Archive/Image/Media`）+ `.ext` 默认值与
  `OpenWithProgids`；覆盖前先备份到 `OpenItGo.bak\<ext>`，取消时仅在默认值仍是本
  程序 ProgID 才删除并恢复备份；完成后 `SHChangeNotify`。分组图标在
  `assets/icon/openitgo.rc`（4 个 ICON 资源，`AssocGroup::icon_index()` 返回
  1/2/3）。UserChoice 优先于 Classes 默认值——UI 提供
  `open_default_apps_settings()` 引导。设置页「文件关联」tab 惰性 `query_status()`
  并缓存，状态以注册表为准不落 Settings。文件关联双击 = 新进程 + argv[1]（无单实例
  机制）。
- **Media OSD**：macOS 走视频 layer 内的 CATextLayer（`MpvNativeView::set_osd`）；
  原生视图停放时由 `MediaView::ui` 用 egui painter 画同文。`show_osd` 存文本 + 1s
  过期；`tick_osd` 未过期时重挂 `request_repaint_after`（egui 定时帧只触发一次且
  略提前，不重挂则空闲时 OSD 永远不清）；`close()` 清 OSD 防泄漏到下一个媒体。
- **Media menus/popups**：媒体 seek bar 需 scoped `ui.spacing_mut().slider_width`
  覆盖（egui 0.35 Slider 仍默认分配 100px `slider_width`）。`menu_overlay_open(ctx)`
  同时用于菜单打开时阻止全屏工具栏自动隐藏。
- **Media preferences**：volume/speed/audio-device 全局持久化，打开后由
  `apply_startup_settings` 应用；音量/速度立即生效，音频设备延迟
  （`pending_startup_device`）到异步 `audio-device-list` 回复落进
  `PlayerState::audio_devices` 后在 `sync_state` 校验——失效设备回退 "auto" 并经
  `take_startup_device_invalid` 一次性上报。
- **Media auto-next（自动续播）**：`maybe_auto_next_media` 在 `render_media` 里
  `sync_state` 之后运行（`media_end_action` = `NextInDir` 才启用）；`ended` 且无
  `error` 时打开同目录下一个媒体，每个媒体只触发一次（`auto_next_fired`）；后继经
  `next_media_in_dir` 按数字感知 `natural_cmp` 排序。「自动播放下一集」OSD 存
  `pending_open_osd` 由下一次 `open` 显示（提前画会画在被销毁的旧视图上）。播放
  错误不触发续播；最后一集弹一次性 `已是最后一集` OSD。
- **Comic end action**：`settings.comic_end_action` —— 请求下一页但 `current_page`
  不前进时：`DoNothing`（默认）/`WrapToFirst`/`NextSibling`（经
  `next_comic_sibling`，自然序；最后一个时 `error_message` = `已是最后一个漫画`）。
- **Wide page in double-page mode**：aspect ≥ `wide_page_threshold` → `spread_pages`
  返回 `(Some(current), None)` 居中；空左槽必须用零尺寸（不是
  `FALLBACK_PAGE_SIZE`），否则 spread 获得幻影宽度偏离中心（LTR 单封面同理）。
- **Window title**：`ReaderApp::sync_window_title` 每帧变化时设
  `ViewportCommand::Title`——Reader: `{page} — {comic} - OpenItGo`；
  Ebook/Media/Loading: `{file} - OpenItGo`；Library/Settings: `OpenItGo`。页名来自
  `PageSource`（压缩包条目 basename，PDF 为 `第 N 页`）。
- **MpvPlayer command rule**：UI 线程的 mpv 命令/属性调用必须用异步 API
  （`mpv_command_async` / `mpv_set_property_async` / `mpv_get_property_async`，见
  `openitgo-media/src/player.rs`）。阻塞调用会把 UI 线程停在 mpv dispatch 队列上，
  与首帧 DR 分配（只能由 UI 线程响应 `mpv_render_context_update()`）形成循环等待，
  冻结窗口。
- **MpvPlayer observe/userdata 分配**：观察 id 1-9（9 = `chapter`），异步查询
  userdata 100（`audio-device-list`）/ 101（`chapter-list`），常量在 `apply.rs`
  （`AUDIO_DEVICES_REPLY_USERDATA`/`CHAPTER_LIST_REPLY_USERDATA`）；下一可用：观察
  id 10、userdata 102。`chapter-list` 在 FILE_LOADED 与需要时经
  `request_chapter_list` 拉取进 `PlayerState.chapters`。
- **MpvPlayer teardown**：`Drop` 先设 quit flag 并 join `mpv-events` 线程（50ms
  `mpv_wait_event` 超时），再 `mpv_terminate_destroy`——颠倒顺序会因
  `mpv_wait_event` 与 handle free 竞争而 segfault。
- **滚轮翻页（漫画）**：egui 0.35 移除了 `raw_scroll_delta`，`smooth_scroll_delta`
  会把一次滚轮刻度摊到多帧，按帧阈值判定必然"滚了不动"或"一跳多页"。
  `render_reader` 用 `raw_wheel_delta_y(&i.events)` 汇总本帧增量（Line/Page ×
  40pt），经 `accumulate_page_turn` 累加：满阈值翻一页清零，单帧巨幅也只翻一页，
  余量跨帧保留。阈值 = `page_scroll_threshold`（默认 12pt，1–40 可调）。
  Ctrl/Cmd+滚轮缩放共用同一事件和，±2.0 阈值。**改滚轮代码不要回退到
  `smooth_scroll_delta`**（webtoon 连续滚动除外）。
- **图标字体（egui_phosphor_icons）**：`fonts::setup_fonts` 把 `phosphor-icons`
  追加进 Proportional 回退链，系统 CJK 字体（`cjk`）同时挂进 Proportional 与
  Monospace（Monospace 用于压缩包预览文本，缺挂会中文豆腐块；非 UTF-8 文本经
  `decode_text_guess` 识别）。**字体未注册时 epaint 0.35 直接 panic**——任何构造
  `ReaderApp` 的 example 必须在 `run_native` creator 里先调
  `openitgo_app::fonts::setup_fonts`；带 UI 的 example 还需每帧
  `request_repaint_after` 轮询排空 loader 结果（ui_smoke/profile_* 已内置）。
- **Packaging**：`scripts/package-macos.sh` 签名前跑 `bundle_mpv`，把 libmpv 及
  Homebrew 依赖复制进 `Contents/Frameworks` 并改写 install names 为 `@rpath`。
- **Dock open (macOS)**：`platform::macos::dock_open` swizzle NSApplication
  delegate 把 openURLs/openFiles 入 `OPEN_QUEUE`，每帧在 `App::ui` 排空。idle egui
  不重绘，回调必须唤醒：`set_wake_context`（main.rs creator 注册）存
  `egui::Context`，`enqueue_paths` 入队后 `request_repaint()`——删掉会复现
  "dock 打开的文件卡住"bug。macOS 平台层基于 **objc2 0.6** +
  `objc2-core-foundation`——**不要重新引入 `objc` 0.2**（已从依赖树完全移除）。
  备查：`mpv_view` 部分宽松编码写法依赖 wry 链带入的 objc2
  `disable-encoding-assertions` feature；该 feature 消失时 debug 构建会 panic，
  届时开 objc2 `relax-void-encoding` feature 缓释。
- **Startup open (non-macOS)**：`initial_open_path`（app.rs）从 `OPENITGO_OPEN`
  环境变量（优先）或 `argv[1]` 取路径经 `open_path` 打开；`exists()` 检查天然过滤
  无效参数。`open_path` 分发顺序：ebook → media → 图片（`is_image_file`）→
  zip/cbz/rar/cbr（`open_archive_auto` 启发式分流）→ 纯压缩包
  （`open_archive_browser`）→ comic。文件菜单「打开文件…」也走 `open_path`（过滤器
  扩展名共用 `COMIC_EXTS`/`EBOOK_EXTS` 等常量）。图片分支 `open_image_as_comic` 把
  父目录作为漫画打开并置一次性 `pending_open_options`：`poll_opener` 在每书设置与
  历史恢复之后消费——`go_to_page` 定位该图（`find_image_page_index`）并强制单页；
  应用后同步 `last_saved_comic_settings` 快照，防止「单页」被误存为长期每书设置。

## Diagnostic Examples

带 UI 的用法均为 `cargo run -p openitgo-app --example <name> -- <路径>`：

- 漫画（`openitgo-app/examples/`）：`flip_through.rs`（PageLoader 全页遍历冒烟）、
  `rapid_flip.rs`（80ms 连续翻页压力回归）、`profile_open.rs`（打开 10 秒后打印
  缓存快照退出）、`profile_view.rs`（持续运行，每 10 秒快照）、`ui_smoke.rs`（当前
  页入缓存即退出，30 秒超时）。
- 媒体：`media_smoke.rs`（完整 UI 打开媒体，播放推进即退出）、`probe_visible.rs`、
  `probe_mpv_view.rs`、`probe_video_overlay.rs`（图层合成验证）；
  `openitgo-media/examples/{probe,probe_render,probe_cover}.rs`（无头播放器/渲染
  上下文/封面，`probe_cover` 跨平台，`vo=image` 无需窗口）。
- 电子书：`probe_ebook_menu.rs`（菜单停放 #52 验证）。
- 文件管理器：`fm_smoke.rs`（双栏列举就绪即退出）。
- `OPENITGO_MPV_LOG=1` 开启 mpv debug 日志（stderr）。

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
