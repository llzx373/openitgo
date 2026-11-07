use crate::chapters::text_ebook;
use crate::traits::ParseError;
use encoding_rs::{Encoding, UTF_8};
use openitgo_core::ebook::{Ebook, EbookChapter, EbookTextChapter, EbookTextSource};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) const STREAM_THRESHOLD_BYTES: u64 = 256 * 1024;
const UNIT_BYTES: usize = 64 * 1024;
const SNIFF_BYTES: usize = 256 * 1024;
const TARGET_CHAPTER_BYTES: usize = 256 * 1024;
pub(crate) const MAX_CHAPTER_BYTES: u64 = 512 * 1024;
const MAX_CHAPTERS: usize = 20_000;
const CHAPTER_WORDS: usize = 3000;

struct Unit {
    start: u64,
    bytes: Vec<u8>,
    line_end: bool,
}

struct UnitReader {
    file: File,
    encoding: &'static Encoding,
    carry: Vec<u8>,
    carry_start: u64,
    eof: bool,
    cancel: Option<Arc<AtomicBool>>,
}

impl UnitReader {
    fn new(
        path: &Path,
        encoding: &'static Encoding,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Self, ParseError> {
        Ok(Self {
            file: File::open(path).map_err(io_error)?,
            encoding,
            carry: Vec::with_capacity(UNIT_BYTES * 2),
            carry_start: 0,
            eof: false,
            cancel,
        })
    }

    fn next_unit(&mut self) -> Result<Option<Unit>, ParseError> {
        loop {
            if self
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
            {
                return Err(ParseError::InvalidText("已取消".to_string()));
            }
            if let Some(newline) = self.carry.iter().position(|&b| b == b'\n') {
                if newline < UNIT_BYTES {
                    return Ok(Some(self.take_unit(newline + 1, true)));
                }
            }
            if self.carry.len() >= UNIT_BYTES {
                let cut = self.aligned_cut(UNIT_BYTES);
                return Ok(Some(self.take_unit(cut, false)));
            }
            if self.eof {
                if self.carry.is_empty() {
                    return Ok(None);
                }
                let len = self.carry.len();
                return Ok(Some(self.take_unit(len, true)));
            }

            let mut buffer = [0u8; UNIT_BYTES];
            let read = self.file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                self.eof = true;
            } else {
                self.carry.extend_from_slice(&buffer[..read]);
            }
        }
    }

    fn aligned_cut(&self, proposed: usize) -> usize {
        let mut cut = proposed.min(self.carry.len());
        if self.encoding == UTF_8 {
            while cut > proposed.saturating_sub(4)
                && !std::str::from_utf8(&self.carry[..cut]).is_ok()
            {
                cut -= 1;
            }
            return cut.max(1);
        }
        for _ in 0..4 {
            let (decoded, _, _) = self.encoding.decode(&self.carry[..cut]);
            if !decoded.ends_with('\u{fffd}') || cut == 1 {
                break;
            }
            cut -= 1;
        }
        cut
    }

    fn take_unit(&mut self, len: usize, line_end: bool) -> Unit {
        let bytes: Vec<u8> = self.carry.drain(..len).collect();
        let start = self.carry_start;
        self.carry_start += len as u64;
        Unit {
            start,
            bytes,
            line_end,
        }
    }
}

struct DecodedUnit {
    start: u64,
    end: u64,
    text: String,
    line_end: bool,
}

impl DecodedUnit {
    fn new(unit: Unit, encoding: &'static Encoding) -> Self {
        let mut bytes = unit.bytes.as_slice();
        if unit.start == 0 {
            bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        }
        let (text, _, _) = encoding.decode(bytes);
        Self {
            start: unit.start,
            end: unit.start + unit.bytes.len() as u64,
            text: text.into_owned(),
            line_end: unit.line_end,
        }
    }

    fn is_whitespace(&self) -> bool {
        self.text.chars().all(char::is_whitespace)
    }
}

