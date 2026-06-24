# 电子书阅读功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在当前 rustReader 桌面应用内新增电子书阅读模式，支持 EPUB、TXT 格式，使用 wry 作为渲染引擎，提供单页、双页、连续滚动三种阅读布局，并拥有独立于漫画模式的菜单栏与工具栏。

**Architecture:** 在现有 crate 内扩展：rust-reader-core 增加电子书领域模型，rust-reader-parser 增加 EPUB/TXT 解析，rust-reader-storage 增加电子书设置，rust-reader-app 增加 wry 子 webview 封装、EbookView 以及电子书专用的 APP 级菜单/工具栏。通过文件扩展名区分漫画与电子书，打开后进入 `View::Ebook`。

**Tech Stack:** wry 0.55、epub crate、quick-xml、已有 zip/eframe/egui。

---

## 实现状态摘要（2026-06-24 更新）

本计划 **Phase 1 ~ Phase 3 的核心基础设施已实现并验证通过**，Phase 4 的目录面板、历史/书签、书架混排仍待完成。

已拍板决策：
1. **书架混排**：暂时只通过"打开文件"阅读，不进入书架（C）。
2. **Linux 平台**：先接受 X11 only（A）。
3. **EPUB 渲染策略**：使用自研 HTML + 内嵌 CSS/JS（A）。
4. **TXT 分章**：按空行/`# 章节名`/`Chapter`/`第 X 章` 分章，无标记时按 3000 字虚拟章节（A）。
5. **电子书设置最小集合**：字体、字号、行高、页边距、主题、阅读模式；暂不加入翻页动画、字重、对齐方式。
6. **历史与书签**：复用现有 `History` / `Bookmarks`，把 `page_index` 解释为章节索引，字符偏移后续再扩展（A）。
7. **文件扩展名**：`.epub`、`.txt`、`.mobi`/`.azw3`/`.azw`、`.md`。

实际实现与原始计划的关键偏差：
- `EbookRenderer` 的 JS 章节加载 URL 从 `ebook://chapter/N` 改为 **`ebook://reader?chapter=N`**，Rust 协议处理程序按 **查询参数优先、壳页面兜底** 的顺序处理。
- CSS 模式类名从 `single`/`double` 改为 **`single paginated`** / **`double paginated`**，以同时匹配 `body.paginated` 与 `body.double` 选择器。
- 为防止 wry 在 macOS 上 `window.ipc` 注入时机不稳定，JS 侧增加了 **`sendIpc`** 重试包装器。
- 协议响应增加了 **`Cache-Control: no-cache`**，避免壳页面/章节被 WebKit 缓存导致 reload 异常。
- 增加环境变量 **`RUST_READER_OPEN`**，方便开发/测试时自动打开指定漫画或电子书。

已知问题：
- 打开 EPUB 后，WebView 会重复 reload 2~3 次，随后稳定。内容已能正常加载，不影响阅读，但需后续定位根因（可能与 EPUB 章节 HTML 内含的 `<base>`/脚本或 WebKit 自定义协议行为有关）。

---

## 需要你拍板的问题（先回答再继续）

1. **书架是否混排电子书与漫画？**
   - A) 同一书架，LibraryEntry 增加 `media_type` 字段区分（推荐，改动小）
   - B) 完全独立的书架/标签页，像 LibraryView 里再加一个"电子书"tab
   - C) 暂时只通过"打开文件"阅读，不进入书架

2. **Linux 平台要求？**
   - wry 的 `build_as_child` 在 Linux 仅支持 X11，不支持 Wayland。
   - A) 先接受 X11 only，后续再处理 Wayland（推荐，实现最简单）
   - B) 必须支持 Wayland，需要引入 gtk 路径（`WebViewBuilderExtUnix::new_gtk` + `gtk::Fixed`），复杂度显著增加

3. **EPUB 渲染策略？**
   - A) 用 wry 加载我们生成的单页 HTML + 内嵌 CSS/JS，自己控制分页/主题/字体（推荐，轻量、可控）
   - B) 集成 Readium 等成熟 JS 阅读引擎（功能强但集成重，本计划按 A 写）

4. **TXT 分章/分页策略？**
   - A) 按空行或 `# 章节名` 标记分章，无标记时按固定字数（如 3000 字）分虚拟章节（推荐）
   - B) 只把整个文件当一章，靠 CSS 列分页

5. **电子书设置最小集合？**
   - 必选项：字体、字号、行高、页边距、主题（白天/夜晚/sepia）、阅读模式（单页/双页/连续）。
   - 是否需要：翻页动画、字重、对齐方式、横竖屏适配？

6. **历史与书签是否复用现有结构？**
   - A) 复用 `History` / `Bookmarks`，把 `page_index` 解释为章节索引，再新增字符偏移字段（推荐，改动小）
   - B) 新建独立的 `EbookHistory` / `EbookBookmarks`

7. **文件扩展名范围？**
   - 暂定 `.epub`、`.txt`。是否需要 `.mobi` / `.azw3` / `.md`？

请回复上述问题的选项或补充说明。得到答案后，我会把计划里对应的占位决策替换为具体实现。

---

## 文件结构总览

| 文件 | 说明 |
|------|------|
| `rust-reader-core/src/ebook.rs` | 新增：电子书模型 `Ebook`、`EbookChapter`、`EbookResource`、`ReadingMode` 等 |
| `rust-reader-core/src/lib.rs` | 导出 `ebook` 模块 |
| `rust-reader-parser/src/lib.rs` | 新增 `parse_ebook(path)` 分发函数 |
| `rust-reader-parser/src/epub.rs` | EPUB 解析器，依赖 `epub` crate |
| `rust-reader-parser/src/txt.rs` | TXT 解析器，分章并生成伪 Ebook |
| `rust-reader-parser/src/traits.rs` | `ParseError` 增加 `InvalidEpub` / `InvalidText` 等变体 |
| `rust-reader-storage/src/models.rs` | 新增 `EbookSettings` 及嵌入 `Settings` |
| `rust-reader-app/Cargo.toml` | 新增 `wry`、`epub`、`quick-xml` 依赖 |
| `rust-reader-app/src/ebook_renderer.rs` | 新增：wry webview 生命周期、自定义协议、Rust/JS 通信 |
| `rust-reader-app/src/views/ebook.rs` | 新增：`EbookView` 状态机与 egui UI |
| `rust-reader-app/src/app.rs` | 新增 `View::Ebook`、电子书菜单栏/工具栏、文件分发逻辑 |
| `rust-reader-app/src/views/settings.rs` | 新增电子书设置面板 |

---

## Phase 1: 电子书领域模型 ✅

### Task 1: 在 rust-reader-core 新增 ebook 模块 ✅

**Files:**
- Create: `rust-reader-core/src/ebook.rs`
- Modify: `rust-reader-core/src/lib.rs`
- Test: `rust-reader-core/src/ebook.rs` (inline `#[cfg(test)]`)

**假设：** 复用核心 crate，不新建 crate；电子书阅读模式沿用"单页/双页/连续"三种布局。

- [ ] **Step 1: 编写失败测试**

```rust
#[test]
fn test_ebook_reading_mode_from_str() {
    assert_eq!("single".parse::<ReadingMode>().unwrap(), ReadingMode::SinglePage);
    assert_eq!("double".parse::<ReadingMode>().unwrap(), ReadingMode::DoublePage);
    assert_eq!("scroll".parse::<ReadingMode>().unwrap(), ReadingMode::Scroll);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-core test_ebook_reading_mode_from_str -- --nocapture`
