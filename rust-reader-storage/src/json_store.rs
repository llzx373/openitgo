use crate::models::{Bookmarks, History, Library, Settings};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
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
        dirs::config_dir().map(|d| d.join("rust-reader"))
    }

    pub fn ensure_dir(&self) -> Result<(), StorageError> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), StorageError> {
        self.write_json("settings.json", settings)
    }

    pub fn load_settings(&self) -> Result<Settings, StorageError> {
        self.read_json("settings.json")
    }

    pub fn save_library(&self, library: &Library) -> Result<(), StorageError> {
        self.write_json("library.json", library)
    }

    pub fn load_library(&self) -> Result<Library, StorageError> {
        self.read_json("library.json")
    }

    pub fn save_history(&self, history: &History) -> Result<(), StorageError> {
        self.write_json("history.json", history)
    }

    pub fn load_history(&self) -> Result<History, StorageError> {
        self.read_json("history.json")
    }

    pub fn save_bookmarks(&self, bookmarks: &Bookmarks) -> Result<(), StorageError> {
        self.write_json("bookmarks.json", bookmarks)
    }

    pub fn load_bookmarks(&self) -> Result<Bookmarks, StorageError> {
        self.read_json("bookmarks.json")
    }

    fn write_json<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), StorageError> {
        self.ensure_dir()?;
        let path = self.dir.join(name);
        let json = serde_json::to_string_pretty(value)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn read_json<T: serde::de::DeserializeOwned + Default>(
        &self,
        name: &str,
    ) -> Result<T, StorageError> {
        let path = self.dir.join(name);
        if !path.exists() {
            return Ok(T::default());
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Settings;

    #[test]
    fn test_roundtrip_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();
        assert_eq!(settings, loaded);
    }
}
