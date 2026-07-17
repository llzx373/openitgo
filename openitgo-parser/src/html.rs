use crate::chapters::{split_by_heading, split_by_word_count};
use crate::traits::ParseError;
use openitgo_core::ebook::Ebook;
use std::path::Path;

const CHAPTER_WORDS: usize = 3000;

/// Render a single chapter of an ebook as HTML.
pub fn render_chapter_html(ebook: &Ebook, chapter_index: usize) -> Result<String, ParseError> {
    let path = &ebook.path;

    if is_text_like_path(path) {
        let text = crate::text_encoding::read_text_lossy(path)?;
        let parts = if is_markdown_path(path) {
            let mut ch = split_markdown(&text);
            if ch.is_empty() {
                ch = split_by_word_count(&text, CHAPTER_WORDS);
            }
            ch
        } else {
            let mut ch = split_txt(&text);
            if ch.is_empty() {
                ch = split_by_word_count(&text, CHAPTER_WORDS);
            }
            ch
        };
        let (_, body) = parts.get(chapter_index).ok_or(ParseError::NoPages)?.clone();
        let html = if is_markdown_path(path) {
            markdown_to_html(&body)
        } else {
            plain_text_to_html(&body)
        };
        Ok(format!("<div class=\"chapter\">{}</div>", html))
    } else if is_mobi_path(path) {
        let book =
            mobi::Mobi::from_path(path).map_err(|e| ParseError::InvalidMobi(format!("{}", e)))?;
        let text = book.content_as_string_lossy();
        let parts = split_by_word_count(&text, CHAPTER_WORDS);
        let (_, body) = parts.get(chapter_index).ok_or(ParseError::NoPages)?.clone();
        Ok(format!(
            "<div class=\"chapter\">{}</div>",
            plain_text_to_html(&body)
        ))
    } else if is_epub_path(path) {
        let mut doc =
            epub::doc::EpubDoc::new(path).map_err(|e| ParseError::InvalidEpub(format!("{}", e)))?;
        let chapter = ebook
            .chapters
            .get(chapter_index)
            .ok_or(ParseError::NoPages)?;
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
    } else {
        Err(ParseError::Unsupported)
    }
}

fn split_txt(text: &str) -> Vec<(Option<String>, String)> {
    split_by_heading(text, txt_extract_title, txt_is_heading)
}

/// Sanitizes EPUB chapter HTML for display inside a controlled WebView shell.
/// Removes `<base>` tags (which would change the shell's base URL and cause
/// reloads), scripts, stylesheet links, and disables anchor navigation.
pub fn sanitize_epub_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let chars: Vec<(usize, char)> = html.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start_idx, c) = chars[i];
        if c != '<' {
            out.push(c);
            i += 1;
            continue;
        }

        let mut end_j = None;
        let mut in_quote = None::<char>;
        let mut j = i + 1;
        while j < chars.len() {
            let (_, ch) = chars[j];
            match in_quote {
                None => {
                    if ch == '"' || ch == '\'' {
                        in_quote = Some(ch);
                    } else if ch == '>' {
                        end_j = Some(j);
                        break;
                    }
                }
                Some(q) => {
                    if ch == q {
                        in_quote = None;
                    }
                }
            }
            j += 1;
        }

        let end_j = match end_j {
            Some(e) => e,
            None => {
                out.push_str(&html[start_idx..]);
                break;
            }
        };

        let tag = &html[start_idx..=chars[end_j].0];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<script") {
            // Drop the whole script block, including its body.
            let after = &html[chars[end_j].0 + 1..];
            let close = after.to_ascii_lowercase().find("</script>");
            let skip = match close {
                Some(pos) => after[pos..]
                    .char_indices()
                    .nth("</script>".len())
                    .map(|(i, _)| pos + i)
                    .unwrap_or(after.len()),
                None => after.len(),
            };
            let new_pos = chars[end_j].0 + 1 + skip;
            i = match chars.binary_search_by_key(&new_pos, |(idx, _)| *idx) {
                Ok(p) => p,
                Err(p) => p.min(chars.len()),
            };
            continue;
        }
        if lower.starts_with("<base") || lower.starts_with("<link") {
            // Drop these tags entirely.
        } else if is_anchor_open_tag(&lower) {
            out.push_str("<span");
            strip_attributes(&lower, tag, &mut out, &["href", "onclick"]);
        } else if is_anchor_close_tag(&lower) {
            out.push_str("</span>");
        } else {
            out.push_str(tag);
        }
        i = end_j + 1;
    }
    out
}

