//! macOS 系统剪贴板文件列表（阶段 Z，best-effort）：NSPasteboard 写/读
//! 文件 URL 数组。macOS 无剪贴板 cut 惯例——`set_files` 忽略 cut 参数
//! （恒 copy），`get_files` 返回的 is_cut 恒 false。
//!
//! 用法风格与 `dock_open` 一致：raw `msg_send!` + `#[link(Cocoa)]`，
//! 不引入 objc2-app-kit/objc2-foundation（树内 objc2-foundation 0.2 与
//! objc2 0.6 不兼容）。**本模块未经本地编译验证（开发机只有 Windows
//! target），依赖 CI/真机验证。**

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{class, msg_send};

#[link(name = "Cocoa", kind = "framework")]
extern "C" {}

/// &str → autoreleased NSString（UTF-8；含 NUL 的串返回 None）。
fn ns_string(s: &str) -> Option<*mut AnyObject> {
    let c = CString::new(s).ok()?;
    let obj: *mut AnyObject =
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] };
    (!obj.is_null()).then_some(obj)
}

/// NSURL → 路径（经 -[NSURL path] → UTF8String）。
fn path_from_nsurl(url: *mut AnyObject) -> Option<PathBuf> {
    unsafe {
        let path: *mut AnyObject = msg_send![url, path];
        if path.is_null() {
            return None;
        }
        let utf8: *const c_char = msg_send![path, UTF8String];
        if utf8.is_null() {
            return None;
        }
        Some(PathBuf::from(
            CStr::from_ptr(utf8).to_string_lossy().into_owned(),
        ))
    }
}

/// 把路径表写入系统剪贴板（先 clearContents 再 writeObjects NSURL 数组）。
/// macOS 无 cut 惯例，`_cut` 忽略（恒 copy）。
pub fn set_files(paths: &[PathBuf], _cut: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可复制的文件".to_string());
    }
    unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        if pb.is_null() {
            return Err("NSPasteboard 不可用".to_string());
        }
        let _: isize = msg_send![pb, clearContents];
        let mut urls: Vec<*mut AnyObject> = Vec::with_capacity(paths.len());
        for p in paths {
            let s = p.as_os_str().to_string_lossy();
            let ns = ns_string(&s).ok_or_else(|| "NSString 编码失败".to_string())?;
            let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: ns];
            if url.is_null() {
                return Err("NSURL 创建失败".to_string());
            }
            urls.push(url);
        }
        let arr: *mut AnyObject =
            msg_send![class!(NSArray), arrayWithObjects: urls.as_ptr() count: urls.len()];
        if arr.is_null() {
            return Err("NSArray 创建失败".to_string());
        }
        let ok: Bool = msg_send![pb, writeObjects: arr];
        if ok.as_bool() {
            Ok(())
        } else {
            Err("NSPasteboard 写入失败".to_string())
        }
    }
}

/// 从系统剪贴板读文件路径表（readObjectsForClasses NSURL）；无文件内容
/// 返回 None。macOS 无 cut 惯例，is_cut 恒 false。
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        if pb.is_null() {
            return None;
        }
        let cls: *const AnyClass = class!(NSURL);
        let classes = [cls as *mut AnyObject];
        let class_arr: *mut AnyObject =
            msg_send![class!(NSArray), arrayWithObjects: classes.as_ptr() count: classes.len()];
        if class_arr.is_null() {
            return None;
        }
        let objs: *mut AnyObject =
            msg_send![pb, readObjectsForClasses: class_arr options: std::ptr::null::<AnyObject>()];
        if objs.is_null() {
            return None;
        }
        let count: usize = msg_send![objs, count];
        let mut out = Vec::new();
        for i in 0..count {
            let url: *mut AnyObject = msg_send![objs, objectAtIndex: i];
            if !url.is_null() {
                if let Some(p) = path_from_nsurl(url) {
                    out.push(p);
                }
            }
        }
        if out.is_empty() {
            None
        } else {
            Some((out, false))
        }
    }
}

/// 清空系统剪贴板。
pub fn clear() {
    unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        if !pb.is_null() {
            let _: isize = msg_send![pb, clearContents];
        }
    }
}
