//! Windows：把「取消最大化时还原到的矩形」写成保存的窗口几何。
//!
//! 启动最大化走「不传 inner_size」方案后创建期最大化存活，但窗口的
//! normal rect 是 Windows 默认值（CW_USEDEFAULT 位置/尺寸），用户首次
//! 取消最大化不会回到保存的几何，持久化还会把默认尺寸写进 settings。
//! 本模块在启动最大化确认后调用一次 `SetWindowPlacement`，仅改写
//! `WINDOWPLACEMENT.rcNormalPosition`（showCmd/flags 保持原值，不改变
//! 当前最大化状态）。
//!
//! 坐标口径：settings 的 `window_size` 是 egui 逻辑点的**客户区**尺寸、
//! `window_pos` 是外框左上角；`rcNormalPosition` 是物理像素的**外框**
//! 矩形。故尺寸先按窗口 DPI 转物理像素，再经 `AdjustWindowRectExForDpi`
//! 加回标题栏/边框（不转换会让每次「最大化→还原」都瘦一圈窗口框架）。

use windows_sys::core::{BOOL, PWSTR};
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongW, GetWindowPlacement, GetWindowThreadProcessId,
    SetWindowPlacement, GWL_EXSTYLE, GWL_STYLE, WINDOWPLACEMENT, WS_MAXIMIZE, WS_MINIMIZE,
};

/// winit 用户窗口的窗口类名（`Winit Thread Event Target` 等内部窗口也挂在
/// 本进程且带 WS_VISIBLE，仅靠可见性过滤会误中——`EnumWindows` 的 Z 序在
/// 启动早期不保证主窗口在前，曾把还原矩形写进事件目标窗口）。反向地，本
/// 函数在启动极早期被调用时主窗口可能尚未置 WS_VISIBLE，故匹配只看类名
/// 不看可见性。
const WINIT_WINDOW_CLASS: &[u16] = &[
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'd' as u16,
    b'o' as u16,
    b'w' as u16,
    b' ' as u16,
    b'C' as u16,
    b'l' as u16,
    b'a' as u16,
    b's' as u16,
    b's' as u16,
    0,
];

struct EnumCtx {
    pid: u32,
    found: HWND,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam 指向 EnumWindows 调用栈上有效的 EnumCtx；
    // hwnd 为系统传入的有效窗口句柄，pid 写出地址有效。
    let ctx = unsafe { &mut *(lparam as *mut EnumCtx) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid != ctx.pid {
        return 1;
    }
    let mut class = [0u16; 32];
    // SAFETY: class 为有效栈缓冲，长度字段与缓冲一致。
    let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr() as PWSTR, class.len() as i32) };
    if len > 0 && class[..len as usize] == WINIT_WINDOW_CLASS[..WINIT_WINDOW_CLASS.len() - 1] {
        ctx.found = hwnd;
        return 0; // 命中即停止枚举
    }
    1
}

/// 查找本进程主窗口（类名为 winit 用户窗口类的顶层窗口）。
fn main_hwnd() -> Option<HWND> {
    let mut ctx = EnumCtx {
        pid: std::process::id(),
        found: std::ptr::null_mut(),
    };
    // SAFETY: ctx 在同步的 EnumWindows 调用期间存活。
    unsafe { EnumWindows(Some(enum_proc), &mut ctx as *mut EnumCtx as LPARAM) };
    (!ctx.found.is_null()).then_some(ctx.found)
}

/// 客户区物理尺寸加回窗口框架（标题栏/边框），失败时原样返回。
fn outer_size_for_dpi(hwnd: HWND, w: i32, h: i32, dpi: u32) -> (i32, i32) {
    // 状态位（最大化/最小化）不影响框架厚度，剥掉再算。
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32 & !(WS_MAXIMIZE | WS_MINIMIZE);
    let exstyle = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    let mut adj = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    // SAFETY: adj 为有效栈结构；style/exstyle/dpi 均取自该窗口；无菜单传 0。
    if unsafe { AdjustWindowRectExForDpi(&mut adj, style, 0, exstyle, dpi) } == 0 {
        return (w, h);
    }
    (adj.right - adj.left, adj.bottom - adj.top)
}

/// 把主窗口的还原矩形设为保存的几何（egui 逻辑点；size 为客户区、pos 为
/// 外框左上角，内部按窗口 DPI 换算物理像素并加回窗口框架）。
pub fn set_saved_restore_rect(size: (f32, f32), pos: Option<(f32, f32)>) {
    let Some(hwnd) = main_hwnd() else {
        return;
    };
    // SAFETY: 全零初始化后立即按文档设置 length。
    let mut wp: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    // SAFETY: hwnd 为本进程有效顶层窗口；wp 为正确初始化的栈结构。
    if unsafe { GetWindowPlacement(hwnd, &mut wp) } == 0 {
        return;
    }
    // SAFETY: hwnd 有效。
    let dpi_raw = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi_raw > 0 {
        dpi_raw as f32 / 96.0
    } else {
        1.0
    };
    let w = (size.0 * scale).round() as i32;
    let h = (size.1 * scale).round() as i32;
    let (w, h) = outer_size_for_dpi(hwnd, w, h, dpi_raw);
    let left = pos
        .map(|p| (p.0 * scale).round() as i32)
        .unwrap_or(wp.rcNormalPosition.left);
    let top = pos
        .map(|p| (p.1 * scale).round() as i32)
        .unwrap_or(wp.rcNormalPosition.top);
    wp.rcNormalPosition = RECT {
        left,
        top,
        right: left + w,
        bottom: top + h,
    };
    // SAFETY: 同 GetWindowPlacement；仅改写 rcNormalPosition。
    unsafe { SetWindowPlacement(hwnd, &wp) };
}
