//! 文件管理器操作对话框：复制/移动确认、删除确认（防误删 + 「不再询问」）、
//! 重命名、新建文件夹/文本文件、选择组（通配模式选择/取消选择）。对齐
//! `extract_dialog.rs` 模式：状态 struct + 每帧 `ui(ctx)`，`None` = 仍开着，
//! `Some(FmDialogOutcome)` = 本帧关闭（确认或取消），由 FileManagerView
//! 统一消费。全部 UI 文本中文。

use crate::views::archive::{format_mtime, human_size};
use crate::views::file_manager_rename::{plan_renames, CounterRule, RenamePlan, RenameRule};
use crate::views::file_ops::{
    resolve_conflict_name, validate_entry_name, verbatim_path, ConflictAction, ConflictAnswer,
    ConflictMode, ConflictQuery, OpKind,
};
use std::path::{Path, PathBuf};

/// 对话框统一入口。
pub enum FmDialog {
    CopyMove(CopyMoveDialog),
    Delete(DeleteDialog),
    Rename(RenameDialog),
    NewDir(NewDirDialog),
    NewFile(NewFileDialog),
    Compress(CompressDialog),
    SelectGroup(SelectGroupDialog),
    MultiRename(MultiRenameDialog),
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
    ConfirmNewFile {
        parent: PathBuf,
        name: String,
    },
    ConfirmCompress {
        sources: Vec<PathBuf>,
        dest_zip: PathBuf,
    },
    /// 「选择组」确认：通配模式 + 方向（select=true 选择 / false 取消选择）。
    SelectGroup {
        pattern: String,
        select: bool,
        files_only: bool,
    },
    /// 批量重命名确认：仅含可执行（非 skip 且无 error）的计划。
    ConfirmMultiRename {
        plans: Vec<RenamePlan>,
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
            FmDialog::NewFile(d) => d.ui(ctx),
            FmDialog::Compress(d) => d.ui(ctx),
            FmDialog::SelectGroup(d) => d.ui(ctx),
            FmDialog::MultiRename(d) => d.ui(ctx),
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
            OpKind::Delete | OpKind::Compress => unreachable!("Delete/Compress 各有对话框"),
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
                        OpKind::Delete | OpKind::Compress => unreachable!(),
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
        ConflictMode::Ask => "逐个询问",
    }
}

/// 执行期同名冲突问答（ConflictMode::Ask「逐个询问」的弹窗）：worker 阻塞
/// 等答，窗口无关闭按钮——必须显式选择（整体取消走状态栏「取消」，经
/// cancel 旗标让 worker 按 Cancel 收拢）。目录冲突只给「合并/跳过」。
pub struct ConflictDialog {
    query: ConflictQuery,
    apply_all: bool,
}

impl ConflictDialog {
    pub fn new(query: ConflictQuery) -> Self {
        Self {
            query,
            apply_all: false,
        }
    }

    /// 每帧渲染；Some(answer) = 用户已选择（回发 worker）。
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<ConflictAnswer> {
        let mut out = None;
        egui::Window::new("同名冲突")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let q = &self.query;
                if q.is_dir {
                    ui.label("目标已存在同名文件夹：");
                } else {
                    ui.label("目标已存在同名文件：");
                }
                ui.add_space(4.0);
                let name = q
                    .src
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                egui::Grid::new("fm_conflict_cmp")
                    .num_columns(3)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("源：").weak());
                        ui.label(&name).on_hover_text(q.src.display().to_string());
                        ui.label(entry_detail(q.src_size, q.src_mtime));
                        ui.end_row();
                        ui.label(egui::RichText::new("目标：").weak());
                        ui.label(&name).on_hover_text(q.dst.display().to_string());
                        ui.label(entry_detail(q.dst_size, q.dst_mtime));
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let mut pick = |action: ConflictAction, apply_all: bool| {
                        out = Some(ConflictAnswer { action, apply_all });
                    };
                    if q.is_dir {
                        if ui.button("合并").clicked() {
                            pick(ConflictAction::Overwrite, self.apply_all);
                        }
                        if ui.button("跳过").clicked() {
                            pick(ConflictAction::Skip, self.apply_all);
                        }
                    } else {
                        if ui.button("覆盖").clicked() {
                            pick(ConflictAction::Overwrite, self.apply_all);
                        }
                        if ui.button("跳过").clicked() {
                            pick(ConflictAction::Skip, self.apply_all);
                        }
                        if ui.button("自动改名").clicked() {
                            pick(ConflictAction::AutoRename, self.apply_all);
                        }
                    }
                    ui.checkbox(&mut self.apply_all, "本次操作全部应用");
                    if ui.button("取消操作").clicked() {
                        // Cancel 无 apply_all（取消恒作用整批）。
                        pick(ConflictAction::Cancel, false);
                    }
                });
            });
        out
    }
}

