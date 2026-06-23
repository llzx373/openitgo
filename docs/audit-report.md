# rustReader 漫画阅读器审计与改进计划

> **状态：已归档。** 本文档是 2026-06-17 左右的全量审计快照。P0/P1/P2/P3 列出的绝大多数问题已经实现并验证，当前代码状态请见 `TODO.md` 与 `CHANGELOG.md`。以下清单保留历史上下文，但“当前状态”列可能已过时。

## 一、审计方法

- 代码层面：使用多个 read-only explore subagent 对 `rust-reader-app`、`rust-reader-core`、`rust-reader-parser`、`rust-reader-storage` 四个 crate 进行分模块审计。
- 运行层面：`cargo check`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 当前全部通过。
- 交互层面：以“作为一款本地漫画阅读器，用户从打开应用到读完一话”的完整动线进行走查。

## 二、总体结论

rustReader 已经是一款**功能基本可用、加载管线成熟、macOS 平台优化到位**的漫画阅读器。P0/P1 阶段的核心需求（缩略图优先管线、缓存预算管理、方向性感知预读、双页模式、键盘快捷键、错误占位图等）都已经落地。

当前最大的体验缺口集中在：

1. **首页/书架没有缩略图**，只有纯文字列表，用户难以快速识别漫画。
2. **Webtoon 模式只是“单页+滚轮翻页”**，没有真正的连续滚动。
3. **`FitMode` 在阅读器 UI 中没有真正落地**，缩放/适配状态与设置不一致。
4. **PDF/RAR 没有 IO/解析缓存**，大文件重复读取开销高。
5. **项目文档与工程化**（LICENSE、CI、AGENTS.md）还不完善。

下面给出详细的问题清单和可执行的改进计划。

---

## 三、当前已实现能力（简要）

- 支持 ZIP/CBR/PDF/文件夹 四种漫画来源。
- 书架、历史、书签三个 tab，支持搜索、排序、编辑标题、删除。
- 阅读器支持 Ltr/Rtl/Webtoon、单页/双页、多种适配模式、缩放/平移、翻页动画。
- 缩略图优先 + 全图预读的多级缓存管线；缓存预算动态控制；方向性感知预读。
- macOS ImageIO 快速缩略图、DXT5 压缩、GPU(Metal) 渲染。
- 设置持久化、快捷键自定义、全屏自动隐藏工具栏。

---

## 四、详细问题清单

### 4.1 首页 / 书架 / 历史 / 书签 UI

| # | 问题 | 当前状态 | 影响 | 建议文件 |
|---|------|----------|------|----------|
| 1 | **书架没有漫画缩略图** | `LibraryEntry.cover_path` 字段始终为 `None`，UI 只显示标题文本 | 用户无法快速识别漫画 | `rust-reader-app/src/views/library.rs`、`rust-reader-storage/src/models.rs` |
| 2 | **没有封面生成逻辑** | 添加漫画/文件夹时不会提取第一页生成封面 | 导致问题 1 无法自愈 | `rust-reader-app/src/app.rs` 的添加流程 |
| 3 | **搜索结果无缩略图/元数据预览** | 搜索只是按标题过滤，结果仍是文字行 | 搜索体验差 | `rust-reader-app/src/views/library.rs` |
| 4 | **历史/书签列表无缩略图** | 仅显示标题、页码、时间 | 难以定位想继续看的漫画 | `rust-reader-app/src/views/library.rs` |
| 5 | **书架不显示页数/阅读进度** | 没有“读到第 X 页 / 共 Y 页”或进度条 | 用户无法判断进度 | `rust-reader-app/src/views/library.rs`、`rust-reader-storage/src/models.rs` |
| 6 | **空书架缺少引导** | 空状态只显示“没有匹配的漫画”，没有明显的“打开文件夹”按钮 | 首次使用迷茫 | `rust-reader-app/src/views/library.rs` |
| 7 | **书架无右键菜单** | 只有阅读器页面有右键菜单 | 删除/编辑/打开操作不便捷 | `rust-reader-app/src/views/library.rs` |
| 8 | **不支持递归扫描** | 只能添加单个文件夹，不能批量导入根目录下的多个漫画 | 整理好的漫画库导入困难 | `rust-reader-app/src/app.rs` |
| 9 | **书签 note 不可编辑** | `Bookmark.note` 字段存在，但添加时永远为 `None`，UI 只读 | 书签功能不完整 | `rust-reader-app/src/views/library.rs` |
| 10 | **历史无法单条删除/清空** | 历史列表只有“继续阅读”，没有删除或清空入口 | 隐私/整理困难 | `rust-reader-app/src/views/library.rs` |
| 11 | **`LibraryEntry.comic_id` 生成不一致** | `ensure_in_library` 用目录名 stem，`add_folder_to_library` 用 parser 生成的 id，同名目录会冲突 | 数据持久化可能串号 | `rust-reader-app/src/app.rs` |
| 12 | **主题设置未生效** | `Settings.theme` 已存储，但代码中没有调用 `ctx.set_visuals` 切换 egui 主题 | 设置项无效 | `rust-reader-app/src/app.rs` 或 `rust-reader-app/src/views/settings.rs` |
| 13 | **设置里没有工具栏/状态栏开关** | 字段存在，但只在阅读器工具栏 × 按钮和右键菜单里切换 | 设置中心不统一 | `rust-reader-app/src/views/settings.rs` |

