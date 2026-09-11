//! 有限撤销（阶段 AG）：会话内撤销栈（上限 32，超出丢弃最旧），
//! Ctrl+Z / 右键菜单顶部动态项撤销栈顶。
//!
//! 可撤销：Copy（撤销 = 目标集走回收站删除任务）、Move（撤销 = 移回）、
//! Rename/MultiRename（反向改名）、NewFile/NewDir（撤销 = 回收站）。
//! 不可撤销（不入栈也不清空栈，TC 同）：Delete（回收站已可恢复）、
//! 覆盖写、Compress/Split/Merge、属性/时间戳修改。
//!
//! 纯函数部分：`plan_undo`（操作 → 撤销计划）与 `precheck`（存在性预检）
//! 不碰 UI；执行侧在 `file_manager.rs::undo_top`。

use std::path::PathBuf;

/// 撤销栈上限（超出丢弃最旧）。
pub const UNDO_LIMIT: usize = 32;

/// 一条可撤销操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoableOp {
    /// 复制完成的目标集（AutoRename 后为准）：撤销 = 逐个移入回收站。
    Copy { targets: Vec<PathBuf> },
    /// 移动完成的 (dest, src) 配对（源已移除项）：撤销 = dest 改名回 src。
    Move { pairs: Vec<(PathBuf, PathBuf)> },
    /// 单项重命名 from → to：撤销 = to 改名回 from。
    Rename { from: PathBuf, to: PathBuf },
    /// 批量重命名（from, to) 原方向配对：撤销 = 逆序逐个 to 改名回 from
    /// （逆序避免互换类命名（A→B、B→A）回改时撞上未撤销项）。
    MultiRename { pairs: Vec<(PathBuf, PathBuf)> },
    /// 新建文本文件：撤销 = 移入回收站。
    NewFile { path: PathBuf },
    /// 新建文件夹：撤销 = 移入回收站。
    NewDir { path: PathBuf },
}

impl UndoableOp {
    /// 右键菜单/Ctrl+Z 提示文案（「撤销 {label}」）。
    pub fn label(&self) -> String {
        match self {
            UndoableOp::Copy { targets } => format!("复制 {} 项", targets.len()),
            UndoableOp::Move { pairs } => format!("移动 {} 项", pairs.len()),
            UndoableOp::Rename { .. } => "重命名".to_string(),
            UndoableOp::MultiRename { pairs } => format!("批量重命名 {} 项", pairs.len()),
            UndoableOp::NewFile { .. } => "新建文件".to_string(),
            UndoableOp::NewDir { .. } => "新建文件夹".to_string(),
        }
    }
}

/// 撤销计划（`plan_undo` 产出）：执行侧只需这两种语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoPlan {
    /// 目标集移入回收站（Copy/NewFile/NewDir 的撤销；走 file_ops Delete
    /// 任务，回收站保底）。
    Trash { targets: Vec<PathBuf> },
    /// 逐个改名回（Move/Rename/MultiRename 的撤销）：(当前位置, 改回位置)。
    RenameBack { pairs: Vec<(PathBuf, PathBuf)> },
}

/// 操作 → 撤销计划（纯函数，不查磁盘）。
pub fn plan_undo(op: &UndoableOp) -> UndoPlan {
    match op {
        UndoableOp::Copy { targets } => UndoPlan::Trash {
            targets: targets.clone(),
        },
        UndoableOp::Move { pairs } => UndoPlan::RenameBack {
            pairs: pairs.clone(),
        },
        UndoableOp::Rename { from, to } => UndoPlan::RenameBack {
            pairs: vec![(to.clone(), from.clone())],
        },
        UndoableOp::MultiRename { pairs } => UndoPlan::RenameBack {
            pairs: pairs
                .iter()
                .rev()
                .map(|(f, t)| (t.clone(), f.clone()))
                .collect(),
        },
        UndoableOp::NewFile { path } | UndoableOp::NewDir { path } => UndoPlan::Trash {
            targets: vec![path.clone()],
        },
    }
}

/// 存在性预检（执行前一次性过滤，执行期不再询问）：
/// Trash —— 目标仍在才纳入（已不在 = 跳过「目标已不存在」）；
/// RenameBack —— 当前位置在且改回位置空闲才纳入（当前位置缺失跳过
/// 「已不在撤销位置」；改回位置被占用跳过「原位置已被占用」，防覆盖
/// 第三方文件）。返回 (可执行计划, 跳过项与原因)。
pub fn precheck(plan: UndoPlan) -> (UndoPlan, Vec<(PathBuf, String)>) {
    match plan {
        UndoPlan::Trash { targets } => {
            let mut keep = Vec::new();
            let mut skipped = Vec::new();
            for t in targets {
                if t.exists() {
                    keep.push(t);
                } else {
                    skipped.push((t, "目标已不存在".to_string()));
                }
            }
            (UndoPlan::Trash { targets: keep }, skipped)
        }
        UndoPlan::RenameBack { pairs } => {
            let mut keep = Vec::new();
            let mut skipped = Vec::new();
            for (cur, dst) in pairs {
                if !cur.exists() {
                    skipped.push((cur, "已不在撤销位置".to_string()));
                } else if dst.exists() {
                    skipped.push((cur, "原位置已被占用".to_string()));
                } else {
                    keep.push((cur, dst));
                }
            }
            (UndoPlan::RenameBack { pairs: keep }, skipped)
        }
    }
}

/// 撤销栈（会话内，FileManagerView 持有；上限 UNDO_LIMIT，超出丢弃最旧）。
#[derive(Default)]
pub struct UndoStack {
    ops: Vec<UndoableOp>,
}

