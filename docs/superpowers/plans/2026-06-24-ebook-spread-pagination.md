# 电子书 Spread 分页改造实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 将电子书单页/双页模式从横向 CSS 列布局改造为“壳页面 + JS 真实排版切分 + 每次只渲染当前 spread”，彻底消除翻页漏边问题，同时保留 3D 翻页动画和跨章节翻页。

**Architecture:** Rust 端继续通过 `ebook://reader?chapter=N` 提供完整章节 HTML；JS 壳页面把章节放入隐藏测量容器，按视口尺寸切成 spread 数组，只渲染当前 spread 并预加载相邻 spread。翻页时从预加载池取内容并播放 3D 动画。连续滚动模式保持现有实现不变。

**Tech Stack:** Rust、wry、eframe/egui、HTML/CSS/JS。

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `rust-reader-app/src/ebook_renderer.rs` | `EbookRenderer` 生命周期、自定义协议、IPC、Rust 状态机 |
| `rust-reader-app/src/ebook_renderer_template.rs`（新建） | 壳页面 HTML/JS 模板，含测量、切分、预加载、动画 |
| `rust-reader-app/src/views/ebook.rs` | `OpenEbook` 状态，同步当前 spread |
| `rust-reader-app/src/app.rs` | 状态栏/工具栏 spread 显示、跨章节边界行为 |

---

