# 电子书半成品接线与收尾（TODO 36–42）设计

> 状态：已批准（2026-07-17）。范围：TODO.md "新一轮待办" 的 P0+P1（条目 36–42），执行方式为 main 分支串行、每任务独立 commit（方案 A）。

## 背景

2026-07-17 全面检查后确认：TODO 36–60 中 P0/P1 共 7 项为本次范围。关键代码事实（已核实，修正早期假设）：

- 电子书搜索：JS 侧 `findText/findNext/findPrev/clearHighlights` 与 Rust API 均已实现，仅未接 UI（`openitgo-app/src/ebook_renderer.rs:207-237`，4 处 `#[allow(dead_code)]`）。JS 重排会丢弃高亮，模板注释约定"调用方需在布局稳定后重新执行 findText"（`ebook_renderer_template.rs:669-671`）。
- EPUB 资源：`Ebook.resources` 已含 `href` + `mime_type`（`openitgo-core/src/ebook.rs:25-30`）；协议 handler 对非章节请求一律返回空 200（`ebook_renderer.rs:325-333`），导致 `<img>` 全部裂图。
- sanitize 丢弃 `<link>`（`openitgo-parser/src/html.rs:132-134`），内嵌字体随之丢失。
- `ebook.font_family` 已持久化并传给 JS（`ebook_renderer.rs:65`），但设置面板无入口，是唯一死字段。
- TXT/Markdown 读取均用 `fs::read_to_string` 仅认 UTF-8，共 3 处调用点：`openitgo-parser/src/txt.rs:34`、`markdown.rs:28`、`html.rs:14`。
- 电子书视图快捷键已走 `settings.shortcuts.next_page/prev_page`（`app.rs:1792-1799`），仅缺 `back_to_library`（Escape）与 `page_down/page_up`。
- 测试 fixture `openitgo-parser/tests/fixtures/minimal.epub` 已存在。

## 任务分解（执行顺序即提交顺序）

### Task 36 — 更名收尾提交

先跑完整验证流水线（`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`），全绿后提交当前未提交的 3 文件改动（`openitgo-storage/src/json_store.rs`、`CHANGELOG.md`、`docs/superpowers/README.md`），保证开工前工作区干净。

### Task 41 — TXT/Markdown 编码检测

- 新增 `openitgo-parser/src/text_encoding.rs`，提供 `read_text_lossy(path) -> Result<String, ParseError>`：
  1. 读字节；UTF-8（含 BOM）直接通过；
  2. 否则 `chardetng` 检测编码，`encoding_rs` 解码（覆盖 GBK/GB18030/Big5 等，非法字节替换为 U+FFFD，不报错）。
- 替换 3 处调用点（txt.rs / markdown.rs / html.rs）。
- 新增依赖：`chardetng`、`encoding-rs`（仅 `openitgo-parser`）。

### Task 38/39 — EPUB 资源服务 + 内嵌字体（同一机制，一个 commit）

**资源通道 `ebook://res/<路径>`**：

- 协议 handler 识别 `/res/` 前缀 → percent-decode 路径 → `epub::doc::EpubDoc::get_resource_by_path` 取字节 → MIME 从 `ebook.resources` 查表，扩展名兜底；找不到返回空 200（保持现有"空 200 防 reload"约定）。
- 新增纯函数 `rewrite_epub_urls(html, chapter_dir) -> String`（`openitgo-parser/src/html.rs`）：把 `<img>`/`<image>` 的 `src` 等相对路径按章节所在目录 resolve（归一化 `..`/`.`，去前导 `/`）后改写为 `ebook://res/<percent-encoded>`；`data:`、绝对 URL（含 scheme）不改写。
- `render_chapter_html` 的 EPUB 分支：sanitize 后追加 URL 改写。

**内嵌字体（受控方案）**：

- sanitize 维持丢弃 `<link>` 整表（避免书籍 CSS 冲击 CSS columns 分页器——已知限制见 `docs/superpowers/reports/2026-06-26-css-columns-test-plan.md`）。
- 新增 `extract_font_face_css(doc) -> String`：遍历 EPUB CSS 资源，提取 `@font-face` 块，块内 `url()` 按该 CSS 文件目录 resolve 改写为 `ebook://res/...`；拼接为 inline `<style>` 注入每章 HTML（chapter div 之前）。只拿字体保真，不引入排版风险。

