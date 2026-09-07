//! 通用压缩包浏览：统一列出 ZIP/RAR/7z/TAR 的条目清单，
//! 解压引擎见同目录 `extract.rs`。

mod extract;

pub use extract::{extract_archive, ExtractOptions, ExtractProgress};

use crate::traits::ParseError;
use std::io::Read;
use std::path::Path;

/// 压缩包内单条目的元数据（目录项与非图片文件也在列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// 条目在包内的路径名（与 `extract_archive` 的 selection 精确匹配）。
    pub name: String,
    pub is_dir: bool,
    /// 解压后大小（字节）。
    pub size: u64,
    /// 压缩后大小；RAR/TAR 接口拿不到时为 None。
    pub compressed_size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    Rar,
    SevenZ,
    Tar,
}

/// TAR 的压缩变体（按文件名后缀识别，含 .tar.gz 等双后缀）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TarCompression {
    None,
    Gz,
    Xz,
    Zst,
    Bz2,
}

/// 按扩展名识别压缩包格式；不支持的扩展名返回 None。
pub fn archive_kind(path: &Path) -> Option<ArchiveKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".zip") || name.ends_with(".cbz") {
        Some(ArchiveKind::Zip)
    } else if name.ends_with(".rar") || name.ends_with(".cbr") {
        Some(ArchiveKind::Rar)
    } else if name.ends_with(".7z") {
        Some(ArchiveKind::SevenZ)
    } else if tar_compression(&name).is_some() {
        Some(ArchiveKind::Tar)
    } else {
        None
    }
}

/// 识别 TAR 压缩变体；双后缀（.tar.gz）须先于单后缀判断。
fn tar_compression(lower_name: &str) -> Option<TarCompression> {
    if lower_name.ends_with(".tar.gz") || lower_name.ends_with(".tgz") {
        Some(TarCompression::Gz)
    } else if lower_name.ends_with(".tar.xz") || lower_name.ends_with(".txz") {
        Some(TarCompression::Xz)
    } else if lower_name.ends_with(".tar.zst") {
        Some(TarCompression::Zst)
    } else if lower_name.ends_with(".tar.bz2") || lower_name.ends_with(".tbz2") {
        Some(TarCompression::Bz2)
    } else if lower_name.ends_with(".tar") {
        Some(TarCompression::None)
    } else {
        Option::None
    }
}

/// 按压缩变体给 TAR 文件套上对应的流式解码器。
pub(crate) fn tar_reader(path: &Path) -> Result<Box<dyn Read + Send>, ParseError> {
    let file = std::fs::File::open(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(
        match tar_compression(&name).unwrap_or(TarCompression::None) {
            TarCompression::None => Box::new(file),
            TarCompression::Gz => Box::new(flate2::read::GzDecoder::new(file)),
            TarCompression::Xz => Box::new(xz2::read::XzDecoder::new(file)),
            TarCompression::Zst => Box::new(zstd::stream::Decoder::new(file)?),
            TarCompression::Bz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        },
    )
}

/// 列出压缩包全部条目（含目录项与非图片文件）。
/// TAR 无加密概念，忽略 `password`。
pub fn list_entries(path: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ParseError> {
    match archive_kind(path) {
        Some(ArchiveKind::Zip) => list_zip(path, password),
        Some(ArchiveKind::Rar) => list_rar(path, password),
        Some(ArchiveKind::SevenZ) => list_sevenz(path, password),
        Some(ArchiveKind::Tar) => list_tar(path),
        None => Err(ParseError::Unsupported),
    }
}

/// ZIP：by_index_raw 不解密即可拿到加密条目的名称/大小元数据；
/// 发现加密条目时按 zip.rs 模式尽早验证密码。
fn list_zip(path: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ParseError> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ParseError::InvalidArchive(e.to_string()))?;

    let mut entries = Vec::with_capacity(archive.len());
    let mut first_encrypted: Option<usize> = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
        if entry.encrypted() && first_encrypted.is_none() {
            first_encrypted = Some(i);
        }
        entries.push(ArchiveEntry {
            name: entry.name().to_string(),
            is_dir: entry.is_dir(),
            size: entry.size(),
            compressed_size: Some(entry.compressed_size()),
        });
    }

    if let Some(idx) = first_encrypted {
        let pw = password.ok_or(ParseError::PasswordRequired)?;
        match archive.by_index_decrypt(idx, pw.as_bytes()) {
            Ok(_) => {}
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ParseError::PasswordIncorrect);
            }
            Err(e) => return Err(ParseError::InvalidArchive(e.to_string())),
        }
    }
    Ok(entries)
}

