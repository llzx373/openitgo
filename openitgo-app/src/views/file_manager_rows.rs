//! 双栏文件管理器的纯函数行模型：目录优先 + 排序 + 过滤。不依赖 egui，便于单测。
//! 排序约定与 `archive_tree.rs` 对齐：名称比较用 `natural_cmp`（数字感知、
//! 大小写不敏感），mtime 为 None 的条目恒垫底（不随升降序移位）。

use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::SystemTime;

/// 文件系统目录列表中的一个条目（由调用方经 `read_dir` 收集）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// 文件大小；目录为 None。
    pub size: Option<u64>,
    pub mtime: Option<SystemTime>,
    pub is_symlink: bool,
    /// 隐藏文件（`.` 开头或 Windows FILE_ATTRIBUTE_HIDDEN）。
    pub is_hidden: bool,
}

/// 跨平台一致的隐藏判定：文件名以 `.` 开头。
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// 排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Mtime,
}

/// 数字感知、大小写不敏感的自然排序比较（"EP2" < "EP10"）。
/// 连续数字段按数值比较，其余字符按小写后的字典序逐字符比较。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut s = String::new();
        while let Some(c) = it.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            s.push(*c);
            it.next();
        }
        s
    }

    let mut ca = a.chars().peekable();
    let mut cb = b.chars().peekable();
    loop {
        match (ca.peek().copied(), cb.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_digits(&mut ca);
                let nb = take_digits(&mut cb);
                // 去掉前导零后先比长度再比字典序，即数值比较（无溢出风险）。
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ca.next();
                cb.next();
            }
        }
    }
}

