//! Windows 文件关联注册（HKCU\Software\Classes，免管理员、不碰 HKCR/HKLM）。
//!
//! 每个扩展名指向所属分组的 ProgID（`OpenItGo.Archive/Image/Media`），
//! ProgID 键携带中文描述、`DefaultIcon` 与 `shell\open\command`。
//! DefaultIcon 指向 exe 内嵌图标资源：`assets/icon/openitgo.rc` 按
//! ID 1..=4 编入 AppIcon/Archive/Image/Media.ico，对应图标索引
//! 0..=3（`"<exe>",<index>"`）；索引 0 是应用主图标，三组分组各用
//! 1/2/3（见 `AssocGroup::icon_index`）。覆盖他人默认值前先备份到
//! `HKCU\Software\Classes\OpenItGo.bak\<ext>`（已有备份不覆盖），注销时
//! 仅当默认值仍是本程序 ProgID 才恢复备份并删除；ProgID 键在不再被任何
//! 扩展名引用时整体删除。注册/注销后广播 `SHChangeNotify(SHCNE_ASSOCCHANGED)`。

use std::io;

use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::{RegKey, RegValue};

/// 扩展名分组（压缩包 / 图片 / 影视）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssocGroup {
    /// 压缩包（zip/cbz/rar/…）
    Archive,
    /// 图片（jpg/png/…）
    Image,
    /// 影视（音视频）
    Media,
}

impl AssocGroup {
    /// 分组显示名（「压缩包」/「图片」/「影视」）。
    pub fn label(self) -> &'static str {
        match self {
            AssocGroup::Archive => "压缩包",
            AssocGroup::Image => "图片",
            AssocGroup::Media => "影视",
        }
    }

    /// 该组的 ProgID，如 `OpenItGo.Archive`。
    pub fn progid(self) -> &'static str {
        match self {
            AssocGroup::Archive => "OpenItGo.Archive",
            AssocGroup::Image => "OpenItGo.Image",
            AssocGroup::Media => "OpenItGo.Media",
        }
    }

    /// ProgID 键默认值描述（「OpenItGo 压缩包」等）。
    pub fn description(self) -> String {
        format!("OpenItGo {}", self.label())
    }

    /// DefaultIcon 图标索引（`"<exe>",<index>"`）：对应 rc 资源 ID 2/3/4。
    pub fn icon_index(self) -> i32 {
        match self {
            AssocGroup::Archive => 1,
            AssocGroup::Image => 2,
            AssocGroup::Media => 3,
        }
    }
}

/// 单个扩展名的关联状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssocState {
    /// 已关联到本程序。
    Ours,
    /// 被其他程序占用（值为现有 ProgID / 应用标识）。
    Other(String),
    /// 无关联。
    None,
}

/// 扩展名关联条目（query_status 的返回元素）。
#[derive(Debug, Clone)]
pub struct ExtAssoc {
    /// 扩展名（小写、不含点）。
    pub ext: &'static str,
    /// 所属分组。
    pub group: AssocGroup,
    /// 当前关联状态。
    pub state: AssocState,
    /// 设置页勾选框默认值（已关联到本程序时默认勾选）。
    pub selected: bool,
}

/// 静态扩展名分组表：`(分组, 组显示名, 扩展名列表)`。
pub const EXT_GROUPS: &[(AssocGroup, &str, &[&str])] = &[
    (
        AssocGroup::Archive,
        "压缩包",
        &["zip", "cbz", "rar", "cbr", "7z", "tar", "tgz", "tbz2"],
    ),
    (
        AssocGroup::Image,
        "图片",
        &[
            "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "avif",
        ],
    ),
    (
        AssocGroup::Media,
        "影视",
        &[
            "mp4", "m4v", "mkv", "webm", "avi", "mov", "wmv", "flv", "ts", "m2ts", "mpg", "mpeg",
            "3gp", "mp3", "flac", "aac", "m4a", "ogg", "oga", "opus", "wav", "aiff", "ape", "wma",
        ],
    ),
];

const CLASSES_PATH: &str = "Software\\Classes";
const BACKUP_ROOT: &str = "OpenItGo.bak";

