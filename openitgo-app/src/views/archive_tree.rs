//! 压缩包三栏视图的纯函数查询：目录树行构建（左栏）、当前目录直接子项与
//! 中栏行模型（目录优先 + 排序 + 过滤）、面包屑路径段。不依赖 egui，便于单测。
//! 统一约定：条目名按 `/` 与 `\\` 切分；目录 full_path 归一化为 `/`
//! 分隔、无尾部分隔符；选择/解压身份恒用原始 entry.name（不经此处）。

use crate::app::natural_cmp;
use openitgo_parser::archive::ArchiveEntry;
use std::collections::HashSet;

/// 目录树（左栏）中的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// 缩进层级（顶层为 0）。
    pub depth: usize,
    /// 目录名（该级组件名，非完整路径）。
    pub name: String,
    /// 归一化完整路径（`/` 分隔，无尾部分隔符）：折叠集合的 key、
    /// 级联勾选的前缀、current_dir 的值。
    pub full_path: String,
    /// 是否有子目录（决定折叠三角的绘制）。
    pub has_children: bool,
}

/// 目录树节点（构建期内部结构，只含目录）。
struct Node {
    name: String,
    full_path: String,
    children: Vec<Node>,
}

impl Node {
    fn root() -> Self {
        Self {
            name: String::new(),
            full_path: String::new(),
            children: Vec::new(),
        }
    }

    /// 取或建名为 `name` 的子目录节点。
    fn dir_child_mut(&mut self, name: &str, full_path: String) -> &mut Node {
        if let Some(pos) = self.children.iter().position(|c| c.name == name) {
            return &mut self.children[pos];
        }
        self.children.push(Node {
            name: name.to_string(),
            full_path,
            children: Vec::new(),
        });
        self.children.last_mut().expect("just pushed")
    }
}

/// 由条目列表建目录树（`/`、`\\` 均作分隔符；隐式目录补全：
/// 条目 `a/b.png` 而无 `a/` 条目时也生成 `a` 节点）。
fn build_dir_root(entries: &[ArchiveEntry]) -> Node {
    let mut root = Node::root();
    for entry in entries {
        let components: Vec<&str> = entry
            .name
            .split(['/', '\\'])
            .filter(|c| !c.is_empty())
            .collect();
        // 文件条目的最后一个组件是文件名，不进目录树。
        let dir_depth = if entry.is_dir {
            components.len()
        } else {
            components.len().saturating_sub(1)
        };
        let mut node = &mut root;
        let mut path = String::new();
        for comp in &components[..dir_depth] {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(comp);
            node = node.dir_child_mut(comp, path.clone());
        }
    }
    root
}

/// 左栏目录树行：仅目录节点，同级按名字自然序；子树默认展开，
/// `collapsed` 含目录 full_path 时隐藏其子树。
pub fn build_dir_rows(entries: &[ArchiveEntry], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let root = build_dir_root(entries);
    let mut rows = Vec::new();
    emit_dir_rows(&root, 0, collapsed, &mut rows);
    rows
}

fn emit_dir_rows(node: &Node, depth: usize, collapsed: &HashSet<String>, rows: &mut Vec<TreeRow>) {
    let mut dirs: Vec<&Node> = node.children.iter().collect();
    dirs.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    for child in dirs {
        rows.push(TreeRow {
            depth,
            name: child.name.clone(),
            full_path: child.full_path.clone(),
            has_children: !child.children.is_empty(),
        });
        if !collapsed.contains(&child.full_path) {
            emit_dir_rows(child, depth + 1, collapsed, rows);
        }
    }
}

