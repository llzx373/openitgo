//! Gallery Immersive design tokens and egui Visuals/Style.
//!
//! Dark-first reading shell: cool charcoal surfaces, warm amber accent,
//! soft rounding, minimal borders. Light mode uses a cool gallery gray
//! (not warm cream).

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Style, Theme, Visuals};
use openitgo_storage::models::EbookTheme;

/// Warm amber accent — used for selection, progress, and active chrome.
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0xD4, 0xA5, 0x74);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0xB8, 0x83, 0x4A);

/// Ebook webview palette aligned with Gallery shell (Dark/Light) or sepia paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EbookPalette {
    pub bg: Color32,
    pub fg: Color32,
    pub accent: Color32,
}

impl EbookPalette {
    pub fn for_theme(theme: EbookTheme) -> Self {
        match theme {
            // Cool gallery gray + ink (matches light_visuals panel / text).
            EbookTheme::Light => Self {
                bg: Color32::from_rgb(0xEE, 0xEF, 0xF1),
                fg: Color32::from_rgb(0x1A, 0x1A, 0x1C),
                accent: ACCENT_LIGHT,
            },
            // Charcoal panel + warm off-white (matches dark_visuals).
            EbookTheme::Dark => Self {
                bg: Color32::from_rgb(0x14, 0x14, 0x16),
                fg: Color32::from_rgb(0xF0, 0xEE, 0xE8),
                accent: ACCENT_DARK,
            },
            // Independent parchment; keep sepia identity.
            EbookTheme::Sepia => Self {
                bg: Color32::from_rgb(0xF4, 0xEC, 0xD8),
                fg: Color32::from_rgb(0x5B, 0x46, 0x36),
                accent: Color32::from_rgb(0xA0, 0x78, 0x48),
            },
        }
    }

    pub fn bg_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.bg.r(), self.bg.g(), self.bg.b())
    }

    pub fn fg_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.fg.r(), self.fg.g(), self.fg.b())
    }

    pub fn accent_hex(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            self.accent.r(),
            self.accent.g(),
            self.accent.b()
        )
    }
}

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

/// Compact Dark/Light swatch used next to the theme picker.
pub fn theme_swatch(ui: &mut egui::Ui, panel: Color32, accent: Color32) -> egui::Response {
    let size = egui::vec2(28.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let rounding = CornerRadius::same(4);
    ui.painter().rect_filled(rect, rounding, panel);
    let accent_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 12.0, rect.center().y - 5.0),
        egui::vec2(8.0, 10.0),
    );
    ui.painter()
        .rect_filled(accent_rect, CornerRadius::same(2), accent);
    ui.painter().rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Outside,
    );
    response
}

/// Tag / filter chip (rounded capsule) with selected + hover fills.
pub fn tag_chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let accent = accent_for(dark);
    let id = ui.id().with(("tag_chip", label));
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());

    let (fill, fg, stroke) = if selected {
        (
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                if dark { 56 } else { 40 },
            ),
            accent,
            Stroke::new(1.0, accent),
        )
    } else if hovered {
        (
            ui.visuals().widgets.hovered.weak_bg_fill,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.hovered.fg_stroke.color),
            Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 160),
            ),
        )
    } else {
        (
            ui.visuals().extreme_bg_color,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.inactive.fg_stroke.color)
                .gamma_multiply(0.75),
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        )
    };

    let inner = egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(RADIUS_CHIP))
        .inner_margin(Margin::symmetric(10, 4))
        .stroke(stroke)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(label).size(12.5).color(fg));
        });
    ui.interact(inner.response.rect, id, egui::Sense::click())
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

