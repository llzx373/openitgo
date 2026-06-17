use rust_reader_core::models::Comic;
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
}

pub trait Parser: Send + Sync {
    fn supports(path: &Path) -> bool
    where
        Self: Sized;
    fn parse(path: &Path) -> Result<Comic, ParseError>
    where
        Self: Sized;
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