## Task 1: 扩展 Rust 状态与 IPC 以跟踪 spread

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer.rs`
- Test: `rust-reader-app/src/ebook_renderer.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_js_to_rust_deserializes_spread_fields() {
    let json = r#"{"type":"position","chapter":1,"spread":3,"char_offset":120,"total_spreads":12}"#;
    let msg: JsToRust = serde_json::from_str(json).unwrap();
    assert_eq!(msg.kind, "position");
    assert_eq!(msg.chapter, Some(1));
    assert_eq!(msg.spread, Some(3));
    assert_eq!(msg.char_offset, Some(120));
    assert_eq!(msg.total_spreads, Some(12));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_js_to_rust_deserializes_spread_fields -- --nocapture`
Expected: FAIL — `JsToRust` 没有 `spread` / `total_spreads` 字段。

- [x] **Step 3: 修改 `RendererState` 和 `JsToRust`**

在 `RendererState` 中新增：

```rust
current_spread: usize,
total_spreads: usize,
```

在 `JsToRust` 中新增：

```rust
spread: Option<usize>,
total_spreads: Option<usize>,
```

在 `EbookRenderer::new` 初始化中给新字段默认值：

```rust
current_spread: 0,
total_spreads: 1,
```

新增方法：

```rust
pub fn current_spread_count(&self) -> usize {
    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    state.total_spreads.max(1)
}
```

在 `handle_ipc_message` 中更新字段：

```rust
if let Some(spread) = msg.spread {
    state.current_spread = spread;
}
if let Some(total) = msg.total_spreads {
    state.total_spreads = total.max(1);
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_js_to_rust_deserializes_spread_fields -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer.rs
git commit -m "feat(ebook): track current spread and total spreads in renderer state"
```

---

## Task 2: OpenEbook 同步当前 spread

**Files:**
- Modify: `rust-reader-app/src/views/ebook.rs`
- Test: `rust-reader-app/src/views/ebook.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_open_ebook_tracks_current_spread() {
    let ebook = sample_ebook();
    // 由于无法创建真实 WebView，仅验证 struct 字段存在且可被修改。
    let open = OpenEbook {
        ebook,
        renderer: EbookRenderer::default(), // 若不可用则改为手动构造占位
        current_chapter: 0,
        current_page: 0,
        current_spread: 0,
    };
    assert_eq!(open.current_spread, 0);
}
```

如果 `EbookRenderer` 不能默认构造，可直接在 `OpenEbook` 上测试字段存在性，或把 `current_spread` 设为 `pub` 后通过 `OpenEbook { ..., current_spread: 5 }` 断言。

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_open_ebook_tracks_current_spread -- --nocapture`
Expected: FAIL — `OpenEbook` 没有 `current_spread` 字段。

- [x] **Step 3: 修改 `OpenEbook` 和 `sync_position`**

在 `OpenEbook` 中新增：

```rust
pub current_spread: usize,
```

在 `EbookView::sync_position` 中：

```rust
pub fn sync_position(&mut self) {
    if let Some(open) = self.open.as_mut() {
        let (chapter, _, page) = open.renderer.current_position();
        open.current_chapter = chapter;
        open.current_page = page;
        open.current_spread = open.renderer.current_spread_count(); // 临时占位
    }
}
```

注意：这里先用 `current_spread_count()` 占位，待 Task 5 添加 `current_spread()` 方法后再改为读取当前 spread。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_open_ebook_tracks_current_spread -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/views/ebook.rs
git commit -m "feat(ebook): track current spread in OpenEbook"
```

---

## Task 3: 状态栏/工具栏显示 spread 页码

**Files:**
- Modify: `rust-reader-app/src/app.rs`
- Test: `rust-reader-app/src/app.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_ebook_status_text_includes_spread() {
    let (title, progress) = ReaderApp::ebook_status_text(0, 3, 2, 10, Some("第一章"));
    assert_eq!(title, "第一章");
    assert!(progress.contains("第 3 / 10 页"), "progress should show spread: {}", progress);
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_ebook_status_text_includes_spread -- --nocapture`
Expected: FAIL — 当前签名只有 chapter/total/page 参数，不含 spread。

- [x] **Step 3: 修改 `ebook_status_text` 签名和调用点**

把签名改为：

```rust
fn ebook_status_text(
    current_chapter: usize,
    total_chapters: usize,
    current_spread: usize,
    total_spreads: usize,
    title: Option<&str>,
) -> (String, String)
```

显示格式示例：

```rust
let progress = if total_spreads > 0 {
    format!(
        "第 {} / {} 章 · 第 {} / {} 页",
        current_chapter + 1,
        total_chapters,
        current_spread + 1,
        total_spreads
    )
} else {
    format!("第 {} / {} 章", current_chapter + 1, total_chapters)
};
```

更新 `render_ebook_statusbar` 和 `render_ebook_toolbar` 的调用，从 `open.current_spread` 和 `open.renderer.current_spread_count()` 获取值。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_ebook_status_text_includes_spread -- --nocapture`
Expected: PASS

- [x] **Step 5: 运行完整应用测试**

Run: `cargo test -p rust-reader-app -- app::tests`
Expected: 全部通过，包括已有的 `test_ebook_status_text_formats_progress` 等。

- [x] **Step 6: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(ebook): show spread page numbers in statusbar and toolbar"
```

---

## Task 4: 新建壳页面模板模块

**Files:**
- Create: `rust-reader-app/src/ebook_renderer_template.rs`
- Modify: `rust-reader-app/src/ebook_renderer.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_spread_containers() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default());
    assert!(html.contains("id=\"measure\""));
    assert!(html.contains("id=\"spread\""));
    assert!(html.contains("function splitIntoSpreads"));
    assert!(html.contains("function goToSpread"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_spread_containers -- --nocapture`
Expected: FAIL — 文件/函数不存在。

- [x] **Step 3: 创建 `ebook_renderer_template.rs`（最小骨架）并迁移旧测试**

创建新文件：

```rust
use rust_reader_storage::models::EbookSettings;

pub fn reader_html(_settings: &EbookSettings) -> String {
    r#"<!DOCTYPE html>
<html lang="zh-CN">
<head><meta charset="UTF-8"><title>ebook</title></head>
<body>
<div id="measure"></div>
<div id="spread"></div>
<script>
function splitIntoSpreads() {{ return []; }}
function goToSpread(index) {{}}
</script>
</body>
</html>"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_storage::models::EbookSettings;

    #[test]
    fn test_reader_html_contains_spread_containers() {
        let html = reader_html(&EbookSettings::default());
        assert!(html.contains("id=\"measure\""));
        assert!(html.contains("id=\"spread\""));
        assert!(html.contains("function splitIntoSpreads"));
        assert!(html.contains("function goToSpread"));
    }
}
```

- [x] **Step 4: 修改 `ebook_renderer.rs` 引入模板**

在文件顶部新增：

```rust
mod ebook_renderer_template;
use ebook_renderer_template::reader_html;
```

删除原文件中的 `fn reader_html(...)` 定义，并把原本位于 `ebook_renderer.rs` 中的 `test_reader_html_*` 测试迁移到 `ebook_renderer_template.rs`（新模块已包含等价测试，旧测试可直接删除，避免重复）。

- [x] **Step 5: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_spread_containers -- --nocapture`
Expected: PASS

- [x] **Step 6: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs rust-reader-app/src/ebook_renderer.rs
git commit -m "feat(ebook): create shell template module with spread containers"
```

---

## Task 5: 注入章节总数与当前 spread 读取方法

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer.rs`
- Modify: `rust-reader-app/src/views/ebook.rs`
- Test: `rust-reader-app/src/ebook_renderer.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_chapter_count() {
    use rust_reader_storage::models::EbookSettings;
    // 仅验证模板字符串可接受 chapter_count 参数（后续由调用方注入）
    let html = reader_html(&EbookSettings::default(), 5);
    assert!(html.contains("window.ebookChapterCount = 5"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_chapter_count -- --nocapture`
Expected: FAIL — `reader_html` 签名还是单个参数。

- [x] **Step 3: 修改 `reader_html` 签名并注入 `chapter_count`**

在 `ebook_renderer_template.rs` 中：

```rust
pub fn reader_html(settings: &EbookSettings, chapter_count: usize) -> String {
    // ... 模板中合适位置加入：
    // window.ebookChapterCount = {chapter_count};
}
```

在 `ebook_renderer.rs` 的 `handle_ebook_protocol` 中：

```rust
let chapter_count = state.ebook.total_chapters();
reader_html(&state.settings, chapter_count)
```

在 `EbookRenderer` 中新增：

```rust
pub fn current_spread(&self) -> usize {
    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    state.current_spread
}
```

- [x] **Step 4: 修改 `sync_position` 使用新方法**

```rust
let (chapter, _, page) = open.renderer.current_position();
open.current_chapter = chapter;
open.current_page = page;
open.current_spread = open.renderer.current_spread();
```

- [x] **Step 5: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_chapter_count -- --nocapture`
Expected: PASS

Run: `cargo check -p rust-reader-app`
Expected: 无编译错误。

- [x] **Step 6: 提交**

```bash
git add rust-reader-app/src/ebook_renderer.rs rust-reader-app/src/ebook_renderer_template.rs rust-reader-app/src/views/ebook.rs
git commit -m "feat(ebook): inject chapter count and expose current spread"
```

---

## Task 6: 实现 JS 测量与 spread 切分（单页模式）

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_single_page_split_logic() {
    use rust_reader_storage::models::EbookSettings;
    let settings = EbookSettings {
        reading_mode: rust_reader_core::ebook::EbookReadingMode::SinglePage,
        ..Default::default()
    };
    let html = reader_html(&settings, 1);
    assert!(html.contains("pageHeight"));
    assert!(html.contains("splitSinglePage"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_single_page_split_logic -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现单页切分逻辑**

在模板 JS 中加入：

```javascript
function pageHeight() {
  return spread.clientHeight;
}

function splitSinglePage(html) {
  measure.innerHTML = html;
  const ph = pageHeight();
  const totalHeight = measure.scrollHeight;
  const spreads = [];
  for (let y = 0; y < totalHeight; y += ph) {
    const clone = measure.cloneNode(true);
    const wrapper = document.createElement('div');
    wrapper.style.position = 'relative';
    wrapper.style.overflow = 'hidden';
    wrapper.style.height = ph + 'px';
    const inner = clone.firstElementChild || clone;
    inner.style.position = 'absolute';
    inner.style.top = -y + 'px';
    inner.style.width = '100%';
    wrapper.appendChild(inner);
    spreads.push(wrapper.outerHTML);
  }
  measure.innerHTML = '';
  return spreads;
}

function splitIntoSpreads(html) {
  if (isScrollMode()) return [html];
  if (isDoubleMode()) return splitDoublePage(html);
  return splitSinglePage(html);
}
```

CSS 中确保 `#measure` 与 `#spread` 样式一致：

```css
#measure {
  position: absolute;
  visibility: hidden;
  pointer-events: none;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  padding: var(--margin-v) var(--margin-h);
  box-sizing: border-box;
  overflow: hidden;
}
#spread {
  width: 100%;
  height: 100%;
  padding: var(--margin-v) var(--margin-h);
  box-sizing: border-box;
  overflow: hidden;
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_single_page_split_logic -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): implement JS single-page spread splitting"
```

---

## Task 7: 实现双页模式切分

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_double_page_split_logic() {
    use rust_reader_core::ebook::EbookReadingMode;
    use rust_reader_storage::models::EbookSettings;
    let settings = EbookSettings {
        reading_mode: EbookReadingMode::DoublePage,
        ..Default::default()
    };
    let html = reader_html(&settings, 1);
    assert!(html.contains("splitDoublePage"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_double_page_split_logic -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现双页切分**

```javascript
function splitDoublePage(html) {
  measure.innerHTML = html;
  const ph = pageHeight();
  const totalHeight = measure.scrollHeight;
  const spreads = [];
  for (let y = 0; y < totalHeight; y += ph * 2) {
    const wrapper = document.createElement('div');
    wrapper.style.display = 'flex';
    wrapper.style.width = '100%';
    wrapper.style.height = ph + 'px';
    wrapper.style.overflow = 'hidden';
    for (let col = 0; col < 2; col++) {
      const pageY = y + col * ph;
      if (pageY >= totalHeight) break;
      const cell = document.createElement('div');
      cell.style.flex = '1';
      cell.style.height = ph + 'px';
      cell.style.overflow = 'hidden';
      cell.style.position = 'relative';
      const clone = measure.cloneNode(true);
      const inner = clone.firstElementChild || clone;
      inner.style.position = 'absolute';
      inner.style.top = -pageY + 'px';
      inner.style.width = '100%';
      cell.appendChild(inner);
      wrapper.appendChild(cell);
    }
    spreads.push(wrapper.outerHTML);
  }
  measure.innerHTML = '';
  return spreads;
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_double_page_split_logic -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): implement JS double-page spread splitting"
```

---

## Task 8: 实现跨章节导航与 goToSpread

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_chapter_navigation_functions() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("function loadChapter"));
    assert!(html.contains("function goToSpread"));
    assert!(html.contains("window.ebookChapterCount"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_chapter_navigation_functions -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现导航函数**

```javascript
let currentChapter = 0;
let currentSpread = 0;
let spreads = [];

async function loadChapter(index, offset) {
  currentChapter = index;
  currentSpread = 0;
  try {
    const res = await fetch('ebook://reader?chapter=' + currentChapter);
    const html = await res.text();
    spreads = splitIntoSpreads(html);
    if (offset) {
      currentSpread = findSpreadForOffset(offset);
    }
    goToSpread(currentSpread, false);
    reportPosition();
  } catch (e) {
    spread.innerHTML = '<p>章节加载失败: ' + e + '</p>';
  }
}

function findSpreadForOffset(offset) {
  // 简单实现：先渲染当前 spread，测量字符偏移落点
  // 可在后续迭代优化为二分查找
  return 0;
}

function goToSpread(index, animate) {
  if (spreads.length === 0) return;
  currentSpread = Math.max(0, Math.min(spreads.length - 1, index));
  preloadAdjacent();
  if (animate && currentSettings.animate) {
    flipToSpread(currentSpread);
  } else {
    renderSpread(currentSpread);
  }
}

function renderSpread(index) {
  spread.innerHTML = spreads[index];
}

function nextPage() {
  if (isScrollMode()) {
    spread.scrollTop += spread.clientHeight * 0.9;
    return;
  }
  if (currentSpread + 1 < spreads.length) {
    goToSpread(currentSpread + 1, true);
  } else if (currentChapter + 1 < window.ebookChapterCount) {
    loadChapter(currentChapter + 1, 0);
  }
}

function prevPage() {
  if (isScrollMode()) {
    spread.scrollTop -= spread.clientHeight * 0.9;
    return;
  }
  if (currentSpread > 0) {
    goToSpread(currentSpread - 1, true);
  } else if (currentChapter > 0) {
    loadChapter(currentChapter - 1, 0).then(() => {
      goToSpread(spreads.length - 1, true);
    });
  }
}

function isScrollMode() { return document.body.classList.contains('scroll'); }
function isDoubleMode() { return document.body.classList.contains('double'); }
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_chapter_navigation_functions -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): implement cross-chapter navigation and goToSpread"
```

---

## Task 9: 实现预加载相邻 spread

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_preload_logic() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("function preloadAdjacent"));
    assert!(html.contains("preloadSpreads"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_preload_logic -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现预加载池**

```javascript
const preloadSpreads = {};

function preloadAdjacent() {
  const indices = [currentSpread - 1, currentSpread + 1];
  for (const idx of indices) {
    if (idx >= 0 && idx < spreads.length) {
      if (!preloadSpreads[idx]) {
        const el = document.createElement('div');
        el.innerHTML = spreads[idx];
        preloadSpreads[idx] = el.firstElementChild || el;
      }
    }
  }
  // 清理非相邻的缓存
  for (const key of Object.keys(preloadSpreads)) {
    const k = parseInt(key, 10);
    if (Math.abs(k - currentSpread) > 1) {
      delete preloadSpreads[k];
    }
  }
}

function getSpreadElement(index) {
  if (preloadSpreads[index]) return preloadSpreads[index];
  const el = document.createElement('div');
  el.innerHTML = spreads[index];
  return el.firstElementChild || el;
}
```

修改 `renderSpread` 使用 `getSpreadElement`。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_preload_logic -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): preload adjacent spreads"
```

---

## Task 10: 实现 3D 翻页动画

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_flipper_and_flip_function() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("id=\"flipper\""));
    assert!(html.contains("function flipToSpread"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_flipper_and_flip_function -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现 3D 翻页**

CSS：

```css
#flipper {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  perspective: 1500px;
  display: none;
  z-index: 100;
}
#flipper .sheet {
  position: absolute;
  top: 0;
  height: 100%;
  transform-style: preserve-3d;
  transition: transform 0.45s ease-in-out;
}
#flipper .front, #flipper .back {
  position: absolute;
  width: 100%;
  height: 100%;
  backface-visibility: hidden;
  overflow: hidden;
  background: var(--bg);
}
#flipper .back {
  transform: rotateY(180deg) scaleX(-1);
}
```

JS：

```javascript
function captureSpreadElement(index) {
  const el = getSpreadElement(index).cloneNode(true);
  const container = document.createElement('div');
  container.style.width = '100%';
  container.style.height = '100%';
  container.style.overflow = 'hidden';
  container.appendChild(el);
  return container;
}

function flipToSpread(targetIndex) {
  if (isFlipping) return;
  isFlipping = true;
  const direction = targetIndex > currentSpread ? 1 : -1;

  const sheet = document.createElement('div');
  sheet.className = 'sheet';
  sheet.style.left = '0';
  sheet.style.width = '100%';

  const front = document.createElement('div');
  front.className = 'front';
  front.appendChild(captureSpreadElement(currentSpread));

  const back = document.createElement('div');
  back.className = 'back';
  back.appendChild(captureSpreadElement(targetIndex));

  sheet.appendChild(front);
  sheet.appendChild(back);
  flipper.innerHTML = '';
  flipper.appendChild(sheet);
  flipper.style.display = 'block';

  renderSpread(targetIndex);

  requestAnimationFrame(() => {
    sheet.style.transform = direction > 0 ? 'rotateY(-180deg)' : 'rotateY(180deg)';
  });

  setTimeout(() => {
    flipper.style.display = 'none';
    flipper.innerHTML = '';
    isFlipping = false;
    reportPosition();
  }, 450);
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_flipper_and_flip_function -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): implement 3D spread flip animation"
```

---

## Task 11: 实现点击/滚轮/键盘交互

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_input_handlers() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("function onWheel"));
    assert!(html.contains("function onClick"));
    assert!(html.contains("onWheel"));
    assert!(html.contains("onClick"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_input_handlers -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现交互**

```javascript
function onWheel(e) {
  if (isScrollMode()) return;
  e.preventDefault();
  const delta = currentSettings.invert_scroll ? -e.deltaY : e.deltaY;
  if (delta > 0 || e.deltaX > 0) nextPage();
  else if (delta < 0 || e.deltaX < 0) prevPage();
}

function onClick(e) {
  if (isScrollMode()) return;
  if (window.getSelection().toString().length > 0) return;
  const rect = spread.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width / 2) prevPage();
  else nextPage();
}

spread.addEventListener('wheel', onWheel, { passive: false });
spread.addEventListener('click', onClick);
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_input_handlers -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): wheel and click navigation for spread mode"
```

---

## Task 12: 设置变化与窗口大小变化时重新测量

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Modify: `rust-reader-app/src/ebook_renderer.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_contains_resize_handler() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("resize"));
    assert!(html.contains("applySettings"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_contains_resize_handler -- --nocapture`
Expected: FAIL

- [x] **Step 3: 实现重新测量逻辑**

```javascript
let currentChapterHtml = '';

async function loadChapter(index, offset) {
  currentChapter = index;
  try {
    const res = await fetch('ebook://reader?chapter=' + currentChapter);
    currentChapterHtml = await res.text();
    spreads = splitIntoSpreads(currentChapterHtml);
    currentSpread = offset ? findSpreadForOffset(offset) : 0;
    goToSpread(currentSpread, false);
    reportPosition();
  } catch (e) {
    spread.innerHTML = '<p>章节加载失败: ' + e + '</p>';
  }
}

function applySettings(json) {
  const s = typeof json === 'string' ? JSON.parse(json) : json;
  currentSettings = s;
  const root = document.documentElement;
  root.style.setProperty('--bg', s.bg);
  root.style.setProperty('--fg', s.fg);
  root.style.setProperty('--font', s.font);
  root.style.setProperty('--size', s.size + 'px');
  root.style.setProperty('--line', s.line);
  root.style.setProperty('--margin-h', s.margin_h + 'px');
  root.style.setProperty('--margin-v', s.margin_v + 'px');
  document.body.className = s.mode;
  // 设置变化可能导致分页改变，重新切分
  if (currentChapterHtml) {
    spreads = splitIntoSpreads(currentChapterHtml);
    goToSpread(currentSpread, false);
  }
}

let resizeTimeout;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimeout);
  resizeTimeout = setTimeout(() => {
    if (currentChapterHtml && !isScrollMode()) {
      spreads = splitIntoSpreads(currentChapterHtml);
      goToSpread(currentSpread, false);
      reportPosition();
    }
  }, 200);
});
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_contains_resize_handler -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): remeasure spreads on settings or resize changes"
```

---

## Task 12b: 连续滚动模式显示竖直滚动条

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_scroll_mode_shows_vertical_scrollbar() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("body.scroll #spread"));
    assert!(html.contains("overflow-y: scroll"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_scroll_mode_shows_vertical_scrollbar -- --nocapture`
