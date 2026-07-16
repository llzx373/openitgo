use crate::chapters::{build_chapters, split_by_word_count};
use crate::stable_comic_id;
use crate::traits::ParseError;
use openitgo_core::ebook::{Ebook, EbookChapter};
use std::path::Path;

const CHAPTER_WORDS: usize = 3000;

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
        let book =
            mobi::Mobi::from_path(path).map_err(|e| ParseError::InvalidMobi(format!("{}", e)))?;

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
            .map_err(|e| ParseError::InvalidMobi(format!("{}", e)))?;

        let raw_chapters = split_by_word_count(&content, CHAPTER_WORDS);
        let chapters: Vec<EbookChapter> = build_chapters(raw_chapters);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_supports_mobi() {
        assert!(MobiParser::supports(Path::new("book.mobi")));
        assert!(MobiParser::supports(Path::new("book.azw")));
        assert!(MobiParser::supports(Path::new("book.azw3")));
        assert!(!MobiParser::supports(Path::new("book.epub")));
        assert!(!MobiParser::supports(Path::new("book.txt")));
    }

    #[test]
    fn test_parse_fake_mobi_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fake.mobi");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"not a real mobi file").unwrap();

        let result = MobiParser::parse(&path);
        assert!(result.is_err());
    }
}
