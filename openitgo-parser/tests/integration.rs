use openitgo_parser::archive::{archive_kind, ArchiveKind};
use openitgo_parser::parse;
use openitgo_parser::traits::ParseError;
use std::fs;
use std::io::Write;
use zip::write::SimpleFileOptions;

fn write_fake_image(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(name), b"fake").unwrap();
}

#[test]
fn test_parse_folder_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    write_fake_image(tmp.path(), "01.png");
    write_fake_image(tmp.path(), "02.jpg");
    write_fake_image(tmp.path(), "03.webp");

    let comic = parse(tmp.path()).unwrap();
    assert_eq!(comic.volumes.len(), 1);
    assert_eq!(comic.volumes[0].pages.len(), 3);
    assert_eq!(comic.path, tmp.path());
    assert!(!comic.id.is_empty());
}

#[test]
fn test_parse_cbz_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.cbz");
    {
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("01.png", options).unwrap();
        zip.write_all(b"fake").unwrap();
        zip.start_file("02.jpg", options).unwrap();
        zip.write_all(b"fake").unwrap();
        zip.finish().unwrap();
    }

    let comic = parse(&path).unwrap();
    assert_eq!(comic.volumes.len(), 1);
    assert_eq!(comic.volumes[0].pages.len(), 2);
    assert_eq!(comic.path, path);
}

#[test]
fn test_parse_sample_cbr() {
    let cbr = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sample.cbr");
    let comic = parse(&cbr).unwrap();
    assert!(!comic.volumes.is_empty());
    assert!(!comic.volumes[0].pages.is_empty());
}

#[test]
fn test_parse_sample_pdf() {
    let pdf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sample.pdf");
    let comic = parse(&pdf).unwrap();
    assert!(!comic.volumes.is_empty());
    assert!(!comic.volumes[0].pages.is_empty());
}

#[test]
fn test_parse_corrupt_pdf_returns_invalid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.pdf");
    std::fs::write(&path, b"definitely not a pdf").unwrap();
    let err = parse(&path).unwrap_err();
    assert!(matches!(err, ParseError::InvalidArchive(_)));
}

#[test]
fn test_archive_kind_dispatch_table() {
    use std::path::Path;
    let cases: &[(&str, Option<ArchiveKind>)] = &[
        ("book.zip", Some(ArchiveKind::Zip)),
        ("book.cbz", Some(ArchiveKind::Zip)),
        ("book.rar", Some(ArchiveKind::Rar)),
        ("book.cbr", Some(ArchiveKind::Rar)),
        ("book.7z", Some(ArchiveKind::SevenZ)),
        ("book.tar", Some(ArchiveKind::Tar)),
        ("book.tgz", Some(ArchiveKind::Tar)),
        ("book.tar.gz", Some(ArchiveKind::Tar)),
        ("book.txz", Some(ArchiveKind::Tar)),
        ("book.tar.xz", Some(ArchiveKind::Tar)),
        ("book.tar.zst", Some(ArchiveKind::Tar)),
        ("book.tar.bz2", Some(ArchiveKind::Tar)),
        ("book.tbz2", Some(ArchiveKind::Tar)),
        ("book.TAR.GZ", Some(ArchiveKind::Tar)),
        ("book.pdf", None),
        ("book.epub", None),
        ("book.gz", None),
    ];
    for (name, want) in cases {
        assert_eq!(archive_kind(Path::new(name)), *want, "case: {name}");
    }
}