#[derive(Default)]
struct IndexBuilder {
    chapters: Vec<EbookChapter>,
    title: Option<String>,
    logical_chapter: usize,
    byte_start: Option<u64>,
    byte_end: u64,
    char_start: usize,
    char_count: usize,
    next_char_start: usize,
    normalize_whitespace: bool,
    logical_open: bool,
}

impl IndexBuilder {
    fn append(&mut self, unit: &DecodedUnit) -> Result<(), ParseError> {
        self.logical_open = true;
        if self.byte_start.is_none() {
            self.byte_start = Some(unit.start);
            self.char_start = self.next_char_start;
        }
        self.byte_end = unit.end;
        self.char_count = self.char_count.saturating_add(unit.text.chars().count());
        let byte_start = self.byte_start.unwrap_or(unit.start);
        if self.byte_end.saturating_sub(byte_start) >= TARGET_CHAPTER_BYTES as u64 {
            self.push_physical();
        }
        if self.chapters.len() > MAX_CHAPTERS {
            return Err(ParseError::InvalidText(
                "章节数量过多，无法安全打开".to_string(),
            ));
        }
        Ok(())
    }

    fn push_physical(&mut self) {
        let Some(byte_start) = self.byte_start.take() else {
            return;
        };
        let index = self.chapters.len();
        let id = format!("chapter-{}", index + 1);
        self.chapters.push(EbookChapter {
            index,
            id: id.clone(),
            href: format!("#{id}"),
            title: self.title.clone(),
            level: 0,
            text: Some(EbookTextChapter {
                byte_start,
                byte_end: self.byte_end,
                char_start: self.char_start,
                logical_chapter: self.logical_chapter,
                normalize_whitespace: self.normalize_whitespace,
            }),
        });
        self.next_char_start = self.next_char_start.saturating_add(self.char_count);
        self.char_count = 0;
    }

