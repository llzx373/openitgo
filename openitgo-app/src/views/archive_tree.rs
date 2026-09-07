//! 压缩包树形视图的纯函数行构建：把扁平条目列表展开为带缩进层级的
//! 目录/文件行，供 `ArchiveView` 树形模式渲染。不依赖 egui，便于单测。

use openitgo_parser::archive::ArchiveEntry;
use std::collections::HashSet;

/// 树形模式下的一行：目录行（可折叠/级联勾选）或文件行（对应一个条目）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// 缩进层级（顶层为 0）。
    pub depth: usize,
    /// 展示名（该级组件名，非完整路径）。
    pub name: String,
    /// 归一化完整路径（`/` 分隔，无尾部分隔符）：目录行用作折叠集合的
    /// key 与级联勾选的前缀；文件行仅供调试，选择/解压恒用原始 entry.name。
    pub full_path: String,
    pub is_dir: bool,
    /// 文件行对应 `entries` 的下标；目录行（含显式目录条目）恒为 None。
    pub entry_idx: Option<usize>,
    /// 是否有可见子节点（决定折叠三角的绘制）。
    pub has_children: bool,
}

/// 树节点（构建期内部结构）。
struct Node {
    name: String,
    full_path: String,
    is_dir: bool,
    entry_idx: Option<usize>,
    children: Vec<Node>,
}

impl Node {
    fn root() -> Self {
        Self {
            name: String::new(),
            full_path: String::new(),
            is_dir: true,
            entry_idx: None,
            children: Vec::new(),
        }
    }

    /// 取或建名为 `name` 的子目录节点。
    fn dir_child_mut(&mut self, name: &str, full_path: String) -> &mut Node {
        if let Some(pos) = self
            .children
            .iter()
            .position(|c| c.is_dir && c.name == name)
        {
            return &mut self.children[pos];
        }
        self.children.push(Node {
            name: name.to_string(),
            full_path,
            is_dir: true,
            entry_idx: None,
            children: Vec::new(),
        });
        self.children.last_mut().expect("just pushed")
    }
}

/// 把扁平条目列表展开为树形行。
///
/// - 条目名按 `/` 与 `\\` 切分（`\\` 仅参与建树，文件行身份仍是原始名）。
/// - 隐式目录（条目 `a/b.png` 而无 `a/` 条目）也会生成目录行。
/// - 每级排序：目录在前，同级内按名字节序。
/// - 目录子树默认展开；`collapsed` 含其 full_path 时隐藏子树。
/// - 过滤（大小写不敏感子串，与列表模式同语义）：保留匹配的文件行及其
///   祖先目录行；过滤激活时忽略折叠集合（展示全部匹配项）。
pub fn build_tree_rows(
    entries: &[ArchiveEntry],
    collapsed: &HashSet<String>,
    filter: &str,
) -> Vec<TreeRow> {
    let mut root = Node::root();
    for (idx, entry) in entries.iter().enumerate() {
        let components: Vec<&str> = entry
            .name
            .split(['/', '\\'])
            .filter(|c| !c.is_empty())
            .collect();
        if components.is_empty() {
            continue;
        }
        let dir_depth = if entry.is_dir {
            components.len()
        } else {
            components.len() - 1
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
        if !entry.is_dir {
            let name = components[components.len() - 1];
            let full_path = if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            };
            node.children.push(Node {
                name: name.to_string(),
                full_path,
                is_dir: false,
                entry_idx: Some(idx),
                children: Vec::new(),
            });
        }
    }

    let needle = filter.trim().to_lowercase();
    let filter_active = !needle.is_empty();
    if filter_active {
        prune(&mut root, &needle, entries);
    }

    let mut rows = Vec::new();
    emit_rows(&root, 0, collapsed, filter_active, &mut rows);
    rows
}

/// 过滤剪枝：文件节点按条目名匹配保留，目录节点有存活子节点才保留。
/// 返回该节点是否有存活子节点（root 调用的返回值无意义）。
fn prune(node: &mut Node, needle: &str, entries: &[ArchiveEntry]) -> bool {
    node.children.retain_mut(|child| {
        if child.is_dir {
            prune(child, needle, entries)
        } else {
            child
                .entry_idx
                .is_some_and(|i| entries[i].name.to_lowercase().contains(needle))
        }
    });
    !node.children.is_empty()
}

