use rust_reader_core::models::{FitMode, ReadingMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub window_size: (f32, f32),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub entries: Vec<LibraryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub comic_id: String,
    pub volume_index: usize,
    pub page_index: usize,
    pub last_read_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub comic_id: String,
    pub volume_index: usize,
    pub page_index: usize,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bookmarks {
    pub entries: Vec<Bookmark>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let s = Settings::default();
        assert!(matches!(s.theme, Theme::System));
    }

    #[test]
    fn test_library_serialize() {
        let lib = Library {
            entries: vec![LibraryEntry {
                comic_id: "id".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp"),
                cover_path: None,
            }],
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(json.contains("Test"));
    }
}
