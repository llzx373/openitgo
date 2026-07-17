use crate::models::{Bookmarks, ComicReadingSettings, History, Library, Settings};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Settings validation error: {0}")]
    InvalidSettings(String),
}

pub struct JsonStore {
    dir: PathBuf,
}

impl JsonStore {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    pub fn default_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("openitgo"))
    }

    pub fn ensure_dir(&self) -> Result<(), StorageError> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), StorageError> {
        settings.validate().map_err(StorageError::InvalidSettings)?;
        self.write_json("settings.json", settings)
    }

    /// Load settings, falling back to the backup file if the main file is
    /// corrupt. If validation fails, the settings are clamped to valid ranges
    /// and returned alongside a boxed error so the caller can both use them and
    /// inform the user.
    pub fn load_settings(&self) -> Result<Settings, Box<(Settings, StorageError)>> {
        let mut settings: Settings = match self.read_json_with_backup("settings.json") {
            Ok(s) => s,
            Err(e) => return Err(Box::new((Settings::default(), e))),
        };
        if let Err(e) = settings.validate() {
            settings.clamp();
            return Err(Box::new((settings, StorageError::InvalidSettings(e))));
        }
        Ok(settings)
    }

    pub fn save_library(&self, library: &Library) -> Result<(), StorageError> {
        self.write_json("library.json", library)
    }

    pub fn load_library(&self) -> Result<Library, StorageError> {
        self.read_json_with_backup("library.json")
    }

    pub fn save_history(&self, history: &History) -> Result<(), StorageError> {
        self.write_json("history.json", history)
    }

    pub fn load_history(&self) -> Result<History, StorageError> {
        self.read_json_with_backup("history.json")
    }

    pub fn save_bookmarks(&self, bookmarks: &Bookmarks) -> Result<(), StorageError> {
        self.write_json("bookmarks.json", bookmarks)
    }

    pub fn load_bookmarks(&self) -> Result<Bookmarks, StorageError> {
        self.read_json_with_backup("bookmarks.json")
    }

    pub fn save_comic_settings(
        &self,
        settings: &HashMap<String, ComicReadingSettings>,
    ) -> Result<(), StorageError> {
        self.write_json("comic_settings.json", settings)
    }

    pub fn load_comic_settings(
        &self,
    ) -> Result<HashMap<String, ComicReadingSettings>, StorageError> {
        self.read_json_with_backup("comic_settings.json")
    }

    /// Write a JSON file atomically:
    /// 1. Serialize to a temporary file in the same directory.
    /// 2. If a previous file exists, copy it to `<name>.bak`.
    /// 3. Rename the temporary file to the target name.
    fn write_json<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), StorageError> {
        self.ensure_dir()?;
        let path = self.dir.join(name);
        let tmp_path = self.dir.join(format!("{}.tmp", name));
        let backup_path = self.dir.join(format!("{}.bak", name));

        let json = serde_json::to_string_pretty(value)?;
        std::fs::write(&tmp_path, json)?;

        if path.exists() {
            std::fs::copy(&path, &backup_path)?;
        }

        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    /// Read a JSON file. If parsing fails and a backup file exists, try to
    /// restore from the backup. If both fail, return the original error.
    fn read_json_with_backup<T: serde::de::DeserializeOwned + Default>(
        &self,
        name: &str,
    ) -> Result<T, StorageError> {
        let path = self.dir.join(name);
        if !path.exists() {
            return Ok(T::default());
        }

        match self.read_json_file(&path) {
            Ok(value) => Ok(value),
            Err(original) => {
                let backup_path = self.dir.join(format!("{}.bak", name));
                if backup_path.exists() {
                    match self.read_json_file(&backup_path) {
                        Ok(value) => Ok(value),
                        Err(_) => Err(original),
                    }
                } else {
                    Err(original)
                }
            }
        }
    }

    fn read_json_file<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<T, StorageError> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ComicReadingSettings, Settings};
    use openitgo_core::models::{FitMode, ReadingMode};
    use std::collections::HashMap;

    fn sample_comic_settings() -> HashMap<String, ComicReadingSettings> {
        let mut map = HashMap::new();
        map.insert(
            "comic-1".to_string(),
            ComicReadingSettings {
                mode: ReadingMode::Rtl,
                double_page: true,
                fit: FitMode::Width,
            },
        );
        map
    }

    #[test]
    fn test_roundtrip_comic_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let map = sample_comic_settings();
        store.save_comic_settings(&map).unwrap();
        let loaded = store.load_comic_settings().unwrap();
        assert_eq!(map, loaded);
    }

    #[test]
    fn test_load_comic_settings_missing_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let loaded = store.load_comic_settings().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_comic_settings_load_falls_back_to_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        store.save_comic_settings(&sample_comic_settings()).unwrap();
        let mut newer = HashMap::new();
        newer.insert(
            "comic-1".to_string(),
            ComicReadingSettings {
                mode: ReadingMode::Webtoon,
                double_page: false,
                fit: FitMode::Height,
            },
        );
        store.save_comic_settings(&newer).unwrap();

        // 损坏主文件；备份仍是上一份有效数据（与 settings 一致的行为）。
        std::fs::write(tmp.path().join("comic_settings.json"), "not json").unwrap();

        let loaded = store.load_comic_settings().unwrap();
        assert_eq!(loaded, sample_comic_settings());
    }

    #[test]
    fn test_comic_settings_backup_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        store.save_comic_settings(&sample_comic_settings()).unwrap();
        store.save_comic_settings(&HashMap::new()).unwrap();
        assert!(tmp.path().join("comic_settings.json.bak").exists());
    }

    #[test]
    fn test_roundtrip_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn test_settings_backup_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let settings = Settings {
            cache_size_mb: 200,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();

        let settings = Settings {
            cache_size_mb: 400,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();

        let backup = std::fs::read_to_string(tmp.path().join("settings.json.bak")).unwrap();
        assert!(backup.contains("200"));
    }

    #[test]
    fn test_settings_load_falls_back_to_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let settings = Settings {
            cache_size_mb: 200,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let settings = Settings {
            cache_size_mb: 256,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();

        // Corrupt the main file; the backup still holds the previous valid version.
        std::fs::write(tmp.path().join("settings.json"), "not json").unwrap();

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.cache_size_mb, 200);
    }

    #[test]
    fn test_settings_validation_rejects_invalid_values() {
        let settings = Settings {
            cache_size_mb: 50,
            ..Default::default()
        };
        assert!(settings.validate().is_err());

        assert!(Settings::default().validate().is_ok());
    }

    #[test]
    fn test_settings_load_clamps_invalid_values() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());

        // Write an invalid value directly; save_settings would reject it.
        std::fs::write(tmp.path().join("settings.json"), r#"{"cache_size_mb": 50}"#).unwrap();

        // After loading, invalid values are clamped and an error is reported.
        let (clamped, err) = *store.load_settings().unwrap_err();
        assert!(matches!(err, StorageError::InvalidSettings(_)));
        assert_eq!(clamped.cache_size_mb, 100);
    }

    #[test]
    fn test_media_settings_validate_and_clamp() {
        let mut settings = crate::models::Settings {
            media_volume: 150.0,
            media_speed: 0.0,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
        settings.clamp();
        assert_eq!(settings.media_volume, 100.0);
        assert_eq!(settings.media_speed, 0.1);
        assert!(settings.validate().is_ok());
        assert!(crate::models::Settings::default().validate().is_ok());

        // 另一侧边界：音量下限与倍速上限
        let mut settings = crate::models::Settings {
            media_volume: -5.0,
            media_speed: 20.0,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
        settings.clamp();
        assert_eq!(settings.media_volume, 0.0);
        assert_eq!(settings.media_speed, 16.0);
        assert!(settings.validate().is_ok());
    }
}
