use crate::loader::{dxt5_padded_size, CompressedFormat, LoadedImage};
use egui::{load::SizedTexture, TextureHandle, TextureId, TextureOptions};
use glow::HasContext;

/// A handle to a GPU texture. Managed variants are freed by egui when the
/// last clone is dropped; native variants are owned by `PageCache` and must
/// not be kept alive after the cache entry is evicted.
#[derive(Clone)]
pub enum TextureSlot {
    Managed(TextureHandle),
    /// Native GL texture. The associated GL texture is owned by `PageCache` and
    /// deleted on eviction; do not let clones outlive the cache entry.
    Native(TextureId, #[allow(missing_docs)] [u32; 2]),
}

impl TextureSlot {
    /// Returns the original display size (width, height) in pixels.
    pub fn size(&self) -> [usize; 2] {
        match self {
            TextureSlot::Managed(h) => h.size(),
            TextureSlot::Native(_, original_size) => {
                [original_size[0] as usize, original_size[1] as usize]
            }
        }
    }

    /// Returns the GPU size (padded to 4×4 blocks) for native DXT5 textures.
    pub fn gpu_size(&self) -> Option<[u32; 2]> {
        match self {
            TextureSlot::Managed(_) => None,
            TextureSlot::Native(_, original_size) => {
                let (w, h) = dxt5_padded_size(original_size[0], original_size[1]);
                Some([w, h])
            }
        }
    }

    /// Returns the UV rect that hides DXT5 4×4 padding. `None` for managed textures.
    pub fn uv_rect(&self) -> Option<egui::Rect> {
        let gpu_size = self.gpu_size()?;
        let original_size = self.size();
        let uv_max = egui::pos2(
            original_size[0] as f32 / gpu_size[0] as f32,
            original_size[1] as f32 / gpu_size[1] as f32,
        );
        Some(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max))
    }

    /// Build an `egui::ImageSource` suitable for `egui::Image::new`.
    /// For native DXT5 textures the caller should apply [`Self::uv_rect()`]
    /// to hide the 4×4 padding.
    pub fn image_source(&self) -> egui::ImageSource<'static> {
        match self {
            TextureSlot::Managed(handle) => egui::ImageSource::Texture(handle.into()),
            TextureSlot::Native(id, original_size) => {
                egui::ImageSource::Texture(SizedTexture::new(
                    *id,
                    egui::vec2(original_size[0] as f32, original_size[1] as f32),
                ))
            }
        }
    }
}

pub fn upload_image(
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
    label: &str,
    image: LoadedImage,
    supports_dxt5: bool,
) -> TextureSlot {
    match image {
        LoadedImage::Compressed {
            data,
            rgba,
            original_size,
            gpu_size,
            format,
            ..
        } if supports_dxt5 => match format {
            CompressedFormat::Dxt5Srgb => {
                match upload_compressed_native(frame, label, data, gpu_size, original_size) {
                    Ok(slot) => slot,
                    Err(err) => {
                        eprintln!("native DXT5 upload failed, falling back: {err}");
                        TextureSlot::Managed(ctx.load_texture(label, rgba, TextureOptions::LINEAR))
                    }
                }
            }
        },
        LoadedImage::Compressed { rgba, .. } => {
            TextureSlot::Managed(ctx.load_texture(label, rgba, TextureOptions::LINEAR))
        }
        LoadedImage::Color(image) => {
            TextureSlot::Managed(ctx.load_texture(label, image, TextureOptions::LINEAR))
        }
    }
}

fn upload_compressed_native(
    frame: &mut eframe::Frame,
    _label: &str,
    data: Vec<u8>,
    gpu_size: [u32; 2],
    original_size: [u32; 2],
) -> Result<TextureSlot, String> {
    let gl = frame.gl().ok_or("glow context required")?;
    let texture =
        unsafe { gl.create_texture() }.map_err(|e| format!("failed to create texture: {e}"))?;
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.compressed_tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT as i32,
            gpu_size[0] as i32,
            gpu_size[1] as i32,
            0,
            data.len() as i32,
            &data,
        );
        let err = gl.get_error();
        if err != glow::NO_ERROR {
            gl.delete_texture(texture);
            gl.bind_texture(glow::TEXTURE_2D, None);
            return Err(format!("compressed_tex_image_2d failed: {err}"));
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    let id = frame.register_native_glow_texture(texture);
    Ok(TextureSlot::Native(id, original_size))
}
