use crate::stable_comic_id;
use openitgo_core::ebook::{Ebook, EbookChapter};
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

/// Split markdown `text` into chapters at every heading (ATX `#`–`######` and
/// setext `===`/`---`), returning `(depth, title, body)` tuples in document
/// order. Heading detection uses the real markdown parser, so `#` lines inside
/// fenced code blocks are not mistaken for headings. `depth` is the heading
/// level normalized to nesting depth (a document starting at h2 treats h2 as
/// depth 0; skipped levels collapse). A chapter's body excludes its own
/// heading line; text before the first heading becomes a `(0, None, body)`
/// preamble chapter.
pub fn split_markdown(text: &str) -> Vec<(usize, Option<String>, String)> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    // (heading_start, heading_end, level 1..=6, title)
    let mut headings: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut current: Option<(usize, usize, String)> = None;
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                current = Some((level, range.start, String::new()));
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, _, title)) = current.as_mut() {
                    title.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, title)) = current.as_mut() {
                    title.push(' ');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, title)) = current.take() {
                    headings.push((start, range.end, level, title.trim().to_string()));
                }
            }
            _ => {}
        }
    }
    if headings.is_empty() {
        return Vec::new();
    }

    // Normalize heading levels to nesting depths: pop every stacked level >=
    // the new one, depth = remaining stack depth.
    let mut depths: Vec<usize> = Vec::with_capacity(headings.len());
    let mut stack: Vec<usize> = Vec::new();
    for (_, _, level, _) in &headings {
        while stack.last().is_some_and(|top| top >= level) {
            stack.pop();
        }
        depths.push(stack.len());
        stack.push(*level);
    }

    let mut chapters: Vec<(usize, Option<String>, String)> = Vec::new();
    let preamble = text[..headings[0].0].trim();
    if !preamble.is_empty() {
        chapters.push((0, None, preamble.to_string()));
    }
    for (i, (_, end, _, title)) in headings.iter().enumerate() {
        let body_end = headings
            .get(i + 1)
            .map(|(s, _, _, _)| *s)
            .unwrap_or(text.len());
        let title = (!title.is_empty()).then(|| title.clone());
        chapters.push((depths[i], title, text[*end..body_end].trim().to_string()));
    }
    chapters
}

/// Build indexed `EbookChapter` metadata from flat chapter `(title, body)`
/// tuples (txt/mobi/word-count splits); every chapter is top-level.
pub fn build_chapters(parts: Vec<(Option<String>, String)>) -> Vec<EbookChapter> {
    build_chapters_leveled(
        parts
            .into_iter()
            .map(|(title, body)| (0, title, body))
            .collect(),
    )
}

/// Build indexed `EbookChapter` metadata from `(level, title, body)` tuples.
pub fn build_chapters_leveled(parts: Vec<(usize, Option<String>, String)>) -> Vec<EbookChapter> {
    parts
        .into_iter()
        .enumerate()
        .map(|(idx, (level, title, _body))| {
            let id = format!("chapter-{}", idx + 1);
            EbookChapter {
                index: idx,
                id: id.clone(),
                href: format!("#{}", id),
                title,
                level,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_markdown_nested_levels() {
        let text = "# A\na1\n## B\nb1\n### C\nc1\n## D\nd1\n# E\ne1\n";
        let parts = split_markdown(text);
        let got: Vec<(usize, Option<String>, String)> = parts;
        let levels: Vec<usize> = got.iter().map(|(l, _, _)| *l).collect();
        let titles: Vec<Option<&str>> = got.iter().map(|(_, t, _)| t.as_deref()).collect();
        let bodies: Vec<&str> = got.iter().map(|(_, _, b)| b.as_str()).collect();
        assert_eq!(levels, vec![0, 1, 2, 1, 0]);
        assert_eq!(
            titles,
            vec![Some("A"), Some("B"), Some("C"), Some("D"), Some("E")]
        );
        assert_eq!(bodies, vec!["a1", "b1", "c1", "d1", "e1"]);
    }

    #[test]
    fn test_split_markdown_preamble_chapter() {
        let parts = split_markdown("序言文字\n\n# A\na1\n");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], (0, None, "序言文字".to_string()));
        assert_eq!(parts[1].1.as_deref(), Some("A"));
    }

    #[test]
    fn test_split_markdown_skipped_levels_collapse() {
        // h1 -> h3 跳级：h3 归一化为 depth 1；其后 h2 弹出 h3 后同为 depth 1。
        let parts = split_markdown("# A\na\n### C\nc\n## B\nb\n");
        let levels: Vec<usize> = parts.iter().map(|(l, _, _)| *l).collect();
        assert_eq!(levels, vec![0, 1, 1]);
    }

    #[test]
    fn test_split_markdown_document_starting_at_h2() {
        let parts = split_markdown("## A\na\n### B\nb\n");
        let levels: Vec<usize> = parts.iter().map(|(l, _, _)| *l).collect();
        assert_eq!(levels, vec![0, 1]);
    }

    #[test]
    fn test_split_markdown_ignores_hash_in_code_block() {
        let text = "# A\na1\n```\n# 不是标题\n```\n## B\nb1\n";
        let parts = split_markdown(text);
        let titles: Vec<Option<&str>> = parts.iter().map(|(_, t, _)| t.as_deref()).collect();
        assert_eq!(titles, vec![Some("A"), Some("B")]);
        assert!(parts[0].2.contains("# 不是标题"));
    }

    #[test]
    fn test_split_markdown_setext_heading() {
        let parts = split_markdown("标题一\n===\nbody\n");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].0, 0);
        assert_eq!(parts[0].1.as_deref(), Some("标题一"));
        assert_eq!(parts[0].2, "body");
    }

    #[test]
    fn test_split_markdown_no_headings_returns_empty() {
        assert!(split_markdown("普通文字\n\n没有标题\n").is_empty());
    }

    #[test]
    fn test_split_markdown_heading_with_inline_formatting() {
        let parts = split_markdown("# 带 `code` 的标题\nbody\n");
        assert_eq!(parts[0].1.as_deref(), Some("带 code 的标题"));
    }

    #[test]
    fn test_build_chapters_leveled() {
        let chapters = build_chapters_leveled(vec![
            (0, Some("A".to_string()), "a".to_string()),
            (1, Some("B".to_string()), "b".to_string()),
        ]);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].level, 0);
        assert_eq!(chapters[1].level, 1);
        assert_eq!(chapters[1].index, 1);
        assert_eq!(chapters[1].href, "#chapter-2");
    }

    #[test]
    fn test_build_chapters_flat_defaults_level_zero() {
        let chapters = build_chapters(vec![(Some("A".to_string()), "a".to_string())]);
        assert_eq!(chapters[0].level, 0);
    }
}
