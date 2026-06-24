use crate::loader::PageLoader;
use crate::opener::{AsyncOpener, OpenStatus};
use crate::shortcuts::is_shortcut_pressed;
use crate::timing;
use crate::views::settings::SettingsView;
use crate::views::{
    ebook::EbookView,
    library::{LibraryCallbacks, LibraryView},
    reader::ReaderView,
};
use egui_phosphor::regular;
use rust_reader_core::ebook::Ebook;
use rust_reader_core::models::{Comic, FitMode, PageSource, ReadingMode};
use rust_reader_core::state::ReadingState;
use rust_reader_storage::{
    json_store::JsonStore,
    models::{
        Bookmarks, EbookTheme, History, HistoryEntry, Library, MediaType, Settings, Theme,
        ToolbarDisplayMode,
    },
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn is_ebook_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "epub" | "mobi" | "azw" | "azw3" | "txt" | "md" | "markdown"
            )
        })
        .unwrap_or(false)
}

fn media_type_for_path(path: &std::path::Path) -> MediaType {
    if is_ebook_file(path) {
        MediaType::Ebook
    } else {
        MediaType::Comic
    }
}

pub struct ReaderApp {
    pub current_view: View,
    pub last_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
    pub ebook_view: EbookView,
    pub settings_view: SettingsView,
    pub store: JsonStore,
    pub history: History,
    pub bookmarks: Bookmarks,
    pub error_message: Option<String>,
    pub page_loader: PageLoader,
    pub cover_loader: PageLoader,
    pub opener: Option<AsyncOpener<Comic>>,
    pub ebook_opener: Option<AsyncOpener<Ebook>>,
    /// Cover requests in flight: epoch -> (comic_id, comic_path).
    pub pending_covers: HashMap<crate::loader::Epoch, (String, PathBuf)>,
    /// Comic ids for which a cover generation has already been requested.
    pub requested_cover_ids: HashSet<String>,
    /// The theme currently applied to egui, used to avoid redundant updates.
    pub current_theme: Theme,
}

impl Default for ReaderApp {
    fn default() -> Self {
        let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| PathBuf::from(".")));
        let (settings, settings_error) = load_settings_with_error(&store);
        let library = store.load_library().unwrap_or_default();
        let mut history = store.load_history().unwrap_or_default();
        let mut bookmarks = store.load_bookmarks().unwrap_or_default();
        let mut library_view = LibraryView::default();
        library_view.library = library;
        let covers_dir = store.dir().join("covers");
        if migrate_library_ids(
            &mut library_view.library,
            &mut history,
            &mut bookmarks,
            &covers_dir,
        ) {
            let _ = store.save_library(&library_view.library);
            let _ = store.save_history(&history);
            let _ = store.save_bookmarks(&bookmarks);
        }
        let page_loader = PageLoader::new_with_compress(
            settings.compress_images,
            settings.decode_threads as usize,
        );
        let cover_loader = PageLoader::new_with_compress(false, 1);
        Self {
            current_view: View::Library,
            last_view: View::Library,
            settings,
            library_view,
            reader_view: ReaderView::default(),
            ebook_view: EbookView::default(),
            settings_view: SettingsView::default(),
            store,
            history,
            bookmarks,
            error_message: settings_error,
            page_loader,
            cover_loader,
            opener: None,
            ebook_opener: None,
            pending_covers: HashMap::new(),
            requested_cover_ids: HashSet::new(),
            current_theme: Theme::System,
        }
    }
}

impl eframe::App for ReaderApp {
    fn on_exit(&mut self) {
        self.record_reader_history();
        self.record_ebook_history();
        self.reader_view.close();
        let _ = self.store.save_settings(&self.settings);
        let _ = self.store.save_library(&self.library_view.library);
        let _ = self.store.save_history(&self.history);
        let _ = self.store.save_bookmarks(&self.bookmarks);
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if matches!(self.last_view, View::Reader) && !matches!(self.current_view, View::Reader) {
            self.record_reader_history();
            self.reader_view.clear_cache();
        }
        if matches!(self.last_view, View::Ebook) && !matches!(self.current_view, View::Ebook) {
            self.record_ebook_history();
            self.ebook_view.close();
        }
        self.last_view = self.current_view.clone();

        if self.settings.theme != self.current_theme {
            self.apply_theme(ctx);
            self.current_theme = self.settings.theme.clone();
        }

        self.handle_global_input(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.ctrl) {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.add_folder_to_library(path);
            }
        }
        self.handle_dropped_files(ctx);
        #[cfg(target_os = "macos")]
        {
            let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
            self.handle_open_paths(dock_paths);
        }
        self.poll_opener(ctx);
        self.poll_ebook_opener(ctx, frame);

        let cache_size_mb = self.settings.cache_size_mb as usize;
        self.page_loader.set_compress(self.settings.compress_images);
        self.reader_view
            .update(ctx, &self.page_loader, cache_size_mb);
        self.reader_view.request_preloads(
            &self.page_loader,
            cache_size_mb,
            self.settings.real_image_cache_pages as usize,
        );
        self.poll_cover_results();

        self.render_menu_bar(ctx);

        match self.current_view.clone() {
            View::Library => self.render_library(ctx),
            View::Reader => self.render_reader(ctx),
            View::Ebook => self.render_ebook(ctx),
            View::Settings => self.render_settings(ctx),
            View::Loading(path) => self.render_loading(ctx, path),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Library,
    Reader,
    Ebook,
    Settings,
    Loading(PathBuf),
}

enum BarEdge {
    Top,
    Bottom,
}

fn toolbar_button(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    match mode {
        ToolbarDisplayMode::IconOnly => ui.button(icon).on_hover_text(text),
        ToolbarDisplayMode::TextOnly => ui.button(text),
        ToolbarDisplayMode::IconAndText => ui.button(format!("{} {}", icon, text)),
    }
}

fn toolbar_selectable(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    active: bool,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    let label = match mode {
        ToolbarDisplayMode::IconOnly => egui::WidgetText::from(icon),
        ToolbarDisplayMode::TextOnly => egui::WidgetText::from(text),
        ToolbarDisplayMode::IconAndText => egui::WidgetText::from(format!("{} {}", icon, text)),
    };
    ui.selectable_label(active, label).on_hover_text(text)
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn poll_opener(&mut self, ctx: &egui::Context) {
        let Some(mut opener) = self.opener.take() else {
            return;
        };
        match opener.poll() {
            OpenStatus::Loading => {
                self.opener = Some(opener);
            }
            OpenStatus::Ready(result) => match result {
                Ok(comic) => {
                    timing::log(&format!(
                        "poll_opener comic ready: {} pages",
                        comic.total_pages()
                    ));
                    let total = comic.volumes.first().map(|v| v.pages.len()).unwrap_or(0);
                    let mut state = ReadingState::new(self.settings.default_mode, total);
                    state.set_double_page(self.settings.double_page, total);
                    state.fit_mode = self.settings.default_fit;
                    if let Some(h) = self
                        .history
                        .entries
                        .iter()
                        .find(|h| history_matches(h, &comic.id, &comic.path))
                    {
                        state.go_to_page(h.page_index, total);
                    }
                    self.reader_view.open(
                        ctx,
                        comic,
                        state,
                        &self.page_loader,
                        self.settings.wide_page_threshold,
                        self.settings.enable_page_animation,
                    );
                    self.current_view = View::Reader;
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("无法打开漫画: {}", e));
                    self.current_view = View::Library;
                }
            },
        }
    }

