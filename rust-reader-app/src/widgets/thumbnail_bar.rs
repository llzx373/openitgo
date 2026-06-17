use rust_reader_core::models::Comic;

pub fn thumbnail_bar(
    ui: &mut egui::Ui,
    comic: &Comic,
    current_page: usize,
    on_select: &mut dyn FnMut(usize),
) {
    let total = comic.volumes.first().map(|v| v.pages.len()).unwrap_or(0);
    if total == 0 {
        return;
    }
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for idx in 0..total {
                let selected = idx == current_page;
                let label = format!("{}", idx + 1);
                if ui.selectable_label(selected, label).clicked() {
                    on_select(idx);
                }
            }
        });
    });
}
