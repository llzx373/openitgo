//! Windows Shell 动词：文件「属性」与「打开方式…」系统对话框，经
//! `ShellExecuteExW` + `SEE_MASK_INVOKEIDLIST` 调用 Shell 上下文菜单动词
//! （与资源管理器右键「属性」/「打开方式」同一路径，对话框由 Shell 自管，
//! 无需父窗口句柄）。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

/// 本模块仅在 Windows 编译，恒为 true（供跨平台调用点统一判定）。
pub fn is_supported() -> bool {
    true
}

/// 系统「属性」对话框（Explorer 右键「属性」/ Alt+Enter 同款）。
pub fn show_properties(path: &Path) -> Result<(), String> {
    shell_verb(path, "properties")
}

/// 系统「打开方式…」对话框（仅对文件有意义；目录动词缺失时 Shell 报错）。
pub fn show_open_with(path: &Path) -> Result<(), String> {
    shell_verb(path, "openas")
}

/// 系统「编辑」动词（Explorer 右键「编辑」/ F4 同款）：无 edit 关联或
/// 动词失败时回退「打开方式…」对话框。
pub fn edit_file(path: &Path) -> Result<(), String> {
    shell_verb(path, "edit").or_else(|_| shell_verb(path, "openas"))
}

/// 以管理员身份运行（verb `"runas"`，触发 UAC 提权）。
pub fn run_as_admin(path: &Path) -> Result<(), String> {
    shell_verb(path, "runas")
}

fn shell_verb(path: &Path, verb: &str) -> Result<(), String> {
    let file: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = verb.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd: HWND::default(),
        lpVerb: PCWSTR::from_raw(verb.as_ptr()),
        lpFile: PCWSTR::from_raw(file.as_ptr()),
        nShow: SW_SHOW.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
        .map_err(|e| format!("系统对话框打开失败（{}）: {e}", path.display()))
}
