use crate::traits::{ParseError, Parser};
use rust_reader_core::models::Comic;
use std::path::Path;

pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    fn parse(_path: &Path) -> Result<Comic, ParseError> {
        // Future: integrate with pdf-rs or mupdf bindings.
        Err(ParseError::Unsupported)
    }
}
