//! 双栏文件管理器的纯函数行模型：目录优先 + 排序 + 过滤。不依赖 egui，便于单测。
//! 排序约定与 `archive_tree.rs` 对齐：名称比较用 `natural_cmp`（数字感知、
//! 大小写不敏感），mtime 为 None 的条目恒垫底（不随升降序移位）。

use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::SystemTime;

/// 文件系统目录列表中的一个条目（由调用方经 `read_dir` 收集）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    /// 显示名：普通列举 = 文件名；分支视图（Ctrl+B）= `rel_dir/name`
    /// （顶层仅文件名），排序/过滤/type-ahead/渲染统一按此字段工作。
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// 文件大小；目录为 None。
    pub size: Option<u64>,
    pub mtime: Option<SystemTime>,
    pub is_symlink: bool,
    /// 隐藏文件（`.` 开头或 Windows FILE_ATTRIBUTE_HIDDEN）。
    pub is_hidden: bool,
    /// 只读（metadata.permissions().readonly()）。
    pub is_readonly: bool,
    /// Windows FILE_ATTRIBUTE_SYSTEM（非 Windows 恒 false）。
    pub is_system: bool,
    /// 相对当前目录的子目录路径（`/` 分隔；普通列举与分支顶层为空串）。
    pub rel_dir: String,
}

/// 跨平台一致的隐藏判定：文件名以 `.` 开头。
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// 排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Mtime,
    /// 按小写扩展名（无扩展名恒垫底，同 Mtime 的 None 约定），回退 natural_cmp。
    Ext,
    /// 不排序：保持 read_dir 物理序（过滤仍生效），asc=false 整体反向；
    /// dirs_first 分组不适用（物理序本来就交错）。
    Unsorted,
    /// 按属性串（R/H/S，见 attr_string）字典序，无属性恒垫底（同 Ext 约定）。
    Attr,
}

/// 属性列文本（TC 风格属性字母）：R = 只读、H = 隐藏、S = 系统，按此序
/// 拼接；无属性返回空串。排序与列显示共用同一来源。
pub fn attr_string(readonly: bool, hidden: bool, system: bool) -> String {
    let mut s = String::with_capacity(3);
    if readonly {
        s.push('R');
    }
    if hidden {
        s.push('H');
    }
    if system {
        s.push('S');
    }
    s
}

/// 小写扩展名：无 `.`、仅起始 `.`（如 `.env`）或 `.` 结尾 → None。
pub(crate) fn lower_ext(name: &str) -> Option<String> {
    let pos = name.rfind('.')?;
    if pos == 0 {
        return None;
    }
    let ext = &name[pos + 1..];
    if ext.is_empty() {
        return None;
    }
    Some(ext.to_lowercase())
}

/// `;` 分隔的多模式通配匹配（TC「选择组」语义）：支持 `*`（任意串）与
/// `?`（单字符），不区分大小写；空白段忽略，全空 pattern 恒不匹配。
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    pattern
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|p| {
            let pat: Vec<char> = p.to_lowercase().chars().collect();
            wildcard_segment_match(&pat, &name)
        })
}

/// 单段通配匹配（调用方已小写化）：经典星号回溯法。
fn wildcard_segment_match(pat: &[char], name: &[char]) -> bool {
    let (mut pi, mut ni) = (0, 0);
    // 最近一次 `*` 的位置与其时已消费的 name 长度（无 `*` 时 star_p = MAX）。
    let (mut star_p, mut star_n) = (usize::MAX, 0);
    while ni < name.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_p = pi;
            star_n = ni;
            pi += 1;
        } else if star_p != usize::MAX {
            // 失配：回溯到最近的 `*`，让它多消费一个字符。
            pi = star_p + 1;
            star_n += 1;
            ni = star_n;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// 「选择组」模式匹配：返回匹配条目的 UI 行索引（1 起，0 = 「..」行）。
/// rows 为 list_rows 的输出（UI 行索引 = 下标 + 1）；files_only = 只匹配文件。
pub fn select_by_pattern(
    entries: &[FsEntry],
    rows: &[usize],
    pattern: &str,
    files_only: bool,
) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, &i)| {
            let e = &entries[i];
            (!files_only || !e.is_dir) && wildcard_match(pattern, &e.name)
        })
        .map(|(r, _)| r + 1)
        .collect()
}