### 4.2 阅读器核心体验

| # | 问题 | 当前状态 | 影响 | 建议文件 |
|---|------|----------|------|----------|
| 14 | **Webtoon 不是连续滚动** | `layout.rs` 有垂直布局计算，但 `reader.rs` 仍用左右双页居中逻辑，滚轮映射为 page_up/down | Webtoon 阅读体验差 | `rust-reader-app/src/views/reader.rs`、`rust-reader-core/src/layout.rs` |
| 15 | **双页 spread 处理简单** | 只是把 `current` 和 `current+1` 拼一起，没有跨页/宽页检测 | 遇到跨页图会显示错误 | `rust-reader-app/src/views/reader.rs`、`rust-reader-core/src/layout.rs` |
| 16 | **`FitMode` 与 `QuickFit` 重复** | `ReadingState` 保存 `fit_mode`，但 `ReaderView` 只使用自己的 `pending_fit = QuickFit::Page`，设置里的 `default_fit` 不生效 | 缩放/适配行为不一致 | `rust-reader-app/src/views/reader.rs`、`rust-reader-core/src/state.rs` |
| 17 | **缩放交互单一** | 只有工具栏 +/-；缺少滚轮/Ctrl+滚轮/捏合缩放、双击 100% | 操作不自然 | `rust-reader-app/src/views/reader.rs` |
| 18 | **窗口大小变化不自动 fit** | 调整后需要手动点适应按钮 |  resized 后图片可能溢出或留空 | `rust-reader-app/src/views/reader.rs` |
| 19 | **平移边界粗糙** | 小图缩放后可能完全拖出视口 | 用户体验差 | `rust-reader-app/src/views/reader.rs` |
| 20 | **动画与当前 zoom/pan 脱节** | 动画固定按 `available` 做 Page fit，忽略用户当前缩放 | 翻页跳跃 | `rust-reader-app/src/views/reader.rs` |
| 21 | **双页/Webtoon 无动画** | `can_animate_turn()` 直接禁用 | 翻页生硬 | `rust-reader-app/src/views/reader.rs` |
| 22 | **页面跳转输入框需要回车** | `DragValue` 修改后未在失去焦点时生效 | 用户可能误以为已跳转 | `rust-reader-app/src/views/reader.rs` |
| 23 | **进度条悬停缩略图固定 80×120** | 不保持原图比例，可能拉伸 | 预览图变形 | `rust-reader-app/src/thumbnail_progress_bar.rs` |
| 24 | **缩略图失败无提示/重试** | `reader.update` 直接忽略 thumbnail 错误 | 用户长时间看到“加载中”占位 | `rust-reader-app/src/views/reader.rs`、`rust-reader-app/src/loader.rs` |
| 25 | **错误重试无退避** | 点击占位图即立刻重发，损坏文件可能无限循环 | 可能卡死/爆日志 | `rust-reader-app/src/views/reader.rs` |
| 26 | **缺少鼠标前进/后退键翻页** | 未处理额外鼠标按钮 | 鼠标侧键用户无法使用 | `rust-reader-app/src/app.rs` |

### 4.3 加载与性能

