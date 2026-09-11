//! 「修改属性/时间戳…」对话框（阶段 AC）：应用内可编辑版属性框（区别于
//! Alt+Enter 的系统属性框）。属性位 = 只读/隐藏/系统/存档（后三者仅
//! Windows 显示；unix 只有只读位有效），读写走 `platform::file_attr`
//! （Windows `GetFileAttributesW`/`SetFileAttributesW`，其余位保留）；
//! 时间戳 = 修改/访问时间各自可勾选「设为指定时间」（文本
//! `YYYY-MM-DD HH:MM:SS`，本地时区解析，纯函数 `parse_datetime_local`）
//! 或「设为当前时间」，写入走 `filetime`（已在依赖树 ← tar，跨平台）。
//! 对话框本身不做 IO：`ui()` 返回 `AttrAction::Apply` 由 FileManagerView
//! 逐项应用并刷新栏（同 comment_dialog 模式），失败回传 error 保持打开。

use crate::platform::file_attr::FileAttrBits;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// 日期时间格式（本地时区）：`YYYY-MM-DD HH:MM:SS`。
const DATETIME_FMT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// 本地时区偏移；取不到（unix 多线程限制等）回退 UTC。
fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// 解析 `YYYY-MM-DD HH:MM:SS`（本地时区）为 SystemTime（纯函数）。
pub fn parse_datetime_local(text: &str) -> Option<SystemTime> {
    let dt = PrimitiveDateTime::parse(text.trim(), DATETIME_FMT).ok()?;
    Some(dt.assume_offset(local_offset()).into())
}

/// 反向格式化（预填输入框用；与 parse_datetime_local 同一时区口径）。
pub fn format_datetime_local(t: SystemTime) -> String {
    let dt: OffsetDateTime = t.into();
    dt.to_offset(local_offset())
        .format(DATETIME_FMT)
        .unwrap_or_default()
}

/// 对话框动作（FileManagerView 消费）。
pub enum AttrAction {
    /// 关闭（取消/窗口 X/应用成功由视图判）。
    Close,
    /// 确认：属性位 + 两个可选时间戳。
    Apply {
        bits: FileAttrBits,
        mtime: Option<SystemTime>,
        atime: Option<SystemTime>,
    },
}

/// 「修改属性/时间戳…」对话框状态（FileManagerView 以 Option 持有，
/// comment_dialog 同款非模态 egui::Window 模式）。
pub struct AttrTimestampDialog {
    paths: Vec<PathBuf>,
    bits: FileAttrBits,
    mtime_enabled: bool,
    mtime_text: String,
    atime_enabled: bool,
    atime_text: String,
    /// 应用失败回显（视图侧写入）。
    pub error: Option<String>,
}

