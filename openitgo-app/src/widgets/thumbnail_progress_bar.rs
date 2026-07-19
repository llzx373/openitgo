use crate::cache::PageCache;
use egui::Context;

/// Maximum long-edge dimension of the progress-bar hover thumbnail.
const THUMB_MAX_SIZE: f32 = 160.0;

pub fn page_thumbnail_tooltip(
    ui: &mut egui::Ui,
    ctx: &Context,
    cache: &mut PageCache,
    page_index: usize,
    tooltip_pos: egui::Pos2,
) -> egui::Response {
    // Preserve the page aspect ratio instead of using a fixed 80x120 rect.
    let size = thumbnail_size(cache, page_index);
    let center = egui::pos2(tooltip_pos.x, tooltip_pos.y - size.y / 2.0);
    let rect = egui::Rect::from_center_size(center, size);

    let response = ui.allocate_rect(rect, egui::Sense::hover());

    ui.painter()
        .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

    if let Some(handle) = cache.get_texture(ctx, page_index) {
        // Maintain aspect ratio and letter-box inside the tooltip rect.
        ui.put(rect, egui::Image::new(&handle).max_size(rect.size()));
    } else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(
                        egui::RichText::new((page_index + 1).to_string())
                            .size(14.0)
                            .color(ui.visuals().text_color()),
                    );
                },
            );
        });
    }

    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, ui.visuals().window_stroke.color),
        egui::StrokeKind::Inside,
    );

    response
}

fn thumbnail_size(cache: &PageCache, page_index: usize) -> egui::Vec2 {
    let [w, h] = cache.get_original_size(page_index).unwrap_or([80, 120]);
    if w == 0 || h == 0 {
        return egui::vec2(80.0, 120.0);
    }
    let (w, h) = (w as f32, h as f32);
    let ratio = w / h;
    if ratio >= 1.0 {
        let width = THUMB_MAX_SIZE;
        let height = (width / ratio).max(1.0);
        egui::vec2(width, height)
    } else {
        let height = THUMB_MAX_SIZE;
        let width = (height * ratio).max(1.0);
        egui::vec2(width, height)
    }
}
