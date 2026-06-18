use rust_reader_storage::models::{Bookmarks, History, Library, LibraryEntry, LibrarySort};
use std::path::PathBuf;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    #[default]
    Library,
    History,
    Bookmarks,
}

pub struct LibraryView {
    pub library: Library,
    pub mode: LibraryMode,
    pub search_query: String,
    edit_buffer: Option<(usize, String)>,
    pending_delete: Option<usize>,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            library: Library::default(),
            mode: LibraryMode::Library,
            search_query: String::new(),
            edit_buffer: None,
            pending_delete: None,
        }
    }
}

pub struct LibraryCallbacks<'a> {
    pub on_open_library: &'a mut dyn FnMut(usize),
    pub on_open_path: &'a mut dyn FnMut(PathBuf),
    pub on_add: &'a mut dyn FnMut(),
    pub on_delete_bookmark: &'a mut dyn FnMut(usize),
    pub on_update_title: &'a mut dyn FnMut(usize, String),
    pub on_delete_library: &'a mut dyn FnMut(usize),
}

impl LibraryView {
    pub fn entry_at(&self, idx: usize) -> Option<&LibraryEntry> {
        self.library.entries.get(idx)
    }

    pub fn find_by_id(&self, comic_id: &str) -> Option<&LibraryEntry> {
        self.library.entries.iter().find(|e| e.comic_id == comic_id)
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        history: &History,
        bookmarks: &Bookmarks,
        library_sort: &mut LibrarySort,
        callbacks: LibraryCallbacks<'_>,
    ) {
        ui.horizontal(|ui| {
            ui.heading("书架");
            ui.separator();
            if ui
                .selectable_label(self.mode == LibraryMode::Library, "漫画")
                .clicked()
            {
                self.mode = LibraryMode::Library;
            }
            if ui
                .selectable_label(self.mode == LibraryMode::History, "历史")
                .clicked()
            {
                self.mode = LibraryMode::History;
            }
            if ui
                .selectable_label(self.mode == LibraryMode::Bookmarks, "书签")
                .clicked()
            {
                self.mode = LibraryMode::Bookmarks;
            }
            if self.mode == LibraryMode::Library {
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text("搜索漫画"));
                egui::ComboBox::from_id_salt("library_sort")
                    .selected_text(sort_label(*library_sort))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(library_sort, LibrarySort::LastRead, "最近阅读");
                        ui.selectable_value(library_sort, LibrarySort::Title, "标题");
                        ui.selectable_value(library_sort, LibrarySort::Added, "添加时间");
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("打开文件夹").clicked() {
                    (callbacks.on_add)();
                }
            });
        });

        match self.mode {
            LibraryMode::Library => self.render_library(ui, history, *library_sort, callbacks),
            LibraryMode::History => self.render_history(ui, history, callbacks),
            LibraryMode::Bookmarks => self.render_bookmarks(ui, bookmarks, callbacks),
        }
    }

    fn render_library(
        &mut self,
        ui: &mut egui::Ui,
        history: &History,
        sort: LibrarySort,
        callbacks: LibraryCallbacks<'_>,
    ) {
        let entries = self.filtered_entries(history, sort);
        if entries.is_empty() {
            ui.label("没有匹配的漫画。");
            return;
        }
        egui::Grid::new("library_grid").show(ui, |ui| {
            for (original_idx, entry) in entries {
                ui.vertical(|ui| {
                    if self.edit_buffer.as_ref().map(|b| b.0) == Some(original_idx) {
                        let title = &mut self.edit_buffer.as_mut().unwrap().1;
                        ui.text_edit_singleline(title);
                        let mut save = false;
                        let mut cancel = false;
                        ui.horizontal(|ui| {
                            if ui.button("保存").clicked() {
                                save = true;
                            }
                            if ui.button("取消").clicked() {
                                cancel = true;
                            }
                        });
                        if save {
                            let new_title = title.trim().to_string();
                            if !new_title.is_empty() {
                                (callbacks.on_update_title)(original_idx, new_title);
                            }
                            self.edit_buffer = None;
                        } else if cancel {
                            self.edit_buffer = None;
                        }
                    } else {
                        ui.label(&entry.title);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("打开").clicked() {
                            (callbacks.on_open_library)(original_idx);
                        }
                        if ui.button("编辑").clicked() {
                            self.edit_buffer = Some((original_idx, entry.title.clone()));
                            self.pending_delete = None;
                        }
                        if self.pending_delete == Some(original_idx) {
                            ui.label("确定删除？");
                            if ui.button("是").clicked() {
                                (callbacks.on_delete_library)(original_idx);
                                self.pending_delete = None;
                                self.edit_buffer = None;
                            }
                            if ui.button("否").clicked() {
                                self.pending_delete = None;
                            }
                        } else if ui.button("删除").clicked() {
                            self.pending_delete = Some(original_idx);
                            self.edit_buffer = None;
                        }
                    });
                });
                ui.end_row();
            }
        });
    }

    fn filtered_entries(&self, history: &History, sort: LibrarySort) -> Vec<(usize, LibraryEntry)> {
        let query = self.search_query.trim().to_lowercase();
        let mut entries: Vec<(usize, LibraryEntry)> = self
            .library
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| query.is_empty() || e.title.to_lowercase().contains(&query))
            .map(|(i, e)| (i, e.clone()))
            .collect();

        entries.sort_by(|(_, a), (_, b)| match sort {
            LibrarySort::Title => a.title.cmp(&b.title),
            LibrarySort::Added => b.added_at.cmp(&a.added_at),
            LibrarySort::LastRead => {
                let last_a = last_read_at(history, &a.comic_id);
                let last_b = last_read_at(history, &b.comic_id);
                last_b.cmp(&last_a)
            }
        });

        entries
    }

    fn render_history(
        &mut self,
        ui: &mut egui::Ui,
        history: &History,
        callbacks: LibraryCallbacks<'_>,
    ) {
        if history.entries.is_empty() {
            ui.label("暂无阅读历史。");
            return;
        }
        egui::Grid::new("history_grid").show(ui, |ui| {
            for entry in history.entries.iter() {
                let (title, path) = match self.find_by_id(&entry.comic_id) {
                    Some(lib) => (lib.title.clone(), Some(lib.path.clone())),
                    None => (entry.comic_id.clone(), None),
                };
                ui.label(&title);
                ui.label(format!("第 {} 页", entry.page_index + 1));
                ui.label(format_timestamp(entry.last_read_at));
                if let Some(path) = path {
                    if ui.button("继续阅读").clicked() {
                        (callbacks.on_open_path)(path);
                    }
                } else {
                    ui.label("未在书架中");
                }
                ui.end_row();
            }
        });
    }

    fn render_bookmarks(
        &mut self,
        ui: &mut egui::Ui,
        bookmarks: &Bookmarks,
        callbacks: LibraryCallbacks<'_>,
    ) {
        if bookmarks.entries.is_empty() {
            ui.label("暂无书签。");
            return;
        }
        egui::Grid::new("bookmarks_grid").show(ui, |ui| {
            for (idx, entry) in bookmarks.entries.iter().enumerate() {
                let (title, path) = match self.find_by_id(&entry.comic_id) {
                    Some(lib) => (lib.title.clone(), Some(lib.path.clone())),
                    None => (entry.comic_id.clone(), None),
                };
                ui.label(&title);
                ui.label(format!("第 {} 页", entry.page_index + 1));
                if let Some(note) = &entry.note {
                    ui.label(note);
                } else {
                    ui.label("-");
                }
                ui.horizontal(|ui| {
                    if let Some(path) = path {
                        if ui.button("打开").clicked() {
                            (callbacks.on_open_path)(path);
                        }
                    }
                    if ui.button("删除").clicked() {
                        (callbacks.on_delete_bookmark)(idx);
                    }
                });
                ui.end_row();
            }
        });
    }
}

