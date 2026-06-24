use crate::stable_comic_id;
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookResource};
use std::path::Path;

pub struct EpubParser;

impl EpubParser {
    pub fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("epub"))
            .unwrap_or(false)
    }

    pub fn parse(path: &Path) -> Result<Ebook, ParseError> {
        let doc =
            epub::doc::EpubDoc::new(path).map_err(|e| ParseError::InvalidEpub(format!("{}", e)))?;

        let title = doc
            .mdata("title")
            .map(|m| m.value.clone())
            .unwrap_or_default();
        let language = doc.mdata("language").map(|m| m.value.clone());
        let authors: Vec<String> = doc
            .metadata
            .iter()
            .filter(|m| m.property == "creator")
            .map(|m| m.value.clone())
            .collect();

        let resources: Vec<EbookResource> = doc
            .resources
            .iter()
            .map(|(id, item)| EbookResource {
                id: id.clone(),
                href: item.path.to_string_lossy().to_string(),
                mime_type: item.mime.clone(),
            })
            .collect();

        let spine: Vec<String> = doc.spine.iter().map(|item| item.idref.clone()).collect();

        fn collect_navpoints(
            points: &[epub::doc::NavPoint],
            base_idx: &mut usize,
        ) -> Vec<EbookChapter> {
            let mut chapters = Vec::new();
            for point in points {
                chapters.push(EbookChapter {
                    index: *base_idx,
                    id: point.content.to_string_lossy().to_string(),
                    href: point.content.to_string_lossy().to_string(),
                    title: Some(point.label.clone()),
                });
                *base_idx += 1;
                chapters.extend(collect_navpoints(&point.children, base_idx));
            }
            chapters
        }

        let mut chapters: Vec<EbookChapter> = collect_navpoints(&doc.toc, &mut 0);

        if chapters.is_empty() {
            chapters = doc
                .spine
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| {
                    doc.resources.get(&item.idref).map(|resource| EbookChapter {
                        index: idx,
                        id: item.idref.clone(),
                        href: resource.path.to_string_lossy().to_string(),
                        title: None,
                    })
                })
                .collect();
        }

        if chapters.is_empty() {
            return Err(ParseError::NoPages);
        }

        Ok(Ebook {
            id: stable_comic_id(path),
            title,
            path: path.to_path_buf(),
            authors,
            language,
            resources,
            spine,
            chapters,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_epub() {
        assert!(EpubParser::supports(Path::new("book.epub")));
        assert!(!EpubParser::supports(Path::new("book.pdf")));
        assert!(!EpubParser::supports(Path::new("book.mobi")));
    }

    #[test]
    fn test_parse_missing_epub_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("missing.epub");

        let result = EpubParser::parse(&path);
        assert!(result.is_err());
    }
}
