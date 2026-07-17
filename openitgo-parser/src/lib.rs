mod chapters;
pub mod epub;
pub mod folder;
pub mod html;
pub mod markdown;
pub mod mobi;
pub mod pdf;
pub mod rar;
pub mod text_encoding;
pub mod traits;
pub mod txt;
pub mod zip;

pub use traits::stable_comic_id;

use crate::epub::EpubParser;
use crate::markdown::MarkdownParser;
use crate::mobi::MobiParser;
use crate::txt::TxtParser;
use openitgo_core::ebook::Ebook;
use openitgo_core::models::Comic;
use rar::RarParser;
use std::path::Path;
use traits::{ParseError, Parser};

pub fn parse(path: &Path) -> Result<Comic, ParseError> {
    parse_with_password(path, None)
}

/// Parse a comic archive/folder, decrypting encrypted ZIP/RAR entries with
/// `password` when needed. Folder and PDF sources ignore `password`.
pub fn parse_with_password(path: &Path, password: Option<&str>) -> Result<Comic, ParseError> {
    if folder::FolderParser::supports(path) {
        folder::FolderParser::parse(path)
    } else if zip::ZipParser::supports(path) {
        zip::parse_zip(path, password)
    } else if RarParser::supports(path) {
        rar::parse_rar(path, password)
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