Expected: FAIL — `ReadingMode` not found.

- [ ] **Step 3: 实现模型代码**

`rust-reader-core/src/ebook.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EbookReadingMode {
    #[default]
    SinglePage,
    DoublePage,
    Scroll,
}

impl FromStr for EbookReadingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "single" | "singlepage" => Ok(Self::SinglePage),
            "double" | "doublepage" => Ok(Self::DoublePage),
            "scroll" | "continuous" => Ok(Self::Scroll),
            _ => Err(format!("unknown ebook reading mode: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookResource {
    pub id: String,
    pub href: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookChapter {
    pub index: usize,
    pub id: String,
    pub href: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ebook {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub resources: Vec<EbookResource>,
    pub spine: Vec<String>,        // manifest idrefs in reading order
    pub chapters: Vec<EbookChapter>, // table of contents / navigable chapters
}

impl Ebook {
    pub fn total_chapters(&self) -> usize {
        self.chapters.len()
    }

    pub fn chapter_source(&self, index: usize) -> Option<&EbookChapter> {
        self.chapters.get(index)
    }
}
```

`rust-reader-core/src/lib.rs` 增加：

```rust
pub mod ebook;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-core test_ebook_reading_mode_from_str -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust-reader-core/src/ebook.rs rust-reader-core/src/lib.rs
git commit -m "feat(core): add ebook domain model with reading modes"
```

---

### Task 2: 扩展 ParseError 以支持电子书错误 ✅

**Files:**
- Modify: `rust-reader-parser/src/traits.rs`
- Test: `rust-reader-parser/src/traits.rs` (inline test)

- [ ] **Step 1: 修改 `ParseError`**

在现有 enum 中增加变体：

```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid archive: {0}")]
    InvalidArchive(String),
    #[error("Unsupported file type")]
    Unsupported,
    #[error("No pages")]
    NoPages,
    #[error("Invalid EPUB: {0}")]
    InvalidEpub(String),
    #[error("Invalid text file: {0}")]
    InvalidText(String),
}
```

- [ ] **Step 2: 运行检查**

Run: `cargo check -p rust-reader-parser`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add rust-reader-parser/src/traits.rs
git commit -m "feat(parser): extend ParseError for ebook formats"
```

---

### Task 3: 实现 EPUB 解析器 ✅

**Files:**
- Create: `rust-reader-parser/src/epub.rs`
- Modify: `rust-reader-parser/Cargo.toml`, `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/src/epub.rs` (inline test with fixture)

**依赖决策：** 使用 `epub = "2.1"` crate，避免手写 EPUB 3 解析。

- [ ] **Step 1: 添加依赖**

`rust-reader-parser/Cargo.toml`:

```toml
[dependencies]
epub = "2.1"
```

- [ ] **Step 2: 实现解析器**

`rust-reader-parser/src/epub.rs`:

```rust
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookResource};
use std::path::Path;

pub struct EpubParser;

impl EpubParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("epub"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let mut doc = epub::doc::EpubDoc::new(path)
            .map_err(|e| ParseError::InvalidEpub(e.to_string()))?;

        let title = doc
            .mdata("title")
            .map(|m| m.value.clone())
            .unwrap_or_else(|| "Untitled".to_string());
        let language = doc.mdata("language").map(|m| m.value.clone());
        let authors: Vec<String> = doc
            .metadata
            .iter()
            .filter(|m| m.property == "creator")
            .map(|m| m.value.clone())
            .collect();

        let resources: Vec<EbookResource> = doc
            .resources
            .iter()
            .map(|(id, (href, mime))| EbookResource {
                id: id.clone(),
                href: href.clone(),
                mime_type: mime.clone(),
            })
            .collect();

        let spine: Vec<String> = doc.spine.iter().map(|s| s.idref.clone()).collect();

        let mut chapters: Vec<EbookChapter> = Vec::new();
        for (idx, toc) in doc.toc.iter().enumerate() {
            chapters.push(EbookChapter {
                index: idx,
                id: format!("toc-{}", idx),
                href: toc.content.clone(),
                title: Some(toc.label.clone()),
            });
        }
        // Fallback: if TOC is empty, expose spine items as chapters.
        if chapters.is_empty() {
            for (idx, idref) in spine.iter().enumerate() {
                if let Some(res) = doc.resources.get(idref) {
                    chapters.push(EbookChapter {
                        index: idx,
                        id: idref.clone(),
                        href: res.0.clone(),
                        title: None,
                    });
                }
            }
        }

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(Ebook {
            id: crate::stable_comic_id(path),
            title,
            path: path.to_path_buf(),
            authors,
            language,
            resources,
            spine,
            chapters,
        })
    }
}
```

`rust-reader-parser/src/lib.rs` 增加：

```rust
pub mod epub;
```

- [ ] **Step 3: 添加解析测试**

在 `epub.rs` 底部添加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epub_parser_supports_epub_extension() {
        assert!(EpubParser::supports(Path::new("book.epub")));
        assert!(!EpubParser::supports(Path::new("book.pdf")));
    }
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust-reader-parser/src/epub.rs rust-reader-parser/src/lib.rs rust-reader-parser/Cargo.toml
git commit -m "feat(parser): add EPUB parser"
```

---

### Task 3.5: 实现 MOBI/AZW3 解析器 ✅

**Files:**
- Create: `rust-reader-parser/src/mobi.rs`
- Modify: `rust-reader-parser/Cargo.toml`, `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/src/mobi.rs` (inline test)

**说明：** `mobi` crate 没有提供章节分片 API，因此先按 3000 字一个虚拟章节切分全部可读文本。

- [ ] **Step 1: 添加依赖**

`rust-reader-parser/Cargo.toml`:

```toml
[dependencies]
mobi = "0.8"
```

- [ ] **Step 2: 实现解析器**

`rust-reader-parser/src/mobi.rs`:

```rust
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter};
use std::path::Path;

const CHAPTER_WORDS: usize = 3000;

pub struct MobiParser;

impl MobiParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mobi" | "azw3" | "azw"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let mobi = mobi::Mobi::from_path(path)
            .map_err(|e| ParseError::InvalidEpub(format!("mobi: {}", e)))?;

        let title = mobi.title().unwrap_or("Untitled").to_string();
        let authors: Vec<String> = mobi
            .author()
            .map(|a| vec![a.to_string()])
            .unwrap_or_default();
        let language = mobi.language().map(|s| s.to_string());

        let text = mobi
            .content()
            .map_err(|e| ParseError::InvalidEpub(format!("mobi content: {}", e)))?;

        let words: Vec<&str> = text.split_whitespace().collect();
        let chapters: Vec<EbookChapter> = words
            .chunks(CHAPTER_WORDS)
            .enumerate()
            .map(|(idx, chunk)| EbookChapter {
                index: idx,
                id: format!("ch-{}", idx),
                href: format!("#ch-{}", idx),
                title: Some(format!("第 {} 章", idx + 1)),
            })
            .collect();

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(Ebook {
            id: crate::stable_comic_id(path),
            title,
            path: path.to_path_buf(),
            authors,
            language,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters,
        })
    }
}
```

`rust-reader-parser/src/lib.rs` 增加 `pub mod mobi;`。

