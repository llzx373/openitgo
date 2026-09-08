//! 通用压缩包浏览：统一列出 ZIP/RAR/7z/TAR 的条目清单，
//! 解压引擎见同目录 `extract.rs`。

mod encoding;
mod extract;

pub use encoding::{decode_text_guess, decode_zip_entry_name};
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
            name: decode_zip_entry_name(entry.name_raw(), false),
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

/// 压缩包内容分类：按图片占比启发式判断是漫画包还是文件包。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveClass {
    Comic,
    Files,
}

/// ≥80% 图片判定为漫画包：images*5 >= files*4 且 files > 0。
/// 目录项与垃圾名（见 `is_junk_name`）不计入分子分母。
pub fn classify_archive(entries: &[ArchiveEntry]) -> ArchiveClass {
    let mut files = 0usize;
    let mut images = 0usize;
    for e in entries {
        if e.is_dir || is_junk_name(&e.name) {
            continue;
        }
        files += 1;
        if crate::traits::is_comic_image_name(&e.name) {
            images += 1;
        }
    }
    if files > 0 && images * 5 >= files * 4 {
        ArchiveClass::Comic
    } else {
        ArchiveClass::Files
    }
}

/// 垃圾名判定：任一路径组件为 `__MACOSX`（`/` 与 `\` 都算分隔符），
/// 或 basename 以 `._` 开头 / 为 `.DS_Store`（大小写不敏感）。
fn is_junk_name(name: &str) -> bool {
    let basename = name.split(['/', '\\']).next_back().unwrap_or("");
    if basename.starts_with("._") || basename.eq_ignore_ascii_case(".DS_Store") {
        return true;
    }
    name.split(['/', '\\']).any(|part| part == "__MACOSX")
}

/// 读取单个条目内容到内存（预览用）。TAR 无加密概念，忽略 `password`。
pub fn read_entry(path: &Path, name: &str, password: Option<&str>) -> Result<Vec<u8>, ParseError> {
    match archive_kind(path) {
        Some(ArchiveKind::Zip) => read_zip_entry(path, name, password),
        Some(ArchiveKind::Rar) => read_rar_entry(path, name, password),
        Some(ArchiveKind::SevenZ) => read_sevenz_entry(path, name, password),
        Some(ArchiveKind::Tar) => read_tar_entry(path, name),
        None => Err(ParseError::Unsupported),
    }
}

/// ZIP 条目名经 `decode_zip_entry_name` 重解码，与 zip crate 内部的
/// CP437 解码名不再一致，因此 `by_name` 无法命中——改为按索引扫描比较
/// 解码后的名字（首个命中生效）。
fn find_zip_index(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<usize, ParseError> {
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
        if decode_zip_entry_name(entry.name_raw(), false) == name {
            return Ok(i);
        }
    }
    Err(ParseError::InvalidArchive(format!(
        "entry not found: {name}"
    )))
}

fn read_zip_entry(path: &Path, name: &str, password: Option<&str>) -> Result<Vec<u8>, ParseError> {
    use zip::result::ZipError;
    let map_open_err = |e: ZipError| match e {
        ZipError::InvalidPassword => ParseError::PasswordIncorrect,
        ZipError::UnsupportedArchive(msg) if msg == ZipError::PASSWORD_REQUIRED => {
            ParseError::PasswordRequired
        }
        ZipError::FileNotFound => ParseError::InvalidArchive(format!("entry not found: {name}")),
        other => ParseError::InvalidArchive(other.to_string()),
    };
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
    let index = find_zip_index(&mut archive, name)?;
    let mut entry = match password {
        Some(pw) => archive
            .by_index_decrypt(index, pw.as_bytes())
            .map_err(map_open_err)?,
        None => archive.by_index(index).map_err(map_open_err)?,
    };
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut data)?;
    Ok(data)
}

/// RAR：顺序扫描头部，按文件名匹配（预览不需要 header_position 跳转）。
fn read_rar_entry(path: &Path, name: &str, password: Option<&str>) -> Result<Vec<u8>, ParseError> {
    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    let mut archive = builder
        .open_for_processing()
        .map_err(crate::rar::classify_rar_error)?;
    loop {
        let Some(entry) = archive
            .read_header()
            .map_err(crate::rar::classify_rar_error)?
        else {
            return Err(ParseError::InvalidArchive(format!(
                "entry not found: {name}"
            )));
        };
        if entry.entry().is_file() && entry.entry().filename.to_string_lossy() == name {
            let (data, _rest) = entry.read().map_err(crate::rar::classify_rar_error)?;
            return Ok(data);
        }
        archive = entry.skip().map_err(crate::rar::classify_rar_error)?;
    }
}

