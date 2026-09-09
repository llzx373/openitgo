//! 批量重命名（Ctrl+M，TC Multi-Rename 简化版）的规则模型与计划生成。
//! 纯函数，不依赖 egui，可单测；对话框在 `file_manager_dialog.rs`
//! （`MultiRenameDialog`），执行走 `file_ops::rename_entry` 同目录瞬时改名。
//! 查找替换为纯文本语义（openitgo-app 未直接依赖 regex，不提供正则选项）。

use crate::views::file_ops::validate_entry_name;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 计数器规则：模板 `[C]` token 的值（按 pad 补零）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterRule {
    pub start: i32,
    pub step: i32,
    /// 补零位数（0 = 不补零）。
    pub pad: usize,
}

/// 批量重命名规则：先对原名（不含扩展名）做查找替换，再套模板生成目标名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameRule {
    /// 查找串（空 = 不替换）。
    pub search: String,
    pub replace: String,
    pub counter: Option<CounterRule>,
    /// 输出模板（默认 `[O][E]`）：`[O]` = 原名（不含扩展名，查找替换后）、
    /// `[E]` = 扩展名（含点，无扩展名/目录为空）、`[C]` = 计数器；
    /// 其余字符（含未知 token）原样输出。
    pub template: String,
}

impl Default for RenameRule {
    fn default() -> Self {
        Self {
            search: String::new(),
            replace: String::new(),
            counter: None,
            template: "[O][E]".to_string(),
        }
    }
}

/// 单个重命名计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    pub src: PathBuf,
    pub dst_name: String,
    /// 非法结果（空名/非法字符/`.`/`..`/重名冲突）；Some 时不可执行。
    pub error: Option<String>,
    /// 目标与源同名（无变化）：不执行也不计错误，预览弱色显示。
    pub skip: bool,
}

/// 拆分文件名：（不含扩展名的主名, 含点扩展名）。目录与无扩展名文件
/// 整体进主名、扩展名为空；`.env` 这类仅起始点的名字同样无扩展名。
fn split_stem_ext(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(pos) if pos > 0 && pos + 1 < name.len() => (&name[..pos], &name[pos..]),
        _ => (name, ""),
    }
}

