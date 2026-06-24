use crate::stable_comic_id;
use crate::traits::ParseError;
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

        let title = book.title();
        let author = book.author().unwrap_or_default();
        let language = {
            let lang = book.language();
            match lang {
                mobi::headers::Language::Neutral | mobi::headers::Language::Unknown => None,
                _ => Some(format!("{:?}", lang)),
            }
        };

        let content = book
            .content_as_string()
            .map_err(|e| ParseError::InvalidArchive(format!("{}", e)))?;

        let words: Vec<&str> = content.split_whitespace().collect();
        let chunk_size = 3000;
        let chapters: Vec<EbookChapter> = words
            .chunks(chunk_size)
            .enumerate()
            .map(|(idx, _chunk)| {
                let id = format!("chapter-{}", idx + 1);
                EbookChapter {
                    index: idx,
                    id: id.clone(),
                    href: format!("#{}", id),
                    title: Some(format!("第 {} 章", idx + 1)),
                }
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
