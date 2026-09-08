//! 临时解压：双击外部打开与拖出解压共用的临时目录管理。
//! 外部打开拍平到 `temp_dir()/openitgo-open/<包hash>/<basename>`（同名覆盖复用）；
//! 拖出解压放进 `temp_dir()/openitgo-drag/<包hash>/<计数>/` 的本次专属暂存目录，
//! 按包内相对路径保结构落地；启动时由 `clean_stale` 回收超时文件。

use crate::opener::AsyncOpener;
use openitgo_parser::archive::read_entry;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 临时文件回收年龄上限（24h）。
pub const STALE_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// 包对应的临时目录：temp_dir()/openitgo-<kind>/<hash>（同包多次操作复用）。
pub fn temp_root(kind: &str, archive: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    archive.hash(&mut hasher);
    std::env::temp_dir()
        .join(format!("openitgo-{kind}"))
        .join(format!("{:016x}", hasher.finish()))
}

/// 条目名的安全 basename：只取末段（`/` 与 `\\` 都算分隔符），
/// 空或纯点段回退 "entry"，杜绝路径穿越（`..`、绝对路径、盘符）。
pub fn safe_basename(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if last.is_empty() || last.chars().all(|c| c == '.') {
        "entry".to_string()
    } else {
        last.to_string()
    }
}

