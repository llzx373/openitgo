use egui::TextureOptions;

pub fn upload_color_image(
    ctx: &egui::Context,
    image: egui::ColorImage,
    label: String,
) -> egui::TextureHandle {
    ctx.load_texture(label, image, TextureOptions::LINEAR)
}
