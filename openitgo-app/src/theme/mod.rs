//! Gallery Immersive design tokens and egui Visuals/Style.
//!
//! Dark-first reading shell: cool charcoal surfaces, warm amber accent,
//! soft rounding, minimal borders. Light mode uses a cool gallery gray
//! (not warm cream).

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Style, Theme, Visuals};

/// Warm amber accent — used for selection, progress, and active chrome.
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0xD4, 0xA5, 0x74);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0xB8, 0x83, 0x4A);

/// Cover / card corner radius.
pub const RADIUS_COVER: u8 = 10;
/// Chip / segmented control corner radius.
pub const RADIUS_CHIP: u8 = 12;
/// Standard control corner radius.
pub const RADIUS_CONTROL: u8 = 6;

/// Install Gallery Immersive visuals + spacing for both egui themes.
/// Safe to call repeatedly; System preference then picks dark/light correctly.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, dark_visuals());
    ctx.set_visuals_of(Theme::Light, light_visuals());
    ctx.style_mut_of(Theme::Dark, apply_gallery_style);
    ctx.style_mut_of(Theme::Light, apply_gallery_style);
}

pub fn accent_for(dark: bool) -> Color32 {
    if dark {
        ACCENT_DARK
    } else {
        ACCENT_LIGHT
    }
}

pub fn dark_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(Color32::from_rgb(0xF0, 0xEE, 0xE8));
    v.weak_text_alpha = 0.55;
    v.panel_fill = Color32::from_rgb(0x14, 0x14, 0x16);
    v.window_fill = Color32::from_rgb(0x1C, 0x1C, 0x20);
    v.extreme_bg_color = Color32::from_rgb(0x0A, 0x0A, 0x0C);
    v.faint_bg_color = Color32::from_rgb(0x1A, 0x1A, 0x1E);
    v.code_bg_color = Color32::from_rgb(0x22, 0x22, 0x28);
    v.hyperlink_color = ACCENT_DARK;
    v.warn_fg_color = Color32::from_rgb(0xE8, 0xB8, 0x6A);
    v.error_fg_color = Color32::from_rgb(0xE0, 0x70, 0x70);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2A, 0x30));
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(80),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(60),
    };
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xD4, 0xA5, 0x74, 72);
    v.selection.stroke = Stroke::new(1.0, ACCENT_DARK);
    polish_widgets(&mut v, true);
    v
}

pub fn light_visuals() -> Visuals {
    let mut v = Visuals::light();
    v.dark_mode = false;
    v.override_text_color = Some(Color32::from_rgb(0x1A, 0x1A, 0x1C));
    v.weak_text_alpha = 0.5;
    // Cool gallery gray — avoid warm cream #F4F1EA.
    v.panel_fill = Color32::from_rgb(0xEE, 0xEF, 0xF1);
    v.window_fill = Color32::from_rgb(0xF7, 0xF8, 0xF9);
    v.extreme_bg_color = Color32::from_rgb(0xE2, 0xE4, 0xE8);
    v.faint_bg_color = Color32::from_rgb(0xE8, 0xEA, 0xED);
    v.code_bg_color = Color32::from_rgb(0xE6, 0xE8, 0xEC);
    v.hyperlink_color = ACCENT_LIGHT;
    v.warn_fg_color = Color32::from_rgb(0xB0, 0x70, 0x20);
    v.error_fg_color = Color32::from_rgb(0xC0, 0x40, 0x40);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0xD0, 0xD2, 0xD6));
    v.window_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
    v.popup_shadow = Shadow {
        offset: [0, 3],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(20),
    };
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 56);
    v.selection.stroke = Stroke::new(1.0, ACCENT_LIGHT);
    polish_widgets(&mut v, false);
    v
}

fn polish_widgets(v: &mut Visuals, dark: bool) {
    let radius = CornerRadius::same(RADIUS_CONTROL);
    let accent = accent_for(dark);

    v.widgets.noninteractive.corner_radius = radius;
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.corner_radius = radius;
    v.widgets.open.corner_radius = radius;

    if dark {
        v.widgets.noninteractive.bg_fill = Color32::from_rgb(0x14, 0x14, 0x16);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0x1A, 0x1A, 0x1E);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x28, 0x28, 0x2E));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC8, 0xC6, 0xC0));

        v.widgets.inactive.bg_fill = Color32::from_rgb(0x22, 0x22, 0x28);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x22, 0x22, 0x28);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x32, 0x32, 0x3A));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xE8, 0xE6, 0xE0));

        v.widgets.hovered.bg_fill = Color32::from_rgb(0x2C, 0x2C, 0x34);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2C, 0x2C, 0x34);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x48, 0x44, 0x3A));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xF4, 0xF2, 0xEC));

        v.widgets.active.bg_fill = Color32::from_rgb(0x34, 0x30, 0x28);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(0x34, 0x30, 0x28);
        v.widgets.active.bg_stroke = Stroke::new(1.0, accent);
        v.widgets.active.fg_stroke = Stroke::new(1.0, accent);

        v.widgets.open.bg_fill = Color32::from_rgb(0x2A, 0x28, 0x24);
        v.widgets.open.weak_bg_fill = Color32::from_rgb(0x2A, 0x28, 0x24);
        v.widgets.open.bg_stroke = Stroke::new(1.0, accent);
        v.widgets.open.fg_stroke = Stroke::new(1.0, accent);
    } else {
        v.widgets.noninteractive.bg_fill = Color32::from_rgb(0xEE, 0xEF, 0xF1);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0xE8, 0xEA, 0xED);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xD0, 0xD2, 0xD6));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4A, 0x4A, 0x50));

        v.widgets.inactive.bg_fill = Color32::from_rgb(0xF7, 0xF8, 0xF9);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xF7, 0xF8, 0xF9);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC8, 0xCA, 0xD0));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0x1A, 0x1A, 0x1C));

        v.widgets.hovered.bg_fill = Color32::from_rgb(0xFF, 0xFF, 0xFF);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xFF, 0xFF, 0xFF);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC0, 0xA0, 0x70));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0x1A, 0x1A, 0x1C));

        v.widgets.active.bg_fill = Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 40);
        v.widgets.active.weak_bg_fill = Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 40);
        v.widgets.active.bg_stroke = Stroke::new(1.0, accent);
        v.widgets.active.fg_stroke = Stroke::new(1.0, accent);

        v.widgets.open.bg_fill = Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 32);
        v.widgets.open.weak_bg_fill = Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 32);
        v.widgets.open.bg_stroke = Stroke::new(1.0, accent);
        v.widgets.open.fg_stroke = Stroke::new(1.0, accent);
    }
}

