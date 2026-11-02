//! Windows 系统剪贴板文件列表：写/读 `CF_HDROP`（DROPFILES + 双 NUL 结尾
//! 宽字符路径表）与 `CFSTR_PREFERREDDROPEFFECT`（区分复制/剪切）。
//! 文件管理器 Ctrl+C/X/V 与 Explorer 互通用。
//!
//! HDROP 负载编码（`build_hdrop`/`wide_path_list`）复用 `drag_out` 的
//! 既有实现（那里有对应单测）。

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use super::drag_out::build_hdrop;
use windows::core::w;
use windows::Win32::Foundation::{
    GetLastError, GlobalFree, ERROR_CLIPBOARD_NOT_OPEN, HANDLE, HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// Preferred DropEffect 值（与 OLE DROPEFFECT 一致）：1 = COPY，2 = MOVE。
const DROPEFFECT_VALUE_COPY: u32 = 1;
const DROPEFFECT_VALUE_MOVE: u32 = 2;

/// CF_HDROP 的 u32 格式码（DataExchange API 取原始 u32）。
const CF_HDROP_U32: u32 = CF_HDROP.0 as u32;

/// Win10/11 已知缺陷：`OpenClipboard(NULL)` 即使返回成功，同一打开会话内的
/// 后续 `Get/SetClipboardData` 仍可能以 `ERROR_CLIPBOARD_NOT_OPEN` 失败——
/// 刚写入剪贴板后的第一次读取最容易触发（Cygwin 亦有同样报告与重试规避）。
/// 因此一次尝试只做一轮数据访问，失败即关闭重开会话重试，最多 20 次。
const OPEN_ATTEMPTS: usize = 20;
const RETRY_DELAY: Duration = Duration::from_millis(1);

/// 当前线程的剪贴板打开会话已失效（1418）。
/// 必须在 API 调用失败后立刻调用，中间不要插入其它 Win32 调用。
fn clipboard_session_lost() -> bool {
    unsafe { GetLastError() == ERROR_CLIPBOARD_NOT_OPEN }
}

/// 把文件列表写入系统剪贴板（cut=true 标记为「剪切」，即
/// Preferred DropEffect = MOVE）。写成功前会 EmptyClipboard。
pub fn set_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可写入剪贴板的文件".to_string());
    }
    let mut last_error = "OpenClipboard 失败: 剪贴板被其他程序占用".to_string();
    for attempt in 0..OPEN_ATTEMPTS {
        if attempt > 0 {
            sleep(RETRY_DELAY);
        }
        unsafe {
            if let Err(e) = OpenClipboard(None) {
                last_error = format!("OpenClipboard 失败: {e}");
                continue;
            }
            let result = set_files_inner(paths, cut);
            let _ = CloseClipboard();
            match result {
                Ok(()) => return Ok(()),
                Err(WriteError::SessionLost) => {}
                Err(WriteError::Message(msg)) => return Err(msg),
            }
        }
    }
    Err(last_error)
}

enum WriteError {
    /// 1418：本次打开会话无效，关闭重开后可以重试。
    SessionLost,
    /// 其它错误：重试也不会好转，直接上报。
    Message(String),
}

/// 把 API 失败归类；会立刻读取 `GetLastError`，因此必须紧跟失败调用。
fn write_error(context: &str, e: windows::core::Error) -> WriteError {
    if clipboard_session_lost() {
        WriteError::SessionLost
    } else {
        WriteError::Message(format!("{context}: {e}"))
    }
}

unsafe fn set_files_inner(paths: &[PathBuf], cut: bool) -> Result<(), WriteError> {
    EmptyClipboard().map_err(|e| write_error("EmptyClipboard 失败", e))?;
    // SetClipboardData 成功后 HGLOBAL 所有权归系统，不得再 GlobalFree。
    let hdrop =
        build_hdrop(paths).map_err(|e| WriteError::Message(format!("构造 HDROP 失败: {e}")))?;
    if let Err(e) = SetClipboardData(CF_HDROP_U32, Some(HANDLE(hdrop.0))) {
        // 失败时所有权仍在我们手里，释放防泄漏。
        let err = write_error("SetClipboardData(CF_HDROP) 失败", e);
        let _ = GlobalFree(Some(HGLOBAL(hdrop.0)));
        return Err(err);
    }
    // Preferred DropEffect：Explorer 据此决定粘贴 = 复制还是移动。
    let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if fmt != 0 {
        let effect = if cut {
            DROPEFFECT_VALUE_MOVE
        } else {
            DROPEFFECT_VALUE_COPY
        };
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>()) {
            let ptr = GlobalLock(hg);
            if ptr.is_null() {
                let _ = GlobalFree(Some(hg));
            } else {
                ptr.cast::<u32>().write(effect);
                let _ = GlobalUnlock(hg);
                if SetClipboardData(fmt, Some(HANDLE(hg.0))).is_err() {
                    let lost = clipboard_session_lost();
                    let _ = GlobalFree(Some(hg));
                    if lost {
                        return Err(WriteError::SessionLost);
                    }
                    // DropEffect 写不上不致命（粘贴方按复制处理）。
                }
            }
        }
    }
    Ok(())
}

