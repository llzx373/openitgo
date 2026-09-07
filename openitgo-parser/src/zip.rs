use crate::traits::{is_comic_image_name, ParseError, Parser};
use openitgo_core::models::{Comic, Page, PageSource, Volume};
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
        parse_zip(path, None)
    }
}

/// Parse a ZIP/CBZ archive, decrypting encrypted entries with `password`.
/// Without a password the first encrypted image entry aborts the listing
/// with [`ParseError::PasswordRequired`]; a wrong password maps to
/// [`ParseError::PasswordIncorrect`].
pub fn parse_zip(path: &Path, password: Option<&str>) -> Result<Comic, ParseError> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ParseError::InvalidArchive(e.to_string()))?;

    let archive_path = path.to_path_buf();
    let mut entries: Vec<(usize, String)> = Vec::new();
    for i in 0..archive.len() {
        // The `by_index` temporary borrows `archive` until this statement
        // ends, so the decrypt retry happens in a separate statement below.
        // 条目名经 decode_zip_entry_name 重解码（zip crate 对非 UTF-8 名字
        // 按 CP437 解码会乱码；UTF-8 标志位未公开暴露，靠原始字节自判）。
        let encrypted = match archive.by_index(i) {
            Ok(entry) => {
                let name = crate::archive::decode_zip_entry_name(entry.name_raw(), false);
                if entry.is_file() && is_comic_image_name(&name) {
                    entries.push((i, name));
                }
                false
            }
            Err(zip::result::ZipError::UnsupportedArchive(msg))
                if msg == zip::result::ZipError::PASSWORD_REQUIRED =>
            {
                true
            }
            Err(e) => return Err(ParseError::InvalidArchive(e.to_string())),
        };
        if encrypted {
            let Some(pw) = password else {
                return Err(ParseError::PasswordRequired);
            };
            match archive.by_index_decrypt(i, pw.as_bytes()) {
                Ok(entry) => {
                    let name = crate::archive::decode_zip_entry_name(entry.name_raw(), false);
                    if entry.is_file() && is_comic_image_name(&name) {
                        entries.push((i, name));
                    }
                }
                Err(zip::result::ZipError::InvalidPassword) => {
                    return Err(ParseError::PasswordIncorrect);
                }
                Err(e) => return Err(ParseError::InvalidArchive(e.to_string())),
            }
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
        id: crate::stable_comic_id(path),
        title,
        path: path.to_path_buf(),
        volumes: vec![Volume {
            title: "Default".to_string(),
            pages,
        }],
    })
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
    fn test_parse_cbz_skips_appledouble_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("macos.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            // AppleDouble sorts before the real page ('.' < '0') and is not a JPEG.
            zip.start_file("folder/._001.jpg", options).unwrap();
            zip.write_all(&[0x00, 0x05, 0x16, 0x07]).unwrap();
            zip.start_file("folder/001.jpg", options).unwrap();
            zip.write_all(b"fake-jpeg").unwrap();
            zip.start_file("__MACOSX/folder/._001.jpg", options)
                .unwrap();
            zip.write_all(&[0x00, 0x05, 0x16, 0x07]).unwrap();
            zip.finish().unwrap();
        }
        let comic = ZipParser::parse(&path).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 1);
        match &comic.volumes[0].pages[0].source {
            PageSource::ZipEntry { name, .. } => assert_eq!(name, "folder/001.jpg"),
            other => panic!("unexpected source: {other:?}"),
        }
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

    #[test]
    fn test_parse_cbz_decodes_shift_jis_names() {
        // 非 UTF-8 原始条目名（手写 stored zip，未置 UTF-8 标志位）：
        // 页面名按 CJK 启发式解码，而非 CP437 乱码。
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sjis.cbz");
        let (raw_name, _, _) = encoding_rs::SHIFT_JIS.encode("日本語01.png");
        crate::archive::tests::write_raw_zip(&path, &[(&raw_name, b"fake")], b"");
        let comic = ZipParser::parse(&path).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 1);
        match &comic.volumes[0].pages[0].source {
            PageSource::ZipEntry { name, .. } => assert_eq!(name, "日本語01.png"),
            other => panic!("unexpected source: {other:?}"),
        }
    }

    fn write_encrypted_cbz(path: &std::path::Path, password: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .with_aes_encryption(zip::AesMode::Aes256, password);
        zip.start_file("01.png", options).unwrap();
        zip.write_all(b"fake-png").unwrap();
        zip.start_file("02.jpg", options).unwrap();
        zip.write_all(b"fake-jpg").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn test_parse_encrypted_zip_requires_password() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.cbz");
        write_encrypted_cbz(&path, "s3cret");
        assert!(matches!(
            parse_zip(&path, None),
            Err(ParseError::PasswordRequired)
        ));
    }

    #[test]
    fn test_parse_encrypted_zip_wrong_password() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.cbz");
        write_encrypted_cbz(&path, "s3cret");
        assert!(matches!(
            parse_zip(&path, Some("nope")),
            Err(ParseError::PasswordIncorrect)
        ));
    }

    #[test]
    fn test_parse_encrypted_zip_correct_password() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.cbz");
        write_encrypted_cbz(&path, "s3cret");
        let comic = parse_zip(&path, Some("s3cret")).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 2);
    }
}
