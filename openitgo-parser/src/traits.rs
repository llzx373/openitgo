use openitgo_core::models::Comic;
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
    #[error("archive is encrypted and requires a password")]
    PasswordRequired,
    #[error("incorrect archive password")]
    PasswordIncorrect,
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
/// same filename in different directories. The id is a blake3 digest prefix
/// (16 hex chars) so it stays stable across Rust toolchain upgrades.
pub fn stable_comic_id(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let hash = blake3::hash(canonical.as_os_str().as_encoded_bytes());
    hash.as_bytes()[..8]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
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

    #[test]
    fn stable_comic_id_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.cbz");
        std::fs::write(&path, b"x").unwrap();
        let a = stable_comic_id(&path);
        let b = stable_comic_id(&path);
        assert_eq!(a, b);
    }

    #[test]
    fn stable_comic_id_differs_for_different_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.cbz");
        let b = tmp.path().join("b.cbz");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        assert_ne!(stable_comic_id(&a), stable_comic_id(&b));
    }

    #[test]
    fn stable_comic_id_is_16_hex_chars() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.cbz");
        std::fs::write(&path, b"x").unwrap();
        let id = stable_comic_id(&path);
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, id.to_ascii_lowercase());
    }

    #[test]
    fn stable_comic_id_uses_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.cbz");
        std::fs::write(&real, b"x").unwrap();
        #[cfg(unix)]
        {
            let link = tmp.path().join("link.cbz");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(stable_comic_id(&real), stable_comic_id(&link));
        }
    }
}