- [ ] **Step 3: 添加测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mobi_parser_supports_extensions() {
        assert!(MobiParser::supports(Path::new("book.mobi")));
        assert!(MobiParser::supports(Path::new("book.azw3")));
        assert!(!MobiParser::supports(Path::new("book.epub")));
    }
}
```

- [ ] **Step 4: 运行测试并提交**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

```bash
git add rust-reader-parser/src/mobi.rs rust-reader-parser/src/lib.rs rust-reader-parser/Cargo.toml
git commit -m "feat(parser): add MOBI/AZW3 parser"
```

---

### Task 4: 实现 TXT 解析器 ✅

**Files:**
- Create: `rust-reader-parser/src/txt.rs`
- Modify: `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/src/txt.rs` (inline test)

**假设：** 按空行或行首 `#` / `Chapter` 分章；无章节标记则每 3000 字为一个虚拟章节。

- [ ] **Step 1: 实现解析器**

`rust-reader-parser/src/txt.rs`:

```rust
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookResource};
use std::fs;
use std::path::Path;

const DEFAULT_CHAPTER_WORDS: usize = 3000;

pub struct TxtParser;

impl TxtParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("txt"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let text = fs::read_to_string(path)
            .map_err(|e| ParseError::InvalidText(e.to_string()))?;
        if text.is_empty() {
            return Err(ParseError::NoPages);
        }

        let raw_chapters = split_chapters(&text);
        let chapters: Vec<EbookChapter> = raw_chapters
            .into_iter()
            .enumerate()
            .map(|(idx, (title, _body))| EbookChapter {
                index: idx,
                id: format!("ch-{}", idx),
                href: format!("#ch-{}", idx),
                title,
            })
            .collect();

        Ok(Ebook {
            id: crate::stable_comic_id(path),
            title: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string(),
            path: path.to_path_buf(),
            authors: Vec::new(),
            language: None,
            resources: vec![EbookResource {
                id: "text".to_string(),
                href: "text.txt".to_string(),
                mime_type: "text/plain".to_string(),
            }],
            spine: vec!["text".to_string()],
            chapters,
        })
    }
}

fn split_chapters(text: &str) -> Vec<(Option<String>, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_body: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if is_chapter_heading(trimmed) {
            if !current_body.is_empty() {
                chapters.push((current_title.take(), current_body.join("\n")));
                current_body.clear();
            }
            current_title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            current_body.push(line.to_string());
        }
    }
    if !current_body.is_empty() || current_title.is_some() {
        chapters.push((current_title.take(), current_body.join("\n")));
    }

    if chapters.is_empty() {
        // Fallback: split by word count.
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(DEFAULT_CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
}

fn is_chapter_heading(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    if line.starts_with('#') {
        return true;
    }
    let lower = line.to_ascii_lowercase();
    lower.starts_with("chapter ") || lower.starts_with("第") && lower.contains("章")
}
```

`rust-reader-parser/src/lib.rs` 增加 `pub mod txt;`。

- [ ] **Step 2: 添加测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txt_parser_supports_txt_extension() {
        assert!(TxtParser::supports(Path::new("book.txt")));
        assert!(!TxtParser::supports(Path::new("book.epub")));
    }

    #[test]
    fn test_split_chapters_by_heading() {
        let text = "# Chapter 1\nHello world.\n\n# Chapter 2\nMore text.";
        let chapters = split_chapters(text);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].0.as_deref(), Some("Chapter 1"));
        assert!(chapters[0].1.contains("Hello world"));
    }
}
```

- [ ] **Step 3: 运行测试并提交**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

```bash
git add rust-reader-parser/src/txt.rs rust-reader-parser/src/lib.rs
git commit -m "feat(parser): add TXT parser"
```

---

### Task 4.5: 实现 Markdown 解析器 ✅

**Files:**
- Create: `rust-reader-parser/src/markdown.rs`
- Modify: `rust-reader-parser/Cargo.toml`, `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/src/markdown.rs` (inline test)

**说明：** 用 `pulldown-cmark` 把 Markdown 转成 HTML；按 `#` / `##` 标题分章，无标题则按字数分虚拟章节。

- [ ] **Step 1: 添加依赖**

`rust-reader-parser/Cargo.toml`:

```toml
[dependencies]
pulldown-cmark = "0.13"
```

- [ ] **Step 2: 实现解析器**

`rust-reader-parser/src/markdown.rs`:

```rust
use crate::traits::{ParseError, Parser};
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookResource};
use std::fs;
use std::path::Path;

const CHAPTER_WORDS: usize = 3000;

pub struct MarkdownParser;

impl MarkdownParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let text = fs::read_to_string(path)
            .map_err(|e| ParseError::InvalidText(e.to_string()))?;
        if text.is_empty() {
            return Err(ParseError::NoPages);
        }

        let raw_chapters = split_markdown_chapters(&text);
        let chapters: Vec<EbookChapter> = raw_chapters
            .into_iter()
            .enumerate()
            .map(|(idx, (title, _body))| EbookChapter {
                index: idx,
                id: format!("ch-{}", idx),
                href: format!("#ch-{}", idx),
                title,
            })
            .collect();

        Ok(Ebook {
            id: crate::stable_comic_id(path),
            title: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string(),
            path: path.to_path_buf(),
            authors: Vec::new(),
            language: None,
            resources: vec![EbookResource {
                id: "text".to_string(),
                href: "text.md".to_string(),
                mime_type: "text/markdown".to_string(),
            }],
            spine: vec!["text".to_string()],
            chapters,
        })
    }
}

fn split_markdown_chapters(text: &str) -> Vec<(Option<String>, String)> {
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
            if !body.is_empty() {
                chapters.push((title.take(), body.join("\n")));
                body.clear();
            }
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if !body.is_empty() || title.is_some() {
        chapters.push((title.take(), body.join("\n")));
    }

    if chapters.is_empty() {
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
}
```

`rust-reader-parser/src/lib.rs` 增加 `pub mod markdown;`。

- [ ] **Step 3: 添加测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_parser_supports_md_extension() {
        assert!(MarkdownParser::supports(Path::new("book.md")));
        assert!(!MarkdownParser::supports(Path::new("book.txt")));
    }

    #[test]
    fn test_split_markdown_by_heading() {
        let text = "# Chapter 1\nHello world.\n\n## Chapter 2\nMore text.";
        let chapters = split_markdown_chapters(text);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].0.as_deref(), Some("Chapter 1"));
    }
}
```

- [ ] **Step 4: 运行测试并提交**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

```bash
git add rust-reader-parser/src/markdown.rs rust-reader-parser/src/lib.rs rust-reader-parser/Cargo.toml
git commit -m "feat(parser): add Markdown parser"
```

---

### Task 5: 在 parser 入口统一分发电子书 ✅

**Files:**
- Modify: `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/tests/integration.rs`

- [ ] **Step 1: 实现 `parse_ebook`**

在 `rust-reader-parser/src/lib.rs` 中保留现有 `parse`（漫画）不变，新增：

```rust
use crate::epub::EpubParser;
use crate::mobi::MobiParser;
use crate::txt::TxtParser;
use crate::markdown::MarkdownParser;

pub fn parse_ebook(path: &Path) -> Result<Ebook, ParseError> {
    if EpubParser::supports(path) {
        EpubParser::parse(path)
    } else if MobiParser::supports(path) {
        MobiParser::parse(path)
    } else if TxtParser::supports(path) {
        TxtParser::parse(path)
    } else if MarkdownParser::supports(path) {
        MarkdownParser::parse(path)
    } else {
        Err(ParseError::Unsupported)
    }
}
```

- [ ] **Step 2: 添加集成测试（使用 fixture 或临时文件）**

`rust-reader-parser/tests/ebook_integration.rs`:

```rust
use rust_reader_parser::parse_ebook;
use std::io::Write;

