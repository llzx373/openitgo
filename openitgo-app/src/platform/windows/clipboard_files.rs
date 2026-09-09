//! Windows 系统剪贴板文件列表：写/读 `CF_HDROP`（DROPFILES + 双 NUL 结尾
//! 宽字符路径表）与 `CFSTR_PREFERREDDROPEFFECT`（区分复制/剪切）。
//! 文件管理器 Ctrl+C/X/V 与 Explorer 互通用。
//!
//! HDROP 负载编码（`build_hdrop`/`wide_path_list`）复用 `drag_out` 的
//! 既有实现（那里有对应单测）。

use std::path::PathBuf;

use super::drag_out::build_hdrop;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
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

/// 把文件列表写入系统剪贴板（cut=true 标记为「剪切」，即
/// Preferred DropEffect = MOVE）。写成功前会 EmptyClipboard。
pub fn set_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可写入剪贴板的文件".to_string());
    }
    unsafe {
        OpenClipboard(None).map_err(|e| format!("OpenClipboard 失败: {e}"))?;
        let result = set_files_inner(paths, cut);
        let _ = CloseClipboard();
        result
    }
}

unsafe fn set_files_inner(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    EmptyClipboard().map_err(|e| format!("EmptyClipboard 失败: {e}"))?;
    // SetClipboardData 成功后 HGLOBAL 所有权归系统，不得再 GlobalFree。
    let hdrop = build_hdrop(paths).map_err(|e| format!("构造 HDROP 失败: {e}"))?;
    SetClipboardData(CF_HDROP_U32, Some(HANDLE(hdrop.0)))
        .map_err(|e| format!("SetClipboardData(CF_HDROP) 失败: {e}"))?;
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
            if !ptr.is_null() {
                ptr.cast::<u32>().write(effect);
                let _ = GlobalUnlock(hg);
                // DropEffect 写不上不致命（粘贴方按复制处理）。
                let _ = SetClipboardData(fmt, Some(HANDLE(hg.0)));
            }
        }
    }
    Ok(())
}

/// 读系统剪贴板的文件列表：无 CF_HDROP 返回 None；第二元素 = 是否剪切
/// （Preferred DropEffect = MOVE，缺失时按复制处理）。
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP_U32).is_err() {
            return None;
        }
        OpenClipboard(None).ok()?;
        let result = get_files_inner();
        let _ = CloseClipboard();
        result
    }
}

unsafe fn get_files_inner() -> Option<(Vec<PathBuf>, bool)> {
    // GetClipboardData 返回的句柄归系统所有，只读不释放。
    let handle = GetClipboardData(CF_HDROP_U32).ok()?;
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
        return None;
    }
    let mut is_cut = false;
    let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if fmt != 0 {
        if let Ok(h) = GetClipboardData(fmt) {
            let hg = HGLOBAL(h.0);
            let ptr = GlobalLock(hg);
            if !ptr.is_null() {
                is_cut = ptr.cast::<u32>().read() == DROPEFFECT_VALUE_MOVE;
                let _ = GlobalUnlock(hg);
            }
        }
    }
    Some((paths, is_cut))
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
