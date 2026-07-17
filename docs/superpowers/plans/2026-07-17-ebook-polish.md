# 电子书半成品接线与收尾（TODO 36–42）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 TODO.md 36–42：提交更名收尾、TXT/Markdown 编码检测、EPUB 内嵌图片与字体、font_family 设置 UI、电子书搜索 UI、电子书快捷键补全，并全程配套测试。

**Architecture:** 按 `docs/superpowers/specs/2026-07-17-ebook-polish-design.md` 执行。解析层（openitgo-parser）新增 `text_encoding` 模块与 EPUB 资源/字体提取纯函数；渲染层（openitgo-app 的 `ebook_renderer.rs`）新增 `ebook://res/` 资源通道与 search IPC；视图层（`views/ebook.rs`、`app.rs`）接搜索条与快捷键。所有 URL 改写、编码检测、CSS 提取均为纯函数，可单测。

**Tech Stack:** Rust workspace（eframe/egui 0.29、wry 0.55、epub 2.1.5、chardetng、encoding_rs、percent-encoding、serde_json）。

## Global Constraints

- 验证流水线（每个任务提交前必须通过）：`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`。
- UI 文本一律中文（专有名词/技术标识符除外）。
- 最小改动：不重构无关代码，不改变现有公开行为（除本计划明确点名的）。
- 提交信息：中文摘要 + 涉及 crate，遵循仓库现有风格（如 `feat(parser): ...`）。
- `epub` crate 锁定 2.1.5：`get_resource_by_path`/`get_resource_str_by_path`/`get_resource_mime_by_path` 均为 `&mut self`（mime 版为 `&self`），返回 `Option`。`EpubDoc::new(path)` 返回 `EpubDoc<BufReader<File>>`。
- JS 模板 `ebook_renderer_template.rs` 是 Rust `format!` 字符串：JS 代码中的花括号必须双写 `{{` / `}}`。
- 协议 handler 对未知请求必须维持"空 200"约定（404 会触发 WebKit 重新加载壳页面）。
- 新增依赖仅允许：`chardetng`、`encoding-rs`、`percent-encoding`（均加在 `openitgo-parser`）。

---

### Task 1: 提交更名收尾（TODO 36）

**Files:**
- Modify（已改未提交，直接提交）: `openitgo-storage/src/json_store.rs`、`CHANGELOG.md`、`docs/superpowers/README.md`

**Interfaces:**
- Consumes: 无
- Produces: 干净的工作区，后续任务从此基线开始

- [ ] **Step 1: 运行完整验证流水线**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部通过，无警告。若失败，先修复再提交（失败内容与本计划无关，停下来向用户报告）。

- [ ] **Step 2: 提交**

```bash
git add openitgo-storage/src/json_store.rs CHANGELOG.md docs/superpowers/README.md
git commit -m "chore(storage): 收尾更名——移除旧 rust-reader 配置目录迁移逻辑"
```

---

### Task 2: TXT/Markdown 编码检测（TODO 41）

**Files:**
- Create: `openitgo-parser/src/text_encoding.rs`
- Modify: `openitgo-parser/Cargo.toml`、`openitgo-parser/src/lib.rs`、`openitgo-parser/src/txt.rs:32-35`、`openitgo-parser/src/markdown.rs:26-29`、`openitgo-parser/src/html.rs:12-14`
- Test: `openitgo-parser/src/text_encoding.rs`（模块内）、`openitgo-parser/tests/ebook_integration.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub fn read_text_lossy(path: &Path) -> Result<String, ParseError>` — 读文件并检测编码，后续 Task 不再使用（本任务内全部接线完毕）。
  - `pub fn decode_text_bytes(bytes: &[u8]) -> String` — 纯函数，UTF-8（含 BOM）直通，否则 chardetng + encoding_rs 转码。

- [ ] **Step 1: 添加依赖**

`openitgo-parser/Cargo.toml` 的 `[dependencies]` 追加：

```toml
chardetng = "0.1"
encoding-rs = "0.8"
```

- [ ] **Step 2: 写失败测试**

创建 `openitgo-parser/src/text_encoding.rs`，先只写测试与空实现：

```rust
use crate::traits::ParseError;
use std::path::Path;

/// Read a text file as UTF-8, detecting legacy encodings (GBK/GB18030/Big5/...)
/// when the bytes are not valid UTF-8. Invalid sequences are replaced (lossy).
pub fn read_text_lossy(path: &Path) -> Result<String, ParseError> {
    let bytes = std::fs::read(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;
    Ok(decode_text_bytes(&bytes))
}

/// Decode raw bytes: UTF-8 fast path (optional BOM stripped), otherwise
/// chardetng detection + encoding_rs transcoding.
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_utf8() {
        assert_eq!(decode_text_bytes("你好，世界".as_bytes()), "你好，世界");
    }

    #[test]
    fn test_decode_utf8_bom_stripped() {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend("第一章".as_bytes());
        assert_eq!(decode_text_bytes(&bytes), "第一章");
    }

    #[test]
    fn test_decode_gbk() {
        let (bytes, _, _) = encoding_rs::GBK.encode("第一章 新的开始\n\n他睁开了眼睛。");
        assert_eq!(decode_text_bytes(&bytes), "第一章 新的开始\n\n他睁开了眼睛。");
    }

    #[test]
    fn test_decode_gb18030() {
        let (bytes, _, _) = encoding_rs::GB18030.encode("第二章 另一个故事的开端");
        assert_eq!(decode_text_bytes(&bytes), "第二章 另一个故事的开端");
    }

    #[test]
    fn test_decode_big5() {
        let (bytes, _, _) = encoding_rs::BIG5.encode("第三章 命中注定我愛你，睜開眼之後");
        assert_eq!(decode_text_bytes(&bytes), "第三章 命中注定我愛你，睜開眼之後");
    }

    #[test]
    fn test_decode_invalid_bytes_lossy_no_panic() {
        let text = decode_text_bytes(&[0xff, 0xfe, 0x41, 0x80]);
        assert!(!text.is_empty());
    }

    #[test]
    fn test_read_text_lossy_missing_file() {
        assert!(read_text_lossy(Path::new("/nonexistent/nope.txt")).is_err());
    }

    #[test]
    fn test_read_text_lossy_gbk_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gbk.txt");
        let (bytes, _, _) = encoding_rs::GBK.encode("第一章 风起");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_text_lossy(&path).unwrap(), "第一章 风起");
    }
}
```

