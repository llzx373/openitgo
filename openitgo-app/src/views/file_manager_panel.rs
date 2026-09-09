//! 双栏文件管理器的单面板状态：目录列举（`AsyncOpener` 后台线程 + 每帧
//! `poll()` 接收）、Explorer 式选择、排序、导航历史。行渲染与双栏布局在
//! `file_manager.rs`；纯函数行模型在 `file_manager_rows.rs`。
//!
//! 选择模型与 Archive 视图不同：`selected` 直接存 `PathBuf` 且**含目录**
//! （本地 FS 的目录是一等操作对象），选择/焦点用 UI 行索引
//! （0 = 「..」上级行，1..= 对应 `rows()[i-1]`）。

use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::file_manager_rows::{
    is_hidden_name, list_rows, select_by_pattern, type_ahead_match, FsEntry, SortKey,
};
use crate::views::file_ops::dir_size;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// FS watch 事件去抖窗口：距最后一次事件满此时长才触发 refresh
/// （批量外部改动合并为一次重列）。
pub(crate) const WATCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// type-ahead 缓冲的有效窗口：距上次按键超过此时长则下次按键重开缓冲。
pub(crate) const TYPE_AHEAD_TIMEOUT: Duration = Duration::from_millis(800);

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
    show_hidden: bool,
}

/// 当前目录的 FS 变更监听（非递归）：事件经 channel 汇入 poll 去抖后
/// 触发 refresh；watcher drop 即停止监听。
struct PanelWatch {
    /// 监听句柄（仅保活，drop 停止监听）。
    _watcher: notify::RecommendedWatcher,
    /// 已监听的目录（与 dir 相同则跳过重建）。
    path: PathBuf,
    rx: crossbeam_channel::Receiver<()>,
    /// 最近一次事件时间（去抖基准；None = 无待刷新事件）。
    last_event: Option<Instant>,
}

/// 去抖判定：距最后一次事件已满 WATCH_DEBOUNCE 窗口。
fn watch_debounce_ready(last: Option<Instant>, now: Instant) -> bool {
    match last {
        Some(t) => now.duration_since(t) >= WATCH_DEBOUNCE,
        None => false,
    }
}