fn apply_gallery_style(style: &mut Style) {
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    // Symmetric padding keeps glyph boxes visually centered in the button frame.
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = Margin::same(10);
    style.spacing.menu_margin = Margin::same(6);
    // Shared default height so toolbar controls line up top/bottom.
    style.spacing.interact_size = egui::vec2(40.0, TOOLBAR_BUTTON_HEIGHT);
    style.spacing.slider_rail_height = 6.0;
    style.visuals.button_frame = true;
    style.visuals.striped = false;
}

/// Uniform height for toolbar / chrome controls (matches `interact_size.y`).
pub const TOOLBAR_BUTTON_HEIGHT: f32 = 28.0;

fn opacity_u8(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Semi-opaque elevated fill for immersive top/bottom chrome bars.
pub fn chrome_fill(visuals: &Visuals, opacity: f32) -> Color32 {
    let base = visuals.window_fill;
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), opacity_u8(opacity))
}

/// Comic reader central-panel fill: same opacity as chrome bars, against the
/// transparent window clear (desktop), not against page pixels.
pub fn reader_background_fill(rgb: [u8; 3], opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], opacity_u8(opacity))
}

/// Frame used by reader / ebook / media toolbars and status bars.
pub fn chrome_bar_frame(visuals: &Visuals, opacity: f32) -> egui::Frame {
    let stroke_alpha = opacity_u8(opacity * 0.4);
    egui::Frame::new()
        .fill(chrome_fill(visuals, opacity))
        // Tight vertical margin so buttons nearly fill the chrome row.
        .inner_margin(Margin::symmetric(12, 4))
        .stroke(Stroke::new(
            1.0,
            if visuals.dark_mode {
                Color32::from_rgba_unmultiplied(0x40, 0x40, 0x48, stroke_alpha)
            } else {
                Color32::from_rgba_unmultiplied(0xC0, 0xC2, 0xC8, stroke_alpha)
            },
        ))
}

/// Prepare a chrome toolbar row: fixed height, vertically centered children.
pub fn begin_chrome_row(ui: &mut egui::Ui) {
    ui.set_min_height(TOOLBAR_BUTTON_HEIGHT);
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.spacing_mut().item_spacing.y = 0.0;
}

/// Soft dashed horizontal rule — used to separate library header from content.
pub fn dashed_separator(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
    let y = rect.center().y;
    let color = if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(0xD4, 0xA5, 0x74, 90)
    } else {
        Color32::from_rgba_unmultiplied(0xB8, 0x83, 0x4A, 100)
    };
    let stroke = Stroke::new(1.0, color);
    // Inset a little so the dash doesn't kiss the window edge.
    let inset = 4.0;
    let left = rect.left() + inset;
    let right = rect.right() - inset;
    let dash = 6.0;
    let gap = 5.0;
    let mut x = left;
    let painter = ui.painter();
    while x < right {
        let x2 = (x + dash).min(right);
        painter.line_segment([egui::pos2(x, y), egui::pos2(x2, y)], stroke);
        x += dash + gap;
    }
    ui.add_space(8.0);
}

/// Tag / filter chip (rounded capsule).
pub fn tag_chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let accent = accent_for(dark);
    let (fill, fg) = if selected {
        (
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                if dark { 56 } else { 40 },
            ),
            accent,
        )
    } else {
        (
            ui.visuals().extreme_bg_color,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.inactive.fg_stroke.color)
                .gamma_multiply(0.75),
        )
    };
    let inner = egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(RADIUS_CHIP))
        .inner_margin(Margin::symmetric(10, 4))
        .stroke(if selected {
            Stroke::new(1.0, accent)
        } else {
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        })
        .show(ui, |ui| {
            ui.label(egui::RichText::new(label).size(12.5).color(fg));
        });
    inner.response.interact(egui::Sense::click())
}

/// Segmented control background wrapping selectable labels.
pub fn segmented_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(3))
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            add_contents(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_visuals_differ() {
        let dark = dark_visuals();
        let light = light_visuals();
        assert!(dark.dark_mode);
        assert!(!light.dark_mode);
        assert_ne!(dark.panel_fill, light.panel_fill);
        assert_eq!(dark.hyperlink_color, ACCENT_DARK);
        assert_eq!(light.hyperlink_color, ACCENT_LIGHT);
    }

    #[test]
    fn chrome_fill_is_translucent() {
        let dark = dark_visuals();
        let fill = chrome_fill(&dark, 0.85);
        assert!(fill.a() < 255);
        assert_eq!(chrome_fill(&dark, 1.0).a(), 255);
    }
}
