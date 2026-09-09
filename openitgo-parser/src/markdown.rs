use crate::chapters::{
    build_chapters, build_chapters_leveled, split_by_word_count, split_markdown, text_ebook,
};
use crate::traits::ParseError;
use openitgo_core::ebook::Ebook;
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
        let text = crate::text_encoding::read_text_lossy(path)?;

        if text.trim().is_empty() {
            return Err(ParseError::NoPages);
        }

        let parts = split_markdown(&text);
        let chapters = if parts.is_empty() {
            build_chapters(split_by_word_count(&text, 3000))
        } else {
            build_chapters_leveled(parts)
        };

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(text_ebook(path, chapters))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_markdown() {
        assert!(MarkdownParser::supports(Path::new("book.md")));
        assert!(MarkdownParser::supports(Path::new("book.markdown")));
        assert!(!MarkdownParser::supports(Path::new("book.txt")));
    }
}
