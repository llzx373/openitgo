use rust_reader_storage::{
    json_store::JsonStore,
    models::{Bookmark, Bookmarks, History, HistoryEntry, Library, LibraryEntry, Settings},
};
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
            note: Some("remember this page".to_string()),
        }],
    };
    store.save_bookmarks(&bookmarks).unwrap();
    let loaded = store.load_bookmarks().unwrap();
    assert_eq!(bookmarks, loaded);
}

#[test]
fn test_all_persisted_files_are_created() {
    let tmp = tempfile::tempdir().unwrap();
    let store = JsonStore::new(tmp.path());
    store.save_settings(&Settings::default()).unwrap();
    store.save_library(&Library::default()).unwrap();
    store.save_history(&History::default()).unwrap();
    store.save_bookmarks(&Bookmarks::default()).unwrap();

    assert!(tmp.path().join("settings.json").exists());
    assert!(tmp.path().join("library.json").exists());
    assert!(tmp.path().join("history.json").exists());
    assert!(tmp.path().join("bookmarks.json").exists());
}
