use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Shell::{
    FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FOF_ALLOWUNDO,
    FOF_NO_UI, FOF_WANTNUKEWARNING,
};

struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { initialized: false });
        }
        hr.ok().map_err(|e| format!("COM 初始化失败: {e}"))?;
        Ok(Self { initialized: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("路径不能为空".to_string());
    }
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("无法获取当前目录: {e}"))?
            .join(path)
    };
    let parent = target
        .parent()
        .ok_or_else(|| "不能删除根目录".to_string())?;
    let name = target
        .file_name()
        .ok_or_else(|| "不能删除根目录".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|e| format!("无法访问父目录 {}: {e}", parent.display()))?;
    Ok(parent.join(name))
}

fn shell_parsing_name(path: &Path) -> Vec<u16> {
    const BACKSLASH: u16 = '\\' as u16;
    const VERBATIM: [u16; 4] = [BACKSLASH, BACKSLASH, '?' as u16, BACKSLASH];
    const VERBATIM_UNC: [u16; 8] = [
        BACKSLASH, BACKSLASH, '?' as u16, BACKSLASH, 'U' as u16, 'N' as u16, 'C' as u16, BACKSLASH,
    ];
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.starts_with(&VERBATIM_UNC) {
        wide.splice(..VERBATIM_UNC.len(), [BACKSLASH, BACKSLASH]);
    } else if wide.starts_with(&VERBATIM) {
        wide.drain(..VERBATIM.len());
    }
    wide.push(0);
    wide
}

pub fn delete(path: &Path) -> Result<(), String> {
    let path = absolute_path(path)?;
    let _apartment = ComApartment::initialize()?;
    let wide = shell_parsing_name(&path);
    unsafe {
        let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
            .map_err(|e| format!("无法创建回收站操作: {e}"))?;
        operation
            .SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING)
            .map_err(|e| format!("无法配置回收站操作: {e}"))?;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
            .map_err(|e| format!("无法访问 {}: {e}", path.display()))?;
        operation
            .DeleteItem(&item, None)
            .map_err(|e| format!("无法加入回收站操作: {e}"))?;
        operation
            .PerformOperations()
            .map_err(|e| format!("移入回收站失败: {e}"))?;
        if operation
            .GetAnyOperationsAborted()
            .map_err(|e| format!("无法确认回收站操作结果: {e}"))?
            .as_bool()
        {
            return Err("回收站操作已中止".to_string());
        }
    }
    Ok(())
}
