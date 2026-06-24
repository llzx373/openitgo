use rust_reader_parser::parse_ebook;
use std::io::Write;

fn write_text_file(path: &std::path::Path, content: &str) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

#[test]
fn test_parse_txt_with_headings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.txt");
    write_text_file(
        &path,
        "# Chapter 1\nHello world.\n\n# Chapter 2\nMore text.",
    );

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("Chapter 1"));
    assert_eq!(ebook.chapters[1].title.as_deref(), Some("Chapter 2"));
}

#[test]
fn test_parse_txt_without_headings_falls_back_to_word_count() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.txt");
    let words: Vec<String> = (0..6000).map(|i| format!("word{}", i)).collect();
    write_text_file(&path, &words.join(" "));

    let ebook = parse_ebook(&path).unwrap();
    assert!(ebook.total_chapters() >= 2);
}

#[test]
fn test_parse_markdown_with_headings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.md");
    write_text_file(
        &path,
        "# Chapter 1\nHello world.\n\n## Chapter 2\nMore text.",
    );

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("Chapter 1"));
}

#[test]
fn test_parse_empty_text_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("empty.txt");
    write_text_file(&path, "");

    let err = parse_ebook(&path).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("no pages"));
}

fn build_minimal_epub(_dir: &std::path::Path, include_ncx: bool) -> Vec<u8> {
    use std::io::Cursor;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("mimetype", options).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    zip.start_file("META-INF/container.xml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#,
    )
    .unwrap();

    let opf = if include_ncx {
        br#"<?xml version="1.0"?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Minimal Epub</dc:title>
    <dc:identifier id="bookid">minimal</dc:identifier>
  </metadata>
  <manifest>
    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="chapter1"/>
  </spine>
</package>
"# as &[u8]
    } else {
        br#"<?xml version="1.0"?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Minimal Epub No Toc</dc:title>
    <dc:identifier id="bookid">minimal</dc:identifier>
  </metadata>
  <manifest>
    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter1"/>
  </spine>
</package>
"# as &[u8]
    };
    zip.start_file("OEBPS/content.opf", options).unwrap();
    zip.write_all(opf).unwrap();

    zip.start_file("OEBPS/chapter1.xhtml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body><p>Hello world.</p></body>
</html>
"#,
    )
    .unwrap();

    if include_ncx {
        zip.start_file("OEBPS/toc.ncx", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <navMap>
    <navPoint id="navpoint-1" playOrder="1">
      <navLabel><text>Chapter 1</text></navLabel>
      <content src="chapter1.xhtml"/>
    </navPoint>
  </navMap>
</ncx>
"#,
        )
        .unwrap();
    }

    zip.finish().unwrap().into_inner()
}

#[test]
fn test_parse_minimal_epub() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("minimal.epub");
    let bytes = build_minimal_epub(tmp.path(), true);
    std::fs::write(&path, bytes).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.title, "Minimal Epub");
    assert!(!ebook.chapters.is_empty());
}

#[test]
fn test_epub_without_toc_uses_spine() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("no-toc.epub");
    let bytes = build_minimal_epub(tmp.path(), false);
    std::fs::write(&path, bytes).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert!(!ebook.chapters.is_empty());
}