    fn finish_logical(&mut self) -> Result<(), ParseError> {
        if !self.logical_open {
            return Ok(());
        }
        if self.byte_start.is_none() {
            self.byte_start = Some(self.byte_end);
            self.char_start = self.next_char_start;
        }
        self.push_physical();
        self.logical_chapter = self.logical_chapter.saturating_add(1);
        self.logical_open = false;
        if self.chapters.len() > MAX_CHAPTERS {
            return Err(ParseError::InvalidText(
                "章节数量过多，无法安全打开".to_string(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn detect_encoding(path: &Path) -> Result<&'static Encoding, ParseError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::with_capacity(SNIFF_BYTES);
    file.by_ref()
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.starts_with(b"\xef\xbb\xbf") {
        return Ok(UTF_8);
    }
    match std::str::from_utf8(&bytes) {
        Ok(_) => return Ok(UTF_8),
        Err(error) if error.error_len().is_none() => return Ok(UTF_8),
        Err(_) => {}
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(&bytes, true);
    Ok(detector.guess(None, true))
}

#[cfg(test)]
pub(crate) fn parse_streaming(path: &Path) -> Result<Ebook, ParseError> {
    parse_streaming_with_cancel(path, None)
}

pub(crate) fn parse_streaming_with_cancel(
    path: &Path,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Ebook, ParseError> {
    let encoding = detect_encoding(path)?;
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);

    let (mut builder, saw_heading, non_whitespace) = scan_headings(path, encoding, cancel.clone())?;
    let normalize_whitespace = !saw_heading;
    if !saw_heading {
        if !non_whitespace {
            return Err(ParseError::NoPages);
        }
        builder = scan_words(path, encoding, cancel)?;
    }
    if builder.chapters.is_empty() {
        return Err(ParseError::NoPages);
    }

    let mut ebook = text_ebook(path, builder.chapters);
    ebook.text = Some(EbookTextSource {
        encoding: encoding.name().to_string(),
        file_len: metadata.len(),
        modified_ms,
    });
    ebook.chapters.iter_mut().for_each(|chapter| {
        if let Some(text) = chapter.text.as_mut() {
            text.normalize_whitespace = normalize_whitespace;
        }
    });
    Ok(ebook)
}

fn scan_headings(
    path: &Path,
    encoding: &'static Encoding,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(IndexBuilder, bool, bool), ParseError> {
    let mut reader = UnitReader::new(path, encoding, cancel)?;
    let mut builder = IndexBuilder::default();
    let mut saw_heading = false;
    let mut non_whitespace = false;
    while let Some(unit) = reader.next_unit()? {
        let unit = DecodedUnit::new(unit, encoding);
        non_whitespace |= !unit.is_whitespace();
        let is_heading = unit.line_end && unit.text.len() <= 4096 && is_heading(&unit.text);
        if is_heading {
            builder.finish_logical()?;
            saw_heading = true;
            builder.title = extract_title(&unit.text);
            builder.byte_end = unit.end;
            builder.logical_open = true;
        } else {
            builder.append(&unit)?;
        }
    }
    if saw_heading {
        builder.finish_logical()?;
    }
    Ok((builder, saw_heading, non_whitespace))
}

fn scan_words(
    path: &Path,
    encoding: &'static Encoding,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<IndexBuilder, ParseError> {
    let mut reader = UnitReader::new(path, encoding, cancel)?;
    let mut builder = IndexBuilder {
        normalize_whitespace: true,
        ..Default::default()
    };
    let mut words = 0usize;
    let mut in_word = false;
    while let Some(unit) = reader.next_unit()? {
        let unit = DecodedUnit::new(unit, encoding);
        for ch in unit.text.chars() {
            if ch.is_whitespace() {
                in_word = false;
            } else if !in_word {
                words = words.saturating_add(1);
                in_word = true;
            }
        }
        if builder.byte_start.is_none() && unit.is_whitespace() {
            continue;
        }
        builder.title = Some(format!("第 {} 章", builder.logical_chapter + 1));
        builder.append(&unit)?;
        if words >= CHAPTER_WORDS {
            words = 0;
            builder.finish_logical()?;
        }
    }
    if builder.byte_start.is_some() {
        builder.finish_logical()?;
    }
    Ok(builder)
}

pub(crate) fn read_chapter(ebook: &Ebook, chapter_index: usize) -> Result<String, ParseError> {
    let source = ebook.text.as_ref().ok_or(ParseError::Unsupported)?;
    let chapter = ebook
        .chapters
        .get(chapter_index)
        .and_then(|chapter| chapter.text.as_ref())
        .ok_or(ParseError::NoPages)?;
    let metadata = std::fs::metadata(&ebook.path).map_err(io_error)?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    if metadata.len() != source.file_len || modified_ms != source.modified_ms {
        return Err(ParseError::InvalidText(
            "文件已变化，请重新打开".to_string(),
        ));
    }
    let len = chapter.byte_end.saturating_sub(chapter.byte_start);
    if len > MAX_CHAPTER_BYTES {
        return Err(ParseError::InvalidText(
            "章节过大，无法安全渲染".to_string(),
        ));
    }
    let encoding = Encoding::for_label(source.encoding.as_bytes())
        .ok_or_else(|| ParseError::InvalidText("未知文本编码".to_string()))?;
    let mut file = File::open(&ebook.path).map_err(io_error)?;
    file.seek(SeekFrom::Start(chapter.byte_start))
        .map_err(io_error)?;
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(len).read_to_end(&mut bytes).map_err(io_error)?;
    if chapter.byte_start == 0 {
        bytes = bytes
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&bytes)
            .to_vec();
    }
    let (text, _, _) = encoding.decode(&bytes);
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if chapter.normalize_whitespace {
        let mut normalized = String::with_capacity(text.len());
        for (index, word) in text.split_whitespace().enumerate() {
            if index > 0 {
                normalized.push(' ');
            }
            normalized.push_str(word);
        }
        text = normalized;
    } else if text.ends_with('\n') {
        text.pop();
    }
    Ok(text)
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with('#')
        || trimmed.to_ascii_lowercase().starts_with("chapter ")
        || (trimmed.starts_with('第') && trimmed.contains('章'))
}

fn extract_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    Some(trimmed.trim_start_matches('#').trim().to_string())
}

fn io_error(error: std::io::Error) -> ParseError {
    ParseError::InvalidText(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_indexes_long_line_without_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        std::fs::write(&path, "汉".repeat(TARGET_CHAPTER_BYTES * 2)).unwrap();
        let ebook = parse_streaming(&path).unwrap();
        assert!(ebook.chapters.len() > 1);
        assert!(ebook.chapters.iter().all(|chapter| {
            chapter
                .text
                .as_ref()
                .is_some_and(|text| text.byte_end - text.byte_start <= MAX_CHAPTER_BYTES)
        }));
        let body = read_chapter(&ebook, 0).unwrap();
        assert!(!body.is_empty());
        assert!(body.chars().all(|ch| ch == '汉'));
    }

    #[test]
    fn streaming_preserves_heading_titles_and_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "# 第一章\n正文一\n# 第二章\n正文二").unwrap();
        let ebook = parse_streaming(&path).unwrap();
        assert_eq!(ebook.chapters.len(), 2);
        assert_eq!(ebook.chapters[0].title.as_deref(), Some("第一章"));
        assert_eq!(read_chapter(&ebook, 0).unwrap(), "正文一");
        assert_eq!(read_chapter(&ebook, 1).unwrap(), "正文二");
    }

    #[test]
    fn streaming_handles_gbk_across_read_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        let text = "汉".repeat(UNIT_BYTES + 100);
        let (bytes, _, _) = encoding_rs::GBK.encode(&text);
        std::fs::write(&path, bytes).unwrap();
        let ebook = parse_streaming(&path).unwrap();
        let body = read_chapter(&ebook, 0).unwrap();
        assert!(body.chars().all(|ch| ch == '汉'));
    }

    #[test]
    fn changed_file_is_rejected_before_range_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "汉".repeat(TARGET_CHAPTER_BYTES)).unwrap();
        let ebook = parse_streaming(&path).unwrap();
        std::fs::write(&path, "x").unwrap();
        assert!(read_chapter(&ebook, 0).is_err());
    }

    #[test]
    fn streaming_keeps_empty_heading_chapters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "# 一\n# 二\n正文").unwrap();
        let ebook = parse_streaming(&path).unwrap();
        assert_eq!(ebook.chapters.len(), 2);
        assert_eq!(read_chapter(&ebook, 0).unwrap(), "");
        assert_eq!(read_chapter(&ebook, 1).unwrap(), "正文");
    }

    #[test]
    fn physical_splits_keep_logical_chapter_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        let content = format!(
            "# 长章\n{}\n# 下一章\n结尾",
            "x".repeat(TARGET_CHAPTER_BYTES * 2)
        );
        std::fs::write(&path, content).unwrap();
        let ebook = parse_streaming(&path).unwrap();
        assert!(ebook.chapters.len() > 2);
        assert!(ebook.chapters[..ebook.chapters.len() - 1]
            .iter()
            .all(|chapter| chapter.text.as_ref().unwrap().logical_chapter == 0));
        assert_eq!(
            ebook
                .chapters
                .last()
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .logical_chapter,
            1
        );
        assert_eq!(ebook.legacy_text_position(1, 0).0, ebook.chapters.len() - 1);
    }

    #[test]
    fn streaming_decodes_bom_and_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(
            &path,
            b"\xef\xbb\xbf# \xe4\xb8\x80\r\n\xe6\xad\xa3\xe6\x96\x87\r\n",
        )
        .unwrap();
        let ebook = parse_streaming(&path).unwrap();
        assert_eq!(ebook.chapters[0].title.as_deref(), Some("一"));
        assert_eq!(read_chapter(&ebook, 0).unwrap(), "正文");
    }

    #[test]
    fn streaming_honors_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "正文").unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let result = parse_streaming_with_cancel(&path, Some(cancel));
        assert!(result.is_err());
    }
}
