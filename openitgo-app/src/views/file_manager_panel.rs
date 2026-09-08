//! 双栏文件管理器的单面板状态：目录列举（`AsyncOpener` 后台线程 + 每帧
//! `poll()` 接收）、Explorer 式选择、排序、导航历史。行渲染与双栏布局在
//! `file_manager.rs`；纯函数行模型在 `file_manager_rows.rs`。
//!
//! 选择模型与 Archive 视图不同：`selected` 直接存 `PathBuf` 且**含目录**
//! （本地 FS 的目录是一等操作对象），选择/焦点用 UI 行索引
//! （0 = 「..」上级行，1..= 对应 `rows()[i-1]`）。

use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::file_manager_rows::{list_rows, FsEntry, SortKey};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 明细列表行高（pt），虚拟化滚动要求固定行高（同 archive.rs 约定）。
pub(crate) const ROW_HEIGHT: f32 = 22.0;
/// 明细列表「大小」「修改时间」列的默认宽度（pt），右对齐。
pub(crate) const SIZE_COL_WIDTH: f32 = 90.0;
pub(crate) const MTIME_COL_WIDTH: f32 = 110.0;
/// 列宽拖拽的取值范围（pt）。
pub(crate) const COL_MIN_WIDTH: f32 = 60.0;
pub(crate) const COL_MAX_WIDTH: f32 = 400.0;
/// 名称列最小宽度（pt）：分隔线左拖 / 列块左移的下限。
pub(crate) const NAME_COL_MIN: f32 = 80.0;
/// 列内容右缘内边距（pt）：列锚点在右缘内 6pt 处，表头与行内容共用。
pub(crate) const COL_RIGHT_PAD: f32 = 6.0;

/// 面板目录列举状态机：Idle（未初始化，等恢复目录）→ Loading → Ready/Failed。
/// （`AsyncOpener` 的通道本身携带 `Result<T, String>`，无需再套一层。）
#[derive(Default)]
pub enum PanelLoadState {
    #[default]
    Idle,
    Loading(AsyncOpener<Vec<FsEntry>>),
    Ready,
    Failed(String),
}

/// 键盘焦点移动模式（move_focus 共用核心，同 archive.rs 的 FocusMove）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMove {
    /// 单选新焦点行（↑/↓、Home/End、PgUp/PgDn 默认；anchor 跟随）。
    Select,
    /// Shift：替换为 anchor 到新焦点行的行序区间选中，anchor 不动。
    Extend,
    /// Ctrl：只移焦点，选中与 anchor 均不动。
    FocusOnly,
}

/// rows_cache 的命中键：条目版本 + 影响行模型的全部视图状态。
#[derive(Debug, Clone, PartialEq)]
struct RowsKey {
    entries_version: u64,
    filter: String,
    sort_key: SortKey,
    sort_asc: bool,
}

pub struct FsPanel {
    /// 当前目录。
    pub dir: PathBuf,
    /// 当前目录的条目（read_dir 收集，未排序；显示顺序见 rows()）。
    pub entries: Vec<FsEntry>,
    pub state: PanelLoadState,
    /// 选中集（含目录——与 Archive 的派生模型不同）。
    pub selected: HashSet<PathBuf>,
    /// 键盘焦点行（UI 行索引，0 = 「..」上级行）。
    pub focus: Option<usize>,
    /// Shift 范围选锚点（UI 行索引）。
    anchor: Option<usize>,
    pub sort_key: SortKey,
    pub sort_asc: bool,
    pub filter: String,
    /// 导航历史（访问顺序）；history_pos = 当前位置（当前目录 =
    /// history[history_pos-1]），前进分支在 navigate_to 时截断。
    history: Vec<PathBuf>,
    history_pos: usize,
    /// 行模型缓存：RowsKey 命中直接复用，避免每帧重算 list_rows。
    rows_cache: Option<(RowsKey, Vec<usize>)>,
    /// 条目版本号：entries 变更时 +1，rows_cache 的失效依据。
    entries_version: u64,
    /// 大小/时间列宽（pt，分隔竖线拖拽可调，clamp 60..=400）；会话内有效。
    pub col_width_size: f32,
    pub col_width_mtime: f32,
    /// 右侧两列的整体平移（pt，≤0）：拖分隔线时其右侧列保持宽度随鼠标
    /// 平移（Explorer 手感）；0 = 列块贴右缘。
    pub col_shift: f32,
    /// 键盘移动焦点后置位，下一帧按 Explorer 最小滚动语义揭示焦点行。
    pub focus_scroll_pending: bool,
    /// 上一帧列表的滚动偏移与视口高度（最小滚动计算的基准；0 = 未知）。
    pub last_scroll_offset: f32,
    pub last_viewport_height: f32,
    /// 实际行距（渲染时每帧更新）：PgUp/PgDn 步进与焦点滚动定位用。
    pub last_row_pitch: f32,
}