impl AttrTimestampDialog {
    /// 打开：属性位与时间输入框预填第一个路径的现值（查不到用默认）。
    pub fn new(paths: Vec<PathBuf>) -> Self {
        let first = paths.first().map(PathBuf::as_path);
        let bits = first
            .and_then(|p| crate::platform::file_attr::query_attributes(p).ok())
            .unwrap_or_default();
        let meta = first.and_then(|p| std::fs::metadata(p).ok());
        let mtime_text = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(format_datetime_local)
            .unwrap_or_default();
        let atime_text = meta
            .and_then(|m| m.accessed().ok())
            .map(format_datetime_local)
            .unwrap_or_default();
        Self {
            paths,
            bits,
            mtime_enabled: false,
            mtime_text,
            atime_enabled: false,
            atime_text,
            error: None,
        }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// 渲染；返回 Some 表示本帧有关闭/确认动作。
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<AttrAction> {
        let mut open = true;
        let mut action: Option<AttrAction> = None;
        egui::Window::new("修改属性/时间戳")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                action = self.ui_body(ui);
            });
        if !open && action.is_none() {
            action = Some(AttrAction::Close);
        }
        action
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) -> Option<AttrAction> {
        // 目标摘要（>5 项折叠）。
        ui.label(format!("共 {} 项：", self.paths.len()));
        for p in self.paths.iter().take(5) {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            ui.label(format!("  {name}"))
                .on_hover_text(p.display().to_string());
        }
        if self.paths.len() > 5 {
            ui.label(format!("  …共 {} 项", self.paths.len()));
        }
        ui.add_space(6.0);

        // 属性位。
        ui.label(egui::RichText::new("属性").strong());
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.bits.readonly, "只读");
            // 隐藏/系统/存档为 Windows 属性位（unix stub 忽略，不显示）。
            if cfg!(windows) {
                ui.checkbox(&mut self.bits.hidden, "隐藏");
                ui.checkbox(&mut self.bits.system, "系统");
                ui.checkbox(&mut self.bits.archive, "存档");
            }
        });
        ui.add_space(6.0);

        // 时间戳。
        ui.label(egui::RichText::new("时间戳").strong());
        egui::Grid::new("fm-attr-times")
            .num_columns(3)
            .show(ui, |ui| {
                ui.checkbox(&mut self.mtime_enabled, "修改时间:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.mtime_text)
                        .hint_text("YYYY-MM-DD HH:MM:SS")
                        .desired_width(180.0),
                );
                if ui.button("设为当前时间").clicked() {
                    let now = format_datetime_local(SystemTime::now());
                    self.mtime_text = now;
                    self.mtime_enabled = true;
                }
                ui.end_row();
                ui.checkbox(&mut self.atime_enabled, "访问时间:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.atime_text)
                        .hint_text("YYYY-MM-DD HH:MM:SS")
                        .desired_width(180.0),
                );
                if ui.button("设为当前时间").clicked() {
                    let now = format_datetime_local(SystemTime::now());
                    self.atime_text = now;
                    self.atime_enabled = true;
                }
                ui.end_row();
            });

        // 解析校验（确认前拦下，不落任何写）。
        let mtime = if self.mtime_enabled {
            parse_datetime_local(&self.mtime_text)
        } else {
            None
        };
        let atime = if self.atime_enabled {
            parse_datetime_local(&self.atime_text)
        } else {
            None
        };
        let mut parse_error: Option<&str> = None;
        if self.mtime_enabled && mtime.is_none() {
            parse_error = Some("修改时间格式无效（YYYY-MM-DD HH:MM:SS）");
        }
        if self.atime_enabled && atime.is_none() {
            parse_error = Some("访问时间格式无效（YYYY-MM-DD HH:MM:SS）");
        }
        if let Some(err) = parse_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(8.0);

        let mut action = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(parse_error.is_none(), egui::Button::new("确定"))
                .clicked()
            {
                action = Some(AttrAction::Apply {
                    bits: self.bits,
                    mtime,
                    atime,
                });
            }
            if ui.button("取消").clicked() {
                action = Some(AttrAction::Close);
            }
        });
        action
    }
}

/// 单项应用（视图侧逐路径调用）：时间戳先于属性位写入——只读文件在
/// Windows 上 SetFileTime 会被拒（os error 5），先改时间再置只读位。
pub fn apply_to_path(
    path: &Path,
    bits: FileAttrBits,
    mtime: Option<SystemTime>,
    atime: Option<SystemTime>,
) -> Result<(), String> {
    let vp = crate::views::file_ops::verbatim_path(path);
    if let Some(t) = mtime {
        filetime::set_file_mtime(&vp, filetime::FileTime::from_system_time(t))
            .map_err(|e| format!("设置修改时间失败: {e}"))?;
    }
    if let Some(t) = atime {
        filetime::set_file_atime(&vp, filetime::FileTime::from_system_time(t))
            .map_err(|e| format!("设置访问时间失败: {e}"))?;
    }
    crate::platform::file_attr::set_attributes(path, bits)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_datetime_valid_roundtrip() {
        let st = parse_datetime_local("2024-01-15 10:30:00").expect("valid");
        // 与 format_datetime_local 同一时区口径，往返应还原原串。
        assert_eq!(format_datetime_local(st), "2024-01-15 10:30:00");
    }

    #[test]
    fn parse_datetime_rejects_malformed() {
        assert!(parse_datetime_local("").is_none());
        assert!(parse_datetime_local("2024-13-01 00:00:00").is_none());
        assert!(parse_datetime_local("2024-01-15").is_none());
        assert!(parse_datetime_local("abc").is_none());
        assert!(parse_datetime_local("2024-01-15 25:00:00").is_none());
    }

    #[test]
    fn apply_to_path_sets_readonly_and_mtime() {
        let dir = std::env::temp_dir().join(format!("openitgo-attrdlg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.txt");
        std::fs::write(&file, b"hello").unwrap();

        let mtime = parse_datetime_local("2020-06-01 12:00:00").expect("valid");
        let bits = FileAttrBits {
            readonly: true,
            ..Default::default()
        };
        apply_to_path(&file, bits, Some(mtime), None).expect("apply");

        let meta = std::fs::metadata(&file).unwrap();
        assert!(meta.permissions().readonly());
        // filetime 秒级精度（FAT 更粗，NTFS 足够）；允许 2s 误差。
        let got = meta.modified().unwrap();
        let diff = got
            .duration_since(mtime)
            .or_else(|_| mtime.duration_since(got))
            .unwrap();
        assert!(diff.as_secs() <= 2, "mtime diff {:?}", diff);

        // 清理前复位只读。
        apply_to_path(&file, FileAttrBits::default(), None, None).expect("restore");
        std::fs::remove_file(&file).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
