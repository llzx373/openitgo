//! 预览内容分类的共享层（Archive 视图与文件管理器共用）：字节流分类
//! （图片解码 / 文本嗅探 / 截断）、可预览名门槛、本地文件预览加载。
//! 从 `archive.rs` 原样抽取，行为不变。

use std::path::Path;

/// 预览读取上限：超过该大小的条目/文件不读取（防爆内存）。
pub(crate) const PREVIEW_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 文本嗅探只取内容的前 256KB。
const TEXT_SNIFF_BYTES: usize = 256 * 1024;
/// 文本预览展示的字符数上限。
const TEXT_PREVIEW_MAX_CHARS: usize = 64_000;

/// 后台线程产出的预览内容。
#[derive(Debug, Clone)]
pub(crate) enum PreviewData {
    Image(egui::ColorImage),
    Text(String),
    Unsupported,
    /// 附带说明（过大/解码失败等）。
    Note(String),
}

/// 预览加载结果（阶段 R）：分类内容 + 原始字节（HEX 查看与编码手动切换
/// 重解码用；超过 PREVIEW_MAX_BYTES 未读取时 bytes = None）。
#[derive(Debug, Clone)]
pub(crate) struct PreviewOutcome {
    pub data: PreviewData,
    pub bytes: Option<std::sync::Arc<[u8]>>,
}

/// 本地文件预览加载（文件管理器）：>64MB 不读，`fs::read` 后按
/// 扩展名 + 内容嗅探分类（与压缩包条目预览同一管线）。
pub(crate) fn load_file_preview(path: &Path) -> Result<PreviewOutcome, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("无法读取文件信息: {e}"))?
        .len();
    if size > PREVIEW_MAX_BYTES {
        return Ok(PreviewOutcome {
            data: PreviewData::Note("文件过大，不预览".to_string()),
            bytes: None,
        });
    }
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes: std::sync::Arc<[u8]> = bytes.into();
    Ok(PreviewOutcome {
        data: classify_preview_bytes(&name, &bytes),
        bytes: Some(bytes),
    })
}

/// 预览图解码的宽/高上限：预览纹理最终经 `ctx.load_texture` 进 wgpu，任一边
/// 超过 `max_texture_dimension_2d`（一般 8192）会在 create_texture 校验 panic
/// 闪退；同时限制解码分配，防解压炸弹 OOM（长条图/超大图转 Note 说明）。
const PREVIEW_IMAGE_MAX_DIM: u32 = 8192;

/// 按扩展名与内容嗅探分类已读出的字节（纯函数，便于单测）。
pub(crate) fn classify_preview_bytes(name: &str, bytes: &[u8]) -> PreviewData {
    let is_image = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(openitgo_parser::traits::is_image_extension);
    if is_image {
        let mut reader =
            match image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() {
                Ok(r) => r,
                Err(e) => return PreviewData::Note(format!("无法识别图片格式: {e}")),
            };
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(PREVIEW_IMAGE_MAX_DIM);
        limits.max_image_height = Some(PREVIEW_IMAGE_MAX_DIM);
        reader.limits(limits);
        return match reader.decode() {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                PreviewData::Image(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
            }
            Err(e) => {
                let msg = if matches!(e, image::ImageError::Limits(_)) {
                    format!("图片尺寸超过 {PREVIEW_IMAGE_MAX_DIM}px，不预览")
                } else {
                    format!("无法解码图片: {e}")
                };
                PreviewData::Note(msg)
            }
        };
    }
    match decode_preview_text(bytes, None) {
        Some(t) => PreviewData::Text(t),
        None => PreviewData::Unsupported,
    }
}

/// 预览文本解码（阶段 R：自动检测 / 指定编码手动切换共用入口）：嗅探前
/// 256KB，含 NUL 视为二进制，展示截断 64k 字符。label ∈ None（自动）|
/// "utf-8"|"gbk"|"shift-jis"|"big5"（见 parser decode_text_with）。
pub(crate) fn decode_preview_text(bytes: &[u8], label: Option<&str>) -> Option<String> {
    let prefix = &bytes[..bytes.len().min(TEXT_SNIFF_BYTES)];
    let decoded = match label {
        None => openitgo_parser::archive::decode_text_guess(prefix),
        Some(l) => openitgo_parser::archive::decode_text_with(prefix, l),
    }?;
    if decoded.contains('\0') {
        return None;
    }
    Some(truncate_preview_text(decoded))
}

