//! 压缩包浏览视图：后台列出全部条目（含目录与非图片），勾选后外抛
//! 解压意图；状态机 Idle → Listing → Ready / NeedPassword / Failed。

use crate::app::{PASSWORD_INCORRECT_MARKER, PASSWORD_REQUIRED_MARKER};
use crate::opener::{AsyncOpener, OpenStatus};
use egui_phosphor_icons::icons;
use openitgo_parser::archive::{list_entries, ArchiveEntry};
use openitgo_parser::traits::ParseError;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArchiveViewState {
    #[default]
    Idle,
    Listing,
    Ready,
    /// 列表遇到 PasswordRequired/Incorrect；点击「输入密码」走密码对话框。
    NeedPassword,
    Failed(String),
}

pub struct ArchiveView {
    pub path: Option<PathBuf>,
    pub entries: Vec<ArchiveEntry>,
    /// 选中的条目名（仅文件条目可勾选）。
    pub selected: HashSet<String>,
    pub filter: String,
    pub state: ArchiveViewState,
    /// open_with_password 使用的密码；列表成功后由 app 取走记入密码本。
    pub tried_password: Option<String>,
    /// 带密码列目录仍遇密码错误：app 据此以 incorrect 复现密码对话框。
    pub password_failed: bool,
    listing: Option<AsyncOpener<Vec<ArchiveEntry>>>,
}

impl Default for ArchiveView {
    fn default() -> Self {
        Self {
            path: None,
            entries: Vec::new(),
            selected: HashSet::new(),
            filter: String::new(),
            state: ArchiveViewState::Idle,
            tried_password: None,
            password_failed: false,
            listing: None,
        }
    }
}

pub struct ArchiveCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    pub on_extract_all: &'a mut dyn FnMut(),
    pub on_extract_selected: &'a mut dyn FnMut(Vec<String>),
    /// NeedPassword 状态下点击「输入密码」。
    pub on_need_password: &'a mut dyn FnMut(),
}

impl ArchiveView {
    /// 后台列出条目（不带密码；加密包落入 NeedPassword 状态）。
    pub fn open(&mut self, path: PathBuf) {
        self.path = Some(path.clone());
        self.entries.clear();
        self.selected.clear();
        self.filter.clear();
        self.state = ArchiveViewState::Listing;
        self.tried_password = None;
        self.password_failed = false;
        self.listing = Some(AsyncOpener::open(path, |p| {
            list_entries(p, None).map_err(|e| match e {
                ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                other => other.to_string(),
            })
        }));
    }

    /// 带密码重跑列目录（密码对话框确认后调用）。
    pub fn open_with_password(&mut self, path: PathBuf, password: String) {
        self.path = Some(path.clone());
        self.entries.clear();
        self.selected.clear();
        self.filter.clear();
        self.state = ArchiveViewState::Listing;
        self.password_failed = false;
        self.tried_password = Some(password.clone());
        self.listing = Some(AsyncOpener::open(path, move |p| {
            list_entries(p, Some(&password)).map_err(|e| match e {
                ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                other => other.to_string(),
            })
        }));
    }

    /// 每帧排空列表结果；视图不处于前台时结果留在通道里，下次 open 直接替换。
    pub fn poll(&mut self) {
        let Some(mut listing) = self.listing.take() else {
            return;
        };
        match listing.poll() {
            OpenStatus::Loading => self.listing = Some(listing),
            OpenStatus::Ready(result) => self.apply_listing_result(result),
        }
    }

    fn apply_listing_result(&mut self, result: Result<Vec<ArchiveEntry>, String>) {
        match result {
            Ok(entries) => {
                self.entries = entries;
                self.state = ArchiveViewState::Ready;
                self.password_failed = false;
            }
            Err(e) if e == PASSWORD_REQUIRED_MARKER || e == PASSWORD_INCORRECT_MARKER => {
                // 带密码尝试后仍失败 = 密码错误，对话框以 incorrect 复现。
                self.password_failed = self.tried_password.is_some();
                self.state = ArchiveViewState::NeedPassword;
            }
            Err(e) => self.state = ArchiveViewState::Failed(e),
        }
    }

