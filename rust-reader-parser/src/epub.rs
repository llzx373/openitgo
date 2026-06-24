use crate::stable_comic_id;
use crate::traits::ParseError;
use rust_reader_core::ebook::{Ebook, EbookChapter, EbookResource};
use std::path::Path;

fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut entity = String::new();
        while let Some(&ch) = chars.peek() {
            if ch == ';' || ch == '&' {
                break;
            }
            entity.push(ch);
            chars.next();
        }
        if chars.peek() != Some(&';') {
            out.push('&');
            out.push_str(&entity);
            continue;
        }
        chars.next(); // consume ';'
        let decoded = if let Some(stripped) = entity.strip_prefix('#') {
            if let Some(hex) = stripped
                .strip_prefix('x')
                .or_else(|| stripped.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else {
                stripped.parse::<u32>().ok().and_then(char::from_u32)
            }
        } else {
            match entity.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => None,
            }
        };
        if let Some(d) = decoded {
            out.push(d);
        } else {
            out.push('&');
            out.push_str(&entity);
            out.push(';');
        }
    }
    out
}

fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title")?;
    let after_tag = html[start..].find('>')? + start + 1;
    let end = html[after_tag..].find("</title>")? + after_tag;
    let title = decode_html_entities(html[after_tag..end].trim());
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

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
                    title: Some(point.label.trim().to_string()),
                });
                *base_idx += 1;
                chapters.extend(collect_navpoints(&point.children, base_idx));
            }
            chapters
        }

        let mut chapters: Vec<EbookChapter> = collect_navpoints(&doc.toc, &mut 0);

        if chapters.is_empty() {
            let spine_items: Vec<(usize, String, String)> = doc
                .spine
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| {
                    doc.resources.get(&item.idref).map(|resource| {
                        (
                            idx,
                            item.idref.clone(),
                            resource.path.to_string_lossy().to_string(),
                        )
                    })
                })
                .collect();
            for (idx, idref, href) in spine_items {
                let title = doc
                    .get_resource_str(&idref)
                    .and_then(|(html, _)| extract_title(&html));
                chapters.push(EbookChapter {
                    index: idx,
                    id: idref,
                    href,
                    title,
                });
            }
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

    #[test]
    fn test_extract_title_normal() {
        let html = "<html><head><title>Chapter 1</title></head></html>";
        assert_eq!(extract_title(html), Some("Chapter 1".to_string()));
    }

    #[test]
    fn test_extract_title_trims_whitespace_and_newlines() {
        let html = "<title>\n  Part Two  \n</title>";
        assert_eq!(extract_title(html), Some("Part Two".to_string()));
    }

    #[test]
    fn test_extract_title_decodes_entities() {
        let html = "<title>Tom &amp; Jerry &#x2014; &#39;Quote&#39;</title>";
        assert_eq!(
            extract_title(html),
            Some("Tom & Jerry — 'Quote'".to_string())
        );
    }

    #[test]
    fn test_extract_title_missing() {
        assert_eq!(extract_title("<html><body></body></html>"), None);
    }

    #[test]
    fn test_extract_title_empty() {
        assert_eq!(extract_title("<title>   </title>"), None);
    }

    #[test]
    fn test_decode_html_entities_basic() {
        assert_eq!(decode_html_entities("&amp;"), "&");
        assert_eq!(decode_html_entities("&lt;"), "<");
        assert_eq!(decode_html_entities("&gt;"), ">");
        assert_eq!(decode_html_entities("&quot;"), "\"");
        assert_eq!(decode_html_entities("&apos;"), "'");
        assert_eq!(decode_html_entities("&#123;"), "{");
        assert_eq!(decode_html_entities("&#x7B;"), "{");
    }

    #[test]
    fn test_decode_html_entities_unknown_preserved() {
        assert_eq!(decode_html_entities("&unknown;"), "&unknown;");
        assert_eq!(decode_html_entities("&amp"), "&amp");
    }
}