### Task 40 — font_family 设置 UI

- 设置面板电子书节（`openitgo-app/src/views/settings.rs`）新增字体 `ComboBox`：预设 `system-ui`、`serif`、`sans-serif`、`PingFang SC`、`Songti SC`、`Kaiti SC`、`Hiragino Sans GB`、`monospace`；当前值为自定义时并入列表保持可选。
- `Settings::validate()` 增加 `font_family` 非空校验；不引新依赖。

### Task 37 — 电子书搜索 UI

- `EbookView`（`openitgo-app/src/views/ebook.rs`）新增搜索状态：`show_search: bool`、`query: String`、`match_count: usize`、`active_index: usize`。
- 入口：工具栏放大镜按钮 + Cmd/Ctrl+F。搜索条：输入框（自动聚焦、输入即搜 `find_text`）、`n/m` 计数、上一个/下一个按钮、关闭按钮。
- 键位：Enter=`find_next`，Shift+Enter=`find_prev`，Esc=关闭并 `clear_highlights`。
- JS 模板（`ebook_renderer_template.rs`）：`setSearchActiveIndex` 内 `sendIpc({type:'search', count, active})` 回传；重排（applySettings/resize/换章）完成后若 `ebookSearchQuery` 非空自动重放 `findText`。
- Rust：`JsToRust` 增加 search 消息字段；`RendererState` 存 `search_count/search_active`；`EbookRenderer::search_state()` 暴露；移除 4 处 `#[allow(dead_code)]`。

### Task 42 — 电子书快捷键补全

- `app.rs` `View::Ebook` 分支增加：`back_to_library`（默认 Escape）→ 返回书架；`page_down`→下一页、`page_up`→上一页。沿用 `Shortcuts` 可配置体系。

## 错误处理

- 编码检测失败/未知编码：fallback UTF-8 lossy，不向用户报错（文本可读优先）。
- EPUB 资源缺失或路径非法：返回空 200 并 `eprintln!` 日志，不影响章节渲染。
- 搜索无命中：`n/m` 显示 `0/0`，上/下按钮禁用。
- 字体 `@font-face` 提取失败（无 CSS 资源）：注入空样式，行为与现状一致。

## 测试方案（随各任务提交）

- **41**：`read_text_lossy` 单测（UTF-8、UTF-8 BOM、GBK、GB18030、Big5、空文件）；`tests/ebook_integration.rs` 加 GBK fixture 集成测试。
- **38/39**：`rewrite_epub_urls` 单测（相对路径、`..` 解析、`data:` 跳过、带 scheme URL 跳过）；`@font-face` 提取与 url 改写单测；基于 `tests/fixtures/minimal.epub`（必要时重打包加入图片/字体）的资源读取集成测试；协议路由测试（章节/壳页/res/未知）。
- **40**：设置校验单测（空 `font_family` 报错、钳制）；旧 JSON 反序列化兼容测试。
- **37**：模板测试沿用现有模式（断言 JS 含 search IPC 上报与重排自动重放）；`JsToRust` search 消息解析单测；`search_state` 单测。搜索框交互走人工走查。
- **42**：ebook 分支绑定逻辑抽纯函数后单测；交互走人工走查。
- **收尾**：全流水线 → `scripts/package-macos.sh` 构建 .app → 人工走查清单（搜索 n/m 与高亮、EPUB 图片/字体渲染、字体切换生效、GBK 文本打开、Escape 返回）→ 勾选 TODO.md 36–42、更新 CHANGELOG。

## 不做（YAGNI）

- 不保留 EPUB 整份书籍 CSS（39 受控方案的明确取舍，未来可另立任务评估）。
- 不做搜索跨章节跳转、大小写/正则选项。
- 不做字体枚举系统字体列表（只用预设 + 自定义值）。
- MOBI 排版保真、电子书视图菜单被 webview 遮盖等 P2 项不在本次范围。
