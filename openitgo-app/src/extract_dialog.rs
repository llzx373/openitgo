//! 解压目的地与选项对话框：浏览视图「解压全部/解压选中」与库卡片
//! 「解压到…」共用。用户确认后由 app 侧计算输出目录并启动解压任务。

use crate::app::extract_output_base;
use openitgo_storage::models::Settings;
use std::path::PathBuf;

/// 子目录策略（对应 `Settings::extract_wrap` 的 "smart"/"always"/"never"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractWrap {
    /// 智能：包内无单一顶层目录才建与压缩包同名的子目录（Bandizip 语义）。
    Smart,
    /// 总是在目标文件夹下建与压缩包同名的子目录。
    Always,
    /// 直接解压进目标文件夹。
    Never,
}

pub struct ExtractDialogState {
    pub archive: PathBuf,
    /// 选中的条目（None = 全部）。
    pub selection: Option<Vec<String>>,
    /// 流式格式（RAR/7z/TAR）的字节总量估值（浏览视图按选中条目求和）。
    pub total_bytes_hint: Option<u64>,
    /// 目标文件夹（可编辑文本）。
    pub dest: String,
    pub wrap: ExtractWrap,
    pub delete_archive: bool,
    pub open_folder: bool,
    /// 用户点了「取消」或关闭窗口（app 侧据此丢弃对话框）。
    pub cancelled: bool,
}

impl ExtractDialogState {
    pub fn new(
        archive: PathBuf,
        selection: Option<Vec<String>>,
        hint: Option<u64>,
        settings: &Settings,
    ) -> Self {
        Self {
            dest: extract_output_base(settings, &archive)
                .display()
                .to_string(),
            wrap: match settings.extract_wrap.as_str() {
                "always" => ExtractWrap::Always,
                "never" => ExtractWrap::Never,
                _ => ExtractWrap::Smart,
            },
            delete_archive: settings.extract_delete_archive,
            open_folder: settings.extract_open_folder,
            archive,
            selection,
            total_bytes_hint: hint,
            cancelled: false,
        }
    }

    /// 返回 true 表示用户点了「开始解压」；取消置 `cancelled` 由 app 侧清理。
    pub fn ui(&mut self, ctx: &egui::Context) -> bool {
        let name = self
            .archive
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("压缩包")
            .to_string();
        let mut start = false;
        let mut open = true;
        egui::Window::new("解压到")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("解压「{name}」到："));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.dest).desired_width(360.0));
                    if ui.button("浏览…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.dest = dir.display().to_string();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.radio_value(
                    &mut self.wrap,
                    ExtractWrap::Smart,
                    "智能（包内无单一顶层文件夹时才创建同名子文件夹）",
                );
                ui.radio_value(
                    &mut self.wrap,
                    ExtractWrap::Always,
                    "总是创建与压缩包同名的子文件夹",
                );
                ui.radio_value(&mut self.wrap, ExtractWrap::Never, "直接解压进目标文件夹");
                ui.add_space(4.0);
                ui.checkbox(
                    &mut self.delete_archive,
                    "解压完成后删除压缩包（移入回收站）",
                );
                ui.checkbox(&mut self.open_folder, "解压完成后打开目标文件夹");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let dest_ok = !self.dest.trim().is_empty();
                    if ui
                        .add_enabled(dest_ok, egui::Button::new("开始解压"))
                        .clicked()
                    {
                        start = true;
                    }
                    if ui.button("取消").clicked() {
                        self.cancelled = true;
                    }
                });
            });
        if !open {
            self.cancelled = true;
        }
        start
    }
}
