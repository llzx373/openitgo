use crate::stable_comic_id;
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter};
use std::fs;
use std::path::Path;

pub struct MarkdownParser;

impl MarkdownParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let text =
            fs::read_to_string(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;

        if text.trim().is_empty() {
            return Err(ParseError::NoPages);
        }

        let lines: Vec<&str> = text.lines().collect();
        let mut chapters: Vec<EbookChapter> = Vec::new();
        let mut current_title: Option<String> = None;
        let mut current_lines: Vec<String> = Vec::new();
        let mut idx: usize = 0;

        for line in lines {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
                if !current_lines.is_empty() {
                    let id = format!("chapter-{}", idx + 1);
                    chapters.push(EbookChapter {
                        index: idx,
                        id: id.clone(),
                        href: format!("#{}", id),
                        title: current_title,
                    });
                    idx += 1;
                    current_lines.clear();
                }
                current_title = Some(trimmed.trim_start_matches('#').trim().to_string());
            } else {
                current_lines.push(line.to_string());
            }
        }

        if !current_lines.is_empty() || (chapters.is_empty() && current_title.is_some()) {
            let id = format!("chapter-{}", idx + 1);
            chapters.push(EbookChapter {
                index: idx,
                id: id.clone(),
                href: format!("#{}", id),
                title: current_title,
            });
        }

        if chapters.is_empty() {
            let words: Vec<&str> = text.split_whitespace().collect();
            let chunk_size = 3000;
            chapters = words
                .chunks(chunk_size)
                .enumerate()
                .map(|(cidx, _)| {
                    let id = format!("chapter-{}", cidx + 1);
                    EbookChapter {
                        index: cidx,
                        id: id.clone(),
                        href: format!("#{}", id),
                        title: Some(format!("第 {} 章", cidx + 1)),
                    }
                })
                .collect();
        }

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(Ebook {
            id: stable_comic_id(path),
            title: path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: path.to_path_buf(),
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters,
        })
    }
}
