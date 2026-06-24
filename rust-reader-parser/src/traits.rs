use rust_reader_core::models::Comic;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid archive: {0}")]
    InvalidArchive(String),
    #[error("Unsupported format")]
    Unsupported,
    #[error("No pages found")]
    NoPages,
    #[error("Invalid EPUB: {0}")]
    InvalidEpub(String),
    #[error("Invalid MOBI: {0}")]
    InvalidMobi(String),
    #[error("Invalid text file: {0}")]
    InvalidText(String),
}

pub trait Parser: Send + Sync {
    fn supports(path: &Path) -> bool
    where
        Self: Sized;
    fn parse(path: &Path) -> Result<Comic, ParseError>
    where
        Self: Sized;
}

/// Generate a stable, unique comic id from a filesystem path.
///
/// Using the canonicalized path avoids collisions when two comics have the
/// same filename in different directories. The hash is deterministic for a
/// given Rust release and host; it only needs to be stable within this app.
pub fn stable_comic_id(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.as_os_str().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Supported image file extensions, all lowercase.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "avif"];

/// Returns `true` if `ext` matches a supported image extension, case-insensitively.
pub fn is_image_extension(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_extensions_case_insensitive() {
        for ext in IMAGE_EXTENSIONS {
            assert!(is_image_extension(ext));
            assert!(is_image_extension(&ext.to_uppercase()));
        }
        assert!(!is_image_extension("txt"));
        assert!(!is_image_extension(""));
    }
}