/// type-to-select 快速定位：从 after_row（UI 行索引）+1 起环形查找第一个
/// `name.to_lowercase().starts_with(needle)` 的可见行，返回 UI 行索引
/// （1 起）。needle 为空 / 无匹配返回 None；after_row=None 从头找。
pub fn type_ahead_match(
    entries: &[FsEntry],
    rows: &[usize],
    needle: &str,
    after_row: Option<usize>,
) -> Option<usize> {
    let needle = needle.to_lowercase();
    if needle.is_empty() || rows.is_empty() {
        return None;
    }
    let count = rows.len();
    // 起点（rows 下标）：after_row 是 UI 行索引（= 下标 + 1），其后一条目
    // 的下标 = after_row；越界/末行取模回卷。None 从下标 0 开始。
    let start = after_row.map_or(0, |r| r % count);
    for off in 0..count {
        let i = (start + off) % count;
        if entries[rows[i]].name.to_lowercase().starts_with(&needle) {
            return Some(i + 1);
        }
    }
    None
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

/// 过滤匹配纯函数（阶段 T）：trim 后为空恒 true；含 `*` 或 `?` 时走
/// `wildcard_match`（`;` 分隔多模式），否则不区分大小写子串。
pub fn filter_matches(filter: &str, name: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if f.contains(['*', '?']) {
        wildcard_match(f, name)
    } else {
        name.to_lowercase().contains(&f.to_lowercase())
    }
}

/// 返回排序+过滤后的条目索引。dirs_first=true 时目录恒排在文件前
/// （false = 目录文件混排、统一排序，fm_dirs_first 设置）；
/// 过滤经 `filter_matches`（子串 / 通配符）；show_hidden=false 时
/// 隐藏条目直接排除。
#[allow(clippy::too_many_arguments)]
pub fn list_rows(
    entries: &[FsEntry],
    filter: &str,
    sort: SortKey,
    asc: bool,
    show_hidden: bool,
    dirs_first: bool,
) -> Vec<usize> {
    let filter = filter.trim();
    // 不排序：read_dir 物理序（过滤/隐藏筛选仍生效），asc=false 整体反向；
    // 不做目录/文件分组（物理序本来就交错，分组反而违背「不排序」语义）。
    if sort == SortKey::Unsorted {
        let mut rows: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| (show_hidden || !e.is_hidden) && filter_matches(filter, &e.name))
            .map(|(i, _)| i)
            .collect();
        if !asc {
            rows.reverse();
        }
        return rows;
    }
    let mut dirs: Vec<usize> = Vec::new();
    let mut files: Vec<usize> = Vec::new();
    for (idx, e) in entries.iter().enumerate() {
        if !show_hidden && e.is_hidden {
            continue;
        }
        if !filter_matches(filter, &e.name) {
            continue;
        }
        if e.is_dir && dirs_first {
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
            SortKey::Ext => {
                // 无扩展名恒垫底（同 Mtime 的 None 约定）：升降序在键内吸收。
                let core = match (lower_ext(&ea.name), lower_ext(&eb.name)) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (None, Some(_)) => return Ordering::Greater,
                    (Some(_), None) => return Ordering::Less,
                    (None, None) => Ordering::Equal,
                };
                let core = if asc { core } else { core.reverse() };
                return core.then_with(|| natural_cmp(&ea.name, &eb.name));
            }
            SortKey::Attr => {
                // 无属性（空串）恒垫底（同 Ext 约定）：升降序在键内吸收。
                let (sa, sb) = (
                    attr_string(ea.is_readonly, ea.is_hidden, ea.is_system),
                    attr_string(eb.is_readonly, eb.is_hidden, eb.is_system),
                );
                let core = match (sa.is_empty(), sb.is_empty()) {
                    (false, false) => sa.cmp(&sb),
                    (true, false) => return Ordering::Greater,
                    (false, true) => return Ordering::Less,
                    (true, true) => Ordering::Equal,
                };
                let core = if asc { core } else { core.reverse() };
                return core.then_with(|| natural_cmp(&ea.name, &eb.name));
            }
            SortKey::Unsorted => unreachable!("Unsorted 在 list_rows 入口早退"),
        };
        ord.then_with(|| natural_cmp(&ea.name, &eb.name))
    };
    dirs.sort_by(|&a, &b| cmp(a, b));
    files.sort_by(|&a, &b| cmp(a, b));
    // Mtime/Ext/Attr 的升降序已在排序键内处理（None/无扩展名/无属性恒垫底），
    // 不再整体反转。
    if !asc && !matches!(sort, SortKey::Mtime | SortKey::Ext | SortKey::Attr) {
        dirs.reverse();
        files.reverse();
    }
    // dirs_first=false 时目录已并入 files 桶统一排序，dirs 为空。
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
            is_readonly: false,
            is_system: false,
            rel_dir: String::new(),
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
        let rows = list_rows(&entries, "", SortKey::Name, true, true, true);
        assert_eq!(names(&entries, &rows), ["aaa", "mmm.txt", "zzz.txt"]);
        // 降序目录仍在前
        let rows = list_rows(&entries, "", SortKey::Name, false, true, true);
        assert_eq!(names(&entries, &rows), ["aaa", "zzz.txt", "mmm.txt"]);
    }

    /// fm_dirs_first=false：目录文件混排，统一按排序键排（目录不额外分组）。
    #[test]
    fn dirs_first_false_mixes_dirs_and_files() {
        let entries = vec![
            entry("zzz.txt", false, Some(1), None),
            entry("aaa", true, None, None),
            entry("mmm.txt", false, Some(5), None),
            entry("bbb", true, None, None),
        ];
        // 名称升序混排
        let rows = list_rows(&entries, "", SortKey::Name, true, true, false);
        assert_eq!(names(&entries, &rows), ["aaa", "bbb", "mmm.txt", "zzz.txt"]);
        // 名称降序混排
        let rows = list_rows(&entries, "", SortKey::Name, false, true, false);
        assert_eq!(names(&entries, &rows), ["zzz.txt", "mmm.txt", "bbb", "aaa"]);
        // 大小升序混排（目录 size=None 按 0 参与）
        let rows = list_rows(&entries, "", SortKey::Size, true, true, false);
        assert_eq!(names(&entries, &rows), ["aaa", "bbb", "zzz.txt", "mmm.txt"]);
    }

    #[test]
    fn sort_by_name_natural_order() {
        let entries = vec![
            entry("vol10", false, Some(1), None),
            entry("vol2", false, Some(1), None),
            entry("vol1", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Name, true, true, true);
        assert_eq!(names(&entries, &rows), ["vol1", "vol2", "vol10"]);
        let rows = list_rows(&entries, "", SortKey::Name, false, true, true);
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
        let rows = list_rows(&entries, "", SortKey::Size, true, true, true);
        assert_eq!(names(&entries, &rows), ["dir", "small", "mid", "big"]);
        let rows = list_rows(&entries, "", SortKey::Size, false, true, true);
        assert_eq!(names(&entries, &rows), ["dir", "big", "mid", "small"]);
    }

    #[test]
    fn sort_by_mtime_none_always_last() {
        let entries = vec![
            entry("no_mtime", false, Some(1), None),
            entry("new", false, Some(1), Some(t(200))),
            entry("old", false, Some(1), Some(t(100))),
        ];
        let rows = list_rows(&entries, "", SortKey::Mtime, true, true, true);
        assert_eq!(names(&entries, &rows), ["old", "new", "no_mtime"]);
        // 降序 None 仍垫底
        let rows = list_rows(&entries, "", SortKey::Mtime, false, true, true);
        assert_eq!(names(&entries, &rows), ["new", "old", "no_mtime"]);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let entries = vec![
            entry("Photos", true, None, None),
            entry("photo1.jpg", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "PHOTO", SortKey::Name, true, true, true);
        assert_eq!(names(&entries, &rows), ["Photos", "photo1.jpg"]);
        // 空白过滤串不过滤
        let rows = list_rows(&entries, "  ", SortKey::Name, true, true, true);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn filter_matches_wildcard_branch() {
        // 含 * / ? 走通配（; 多模式、不区分大小写）。
        assert!(filter_matches("*.zip", "a.zip"));
        assert!(filter_matches("*.ZIP; *.rar", "b.RAR"));
        assert!(filter_matches("vol?.cbz", "Vol1.cbz"));
        assert!(!filter_matches("*.zip", "a.zip.bak"));
        assert!(!filter_matches("vol?.cbz", "vol10.cbz"));
        // 无通配符维持子串语义（含「* 仅作为普通字符不存在于文件名」的边界：
        // 子串串里含 * 必走通配分支）。
        assert!(filter_matches("photo", "Photo1.JPG"));
        assert!(!filter_matches("photo", "notes.txt"));
        // trim 后为空恒匹配。
        assert!(filter_matches("  ", "anything"));
        assert!(filter_matches("", "anything"));
        // list_rows 集成：通配过滤。
        let entries = vec![
            entry("a.zip", false, Some(1), None),
            entry("b.rar", false, Some(1), None),
            entry("c.txt", false, Some(1), None),
            entry("zips", true, None, None),
        ];
        let rows = list_rows(&entries, "*.zip;*.rar", SortKey::Name, true, true, true);
        assert_eq!(names(&entries, &rows), ["a.zip", "b.rar"]);
        // 子串分支不受通配改造影响。
        let rows = list_rows(&entries, "zip", SortKey::Name, true, true, true);
        assert_eq!(names(&entries, &rows), ["zips", "a.zip"]);
    }

    #[test]
    fn show_hidden_false_filters_hidden_entries() {
        let entries = vec![
            entry(".config", true, None, None),
            entry("docs", true, None, None),
            entry(".env", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Name, true, false, true);
        assert_eq!(names(&entries, &rows), ["docs", "notes.txt"]);
        // 默认（true）全量显示
        let rows = list_rows(&entries, "", SortKey::Name, true, true, true);
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
        let rows = list_rows(&entries, "photo", SortKey::Name, true, false, true);
        assert_eq!(names(&entries, &rows), ["photo1.jpg"]);
    }

    #[test]
    fn wildcard_match_star_and_question() {
        assert!(wildcard_match("*.zip", "a.zip"));
        assert!(wildcard_match("*.zip", "a.ZIP"));
        assert!(!wildcard_match("*.zip", "a.zip.bak"));
        assert!(wildcard_match("EP*", "EP2.txt"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("vol?.cbz", "vol1.cbz"));
        assert!(!wildcard_match("vol?.cbz", "vol10.cbz"));
        assert!(wildcard_match("a*c", "abc"));
        assert!(wildcard_match("a*c", "ac"));
        assert!(!wildcard_match("a*c", "abcd"));
        // 连续星号与首尾星号
        assert!(wildcard_match("**a**", "banana"));
        assert!(!wildcard_match("**a**", "xyz"));
    }

    #[test]
    fn wildcard_match_multi_pattern_and_case() {
        assert!(wildcard_match("*.zip;*.rar", "b.rar"));
        assert!(wildcard_match("*.zip; *.rar", "a.zip"));
        assert!(wildcard_match("EP*;vol?", "VOL3"));
        assert!(wildcard_match("*.JPG", "photo.jpg"));
        // 空白段忽略；全空 pattern 恒不匹配
        assert!(!wildcard_match(";;", "a.zip"));
        assert!(!wildcard_match("", "a.zip"));
        assert!(wildcard_match(";*.zip;", "a.zip"));
    }

    #[test]
    fn sort_by_ext_no_ext_always_last() {
        let entries = vec![
            entry("noext", false, Some(1), None),
            entry("b.TXT", false, Some(1), None),
            entry("a.zip", false, Some(1), None),
            entry("c.rar", false, Some(1), None),
        ];
        // 升序：按扩展名字典序，无扩展名垫底；同扩展名回退 natural_cmp。
        let rows = list_rows(&entries, "", SortKey::Ext, true, true, true);
        assert_eq!(names(&entries, &rows), ["c.rar", "b.TXT", "a.zip", "noext"]);
        // 降序：扩展名倒序，无扩展名仍垫底。
        let rows = list_rows(&entries, "", SortKey::Ext, false, true, true);
        assert_eq!(names(&entries, &rows), ["a.zip", "b.TXT", "c.rar", "noext"]);
    }

    #[test]
    fn sort_by_ext_fallback_and_dotfile() {
        let entries = vec![
            entry("b10.zip", false, Some(1), None),
            entry("b2.zip", false, Some(1), None),
            entry(".env", false, Some(1), None),
            entry("trailing.", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Ext, true, true, true);
        // 同扩展名按 natural_cmp；`.env`（仅起始点）与 `trailing.`（点结尾）算无扩展名垫底，
        // 两者并列回退 natural_cmp。
        assert_eq!(
            names(&entries, &rows),
            ["b2.zip", "b10.zip", ".env", "trailing."]
        );
    }

    #[test]
    fn select_by_pattern_returns_ui_row_indices() {
        let entries = vec![
            entry("EP1", true, None, None),
            entry("docs", true, None, None),
            entry("EP2.zip", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
            entry("ep10.rar", false, Some(1), None),
        ];
        // 名称升序：docs, EP1, EP2.zip, ep10.rar, notes.txt
        let rows = list_rows(&entries, "", SortKey::Name, true, true, true);
        assert_eq!(
            names(&entries, &rows),
            ["docs", "EP1", "EP2.zip", "ep10.rar", "notes.txt"]
        );
        // 含目录：EP1（行 2）、EP2.zip（行 3）、ep10.rar（行 4）
        let hit = select_by_pattern(&entries, &rows, "EP*", false);
        assert_eq!(hit, [2, 3, 4]);
        // 仅文件：去掉 EP1 目录
        let hit = select_by_pattern(&entries, &rows, "EP*", true);
        assert_eq!(hit, [3, 4]);
        // 多模式 + 大小写不敏感
        let hit = select_by_pattern(&entries, &rows, "*.ZIP;*.txt", false);
        assert_eq!(hit, [3, 5]);
        // 无匹配 / 空 pattern
        assert!(select_by_pattern(&entries, &rows, "*.7z", false).is_empty());
        assert!(select_by_pattern(&entries, &rows, "  ", false).is_empty());
    }

    #[test]
    fn type_ahead_match_searches_after_row_with_wrap() {
        let entries = vec![
            entry("docs", true, None, None),
            entry("ep1.zip", false, Some(1), None),
            entry("ep2.zip", false, Some(1), None),
            entry("notes.txt", false, Some(1), None),
        ];
        // 名称升序：docs(1), ep1(2), ep2(3), notes(4)
        let rows = list_rows(&entries, "", SortKey::Name, true, true, true);
        assert_eq!(
            names(&entries, &rows),
            ["docs", "ep1.zip", "ep2.zip", "notes.txt"]
        );
        // 从头（None）找第一个 ep* → 行 2
        assert_eq!(type_ahead_match(&entries, &rows, "ep", None), Some(2));
        // 从行 2 之后找 → 行 3
        assert_eq!(type_ahead_match(&entries, &rows, "ep", Some(2)), Some(3));
        // 从行 3 之后找 → 环形回到行 2
        assert_eq!(type_ahead_match(&entries, &rows, "ep", Some(3)), Some(2));
        // 从末行之后找同样回卷
        assert_eq!(type_ahead_match(&entries, &rows, "ep", Some(4)), Some(2));
        // 大小写不敏感（needle 与 name 双向）
        assert_eq!(type_ahead_match(&entries, &rows, "EP1", None), Some(2));
        assert_eq!(type_ahead_match(&entries, &rows, "DOC", None), Some(1));
        // 无匹配 / 空 needle
        assert_eq!(type_ahead_match(&entries, &rows, "zzz", None), None);
        assert_eq!(type_ahead_match(&entries, &rows, "", None), None);
        // 前缀匹配不是子串匹配
        assert_eq!(type_ahead_match(&entries, &rows, "p1", None), None);
    }

    #[test]
    fn attr_string_letters_in_rhs_order() {
        assert_eq!(attr_string(false, false, false), "");
        assert_eq!(attr_string(true, false, false), "R");
        assert_eq!(attr_string(false, true, false), "H");
        assert_eq!(attr_string(false, false, true), "S");
        assert_eq!(attr_string(true, true, true), "RHS");
        assert_eq!(attr_string(false, true, true), "HS");
    }

    #[test]
    fn list_rows_unsorted_keeps_physical_order() {
        let entries = vec![
            entry("zebra.txt", false, Some(1), None),
            entry("docs", true, None, None),
            entry("alpha.txt", false, Some(1), None),
        ];
        // 物理序：不分组、不按名称排序。
        let rows = list_rows(&entries, "", SortKey::Unsorted, true, true, true);
        assert_eq!(names(&entries, &rows), ["zebra.txt", "docs", "alpha.txt"]);
        // asc=false：整体反向。
        let rows = list_rows(&entries, "", SortKey::Unsorted, false, true, true);
        assert_eq!(names(&entries, &rows), ["alpha.txt", "docs", "zebra.txt"]);
        // 过滤仍生效。
        let rows = list_rows(&entries, "*.txt", SortKey::Unsorted, true, true, true);
        assert_eq!(names(&entries, &rows), ["zebra.txt", "alpha.txt"]);
        // 隐藏筛选仍生效。
        let entries = vec![
            entry(".hidden", false, Some(1), None),
            entry("a.txt", false, Some(1), None),
        ];
        let rows = list_rows(&entries, "", SortKey::Unsorted, true, false, true);
        assert_eq!(names(&entries, &rows), ["a.txt"]);
    }

    #[test]
    fn list_rows_attr_sorts_with_empty_last() {
        let mut readonly = entry("b.txt", false, Some(1), None);
        readonly.is_readonly = true;
        let mut system = entry("c.txt", false, Some(1), None);
        system.is_system = true;
        let mut both = entry("d.txt", false, Some(1), None);
        both.is_readonly = true;
        both.is_system = true;
        let entries = vec![entry("a.txt", false, Some(1), None), readonly, system, both];
        // 属性串字典序：R < RS < S，无属性垫底；不随升降序移位。
        let rows = list_rows(&entries, "", SortKey::Attr, true, true, true);
        assert_eq!(names(&entries, &rows), ["b.txt", "d.txt", "c.txt", "a.txt"]);
        let rows = list_rows(&entries, "", SortKey::Attr, false, true, true);
        assert_eq!(names(&entries, &rows), ["c.txt", "d.txt", "b.txt", "a.txt"]);
    }
}
