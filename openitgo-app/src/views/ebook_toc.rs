//! 电子书目录面板的纯函数行模型：按 `EbookChapter.level`（嵌套深度）把扁平
//! 章节列表展开为树形行，支持折叠。不依赖 egui，便于单测。

use openitgo_core::ebook::EbookChapter;
use std::collections::HashSet;

/// 目录树中的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocRow {
    /// 对应 `Ebook.chapters` 里的章节索引。
    pub chapter_index: usize,
    /// 嵌套深度（0 = 顶层），用于缩进。
    pub depth: usize,
    /// 是否有子节点（决定折叠三角的绘制）。
    pub has_children: bool,
    /// 当前是否处于折叠态（其子树已被隐藏）。
    pub collapsed: bool,
}

/// 把扁平章节列表展开为目录树行：节点是其后连续 `level` 更大节点的父；
/// `collapsed` 含某节点章节索引时，其整棵子树隐藏。
pub fn toc_rows(chapters: &[EbookChapter], collapsed: &HashSet<usize>) -> Vec<TocRow> {
    let mut rows = Vec::with_capacity(chapters.len());
    // 处于折叠隐藏中的祖先 level；level 更大的节点都属于其子树。
    let mut hidden_under: Option<usize> = None;
    for (i, ch) in chapters.iter().enumerate() {
        if let Some(level) = hidden_under {
            if ch.level > level {
                continue;
            }
            hidden_under = None;
        }
        let has_children = chapters
            .get(i + 1)
            .is_some_and(|next| next.level > ch.level);
        let is_collapsed = collapsed.contains(&ch.index);
        if is_collapsed && has_children {
            hidden_under = Some(ch.level);
        }
        rows.push(TocRow {
            chapter_index: ch.index,
            depth: ch.level,
            has_children,
            collapsed: is_collapsed,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapters(levels: &[usize]) -> Vec<EbookChapter> {
        levels
            .iter()
            .enumerate()
            .map(|(i, &level)| EbookChapter {
                index: i,
                id: format!("c{i}"),
                href: format!("#c{i}"),
                title: Some(format!("章 {i}")),
                level,
            })
            .collect()
    }

    #[test]
    fn test_flat_chapters_all_top_level() {
        let rows = toc_rows(&chapters(&[0, 0, 0]), &HashSet::new());
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.depth == 0 && !r.has_children));
    }

    #[test]
    fn test_nested_rows_have_depth_and_children() {
        let rows = toc_rows(&chapters(&[0, 1, 2, 1, 0]), &HashSet::new());
        let depths: Vec<usize> = rows.iter().map(|r| r.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 1, 0]);
        assert!(rows[0].has_children);
        assert!(rows[1].has_children);
        assert!(!rows[2].has_children);
        assert!(!rows[3].has_children);
        assert!(!rows[4].has_children);
    }

    #[test]
    fn test_collapsed_hides_subtree() {
        let collapsed: HashSet<usize> = [0].into_iter().collect();
        let rows = toc_rows(&chapters(&[0, 1, 2, 1, 0]), &collapsed);
        let indices: Vec<usize> = rows.iter().map(|r| r.chapter_index).collect();
        assert_eq!(indices, vec![0, 4]);
        assert!(rows[0].collapsed);
        assert!(!rows[1].collapsed);
    }

    #[test]
    fn test_collapsed_leaf_is_noop() {
        let collapsed: HashSet<usize> = [2].into_iter().collect();
        let rows = toc_rows(&chapters(&[0, 1, 2, 1, 0]), &collapsed);
        assert_eq!(rows.len(), 5);
        assert!(rows[2].collapsed);
    }

    #[test]
    fn test_collapsed_middle_level_hides_only_own_subtree() {
        let collapsed: HashSet<usize> = [1].into_iter().collect();
        let rows = toc_rows(&chapters(&[0, 1, 2, 1, 0]), &collapsed);
        let indices: Vec<usize> = rows.iter().map(|r| r.chapter_index).collect();
        assert_eq!(indices, vec![0, 1, 3, 4]);
    }
}
