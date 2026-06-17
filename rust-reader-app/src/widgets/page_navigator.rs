use rust_reader_core::models::Comic;

pub fn page_navigator(
    ui: &mut egui::Ui,
    comic: &Comic,
    current_page: usize,
    on_select: &mut dyn FnMut(usize),
) {
    if comic.volumes.is_empty() {
        return;
    }

    ui.horizontal(|ui| {
        for (idx, _page) in comic.volumes[0].pages.iter().enumerate() {
            let selected = idx == current_page;
            let label = (idx + 1).to_string();
            if ui.selectable_label(selected, label).clicked() {
                on_select(idx);
            }
        }
    });
}
