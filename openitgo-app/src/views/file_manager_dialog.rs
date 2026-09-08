//! 文件管理器操作对话框：复制/移动确认、删除确认（防误删 + 「不再询问」）、
//! 重命名、新建文件夹。对齐 `extract_dialog.rs` 模式：状态 struct + 每帧
//! `ui(ctx)`，`None` = 仍开着，`Some(FmDialogOutcome)` = 本帧关闭
//! （确认或取消），由 FileManagerView 统一消费。全部 UI 文本中文。

use crate::views::archive::human_size;
use crate::views::file_ops::{validate_entry_name, ConflictMode, OpKind};
use std::path::{Path, PathBuf};

/// 对话框统一入口。
pub enum FmDialog {
    CopyMove(CopyMoveDialog),
    Delete(DeleteDialog),
    Rename(RenameDialog),
    NewDir(NewDirDialog),
}

/// 对话框关闭结果（确认携带全部执行参数；取消为 Cancelled）。
pub enum FmDialogOutcome {
    Cancelled,
    ConfirmCopyMove {
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        conflict: ConflictMode,
    },
    ConfirmDelete {
        sources: Vec<PathBuf>,
        /// 「不再询问」勾选状态（阶段五写回 settings.fm_confirm_delete）。
        dont_ask_again: bool,
    },
    ConfirmRename {
        path: PathBuf,
        new_name: String,
    },
    ConfirmNewDir {
        parent: PathBuf,
        name: String,
    },
}

impl FmDialog {
    /// 每帧渲染；返回 Some 表示本帧关闭（确认/取消/窗口 X）。
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        match self {
            FmDialog::CopyMove(d) => d.ui(ctx),
            FmDialog::Delete(d) => d.ui(ctx),
            FmDialog::Rename(d) => d.ui(ctx),
            FmDialog::NewDir(d) => d.ui(ctx),
        }
    }
}

/// 源项列表（>10 项折叠为前 10 项 + 「…共 N 项」）。
fn render_source_list(ui: &mut egui::Ui, sources: &[PathBuf]) {
    const MAX_LISTED: usize = 10;
    for path in sources.iter().take(MAX_LISTED) {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        ui.label(format!("  {name}"))
            .on_hover_text(path.display().to_string());
    }
    if sources.len() > MAX_LISTED {
        ui.label(format!("  …共 {} 项", sources.len()));
    }
}

fn sources_summary(sources: &[PathBuf]) -> String {
    let total: u64 = sources
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum();
    format!("{} 项 · {}", sources.len(), human_size(total))
}

/// 复制/移动确认：源列表 + 目标路径（默认非焦点栏目录）+ 冲突策略。
pub struct CopyMoveDialog {
    kind: OpKind,
    sources: Vec<PathBuf>,
    dest: String,
    conflict: ConflictMode,
}

impl CopyMoveDialog {
    pub fn new(kind: OpKind, sources: Vec<PathBuf>, dest: &Path) -> Self {
        Self {
            kind,
            sources,
            dest: dest.display().to_string(),
            // 默认「自动改名」（对齐 extract_overwrite=false 的语义）。
            conflict: ConflictMode::AutoRename,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let title = match self.kind {
            OpKind::Copy => "复制到",
            OpKind::Move => "移动到",
            OpKind::Delete => unreachable!("Delete 走 DeleteDialog"),
        };
        let mut outcome = None;
        let mut open = true;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{}以下内容：",
                    match self.kind {
                        OpKind::Copy => "复制",
                        OpKind::Move => "移动",
                        OpKind::Delete => unreachable!(),
                    }
                ));
                render_source_list(ui, &self.sources);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("目标文件夹：");
                    ui.add(egui::TextEdit::singleline(&mut self.dest).desired_width(360.0));
                    if ui.button("浏览…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.dest = dir.display().to_string();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("同名冲突：");
                    egui::ComboBox::from_id_salt("fm_conflict")
                        .selected_text(conflict_label(self.conflict))
                        .show_ui(ui, |ui| {
                            for mode in [
                                ConflictMode::AutoRename,
                                ConflictMode::Overwrite,
                                ConflictMode::Skip,
                                ConflictMode::Ask,
                            ] {
                                ui.selectable_value(&mut self.conflict, mode, conflict_label(mode));
                            }
                        });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let dest = PathBuf::from(self.dest.trim());
                    let dest_ok = !self.dest.trim().is_empty() && dest.is_dir();
                    if !self.dest.trim().is_empty() && !dest_ok {
                        ui.colored_label(ui.visuals().error_fg_color, "目标文件夹不存在");
                    }
                    if ui.add_enabled(dest_ok, egui::Button::new(title)).clicked() {
                        outcome = Some(FmDialogOutcome::ConfirmCopyMove {
                            kind: self.kind,
                            sources: self.sources.clone(),
                            dest,
                            conflict: self.conflict,
                        });
                    }
                    if ui.button("取消").clicked() {
                        outcome = Some(FmDialogOutcome::Cancelled);
                    }
                });
            });
        if !open {
            outcome = Some(FmDialogOutcome::Cancelled);
        }
        outcome
    }
}

