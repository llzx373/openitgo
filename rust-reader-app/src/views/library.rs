use rust_reader_storage::models::{
    Bookmarks, History, Library, LibraryEntry, LibrarySort, MediaType,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    #[default]
    Library,
    Ebooks,
    History,
    Bookmarks,
}

pub struct LibraryView {
    pub library: Library,
    pub mode: LibraryMode,
    pub search_query: String,
    edit_buffer: Option<(usize, String)>,
    pending_delete: Option<usize>,
    cover_textures: HashMap<String, egui::TextureHandle>,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            library: Library::default(),
            mode: LibraryMode::Library,
            search_query: String::new(),
            edit_buffer: None,
            pending_delete: None,
            cover_textures: HashMap::new(),
        }
    }
}

pub struct LibraryCallbacks<'a> {
    pub on_open_library: &'a mut dyn FnMut(usize),
    pub on_open_path: &'a mut dyn FnMut(PathBuf),
    pub on_add: &'a mut dyn FnMut(),
    pub on_request_cover: &'a mut dyn FnMut(usize),
    pub on_remove_missing: &'a mut dyn FnMut(),
    pub on_delete_bookmark: &'a mut dyn FnMut(usize),
    pub on_update_bookmark: &'a mut dyn FnMut(usize, Option<String>),
    pub on_update_title: &'a mut dyn FnMut(usize, String),
    pub on_delete_library: &'a mut dyn FnMut(usize),
    pub on_clear_history: &'a mut dyn FnMut(),
    pub on_delete_history: &'a mut dyn FnMut(usize),
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
                .selectable_label(self.mode == LibraryMode::Ebooks, "电子书")
                .clicked()
            {
                self.mode = LibraryMode::Ebooks;
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
            if matches!(self.mode, LibraryMode::Library | LibraryMode::Ebooks) {
                ui.separator();
                let hint = if self.mode == LibraryMode::Ebooks {
                    "搜索电子书"
                } else {
                    "搜索漫画"
                };
                ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text(hint));
                egui::ComboBox::from_id_salt("library_sort")
                    .selected_text(sort_label(*library_sort))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(library_sort, LibrarySort::LastRead, "最近阅读");
                        ui.selectable_value(library_sort, LibrarySort::Title, "标题");
                        ui.selectable_value(library_sort, LibrarySort::Added, "添加时间");
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if matches!(self.mode, LibraryMode::Library | LibraryMode::Ebooks)
                    && ui
                        .button("清理已删除")
                        .on_hover_text("移除书库中文件已经不存在的条目")
                        .clicked()
                {
                    (callbacks.on_remove_missing)();
                }
                if ui.button("打开文件夹").clicked() {
                    (callbacks.on_add)();
                }
            });
        });

        match self.mode {
            LibraryMode::Library | LibraryMode::Ebooks => {
                self.render_library(ui, history, *library_sort, callbacks)
            }
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
            self.render_empty_library(ui, callbacks);
            return;
        }
        const CARD_WIDTH: f32 = 140.0;
        const CARD_HEIGHT: f32 = 260.0;
        const COVER_WIDTH: f32 = 120.0;
        const COVER_HEIGHT: f32 = 170.0;
        let cover_size = egui::vec2(COVER_WIDTH, COVER_HEIGHT);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(16.0, 16.0);
                for (original_idx, entry) in entries {
                    let is_editing = self.edit_buffer.as_ref().map(|b| b.0) == Some(original_idx);
                    let file_missing = !entry.path.exists();
                    let cover_missing = entry
                        .cover_path
                        .as_ref()
                        .map(|p| !p.exists())
                        .unwrap_or(true);

                    if !file_missing && cover_missing {
                        (callbacks.on_request_cover)(original_idx);
                    }

                    let card_response = ui
                        .allocate_ui_with_layout(
                            egui::vec2(CARD_WIDTH, CARD_HEIGHT),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.group(|ui| {
                                    ui.set_min_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT));
                                    ui.vertical_centered(|ui| {
                                        let cover_rect = if let Some(texture) = self.cover_texture(
                                            ui.ctx(),
                                            &entry.comic_id,
                                            entry.cover_path.as_ref(),
                                        ) {
                                            let desired = texture.size_vec2();
                                            let scale = (COVER_WIDTH / desired.x)
                                                .min(COVER_HEIGHT / desired.y);
                                            let size = desired * scale;
                                            let (rect, _response) =
                                                ui.allocate_exact_size(size, egui::Sense::hover());
                                            ui.painter().image(
                                                texture.id(),
                                                rect,
                                                egui::Rect::from_min_max(
                                                    egui::pos2(0.0, 0.0),
                                                    egui::pos2(1.0, 1.0),
                                                ),
                                                egui::Color32::WHITE,
                                            );
                                            rect
                                        } else {
                                            let (rect, _response) = ui.allocate_exact_size(
                                                cover_size,
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                0.0,
                                                ui.visuals().widgets.inactive.bg_fill,
                                            );
                                            rect
                                        };

                                        if file_missing {
                                            let overlay = cover_rect.expand2(egui::vec2(4.0, 4.0));
                                            ui.painter().rect_filled(
                                                overlay,
                                                0.0,
                                                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                                            );
                                            ui.painter().text(
                                                overlay.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "已删除",
                                                egui::FontId::proportional(14.0),
                                                egui::Color32::WHITE,
                                            );
                                        }

                                        ui.add_space(4.0);

                                        if is_editing {
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
                                                    (callbacks.on_update_title)(
                                                        original_idx,
                                                        new_title,
                                                    );
                                                }
                                                self.edit_buffer = None;
                                            } else if cancel {
                                                self.edit_buffer = None;
                                            }
                                        } else {
                                            ui.label(
                                                egui::RichText::new(&entry.title)
                                                    .strong()
                                                    .text_style(egui::TextStyle::Button),
                                            );
                                        }

                                        let progress = library_progress(history, &entry.comic_id);
                                        if progress.total > 0 {
                                            ui.add(egui::ProgressBar::new(progress.ratio()).text(
                                                format!("{}/{}", progress.read, progress.total),
                                            ));
                                        }
                                    });
                                })
                                .response
                                .interact(egui::Sense::click())
                            },
                        )
                        .inner;

                    if !is_editing && card_response.clicked() {
                        (callbacks.on_open_library)(original_idx);
                    }
                    card_response.context_menu(|ui| {
                        if ui.button("打开").clicked() {
                            (callbacks.on_open_library)(original_idx);
                            ui.close_menu();
                        }
                        if ui.button("编辑标题").clicked() {
                            self.edit_buffer = Some((original_idx, entry.title.clone()));
                            self.pending_delete = None;
                            ui.close_menu();
                        }
                        if self.pending_delete == Some(original_idx) {
                            ui.label("确定删除？");
                            if ui.button("是").clicked() {
                                (callbacks.on_delete_library)(original_idx);
                                self.pending_delete = None;
                                self.edit_buffer = None;
                                ui.close_menu();
                            }
                            if ui.button("否").clicked() {
                                self.pending_delete = None;
                                ui.close_menu();
                            }
                        } else if ui.button("删除").clicked() {
                            self.pending_delete = Some(original_idx);
                            ui.close_menu();
                        }
                    });
                }
            });
        });
    }

    fn render_empty_library(&mut self, ui: &mut egui::Ui, callbacks: LibraryCallbacks<'_>) {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.label(egui::RichText::new("书架还是空的").size(20.0).strong());
            ui.add_space(8.0);
            ui.label("拖拽漫画文件/文件夹到窗口，或点击下面按钮导入。");
            ui.add_space(16.0);
            if ui.button("打开文件夹").clicked() {
                (callbacks.on_add)();
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
            .filter(|(_, e)| {
                let media_ok = match self.mode {
                    LibraryMode::Ebooks => e.media_type == MediaType::Ebook,
                    LibraryMode::Library => e.media_type == MediaType::Comic,
                    _ => true,
                };
                media_ok && (query.is_empty() || e.title.to_lowercase().contains(&query))
            })
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

    fn cover_texture(
        &mut self,
        ctx: &egui::Context,
        comic_id: &str,
        cover_path: Option<&PathBuf>,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.cover_textures.get(comic_id) {
            return Some(handle.clone());
        }
        let path = cover_path?;
        let image = image::open(path).ok()?;
        let size = [image.width() as usize, image.height() as usize];
        let rgba = image.to_rgba8().into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
        let handle = ctx.load_texture(
            format!("cover_{}", comic_id),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        self.cover_textures
            .insert(comic_id.to_string(), handle.clone());
        Some(handle)
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
        ui.horizontal(|ui| {
            ui.heading("历史");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("清空").clicked() {
                    (callbacks.on_clear_history)();
                }
            });
        });
        ui.separator();
        egui::Grid::new("history_grid").show(ui, |ui| {
            for (idx, entry) in history.entries.iter().enumerate() {
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
                if ui.button("删除").clicked() {
                    (callbacks.on_delete_history)(idx);
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
                if self.edit_buffer.as_ref().map(|b| b.0) == Some(idx) {
                    let note = &mut self.edit_buffer.as_mut().unwrap().1;
                    ui.text_edit_singleline(note);
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
                        let trimmed = note.trim().to_string();
                        (callbacks.on_update_bookmark)(
                            idx,
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed)
                            },
                        );
                        self.edit_buffer = None;
                    } else if cancel {
                        self.edit_buffer = None;
                    }
                } else if let Some(note) = &entry.note {
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
                    if ui.button("编辑").clicked() {
                        self.edit_buffer = Some((idx, entry.note.clone().unwrap_or_default()));
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

#[derive(Debug, Clone, Copy, Default)]
struct Progress {
    read: usize,
    total: usize,
}

impl Progress {
    fn ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.read as f32 / self.total as f32).min(1.0)
        }
    }
}

fn library_progress(_history: &History, _comic_id: &str) -> Progress {
    // Placeholder: the storage models don't track total pages yet, so we show
    // a 0 % bar until P3-23 adds richer metadata.
    Progress::default()
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
                media_type: MediaType::Comic,
            }],
        }
    }

    #[test]
    fn test_find_by_id_returns_entry_when_present() {
        let view = LibraryView {
            library: sample_library(),
            mode: LibraryMode::Library,
            ..Default::default()
        };
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
        let view = LibraryView {
            library: Library {
                entries: vec![
                    LibraryEntry {
                        comic_id: "a".to_string(),
                        title: "Alpha Comic".to_string(),
                        path: PathBuf::from("/a"),
                        cover_path: None,
                        added_at: 1,
                        media_type: MediaType::Comic,
                    },
                    LibraryEntry {
                        comic_id: "b".to_string(),
                        title: "Beta Comic".to_string(),
                        path: PathBuf::from("/b"),
                        cover_path: None,
                        added_at: 2,
                        media_type: MediaType::Comic,
                    },
                ],
            },
            search_query: "alpha".to_string(),
            ..Default::default()
        };
        let filtered = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1.title, "Alpha Comic");
    }

    #[test]
    fn test_sort_by_title() {
        let view = LibraryView {
            library: Library {
                entries: vec![
                    LibraryEntry {
                        comic_id: "b".to_string(),
                        title: "Beta".to_string(),
                        path: PathBuf::from("/b"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                    },
                    LibraryEntry {
                        comic_id: "a".to_string(),
                        title: "Alpha".to_string(),
                        path: PathBuf::from("/a"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                    },
                ],
            },
            ..Default::default()
        };
        let sorted = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(sorted[0].1.title, "Alpha");
        assert_eq!(sorted[1].1.title, "Beta");
    }

    #[test]
    fn test_sort_by_added_time() {
        let view = LibraryView {
            library: Library {
                entries: vec![
                    LibraryEntry {
                        comic_id: "old".to_string(),
                        title: "Old".to_string(),
                        path: PathBuf::from("/old"),
                        cover_path: None,
                        added_at: 100,
                        media_type: MediaType::Comic,
                    },
                    LibraryEntry {
                        comic_id: "new".to_string(),
                        title: "New".to_string(),
                        path: PathBuf::from("/new"),
                        cover_path: None,
                        added_at: 200,
                        media_type: MediaType::Comic,
                    },
                ],
            },
            ..Default::default()
        };
        let sorted = view.filtered_entries(&History::default(), LibrarySort::Added);
        assert_eq!(sorted[0].1.title, "New");
        assert_eq!(sorted[1].1.title, "Old");
    }

    #[test]
    fn test_sort_by_last_read_uses_history() {
        let view = LibraryView {
            library: Library {
                entries: vec![
                    LibraryEntry {
                        comic_id: "recent".to_string(),
                        title: "Recent".to_string(),
                        path: PathBuf::from("/recent"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                    },
                    LibraryEntry {
                        comic_id: "old".to_string(),
                        title: "Old Read".to_string(),
                        path: PathBuf::from("/old"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                    },
                ],
            },
            ..Default::default()
        };
        let history = History {
            entries: vec![
                HistoryEntry {
                    comic_id: "old".to_string(),
                    path: std::path::PathBuf::new(),
                    volume_index: 0,
                    page_index: 0,
                    last_read_at: 100,
                },
                HistoryEntry {
                    comic_id: "recent".to_string(),
                    path: std::path::PathBuf::new(),
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

    #[test]
    fn test_filter_by_media_type() {
        let mut view = LibraryView {
            library: Library {
                entries: vec![
                    LibraryEntry {
                        comic_id: "comic-1".to_string(),
                        title: "Comic One".to_string(),
                        path: PathBuf::from("/c1"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                    },
                    LibraryEntry {
                        comic_id: "ebook-1".to_string(),
                        title: "Ebook One".to_string(),
                        path: PathBuf::from("/e1.epub"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Ebook,
                    },
                ],
            },
            ..Default::default()
        };

        view.mode = LibraryMode::Library;
        let comics = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(comics.len(), 1);
        assert_eq!(comics[0].1.media_type, MediaType::Comic);

        view.mode = LibraryMode::Ebooks;
        let ebooks = view.filtered_entries(&History::default(), LibrarySort::Title);
        assert_eq!(ebooks.len(), 1);
        assert_eq!(ebooks[0].1.media_type, MediaType::Ebook);
    }
}
