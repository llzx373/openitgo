# 电子书功能后续实现清单

> **状态：已完成。** 本清单中所有任务已实现并验证，后续电子书改进请查看 `CHANGELOG.md` 与 `TODO.md`。

本文件记录 `docs/superpowers/plans/2026-06-24-ebook-reader.md` 中尚未完成、以及围绕电子书功能需要补全的测试与实现项。按依赖顺序逐步实现，每个任务完成后立即补充对应测试并运行完整验证流水线。

**验证流水线（每个任务完成后必须运行）：**

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Task 1: 目录面板（Table of Contents）

**目标**：在电子书阅读界面提供可打开的目录面板，列出所有章节标题，点击后跳转到对应章节。

**实现要点：**
- [x] 在 `EbookView` 中增加 `show_toc: bool` 状态与当前高亮章节索引。
- [x] 在 `openitgo-app/src/app.rs` 的 `render_ebook` 中增加侧边栏/弹窗形式的目录 UI。
- [x] 工具栏/菜单栏的"目录"按钮绑定到打开/关闭目录面板。
- [x] 目录项点击调用 `ebook_view.goto_chapter(index)`。
- [x] 当前阅读章节在目录中高亮显示（可基于 `renderer.current_position().0` 同步）。

**测试要求：**
- [x] 单元测试：`EbookView::goto_chapter` 更新 `current_chapter` 并调用 renderer。
- [x] 单元测试：目录高亮索引计算正确。

---

## Task 2: 电子书阅读位置持久化

**目标**：关闭应用或切换视图时保存电子书的当前章节与字符偏移，下次打开时恢复。

**实现要点：**
- [x] 扩展 `HistoryEntry` 或新增 `EbookHistoryEntry` 存储 `chapter_index` 与 `char_offset`。
- [x] 在 `record_ebook_history` 中从 `ebook_view.renderer.current_position()` 读取位置并写入历史。
- [x] 在 `poll_ebook_opener` 打开电子书时查询历史，若存在则调用 `goto_chapter(chapter, offset)` 恢复位置。
- [x] 定期同步阅读位置（翻页、切换章节时更新 `ebook_view.sync_position()`）。

**测试要求：**
- [x] 单元测试：保存历史时 `chapter_index` 与 `char_offset` 正确。
- [x] 单元测试：打开电子书时从历史恢复位置正确。
- [x] 集成测试：打开 EPUB → 跳转章节 → 关闭/重新打开 → 位置恢复。

---

## Task 3: 电子书书签

**目标**：在电子书模式下支持添加/删除/编辑书签，保存到现有书签存储中。

**实现要点：**
- [x] 扩展现有 `Bookmark` 结构以支持电子书（`media_type: MediaType`，`chapter_index`，`char_offset`）。
- [x] 在电子书工具栏/菜单栏添加"添加书签"按钮。
- [x] 在"阅读 → 书签"菜单中列出当前电子书的书签，点击跳转。
- [x] 书签弹窗/面板允许编辑 note。

**测试要求：**
- [x] 单元测试：添加电子书书签保存 `chapter_index` 与 `char_offset`。
- [x] 单元测试：按 `comic_id` / `media_type` 过滤书签。
- [x] 集成测试：添加书签 → 持久化到 JSON → 重新加载后书签存在。

---

## Task 4: 书架混排电子书

**目标**：书架中显示电子书条目，支持按类型过滤，点击电子书条目进入阅读模式。

**实现要点：**
- [x] 在 `LibraryEntry` 中增加 `media_type: MediaType` 字段（默认 `Comic`，兼容旧数据）。
- [x] 在 `JsonStore` 加载 library 时对旧条目进行 `media_type` 迁移（默认 `Comic`）。
- [x] `add_file_to_library` / `add_folder_to_library` 根据扩展名设置 `media_type`。
- [x] 为电子书生成封面（可先用纯色占位 + 标题，或提取 EPUB 封面图片）。
- [x] `LibraryView` 增加"全部 / 漫画 / 电子书"过滤 tab。
- [x] 点击电子书条目调用 `open_ebook(path)`。
- [x] 右键菜单支持打开、编辑标题、删除（与漫画一致）。

**测试要求：**
- [x] 单元测试：`media_type_for_path` 正确识别漫画与电子书。
- [x] 单元测试：旧 library JSON 反序列化时 `media_type` 默认 `Comic`。
- [x] 单元测试：LibraryView 按 `media_type` 过滤正确。
- [x] 集成测试：添加 `.epub` 到书架 → 条目 `media_type == Ebook` → 点击打开进入 `View::Ebook`。

---

## Task 5: 电子书设置校验与默认值

