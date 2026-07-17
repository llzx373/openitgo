use openitgo_parser::parse_ebook;
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

#[test]
fn test_parse_gbk_txt_with_headings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book_gbk.txt");
    let (bytes, _, _) = encoding_rs::GBK.encode(
        "第一章 风起\n\n他睁开眼睛，发现自己躺在陌生的床上。\n\n第二章 云涌\n\n她合上书本，望向窗外。",
    );
    std::fs::write(&path, &bytes).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    assert_eq!(ebook.total_chapters(), 2);
    assert_eq!(ebook.chapters[0].title.as_deref(), Some("第一章 风起"));
    assert_eq!(ebook.chapters[1].title.as_deref(), Some("第二章 云涌"));
}

fn build_epub_with_resources() -> Vec<u8> {
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

    zip.start_file("OEBPS/content.opf", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Resource Book</dc:title>
    <dc:identifier id="id">res-1</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="Images/pic.png" media-type="image/png"/>
    <item id="font" href="Fonts/f.ttf" media-type="font/ttf"/>
    <item id="css" href="Styles/s.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"#,
    )
    .unwrap();

    zip.start_file("OEBPS/Text/ch1.xhtml", options).unwrap();
    zip.write_all(
        br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head><body><p>before</p><img src="../Images/pic.png"/><p>after</p></body></html>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/Images/pic.png", options).unwrap();
    zip.write_all(b"\x89PNG\r\n\x1a\nFAKE").unwrap();

    zip.start_file("OEBPS/Fonts/f.ttf", options).unwrap();
    zip.write_all(b"\x00\x01\x00\x00FAKETTF").unwrap();

    zip.start_file("OEBPS/Styles/s.css", options).unwrap();
    zip.write_all(
        br#"body { color: red; }
@font-face { font-family: "Embedded"; src: url("../Fonts/f.ttf"); }
"#,
    )
    .unwrap();

    zip.finish().unwrap().into_inner()
}

#[test]
fn test_epub_chapter_rewrites_img_and_injects_font_face() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("res.epub");
    std::fs::write(&path, build_epub_with_resources()).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    let html = openitgo_parser::html::render_chapter_html(&ebook, 0).unwrap();
    assert!(
        html.contains("ebook://reader/res/OEBPS/Images/pic.png"),
        "got: {html}"
    );
    assert!(html.contains("@font-face"), "got: {html}");
    assert!(
        html.contains("ebook://reader/res/OEBPS/Fonts/f.ttf"),
        "got: {html}"
    );
    assert!(
        !html.contains("color: red"),
        "book layout CSS must be dropped: {html}"
    );
}

#[test]
fn test_read_epub_resource() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("res.epub");
    std::fs::write(&path, build_epub_with_resources()).unwrap();

    let ebook = parse_ebook(&path).unwrap();
    let (mime, bytes) =
        openitgo_parser::html::read_epub_resource(&ebook, "OEBPS/Images/pic.png").unwrap();
    assert_eq!(mime, "image/png");
    assert!(bytes.starts_with(b"\x89PNG"));
    assert!(openitgo_parser::html::read_epub_resource(&ebook, "OEBPS/nope.png").is_none());
}
