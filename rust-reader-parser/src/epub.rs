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
        let mut doc =
            epub::doc::EpubDoc::new(path).map_err(|e| ParseError::InvalidEpub(format!("{}", e)))?;

        let title = doc.mdata("title").unwrap_or_default();
        let language = doc.mdata("language");
        let authors: Vec<String> = doc
            .metadata
            .get("creator")
            .map(|v| v.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let resources: Vec<EbookResource> = doc
            .resources
            .iter()
            .map(|(id, (href, mime_type))| EbookResource {
                id: id.clone(),
                href: href.clone(),
                mime_type: mime_type.clone(),
            })
            .collect();

        let spine: Vec<String> = doc.spine.clone();

        let mut chapters: Vec<EbookChapter> = doc
            .toc
            .iter()
            .enumerate()
            .map(|(idx, toc)| EbookChapter {
                index: idx,
                id: toc.content.clone(),
                href: toc.content.clone(),
                title: Some(toc.label.clone()),
            })
            .collect();

        if chapters.is_empty() {
            chapters = spine
                .iter()
                .enumerate()
                .filter_map(|(idx, idref)| {
                    doc.resources.get(idref).map(|(href, _)| EbookChapter {
                        index: idx,
                        id: idref.clone(),
                        href: href.clone(),
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