#[test]
fn test_parse_txt_ebook() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "# Chapter 1\nHello world.").unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 1);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("Chapter 1"));
}
```

- [ ] **Step 3: 运行并提交**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

```bash
git add rust-reader-parser/src/lib.rs rust-reader-parser/tests/ebook_integration.rs
git commit -m "feat(parser): add parse_ebook dispatch and integration tests"
```

## Phase 2: 电子书设置与存储 ✅

### Task 6: 新增 EbookSettings 模型 ✅

**Files:**
- Modify: `rust-reader-storage/src/models.rs`
- Test: `rust-reader-storage/src/models.rs`

**假设：** 电子书设置作为 `Settings` 的子结构；主题沿用现有 `Theme`；阅读模式沿用 core 的 `EbookReadingMode`。

- [ ] **Step 1: 编写失败测试**

```rust
#[test]
fn test_ebook_settings_default() {
    let s = EbookSettings::default();
    assert_eq!(s.font_size, 16);
    assert!(matches!(s.reading_mode, EbookReadingMode::SinglePage));
}
```

- [ ] **Step 2: 实现模型**

在 `rust-reader-storage/src/models.rs` 中：

```rust
use rust_reader_core::ebook::EbookReadingMode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EbookSettings {
    pub reading_mode: EbookReadingMode,
    pub font_family: String,
    pub font_size: u32,
    pub line_height: f32,
    pub margin_horizontal: u32,
    pub margin_vertical: u32,
    pub theme: EbookTheme,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EbookTheme {
    #[default]
    Light,
    Dark,
    Sepia,
}

impl Default for EbookSettings {
    fn default() -> Self {
        Self {
            reading_mode: EbookReadingMode::SinglePage,
            font_family: "system-ui".to_string(),
            font_size: 16,
            line_height: 1.6,
            margin_horizontal: 24,
            margin_vertical: 24,
            theme: EbookTheme::Light,
        }
    }
}
```

并在 `Settings` 结构体中增加：

```rust
pub struct Settings {
    // ... existing fields ...
    pub ebook: EbookSettings,
}
```

更新 `Settings::default()` 和 `Settings::validate()` / `Settings::clamp()`：

`Settings::default()` 增加字段：

```rust
Self {
    // ... existing fields ...
    ebook: EbookSettings::default(),
}
```

```rust
pub fn validate(&self) -> Result<(), String> {
    // ... existing checks ...
    if !(10..=72).contains(&self.ebook.font_size) {
        return Err(format!("ebook font_size must be 10..=72, got {}", self.ebook.font_size));
    }
    if self.ebook.line_height < 1.0 || self.ebook.line_height > 3.0 {
        return Err(format!("ebook line_height must be 1.0..=3.0, got {}", self.ebook.line_height));
    }
    Ok(())
}

pub fn clamp(&mut self) {
    // ... existing clamps ...
    self.ebook.font_size = self.ebook.font_size.clamp(10, 72);
    self.ebook.line_height = self.ebook.line_height.clamp(1.0, 3.0);
}
```

- [ ] **Step 3: 运行测试并提交**

Run: `cargo test -p rust-reader-storage`
Expected: PASS

```bash
git add rust-reader-storage/src/models.rs
git commit -m "feat(storage): add EbookSettings model"
```

---

### Task 7: 在 SettingsView 添加电子书设置 UI ✅

**Files:**
- Modify: `rust-reader-app/src/views/settings.rs`

- [ ] **Step 1: 实现 UI**

在 `SettingsView::ui` 末尾（或新增 `ebook_settings_ui` 方法）：

```rust
ui.collapsing("电子书", |ui| {
    ui.horizontal(|ui| {
        ui.label("阅读模式:");
        egui::ComboBox::from_id_salt("ebook_mode")
            .selected_text(ebook_mode_label(settings.ebook.reading_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.ebook.reading_mode, EbookReadingMode::SinglePage, "单页");
                ui.selectable_value(&mut settings.ebook.reading_mode, EbookReadingMode::DoublePage, "双页");
                ui.selectable_value(&mut settings.ebook.reading_mode, EbookReadingMode::Scroll, "连续滚动");
            });
    });

    ui.horizontal(|ui| {
        ui.label("字体大小:");
        ui.add(egui::Slider::new(&mut settings.ebook.font_size, 10..=72));
    });

    ui.horizontal(|ui| {
        ui.label("行间距:");
        ui.add(egui::Slider::new(&mut settings.ebook.line_height, 1.0..=3.0).step_by(0.05));
    });

    ui.horizontal(|ui| {
        ui.label("主题:");
        egui::ComboBox::from_id_salt("ebook_theme")
            .selected_text(ebook_theme_label(settings.ebook.theme))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Light, "白天");
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Dark, "夜晚");
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Sepia, "Sepia");
            });
    });
});
```

新增辅助函数 `ebook_mode_label` 和 `ebook_theme_label`。

- [ ] **Step 2: 运行 clippy 并提交**

Run: `cargo clippy -p rust-reader-app --all-targets -- -D warnings`
Expected: PASS

```bash
git add rust-reader-app/src/views/settings.rs
git commit -m "feat(ui): add ebook settings panel"
```

---

## Phase 3: wry 渲染层 ✅

### Task 8: 添加 wry 依赖 ✅

**Files:**
- Modify: `rust-reader-app/Cargo.toml`

- [ ] **Step 1: 添加依赖**

```toml
[dependencies]
wry = "0.55"
serde_json = { workspace = true }
pulldown-cmark = "0.13"
mobi = "0.8"
```

- [ ] **Step 2: 运行 cargo check 确认版本兼容**

Run: `cargo check -p rust-reader-app`
Expected: PASS（注意：wry 会拉取大量系统依赖，首次可能较慢）

- [ ] **Step 3: Commit**

```bash
git add rust-reader-app/Cargo.toml Cargo.lock
git commit -m "chore(deps): add wry for ebook rendering"
```

---

### Task 9: 创建 EbookRenderer 封装 wry WebView ✅

**Files:**
- Create: `rust-reader-app/src/ebook_renderer.rs`
- Modify: `rust-reader-app/src/main.rs`（注册模块）

**核心设计：**
- `EbookRenderer` 持有 `wry::WebView`。
- 使用自定义协议 `ebook://` 提供 EPUB 章节和资源。
- 使用 `evaluate_script` 向 JS 发送设置/位置变更。
- 使用 `with_ipc_handler` 接收 JS 的滚动/翻页/位置报告。

**平台说明：**
- 默认使用 `WebViewBuilder::build_as_child(&frame)`，依赖 eframe `Frame` 实现 `HasWindowHandle`。
- Linux X11 only。若用户选择必须支持 Wayland，则此 Task 需要改为 `WebViewBuilderExtUnix::new_gtk` 分支。

- [ ] **Step 1: 定义消息类型与结构体**

`rust-reader-app/src/ebook_renderer.rs` 开头：

