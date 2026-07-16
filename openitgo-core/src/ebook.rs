use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EbookReadingMode {
    #[default]
    SinglePage,
    DoublePage,
    Scroll,
}

impl FromStr for EbookReadingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "single" | "singlepage" => Ok(Self::SinglePage),
            "double" | "doublepage" => Ok(Self::DoublePage),
            "scroll" | "continuous" => Ok(Self::Scroll),
            _ => Err(format!("unknown ebook reading mode: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookResource {
    pub id: String,
    pub href: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookChapter {
    pub index: usize,
    pub id: String,
    pub href: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ebook {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub resources: Vec<EbookResource>,
    pub spine: Vec<String>,          // manifest idrefs in reading order
    pub chapters: Vec<EbookChapter>, // table of contents / navigable chapters
}

impl Ebook {
    pub fn total_chapters(&self) -> usize {
        self.chapters.len()
    }

    pub fn chapter_source(&self, index: usize) -> Option<&EbookChapter> {
        self.chapters.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ebook_reading_mode_from_str() {
        assert_eq!(
            "single".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::SinglePage
        );
        assert_eq!(
            "singlepage".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::SinglePage
        );
        assert_eq!(
            "Single".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::SinglePage
        );
        assert_eq!(
            "double".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::DoublePage
        );
        assert_eq!(
            "doublepage".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::DoublePage
        );
        assert_eq!(
            "scroll".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::Scroll
        );
        assert_eq!(
            "continuous".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::Scroll
        );
        assert_eq!(
            "SCROLL".parse::<EbookReadingMode>().unwrap(),
            EbookReadingMode::Scroll
        );
        assert!("unknown".parse::<EbookReadingMode>().is_err());
    }
}
