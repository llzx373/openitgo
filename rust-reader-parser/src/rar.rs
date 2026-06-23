use crate::traits::{is_image_extension, ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::collections::HashMap;
use std::io::Error as IoError;
use std::path::Path;

pub struct RarParser;

impl Parser for RarParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("rar") || e.eq_ignore_ascii_case("cbr"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let archive_path = path.to_path_buf();

        let open_archive = unrar::Archive::new(path)
            .open_for_listing()
            .map_err(|e| ParseError::Io(IoError::other(e)))?;

        let mut names: Vec<String> = Vec::new();
        // Build a name -> header-position index so readers can jump directly to
        // the desired entry instead of scanning the archive from the start every
        // time. This index is stored alongside each PageSource.
        let mut header_positions: HashMap<String, usize> = HashMap::new();
        for (position, entry) in open_archive.enumerate() {
            let header = entry.map_err(|e| ParseError::Io(IoError::other(e)))?;
            if header.is_file() {
                let name = header.filename.to_string_lossy().to_string();
                header_positions.insert(name.clone(), position);
                if is_image_name(&name) {
                    names.push(name);
                }
            }
        }

        names.sort();

        if names.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = names
            .into_iter()
            .enumerate()
            .map(|(idx, name)| {
                let header_position = header_positions.get(&name).copied().unwrap_or(usize::MAX);
                Page {
                    index: idx,
                    source: PageSource::RarEntry {
                        archive: archive_path.clone(),
                        name,
                        header_position,
                    },
                }
            })
            .collect();

        let title = path
            .file_stem()
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

fn is_image_name(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(is_image_extension)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_cbr() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/sample.cbr"));
        let comic = RarParser::parse(&path).unwrap();
        assert!(!comic.volumes[0].pages.is_empty());
        assert_eq!(
            comic.volumes[0].pages[0].source,
            PageSource::RarEntry {
                archive: path,
                name: "01.png".to_string(),
                header_position: 0,
            }
        );
    }
}