    fn render_library(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error_message {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            let mut open_idx = None;
            let mut open_path: Option<PathBuf> = None;
            let mut add_requested = false;
            let mut request_cover_idx: Option<usize> = None;
            let mut remove_missing = false;
            let mut delete_bookmark_idx: Option<usize> = None;
            let mut update_bookmark: Option<(usize, Option<String>)> = None;
            let mut update_title: Option<(usize, String)> = None;
            let mut delete_library_idx: Option<usize> = None;
            let mut clear_history = false;
            let mut delete_history_idx: Option<usize> = None;
            self.library_view.ui(
                ui,
                &self.history,
                &self.bookmarks,
                &mut self.settings.library_sort,
                LibraryCallbacks {
                    on_open_library: &mut |idx| open_idx = Some(idx),
                    on_open_path: &mut |path| open_path = Some(path),
                    on_add: &mut || add_requested = true,
                    on_request_cover: &mut |idx| request_cover_idx = Some(idx),
                    on_remove_missing: &mut || remove_missing = true,
                    on_delete_bookmark: &mut |idx| delete_bookmark_idx = Some(idx),
                    on_update_bookmark: &mut |idx, note| update_bookmark = Some((idx, note)),
                    on_update_title: &mut |idx, title| update_title = Some((idx, title)),
                    on_delete_library: &mut |idx| delete_library_idx = Some(idx),
                    on_clear_history: &mut || clear_history = true,
                    on_delete_history: &mut |idx| delete_history_idx = Some(idx),
                },
            );
            if add_requested {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.add_folder_to_library(path);
                }
            }
            if let Some(idx) = open_idx {
                if let Some(entry) = self.library_view.entry_at(idx).cloned() {
                    self.open_path(entry.path);
                }
            }
            if let Some(path) = open_path {
                self.open_path(path);
            }
            if let Some(idx) = request_cover_idx {
                self.request_cover_for_library_entry(idx);
            }
            if remove_missing {
                self.remove_missing_library_entries();
            }
            if let Some((idx, title)) = update_title {
                if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
                    entry.title = title;
                }
            }
            if let Some(idx) = delete_library_idx {
                self.library_view.library.entries.remove(idx);
            }
            if let Some((idx, note)) = update_bookmark {
                if let Some(entry) = self.bookmarks.entries.get_mut(idx) {
                    entry.note = note;
                }
            }
            if let Some(idx) = delete_bookmark_idx {
                self.bookmarks.entries.remove(idx);
            }
            if clear_history {
                self.history.entries.clear();
            }
            if let Some(idx) = delete_history_idx {
                self.history.entries.remove(idx);
            }
        });
    }

    fn render_reader(&mut self, ctx: &egui::Context) {
        let Some(reader) = self.reader_view.open.as_ref() else {
            self.current_view = View::Library;
            return;
        };
        let total_pages = reader.total_pages();
        let current_page = reader.state.current_page;
        let mode = reader.state.mode;
        let zoom = reader.state.zoom;

        let (fullscreen, mouse_pos, screen_size) = ctx.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or(false),
                i.pointer.latest_pos(),
                i.screen_rect().size(),
            )
        });
        if Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        ) {
            self.render_reader_toolbar(ctx, total_pages, current_page, mode, zoom);
        }
        let (progress_bar_rect, hovered_page) = if Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        ) {
            self.render_reader_statusbar(ctx)
        } else {
            (None, None)
        };

        let bg = self.settings.background_color;
        let frame = egui::Frame::central_panel(&ctx.style())
            .fill(egui::Color32::from_rgb(bg[0], bg[1], bg[2]));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let response = self.reader_view.ui(ctx, ui, &self.page_loader);

            // Floating thumbnail tooltip above the cursor when hovering the progress bar.
            if progress_bar_rect.is_some() {
                self.reader_view
                    .render_progress_thumbnail(ctx, ui, hovered_page);
            }

            // Right-click context menu on the page area.
            if let Some(response) = response {
                // Click left/right halves to turn page (disabled in webtoon mode).
                if mode != ReadingMode::Webtoon && response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let center_x = response.rect.center().x;
                        let rtl = mode == ReadingMode::Rtl;
                        if pos.x < center_x {
                            if rtl {
                                self.reader_next_page();
                            } else {
                                self.reader_prev_page();
                            }
                        } else if rtl {
                            self.reader_prev_page();
                        } else {
                            self.reader_next_page();
                        }
                    }
                }
                response.context_menu(|ui| {
                    self.context_menu_items(ui);
                });
            }

            // Scroll wheel navigation.
            let mut scroll = ui.input(|i| i.raw_scroll_delta.y);
            if self.settings.invert_scroll {
                scroll = -scroll;
            }
            let modifiers = ui.input(|i| i.modifiers);
            let ctrl_or_cmd = modifiers.command || modifiers.ctrl;
            if ctrl_or_cmd {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    if scroll > 2.0 {
                        reader.zoom_in();
                    } else if scroll < -2.0 {
                        reader.zoom_out();
                    }
                }
            } else if mode != ReadingMode::Webtoon {
                if scroll > 2.0 {
                    self.reader_page_down();
                } else if scroll < -2.0 {
                    self.reader_page_up();
                }
            }

            // Double-click toggles between Original (100%) and Page fit.
            if ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    let new_fit = if reader.state.fit_mode == FitMode::Original {
                        FitMode::Page
                    } else {
                        FitMode::Original
                    };
                    reader.request_fit(new_fit);
                }
            }
        });
    }

    fn render_ebook(&mut self, ctx: &egui::Context) {
        if self.ebook_view.open.is_none() {
            self.current_view = View::Library;
            return;
        }

        let (fullscreen, mouse_pos, screen_size) = ctx.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or(false),
                i.pointer.latest_pos(),
                i.screen_rect().size(),
            )
        });
        let show_toolbar = Self::should_show_bar(
            self.settings.show_toolbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Top,
        );
        let show_statusbar = Self::should_show_bar(
            self.settings.show_statusbar,
            fullscreen,
            mouse_pos,
            screen_size,
            BarEdge::Bottom,
        );
        if show_toolbar {
            self.render_ebook_toolbar(ctx);
        }
        if show_statusbar {
            self.render_ebook_statusbar(ctx);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(rect.min.x, rect.min.y).into(),
                size: wry::dpi::LogicalSize::new(rect.width(), rect.height()).into(),
            };
            self.ebook_view.update_bounds(bounds);
            self.ebook_view.ui(ctx, ui);
        });
    }

    fn render_reader_toolbar(
        &mut self,
        ctx: &egui::Context,
        total_pages: usize,
        current_page: usize,
        mode: ReadingMode,
        zoom: f32,
    ) {
        let display_mode = self.settings.toolbar_display_mode;
        egui::TopBottomPanel::top("reader_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if toolbar_button(ui, regular::HOUSE, "书架", display_mode).clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();

                let modes = [
                    (ReadingMode::Ltr, regular::ARROW_RIGHT, "国漫"),
                    (ReadingMode::Rtl, regular::ARROW_LEFT, "日漫"),
                    (ReadingMode::Webtoon, regular::ARROW_DOWN, "韩漫"),
                ];
                for (m, icon, label) in modes {
                    if toolbar_selectable(ui, icon, label, mode == m, display_mode).clicked() {
                        if let Some(reader) = self.reader_view.open.as_mut() {
                            reader.state.set_mode(m, total_pages);
                        }
                    }
                }
                ui.separator();

                if mode != ReadingMode::Webtoon {
                    let double_page = self
                        .reader_view
                        .open
                        .as_ref()
                        .map(|r| r.state.double_page)
                        .unwrap_or(self.settings.double_page);
                    if toolbar_selectable(ui, regular::BOOK_OPEN, "双页", double_page, display_mode)
                        .on_hover_text("切换到双页模式")
                        .clicked()
                    {
                        let new_double = !double_page;
                        self.settings.double_page = new_double;
                        if let Some(reader) = self.reader_view.open.as_mut() {
                            reader.state.set_double_page(new_double, total_pages);
                            reader.pending_fit = Some(FitMode::Page);
                        }
                    }
                    ui.separator();
                }

                if toolbar_button(ui, regular::MINUS, "", display_mode).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_out();
                    }
                }
                ui.label(format!("{:.0}%", zoom * 100.0));
                if toolbar_button(ui, regular::PLUS, "", display_mode).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_in();
                    }
                }
                if toolbar_button(
                    ui,
                    regular::ARROWS_OUT_LINE_HORIZONTAL,
                    "适应宽度",
                    display_mode,
                )
                .clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Width);
                    }
                }
                if toolbar_button(
                    ui,
                    regular::ARROWS_OUT_LINE_VERTICAL,
                    "适应高度",
                    display_mode,
                )
                .clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Height);
                    }
                }
                if toolbar_button(ui, regular::FRAME_CORNERS, "自动适应", display_mode).clicked()
                {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Page);
                    }
                }
                ui.separator();

                if toolbar_button(ui, regular::CARET_LEFT, "上一页", display_mode).clicked() {
                    self.reader_prev_page();
                }
                let mut displayed_page = current_page + 1;
                let page_response = ui.add(
                    egui::DragValue::new(&mut displayed_page)
                        .speed(1.0)
                        .range(1..=total_pages.max(1))
                        .update_while_editing(false),
                );
                if page_response.lost_focus() {
                    let target = displayed_page
                        .saturating_sub(1)
                        .min(total_pages.saturating_sub(1));
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        if target != reader.state.current_page {
                            reader.state.go_to_page(target, total_pages);
                            reader.mark_page_turn();
                        }
                    }
                }
                ui.label(format!("/ {}", total_pages));
                if toolbar_button(ui, regular::CARET_RIGHT, "下一页", display_mode).clicked() {
                    self.reader_next_page();
                }
                ui.separator();

                if toolbar_button(ui, regular::BOOKMARK, "添加书签", display_mode).clicked() {
                    self.add_bookmark(current_page);
                }
                if toolbar_button(ui, regular::ARROWS_OUT_SIMPLE, "全屏", display_mode).clicked()
                {
                    self.toggle_fullscreen(ctx);
                }
                if toolbar_button(ui, regular::GEAR, "设置", display_mode).clicked() {
                    self.current_view = View::Settings;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(regular::X).on_hover_text("隐藏工具栏").clicked() {
                        self.settings.show_toolbar = false;
                    }
                });
            });
        });
    }

    fn render_reader_statusbar(
        &mut self,
        ctx: &egui::Context,
    ) -> (Option<egui::Rect>, Option<usize>) {
        let mut progress_rect = None;
        let mut hovered_page = None;
        egui::TopBottomPanel::bottom("reader_statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let bar_response = self.reader_view.render_progress_bar(ui);
                hovered_page = bar_response.hovered_page;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").on_hover_text("隐藏状态栏").clicked() {
                        self.settings.show_statusbar = false;
                    }
                });
            });
            progress_rect = Some(ui.min_rect());
        });
        (progress_rect, hovered_page)
    }

    fn render_ebook_toolbar(&mut self, ctx: &egui::Context) {
        self.ebook_view.sync_position();
        let total = self
            .ebook_view
            .open
            .as_ref()
            .map(|e| e.ebook.total_chapters())
            .unwrap_or(0);
        let current = self
            .ebook_view
            .open
            .as_ref()
            .map(|e| e.current_chapter)
            .unwrap_or(0);

        egui::TopBottomPanel::top("ebook_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("书架").clicked() {
                    self.current_view = View::Library;
                }
                ui.separator();
                if ui.button("目录").clicked() {
                    // TODO: open TOC panel
                }
                if ui.button("上一页").clicked() {
                    self.ebook_view.prev_page();
                }
                if ui.button("下一页").clicked() {
                    self.ebook_view.next_page();
                }
                ui.label(format!("{} / {}", current + 1, total));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("设置").clicked() {
                        self.current_view = View::Settings;
                    }
                });
            });
        });
    }

    fn render_ebook_statusbar(&mut self, ctx: &egui::Context) {
        self.ebook_view.sync_position();
        let (title, progress) = self
            .ebook_view
            .open
            .as_ref()
            .map(|e| {
                let title = e
                    .ebook
                    .chapters
                    .get(e.current_chapter)
                    .and_then(|c| c.title.clone())
                    .unwrap_or_else(|| "无标题".to_string());
                let progress = if e.ebook.total_chapters() > 0 {
                    (e.current_chapter + 1) as f32 / e.ebook.total_chapters() as f32 * 100.0
                } else {
                    0.0
                };
                (title, progress)
            })
            .unwrap_or_else(|| ("".to_string(), 0.0));

        egui::TopBottomPanel::bottom("ebook_statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}%", progress));
                });
            });
        });
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("← 返回").clicked() {
                    self.current_view = if self.reader_view.open.is_some() {
                        View::Reader
                    } else if self.ebook_view.open.is_some() {
                        View::Ebook
                    } else {
                        View::Library
                    };
                }
            });
            ui.separator();
            self.settings_view.ui(ui, &mut self.settings);
        });
    }

    fn render_loading(&mut self, ctx: &egui::Context, path: PathBuf) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.vertical_centered(|ui| {
                        // Static icon avoids the continuous repaint that egui's
                        // animated spinner triggers.
                        ui.label(egui::RichText::new("⏳").size(24.0));
                        ui.label("正在打开漫画...");
                        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                            ui.label(egui::RichText::new(name).size(14.0).strong());
                        }
                        ui.add_space(16.0);
                        if ui.button("取消").clicked() {
                            self.opener = None;
                            self.ebook_opener = None;
                            self.current_view = View::Library;
                        }
                        if let Some(err) = &self.error_message {
                            ui.colored_label(ui.visuals().error_fg_color, err);
                        }
                    });
                },
            );
        });
    }

    fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("文件", |ui| {
                    if ui.button("打开文件夹").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.add_folder_to_library(path);
                        }
                        ui.close_menu();
                    }
                    ui.menu_button("打开最近", |ui| {
                        let recent: Vec<_> = self
                            .history
                            .entries
                            .iter()
                            .filter_map(|h| {
                                self.library_view
                                    .find_by_id(&h.comic_id)
                                    .map(|e| (e.title.clone(), e.path.clone()))
                            })
                            .collect();
                        if recent.is_empty() {
                            ui.weak("暂无记录");
                        } else {
                            for (title, path) in recent {
                                if ui.button(&title).clicked() {
                                    self.open_path(path);
                                    ui.close_menu();
                                }
                            }
                        }
                    });
                    ui.separator();
                    let can_back = matches!(self.current_view, View::Reader | View::Settings);
                    if ui
                        .add_enabled(can_back, egui::Button::new("返回书架"))
                        .clicked()
                    {
                        self.current_view = View::Library;
                        ui.close_menu();
                    }
                });

                ui.menu_button("视图", |ui| {
                    let toolbar_label = if self.settings.show_toolbar {
                        "隐藏工具栏"
                    } else {
                        "显示工具栏"
                    };
                    if ui.button(toolbar_label).clicked() {
                        self.settings.show_toolbar = !self.settings.show_toolbar;
                        ui.close_menu();
                    }
                    let statusbar_label = if self.settings.show_statusbar {
                        "隐藏状态栏"
                    } else {
                        "显示状态栏"
                    };
                    if ui.button(statusbar_label).clicked() {
                        self.settings.show_statusbar = !self.settings.show_statusbar;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("全屏").clicked() {
                        self.toggle_fullscreen(ctx);
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("主题", |ui| {
                        use rust_reader_storage::models::Theme;
                        for (theme, label) in [
                            (Theme::System, "跟随系统"),
                            (Theme::Light, "浅色"),
                            (Theme::Dark, "深色"),
                        ] {
                            if ui
                                .selectable_label(self.settings.theme == theme, label)
                                .clicked()
                            {
                                self.settings.theme = theme;
                                ui.close_menu();
                            }
                        }
                    });
                });

                let is_reader = matches!(self.current_view, View::Reader);
                let is_ebook = matches!(self.current_view, View::Ebook);
                ui.add_enabled_ui(is_reader || is_ebook, |ui| {
                    ui.menu_button("阅读", |ui| {
                        if is_ebook {
                            self.render_ebook_menu(ui);
                        } else {
                            self.render_reader_menu(ui);
                        }
                    });
                });

                ui.menu_button("工具", |ui| {
                    if ui.button("设置").clicked() {
                        self.current_view = View::Settings;
                        ui.close_menu();
                    }
                });

                ui.menu_button("帮助", |ui| {
                    if ui.button("关于 rustReader").clicked() {
                        self.error_message = Some(format!(
                            "rustReader v{}\n一个用 Rust 写的漫画阅读器。",
                            env!("CARGO_PKG_VERSION")
                        ));
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn render_reader_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("上一页").clicked() {
            self.reader_prev_page();
            ui.close_menu();
        }
        if ui.button("下一页").clicked() {
            self.reader_next_page();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("首页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
            ui.close_menu();
        }
        if ui.button("末页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            if let Some(reader) = self.reader_view.open.as_ref() {
                self.add_bookmark(reader.state.current_page);
            }
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("模式", |ui| {
            let mode = self
                .reader_view
                .open
                .as_ref()
                .map(|r| r.state.mode)
                .unwrap_or(self.settings.default_mode);
            let total_pages = self
                .reader_view
                .open
                .as_ref()
                .map(|r| r.total_pages())
                .unwrap_or(0);
            for (m, label) in [
                (ReadingMode::Ltr, "国漫（从左到右）"),
                (ReadingMode::Rtl, "日漫（从右到左）"),
                (ReadingMode::Webtoon, "韩漫（条漫）"),
            ] {
                if ui.selectable_label(mode == m, label).clicked() {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.state.set_mode(m, total_pages);
                    }
                    ui.close_menu();
                }
            }
        });
        let mode = self
            .reader_view
            .open
            .as_ref()
            .map(|r| r.state.mode)
            .unwrap_or(self.settings.default_mode);
        let double_page = self
            .reader_view
            .open
            .as_ref()
            .map(|r| r.state.double_page)
            .unwrap_or(self.settings.double_page);
        if ui
            .add_enabled(
                !mode.is_webtoon(),
                egui::Button::new(if double_page {
                    "关闭双页"
                } else {
                    "双页"
                }),
            )
            .clicked()
        {
            let new_double = !double_page;
            self.settings.double_page = new_double;
            if let Some(reader) = self.reader_view.open.as_mut() {
                let total = reader.total_pages();
                reader.state.set_double_page(new_double, total);
                reader.pending_fit = Some(FitMode::Page);
            }
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("缩放", |ui| {
            if ui.button("放大").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.zoom_in();
                }
                ui.close_menu();
            }
            if ui.button("缩小").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.zoom_out();
                }
                ui.close_menu();
            }
            ui.separator();
            if ui.button("适应宽度").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Width);
                }
                ui.close_menu();
            }
            if ui.button("适应高度").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Height);
                }
                ui.close_menu();
            }
            if ui.button("自动适应").clicked() {
                if let Some(reader) = self.reader_view.open.as_mut() {
                    reader.request_fit(FitMode::Page);
                }
                ui.close_menu();
            }
        });
    }

    fn render_ebook_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("上一页").clicked() {
            self.ebook_view.prev_page();
            ui.close_menu();
        }
        if ui.button("下一页").clicked() {
            self.ebook_view.next_page();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("目录").clicked() {
            // TODO: open TOC panel
            ui.close_menu();
        }
        ui.separator();
        if ui.button("增大字体").clicked() {
            self.settings.ebook.font_size += 1;
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close_menu();
        }
        if ui.button("减小字体").clicked() {
            if self.settings.ebook.font_size > 1 {
                self.settings.ebook.font_size -= 1;
            }
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("切换主题").clicked() {
            self.settings.ebook.theme = match self.settings.ebook.theme {
                EbookTheme::Light => EbookTheme::Dark,
                EbookTheme::Dark => EbookTheme::Sepia,
                EbookTheme::Sepia => EbookTheme::Light,
            };
            self.ebook_view.apply_settings(&self.settings.ebook);
            ui.close_menu();
        }
    }

    fn context_menu_items(&mut self, ui: &mut egui::Ui) {
        if ui.button("下一页").clicked() {
            self.reader_next_page();
            ui.close_menu();
        }
        if ui.button("上一页").clicked() {
            self.reader_prev_page();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("首页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.first_page();
            }
            ui.close_menu();
        }
        if ui.button("末页").clicked() {
            if let Some(reader) = self.reader_view.open.as_mut() {
                reader.last_page();
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("添加书签").clicked() {
            if let Some(reader) = self.reader_view.open.as_ref() {
                self.add_bookmark(reader.state.current_page);
            }
            ui.close_menu();
        }
        if ui.button("全屏").clicked() {
            self.toggle_fullscreen(ui.ctx());
            ui.close_menu();
        }
        ui.separator();
        let toolbar_label = if self.settings.show_toolbar {
            "隐藏工具栏"
        } else {
            "显示工具栏"
        };
        if ui.button(toolbar_label).clicked() {
            self.settings.show_toolbar = !self.settings.show_toolbar;
            ui.close_menu();
        }
        let statusbar_label = if self.settings.show_statusbar {
            "隐藏状态栏"
        } else {
            "显示状态栏"
        };
        if ui.button(statusbar_label).clicked() {
            self.settings.show_statusbar = !self.settings.show_statusbar;
            ui.close_menu();
        }
        ui.separator();
        if ui.button("返回书架").clicked() {
            self.current_view = View::Library;
            ui.close_menu();
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let preference = match self.settings.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        };
        ctx.set_theme(preference);
    }

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        if is_shortcut_pressed(ctx, &self.settings.shortcuts.fullscreen) {
            self.toggle_fullscreen(ctx);
        }

        match self.current_view {
            View::Reader => {
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.back_to_library) {
                    if ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
                        self.toggle_fullscreen(ctx);
                    } else {
                        self.current_view = View::Library;
                    }
                }

                let rtl = self
                    .reader_view
                    .open
                    .as_ref()
                    .map(|r| r.state.mode == ReadingMode::Rtl)
                    .unwrap_or(false);
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                    if rtl {
                        self.reader_prev_page();
                    } else {
                        self.reader_next_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                    if rtl {
                        self.reader_next_page();
                    } else {
                        self.reader_prev_page();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_down) {
                    self.reader_page_down();
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.page_up) {
                    self.reader_page_up();
                }

                // Mouse side buttons: Extra1 is typically "back" and Extra2 is "forward".
                let extra1_pressed =
                    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1));
                let extra2_pressed =
                    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2));
                if extra1_pressed {
                    if rtl {
                        self.reader_next_page();
                    } else {
                        self.reader_prev_page();
                    }
                }
                if extra2_pressed {
                    if rtl {
                        self.reader_prev_page();
                    } else {
                        self.reader_next_page();
                    }
                }

                if is_shortcut_pressed(ctx, &self.settings.shortcuts.zoom_in) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_in();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.zoom_out) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.zoom_out();
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_page) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Page);
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_width) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Width);
                    }
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.fit_height) {
                    if let Some(reader) = self.reader_view.open.as_mut() {
                        reader.request_fit(FitMode::Height);
                    }
                }
            }
            View::Ebook => {
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.next_page) {
                    self.ebook_view.next_page();
                }
                if is_shortcut_pressed(ctx, &self.settings.shortcuts.prev_page) {
                    self.ebook_view.prev_page();
                }
            }
            _ => {}
        }
    }

    fn reader_next_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.next_page_with_animation();
        }
    }

    fn reader_prev_page(&mut self) {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.prev_page_with_animation();
        }
    }

    fn reader_page_down(&mut self) {
        self.reader_next_page();
    }

    fn reader_page_up(&mut self) {
        self.reader_prev_page();
    }

    fn toggle_fullscreen(&self, ctx: &egui::Context) {
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
    }

    fn should_show_bar(
        show_setting: bool,
        fullscreen: bool,
        mouse_pos: Option<egui::Pos2>,
        screen_size: egui::Vec2,
        edge: BarEdge,
    ) -> bool {
        if !show_setting {
            return false;
        }
        if !fullscreen {
            return true;
        }
        const THRESHOLD: f32 = 20.0;
        match edge {
            BarEdge::Top => mouse_pos.map(|p| p.y <= THRESHOLD).unwrap_or(false),
            BarEdge::Bottom => mouse_pos
                .map(|p| p.y >= screen_size.y - THRESHOLD)
                .unwrap_or(false),
        }
    }

    fn record_reader_history(&mut self) {
        if let Some(reader) = self.reader_view.open.as_ref() {
            let comic_id = reader.comic.id.clone();
            let path = reader.comic.path.clone();
            let page_index = reader.state.current_page;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| history_matches(h, &comic_id, &path))
            {
                entry.comic_id = comic_id;
                entry.path = path;
                entry.page_index = page_index;
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id,
                    path,
                    volume_index: 0,
                    page_index,
                    char_offset: None,
                    last_read_at: now,
                });
            }
        }
    }

    fn record_ebook_history(&mut self) {
        self.ebook_view.sync_position();
        if let Some(open) = self.ebook_view.open.as_ref() {
            let ebook_id = open.ebook.id.clone();
            let path = open.ebook.path.clone();
            let chapter = open.current_chapter;
            let char_offset = open.renderer.current_position().1;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self
                .history
                .entries
                .iter_mut()
                .find(|h| history_matches(h, &ebook_id, &path))
            {
                entry.comic_id = ebook_id;
                entry.path = path;
                entry.page_index = chapter;
                entry.char_offset = Some(char_offset);
                entry.last_read_at = now;
            } else {
                self.history.entries.push(HistoryEntry {
                    comic_id: ebook_id,
                    path,
                    volume_index: 0,
                    page_index: chapter,
                    char_offset: Some(char_offset),
                    last_read_at: now,
                });
            }
        }
    }

    fn covers_dir(&self) -> PathBuf {
        self.store.dir().join("covers")
    }

    fn request_cover(&mut self, comic_id: &str, path: &Path, source: PageSource) {
        if !self.requested_cover_ids.insert(comic_id.to_string()) {
            return;
        }
        let epoch = self.cover_loader.next_epoch();
        if self.cover_loader.request_thumbnail_high(epoch, 0, source) {
            self.pending_covers
                .insert(epoch, (comic_id.to_string(), path.to_path_buf()));
        } else {
            self.requested_cover_ids.remove(comic_id);
        }
    }

    fn poll_cover_results(&mut self) {
        while let Some(result) = self.cover_loader.try_recv() {
            if !result.thumbnail || result.dropped {
                continue;
            }
            let (comic_id, _path) = match self.pending_covers.remove(&result.epoch) {
                Some(v) => v,
                None => continue,
            };
            let image = match result.image {
                Ok(crate::loader::LoadedImage::Color(img)) => img,
                _ => continue,
            };
            if let Some(cover_path) = save_cover_image(&self.covers_dir(), &comic_id, &image) {
                if let Some(entry) = self
                    .library_view
                    .library
                    .entries
                    .iter_mut()
                    .find(|e| e.comic_id == comic_id)
                {
                    entry.cover_path = Some(cover_path);
                    let _ = self.store.save_library(&self.library_view.library);
                }
            }
        }
    }

    fn add_folder_to_library(&mut self, path: std::path::PathBuf) {
        if path.is_file() {
            self.add_file_to_library(path);
            return;
        }

        if let Ok(comic) = rust_reader_parser::parse(&path) {
            self.add_comic_to_library(comic, &path);
            return;
        }

        // If the selected path is not a comic itself, recursively scan it for
        // supported archives/folders and ebooks and import everything found.
        let mut found = 0;
        for entry in walk_supported_files(&path) {
            self.add_file_to_library(entry);
            found += 1;
        }
        if found == 0 {
            self.error_message = Some(format!("无法添加文件或文件夹: {}", path.display()));
        }
    }

    fn add_comic_to_library(
        &mut self,
        comic: rust_reader_core::models::Comic,
        path: &std::path::Path,
    ) {
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let comic_id = comic.id.clone();
        if !self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            if let Some(page) = comic.volumes.first().and_then(|v| v.pages.first()) {
                self.request_cover(&comic_id, path, page.source.clone());
            }
            self.library_view
                .library
                .entries
                .push(rust_reader_storage::models::LibraryEntry {
                    comic_id,
                    title: comic.title.clone(),
                    path: path.to_path_buf(),
                    cover_path: None,
                    added_at,
                    media_type: media_type_for_path(path),
                });
        }
    }

    fn add_ebook_to_library(&mut self, path: std::path::PathBuf) {
        let Ok(ebook) = rust_reader_parser::parse_ebook(&path) else {
            return;
        };
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let media_type = media_type_for_path(&path);
        if !self
            .library_view
            .library
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            self.library_view
                .library
                .entries
                .push(rust_reader_storage::models::LibraryEntry {
                    comic_id: ebook.id,
                    title: ebook.title,
                    path,
                    cover_path: None,
                    added_at,
                    media_type,
                });
        }
    }

    fn add_file_to_library(&mut self, path: std::path::PathBuf) {
        if is_ebook_file(&path) {
            self.add_ebook_to_library(path);
        } else if let Ok(comic) = rust_reader_parser::parse(&path) {
            self.add_comic_to_library(comic, &path);
        }
    }

    fn add_bookmark(&mut self, page_index: usize) {
        if let Some(reader) = self.reader_view.open.as_ref() {
            let comic_id = reader.comic.id.clone();
            let exists = self.bookmarks.entries.iter().any(|b| {
                b.comic_id == comic_id && b.volume_index == 0 && b.page_index == page_index
            });
            if !exists {
                self.bookmarks
                    .entries
                    .push(rust_reader_storage::models::Bookmark {
                        comic_id,
                        volume_index: 0,
                        page_index,
                        note: None,
                    });
            }
        }
    }

    fn request_cover_for_library_entry(&mut self, idx: usize) {
        let Some(entry) = self.library_view.library.entries.get(idx) else {
            return;
        };
        if !entry.path.exists() {
            return;
        }
        if !self.requested_cover_ids.insert(entry.comic_id.clone()) {
            return;
        }
        let comic = match rust_reader_parser::parse(&entry.path) {
            Ok(c) => c,
            Err(_) => {
                self.requested_cover_ids.remove(&entry.comic_id);
                return;
            }
        };
        let Some(page) = comic.volumes.first().and_then(|v| v.pages.first()) else {
            self.requested_cover_ids.remove(&entry.comic_id);
            return;
        };
        let epoch = self.cover_loader.next_epoch();
        if !self
            .cover_loader
            .request_thumbnail_high(epoch, 0, page.source.clone())
        {
            self.requested_cover_ids.remove(&entry.comic_id);
            return;
        }
        self.pending_covers
            .insert(epoch, (entry.comic_id.clone(), entry.path.clone()));
    }

    fn remove_missing_library_entries(&mut self) {
        let before = self.library_view.library.entries.len();
        self.library_view
            .library
            .entries
            .retain(|e| e.path.exists());
        let removed = before.saturating_sub(self.library_view.library.entries.len());
        if removed > 0 {
            timing::log(&format!("removed {} missing library entries", removed));
        }
    }

    fn open_comic(&mut self, path: std::path::PathBuf) {
        timing::log(&format!("open_comic {:?}", path));
        self.opener = Some(AsyncOpener::open(path.clone(), |p| {
            rust_reader_parser::parse(p).map_err(|e| e.to_string())
        }));
        self.current_view = View::Loading(path);
        self.error_message = None;
    }

    fn open_ebook(&mut self, path: std::path::PathBuf) {
        timing::log(&format!("open_ebook {:?}", path));
        self.ebook_opener = Some(AsyncOpener::open(path.clone(), |p| {
            rust_reader_parser::parse_ebook(p).map_err(|e| e.to_string())
        }));
        self.current_view = View::Loading(path);
        self.error_message = None;
    }

    fn open_path(&mut self, path: std::path::PathBuf) {
        if is_ebook_file(&path) {
            self.open_ebook(path);
        } else {
            self.open_comic(path);
        }
    }

    fn poll_ebook_opener(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(mut opener) = self.ebook_opener.take() else {
            return;
        };
        match opener.poll() {
            OpenStatus::Loading => self.ebook_opener = Some(opener),
            OpenStatus::Ready(result) => match result {
                Ok(ebook) => {
                    let screen = ctx.screen_rect();
                    let bounds = wry::Rect {
                        position: wry::dpi::LogicalPosition::new(screen.min.x, screen.min.y).into(),
                        size: wry::dpi::LogicalSize::new(screen.width(), screen.height()).into(),
                    };
                    match self
                        .ebook_view
                        .open(frame, bounds, ebook.clone(), &self.settings.ebook)
                    {
                        Ok(()) => {
                            if let Some(h) = self
                                .history
                                .entries
                                .iter()
                                .find(|h| history_matches(h, &ebook.id, &ebook.path))
                            {
                                let chapter = h.page_index;
                                let offset = h.char_offset.unwrap_or(0);
                                self.ebook_view.goto_chapter(chapter);
                                self.ebook_view
                                    .open
                                    .as_mut()
                                    .map(|o| o.renderer.goto_chapter(chapter, offset));
                            }
                            self.current_view = View::Ebook;
                            self.error_message = None;
                        }
                        Err(e) => {
                            self.error_message = Some(format!("无法创建阅读器: {}", e));
                            self.current_view = View::Library;
                        }
                    }
                }
                Err(e) => {
                    self.error_message = Some(format!("无法打开电子书: {}", e));
                    self.current_view = View::Library;
                }
            },
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<_> = ctx
            .input(|i| i.raw.dropped_files.clone())
            .into_iter()
            .filter_map(|f| f.path)
            .collect();
        self.handle_open_paths(paths);
    }

    fn handle_open_paths(&mut self, paths: Vec<PathBuf>) {
        for path in &paths {
            self.add_folder_to_library(path.clone());
        }
        if let Some(path) = paths.first() {
            if is_ebook_file(path) || rust_reader_parser::parse(path).is_ok() {
                self.open_path(path.clone());
            }
        }
    }
}