fn last_read_at(history: &History, comic_id: &str) -> u64 {
    history
        .entries
        .iter()
        .filter(|h| h.comic_id == comic_id)
        .map(|h| h.last_read_at)
        .max()
        .unwrap_or(0)
}

fn sort_label(sort: LibrarySort) -> &'static str {
    match sort {
        LibrarySort::LastRead => "最近阅读",
        LibrarySort::Title => "标题",
        LibrarySort::Added => "添加时间",
    }
}

fn format_timestamp(ts: u64) -> String {
    if ts == 0 {
        return "-".to_string();
    }
    let Some(dt) = time::OffsetDateTime::from_unix_timestamp(ts as i64).ok() else {
        return "-".to_string();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_storage::models::HistoryEntry;

    use std::path::PathBuf;

    fn sample_library() -> Library {
        Library {
            entries: vec![LibraryEntry {
                comic_id: "comic-1".to_string(),
                title: "Test Comic".to_string(),
                path: PathBuf::from("/tmp/comic-1"),
                cover_path: None,
                added_at: 0,
            }],
        }
    }

    #[test]
    fn test_find_by_id_returns_entry_when_present() {
        let mut view = LibraryView::default();
        view.library = sample_library();
        view.mode = LibraryMode::Library;
        assert!(view.find_by_id("comic-1").is_some());
        assert!(view.find_by_id("missing").is_none());
    }

    #[test]
    fn test_format_timestamp_formats_unix_time() {
        // 2024-01-02 03:04:00 UTC
        let ts = 1704164640;
        assert_eq!(format_timestamp(ts), "2024-01-02 03:04");
    }

    #[test]
    fn test_format_timestamp_returns_dash_for_zero() {
        assert_eq!(format_timestamp(0), "-");
    }

    #[test]
    fn test_search_filters_by_title_case_insensitive() {
        let mut view = LibraryView::default();
        view.library = Library {
            entries: vec![
                LibraryEntry {
                    comic_id: "a".to_string(),
                    title: "Alpha Comic".to_string(),
                    path: PathBuf::from("/a"),
                    cover_path: None,
                    added_at: 1,
                },
                LibraryEntry {
                    comic_id: "b".to_string(),
                    title: "Beta Comic".to_string(),
                    path: PathBuf::from("/b"),
                    cover_path: None,
                    added_at: 2,
                },
            ],
        };
        view.search_query = "alpha".to_string();
        let filtered = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1.title, "Alpha Comic");
    }

    #[test]
    fn test_sort_by_title() {
        let mut view = LibraryView::default();
        view.library = Library {
            entries: vec![
                LibraryEntry {
                    comic_id: "b".to_string(),
                    title: "Beta".to_string(),
                    path: PathBuf::from("/b"),
                    cover_path: None,
                    added_at: 0,
                },
                LibraryEntry {
                    comic_id: "a".to_string(),
                    title: "Alpha".to_string(),
                    path: PathBuf::from("/a"),
                    cover_path: None,
                    added_at: 0,
                },
            ],
        };
        let sorted = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(sorted[0].1.title, "Alpha");
        assert_eq!(sorted[1].1.title, "Beta");
    }

    #[test]
    fn test_sort_by_added_time() {
        let mut view = LibraryView::default();
        view.library = Library {
            entries: vec![
                LibraryEntry {
                    comic_id: "old".to_string(),
                    title: "Old".to_string(),
                    path: PathBuf::from("/old"),
                    cover_path: None,
                    added_at: 100,
                },
                LibraryEntry {
                    comic_id: "new".to_string(),
                    title: "New".to_string(),
                    path: PathBuf::from("/new"),
                    cover_path: None,
                    added_at: 200,
                },
            ],
        };
        let sorted = view.filtered_entries(&History::default(), LibrarySort::Added);
        assert_eq!(sorted[0].1.title, "New");
        assert_eq!(sorted[1].1.title, "Old");
    }

    #[test]
    fn test_sort_by_last_read_uses_history() {
        let mut view = LibraryView::default();
        view.library = Library {
            entries: vec![
                LibraryEntry {
                    comic_id: "recent".to_string(),
                    title: "Recent".to_string(),
                    path: PathBuf::from("/recent"),
                    cover_path: None,
                    added_at: 0,
                },
                LibraryEntry {
                    comic_id: "old".to_string(),
                    title: "Old Read".to_string(),
                    path: PathBuf::from("/old"),
                    cover_path: None,
                    added_at: 0,
                },
            ],
        };
        let history = History {
            entries: vec![
                HistoryEntry {
                    comic_id: "old".to_string(),
                    volume_index: 0,
                    page_index: 0,
                    last_read_at: 100,
                },
                HistoryEntry {
                    comic_id: "recent".to_string(),
                    volume_index: 0,
                    page_index: 0,
                    last_read_at: 300,
                },
            ],
        };
        let sorted = view.filtered_entries(&history, LibrarySort::LastRead);
        assert_eq!(sorted[0].1.comic_id, "recent");
        assert_eq!(sorted[1].1.comic_id, "old");
    }
}
