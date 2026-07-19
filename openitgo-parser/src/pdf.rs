use crate::traits::{ParseError, Parser};
use openitgo_core::models::{Comic, Page, PageSource, Volume};
use pdf_syntax::Pdf;
use std::path::Path;

pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let data = std::fs::read(path).map_err(ParseError::Io)?;
        // LoadPdfError 只实现 Debug（无 Display），用 {:?} 记录。
        let pdf = Pdf::new(data).map_err(|e| ParseError::InvalidArchive(format!("{e:?}")))?;

        let num_pages = pdf.pages().len();
        if num_pages == 0 {
            return Err(ParseError::NoPages);
        }

        let document = path.to_path_buf();
        let pages: Vec<Page> = (0..num_pages)
            .map(|page_number| Page {
                index: page_number,
                source: PageSource::PdfPage {
                    document: document.clone(),
                    page_number,
                },
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
            path: document,
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
    use std::path::PathBuf;

    #[test]
    fn test_parse_pdf() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/sample.pdf"));
        let comic = PdfParser::parse(&path).unwrap();
        assert!(!comic.volumes[0].pages.is_empty());
        assert_eq!(
            comic.volumes[0].pages[0].source,
            PageSource::PdfPage {
                document: path,
                page_number: 0,
            }
        );
    }
}
