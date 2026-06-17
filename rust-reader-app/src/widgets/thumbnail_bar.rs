use rust_reader_core::models::Comic;

pub fn thumbnail_bar(
    ui: &mut egui::Ui,
    // ctx is reserved for future thumbnail texture loading
    ctx: &egui::Context,
    comic: &Comic,
    current_page: usize,
    on_select: &mut dyn FnMut(usize),
) {
    let _ = ctx;
    ui.horizontal(|ui| {
        for (idx, _page) in comic.volumes[0].pages.iter().enumerate() {
            let selected = idx == current_page;
            let label = format!("{}", idx + 1);
            if ui.selectable_label(selected, label).clicked() {
                on_select(idx);
            }
        }
    });
}