fn load_settings_with_error(store: &JsonStore) -> (Settings, Option<String>) {
    match store.load_settings() {
        Ok(s) => (s, None),
        Err(err) => {
            let (s, e) = *err;
            (s, Some(e.to_string()))
        }
    }
}

fn history_matches(entry: &HistoryEntry, comic_id: &str, path: &std::path::Path) -> bool {
    if entry.comic_id == comic_id {
        return true;
    }
    !entry.path.as_os_str().is_empty() && entry.path == path
}

/// Recursively walk `root` and return paths that look like supported comic
/// files, ebook files, or folders containing images.
fn walk_supported_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if is_supported_comic_file(&path) || is_ebook_file(&path) {
                result.push(path);
            }
        }
    }
    result.sort();
    result
}

fn is_supported_comic_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "zip" | "cbz" | "rar" | "cbr" | "pdf"
            )
        })
        .unwrap_or(false)
}

fn cover_filename(comic_id: &str) -> String {
    let safe: String = comic_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    comic_id.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{}_{:016x}.jpg", safe, hash)
}

fn cover_path_for_comic_id(covers_dir: &Path, comic_id: &str) -> PathBuf {
    covers_dir.join(cover_filename(comic_id))
}

fn save_cover_image(
    covers_dir: &Path,
    comic_id: &str,
    image: &egui::ColorImage,
) -> Option<PathBuf> {
    std::fs::create_dir_all(covers_dir).ok()?;
    let path = cover_path_for_comic_id(covers_dir, comic_id);
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
        .collect();
    let img = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, rgba)?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    rgb.save_with_format(&path, image::ImageFormat::Jpeg).ok()?;
    Some(path)
}

