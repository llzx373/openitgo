//! 压缩包三栏视图的纯函数查询：目录树行构建（左栏）、当前目录直接子项
//! （中栏）、面包屑路径段。不依赖 egui，便于单测。
//! 统一约定：条目名按 `/` 与 `\\` 切分；目录 full_path 归一化为 `/`
//! 分隔、无尾部分隔符；选择/解压身份恒用原始 entry.name（不经此处）。

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

/// 左栏目录树行：仅目录节点，同级按名字节序；子树默认展开，
/// `collapsed` 含目录 full_path 时隐藏其子树。
pub fn build_dir_rows(entries: &[ArchiveEntry], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let root = build_dir_root(entries);
    let mut rows = Vec::new();
    emit_dir_rows(&root, 0, collapsed, &mut rows);
    rows
}

fn emit_dir_rows(node: &Node, depth: usize, collapsed: &HashSet<String>, rows: &mut Vec<TreeRow>) {
    let mut dirs: Vec<&Node> = node.children.iter().collect();
    dirs.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
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
/// `current_dir` 为 None 时返回全部文件条目索引（「全部文件」扁平模式），
/// 子目录列表为空。目录前缀边界严格（`a` 不命中 `ab/` 与同名文件 `a.txt`），
/// `/` 与 `\\` 均作分隔符；子目录同时来自显式目录条目与更深路径的隐式首组件。
pub fn direct_children(
    entries: &[ArchiveEntry],
    current_dir: Option<&str>,
) -> (Vec<usize>, Vec<String>) {
    let Some(dir) = current_dir else {
        return (
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.is_dir)
                .map(|(i, _)| i)
                .collect(),
            Vec::new(),
        );
    };
    let dir = dir.trim_end_matches(['/', '\\']);
    let prefix_slash = format!("{dir}/");
    let prefix_backslash = format!("{dir}\\");
    let mut files = Vec::new();
    let mut subdirs: Vec<String> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let remainder = if let Some(r) = e.name.strip_prefix(&prefix_slash) {
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
    fn dir_rows_sort_byte_wise_and_respect_collapse() {
        let entries = vec![
            entry("m/f.png", false),
            entry("a/f.png", false),
            entry("a/b/f.png", false),
        ];
        let rows = build_dir_rows(&entries, &HashSet::new());
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "m"]);
        // 折叠 "a" 隐藏其子目录（"a" 行保留且仍标 has_children）。
        let collapsed = HashSet::from(["a".to_string()]);
        let rows = build_dir_rows(&entries, &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "m"]);
        assert!(rows[0].has_children);
    }

    #[test]
    fn dir_rows_empty_entries_and_blank_names() {
        assert!(build_dir_rows(&[], &HashSet::new()).is_empty());
        // 全为分隔符的名字不产生目录节点。
        assert!(build_dir_rows(&[entry("/", true)], &HashSet::new()).is_empty());
    }

    #[test]
    fn direct_children_flat_mode_returns_all_files() {
        let entries = vec![
            entry("d/", true),
            entry("d/a.png", false),
            entry("b.txt", false),
        ];
        let (files, subdirs) = direct_children(&entries, None);
        assert_eq!(files, vec![1, 2]);
        assert!(subdirs.is_empty());
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
}
