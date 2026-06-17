use rust_reader_storage::models::Library;

pub struct LibraryView {
    pub library: Library,
}

#[allow(clippy::derivable_impls)]
impl Default for LibraryView {
    fn default() -> Self {
        Self {
            library: Library::default(),
        }
    }
}

impl LibraryView {
    pub fn ui(&mut self, ui: &mut egui::Ui, on_open: &mut dyn FnMut(usize)) {
        ui.heading("书架");
        if self.library.entries.is_empty() {
            ui.label("暂无漫画，请点击“打开”按钮添加。");
            return;
        }
        egui::Grid::new("library_grid").show(ui, |ui| {
            for (idx, entry) in self.library.entries.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.label(&entry.title);
                    if ui.button("打开").clicked() {
                        on_open(idx);
                    }
                });
                ui.end_row();
            }
        });
    }
}
