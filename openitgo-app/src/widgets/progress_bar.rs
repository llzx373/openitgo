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
) -> ProgressBarResponse {
    let available_width = ui.available_width();
    let desired_size = egui::vec2(available_width, BAR_HEIGHT);
    let (rect, response) = ui.allocate_at_least(desired_size, egui::Sense::click());

    if total_pages == 0 {
        draw_empty_bar(ui, rect);
        return ProgressBarResponse {
            response,
            hovered_page: None,
        };
    }

    draw_filled_bar(ui, rect, current_page, total_pages);

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

fn draw_empty_bar(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(BAR_RADIUS),
        ui.visuals().extreme_bg_color,
    );
}

fn draw_filled_bar(ui: &mut egui::Ui, rect: egui::Rect, current_page: usize, total_pages: usize) {
    let rounding = egui::CornerRadius::same(BAR_RADIUS);
    let bg_color = ui.visuals().extreme_bg_color;
    ui.painter().rect_filled(rect, rounding, bg_color);

    let progress = (current_page + 1).min(total_pages) as f32 / total_pages as f32;
    let fill_width = rect.width() * progress;
    let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, rect.height()));
    // Prefer selection stroke (accent) for a clearer Gallery progress fill.
    let fill_color = ui.visuals().selection.stroke.color;
    ui.painter().rect_filled(fill_rect, rounding, fill_color);
}

pub(crate) fn page_at_x(x: f32, rect: egui::Rect, total_pages: usize) -> usize {
    if rect.width() <= 0.0 {
        return 0;
    }
    let ratio = ((x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
    let page = (ratio * total_pages as f32).floor() as usize;
    page.min(total_pages - 1)
}
