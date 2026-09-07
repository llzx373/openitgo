//! ZIP 条目名/注释的编码修复：zip crate 在 UTF-8 标志位（general purpose
//! bit 11）未置位时按 CP437 解码文件名，CJK 归档器（Shift-JIS/GBK/EUC-KR/
//! Big5）产出的名字因此乱码。本模块按启发式重解码原始字节。

use encoding_rs::{BIG5, EUC_KR, GBK, SHIFT_JIS, WINDOWS_1252};

/// 解码 zip 条目原始字节名：UTF-8 标志位置位或本身是合法 UTF-8 → 直用；
/// 否则按启发式识别 CJK 编码（Shift-JIS/GBK/EUC-KR/Big5），全部失败回退
/// WINDOWS_1252 lossy（encoding_rs 无 CP437，1252 同为拉丁兜底）。
///
/// 启发式（短文件名上纯“首个可完整解码”规则不可靠：GB2312 汉字总能按
/// SJIS 解成半角假名，EUC-KR/Big5 又总能按 GBK 解成乱码汉字）：
/// 1. SJIS 严格解码成功且结果不含半角假名（U+FF61–FF9F，SJIS 误判的
///    标志）→ SJIS；
/// 2. chardetng 统计识别命中 GBK/EUC-KR/Big5 且可严格解码 → 采用；
/// 3. 按 GBK → EUC-KR → Big5 顺序取首个严格解码成功的；
/// 4. 全部失败 → WINDOWS_1252 lossy。
pub fn decode_zip_entry_name(raw: &[u8], utf8_flag: bool) -> String {
    if utf8_flag {
        return String::from_utf8_lossy(raw).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_string();
    }
    if let Some(s) = strict(SHIFT_JIS, raw) {
        if !s.chars().any(is_halfwidth_katakana) {
            return s;
        }
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(raw, true);
    let guessed = detector.guess(None, true);
    if guessed == GBK || guessed == EUC_KR || guessed == BIG5 {
        if let Some(s) = strict(guessed, raw) {
            return s;
        }
    }
    for encoding in [GBK, EUC_KR, BIG5] {
        if let Some(s) = strict(encoding, raw) {
            return s;
        }
    }
    WINDOWS_1252.decode_without_bom_handling(raw).0.into_owned()
}

fn strict(encoding: &'static encoding_rs::Encoding, raw: &[u8]) -> Option<String> {
    encoding
        .decode_without_bom_handling_and_without_replacement(raw)
        .map(|s| s.into_owned())
}

/// 半角假名（SJIS 单字节区 0xA1–0xDF 的映射结果）；GBK/EUC-KR/Big5 字节
/// 被误当 SJIS 解码时典型地产出半角假名，而真实日文文件名几乎只用全角。
fn is_halfwidth_katakana(c: char) -> bool {
    ('\u{ff61}'..='\u{ff9f}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::Encoding;

    fn encode(enc: &'static Encoding, s: &str) -> Vec<u8> {
        let (bytes, _, had_errors) = enc.encode(s);
        assert!(!had_errors, "sample must encode cleanly: {s}");
        bytes.into_owned()
    }

    #[test]
    fn ascii_and_utf8_passthrough() {
        assert_eq!(decode_zip_entry_name(b"plain.txt", false), "plain.txt");
        let utf8 = "日本語.png".as_bytes();
        assert_eq!(decode_zip_entry_name(utf8, false), "日本語.png");
        assert_eq!(decode_zip_entry_name(utf8, true), "日本語.png");
    }

    #[test]
    fn utf8_flag_tolerates_invalid_bytes() {
        // 标志位置位但字节非法 UTF-8（损坏包）：lossy 直用，不走 CJK 猜测。
        let raw = b"bad\xffname.txt";
        assert_eq!(decode_zip_entry_name(raw, true), "bad\u{fffd}name.txt");
    }

    #[test]
    fn shift_jis_names() {
        for s in ["日本語ファイル.png", "第01話.png", "漫画-テスト.zip"] {
            let raw = encode(SHIFT_JIS, s);
            assert_eq!(decode_zip_entry_name(&raw, false), s);
        }
    }

    #[test]
    fn gbk_names() {
        for s in [
            "中文漫画.png",
            "图片合集01.zip",
            "第一章.png",
            "海贼王102.png",
        ] {
            let raw = encode(GBK, s);
            assert_eq!(decode_zip_entry_name(&raw, false), s);
        }
    }

    #[test]
    fn euc_kr_names() {
        for s in ["한국어.png", "만화책01.zip"] {
            let raw = encode(EUC_KR, s);
            assert_eq!(decode_zip_entry_name(&raw, false), s);
        }
    }

    #[test]
    fn big5_names() {
        for s in ["漫畫下載.png", "最新章節01.zip"] {
            let raw = encode(BIG5, s);
            assert_eq!(decode_zip_entry_name(&raw, false), s);
        }
    }

    #[test]
    fn undecodable_falls_back_to_windows1252() {
        // 0xFF 在四种 CJK 编码均不可解码 → WINDOWS_1252 lossy 兜底。
        // （注意 0x80 不够：encoding_rs 的 SJIS 把它映射为 U+0080。）
        let raw = b"\xff.txt";
        let expected = WINDOWS_1252.decode_without_bom_handling(raw).0.into_owned();
        assert_eq!(decode_zip_entry_name(raw, false), expected);
    }
}