/// 当前目录的直接子项：(文件条目索引（包内顺序）, 直接子目录名（字节序排序去重）)。
/// `current_dir` 为 None 时表示**根目录**（顶层文件 + 顶层子目录）。
/// 目录前缀边界严格（`a` 不命中 `ab/` 与同名文件 `a.txt`），
/// `/` 与 `\\` 均作分隔符；子目录同时来自显式目录条目与更深路径的隐式首组件。
pub fn direct_children(
    entries: &[ArchiveEntry],
    current_dir: Option<&str>,
) -> (Vec<usize>, Vec<String>) {
    let (prefix_slash, prefix_backslash) = match current_dir {
        // 根目录：空前缀匹配所有条目。
        None => (String::new(), String::new()),
        Some(dir) => {
            let dir = dir.trim_end_matches(['/', '\\']);
            if dir.is_empty() {
                (String::new(), String::new())
            } else {
                (format!("{dir}/"), format!("{dir}\\"))
            }
        }
    };
    let mut files = Vec::new();
    let mut subdirs: Vec<String> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let remainder = if prefix_slash.is_empty() {
            e.name.as_str()
        } else if let Some(r) = e.name.strip_prefix(&prefix_slash) {
            r
        } else if let Some(r) = e.name.strip_prefix(&prefix_backslash) {
            r
        } else {
            continue;
        };
        let remainder = remainder.trim_end_matches(['/', '\\']);
        // 目录条目自身（如 dir == "a" 时的 "a/" 条目）不算子项。
        if remainder.is_empty() {
            continue;
        }
        if remainder.contains(['/', '\\']) {
            // 更深层级：首组件是一个（隐式或显式）直接子目录。
            let first = remainder.split(['/', '\\']).next().unwrap_or_default();
            if !first.is_empty() && !subdirs.iter().any(|d| d == first) {
                subdirs.push(first.to_string());
            }
        } else if e.is_dir {
            if !subdirs.iter().any(|d| d == remainder) {
                subdirs.push(remainder.to_string());
            }
        } else {
            files.push(i);
        }
    }
    subdirs.sort();
    (files, subdirs)
}

/// 全部文件条目索引（包内顺序）：「全部文件」扁平模式与过滤搜索用。
pub fn all_file_indices(entries: &[ArchiveEntry]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_dir)
        .map(|(i, _)| i)
        .collect()
}

/// 中栏明细列表的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRow {
    /// 目录行：full_path 归一化（`/` 分隔、无尾部分隔符），name 为该级组件名。
    Dir { full_path: String, name: String },
    /// 文件行：entries 中的索引。
    File { idx: usize },
}

/// 明细列表排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Packed,
}

/// 中栏行模型：flat_all 或过滤激活 → 全包文件行（忽略 current_dir）；
/// 否则当前目录的直接子项，目录行恒在前（自然序升序，不随降序反转），
/// 文件行按 sort/asc 排序（Name 自然序大小写不敏感按显示名；
/// Size/Packed 按数值、同值按名称兜底；desc 仅反转文件行）。
pub fn list_rows(
    entries: &[ArchiveEntry],
    current_dir: Option<&str>,
    flat_all: bool,
    filter: &str,
    sort: SortKey,
    asc: bool,
) -> Vec<ListRow> {
    let needle = filter.trim().to_lowercase();
    let flat = flat_all || !needle.is_empty();
    let (mut files, subdirs) = if flat {
        let indices = if needle.is_empty() {
            all_file_indices(entries)
        } else {
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.is_dir && e.name.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect()
        };
        (indices, Vec::new())
    } else {
        direct_children(entries, current_dir)
    };
    files.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        let ord = match sort {
            SortKey::Name => natural_cmp(file_basename(&ea.name), file_basename(&eb.name)),
            SortKey::Size => ea.size.cmp(&eb.size),
            SortKey::Packed => ea
                .compressed_size
                .unwrap_or(ea.size)
                .cmp(&eb.compressed_size.unwrap_or(eb.size)),
        };
        ord.then_with(|| natural_cmp(&ea.name, &eb.name))
    });
    if !asc {
        files.reverse();
    }
    let mut rows = Vec::with_capacity(subdirs.len() + files.len());
    if !flat {
        let mut dirs = subdirs;
        dirs.sort_by(|a, b| natural_cmp(a, b));
        let parent = current_dir
            .map(|d| d.trim_end_matches(['/', '\\']))
            .filter(|d| !d.is_empty());
        for name in dirs {
            let full_path = match parent {
                Some(p) => format!("{p}/{name}"),
                None => name.clone(),
            };
            rows.push(ListRow::Dir { full_path, name });
        }
    }
    rows.extend(files.into_iter().map(|idx| ListRow::File { idx }));
    rows
}