/// Fixed-size tab label: hover / selected only change fill — never width.
fn tab_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    size: egui::Vec2,
    page_fill: Color32,
) -> egui::Response {
    let font = egui::FontId::proportional(13.5);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();

    let (fill, fg) = if selected {
        (
            page_fill,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.active.fg_stroke.color),
        )
    } else if hovered {
        (
            ui.visuals().widgets.hovered.weak_bg_fill,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.hovered.fg_stroke.color),
        )
    } else {
        (
            Color32::TRANSPARENT,
            ui.visuals()
                .override_text_color
                .unwrap_or(ui.visuals().widgets.inactive.fg_stroke.color)
                .gamma_multiply(0.72),
        )
    };

    let rounding = CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, rounding, fill);
    }
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
    let text_pos = egui::pos2(
        rect.center().x - galley.size().x * 0.5,
        rect.center().y - galley.size().y * 0.5,
    );
    ui.painter().galley(text_pos, galley, fg);

    response
}

/// Connected tab strip + page body (no gap / double stroke between them).
///
/// Selected tab uses the same fill as the page so it reads as one panel.
/// Returns the (possibly updated) selected tab.
pub fn tabbed_page<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    tabs: &[(T, &str)],
    mut current: T,
    add_contents: impl FnOnce(&mut egui::Ui, T),
) -> T {
    if tabs.is_empty() {
        add_contents(ui, current);
        return current;
    }

    let font = egui::FontId::proportional(13.5);
    let pad_x = 16.0;
    let pad_y = 7.0;
    let mut max_w = 64.0_f32;
    let mut tab_h = 28.0_f32;
    for (_, label) in tabs {
        let galley = ui
            .painter()
            .layout_no_wrap((*label).to_owned(), font.clone(), Color32::WHITE);
        max_w = max_w.max(galley.size().x + pad_x * 2.0);
        tab_h = tab_h.max(galley.size().y + pad_y * 2.0);
    }
    let size = egui::vec2(max_w, tab_h);

    let border = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let page_fill = ui.visuals().window_fill;
    let strip_fill = ui.visuals().extreme_bg_color;
    let accent = accent_for(ui.visuals().dark_mode);

    let mut tab_rects: Vec<(T, egui::Rect)> = Vec::with_capacity(tabs.len());

    // Kill inter-widget gap so strip and body paint as one surface.
    let prev_spacing_y = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;

    egui::Frame::new()
        .stroke(border)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            // Tab strip (darker rail); selected tab is painted with page_fill.
            let strip = egui::Frame::new()
                .fill(strip_fill)
                .inner_margin(Margin::symmetric(4, 0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for &(tab, label) in tabs {
                            let selected = current == tab;
                            let response = tab_button_sized(ui, label, selected, size, page_fill);
                            tab_rects.push((tab, response.rect));
                            if response.clicked() && !selected {
                                current = tab;
                            }
                        }
                        // Fill the rest of the strip width with strip_fill (already bg).
                        ui.allocate_at_least(
                            egui::vec2(ui.available_width(), size.y),
                            egui::Sense::hover(),
                        );
                    });
                });

            // Divider under inactive tabs only — gap under the selected tab
            // so the page fill continues unbroken into the selected tab.
            let strip_rect = strip.response.rect;
            let y = strip_rect.bottom() - 0.5;
            let selected_rect = tab_rects
                .iter()
                .find(|(tab, _)| *tab == current)
                .map(|(_, rect)| *rect);
            if let Some(sel) = selected_rect {
                // Re-paint selected tab with page fill in case the click just switched.
                ui.painter().rect_filled(
                    sel,
                    CornerRadius {
                        nw: 6,
                        ne: 6,
                        sw: 0,
                        se: 0,
                    },
                    page_fill,
                );
                // Redraw label on top of the refilled tab.
                if let Some((_, label)) = tabs.iter().find(|(tab, _)| *tab == current) {
                    let font = egui::FontId::proportional(13.5);
                    let fg = ui
                        .visuals()
                        .override_text_color
                        .unwrap_or(ui.visuals().widgets.active.fg_stroke.color);
                    let galley = ui.painter().layout_no_wrap((*label).to_owned(), font, fg);
                    let text_pos = egui::pos2(
                        sel.center().x - galley.size().x * 0.5,
                        sel.center().y - galley.size().y * 0.5,
                    );
                    ui.painter().galley(text_pos, galley, fg);
                }

                if sel.left() > strip_rect.left() + 0.5 {
                    ui.painter()
                        .hline(strip_rect.left()..=sel.left(), y, border);
                }
                if sel.right() < strip_rect.right() - 0.5 {
                    ui.painter()
                        .hline(sel.right()..=strip_rect.right(), y, border);
                }
                // Accent on the top of the selected tab — keeps the join seamless.
                ui.painter().hline(
                    (sel.left() + 8.0)..=(sel.right() - 8.0),
                    sel.top() + 1.5,
                    Stroke::new(2.0, accent),
                );
            } else {
                ui.painter().hline(strip_rect.x_range(), y, border);
            }

            egui::Frame::new()
                .fill(page_fill)
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    add_contents(ui, current);
                });
        });

    ui.spacing_mut().item_spacing.y = prev_spacing_y;
    current
}