fn migrate_library_ids(
    library: &mut Library,
    history: &mut History,
    bookmarks: &mut Bookmarks,
    covers_dir: &Path,
) -> bool {
    let mut id_map = HashMap::new();
    for entry in &mut library.entries {
        let expected = rust_reader_parser::stable_comic_id(&entry.path);
        if entry.comic_id != expected {
            id_map.insert(entry.comic_id.clone(), expected.clone());
            entry.comic_id = expected;
        }
    }
    if id_map.is_empty() {
        return false;
    }
    for h in &mut history.entries {
        if let Some(new) = id_map.get(&h.comic_id) {
            h.comic_id = new.clone();
            if let Some(entry) = library.entries.iter().find(|e| e.comic_id == *new) {
                h.path = entry.path.clone();
            }
        }
    }
    for b in &mut bookmarks.entries {
        if let Some(new) = id_map.get(&b.comic_id) {
            b.comic_id = new.clone();
        }
    }
    let _ = std::fs::create_dir_all(covers_dir);
    for (old_id, new_id) in &id_map {
        let old_path = cover_path_for_comic_id(covers_dir, old_id);
        let new_path = cover_path_for_comic_id(covers_dir, new_id);
        if old_path.exists() && !new_path.exists() && std::fs::rename(&old_path, &new_path).is_ok()
        {
            for entry in &mut library.entries {
                if entry.comic_id == *new_id && entry.cover_path.as_deref() == Some(&*old_path) {
                    entry.cover_path = Some(new_path.clone());
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_core::models::{Comic, Page, PageSource, Volume};
    use rust_reader_core::state::ReadingState;
    use std::path::Path;

    fn dummy_comic() -> Comic {
        Comic {
            id: "test-comic".to_string(),
            title: "Test Comic".to_string(),
            path: PathBuf::from("/tmp/test-comic"),
            volumes: vec![Volume {
                title: "Vol 1".to_string(),
                pages: (0..10)
                    .map(|i| Page {
                        index: i,
                        source: PageSource::File(PathBuf::from(format!("page{}.png", i))),
                    })
                    .collect(),
            }],
        }
    }

    fn app_with_temp_store() -> (ReaderApp, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let app = ReaderApp::with_store_dir(tmp.path());
        (app, tmp)
    }

    fn write_dummy_image(dir: &Path, name: &str) {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        img.save(dir.join(name)).unwrap();
    }

    impl ReaderApp {
        fn with_store_dir(dir: &Path) -> Self {
            let store = JsonStore::new(dir);
            let (settings, _settings_error) = load_settings_with_error(&store);
            let library = store.load_library().unwrap_or_default();
            let history = store.load_history().unwrap_or_default();
            let bookmarks = store.load_bookmarks().unwrap_or_default();
            let mut library_view = LibraryView::default();
            library_view.library = library;
            let page_loader = PageLoader::new_with_compress(
                settings.compress_images,
                settings.decode_threads as usize,
            );
            let cover_loader = PageLoader::new_with_compress(false, 1);
            Self {
                current_view: View::Library,
                last_view: View::Library,
                settings,
                library_view,
                reader_view: ReaderView::default(),
                ebook_view: EbookView::default(),
                settings_view: SettingsView::default(),
                store,
                history,
                bookmarks,
                error_message: None,
                page_loader,
                cover_loader,
                opener: None,
                ebook_opener: None,
                pending_covers: HashMap::new(),
                requested_cover_ids: HashSet::new(),
                current_theme: Theme::System,
            }
        }
    }

    #[test]
    fn test_record_reader_history_creates_entry() {
        let (mut app, _tmp) = app_with_temp_store();
        let comic = dummy_comic();
        let ctx = egui::Context::default();
        app.reader_view.open(
            &ctx,
            comic.clone(),
            ReadingState::new(ReadingMode::Ltr, 10),
            &PageLoader::default(),
            1.4,
            true,
        );
        app.reader_view.open.as_mut().unwrap().state.current_page = 5;
        app.record_reader_history();
        let entry = app
            .history
            .entries
            .iter()
            .find(|h| h.comic_id == comic.id)
            .expect("history entry should exist");
        assert_eq!(entry.page_index, 5);
    }

    #[test]
    fn test_poll_opener_restores_page_from_history() {
        let (mut app, _tmp) = app_with_temp_store();
        let tmp_dir = tempfile::tempdir().unwrap();
        write_dummy_image(tmp_dir.path(), "page0.png");
        write_dummy_image(tmp_dir.path(), "page1.png");

        let comic_id = rust_reader_parser::stable_comic_id(tmp_dir.path());
        app.history.entries.push(HistoryEntry {
            comic_id: comic_id.clone(),
            path: tmp_dir.path().to_path_buf(),
            volume_index: 0,
            page_index: 1,
            char_offset: None,
            last_read_at: 0,
        });

        app.open_comic(tmp_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app.poll_opener(&ctx);
            if app.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.current_view, View::Reader);
        let reader = app
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.comic.id, comic_id);
        assert_eq!(reader.state.current_page, 1);
    }

    #[test]
    fn test_history_roundtrip_via_storage() {
        let (mut app1, store_tmp) = app_with_temp_store();
        // Use a stable folder name so the parsed comic id matches the saved history.
        let comic_dir = store_tmp.path().join("test-comic");
        std::fs::create_dir(&comic_dir).unwrap();
        for i in 0..10 {
            write_dummy_image(&comic_dir, &format!("page{}.png", i));
        }

        app1.open_comic(comic_dir.clone());
        let ctx = egui::Context::default();
        for _ in 0..100 {
            app1.poll_opener(&ctx);
            if app1.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app1.current_view, View::Reader);
        app1.reader_view.open.as_mut().unwrap().state.current_page = 6;
        app1.record_reader_history();
        app1.store.save_history(&app1.history).unwrap();

        let expected_id = rust_reader_parser::stable_comic_id(&comic_dir);
        let mut app2 = ReaderApp::with_store_dir(store_tmp.path());
        app2.open_comic(comic_dir);
        for _ in 0..100 {
            app2.poll_opener(&ctx);
            if app2.current_view == View::Reader {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let reader = app2
            .reader_view
            .open
            .as_ref()
            .expect("reader should be open");
        assert_eq!(reader.comic.id, expected_id);
        assert_eq!(reader.state.current_page, 6);
    }

    fn should_show_bar(
        show_setting: bool,
        fullscreen: bool,
        mouse_pos: Option<egui::Pos2>,
        screen_size: egui::Vec2,
        edge: BarEdge,
    ) -> bool {
        ReaderApp::should_show_bar(show_setting, fullscreen, mouse_pos, screen_size, edge)
    }

    #[test]
    fn test_bar_hidden_when_setting_off() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(!should_show_bar(
            false,
            false,
            Some(egui::pos2(0.0, 0.0)),
            screen,
            BarEdge::Top
        ));
        assert!(!should_show_bar(
            false,
            true,
            Some(egui::pos2(0.0, 0.0)),
            screen,
            BarEdge::Top
        ));
    }

    #[test]
    fn test_bar_shown_when_not_fullscreen_and_setting_on() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(true, false, None, screen, BarEdge::Top));
        assert!(should_show_bar(
            true,
            false,
            Some(egui::pos2(500.0, 500.0)),
            screen,
            BarEdge::Bottom
        ));
    }

    #[test]
    fn test_top_bar_shown_in_fullscreen_near_top_edge() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 10.0)),
            screen,
            BarEdge::Top
        ));
        assert!(!should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 100.0)),
            screen,
            BarEdge::Top
        ));
    }

    #[test]
    fn test_bottom_bar_shown_in_fullscreen_near_bottom_edge() {
        let screen = egui::vec2(1920.0, 1080.0);
        assert!(should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 1070.0)),
            screen,
            BarEdge::Bottom
        ));
        assert!(!should_show_bar(
            true,
            true,
            Some(egui::pos2(100.0, 500.0)),
            screen,
            BarEdge::Bottom
        ));
    }
}