impl FsPanel {
    pub fn new(sort_key: SortKey, sort_asc: bool) -> Self {
        Self {
            dir: PathBuf::new(),
            entries: Vec::new(),
            state: PanelLoadState::Idle,
            selected: HashSet::new(),
            focus: None,
            anchor: None,
            sort_key,
            sort_asc,
            filter: String::new(),
            history: Vec::new(),
            history_pos: 0,
            rows_cache: None,
            entries_version: 0,
            col_width_size: SIZE_COL_WIDTH,
            col_width_mtime: MTIME_COL_WIDTH,
            col_shift: 0.0,
            focus_scroll_pending: false,
            last_scroll_offset: 0.0,
            last_viewport_height: 0.0,
            last_row_pitch: 0.0,
        }
    }

    /// 后台列举目录（清选择/焦点/缓存）；调用方负责压历史。
    fn start_listing(&mut self, path: PathBuf) {
        self.dir = path.clone();
        self.entries.clear();
        self.entries_version += 1;
        self.rows_cache = None;
        self.selected.clear();
        self.focus = None;
        self.anchor = None;
        self.focus_scroll_pending = false;
        self.last_scroll_offset = 0.0;
        self.last_viewport_height = 0.0;
        self.state = PanelLoadState::Loading(AsyncOpener::open(path, read_dir_entries));
    }

    /// 导航到目录（双击/Enter/面包屑共用）：截断前进分支后压历史。
    /// 目标与当前目录相同且已就绪/加载中时 no-op（避免点当前面包屑段重列）。
    pub fn navigate_to(&mut self, path: PathBuf) {
        if path == self.dir
            && matches!(
                self.state,
                PanelLoadState::Ready | PanelLoadState::Loading(_)
            )
        {
            return;
        }
        self.history.truncate(self.history_pos);
        self.history.push(path.clone());
        self.history_pos = self.history.len();
        self.start_listing(path);
    }

    /// Alt+←：回退到历史中的上一个目录。
    pub fn go_back(&mut self) -> bool {
        if self.history_pos > 1 {
            self.history_pos -= 1;
            let path = self.history[self.history_pos - 1].clone();
            self.start_listing(path);
            true
        } else {
            false
        }
    }

    /// Alt+→：前进到历史中的下一个目录。
    pub fn go_forward(&mut self) -> bool {
        if self.history_pos < self.history.len() {
            let path = self.history[self.history_pos].clone();
            self.history_pos += 1;
            self.start_listing(path);
            true
        } else {
            false
        }
    }

    /// Backspace/「..」行：进入上级目录；已在根目录（如 `C:\`）时 no-op。
    pub fn parent_dir(&mut self) -> bool {
        match self.dir.parent() {
            Some(parent) => {
                self.navigate_to(parent.to_path_buf());
                true
            }
            None => false,
        }
    }

    /// 是否已在根目录（「..」行禁用判定）。
    pub fn is_root(&self) -> bool {
        self.dir.parent().is_none()
    }

    /// Ctrl+R 重列当前目录：不动历史、不清选中；poll 成功时按新条目集
    /// 过滤掉已不存在项的选中态（焦点索引失效，一并清空）。
    pub fn refresh(&mut self) {
        if matches!(self.state, PanelLoadState::Loading(_)) {
            return;
        }
        let dir = self.dir.clone();
        self.state = PanelLoadState::Loading(AsyncOpener::open(dir, read_dir_entries));
    }

