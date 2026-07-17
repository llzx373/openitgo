use crate::traits::{is_image_extension, ParseError, Parser};
use openitgo_core::models::{Comic, Page, PageSource, Volume};
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
        parse_rar(path, None)
    }
}

/// Map unrar errors to password-specific variants; everything else keeps
/// the legacy IO flattening. 保守映射：只有明确的 MissingPassword /
/// BadPassword 才转换，不误伤真实损坏的包。
fn classify_rar_error(e: unrar::error::UnrarError) -> ParseError {
    match e.code {
        unrar::error::Code::MissingPassword => ParseError::PasswordRequired,
        unrar::error::Code::BadPassword => ParseError::PasswordIncorrect,
        _ => ParseError::Io(IoError::other(e)),
    }
}

/// Parse a RAR/CBR archive, decrypting with `password` when needed.
///
/// unrar 0.5 exposes no per-header encryption flag, so after a successful
/// listing we probe-read the first image entry: data-encrypted (`rar -p`)
/// archives only fail at read time (`MissingPassword`), and a wrong
/// password surfaces as `BadPassword` or a CRC `BadData` (the latter is
/// treated as PasswordIncorrect — 宁可误报密码错误，不可静默接受乱码数据；
/// 见 probe_rar_password 实测记录）。
pub fn parse_rar(path: &Path, password: Option<&str>) -> Result<Comic, ParseError> {
    let archive_path = path.to_path_buf();

    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    let open_archive = builder.open_for_listing().map_err(classify_rar_error)?;

    let mut names: Vec<String> = Vec::new();
    // Build a name -> header-position index so readers can jump directly to
    // the desired entry instead of scanning the archive from the start every
    // time. This index is stored alongside each PageSource.
    let mut header_positions: HashMap<String, usize> = HashMap::new();
    for (position, entry) in open_archive.enumerate() {
        let header = entry.map_err(classify_rar_error)?;
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

    // 读探针：数据加密的包列表可成功，只有读条目时才暴露密码需求。
    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    let mut processing = builder.open_for_processing().map_err(classify_rar_error)?;
    let target_position = header_positions.get(&names[0]).copied().unwrap_or(0);
    let mut current_position: usize = 0;
    let probe_result: Result<(), ParseError> = loop {
        let maybe_entry = match processing.read_header() {
            Ok(e) => e,
            Err(e) => break Err(classify_rar_error(e)),
        };
        let Some(entry) = maybe_entry else {
            break Err(ParseError::InvalidArchive(
                "rar probe: entry vanished".to_string(),
            ));
        };
        if current_position >= target_position {
            break match entry.read() {
                Ok(_) => Ok(()),
                Err(e) => {
                    if password.is_some() && e.code == unrar::error::Code::BadData {
                        // 错误密码解密出的乱码过不了 CRC（probe 实测见 Step 2c）。
                        Err(ParseError::PasswordIncorrect)
                    } else {
                        Err(classify_rar_error(e))
                    }
                }
            };
        }
        match entry.skip() {
            Ok(a) => {
                processing = a;
                current_position += 1;
            }
            Err(e) => break Err(classify_rar_error(e)),
        }
    };
    probe_result?;

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

    #[test]
    fn test_classify_rar_error_maps_password_codes() {
        use unrar::error::{Code, UnrarError, When};
        assert!(matches!(
            classify_rar_error(UnrarError::from(Code::MissingPassword, When::Open)),
            ParseError::PasswordRequired
        ));
        assert!(matches!(
            classify_rar_error(UnrarError::from(Code::BadPassword, When::Process)),
            ParseError::PasswordIncorrect
        ));
        assert!(matches!(
            classify_rar_error(UnrarError::from(Code::BadArchive, When::Open)),
            ParseError::Io(_)
        ));
    }

    #[test]
    fn test_parse_header_encrypted_rar_requires_password() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/encrypted-header-pw123.rar"
        ));
        assert!(matches!(
            parse_rar(&path, None),
            Err(ParseError::PasswordRequired)
        ));
    }

    #[test]
    fn test_parse_header_encrypted_rar_correct_password() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/encrypted-header-pw123.rar"
        ));
        let comic = parse_rar(&path, Some("pw123")).unwrap();
        assert!(!comic.volumes[0].pages.is_empty());
    }

    #[test]
    fn test_parse_data_encrypted_rar_requires_password_on_read_probe() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/encrypted-files-pw123.rar"
        ));
        assert!(matches!(
            parse_rar(&path, None),
            Err(ParseError::PasswordRequired)
        ));
    }

    #[test]
    fn test_parse_data_encrypted_rar_wrong_password() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/encrypted-files-pw123.rar"
        ));
        assert!(matches!(
            parse_rar(&path, Some("nope")),
            Err(ParseError::PasswordIncorrect)
        ));
    }

    #[test]
    fn test_parse_data_encrypted_rar_correct_password() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/encrypted-files-pw123.rar"
        ));
        let comic = parse_rar(&path, Some("pw123")).unwrap();
        assert!(!comic.volumes[0].pages.is_empty());
    }
}
