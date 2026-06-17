use egui::{ColorImage, TextureHandle, TextureOptions};

pub fn load_texture_from_bytes(
    ctx: &egui::Context,
    bytes: &[u8],
    label: &str,
) -> Result<TextureHandle, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let size = [image.width() as _, image.height() as _];
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.as_flat_samples();
    let color_image = ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
    Ok(ctx.load_texture(label.to_string(), color_image, TextureOptions::default()))
}

pub fn load_texture_from_path(
    ctx: &egui::Context,
    path: &std::path::Path,
    label: &str,
) -> Result<TextureHandle, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    load_texture_from_bytes(ctx, &bytes, label)
}