/// 模板展开：`[O]`/`[E]`/`[C]` 替换，未知或不完整 token 按字面输出。
fn render_template(template: &str, stem: &str, ext: &str, counter: Option<String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(pos) = rest.find('[') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(end) = tail.find(']') else {
            out.push_str(tail);
            return out;
        };
        match &tail[..=end] {
            "[O]" => out.push_str(stem),
            "[E]" => out.push_str(ext),
            "[C]" => out.push_str(counter.as_deref().unwrap_or_default()),
            // 未知 token 原样保留。
            other => out.push_str(other),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// 生成批量重命名计划。items = (路径, 是否目录)（目录无扩展名概念，
/// 整个名字进 `[O]`）；exists 为磁盘存在性探测（注入便于单测不碰磁盘，
/// 传入的 dst 与同目录已有项冲突时报错，与 src 自身同名不算冲突）。
/// 错误优先级：无变化（skip）> 非法名 > 计划内部重名 > 磁盘重名。
pub fn plan_renames(
    items: &[(PathBuf, bool)],
    rule: &RenameRule,
    exists: impl Fn(&Path) -> bool,
) -> Vec<RenamePlan> {
    let mut plans = Vec::with_capacity(items.len());
    // 已分配的目标名（同目录，小写比较对齐 Windows 大小写不敏感语义）。
    let mut used: HashSet<(PathBuf, String)> = HashSet::new();
    for (i, (src, is_dir)) in items.iter().enumerate() {
        let file_name = src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let parent = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let (stem, ext) = split_stem_ext(&file_name, *is_dir);
        // 第一步：查找替换（空 search = 不替换）。
        let stem = if rule.search.is_empty() {
            stem.to_string()
        } else {
            stem.replace(&rule.search, &rule.replace)
        };
        // 第二步：套模板。
        let counter = rule.counter.as_ref().map(|c| {
            let value = c.start + c.step * i as i32;
            format!("{:0>1$}", value, c.pad)
        });
        let dst_name = render_template(&rule.template, &stem, ext, counter);
        // 无变化：与源同名 → skip（不算错误）。
        if dst_name == file_name {
            plans.push(RenamePlan {
                src: src.clone(),
                dst_name,
                error: None,
                skip: true,
            });
            continue;
        }
        // 非法名（空/非法字符/`.`/`..`）。
        if let Err(e) = validate_entry_name(&dst_name) {
            plans.push(RenamePlan {
                src: src.clone(),
                dst_name,
                error: Some(e),
                skip: false,
            });
            continue;
        }
        let key = (parent.clone(), dst_name.to_lowercase());
        // 计划内部互相冲突。
        if !used.insert(key) {
            plans.push(RenamePlan {
                src: src.clone(),
                dst_name,
                error: Some("批量结果内部重名".to_string()),
                skip: false,
            });
            continue;
        }
        // 磁盘已存在同名项（dst == src 的无变化已在上面排除）。
        if exists(&parent.join(&dst_name)) {
            plans.push(RenamePlan {
                src: src.clone(),
                dst_name,
                error: Some("已存在同名文件或文件夹".to_string()),
                skip: false,
            });
            continue;
        }
        plans.push(RenamePlan {
            src: src.clone(),
            dst_name,
            error: None,
            skip: false,
        });
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_disk(_: &Path) -> bool {
        false
    }

    fn items(names: &[&str]) -> Vec<(PathBuf, bool)> {
        names
            .iter()
            .map(|n| (PathBuf::from("/dir").join(n), false))
            .collect()
    }

    fn dst_names(plans: &[RenamePlan]) -> Vec<&str> {
        plans.iter().map(|p| p.dst_name.as_str()).collect()
    }

    #[test]
    fn search_replace_applies_to_stem_only() {
        let rule = RenameRule {
            search: "EP".to_string(),
            replace: "Vol".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["EP1.zip", "EP2.zip", "note.txt"]), &rule, no_disk);
        assert_eq!(dst_names(&plans), ["Vol1.zip", "Vol2.zip", "note.txt"]);
        assert!(!plans[0].skip && !plans[1].skip);
        // 未命中 search → 无变化 → skip。
        assert!(plans[2].skip);
        assert!(plans.iter().all(|p| p.error.is_none()));
        // 空 search = 不替换（全部无变化 → skip）。
        let rule = RenameRule::default();
        let plans = plan_renames(&items(&["a.txt"]), &rule, no_disk);
        assert!(plans[0].skip);
        assert_eq!(plans[0].dst_name, "a.txt");
    }

    #[test]
    fn template_tokens_and_literals() {
        let rule = RenameRule {
            template: "漫画-[O]-完[E]".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["EP1.zip", "noext"]), &rule, no_disk);
        assert_eq!(dst_names(&plans), ["漫画-EP1-完.zip", "漫画-noext-完"]);
        // 未知/不完整 token 按字面输出。
        let rule = RenameRule {
            template: "[X][O".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.txt"]), &rule, no_disk);
        assert_eq!(plans[0].dst_name, "[X][O");
    }

    #[test]
    fn counter_pad_step_and_negative() {
        let rule = RenameRule {
            template: "[O]-[C][E]".to_string(),
            counter: Some(CounterRule {
                start: 10,
                step: -3,
                pad: 3,
            }),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.zip", "b.zip", "c.zip"]), &rule, no_disk);
        assert_eq!(dst_names(&plans), ["a-010.zip", "b-007.zip", "c-004.zip"]);
        // pad=0 不补零。
        let rule = RenameRule {
            template: "[C][E]".to_string(),
            counter: Some(CounterRule {
                start: 1,
                step: 1,
                pad: 0,
            }),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.zip", "b.zip"]), &rule, no_disk);
        assert_eq!(dst_names(&plans), ["1.zip", "2.zip"]);
    }

    #[test]
    fn dir_uses_whole_name_as_stem() {
        let mut it = items(&["photos.2024"]);
        it[0].1 = true; // 目录：整个名字进 [O]，[E] 为空
        let rule = RenameRule {
            template: "备份-[O][E]".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&it, &rule, no_disk);
        assert_eq!(plans[0].dst_name, "备份-photos.2024");
    }

    #[test]
    fn conflicts_detected_internal_disk_and_invalid() {
        // 计划内部重名：两个不同源映射到同一目标。
        let rule = RenameRule {
            search: "a".to_string(),
            replace: "x".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a1.txt", "x1.txt"]), &rule, no_disk);
        // a1 → x1.txt；x1 无 'a' 可替换 → 同名 skip（不占目标名）。
        assert_eq!(plans[0].dst_name, "x1.txt");
        assert!(plans[0].error.is_none());
        assert!(plans[1].skip);
        let rule = RenameRule {
            template: "same.txt".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.zip", "b.rar"]), &rule, no_disk);
        assert!(plans[0].error.is_none());
        assert_eq!(plans[1].error.as_deref(), Some("批量结果内部重名"));

        // 磁盘重名（注入 exists）；dst == src 的无变化不算冲突。
        let rule = RenameRule::default();
        let plans = plan_renames(&items(&["taken.txt"]), &rule, |p| {
            p == Path::new("/dir/taken.txt")
        });
        assert!(plans[0].skip);
        let rule = RenameRule {
            template: "taken.txt".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["other.txt"]), &rule, |p| {
            p == Path::new("/dir/taken.txt")
        });
        assert_eq!(plans[0].error.as_deref(), Some("已存在同名文件或文件夹"));

        // 非法名：结果为空 / 含非法字符。
        let rule = RenameRule {
            template: "".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.txt"]), &rule, no_disk);
        assert!(plans[0].error.is_some());
        let rule = RenameRule {
            template: "a/b.txt".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.txt"]), &rule, no_disk);
        assert!(plans[0].error.is_some());
    }

    #[test]
    fn no_change_marked_skip_not_error() {
        let rule = RenameRule {
            search: "zzz".to_string(),
            replace: "yyy".to_string(),
            ..Default::default()
        };
        let plans = plan_renames(&items(&["a.txt", "zzz.txt"]), &rule, no_disk);
        assert!(plans[0].skip);
        assert!(plans[0].error.is_none());
        assert!(!plans[1].skip);
        assert_eq!(plans[1].dst_name, "yyy.txt");
    }
}
