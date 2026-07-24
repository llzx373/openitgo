//! Classic right-triangle volume control (thin left → tall right).

use egui::{Color32, Mesh, Pos2, Sense, Shape, Stroke, Vec2};

const WEDGE_WIDTH: f32 = 88.0;
const WEDGE_HEIGHT: f32 = 16.0;

/// Interactive right-triangle volume wedge. `volume` is 0..=100.
/// Returns the response; caller applies volume when `changed`.
pub fn volume_wedge(
    ui: &mut egui::Ui,
    volume: &mut f32,
    enabled: bool,
    opacity: f32,
) -> egui::Response {
    let desired = Vec2::new(WEDGE_WIDTH, WEDGE_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(
        desired,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );

    if enabled {
        if let Some(pos) = response.interact_pointer_pos() {
            let next = volume_at_x(pos.x, rect) * 100.0;
            if (next - *volume).abs() > f32::EPSILON {
                *volume = next;
                response.mark_changed();
            }
        }
    }

    let ratio = (*volume / 100.0).clamp(0.0, 1.0);
    draw_wedge(ui, rect, ratio, enabled, opacity);
    response
}

fn alpha(opacity: f32, scale: f32) -> u8 {
    ((opacity.clamp(0.0, 1.0) * scale).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn draw_wedge(ui: &mut egui::Ui, rect: egui::Rect, ratio: f32, enabled: bool, opacity: f32) {
    // High-contrast empty wedge so the silhouette reads on chrome bars.
    let track_a = if enabled {
        alpha(opacity, 0.95)
    } else {
        alpha(opacity, 0.4)
    };
    let track_color = if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(0xA8, 0xA8, 0xB0, track_a)
    } else {
        Color32::from_rgba_unmultiplied(0x55, 0x57, 0x5E, track_a)
    };

    let accent = ui.visuals().selection.stroke.color;
    let fill_a = if enabled {
        alpha(opacity.max(0.92), 1.0)
    } else {
        alpha(opacity, 0.3)
    };
    let fill_color = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), fill_a);

    let painter = ui.painter();
    painter.add(triangle_mesh(rect, 1.0, track_color));
    if ratio > 0.001 {
        painter.add(triangle_mesh(rect, ratio, fill_color));
    }

    let stroke = Stroke::new(
        1.25,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha(opacity, 0.85)),
    );
    let (a, b, c) = triangle_points(rect, 1.0);
    painter.add(Shape::closed_line(vec![a, b, c], stroke));
}

/// Right triangle: tip at bottom-left, vertical leg on the right.
fn triangle_points(rect: egui::Rect, scale: f32) -> (Pos2, Pos2, Pos2) {
    let scale = scale.clamp(0.0, 1.0);
    let w = rect.width() * scale;
    let h = rect.height() * scale;
    let left = rect.min.x;
    let bottom = rect.max.y;
    (
        Pos2::new(left, bottom),
        Pos2::new(left + w, bottom),
        Pos2::new(left + w, bottom - h),
    )
}

fn triangle_mesh(rect: egui::Rect, scale: f32, color: Color32) -> Shape {
    let (a, b, c) = triangle_points(rect, scale);
    let mut mesh = Mesh::default();
    let i0 = mesh.vertices.len() as u32;
    mesh.colored_vertex(a, color);
    mesh.colored_vertex(b, color);
    mesh.colored_vertex(c, color);
    mesh.add_triangle(i0, i0 + 1, i0 + 2);
    Shape::mesh(mesh)
}

pub(crate) fn volume_at_x(x: f32, rect: egui::Rect) -> f32 {
    if rect.width() <= 0.0 {
        return 0.0;
    }
    ((x - rect.min.x) / rect.width()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_at_x_maps_edges_and_mid() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(100.0, 16.0));
        assert!((volume_at_x(10.0, rect) - 0.0).abs() < f32::EPSILON);
        assert!((volume_at_x(110.0, rect) - 1.0).abs() < f32::EPSILON);
        assert!((volume_at_x(60.0, rect) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn similar_triangle_scales_from_left_tip() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(80.0, 20.0));
        let (a, b, c) = triangle_points(rect, 0.5);
        assert_eq!(a, egui::pos2(0.0, 20.0));
        assert_eq!(b, egui::pos2(40.0, 20.0));
        assert_eq!(c, egui::pos2(40.0, 10.0));
    }
}
