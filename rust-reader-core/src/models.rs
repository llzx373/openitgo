use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Comic {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub volumes: Vec<Volume>,
}

impl Comic {
    pub fn total_pages(&self) -> usize {
        self.volumes.first().map(|v| v.pages.len()).unwrap_or(0)
    }

    pub fn page_source(&self, page_index: usize) -> Option<&PageSource> {
        self.volumes
            .first()?
            .pages
            .get(page_index)
            .map(|p| &p.source)
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PageSource {
    File(PathBuf),
    ZipEntry {
        archive: PathBuf,
        name: String,
    },
    RarEntry {
        archive: PathBuf,
        name: String,
    },
    PdfPage {
        document: PathBuf,
        page_number: usize,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReadingMode {
    #[default]
    Ltr,
    Rtl,
    Webtoon,
}

impl ReadingMode {
    pub fn is_webtoon(&self) -> bool {
        matches!(self, ReadingMode::Webtoon)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FitMode {
    #[default]
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
