use crate::chapters::{build_chapters, split_by_heading, split_by_word_count, text_ebook};
use crate::traits::ParseError;
use openitgo_core::ebook::Ebook;
use std::fs;
use std::path::Path;

pub struct TxtParser;

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with('#')
        || trimmed.to_ascii_lowercase().starts_with("chapter ")
        || (trimmed.starts_with('第') && trimmed.contains('章'))
}

fn extract_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    Some(trimmed.trim_start_matches('#').trim().to_string())
}

impl TxtParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("txt"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let text =
            fs::read_to_string(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;

        if text.trim().is_empty() {
            return Err(ParseError::NoPages);
        }

        let parts = split_by_heading(&text, extract_title, is_heading);
        let chapters = if parts.is_empty() {
            build_chapters(split_by_word_count(&text, 3000))
        } else {
            build_chapters(parts)
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
    fn test_supports_txt() {
        assert!(TxtParser::supports(Path::new("book.txt")));
        assert!(!TxtParser::supports(Path::new("book.epub")));
        assert!(!TxtParser::supports(Path::new("book.md")));
    }
}