/// 为目录建立非递归 FS 监听：事件发 channel 信号并经 wake_ctx 唤醒 UI
/// （egui 空闲不重绘）；建立失败（网络盘/权限/路径不存在）静默降级 None。
fn try_watch(path: &Path, wake_ctx: Option<egui::Context>) -> Option<PanelWatch> {
    use notify::{RecursiveMode, Watcher};
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
        let _ = tx.send(());
        if let Some(ctx) = &wake_ctx {
            ctx.request_repaint();
        }
    })
    .ok()?;
    watcher.watch(path, RecursiveMode::NonRecursive).ok()?;
    Some(PanelWatch {
        _watcher: watcher,
        path: path.to_path_buf(),
        rx,
        last_event: None,
    })
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
    /// 是否显示隐藏文件（settings.fm_show_hidden 经 ui() 每帧下发；
    /// 纳入 RowsKey，切换时 rows_cache 自动失效）。
    pub show_hidden: bool,
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
    /// 已算出的目录总大小（右键「计算大小」/空格触发，大小列显示用）；
    /// navigate/refresh 时清空（目录内容可能已变）。
    pub dir_sizes: HashMap<PathBuf, u64>,
    /// 在途目录大小计算的接收端与取消标志（worker 逐个目录累加后经
    /// channel 回报，poll 排空进 dir_sizes）。
    dir_size_rx: Option<Receiver<(PathBuf, u64)>>,
    dir_size_cancel: Option<Arc<AtomicBool>>,
    /// 已请求未回报的目录（去重 + 重启任务时并入新线程清单）。
    dir_size_pending: Vec<PathBuf>,
    /// 当前目录的 FS 变更监听；navigate 时随 start_listing 丢弃，
    /// 列举就绪（Ready）后按当前 dir 惰性建立。
    watch: Option<PanelWatch>,
    /// 监听回调唤醒 UI 用的 egui Context（视图首帧 ui() 注入；无头测试
    /// 为 None，回调只发 channel 信号）。
    wake_ctx: Option<egui::Context>,
    /// type-ahead（type-to-select）缓冲：累计字符 + 最后按键时间；
    /// 超 TYPE_AHEAD_TIMEOUT 未续键即失效（状态栏显示与 Esc 清理由
    /// type_ahead_buffer/clear_type_ahead 统一处理）。
    type_ahead: Option<(String, Instant)>,
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
            show_hidden: true,
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
            dir_sizes: HashMap::new(),
            dir_size_rx: None,
            dir_size_cancel: None,
            dir_size_pending: Vec::new(),
            watch: None,
            wake_ctx: None,
            type_ahead: None,
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
        self.clear_dir_sizes();
        self.watch = None;
        self.type_ahead = None;
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

    /// 是否可后退（历史 UI 按钮的 disabled 态）。
    pub fn can_go_back(&self) -> bool {
        self.history_pos > 1
    }

    /// 是否可前进。
    pub fn can_go_forward(&self) -> bool {
        self.history_pos < self.history.len()
    }

    /// 历史列表（新→旧）：第二元素 = 是否当前目录（历史下拉菜单展示用）。
    pub fn history_list(&self) -> Vec<(PathBuf, bool)> {
        (0..self.history.len())
            .rev()
            .map(|i| (self.history[i].clone(), i + 1 == self.history_pos))
            .collect()
    }

    /// 历史直跳（历史下拉菜单用）：移动 history_pos 到 pos（1 起，对应
    /// history[pos-1]）并切目录；不截断历史、不重复压栈（同 go_back/
    /// go_forward 的目录切换路径）。非法位置或与当前相同为 no-op。
    pub fn navigate_history_to(&mut self, pos: usize) {
        if pos == 0 || pos > self.history.len() || pos == self.history_pos {
            return;
        }
        self.history_pos = pos;
        let path = self.history[pos - 1].clone();
        self.start_listing(path);
    }

    /// 后退的目标目录（tooltip 用）。
    pub fn back_target(&self) -> Option<&Path> {
        self.can_go_back()
            .then(|| self.history[self.history_pos - 2].as_path())
    }

    /// 前进的目标目录（tooltip 用）。
    pub fn forward_target(&self) -> Option<&Path> {
        self.can_go_forward()
            .then(|| self.history[self.history_pos].as_path())
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
        self.clear_dir_sizes();
        let dir = self.dir.clone();
        self.state = PanelLoadState::Loading(AsyncOpener::open(dir, read_dir_entries));
    }

    /// 每帧排空列举结果；返回 true = 仍在 Loading（调用方据此
    /// `request_repaint_after(100ms)`，遵循 egui 空闲不重绘约定）。
    pub fn poll(&mut self) -> bool {
        self.poll_dir_sizes();
        self.poll_watch();
        // mem::take 会把 state 先换成 Default(Idle)，必须先确认 Loading 再
        // take——否则 Ready/Failed 会被静默打回 Idle，触发 app 侧「Idle =
        // 首次进入」恢复逻辑每帧重列目录（列表闪烁 + 选中/焦点被清）。
        if !matches!(self.state, PanelLoadState::Loading(_)) {
            return false;
        }
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
                self.ensure_watch();
                false
            }
            OpenStatus::Ready(Err(e)) => {
                self.state = PanelLoadState::Failed(e);
                false
            }
        }
    }

    /// 视图首帧注入唤醒上下文（FS watch 回调 request_repaint 用）。
    pub fn set_wake_ctx(&mut self, ctx: egui::Context) {
        if self.wake_ctx.is_none() {
            self.wake_ctx = Some(ctx);
        }
    }

    /// 为当前目录建立 FS 监听；已监听同一路径则跳过（避免每帧重建）。
    fn ensure_watch(&mut self) {
        if self
            .watch
            .as_ref()
            .map(|w| w.path == self.dir)
            .unwrap_or(false)
        {
            return;
        }
        self.watch = try_watch(&self.dir, self.wake_ctx.clone());
    }

    /// 排空 FS 监听事件：去抖 300ms 后触发 refresh（保留选中，同 Ctrl+R）。
    /// Loading 期间到达的事件保留到列举完成后再判定。
    fn poll_watch(&mut self) {
        let should_refresh = {
            let Some(watch) = &mut self.watch else {
                return;
            };
            if watch.rx.try_iter().count() > 0 {
                watch.last_event = Some(Instant::now());
            }
            let ready = watch_debounce_ready(watch.last_event, Instant::now())
                && !matches!(self.state, PanelLoadState::Loading(_));
            if ready {
                watch.last_event = None;
            }
            ready
        };
        if should_refresh {
            self.refresh();
        }
    }

    /// 有待去抖的 FS 变更事件（调用方据此 request_repaint_after 推进
    /// 去抖窗口，直到 refresh 触发）。
    pub fn watch_refresh_pending(&self) -> bool {
        self.watch
            .as_ref()
            .map(|w| w.last_event.is_some())
            .unwrap_or(false)
    }

    /// 按需后台计算目录总大小：过滤掉已算出/已请求的目录；已有在途任务
    /// 时取消并以「剩余 pending + 本次新增」重启 worker（旧 receiver 丢弃，
    /// 迟到的结果自然失效）。结果经 channel 回报，poll 排空进 dir_sizes。
    pub fn request_dir_sizes(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if !self.dir_sizes.contains_key(&p) && !self.dir_size_pending.contains(&p) {
                self.dir_size_pending.push(p);
            }
        }
        if self.dir_size_pending.is_empty() {
            return;
        }
        if let Some(cancel) = self.dir_size_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.dir_size_rx = None;
        let paths = self.dir_size_pending.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("fm-dir-size".into())
            .spawn(move || {
                for path in paths {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let size = dir_size(&path, &worker_cancel);
                    if worker_cancel.load(Ordering::Relaxed) || tx.send((path, size)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn fm-dir-size worker");
        self.dir_size_cancel = Some(cancel);
        self.dir_size_rx = Some(rx);
    }

    /// 排空目录大小结果进 dir_sizes；worker 结束（channel 断开）后清任务句柄。
    fn poll_dir_sizes(&mut self) {
        let Some(rx) = self.dir_size_rx.take() else {
            return;
        };
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok((path, size)) => {
                    self.dir_size_pending.retain(|p| p != &path);
                    self.dir_sizes.insert(path, size);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.dir_size_cancel = None;
            self.dir_size_pending.clear();
        } else {
            self.dir_size_rx = Some(rx);
        }
    }

    /// 目录大小计算是否在途（调用方据此 request_repaint_after 排空结果）。
    pub fn dir_sizes_in_flight(&self) -> bool {
        self.dir_size_rx.is_some()
    }

    /// 取消在途目录大小计算并清空已算结果（navigate/refresh：目录内容
    /// 可能已变）。worker 看到 cancel 或 send 失败即退出。
    fn clear_dir_sizes(&mut self) {
        if let Some(cancel) = self.dir_size_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.dir_size_rx = None;
        self.dir_size_pending.clear();
        self.dir_sizes.clear();
    }

    /// 当前行模型（目录优先 + 排序 + 过滤，见 file_manager_rows::list_rows）；
    /// RowsKey 命中时直接复用缓存，避免每帧重算。
    pub fn rows(&mut self) -> Vec<usize> {
        let key = RowsKey {
            entries_version: self.entries_version,
            filter: self.filter.clone(),
            sort_key: self.sort_key,
            sort_asc: self.sort_asc,
            show_hidden: self.show_hidden,
        };
        if let Some((k, rows)) = &self.rows_cache {
            if *k == key {
                return rows.clone();
            }
        }
        let rows = list_rows(
            &self.entries,
            &self.filter,
            self.sort_key,
            self.sort_asc,
            self.show_hidden,
        );
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

    /// 焦点行对应的条目（「..」行/无焦点/越界 → None）。预览跟随用。
    pub fn focused_entry(&mut self) -> Option<FsEntry> {
        let row = self.focus?;
        if row == 0 {
            return None;
        }
        let rows = self.rows();
        rows.get(row - 1).map(|&i| self.entries[i].clone())
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

    /// 反选当前可见行（不含「..」行；可见性受过滤/隐藏开关影响）。
    pub fn invert_selection(&mut self) {
        let row_count = self.rows().len() + 1;
        for r in 1..row_count {
            if let Some(path) = self.row_path(r) {
                if !self.selected.remove(&path) {
                    self.selected.insert(path);
                }
            }
        }
    }

    /// 「选择组」通配模式应用（`;` 分隔多模式，`*`/`?`，不区分大小写）：
    /// select=true 把匹配的可见行加入选中集，false 从选中集移除；
    /// files_only 只作用于文件。只增删选中，不动焦点/锚点。
    pub fn apply_pattern_selection(&mut self, pattern: &str, select: bool, files_only: bool) {
        let rows = self.rows();
        for row in select_by_pattern(&self.entries, &rows, pattern, files_only) {
            // rows 是本帧刚取的行模型，entries 未变，索引安全（get 防御）。
            if let Some(&i) = rows.get(row - 1) {
                let path = self.entries[i].path.clone();
                if select {
                    self.selected.insert(path);
                } else {
                    self.selected.remove(&path);
                }
            }
        }
    }

    /// type-ahead（type-to-select）缓冲追加一个可打印字符：距上次按键超
    /// TYPE_AHEAD_TIMEOUT 先重置缓冲；相同单字符重复输入（"eee"）时匹配串
    /// 保持该单字符，从当前焦点后环形跳下一个匹配（Explorer 语义）。
    /// 返回命中行的 UI 行索引（调用方据此 focus_row 移动焦点并单选）。
    pub fn type_ahead_push(&mut self, c: char) -> Option<usize> {
        let now = Instant::now();
        let prev = match self.type_ahead.take() {
            Some((buf, t)) if now.duration_since(t) <= TYPE_AHEAD_TIMEOUT => buf,
            _ => String::new(),
        };
        let cycle = !prev.is_empty() && prev.chars().all(|b| b == c);
        let mut buf = prev;
        buf.push(c);
        let needle: String = if cycle { c.to_string() } else { buf.clone() };
        let rows = self.rows();
        let hit = type_ahead_match(&self.entries, &rows, &needle, self.focus);
        self.type_ahead = Some((buf, now));
        hit
    }

    /// 当前 type-ahead 缓冲（状态栏显示用）；超窗未续键视为已失效。
    pub fn type_ahead_buffer(&self) -> Option<&str> {
        match &self.type_ahead {
            Some((buf, t)) if t.elapsed() <= TYPE_AHEAD_TIMEOUT => Some(buf.as_str()),
            _ => None,
        }
    }

    /// 清空 type-ahead 缓冲（Esc 第一级语义）。
    pub fn clear_type_ahead(&mut self) {
        self.type_ahead = None;
    }

    /// 焦点直达指定 UI 行（type-ahead 命中用）：语义与 ↑/↓ 相同，
    /// 由 mode 决定选中行为；越界行 no-op。
    pub fn focus_row(&mut self, row: usize, mode: FocusMove) {
        let row_count = self.rows().len() + 1;
        if row < row_count {
            self.apply_focus_move(row_count, row, mode);
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
        let is_hidden = is_hidden_name(&name) || windows_attr_hidden(meta.as_ref());
        let mtime = meta.and_then(|m| m.modified().ok());
        entries.push(FsEntry {
            name,
            path,
            is_dir,
            size,
            mtime,
            is_symlink,
            is_hidden,
        });
    }
    Ok(entries)
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

/// 枚举可用盘符/卷（盘符下拉用；断开的映射盘/光驱可能阻塞数秒，
/// 调用方应放后台线程）：Windows 列 A–Z 中 `read_dir` 成功的 `X:\`
/// （光驱无盘/断开映射盘 `exists` 可能为真但读取失败，以可列出为准；
/// 空盘 `read_dir` 成功，不误伤）；macOS/Linux 返回 `/` 加 `/Volumes/*`
/// 下的可用卷。
pub fn list_drives() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if std::fs::read_dir(&root).is_ok() {
                drives.push(root);
            }
        }
        drives
    }
    #[cfg(not(windows))]
    {
        let mut drives = vec![PathBuf::from("/")];
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for item in rd.flatten() {
                let p = item.path();
                if p.is_dir() {
                    drives.push(p);
                }
            }
        }
        drives
    }
}

/// 目标不存在时逐级回退到最近存在的祖先目录；全灭回用户主目录
/// （书签跳转与启动恢复 fm_dir_* 共用）。
pub fn fallback_existing_dir(path: PathBuf) -> PathBuf {
    let mut path = path;
    loop {
        if path.is_dir() {
            return path;
        }
        if !path.pop() {
            return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：poll() 在非 Loading 状态必须原样返回——mem::take 会先把 state
    /// 换成 Default(Idle)，若不先判 Loading 就 take，Ready/Failed 会被静默
    /// 打回 Idle，触发 app 侧「Idle = 首次进入」恢复逻辑每帧重列目录
    /// （列表周期性塌缩闪烁、选中/焦点被清）。
    #[test]
    fn poll_preserves_non_loading_state() {
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.state = PanelLoadState::Ready;
        assert!(!panel.poll());
        assert!(matches!(panel.state, PanelLoadState::Ready));

        panel.state = PanelLoadState::Failed("boom".to_string());
        assert!(!panel.poll());
        assert!(matches!(panel.state, PanelLoadState::Failed(_)));

        assert!(!panel.poll());
        assert!(matches!(panel.state, PanelLoadState::Failed(_)));
    }

    /// 盘符枚举：Windows 下至少能列出系统盘 C:\（CI/开发机均成立）。
    #[cfg(windows)]
    #[test]
    fn list_drives_includes_system_drive() {
        let drives = list_drives();
        assert!(
            drives.iter().any(|d| d == &PathBuf::from("C:\\")),
            "应列出 C:\\，实际: {drives:?}"
        );
        // 每个列出的盘符都必须真的可读（不以 exists 为准）。
        for d in &drives {
            assert!(std::fs::read_dir(d).is_ok(), "{d:?} 应可列出");
        }
    }

    /// 书签跳转/启动恢复的回退：目标存在原样返回；尾部若干级不存在
    /// 时回到最近存在的祖先目录。
    #[test]
    fn fallback_existing_dir_climbs_to_nearest_existing_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(fallback_existing_dir(deep.clone()), deep);

        let missing = deep.join("gone").join("deeper");
        assert_eq!(fallback_existing_dir(missing), deep);
    }

    /// show_hidden=false 时 rows() 排除隐藏条目（RowsKey 含 show_hidden，
    /// 切换后缓存自动失效）；「..」上级行是 UI 行索引 0，不在行模型内，
    /// 恒不受影响（row_path(0) 恒为 None）。
    #[test]
    fn rows_respect_show_hidden_and_parent_row_untouched() {
        let mk = |name: &str, is_dir: bool, is_hidden: bool| FsEntry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir,
            size: if is_dir { None } else { Some(1) },
            mtime: None,
            is_symlink: false,
            is_hidden,
        };
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.entries = vec![
            mk(".config", true, true),
            mk("docs", true, false),
            mk(".env", false, true),
            mk("notes.txt", false, false),
        ];
        panel.entries_version = 1;
        panel.state = PanelLoadState::Ready;

        let rows = panel.rows();
        assert_eq!(rows.len(), 4);

        panel.show_hidden = false;
        let rows = panel.rows();
        let names: Vec<&str> = rows
            .iter()
            .map(|&i| panel.entries[i].name.as_str())
            .collect();
        assert_eq!(names, ["docs", "notes.txt"]);
        assert_eq!(panel.row_path(0), None);
        assert_eq!(panel.row_path(1), Some(PathBuf::from("docs")));

        panel.show_hidden = true;
        assert_eq!(panel.rows().len(), 4);
    }

    /// 目录大小：request → poll 排空进 dir_sizes；重复请求已算出的目录为
    /// no-op；refresh/navigate 取消在途任务并清空已算结果。
    #[test]
    fn dir_sizes_request_poll_and_clear() {
        let root = std::env::temp_dir().join(format!(
            "openitgo-fm-dirsize-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/f.txt"), b"123456").unwrap();
        std::fs::write(root.join("g.txt"), b"12").unwrap();

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.request_dir_sizes(vec![root.join("sub")]);
        assert!(panel.dir_sizes_in_flight());
        for _ in 0..200 {
            panel.poll();
            if !panel.dir_sizes_in_flight() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(panel.dir_sizes.get(&root.join("sub")), Some(&6));
        assert!(!panel.dir_sizes_in_flight());

        // 已算出的目录重复请求 = no-op（不起新任务）。
        panel.request_dir_sizes(vec![root.join("sub")]);
        assert!(!panel.dir_sizes_in_flight());

        // refresh 取消在途任务并清空结果。
        panel.request_dir_sizes(vec![root.join("sub"), root.clone()]);
        panel.refresh();
        assert!(panel.dir_sizes.is_empty());
        assert!(!panel.dir_sizes_in_flight());

        // navigate 同样取消并清空。
        panel.request_dir_sizes(vec![root.join("sub")]);
        panel.navigate_to(root.clone());
        assert!(panel.dir_sizes.is_empty());
        assert!(!panel.dir_sizes_in_flight());

        std::fs::remove_dir_all(&root).ok();
    }

    /// 导航历史查询：初始不可后退/前进；navigate 两次后可后退；
    /// 后退后可前进；新 navigate 截断前进分支后不可再前进。
    #[test]
    fn can_go_back_forward_tracks_history() {
        let mut panel = FsPanel::new(SortKey::Name, true);
        assert!(!panel.can_go_back());
        assert!(!panel.can_go_forward());
        assert_eq!(panel.back_target(), None);
        assert_eq!(panel.forward_target(), None);

        let dir_a = std::env::temp_dir();
        let dir_b = dir_a.join("openitgo-test-history");
        let dir_c = dir_a.join("openitgo-test-history-c");
        panel.navigate_to(dir_a.clone());
        assert!(!panel.can_go_back());
        assert!(!panel.can_go_forward());

        panel.navigate_to(dir_b.clone());
        assert!(panel.can_go_back());
        assert!(!panel.can_go_forward());
        assert_eq!(panel.back_target(), Some(dir_a.as_path()));

        assert!(panel.go_back());
        assert!(!panel.can_go_back());
        assert!(panel.can_go_forward());
        assert_eq!(panel.forward_target(), Some(dir_b.as_path()));

        panel.navigate_to(dir_c);
        assert!(panel.can_go_back());
        assert!(!panel.can_go_forward());
        assert_eq!(panel.forward_target(), None);
    }

    /// navigate 到真实目录 → poll 至 Ready 后状态稳定，再 poll 不再变化。
    #[test]
    fn poll_settles_ready_after_listing() {
        let dir = std::env::temp_dir();
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(dir);
        for _ in 0..100 {
            if !panel.poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert!(!panel.poll());
        assert!(matches!(panel.state, PanelLoadState::Ready));
    }

    /// 去抖判定：无事件不就绪；距最后事件满 WATCH_DEBOUNCE 窗口才就绪。
    #[test]
    fn watch_debounce_ready_waits_quiet_window() {
        let now = Instant::now();
        assert!(!watch_debounce_ready(None, now));
        assert!(!watch_debounce_ready(Some(now), now));
        assert!(watch_debounce_ready(Some(now - WATCH_DEBOUNCE), now));
        assert!(!watch_debounce_ready(
            Some(now - WATCH_DEBOUNCE + Duration::from_millis(1)),
            now
        ));
    }

    /// watch 建立失败（路径不存在）静默降级为 None。macOS FSEvents 对
    /// 不存在路径也可能建流成功，严格断言只限 Windows/Linux 后端。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn watch_missing_dir_degrades_to_none() {
        let missing = std::env::temp_dir().join(format!(
            "openitgo-fm-watch-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(try_watch(&missing, None).is_none());
    }

    /// 集成：navigate 就绪后建立 watch；外部在目录里新建文件，事件经
    /// channel 去抖后触发 refresh，新条目出现在 entries。navigate 到别的
    /// 目录后旧 watcher 被替换（路径跟踪不串目录）。
    #[test]
    fn watch_event_triggers_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(tmp.path().to_path_buf());
        for _ in 0..200 {
            if !panel.poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert!(panel.watch.is_some());

        std::fs::write(tmp.path().join("watched-new-file.txt"), b"x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            panel.poll();
            if panel
                .entries
                .iter()
                .any(|e| e.name == "watched-new-file.txt")
            {
                break;
            }
            assert!(Instant::now() < deadline, "watch 事件未触发 refresh");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
        // refresh 后 watch 仍在且路径不变（未每帧重建）。
        let watch = panel.watch.as_ref().unwrap();
        assert_eq!(watch.path, tmp.path());

        // navigate 换目录：旧 watcher 丢弃，就绪后新 watcher 跟踪新目录。
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        panel.navigate_to(sub.clone());
        assert!(panel.watch.is_none());
        for _ in 0..200 {
            if !panel.poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert_eq!(panel.watch.as_ref().map(|w| w.path.clone()), Some(sub));
    }

    /// 反选与「选择组」模式选择：作用于当前可见行（不含「..」行），
    /// 只改选中集、不动焦点；files_only 排除目录。
    #[test]
    fn invert_and_pattern_selection() {
        let mk = |name: &str, is_dir: bool| FsEntry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir,
            size: if is_dir { None } else { Some(1) },
            mtime: None,
            is_symlink: false,
            is_hidden: false,
        };
        let mut panel = FsPanel::new(SortKey::Name, true);
        // 名称升序：docs, EP1（目录）, EP2.zip, notes.txt
        panel.entries = vec![
            mk("EP2.zip", false),
            mk("docs", true),
            mk("notes.txt", false),
            mk("EP1", true),
        ];
        panel.entries_version = 1;
        panel.state = PanelLoadState::Ready;

        // 全选后反选 = 空；再反选 = 全选。
        panel.select_all_visible();
        assert_eq!(panel.selected.len(), 4);
        panel.invert_selection();
        assert!(panel.selected.is_empty());
        panel.invert_selection();
        assert_eq!(panel.selected.len(), 4);

        // 模式选择（含目录）：EP1 与 EP2.zip。
        panel.apply_pattern_selection("EP*", true, false);
        assert!(panel.selected.contains(&PathBuf::from("EP1")));
        assert!(panel.selected.contains(&PathBuf::from("EP2.zip")));
        // 模式取消选择（仅文件）：移除 EP2.zip，EP1 目录保留。
        panel.apply_pattern_selection("EP*", false, true);
        assert!(panel.selected.contains(&PathBuf::from("EP1")));
        assert!(!panel.selected.contains(&PathBuf::from("EP2.zip")));
        // 空 pattern 不动选中集。
        panel.apply_pattern_selection("", true, false);
        assert_eq!(panel.selected.len(), 3);
        assert!(panel.focus.is_none());
    }

    /// type-ahead：连续按键累计匹配；相同单字符重复输入环形跳下一个；
    /// 命中后 focus_row(Select) 单选并置最小滚动揭示；Esc 清缓冲。
    #[test]
    fn type_ahead_cycles_and_selects() {
        let mk = |name: &str| FsEntry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir: false,
            size: Some(1),
            mtime: None,
            is_symlink: false,
            is_hidden: false,
        };
        let mut panel = FsPanel::new(SortKey::Name, true);
        // 名称升序：abc, ep1, ep2, notes（UI 行 1..=4）
        panel.entries = vec![mk("ep2"), mk("notes"), mk("abc"), mk("ep1")];
        panel.entries_version = 1;
        panel.state = PanelLoadState::Ready;

        // "e" → 行 2（ep1）；再 "e"（同字符重复）→ 行 3（ep2）；再 "e" 回卷行 2。
        assert_eq!(panel.type_ahead_push('e'), Some(2));
        panel.focus_row(2, FocusMove::Select);
        assert_eq!(panel.type_ahead_push('e'), Some(3));
        panel.focus_row(3, FocusMove::Select);
        assert_eq!(panel.type_ahead_push('e'), Some(2));
        assert_eq!(panel.type_ahead_buffer(), Some("eee"));
        // 命中后焦点/选中/揭示标记（Select 语义同 ↑↓）。
        panel.focus_row(2, FocusMove::Select);
        assert_eq!(panel.focus, Some(2));
        assert!(panel.selected.contains(&PathBuf::from("ep1")));
        assert!(panel.focus_scroll_pending);
        // 缓冲超时后下次按键重开（直接改写时间戳模拟）。
        if let Some((_, t)) = &mut panel.type_ahead {
            *t = Instant::now() - TYPE_AHEAD_TIMEOUT - Duration::from_millis(1);
        }
        assert_eq!(panel.type_ahead_buffer(), None);
        assert_eq!(panel.type_ahead_push('n'), Some(4));
        assert_eq!(panel.type_ahead_buffer(), Some("n"));
        // Esc 清缓冲。
        panel.clear_type_ahead();
        assert_eq!(panel.type_ahead_buffer(), None);
    }

    /// 历史下拉：history_list 新→旧且标记当前项；navigate_history_to
    /// 直跳不截断历史、不压栈，可继续后退/前进。
    #[test]
    fn history_list_and_direct_jump() {
        let mut panel = FsPanel::new(SortKey::Name, true);
        let dir_a = std::env::temp_dir().join("openitgo-test-histlist-a");
        let dir_b = std::env::temp_dir().join("openitgo-test-histlist-b");
        let dir_c = std::env::temp_dir().join("openitgo-test-histlist-c");
        panel.navigate_to(dir_a.clone());
        panel.navigate_to(dir_b.clone());
        panel.navigate_to(dir_c.clone());

        let list = panel.history_list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], (dir_c.clone(), true));
        assert_eq!(list[1], (dir_b.clone(), false));
        assert_eq!(list[2], (dir_a.clone(), false));

        // 直跳到最旧（pos 1 = dir_a）：历史不截断，仍可前进回 dir_c。
        panel.navigate_history_to(1);
        assert!(panel.can_go_forward());
        assert_eq!(panel.forward_target(), Some(dir_b.as_path()));
        assert_eq!(panel.history_list()[2], (dir_a.clone(), true));

        // 跳到 pos 3（dir_c）：可后退。
        panel.navigate_history_to(3);
        assert!(panel.can_go_back());
        assert!(!panel.can_go_forward());

        // 非法位置 / 当前位置 = no-op。
        panel.navigate_history_to(0);
        panel.navigate_history_to(99);
        panel.navigate_history_to(3);
        assert!(panel.can_go_back());
        assert!(!panel.can_go_forward());
    }
}