```rust
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookReadingMode};
use rust_reader_storage::models::{EbookSettings, EbookTheme};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wry::{Rect, WebView, WebViewBuilder};

pub enum EbookIpcMessage {
    PositionReport { chapter: usize, char_offset: usize },
    PageCount(usize),
    Ready,
}

pub struct EbookRenderer {
    webview: WebView,
    state: Arc<Mutex<RendererState>>,
}

struct RendererState {
    ebook: Ebook,
    current_chapter: usize,
    char_offset: usize,
    settings: EbookSettings,
}

#[derive(Debug, serde::Deserialize)]
struct JsToRust {
    #[serde(rename = "type")]
    kind: String,
    chapter: Option<usize>,
    char_offset: Option<usize>,
    page_count: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
struct JsSettings {
    mode: String,
    bg: String,
    fg: String,
    font: String,
    size: u32,
    line: f32,
    margin_h: u32,
    margin_v: u32,
}

impl From<&EbookSettings> for JsSettings {
    fn from(s: &EbookSettings) -> Self {
        let (bg, fg) = match s.theme {
            EbookTheme::Light => ("#ffffff".to_string(), "#000000".to_string()),
            EbookTheme::Dark => ("#1a1a1a".to_string(), "#e0e0e0".to_string()),
            EbookTheme::Sepia => ("#f4ecd8".to_string(), "#433422".to_string()),
        };
        Self {
            mode: match s.reading_mode {
                EbookReadingMode::SinglePage => "single",
                EbookReadingMode::DoublePage => "double",
                EbookReadingMode::Scroll => "scroll",
            }
            .to_string(),
            bg,
            fg,
            font: s.font_family.clone(),
            size: s.font_size,
            line: s.line_height,
            margin_h: s.margin_horizontal,
            margin_v: s.margin_vertical,
        }
    }
}

fn handle_ipc_message(msg: JsToRust, state: &Arc<Mutex<RendererState>>) {
    let mut state = state.lock().unwrap();
    match msg.kind.as_str() {
        "position" => {
            if let Some(chapter) = msg.chapter {
                state.current_chapter = chapter;
            }
            if let Some(offset) = msg.char_offset {
                state.char_offset = offset;
            }
        }
        "page_count" => {
            // Store for progress calculation if needed.
        }
        "ready" => {
            // WebView is ready to receive settings.
        }
        _ => {}
    }
}
```

- [ ] **Step 2: 实现构造函数**

```rust
impl EbookRenderer {
    pub fn new<W: wry::raw_window_handle::HasWindowHandle>(
        parent: &W,
        bounds: Rect,
        ebook: Ebook,
        settings: EbookSettings,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            settings,
        }));

        let ipc_state = state.clone();
        let webview = WebViewBuilder::new()
            .with_bounds(bounds)
            .with_custom_protocol("ebook".to_string(), {
                let state = state.clone();
                move |_id, request| handle_ebook_protocol(&state, request)
            })
            .with_ipc_handler(move |request| {
                let body = request.body();
                if let Ok(msg) = serde_json::from_str::<JsToRust>(body) {
                    handle_ipc_message(msg, &ipc_state);
                }
            })
            .with_url("ebook://reader")
            .build_as_child(parent)
            .map_err(|e| e.to_string())?;

        Ok(Self { webview, state })
    }

    // ...
}
```

- [ ] **Step 3: 实现自定义协议处理**

```rust
fn handle_ebook_protocol(
    state: &Arc<Mutex<RendererState>>,
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let path = request.uri().path();
    let state = state.lock().unwrap();

    if path == "/reader" {
        return wry::http::Response::builder()
            .header("Content-Type", "text/html; charset=utf-8")
            .body(reader_html(&state.settings).into_bytes().into())
            .unwrap();
    }

    // path like "/chapter/0" or "/resource/cover.png"
    if let Some(rest) = path.strip_prefix("/chapter/") {
        if let Ok(idx) = rest.parse::<usize>() {
            if let Some(chapter) = state.ebook.chapter_source(idx) {
                if let Some(html) = load_chapter_html(&state.ebook, chapter) {
                    return wry::http::Response::builder()
                        .header("Content-Type", "application/xhtml+xml; charset=utf-8")
                        .body(html.into_bytes().into())
                        .unwrap();
                }
            }
        }
    }

    wry::http::Response::builder()
        .status(404)
        .body(Vec::new().into())
        .unwrap()
}

fn load_chapter_html(ebook: &Ebook, chapter: &EbookChapter) -> Option<String> {
    if is_text_like_path(&ebook.path) {
        let text = std::fs::read_to_string(&ebook.path).ok()?;
        let raw_chapters = if is_markdown_path(&ebook.path) {
            split_markdown_chapters(&text)
        } else {
            txt_split_chapters(&text)
        };
        let (_, body) = raw_chapters.get(chapter.index)?.clone();
        let html = if is_markdown_path(&ebook.path) {
            markdown_to_html(&body)
        } else {
            plain_text_to_html(&body)
        };
        Some(format!("<div class=\"chapter\">{}</div>", html))
    } else if is_mobi_path(&ebook.path) {
        let mobi = mobi::Mobi::from_path(&ebook.path).ok()?;
        let text = mobi.content().ok()?;
        let words: Vec<&str> = text.split_whitespace().collect();
        let chunk = words.chunks(3000).nth(chapter.index)?;
        Some(format!(
            "<div class=\"chapter\">{}</div>",
            plain_text_to_html(&chunk.join(" "))
        ))
    } else {
        let mut doc = epub::doc::EpubDoc::new(&ebook.path).ok()?;
        doc.set_current_chapter(chapter.index);
        let (bytes, _mime) = doc.get_current()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn plain_text_to_html(body: &str) -> String {
    let escaped = escape_html(body);
    escaped
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", p.replace('\n', "<br>")))
        .collect::<String>()
}

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let parser = Parser::new_ext(md, Options::all());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

> **实际实现偏差：** 协议路由已改为先检查查询参数 `chapter=`，再回退到阅读器壳页面。章节 URL 为 `ebook://reader?chapter=N`，壳页面 URL 为 `ebook://reader`。响应头增加了 `Cache-Control: no-cache, no-store, must-revalidate`。章节 HTML 由 `rust_reader_parser::html::render_chapter_html` 统一渲染，不在 renderer 内重复实现。
>
> JS 侧所有 `window.ipc.postMessage` 调用已封装为 `sendIpc(obj)`，在 `window.ipc` 尚未注入时自动重试，避免 macOS 上偶发的 IPC 桥未就绪导致脚本崩溃。`JsSettings.mode` 实际输出为 `"single paginated"` / `"double paginated"` / `"scroll"`，以匹配 CSS 选择器 `body.paginated` 与 `body.double`。

fn is_text_like_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "txt" | "md"))
        .unwrap_or(false)
}

fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn is_mobi_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mobi" | "azw" | "azw3"))
        .unwrap_or(false)
}

