//! 目录树面板（阶段 AH，Alt+F10）：双栏下整栏替换非活动栏（与 Ctrl+Q
//! 快览互斥），树驱动活动栏导航（TC 手感）。
//!
//! 数据懒加载：根 = 盘符列表（复用 `list_drives`，一次性后台线程）；
//! 节点展开时经单 worker 线程列举直接子目录一层（natural_cmp 排序，
//! 隐藏目录按 fm_show_hidden 过滤），结果缓存 `children`。行模型
//! （`visible_rows`/`move_cursor`/`parent_in_rows`）为纯函数可单测。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::views::file_manager_panel::list_drives;
use crate::views::file_manager_rows::{is_hidden_name, natural_cmp};

/// 树行（渲染用快照）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: PathBuf,
    /// 缩进深度（根 = 0）。
    pub depth: usize,
    /// 是否显示展开三角：已加载且无子目录 = false；未加载 = true
    /// （乐观显示，展开后空目录三角消失）。
    pub expandable: bool,
    pub expanded: bool,
    /// 子列表加载中（行尾显示「…」）。
    pub loading: bool,
}

/// 可见行序列（纯函数）：根列表顺序输出，展开节点递归插入子行；
/// 展开但未加载完成的节点不出子行（loading 标志在行上）。
pub fn visible_rows(
    roots: &[PathBuf],
    expanded: &HashSet<PathBuf>,
    children: &HashMap<PathBuf, Vec<PathBuf>>,
    loading: &HashSet<PathBuf>,
) -> Vec<TreeRow> {
    fn walk(
        path: &Path,
        depth: usize,
        expanded: &HashSet<PathBuf>,
        children: &HashMap<PathBuf, Vec<PathBuf>>,
        loading: &HashSet<PathBuf>,
        out: &mut Vec<TreeRow>,
    ) {
        let is_expanded = expanded.contains(path);
        out.push(TreeRow {
            path: path.to_path_buf(),
            depth,
            expandable: children.get(path).map(|c| !c.is_empty()).unwrap_or(true),
            expanded: is_expanded,
            loading: loading.contains(path),
        });
        if is_expanded {
            if let Some(kids) = children.get(path) {
                for kid in kids {
                    walk(kid, depth + 1, expanded, children, loading, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for root in roots {
        walk(root, 0, expanded, children, loading, &mut out);
    }
    out
}

/// 光标上下移动（纯函数）：clamp 在可见行范围内；空序列/无光标时
/// 返回首行（delta ≥ 0）或 None。
pub fn move_cursor(rows: &[TreeRow], cursor: Option<&Path>, delta: i64) -> Option<PathBuf> {
    if rows.is_empty() {
        return None;
    }
    let cur = cursor.and_then(|c| rows.iter().position(|r| r.path == c));
    let next = match cur {
        Some(i) => (i as i64 + delta).clamp(0, rows.len() as i64 - 1) as usize,
        None => 0,
    };
    Some(rows[next].path.clone())
}

/// 行序列中的父节点（纯函数）：光标行之前最近的 depth 小 1 的行。
/// 不查 fs——折叠链中间节点缺失时仍指向可见意义上的父。
pub fn parent_in_rows(rows: &[TreeRow], path: &Path) -> Option<PathBuf> {
    let idx = rows.iter().position(|r| r.path == path)?;
    let depth = rows[idx].depth;
    if depth == 0 {
        return None;
    }
    rows[..idx]
        .iter()
        .rev()
        .find(|r| r.depth == depth - 1)
        .map(|r| r.path.clone())
}

/// 列举直接子目录（纯函数+fs；worker 与测试共用）：natural_cmp 排序；
/// show_hidden=false 过滤 `.` 开头与 Windows 隐藏属性目录；单项失败跳过；
/// 符号链接目录跟随（树展开是手动逐层的，环不构成无限递归）。
pub fn list_child_dirs(dir: &Path, show_hidden: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let meta = item.metadata().ok();
        let is_dir = meta
            .as_ref()
            .map(|m| m.is_dir())
            .unwrap_or_else(|| item.file_type().map(|t| t.is_dir()).unwrap_or(false));
        if !is_dir {
            continue;
        }
        if !show_hidden && (is_hidden_name(&name) || windows_attr_hidden(meta.as_ref())) {
            continue;
        }
        out.push(item.path());
    }
    out.sort_by(|a, b| {
        natural_cmp(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    out
}

#[cfg(windows)]
fn windows_attr_hidden(meta: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    meta.map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn windows_attr_hidden(_: Option<&std::fs::Metadata>) -> bool {
    false
}

/// worker 请求/回复携带的过滤代际：show_hidden 变化时 +1，过期回复丢弃。
type TreeRequest = (PathBuf, bool, u64);
type TreeResponse = (PathBuf, Vec<PathBuf>, u64);

/// 目录树状态（FileManagerView 持有；仅双栏 Alt+F10 开启时使用）。
pub struct DirTree {
    /// 根列表（盘符/卷）；空 + roots_loading = 根加载中。
    pub roots: Vec<PathBuf>,
    /// 展开集合。
    pub expanded: HashSet<PathBuf>,
    /// 树内光标（选中节点）。
    pub cursor: Option<PathBuf>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
    loading: HashSet<PathBuf>,
    req_tx: Sender<TreeRequest>,
    res_rx: Receiver<TreeResponse>,
    roots_rx: Option<Receiver<Vec<PathBuf>>>,
    roots_loading: bool,
    show_hidden: bool,
    gen: u64,
    /// open() 传入的锚点：根列表到达后自动展开根 → 锚点的祖先链。
    pending_anchor: Option<PathBuf>,
}

impl Default for DirTree {
    fn default() -> Self {
        let (req_tx, req_rx) = channel::<TreeRequest>();
        let (res_tx, res_rx) = channel::<TreeResponse>();
        // 单 worker 线程顺序处理列举请求（树展开是低频手动操作，无需
        // 并发；慢盘符阻塞只拖住后续树请求，不影响 UI）。req_tx 全部
        // drop（DirTree drop）后 recv 出错退出。
        std::thread::Builder::new()
            .name("fm-tree".to_string())
            .spawn(move || {
                while let Ok((dir, show_hidden, gen)) = req_rx.recv() {
                    let kids = list_child_dirs(&dir, show_hidden);
                    // 发送失败 = UI 侧已销毁，退出。
                    if res_tx.send((dir, kids, gen)).is_err() {
                        break;
                    }
                }
            })
            .expect("fm-tree worker 创建失败");
        Self {
            roots: Vec::new(),
            expanded: HashSet::new(),
            cursor: None,
            children: HashMap::new(),
            loading: HashSet::new(),
            req_tx,
            res_rx,
            roots_rx: None,
            roots_loading: false,
            show_hidden: true,
            gen: 0,
            pending_anchor: None,
        }
    }
}

impl DirTree {
    /// fm_show_hidden 每帧下发：变化时清子目录缓存/在途并 bump 代际，
    /// 已展开节点下次 poll 重新请求（expand 集合保留）。
    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        if self.show_hidden != show_hidden {
            self.show_hidden = show_hidden;
            self.gen += 1;
            self.children.clear();
            self.loading.clear();
        }
    }

    /// 打开树面板：根列表未加载则起一次性后台枚举（断开的映射盘可能
    /// 阻塞数秒，必须离 UI 线程）；锚点记入 pending_anchor，根到达后
    /// 自动展开锚点祖先链并置光标。
    pub fn open(&mut self, anchor: &Path) {
        self.pending_anchor = Some(anchor.to_path_buf());
        self.cursor = Some(anchor.to_path_buf());
        if self.roots.is_empty() && !self.roots_loading {
            self.roots_loading = true;
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(list_drives());
            });
            self.roots_rx = Some(rx);
        }
    }

    /// 关闭树面板：光标/锚点清理（展开集合与缓存保留，下次打开原样）。
    pub fn close(&mut self) {
        self.cursor = None;
        self.pending_anchor = None;
    }

    /// 请求子目录列表（去重：已缓存/在途不重复发）。
    pub fn request_children(&mut self, dir: &Path) {
        if self.children.contains_key(dir) || self.loading.contains(dir) {
            return;
        }
        self.loading.insert(dir.to_path_buf());
        // 发送失败 = worker 已死（不会发生；worker 随 DirTree 存活）。
        let _ = self
            .req_tx
            .send((dir.to_path_buf(), self.show_hidden, self.gen));
    }

    /// 展开/折叠：展开时顺带请求子目录。
    pub fn toggle(&mut self, path: &Path) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_path_buf());
            self.request_children(path);
        }
    }

    /// 可见行快照（渲染与键盘共用同一序列，保证光标移动与显示一致）。
    pub fn rows(&self) -> Vec<TreeRow> {
        visible_rows(&self.roots, &self.expanded, &self.children, &self.loading)
    }

    /// →：展开光标节点（已展开/无三角则不动）。
    pub fn cursor_expand(&mut self) {
        let Some(cur) = self.cursor.clone() else {
            return;
        };
        if !self.expanded.contains(&cur) {
            self.toggle(&cur);
        }
    }

    /// ←：已展开 = 折叠；否则光标回父级（可见行意义上的父）。
    pub fn cursor_collapse_or_parent(&mut self) {
        let Some(cur) = self.cursor.clone() else {
            return;
        };
        if self.expanded.contains(&cur) {
            self.expanded.remove(&cur);
        } else if let Some(parent) = parent_in_rows(&self.rows(), &cur) {
            self.cursor = Some(parent);
        }
    }

    /// 每帧排空：根列表到达后展开锚点祖先链；子目录回复按代际过滤
    /// 入库。返回 true = 仍有在途加载（UI 侧据此 request_repaint_after）。
    pub fn poll(&mut self) -> bool {
        if let Some(rx) = &self.roots_rx {
            if let Ok(roots) = rx.try_recv() {
                self.roots = roots;
                self.roots_rx = None;
                self.roots_loading = false;
                self.expand_to_anchor();
            }
        }
        while let Ok((dir, kids, gen)) = self.res_rx.try_recv() {
            if gen != self.gen {
                continue;
            }
            self.loading.remove(&dir);
            self.children.insert(dir, kids);
        }
        // show_hidden 变化清了缓存：已展开节点需要重新请求。
        let stale: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter(|d| !self.children.contains_key(*d) && !self.loading.contains(*d))
            .cloned()
            .collect();
        for dir in stale {
            self.request_children(&dir);
        }
        self.roots_loading || !self.loading.is_empty()
    }

    /// 根列表到达后：找到锚点所在根（最长前缀匹配），从根到锚点逐级
    /// 展开 + 请求子目录。无匹配根（UNC 等）只留光标。
    fn expand_to_anchor(&mut self) {
        let Some(anchor) = self.pending_anchor.take() else {
            return;
        };
        let Some(root) = self
            .roots
            .iter()
            .filter(|r| anchor.starts_with(r))
            .max_by_key(|r| r.as_os_str().len())
            .cloned()
        else {
            return;
        };
        // 根本身展开，再逐级展开锚点的祖先（不含锚点本身）。
        self.expanded.insert(root.clone());
        self.request_children(&root);
        let mut cur = anchor.as_path();
        let mut chain = Vec::new();
        while let Some(parent) = cur.parent() {
            if parent == root {
                break;
            }
            chain.push(parent.to_path_buf());
            cur = parent;
        }
        for dir in chain.into_iter().rev() {
            self.expanded.insert(dir.clone());
            self.request_children(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn row_paths(rows: &[TreeRow]) -> Vec<PathBuf> {
        rows.iter().map(|r| r.path.clone()).collect()
    }

    #[test]
    fn visible_rows_expands_only_loaded_children() {
        let roots = vec![p("C:\\")];
        let mut expanded = HashSet::new();
        let mut children = HashMap::new();
        let loading = HashSet::new();
        // 未展开：只有根行。
        assert_eq!(
            visible_rows(&roots, &expanded, &children, &loading).len(),
            1
        );
        // 展开但未加载：仍只有根行，三角乐观显示。
        expanded.insert(p("C:\\"));
        let rows = visible_rows(&roots, &expanded, &children, &loading);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].expandable);
        // 加载完成：子行按 children 顺序插入，深度 +1。
        children.insert(p("C:\\"), vec![p("C:\\a"), p("C:\\b")]);
        let rows = visible_rows(&roots, &expanded, &children, &loading);
        assert_eq!(row_paths(&rows), vec![p("C:\\"), p("C:\\a"), p("C:\\b")]);
        assert_eq!(rows[1].depth, 1);
        // 二级展开。
        expanded.insert(p("C:\\a"));
        children.insert(p("C:\\a"), vec![p("C:\\a\\x")]);
        let rows = visible_rows(&roots, &expanded, &children, &loading);
        assert_eq!(
            row_paths(&rows),
            vec![p("C:\\"), p("C:\\a"), p("C:\\a\\x"), p("C:\\b")]
        );
        assert_eq!(rows[2].depth, 2);
        // 空目录（已加载无子目录）：不显示三角。
        children.insert(p("C:\\b"), Vec::new());
        let rows = visible_rows(&roots, &expanded, &children, &loading);
        let b = rows.iter().find(|r| r.path == p("C:\\b")).unwrap();
        assert!(!b.expandable);
    }

    #[test]
    fn visible_rows_marks_loading() {
        let roots = vec![p("R")];
        let mut expanded = HashSet::new();
        expanded.insert(p("R"));
        let mut loading = HashSet::new();
        loading.insert(p("R"));
        let rows = visible_rows(&roots, &expanded, &HashMap::new(), &loading);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].loading);
        assert!(rows[0].expanded);
    }

    #[test]
    fn move_cursor_clamps_and_defaults() {
        let rows = vec![
            TreeRow {
                path: p("a"),
                depth: 0,
                expandable: true,
                expanded: true,
                loading: false,
            },
            TreeRow {
                path: p("b"),
                depth: 1,
                expandable: true,
                expanded: false,
                loading: false,
            },
            TreeRow {
                path: p("c"),
                depth: 0,
                expandable: true,
                expanded: false,
                loading: false,
            },
        ];
        // 无光标：落到首行。
        assert_eq!(move_cursor(&rows, None, 1), Some(p("a")));
        assert_eq!(move_cursor(&rows, Some(&p("b")), 1), Some(p("c")));
        assert_eq!(move_cursor(&rows, Some(&p("b")), -1), Some(p("a")));
        // clamp：首行再上移不动，末行再下移不动。
        assert_eq!(move_cursor(&rows, Some(&p("a")), -5), Some(p("a")));
        assert_eq!(move_cursor(&rows, Some(&p("c")), 5), Some(p("c")));
        // 光标不在可见集（被折叠）：按无光标处理。
        assert_eq!(move_cursor(&rows, Some(&p("zz")), 1), Some(p("a")));
        assert_eq!(move_cursor(&[], None, 1), None);
    }

    #[test]
    fn parent_in_rows_walks_visible_sequence() {
        let roots = vec![p("R")];
        let mut expanded = HashSet::new();
        expanded.insert(p("R"));
        expanded.insert(p("R\\a"));
        let mut children = HashMap::new();
        children.insert(p("R"), vec![p("R\\a"), p("R\\b")]);
        children.insert(p("R\\a"), vec![p("R\\a\\x")]);
        let rows = visible_rows(&roots, &expanded, &children, &HashSet::new());
        assert_eq!(parent_in_rows(&rows, &p("R\\a\\x")), Some(p("R\\a")));
        assert_eq!(parent_in_rows(&rows, &p("R\\b")), Some(p("R")));
        assert_eq!(parent_in_rows(&rows, &p("R")), None);
        assert_eq!(parent_in_rows(&rows, &p("zz")), None);
    }

    #[test]
    fn list_child_dirs_dirs_only_sorted_and_hidden_filtered() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("d10")).unwrap();
        std::fs::create_dir(tmp.path().join("d2")).unwrap();
        std::fs::create_dir(tmp.path().join(".hidden")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();
        let all = list_child_dirs(tmp.path(), true);
        assert_eq!(
            all,
            vec![
                tmp.path().join(".hidden"),
                tmp.path().join("d2"),
                tmp.path().join("d10")
            ],
            "仅目录、natural 序、含隐藏"
        );
        let visible = list_child_dirs(tmp.path(), false);
        assert_eq!(
            visible,
            vec![tmp.path().join("d2"), tmp.path().join("d10")],
            "show_hidden=false 过滤 . 前缀目录"
        );
        // 不存在的目录 = 空列表（不报错）。
        assert!(list_child_dirs(&tmp.path().join("nope"), true).is_empty());
    }

    #[test]
    fn toggle_requests_children_once() {
        let mut tree = DirTree::default();
        let dir = p("some-dir");
        tree.toggle(&dir);
        assert!(tree.expanded.contains(&dir));
        assert!(tree.loading.contains(&dir));
        // 在途不重复请求（loading 去重）。
        tree.request_children(&dir);
        tree.toggle(&dir);
        assert!(!tree.expanded.contains(&dir));
    }
}