    /// 每帧排空列举结果；返回 true = 仍在 Loading（调用方据此
    /// `request_repaint_after(100ms)`，遵循 egui 空闲不重绘约定）。
    pub fn poll(&mut self) -> bool {
        let PanelLoadState::Loading(mut opener) = std::mem::take(&mut self.state) else {
            return false;
        };
        match opener.poll() {
            OpenStatus::Loading => {
                self.state = PanelLoadState::Loading(opener);
                true
            }
            OpenStatus::Ready(Ok(entries)) => {
                self.entries = entries;
                self.entries_version += 1;
                // refresh 路径：丢弃已不存在项的选中态；navigate 路径
                // selected 已清空，retain 为 no-op。
                let existing: HashSet<&Path> =
                    self.entries.iter().map(|e| e.path.as_path()).collect();
                self.selected.retain(|p| existing.contains(p.as_path()));
                self.focus = None;
                self.anchor = None;
                self.state = PanelLoadState::Ready;
                false
            }
            OpenStatus::Ready(Err(e)) => {
                self.state = PanelLoadState::Failed(e);
                false
            }
        }
    }

    /// 当前行模型（目录优先 + 排序 + 过滤，见 file_manager_rows::list_rows）；
    /// RowsKey 命中时直接复用缓存，避免每帧重算。
    pub fn rows(&mut self) -> Vec<usize> {
        let key = RowsKey {
            entries_version: self.entries_version,
            filter: self.filter.clone(),
            sort_key: self.sort_key,
            sort_asc: self.sort_asc,
        };
        if let Some((k, rows)) = &self.rows_cache {
            if *k == key {
                return rows.clone();
            }
        }
        let rows = list_rows(&self.entries, &self.filter, self.sort_key, self.sort_asc);
        self.rows_cache = Some((key, rows.clone()));
        rows
    }

    /// UI 行索引 → 条目路径；0（「..」行）与越界返回 None。
    pub fn row_path(&mut self, row: usize) -> Option<PathBuf> {
        if row == 0 {
            return None;
        }
        let rows = self.rows();
        rows.get(row - 1).map(|&i| self.entries[i].path.clone())
    }

    fn set_row_selected(&mut self, row: usize, on: bool) {
        if let Some(path) = self.row_path(row) {
            if on {
                self.selected.insert(path);
            } else {
                self.selected.remove(&path);
            }
        }
    }

    /// Explorer 式点击选择：无修饰 = 单选；Ctrl = 切换；Shift = 以 anchor
    /// 到目标的行序区间替换式选中（Ctrl+Shift 追加；无锚点退化为普通点击）。
    /// 任何点击都更新 anchor 与 focus；「..」行例外：只设焦点，不动选中/anchor。
    pub fn click_row(&mut self, row: usize, ctrl: bool, shift: bool) {
        if row == 0 {
            self.focus = Some(0);
            return;
        }
        if shift {
            if let Some(anchor) = self.anchor {
                let row_count = self.rows().len() + 1;
                if anchor < row_count {
                    if !ctrl {
                        self.selected.clear();
                    }
                    let (lo, hi) = if anchor <= row {
                        (anchor, row)
                    } else {
                        (row, anchor)
                    };
                    for r in lo..=hi {
                        self.set_row_selected(r, true);
                    }
                    self.focus = Some(row);
                    return;
                }
            }
            // 无锚点或锚点已不可见：退化为普通点击。
        }
        if ctrl {
            if let Some(path) = self.row_path(row) {
                if !self.selected.insert(path.clone()) {
                    self.selected.remove(&path);
                }
            }
        } else {
            self.selected.clear();
            self.set_row_selected(row, true);
        }
        self.anchor = Some(row);
        self.focus = Some(row);
    }

    /// 全选当前可见行（不含「..」行）。
    pub fn select_all_visible(&mut self) {
        let row_count = self.rows().len() + 1;
        for r in 1..row_count {
            self.set_row_selected(r, true);
        }
    }