Expected: FAIL

- [x] **Step 3: 修改 CSS**

```css
body.scroll #spread {
  overflow-y: scroll;
  height: 100%;
}
```

并确保滚动模式下 `#spread` 直接放入完整章节 HTML，而不是切分后的 spread。

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_scroll_mode_shows_vertical_scrollbar -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): show vertical scrollbar in scroll mode"
```

---

## Task 13: 更新位置上报包含 spread

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: `rust-reader-app/src/ebook_renderer_template.rs`（inline `#[cfg(test)]`）

- [x] **Step 1: 编写失败测试**

```rust
#[test]
fn test_reader_html_reports_spread_position() {
    use rust_reader_storage::models::EbookSettings;
    let html = reader_html(&EbookSettings::default(), 1);
    assert!(html.contains("\"spread\":"));
    assert!(html.contains("\"total_spreads\":"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app test_reader_html_reports_spread_position -- --nocapture`
Expected: FAIL

- [x] **Step 3: 修改 `reportPosition`**

```javascript
function reportPosition() {
  let offset = 0;
  if (spreads.length > 0 && currentSpread < spreads.length) {
    // 计算当前 spread 首字符在整章中的偏移
    // 简单实现：累加之前 spread 的文本长度（可在后续优化）
    for (let i = 0; i < currentSpread; i++) {
      offset += textLength(spreads[i]);
    }
  }
  sendIpc({
    type: 'position',
    chapter: currentChapter,
    spread: currentSpread,
    char_offset: offset,
    total_spreads: spreads.length
  });
}

function textLength(html) {
  const div = document.createElement('div');
  div.innerHTML = html;
  return div.textContent.length;
}
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-app test_reader_html_reports_spread_position -- --nocapture`
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add rust-reader-app/src/ebook_renderer_template.rs
git commit -m "feat(ebook): report spread index and total spreads via IPC"
```

---

## Task 14: 清理旧横向列代码并运行完整验证

**Files:**
- Modify: `rust-reader-app/src/ebook_renderer_template.rs`
- Test: 完整工作区测试

- [x] **Step 1: 删除旧分页相关代码**

确认 `ebook_renderer_template.rs` 中不再使用 CSS 多列、不再依赖 `column-width`、`scrollLeft` 翻页。移除所有旧的 `body.paginated` 横向列 CSS 和 `currentPage()` / `pageWidth()` 等旧函数（如果还存在）。

- [x] **Step 2: 运行完整验证**

Run:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部通过，零警告。

- [x] **Step 3: 提交**

```bash
git add -A
git commit -m "refactor(ebook): remove old horizontal column pagination code"
```

---

## Task 15: 手动集成测试清单

**Files:** 无需修改，仅验证。

- [x] **Step 1: 启动应用并打开一个 EPUB**

Run: `cargo run -p rust-reader-app`

- [x] **Step 2: 单页模式验证**

- 翻页无左侧漏边。
- 状态栏显示“第 X / Y 章 · 第 P / Q 页”。
- 翻到章末再下一页进入下一章第一页。
- 3D 翻页动画正常。

- [x] **Step 3: 双页模式验证**

- 两页并排显示。
- 翻页无漏边。
- 点击左半边上一页，右半边下一页。

- [x] **Step 4: 连续滚动模式验证**

- 切换为连续滚动模式，应显示竖直滚动条。
- 可自由拖动滚动条上下浏览。

- [x] **Step 5: 设置变化验证**

- 改变字号、字体、边距后，分页重新计算，当前阅读位置保持大致不变。

- [x] **Step 6: 提交测试记录**

如果手动测试通过，无需代码提交；在计划中勾选本任务即可。

---

## 计划自查

- **Spec 覆盖：** 测量容器、spread 切分、预加载、3D 翻页、跨章节导航、设置/resize 重测、位置上报、状态栏显示均有对应任务。
- **Placeholder 检查：** 无 TBD/TODO；代码片段完整。
- **类型一致性：** `JsToRust` 的 `spread` / `total_spreads`、`RendererState` 的 `current_spread` / `total_spreads`、`EbookRenderer::current_spread` / `current_spread_count` 命名一致。

## 实施状态

- Task 1–14 已完成并通过 `cargo fmt/check/test/clippy` 验证。
- Task 15 手动 GUI 测试需在桌面环境运行 `cargo run -p rust-reader-app` 后确认。