/// 按默认注册值分类关联状态。
fn classify_state(default: Option<String>, progid: &str) -> AssocState {
    match default {
        Some(v) if v == progid => AssocState::Ours,
        Some(v) => AssocState::Other(v),
        None => AssocState::None,
    }
}

/// 覆盖前是否需要备份：存在现有默认值且非本程序 ProgID。
fn should_backup(default: Option<&str>, progid: &str) -> bool {
    matches!(default, Some(v) if v != progid)
}

/// 注销时是否恢复备份：当前默认值仍是本程序 ProgID 且存在备份。
fn should_restore(current: Option<&str>, backup: Option<&str>, progid: &str) -> bool {
    current == Some(progid) && backup.is_some()
}

/// 按扩展名查找所属分组。
fn group_for_ext(ext: &str) -> Option<AssocGroup> {
    EXT_GROUPS
        .iter()
        .find(|(_, _, exts)| exts.contains(&ext))
        .map(|(group, _, _)| *group)
}

/// 打开 HKCU\Software\Classes（可读写）。
fn classes_key() -> Result<RegKey, String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(CLASSES_PATH, KEY_READ | KEY_WRITE)
        .map_err(|e| format!("无法打开 HKCU\\{CLASSES_PATH}: {e}"))
}

/// 读取 `.<ext>` 键默认值；键/值不存在或为空串均视为 None。
fn read_default(ext: &str) -> Result<Option<String>, String> {
    let classes = classes_key()?;
    match classes.open_subkey_with_flags(format!(".{ext}"), KEY_READ) {
        Ok(key) => match key.get_value::<String, _>("") {
            Ok(v) if v.is_empty() => Ok(None),
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取 .{ext} 默认值失败: {e}")),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("打开 .{ext} 键失败: {e}")),
    }
}

/// 读取 `OpenItGo.bak\<ext>` 备份值。
fn read_backup(ext: &str) -> Result<Option<String>, String> {
    let classes = classes_key()?;
    match classes.open_subkey_with_flags(format!("{BACKUP_ROOT}\\{ext}"), KEY_READ) {
        Ok(key) => match key.get_value::<String, _>("") {
            Ok(v) if v.is_empty() => Ok(None),
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取 .{ext} 备份失败: {e}")),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("打开 .{ext} 备份键失败: {e}")),
    }
}

/// 备份 `.<ext>` 现有默认值；已有非空备份则不覆盖（保住最早的原始值）。
fn backup_value(ext: &str, value: &str) -> Result<(), String> {
    let classes = classes_key()?;
    let (bak, _) = classes
        .create_subkey(format!("{BACKUP_ROOT}\\{ext}"))
        .map_err(|e| format!("创建 .{ext} 备份键失败: {e}"))?;
    match bak.get_value::<String, _>("") {
        Ok(existing) if !existing.is_empty() => Ok(()),
        _ => bak
            .set_value("", &value)
            .map_err(|e| format!("写入 .{ext} 备份失败: {e}")),
    }
}

/// 删除 `OpenItGo.bak\<ext>` 备份键（不存在则忽略）。
fn delete_backup(ext: &str) -> Result<(), String> {
    let classes = classes_key()?;
    match classes.delete_subkey(format!("{BACKUP_ROOT}\\{ext}")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除 .{ext} 备份键失败: {e}")),
    }
}

/// 确保分组 ProgID 键存在（描述 / DefaultIcon / shell\open\command）。
fn ensure_progid(group: AssocGroup) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取程序路径: {e}"))?;
    let exe = exe.to_string_lossy();
    let classes = classes_key()?;
    let progid = group.progid();
    let (key, _) = classes
        .create_subkey(progid)
        .map_err(|e| format!("创建 {progid} 键失败: {e}"))?;
    key.set_value("", &group.description())
        .map_err(|e| format!("写入 {progid} 描述失败: {e}"))?;
    let (icon, _) = key
        .create_subkey("DefaultIcon")
        .map_err(|e| format!("创建 {progid}\\DefaultIcon 失败: {e}"))?;
    icon.set_value("", &format!("\"{exe}\",{}", group.icon_index()))
        .map_err(|e| format!("写入 {progid} 图标失败: {e}"))?;
    let (cmd, _) = key
        .create_subkey("shell\\open\\command")
        .map_err(|e| format!("创建 {progid}\\shell\\open\\command 失败: {e}"))?;
    cmd.set_value("", &format!("\"{exe}\" \"%1\""))
        .map_err(|e| format!("写入 {progid} 打开命令失败: {e}"))?;
    Ok(())
}

/// ProgID 是否仍被任一已知扩展名引用。
fn progid_in_use(progid: &str) -> Result<bool, String> {
    for (_, _, exts) in EXT_GROUPS {
        for ext in *exts {
            if read_default(ext)?.as_deref() == Some(progid) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// 广播关联变更通知，让资源管理器等刷新图标与「打开方式」。
fn notify_assoc_changed() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    // SAFETY: 纯通知广播；两个 item 参数按文档允许为 NULL，
    // 事件/标志常量均为合法值。无内存生命周期约束。
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// 查询全部已知扩展名的当前关联状态。
pub fn query_status() -> Result<Vec<ExtAssoc>, String> {
    let mut out = Vec::new();
    for (group, _, exts) in EXT_GROUPS {
        for ext in *exts {
            let state = classify_state(read_default(ext)?, group.progid());
            let selected = state == AssocState::Ours;
            out.push(ExtAssoc {
                ext,
                group: *group,
                state,
                selected,
            });
        }
    }
    Ok(out)
}

/// 注册文件关联：把给定扩展名的默认值指向本程序 ProgID。
/// 返回成功处理的扩展名个数。
pub fn register(exts: &[&str]) -> Result<usize, String> {
    let classes = classes_key()?;
    let mut done = 0usize;
    for ext in exts {
        let group = group_for_ext(ext).ok_or_else(|| format!("未知扩展名: {ext}"))?;
        ensure_progid(group)?;
        let progid = group.progid();
        let current = read_default(ext)?;
        if should_backup(current.as_deref(), progid) {
            backup_value(ext, current.as_deref().unwrap_or_default())?;
        }
        let (key, _) = classes
            .create_subkey(format!(".{ext}"))
            .map_err(|e| format!("创建 .{ext} 键失败: {e}"))?;
        key.set_value("", &progid)
            .map_err(|e| format!("写入 .{ext} 默认值失败: {e}"))?;
        // OpenWithProgids 登记（REG_NONE 空值），出现在「打开方式」列表。
        let (owp, _) = key
            .create_subkey("OpenWithProgids")
            .map_err(|e| format!("创建 .{ext}\\OpenWithProgids 失败: {e}"))?;
        owp.set_raw_value(
            progid,
            &RegValue {
                vtype: RegType::REG_NONE,
                bytes: Vec::new(),
            },
        )
        .map_err(|e| format!("写入 .{ext}\\OpenWithProgids 失败: {e}"))?;
        done += 1;
    }
    notify_assoc_changed();
    Ok(done)
}

/// 注销文件关联：默认值仍是本程序 ProgID 时才删除并恢复备份；
/// 返回成功处理的扩展名个数。
pub fn unregister(exts: &[&str]) -> Result<usize, String> {
    let classes = classes_key()?;
    let mut done = 0usize;
    let mut touched: Vec<AssocGroup> = Vec::new();
    for ext in exts {
        let group = group_for_ext(ext).ok_or_else(|| format!("未知扩展名: {ext}"))?;
        if !touched.contains(&group) {
            touched.push(group);
        }
        let progid = group.progid();
        let key = match classes.open_subkey_with_flags(format!(".{ext}"), KEY_READ | KEY_WRITE) {
            Ok(key) => key,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("打开 .{ext} 键失败: {e}")),
        };
        let current = match key.get_value::<String, _>("") {
            Ok(v) if !v.is_empty() => Some(v),
            Ok(_) => None,
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("读取 .{ext} 默认值失败: {e}")),
        };
        if current.as_deref() == Some(progid) {
            key.delete_value("")
                .map_err(|e| format!("删除 .{ext} 默认值失败: {e}"))?;
            let backup = read_backup(ext)?;
            if should_restore(current.as_deref(), backup.as_deref(), progid) {
                let value = backup.unwrap_or_default();
                key.set_value("", &value)
                    .map_err(|e| format!("恢复 .{ext} 备份失败: {e}"))?;
                delete_backup(ext)?;
            }
            done += 1;
        }
        // OpenWithProgids 中移除本程序 ProgID（键/值不存在均忽略）。
        if let Ok(owp) = key.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
            let _ = owp.delete_value(progid);
        }
    }
    // ProgID 键在不再被任何扩展名引用时整体删除。
    for group in touched {
        if !progid_in_use(group.progid())? {
            match classes.delete_subkey_all(group.progid()) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("删除 {} 键失败: {e}", group.progid())),
            }
        }
    }
    notify_assoc_changed();
    Ok(done)
}

