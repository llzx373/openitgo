//! Windows OLE 拖出解压：把已解压到临时目录的文件以 `CF_HDROP` 格式交给
//! `DoDragDrop`，用户从压缩包浏览器拖到资源管理器/桌面即完成复制。
//!
//! `DoDragDrop` 是模态阻塞调用（自带消息循环），在 egui UI 线程直接调用即可，
//! 期间窗口消息仍被处理；返回后本次拖出结束。

use std::mem::ManuallyDrop;
use std::path::PathBuf;

use windows::core::{implement, Ref, BOOL, HRESULT};
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, E_NOTIMPL, HGLOBAL, POINT,
    S_FALSE, S_OK,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, FORMATETC,
    STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, CF_HDROP,
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::DROPFILES;

/// 本模块仅在 Windows 编译，恒为 true（供跨平台调用点统一判定）。
pub fn is_supported() -> bool {
    true
}

/// 对一组已落盘文件发起 OLE 拖出（阻塞至拖放结束或被取消）。
pub fn do_drag_drop(files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Err("没有可拖出的文件".to_string());
    }
    unsafe {
        OleInitialize(None).map_err(|e| format!("OleInitialize 失败: {e}"))?;
        let result = drag_drop_inner(files);
        OleUninitialize();
        result
    }
}

unsafe fn drag_drop_inner(files: &[PathBuf]) -> Result<(), String> {
    let data: IDataObject = FileDataObject {
        files: files.to_vec(),
    }
    .into();
    let source: IDropSource = SimpleDropSource.into();
    let mut effect = DROPEFFECT_NONE;
    // 压缩包拖出惯例只允许 COPY（源是只读压缩包，MOVE 无意义）。
    let hr = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    // DRAGDROP_S_CANCEL（用户 Esc/右键取消）是 S 开头的成功码，不算错误。
    if hr == DRAGDROP_S_CANCEL {
        return Ok(());
    }
    hr.ok().map_err(|e| format!("拖放失败: {e}"))
}

/// 把路径表编码为 HDROP 的宽字符负载：各路径 UTF-16 + NUL，末尾再补一个 NUL。
/// （clipboard_files 的系统剪贴板 CF_HDROP 复用。）
pub(crate) fn wide_path_list(files: &[PathBuf]) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut out = Vec::new();
    for f in files {
        out.extend(f.as_os_str().encode_wide());
        out.push(0);
    }
    out.push(0);
    out
}

/// 构造 HDROP 全局内存块（DROPFILES 头 + 宽字符路径表）。调用方取得所有权。
/// （clipboard_files 写系统剪贴板复用；SetClipboardData 成功后所有权归系统。）
pub(crate) fn build_hdrop(files: &[PathBuf]) -> windows::core::Result<HGLOBAL> {
    let wide = wide_path_list(files);
    let header = std::mem::size_of::<DROPFILES>();
    let total = header + wide.len() * std::mem::size_of::<u16>();
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, total)?;
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return Err(windows::core::Error::from(E_NOTIMPL));
        }
        let dropfiles = DROPFILES {
            pFiles: header as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: BOOL(0),
            fWide: BOOL(1),
        };
        ptr.cast::<DROPFILES>().write(dropfiles);
        let dst = ptr.cast::<u8>().add(header).cast::<u16>();
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        let _ = GlobalUnlock(hglobal);
        Ok(hglobal)
    }
}

/// 是否是我们提供的格式（CF_HDROP + TYMED_HGLOBAL）。
fn is_hdrop_format(fmt: &FORMATETC) -> bool {
    fmt.cfFormat == CF_HDROP.0 && (fmt.tymed & TYMED_HGLOBAL.0 as u32) != 0
}

#[implement(IDataObject)]
struct FileDataObject {
    files: Vec<PathBuf>,
}

impl IDataObject_Impl for FileDataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let fmt = unsafe { pformatetcin.as_ref() }
            .ok_or_else(|| windows::core::Error::from(E_NOTIMPL))?;
        if !is_hdrop_format(fmt) {
            return Err(windows::core::Error::from(E_NOTIMPL));
        }
        let hglobal = build_hdrop(&self.files)?;
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: hglobal },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }

    fn GetDataHere(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        match unsafe { pformatetc.as_ref() } {
            Some(fmt) if is_hdrop_format(fmt) => S_OK,
            _ => S_FALSE,
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        _pformatectin: *const FORMATETC,
        _pformatetcout: *mut FORMATETC,
    ) -> HRESULT {
        E_NOTIMPL
    }

    fn SetData(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *const STGMEDIUM,
        _frelease: BOOL,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }

    fn EnumFormatEtc(&self, _dwdirection: u32) -> windows::core::Result<IEnumFORMATETC> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }

    fn DUnadvise(&self, _dwconnection: u32) -> windows::core::Result<()> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(windows::core::Error::from(E_NOTIMPL))
    }
}

#[implement(IDropSource)]
struct SimpleDropSource;

impl IDropSource_Impl for SimpleDropSource_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if (grfkeystate & MK_LBUTTON).0 == 0 {
            // 左键已松开：落点接受则完成拖放。
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_path_list_is_double_nul_terminated() {
        let files = vec![
            PathBuf::from(r"C:\tmp\a.txt"),
            PathBuf::from(r"C:\tmp\b.png"),
        ];
        let wide = wide_path_list(&files);
        // 末尾双 NUL。
        assert_eq!(wide[wide.len() - 1], 0);
        assert_eq!(wide[wide.len() - 2], 0);
        // 恰好 3 个 NUL（两条路径各一个 + 结尾一个）。
        assert_eq!(wide.iter().filter(|&&c| c == 0).count(), 3);
    }

    #[test]
    fn build_hdrop_writes_header_and_paths() {
        let files = vec![PathBuf::from(r"C:\tmp\a.txt")];
        let hglobal = build_hdrop(&files).expect("alloc");
        unsafe {
            let ptr = GlobalLock(hglobal);
            assert!(!ptr.is_null());
            let header = ptr.cast::<DROPFILES>().read();
            assert_eq!(header.pFiles as usize, std::mem::size_of::<DROPFILES>());
            assert!(header.fWide.as_bool());
            let _ = GlobalUnlock(hglobal);
        }
    }
}
