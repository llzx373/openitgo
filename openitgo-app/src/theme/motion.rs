const REDUCE_MOTION_KEY: &str = "fluent_reduce_motion";

pub fn set_reduced(ctx: &egui::Context, reduced: bool) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new(REDUCE_MOTION_KEY), reduced);
    });
}

pub fn is_reduced(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<bool>(egui::Id::new(REDUCE_MOTION_KEY))
            .unwrap_or(false)
    })
}

pub fn animate_visibility(ctx: &egui::Context, id: egui::Id, visible: bool) -> f32 {
    if is_reduced(ctx) {
        if visible {
            1.0
        } else {
            0.0
        }
    } else {
        ctx.animate_bool_with_time_and_easing(id, visible, 0.12, egui::emath::easing::cubic_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_skips_animation() {
        let ctx = egui::Context::default();
        set_reduced(&ctx, true);
        assert_eq!(animate_visibility(&ctx, egui::Id::new("bar"), false), 0.0);
        assert_eq!(animate_visibility(&ctx, egui::Id::new("bar"), true), 1.0);
    }
}
