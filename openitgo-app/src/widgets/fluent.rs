#![allow(dead_code)]

use crate::theme::fluent::palette;
use crate::theme::tokens::{radius, size, spacing, text};
use egui::{
    Align, Color32, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, WidgetInfo, WidgetType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Standard,
    Primary,
    Subtle,
}

pub fn button(ui: &mut Ui, label: impl Into<String>, kind: ButtonKind) -> Response {
    let label = label.into();
    let galley = ui.painter().layout_no_wrap(
        label.clone(),
        FontId::proportional(text::BODY),
        Color32::WHITE,
    );
    let width = (galley.size().x + spacing::XL).max(80.0);
    button_impl(ui, label, kind, egui::vec2(width, size::CONTROL), true)
}

pub fn icon_button(ui: &mut Ui, icon: impl Into<String>, kind: ButtonKind) -> Response {
    button_impl(
        ui,
        icon.into(),
        kind,
        egui::vec2(size::CONTROL, size::CONTROL),
        true,
    )
}

fn button_impl(
    ui: &mut Ui,
    label: String,
    kind: ButtonKind,
    size: egui::Vec2,
    wrap: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let p = palette(ui.visuals().dark_mode);
    let enabled = ui.is_enabled();
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() || response.has_focus();
    let (mut fill, mut fg, mut stroke) = match kind {
        ButtonKind::Primary => (p.accent, p.on_accent, Stroke::NONE),
        ButtonKind::Standard => (p.control, p.text, Stroke::new(1.0, p.stroke_strong)),
        ButtonKind::Subtle => (Color32::TRANSPARENT, p.text, Stroke::NONE),
    };
    if enabled && hovered {
        fill = match kind {
            ButtonKind::Primary => p.accent_hover,
            ButtonKind::Standard | ButtonKind::Subtle => p.control_hover,
        };
    }
    if enabled && pressed {
        fill = match kind {
            ButtonKind::Primary => p.accent_pressed,
            ButtonKind::Standard | ButtonKind::Subtle => p.control_pressed,
        };
        if kind != ButtonKind::Primary {
            fg = p.text_secondary;
        }
    }
    if !enabled {
        fill = if kind == ButtonKind::Subtle {
            Color32::TRANSPARENT
        } else {
            p.surface_alt
        };
        fg = p.text_secondary;
        stroke = if kind == ButtonKind::Standard {
            Stroke::new(1.0, p.stroke)
        } else {
            Stroke::NONE
        };
    }

    let rounding = CornerRadius::same(radius::CONTROL);
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, rounding, fill);
    }
    if stroke != Stroke::NONE {
        ui.painter()
            .rect_stroke(rect, rounding, stroke, StrokeKind::Inside);
    }
    if response.has_focus() {
        paint_focus_ring(ui, rect);
    }
    let font = FontId::proportional(text::BODY);
    let galley = if wrap {
        ui.painter().layout_no_wrap(label, font, fg)
    } else {
        ui.painter().layout(label, font, fg, rect.width())
    };
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        fg,
    );
    response
}

pub fn toggle(ui: &mut Ui, value: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(40.0, 20.0), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *value, "开关")
    });
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let p = palette(ui.visuals().dark_mode);
    let enabled = ui.is_enabled();
    let fill = if !enabled {
        p.surface_alt
    } else if *value {
        if response.is_pointer_button_down_on() {
            p.accent_pressed
        } else if response.hovered() {
            p.accent_hover
        } else {
            p.accent
        }
    } else if response.is_pointer_button_down_on() {
        p.control_pressed
    } else if response.hovered() {
        p.control_hover
    } else {
        p.control
    };
    let stroke = if *value {
        Stroke::NONE
    } else {
        Stroke::new(1.0, p.stroke_strong)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let knob_radius = if response.is_pointer_button_down_on() {
        6.0
    } else {
        5.0
    };
    let knob_x = if *value {
        rect.right() - 10.0
    } else {
        rect.left() + 10.0
    };
    let knob_color = if *value {
        p.on_accent
    } else {
        p.text_secondary
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, rect.center().y), knob_radius, knob_color);
    if response.has_focus() {
        paint_focus_ring(ui, rect.expand(2.0));
    }
    response
}

pub fn navigation_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, size::CONTROL + 4.0), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let p = palette(ui.visuals().dark_mode);
    if selected || response.hovered() || response.has_focus() {
        let fill = if selected {
            p.surface_alt
        } else if response.is_pointer_button_down_on() {
            p.control_pressed
        } else {
            p.control_hover
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    }
    if selected {
        let indicator = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            egui::vec2(3.0, 16.0),
        );
        ui.painter()
            .rect_filled(indicator, CornerRadius::same(2), p.accent);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(text::BODY),
        if selected { p.text } else { p.text_secondary },
    );
    if response.has_focus() {
        paint_focus_ring(ui, rect);
    }
    response
}

pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    ui.scope(|ui| {
        let p = palette(ui.visuals().dark_mode);
        ui.spacing_mut().slider_width = 160.0;
        ui.spacing_mut().slider_rail_height = 4.0;
        let visuals = ui.visuals_mut();
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.bg_fill = p.control_pressed;
            widget.weak_bg_fill = p.control_pressed;
        }
        visuals.selection.bg_fill = p.accent;
        ui.add(slider)
    })
    .inner
}

pub fn setting_group(
    ui: &mut Ui,
    title: &str,
    add_contents: impl FnOnce(&mut Ui),
) -> egui::InnerResponse<()> {
    let p = palette(ui.visuals().dark_mode);
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(egui::Margin::same(spacing::LG as i8))
        .show(ui, |ui| {
            ui.set_width((width - spacing::LG * 2.0).max(0.0));
            ui.spacing_mut().item_spacing.y = spacing::MD;
            ui.label(
                RichText::new(title)
                    .font(FontId::proportional(text::SUBTITLE))
                    .strong()
                    .color(p.text),
            );
            add_contents(ui);
        })
}

pub fn setting_row(
    ui: &mut Ui,
    title: &str,
    description: Option<&str>,
    add_contents: impl FnOnce(&mut Ui),
) {
    let stacked = use_stacked_setting_row(ui.available_width());
    if stacked {
        ui.vertical(|ui| {
            setting_labels(ui, title, description);
            ui.add_space(spacing::XS);
            ui.horizontal(add_contents);
        });
    } else {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(560.0);
                setting_labels(ui, title, description);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), add_contents);
        });
    }
}

pub fn use_stacked_setting_row(available_width: f32) -> bool {
    available_width < 560.0
}

fn setting_labels(ui: &mut Ui, title: &str, description: Option<&str>) {
    ui.spacing_mut().item_spacing.y = spacing::XS;
    let p = palette(ui.visuals().dark_mode);
    ui.label(
        RichText::new(title)
            .font(FontId::proportional(text::BODY))
            .color(p.text),
    );
    if let Some(description) = description {
        ui.label(
            RichText::new(description)
                .font(FontId::proportional(text::SMALL))
                .color(p.text_secondary),
        );
    }
}

fn paint_focus_ring(ui: &Ui, rect: egui::Rect) {
    let p = palette(ui.visuals().dark_mode);
    ui.painter().rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(radius::CONTROL + 2),
        Stroke::new(2.0, p.accent),
        StrokeKind::Outside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_rows_stack_only_when_narrow() {
        assert!(use_stacked_setting_row(480.0));
        assert!(!use_stacked_setting_row(560.0));
        assert!(!use_stacked_setting_row(960.0));
    }
}
