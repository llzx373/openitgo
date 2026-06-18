use crate::loader::{CompressedFormat, LoadedImage};
use egui::{Color32, ColorImage, TextureHandle, TextureId, TextureOptions};

#[derive(Clone)]
pub enum TextureSlot {
    Managed(TextureHandle),
    Native(TextureId, [u32; 2]), // display size
}

impl TextureSlot {
    pub fn size(&self) -> [u32; 2] {
        match self {
            TextureSlot::Managed(h) => h.size(),
            TextureSlot::Native(_, s) => *s,
        }
    }

    pub fn id(&self) -> TextureId {
        match self {
            TextureSlot::Managed(h) => h.id(),
            TextureSlot::Native(id, _) => *id,
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
            original_size,
            gpu_size,
            format,
        } if supports_dxt5 => match format {
            CompressedFormat::Dxt5Srgb => {
                upload_compressed_native(frame, label, data, gpu_size, original_size)
            }
        },
        LoadedImage::Compressed { original_size, .. } => {
            let color = ColorImage::new(
                [original_size[0] as usize, original_size[1] as usize],
                Color32::MAGENTA,
            );
            TextureSlot::Managed(ctx.load_texture(label, color, TextureOptions::LINEAR))
        }
        LoadedImage::Color(image) => TextureSlot::Managed(
            ctx.load_texture(label, image, TextureOptions::LINEAR),
        ),
    }
}

fn upload_compressed_native(
    frame: &mut eframe::Frame,
    _label: &str,
    data: Vec<u8>,
    gpu_size: [u32; 2],
    display_size: [u32; 2],
) -> TextureSlot {
    let gl = frame.gl().expect("glow context required");
    let texture = unsafe { gl.create_texture() }.expect("failed to create texture");
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.compressed_tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT,
            gpu_size[0] as i32,
            gpu_size[1] as i32,
            0,
            data.len() as i32,
            &data,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    let id = frame.register_native_texture(texture);
    TextureSlot::Native(id, display_size)
}
