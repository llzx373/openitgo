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
  cancel, paused)`（FM「压缩为 zip」用）——条目名 = 相对各 source 父目录（多 source 各自
  basename 为根，'/' 分隔），目录写 `name/` 条目，文件 256KB 分块写入（块间查
  暂停/取消，阶段 AB 起支持暂停），符号链接
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
  加 `\\?\` 前缀（>240 字符才加；manifest 未声明 longPathAware）。
  **file_ops 队列化（阶段 AA）**：并发上限 `max_concurrent`
  （settings `fm_op_threads`，默认 2，0 = 不限，validate/clamp ≤ 8）——
  `submit` 超上限进 `queued: VecDeque<QueuedTask>`（不起线程、无进度），
  `poll` 移除完成项后 `pump_queue` FIFO 放行；排队取消 = 直接出队（静默，
  无 Finished 事件）。`set_max_concurrent` 经 `FmBehaviorOptions.op_threads`
  每帧下发：在途不动，调高立即放行、调低只影响后续。`task_summaries()`
  给任务面板行快照（在途提交序 + 排队 FIFO 序）；`FinishedOp` 增带
  dest/conflict/delete_permanent 供「重试失败项」重建同参数任务
  （`retry_sources` 从 errors 过滤仍在的源、去重保序；errors 恒为源路径）。
  **fm_* settings**：
  `fm_layout`/`fm_dual_ratio`/`fm_preview_open`/`fm_sort_key`/`fm_sort_asc`/
  `fm_dir_left`/`fm_dir_right`/`fm_confirm_delete`/`fm_show_hidden`（隐藏 =
  `.` 开头或 Windows FILE_ATTRIBUTE_HIDDEN，行模型过滤不进快照，权威在
  settings，与 confirm_delete 同走 ui() 每帧下发）/`fm_view_mode`（"list"|
  "brief"|"thumbs"，取活动栏）/`fm_tabs_left`/`fm_tabs_right`/`fm_active_tab_left`/
  `fm_active_tab_right`（标签页目录列表与活动索引，活动标签 = 实时目录，
  越界 clamp）/`fm_bookmark_groups`
  （常用目录书签分组 `FmBookmarkGroup{name, items}`，两栏共享；旧扁平
  `fm_bookmarks` 仅读取兼容——clamp 时非空即并入「常用」组并清空，
  保存 skip 空 Vec 不再写出；分组空名修「未命名」、组内去重、空组保留。
  面包屑星标菜单：「添加当前目录 ▸」分组子菜单（已在组内打勾禁用）+
  「新建分组…」（非模态 egui::Window，菜单内联输入与 CloseOnClickOutside
  冲突故走独立窗口）+ 各分组 SubMenuButton 子菜单（书签点击跳转经
  `fallback_existing_dir` 逐级回退最近存在祖先、与启动恢复 fm_dir_* 共用；
  ✕ 移除、组尾重命名/删除分组——删除从简无确认））/`fm_col_size_width`/
  `fm_col_mtime_width`/`fm_col_shift`（列宽与列块平移，全局单值
  取活动栏——同 fm_sort_key 先例；阶段 V 起列宽两字段为 `fm_columns`
  的兼容回写（权威在后者）；sanitize clamp 列宽 60..=400、shift
  ≤0 且 ≥ -(两列宽之和)，默认值同 panel.rs SIZE_COL_WIDTH/MTIME_COL_WIDTH/0
  在 storage 侧硬编码同步）/`fm_delete_mode`/`fm_space_action`/`fm_dirs_first`/
  `fm_drag_confirm`/`fm_archive_open`/`fm_esc_keep_selection`（行为设置包，
  见下「行为设置包（阶段 O）」段）/`fm_dblclick_blank_up`（双击空白回上级，
  见「键位补齐包（阶段 P）」段）/`fm_system_icons`（系统真实图标，
  见「系统真实图标（阶段 S）」段）/`fm_saved_filters`/`fm_filter_bar_bottom`
  （见「过滤增强（阶段 T）」段）/`fm_rubber_band`（见「选择增强（阶段 U）」
  段）/`fm_columns`（明细列表列配置，见「显示模式与自定义列（阶段 V）」
  段）/`fm_command_bar`（命令行输入条，见「命令行输入条（阶段 X）」段）/
  `fm_button_bar`（自定义按钮栏，见「自定义按钮栏（阶段 Y）」段）/
  `fm_watch_recursive`（递归目录 watch，见「拖放/剪贴板/watch 收尾（阶段 Z）」
  段）/`fm_op_threads`（文件操作并发上限，见「任务队列 + 错误汇总（阶段 AA）」
  段）；`FileManagerView::snapshot()`
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
  **阶段 A–I 增量（TC 对齐）**：选择增强——`*` 反选 / `+`「选择组」/ `-`
  同框预置取消选择（`SelectGroupDialog`，`wildcard_match` 通配 `;` 多模式
  不区分大小写、「仅选文件」变体，模式串会话记忆；egui 无小键盘键、主键盘
  `*` = Shift+8，统一经 `Event::Text` 捕获）；其余可打印字符进 type-ahead
  缓冲（`FsPanel::type_ahead_push`，命中即 Select 焦点 + 最小滚动揭示；
  空格保留给目录大小不进 type-ahead）；`SortKey::Ext` 扩展名排序键；状态栏
  「已选 N 项 · 合计 X」（文件直接求和，目录仅计 `dir_sizes` 已缓存值、
  不触发计算，0 字节不显示合计）。Alt+↓ 开/关焦点栏目录历史下拉
  （`history_menu_toggle` 一次性请求，菜单项新→旧、当前项 ✓，点击直跳
  `navigate_history_to`）。栏间快捷键（仅双栏）：Ctrl+U `panels.swap(0,1)`
  交换两栏（watcher/loader 随结构体走，active 不变）、Ctrl+→ 把本栏焦点目录
  （非目录则当前目录）带给另一栏 / Ctrl+← 反向、Ctrl+\ 回本栏根目录。
  进度增强：单文件进度 + `OpSpeedMeter` EMA 速度/ETA（500ms 采样节流）+
  `FileOpManager::set_paused` 暂停（worker `wait_if_paused` 自旋，cancel
  即时退出）。Ctrl+Q 快速预览（双栏：非活动栏整栏替换为预览面板、目标 =
  活动栏焦点文件、焦点移动跟随，关闭即 `clear_preview`；单栏 = 预览面板
  开关；Q 不抢 type-ahead——后者有 `!mods.command` 门控）。Ctrl+B 分支视图
  （`toggle_branch_view`：当前目录 + 所有子目录文件扁平列出，列举有截断
  上限并在状态栏提示，面包屑显示 [分支]，navigate/refresh 退出）。
  Ctrl+M 批量重命名（`file_manager_rename.rs::plan_renames` 纯函数计划 +
  `MultiRenameDialog` 实时预览，仅执行非 skip 且无 error 项，逐项失败汇总）。
  栏内标签页：Ctrl+T 新建（复制当前目录）/ Ctrl+W 关闭（剩 1 个 no-op）/
  Ctrl(+Shift)+Tab 循环；纯 Tab 切焦点栏与 Ctrl+Tab 互不干扰。缩略图视图：
  `file_manager_thumbs.rs` `ThumbCache`（FileManagerView 级持有、两栏共享，
  网格 cell 176×200，行索引/选择/过滤/type-ahead 与列表模式完全共用——
  网格只是渲染层；可见范围变化 bump 请求代次，worker 丢弃过期请求）。
  系统剪贴板互通：`platform::clipboard_files`（CF_HDROP + 剪切标志，与
  Explorer 互贴；写失败回退应用内路径列表；剪切粘贴生效后按 Explorer 惯例
  清空系统剪贴板防重复粘贴）。外部拖入：app 侧 `handle_dropped_files` 经
  `FileManagerView::panel_rect_at` 命中落点栏；行拖出窗口复用
  `platform::drag_out::do_drag_drop`（OLE HDROP，文件本就在盘上直接拖）。
  **新建文本文件（Shift+F4 / 右键 FILE_PLUS）**：`file_ops::suggest_text_file_name`
  （「新建文本文件.txt」重名 `(2)` 递增，扩展名固定末尾）+ `create_text_file`
  （`create_new(true)` 防竞态覆盖）；确认经 `FmDialogOutcome::ConfirmNewFile` →
  `refresh_panel_of` 创建后选中（同新建文件夹）。
  **Shell 动词（Windows only）**：`platform::shell_verbs`（非 Windows stub
  `is_supported()` false，菜单项隐藏）——`ShellExecuteExW` +
  `SEE_MASK_INVOKEIDLIST` 调 `properties`/`openas` 动词（Explorer 右键同款
  系统对话框，Shell 自管无需父窗口）；`edit_file` = `edit` 动词失败回退
  `openas`（F4）；`run_as_admin` = `runas` 动词触发 UAC（Ctrl+Shift+Enter）。
  入口 = Alt+Enter（焦点项属性）+
  右键「打开方式…」（仅文件行，「打开」下方）/ 菜单末尾「属性」(INFO)。
  **面包屑路径编辑**：铅笔按钮 → TextEdit 替换分段（`breadcrumb_edit`
  会话态，Enter/Esc 在编辑 UI 内自测——egui_wants_keyboard_input 会屏蔽
  面板全局键）；Enter 校验 `is_dir()` 后 `navigate_to(fallback_existing_dir)`
  兜底，无效红字「路径不存在」保持编辑（文本变化即清），Esc 还原。
  **行为设置包（阶段 O）**：六个可选行为设置，默认值 = 一期现状——
  `fm_delete_mode`（"trash"/"permanent"：Delete 执行路径分派 trash::delete /
  `permanent_delete` 物理删除；DeleteDialog permanent 档红色「永久删除，
  无法恢复」警示；**Shift+Del = 另一档快捷** = 设置档 XOR Shift）、
  `fm_space_action`（"dir_size"/"toggle_select"：空格 TC 勾选下移经
  `FsPanel::toggle_focused_selection_and_advance`；Insert 无条件同语义）、
  `fm_dirs_first`（bool，`list_rows` 加 dirs_first 参数、false 目录文件混排；
  RowsKey 同步——Archive 的 list_rows 是 archive_tree.rs 独立签名不受影响）、
  `fm_drag_confirm`（false 栏间拖放直拷 AutoRename，Shift = 移动且徽标
  「N 项 · 移动」；确认模式 Shift 不区分）、`fm_archive_open`（"archive"/
  "comic"/"ask"：open_ui_row 压缩包分支分派，ask 经 FmIntents.archive_ask
  帧尾转 `archive_ask` 状态、鼠标处 Area+Frame::popup 二选一，Esc 链首档/
  点击外关闭）、`fm_esc_keep_selection`（true 时 Esc 链止于清过滤，选中
  保留、分支不退出）。统一 `FmBehaviorOptions`（file_manager.rs，含
  confirm_delete/show_hidden；`FmArchiveOpen` 枚举）由 app.rs
  `fm_behavior_options(&settings)` 每帧构造经 `view.ui(ui, callbacks, options)`
  下发（同 show_hidden 模式，不进 FmStateSnapshot）；面板级字段 show_hidden/
  dirs_first 逐栏下发，RowsKey 自动失效。设置页「文件管理器」tab 分「删除/
  显示与布局/交互行为」三子区块。
  **键位补齐包（阶段 P）**：F4 = 焦点文件系统「编辑」动词（`edit_file`，
  无 edit 关联回退「打开方式…」，目录忽略，非 Windows 无此键位）；
  Ctrl+D = 开/关焦点栏书签菜单（`bookmarks_menu_toggle` 一次性请求，
  同 Alt+↓ 历史菜单机制——书签菜单已从 MenuButton 重构为 Button +
  `Popup::menu` + `open_memory(SetOpenCommand::Toggle)` 模式以支持键盘开关，
  弹层 Id = ("fm_bookmarks_menu", idx)）；Ctrl+L = 选中集内目录批量
  `request_dir_sizes`（选中集无目录回退焦点目录）；Alt+F5 = 压缩为 zip
  对话框、Alt+F9 = 解压对话框（选中集恰为单个压缩包时，目标 = 异目录另一栏
  否则当前目录，同右键链路）；Ctrl+Shift+Enter = `run_as_admin`（焦点
  文件/目录，verb "runas"）。**egui `key_pressed` 不认修饰键**——纯 Enter
  打开分支已排 command/alt（否则 Alt+Enter/Ctrl+Shift+Enter 同帧双触发），
  纯 F5 排 Alt（Alt+F5 压缩）；Alt+F4 系统关窗不拦。**双击空白处回上级**
  （`fm_dblclick_blank_up` 默认 true）：列表/网格 ScrollArea 视口内、
  内容区之外的双击 → `parent_dir()`；判定纯函数 `dblclick_hits_blank`
  （内容坐标 = 指针 - 视口左上 + 滚动偏移；网格经 `GridBlankGeom` 把每行
  右侧余量与末行未排满部分也算空白），接线在 render_list/render_grid 帧尾
  `blank_dblclick_up`——行/cell 双击几何上不落在空白区，互不冲突。
  **状态栏增强（阶段 Q）**：驱动器剩余空间——`platform::drive_info`
  （Windows `platform/windows/drive_info.rs` `GetDiskFreeSpaceExW` 取
  `lpFreeBytesAvailableToCaller` 调用者可用口径；`volume_key` 从路径提取
  卷根缓存键：盘符根大写 `C:\` / UNC `\\server\share\`；非 Windows 为
  statvfs 实现而非 stub——libc 已是 openitgo-media 直接依赖，openitgo-app
  以 `[target.'cfg(unix)'] libc` 复用，unix 缓存键 = 目录路径本身）。
  状态栏右侧路径前弱色显示 `剩余 X GB`（human_size）；`status_drive_free`
  30s 会话内缓存（卷 key + 字节 + 时刻；失败 None 同样缓存防每帧系统
  调用），卷 key 变化立即重查，每帧调用即「导航/刷新/切栏自然触发」。
  过滤激活指示：焦点栏过滤非空时状态栏弱色 `过滤: xxx`，与 type-ahead
  `定位:` 同区域并列。
  **预览器增强（阶段 R，FM 三处共用 `draw_preview_content(ui, in_popup)`：
  单栏预览面板 / Ctrl+Q 快览 / F3 弹窗；Archive 预览面板维持原行为仅
  适配签名）**：加载器返回 `PreviewOutcome { data: PreviewData, bytes:
  Option<Arc<[u8]>> }`（preview_bytes.rs）——原始字节留档是 HEX 查看与
  编码重解码的数据源（超 64MB 未读为 None）；FM `set_preview_target`
  不再按 `is_previewable_name` 门槛分流，所有类型都读取，二进制落 HEX
  模式而非「不支持预览」占位。模式 tab（文本/图片/HEX）按内容自动初值、
  可用性门控、手切换目标不保留（`preview_mode` 随目标重置）。HEX 渲染
  = `format_hex_line(bytes, line)` 纯函数按需生成 + show_rows 虚拟化
  （16 字节/行 `偏移  hex 对  |ASCII|`，不物化全表——64MB = 4M 行）。
  文本编码手动切换：parser 新增 `decode_text_with(bytes, label)`
  （utf-8 严格；gbk/shift-jis/big5 有损——截断多字节尾巴不致整篇空白），
  app 侧 `decode_preview_text(bytes, label)` 统一嗅探上限/NUL 拒绝/64k
  字符截断，`preview_redecode` 用已读字节重解码不重读文件，换新目标回
  自动。文本内搜索：`find_text_matches` 纯函数（不区分大小写行号列表）；
  **无搜索词时维持 TextEdit 只读多行（自动换行可选中），有搜索词才切换
  行级虚拟化视图**（匹配行 faint 底色、当前匹配选中色、◀▶ 循环 +
  `preview_search_reveal` 顶对齐揭示）。图片旋转 = `paint_rotated_image`
  Mesh UV 角点轮换（顺时针 k×90°，k 奇数时分配 rect 交换宽高；不写文件）。
  F3 弹窗最大化：egui Window 不支持自定义标题栏按钮——「最大化」按钮在
  内容头部（in_popup=true 时），最大化态走全屏 Area（Order::Foreground +
  content_rect）自绘标题行（还原/关闭），不经 Window 内存避免位置串扰。
  **系统真实图标（阶段 S，Windows only）**：`platform/windows/file_icons.rs`
  `extract_icon(path, is_dir, large) -> Option<ColorImage>`——
  `SHGetFileInfoW` 取 Shell 图标（目录/普通扩展名用
  `SHGFI_USEFILEATTRIBUTES` 伪属性免读盘，exe/lnk/ico 图标内嵌于文件
  自身走实路径）；HICON→RGBA 经 `GetIconInfo`+`GetDIBits`（32bpp、
  biHeight 取负值得 top-down 免翻转；BGRA 交换、全零 alpha 兜底不透明、
  预乘转直通并钳位；hbmColor 为空的单色图标返回 None；DestroyIcon/
  DeleteObject 经 guard 清理）。非 Windows stub 恒 None（platform.rs
  内联）。缓存 `views/file_manager_icons.rs`（`SysIconCache`，仿
  ThumbCache 单 worker + channel + generation 丢弃）：key =
  `SysIconKind`（目录 = Dir 单一键；exe/lnk/ico = 完整路径；其他 =
  小写扩展名、无扩展名为空串，`sys_icon_kind` 纯函数）+ 尺寸档
  （large bool），容量 256 按插入序逐出（stamp 防重插序项误删），
  无有效性维度（图标不随 mtime 变化）。接线：`fm_system_icons`
  （默认 true，设置页「显示与布局」区块）经 `FmBehaviorOptions.
  system_icons` 下发；列表行 16pt（`egui::Image` widget）、网格
  is_dir 与文件 `!drawn` 分支 32pt 档（painter.image 居中于缩略图区），
  「..」行不动、图片缩略图优先不变；thumb_visible bump 处同 bump
  sys_icons 代次；Miss/Failed 回退字体图标（`entry_icon`）。
  **过滤增强（阶段 T）**：`list_rows` 过滤经 `filter_matches` 纯函数
  （file_manager_rows.rs）——过滤串含 `*`/`?` 走 `wildcard_match`
  （`;` 分隔多模式），否则维持不区分大小写子串。过滤条渲染抽为
  `render_filter_bar`（输入框固定 Id `fm_filter_edit` + FUNNEL 下拉：
  保存方案点击应用 / 会话历史最近 8 条 / 「保存当前过滤」过滤非空可用），
  两处调用点——默认顶栏右侧，`fm_filter_bar_bottom`（默认 false）时
  render_panel 先给内容区留 28pt 再在焦点栏底部渲染（只作用于焦点栏，
  切栏跟随）。`saved_filters` 视图持有、构造后 `set_saved_filters` 注入、
  入 `FmStateSnapshot` 走快照写回 settings（同 bookmark_groups 模式）；
  `filter_history` 会话内不落盘（`push_history_capped` 纯函数去重置顶
  截断，记录点 = 菜单应用/失焦/Esc 清空）。**Ctrl+S** 置
  `filter_focus_request` 一次性请求（必须在 handle_keyboard 的
  egui_wants_keyboard_input 检查之前——过滤框聚焦时该检查恒 true，
  「已聚焦则选中全文」经 TextEditState set_char_range 实现）。过滤框
  聚焦时 Esc = 清空 + surrender_focus 并置 `filter_esc_handled`，
  handle_keyboard 的 Esc 链见到标记跳过（渲染先于键盘处理，交还焦点后
  wants_keyboard_input 变 false，无此标记会同帧双消费）。
  **选择增强（阶段 U）**：**鼠标框选**（`fm_rubber_band` = "right"（默认）
  /"left"/"off"，设置页「交互行为」）：视图级 `band: Option<RubberBand>`
  状态机（起点/按钮/所属栏/active），render_list/render_grid 帧尾
  `rubber_band()` 处理——"right" = 右键视口内按下（egui click 判定自带
  位移阈值，行/cell 渲染处另有 `band.active` 门控保险防弹菜单）；
  "left" = 左键空白区按下（行/cell 上左键起拖维持拖放 payload，空白判定
  复用 `dblclick_hits_blank` 几何）。位移超 `BAND_THRESHOLD`(6pt) 才
  active；拖动中只画半透明矩形（painter.with_clip_rect(viewport)，不
  实时改选中）；松开按命中应用：**无修饰 = 替换、Shift = 追加、Ctrl =
  切换，anchor/focus 不动**（框选是区域语义，不参与锚点区间）；「..」
  行（索引 0）跳过；丢失的松开（拖出窗释放）清状态防滞留。命中纯函数
  `rows_in_rect`（竖向重叠，内容坐标 = 指针 - 视口左上 + 滚动偏移）/
  `cells_in_rect`（末行未排满与行右空白自动丢弃），底/右边压界减 eps
  不命中下一行/列。**保存/恢复选择集**：`saved_selections:
  Vec<(String, Vec<PathBuf>)>`（会话内不落盘）；右键菜单「选择」子菜单
  ——「保存当前选择…」（`SaveSelectionDialog` 复用 BookmarkGroupDialog
  单输入模式，打开时捕获选择集快照，重名拒绝）+ 各已存项（点击 =
  `restore_selection` 替换式恢复到该栏：按路径匹配当前栏可见项，不在
  当前目录的忽略并经 op_error 提示「N 项不在当前目录」，焦点设到首个
  命中行 + 滚动揭示）+ ✕ 删除不收起菜单。
  **显示模式与自定义列（阶段 V）**：**三态视图**——`PanelViewMode` 加
  `Brief`（简表），顶栏「视图」按钮按当前模式显示并经 `Popup::menu`
  三选（列表/简表/缩略图）；简表 = 多列排布的紧凑名称行（16pt 图标 +
  单行截断名称，行高 = ROW_HEIGHT，**行主序**），列宽 =
  `brief_col_width(最长名称 char-unit)`（×7pt + 28 clamp 120..=300），
  与网格共用线性行号/`last_grid_cols` 键盘步进（`is_grid_like()` =
  非 List）/`GridBlankGeom` 空白双击/`rubber_band` 框选（grid 参数加
  cell 宽）。**自定义列**：`ColumnKind{Name, Ext, Size, Mtime, Attr,
  Comment}`（panel.rs；`as_str`/`from_setting`/`label`/`sort_key`/
  `default_width`，Comment 无排序键——数据见阶段 W）；`FsPanel::columns:
  Vec<(ColumnKind, f32)>` 不变式 **columns[0] 恒为 Name 且弹性宽度**
  （宽度值忽略）、固定列 ≥1（`toggle_column` 维持，列头右键勾选增删，
  唯一固定列置灰）；`column_layout(right, shift, columns)` 从右往左排
  固定列（文字右锚点 = 列右缘，最右列 = 行右缘内 COL_RIGHT_PAD +
  col_shift），默认三列下与旧硬编码公式逐项等价（有单测锚定）；
  `drag_column_sep` 泛化（sep i = columns[i]|columns[i+1] 分隔线，sep0
  只动 shift，i≥1 调 columns[i] 宽 + shift 吸收，min_shift 按固定列宽
  合计）；列头每格都可右键（排序菜单含「不排序」「属性」+ 升降序 +
  列勾选）；行直绘按 layout.fixed 遍历（Size 目录弱档/Ext 目录留空/
  Attr = `attr_string`(R/H/S，空留空）/Comment = 注释）。**排序键**：
  `SortKey::Unsorted`（list_rows 入口早退：read_dir 物理序，不做目录/
  文件分组，asc=false 整体反向）与 `SortKey::Attr`（属性串字典序，
  空垫底同 Ext 约定）；FsEntry 加 `is_readonly`/`is_system`（Windows
  FILE_ATTRIBUTE_SYSTEM 经 `windows_attr_system`，非 Windows 恒 false）。
  **持久化**：`fm_columns: Vec<String>`（"kind" 或 "kind:width"，Name
  恒首位不带宽；`parse_columns` 规范化：未知丢弃/宽度 clamp/去重/补
  Name/固定列不足补默认；空 = 旧档，`restore_columns` 用 legacy
  fm_col_size_width/mtime 播种默认三列；`serialize_columns` 写回，同时
  回写 fm_col_size_width/mtime 兼容旧读者）；`fm_sort_key` 白名单加
  "unsorted"/"attr"、`fm_view_mode` 加 "brief"。**标签快照**：
  `PanelTabSnapshot` 加 `sort_key`/`sort_asc`/`view_mode`/`columns`
  （仅会话内——标签持久化仍只存目录；`restore_tabs` 的标签继承面板
  当前值防重置，空 columns = restore_tab 不覆盖）；`new_tab` 继承当前
  排序/列/模式（选中/过滤/焦点仍不带入）。
  **文件注释（阶段 W）**：`views/fm_comments.rs` 读写 descript.ion
  （TC 惯例：每行 `文件名 注释`，名含空白用双引号包裹）。纯函数
  `parse_comments`（保序、同名后者覆盖、空注释/畸形行跳过）/
  `format_comments`（引号名、注释内换行与连续空白折叠为单空格——
  行格式不支持多行）；`read_comments` 经 parser `decode_text_guess`
  探测编码（UTF-8 BOM 手动剥前缀；GBK 等 ANSI 经 chardetng）；
  `write_comment(dir, name, Option)`（None/空 = 删除该条；保序重写、
  新条目追加尾部、条目清空删文件、UTF-8 无 BOM 写出）。**接线**：
  `FsPanel.comments: HashMap<String,String>` 在列举就绪（poll → Ready）
  时读入，`reload_comments()` 供编辑写回后重读（FS watch 对
  descript.ion 的变更经既有 refresh 链路天然重读）；Comment 列直绘与
  行悬停 tooltip（`row_hover_tip` 第三参数，有注释追加「注释：」行）
  按**文件名**匹配（分支视图子目录项 rel_dir 非空时注释在其各自目录，
  不递归读，编辑入口同步禁用）。编辑入口 = 右键「编辑注释…」+
  Ctrl+Shift+Z（**Ctrl+Z 预留阶段 AG 撤销，勿占**）；`CommentDialog`
  非模态 egui::Window（同 SaveSelectionDialog 模式），多行输入预填、
  Ctrl+Enter/确定写回（空 = 删除），失败保持打开显示错误。
  **命令行输入条（阶段 X）**：`fm_command_bar`（默认开，设置页「文件管理器」
  tab 开关）——FM 底部、状态栏上方（`egui::Panel::bottom("fm_command_bar")`
  在状态栏 panel 之后注册故叠在其上），弱色 `{dir}>` 前缀 + 单行
  TextEdit。Enter 执行：`spawn_shell_command`（Windows `cmd /c` / 其余
  `sh -c`，current_dir = 焦点栏目录，后台 spawn 不等待不捕获输出），失败
  → `intents.op_error`，成功清空并 `push_history_capped` 入会话内历史
  （去重置顶，`COMMAND_HISTORY_CAP` = 32，不落盘）；聚焦时 ↑/↓ 回填历史
  （↓ 越过最新回空白），Esc 清空并交还焦点。快捷键（`handle_keyboard` 内、
  `egui_wants_keyboard_input` 检查**之前**，同 Ctrl+S 先例；均以
  `options.command_bar` 门控）：Ctrl+P 追加焦点栏当前路径、Ctrl+Enter 追加
  焦点项文件名（Ctrl+Shift+Enter 已是 runas，纯 Ctrl+Enter 空闲）。
  **egui 0.35 焦点锁滤波三坑**（命令行 TextEdit 全踩过）：① 单行框默认
  `return_key = Some(Enter)` 会在 Enter 时 surrender 焦点——必须
  `.return_key(None)` 由命令行自处理 Enter；② 焦点锁滤波
  （`set_focus_lock_filter`，TextEdit 聚焦后自设方向键归输入框）只在
  「上帧与本帧都聚焦」时生效，而 `request_focus` 会重建 FocusWidget 把
  滤波重置回默认——已聚焦时**不得**重复 request_focus，否则下一帧
  ↑/↓ 被 egui 当焦点漫游键把焦点移走（`Focus::begin_pass` 对未匹配滤波的
  裸方向键置 focus_direction，end_pass 移焦）；③ Esc 在帧首
  （begin_pass）就按滤波（TextEdit 的 `EventFilter.escape = false`）收走
  焦点——渲染期 `has_focus()` 已为 false，Esc 分支必须认
  `lost_focus() || has_focus()`（过滤框的 Esc 分支同此问题，其清空语义
  实际由 lost_focus 记录历史兜底，留意勿按旧注释理解）。执行分支还须排
  修饰键（`!command && !alt && !shift`）：Ctrl+Enter 粘贴文件名与执行同帧，
  handle_keyboard 在命令行渲染之后跑，不排则边粘贴边执行。
  `command_esc_handled` 帧内标记（同 filter_esc_handled 模式，当帧被
  handle_keyboard 的 Esc 链消费清零）。**顺带修复**：`comment_dialog`
  补进 handle_keyboard 的对话框屏蔽列表（阶段 W 遗漏）；测试基座
  `headless_frame` 现在从注入的 Key 事件回填 `RawInput.modifiers`
  （egui 不从事件推导修饰键状态，不带修饰键的快捷键测试此前无法模拟）。
  **自定义按钮栏（阶段 Y）**：`fm_button_bar: Vec<FmButton>`（label/
  command/tooltip；clamp 去空白、label/command 缺一则移除）——顶栏下方
  一条按钮条，**列表为空不渲染不占垂直空间**（空栏的「+ 添加按钮」占位
  提示在顶栏「搜索」后，非空时不再重复）。点击经 `spawn_shell_command`
  同命令行条后台执行（current_dir = 焦点栏目录，失败 → op_error）；
  悬停显示 tooltip（空 = 展开后的命令）；命令占位符 `%P` = 焦点栏目录、
  `%N` = 焦点项名称（无焦点 = 空串）、`%p` = 另一栏目录，展开为纯函数
  `expand_button_command`（无转义、未知占位符原样保留、大小写敏感）。
  右键按钮弹 egui `context_menu`「编辑…/删除」；`ButtonDialog`
  （非模态 egui::Window，BookmarkGroupDialog 模式扩展为三字段，已进
  handle_keyboard 对话框屏蔽列表）。**写回路径**同 fm_bookmark_groups：
  视图持 `button_bar` 副本（`set_button_bar` 注入/同步）、快照
  `FmStateSnapshot.button_bar` 经 maybe_save_fm_state diff 写回；设置页
  「文件管理器」tab「按钮栏」子区块（列表 ↑↓/✕ + 三输入添加表单，
  `file_manager_ui` 因此从静态方法改为 `&mut self`）直接改 settings，
  render_settings 前后 diff 后经 `set_button_bar` 同步休眠视图（同
  apply_layout_settings 先例，防快照写回覆盖）。
  **拖放/剪贴板/watch 收尾（阶段 Z）**：① **拖到面包屑段/标签复制**——
  `render_breadcrumb`/`render_tab_bar` 渲染时把「目录 → rect」记进
  `breadcrumb_drop_rects`/`tab_drop_rects`（帧首 clear，同 panel_drop_rects
  模式；标签只记非当前标签）；`poll_inter_panel_dnd` 在 Dual 早退**之前**
  遍历两列表做落点判定（单栏也接收），跳过源栏当前目录（当前段/当前标签
  天然命中），悬停淡底+选中色描边高亮，松开按 `fm_drag_confirm` 弹
  CopyMove 框或直拷自动改名（此落点不做 Shift 移动，栏体落点才支持）。
  ② **拖出 MOVE 语义**：`do_drag_drop(files, allow_move) -> Result<bool,
  String>`（Ok(true) = 落点实际执行了 MOVE；DRAGDROP_S_CANCEL 归一为
  Ok(false)）；FM `poll_drag_out_external` 在进 OLE 模态**前**快照 Shift
  （模态内 egui 输入不更新），Shift = allow_move，返回 MOVE 且成功 → 源经
  `start_delete(sources, false)` 走既有回收站 Delete 任务；Archive 视图
  调用点维持 COPY-only（传 false）。③ **macOS 剪贴板文件列表**：
  `platform/macos/clipboard_files.rs`（raw `msg_send!` + `#[link(Cocoa)]`，
  不引 objc2-app-kit——树内 objc2-foundation 0.2 与 objc2 0.6 不兼容；
  **本机只编 Windows，该文件未经本地编译验证，依赖 CI/真机**）：
  `set_files` 经 generalPasteboard `writeObjects:` NSURL 数组（cut 语义
  忽略，恒复制），`get_files` 经 `readObjectsForClasses:` 读回且
  is_cut 恒 false；clipboard stub 的 cfg 收窄为 `not(any(windows, macos))`。
  ④ **`fm_watch_recursive`**（默认 false）：`FsPanel.watch_recursive` 经
  `FmBehaviorOptions` 每帧下发，`try_watch` 按标志选 RecursiveMode；
  `ensure_watch` 跳过条件含 `w.recursive == self.watch_recursive`——选项
  变化自动重建 watcher。大目录/网络盘递归监听开销大，设置页 hint 已注明。
  **任务队列 + 错误汇总（阶段 AA）**：**任务面板**——顶栏「任务」按钮
  （`icons::LIST_CHECKS`，无任务置灰）与点击状态栏进度区（ProgressBar
  `interact(Sense::click())`）开关非模态窗口 `render_task_panel`：行 =
  动词图标（Copy/Move/Delete/Compress → COPY/ARROW_RIGHT/TRASH/FILE_ZIP）
  + 源→目标摘要（首项 + 共 N 项）+ 进度条（排队任务显示「排队中」）+
  暂停/继续（排队禁用；压缩自阶段 AB 起支持）+ 取消（排队任务经
  `ops.cancel` 直接出队）。
  状态栏进度区仍只显示第一个在途任务，其余经后缀 `（+N 在途 +M 排队）`
  提示。**错误汇总窗**——`on_op_finished` 见 errors 非空即留存
  `OpErrorReport`（kind/dest/dest_dir/conflict/delete_permanent/errors
  全集；新报告覆盖旧窗），`render_op_error_report` 非模态窗虚拟化
  （`show_rows`）列出全部失败项（路径 + 原因），toast 前 3 项不变；
  「重试失败项」经 `retry_sources` 重建同参数任务（Copy/Move 用原
  dest_dir 与冲突策略、Delete 用原 permanent 档、Compress 兜底原
  dest_zip；源全消失则无可重试直接关窗；重试**不带** verify/filter
  高级选项，代码注释已注明）。两窗无文本输入，不进
  handle_keyboard 对话框屏蔽列表。
  **复制校验/过滤/压缩暂停（阶段 AB）**：`CopyOptions{verify,
  filter_pattern, filter_newer_days}` 随 CopyMoveDialog 下发（「复制完成后
  校验」checkbox 默认关 +「高级」折叠区两项过滤；`FmDialogOutcome::
  ConfirmCopyMove` 增带 opts，重试失败项用默认 opts）。过滤经纯函数
  `copy_filter_matches`（通配符 + 仅最近 N 天，预扫描 `count_source` 与
  执行共用同一判定——目录恒保留只过滤文件，total 按过滤后集合）。
  verify 在复制完成后经 `files_identical`（长度快查 + 分块比对 + cancel，
  第二轮进度）逐文件比对，不一致记 errors「校验失败」。
  **过滤 + Move 语义**（spec 未覆盖，自定）：filter 开启时 rename 快速
  路径禁用；`ctx.move_prune`（kind==Move && filter_active）在
  copy_recursive 内逐文件 trash 已拷源文件 + remove_dir 清空空目录（TC 式
  部分搬运）；`ctx.drop_source_unsafe`（过滤跳过/校验失败/复制 IO 失败/
  删源失败时置位）阻止整删源、改记 errors——顺带修了预存 bug：非过滤
  Move 回退路径在逐文件复制失败时原本仍会整删源。**压缩暂停**：
  parser `create_zip` 增 `paused: Option<Arc<AtomicBool>>`（None 兼容；
  `copy_entry_chunks` 256KB 分块取代 io::copy，块/条目边界 200ms 轮询
  paused、等待中仍查 cancel），FM 侧 run_compress 传 Some(paused)，
  状态栏/任务面板的压缩暂停禁用随之解除。**删除字节进度**：Delete 循环
  早已按 source 粒度累加 done_bytes（trash::delete 无法更细），无需改动。
  **校验和 + 属性/时间戳（阶段 AC）**：右键「校验和…」开
  `ChecksumDialog`（`file_manager_checksum.rs`，非模态 egui::Window，
  search 同款 worker 模式；选中集恰为单个 .md5/.sfv/.sha1/.sha256 直接进
  验证模式）。计算 = 后台逐文件 256KB 分块单次过同算 CRC32/SHA-1/SHA-256
  （`HashAccumulator`；crc32fast/sha1/sha2 均已在依赖树，openitgo-app
  显式声明复用；**MD5 无依赖不引入**，校验文件中 md5 条目标「不支持」），
  算法经 ComboBox 切换、点击行复制 hex；验证 = `parse_checksum_file`
  （md5sum 族按 hash 长度 32/40/64 分 md5/sha1/sha256，sfv 取末位 8 位
  hex token，`;`/`#` 注释行跳过）+ 逐条重算标 ✓/✗/缺失/不支持。
  右键「修改属性/时间戳…」开 `AttrTimestampDialog`
  （`file_manager_attr.rs`，comment_dialog 同款 Option 模式；对话框不做
  IO，`AttrAction::Apply` 由 `render_attr_dialog` 经 `apply_to_path`
  逐项应用——**时间戳先于属性位写入**（只读文件 SetFileTime 被拒），
  失败回传 error 保持打开，全成功刷新涉及栏）。属性位读写走
  `platform::file_attr`（Windows `GetFileAttributesW`/`SetFileAttributesW`
  仅改 READONLY/HIDDEN/SYSTEM/ARCHIVE 四位；unix stub 仅 readonly 经
  `fs::set_permissions`）；时间文本 `YYYY-MM-DD HH:MM:SS` 本地时区解析
  （`parse_datetime_local`/`format_datetime_local`，time crate
  local-offset，取不到回退 UTC），写入走 `filetime`（已在依赖树 ← tar）。
  时间戳/属性位改动**无独立 settings**（即时生效不落盘）。
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
