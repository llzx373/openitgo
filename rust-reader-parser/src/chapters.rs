use crate::stable_comic_id;
use rust_reader_core::ebook::{Ebook, EbookChapter};
use std::path::Path;

/// Split `text` into chapters whenever `is_heading` returns `true` for a line.
/// `extract_title` converts a heading line into its display title.
/// Returns a vector of `(title, body)` tuples in document order.
pub fn split_by_heading(
    text: &str,
    extract_title: impl Fn(&str) -> Option<String>,
    is_heading: impl Fn(&str) -> bool,
) -> Vec<(Option<String>, String)> {
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if is_heading(line) {
            if !current_lines.is_empty() || current_title.is_some() {
                chapters.push((current_title, current_lines.join("\n")));
                current_lines.clear();
            }
            current_title = extract_title(line);
        } else {
            current_lines.push(line.to_string());
        }
    }

    if current_title.is_some() || !chapters.is_empty() {
        chapters.push((current_title, current_lines.join("\n")));
    }

    chapters
}

/// Split `text` into fixed-size virtual chapters of approximately `chunk_words`
/// whitespace-separated words. Each chapter is titled `第 N 章`.
pub fn split_by_word_count(text: &str, chunk_words: usize) -> Vec<(Option<String>, String)> {
    if chunk_words == 0 {
        return Vec::new();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    words
        .chunks(chunk_words)
        .enumerate()
        .map(|(idx, chunk)| {
            let title = Some(format!("第 {} 章", idx + 1));
            let body = chunk.join(" ");
            (title, body)
        })
        .collect()
}

/// Build indexed `EbookChapter` metadata from chapter `(title, body)` tuples.
pub fn build_chapters(parts: Vec<(Option<String>, String)>) -> Vec<EbookChapter> {
    parts
        .into_iter()
        .enumerate()
        .map(|(idx, (title, _body))| {
            let id = format!("chapter-{}", idx + 1);
            EbookChapter {
                index: idx,
                id: id.clone(),
                href: format!("#{}", id),
                title,
            }
        })
        .collect()
}

/// Construct an `Ebook` for a plain-text format using the file stem as the title.
pub fn text_ebook(path: &Path, chapters: Vec<EbookChapter>) -> Ebook {
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    Ebook {
        id: stable_comic_id(path),
        title,
        path: path.to_path_buf(),
        authors: Vec::new(),
        language: None,
        resources: Vec::new(),
        spine: Vec::new(),
        chapters,
    }
}