    /// 当前过滤条件下的可见条目索引（大小写不敏感子串匹配）。
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| needle.is_empty() || e.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// 选中文件条目数与总大小（解压后字节）。
    fn selected_stats(&self) -> (usize, u64) {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && self.selected.contains(&e.name))
            .fold((0, 0), |(n, bytes), e| (n + 1, bytes + e.size))
    }

    /// 全选：选中当前过滤条件下的全部文件条目。
    fn select_all_visible_files(&mut self) {
        for idx in self.visible_indices() {
            let entry = &self.entries[idx];
            if !entry.is_dir {
                self.selected.insert(entry.name.clone());
            }
        }
    }

    /// 选中的条目名，按包内顺序排列。
    fn selected_names_in_order(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, callbacks: ArchiveCallbacks<'_>) {
        let ArchiveCallbacks {
            on_back,
            on_extract_all,
            on_extract_selected,
            on_need_password,
        } = callbacks;

        ui.horizontal(|ui| {
            if ui
                .button((icons::HOUSE, " 书架"))
                .on_hover_text("返回书架")
                .clicked()
            {
                on_back();
            }
            ui.separator();
            if let Some(path) = &self.path {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.label(egui::RichText::new(name).strong())
                    .on_hover_text(path.display().to_string());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("过滤条目")
                        .desired_width(160.0),
                );
                if self.state == ArchiveViewState::Ready {
                    if ui.button("清空选中").clicked() {
                        self.selected.clear();
                    }
                    if ui.button("全选").clicked() {
                        self.select_all_visible_files();
                    }
                }
            });
        });
        ui.separator();

        let ready = self.state == ArchiveViewState::Ready;
        if ready {
            egui::Panel::bottom("archive_bottom_bar").show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let file_count = self.entries.iter().filter(|e| !e.is_dir).count();
                    let (selected_count, selected_bytes) = self.selected_stats();
                    ui.label(format!("共 {file_count} 个文件"));
                    ui.separator();
                    ui.label(format!(
                        "已选 {selected_count} 项 · {}",
                        human_size(selected_bytes)
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button((icons::EXPORT, " 解压全部"))
                            .on_hover_text("解压到包同目录的同名子目录")
                            .clicked()
                        {
                            on_extract_all();
                        }
                        let selected_button =
                            egui::Button::new(format!("解压选中 ({selected_count})"));
                        if ui
                            .add_enabled(selected_count > 0, selected_button)
                            .clicked()
                        {
                            on_extract_selected(self.selected_names_in_order());
                        }
                    });
                });
                ui.add_space(4.0);
            });
        }

        match &self.state {
            ArchiveViewState::Idle => {}
            ArchiveViewState::Listing => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("⏳").size(24.0));
                    ui.label("正在读取压缩包…");
                });
            }
            ArchiveViewState::NeedPassword => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::LOCK.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("该压缩包已加密，需要密码").size(16.0));
                    ui.label(egui::RichText::new("输入密码后可浏览条目并解压").weak());
                    ui.add_space(8.0);
                    if ui.button((icons::KEY, " 输入密码")).clicked() {
                        on_need_password();
                    }
                });
            }
            ArchiveViewState::Failed(message) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::WARNING.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("无法读取压缩包: {message}"),
                    );
                });
            }
            ArchiveViewState::Ready => {
                self.render_entry_list(ui);
            }
        }
    }

    fn render_entry_list(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(egui::RichText::new("没有匹配的条目").weak());
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for idx in visible {
                    let entry = &self.entries[idx];
                    let is_dir = entry.is_dir;
                    let name = entry.name.clone();
                    let checked = self.selected.contains(&name);
                    ui.horizontal(|ui| {
                        if is_dir {
                            // 目录条目不可勾选，置灰展示。
                            ui.add_enabled_ui(false, |ui| {
                                ui.checkbox(&mut false, "");
                            });
                            ui.label(egui::RichText::new(icons::FOLDER.as_str()).weak());
                            ui.label(egui::RichText::new(&name).weak());
                        } else {
                            let mut now = checked;
                            if ui.checkbox(&mut now, "").clicked() {
                                if now {
                                    self.selected.insert(name.clone());
                                } else {
                                    self.selected.remove(&name);
                                }
                            }
                            ui.label(icons::FILE);
                            ui.label(&name);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(human_size(entry.size)).weak());
                                    if let Some(compressed) = entry.compressed_size {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "压缩 {}",
                                                human_size(compressed)
                                            ))
                                            .weak(),
                                        );
                                    }
                                },
                            );
                        }
                    });
                }
            });
    }
}

