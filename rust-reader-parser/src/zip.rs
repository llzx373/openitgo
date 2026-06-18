use crate::traits::{is_image_extension, ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
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

        let archive_path = path.to_path_buf();
        let mut entries: Vec<(usize, String)> = Vec::new();
        for i in 0..archive.len() {
            let entry = archive
                .by_index(i)
                .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
            if entry.is_file() && is_image_name(entry.name()) {
                entries.push((i, entry.name().to_string()));
            }
        }

        entries.sort_by(|a, b| a.1.cmp(&b.1));

        if entries.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = entries
            .into_iter()
            .enumerate()
            .map(|(idx, (zip_index, name))| Page {
                index: idx,
                source: PageSource::ZipEntry {
                    archive: archive_path.clone(),
                    name,
                    index: zip_index,
                },
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
    name.rsplit('.')
        .next()
        .map(is_image_extension)
        .unwrap_or(false)
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
        assert_eq!(
            comic.volumes[0].pages[0].source,
            PageSource::ZipEntry {
                archive: path,
                name: "01.png".to_string(),
                index: 0,
            }
        );
    }

    #[test]
    fn test_parse_cbz_accepts_all_image_extensions() {
        use crate::traits::IMAGE_EXTENSIONS;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (i, ext) in IMAGE_EXTENSIONS.iter().enumerate() {
                zip.start_file(format!("{:02}.{}", i, ext.to_uppercase()), options)
                    .unwrap();
                zip.write_all(b"fake").unwrap();
            }
            zip.start_file("skip.txt", options).unwrap();
            zip.write_all(b"not an image").unwrap();
            zip.finish().unwrap();
        }
        let comic = ZipParser::parse(&path).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), IMAGE_EXTENSIONS.len());
    }
}
