use crate::loader::{CompressedFormat, LoadedImage};
use egui::{TextureHandle, TextureId, TextureOptions};
use glow::HasContext;

#[derive(Clone)]
pub enum TextureSlot {
    Managed(TextureHandle),
    Native(TextureId, [u32; 2]), // display size
}

impl TextureSlot {
    pub fn size(&self) -> [usize; 2] {
        match self {
            TextureSlot::Managed(h) => h.size(),
            TextureSlot::Native(_, s) => [s[0] as usize, s[1] as usize],
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
    display_size: [u32; 2],
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
            return Err(format!("compressed_tex_image_2d failed: {err}"));
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    let id = frame.register_native_glow_texture(texture);
    Ok(TextureSlot::Native(id, display_size))
}