/// 读系统剪贴板的文件列表：无 CF_HDROP 返回 None；第二元素 = 是否剪切
/// （Preferred DropEffect = MOVE，缺失时按复制处理）。
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    if unsafe { IsClipboardFormatAvailable(CF_HDROP_U32) }.is_err() {
        return None;
    }
    for attempt in 0..OPEN_ATTEMPTS {
        if attempt > 0 {
            sleep(RETRY_DELAY);
        }
        unsafe {
            if OpenClipboard(None).is_err() {
                continue;
            }
            let outcome = get_files_inner();
            let _ = CloseClipboard();
            match outcome {
                ReadOutcome::Value(files, is_cut) => return Some((files, is_cut)),
                ReadOutcome::Absent => return None,
                ReadOutcome::SessionLost => {}
            }
        }
    }
    None
}

enum ReadOutcome {
    Value(Vec<PathBuf>, bool),
    /// 剪贴板里没有文件列表。
    Absent,
    /// 1418：本次打开会话无效，关闭重开后可以重试。
    SessionLost,
}

unsafe fn get_files_inner() -> ReadOutcome {
    // GetClipboardData 返回的句柄归系统所有，只读不释放。
    let handle = match GetClipboardData(CF_HDROP_U32) {
        Ok(h) => h,
        Err(_) => {
            return if clipboard_session_lost() {
                ReadOutcome::SessionLost
            } else {
                ReadOutcome::Absent
            }
        }
    };
    let hdrop = HDROP(handle.0);
    let count = DragQueryFileW(hdrop, u32::MAX, None);
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
        if got == 0 {
            continue;
        }
        buf.truncate(got);
        paths.push(PathBuf::from(String::from_utf16_lossy(&buf)));
    }
    if paths.is_empty() {
        return ReadOutcome::Absent;
    }
    let mut is_cut = false;
    let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if fmt != 0 {
        match GetClipboardData(fmt) {
            Ok(h) => {
                let hg = HGLOBAL(h.0);
                let ptr = GlobalLock(hg);
                if !ptr.is_null() {
                    is_cut = ptr.cast::<u32>().read() == DROPEFFECT_VALUE_MOVE;
                    let _ = GlobalUnlock(hg);
                }
            }
            Err(_) => {
                if clipboard_session_lost() {
                    return ReadOutcome::SessionLost;
                }
                // 其它错误：DropEffect 缺失按复制处理。
            }
        }
    }
    ReadOutcome::Value(paths, is_cut)
}

/// 清空系统剪贴板（剪切粘贴完成后调用——Explorer 惯例：移动粘贴生效后
/// 清除剪贴板，防同一份「剪切」被重复粘贴）。
pub fn clear() {
    unsafe {
        if OpenClipboard(None).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写入→读回端到端：路径列表与 cut 标志应原样返回。
    /// 注意：会覆盖当前用户剪贴板（测试机/CI 可接受）；剪贴板被其他
    /// 程序占用导致 OpenClipboard 失败时跳过（不算失败）。
    #[test]
    fn set_then_get_roundtrip() {
        let files = vec![
            PathBuf::from(r"C:\tmp\clip-a.txt"),
            PathBuf::from(r"D:\漫画\第01话.cbz"),
        ];
        if set_files(&files, true).is_err() {
            eprintln!("clipboard busy, skip roundtrip");
            return;
        }
        let (got, is_cut) = get_files().expect("应有 CF_HDROP");
        assert_eq!(got, files);
        assert!(is_cut, "cut 标志应读回");
        // 复制标志：重写为 copy 后 is_cut = false。
        set_files(&files, false).expect("重写剪贴板");
        let (_, is_cut) = get_files().expect("应有 CF_HDROP");
        assert!(!is_cut);
        clear();
    }
}
