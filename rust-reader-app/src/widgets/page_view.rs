use egui::{ColorImage, TextureHandle, TextureOptions};

pub fn load_texture_from_bytes(ctx: &egui::Context, bytes: &[u8]) -> Option<TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?;
    let size = [image.width() as _, image.height() as _];
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.as_flat_samples();
    let color_image = ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
    Some(ctx.load_texture("page", color_image, TextureOptions::default()))
}

pub fn load_texture_from_path(
    ctx: &egui::Context,
    path: &std::path::Path,
) -> Option<TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    load_texture_from_bytes(ctx, &bytes)
}