/// 人类可读大小：B / KB / MB / GB。
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size,
            compressed_size: None,
        }
    }

    #[test]
    fn apply_listing_result_state_transitions() {
        let mut view = ArchiveView::default();
        view.apply_listing_result(Ok(vec![entry("a.txt", false, 1)]));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert_eq!(view.entries.len(), 1);

        view.apply_listing_result(Err(PASSWORD_REQUIRED_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);

        view.apply_listing_result(Err("IO error: x".to_string()));
        assert_eq!(
            view.state,
            ArchiveViewState::Failed("IO error: x".to_string())
        );
    }

    #[test]
    fn apply_listing_result_tracks_password_attempt_outcome() {
        let mut view = ArchiveView::default();
        // 无密码尝试时的密码错误：不触发 incorrect 复现。
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(!view.password_failed);
        // 带密码尝试后仍失败：password_failed 置位，tried_password 保留。
        view.tried_password = Some("pw".to_string());
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(view.password_failed);
        // 带密码尝试成功：Ready 且 password_failed 清除。
        view.apply_listing_result(Ok(vec![entry("a.txt", false, 1)]));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert!(!view.password_failed);
        assert_eq!(view.tried_password.as_deref(), Some("pw"));
    }

    #[test]
    fn visible_indices_filter_is_case_insensitive() {
        let mut view = ArchiveView {
            entries: vec![
                entry("Dir/", true, 0),
                entry("Page01.PNG", false, 10),
                entry("notes.md", false, 5),
            ],
            ..Default::default()
        };
        assert_eq!(view.visible_indices(), vec![0, 1, 2]);
        view.filter = "png".to_string();
        assert_eq!(view.visible_indices(), vec![1]);
        view.filter = " PAGE ".to_string();
        assert_eq!(view.visible_indices(), vec![1]);
    }

    #[test]
    fn select_all_visible_files_skips_directories() {
        let mut view = ArchiveView {
            entries: vec![
                entry("dir/", true, 0),
                entry("dir/b.png", false, 9),
                entry("a.txt", false, 7),
            ],
            ..Default::default()
        };
        view.select_all_visible_files();
        assert_eq!(view.selected.len(), 2);
        assert!(!view.selected.contains("dir/"));
        // 过滤后全选只选可见文件。
        view.selected.clear();
        view.filter = "png".to_string();
        view.select_all_visible_files();
        assert_eq!(view.selected.len(), 1);
        assert!(view.selected.contains("dir/b.png"));
    }

    #[test]
    fn selected_stats_and_order() {
        let mut view = ArchiveView {
            entries: vec![
                entry("z.png", false, 3),
                entry("dir/", true, 0),
                entry("a.png", false, 4),
            ],
            ..Default::default()
        };
        view.selected.insert("a.png".to_string());
        view.selected.insert("z.png".to_string());
        assert_eq!(view.selected_stats(), (2, 7));
        // 按包内顺序而非插入顺序。
        assert_eq!(view.selected_names_in_order(), vec!["z.png", "a.png"]);
    }

    #[test]
    fn human_size_formats_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
