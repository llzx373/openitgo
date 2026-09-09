use openitgo_core::models::{FitMode, ReadingMode};
use openitgo_storage::{
    json_store::JsonStore,
    models::{
        Bookmark, Bookmarks, ComicReadingSettings, History, HistoryEntry, Library, LibraryEntry,
        MediaType, Settings,
    },
};
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_settings_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let settings = Settings {
        cache_size_mb: 512,
        real_image_cache_pages: 20,
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let loaded = store.load_settings().unwrap();
    assert_eq!(settings, loaded);
}

#[test]
fn test_library_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let library = Library {
        entries: vec![LibraryEntry {
            comic_id: "id1".to_string(),
            title: "Test Comic".to_string(),
            path: PathBuf::from("/tmp/comic"),
            cover_path: Some(PathBuf::from("/tmp/cover.jpg")),
            added_at: 123,
            media_type: MediaType::Comic,
            tags: Vec::new(),
            page_count: Some(10),
        }],
    };
    store.save_library(&library).unwrap();
    let loaded = store.load_library().unwrap();
    assert_eq!(library, loaded);
}

#[test]
fn test_history_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let history = History {
        entries: vec![HistoryEntry {
            comic_id: "id1".to_string(),
            path: PathBuf::from("/tmp/comic"),
            volume_index: 0,
            page_index: 7,
            char_offset: None,
            last_read_at: 456,
        }],
    };
    store.save_history(&history).unwrap();
    let loaded = store.load_history().unwrap();
    assert_eq!(history, loaded);
}

#[test]
fn test_bookmarks_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let bookmarks = Bookmarks {
        entries: vec![Bookmark {
            comic_id: "id1".to_string(),
            volume_index: 0,
            page_index: 3,
            char_offset: Some(100),
            note: Some("remember this page".to_string()),
        }],
    };
    store.save_bookmarks(&bookmarks).unwrap();
    let loaded = store.load_bookmarks().unwrap();
    assert_eq!(bookmarks, loaded);
}

#[test]
fn test_comic_settings_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let mut settings = HashMap::new();
    settings.insert(
        "id1".to_string(),
        ComicReadingSettings {
            mode: ReadingMode::Rtl,
            double_page: true,
            fit: FitMode::Page,
            rotation: 0,
        },
    );
    store.save_comic_settings(&settings).unwrap();
    let loaded = store.load_comic_settings().unwrap();
    assert_eq!(settings, loaded);
}

#[test]
fn test_all_persisted_files_are_created() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    store.save_settings(&Settings::default()).unwrap();
    store.save_library(&Library::default()).unwrap();
    store.save_history(&History::default()).unwrap();
    store.save_bookmarks(&Bookmarks::default()).unwrap();
    store.save_comic_settings(&HashMap::new()).unwrap();
    store.save_reading_stats(&HashMap::new()).unwrap();

    assert!(tmp.path().join("settings.json").exists());
    assert!(tmp.path().join("library.json").exists());
    assert!(tmp.path().join("history.json").exists());
    assert!(tmp.path().join("bookmarks.json").exists());
    assert!(tmp.path().join("comic_settings.json").exists());
    assert!(tmp.path().join("reading_stats.json").exists());
}

#[test]
fn test_settings_deserialize_missing_fm_fields() {
    // 旧版 settings.json 无文件管理器字段 → 取默认值
    let json = r#"{"theme":"Dark"}"#;
    let s: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(s.fm_layout, "dual");
    assert!((s.fm_dual_ratio - 0.5).abs() < f32::EPSILON);
    assert!(s.fm_preview_open);
    assert_eq!(s.fm_sort_key, "name");
    assert!(s.fm_sort_asc);
    assert!(s.fm_confirm_delete);
    assert!(s.fm_show_hidden);
    assert_eq!(s.fm_dir_left, "");
    assert_eq!(s.fm_dir_right, "");
}

#[test]
fn test_settings_fm_fields_clamp() {
    let mut s = Settings {
        fm_dual_ratio: 0.9,
        fm_layout: "weird".to_string(),
        fm_sort_key: "x".to_string(),
        ..Default::default()
    };
    assert!(s.validate().is_err());
    s.clamp();
    assert!((s.fm_dual_ratio - 0.8).abs() < f32::EPSILON);
    assert_eq!(s.fm_layout, "dual");
    assert_eq!(s.fm_sort_key, "name");
    assert!(s.validate().is_ok());

    let mut s = Settings {
        fm_dual_ratio: 0.1,
        fm_layout: "single".to_string(),
        fm_sort_key: "mtime".to_string(),
        ..Default::default()
    };
    assert!(s.validate().is_err());
    s.clamp();
    assert!((s.fm_dual_ratio - 0.2).abs() < f32::EPSILON);
    assert_eq!(s.fm_layout, "single");
    assert_eq!(s.fm_sort_key, "mtime");
    assert!(s.validate().is_ok());
}

#[test]
fn test_settings_fm_fields_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    let settings = Settings {
        fm_layout: "single".to_string(),
        fm_dual_ratio: 0.35,
        fm_preview_open: false,
        fm_sort_key: "size".to_string(),
        fm_sort_asc: false,
        fm_confirm_delete: false,
        fm_show_hidden: false,
        fm_dir_left: "F:\\comics".to_string(),
        fm_dir_right: "D:\\downloads".to_string(),
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let loaded = store.load_settings().unwrap();
    assert_eq!(settings, loaded);
}

#[test]
fn test_settings_fm_sort_key_ext() {
    // ext 排序键（扩展名）：validate/clamp 均视为合法值，不回退 name。
    let mut s = Settings {
        fm_sort_key: "ext".to_string(),
        ..Default::default()
    };
    assert!(s.validate().is_ok());
    s.clamp();
    assert_eq!(s.fm_sort_key, "ext");
}
