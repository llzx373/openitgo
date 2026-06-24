use rust_reader_parser::parse_ebook;
use std::io::Write;

#[test]
fn test_parse_txt_ebook() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "# Chapter 1\nHello world.").unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 1);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("Chapter 1"));
}

#[test]
fn test_parse_markdown_ebook() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.md");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "# Part 1\n\nHello.").unwrap();
    writeln!(file, "## Chapter 2\n\nWorld.").unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("Part 1"));
    assert_eq!(ebook.chapters[1].title.as_deref(), Some("Chapter 2"));
}

#[test]
fn test_parse_txt_fallback_chunks() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    let words: Vec<String> = (0..6000).map(|i| format!("word{}", i)).collect();
    writeln!(file, "{}", words.join(" ")).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("第 1 章"));
}

#[test]
fn test_parse_unsupported_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.unknown");
    std::fs::write(&path, b"hello").unwrap();

    let result = parse_ebook(&path);
    assert!(result.is_err());
}
