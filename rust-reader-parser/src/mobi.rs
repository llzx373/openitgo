use crate::traits::ParseError;
use crate::stable_comic_id;
use rust_reader_core::ebook::{Ebook, EbookChapter};
use std::path::Path;

pub struct MobiParser;

impl MobiParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                e.eq_ignore_ascii_case("mobi")
                    || e.eq_ignore_ascii_case("azw")
                    || e.eq_ignore_ascii_case("azw3")
            })
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let book = mobi::Mobi::from_path(path)
            .map_err(|e| ParseError::InvalidArchive(format!("{}", e)))?;

        let metadata = book.metadata();
        let title = metadata.title().unwrap_or_default().to_string();
        let author = metadata.author().unwrap_or_default().to_string();
        let language = metadata.language().map(|s| s.to_string());

        let content = book
            .content_as_string()
            .map_err(|e| ParseError::InvalidArchive(format!("{}", e)))?;

        let words: Vec<&str> = content.split_whitespace().collect();
        let chunk_size = 3000;
        let chapters: Vec<EbookChapter> = words
            .chunks(chunk_size)
            .enumerate()
            .map(|(idx, chunk)| EbookChapter {
                index: idx,
                id: format!("chapter-{}", idx + 1),
                href: format!("#chapter-{}", idx + 1),
                title: Some(format!("第 {} 章", idx + 1)),
            })
            .collect();

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(Ebook {
            id: stable_comic_id(path),
            title,
            path: path.to_path_buf(),
            authors: if author.is_empty() {
                Vec::new()
            } else {
                vec![author]
            },
            language,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters,
        })
    }
}