| # | 问题 | 当前状态 | 影响 | 建议文件 |
|---|------|----------|------|----------|
| 27 | **CPU/GPU 双份内存** | `PageCache` 上传为纹理后仍保留 CPU 端 `ColorImage`，未释放 | 内存占用翻倍 | `rust-reader-app/src/cache.rs` |
| 28 | **缩略图批次未按当前页优先** | 后台缩略图从第 0 页顺序生成，大漫开头可能长时间看不到当前页 | 首屏体验差 | `rust-reader-app/src/views/reader.rs` |
| 29 | **PDF 没有文档缓存** | 每页渲染都重新 `read` + 解析整个 PDF | 大 PDF 翻页慢 | `rust-reader-app/src/loader.rs`、`rust-reader-parser/src/pdf.rs` |
| 30 | **RAR 没有索引缓存** | `read_rar_entry` 每次都线性扫描 header | 大 RAR 预读慢 | `rust-reader-app/src/loader.rs`、`rust-reader-parser/src/rar.rs` |
| 31 | **`protected_page_indices` 用 Vec** | `contains` 线性查找，页数多时影响淘汰 | 可优化为 HashSet | `rust-reader-app/src/cache.rs` |
| 32 | **`SharedRawCache` 使用 Mutex 且锁范围偏大** | 多个 IO worker 频繁竞争 | 高并发归档读取可能成为热点 | `rust-reader-app/src/loader.rs` |
| 33 | **高优先级队列容量固定 64** | 快速翻页/动画时多个可见页请求可能填满 | 新可见页请求被拒绝 | `rust-reader-app/src/loader.rs` |
| 34 | **`compress` AtomicBool 用 Relaxed** | 当前无问题，但扩展时可能引入同步隐患 | 建议改为 Acquire/Release | `rust-reader-app/src/loader.rs` |

### 4.4 数据持久化与元数据

| # | 问题 | 当前状态 | 影响 | 建议文件 |
|---|------|----------|------|----------|
| 35 | **历史只存 comic_id，漫画改名/移动后无法匹配** | 没有 path 兜底 | 历史记录可能失效 | `rust-reader-storage/src/models.rs` |
| 36 | **缺少阅读统计** | 无每次阅读时长、会话次数 | 无法支持“继续阅读”排序/阅读时长 | `rust-reader-storage/src/models.rs` |
| 37 | **设置加载错误静默 fallback** | JSON 损坏时直接恢复默认，不提示用户 | 用户不知道数据丢失 | `rust-reader-storage/src/json_store.rs` |
| 38 | **缺少设置校验** | `decode_threads`、`cache_size_mb` 等未校验范围 | 极端配置可能崩溃 | `rust-reader-storage/src/models.rs` |
| 39 | **缺少原子写/备份** | 当前 JSON 写入未使用 temp+rename，也没有备份 | 崩溃时可能损坏配置 | `rust-reader-storage/src/json_store.rs` |

### 4.5 工程与文档

| # | 问题 | 当前状态 | 影响 | 建议文件 |
|---|------|----------|------|----------|
| 40 | **缺少 LICENSE 文件** | `Cargo.toml` 声明 MIT，但仓库没有 LICENSE | 开源合规不完整 | 仓库根目录 |
| 41 | **缺少 AGENTS.md** | 没有给后续 agent/collaborator 的构建/测试/风格说明 | 协作效率低 | 仓库根目录 |
| 42 | **缺少 CI** | 无 GitHub Actions 等自动化 | 代码质量无法持续保证 | `.github/workflows/` |
| 43 | **设计文档可能过时** | `docs/` 中部分文档提到 glow、`page_view.rs` 等已不存在的实现 | 会误导新成员 | `docs/` |
| 44 | **缺少 CHANGELOG** | 最近功能迭代快，但没有变更记录 | 用户/开发者难以追踪 | `CHANGELOG.md` |
| 45 | **缺少非 GUI 集成测试** | parser、storage、loader 的并发/IO 行为缺少集成测试 | 回归风险 | 各 crate `tests/` |
| 46 | **`target/` 目录在本地占用大** | 已在 TODO 中处理过清理，但需确保不被误提交 | 仓库体积 | `.gitignore` 已配置 |

---

## 五、推荐实施路线图

### P0 — 影响基础可用性（建议优先）

1. **书架封面缩略图**
   - 在 `LibraryEntry` 添加/更新时，异步提取第一页并生成封面缩略图缓存到本地。
   - 在 `library.rs` 中以网格/卡片形式展示封面 + 标题 + 阅读进度。
2. **统一 comic_id 生成**
   - 统一使用 parser 生成的 `comic.id`（如文件路径 hash），避免同名目录冲突。
3. **主题设置生效**
   - 在 app update 中根据 `Settings.theme` 调用 `ctx.set_visuals` 切换 Dark/Light。
4. **Webtoon 真正连续滚动**
   - 让 `reader.rs` 在 Webtoon 模式下使用 `layout.rs` 的垂直布局，连续绘制多页，滚轮垂直滚动。
5. **`FitMode` 与设置打通**
   - 移除/合并 `QuickFit`，让阅读器直接使用 `ReadingState.fit_mode` 和 `settings.default_fit`。

### P1 — 显著提升体验

