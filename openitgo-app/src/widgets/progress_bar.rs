pub struct ProgressBarResponse {
    pub response: egui::Response,
    pub hovered_page: Option<usize>,
}

const BAR_HEIGHT: f32 = 10.0;
const BAR_RADIUS: u8 = 4;

pub fn comic_progress_bar(
    ui: &mut egui::Ui,
    current_page: usize,
    total_pages: usize,
    opacity: f32,
) -> ProgressBarResponse {
    let available_width = ui.available_width();
    let desired_size = egui::vec2(available_width, BAR_HEIGHT);
    let (rect, response) = ui.allocate_at_least(desired_size, egui::Sense::click());

    if total_pages == 0 {
        draw_empty_bar(ui, rect, opacity);
        return ProgressBarResponse {
            response,
            hovered_page: None,
        };
    }

    draw_filled_bar(ui, rect, current_page, total_pages, opacity);

    let hovered_page = if response.hovered() {
        ui.input(|i| i.pointer.hover_pos())
            .map(|pos| page_at_x(pos.x, rect, total_pages))
    } else {
        None
    };

    ProgressBarResponse {
        response,
        hovered_page,
    }
}

fn alpha(opacity: f32, scale: f32) -> u8 {
    ((opacity.clamp(0.0, 1.0) * scale).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn draw_empty_bar(ui: &mut egui::Ui, rect: egui::Rect, opacity: f32) {
    let base = ui.visuals().extreme_bg_color;
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(BAR_RADIUS),
        egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha(opacity, 0.55)),
    );
}

fn draw_filled_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    current_page: usize,
    total_pages: usize,
    opacity: f32,
) {
    let rounding = egui::CornerRadius::same(BAR_RADIUS);
    let bg = ui.visuals().extreme_bg_color;
    ui.painter().rect_filled(
        rect,
        rounding,
        egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), alpha(opacity, 0.55)),
    );

    let progress = (current_page + 1).min(total_pages) as f32 / total_pages as f32;
    let fill_width = rect.width() * progress;
    let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, rect.height()));
    let accent = ui.visuals().selection.stroke.color;
    ui.painter().rect_filled(
        fill_rect,
        rounding,
        egui::Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            alpha(opacity, 0.92),
        ),
    );
}

pub(crate) fn page_at_x(x: f32, rect: egui::Rect, total_pages: usize) -> usize {
    if rect.width() <= 0.0 {
        return 0;
    }
    let ratio = ((x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
    let page = (ratio * total_pages as f32).floor() as usize;
    page.min(total_pages - 1)
}