/// 冲突对比行的「大小 · 时间」详情（目录/元数据缺失相应留空）。
fn entry_detail(size: Option<u64>, mtime: Option<std::time::SystemTime>) -> String {
    let mut parts = Vec::new();
    if let Some(size) = size {
        parts.push(human_size(size));
    }
    let ts = mtime
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let mtime = format_mtime(ts);
    if !mtime.is_empty() {
        parts.push(mtime);
    }
    parts.join(" · ")
}

/// 压缩为 zip：目标目录（默认非焦点栏目录）+ 文件名（默认首个 source
/// basename.zip）；确认时目标已存在自动改名 "name (1).zip"。
pub struct CompressDialog {
    sources: Vec<PathBuf>,
    dest_dir: String,
    name: String,
}

impl CompressDialog {
    pub fn new(sources: Vec<PathBuf>, dest_dir: &Path) -> Self {
        let name = sources
            .first()
            .and_then(|p| p.file_name())
            .map(|s| format!("{}.zip", s.to_string_lossy()))
            .unwrap_or_else(|| "archive.zip".to_string());
        Self {
            sources,
            dest_dir: dest_dir.display().to_string(),
            name,
        }
    }

    /// 目标 zip 完整路径：文件名缺 .zip 后缀时自动补上。
    fn dest_zip(&self) -> PathBuf {
        let name = self.name.trim();
        let name = if name.to_ascii_lowercase().ends_with(".zip") {
            name.to_string()
        } else {
            format!("{name}.zip")
        };
        PathBuf::from(self.dest_dir.trim()).join(name)
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("压缩为 zip")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("压缩以下内容：");
                render_source_list(ui, &self.sources);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("目标文件夹：");
                    ui.add(egui::TextEdit::singleline(&mut self.dest_dir).desired_width(360.0));
                    if ui.button("浏览…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.dest_dir = dir.display().to_string();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("文件名：");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(240.0));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let dest_dir = PathBuf::from(self.dest_dir.trim());
                    let dir_ok = !self.dest_dir.trim().is_empty() && dest_dir.is_dir();
                    let name_err = validate_entry_name(self.name.trim()).err();
                    if !self.dest_dir.trim().is_empty() && !dir_ok {
                        ui.colored_label(ui.visuals().error_fg_color, "目标文件夹不存在");
                    }
                    if let Some(err) = &name_err {
                        ui.colored_label(ui.visuals().error_fg_color, err);
                    }
                    let dest_zip = self.dest_zip();
                    let dest_zip = resolve_conflict_name(&dest_zip);
                    if dir_ok && name_err.is_none() && dest_zip != self.dest_zip() {
                        let renamed = dest_zip
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.label(
                            egui::RichText::new(format!("已存在同名文件，将保存为「{renamed}」"))
                                .weak(),
                        );
                    }
                    if ui
                        .add_enabled(dir_ok && name_err.is_none(), egui::Button::new("压缩"))
                        .clicked()
                    {
                        outcome = Some(FmDialogOutcome::ConfirmCompress {
                            sources: self.sources.clone(),
                            dest_zip,
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

/// 新建文本文件：默认名「新建文本文件.txt」（重名自动递增建议），
/// 确认后创建空文件并选中。
pub struct NewFileDialog {
    parent: PathBuf,
    name: String,
}

impl NewFileDialog {
    pub fn new(parent: PathBuf, suggested: String) -> Self {
        Self {
            parent,
            name: suggested,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("新建文本文件")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("文件名称：");
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
                        outcome = Some(FmDialogOutcome::ConfirmNewFile {
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

/// 「选择组」对话框（TC 语义）：通配模式（`;` 分隔多模式，`*`/`?`，
/// 不区分大小写）选择/取消选择当前可见行。deselect 预置回车的默认动作
/// （`+` 打开 = 选择，`-` 打开 = 取消选择）。
pub struct SelectGroupDialog {
    pattern: String,
    files_only: bool,
    deselect: bool,
    /// 首帧自动聚焦模式输入框（request_focus 只需一次）。
    focused: bool,
}

impl SelectGroupDialog {
    pub fn new(pattern: String, deselect: bool) -> Self {
        Self {
            pattern,
            files_only: false,
            deselect,
            focused: false,
        }
    }

    fn confirm(&self, select: bool) -> FmDialogOutcome {
        FmDialogOutcome::SelectGroup {
            pattern: self.pattern.trim().to_string(),
            select,
            files_only: self.files_only,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new("选择组")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("模式：");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.pattern)
                            .hint_text("如 *.zip;EP*")
                            .desired_width(280.0),
                    );
                    if !self.focused {
                        response.request_focus();
                        self.focused = true;
                    }
                    // 回车 = 执行预置动作（单行输入框 Enter 自动失焦）。
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        outcome = Some(self.confirm(!self.deselect));
                    }
                });
                ui.checkbox(&mut self.files_only, "仅文件");
                ui.label(egui::RichText::new("支持 * ? 通配，; 分隔多模式，不区分大小写").weak());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("选择").clicked() {
                        outcome = Some(self.confirm(true));
                    }
                    if ui.button("取消选择").clicked() {
                        outcome = Some(self.confirm(false));
                    }
                    if ui.button("关闭").clicked() {
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

/// 批量重命名（Ctrl+M，TC Multi-Rename 简化版）：查找替换 + 计数器 + 模板，
/// 实时预览全部结果；仅当无任何错误且至少一项可执行时可确认。
pub struct MultiRenameDialog {
    /// 待重命名项（路径, 是否目录）。
    items: Vec<(PathBuf, bool)>,
    search: String,
    replace: String,
    counter_enabled: bool,
    counter_start: i32,
    counter_step: i32,
    counter_pad: usize,
    template: String,
}

impl MultiRenameDialog {
    pub fn new(items: Vec<(PathBuf, bool)>) -> Self {
        Self {
            items,
            search: String::new(),
            replace: String::new(),
            counter_enabled: false,
            counter_start: 1,
            counter_step: 1,
            counter_pad: 1,
            template: "[O][E]".to_string(),
        }
    }

    fn rule(&self) -> RenameRule {
        RenameRule {
            search: self.search.clone(),
            replace: self.replace.clone(),
            counter: self.counter_enabled.then_some(CounterRule {
                start: self.counter_start,
                step: self.counter_step,
                pad: self.counter_pad,
            }),
            template: self.template.clone(),
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> Option<FmDialogOutcome> {
        let mut outcome = None;
        let mut open = true;
        egui::Window::new(format!("批量重命名（{} 项）", self.items.len()))
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(520.0, 420.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("查找：");
                    ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(140.0));
                    ui.label("替换为：");
                    ui.add(egui::TextEdit::singleline(&mut self.replace).desired_width(140.0));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.counter_enabled, "计数器：");
                    ui.add_enabled(
                        self.counter_enabled,
                        egui::DragValue::new(&mut self.counter_start).prefix("起始 "),
                    );
                    ui.add_enabled(
                        self.counter_enabled,
                        egui::DragValue::new(&mut self.counter_step).prefix("步长 "),
                    );
                    ui.add_enabled(
                        self.counter_enabled,
                        egui::DragValue::new(&mut self.counter_pad)
                            .range(0..=8)
                            .prefix("补零 "),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("模板：");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.template)
                            .hint_text("[O][E]")
                            .desired_width(280.0),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "[O] 原名（查找替换后）　[E] 扩展名　[C] 计数器　其余字符原样输出",
                    )
                    .weak(),
                );
                ui.add_space(4.0);

                let plans = plan_renames(&self.items, &self.rule(), |p| verbatim_path(p).exists());
                let runnable = plans
                    .iter()
                    .filter(|p| !p.skip && p.error.is_none())
                    .count();
                let errors = plans.iter().filter(|p| p.error.is_some()).count();

                egui::ScrollArea::vertical()
                    .id_salt("fm_multi_rename_preview")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("fm_multi_rename_grid")
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                for plan in &plans {
                                    let old = plan
                                        .src
                                        .file_name()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    ui.label(&old).on_hover_text(plan.src.display().to_string());
                                    ui.label("→");
                                    if let Some(err) = &plan.error {
                                        ui.colored_label(ui.visuals().error_fg_color, err);
                                    } else if plan.skip {
                                        ui.label(egui::RichText::new(&plan.dst_name).weak());
                                    } else {
                                        ui.label(&plan.dst_name);
                                    }
                                    ui.end_row();
                                }
                            });
                    });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ok = runnable > 0 && errors == 0;
                    if errors > 0 {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("{errors} 项存在冲突或非法名称"),
                        );
                    }
                    if ui
                        .add_enabled(ok, egui::Button::new(format!("重命名 {runnable} 项")))
                        .clicked()
                    {
                        let plans = plans
                            .into_iter()
                            .filter(|p| !p.skip && p.error.is_none())
                            .collect();
                        outcome = Some(FmDialogOutcome::ConfirmMultiRename { plans });
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