6. **阅读器缩放/平移增强**
   - Ctrl/Command + 滚轮缩放、双击 100%/fit、窗口 resize 自动 fit、限制 pan 边界。
7. **缩略图失败提示与重试退避**
   - 缩略图加载失败也进入 `page_errors` 或显示降级占位；错误重试最多 3 次并带指数退避。
8. **PDF/RAR 缓存**
   - PDF：在 loader 中缓存已读文件字节或解析后的文档。
   - RAR：建立 `name -> header position` 索引，避免线性扫描。
9. **书架/历史/书签右键菜单与元数据**
   - 右键打开/删除/编辑；显示页数、阅读百分比、添加时间。
10. **进度条悬停缩略图保持比例**
    - 按原图比例缩放，限制最大尺寸。
11. **空书架引导**
    - 添加大大的“打开文件夹”按钮和拖拽提示。

### P2 — 功能完善

12. **书签 note 编辑**
13. **历史单条删除/清空**
14. **递归扫描导入**
15. **跨页/宽页检测与显示选项**
16. **动画与当前 zoom/fit 状态一致，或提供关闭动画开关**
17. **页面跳转输入框失去焦点/回车即时生效**
18. **鼠标前进/后退键翻页**

### P3 — 工程精进

19. 上传纹理后释放 CPU 端 `ColorImage`。
20. `protected_page_indices` 改为 `HashSet`。
21. `SharedRawCache` 锁粒度优化。
22. 设置 JSON 原子写 + 备份 + 加载错误提示 + 范围校验。
23. 历史记录同时保存 comic_id 与 path，提高容错。
24. 添加 LICENSE、AGENTS.md、CI、CHANGELOG，清理/更新 docs。
25. 增加非 GUI 集成测试。

---

## 六、关键实现细节建议

### 6.1 书架缩略图

- 在 `rust-reader-app/src/app.rs` 的 `add_folder_to_library` / `ensure_in_library` 流程中，使用 `PageLoader::request_thumbnail` 异步获取第一页缩略图。
- 封面缓存可放在 `rust-reader-app/data/covers/<comic_id>.jpg` 或 `rust-reader-storage` 管理的 `covers/` 目录。
- `LibraryEntry.cover_path` 指向该文件；若文件不存在则在 UI 中显示占位色块并触发后台生成。
- UI 从文字 `Grid` 改为 `ScrollArea` + 卡片网格，每张卡片包含：封面、标题、进度条、悬停操作按钮。

### 6.2 Webtoon 连续滚动

- 在 `reader.rs` 中增加 `render_webtoon` 分支：
  - 用 `layout::compute_layout` 计算每页在当前 viewport 中的垂直偏移。
  - 绘制从 `scroll_offset` 上方一页到 viewport 底部下方一页的所有页面。
  - 滚轮 delta 直接累加到 `scroll_offset`，到达边界时再翻页。
  - 可见页仍走高优先级缩略图→全图管线。
- 需要新增 `ReadingState.webtoon_scroll_offset`。

### 6.3 FitMode 统一

- 将 `ReaderView` 中的 `pending_fit: QuickFit` 替换为使用 `reader.state.fit_mode`。
- 工具栏的 fit 按钮直接修改 `reader.state.fit_mode`。
- 打开新漫画时应用 `settings.default_fit`。
- 删除 `QuickFit` 类型或保留作为 `FitMode` 的别名。

### 6.4 PDF/RAR 缓存

- PDF：在 `PageLoader` 中为每个来源维护 `Arc<Mutex<PdfCache>>` 或按文件路径缓存 `Vec<u8>`。
- RAR：解析一次后缓存 `Vec<(entry_name, header_offset)>`，后续按名直接 seek。
- 注意生命周期：关闭漫画或 epoch 变化时释放缓存。

### 6.5 错误重试退避

- `page_errors` 中记录失败次数和最后失败时间。
- 重试间隔 = `min(2^n * 1s, 30s)`，最多 3 次后显示“加载失败，点击重试”。
- 缩略图失败可降级为纯文字页码占位。

---

## 七、验收标准

- [x] `cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 全部通过。
- [x] 书架以卡片/网格形式展示漫画封面缩略图。
- [x] Webtoon 模式支持连续垂直滚动，滚轮不再整页翻页。
- [x] 设置里的默认适配模式在阅读器打开时生效，缩放/平移交互增强。
- [x] PDF/RAR 大文件翻页时不再重复全量读取。
- [x] 设计文档与工程文件（LICENSE、AGENTS.md、CI）补齐。

---

**说明**：本审计清单中的问题已基本完成。后续新需求或新发现的问题请直接更新 `TODO.md` 与 `CHANGELOG.md`。
