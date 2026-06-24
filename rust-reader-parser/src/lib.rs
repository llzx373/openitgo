mod chapters;
pub mod epub;
pub mod folder;
pub mod markdown;
pub mod mobi;
pub mod pdf;
pub mod rar;
pub mod traits;
pub mod txt;
pub mod zip;

pub use traits::stable_comic_id;

use crate::epub::EpubParser;
use crate::markdown::MarkdownParser;
use crate::mobi::MobiParser;
use crate::txt::TxtParser;
use rar::RarParser;
use rust_reader_core::ebook::Ebook;
use rust_reader_core::models::Comic;
use std::path::Path;
use traits::{ParseError, Parser};

pub fn parse(path: &Path) -> Result<Comic, ParseError> {
    if folder::FolderParser::supports(path) {
        folder::FolderParser::parse(path)
    } else if zip::ZipParser::supports(path) {
        zip::ZipParser::parse(path)
    } else if RarParser::supports(path) {
        RarParser::parse(path)
    } else if pdf::PdfParser::supports(path) {
        pdf::PdfParser::parse(path)
    } else {
        Err(ParseError::Unsupported)
    }
}

pub fn parse_ebook(path: &Path) -> Result<Ebook, ParseError> {
    if EpubParser::supports(path) {
        EpubParser::parse(path)
    } else if MobiParser::supports(path) {
        MobiParser::parse(path)
    } else if TxtParser::supports(path) {
        TxtParser::parse(path)
    } else if MarkdownParser::supports(path) {
        MarkdownParser::parse(path)
    } else {
        Err(ParseError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_dispatch_folder() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("page.png"), b"fake").unwrap();
        let comic = parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 1);
    }
}