    /// 清空选中与锚点/焦点（Esc 时用）。
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.focus = None;
    }

    /// 焦点移动的应用核心（↑/↓、Home/End、PgUp/PgDn 共用）：按模式改选中，
    /// 焦点落到 next，置最小滚动揭示标记。
    fn apply_focus_move(&mut self, row_count: usize, next: usize, mode: FocusMove) {
        match mode {
            FocusMove::Select => {
                self.selected.clear();
                self.set_row_selected(next, true);
                self.anchor = Some(next);
            }
            FocusMove::Extend => {
                // 与 Shift+click 同语义：替换为 anchor..focus 区间，anchor 不动。
                match self.anchor.filter(|&a| a < row_count) {
                    Some(a) => {
                        self.selected.clear();
                        let (lo, hi) = if a <= next { (a, next) } else { (next, a) };
                        for r in lo..=hi {
                            self.set_row_selected(r, true);
                        }
                    }
                    None => {
                        // 无锚点或锚点已不可见：退化为单选。
                        self.selected.clear();
                        self.set_row_selected(next, true);
                        self.anchor = Some(next);
                    }
                }
            }
            FocusMove::FocusOnly => {}
        }
        self.focus = Some(next);
        self.focus_scroll_pending = true;
    }

    /// ↑/↓/PgUp/PgDn 移动焦点：无焦点时选中首行/末行（按 delta 方向）；
    /// 有焦点按行序步进，行为由 mode 决定（见 FocusMove）。
    pub fn move_focus(&mut self, delta: isize, mode: FocusMove) {
        let row_count = self.rows().len() + 1;
        let cur = self.focus.filter(|&f| f < row_count);
        let next = match cur {
            None if delta >= 0 => 0,
            None => row_count - 1,
            Some(i) => (i as isize + delta).clamp(0, row_count as isize - 1) as usize,
        };
        self.apply_focus_move(row_count, next, mode);
    }

    /// Home/End：焦点跳首行/末行（语义同 ↑/↓ 单选）。
    pub fn move_focus_edge(&mut self, last: bool, mode: FocusMove) {
        let row_count = self.rows().len() + 1;
        let next = if last { row_count - 1 } else { 0 };
        self.apply_focus_move(row_count, next, mode);
    }

    /// PgUp/PgDn 的整页步进：视口高 / 行距取整；行距未知（0）时按行高兜底，
    /// 视口高度未知时按 10 行兜底。
    pub fn page_step(&self) -> isize {
        if self.last_viewport_height > 0.0 {
            let pitch = if self.last_row_pitch > 0.0 {
                self.last_row_pitch
            } else {
                ROW_HEIGHT
            };
            (self.last_viewport_height / pitch).floor().max(1.0) as isize
        } else {
            10
        }
    }

    /// 列头点击排序：同键切换升/降，换键回到升序。
    pub fn toggle_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
    }

    /// 分隔线拖动（Explorer 语义，同 archive.rs drag_column_sep）：分隔线跟随
    /// 鼠标，其右侧各列保持宽度整体平移，左侧列吸收等量宽度变化（sep0 的
    /// 左侧是弹性的名称列 → 只动 col_shift）。撞限同步停住。
    pub(crate) fn drag_column_sep(&mut self, sep: usize, dx: f32, header_rect: egui::Rect) {
        let min_shift =
            (header_rect.left() + NAME_COL_MIN + self.col_width_size + self.col_width_mtime
                - (header_rect.right() - COL_RIGHT_PAD))
                .min(0.0);
        let shift_room = (self.col_shift + dx).clamp(min_shift, 0.0) - self.col_shift;
        let d = match sep {
            // 名称列是弹性宽度，没有独立字段，列块平移即名称列缩放。
            0 => shift_room,
            _ => {
                let width_room = (self.col_width_size + dx).clamp(COL_MIN_WIDTH, COL_MAX_WIDTH)
                    - self.col_width_size;
                let d = if dx > 0.0 {
                    width_room.min(shift_room)
                } else {
                    width_room.max(shift_room)
                };
                self.col_width_size += d;
                d
            }
        };
        self.col_shift += d;
    }
}

/// 后台线程收集目录条目：单项读取失败（权限等）跳过；metadata 跟随符号
/// 链接（size/mtime/is_dir 取链接目标），失败降级为空值但保留条目。
fn read_dir_entries(path: &Path) -> Result<Vec<FsEntry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| format!("无法读取目录: {e}"))?;
    let mut entries = Vec::new();
    for item in rd {
        let Ok(item) = item else {
            continue;
        };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let is_symlink = item.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let meta = item.metadata().ok();
        let is_dir = meta
            .as_ref()
            .map(|m| m.is_dir())
            .unwrap_or_else(|| item.file_type().map(|t| t.is_dir()).unwrap_or(false));
        let size = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
        let mtime = meta.and_then(|m| m.modified().ok());
        entries.push(FsEntry {
            name,
            path,
            is_dir,
            size,
            mtime,
            is_symlink,
        });
    }
    Ok(entries)
}
