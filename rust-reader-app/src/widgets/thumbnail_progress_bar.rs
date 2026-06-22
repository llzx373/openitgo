use crate::cache::PageCache;
use egui::Context;

const THUMB_SIZE: egui::Vec2 = egui::Vec2::new(80.0, 120.0);

pub fn page_thumbnail_tooltip(
    ui: &mut egui::Ui,
    ctx: &Context,
    cache: &mut PageCache,
    page_index: usize,
    tooltip_pos: egui::Pos2,
) -> egui::Response {
    let center = egui::pos2(tooltip_pos.x, tooltip_pos.y - THUMB_SIZE.y / 2.0);
    let rect = egui::Rect::from_center_size(center, THUMB_SIZE);

    let response = ui.allocate_rect(rect, egui::Sense::hover());

    ui.painter()
        .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

    if let Some(handle) = cache.get_texture(ctx, page_index) {
        ui.put(
            rect,
            egui::Image::new(&handle).fit_to_exact_size(rect.size()),
        );
    } else {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
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
    );

    response
}