/// 条目名的最后一段（`/` 与 `\\` 均作分隔符）。
fn file_basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// 面包屑累计路径段：`a/b/c` → `["a", "a/b", "a/b/c"]`（`\\` 同样切分）。
pub fn breadcrumb_paths(dir: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut current = String::new();
    for comp in dir.split(['/', '\\']).filter(|c| !c.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(comp);
        paths.push(current.clone());
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size: 0,
            compressed_size: None,
            mtime: None,
        }
    }

    #[test]
    fn dir_rows_build_nested_indented_dirs() {
        let entries = vec![
            entry("a/b/c.png", false),
            entry("a/d.png", false),
            entry("top.png", false),
        ];
        let rows = build_dir_rows(&entries, &HashSet::new());
        let summary: Vec<(usize, &str)> = rows.iter().map(|r| (r.depth, r.name.as_str())).collect();
        // 只有目录行；"b" 只含文件、无子目录，故无折叠三角。
        assert_eq!(summary, vec![(0, "a"), (1, "b")]);
        assert_eq!(rows[0].full_path, "a");
        assert_eq!(rows[1].full_path, "a/b");
        assert!(rows[0].has_children);
        assert!(!rows[1].has_children);
    }

    #[test]
    fn dir_rows_merge_explicit_and_implicit_and_handle_backslash() {
        // 显式 "a/" 与隐式 "a"（来自 a/b.png）合并为单节点。
        let entries = vec![entry("a/", true), entry("a/b.png", false)];
        let rows = build_dir_rows(&entries, &HashSet::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "a");
        // 反斜杠同样视为分隔符。
        let entries = vec![entry("x\\y.png", false)];
        let rows = build_dir_rows(&entries, &HashSet::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_path, "x");
    }

    #[test]
    fn dir_rows_sort_natural_and_respect_collapse() {
        let entries = vec![
            entry("m/f.png", false),
            entry("a10/f.png", false),
            entry("a2/f.png", false),
            entry("a2/b/f.png", false),
        ];
        let rows = build_dir_rows(&entries, &HashSet::new());
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // 自然序：a2 < a10（字节序下 a10 会排在 a2 前面）。
        assert_eq!(names, vec!["a2", "b", "a10", "m"]);
        // 折叠 "a2" 隐藏其子目录（"a2" 行保留且仍标 has_children）。
        let collapsed = HashSet::from(["a2".to_string()]);
        let rows = build_dir_rows(&entries, &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a2", "a10", "m"]);
        assert!(rows[0].has_children);
    }

    #[test]
    fn dir_rows_empty_entries_and_blank_names() {
        assert!(build_dir_rows(&[], &HashSet::new()).is_empty());
        // 全为分隔符的名字不产生目录节点。
        assert!(build_dir_rows(&[entry("/", true)], &HashSet::new()).is_empty());
    }

    #[test]
    fn direct_children_root_returns_top_level_items() {
        let entries = vec![
            entry("d/", true),
            entry("d/a.png", false),
            entry("b.txt", false),
            entry("e/f/g.png", false),
        ];
        let (files, subdirs) = direct_children(&entries, None);
        // 根目录：顶层文件 + 顶层子目录（含更深路径的隐式首组件）。
        assert_eq!(files, vec![2]);
        assert_eq!(subdirs, vec!["d", "e"]);
    }

    #[test]
    fn all_file_indices_returns_all_files_in_archive_order() {
        let entries = vec![
            entry("d/", true),
            entry("d/a.png", false),
            entry("b.txt", false),
        ];
        assert_eq!(all_file_indices(&entries), vec![1, 2]);
    }

    #[test]
    fn direct_children_splits_files_and_subdirs() {
        let entries = vec![
            entry("a/x.png", false),
            entry("a/y.txt", false),
            entry("a/sub/z.png", false),
            entry("a/sub2/", true),
            entry("a/", true),
            entry("b.png", false),
        ];
        let (files, subdirs) = direct_children(&entries, Some("a"));
        // 直接文件（包内顺序），更深层文件不进列表。
        assert_eq!(files, vec![0, 1]);
        // 隐式（sub，来自 a/sub/z.png）与显式（sub2）子目录，字节序排序。
        assert_eq!(subdirs, vec!["sub", "sub2"]);
    }

    #[test]
    fn direct_children_boundary_and_backslash() {
        let entries = vec![
            entry("a/b.png", false),
            entry("ab/c.png", false),
            entry("a.txt", false),
            entry("a\\d.png", false),
        ];
        let (files, subdirs) = direct_children(&entries, Some("a"));
        // 边界：`a/` 不命中 `ab/` 与同名文件 `a.txt`；反斜杠条目算直接子项。
        assert_eq!(files, vec![0, 3]);
        assert!(subdirs.is_empty());
        // 尾部斜杠剥掉后照常工作。
        let (files, _) = direct_children(&entries, Some("a/"));
        assert_eq!(files, vec![0, 3]);
    }

    #[test]
    fn breadcrumb_paths_accumulate_segments() {
        assert_eq!(breadcrumb_paths("a"), vec!["a"]);
        assert_eq!(breadcrumb_paths("a/b/c"), vec!["a", "a/b", "a/b/c"]);
        assert_eq!(breadcrumb_paths("a\\b"), vec!["a", "a/b"]);
        assert_eq!(breadcrumb_paths("a/b/"), vec!["a", "a/b"]);
        assert!(breadcrumb_paths("").is_empty());
    }

    fn sized(name: &str, is_dir: bool, size: u64, packed: Option<u64>) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size,
            compressed_size: packed,
            mtime: None,
        }
    }

    fn row_names(rows: &[ListRow], entries: &[ArchiveEntry]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                ListRow::Dir { name, .. } => format!("{name}/"),
                ListRow::File { idx } => entries[*idx].name.clone(),
            })
            .collect()
    }

    #[test]
    fn list_rows_dirs_first_then_files_natural_name_order() {
        let entries = vec![
            sized("m/f.png", false, 1, None),
            sized("page10.png", false, 1, None),
            sized("a/f.png", false, 1, None),
            sized("page2.png", false, 1, None),
        ];
        let rows = list_rows(&entries, None, false, "", SortKey::Name, true);
        // 目录恒在前（自然序），文件按显示名自然序（page2 < page10）。
        assert_eq!(
            row_names(&rows, &entries),
            vec!["a/", "m/", "page2.png", "page10.png"]
        );
    }

    #[test]
    fn list_rows_sort_by_size_and_desc_keeps_dirs_ascending() {
        let entries = vec![
            sized("b/f.png", false, 1, None),
            sized("a/f.png", false, 1, None),
            sized("big.png", false, 100, None),
            sized("small.png", false, 1, None),
        ];
        let rows = list_rows(&entries, None, false, "", SortKey::Size, true);
        assert_eq!(
            row_names(&rows, &entries),
            vec!["a/", "b/", "small.png", "big.png"]
        );
        // 降序只反转文件行，目录行保持自然序升序。
        let rows = list_rows(&entries, None, false, "", SortKey::Size, false);
        assert_eq!(
            row_names(&rows, &entries),
            vec!["a/", "b/", "big.png", "small.png"]
        );
    }

    #[test]
    fn list_rows_sort_by_packed_with_fallback() {
        let entries = vec![
            sized("x.png", false, 10, Some(8)),
            sized("y.png", false, 10, None),
            sized("z.png", false, 10, Some(2)),
        ];
        // 无压缩大小按解压大小计；同值按名称兜底。
        let rows = list_rows(&entries, None, false, "", SortKey::Packed, true);
        assert_eq!(row_names(&rows, &entries), vec!["z.png", "x.png", "y.png"]);
    }

    #[test]
    fn list_rows_flat_and_filter_ignore_current_dir() {
        let entries = vec![
            sized("dir/in.png", false, 1, None),
            sized("dir/sub/deep.png", false, 1, None),
            sized("top.txt", false, 1, None),
        ];
        // flat_all：全包文件扁平，无目录行。
        let rows = list_rows(&entries, Some("dir"), true, "", SortKey::Name, true);
        assert_eq!(
            row_names(&rows, &entries),
            vec!["dir/sub/deep.png", "dir/in.png", "top.txt"]
        );
        // 过滤激活：忽略 current_dir 全包匹配（大小写不敏感）。
        let rows = list_rows(&entries, None, false, "PNG", SortKey::Name, true);
        assert_eq!(
            row_names(&rows, &entries),
            vec!["dir/sub/deep.png", "dir/in.png"]
        );
    }

    #[test]
    fn list_rows_dir_full_path_accumulates() {
        let entries = vec![sized("a/b/c.png", false, 1, None)];
        let rows = list_rows(&entries, None, false, "", SortKey::Name, true);
        assert_eq!(
            rows,
            vec![ListRow::Dir {
                full_path: "a".to_string(),
                name: "a".to_string()
            }]
        );
        let rows = list_rows(&entries, Some("a"), false, "", SortKey::Name, true);
        assert_eq!(
            rows,
            vec![ListRow::Dir {
                full_path: "a/b".to_string(),
                name: "b".to_string()
            }]
        );
    }
}
