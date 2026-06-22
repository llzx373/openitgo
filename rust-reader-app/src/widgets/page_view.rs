use crate::loader::LoadedImage;
use egui::TextureHandle;

#[allow(dead_code)]
pub fn upload_image(
    ctx: &egui::Context,
    label: &str,
    image: LoadedImage,
) -> Result<TextureHandle, String> {
    let color = image.to_color_image()?;
    Ok(ctx.load_texture(label, color, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ColorImage;

    #[test]
    fn test_upload_image_color() {
        let ctx = egui::Context::default();
        let image = LoadedImage::Color(ColorImage::new([4, 4], egui::Color32::WHITE));
        let handle = upload_image(&ctx, "test", image).expect("upload should succeed");
        assert_eq!(handle.size(), [4, 4]);
    }
}
