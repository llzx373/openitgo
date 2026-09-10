//! 驱动器剩余空间（FM 状态栏「剩余 X GB」）：`GetDiskFreeSpaceExW` 取目录
//! 所在卷的可用字节；`volume_key` 从路径提取卷根（盘符根/UNC 前缀）作缓存键。

use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// 路径所在卷的缓存键：盘符根（`C:\`，盘符大写）或 UNC 前缀
/// （`\\server\share\`）；无前缀组件返回 None。
pub fn volume_key(path: &Path) -> Option<String> {
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                Some(format!("{}:\\", (d as char).to_ascii_uppercase()))
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
                "\\\\{}\\{}\\",
                server.to_string_lossy(),
                share.to_string_lossy()
            )),
            _ => None,
        },
        _ => None,
    }
}

/// 路径所在卷的可用字节（`lpFreeBytesAvailableToCaller`——配额/权限下的
/// 调用者可用值，与 Explorer 状态栏口径一致）；失败返回 None。
pub fn free_space(path: &Path) -> Option<u64> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_avail: u64 = 0;
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR::from_raw(wide.as_ptr()),
            Some(&mut free_avail),
            None,
            None,
        )
    }
    .ok()?;
    Some(free_avail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_key_disk_and_unc() {
        assert_eq!(
            volume_key(Path::new(r"c:\foo\bar")),
            Some(r"C:\".to_string())
        );
        assert_eq!(volume_key(Path::new(r"D:\")), Some(r"D:\".to_string()));
        assert_eq!(
            volume_key(Path::new(r"\\server\share\dir")),
            Some(r"\\server\share\".to_string())
        );
        assert_eq!(
            volume_key(Path::new(r"\\?\e:\x")),
            Some(r"E:\".to_string()),
            "VerbatimDisk 归一为盘符根"
        );
        // 相对路径无前缀组件。
        assert_eq!(volume_key(Path::new(r"foo\bar")), None);
    }

    #[test]
    fn free_space_system_drive() {
        let bytes = free_space(Path::new(r"C:\")).expect("C 盘应可查询");
        assert!(bytes > 0);
    }
}
