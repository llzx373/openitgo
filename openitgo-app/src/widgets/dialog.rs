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
    let closing_key = id.with("closing");
    let pending_action_key = id.with("pending_action");
    let pending_close_key = id.with("pending_close");
    ctx.memory_mut(|memory| {
        if !memory.data.get_temp::<bool>(open_marker).unwrap_or(false) {
            memory.data.insert_temp(return_focus_key, memory.focused());
            memory.data.insert_temp(open_marker, true);
        }
    });
    let was_closing = ctx.data(|data| data.get_temp::<bool>(closing_key).unwrap_or(false));
    let opacity = crate::theme::motion::animate_visibility(ctx, id.with("opacity"), !was_closing);

    let mut clicked_action = None;
    let mut default_focus = None;
    let modal = egui::Modal::new(id).show(ctx, |ui| {
        ui.set_opacity(opacity);
        if was_closing {
            ui.disable();
        }
        ui.set_max_width(560.0);
        ui.spacing_mut().item_spacing.y = spacing::SM;
        ui.label(egui::RichText::new(title).font(crate::theme::semibold_font(text::SUBTITLE)));
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
                        clicked_action = Some(index);
                    }
                }
            });
        }
        ui.min_rect()
    });
    let escape_close = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let requested = clicked_action.is_some() || escape_close;
    ctx.memory_mut(|memory| {
        if requested {
            memory.data.insert_temp(closing_key, true);
            memory.data.insert_temp(pending_action_key, clicked_action);
            memory.data.insert_temp(pending_close_key, escape_close);
        }
    });
    let closing = was_closing || requested;
    if let Some(response) = default_focus.filter(|_| !closing) {
        let focus_inside = ctx.memory(|memory| memory.focused()).and_then(|focused| {
            ctx.read_response(focused)
                .map(|focused| modal.inner.contains(focused.rect.center()))
        });
        if focus_inside != Some(true) {
            response.request_focus();
        }
    }

    let finished = closing
        && (crate::theme::motion::is_reduced(ctx)
            || crate::theme::motion::animate_visibility(ctx, id.with("opacity"), false) <= 0.01);
    let (action, should_close) = if finished {
        ctx.data(|data| {
            (
                data.get_temp::<Option<usize>>(pending_action_key)
                    .unwrap_or(None),
                data.get_temp::<bool>(pending_close_key).unwrap_or(false),
            )
        })
    } else {
        (None, false)
    };
    if finished {
        ctx.memory_mut(|memory| {
            if let Some(Some(return_focus)) =
                memory.data.get_temp::<Option<egui::Id>>(return_focus_key)
            {
                memory.request_focus(return_focus);
            }
            memory.data.remove::<bool>(open_marker);
            memory.data.remove::<Option<egui::Id>>(return_focus_key);
            memory.data.remove::<bool>(closing_key);
            memory.data.remove::<Option<usize>>(pending_action_key);
            memory.data.remove::<bool>(pending_close_key);
        });
    }
    DialogResponse {
        action,
        should_close,
    }
}
