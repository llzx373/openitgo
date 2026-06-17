use crate::traits::{ParseError, Parser};
use rust_reader_core::models::Comic;
use std::path::Path;

pub struct RarParser;

impl Parser for RarParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("rar") || e.eq_ignore_ascii_case("cbr"))
            .unwrap_or(false)
    }

    fn parse(_path: &Path) -> Result<Comic, ParseError> {
        // Future: integrate with unrar crate or external unrar binary.
        Err(ParseError::Unsupported)
    }
}