fn split_markdown_chapters(text: &str) -> Vec<(Option<String>, String)> {
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
            if !body.is_empty() {
                chapters.push((title.take(), body.join("\n")));
                body.clear();
            }
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if !body.is_empty() || title.is_some() {
        chapters.push((title.take(), body.join("\n")));
    }

    if chapters.is_empty() {
        const CHAPTER_WORDS: usize = 3000;
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
}

fn txt_split_chapters(text: &str) -> Vec<(Option<String>, String)> {
    // Same logic as rust-reader-parser/src/txt.rs; duplicated here to keep
    // renderer self-contained, or refactor into a shared helper crate later.
    const DEFAULT_CHAPTER_WORDS: usize = 3000;
    let lines: Vec<&str> = text.lines().collect();
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    let is_heading = |line: &str| {
        if line.is_empty() {
            return false;
        }
        if line.starts_with('#') {
            return true;
        }
        let lower = line.to_ascii_lowercase();
        lower.starts_with("chapter ") || (lower.starts_with("第") && lower.contains("章"))
    };

    for line in lines {
        let trimmed = line.trim();
        if is_heading(trimmed) {
            if !body.is_empty() {
                chapters.push((title.take(), body.join("\n")));
                body.clear();
            }
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if !body.is_empty() || title.is_some() {
        chapters.push((title.take(), body.join("\n")));
    }
    if chapters.is_empty() {
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(DEFAULT_CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
}
```

- [ ] **Step 4: 实现 JS 注入与 Rust/JS 通信**

`reader_html` 返回的 HTML 包含：

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <style>
    :root {
      --bg: #fff; --fg: #000; --font: system-ui; --size: 16px; --line: 1.6;
    }
    body {
      margin: 0; padding: var(--m-v) var(--m-h);
      background: var(--bg); color: var(--fg);
      font-family: var(--font); font-size: var(--size); line-height: var(--line);
    }
    #content { column-width: 100vw; column-gap: 0; height: 100vh; overflow: hidden; }
    .scroll #content { column-width: auto; height: auto; overflow: auto; }
  </style>
</head>
<body>
  <div id="content"></div>
  <script>
    // load chapter via fetch, apply settings, report position via window.ipc.postMessage
    async function loadChapter(index) {
      const res = await fetch(`ebook://chapter/${index}`);
      const html = await res.text();
      document.getElementById('content').innerHTML = html;
      reportPosition();
    }
    function applySettings(s) {
      const root = document.documentElement;
      root.style.setProperty('--bg', s.bg);
      root.style.setProperty('--fg', s.fg);
      root.style.setProperty('--font', s.font);
      root.style.setProperty('--size', s.size + 'px');
      root.style.setProperty('--line', s.line);
      root.style.setProperty('--m-h', s.marginH + 'px');
      root.style.setProperty('--m-v', s.marginV + 'px');
      document.body.className = s.mode;
    }
    function reportPosition() {
      // compute current chapter/offset based on scroll or column offset
      window.ipc.postMessage(JSON.stringify({ type: 'position', chapter: currentChapter, charOffset: 0 }));
    }
    window.addEventListener('scroll', reportPosition);
  </script>
</body>
</html>
```

- [ ] **Step 5: 添加方法供 UI 调用**

```rust
impl EbookRenderer {
    pub fn set_bounds(&self, bounds: Rect) {
        let _ = self.webview.set_bounds(bounds);
    }

    pub fn apply_settings(&self, settings: &EbookSettings) {
        let js = format!(
            "applySettings({});",
            serde_json::to_string(&JsSettings::from(settings)).unwrap()
        );
        let _ = self.webview.evaluate_script(&js);
    }

    pub fn goto_chapter(&self, chapter: usize, offset: usize) {
        let js = format!("loadChapter({}); currentChapter = {};", chapter, chapter);
        let _ = self.webview.evaluate_script(&js);
    }

    pub fn next_page(&self) {
        let _ = self.webview.evaluate_script("nextPage();");
    }

    pub fn prev_page(&self) {
        let _ = self.webview.evaluate_script("prevPage();");
    }
}
```

- [ ] **Step 6: 添加单元测试（协议响应测试）**

由于 `WebView` 难以在 headless 测试中使用，重点测试 HTML 生成和协议路由逻辑。可以提取纯函数进行测试。

- [ ] **Step 7: 注册模块**

`rust-reader-app/src/main.rs` 顶部增加：

```rust
mod ebook_renderer;
```

- [ ] **Step 8: 运行 check 并提交**

Run: `cargo check -p rust-reader-app`
Expected: PASS

```bash
git add rust-reader-app/src/ebook_renderer.rs rust-reader-app/src/main.rs
git commit -m "feat(ebook): add wry-based EbookRenderer with custom protocol"
```

## Phase 4: APP 集成与 UI ⚠️ 基础集成已完成，目录面板与书架混排待实现

### Task 10: 创建 EbookView ✅

**Files:**
- Create: `rust-reader-app/src/views/ebook.rs`
- Modify: `rust-reader-app/src/views/mod.rs`

**核心职责：**
- 保存当前 `Ebook`、`EbookRenderer`、阅读进度。
- 响应键盘/菜单/工具栏事件。
- 在 `ui` 方法中计算 webview 的 bounds（避开工具栏/状态栏），并更新 renderer。

- [ ] **Step 1: 定义 EbookView**

`rust-reader-app/src/views/ebook.rs`:

```rust
use crate::ebook_renderer::{EbookRenderer, EbookIpcMessage};
use rust_reader_core::ebook::{Ebook, EbookReadingMode};
use rust_reader_storage::models::EbookSettings;
use wry::Rect;

pub struct EbookView {
    pub open: Option<OpenEbook>,
}

pub struct OpenEbook {
    pub ebook: Ebook,
    pub renderer: EbookRenderer,
    pub current_chapter: usize,
    pub char_offset: usize,
}

impl Default for EbookView {
    fn default() -> Self { Self { open: None } }
}

impl EbookView {
    pub fn open(
        &mut self,
        parent: &impl wry::raw_window_handle::HasWindowHandle,
        bounds: Rect,
        ebook: Ebook,
        settings: &EbookSettings,
    ) -> Result<(), String> {
        let renderer = EbookRenderer::new(parent, bounds, ebook.clone(), settings.clone())?;
        self.open = Some(OpenEbook {
            ebook,
            renderer,
            current_chapter: 0,
            char_offset: 0,
        });
        Ok(())
    }

    pub fn update_bounds(&mut self, bounds: Rect) {
        if let Some(open) = &mut self.open {
            open.renderer.set_bounds(bounds);
        }
    }

    pub fn apply_settings(&mut self, settings: &EbookSettings) {
        if let Some(open) = &mut self.open {
            open.renderer.apply_settings(settings);
        }
    }

    pub fn next_page(&mut self) {
        if let Some(open) = &mut self.open {
            open.renderer.next_page();
        }
    }

    pub fn prev_page(&mut self) {
        if let Some(open) = &mut self.open {
            open.renderer.prev_page();
        }
    }

    pub fn goto_chapter(&mut self, chapter: usize) {
        if let Some(open) = &mut self.open {
            open.current_chapter = chapter.min(open.ebook.total_chapters().saturating_sub(1));
            open.renderer.goto_chapter(open.current_chapter, 0);
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // EbookView does not draw inside egui; the webview is positioned over the panel.
        // We only use this to reserve the central panel area so egui lays out correctly.
        ui.allocate_space(ui.available_size());
    }
}
```

`rust-reader-app/src/views/mod.rs` 增加 `pub mod ebook;`。

- [ ] **Step 2: Commit**

```bash
git add rust-reader-app/src/views/ebook.rs rust-reader-app/src/views/mod.rs
git commit -m "feat(ebook): add EbookView state holder"
```

---

### Task 11: 在 ReaderApp 添加 View::Ebook 与渲染分支 ✅

**Files:**
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: 扩展 View enum**

```rust
pub enum View {
    Library,
    Reader,
    Ebook,
    Settings,
    Loading(PathBuf),
}
```

- [ ] **Step 2: 在 ReaderApp 中加入 ebook_view 字段**

```rust
pub struct ReaderApp {
    // ... existing fields ...
    pub ebook_view: EbookView,
}
```

更新 `Default::default()` 初始化 `ebook_view: EbookView::default()`。

- [ ] **Step 3: 在 update 中添加电子书分支**

```rust
match self.current_view.clone() {
    View::Library => self.render_library(ctx),
    View::Reader => self.render_reader(ctx),
    View::Ebook => self.render_ebook(ctx, frame),
    View::Settings => self.render_settings(ctx),
    View::Loading(path) => self.render_loading(ctx, path),
}
```

- [ ] **Step 4: 实现 render_ebook**

```rust
fn render_ebook(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
    let Some(open) = self.ebook_view.open.as_ref() else {
        self.current_view = View::Library;
        return;
    };

    let (fullscreen, mouse_pos, screen_size) = ctx.input(|i| {
        (
            i.viewport().fullscreen.unwrap_or(false),
            i.pointer.latest_pos(),
            i.screen_rect().size(),
        )
    });

    if Self::should_show_bar(self.settings.show_toolbar, fullscreen, mouse_pos, screen_size, BarEdge::Top) {
        self.render_ebook_toolbar(ctx);
    }
    if Self::should_show_bar(self.settings.show_statusbar, fullscreen, mouse_pos, screen_size, BarEdge::Bottom) {
        self.render_ebook_statusbar(ctx);
    }

    let top_height = ctx.style().spacing.interact_size.y * 2.0; // estimate toolbar height
    let bottom_height = ctx.style().spacing.interact_size.y * 1.5; // estimate statusbar height
    let avail = ctx.screen_rect();
    let bounds = wry::Rect {
        position: wry::dpi::LogicalPosition::new(avail.min.x, avail.min.y + top_height).into(),
        size: wry::dpi::LogicalSize::new(
            avail.width(),
            avail.height() - top_height - bottom_height,
        )
        .into(),
    };
    self.ebook_view.update_bounds(bounds);

    egui::CentralPanel::default().show(ctx, |ui| {
        self.ebook_view.ui(ctx, ui);
    });
}
```

注意：`frame` 需要实现 `HasWindowHandle`。eframe 0.29 的 `Frame` 实现了该 trait，但需要在调用处通过 `&*frame` 或 `frame` 传入。

- [ ] **Step 5: Commit**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(app): add View::Ebook and render_ebook branch"
```

---

### Task 12: 电子书专用菜单栏与工具栏 ⚠️ 菜单/工具栏已集成，"目录"按钮与状态栏为占位待实现

**Files:**
- Modify: `rust-reader-app/src/app.rs`

**核心要求：** 电子书模式下不能出现漫画模式的"国漫/日漫/韩漫"、双页切换、图片缩放等控件。

- [ ] **Step 1: 改造 render_menu_bar**

将"阅读"菜单按当前视图分支：

```rust
let is_reader = matches!(self.current_view, View::Reader);
let is_ebook = matches!(self.current_view, View::Ebook);
ui.add_enabled_ui(is_reader || is_ebook, |ui| {
    ui.menu_button("阅读", |ui| {
        match self.current_view {
            View::Reader => self.render_reader_menu(ui),
            View::Ebook => self.render_ebook_menu(ui),
            _ => {}
        }
    });
});
```

`render_reader_menu` 是将现有 `render_menu_bar` 中"阅读"菜单的实现提取成独立方法（保留漫画相关菜单项）。`render_ebook_menu` 实现如下：

```rust
fn render_ebook_menu(&mut self, ui: &mut egui::Ui) {
    if ui.button("上一章").clicked() {
        self.ebook_view.prev_page();
        ui.close_menu();
    }
    if ui.button("下一章").clicked() {
        self.ebook_view.next_page();
        ui.close_menu();
    }
    ui.separator();
    if ui.button("目录").clicked() {
        // 目录面板实现待定：用户决定采用侧栏/弹窗/独立页面后补充。
        ui.close_menu();
    }
    ui.separator();
    if ui.button("增大字体").clicked() {
        self.settings.ebook.font_size = (self.settings.ebook.font_size + 1).min(72);
        self.ebook_view.apply_settings(&self.settings.ebook);
        ui.close_menu();
    }
    if ui.button("减小字体").clicked() {
        self.settings.ebook.font_size = self.settings.ebook.font_size.saturating_sub(1).max(10);
        self.ebook_view.apply_settings(&self.settings.ebook);
        ui.close_menu();
    }
    ui.separator();
    if ui.button("切换主题").clicked() {
        self.settings.ebook.theme = match self.settings.ebook.theme {
            EbookTheme::Light => EbookTheme::Dark,
            EbookTheme::Dark => EbookTheme::Sepia,
            EbookTheme::Sepia => EbookTheme::Light,
        };
        self.ebook_view.apply_settings(&self.settings.ebook);
        ui.close_menu();
    }
}
```

- [ ] **Step 2: 实现 render_ebook_toolbar**

```rust
fn render_ebook_toolbar(&mut self, ctx: &egui::Context) {
    let total = self.ebook_view.open.as_ref().map(|e| e.ebook.total_chapters()).unwrap_or(0);
    let current = self.ebook_view.open.as_ref().map(|e| e.current_chapter).unwrap_or(0);

    egui::TopBottomPanel::top("ebook_toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("书架").clicked() {
                self.current_view = View::Library;
            }
            ui.separator();

            if ui.button("目录").clicked() {
                // 目录面板实现待定：用户决定采用侧栏/弹窗/独立页面后补充。
            }
            if ui.button("上一章").clicked() {
                self.ebook_view.prev_page();
            }
            if ui.button("下一章").clicked() {
                self.ebook_view.next_page();
            }
            ui.label(format!("{} / {}", current + 1, total));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("设置").clicked() {
                    self.current_view = View::Settings;
                }
            });
        });
    });
}
```

- [ ] **Step 3: 实现 render_ebook_statusbar**

可显示当前章节标题和阅读进度百分比。

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(app): add ebook-specific menu, toolbar and statusbar"
```

---

### Task 13: 文件打开逻辑：漫画 vs 电子书 ✅

**Files:**
- Modify: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-parser/src/lib.rs`

**假设：** 通过扩展名区分；`.epub` / `.txt` 走电子书，其余走漫画。

- [ ] **Step 1: 添加辅助函数**

在 `app.rs` 中：

```rust
fn is_ebook_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "epub" | "mobi" | "azw" | "azw3" | "txt" | "md"
            )
        })
        .unwrap_or(false)
}
```

- [ ] **Step 2: 新增 open_ebook 方法**

```rust
fn open_ebook(&mut self, path: std::path::PathBuf) {
    self.opener = Some(ComicOpener::new(move || {
        rust_reader_parser::parse_ebook(&path).map_err(|e| e.to_string())
    }));
    self.current_view = View::Loading(path);
}
```

注意：`ComicOpener` 当前返回 `Comic`，需要扩展为返回泛型结果，或新建 `EbookOpener`。

**决策点：** 是复用 `ComicOpener` 为通用 `AsyncOpener<T>`，还是新建 `EbookOpener`？

推荐方案：将 `ComicOpener` 泛化为 `AsyncOpener<T>`，这样漫画和电子书都能用。

- [ ] **Step 3: 泛化 Opener**

`rust-reader-app/src/opener.rs`:

```rust
pub struct AsyncOpener<T> {
    receiver: std::sync::mpsc::Receiver<OpenStatus<T>>,
}

pub enum OpenStatus<T> {
    Loading,
    Ready(Result<T, String>),
}

impl<T: Send + 'static> AsyncOpener<T> {
    pub fn new<F>(factory: F) -> Self
    where
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let (tx, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(OpenStatus::Loading);
            let result = factory();
            let _ = tx.send(OpenStatus::Ready(result));
        });
        Self { receiver }
    }

    pub fn poll(&mut self) -> OpenStatus<T> {
        match self.receiver.try_recv() {
            Ok(status) => status,
            Err(_) => OpenStatus::Loading,
        }
    }
}
```

然后 `ReaderApp` 中：
- `pub opener: Option<AsyncOpener<Comic>>`
- `pub ebook_opener: Option<AsyncOpener<Ebook>>`

或统一用 `enum ActiveOpener { Comic(AsyncOpener<Comic>), Ebook(AsyncOpener<Ebook>) }`。

为简单起见，建议新增 `ebook_opener` 字段。

- [ ] **Step 4: 修改 open_comic / open_ebook 调用点**

在所有用户触发打开文件的地方：

```rust
if is_ebook_file(&path) {
    self.open_ebook(path);
} else {
    self.open_comic(path);
}
```

需要修改的位置：
- `render_library` 中 `on_open_library` / `on_open_path`
- `render_menu_bar` 中"打开最近"
- `handle_open_paths` / `handle_dropped_files`
- dock open 事件

- [ ] **Step 5: 新增 poll_ebook_opener**

```rust
fn poll_ebook_opener(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
    let Some(mut opener) = self.ebook_opener.take() else { return; };
    match opener.poll() {
        OpenStatus::Loading => self.ebook_opener = Some(opener),
        OpenStatus::Ready(result) => match result {
            Ok(ebook) => {
                let screen = ctx.screen_rect();
                let toolbar_height = ctx.style().spacing.interact_size.y * 2.0;
                let statusbar_height = ctx.style().spacing.interact_size.y * 1.5;
                let bounds = wry::Rect {
                    position: wry::dpi::LogicalPosition::new(screen.min.x, screen.min.y + toolbar_height).into(),
                    size: wry::dpi::LogicalSize::new(
                        screen.width(),
                        screen.height() - toolbar_height - statusbar_height,
                    )
                    .into(),
                };
                match self.ebook_view.open(frame, bounds, ebook, &self.settings.ebook) {
                    Ok(()) => {
                        self.current_view = View::Ebook;
                        self.error_message = None;
                    }
                    Err(e) => self.error_message = Some(format!("无法创建阅读器: {}", e)),
                }
            }
            Err(e) => {
                self.error_message = Some(format!("无法打开电子书: {}", e));
                self.current_view = View::Library;
            }
        },
    }
}
```

- [ ] **Step 6: Commit**

```bash
git add rust-reader-app/src/opener.rs rust-reader-app/src/app.rs
git commit -m "feat(app): dispatch comic vs ebook based on file extension"
```

---

### Task 14: 书架支持电子书 ❌ 待实现（当前决策：暂不进入书架，仅通过打开文件阅读）

**Files:**
- Modify: `rust-reader-storage/src/models.rs`
- Modify: `rust-reader-app/src/views/library.rs`
- Modify: `rust-reader-app/src/app.rs`

**假设：** LibraryEntry 增加 `media_type: MediaType` 字段（默认 Comic，兼容旧数据）。

- [ ] **Step 1: 扩展 LibraryEntry**

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    #[default]
    Comic,
    Ebook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64,
    pub media_type: MediaType,
}
```

- [ ] **Step 2: 迁移旧数据**

在 `ReaderApp::default()` 的 migrate 逻辑中，旧条目没有 `media_type`，默认设为 `MediaType::Comic`。

- [ ] **Step 3: LibraryView 增加类型过滤/显示**

在 LibraryView 的 tabs 中增加"电子书"：

```rust
if ui.selectable_label(self.mode == LibraryMode::Ebooks, "电子书").clicked() {
    self.mode = LibraryMode::Ebooks;
}
```

过滤逻辑根据 `entry.media_type`。

- [ ] **Step 4: 添加文件时判断类型**

`add_folder_to_library` / `add_file_to_library` 根据扩展名设置 `media_type`。

- [ ] **Step 5: Commit**

```bash
git add rust-reader-storage/src/models.rs rust-reader-app/src/views/library.rs rust-reader-app/src/app.rs
git commit -m "feat(library): support ebook entries in library"
```

## Phase 5: 测试、验证与收尾 ⏳ 集成测试与书架 EPUB fixture 待补充

### Task 15: 电子书集成测试 ⏳ 解析器单元测试已存在，EPUB fixture 与 renderer 纯函数测试待补充

**Files:**
- Create: `rust-reader-app/tests/ebook_view_integration.rs`
- Create: `rust-reader-parser/tests/ebook_integration.rs`（已在 Task 5 创建，此处扩展）

- [ ] **Step 1: 为 EPUB 解析添加 fixture 测试**

若项目暂无真实 EPUB fixture，可用一个最小 EPUB（ZIP 内包含 META-INF/container.xml、OEBPS/content.opf、OEBPS/chapter1.xhtml）。测试文件可放在 `rust-reader-parser/tests/fixtures/minimal.epub`。

```rust
#[test]
fn test_parse_minimal_epub() {
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/minimal.epub"));
    let ebook = rust_reader_parser::parse_ebook(&path).unwrap();
    assert!(!ebook.title.is_empty());
    assert!(!ebook.chapters.is_empty());
}
```

- [ ] **Step 2: 测试 wry 相关纯函数**

由于 wry WebView 需要真实窗口，headless 测试困难。应确保以下逻辑可通过单元测试覆盖：
- `EbookRenderer` 的 HTML 模板生成（提取为纯函数测试）。
- 设置到 CSS 变量的映射。
- 章节 URL 路由解析。

- [ ] **Step 3: 运行全部测试**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add rust-reader-parser/tests rust-reader-app/tests
git commit -m "test(ebook): add EPUB/TXT integration tests"
```

---

### Task 16: 完整验证与代码清理 ⏳ 验证流水线已通过，书架混排与目录面板完成后可收尾

**Files:**
- 所有已修改文件

- [ ] **Step 1: 运行完整验证流水线**

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部通过，0 警告。

- [ ] **Step 2: 手动走查关键路径**

1. 启动应用，进入书架。
2. 通过"文件 → 打开文件夹"或拖放导入包含 `.epub` 和 `.txt` 的文件夹。
3. 点击电子书条目，应进入电子书模式，显示 wry 渲染的 HTML。
4. 确认工具栏是"目录/上一章/下一章/设置"，没有漫画模式的"国漫/日漫/韩漫/缩放"。
5. 切换单页/双页/连续滚动，确认布局变化。
6. 调整字体/主题，确认渲染变化。
7. 关闭应用重新打开，确认阅读位置恢复（若已实现历史记录）。

- [ ] **Step 3: 提交并合并**

```bash
git add .
git commit -m "feat(ebook): full ebook reader support (EPUB, TXT, wry)"
```

---

## 已锁定决策

以下问题已根据实际实现确定：

| 问题 | 最终决策 | 说明 |
|------|---------|------|
| 书架混排 | C) 暂不进入书架 | 当前仅通过"打开文件"阅读；Task 14 保留为未来扩展 |
| Linux Wayland | A) X11 only | 使用 `WebViewBuilder::build_as_child`，Wayland 支持后续再评估 |
| EPUB 渲染 | A) 自研 HTML/CSS/JS | `ebook://reader` 壳页面 + `?chapter=N` 查询加载 |
| TXT 分章 | A) 标题+空行分章，否则字数分章 | 见 `rust-reader-parser/src/txt.rs` |
| 设置范围 | 字体、字号、行高、页边距、主题、阅读模式 | 暂不加入翻页动画、字重、对齐方式 |
| 历史/书签 | A) 复用现有结构 + char offset | `page_index` 对应章节索引，字符偏移后续扩展 |
| 文件扩展名 | `.epub`、`.txt`、`.mobi`/`.azw`/`.azw3`、`.md` | 见 `app.rs` 中的 `is_ebook_file` |

---

## 执行选项

Plan complete and saved to `docs/superpowers/plans/2026-06-24-ebook-reader.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach do you prefer?