/// Capsule segmented control with fixed-size segments (no hover width jump).
pub fn segmented_tabs<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    tabs: &[(T, &str)],
    current: &mut T,
) -> bool {
    if tabs.is_empty() {
        return false;
    }
    let font = egui::FontId::proportional(13.0);
    let pad_x = 12.0;
    let pad_y = 5.0;
    let mut max_w = 48.0_f32;
    let mut tab_h = 26.0_f32;
    for (_, label) in tabs {
        let galley = ui
            .painter()
            .layout_no_wrap((*label).to_owned(), font.clone(), Color32::WHITE);
        max_w = max_w.max(galley.size().x + pad_x * 2.0);
        tab_h = tab_h.max(galley.size().y + pad_y * 2.0);
    }
    let size = egui::vec2(max_w, tab_h);
    let dark = ui.visuals().dark_mode;
    let accent = accent_for(dark);

    let mut changed = false;
    segmented_frame(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for &(tab, label) in tabs {
                let selected = *current == tab;
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let hovered = response.hovered();
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
                } else if hovered {
                    (
                        ui.visuals().widgets.hovered.weak_bg_fill,
                        ui.visuals()
                            .override_text_color
                            .unwrap_or(ui.visuals().widgets.hovered.fg_stroke.color),
                    )
                } else {
                    (
                        Color32::TRANSPARENT,
                        ui.visuals()
                            .override_text_color
                            .unwrap_or(ui.visuals().widgets.inactive.fg_stroke.color)
                            .gamma_multiply(0.8),
                    )
                };
                if fill.a() > 0 {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                }
                let galley = ui
                    .painter()
                    .layout_no_wrap(label.to_owned(), font.clone(), fg);
                let text_pos = egui::pos2(
                    rect.center().x - galley.size().x * 0.5,
                    rect.center().y - galley.size().y * 0.5,
                );
                ui.painter().galley(text_pos, galley, fg);
                if response.clicked() && !selected {
                    *current = tab;
                    changed = true;
                }
            }
        });
    });
    changed
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

    #[test]
    fn ebook_palette_aligns_with_gallery_shell() {
        let dark = EbookPalette::for_theme(EbookTheme::Dark);
        assert_eq!(dark.bg, dark_visuals().panel_fill);
        assert_eq!(dark.fg, Color32::from_rgb(0xF0, 0xEE, 0xE8));
        assert_eq!(dark.accent, ACCENT_DARK);

        let light = EbookPalette::for_theme(EbookTheme::Light);
        assert_eq!(light.bg, light_visuals().panel_fill);
        assert_eq!(light.accent, ACCENT_LIGHT);

        let sepia = EbookPalette::for_theme(EbookTheme::Sepia);
        assert_eq!(sepia.bg_hex(), "#f4ecd8");
    }
}
