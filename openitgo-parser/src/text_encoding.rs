use crate::traits::ParseError;
use std::path::Path;

/// Read a text file as UTF-8, detecting legacy encodings (GBK/GB18030/Big5/...)
/// when the bytes are not valid UTF-8. Invalid sequences are replaced (lossy).
pub fn read_text_lossy(path: &Path) -> Result<String, ParseError> {
    let bytes = std::fs::read(path).map_err(|e| ParseError::InvalidText(format!("{}", e)))?;
    Ok(decode_text_bytes(&bytes))
}

/// Decode raw bytes: UTF-8 fast path (optional BOM stripped), otherwise
/// chardetng detection + encoding_rs transcoding.
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    let stripped = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(stripped) {
        return text.to_string();
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_utf8() {
        assert_eq!(decode_text_bytes("你好，世界".as_bytes()), "你好，世界");
    }

    #[test]
    fn test_decode_utf8_bom_stripped() {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend("第一章".as_bytes());
        assert_eq!(decode_text_bytes(&bytes), "第一章");
    }

    #[test]
    fn test_decode_gbk() {
        let (bytes, _, _) = encoding_rs::GBK.encode("第一章 新的开始\n\n他睁开了眼睛。");
        assert_eq!(
            decode_text_bytes(&bytes),
            "第一章 新的开始\n\n他睁开了眼睛。"
        );
    }

    #[test]
    fn test_decode_gb18030() {
        let (bytes, _, _) = encoding_rs::GB18030.encode("第二章 另一个故事的开端");
        assert_eq!(decode_text_bytes(&bytes), "第二章 另一个故事的开端");
    }

    #[test]
    fn test_decode_big5() {
        let (bytes, _, _) = encoding_rs::BIG5.encode("第三章 命中注定我愛你，睜開眼之後");
        assert_eq!(
            decode_text_bytes(&bytes),
            "第三章 命中注定我愛你，睜開眼之後"
        );
    }

    #[test]
    fn test_decode_invalid_bytes_lossy_no_panic() {
        let text = decode_text_bytes(&[0xff, 0xfe, 0x41, 0x80]);
        assert!(!text.is_empty());
    }

    #[test]
    fn test_read_text_lossy_missing_file() {
        assert!(read_text_lossy(Path::new("/nonexistent/nope.txt")).is_err());
    }

    #[test]
    fn test_read_text_lossy_gbk_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gbk.txt");
        let (bytes, _, _) = encoding_rs::GBK.encode("第一章 风起");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_text_lossy(&path).unwrap(), "第一章 风起");
    }
}
