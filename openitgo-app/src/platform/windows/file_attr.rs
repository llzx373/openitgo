//! Windows 文件属性位读写（FM「修改属性/时间戳」对话框，阶段 AC）：
//! `GetFileAttributesW` / `SetFileAttributesW` 操作 READONLY/HIDDEN/SYSTEM/
//! ARCHIVE 四位；其余属性位（目录/压缩/加密等）查询时忽略、写入时原样保留。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM, FILE_FLAGS_AND_ATTRIBUTES,
};

/// 应用内可编辑的四个属性位（与 Explorer 属性框/TC 属性对话框同集）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileAttrBits {
    pub readonly: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// 长路径保护（同 views/file_ops verbatim_path 约定，>240 字符加 `\\?\`；
/// 独立副本防 platform → views 反向依赖）。
fn verbatim(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    if !p.is_absolute()
        || s.starts_with(r"\\?\")
        || s.starts_with(r"\\.\")
        || s.chars().count() <= 240
    {
        return p.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    PathBuf::from(format!(r"\\?\{s}"))
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn bits_to_flags(bits: FileAttrBits) -> FILE_FLAGS_AND_ATTRIBUTES {
    let mut f = FILE_FLAGS_AND_ATTRIBUTES(0);
    if bits.readonly {
        f |= FILE_ATTRIBUTE_READONLY;
    }
    if bits.hidden {
        f |= FILE_ATTRIBUTE_HIDDEN;
    }
    if bits.system {
        f |= FILE_ATTRIBUTE_SYSTEM;
    }
    if bits.archive {
        f |= FILE_ATTRIBUTE_ARCHIVE;
    }
    f
}

fn flags_to_bits(flags: FILE_FLAGS_AND_ATTRIBUTES) -> FileAttrBits {
    FileAttrBits {
        readonly: flags & FILE_ATTRIBUTE_READONLY != FILE_FLAGS_AND_ATTRIBUTES(0),
        hidden: flags & FILE_ATTRIBUTE_HIDDEN != FILE_FLAGS_AND_ATTRIBUTES(0),
        system: flags & FILE_ATTRIBUTE_SYSTEM != FILE_FLAGS_AND_ATTRIBUTES(0),
        archive: flags & FILE_ATTRIBUTE_ARCHIVE != FILE_FLAGS_AND_ATTRIBUTES(0),
    }
}

/// 查询四个属性位。
pub fn query_attributes(path: &Path) -> Result<FileAttrBits, String> {
    let w = wide(&verbatim(path));
    let flags = unsafe { GetFileAttributesW(PCWSTR::from_raw(w.as_ptr())) };
    if flags == INVALID_FILE_ATTRIBUTES {
        return Err(format!("读取属性失败（{}）", path.display()));
    }
    Ok(flags_to_bits(FILE_FLAGS_AND_ATTRIBUTES(flags)))
}

/// 写入四个属性位（其余位保留原值）。
pub fn set_attributes(path: &Path, bits: FileAttrBits) -> Result<(), String> {
    let p = verbatim(path);
    let w = wide(&p);
    let cur = unsafe { GetFileAttributesW(PCWSTR::from_raw(w.as_ptr())) };
    if cur == INVALID_FILE_ATTRIBUTES {
        return Err(format!("读取属性失败（{}）", path.display()));
    }
    const MASK: FILE_FLAGS_AND_ATTRIBUTES = FILE_FLAGS_AND_ATTRIBUTES(
        FILE_ATTRIBUTE_READONLY.0
            | FILE_ATTRIBUTE_HIDDEN.0
            | FILE_ATTRIBUTE_SYSTEM.0
            | FILE_ATTRIBUTE_ARCHIVE.0,
    );
    let next = (FILE_FLAGS_AND_ATTRIBUTES(cur) & !MASK) | bits_to_flags(bits);
    unsafe { SetFileAttributesW(PCWSTR::from_raw(w.as_ptr()), next) }
        .map_err(|e| format!("设置属性失败（{}）: {e}", path.display()))
}

// GetFileAttributesW 失败返回 0xFFFFFFFF（windows crate 未导出该常量名）。
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_readonly_hidden_archive() {
        let dir = std::env::temp_dir().join(format!("openitgo-attr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let orig = query_attributes(&file).expect("query");

        let want = FileAttrBits {
            readonly: true,
            hidden: false,
            system: false,
            archive: true,
        };
        set_attributes(&file, want).expect("set");
        let got = query_attributes(&file).expect("re-query");
        assert_eq!(got, want);
        // 清理前复位（只读文件无法删除）。
        set_attributes(&file, orig).expect("restore");
        let got = query_attributes(&file).expect("final query");
        assert_eq!(got, orig);
        std::fs::remove_file(&file).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn query_missing_file_errors() {
        let p = std::env::temp_dir().join("openitgo-attr-nonexistent-xyz.tmp");
        assert!(query_attributes(&p).is_err());
    }
}