**目标**：确保 `EbookSettings` 在加载/保存时经过校验与钳制。

**实现要点：**
- [x] 在 `Settings::validate()` 中增加 `ebook.font_size`（10..=72）、`ebook.line_height`（1.0..=3.0）、`ebook.margin_*`（0..=200）等检查。
- [x] 在 `Settings::clamp()` 中钳制上述字段。
- [x] 非法设置通过 `error_message` 提示用户。

**测试要求：**
- [x] 单元测试：`Settings::validate` 对越界 ebook 设置返回 Err。
- [x] 单元测试：`Settings::clamp` 将越界值钳到合法范围。

---

## Task 6: 解析器测试补全

**目标**：为电子书解析器补充 fixture 与边界条件测试。

**实现要点：**
- [x] 在 `openitgo-parser/tests/fixtures/` 下放置最小 EPUB 文件。
- [x] 为 `parse_ebook` 编写 EPUB、TXT、MOBI、Markdown 集成测试。
- [x] 测试空文件、无章节标记文件、错误格式文件的解析错误。
- [x] 测试 EPUB 目录为空时回退到 spine 的行为。
- [x] 测试 TXT/Markdown 分章逻辑（按标题、按字数 fallback）。

**测试要求：**
- [x] `openitgo-parser/tests/ebook_integration.rs` 至少覆盖 4 种格式。
- [x] 边界测试：空文件返回 `NoPages`。
- [x] 边界测试：EPUB TOC 为空时使用 spine。

---

## Task 7: EbookRenderer 纯函数测试

**目标**：将 `EbookRenderer` 中可测试的逻辑提取为纯函数并覆盖。

**实现要点：**
- [x] 提取 `reader_html(settings)` 为可独立测试的纯函数（已在 `ebook_renderer.rs` 中）。
- [x] 提取协议路由判断逻辑为纯函数（输入 URI，输出请求类型）。
- [x] 测试 `JsSettings::from(&EbookSettings)` 的颜色、模式字符串映射。
- [x] 测试生成的 HTML 包含必要的 JS 函数与样式变量。

**测试要求：**
- [x] 单元测试：`reader_html` 输出包含 `loadChapter`、`applySettings`、`nextPage`、`prevPage`、`reportPosition`。
- [x] 单元测试：`JsSettings` 三种主题颜色正确。
- [x] 单元测试：`JsSettings` 三种阅读模式字符串正确。
- [x] 单元测试：协议路由正确区分壳页面、章节请求、未知请求。

---

## Task 8: 电子书状态栏完善

**目标**：在电子书阅读界面底部状态栏显示当前章节标题与阅读进度。

**实现要点：**
- [x] 实现 `render_ebook_statusbar`，显示当前章节标题、章节进度（当前章 / 总章）。
- [x] 状态栏与工具栏的显示/隐藏逻辑保持一致。
- [x] 全屏时状态栏可随鼠标悬停显示。

**测试要求：**
- [x] 单元测试：进度字符串格式化正确。

---

## Task 9: 修复 WebView 重复 reload

**目标**：解决打开 EPUB 后 WebView 重复 reload 2~3 次的问题。

**实现要点：**
- [x] 分析 EPUB 章节 HTML 是否包含 `<base>`、`<script>`、`<a>` 等导致导航的元素。
- [x] 在 `render_chapter_html` 中对 EPUB HTML 进行清理/重写，移除可能引起 reload 的脚本与链接。
- [x] 调整 `ebook://` 协议处理，拦截并忽略可疑请求。
- [x] 验证修复后只加载一次壳页面与一次章节。

**测试要求：**
- [x] 手动/日志验证：打开 EPUB 后壳页面与章节请求次数正常。

---

## Task 10: 电子书打开流程集成测试

**目标**：覆盖从文件选择到进入 `View::Ebook` 的完整流程。

**实现要点：**
- [x] 测试 `ReaderApp::open_path` 对电子书路径调用 `open_ebook`。
- [x] 测试 `poll_ebook_opener` 成功与失败分支（WebView 创建依赖真实窗口，单元测试覆盖分发逻辑）。
- [x] 测试拖拽电子书文件到窗口后的路径分发。

**测试要求：**
- [x] 单元测试：`is_ebook_file` 与 `open_path` 分发正确。
- [x] 单元测试：`poll_ebook_opener` 成功时切换到 `View::Ebook`。

---

## 完成标准

- [x] 以上所有实现任务完成。
- [x] 每个任务都有对应的单元测试或集成测试。
- [x] `cargo test --workspace` 全部通过。
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 0 警告。
- [x] 手动走查：打开 EPUB、查看目录、跳转章节、添加书签、关闭重开恢复位置、书架显示电子书。
