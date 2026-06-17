use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Comic {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub volumes: Vec<Volume>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Volume {
    pub title: String,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub index: usize,
    pub source: PageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PageSource {
    File(PathBuf),
    Bytes(Vec<u8>),
    PdfRef {
        document_path: PathBuf,
        page_number: usize,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReadingMode {
    Ltr,
    Rtl,
    Webtoon,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum FitMode {
    Height,
    Width,
    Page,
    Original,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_source_file() {
        let page = Page {
            index: 0,
            source: PageSource::File(PathBuf::from("page.png")),
        };
        assert!(matches!(page.source, PageSource::File(_)));
    }

    #[test]
    fn test_reading_mode_serialize() {
        let mode = ReadingMode::Rtl;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"Rtl\"");
    }
}
