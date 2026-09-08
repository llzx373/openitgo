//! libwebp(SIMD)缩略图解码:缩放解码 + 全解码兜底。
//!
//! 纯 Rust 的 image crate 解码 webp 较慢(实测 1762x2500 约 240ms),而缩略图
//! 只需要 256px。libwebp 的高级解码 API 支持边解码边缩放,配合 SIMD 可把单张
//! 缩略图成本压低一个量级。所有 unsafe FFI 集中在本模块;任何失败都返回 None,
//! 由调用方回退到 image crate 路径。

use egui::ColorImage;
use libwebp_sys::*;

/// 用 libwebp 解码 webp 并缩放到长边 ≤ max_dim。
///
/// 返回 `(缩略图, 原始尺寸)`。非 webp、解码失败、或尺寸非法时返回 None。
pub(crate) fn decode_webp_thumbnail(bytes: &[u8], max_dim: u32) -> Option<(ColorImage, [u32; 2])> {
    let (orig_w, orig_h) = webp_dimensions(bytes)?;
    if orig_w == 0 || orig_h == 0 {
        return None;
    }

    let max = orig_w.max(orig_h);
    let (target_w, target_h) = if max > max_dim {
        let ratio = max_dim as f32 / max as f32;
        (
            ((orig_w as f32 * ratio).round() as u32).max(1),
            ((orig_h as f32 * ratio).round() as u32).max(1),
        )
    } else {
        (orig_w, orig_h)
    };

    let (mut img_w, mut img_h, rgba) = decode_scaled(bytes, target_w, target_h)?;

    // libwebp 对 lossless webp 不支持缩放解码(静默输出原尺寸);此时用
    // image crate 的 thumbnail 兜底缩放,保证输出尺寸契约。
    let rgba = if img_w == target_w && img_h == target_h {
        rgba
    } else {
        let len = (img_w as usize) * (img_h as usize) * 4;
        if rgba.len() != len {
            return None;
        }
        let buf = image::RgbaImage::from_raw(img_w, img_h, rgba)?;
        let dyn_img = image::DynamicImage::ImageRgba8(buf);
        let scaled = dyn_img.thumbnail(max_dim, max_dim);
        img_w = scaled.width();
        img_h = scaled.height();
        scaled.to_rgba8().into_raw()
    };

    let expected = (img_w as usize) * (img_h as usize) * 4;
    if rgba.len() != expected {
        return None;
    }
    Some((
        ColorImage::from_rgba_unmultiplied([img_w as usize, img_h as usize], &rgba),
        [orig_w, orig_h],
    ))
}

/// 读取 webp 头部尺寸,不解码像素。
fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    unsafe {
        let mut features = std::mem::MaybeUninit::<WebPBitstreamFeatures>::uninit();
        let status = WebPGetFeatures(bytes.as_ptr(), bytes.len(), features.as_mut_ptr());
        if status != VP8StatusCode::VP8_STATUS_OK {
            return None;
        }
        let features = features.assume_init();
        if features.width <= 0 || features.height <= 0 {
            return None;
        }
        Some((features.width as u32, features.height as u32))
    }
}

/// 经 WebPDecoderConfig 解码为 RGBA,请求缩放到 (target_w, target_h)。
/// 返回实际输出尺寸与像素(libwebp 可能忽略缩放请求)。
fn decode_scaled(bytes: &[u8], target_w: u32, target_h: u32) -> Option<(u32, u32, Vec<u8>)> {
    decode_with_config(bytes, target_w, target_h, true)
        // libwebp 拒绝缩放 lossless webp(整个 WebPDecode 失败而非忽略缩放
        // 请求),此时回退无缩放解码,由调用方用 image crate 兜底缩放。
        .or_else(|| decode_with_config(bytes, target_w, target_h, false))
}

fn decode_with_config(
    bytes: &[u8],
    target_w: u32,
    target_h: u32,
    use_scaling: bool,
) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut config = WebPDecoderConfig::new().ok()?;
        config.output.colorspace = WEBP_CSP_MODE::MODE_RGBA;
        config.options.use_scaling = if use_scaling { 1 } else { 0 };
        config.options.scaled_width = target_w as i32;
        config.options.scaled_height = target_h as i32;
        config.options.no_fancy_upsampling = 1;

        let status = WebPDecode(bytes.as_ptr(), bytes.len(), &mut config);
        if status != VP8StatusCode::VP8_STATUS_OK {
            return None;
        }

        let buf = config.output.u.RGBA;
        // Output dimensions (after scaling) are reported via config.input.
        let w = config.input.width;
        let h = config.input.height;
        let stride = buf.stride;
        let rgba = buf.rgba;
        let ok = w > 0 && h > 0 && !rgba.is_null() && stride >= w * 4;
        let result = if ok {
            // 按 stride 逐行拷贝,去掉行尾 padding。
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as isize {
                let row = rgba.offset(y * stride as isize);
                out.extend_from_slice(std::slice::from_raw_parts(row, (w * 4) as usize));
            }
            Some((w as u32, h as u32, out))
        } else {
            None
        };
        WebPFreeDecBuffer(&mut config.output);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_lossless_webp(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 200, 30, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::WebP)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn small_webp_keeps_size() {
        let bytes = encode_lossless_webp(64, 48);
        let (thumb, original) = decode_webp_thumbnail(&bytes, 256).unwrap();
        assert_eq!(original, [64, 48]);
        assert_eq!(thumb.size, [64, 48]);
    }

    #[test]
    fn large_webp_scales_to_max_dim() {
        let bytes = encode_lossless_webp(1024, 768);
        let (thumb, original) = decode_webp_thumbnail(&bytes, 256).unwrap();
        assert_eq!(original, [1024, 768]);
        let [w, h] = thumb.size;
        assert!(w.max(h) <= 256, "long edge must be <= 256, got {w}x{h}");
        // 纵横比保持(4:3)。
        let ratio = w as f32 / h as f32;
        assert!((ratio - 4.0 / 3.0).abs() < 0.05, "aspect drift: {w}x{h}");
    }

    #[test]
    fn garbage_returns_none() {
        assert!(decode_webp_thumbnail(b"not a webp", 256).is_none());
        assert!(decode_webp_thumbnail(&[], 256).is_none());
        // PNG 字节也应返回 None(由调用方走 image crate)。
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        assert!(decode_webp_thumbnail(&buf.into_inner(), 256).is_none());
    }
}
