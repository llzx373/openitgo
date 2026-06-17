use crate::traits::{ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::path::{Path, PathBuf};

pub struct FolderParser;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "avif"];

impl Parser for FolderParser {
    fn supports(path: &Path) -> bool {
        path.is_dir()
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_image(p))
            .collect();
        entries.sort();

        if entries.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = entries
            .into_iter()
            .enumerate()
            .map(|(idx, path)| Page {
                index: idx,
                source: PageSource::File(path),
            })
            .collect();

        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(Comic {
            id: title.clone(),
            title,
            path: path.to_path_buf(),
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_empty_folder_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = FolderParser::parse(tmp.path());
        assert!(matches!(result, Err(ParseError::NoPages)));
    }

    #[test]
    fn test_parse_folder_with_images() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("01.png"), b"fake").unwrap();
        fs::write(tmp.path().join("02.jpg"), b"fake").unwrap();
        let comic = FolderParser::parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 2);
    }
}