/// 文本预览展示的字符数截断（追加说明尾巴）。
fn truncate_preview_text(s: String) -> String {
    let mut truncated: String = s.chars().take(TEXT_PREVIEW_MAX_CHARS).collect();
    if s.chars().count() > TEXT_PREVIEW_MAX_CHARS {
        truncated.push_str("\n…（内容过长，已截断）");
    }
    truncated
}

/// HEX 预览单行格式化（纯函数，show_rows 虚拟化按需调用——不物化全表，
/// 64MB 上限文件 = 4M 行）：`偏移(8 hex)  16 字节 hex 对（8+8 分组） |ASCII|`，
/// 不可打印字节显示 `.`；line 越界返回 None。
pub(crate) fn format_hex_line(bytes: &[u8], line: usize) -> Option<String> {
    let start = line.checked_mul(16)?;
    if start >= bytes.len() {
        return None;
    }
    let chunk = &bytes[start..(start + 16).min(bytes.len())];
    let mut out = format!("{start:08x}  ");
    for i in 0..16 {
        if i == 8 {
            out.push(' ');
        }
        // 末行不足 16 字节：占位对齐 ASCII 列。
        match chunk.get(i) {
            Some(b) => out.push_str(&format!("{b:02x} ")),
            None => out.push_str("   "),
        }
    }
    out.push_str(" |");
    for b in chunk {
        let c = if b.is_ascii_graphic() || *b == b' ' {
            *b as char
        } else {
            '.'
        };
        out.push(c);
    }
    out.push('|');
    Some(out)
}

/// 常见文本扩展名集合（预览门槛与文件搜索的内容检索共用；小写比较）。
pub(crate) fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "txt"
            | "md"
            | "markdown"
            | "log"
            | "json"
            | "xml"
            | "yaml"
            | "yml"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "csv"
            | "tsv"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "ts"
            | "rs"
            | "py"
            | "java"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "go"
            | "sh"
            | "bat"
            | "ps1"
            | "sql"
            | "srt"
            | "ass"
            | "vtt"
            | "nfo"
    )
}