/// 返回排序+过滤后的条目索引。目录恒排在文件前；
/// 过滤为不区分大小写的 name 子串匹配（trim 后为空则不过滤）；
/// show_hidden=false 时隐藏条目直接排除。
pub fn list_rows(
    entries: &[FsEntry],
    filter: &str,
    sort: SortKey,
    asc: bool,
    show_hidden: bool,
) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    let mut dirs: Vec<usize> = Vec::new();
    let mut files: Vec<usize> = Vec::new();
    for (idx, e) in entries.iter().enumerate() {
        if !show_hidden && e.is_hidden {
            continue;
        }
        if !needle.is_empty() && !e.name.to_lowercase().contains(&needle) {
            continue;
        }
        if e.is_dir {
            dirs.push(idx);
        } else {
            files.push(idx);
        }
    }
    let cmp = |a: usize, b: usize| {
        let (ea, eb) = (&entries[a], &entries[b]);
        let ord = match sort {
            SortKey::Name => natural_cmp(&ea.name, &eb.name),
            SortKey::Size => ea.size.unwrap_or(0).cmp(&eb.size.unwrap_or(0)),
            SortKey::Mtime => {
                // None 恒垫底：排序键内吸收升降序，不能靠事后整体 reverse。
                let core = match (ea.mtime, eb.mtime) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (None, Some(_)) => return Ordering::Greater,
                    (Some(_), None) => return Ordering::Less,
                    (None, None) => Ordering::Equal,
                };
                let core = if asc { core } else { core.reverse() };
                return core.then_with(|| natural_cmp(&ea.name, &eb.name));
            }
        };
        ord.then_with(|| natural_cmp(&ea.name, &eb.name))
    };
    dirs.sort_by(|&a, &b| cmp(a, b));
    files.sort_by(|&a, &b| cmp(a, b));
    // Mtime 的升降序已在排序键内处理（None 恒垫底），不再整体反转。
    if !asc && !matches!(sort, SortKey::Mtime) {
        dirs.reverse();
        files.reverse();
    }
    dirs.extend(files);
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, is_dir: bool, size: Option<u64>, mtime: Option<SystemTime>) -> FsEntry {
        FsEntry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir,
            size,
            mtime,
            is_symlink: false,
            is_hidden: is_hidden_name(name),
        }
    }

    fn names<'a>(entries: &'a [FsEntry], rows: &[usize]) -> Vec<&'a str> {
        rows.iter().map(|&i| entries[i].name.as_str()).collect()
    }

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn is_hidden_name_dot_prefix() {
        assert!(is_hidden_name(".gitignore"));
        assert!(is_hidden_name("."));
        assert!(!is_hidden_name("notes.txt"));
        assert!(!is_hidden_name("a.b"));
        assert!(!is_hidden_name(""));
    }

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("EP2", "EP10"), Less);
        assert_eq!(natural_cmp("EP10", "EP2"), Greater);
        assert_eq!(natural_cmp("EP10", "EP10"), Equal);
        // 同前缀数字段：整段数字按数值比较，而不是逐字符
        assert_eq!(natural_cmp("EP2x", "EP10a"), Less);
        assert_eq!(natural_cmp("file9.mkv", "file10.mkv"), Less);
        // 前导零不影响数值比较
        assert_eq!(natural_cmp("EP02", "EP2"), Equal);
    }

    #[test]
    fn natural_cmp_is_case_insensitive() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("ep2", "EP2"), Equal);
        assert_eq!(natural_cmp("ABC", "abd"), Less);
        assert_eq!(natural_cmp("a", "B"), Less);
    }

    #[test]
    fn natural_cmp_non_digit_parts_compare_lexicographically() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("abc", "abd"), Less);
        // 前缀相同则短串在前
        assert_eq!(natural_cmp("abc", "ab"), Greater);
        assert_eq!(natural_cmp("", ""), Equal);
        assert_eq!(natural_cmp("", "a"), Less);
    }

    #[test]
    fn dirs_always_before_files() {
        let entries = vec![
            entry("zzz.txt", false, Some(1), None),
            entry("aaa", true, None, None),
            entry("mmm.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Name, true, true);
        assert_eq!(names(&entries, &rows), ["aaa", "mmm.txt", "zzz.txt"]);
        // 降序目录仍在前
        let rows = list_rows(&entries, "", SortKey::Name, false, true);
        assert_eq!(names(&entries, &rows), ["aaa", "zzz.txt", "mmm.txt"]);
    }

    #[test]
    fn sort_by_name_natural_order() {
        let entries = vec![
            entry("vol10", false, Some(1), None),
            entry("vol2", false, Some(1), None),
            entry("vol1", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Name, true, true);
        assert_eq!(names(&entries, &rows), ["vol1", "vol2", "vol10"]);
        let rows = list_rows(&entries, "", SortKey::Name, false, true);
        assert_eq!(names(&entries, &rows), ["vol10", "vol2", "vol1"]);
    }

    #[test]
    fn sort_by_size() {
        let entries = vec![
            entry("big", false, Some(300), None),
            entry("dir", true, None, None),
            entry("small", false, Some(10), None),
            entry("mid", false, Some(100), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Size, true, true);
        assert_eq!(names(&entries, &rows), ["dir", "small", "mid", "big"]);
        let rows = list_rows(&entries, "", SortKey::Size, false, true);
        assert_eq!(names(&entries, &rows), ["dir", "big", "mid", "small"]);
    }

    #[test]
    fn sort_by_mtime_none_always_last() {
        let entries = vec![
            entry("no_mtime", false, Some(1), None),
            entry("new", false, Some(1), Some(t(200))),
            entry("old", false, Some(1), Some(t(100))),
        ];
        let rows = list_rows(&entries, "", SortKey::Mtime, true, true);
        assert_eq!(names(&entries, &rows), ["old", "new", "no_mtime"]);
        // 降序 None 仍垫底
        let rows = list_rows(&entries, "", SortKey::Mtime, false, true);
        assert_eq!(names(&entries, &rows), ["new", "old", "no_mtime"]);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let entries = vec![
            entry("Photos", true, None, None),
            entry("photo1.jpg", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "PHOTO", SortKey::Name, true, true);
        assert_eq!(names(&entries, &rows), ["Photos", "photo1.jpg"]);
        // 空白过滤串不过滤
        let rows = list_rows(&entries, "  ", SortKey::Name, true, true);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn show_hidden_false_filters_hidden_entries() {
        let entries = vec![
            entry(".config", true, None, None),
            entry("docs", true, None, None),
            entry(".env", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Name, true, false);
        assert_eq!(names(&entries, &rows), ["docs", "notes.txt"]);
        // 默认（true）全量显示
        let rows = list_rows(&entries, "", SortKey::Name, true, true);
        assert_eq!(
            names(&entries, &rows),
            [".config", "docs", ".env", "notes.txt"]
        );
    }

    #[test]
    fn hidden_filter_composes_with_name_filter() {
        let entries = vec![
            entry(".photo", false, Some(1), None),
            entry("photo1.jpg", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "photo", SortKey::Name, true, false);
        assert_eq!(names(&entries, &rows), ["photo1.jpg"]);
    }
}