fn is_anchor_open_tag(lower: &str) -> bool {
    lower.starts_with("<a")
        && matches!(
            lower.as_bytes().get(2).copied(),
            Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')
        )
}

fn is_anchor_close_tag(lower: &str) -> bool {
    lower.starts_with("</a")
        && matches!(
            lower.as_bytes().get(3).copied(),
            None | Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        )
}

fn strip_attributes(lower_tag: &str, original_tag: &str, out: &mut String, names: &[&str]) {
    // original_tag starts with '<' and ends with '>'; strip both ends.
    let inner = &original_tag[1..original_tag.len().saturating_sub(1)];
    let lower_inner = &lower_tag[1..lower_tag.len().saturating_sub(1)];
    // Skip the tag name ('a').
    let mut i = 1usize;
    let mut first = true;
    while i < inner.len() {
        while i < inner.len() && inner.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= inner.len() {
            break;
        }
        let attr_start = i;
        let mut in_quote = None::<u8>;
        let mut j = i;
        while j < inner.len() {
            let c = inner.as_bytes()[j];
            if in_quote.is_none() && c.is_ascii_whitespace() {
                break;
            }
            if in_quote.is_none() && (c == b'"' || c == b'\'') {
                in_quote = Some(c);
            } else if in_quote == Some(c) {
                in_quote = None;
            }
            j += 1;
        }
        let attr = &inner[attr_start..j];
        let lower_attr = &lower_inner[attr_start..j];
        let name_end = lower_attr
            .find(|c: char| c == '=' || c.is_ascii_whitespace())
            .unwrap_or(lower_attr.len());
        let name = &lower_attr[..name_end];
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            if first {
                out.push(' ');
                first = false;
            } else {
                out.push(' ');
            }
            out.push_str(attr);
        }
        i = j;
    }
    out.push('>');
}

fn txt_is_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with('#')
        || trimmed.to_ascii_lowercase().starts_with("chapter ")
        || (trimmed.starts_with('第') && trimmed.contains('章'))
}

fn txt_extract_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    Some(trimmed.trim_start_matches('#').trim().to_string())
}

fn split_markdown(text: &str) -> Vec<(Option<String>, String)> {
    split_by_heading(text, markdown_extract_title, markdown_is_heading)
}

fn markdown_is_heading(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("# ") || trimmed.starts_with("## ")
}

fn markdown_extract_title(line: &str) -> Option<String> {
    Some(line.trim().trim_start_matches('#').trim().to_string())
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

fn is_text_like_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "txt" | "md" | "markdown"))
        .unwrap_or(false)
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

fn is_mobi_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mobi" | "azw" | "azw3"))
        .unwrap_or(false)
}

fn is_epub_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("epub"))
        .unwrap_or(false)
}

/// Percent-encode set for `ebook://reader/res/` paths: keep path-safe ASCII readable,
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

/// Build an absolute `ebook://reader/res/` URL for a relative resource
/// reference. The URL shares the shell page's host (`reader`) because the
/// custom-protocol callback receives the full absolute URL as an `http::Uri`:
/// the host segment never appears in `uri().path()`, so the `/res/` marker
/// must live in the path — and same-origin font loads avoid CORS issues.
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
    Some(format!("ebook://reader/res/{}", encoded))
}

/// Rewrite relative resource references (`src=` / `xlink:href=` attribute
/// values) in sanitized EPUB chapter HTML to absolute `ebook://reader/res/` URLs,
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

/// Extract all `@font-face { ... }` blocks from a stylesheet, rewriting
/// relative `url(...)` references (resolved against the stylesheet's own
/// directory) to `ebook://reader/res/` URLs. Every other rule is dropped so book
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

