use rust_reader_storage::models::{Library, LibraryEntry};

#[derive(Default)]
pub struct LibraryView {
    pub library: Library,
}

impl LibraryView {
    pub fn entry_at(&self, idx: usize) -> Option<&LibraryEntry> {
        self.library.entries.get(idx)
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        on_open: &mut dyn FnMut(usize),
        on_add: &mut dyn FnMut(),
    ) {
        ui.horizontal(|ui| {
            ui.heading("书架");
            if ui.button("打开文件夹").clicked() {
                on_add();
            }
        });
        if self.library.entries.is_empty() {
            ui.label("暂无漫画，请点击“打开文件夹”按钮添加。");
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