fn conflict_label(mode: ConflictMode) -> &'static str {
    match mode {
        ConflictMode::AutoRename => "自动改名（保留两者）",
        ConflictMode::Overwrite => "覆盖",
        ConflictMode::Skip => "跳过",
        ConflictMode::Ask => "询问（执行期按跳过处理）",
    }
}

/// 删除确认（防误删）：待删项列表 + 回收站说明 + 「不再询问」。
pub struct DeleteDialog {
    sources: Vec<PathBuf>,
    dont_ask_again: bool,
}

impl DeleteDialog {
    pub fn new(sources: Vec<PathBuf>) -> Self {
        Self {
            sources,
            dont_ask_again: false,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("删除确认")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "确定删除以下 {} 吗？",
                    sources_summary(&self.sources)
                ));
                render_source_list(ui, &self.sources);
                ui.add_space(4.0);
                ui.label(egui::RichText::new("将移入回收站，可从系统回收站恢复。").weak());
                ui.checkbox(&mut self.dont_ask_again, "不再询问（设置中可改回）");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let confirm = egui::Button::new(
                        egui::RichText::new("移入回收站").color(egui::Color32::WHITE),
                    )
                    .fill(ui.visuals().error_fg_color);
                    if ui.add(confirm).clicked() {
                        outcome = Some(FmDialogOutcome::ConfirmDelete {
                            sources: self.sources.clone(),
                            dont_ask_again: self.dont_ask_again,
                        });
                    }
                    if ui.button("取消").clicked() {
                        outcome = Some(FmDialogOutcome::Cancelled);
                    }
                });
            });
        if !open {
            outcome = Some(FmDialogOutcome::Cancelled);
        }
        outcome
    }
}

/// 重命名：单行文本框预填现名；非法字符/重名实时红字校验。
/// （egui 无公开的选区 API，basename 选中从略。）
pub struct RenameDialog {
    path: PathBuf,
    name: String,
}

impl RenameDialog {
    pub fn new(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Self { path, name }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("重命名")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("新名称：");
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(320.0));
                let error = name_error(self.path.parent(), &self.name, Some(&self.path));
                if let Some(err) = &error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("重命名"))
                        .clicked()
                    {
                        outcome = Some(FmDialogOutcome::ConfirmRename {
                            path: self.path.clone(),
                            new_name: self.name.trim().to_string(),
                        });
                    }
                    if ui.button("取消").clicked() {
                        outcome = Some(FmDialogOutcome::Cancelled);
                    }
                });
            });
        if !open {
            outcome = Some(FmDialogOutcome::Cancelled);
        }
        outcome
    }
}

/// 新建文件夹：默认名「新建文件夹」（重名自动递增建议）。
pub struct NewDirDialog {
    parent: PathBuf,
    name: String,
}

impl NewDirDialog {
    pub fn new(parent: PathBuf, suggested: String) -> Self {
        Self {
            parent,
            name: suggested,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("新建文件夹")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("文件夹名称：");
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(320.0));
                let error = name_error(Some(&self.parent), &self.name, None);
                if let Some(err) = &error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("创建"))
                        .clicked()
                    {
                        outcome = Some(FmDialogOutcome::ConfirmNewDir {
                            parent: self.parent.clone(),
                            name: self.name.trim().to_string(),
                        });
                    }
                    if ui.button("取消").clicked() {
                        outcome = Some(FmDialogOutcome::Cancelled);
                    }
                });
            });
        if !open {
            outcome = Some(FmDialogOutcome::Cancelled);
        }
        outcome
    }
}

/// 名称实时校验：非法字符 + 同目录重名（重命名时排除自身）。
fn name_error(parent: Option<&Path>, name: &str, exclude: Option<&Path>) -> Option<String> {
    if let Err(e) = validate_entry_name(name) {
        return Some(e);
    }
    if let Some(parent) = parent {
        let candidate = parent.join(name.trim());
        if Some(candidate.as_path()) != exclude && candidate.exists() {
            return Some("已存在同名文件或文件夹".to_string());
        }
    }
    None
}