fn emit_rows(
    node: &Node,
    depth: usize,
    collapsed: &HashSet<String>,
    filter_active: bool,
    rows: &mut Vec<TreeRow>,
) {
    let mut ordered: Vec<&Node> = node.children.iter().collect();
    ordered.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
    });
    for child in ordered {
        let hidden = child.is_dir && !filter_active && collapsed.contains(&child.full_path);
        rows.push(TreeRow {
            depth,
            name: child.name.clone(),
            full_path: child.full_path.clone(),
            is_dir: child.is_dir,
            entry_idx: child.entry_idx,
            has_children: !child.children.is_empty(),
        });
        if child.is_dir && !hidden {
            emit_rows(child, depth + 1, collapsed, filter_active, rows);
        }
    }
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

    fn rows_of(entries: &[ArchiveEntry]) -> Vec<TreeRow> {
        build_tree_rows(entries, &HashSet::new(), "")
    }

    #[test]
    fn nested_entries_build_indented_rows() {
        let entries = vec![
            entry("a/b/c.png", false),
            entry("a/d.png", false),
            entry("top.png", false),
        ];
        let rows = rows_of(&entries);
        let summary: Vec<(usize, &str, bool)> = rows
            .iter()
            .map(|r| (r.depth, r.name.as_str(), r.is_dir))
            .collect();
        assert_eq!(
            summary,
            vec![
                (0, "a", true),
                (1, "b", true),
                (2, "c.png", false),
                (1, "d.png", false),
                (0, "top.png", false),
            ]
        );
        // 文件行带 entry_idx，目录行不带。
        let c_row = rows.iter().find(|r| r.name == "c.png").unwrap();
        assert_eq!(c_row.entry_idx, Some(0));
        assert_eq!(c_row.full_path, "a/b/c.png");
        assert!(rows
            .iter()
            .find(|r| r.name == "a")
            .unwrap()
            .entry_idx
            .is_none());
        assert!(rows.iter().find(|r| r.name == "a").unwrap().has_children);
    }

    #[test]
    fn implicit_dirs_appear_as_dir_rows() {
        // 无显式 "a/" 目录条目，仅文件路径蕴含。
        let entries = vec![entry("a/b.png", false)];
        let rows = rows_of(&entries);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].name, "a");
        assert_eq!(rows[0].entry_idx, None);
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn explicit_dir_entry_merges_with_implicit() {
        let entries = vec![entry("a/", true), entry("a/b.png", false)];
        let rows = rows_of(&entries);
        // 显式与隐式 "a" 合并为单行。
        assert_eq!(rows.iter().filter(|r| r.name == "a").count(), 1);
        // 反斜杠同样视为分隔符。
        let entries = vec![entry("x\\y.png", false)];
        let rows = rows_of(&entries);
        assert!(rows[0].is_dir && rows[0].name == "x");
        assert_eq!(rows[1].full_path, "x/y.png");
        assert_eq!(rows[1].entry_idx, Some(0));
    }

    #[test]
    fn filter_keeps_matching_files_and_ancestors() {
        let entries = vec![
            entry("a/b/page1.png", false),
            entry("a/c/notes.txt", false),
            entry("readme.md", false),
        ];
        let rows = build_tree_rows(&entries, &HashSet::new(), "PAGE");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // 匹配文件与其祖先目录保留，不匹配的子树与顶层文件消失。
        assert_eq!(names, vec!["a", "b", "page1.png"]);
    }

    #[test]
    fn collapse_hides_subtree_and_filter_ignores_collapse() {
        let entries = vec![entry("a/b.png", false), entry("c.png", false)];
        let mut collapsed = HashSet::new();
        collapsed.insert("a".to_string());
        let rows = build_tree_rows(&entries, &collapsed, "");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c.png"]);
        // 折叠的目录行仍标有 has_children（三角显示为收起态）。
        assert!(rows[0].has_children);
        // 过滤激活时忽略折叠集合。
        let rows = build_tree_rows(&entries, &collapsed, "b.png");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b.png"]);
    }

    #[test]
    fn sort_is_dirs_first_then_byte_wise() {
        let entries = vec![
            entry("z.txt", false),
            entry("B.txt", false),
            entry("m/", true),
            entry("a/", true),
        ];
        let rows = rows_of(&entries);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // 目录在前（字节序 a < m），文件字节序 B(0x42) < z(0x7A)。
        assert_eq!(names, vec!["a", "m", "B.txt", "z.txt"]);
    }

    #[test]
    fn empty_entries_and_blank_names() {
        assert!(rows_of(&[]).is_empty());
        // 全为分隔符的名字不产生行。
        assert!(rows_of(&[entry("/", true)]).is_empty());
    }
}
