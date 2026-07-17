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
        Ok(sanitize_epub_html(&html))
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
}
