//! Windows 系统文件图标：经 `SHGetFileInfoW` 取 Shell 图标（目录/普通扩展名
//! 用 `SHGFI_USEFILEATTRIBUTES` 伪属性免读盘，exe/lnk/ico 图标随具体文件而变
//! 须走实路径），HICON → RGBA 经 `GetIconInfo` + `GetDIBits`（32bpp、
//! biHeight 取负值直接得 top-down 位图）。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON,
    SHGFI_USEFILEATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// exe/lnk/ico 的图标内嵌在文件自身（或指向其目标），扩展名伪路径取不到。
fn needs_real_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("exe" | "lnk" | "ico")
    )
}

/// 取 `path` 的系统图标（`large` = 32px 档，否则 16px 档）。
/// 返回 None 时调用方回退字体图标。供后台 worker 线程调用。
pub fn extract_icon(path: &Path, is_dir: bool, large: bool) -> Option<egui::ColorImage> {
    let use_attrs = is_dir || !needs_real_path(path);
    let attrs = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut flags = SHGFI_ICON
        | if large {
            SHGFI_LARGEICON
        } else {
            SHGFI_SMALLICON
        };
    if use_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let mut info = SHFILEINFOW::default();
    // SAFETY: `wide` 以 NUL 结尾且生命周期覆盖调用；`info` 缓冲区尺寸正确。
    let ret = unsafe {
        SHGetFileInfoW(
            PCWSTR::from_raw(wide.as_ptr()),
            attrs,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if ret == 0 || info.hIcon.is_invalid() {
        return None;
    }
    // SAFETY: `info.hIcon` 归调用方所有，转换后销毁。
    let image = unsafe { hicon_to_color_image(info.hIcon) };
    let _ = unsafe { DestroyIcon(info.hIcon) };
    image
}

struct BitmapGuard(HBITMAP);

impl Drop for BitmapGuard {
    fn drop(&mut self) {
        // SAFETY: 位图句柄由本 guard 独占，仅删除一次。
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

// SAFETY 约定：`icon` 为有效 HICON；转换不取得其所有权。
unsafe fn hicon_to_color_image(icon: HICON) -> Option<egui::ColorImage> {
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let color_guard = (!info.hbmColor.is_invalid()).then_some(BitmapGuard(info.hbmColor));
    let _mask_guard = (!info.hbmMask.is_invalid()).then_some(BitmapGuard(info.hbmMask));
    // hbmColor 为空 = 单色图标（mask 同时承担颜色），不支持，回退字体图标。
    let color = color_guard.as_ref()?.0;

    let mut bmp = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(color.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    ) == 0
    {
        return None;
    }
    let (w, h) = (bmp.bmWidth, bmp.bmHeight.abs());
    if w <= 0 || h <= 0 {
        return None;
    }

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // 负值 = top-down 行序，省去翻转。
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return None;
    }
    let lines = GetDIBits(
        hdc,
        color,
        0,
        h as u32,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);
    if lines == 0 {
        return None;
    }
    Some(bgra_to_color_image(w as usize, h as usize, pixels))
}

/// BGRA → RGBA；无 alpha 通道的图标 GetDIBits 会留下全零 alpha，兜底为不透明；
/// 其余按 MSDN 约定视为预乘 alpha，转 egui 直通（钳位防非预乘数据溢出回绕）。
fn bgra_to_color_image(w: usize, h: usize, mut pixels: Vec<u8>) -> egui::ColorImage {
    let all_transparent = pixels.as_chunks::<4>().0.iter().all(|c| c[3] == 0);
    for c in pixels.as_chunks_mut::<4>().0 {
        c.swap(0, 2);
        if all_transparent {
            c[3] = 255;
        } else if c[3] != 0 && c[3] != 255 {
            let a = c[3] as u16;
            c[0] = ((c[0] as u16 * 255) / a).min(255) as u8;
            c[1] = ((c[1] as u16 * 255) / a).min(255) as u8;
            c[2] = ((c[2] as u16 * 255) / a).min(255) as u8;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels)
}