/// 读取条目内容写入临时目录（同名覆盖），返回写出的完整路径。
pub fn extract_entry_to_temp(
    kind: &str,
    archive: &Path,
    name: &str,
    password: Option<&str>,
) -> Result<PathBuf, String> {
    let bytes = read_entry(archive, name, password).map_err(|e| e.to_string())?;
    let dir = temp_root(kind, archive);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(safe_basename(name));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 条目名按 `/` 与 `\\` 切段，每段做 safe_basename 同款消毒
/// （空/纯点段回退 "entry"；`..` 因此变 "entry"，天然防路径穿越）。
fn sanitized_segments(name: &str) -> Vec<String> {
    name.split(['/', '\\'])
        .map(|seg| {
            let seg = seg.trim();
            if seg.is_empty() || seg.chars().all(|c| c == '.') {
                "entry".to_string()
            } else {
                seg.to_string()
            }
        })
        .collect()
}

/// 拖出解压：读取条目内容按包内相对层级写入本次专属暂存目录
/// （同名覆盖），返回落盘路径。多选/目录拖出因此保留完整目录树。
pub(crate) fn extract_entry_to_staging(
    staging_root: &Path,
    archive: &Path,
    name: &str,
    password: Option<&str>,
) -> Result<PathBuf, String> {
    let bytes = read_entry(archive, name, password).map_err(|e| e.to_string())?;
    let mut path = staging_root.to_path_buf();
    for seg in sanitized_segments(name) {
        path.push(seg);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 拖出 HDROP 负载：暂存目录的顶层项集合（按路径排序）。
/// 拖单个目录时负载即该文件夹本身，Explorer 收到完整目录树。
pub(crate) fn staging_payload(staging: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match std::fs::read_dir(staging) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// 双击外部打开：后台提取到 openitgo-open 临时目录后用系统默认程序打开。
/// 返回 AsyncOpener 供 app 每帧排空（Ok = 已打开，Err → error_message）。
pub fn open_entry_external(
    archive: PathBuf,
    name: String,
    password: Option<String>,
) -> AsyncOpener<PathBuf> {
    AsyncOpener::open(archive, move |p| {
        let path = extract_entry_to_temp("open", p, &name, password.as_deref())?;
        open_with_os(&path)?;
        Ok(path)
    })
}

/// 用系统默认程序打开文件。
pub(crate) fn open_with_os(path: &Path) -> Result<(), String> {
    open_with_os_impl(path).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn open_with_os_impl(path: &Path) -> std::io::Result<()> {
    // start 的第一个引号参数是窗口标题，必须给空串，否则路径被当标题吞掉。
    std::process::Command::new("cmd")
        .args(["/c", "start", ""])
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_with_os_impl(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(path).spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_with_os_impl(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

/// 启动时清理 openitgo-open / openitgo-drag 下超过 max_age 的临时目录
/// （按目录 mtime；一切失败静默，不阻断启动）。
pub fn clean_stale(max_age: Duration) {
    for kind in ["openitgo-open", "openitgo-drag"] {
        let dir = std::env::temp_dir().join(kind);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().is_ok_and(|e| e > max_age))
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_basename_takes_last_component() {
        assert_eq!(safe_basename("a/b/c.txt"), "c.txt");
        assert_eq!(safe_basename("a\\b\\c.txt"), "c.txt");
        assert_eq!(safe_basename("c.txt"), "c.txt");
        // 路径穿越段被剥掉，只剩末段。
        assert_eq!(safe_basename("../../etc/passwd"), "passwd");
        // 空/纯点段回退 "entry"。
        assert_eq!(safe_basename("a/"), "entry");
        assert_eq!(safe_basename(".."), "entry");
        assert_eq!(safe_basename(""), "entry");
    }

    #[test]
    fn temp_root_is_kind_scoped_and_deterministic() {
        let archive = Path::new("/tmp/pack.zip");
        let a = temp_root("open", archive);
        let b = temp_root("open", archive);
        let c = temp_root("drag", archive);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with(std::env::temp_dir()));
        assert!(a.to_string_lossy().contains("openitgo-open"));
    }

    #[test]
    fn extract_entry_to_temp_roundtrip_and_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("pack.zip");
        {
            use std::io::Write;
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("dir/hello.txt", options).unwrap();
            zip.write_all(b"hello").unwrap();
            zip.finish().unwrap();
        }
        let path = extract_entry_to_temp("open", &archive, "dir/hello.txt", None).unwrap();
        assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("hello.txt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        // 同名覆盖：再写一次（改成不同内容需重建包，这里验证不报错且文件在）。
        let path2 = extract_entry_to_temp("open", &archive, "dir/hello.txt", None).unwrap();
        assert_eq!(path, path2);
        // 条目不存在 → Err。
        assert!(extract_entry_to_temp("open", &archive, "nope.txt", None).is_err());
        // 清理：24h 内不删，0 年龄全删。
        clean_stale(STALE_MAX_AGE);
        assert!(path.exists());
        clean_stale(Duration::ZERO);
        assert!(!path.exists());
    }

    /// 造一个含多个嵌套条目的 zip 测试包。
    fn make_zip(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("pack.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        (tmp, archive)
    }

    #[test]
    fn staging_extract_preserves_nested_structure() {
        let (_tmp, archive) = make_zip(&[
            ("dir/sub/hello.txt", b"hello"),
            ("dir/other.txt", b"other"),
            ("root.txt", b"root"),
        ]);
        let staging = tempfile::tempdir().unwrap();
        let root = staging.path();

        let p = extract_entry_to_staging(root, &archive, "dir/sub/hello.txt", None).unwrap();
        assert_eq!(p, root.join("dir").join("sub").join("hello.txt"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");

        extract_entry_to_staging(root, &archive, "dir/other.txt", None).unwrap();
        extract_entry_to_staging(root, &archive, "root.txt", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("dir").join("other.txt")).unwrap(),
            "other"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("root.txt")).unwrap(),
            "root"
        );

        // 顶层负载 = 顶层项集合（排序）：目录自身 + 根文件。
        let payload = staging_payload(root);
        assert_eq!(payload, vec![root.join("dir"), root.join("root.txt")]);
    }

    #[test]
    fn staging_segments_sanitize_traversal_and_empty() {
        let staging = tempfile::tempdir().unwrap();
        let root = staging.path();
        // `..`/空段被消毒为 "entry"，拼出的落点永不越出 staging_root。
        for name in ["a/../x.png", "a//b.png", "..\\..\\evil.txt", "./c.txt"] {
            let mut path = root.to_path_buf();
            for seg in sanitized_segments(name) {
                path.push(seg);
            }
            assert!(
                path.starts_with(root),
                "{name} sanitized path escapes staging root"
            );
            assert!(!path.components().any(|c| c.as_os_str() == ".."));
        }
        assert_eq!(
            sanitized_segments("a/../x.png"),
            vec!["a".to_string(), "entry".to_string(), "x.png".to_string()]
        );
        assert_eq!(
            sanitized_segments("a//b.png"),
            vec!["a".to_string(), "entry".to_string(), "b.png".to_string()]
        );
    }

    #[test]
    fn staging_payload_empty_or_missing_dir() {
        let staging = tempfile::tempdir().unwrap();
        // 空目录 → 空负载。
        assert!(staging_payload(staging.path()).is_empty());
        // 目录不存在 → 空负载（不 panic）。
        assert!(staging_payload(&staging.path().join("nope")).is_empty());
    }
}