注：`tempfile` 已在 `[dev-dependencies]`。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p openitgo-parser text_encoding`
Expected: FAIL（`todo!()` panic / not implemented）

- [ ] **Step 4: 实现**

把 `decode_text_bytes` 的 `todo!()` 替换为：

```rust
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    let stripped = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(stripped) {
        return text.to_string();
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p openitgo-parser text_encoding`
Expected: PASS（8 个测试）。若 GBK/Big5 检测不准（短文本误判），把测试字符串加长后重试；不要降低断言标准。

- [ ] **Step 6: 注册模块并替换 3 处调用点**

`openitgo-parser/src/lib.rs` 第 8 行后插入：

```rust
pub mod text_encoding;
```

`openitgo-parser/src/txt.rs`：将

```rust
        let text =
            fs::read_to_string(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;
```

替换为：

```rust
        let text = crate::text_encoding::read_text_lossy(path)?;
```

并删除文件顶部不再使用的 `use std::fs;`（若还有其他用途则保留——本文件无）。

`openitgo-parser/src/markdown.rs`：同样的 `fs::read_to_string` 调用替换为 `crate::text_encoding::read_text_lossy(path)?;`，清理无用的 `fs` import。

`openitgo-parser/src/html.rs`：将

```rust
        let text =
            std::fs::read_to_string(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;
```

替换为：

```rust
        let text = crate::text_encoding::read_text_lossy(path)?;
```

- [ ] **Step 7: 集成测试（GBK 分章）**

`openitgo-parser/tests/ebook_integration.rs` 末尾追加：

```rust
#[test]
fn test_parse_gbk_txt_with_headings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book_gbk.txt");
    let (bytes, _, _) = encoding_rs::GBK.encode(
        "第一章 风起\n\n他睁开眼睛，发现自己躺在陌生的床上。\n\n第二章 云涌\n\n她合上书本，望向窗外。",
    );
    std::fs::write(&path, &bytes).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("第一章 风起"));
    assert_eq!(ebook.chapters[1].title.as_deref(), Some("第二章 云涌"));
}
```

- [ ] **Step 8: 全量验证并提交**

Run: `cargo fmt --all && cargo test -p openitgo-parser && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

```bash
git add openitgo-parser/
git commit -m "feat(parser): TXT/Markdown 编码检测——GBK/GB18030/Big5 经 chardetng+encoding_rs 转 UTF-8"
```

---

### Task 3: EPUB 资源通道与内嵌字体（TODO 38/39）

**Files:**
- Modify: `openitgo-parser/Cargo.toml`（加 `percent-encoding`）、`openitgo-parser/src/html.rs`
- Modify: `openitgo-app/src/ebook_renderer.rs`（协议 handler）
- Test: `openitgo-parser/src/html.rs`（模块内）、`openitgo-parser/tests/ebook_integration.rs`、`openitgo-app/src/ebook_renderer.rs`（模块内）

**Interfaces:**
- Consumes: 无（独立于 Task 2，但按顺序提交）
- Produces（后续任务与协议 handler 依赖的精确签名）:
  - `pub fn rewrite_epub_urls(html: &str, chapter_href: &str) -> String`（html.rs）
  - `pub fn read_epub_resource(ebook: &Ebook, res_path: &str) -> Option<(String, Vec<u8>)>`（html.rs；返回 `(mime, bytes)`）
  - `pub fn extract_font_face_css(ebook: &Ebook, doc: &mut epub::doc::EpubDoc<std::io::BufReader<std::fs::File>>) -> String`（html.rs）
  - `pub fn extract_font_faces(css: &str, css_href: &str) -> String`（html.rs，纯函数）
  - `fn decode_res_path(path: &str) -> Option<String>`（ebook_renderer.rs，私有）
  - `render_chapter_html` 的 EPUB 分支输出变为 `[<style>@font-face…</style>] + 改写后 HTML`（行为变更，调用方不变）

- [ ] **Step 1: 添加依赖**

`openitgo-parser/Cargo.toml` 的 `[dependencies]` 追加：

```toml
percent-encoding = "2.3"
```

- [ ] **Step 2: 写失败测试（URL 改写与路径解析）**

`openitgo-parser/src/html.rs` 的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn test_chapter_dir() {
        assert_eq!(chapter_dir("OEBPS/Text/ch1.xhtml"), "OEBPS/Text");
        assert_eq!(chapter_dir("ch1.xhtml"), "");
    }

    #[test]
    fn test_resolve_resource_path() {
        assert_eq!(
            resolve_resource_path("OEBPS/Text", "../Images/pic.png"),
            "OEBPS/Images/pic.png"
        );
        assert_eq!(
            resolve_resource_path("OEBPS/Text", "img/p.png"),
            "OEBPS/Text/img/p.png"
        );
        assert_eq!(resolve_resource_path("", "a/b.png"), "a/b.png");
        // 越级 .. 在根处钳制
        assert_eq!(resolve_resource_path("A", "../../x.png"), "x.png");
        assert_eq!(resolve_resource_path("A/B", "./c.png"), "A/B/c.png");
    }

    #[test]
    fn test_rewrite_epub_urls_img_relative() {
        let html = r#"<p><img src="../Images/pic.png"/></p>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains(r#"src="ebook://res/OEBPS/Images/pic.png""#), "got: {out}");
    }

    #[test]
    fn test_rewrite_epub_urls_single_quotes_and_space_encoding() {
        let html = "<img src='images/a b.png'/>";
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains("src='ebook://res/OEBPS/Text/images/a%20b.png'"), "got: {out}");
    }

    #[test]
    fn test_rewrite_epub_urls_skips_absolute_data_fragment() {
        let html = r#"<img src="data:image/png;base64,AAAA"/><img src="https://example.com/a.png"/><img src="#frag"/>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert_eq!(out, html);
    }

    #[test]
    fn test_rewrite_epub_urls_xlink_href() {
        let html = r#"<svg><image xlink:href="../Images/p.svg"/></svg>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains(r#"xlink:href="ebook://res/OEBPS/Images/p.svg""#), "got: {out}");
    }

    #[test]
    fn test_rewrite_epub_urls_ignores_prose_and_data_src() {
        // 标签外的 src=" 不改写；data-src= 不被误认为 src=
        let html = r#"<p>see src="x.png" here</p><img data-src="y.png" src="z.png"/>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains(r#"see src="x.png" here"#), "got: {out}");
        assert!(out.contains(r#"data-src="y.png""#), "got: {out}");
        assert!(out.contains(r#"src="ebook://res/OEBPS/Text/z.png""#), "got: {out}");
    }

    #[test]
    fn test_rewrite_epub_urls_non_ascii_filename() {
        let html = r#"<img src="图片.png"/>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains("ebook://res/OEBPS/Text/%E5%9B%BE%E7%89%87.png"), "got: {out}");
    }
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p openitgo-parser html::tests::test_chapter_dir`
Expected: FAIL（`chapter_dir` 未定义，编译错误）

- [ ] **Step 4: 实现路径解析与 URL 改写**

`openitgo-parser/src/html.rs` 中（`sanitize_epub_html` 之后）新增：

```rust
/// Percent-encode set for `ebook://res/` paths: keep path-safe ASCII readable,
/// encode everything else (spaces, non-ASCII, ...).
const RES_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Directory portion of an archive path ("OEBPS/Text/ch1.xhtml" -> "OEBPS/Text").
fn chapter_dir(href: &str) -> String {
    match href.rfind('/') {
        Some(pos) => href[..pos].to_string(),
        None => String::new(),
    }
}

/// Resolve `rel` against `dir`, normalizing `.` and `..` (clamped at the
/// archive root) and dropping empty segments.
fn resolve_resource_path(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Build an absolute `ebook://res/` URL for a relative resource reference.
/// Returns `None` for references that must stay untouched (absolute URLs,
/// `data:` URIs, fragments, empty values).
fn to_res_url(dir: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.starts_with('#') {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if lower.contains("://") || lower.starts_with("data:") {
        return None;
    }
    let resolved = resolve_resource_path(dir, v);
    if resolved.is_empty() {
        return None;
    }
    let encoded = percent_encoding::utf8_percent_encode(&resolved, RES_ENCODE_SET);
    Some(format!("ebook://res/{}", encoded))
}

/// Rewrite relative resource references (`src=` / `xlink:href=` attribute
/// values) in sanitized EPUB chapter HTML to absolute `ebook://res/` URLs,
/// resolved against the chapter's directory inside the archive. Run after
/// [`sanitize_epub_html`], which guarantees tags and quotes are balanced.
pub fn rewrite_epub_urls(html: &str, chapter_href: &str) -> String {
    const ATTRS: [&str; 2] = ["src=", "xlink:href="];
    let dir = chapter_dir(chapter_href);
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    let mut in_tag = false;
    let mut in_quote = None::<u8>;
    while i < html.len() {
        let b = bytes[i];
        // Copy multi-byte UTF-8 chars whole; byte-wise ASCII handling would
        // panic on char-boundary slicing.
        if b >= 0x80 {
            let ch = html[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if let Some(q) = in_quote {
            out.push_str(&html[i..i + 1]);
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => {
                in_quote = Some(b);
                out.push_str(&html[i..i + 1]);
                i += 1;
            }
            b'<' => {
                in_tag = true;
                out.push_str(&html[i..i + 1]);
                i += 1;
            }
            b'>' => {
                in_tag = false;
                out.push_str(&html[i..i + 1]);
                i += 1;
            }
            _ if in_tag => {
                let attr = ATTRS.iter().find(|a| lower[i..].starts_with(**a));
                // Attribute names must start at a boundary (whitespace or `/`)
                // so `data-src=` is not mistaken for `src=`.
                let boundary =
                    i > 0 && (bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'/');
                match attr {
                    Some(name) if boundary => {
                        out.push_str(&html[i..i + name.len()]);
                        i += name.len();
                        while i < html.len() && bytes[i].is_ascii_whitespace() {
                            out.push_str(&html[i..i + 1]);
                            i += 1;
                        }
                        let Some(&q) = bytes.get(i) else {
                            continue;
                        };
                        if q != b'"' && q != b'\'' {
                            continue;
                        }
                        out.push_str(&html[i..i + 1]);
                        i += 1;
                        let value_start = i;
                        while i < html.len() && bytes[i] != q {
                            i += 1;
                        }
                        let value = &html[value_start..i];
                        match to_res_url(&dir, value) {
                            Some(url) => out.push_str(&url),
                            None => out.push_str(value),
                        }
                        // The closing quote is emitted by the main loop.
                    }
                    _ => {
                        out.push_str(&html[i..i + 1]);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push_str(&html[i..i + 1]);
                i += 1;
            }
        }
    }
    out
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p openitgo-parser html::tests`
Expected: 新旧测试全部 PASS。

- [ ] **Step 6: 写失败测试（@font-face 提取）**

`#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn test_extract_font_faces_basic() {
        let css = r#"@font-face { font-family: "My"; src: url("../Fonts/my.ttf"); }"#;
        let out = extract_font_faces(css, "OEBPS/Styles/s.css");
        assert!(out.contains("@font-face"), "got: {out}");
        assert!(out.contains(r#"url("ebook://res/OEBPS/Fonts/my.ttf")"#), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_unquoted_url() {
        let css = "@font-face { src: url(fonts/a.woff2) format('woff2'); }";
        let out = extract_font_faces(css, "OEBPS/Styles/s.css");
        assert!(out.contains(r#"url("ebook://res/OEBPS/Styles/fonts/a.woff2")"#), "got: {out}");
        assert!(out.contains("format('woff2')"), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_absolute_and_data_untouched() {
        let css = r#"@font-face { src: url("https://x.com/a.ttf"); } @font-face { src: url("data:font/ttf;base64,AA"); }"#;
        let out = extract_font_faces(css, "OEBPS/s.css");
        assert!(out.contains(r#"url("https://x.com/a.ttf")"#), "got: {out}");
        assert!(out.contains(r#"url("data:font/ttf;base64,AA")"#), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_drops_non_font_rules() {
        let css = "body { color: red; } p { margin: 0; }";
        assert_eq!(extract_font_faces(css, "OEBPS/s.css"), "");
    }

    #[test]
    fn test_extract_font_faces_multiple_blocks() {
        let css = "@font-face { src: url(a.ttf); } body { color: red; } @font-face { src: url(b.otf); }";
        let out = extract_font_faces(css, "s.css");
        assert_eq!(out.matches("@font-face").count(), 2);
        assert!(out.contains(r#"url("ebook://res/a.ttf")"#), "got: {out}");
        assert!(out.contains(r#"url("ebook://res/b.otf")"#), "got: {out}");
        assert!(!out.contains("color"), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_unbalanced_block_stops() {
        let css = "@font-face { src: url(a.ttf); ";
        assert_eq!(extract_font_faces(css, "s.css"), "");
    }
```

- [ ] **Step 7: 运行测试确认失败**

Run: `cargo test -p openitgo-parser html::tests::test_extract_font_faces`
Expected: FAIL（函数未定义）

- [ ] **Step 8: 实现 @font-face 提取与 CSS url 改写**

`openitgo-parser/src/html.rs` 新增（`rewrite_epub_urls` 之后）：

```rust
/// Extract all `@font-face { ... }` blocks from a stylesheet, rewriting
/// relative `url(...)` references (resolved against the stylesheet's own
/// directory) to `ebook://res/` URLs. Every other rule is dropped so book
/// layout CSS never reaches the paginator.
pub fn extract_font_faces(css: &str, css_href: &str) -> String {
    let dir = chapter_dir(css_href);
    let lower = css.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0usize;
    while let Some(pos) = lower[i..].find("@font-face") {
        let start = i + pos;
        let mut j = start + "@font-face".len();
        while j < css.len() && css.as_bytes()[j].is_ascii_whitespace() {
            j += 1;
        }
        if css.as_bytes().get(j) != Some(&b'{') {
            i = j.max(start + 1);
            continue;
        }
        // Brace-match the block, respecting quotes.
        let mut depth = 0usize;
        let mut in_quote = None::<u8>;
        let mut end = css.len();
        let mut k = j;
        while k < css.len() {
            let b = css.as_bytes()[k];
            match in_quote {
                Some(q) => {
                    if b == q {
                        in_quote = None;
                    }
                }
                None => match b {
                    b'"' | b'\'' => in_quote = Some(b),
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = k;
                            break;
                        }
                    }
                    _ => {}
                },
            }
            k += 1;
        }
        if depth != 0 {
            break; // Unbalanced block; give up on the rest of the file.
        }
        out.push_str(&rewrite_css_urls(&css[start..=end], &dir));
        out.push('\n');
        i = end + 1;
    }
    out
}

/// Rewrite every relative `url(...)` in a CSS block to `ebook://res/` URLs.
/// Absolute/data references are copied untouched.
fn rewrite_css_urls(block: &str, dir: &str) -> String {
    let lower = block.to_ascii_lowercase();
    let mut out = String::with_capacity(block.len());
    let mut i = 0usize;
    while i < block.len() {
        let b = block.as_bytes()[i];
        if b >= 0x80 {
            let ch = block[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if !lower[i..].starts_with("url(") {
            out.push_str(&block[i..i + 1]);
            i += 1;
            continue;
        }
        let mut j = i + 4;
        while j < block.len() && block.as_bytes()[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = match block.as_bytes().get(j) {
            Some(b'"') | Some(b'\'') => Some(block.as_bytes()[j]),
            _ => None,
        };
        let value_start = if quote.is_some() { j + 1 } else { j };
        let mut k = value_start;
        while k < block.len() {
            let b = block.as_bytes()[k];
            let done = match quote {
                Some(q) => b == q,
                None => b == b')' || b.is_ascii_whitespace(),
            };
            if done {
                break;
            }
            k += 1;
        }
        let value = &block[value_start..k];
        if let Some(url) = to_res_url(dir, value) {
            out.push_str("url(\"");
            out.push_str(&url);
            out.push_str("\")");
        } else {
            let close = if quote.is_some() { k + 1 } else { k };
            out.push_str(&block[i..close.min(block.len())]);
        }
        i = if quote.is_some() {
            (k + 1).min(block.len())
        } else {
            k
        };
    }
    out
}

/// Collect rewritten `@font-face` rules from every CSS resource in the book.
/// Returns an empty string when the book declares no fonts (or no CSS).
pub fn extract_font_face_css(
    ebook: &Ebook,
    doc: &mut epub::doc::EpubDoc<std::io::BufReader<std::fs::File>>,
) -> String {
    let mut out = String::new();
    for res in &ebook.resources {
        if !res.mime_type.contains("css") {
            continue;
        }
        if let Some(css) = doc.get_resource_str_by_path(&res.href) {
            out.push_str(&extract_font_faces(&css, &res.href));
        }
    }
    out
}
```

- [ ] **Step 9: 运行测试确认通过**

Run: `cargo test -p openitgo-parser html::tests`
Expected: PASS。

- [ ] **Step 10: 接入 render_chapter_html + 资源读取函数**

`openitgo-parser/src/html.rs` 的 `render_chapter_html` EPUB 分支，把

```rust
        let html = doc
            .get_resource_str_by_path(&chapter.href)
            .ok_or(ParseError::NoPages)?;
        Ok(sanitize_epub_html(&html))
```

替换为：

```rust
        let html = doc
            .get_resource_str_by_path(&chapter.href)
            .ok_or(ParseError::NoPages)?;
        let rewritten = rewrite_epub_urls(&sanitize_epub_html(&html), &chapter.href);
        let fonts = extract_font_face_css(ebook, &mut doc);
        if fonts.is_empty() {
            Ok(rewritten)
        } else {
            Ok(format!("<style>{}</style>{}", fonts, rewritten))
        }
```

同文件新增资源读取入口：

```rust
/// Read a binary resource from an EPUB archive by its full path, returning
/// `(mime, bytes)`. MIME comes from the parsed manifest, with an
/// extension-based fallback. Returns `None` for non-EPUB books or missing
/// resources.
pub fn read_epub_resource(ebook: &Ebook, res_path: &str) -> Option<(String, Vec<u8>)> {
    if !is_epub_path(&ebook.path) {
        return None;
    }
    let mut doc = epub::doc::EpubDoc::new(&ebook.path).ok()?;
    let bytes = doc.get_resource_by_path(res_path)?;
    let mime = ebook
        .resources
        .iter()
        .find(|r| r.href == res_path)
        .map(|r| r.mime_type.clone())
        .unwrap_or_else(|| guess_mime(res_path).to_string());
    Some((mime, bytes))
}

fn guess_mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("css") => "text/css",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
```

注意：`extract_font_face_css(ebook, ...)` 的第一个参数 `ebook` 当前在 `render_chapter_html` 中可用（函数参数名为 `ebook`），直接传入即可。

- [ ] **Step 11: 集成测试（带资源的 EPUB）**

`openitgo-parser/tests/ebook_integration.rs` 末尾追加（复用该文件已有的 zip 构建模式）：

```rust
fn build_epub_with_resources() -> Vec<u8> {
    use std::io::Cursor;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("mimetype", options).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    zip.start_file("META-INF/container.xml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#,
    )
    .unwrap();

    zip.start_file("OEBPS/content.opf", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Resource Book</dc:title>
    <dc:identifier id="id">res-1</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="Images/pic.png" media-type="image/png"/>
    <item id="font" href="Fonts/f.ttf" media-type="font/ttf"/>
    <item id="css" href="Styles/s.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"#,
    )
    .unwrap();

    zip.start_file("OEBPS/Text/ch1.xhtml", options).unwrap();
    zip.write_all(
        br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head><body><p>before</p><img src="../Images/pic.png"/><p>after</p></body></html>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/Images/pic.png", options).unwrap();
    zip.write_all(b"\x89PNG\r\n\x1a\nFAKE").unwrap();

    zip.start_file("OEBPS/Fonts/f.ttf", options).unwrap();
    zip.write_all(b"\x00\x01\x00\x00FAKETTF").unwrap();

    zip.start_file("OEBPS/Styles/s.css", options).unwrap();
    zip.write_all(
        br#"body { color: red; }
@font-face { font-family: "Embedded"; src: url("../Fonts/f.ttf"); }
"#,
    )
    .unwrap();

    zip.finish().unwrap().into_inner()
}

#[test]
fn test_epub_chapter_rewrites_img_and_injects_font_face() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("res.epub");
    std::fs::write(&path, build_epub_with_resources()).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    let html = openitgo_parser::html::render_chapter_html(&ebook, 0).unwrap();
    assert!(html.contains("ebook://res/OEBPS/Images/pic.png"), "got: {html}");
    assert!(html.contains("@font-face"), "got: {html}");
    assert!(html.contains("ebook://res/OEBPS/Fonts/f.ttf"), "got: {html}");
    assert!(!html.contains("color: red"), "book layout CSS must be dropped: {html}");
}

#[test]
fn test_read_epub_resource() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("res.epub");
    std::fs::write(&path, build_epub_with_resources()).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    let (mime, bytes) =
        openitgo_parser::html::read_epub_resource(&ebook, "OEBPS/Images/pic.png").unwrap();
    assert_eq!(mime, "image/png");
    assert!(bytes.starts_with(b"\x89PNG"));
    assert!(openitgo_parser::html::read_epub_resource(&ebook, "OEBPS/nope.png").is_none());
}
```

（若该文件已有 `use std::io::Write;` 与 zip 构建辅助，直接复用；上表为自包含版本。）

- [ ] **Step 12: 运行 parser 全部测试**

Run: `cargo test -p openitgo-parser`
Expected: 全部 PASS。

- [ ] **Step 13: app 侧协议通道——失败测试**

`openitgo-app/src/ebook_renderer.rs` 的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn test_decode_res_path() {
        assert_eq!(
            decode_res_path("/res/OEBPS/Images/pic.png"),
            Some("OEBPS/Images/pic.png".to_string())
        );
        assert_eq!(
            decode_res_path("/res/OEBPS/a%20b.png"),
            Some("OEBPS/a b.png".to_string())
        );
        assert_eq!(decode_res_path("/reader"), None);
        assert_eq!(decode_res_path("/res/"), None);
        assert_eq!(decode_res_path("/other/x"), None);
    }
```

Run: `cargo test -p openitgo-app ebook_renderer`
Expected: FAIL（`decode_res_path` 未定义）

- [ ] **Step 14: 实现协议通道**

`openitgo-app/src/ebook_renderer.rs` 中，`handle_ebook_protocol` 的末尾空 200 返回**之前**插入：

```rust
    if let Some(res_path) = decode_res_path(path) {
        match openitgo_parser::html::read_epub_resource(&state.ebook, &res_path) {
            Some((mime, bytes)) => {
                return wry::http::Response::builder()
                    .header("Content-Type", mime)
                    .header("Cache-Control", "no-cache, no-store, must-revalidate")
                    .body(bytes.into())
                    .unwrap();
            }
            None => {
                eprintln!("EbookRenderer: resource not found: {res_path}");
            }
        }
    }
```

并在 `handle_ebook_protocol` 之后新增：

```rust
/// Extract and percent-decode the archive path from an `ebook://res/<path>`
/// request URI. Returns `None` for non-resource paths.
fn decode_res_path(path: &str) -> Option<String> {
    let raw = path.strip_prefix("/res/")?;
    if raw.is_empty() {
        return None;
    }
    Some(
        percent_encoding::percent_decode_str(raw)
            .decode_utf8()
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| raw.to_string()),
    )
}
```

（`percent_encoding` 已是 openitgo-app 的依赖，`views/ebook.rs` 在用。）

- [ ] **Step 15: 全量验证并提交**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

```bash
git add openitgo-parser/ openitgo-app/src/ebook_renderer.rs
git commit -m "feat(parser,app): EPUB 内嵌图片与字体——ebook://res/ 资源通道 + URL 改写 + @font-face 提取注入"
```

---

### Task 4: font_family 设置 UI 与校验（TODO 40）

**Files:**
- Modify: `openitgo-storage/src/models.rs`（validate/clamp + 测试）
- Modify: `openitgo-app/src/views/settings.rs:135-197`（`ebook_settings_ui`）

**Interfaces:**
- Consumes: 无
- Produces: `Settings::validate` 新增规则——`ebook.font_family` 去空白后非空；`Settings::clamp` 将空值复位为 `"system-ui"`

- [ ] **Step 1: 写失败测试**

`openitgo-storage/src/models.rs` 的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn test_settings_validate_rejects_empty_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = "   ".to_string();
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_settings_clamp_restores_default_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = String::new();
        s.clamp();
        assert_eq!(s.ebook.font_family, "system-ui");
    }
```

Run: `cargo test -p openitgo-storage font_family`
Expected: FAIL（validate 目前不检查 font_family）

- [ ] **Step 2: 实现校验与钳制**

`Settings::validate` 中 ebook 段落（`margin_vertical` 检查之后）追加：

```rust
        if self.ebook.font_family.trim().is_empty() {
            return Err("ebook.font_family must not be empty".to_string());
        }
```

`Settings::clamp` 中 ebook 段落追加：

```rust
        if self.ebook.font_family.trim().is_empty() {
            self.ebook.font_family = "system-ui".to_string();
        }
```

Run: `cargo test -p openitgo-storage font_family`
Expected: PASS。

- [ ] **Step 3: 设置面板字体下拉框**

`openitgo-app/src/views/settings.rs` 的 `ebook_settings_ui` 中，"阅读模式" ComboBox 之后插入：

```rust
        ui.label("字体");
        let current_font = settings.ebook.font_family.clone();
        egui::ComboBox::from_id_salt("ebook_font_family")
            .selected_text(&current_font)
            .show_ui(ui, |ui| {
                const PRESETS: &[&str] = &[
                    "system-ui",
                    "serif",
                    "sans-serif",
                    "monospace",
                    "PingFang SC",
                    "Songti SC",
                    "Kaiti SC",
                    "Hiragino Sans GB",
                ];
                for preset in PRESETS {
                    ui.selectable_value(
                        &mut settings.ebook.font_family,
                        preset.to_string(),
                        *preset,
                    );
                }
                if !PRESETS.contains(&current_font.as_str()) {
                    ui.selectable_value(
                        &mut settings.ebook.font_family,
                        current_font.clone(),
                        current_font.clone(),
                    );
                }
            });
```

- [ ] **Step 4: 全量验证并提交**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

```bash
git add openitgo-storage/src/models.rs openitgo-app/src/views/settings.rs
git commit -m "feat(storage,app): 电子书字体设置——font_family 下拉框与空值校验/钳制"
```

---

### Task 5: 电子书搜索 UI（TODO 37）

**Files:**
- Modify: `openitgo-app/src/ebook_renderer_template.rs`（JS：search IPC 上报 + 重排自动重放 + 模板测试）
- Modify: `openitgo-app/src/ebook_renderer.rs`（IPC 消息、状态、search_state()、去 dead_code）
- Modify: `openitgo-app/src/views/ebook.rs`（SearchState、OpenEbook 字段、视图方法）
- Modify: `openitgo-app/src/app.rs`（工具栏按钮、`render_ebook_search_bar`、`render_ebook` 接线、Cmd+F）

**Interfaces:**
- Consumes: Task 1–4 的基线
- Produces:
  - `EbookRenderer::search_state(&self) -> (usize, i64)` — `(命中数, 当前序号)`，无搜索时 `(0, -1)`
  - `EbookView::{search_visible, toggle_search, close_search, find_next, find_prev}` 与 `SearchState { visible, query }`、`SearchState::{open, close, toggle, take_focus_request}`
  - JS IPC 消息：`{type:'search', count: usize, active: number}`（-1 表示无活动项）

- [ ] **Step 1: JS 模板失败测试**

`openitgo-app/src/ebook_renderer_template.rs` 的测试模块内追加（沿用现有 split 断言风格）：

```rust
    #[test]
    fn test_search_reports_ipc_and_replays_after_relayout() {
        let html = reader_html(&EbookSettings::default(), 3);

        let set_active = html
            .split("function setSearchActiveIndex(index)")
            .nth(1)
            .expect("setSearchActiveIndex not found")
            .split("function findText")
            .next()
            .expect("setSearchActiveIndex body not found");
        assert!(
            set_active.contains("type: 'search'"),
            "setSearchActiveIndex should report search state via IPC"
        );

        let find_text = html
            .split("function findText(query)")
            .nth(1)
            .expect("findText not found")
            .split("function findNext")
            .next()
            .expect("findText body not found");
        assert!(
            find_text.contains("count: 0, active: -1"),
            "findText should report zero matches via IPC"
        );

        assert!(
            html.contains("function restoreSearchAfterLayout"),
            "restoreSearchAfterLayout should exist"
        );

        let apply_settings = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("// Lightweight adjacent-chapter preload")
            .next()
            .expect("applySettings body not found");
        assert!(
            apply_settings.contains("restoreSearchAfterLayout()"),
            "applySettings should replay search after relayout"
        );

        let load_chapter = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter body not found");
        assert!(
            load_chapter.contains("restoreSearchAfterLayout()"),
            "loadChapter should replay search in the new chapter"
        );

        let resize = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("loadChapter(0, 0);")
            .next()
            .expect("resize handler end not found");
        assert!(
            resize.contains("restoreSearchAfterLayout()"),
            "resize handler should replay search after relayout"
        );
    }
```

注意：测试断言针对 `reader_html()` 的**渲染输出**（JS 花括号在输出中为单写），与现有模板测试风格一致；所有 split 标记均取自渲染后文本。

Run: `cargo test -p openitgo-app ebook_renderer_template`
Expected: FAIL（新断言不成立）

- [ ] **Step 2: JS 实现（3 处编辑）**

**编辑 1** — `setSearchActiveIndex` 末尾（模板源码约 593-598 行，`goToCharOffset(offset);` 所在 if/else 之后、函数收尾 `}}` 之前）追加一行：

```js
  sendIpc({{ type: 'search', count: ebookSearchHighlights.length, active: idx }});
```

**编辑 2** — `findText` 末尾，把

```js
  if (ebookSearchHighlights.length > 0) {{
    setSearchActiveIndex(0);
  }}
}}
```

改为：

```js
  if (ebookSearchHighlights.length > 0) {{
    setSearchActiveIndex(0);
  }} else {{
    sendIpc({{ type: 'search', count: 0, active: -1 }});
  }}
}}

// Re-run the active search after a relayout (settings change, resize, chapter
// switch) replaced the chapter DOM and dropped all highlight marks.
function restoreSearchAfterLayout() {{
  if (!ebookSearchQuery) return;
  findText(ebookSearchQuery);
}}
```

**编辑 3** — 三个重排完成点调用 `restoreSearchAfterLayout();`：

a) `applySettings` 的 `if (currentChapterHtml) {{` 块末尾（scroll/paginated 恢复的 if/else 之后、块收尾 `}}` 之前）插入：

```js
    restoreSearchAfterLayout();
```

b) `loadChapter` 的 `preloadChapter(currentChapter + 1);` 之后插入：

```js
    restoreSearchAfterLayout();
```

c) resize 处理器：同宽分支的 `goToSpread(paginatorState.currentSpread);` 之后（`return;` 之前）插入 `restoreSearchAfterLayout();`；完整重排分支的 scroll/paginated 恢复 if/else 之后（`}}, RESIZE_DEBOUNCE_MS);` 之前）插入 `restoreSearchAfterLayout();`。

Run: `cargo test -p openitgo-app ebook_renderer_template`
Expected: 全部 PASS（含既有模板测试）。

- [ ] **Step 3: Rust IPC——失败测试**

`openitgo-app/src/ebook_renderer.rs` 的测试模块内追加：

```rust
    fn test_state() -> Arc<Mutex<RendererState>> {
        use openitgo_core::ebook::EbookChapter;
        use std::path::PathBuf;
        let ebook = Ebook {
            id: "t".to_string(),
            title: "T".to_string(),
            path: PathBuf::from("/tmp/t.epub"),
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters: vec![EbookChapter {
                index: 0,
                id: "c".to_string(),
                href: "c.xhtml".to_string(),
                title: None,
            }],
        };
        Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }))
    }

    #[test]
    fn test_ipc_search_message_updates_state() {
        let state = test_state();
        let ctx = egui::Context::default();
        let msg: JsToRust =
            serde_json::from_str(r#"{"type":"search","count":5,"active":2}"#).unwrap();
        handle_ipc_message(msg, &state, &ctx);
        let s = state.lock().unwrap();
        assert_eq!(s.search_count, 5);
        assert_eq!(s.search_active, 2);
    }

    #[test]
    fn test_ipc_search_message_defaults() {
        let state = test_state();
        let ctx = egui::Context::default();
        let msg: JsToRust = serde_json::from_str(r#"{"type":"search"}"#).unwrap();
        handle_ipc_message(msg, &state, &ctx);
        let s = state.lock().unwrap();
        assert_eq!(s.search_count, 0);
        assert_eq!(s.search_active, -1);
    }
```

Run: `cargo test -p openitgo-app ebook_renderer`
Expected: FAIL（`search_count` 字段不存在）

- [ ] **Step 4: Rust IPC 实现**

`openitgo-app/src/ebook_renderer.rs`：

a) `RendererState` 增加字段：

```rust
struct RendererState {
    ebook: Ebook,
    current_chapter: usize,
    char_offset: usize,
    current_spread: usize,
    total_spreads: usize,
    settings: EbookSettings,
    search_count: usize,
    search_active: i64,
}
```

`EbookRenderer::new` 的初始化相应追加 `search_count: 0, search_active: -1,`。

b) `JsToRust` 增加字段：

```rust
    count: Option<usize>,
    active: Option<i64>,
```

c) `handle_ipc_message` 在 error/debug 分支之后、position 处理之前插入：

```rust
    if msg.kind.as_str() == "search" {
        if let Ok(mut state) = state.lock() {
            state.search_count = msg.count.unwrap_or(0);
            state.search_active = msg.active.unwrap_or(-1);
        }
        repaint.request_repaint();
        return;
    }
```

d) 删除 `find_text`/`find_next`/`find_prev`/`clear_highlights` 上的 4 处 `#[allow(dead_code)]` 与"not yet wired"注释块，并在 `clear_highlights` 之后新增：

```rust
    /// Current search state as reported by the webview: `(match count, active
    /// index)`. `(0, -1)` when no search is active.
    pub fn search_state(&self) -> (usize, i64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.search_count, state.search_active)
    }
```

Run: `cargo test -p openitgo-app ebook_renderer`
Expected: PASS。

- [ ] **Step 5: SearchState 与视图方法——失败测试**

`openitgo-app/src/views/ebook.rs` 的测试模块内追加：

```rust
    #[test]
    fn test_search_state_open_close_toggle() {
        let mut s = SearchState::default();
        assert!(!s.visible);
        s.toggle();
        assert!(s.visible);
        assert!(s.take_focus_request());
        assert!(!s.take_focus_request());
        s.query = "test".to_string();
        s.close();
        assert!(!s.visible);
        assert!(s.query.is_empty());
    }

    #[test]
    fn test_ebook_view_search_methods_without_open_book() {
        let mut view = EbookView::default();
        assert!(!view.search_visible());
        view.toggle_search();
        view.close_search();
        view.find_next();
        view.find_prev();
        assert!(!view.search_visible());
    }
```

Run: `cargo test -p openitgo-app views::ebook`
Expected: FAIL（`SearchState` 未定义）

- [ ] **Step 6: SearchState 与视图方法实现**

`openitgo-app/src/views/ebook.rs`：

a) 新增结构体（`OpenEbook` 定义之前）：

```rust
/// Ebook full-text search bar state. Kept separate from the renderer so the
/// state machine is unit-testable without a WebView.
#[derive(Default)]
pub struct SearchState {
    pub visible: bool,
    pub query: String,
    focus_pending: bool,
}

impl SearchState {
    pub fn open(&mut self) {
        self.visible = true;
        self.focus_pending = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    /// Returns `true` exactly once after `open()` so the UI can focus the
    /// search input on the first frame it appears.
    pub fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_pending)
    }
}
```

b) `OpenEbook` 增加字段：

```rust
    pub search: SearchState,
```

`EbookView::open` 中构造处追加 `search: SearchState::default(),`；同时删除该函数上的 `#[allow(dead_code)]` 和 "Reserved for the future" 注释（它在 `app.rs` 的 `poll_ebook_opener` 中实际被调用）。测试 `test_open_ebook_has_current_spread_field` 中的构造同样追加 `search: SearchState::default(),`。

c) `EbookView` 新增方法（`toggle_toc` 之后）：

```rust
    pub fn search_visible(&self) -> bool {
        self.open.as_ref().map(|o| o.search.visible).unwrap_or(false)
    }

    pub fn toggle_search(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.search.toggle();
            if !open.search.visible {
                open.renderer.clear_highlights();
            }
        }
    }

    pub fn close_search(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.search.close();
            open.renderer.clear_highlights();
        }
    }

    pub fn find_next(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.find_next();
        }
    }

    pub fn find_prev(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.find_prev();
        }
    }
```

Run: `cargo test -p openitgo-app views::ebook`
Expected: PASS。

- [ ] **Step 7: 搜索条 UI 与工具栏按钮**

`openitgo-app/src/app.rs`：

a) `render_ebook_toolbar` 中，"目录"按钮之后插入：

```rust
                if ui.button("搜索").clicked() {
                    self.ebook_view.toggle_search();
                }
```

b) 新增方法（`render_ebook_toolbar` 之后）：

```rust
    fn render_ebook_search_bar(&mut self, ctx: &egui::Context) {
        if !self.ebook_view.search_visible() {
            return;
        }
        egui::TopBottomPanel::top("ebook_search_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("搜索:");
                let mut submitted = None::<bool>; // Some(true)=下一个, Some(false)=上一个
                if let Some(open) = self.ebook_view.open.as_mut() {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut open.search.query)
                            .id(egui::Id::new("ebook_search_input"))
                            .desired_width(240.0),
                    );
                    if open.search.take_focus_request() {
                        response.request_focus();
                    }
                    if response.changed() {
                        let q = open.search.query.clone();
                        open.renderer.find_text(&q);
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submitted = Some(!ui.input(|i| i.modifiers.shift));
                    }
                    let (count, active) = open.renderer.search_state();
                    let label = if count == 0 {
                        "0/0".to_string()
                    } else {
                        format!("{}/{}", active.max(0) as usize + 1, count)
                    };
                    ui.label(label);
                    if ui.add_enabled(count > 0, egui::Button::new("上一个")).clicked() {
                        open.renderer.find_prev();
                    }
                    if ui.add_enabled(count > 0, egui::Button::new("下一个")).clicked() {
                        open.renderer.find_next();
                    }
                    if ui.button("✕").clicked() {
                        open.search.close();
                        open.renderer.clear_highlights();
                    }
                }
                match submitted {
                    Some(true) => self.ebook_view.find_next(),
                    Some(false) => self.ebook_view.find_prev(),
                    None => {}
                }
            });
        });
    }
```

c) `render_ebook` 中，`if show_toolbar { self.render_ebook_toolbar(ctx); }` 之后插入（搜索条独立于工具栏显隐，全屏时也可用）：

```rust
        self.render_ebook_search_bar(ctx);
```

d) Cmd/Ctrl+F 唤起：`handle_keys`（或等价按键处理函数）的 `View::Ebook` 分支内最前面插入：

```rust
                if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.command) {
                    self.ebook_view.toggle_search();
                }
```

- [ ] **Step 8: 全量验证并提交**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

```bash
git add openitgo-app/src/ebook_renderer_template.rs openitgo-app/src/ebook_renderer.rs openitgo-app/src/views/ebook.rs openitgo-app/src/app.rs
git commit -m "feat(app): 电子书全文搜索——工具栏搜索条 + IPC 命中计数 + 重排自动重放"
```

---

### Task 6: 电子书快捷键补全（TODO 42）

**Files:**
- Modify: `openitgo-app/src/app.rs:1792-1799`（`View::Ebook` 按键分支）
- Test: `openitgo-storage/src/models.rs`（默认绑定契约）

**Interfaces:**
- Consumes: Task 5 的 `EbookView::{search_visible, close_search}`
- Produces: 无新 API；行为变更——电子书视图支持 `back_to_library`/`page_down`/`page_up`，搜索条可见时 Escape 优先关闭搜索

- [ ] **Step 1: 写失败测试（默认绑定契约）**

`openitgo-storage/src/models.rs` 的测试模块内追加：

```rust
    #[test]
    fn test_default_shortcuts_cover_ebook_actions() {
        let s = Shortcuts::default();
        assert!(s.back_to_library.contains(&"Escape".to_string()));
        assert!(s.page_down.contains(&"PageDown".to_string()));
        assert!(s.page_up.contains(&"PageUp".to_string()));
    }
```

Run: `cargo test -p openitgo-storage shortcuts`
Expected: PASS（这是对现有契约的固化测试，防止后续改动破坏 Task 6 依赖的默认值；若 FAIL 说明默认绑定已变，需同步 Task 6 行为）。

- [ ] **Step 2: 修改 View::Ebook 按键分支**

`openitgo-app/src/app.rs` 的按键处理中，把

```rust
            View::Ebook => {
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                    self.ebook_view.next_page();
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                    self.ebook_view.prev_page();
                }
            }
```

替换为：

```rust
            View::Ebook => {
                if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.command) {
                    self.ebook_view.toggle_search();
                }
                // 文本框聚焦时不响应翻页类全局键，避免与输入冲突（如搜索框
                // 里按 Space）。
                if !ctx.wants_keyboard_input() {
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                        self.ebook_view.next_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                        self.ebook_view.prev_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_down) {
                        self.ebook_view.next_page();
                    }
                    if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_up) {
                        self.ebook_view.prev_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.back_to_library) {
                    if self.ebook_view.search_visible() {
                        self.ebook_view.close_search();
                    } else {
                        self.current_view = View::Library;
                    }
                }
            }
```

注意：若 Task 5 已在该分支顶部加了 Cmd+F 处理，保留一份即可（去重）。

- [ ] **Step 3: 全量验证并提交**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

```bash
git add openitgo-app/src/app.rs openitgo-storage/src/models.rs
git commit -m "feat(app): 电子书快捷键补全——Escape 返回/关闭搜索、PageUp/PageDown/Space 翻页、Cmd+F 搜索"
```

---

### Task 7: 收尾——流水线、打包、走查清单、文档

**Files:**
- Modify: `TODO.md`（勾选 36–42）、`CHANGELOG.md`（Unreleased 条目）

**Interfaces:**
- Consumes: Task 1–6 全部提交
- Produces: 可人工走查的 .app 与走查清单

- [ ] **Step 1: 完整流水线最终确认**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全绿。

- [ ] **Step 2: 打包**

```bash
./scripts/package-macos.sh
```

Expected: 产出签名 .app（脚本内嵌 libmpv）。若签名环节因本机证书失败，退化为 `cargo build --release` 并向用户说明走查改用 `cargo run --release`。

- [ ] **Step 3: 更新 TODO.md**

把 36–42 的 `- [ ]` 全部改为 `- [x]`。

- [ ] **Step 4: 更新 CHANGELOG.md**

在 `Unreleased` 的 `Added`（或对应小节）追加：

```markdown
- 电子书：全文搜索（工具栏搜索条、命中计数、重排自动恢复高亮）；EPUB 内嵌图片与内嵌字体显示；字体设置下拉框；TXT/Markdown 自动识别 GBK/GB18030/Big5 编码；电子书视图快捷键补全（Escape 返回、PageUp/PageDown 翻页、Cmd+F 搜索）。
```

- [ ] **Step 5: 提交文档**

```bash
git add TODO.md CHANGELOG.md
git commit -m "docs: 勾选 TODO 36-42 并更新 CHANGELOG——电子书搜索/资源/字体/编码/快捷键"
```

- [ ] **Step 6: 输出人工走查清单给用户**

向用户交付以下走查清单（中、英书籍各备一本更稳妥）：

1. 打开一本含插图的 EPUB → 插图正常显示（不再裂图）；含内嵌字体的书排版使用书籍字体。
2. 工具栏点"搜索"或按 Cmd+F → 输入关键词即时高亮、显示 `n/m`；Enter 下一个、Shift+Enter 上一个、Esc 关闭并清除高亮；翻章/改字号/缩放窗口后高亮自动恢复。
3. 设置 → 电子书 → 字体下拉切换 Songti SC 等 → 正文即时生效；重启应用后保持。
4. 打开一本 GBK 编码 TXT → 中文正常显示、自动分章；UTF-8 文件行为不变。
5. 电子书视图按 Escape 返回书架；PageDown/Space 下一页、PageUp 上一页；搜索框输入时 Space 不误翻页。
6. 回归：打开漫画与视频各一，确认阅读/播放不受影响（本批改动集中在电子书路径，预期无影响）。

## 修订记录

- 2026-07-17 终审修订——Task 3（EPUB 资源通道）资源 URL 由 `ebook://res/<path>` 改为 `ebook://reader/res/<path>`：wry 协议回调的 Request URI 为完整绝对 URL，`res` 会被 `http::Uri` 解析为 host，`uri().path()` 永不含 `/res/` 前缀，导致资源请求全部静默落入空 200 兜底。改为与壳页面同 host 后 path 为 `/res/...`，且字体加载与文档同源，规避自定义协议跨源字体 CORS 风险。同步补 handler 级集成测试（`ebook_renderer.rs` 以真实 fixture 走通 `handle_ebook_protocol`）与 AGENTS.md 备忘。上文 Task 3 正文中出现的 `ebook://res/` 字样为修订前原文，保留备查。