/// 7z：ArchiveReader::read_file 按名读取；Io 包装的 CRC 失败
/// （错密码解出乱码）经 extract 侧的 classify_sevenz_io_error 归一。
fn read_sevenz_entry(
    path: &Path,
    name: &str,
    password: Option<&str>,
) -> Result<Vec<u8>, ParseError> {
    use sevenz_rust2::Error as SE;
    let had_password = password.is_some();
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_password(password))
        .map_err(|e| classify_sevenz_error(e, had_password))?;
    reader.read_file(name).map_err(|e| match e {
        SE::FileNotFound => ParseError::InvalidArchive(format!("entry not found: {name}")),
        SE::Io(io_err, _) => extract::classify_sevenz_io_error(io_err, had_password),
        other => classify_sevenz_error(other, had_password),
    })
}

/// TAR：单遍流式扫描，按 `path_bytes` 的 lossy 字符串精确匹配（不做分隔符归一）。
fn read_tar_entry(path: &Path, name: &str) -> Result<Vec<u8>, ParseError> {
    let reader = tar_reader(path)?;
    let mut archive = tar::Archive::new(reader);
    for item in archive.entries()? {
        let mut entry = item?;
        if String::from_utf8_lossy(entry.path_bytes().as_ref()) == name {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            return Ok(data);
        }
    }
    Err(ParseError::InvalidArchive(format!(
        "entry not found: {name}"
    )))
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

/// 读取压缩包注释；非 zip / 无注释 / 读取失败 → None。
/// RAR/7z/TAR 暂不支持（unrar/sevenz/tar API 不暴露注释），恒 None。
/// zip 注释无 UTF-8 标志位，按 `decode_zip_entry_name` 同一启发式解码。
pub fn read_comment(path: &Path) -> Option<String> {
    if archive_kind(path) != Some(ArchiveKind::Zip) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let archive = zip::ZipArchive::new(file).ok()?;
    let raw = archive.comment();
    if raw.is_empty() {
        return None;
    }
    Some(decode_zip_entry_name(raw, false))
}

/// Bandizip 式智能解压目录判定：包内文件全部同居单一顶层组件（单个顶层
/// 目录，或只有一个顶层文件）→ false（直接解压进输出目录）；顶层散乱
/// （多个顶层文件/目录混合）或没有文件条目 → true（需再包一层同名子目录）。
pub fn needs_wrapper_dir(entries: &[ArchiveEntry]) -> bool {
    let mut top: Option<&str> = None;
    for e in entries.iter().filter(|e| !e.is_dir) {
        // 取首个非空路径组件（'/' 与 '\' 都算分隔符）。
        let Some(comp) = e.name.split(['/', '\\']).find(|s| !s.is_empty()) else {
            continue;
        };
        match top {
            None => top = Some(comp),
            Some(t) if t == comp => {}
            Some(_) => return true,
        }
    }
    // 无文件条目 → true（无害）；恰好一个顶层组件 → false。
    top.is_none()
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

    fn file_entry(name: &str) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir: false,
            size: 0,
            compressed_size: None,
        }
    }

    #[test]
    fn classify_all_images_is_comic() {
        let entries: Vec<ArchiveEntry> = ["1.png", "2.jpg", "3.webp"]
            .iter()
            .map(|n| file_entry(n))
            .collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Comic);
    }

    #[test]
    fn classify_exactly_eighty_percent_is_comic() {
        let entries: Vec<ArchiveEntry> = ["1.png", "2.png", "3.png", "4.png", "notes.txt"]
            .iter()
            .map(|n| file_entry(n))
            .collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Comic);
    }

    #[test]
    fn classify_sixty_percent_is_files() {
        let entries: Vec<ArchiveEntry> = ["1.png", "2.png", "3.png", "a.txt", "b.txt"]
            .iter()
            .map(|n| file_entry(n))
            .collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Files);
    }

    #[test]
    fn classify_all_text_is_files() {
        let entries: Vec<ArchiveEntry> = ["a.txt", "b.md"].iter().map(|n| file_entry(n)).collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Files);
    }

    #[test]
    fn classify_empty_is_files() {
        assert_eq!(classify_archive(&[]), ArchiveClass::Files);
    }

    #[test]
    fn classify_dirs_only_is_files() {
        let entries: Vec<ArchiveEntry> = ["sub/", "pics/"]
            .iter()
            .map(|n| ArchiveEntry {
                name: n.to_string(),
                is_dir: true,
                size: 0,
                compressed_size: None,
            })
            .collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Files);
    }

    #[test]
    fn classify_junk_excluded_from_denominator() {
        // 垃圾名不计入 files：4 图 + 3 垃圾仍是漫画包
        // （若垃圾计入则 4/7 < 80%，会误判为 Files）。
        let entries: Vec<ArchiveEntry> = [
            "1.png",
            "2.png",
            "3.png",
            "4.png",
            "__MACOSX/x.png",
            "._a.png",
            ".DS_Store",
        ]
        .iter()
        .map(|n| file_entry(n))
        .collect();
        assert_eq!(classify_archive(&entries), ArchiveClass::Comic);
    }

    #[test]
    fn read_entry_zip_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.zip");
        write_test_zip(&path);
        assert_eq!(read_entry(&path, "a.txt", None).unwrap(), b"hello a");
        assert_eq!(read_entry(&path, "sub/b.png", None).unwrap(), b"png-bytes");
        assert!(matches!(
            read_entry(&path, "missing.txt", None),
            Err(ParseError::InvalidArchive(_))
        ));
    }

    #[test]
    fn read_entry_7z_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.7z");
        write_test_7z(&path);
        assert_eq!(read_entry(&path, "a.txt", None).unwrap(), b"hello a");
        assert_eq!(read_entry(&path, "sub/b.png", None).unwrap(), b"png-bytes");
        assert!(matches!(
            read_entry(&path, "missing.txt", None),
            Err(ParseError::InvalidArchive(_))
        ));
    }

    #[test]
    fn read_entry_tar_gz_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.tar.gz");
        write_test_tar_gz(&path);
        assert_eq!(read_entry(&path, "a.txt", None).unwrap(), b"hello a");
        assert_eq!(read_entry(&path, "sub/b.png", None).unwrap(), b"png-bytes");
        assert!(matches!(
            read_entry(&path, "missing.txt", None),
            Err(ParseError::InvalidArchive(_))
        ));
    }

    #[test]
    fn read_entry_encrypted_zip_password_states() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("enc.zip");
        write_encrypted_zip(&path, "s3cret");
        assert!(matches!(
            read_entry(&path, "secret.txt", None),
            Err(ParseError::PasswordRequired)
        ));
        assert!(matches!(
            read_entry(&path, "secret.txt", Some("nope")),
            Err(ParseError::PasswordIncorrect)
        ));
        assert_eq!(
            read_entry(&path, "secret.txt", Some("s3cret")).unwrap(),
            b"top secret"
        );
    }

    #[test]
    fn read_entry_rar_data_encrypted() {
        // 数据加密包（rar -p）：列表可读，读条目必须带密码。
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted-files-pw123.rar");
        let entries = list_entries(&path, None).unwrap();
        let name = entries
            .iter()
            .find(|e| !e.is_dir)
            .expect("fixture has a file entry")
            .name
            .clone();
        let data = read_entry(&path, &name, Some("pw123")).unwrap();
        assert!(!data.is_empty());
        assert!(read_entry(&path, &name, Some("nope")).is_err());
    }

    #[test]
    fn read_entry_unsupported_extension() {
        assert!(matches!(
            read_entry(Path::new("a.pdf"), "x", None),
            Err(ParseError::Unsupported)
        ));
    }

    /// 手写最小 stored zip（原始字节文件名 + 可选归档注释），用于测试
    /// 非 UTF-8 条目名——zip crate 的 writer 只接受 &str 且会自置 UTF-8 位。
    pub(crate) fn write_raw_zip(path: &Path, files: &[(&[u8], &[u8])], comment: &[u8]) {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for (name, data) in files {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let crc = crc.sum();
            let offset = out.len() as u32;
            // local file header
            out.extend_from_slice(&0x04034b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags（不置 UTF-8 位）
            out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name);
            out.extend_from_slice(data);
            // central directory entry
            central.extend_from_slice(&0x02014b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method
            central.extend_from_slice(&0u16.to_le_bytes()); // mod time
            central.extend_from_slice(&0u16.to_le_bytes()); // mod date
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra len
            central.extend_from_slice(&0u16.to_le_bytes()); // file comment len
            central.extend_from_slice(&0u16.to_le_bytes()); // disk number
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name);
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        // end of central directory
        out.extend_from_slice(&0x06054b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn list_and_read_zip_shift_jis_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sjis.zip");
        let (raw_name, _, _) = encoding_rs::SHIFT_JIS.encode("日本語.txt");
        write_raw_zip(&path, &[(&raw_name, b"hello")], b"");
        let entries = list_entries(&path, None).unwrap();
        assert_eq!(entries[0].name, "日本語.txt");
        // 解码后的名字必须能反向读回条目内容
        assert_eq!(read_entry(&path, "日本語.txt", None).unwrap(), b"hello");
    }

    #[test]
    fn list_and_read_zip_gbk_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gbk.zip");
        let (raw_name, _, _) = encoding_rs::GBK.encode("中文漫画.txt");
        write_raw_zip(&path, &[(&raw_name, b"hello")], b"");
        let entries = list_entries(&path, None).unwrap();
        assert_eq!(entries[0].name, "中文漫画.txt");
        assert_eq!(read_entry(&path, "中文漫画.txt", None).unwrap(), b"hello");
    }

    #[test]
    fn list_zip_utf8_name_without_flag_unaffected() {
        // 未置 UTF-8 标志位但原始字节本身是合法 UTF-8 → 直用，不走 CJK 猜测。
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("utf8.zip");
        write_raw_zip(&path, &[("中文名.txt".as_bytes(), b"hello")], b"");
        let entries = list_entries(&path, None).unwrap();
        assert_eq!(entries[0].name, "中文名.txt");
    }

    #[test]
    fn read_comment_roundtrip_and_absence() {
        let tmp = tempfile::tempdir().unwrap();
        // 无注释 → None
        let plain = tmp.path().join("plain.zip");
        write_test_zip(&plain);
        assert_eq!(read_comment(&plain), None);
        // GBK 注释 → 启发式解码
        let gbk = tmp.path().join("gbk-comment.zip");
        let (raw_comment, _, _) = encoding_rs::GBK.encode("中文漫画 第一卷");
        write_raw_zip(&gbk, &[(b"a.txt", b"hi")], &raw_comment);
        assert_eq!(read_comment(&gbk).as_deref(), Some("中文漫画 第一卷"));
        // 非 zip → None
        let tar = tmp.path().join("t.tar.gz");
        write_test_tar_gz(&tar);
        assert_eq!(read_comment(&tar), None);
    }

    fn wrapper_entry(name: &str, is_dir: bool) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size: 0,
            compressed_size: None,
        }
    }

    #[test]
    fn needs_wrapper_dir_cases() {
        // 单一顶层目录 → 直接解压
        let single_dir = [
            wrapper_entry("sub", true),
            wrapper_entry("sub/a.png", false),
            wrapper_entry("sub/b.png", false),
        ];
        assert!(!needs_wrapper_dir(&single_dir));
        // 文件全部同居一个顶层目录（无目录条目）→ 直接解压
        let implicit_dir = [
            wrapper_entry("sub/a.png", false),
            wrapper_entry("sub/b.png", false),
        ];
        assert!(!needs_wrapper_dir(&implicit_dir));
        // 只有一个顶层文件 → 直接解压
        assert!(!needs_wrapper_dir(&[wrapper_entry("a.png", false)]));
        // 顶层散乱 → 需包一层
        let scattered = [
            wrapper_entry("a.png", false),
            wrapper_entry("sub/b.png", false),
        ];
        assert!(needs_wrapper_dir(&scattered));
        // 顶层目录 + 裸文件混合 → 需包一层
        let mixed = [
            wrapper_entry("sub", true),
            wrapper_entry("sub/a.png", false),
            wrapper_entry("loose.txt", false),
        ];
        assert!(needs_wrapper_dir(&mixed));
        // 反斜杠分隔符同样按组件切分
        let backslash = [
            wrapper_entry("sub\\a.png", false),
            wrapper_entry("sub\\b.png", false),
        ];
        assert!(!needs_wrapper_dir(&backslash));
        // 无文件条目 → true（无害）
        assert!(needs_wrapper_dir(&[]));
        assert!(needs_wrapper_dir(&[wrapper_entry("sub", true)]));
    }
}