impl UndoStack {
    /// 入栈；满上限丢弃最旧一条。
    pub fn push(&mut self, op: UndoableOp) {
        if self.ops.len() >= UNDO_LIMIT {
            self.ops.remove(0);
        }
        self.ops.push(op);
    }

    /// 弹出栈顶（最近一条）。
    pub fn pop(&mut self) -> Option<UndoableOp> {
        self.ops.pop()
    }

    /// 栈顶文案（右键菜单动态项用；栈空 None = 不显示）。
    pub fn peek_label(&self) -> Option<String> {
        self.ops.last().map(UndoableOp::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn stack_push_evicts_oldest_beyond_limit() {
        let mut stack = UndoStack::default();
        for i in 0..UNDO_LIMIT + 3 {
            stack.push(UndoableOp::NewDir {
                path: p(&format!("d{i}")),
            });
        }
        // 最旧 3 条已丢弃：pop 出全部后第一条应是 d3。
        let mut all = Vec::new();
        while let Some(op) = stack.pop() {
            all.push(op);
        }
        assert_eq!(all.len(), UNDO_LIMIT);
        assert_eq!(
            all.last(),
            Some(&UndoableOp::NewDir { path: p("d3") }),
            "最旧保留项应为 d3（d0/d1/d2 已被挤出）"
        );
        assert!(stack.pop().is_none());
    }

    #[test]
    fn plan_undo_copy_and_new_are_trash() {
        let op = UndoableOp::Copy {
            targets: vec![p("a"), p("b")],
        };
        assert_eq!(
            plan_undo(&op),
            UndoPlan::Trash {
                targets: vec![p("a"), p("b")]
            }
        );
        let op = UndoableOp::NewFile { path: p("f") };
        assert_eq!(
            plan_undo(&op),
            UndoPlan::Trash {
                targets: vec![p("f")]
            }
        );
    }

    #[test]
    fn plan_undo_move_and_rename_are_rename_back() {
        let op = UndoableOp::Move {
            pairs: vec![(p("dst/a"), p("src/a"))],
        };
        assert_eq!(
            plan_undo(&op),
            UndoPlan::RenameBack {
                pairs: vec![(p("dst/a"), p("src/a"))]
            }
        );
        let op = UndoableOp::Rename {
            from: p("old"),
            to: p("new"),
        };
        assert_eq!(
            plan_undo(&op),
            UndoPlan::RenameBack {
                pairs: vec![(p("new"), p("old"))]
            }
        );
    }

    #[test]
    fn plan_undo_multi_rename_reverses_order() {
        // 互换类命名（a→b、b→c）：回改必须逆序，否则第一步 b→a 会撞上
        // 尚未撤销的 b。
        let op = UndoableOp::MultiRename {
            pairs: vec![(p("a"), p("b")), (p("b"), p("c"))],
        };
        assert_eq!(
            plan_undo(&op),
            UndoPlan::RenameBack {
                pairs: vec![(p("c"), p("b")), (p("b"), p("a"))]
            }
        );
    }

    #[test]
    fn labels_match_menu_wording() {
        assert_eq!(
            UndoableOp::Copy {
                targets: vec![p("a"), p("b")]
            }
            .label(),
            "复制 2 项"
        );
        assert_eq!(
            UndoableOp::Move {
                pairs: vec![(p("d"), p("s"))]
            }
            .label(),
            "移动 1 项"
        );
        assert_eq!(
            UndoableOp::Rename {
                from: p("a"),
                to: p("b")
            }
            .label(),
            "重命名"
        );
        assert_eq!(UndoableOp::NewDir { path: p("d") }.label(), "新建文件夹");
        assert_eq!(UndoableOp::NewFile { path: p("f") }.label(), "新建文件");
    }

    #[test]
    fn precheck_trash_filters_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("here.txt");
        std::fs::write(&existing, b"x").unwrap();
        let missing = tmp.path().join("gone.txt");
        let (plan, skipped) = precheck(UndoPlan::Trash {
            targets: vec![existing.clone(), missing.clone()],
        });
        assert_eq!(
            plan,
            UndoPlan::Trash {
                targets: vec![existing]
            }
        );
        assert_eq!(skipped, vec![(missing, "目标已不存在".to_string())]);
    }

    #[test]
    fn precheck_rename_back_guards_occupied_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let cur_ok = tmp.path().join("moved.txt");
        std::fs::write(&cur_ok, b"x").unwrap();
        let dst_ok = tmp.path().join("orig.txt");
        // 当前位置缺失项。
        let cur_missing = tmp.path().join("no-longer-here.txt");
        // 改回位置被占用项。
        let cur_occupied = tmp.path().join("occupied-cur.txt");
        std::fs::write(&cur_occupied, b"x").unwrap();
        let dst_occupied = tmp.path().join("occupied-dst.txt");
        std::fs::write(&dst_occupied, b"y").unwrap();
        let (plan, skipped) = precheck(UndoPlan::RenameBack {
            pairs: vec![
                (cur_ok.clone(), dst_ok.clone()),
                (cur_missing.clone(), tmp.path().join("free1")),
                (cur_occupied.clone(), dst_occupied.clone()),
            ],
        });
        assert_eq!(
            plan,
            UndoPlan::RenameBack {
                pairs: vec![(cur_ok, dst_ok)]
            }
        );
        assert_eq!(
            skipped,
            vec![
                (cur_missing, "已不在撤销位置".to_string()),
                (cur_occupied, "原位置已被占用".to_string()),
            ]
        );
    }
}
