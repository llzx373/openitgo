use rust_reader_core::models::{FitMode, ReadingMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub double_page: bool,
    pub cache_size_mb: u32,
    pub window_size: (f32, f32),
    pub show_toolbar: bool,
    pub show_statusbar: bool,
    pub invert_scroll: bool,
    pub background_color: [u8; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            default_mode: ReadingMode::default(),
            default_fit: FitMode::default(),
            double_page: false,
            cache_size_mb: 1024,
            window_size: (1280.0, 720.0),
            show_toolbar: true,
            show_statusbar: true,
            invert_scroll: false,
            background_color: [30, 30, 30],
        }
    }
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
        assert_eq!(s.cache_size_mb, 1024);
        assert!(s.show_toolbar);
        assert!(s.show_statusbar);
        assert!(!s.invert_scroll);
        assert_eq!(s.background_color, [30, 30, 30]);
    }

    #[test]
    fn test_settings_roundtrip_with_background_color() {
        let mut s = Settings::default();
        s.background_color = [12, 34, 56];
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
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
