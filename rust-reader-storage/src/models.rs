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
    pub wide_page_threshold: f32,
    pub enable_page_animation: bool,
    pub compress_images: bool,
    pub decode_threads: u32,
    pub cache_size_mb: u32,
    pub real_image_cache_pages: u32,
    pub window_size: (f32, f32),
    pub show_toolbar: bool,
    pub show_statusbar: bool,
    pub invert_scroll: bool,
    pub background_color: [u8; 3],
    pub shortcuts: Shortcuts,
    pub library_sort: LibrarySort,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            default_mode: ReadingMode::default(),
            default_fit: FitMode::default(),
            double_page: false,
            wide_page_threshold: 1.4,
            enable_page_animation: true,
            compress_images: false,
            decode_threads: 0,
            cache_size_mb: 1024,
            real_image_cache_pages: 10,
            window_size: (1280.0, 720.0),
            show_toolbar: true,
            show_statusbar: true,
            invert_scroll: false,
            background_color: [30, 30, 30],
            shortcuts: Shortcuts::default(),
            library_sort: LibrarySort::default(),
        }
    }
}

impl Settings {
    /// Validate that all numeric fields are within sensible ranges. Returns an
    /// error message describing the first invalid field.
    pub fn validate(&self) -> Result<(), String> {
        if self.decode_threads > 64 {
            return Err(format!(
                "decode_threads must be <= 64, got {}",
                self.decode_threads
            ));
        }
        if !(100..=16384).contains(&self.cache_size_mb) {
            return Err(format!(
                "cache_size_mb must be between 100 and 16384, got {}",
                self.cache_size_mb
            ));
        }
        if !(1..=500).contains(&self.real_image_cache_pages) {
            return Err(format!(
                "real_image_cache_pages must be between 1 and 500, got {}",
                self.real_image_cache_pages
            ));
        }
        if self.wide_page_threshold < 1.0 || self.wide_page_threshold > 3.0 {
            return Err(format!(
                "wide_page_threshold must be between 1.0 and 3.0, got {}",
                self.wide_page_threshold
            ));
        }
        if self.window_size.0 <= 0.0 || self.window_size.1 <= 0.0 {
            return Err(format!(
                "window_size must be positive, got {:?}",
                self.window_size
            ));
        }
        Ok(())
    }

    /// Clamp all numeric fields to their valid ranges. Used when repairing a
    /// settings file that failed validation.
    pub fn clamp(&mut self) {
        self.decode_threads = self.decode_threads.min(64);
        self.cache_size_mb = self.cache_size_mb.clamp(100, 16384);
        self.real_image_cache_pages = self.real_image_cache_pages.clamp(1, 500);
        self.wide_page_threshold = self.wide_page_threshold.clamp(1.0, 3.0);
        self.window_size.0 = self.window_size.0.max(100.0);
        self.window_size.1 = self.window_size.1.max(100.0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Shortcuts {
    pub next_page: Vec<String>,
    pub prev_page: Vec<String>,
    pub page_down: Vec<String>,
    pub page_up: Vec<String>,
    pub fullscreen: Vec<String>,
    pub fit_page: Vec<String>,
    pub fit_width: Vec<String>,
    pub fit_height: Vec<String>,
    pub zoom_in: Vec<String>,
    pub zoom_out: Vec<String>,
    pub back_to_library: Vec<String>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            next_page: vec!["ArrowRight".to_string()],
            prev_page: vec!["ArrowLeft".to_string()],
            page_down: vec!["PageDown".to_string(), "Space".to_string()],
            page_up: vec!["PageUp".to_string()],
            fullscreen: vec!["F11".to_string()],
            fit_page: vec!["Num0".to_string()],
            fit_width: vec!["W".to_string()],
            fit_height: vec!["H".to_string()],
            zoom_in: vec!["Plus".to_string(), "Equals".to_string()],
            zoom_out: vec!["Minus".to_string()],
            back_to_library: vec!["Escape".to_string()],
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    LastRead,
    Title,
    Added,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64,
}

impl Default for LibraryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            title: String::new(),
            path: PathBuf::new(),
            cover_path: None,
            added_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub entries: Vec<LibraryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryEntry {
    pub comic_id: String,
    pub path: std::path::PathBuf,
    pub volume_index: usize,
    pub page_index: usize,
    pub last_read_at: u64,
}

impl Default for HistoryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            path: std::path::PathBuf::new(),
            volume_index: 0,
            page_index: 0,
            last_read_at: 0,
        }
    }
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
        assert!((s.wide_page_threshold - 1.4).abs() < f32::EPSILON);
    }

    #[test]
    fn test_settings_roundtrip_with_background_color() {
        let s = Settings {
            background_color: [12, 34, 56],
            library_sort: LibrarySort::Title,
            ..Default::default()
        };
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
                added_at: 0,
            }],
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(json.contains("Test"));
    }
}
