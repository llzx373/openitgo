use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
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
    pub toolbar_display_mode: ToolbarDisplayMode,
    pub ebook: EbookSettings,
    pub media_volume: f64,
    pub media_speed: f64,
    pub media_audio_device: String,
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
            toolbar_display_mode: ToolbarDisplayMode::default(),
            ebook: EbookSettings::default(),
            media_volume: 100.0,
            media_speed: 1.0,
            media_audio_device: String::new(),
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
        if !(10..=72).contains(&self.ebook.font_size) {
            return Err(format!(
                "ebook.font_size must be between 10 and 72, got {}",
                self.ebook.font_size
            ));
        }
        if self.ebook.line_height < 1.0 || self.ebook.line_height > 3.0 {
            return Err(format!(
                "ebook.line_height must be between 1.0 and 3.0, got {}",
                self.ebook.line_height
            ));
        }
        if !(0..=200).contains(&self.ebook.margin_horizontal) {
            return Err(format!(
                "ebook.margin_horizontal must be between 0 and 200, got {}",
                self.ebook.margin_horizontal
            ));
        }
        if !(0..=200).contains(&self.ebook.margin_vertical) {
            return Err(format!(
                "ebook.margin_vertical must be between 0 and 200, got {}",
                self.ebook.margin_vertical
            ));
        }
        if self.ebook.font_family.trim().is_empty() {
            return Err("ebook.font_family must not be empty".to_string());
        }
        if !(0.0..=100.0).contains(&self.media_volume) {
            return Err(format!(
                "media_volume must be between 0 and 100, got {}",
                self.media_volume
            ));
        }
        if !(0.1..=16.0).contains(&self.media_speed) {
            return Err(format!(
                "media_speed must be between 0.1 and 16, got {}",
                self.media_speed
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
        self.ebook.font_size = self.ebook.font_size.clamp(10, 72);
        self.ebook.line_height = self.ebook.line_height.clamp(1.0, 3.0);
        self.ebook.margin_horizontal = self.ebook.margin_horizontal.clamp(0, 200);
        self.ebook.margin_vertical = self.ebook.margin_vertical.clamp(0, 200);
        if self.ebook.font_family.trim().is_empty() {
            self.ebook.font_family = "system-ui".to_string();
        }
        self.media_volume = self.media_volume.clamp(0.0, 100.0);
        self.media_speed = self.media_speed.clamp(0.1, 16.0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Shortcuts {
    pub next_page: Vec<String>,
    pub prev_page: Vec<String>,
    pub first_page: Vec<String>,
    pub last_page: Vec<String>,
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
            first_page: vec!["Home".to_string()],
            last_page: vec!["End".to_string()],
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarDisplayMode {
    #[default]
    IconAndText,
    IconOnly,
    TextOnly,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    #[default]
    Comic,
    Ebook,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EbookTheme {
    #[default]
    Light,
    Dark,
    Sepia,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EbookSettings {
    pub reading_mode: EbookReadingMode,
    pub font_family: String,
    pub font_size: u32,
    pub line_height: f32,
    pub margin_horizontal: u32,
    pub margin_vertical: u32,
    pub theme: EbookTheme,
    pub enable_page_animation: bool,
    pub invert_scroll: bool,
}

impl Default for EbookSettings {
    fn default() -> Self {
        Self {
            reading_mode: EbookReadingMode::SinglePage,
            font_family: "system-ui".to_string(),
            font_size: 16,
            line_height: 1.6,
            margin_horizontal: 24,
            margin_vertical: 24,
            theme: EbookTheme::Light,
            enable_page_animation: false,
            invert_scroll: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64,
    pub media_type: MediaType,
}

impl Default for LibraryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            title: String::new(),
            path: PathBuf::new(),
            cover_path: None,
            added_at: 0,
            media_type: MediaType::Comic,
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
    #[serde(default)]
    pub char_offset: Option<usize>,
    pub last_read_at: u64,
}

impl Default for HistoryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            path: std::path::PathBuf::new(),
            volume_index: 0,
            page_index: 0,
            char_offset: None,
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
    #[serde(default)]
    pub char_offset: Option<usize>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bookmarks {
    pub entries: Vec<Bookmark>,
}

/// 每本书记忆的阅读设置（模式/双页/缩放/旋转），打开时覆盖全局默认；
/// 以 comic_id 为 key 存于 comic_settings.json。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComicReadingSettings {
    pub mode: ReadingMode,
    pub double_page: bool,
    pub fit: FitMode,
    /// 90° 步进旋转（0/90/180/270）；旧文件无此字段时默认为 0。
    /// （用 u16 而非 u8：270 超出 u8 范围。）
    #[serde(default)]
    pub rotation: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comic_reading_settings_serde_roundtrip() {
        let s = ComicReadingSettings {
            mode: ReadingMode::Rtl,
            double_page: true,
            fit: FitMode::Page,
            rotation: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: ComicReadingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_comic_reading_settings_deserializes_missing_rotation_as_zero() {
        // 旧版 comic_settings.json 不含 rotation 字段（枚举按变体名序列化）
        let json = r#"{"mode":"Ltr","double_page":true,"fit":"Page"}"#;
        let s: ComicReadingSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.rotation, 0);
        assert!(s.double_page);
    }

    #[test]
    fn test_comic_reading_settings_rotation_roundtrip() {
        let s = ComicReadingSettings {
            mode: ReadingMode::Ltr,
            double_page: false,
            fit: FitMode::Width,
            rotation: 270,
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: ComicReadingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }

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
            toolbar_display_mode: ToolbarDisplayMode::IconOnly,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.toolbar_display_mode, ToolbarDisplayMode::IconOnly);
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
                media_type: MediaType::Comic,
            }],
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(json.contains("Test"));
    }

    #[test]
    fn test_library_entry_default_media_type_is_comic() {
        let entry = LibraryEntry::default();
        assert_eq!(entry.media_type, MediaType::Comic);
    }

    #[test]
    fn test_library_entry_deserializes_missing_media_type_as_comic() {
        let json =
            r#"{"comic_id":"id","title":"Test","path":"/tmp","cover_path":null,"added_at":0}"#;
        let entry: LibraryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.media_type, MediaType::Comic);
    }

    #[test]
    fn test_media_type_video_audio_roundtrip() {
        let v = serde_json::to_string(&MediaType::Video).unwrap();
        let a = serde_json::to_string(&MediaType::Audio).unwrap();
        assert_eq!(v, "\"video\"");
        assert_eq!(a, "\"audio\"");
        assert_eq!(
            serde_json::from_str::<MediaType>(&v).unwrap(),
            MediaType::Video
        );
        assert_eq!(
            serde_json::from_str::<MediaType>(&a).unwrap(),
            MediaType::Audio
        );
    }

    #[test]
    fn test_library_entry_deserializes_media_type_video() {
        let json = r#"{"comic_id":"id","title":"T","path":"/tmp/v.mp4","cover_path":null,"added_at":0,"media_type":"video"}"#;
        let entry: LibraryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.media_type, MediaType::Video);
    }

    #[test]
    fn test_ebook_settings_default() {
        let s = EbookSettings::default();
        assert_eq!(s.reading_mode, EbookReadingMode::SinglePage);
        assert_eq!(s.font_family, "system-ui");
        assert_eq!(s.font_size, 16);
        assert!((s.line_height - 1.6).abs() < f32::EPSILON);
        assert_eq!(s.margin_horizontal, 24);
        assert_eq!(s.margin_vertical, 24);
        assert_eq!(s.theme, EbookTheme::Light);
        assert!(!s.enable_page_animation);
        assert!(!s.invert_scroll);
    }

    #[test]
    fn test_history_entry_defaults() {
        let h = HistoryEntry::default();
        assert_eq!(h.volume_index, 0);
        assert_eq!(h.page_index, 0);
        assert_eq!(h.char_offset, None);
    }

    #[test]
    fn test_history_entry_roundtrip_with_char_offset() {
        let h = HistoryEntry {
            comic_id: "ebook1".to_string(),
            path: PathBuf::from("/tmp/book.epub"),
            volume_index: 0,
            page_index: 2,
            char_offset: Some(1500),
            last_read_at: 12345,
        };
        let json = serde_json::to_string(&h).unwrap();
        let loaded: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_index, 2);
        assert_eq!(loaded.char_offset, Some(1500));
        assert_eq!(h, loaded);
    }

    #[test]
    fn test_history_entry_deserializes_missing_char_offset_as_none() {
        let json =
            r#"{"comic_id":"id","path":"/tmp","volume_index":0,"page_index":1,"last_read_at":0}"#;
        let h: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(h.char_offset, None);
    }

    #[test]
    fn test_bookmark_defaults_and_roundtrip() {
        let b = Bookmark {
            comic_id: "ebook1".to_string(),
            volume_index: 0,
            page_index: 3,
            char_offset: Some(1200),
            note: Some("note".to_string()),
        };
        let json = serde_json::to_string(&b).unwrap();
        let loaded: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_index, 3);
        assert_eq!(loaded.char_offset, Some(1200));
        assert_eq!(b, loaded);
    }

    #[test]
    fn test_bookmark_deserializes_missing_char_offset_as_none() {
        let json = r#"{"comic_id":"id","volume_index":0,"page_index":1,"note":null}"#;
        let b: Bookmark = serde_json::from_str(json).unwrap();
        assert_eq!(b.char_offset, None);
    }

    #[test]
    fn test_settings_validate_rejects_bad_ebook_margins() {
        let mut s = Settings::default();
        s.ebook.margin_horizontal = 250;
        assert!(s.validate().is_err());
        s.ebook.margin_horizontal = 24;
        s.ebook.margin_vertical = 250;
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_settings_clamp_ebook_margins() {
        let mut s = Settings::default();
        s.ebook.margin_horizontal = 300;
        s.ebook.margin_vertical = 400;
        s.clamp();
        assert_eq!(s.ebook.margin_horizontal, 200);
        assert_eq!(s.ebook.margin_vertical, 200);
    }

    #[test]
    fn test_settings_validate_rejects_empty_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = "   ".to_string();
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_settings_clamp_restores_default_font_family() {
        let mut s = Settings::default();
        s.ebook.font_family = String::new();
        s.clamp();
        assert_eq!(s.ebook.font_family, "system-ui");
    }

    #[test]
    fn test_default_shortcuts_cover_ebook_actions() {
        let s = Shortcuts::default();
        assert!(s.back_to_library.contains(&"Escape".to_string()));
        assert!(s.page_down.contains(&"PageDown".to_string()));
        assert!(s.page_up.contains(&"PageUp".to_string()));
    }

    #[test]
    fn test_shortcuts_default_first_last_page() {
        let s = Shortcuts::default();
        assert_eq!(s.first_page, vec!["Home".to_string()]);
        assert_eq!(s.last_page, vec!["End".to_string()]);
    }

    #[test]
    fn test_shortcuts_deserialize_missing_first_last_page_uses_defaults() {
        // 旧版 settings.json 不含 first_page/last_page 字段，应落到新默认值
        let json = r#"{"next_page":["ArrowRight"],"prev_page":["ArrowLeft"]}"#;
        let s: Shortcuts = serde_json::from_str(json).unwrap();
        assert_eq!(s.next_page, vec!["ArrowRight".to_string()]);
        assert_eq!(s.first_page, vec!["Home".to_string()]);
        assert_eq!(s.last_page, vec!["End".to_string()]);
    }
}
