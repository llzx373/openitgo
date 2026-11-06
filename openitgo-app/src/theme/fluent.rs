use super::tokens::{self, radius, size, spacing, text, FluentPalette};
use egui::{Color32, CornerRadius, FontId, Margin, Stroke, Style, TextStyle, Visuals};

pub fn palette(dark: bool) -> FluentPalette {
    if dark {
        FluentPalette::dark()
    } else {
        FluentPalette::light()
    }
}

pub fn visuals(dark: bool) -> Visuals {
    let p = palette(dark);
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = dark;
    v.override_text_color = Some(p.text);
    v.weak_text_alpha = 0.72;
    v.panel_fill = p.canvas;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface_alt;
    v.faint_bg_color = p.card_hover;
    v.code_bg_color = p.surface_alt;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = if dark {
        Color32::from_rgb(0xFC, 0xE1, 0x00)
    } else {
        Color32::from_rgb(0x9D, 0x5D, 0x00)
    };
    v.error_fg_color = p.danger;
    v.window_corner_radius = CornerRadius::same(radius::CARD);
    v.menu_corner_radius = CornerRadius::same(radius::CARD);
    v.window_stroke = Stroke::new(1.0, p.stroke);
    v.window_shadow = tokens::elevation::window(dark);
    v.popup_shadow = tokens::elevation::popup(dark);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(
        p.accent.r(),
        p.accent.g(),
        p.accent.b(),
        if dark { 56 } else { 32 },
    );
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let control_radius = CornerRadius::same(radius::CONTROL);
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = control_radius;
    }
    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.weak_bg_fill = p.surface_alt;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary);

    v.widgets.inactive.bg_fill = p.control;
    v.widgets.inactive.weak_bg_fill = p.control;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);

    v.widgets.hovered.bg_fill = p.control_hover;
    v.widgets.hovered.weak_bg_fill = p.control_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);

    v.widgets.active.bg_fill = p.control_pressed;
    v.widgets.active.weak_bg_fill = p.control_pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);

    v.widgets.open.bg_fill = p.surface_alt;
    v.widgets.open.weak_bg_fill = p.surface_alt;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v
}

pub fn apply_style(style: &mut Style) {
    style.spacing.item_spacing = egui::vec2(spacing::SM, spacing::SM);
    style.spacing.button_padding = egui::vec2(spacing::MD, 6.0);
    style.spacing.window_margin = Margin::same(spacing::LG as i8);
    style.spacing.menu_margin = Margin::same(spacing::XS as i8);
    style.spacing.interact_size = egui::vec2(40.0, size::CONTROL);
    style.spacing.slider_rail_height = 4.0;
    style.visuals.button_frame = true;
    style.visuals.striped = false;
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(text::SMALL));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(text::BODY));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(text::BODY));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(text::TITLE));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_palettes_have_distinct_surface_roles() {
        let light = FluentPalette::light();
        let dark = FluentPalette::dark();
        assert_ne!(light.canvas, light.surface);
        assert_ne!(light.card, light.control_hover);
        assert_ne!(dark.canvas, dark.surface);
        assert_ne!(light.accent, dark.accent);
    }

    #[test]
    fn fluent_visuals_use_palette_and_standard_metrics() {
        let egui_ctx = egui::Context::default();
        egui_ctx.style_mut_of(egui::Theme::Light, apply_style);
        let style = egui_ctx.style_of(egui::Theme::Light);
        assert_eq!(style.spacing.interact_size.y, size::CONTROL);
        assert_eq!(style.text_styles[&TextStyle::Body].size, text::BODY);

        let light = visuals(false);
        let dark = visuals(true);
        assert_eq!(light.panel_fill, FluentPalette::light().canvas);
        assert_eq!(dark.panel_fill, FluentPalette::dark().canvas);
        assert_eq!(
            light.widgets.inactive.corner_radius,
            CornerRadius::same(radius::CONTROL)
        );
    }
}
