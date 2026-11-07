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
pub struct EbookTextSource {
    pub encoding: String,
    pub file_len: u64,
    pub modified_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookTextChapter {
    pub byte_start: u64,
    pub byte_end: u64,
    pub char_start: usize,
    pub logical_chapter: usize,
    pub normalize_whitespace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EbookChapter {
    pub index: usize,
    pub id: String,
    pub href: String,
    pub title: Option<String>,
    /// 目录树嵌套深度（0 = 顶层）：markdown 为归一化标题层级，EPUB 为 navpoint 深度，
    /// txt/mobi/字数切分恒为 0。
    #[serde(default)]
    pub level: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<EbookTextChapter>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<EbookTextSource>,
}

impl Ebook {
    pub fn total_chapters(&self) -> usize {
        self.chapters.len()
    }

    pub fn chapter_source(&self, index: usize) -> Option<&EbookChapter> {
        self.chapters.get(index)
    }

    pub fn text_position(&self, global_offset: usize) -> (usize, usize) {
        if self.chapters.is_empty() {
            return (0, 0);
        }
        let index = self
            .chapters
            .partition_point(|chapter| {
                chapter
                    .text
                    .as_ref()
                    .is_some_and(|text| text.char_start <= global_offset)
            })
            .saturating_sub(1)
            .min(self.chapters.len() - 1);
        let char_start = self.chapters[index]
            .text
            .as_ref()
            .map(|text| text.char_start)
            .unwrap_or(0);
        (index, global_offset.saturating_sub(char_start))
    }

    pub fn legacy_text_position(&self, logical_chapter: usize, offset: usize) -> (usize, usize) {
        let Some(chapter) = self.chapters.iter().find(|chapter| {
            chapter
                .text
                .as_ref()
                .is_some_and(|text| text.logical_chapter == logical_chapter)
        }) else {
            return (
                logical_chapter.min(self.total_chapters().saturating_sub(1)),
                offset,
            );
        };
        let global = chapter
            .text
            .as_ref()
            .map(|text| text.char_start.saturating_add(offset))
            .unwrap_or(0);
        self.text_position(global)
    }

    pub fn global_char_offset(&self, chapter: usize, offset: usize) -> Option<usize> {
        self.chapters
            .get(chapter)
            .and_then(|chapter| chapter.text.as_ref())
            .map(|text| text.char_start.saturating_add(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ebook_chapter_level_defaults_to_zero_in_old_json() {
        // 旧格式 JSON（无 level 字段）反序列化为顶层节点。
        let json = r##"{"index":0,"id":"c1","href":"#c1","title":"A"}"##;
        let ch: EbookChapter = serde_json::from_str(json).unwrap();
        assert_eq!(ch.level, 0);
    }

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

    #[test]
    fn text_positions_support_global_and_legacy_offsets() {
        let chapters = [(0, 0), (100, 0), (200, 1)]
            .into_iter()
            .enumerate()
            .map(|(index, (char_start, logical_chapter))| EbookChapter {
                index,
                id: format!("c{index}"),
                href: format!("#c{index}"),
                title: None,
                level: 0,
                text: Some(EbookTextChapter {
                    byte_start: char_start as u64,
                    byte_end: char_start as u64 + 100,
                    char_start,
                    logical_chapter,
                    normalize_whitespace: false,
                }),
            })
            .collect();
        let ebook = Ebook {
            id: "id".to_string(),
            title: "title".to_string(),
            path: PathBuf::from("book.txt"),
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters,
            text: None,
        };
        assert_eq!(ebook.legacy_text_position(0, 120), (1, 20));
        assert_eq!(ebook.legacy_text_position(1, 10), (2, 10));
        assert_eq!(ebook.text_position(250), (2, 50));
        assert_eq!(ebook.global_char_offset(1, 20), Some(120));
    }
}
