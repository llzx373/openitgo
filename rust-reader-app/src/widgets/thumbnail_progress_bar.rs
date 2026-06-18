use crate::cache::PageCache;
use rust_reader_core::models::Comic;

const THUMB_SIZE: egui::Vec2 = egui::Vec2::new(48.0, 64.0);
const THUMB_MARGIN: f32 = 4.0;

pub fn thumbnail_progress_bar(
    ui: &mut egui::Ui,
    cache: &mut PageCache,
    comic: &Comic,
    current_page: usize,
    on_select: &mut dyn FnMut(usize),
) -> egui::Response {
    let total_pages = comic.total_pages();

    let frame_response = egui::Frame::none()
        .fill(ui.visuals().window_fill().gamma_multiply(0.85))
        .inner_margin(egui::Margin::same(THUMB_MARGIN))
        .show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut response: Option<egui::Response> = None;
                        for idx in 0..total_pages {
                            let selected = idx == current_page;
                            let thumb_response = ui
                                .allocate_ui_with_layout(
                                    THUMB_SIZE,
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| render_thumbnail(ui, cache, idx, selected),
                                )
                                .response;

                            if thumb_response.clicked() {
                                on_select(idx);
                            }

                            if response.is_none() {
                                response = Some(thumb_response);
                            } else {
                                response = response.map(|r| r.union(thumb_response));
                            }
                        }
                        response.unwrap_or_else(|| {
                            ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())
                        })
                    })
                    .inner
                })
                .inner
        });

    frame_response.response
}

fn render_thumbnail(ui: &mut egui::Ui, cache: &mut PageCache, idx: usize, selected: bool) {
    let rect = ui.max_rect();

    if let Some(texture) = cache.get(idx) {
        ui.put(
            rect,
            egui::Image::new(&texture).fit_to_exact_size(rect.size()),
        );
    } else {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        ui.label(
            egui::RichText::new((idx + 1).to_string())
                .size(14.0)
                .color(ui.visuals().text_color()),
        );
    }

    if selected {
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
        );
    }
}
