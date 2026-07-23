use openitgo_storage::models::{Bookmarks, History, Library, LibraryEntry, LibrarySort, MediaType};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    #[default]
    Library,
    Ebooks,
    History,
    Bookmarks,
    Stats,
}

pub struct LibraryView {
    pub library: Library,
    pub mode: LibraryMode,
    pub search_query: String,
    edit_buffer: Option<(usize, String)>,
    pending_delete: Option<usize>,
    cover_textures: HashMap<String, egui::TextureHandle>,
    /// 标签过滤 chips 的当前选择（None = 全部）。
    pub tag_filter: Option<String>,
    /// 标签编辑对话框状态：(条目索引, 输入缓冲)。
    tag_edit_buffer: Option<(usize, String)>,
    /// 封面根目录（书签缩略图在 covers/bookmarks/ 下）；测试可为 None。
    pub covers_dir: Option<PathBuf>,
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
            tag_filter: None,
            tag_edit_buffer: None,
            covers_dir: None,
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
    pub on_update_tags: &'a mut dyn FnMut(usize, Vec<String>),
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
        stats: &HashMap<String, openitgo_storage::models::ReadingStat>,
        callbacks: LibraryCallbacks<'_>,
    ) {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("书架").size(22.0).strong());
            ui.add_space(12.0);
            crate::theme::segmented_tabs(
                ui,
                &[
                    (LibraryMode::Library, "漫画"),
                    (LibraryMode::Ebooks, "电子书"),
                    (LibraryMode::History, "历史"),
                    (LibraryMode::Bookmarks, "书签"),
                    (LibraryMode::Stats, "统计"),
                ],
                &mut self.mode,
            );
            if matches!(self.mode, LibraryMode::Library | LibraryMode::Ebooks) {
                ui.add_space(8.0);
                let hint = if self.mode == LibraryMode::Ebooks {
                    "搜索电子书"
                } else {
                    "搜索漫画"
                };
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text(hint)
                        .desired_width(180.0),
                );
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
                if ui
                    .button(format!(
                        "{} 打开文件夹",
                        egui_phosphor_icons::icons::FOLDER_PLUS.as_str()
                    ))
                    .clicked()
                {
                    (callbacks.on_add)();
                }
            });
        });

        if matches!(self.mode, LibraryMode::Library | LibraryMode::Ebooks) {
            let tags = collect_unique_tags(&self.library.entries);
            if !tags.is_empty() {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("标签").weak().size(12.5));
                    if crate::theme::tag_chip(ui, "全部", self.tag_filter.is_none()).clicked() {
                        self.tag_filter = None;
                    }
                    for tag in tags {
                        let selected = self.tag_filter.as_deref() == Some(tag.as_str());
                        if crate::theme::tag_chip(ui, &tag, selected).clicked() {
                            self.tag_filter = if selected { None } else { Some(tag.clone()) };
                        }
                    }
                });
                ui.add_space(4.0);
            }
        }

        // Soft dashed rule between header chrome and content grid.
        crate::theme::dashed_separator(ui);

        if let Some((idx, buffer)) = self.tag_edit_buffer.as_mut() {
            let idx = *idx;
            let mut save = false;
            let mut cancel = false;
            egui::Window::new("编辑标签")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.label("多个标签用逗号分隔：");
                    ui.add(egui::TextEdit::singleline(buffer).desired_width(280.0));
                    ui.horizontal(|ui| {
                        if ui.button("保存").clicked() {
                            save = true;
                        }
                        if ui.button("取消").clicked() {
                            cancel = true;
                        }
                    });
                });
            if save {
                (callbacks.on_update_tags)(idx, parse_tags_input(buffer));
                self.tag_edit_buffer = None;
            } else if cancel {
                self.tag_edit_buffer = None;
            }
        }

        match self.mode {
            LibraryMode::Library | LibraryMode::Ebooks => {
                self.render_library(ui, history, *library_sort, callbacks)
            }
            LibraryMode::History => self.render_history(ui, history, callbacks),
            LibraryMode::Bookmarks => self.render_bookmarks(ui, bookmarks, callbacks),
            LibraryMode::Stats => self.render_stats(ui, stats),
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
        const CARD_WIDTH: f32 = 148.0;
        const CARD_HEIGHT: f32 = 268.0;
        const COVER_WIDTH: f32 = 132.0;
        const COVER_HEIGHT: f32 = 186.0;
        let cover_size = egui::vec2(COVER_WIDTH, COVER_HEIGHT);
        let cover_radius = egui::CornerRadius::same(crate::theme::RADIUS_COVER);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(18.0, 20.0);
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
                                ui.set_min_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT));
                                ui.vertical_centered(|ui| {
                                    let cover_rect = {
                                        let (slot, cover_sense) = ui
                                            .allocate_exact_size(cover_size, egui::Sense::hover());
                                        ui.painter().rect_filled(
                                            slot,
                                            cover_radius,
                                            ui.visuals().extreme_bg_color,
                                        );
                                        if let Some(texture) = self.cover_texture(
                                            ui.ctx(),
                                            &entry.comic_id,
                                            entry.cover_path.as_ref(),
                                        ) {
                                            let desired = texture.size_vec2();
                                            let scale = (COVER_WIDTH / desired.x)
                                                .min(COVER_HEIGHT / desired.y);
                                            let size = desired * scale;
                                            let rect =
                                                egui::Rect::from_center_size(slot.center(), size);
                                            egui::Image::new(&texture)
                                                .fit_to_exact_size(size)
                                                .corner_radius(cover_radius)
                                                .paint_at(ui, rect);
                                        } else {
                                            let placeholder_color =
                                                if entry.media_type == MediaType::Ebook {
                                                    egui::Color32::from_rgb(60, 72, 96)
                                                } else {
                                                    ui.visuals().widgets.inactive.bg_fill
                                                };
                                            ui.painter().rect_filled(
                                                slot,
                                                cover_radius,
                                                placeholder_color,
                                            );
                                            let placeholder_label = match entry.media_type {
                                                MediaType::Ebook => "电子书",
                                                MediaType::Comic => "漫画",
                                                MediaType::Video => "视频",
                                                MediaType::Audio => "音频",
                                            };
                                            ui.painter().text(
                                                slot.center(),
                                                egui::Align2::CENTER_CENTER,
                                                placeholder_label,
                                                egui::FontId::proportional(15.0),
                                                egui::Color32::from_rgba_unmultiplied(
                                                    255, 255, 255, 200,
                                                ),
                                            );
                                        }
                                        if cover_sense.hovered() && !file_missing {
                                            // Subtle lift: warm accent rim, no shadow stack.
                                            let accent =
                                                crate::theme::accent_for(ui.visuals().dark_mode);
                                            ui.painter().rect_stroke(
                                                slot,
                                                cover_radius,
                                                egui::Stroke::new(1.0, accent),
                                                egui::StrokeKind::Outside,
                                            );
                                            ui.painter().rect_filled(
                                                slot,
                                                cover_radius,
                                                egui::Color32::from_rgba_unmultiplied(
                                                    255, 255, 255, 18,
                                                ),
                                            );
                                        }
                                        slot
                                    };

                                    if file_missing {
                                        let overlay = cover_rect;
                                        ui.painter().rect_filled(
                                            overlay,
                                            cover_radius,
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

                                    ui.add_space(8.0);

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
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&entry.title)
                                                    .strong()
                                                    .size(13.0),
                                            )
                                            .truncate(),
                                        );
                                    }

                                    let progress = library_progress(history, &entry);
                                    if progress.total > 0 {
                                        ui.add_space(4.0);
                                        let bar_w = COVER_WIDTH;
                                        let (bar_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(bar_w, 4.0),
                                            egui::Sense::hover(),
                                        );
                                        let rounding = egui::CornerRadius::same(2);
                                        ui.painter().rect_filled(
                                            bar_rect,
                                            rounding,
                                            ui.visuals().extreme_bg_color,
                                        );
                                        let fill_w = bar_rect.width() * progress.ratio();
                                        if fill_w > 0.0 {
                                            let fill = egui::Rect::from_min_size(
                                                bar_rect.min,
                                                egui::vec2(fill_w, bar_rect.height()),
                                            );
                                            ui.painter().rect_filled(
                                                fill,
                                                rounding,
                                                ui.visuals().selection.stroke.color,
                                            );
                                        }
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}/{}",
                                                progress.read, progress.total
                                            ))
                                            .size(11.0)
                                            .weak(),
                                        );
                                    }
                                });
                                ui.interact(
                                    ui.min_rect(),
                                    ui.id().with(("lib_card", original_idx)),
                                    egui::Sense::click(),
                                )
                            },
                        )
                        .inner;

                    if !is_editing && card_response.clicked() {
                        (callbacks.on_open_library)(original_idx);
                    }
                    card_response.context_menu(|ui| {
                        if ui.button("打开").clicked() {
                            (callbacks.on_open_library)(original_idx);
                            ui.close();
                        }
                        if ui.button("编辑标题").clicked() {
                            self.edit_buffer = Some((original_idx, entry.title.clone()));
                            self.pending_delete = None;
                            ui.close();
                        }
                        if ui.button("编辑标签…").clicked() {
                            self.tag_edit_buffer = Some((original_idx, entry.tags.join(", ")));
                            self.pending_delete = None;
                            self.edit_buffer = None;
                            ui.close();
                        }
                        if self.pending_delete == Some(original_idx) {
                            ui.label("确定删除？");
                            if ui.button("是").clicked() {
                                (callbacks.on_delete_library)(original_idx);
                                self.pending_delete = None;
                                self.edit_buffer = None;
                                ui.close();
                            }
                            if ui.button("否").clicked() {
                                self.pending_delete = None;
                                ui.close();
                            }
                        } else if ui.button("删除").clicked() {
                            self.pending_delete = Some(original_idx);
                            ui.close();
                        }
                    });
                }
            });
        });
    }

    fn render_empty_library(&mut self, ui: &mut egui::Ui, callbacks: LibraryCallbacks<'_>) {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(
                egui::RichText::new(egui_phosphor_icons::icons::BOOKS.as_str())
                    .size(48.0)
                    .color(crate::theme::accent_for(ui.visuals().dark_mode)),
            );
            ui.add_space(12.0);
            ui.label(egui::RichText::new("书架还是空的").size(22.0).strong());
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("拖拽漫画、电子书或影音文件到窗口，或点击下方按钮导入。")
                    .weak()
                    .size(14.0),
            );
            ui.add_space(20.0);
            if ui
                .add_sized(
                    egui::vec2(160.0, 32.0),
                    egui::Button::new(format!(
                        "{} 打开文件夹",
                        egui_phosphor_icons::icons::FOLDER_PLUS.as_str()
                    )),
                )
                .clicked()
            {
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
                    LibraryMode::Library => matches!(
                        e.media_type,
                        MediaType::Comic | MediaType::Video | MediaType::Audio
                    ),
                    _ => true,
                };
                media_ok
                    && entry_matches_tag_filter(e, &self.tag_filter)
                    && entry_matches_query(e, &query)
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

    /// 书签行缩略图纹理：书签页缩略图 → 封面 → None（调用方画占位色块）。
    /// 纹理缓存 key 用 `bm_<comic_id>_<page>` 命名空间，避免与封面冲突。
    fn bookmark_thumb_texture(
        &mut self,
        ctx: &egui::Context,
        comic_id: &str,
        page_index: usize,
        cover_path: Option<&PathBuf>,
    ) -> Option<egui::TextureHandle> {
        let key = format!("bm_{}_{}", comic_id, page_index);
        if let Some(handle) = self.cover_textures.get(&key) {
            return Some(handle.clone());
        }
        if let Some(dir) = &self.covers_dir {
            let path = dir
                .join("bookmarks")
                .join(format!("{}-p{}.jpg", comic_id, page_index));
            if path.exists() {
                let image = image::open(&path).ok()?;
                let size = [image.width() as usize, image.height() as usize];
                let rgba = image.to_rgba8().into_raw();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                let handle =
                    ctx.load_texture(key.clone(), color_image, egui::TextureOptions::LINEAR);
                self.cover_textures.insert(key, handle.clone());
                return Some(handle);
            }
        }
        self.cover_texture(ctx, comic_id, cover_path)
    }

    fn render_history(
        &mut self,
        ui: &mut egui::Ui,
        history: &History,
        callbacks: LibraryCallbacks<'_>,
    ) {
        if history.entries.is_empty() {
            render_empty_placeholder(
                ui,
                egui_phosphor_icons::icons::CLOCK_COUNTER_CLOCKWISE.as_str(),
                "暂无阅读历史",
                "打开漫画、电子书或影音后会自动记录到这里。",
            );
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
            render_empty_placeholder(
                ui,
                egui_phosphor_icons::icons::BOOKMARK.as_str(),
                "暂无书签",
                "阅读时添加书签，方便下次回到同一位置。",
            );
            return;
        }
        egui::Grid::new("bookmarks_grid").show(ui, |ui| {
            for (idx, entry) in bookmarks.entries.iter().enumerate() {
                let (title, path, cover_path) = match self.find_by_id(&entry.comic_id) {
                    Some(lib) => (
                        lib.title.clone(),
                        Some(lib.path.clone()),
                        lib.cover_path.clone(),
                    ),
                    None => (entry.comic_id.clone(), None, None),
                };
                let ctx = ui.ctx().clone();
                match self.bookmark_thumb_texture(
                    &ctx,
                    &entry.comic_id,
                    entry.page_index,
                    cover_path.as_ref(),
                ) {
                    Some(texture) => {
                        ui.add(
                            egui::Image::new(&texture).fit_to_exact_size(egui::vec2(40.0, 60.0)),
                        );
                    }
                    None => {
                        // 无缩略图且无封面：占位色块
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(40.0, 60.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
                    }
                }
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

    /// 阅读统计：总时长、条目数、每书时长排行（附标题，按时长降序）。
    fn render_stats(
        &mut self,
        ui: &mut egui::Ui,
        stats: &HashMap<String, openitgo_storage::models::ReadingStat>,
    ) {
        if stats.is_empty() {
            ui.label("暂无阅读统计。阅读漫画、电子书或媒体时会自动累计时长。");
            return;
        }
        let total_seconds: u64 = stats.values().map(|s| s.total_seconds).sum();
        ui.label(format!(
            "共 {} 本读物，累计阅读 {}",
            stats.len(),
            openitgo_storage::models::format_reading_duration(total_seconds)
        ));
        ui.separator();
        let mut rows: Vec<(&String, &openitgo_storage::models::ReadingStat)> =
            stats.iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.total_seconds));
        egui::Grid::new("reading_stats_grid")
            .striped(true)
            .show(ui, |ui| {
                for (comic_id, stat) in rows {
                    let title = self
                        .find_by_id(comic_id)
                        .map(|e| e.title.clone())
                        .unwrap_or_else(|| comic_id.clone());
                    ui.label(title);
                    ui.label(openitgo_storage::models::format_reading_duration(
                        stat.total_seconds,
                    ));
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

/// 书架进度：`read = max(history.page_index) + 1`（1-based 已读页），
/// `total = entry.page_count`；无有效总页数时返回 0%（不显示进度条）。
fn library_progress(history: &History, entry: &LibraryEntry) -> Progress {
    let Some(total) = entry.page_count.filter(|t| *t > 0) else {
        return Progress::default();
    };
    let read = history
        .entries
        .iter()
        .filter(|h| h.comic_id == entry.comic_id)
        .map(|h| h.page_index.saturating_add(1))
        .max()
        .unwrap_or(0);
    Progress {
        read: read.min(total),
        total,
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

fn render_empty_placeholder(ui: &mut egui::Ui, icon: &str, title: &str, subtitle: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(64.0);
        ui.label(
            egui::RichText::new(icon)
                .size(40.0)
                .color(crate::theme::accent_for(ui.visuals().dark_mode)),
        );
        ui.add_space(10.0);
        ui.label(egui::RichText::new(title).size(20.0).strong());
        ui.add_space(6.0);
        ui.label(egui::RichText::new(subtitle).weak().size(13.5));
    });
}

fn sort_label(sort: LibrarySort) -> &'static str {
    match sort {
        LibrarySort::LastRead => "最近阅读",
        LibrarySort::Title => "标题",
        LibrarySort::Added => "添加时间",
    }
}

/// 解析逗号分隔的标签输入（中英文逗号均可）：去空白、去重、去空项。
pub fn parse_tags_input(input: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    input
        .split([',', '，'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// 全库去重排序后的标签集合（chips 行数据源）。
pub fn collect_unique_tags(entries: &[LibraryEntry]) -> Vec<String> {
    let mut tags: Vec<String> = entries
        .iter()
        .flat_map(|e| e.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// 搜索匹配：标题或任一标签包含查询串（大小写不敏感；空串恒真）。
pub fn entry_matches_query(entry: &LibraryEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    entry.title.to_lowercase().contains(query)
        || entry.tags.iter().any(|t| t.to_lowercase().contains(query))
}

/// 标签过滤：None 不过滤；Some(tag) 要求条目含该标签。
pub fn entry_matches_tag_filter(entry: &LibraryEntry, filter: &Option<String>) -> bool {
    match filter {
        None => true,
        Some(tag) => entry.tags.iter().any(|t| t == tag),
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
    use openitgo_storage::models::HistoryEntry;

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
                tags: Vec::new(),
                page_count: None,
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
                        tags: Vec::new(),
                        page_count: None,
                    },
                    LibraryEntry {
                        comic_id: "b".to_string(),
                        title: "Beta Comic".to_string(),
                        path: PathBuf::from("/b"),
                        cover_path: None,
                        added_at: 2,
                        media_type: MediaType::Comic,
                        tags: Vec::new(),
                        page_count: None,
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
                        tags: Vec::new(),
                        page_count: None,
                    },
                    LibraryEntry {
                        comic_id: "a".to_string(),
                        title: "Alpha".to_string(),
                        path: PathBuf::from("/a"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                        tags: Vec::new(),
                        page_count: None,
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
                        tags: Vec::new(),
                        page_count: None,
                    },
                    LibraryEntry {
                        comic_id: "new".to_string(),
                        title: "New".to_string(),
                        path: PathBuf::from("/new"),
                        cover_path: None,
                        added_at: 200,
                        media_type: MediaType::Comic,
                        tags: Vec::new(),
                        page_count: None,
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
                        tags: Vec::new(),
                        page_count: None,
                    },
                    LibraryEntry {
                        comic_id: "old".to_string(),
                        title: "Old Read".to_string(),
                        path: PathBuf::from("/old"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Comic,
                        tags: Vec::new(),
                        page_count: None,
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
                    char_offset: None,
                    last_read_at: 100,
                },
                HistoryEntry {
                    comic_id: "recent".to_string(),
                    path: std::path::PathBuf::new(),
                    volume_index: 0,
                    page_index: 0,
                    char_offset: None,
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
                        tags: Vec::new(),
                        page_count: None,
                    },
                    LibraryEntry {
                        comic_id: "ebook-1".to_string(),
                        title: "Ebook One".to_string(),
                        path: PathBuf::from("/e1.epub"),
                        cover_path: None,
                        added_at: 0,
                        media_type: MediaType::Ebook,
                        tags: Vec::new(),
                        page_count: None,
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

    #[test]
    fn test_library_mode_includes_media_entries() {
        let entry = |id: &str, media_type: MediaType| LibraryEntry {
            comic_id: id.to_string(),
            title: id.to_string(),
            path: PathBuf::from(format!("/{id}")),
            cover_path: None,
            added_at: 0,
            media_type,
            tags: Vec::new(),
            page_count: None,
        };
        let library = Library {
            entries: vec![
                entry("comic", MediaType::Comic),
                entry("video", MediaType::Video),
                entry("audio", MediaType::Audio),
                entry("ebook", MediaType::Ebook),
            ],
        };
        let view = LibraryView {
            library,
            mode: LibraryMode::Library,
            ..Default::default()
        };
        let filtered = view.filtered_entries(&History::default(), LibrarySort::Title);
        let ids: Vec<&str> = filtered.iter().map(|(_, e)| e.comic_id.as_str()).collect();
        assert_eq!(ids, ["audio", "comic", "video"]);
    }

    #[test]
    fn test_parse_tags_input_splits_trims_dedupes() {
        assert_eq!(
            parse_tags_input("热血, 连载中 ,热血，完结"),
            vec!["热血", "连载中", "完结"]
        );
        assert!(parse_tags_input(" , ,").is_empty());
        assert!(parse_tags_input("").is_empty());
    }

    #[test]
    fn test_collect_unique_tags_sorted_deduped() {
        let entries = vec![
            LibraryEntry {
                tags: vec!["b".to_string(), "a".to_string()],
                ..Default::default()
            },
            LibraryEntry {
                tags: vec!["a".to_string(), "c".to_string()],
                ..Default::default()
            },
        ];
        assert_eq!(collect_unique_tags(&entries), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_entry_matches_query_matches_title_or_tag() {
        let entry = LibraryEntry {
            title: "One Piece".to_string(),
            tags: vec!["热血".to_string()],
            ..Default::default()
        };
        assert!(entry_matches_query(&entry, "piece"));
        assert!(entry_matches_query(&entry, "热血"));
        assert!(entry_matches_query(&entry, ""));
        assert!(!entry_matches_query(&entry, "火影"));
    }

    #[test]
    fn test_entry_matches_tag_filter() {
        let entry = LibraryEntry {
            tags: vec!["热血".to_string()],
            ..Default::default()
        };
        assert!(entry_matches_tag_filter(&entry, &None));
        assert!(entry_matches_tag_filter(&entry, &Some("热血".to_string())));
        assert!(!entry_matches_tag_filter(&entry, &Some("完结".to_string())));
    }

    /// 公式：`read = max(page_index)+1`，`ratio = read / page_count`。
    /// page_index=2、page_count=10 → read=3 → 0.3。
    #[test]
    fn library_progress_uses_page_count_and_history() {
        let entry = LibraryEntry {
            comic_id: "c1".to_string(),
            page_count: Some(10),
            ..Default::default()
        };
        let history = History {
            entries: vec![HistoryEntry {
                comic_id: "c1".to_string(),
                page_index: 2,
                ..Default::default()
            }],
        };
        let progress = library_progress(&history, &entry);
        assert_eq!(progress.read, 3);
        assert_eq!(progress.total, 10);
        assert!((progress.ratio() - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn library_progress_zero_when_no_page_count() {
        let entry = LibraryEntry {
            comic_id: "c1".to_string(),
            page_count: None,
            ..Default::default()
        };
        let history = History {
            entries: vec![HistoryEntry {
                comic_id: "c1".to_string(),
                page_index: 5,
                ..Default::default()
            }],
        };
        let progress = library_progress(&history, &entry);
        assert_eq!(progress.read, 0);
        assert_eq!(progress.total, 0);
        assert_eq!(progress.ratio(), 0.0);
    }
}
