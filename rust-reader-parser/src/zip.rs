use crate::traits::{ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::io::Read;
use std::path::Path;

pub struct ZipParser;

impl Parser for ZipParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "cbz"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let file = std::fs::File::open(path)?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| ParseError::InvalidArchive(e.to_string()))?;

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
            if file.is_file() && is_image_name(file.name()) {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                entries.push((file.name().to_string(), bytes));
            }
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        if entries.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = entries
            .into_iter()
            .enumerate()
            .map(|(idx, (_, bytes))| Page {
                index: idx,
                source: PageSource::Bytes(bytes),
            })
            .collect();

        let title = path
            .file_stem()
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

fn is_image_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "avif"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_parse_cbz() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("01.png", options).unwrap();
            zip.write_all(b"fake").unwrap();
            zip.start_file("02.jpg", options).unwrap();
            zip.write_all(b"fake").unwrap();
            zip.finish().unwrap();
        }
        let comic = ZipParser::parse(&path).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 2);
    }
}