/// Rewrite every relative `url(...)` in a CSS block to `ebook://reader/res/` URLs.
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

#[cfg(test)]
mod tests {
    use super::*;
    use openitgo_core::ebook::EbookChapter;
    use std::path::PathBuf;

    fn ebook_with_path(path: PathBuf) -> Ebook {
        Ebook {
            id: "test".to_string(),
            title: "Test".to_string(),
            path,
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters: vec![EbookChapter {
                index: 0,
                id: "ch1".to_string(),
                href: "#ch1".to_string(),
                title: Some("Chapter 1".to_string()),
            }],
        }
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(
            escape_html("<script>alert(\"x\");</script>"),
            "&lt;script&gt;alert(&quot;x&quot;);&lt;/script&gt;"
        );
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_plain_text_to_html() {
        let html = plain_text_to_html("Hello\n\nWorld");
        assert!(html.contains("<p>Hello</p>"));
        assert!(html.contains("<p>World</p>"));
    }

    #[test]
    fn test_markdown_to_html() {
        let html = markdown_to_html("# Hello\n\nworld");
        assert!(html.contains("Hello"));
        assert!(html.contains("world"));
    }

    #[test]
    fn test_render_txt_chapter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.txt");
        std::fs::write(&path, "Chapter One\n\nHello world").unwrap();
        let ebook = ebook_with_path(path);
        let html = render_chapter_html(&ebook, 0).unwrap();
        assert!(html.contains("Hello world"));
        assert!(html.starts_with("<div class=\"chapter\">"));
    }

