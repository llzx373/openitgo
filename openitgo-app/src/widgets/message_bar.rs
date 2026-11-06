use super::fluent::{self, ButtonKind};
use crate::theme::tokens::{radius, spacing};
use egui_phosphor_icons::icons;

#[derive(Clone, Copy)]
pub enum MessageKind {
    Info,
    Success,
    Warning,
    Error,
}

pub fn message_bar(ui: &mut egui::Ui, kind: MessageKind, text: &str) -> egui::Response {
    let (icon, color) = match kind {
        MessageKind::Info => (icons::INFO, crate::theme::accent(ui.visuals())),
        MessageKind::Success => (
            icons::CHECK_CIRCLE,
            if ui.visuals().dark_mode {
                egui::Color32::from_rgb(0x6C, 0xC7, 0x8D)
            } else {
                egui::Color32::from_rgb(0x0F, 0x7B, 0x3D)
            },
        ),
        MessageKind::Warning => (icons::WARNING, ui.visuals().warn_fg_color),
        MessageKind::Error => (icons::X_CIRCLE, ui.visuals().error_fg_color),
    };
    let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 20);
    let mut close = None;
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.65)))
        .corner_radius(egui::CornerRadius::same(radius::CONTROL))
        .inner_margin(egui::Margin::symmetric(
            spacing::MD as i8,
            spacing::XS as i8,
        ))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icon.as_str()).color(color));
                ui.add(
                    egui::Label::new(egui::RichText::new(text).color(ui.visuals().text_color()))
                        .wrap(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = Some(fluent::icon_button(
                        ui,
                        icons::X,
                        "关闭消息",
                        ButtonKind::Subtle,
                    ));
                });
            });
        });
    frame
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    close.expect("message bar close button")
}
