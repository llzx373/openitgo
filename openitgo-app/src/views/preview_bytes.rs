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

/// 本地文件预览加载（文件管理器）：>64MB 不读，`fs::read` 后按
/// 扩展名 + 内容嗅探分类（与压缩包条目预览同一管线）。
pub(crate) fn load_file_preview(path: &Path) -> Result<PreviewData, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("无法读取文件信息: {e}"))?
        .len();
    if size > PREVIEW_MAX_BYTES {
        return Ok(PreviewData::Note("文件过大，不预览".to_string()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(classify_preview_bytes(&name, &bytes))
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
    let prefix = &bytes[..bytes.len().min(TEXT_SNIFF_BYTES)];
    match openitgo_parser::archive::decode_text_guess(prefix) {
        Some(s) if !s.contains('\0') => {
            let mut truncated: String = s.chars().take(TEXT_PREVIEW_MAX_CHARS).collect();
            if s.chars().count() > TEXT_PREVIEW_MAX_CHARS {
                truncated.push_str("\n…（内容过长，已截断）");
            }
            PreviewData::Text(truncated)
        }
        _ => PreviewData::Unsupported,
    }
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
            Ok(PreviewData::Text(t)) => assert_eq!(t, "你好 preview"),
            other => panic!("expected Text, got {other:?}"),
        }
        // 不存在的文件 → Err。
        assert!(load_file_preview(&dir.join("missing.txt")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