/// 打开 Windows「默认应用」设置页（ms-settings:defaultapps）。
pub fn open_default_apps_settings() -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let verb = wide("open");
    let target = wide("ms-settings:defaultapps");
    // SAFETY: verb/target 指向本函数内有效的 NUL 结尾 UTF-16 缓冲区，
    // 调用期间不被释放；hwnd/参数/目录为 NULL 合法。ShellExecuteW 不接管
    // 参数所有权。返回值 > 32 表示成功（ms-settings: 协议 Win8+ 可用）。
    let ret = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (ret as isize) > 32 {
        Ok(())
    } else {
        Err(format!("打开默认应用设置失败（错误码 {}）", ret as isize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ext_table_well_formed() {
        // 三组均非空，扩展名无重复、全小写、不带前导点。
        assert_eq!(EXT_GROUPS.len(), 3);
        let mut seen = HashSet::new();
        for (group, label, exts) in EXT_GROUPS {
            assert!(!exts.is_empty(), "{} 组为空", group.label());
            assert_eq!(*label, group.label());
            for ext in *exts {
                assert!(!ext.is_empty());
                assert_eq!(*ext, ext.to_lowercase(), "{ext} 非小写");
                assert!(!ext.starts_with('.'), "{ext} 含前导点");
                assert!(seen.insert(*ext), "扩展名重复: {ext}");
            }
        }
    }

    #[test]
    fn progids_unique() {
        let ids: HashSet<_> = EXT_GROUPS.iter().map(|(g, _, _)| g.progid()).collect();
        assert_eq!(ids.len(), EXT_GROUPS.len());
    }

    #[test]
    fn classify_state_cases() {
        let progid = AssocGroup::Archive.progid();
        assert_eq!(
            classify_state(Some(progid.to_string()), progid),
            AssocState::Ours
        );
        assert_eq!(
            classify_state(Some("WinRAR".to_string()), progid),
            AssocState::Other("WinRAR".to_string())
        );
        assert_eq!(classify_state(None, progid), AssocState::None);
    }

    #[test]
    fn backup_restore_decisions() {
        let progid = AssocGroup::Archive.progid();
        // 备份：有他人默认值才备份；无值/已是本程序不备份。
        assert!(should_backup(Some("WinRAR"), progid));
        assert!(!should_backup(Some(progid), progid));
        assert!(!should_backup(None, progid));
        // 恢复：当前仍是本程序且有备份才恢复。
        assert!(should_restore(Some(progid), Some("WinRAR"), progid));
        assert!(!should_restore(Some("Other"), Some("WinRAR"), progid));
        assert!(!should_restore(Some(progid), None, progid));
    }

    #[test]
    fn icon_indices_distinct() {
        // 三组图标索引互不相同且都在 1..=3（0 是应用主图标，不用于分组）。
        let indices: HashSet<i32> = EXT_GROUPS.iter().map(|(g, _, _)| g.icon_index()).collect();
        assert_eq!(indices.len(), 3);
        for idx in indices {
            assert!((1..=3).contains(&idx), "图标索引越界: {idx}");
        }
    }

    #[test]
    fn group_lookup() {
        assert_eq!(group_for_ext("zip"), Some(AssocGroup::Archive));
        assert_eq!(group_for_ext("7z"), Some(AssocGroup::Archive));
        assert_eq!(group_for_ext("png"), Some(AssocGroup::Image));
        assert_eq!(group_for_ext("mkv"), Some(AssocGroup::Media));
        assert_eq!(group_for_ext("opus"), Some(AssocGroup::Media));
        assert_eq!(group_for_ext("xyz"), None);
    }
}
