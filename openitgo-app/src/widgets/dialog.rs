use super::fluent::{self, ButtonKind};
use crate::theme::tokens::{spacing, text};

pub struct DialogAction<'a> {
    pub label: &'a str,
    pub kind: ButtonKind,
    pub default_focus: bool,
    pub enabled: bool,
}

impl<'a> DialogAction<'a> {
    pub fn new(label: &'a str, kind: ButtonKind) -> Self {
        Self {
            label,
            kind,
            default_focus: false,
            enabled: true,
        }
    }

    pub fn default_focus(mut self) -> Self {
        self.default_focus = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

pub struct DialogResponse {
    pub action: Option<usize>,
    pub should_close: bool,
}

pub fn modal_dialog(
    ctx: &egui::Context,
    id: egui::Id,
    title: &str,
    actions: &[DialogAction<'_>],
    content: impl FnOnce(&mut egui::Ui),
) -> DialogResponse {
    let open_marker = id.with("open");
    let return_focus_key = id.with("return_focus");
    ctx.memory_mut(|memory| {
        if !memory.data.get_temp::<bool>(open_marker).unwrap_or(false) {
            memory.data.insert_temp(return_focus_key, memory.focused());
            memory.data.insert_temp(open_marker, true);
        }
    });

    let mut action = None;
    let mut default_focus = None;
    let modal = egui::Modal::new(id).show(ctx, |ui| {
        ui.set_max_width(560.0);
        ui.spacing_mut().item_spacing.y = spacing::SM;
        ui.label(egui::RichText::new(title).size(text::SUBTITLE).strong());
        egui::ScrollArea::vertical()
            .max_height(480.0)
            .auto_shrink([true, true])
            .show(ui, content);
        if !actions.is_empty() {
            ui.add_space(spacing::SM);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (index, action_definition) in actions.iter().enumerate() {
                    let response = ui
                        .add_enabled_ui(action_definition.enabled, |ui| {
                            fluent::button(ui, action_definition.label, action_definition.kind)
                        })
                        .inner;
                    if action_definition.default_focus && action_definition.enabled {
                        default_focus = Some(response.clone());
                    }
                    if response.clicked() {
                        action = Some(index);
                    }
                }
            });
        }
        ui.min_rect()
    });
    if let Some(response) = default_focus {
        let focus_inside = ctx.memory(|memory| memory.focused()).and_then(|focused| {
            ctx.read_response(focused)
                .map(|focused| modal.inner.contains(focused.rect.center()))
        });
        if focus_inside != Some(true) {
            response.request_focus();
        }
    }
    let should_close = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if action.is_some() || should_close {
        ctx.memory_mut(|memory| {
            if let Some(Some(return_focus)) =
                memory.data.get_temp::<Option<egui::Id>>(return_focus_key)
            {
                memory.request_focus(return_focus);
            }
            memory.data.remove::<bool>(open_marker);
            memory.data.remove::<Option<egui::Id>>(return_focus_key);
        });
    }
    DialogResponse {
        action,
        should_close,
    }
}
