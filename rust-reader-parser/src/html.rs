use crate::chapters::{split_by_heading, split_by_word_count};
use crate::traits::ParseError;
use rust_reader_core::ebook::Ebook;
use std::path::Path;

const CHAPTER_WORDS: usize = 3000;

/// Render a single chapter of an ebook as HTML.
pub fn render_chapter_html(ebook: &Ebook, chapter_index: usize) -> Result<String, ParseError> {
    let path = &ebook.path;

    if is_text_like_path(path) {
        let text =
            std::fs::read_to_string(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;
        let parts = if is_markdown_path(path) {
            split_markdown(&text)
        } else {
            split_txt(&text)
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
        doc.set_current_chapter(chapter_index);
        let (bytes, _mime) = doc.get_current().ok_or(ParseError::NoPages)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        Err(ParseError::Unsupported)
    }
}

fn split_txt(text: &str) -> Vec<(Option<String>, String)> {
    split_by_heading(text, txt_extract_title, txt_is_heading)
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
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "txt" | "md"))
        .unwrap_or(false)
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
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
    use rust_reader_core::ebook::EbookChapter;
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
}
