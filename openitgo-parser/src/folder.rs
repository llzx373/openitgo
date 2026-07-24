use crate::traits::{is_comic_image_name, ParseError, Parser};
use openitgo_core::models::{Comic, Page, PageSource, Volume};
use std::path::{Path, PathBuf};

pub struct FolderParser;

impl Parser for FolderParser {
    fn supports(path: &Path) -> bool {
        path.is_dir()
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_comic_image_name)
            })
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
            id: crate::stable_comic_id(path),
            title,
            path: path.to_path_buf(),
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
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

    #[test]
    fn test_parse_folder_accepts_all_image_extensions() {
        use crate::traits::IMAGE_EXTENSIONS;

        let tmp = tempfile::tempdir().unwrap();
        for (i, ext) in IMAGE_EXTENSIONS.iter().enumerate() {
            fs::write(
                tmp.path().join(format!("{:02}.{}", i, ext.to_uppercase())),
                b"fake",
            )
            .unwrap();
        }
        fs::write(tmp.path().join("skip.txt"), b"not an image").unwrap();
        let comic = FolderParser::parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), IMAGE_EXTENSIONS.len());
    }

    #[test]
    fn test_parse_folder_skips_appledouble_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("._001.jpg"), [0u8, 5, 0x16, 7]).unwrap();
        fs::write(tmp.path().join("001.jpg"), b"fake").unwrap();
        let comic = FolderParser::parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 1);
        match &comic.volumes[0].pages[0].source {
            PageSource::File(p) => assert_eq!(p.file_name().unwrap(), "001.jpg"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