/// RAR：open_for_listing 全量枚举。数据加密包（rar -p）列表可成功，
/// 密码需求留给解压读条目时暴露（与 parse_rar 语义一致）。
fn list_rar(path: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ParseError> {
    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    let open_archive = builder
        .open_for_listing()
        .map_err(crate::rar::classify_rar_error)?;
    let mut entries = Vec::new();
    for item in open_archive {
        let header = item.map_err(crate::rar::classify_rar_error)?;
        entries.push(ArchiveEntry {
            name: header.filename.to_string_lossy().to_string(),
            is_dir: header.is_directory(),
            size: header.unpacked_size,
            compressed_size: None,
        });
    }
    Ok(entries)
}

/// sevenz-rust2 错误 → ParseError 的保守映射。
/// MaybeBadPassword / 带密码时的 CRC 校验失败 → 密码错误
/// （宁可误报密码错误，不可静默接受乱码数据）。
pub(crate) fn classify_sevenz_error(e: sevenz_rust2::Error, had_password: bool) -> ParseError {
    use sevenz_rust2::Error as SE;
    match e {
        SE::PasswordRequired => ParseError::PasswordRequired,
        SE::MaybeBadPassword(_) => ParseError::PasswordIncorrect,
        SE::ChecksumVerificationFailed if had_password => ParseError::PasswordIncorrect,
        SE::Io(io_err, _) => ParseError::Io(io_err),
        other => ParseError::InvalidArchive(other.to_string()),
    }
}

pub(crate) fn sevenz_password(password: Option<&str>) -> sevenz_rust2::Password {
    match password {
        Some(pw) => sevenz_rust2::Password::new(pw),
        None => sevenz_rust2::Password::empty(),
    }
}

/// 7z：Archive::open_with_password 读头部即可拿到全部条目元数据。
/// 头部加密且无密码时 open 直接报 PasswordRequired；仅内容加密的包
/// 列表可成功，密码需求在解压解码时暴露。
fn list_sevenz(path: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ParseError> {
    let archive = sevenz_rust2::Archive::open_with_password(path, &sevenz_password(password))
        .map_err(|e| classify_sevenz_error(e, password.is_some()))?;
    Ok(archive
        .files
        .iter()
        .map(|f| ArchiveEntry {
            name: f.name.clone(),
            is_dir: f.is_directory,
            size: f.size,
            compressed_size: f.has_stream.then_some(f.compressed_size),
        })
        .collect())
}

/// TAR：按压缩变体套解码器后单遍流式枚举。
fn list_tar(path: &Path) -> Result<Vec<ArchiveEntry>, ParseError> {
    let reader = tar_reader(path)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();
    for item in archive.entries()? {
        let entry = item?;
        let name = String::from_utf8_lossy(entry.path_bytes().as_ref()).to_string();
        entries.push(ArchiveEntry {
            name,
            is_dir: entry.header().entry_type().is_dir(),
            size: entry.header().size().unwrap_or(0),
            compressed_size: None,
        });
    }
    Ok(entries)
}

// pub(crate) 以便 extract.rs 的测试复用造包辅助函数。
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn archive_kind_dispatch_table() {
        let cases: &[(&str, Option<ArchiveKind>)] = &[
            ("a.zip", Some(ArchiveKind::Zip)),
            ("a.CBZ", Some(ArchiveKind::Zip)),
            ("a.rar", Some(ArchiveKind::Rar)),
            ("a.cbr", Some(ArchiveKind::Rar)),
            ("a.7z", Some(ArchiveKind::SevenZ)),
            ("a.tar", Some(ArchiveKind::Tar)),
            ("a.tgz", Some(ArchiveKind::Tar)),
            ("a.tar.gz", Some(ArchiveKind::Tar)),
            ("a.txz", Some(ArchiveKind::Tar)),
            ("a.tar.xz", Some(ArchiveKind::Tar)),
            ("a.tar.zst", Some(ArchiveKind::Tar)),
            ("a.tar.bz2", Some(ArchiveKind::Tar)),
            ("a.tbz2", Some(ArchiveKind::Tar)),
            ("a.pdf", None),
            ("a.epub", None),
            ("a.gz", None),
            ("noext", None),
        ];
        for (name, want) in cases {
            assert_eq!(archive_kind(Path::new(name)), *want, "case: {name}");
        }
    }

    /// 写测试 zip：含子目录、目录项、非图片文件。
    pub(crate) fn write_test_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.add_directory("sub/", options).unwrap();
        zip.start_file("a.txt", options).unwrap();
        zip.write_all(b"hello a").unwrap();
        zip.start_file("sub/b.png", options).unwrap();
        zip.write_all(b"png-bytes").unwrap();
        zip.start_file("notes.md", options).unwrap();
        zip.write_all(b"# notes").unwrap();
        zip.finish().unwrap();
    }

    pub(crate) fn write_encrypted_zip(path: &Path, password: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .with_aes_encryption(zip::AesMode::Aes256, password);
        zip.start_file("secret.txt", options).unwrap();
        zip.write_all(b"top secret").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn list_zip_all_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.zip");
        write_test_zip(&path);
        let entries = list_entries(&path, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub/", "a.txt", "sub/b.png", "notes.md"]);
        assert!(entries[0].is_dir);
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, 7);
        assert!(entries[1].compressed_size.is_some());
    }

    #[test]
    fn list_encrypted_zip_password_states() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.zip");
        write_encrypted_zip(&path, "s3cret");
        assert!(matches!(
            list_entries(&path, None),
            Err(ParseError::PasswordRequired)
        ));
        assert!(matches!(
            list_entries(&path, Some("nope")),
            Err(ParseError::PasswordIncorrect)
        ));
        let entries = list_entries(&path, Some("s3cret")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "secret.txt");
    }

    #[test]
    fn list_rar_header_encrypted_password_states() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted-header-pw123.rar");
        assert!(matches!(
            list_entries(&path, None),
            Err(ParseError::PasswordRequired)
        ));
        assert!(matches!(
            list_entries(&path, Some("nope")),
            Err(ParseError::PasswordIncorrect)
        ));
        let entries = list_entries(&path, Some("pw123")).unwrap();
        assert!(!entries.is_empty());
    }

    #[test]
    fn list_rar_data_encrypted_lists_without_password() {
        // 数据加密包（rar -p）列表可读，与 parse_rar 语义一致。
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted-files-pw123.rar");
        let entries = list_entries(&path, None).unwrap();
        assert!(!entries.is_empty());
    }

    /// 用 sevenz-rust2 writer 现场生成 7z 包。
    pub(crate) fn write_test_7z(path: &Path) {
        let mut writer = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        writer
            .push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_directory("sub"),
                None::<&[u8]>,
            )
            .unwrap();
        writer
            .push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file("a.txt"),
                Some(&b"hello a"[..]),
            )
            .unwrap();
        writer
            .push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file("sub/b.png"),
                Some(&b"png-bytes"[..]),
            )
            .unwrap();
        writer.finish().unwrap();
    }

    /// 生成内容 AES 加密的 7z（头部不加密，列表可读）。
    pub(crate) fn write_encrypted_7z(path: &Path, password: &str) {
        let mut writer = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::from(
            sevenz_rust2::encoder_options::AesEncoderOptions::new(sevenz_rust2::Password::new(
                password,
            )),
        )]);
        writer
            .push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file("secret.txt"),
                Some(&b"top secret"[..]),
            )
            .unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn list_7z_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.7z");
        write_test_7z(&path);
        let entries = list_entries(&path, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.txt", "sub/b.png"]);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].size, 7);
    }

    #[test]
    fn list_encrypted_7z_without_password_still_lists() {
        // 仅内容加密的 7z 头部可读，列表成功；密码需求留给解压暴露。
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.7z");
        write_encrypted_7z(&path, "pw123");
        let entries = list_entries(&path, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "secret.txt");
    }

    /// 用 tar::Builder + flate2 现场生成 .tar.gz 包。
    pub(crate) fn write_test_tar_gz(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "sub", std::io::empty())
            .unwrap();
        let mut file_header = tar::Header::new_gnu();
        file_header.set_entry_type(tar::EntryType::Regular);
        file_header.set_size(7);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder
            .append_data(&mut file_header, "a.txt", &b"hello a"[..])
            .unwrap();
        let mut file_header2 = tar::Header::new_gnu();
        file_header2.set_entry_type(tar::EntryType::Regular);
        file_header2.set_size(9);
        file_header2.set_mode(0o644);
        file_header2.set_cksum();
        builder
            .append_data(&mut file_header2, "sub/b.png", &b"png-bytes"[..])
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn list_tar_gz_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.tar.gz");
        write_test_tar_gz(&path);
        let entries = list_entries(&path, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.txt", "sub/b.png"]);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].size, 7);
    }

    #[test]
    fn list_unsupported_extension() {
        assert!(matches!(
            list_entries(Path::new("a.pdf"), None),
            Err(ParseError::Unsupported)
        ));
    }
}