    #[test]
    fn test_render_markdown_chapter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.md");
        std::fs::write(&path, "# Hello\n\nworld\n\nmore text").unwrap();
        let ebook = ebook_with_path(path);
        let html = render_chapter_html(&ebook, 0).unwrap();
        assert!(html.contains("world"));
        assert!(html.contains("more text"));
        assert!(html.starts_with("<div class=\"chapter\">"));
    }

    #[test]
    fn test_render_markdown_extension_chapter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.markdown");
        std::fs::write(&path, "# Hello\n\nworld").unwrap();
        let ebook = ebook_with_path(path);
        let html = render_chapter_html(&ebook, 0).unwrap();
        assert!(html.contains("world"));
        assert!(html.starts_with("<div class=\"chapter\">"));
    }

    #[test]
    fn test_render_chapter_out_of_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.txt");
        std::fs::write(&path, "Hello world").unwrap();
        let ebook = ebook_with_path(path);
        assert!(render_chapter_html(&ebook, 99).is_err());
    }

    #[test]
    fn test_render_unsupported_extension() {
        let ebook = ebook_with_path(PathBuf::from("book.pdf"));
        assert!(matches!(
            render_chapter_html(&ebook, 0),
            Err(ParseError::Unsupported)
        ));
    }

    #[test]
    fn test_sanitize_epub_html_removes_base_and_scripts() {
        let html = r#"<html><head><base href="OEBPS/"/><script>alert('x');</script><link rel="stylesheet" href="style.css"/></head><body><a href="ch2.xhtml" class="next">下一章</a><p>Hello</p></body></html>"#;
        let clean = sanitize_epub_html(html);
        assert!(!clean.contains("<base"));
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("</script"));
        assert!(!clean.contains("<link"));
        assert!(!clean.contains("href="));
        assert!(!clean.contains("onclick"));
        assert!(clean.contains("<span"));
        assert!(clean.contains("class=\"next\""));
        assert!(clean.contains("下一章"));
        assert!(clean.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_sanitize_epub_html_preserves_non_anchor_tags() {
        let html = r#"<p id="p1" class="text">Hello <abbr title="test">abbr</abbr></p>"#;
        let clean = sanitize_epub_html(html);
        assert_eq!(clean, html);
    }

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
        assert!(
            out.contains(r#"src="ebook://reader/res/OEBPS/Images/pic.png""#),
            "got: {out}"
        );
    }

    #[test]
    fn test_rewrite_epub_urls_single_quotes_and_space_encoding() {
        let html = "<img src='images/a b.png'/>";
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(
            out.contains("src='ebook://reader/res/OEBPS/Text/images/a%20b.png'"),
            "got: {out}"
        );
    }

    #[test]
    fn test_rewrite_epub_urls_skips_absolute_data_fragment() {
        let html = r##"<img src="data:image/png;base64,AAAA"/><img src="https://example.com/a.png"/><img src="#frag"/>"##;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert_eq!(out, html);
    }

    #[test]
    fn test_rewrite_epub_urls_xlink_href() {
        let html = r#"<svg><image xlink:href="../Images/p.svg"/></svg>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(
            out.contains(r#"xlink:href="ebook://reader/res/OEBPS/Images/p.svg""#),
            "got: {out}"
        );
    }

    #[test]
    fn test_rewrite_epub_urls_ignores_prose_and_data_src() {
        // 标签外的 src=" 不改写；data-src= 不被误认为 src=
        let html = r#"<p>see src="x.png" here</p><img data-src="y.png" src="z.png"/>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(out.contains(r#"see src="x.png" here"#), "got: {out}");
        assert!(out.contains(r#"data-src="y.png""#), "got: {out}");
        assert!(
            out.contains(r#"src="ebook://reader/res/OEBPS/Text/z.png""#),
            "got: {out}"
        );
    }

    #[test]
    fn test_rewrite_epub_urls_non_ascii_filename() {
        let html = r#"<img src="图片.png"/>"#;
        let out = rewrite_epub_urls(html, "OEBPS/Text/ch1.xhtml");
        assert!(
            out.contains("ebook://reader/res/OEBPS/Text/%E5%9B%BE%E7%89%87.png"),
            "got: {out}"
        );
    }

    #[test]
    fn test_extract_font_faces_basic() {
        let css = r#"@font-face { font-family: "My"; src: url("../Fonts/my.ttf"); }"#;
        let out = extract_font_faces(css, "OEBPS/Styles/s.css");
        assert!(out.contains("@font-face"), "got: {out}");
        assert!(
            out.contains(r#"url("ebook://reader/res/OEBPS/Fonts/my.ttf")"#),
            "got: {out}"
        );
    }

    #[test]
    fn test_extract_font_faces_unquoted_url() {
        let css = "@font-face { src: url(fonts/a.woff2) format('woff2'); }";
        let out = extract_font_faces(css, "OEBPS/Styles/s.css");
        assert!(
            out.contains(r#"url("ebook://reader/res/OEBPS/Styles/fonts/a.woff2")"#),
            "got: {out}"
        );
        assert!(out.contains("format('woff2')"), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_absolute_and_data_untouched() {
        let css = r#"@font-face { src: url("https://x.com/a.ttf"); } @font-face { src: url("data:font/ttf;base64,AA"); }"#;
        let out = extract_font_faces(css, "OEBPS/s.css");
        assert!(out.contains(r#"url("https://x.com/a.ttf")"#), "got: {out}");
        assert!(
            out.contains(r#"url("data:font/ttf;base64,AA")"#),
            "got: {out}"
        );
    }

    #[test]
    fn test_extract_font_faces_drops_non_font_rules() {
        let css = "body { color: red; } p { margin: 0; }";
        assert_eq!(extract_font_faces(css, "OEBPS/s.css"), "");
    }

    #[test]
    fn test_extract_font_faces_multiple_blocks() {
        let css =
            "@font-face { src: url(a.ttf); } body { color: red; } @font-face { src: url(b.otf); }";
        let out = extract_font_faces(css, "s.css");
        assert_eq!(out.matches("@font-face").count(), 2);
        assert!(
            out.contains(r#"url("ebook://reader/res/a.ttf")"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"url("ebook://reader/res/b.otf")"#),
            "got: {out}"
        );
        assert!(!out.contains("color"), "got: {out}");
    }

    #[test]
    fn test_extract_font_faces_unbalanced_block_stops() {
        let css = "@font-face { src: url(a.ttf); ";
        assert_eq!(extract_font_faces(css, "s.css"), "");
    }
}
