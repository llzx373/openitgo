//! Windows：启动最大化的「cloak 遮蔽」方案，消除启动瞬间的全屏黑窗闪烁。
//!
//! 根因：winit 创建窗口后对 `builder.maximized` 调 `set_maximized`，其内部
//! `ShowWindow(SW_MAXIMIZE)` 会强制显示尚未呈现任何内容的最大化窗口，
//! 紧随的 `SW_HIDE` 再藏回——中间几十毫秒 DWM 会把这块黑窗合成上屏
//! （实测：启动后约 0.5s 处主窗口 vis=True、最大化全屏，约 60ms 后才重新
//! 隐藏，而首帧内容要更晚才呈现）。该序列发生在任何应用代码运行之前，
//! 应用层无法拦截，故 `main.rs` 不再向 winit 传 `with_maximized`，改为
//! 创建普通隐藏窗口，由本模块在 DWM cloak（`DWMWA_CLOAK`：窗口对合成器
//! 不可见，但 `IsWindowVisible` 等仍为真）保护下完成同样的
//! SW_MAXIMIZE→SW_HIDE，把最大化态写入窗口 show state；待 eframe 首帧
//! 渲染完成、窗口几何验证通过后 `uncloak`，用户第一眼即最终画面。

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_MAXIMIZE};

fn set_cloak(hwnd: usize, cloaked: bool) {
    let value: i32 = cloaked.into();
    // SAFETY: hwnd 为本进程有效顶层窗口（由 restore_rect::main_hwnd 取得）；
    // value 指针在本次调用内有效，cbattribute 与 BOOL 口径一致。失败静默：
    // cloak 不可用时退化为旧方案的短暂闪黑，不影响功能。
    unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_CLOAK as u32,
            &value as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
}

/// 遮蔽主窗口并写入最大化 show state。窗口保持隐藏，全程不被 DWM 合成。
/// 之后 eframe 首帧 reveal（`SW_SHOWNOACTIVATE`）会按该 show state 直接
/// 以最大化形态显示。`hwnd` 为 `restore_rect::main_hwnd` 取得的句柄值。
pub fn cloak_and_maximize(hwnd: usize) {
    set_cloak(hwnd, true);
    // SAFETY: hwnd 为本进程有效顶层窗口。
    unsafe {
        ShowWindow(hwnd as HWND, SW_MAXIMIZE);
        ShowWindow(hwnd as HWND, SW_HIDE);
    }
}

/// 解除遮蔽，窗口随下一次合成显示当前已呈现的内容。
pub fn uncloak(hwnd: usize) {
    set_cloak(hwnd, false);
}