/// 按名字判断「大概率可预览」（图片扩展名或常见文本扩展名）——选中时
/// 自动打开预览面板的门槛；最终能否预览仍由内容嗅探决定。
pub(crate) fn is_previewable_name(name: &str) -> bool {
    let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if openitgo_parser::traits::is_image_extension(ext) {
        return true;
    }
    is_text_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_previewable_name_covers_image_and_text() {
        for name in [
            "a.png", "b.JPG", "c.webp", "d.txt", "e.md", "f.json", "g.log",
        ] {
            assert!(is_previewable_name(name), "{name}");
        }
        for name in ["a.exe", "b.dll", "c.zip", "d.bin", "noext", ".gitignore"] {
            assert!(!is_previewable_name(name), "{name}");
        }
    }

    #[test]
    fn classify_preview_bytes_decodes_image() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        match classify_preview_bytes("p.png", &buf.into_inner()) {
            PreviewData::Image(ci) => assert_eq!(ci.size, [2, 2]),
            other => panic!("expected Image, got {other:?}"),
        }
        // 扩展名是图片但内容不可解码 → 说明。
        assert!(matches!(
            classify_preview_bytes("p.png", b"not a png"),
            PreviewData::Note(_)
        ));
    }

    /// 回归（预览闪退）：超过 GPU 纹理上限（8192）的图片必须走 Note，
    /// 不能进 ColorImage——否则 load_texture 后 wgpu create_texture 校验 panic。
    #[test]
    fn classify_preview_bytes_rejects_oversized_image() {
        let wide =
            image::RgbaImage::from_pixel(PREVIEW_IMAGE_MAX_DIM + 1, 1, image::Rgba([0, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(wide)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        match classify_preview_bytes("big.png", &buf.into_inner()) {
            PreviewData::Note(n) => assert!(n.contains("不预览"), "{n}"),
            other => panic!("expected Note, got {other:?}"),
        }
        // 边界值本身仍可正常解码。
        let ok =
            image::RgbaImage::from_pixel(PREVIEW_IMAGE_MAX_DIM, 1, image::Rgba([0, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(ok)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        match classify_preview_bytes("ok.png", &buf.into_inner()) {
            PreviewData::Image(ci) => assert_eq!(ci.size, [PREVIEW_IMAGE_MAX_DIM as usize, 1]),
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn classify_preview_bytes_text_and_unsupported() {
        match classify_preview_bytes("notes.txt", "你好 world".as_bytes()) {
            PreviewData::Text(t) => assert_eq!(t, "你好 world"),
            other => panic!("expected Text, got {other:?}"),
        }
        // 含 NUL → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", b"ab\0cd"),
            PreviewData::Unsupported
        ));
        // 起始即非法 UTF-8 且无法识别 → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", &[0xFF, 0xFE, 0x00]),
            PreviewData::Unsupported
        ));
        // GBK 编码的中文文本经 chardetng 识别后可预览
        // （"你好，世界！这是一段用于编码识别的中文测试文本。" 的 GBK 字节；
        // 统计识别需要较长样本）。
        let gbk: &[u8] = &[
            0xC4, 0xE3, 0xBA, 0xC3, 0xA3, 0xAC, 0xCA, 0xC0, 0xBD, 0xE7, 0xA3, 0xA1, 0xD5, 0xE2,
            0xCA, 0xC7, 0xD2, 0xBB, 0xB6, 0xCE, 0xD3, 0xC3, 0xD3, 0xDA, 0xB1, 0xE0, 0xC2, 0xEB,
            0xCA, 0xB6, 0xB1, 0xF0, 0xB5, 0xC4, 0xD6, 0xD0, 0xCE, 0xC4, 0xB2, 0xE2, 0xCA, 0xD4,
            0xCE, 0xC4, 0xB1, 0xBE, 0xA1, 0xA3,
        ];
        match classify_preview_bytes("a.txt", gbk) {
            PreviewData::Text(t) => {
                assert_eq!(t, "你好，世界！这是一段用于编码识别的中文测试文本。")
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_preview_bytes_truncates_long_text() {
        let long = "x".repeat(TEXT_PREVIEW_MAX_CHARS + 10_000);
        match classify_preview_bytes("a.txt", long.as_bytes()) {
            PreviewData::Text(t) => {
                assert!(t.ends_with("（内容过长，已截断）"));
                assert!(t.chars().count() < long.chars().count());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn load_file_preview_reads_local_file() {
        let dir =
            std::env::temp_dir().join(format!("openitgo-preview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.txt");
        std::fs::write(&file, "你好 preview").unwrap();
        match load_file_preview(&file) {
            Ok(out) => {
                match out.data {
                    PreviewData::Text(t) => assert_eq!(t, "你好 preview"),
                    other => panic!("expected Text, got {other:?}"),
                }
                // 原始字节随结果返回（HEX 查看/编码重解码用）。
                assert_eq!(out.bytes.as_deref(), Some("你好 preview".as_bytes()));
            }
            Err(e) => panic!("expected Ok, got {e}"),
        }
        // 不存在的文件 → Err。
        assert!(load_file_preview(&dir.join("missing.txt")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decode_preview_text_manual_encoding() {
        // GBK 字节（"你好"）：自动检测（样本太短可能识别失败）vs 指定 gbk。
        let gbk: &[u8] = &[0xC4, 0xE3, 0xBA, 0xC3];
        assert_eq!(
            decode_preview_text(gbk, Some("gbk")).as_deref(),
            Some("你好")
        );
        // 指定 utf-8 严格解码失败 → None。
        assert!(decode_preview_text(gbk, Some("utf-8")).is_none());
        // GBK 字节按 big5 有损解码总能产出字符串（含 replacement）。
        assert!(decode_preview_text(gbk, Some("big5")).is_some());
        // 未知 label → None。
        assert!(decode_preview_text(gbk, Some("latin9")).is_none());
        // 含 NUL 恒视为二进制。
        assert!(decode_preview_text(b"ab\0cd", Some("gbk")).is_none());
        assert!(decode_preview_text(b"ab\0cd", None).is_none());
    }

    #[test]
    fn format_hex_line_layout() {
        // 空输入 / 越界 → None。
        assert!(format_hex_line(b"", 0).is_none());
        assert!(format_hex_line(b"abc", 1).is_none());
        // 首行：偏移 + 16 字节 + ASCII（不可打印显示 `.`）。
        let data: Vec<u8> = (0u8..=31).collect();
        let line = format_hex_line(&data, 0).unwrap();
        assert_eq!(
            line,
            "00000000  00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f  |................|"
        );
        let line1 = format_hex_line(&data, 1).unwrap();
        assert!(line1.starts_with("00000010  10 11"), "{line1}");
        // 可打印 ASCII 直通。
        let hello = format_hex_line(b"Hello, World!", 0).unwrap();
        assert!(hello.ends_with("|Hello, World!|"), "{hello}");
        // 末行不足 16 字节：hex 区占位对齐，ASCII 列（首 `|`）位置一致。
        assert_eq!(hello.find('|'), line.find('|'), "短行与满行的 ASCII 列对齐");
    }
}
