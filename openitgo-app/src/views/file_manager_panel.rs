//! 双栏文件管理器的单面板状态：目录列举（`AsyncOpener` 后台线程 + 每帧
//! `poll()` 接收）、Explorer 式选择、排序、导航历史、分支视图（Ctrl+B
//! 递归扁平列举）。行渲染与双栏布局在 `file_manager.rs`；纯函数行模型在
//! `file_manager_rows.rs`。
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
    Loading(AsyncOpener<DirListing>),
    Ready,
    Failed(String),
}

/// 目录列举结果：分支视图（Ctrl+B）递归收集超 `BRANCH_MAX_ENTRIES`
/// 截断时 truncated=true（UI 状态栏提示「结果过多已截断」）。
#[derive(Clone)]
pub struct DirListing {
    pub entries: Vec<FsEntry>,
    pub truncated: bool,
}

/// 分支视图递归收集上限：超过即截断（防巨型目录树拖垮列举）。
pub(crate) const BRANCH_MAX_ENTRIES: usize = 200_000;

/// 面板视图模式：明细列表 / 简表（多列名称行）/ 缩略图网格（每栏独立；
/// 简表与网格只是渲染层，焦点/选中仍是线性 UI 行索引）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelViewMode {
    #[default]
    List,
    /// 简表（Brief）：多列排布的紧凑名称行（图标 + 截断名称），行主序。
    Brief,
    Thumbs,
}

impl PanelViewMode {
    /// settings 字符串解析（非法值回 List，同 clamp 语义）。
    pub fn from_setting(s: &str) -> Self {
        match s {
            "brief" => Self::Brief,
            "thumbs" => Self::Thumbs,
            _ => Self::List,
        }
    }

    /// settings 字符串表示。
    pub fn as_setting(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Brief => "brief",
            Self::Thumbs => "thumbs",
        }
    }

    /// 顶栏三态切换的显示名。
    pub fn label(&self) -> &'static str {
        match self {
            Self::List => "列表",
            Self::Brief => "简表",
            Self::Thumbs => "缩略图",
        }
    }

    /// 简表/缩略图同为网格状布局：键盘 ↑↓ 按列数步进、←→ 步进 1
    /// （列数由渲染侧每帧写入 last_grid_cols）。
    pub fn is_grid_like(&self) -> bool {
        !matches!(self, Self::List)
    }
}

/// 明细列表的可配置列（阶段 V）：`FsPanel::columns` 的元素类型。
/// 不变式：columns[0] 恒为 Name 且弹性宽度（其宽度值忽略），其余为
/// 固定宽右对齐列，从行右缘往左依次排列（整体平移量 col_shift）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Name,
    Ext,
    Size,
    Mtime,
    Attr,
    /// 注释列（占位：无 descript.ion 类数据来源，内容恒空）。
    Comment,
}

impl ColumnKind {
    /// fm_columns 持久化字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Ext => "ext",
            Self::Size => "size",
            Self::Mtime => "mtime",
            Self::Attr => "attr",
            Self::Comment => "comment",
        }
    }

    /// fm_columns 解析（未知串 → None，由调用方丢弃）。
    pub fn from_setting(s: &str) -> Option<Self> {
        match s {
            "name" => Some(Self::Name),
            "ext" => Some(Self::Ext),
            "size" => Some(Self::Size),
            "mtime" => Some(Self::Mtime),
            "attr" => Some(Self::Attr),
            "comment" => Some(Self::Comment),
            _ => None,
        }
    }

    /// 列头标题。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Name => "名称",
            Self::Ext => "扩展名",
            Self::Size => "大小",
            Self::Mtime => "修改时间",
            Self::Attr => "属性",
            Self::Comment => "注释",
        }
    }

    /// 该列对应的排序键（Comment 无数据来源，不可排序）。
    pub fn sort_key(&self) -> Option<SortKey> {
        match self {
            Self::Name => Some(SortKey::Name),
            Self::Ext => Some(SortKey::Ext),
            Self::Size => Some(SortKey::Size),
            Self::Mtime => Some(SortKey::Mtime),
            Self::Attr => Some(SortKey::Attr),
            Self::Comment => None,
        }
    }

    /// 新增列的默认宽度（pt；Name 弹性宽度不用此值）。
    pub fn default_width(&self) -> f32 {
        match self {
            Self::Name => 0.0,
            Self::Ext => 70.0,
            Self::Size => SIZE_COL_WIDTH,
            Self::Mtime => MTIME_COL_WIDTH,
            Self::Attr => 60.0,
            Self::Comment => 120.0,
        }
    }
}

/// 默认列配置（Name + 大小 + 修改时间，与阶段 V 前的硬编码三列一致）。
pub fn default_columns() -> Vec<(ColumnKind, f32)> {
    vec![
        (ColumnKind::Name, 0.0),
        (ColumnKind::Size, SIZE_COL_WIDTH),
        (ColumnKind::Mtime, MTIME_COL_WIDTH),
    ]
}

/// fm_columns 持久化格式：每项 "kind" 或 "kind:width"（Name 恒首位、
/// 不带宽度）。
pub fn serialize_columns(columns: &[(ColumnKind, f32)]) -> Vec<String> {
    columns
        .iter()
        .map(|(kind, w)| {
            if *kind == ColumnKind::Name {
                kind.as_str().to_string()
            } else {
                format!("{}:{}", kind.as_str(), *w as u32)
            }
        })
        .collect()
}

/// fm_columns 解析 + 规范化：未知 kind 丢弃、宽度 clamp 到列宽范围、
/// 去重、Name 强制补到首位、固定列不足 1 个时补默认大小/时间列。
/// 空输入（旧 settings 无此字段）返回 None——由调用方用 legacy
/// fm_col_size_width/mtime 播种默认列。
pub fn parse_columns(items: &[String]) -> Option<Vec<(ColumnKind, f32)>> {
    if items.is_empty() {
        return None;
    }
    let mut columns: Vec<(ColumnKind, f32)> = Vec::new();
    for item in items {
        let (kind_s, width_s) = item.split_once(':').unwrap_or((item.as_str(), ""));
        let Some(kind) = ColumnKind::from_setting(kind_s) else {
            continue;
        };
        if kind == ColumnKind::Name || columns.iter().any(|(k, _)| *k == kind) {
            continue;
        }
        let width = width_s
            .parse::<f32>()
            .unwrap_or_else(|_| kind.default_width())
            .clamp(COL_MIN_WIDTH, COL_MAX_WIDTH);
        columns.push((kind, width));
    }
    if columns.is_empty() {
        columns.push((ColumnKind::Size, SIZE_COL_WIDTH));
        columns.push((ColumnKind::Mtime, MTIME_COL_WIDTH));
    }
    columns.insert(0, (ColumnKind::Name, 0.0));
    Some(columns)
}

/// 启动恢复的列配置：fm_columns 解析（parse_columns 规范化）；空（旧
/// settings 无此字段）→ 用 legacy fm_col_size_width/mtime 播种默认三列。
pub fn restore_columns(
    items: &[String],
    legacy_size_width: f32,
    legacy_mtime_width: f32,
) -> Vec<(ColumnKind, f32)> {
    parse_columns(items).unwrap_or_else(|| {
        vec![
            (ColumnKind::Name, 0.0),
            (
                ColumnKind::Size,
                legacy_size_width.clamp(COL_MIN_WIDTH, COL_MAX_WIDTH),
            ),
            (
                ColumnKind::Mtime,
                legacy_mtime_width.clamp(COL_MIN_WIDTH, COL_MAX_WIDTH),
            ),
        ]
    })
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
    dirs_first: bool,
    /// 分支视图标志（分支/普通列举的 name 语义不同，缓存必须按此失效）。
    branch: bool,
}

/// 当前目录的 FS 变更监听：事件经 channel 汇入 poll 去抖后
/// 触发 refresh；watcher drop 即停止监听。
struct PanelWatch {
    /// 监听句柄（仅保活，drop 停止监听）。
    _watcher: notify::RecommendedWatcher,
    /// 已监听的目录（与 dir 相同且 recursive 一致则跳过重建）。
    path: PathBuf,
    /// 建立时的递归模式（fm_watch_recursive；选项变化时重建）。
    recursive: bool,
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

/// 为目录建立 FS 监听：事件发 channel 信号并经 wake_ctx 唤醒 UI
/// （egui 空闲不重绘）；建立失败（网络盘/权限/路径不存在）静默降级 None。
/// `recursive` = true 时 RecursiveMode::Recursive（fm_watch_recursive，
/// 阶段 Z：分支视图下子目录变化也刷新；大目录有性能取舍，默认非递归）。
fn try_watch(path: &Path, wake_ctx: Option<egui::Context>, recursive: bool) -> Option<PanelWatch> {
    use notify::{RecursiveMode, Watcher};
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
        let _ = tx.send(());
        if let Some(ctx) = &wake_ctx {
            ctx.request_repaint();
        }
    })
    .ok()?;
    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(path, mode).ok()?;
    Some(PanelWatch {
        _watcher: watcher,
        path: path.to_path_buf(),
        recursive,
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
    /// 视图模式（明细列表/简表/缩略图网格；每栏独立）。阶段 V 起纳入
    /// 标签快照（切标签恢复该标签的模式）。
    pub view_mode: PanelViewMode,
    /// 网格状模式（简表/缩略图）当前列数（render_brief/render_grid 每帧
    /// 更新；键盘 ↑↓/PgUp/PgDn 线性步长换算用，列表模式恒 1）。
    pub last_grid_cols: usize,
    /// 是否显示隐藏文件（settings.fm_show_hidden 经 ui() 每帧下发；
    /// 纳入 RowsKey，切换时 rows_cache 自动失效）。
    pub show_hidden: bool,
    /// 递归 FS watch（settings.fm_watch_recursive 经 ui() 每帧下发，阶段 Z）：
    /// true 时分支视图下子目录变化也触发刷新；ensure_watch 按
    /// (路径, recursive) 判定重建。
    pub watch_recursive: bool,
    /// 目录恒排在文件前（settings.fm_dirs_first 经 ui() 每帧下发；
    /// false = 目录文件混排统一排序；纳入 RowsKey 同 show_hidden）。
    pub dirs_first: bool,
    /// 分支视图（Ctrl+B）：当前目录 + 所有子目录的文件扁平列举（目录行
    /// 不列出）。navigate_to/refresh/「..」行/Esc 退出。已知取舍：FS watch
    /// 只监听顶层目录（非递归），分支模式下子目录变化不自动刷新。
    pub branch_view: bool,
    /// 分支列举超 BRANCH_MAX_ENTRIES 被截断（状态栏提示用；普通列举恒 false）。
    pub listing_truncated: bool,
    /// 导航历史（访问顺序）；history_pos = 当前位置（当前目录 =
    /// history[history_pos-1]），前进分支在 navigate_to 时截断。
    history: Vec<PathBuf>,
    history_pos: usize,
    /// 行模型缓存：RowsKey 命中直接复用，避免每帧重算 list_rows。
    rows_cache: Option<(RowsKey, Vec<usize>)>,
    /// 条目版本号：entries 变更时 +1，rows_cache 的失效依据。
    entries_version: u64,
    /// 明细列表列配置（阶段 V）：首列恒为 Name（弹性宽度，宽度值忽略），
    /// 其余为固定宽右对齐列（从行右缘往左排，clamp 60..=400）；会话内有效，
    /// 经 FmStateSnapshot 持久化为 settings.fm_columns。不变式：首列 Name
    /// 且固定列 ≥1（toggle_column 与 parse_columns 维持）。
    pub columns: Vec<(ColumnKind, f32)>,
    /// 右侧固定列块的整体平移（pt，≤0）：拖分隔线时其右侧列保持宽度随
    /// 鼠标平移（Explorer 手感）；0 = 列块贴右缘。
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
    /// 栏内标签页（不变式：非空，tabs[active_tab] 概念上 = 当前面板实时
    /// 状态——快照只在切走/关闭时回写，活动标签的目录以 panel.dir 为准）。
    tabs: Vec<PanelTabSnapshot>,
    active_tab: usize,
    /// restore_tab 后的待恢复选中/焦点名（列举异步，poll 就绪时应用）。
    pending_tab_restore: Option<PendingTabRestore>,
    /// restore_tab 后的待恢复滚动偏移（render_list 首帧消费；精确恢复
    /// 偏移而非焦点最小滚动揭示）。
    pending_scroll_restore: Option<f32>,
    /// 当前目录的 descript.ion 注释缓存（阶段 W；key = 文件名）：
    /// 列举就绪（Ready）时读取，navigate/refresh 自然重读；FS watch
    /// 覆盖 descript.ion 变更（改动触发 refresh → 重列 → 重读）。
    pub comments: HashMap<String, String>,
    /// reveal_path 跨目录/退出注入模式后的待选中项（poll 就绪时应用）。
    pending_reveal: Option<PathBuf>,
}

/// 栏内标签页的可恢复快照（快照式标签：标签里不塞活面板，切换 =
/// 快照当前状态 → 恢复目标快照 → 重新列举，避免 watcher/channel 悬挂）。
/// focus 存文件名而非行索引——列举后按名定位（目录内容可能已变）。
/// 排序/列/视图模式（阶段 V）仅会话内随标签切换恢复；标签持久化仍
/// 只存目录（restore_tabs 时这些字段继承面板当前值）。
/// locked/custom_title（阶段 AI）随快照往返：活动标签的锁定/标题存于
/// tabs[active_tab]，snapshot_tab 回写时原样带出。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelTabSnapshot {
    pub dir: PathBuf,
    pub selected: Vec<PathBuf>,
    pub focus_name: Option<String>,
    pub filter: String,
    pub scroll_offset: f32,
    pub sort_key: SortKey,
    pub sort_asc: bool,
    pub view_mode: PanelViewMode,
    /// 列配置；空 = restore_tab 不覆盖（兼容 Default 快照的「不动」语义）。
    pub columns: Vec<(ColumnKind, f32)>,
    /// 锁定（阶段 AI，TC 语义）：该标签活动时 navigate_to/历史前进后退/
    /// 「..」上级自动改为新开标签到目标目录（本标签保持原目录不动）。
    pub locked: bool,
    /// 自定义标题（阶段 AI；显示优先级 = custom_title > 目录 basename）。
    pub custom_title: Option<String>,
}

/// restore_tab 后待应用的选中/焦点（选中按路径恢复，焦点按文件名定位）。
#[derive(Debug)]
struct PendingTabRestore {
    selected: HashSet<PathBuf>,
    focus_name: Option<String>,
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
            view_mode: PanelViewMode::List,
            last_grid_cols: 1,
            show_hidden: true,
            watch_recursive: false,
            dirs_first: true,
            branch_view: false,
            listing_truncated: false,
            history: Vec::new(),
            history_pos: 0,
            rows_cache: None,
            entries_version: 0,
            columns: default_columns(),
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
            tabs: vec![PanelTabSnapshot::default()],
            active_tab: 0,
            pending_tab_restore: None,
            pending_scroll_restore: None,
            pending_reveal: None,
            comments: HashMap::new(),
        }
    }

    /// 后台列举目录（清选择/焦点/缓存）；调用方负责压历史。
    /// 任何导航都退出分支视图回普通列举。
    fn start_listing(&mut self, path: PathBuf) {
        self.branch_view = false;
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
        // 清掉上一目录遗留的标签恢复负载（restore_tab 在 start_listing
        // 之后重新设置；用户在其就绪前又导航时不串目录）。
        self.pending_tab_restore = None;
        self.pending_scroll_restore = None;
        self.pending_reveal = None;
        self.state = PanelLoadState::Loading(AsyncOpener::open(path, read_dir_entries));
    }

    /// 导航到目录（双击/Enter/面包屑共用）：截断前进分支后压历史。
    /// 目标与当前目录相同且已就绪/加载中时 no-op（避免点当前面包屑段重列）。
    /// 活动标签锁定（阶段 AI）时改为新开标签到目标（锁定标签保持原目录）。
    pub fn navigate_to(&mut self, path: PathBuf) {
        if path == self.dir
            && matches!(
                self.state,
                PanelLoadState::Ready | PanelLoadState::Loading(_)
            )
        {
            return;
        }
        if self.active_tab_locked() {
            self.new_tab_to(path);
            return;
        }
        self.history.truncate(self.history_pos);
        self.history.push(path.clone());
        self.history_pos = self.history.len();
        self.start_listing(path);
    }

    /// 活动标签是否锁定。
    pub fn active_tab_locked(&self) -> bool {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.locked)
            .unwrap_or(false)
    }

    /// 锁定标签的导航落点（阶段 AI）：目标目录开新标签——新标签不继承
    /// 锁定/自定义标题/选中/过滤（同 new_tab 的「干净副本」语义；标签
    /// 切换不进导航历史，见 restore_tab 注释）。
    pub fn new_tab_to(&mut self, path: PathBuf) {
        let current = self.snapshot_tab();
        let mut fresh = current.clone();
        fresh.dir = path;
        fresh.selected = Vec::new();
        fresh.focus_name = None;
        fresh.filter = String::new();
        fresh.scroll_offset = 0.0;
        fresh.locked = false;
        fresh.custom_title = None;
        self.tabs[self.active_tab] = current;
        self.tabs.push(fresh);
        self.active_tab = self.tabs.len() - 1;
        let snap = self.tabs[self.active_tab].clone();
        self.restore_tab(&snap);
    }

    /// Alt+←：回退到历史中的上一个目录。活动标签锁定时同样开新标签
    /// 到历史目标（阶段 AI 从简：历史在锁定标签内不移动）。
    pub fn go_back(&mut self) -> bool {
        if self.history_pos > 1 {
            if self.active_tab_locked() {
                let path = self.history[self.history_pos - 2].clone();
                self.new_tab_to(path);
                return true;
            }
            self.history_pos -= 1;
            let path = self.history[self.history_pos - 1].clone();
            self.start_listing(path);
            true
        } else {
            false
        }
    }

    /// Alt+→：前进到历史中的下一个目录。锁定同 go_back。
    pub fn go_forward(&mut self) -> bool {
        if self.history_pos < self.history.len() {
            if self.active_tab_locked() {
                let path = self.history[self.history_pos].clone();
                self.new_tab_to(path);
                return true;
            }
            self.history_pos += 1;
            let path = self.history[self.history_pos - 1].clone();
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
    /// 活动标签锁定时同样开新标签到目标（阶段 AI，同 go_back）。
    pub fn navigate_history_to(&mut self, pos: usize) {
        if pos == 0 || pos > self.history.len() || pos == self.history_pos {
            return;
        }
        if self.active_tab_locked() {
            let path = self.history[pos - 1].clone();
            self.new_tab_to(path);
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
    /// 同时退出分支视图回普通列举（含 FS watch 触发的自动 refresh）。
    pub fn refresh(&mut self) {
        if matches!(self.state, PanelLoadState::Loading(_)) {
            return;
        }
        self.branch_view = false;
        self.clear_dir_sizes();
        let dir = self.dir.clone();
        self.state = PanelLoadState::Loading(AsyncOpener::open(dir, read_dir_entries));
    }

    /// Ctrl+B 分支视图开关：进入 = 当前目录 + 所有子目录文件扁平列举；
    /// 退出 = 回普通列举（保留选中，嵌套文件的选中态在 poll 时按存在性
    /// 过滤，同 refresh）。
    pub fn toggle_branch_view(&mut self) {
        if self.branch_view {
            self.exit_branch_view();
        } else {
            self.enter_branch_view();
        }
    }

    /// 进入分支视图：递归列举当前目录（清空选择/焦点/缓存，重置语义同
    /// start_listing，但不动导航历史）。
    pub fn enter_branch_view(&mut self) {
        if matches!(self.state, PanelLoadState::Loading(_)) {
            return;
        }
        self.branch_view = true;
        let path = self.dir.clone();
        self.entries.clear();
        self.entries_version += 1;
        self.rows_cache = None;
        self.selected.clear();
        self.focus = None;
        self.anchor = None;
        self.focus_scroll_pending = false;
        self.clear_dir_sizes();
        self.type_ahead = None;
        self.state = PanelLoadState::Loading(AsyncOpener::open(path, read_dir_entries_recursive));
    }

    /// 退出分支视图（「..」行 / Esc / Ctrl+B）：回普通列举（同 refresh）。
    pub fn exit_branch_view(&mut self) {
        self.branch_view = false;
        self.refresh();
    }

    /// 搜索结果「输送到焦点栏」：命中集作为 entries 直接注入（分支视图
    /// 同款展示——name 已含 `rel_dir/name` 显示名）。重置语义同
    /// enter_branch_view（清选择/焦点/缓存，不动导航历史），但跳过
    /// Loading 直接就绪；退出条件与分支视图一致（navigate/refresh 回
    /// 普通列举）。watcher 维持当前目录监听，FS 事件触发 refresh 即退出。
    pub fn inject_entries_branch(&mut self, entries: Vec<FsEntry>) {
        self.branch_view = true;
        self.entries = entries;
        self.listing_truncated = false;
        self.entries_version += 1;
        self.rows_cache = None;
        self.selected.clear();
        self.focus = None;
        self.anchor = None;
        self.focus_scroll_pending = false;
        self.clear_dir_sizes();
        self.type_ahead = None;
        self.pending_reveal = None;
        self.state = PanelLoadState::Ready;
    }

    /// 揭示一个全路径（搜索结果双击/「打开所在目录」）：同目录已就绪且
    /// 普通列举 = 当场选中并滚动揭示；否则导航到所在目录（分支/注入
    /// 模式同目录则 refresh 退出注入），就绪后由 poll 应用 pending_reveal。
    pub fn reveal_path(&mut self, path: PathBuf) {
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if parent == self.dir && matches!(self.state, PanelLoadState::Ready) && !self.branch_view {
            self.apply_reveal(&path);
            return;
        }
        if parent == self.dir {
            // 同目录但在分支/注入视图或 Failed：refresh 回普通列举
            // （Loading 中 refresh no-op，就绪后照常应用 pending_reveal）。
            self.refresh();
        } else {
            self.navigate_to(parent);
        }
        self.pending_reveal = Some(path);
    }

    /// 选中并滚动揭示一个已在 entries 中的路径（reveal_path 的就绪应用）。
    fn apply_reveal(&mut self, path: &Path) {
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        let rows = self.rows();
        self.focus = rows
            .iter()
            .position(|&i| self.entries[i].path == path)
            .map(|pos| pos + 1);
        self.anchor = self.focus;
        self.focus_scroll_pending = self.focus.is_some();
    }

    /// 当前状态打包为标签快照（切走/关闭时回写 tabs[active_tab]）。
    pub fn snapshot_tab(&mut self) -> PanelTabSnapshot {
        let focus_name = self
            .focused_entry()
            .and_then(|e| e.path.file_name().map(|s| s.to_string_lossy().to_string()));
        let mut selected: Vec<PathBuf> = self.selected.iter().cloned().collect();
        selected.sort();
        PanelTabSnapshot {
            dir: self.dir.clone(),
            selected,
            focus_name,
            filter: self.filter.clone(),
            scroll_offset: self.last_scroll_offset,
            sort_key: self.sort_key,
            sort_asc: self.sort_asc,
            view_mode: self.view_mode,
            columns: self.columns.clone(),
            // 锁定/自定义标题随快照往返（阶段 AI）：活动标签的这两项
            // 存于 tabs[active_tab]，回写时原样带出。
            locked: self.tabs[self.active_tab].locked,
            custom_title: self.tabs[self.active_tab].custom_title.clone(),
        }
    }

    /// 恢复标签快照：重新列举目标目录（start_listing 语义——watcher 重建、
    /// 目录大小/type-ahead 清、退出分支视图），选中/焦点/滚动在列举就绪后
    /// 由 poll/render_list 应用。取舍：标签切换不进导航历史，历史随栏
    /// 共享（各标签独立历史需把 history 一并纳入快照，从简不做）。
    pub fn restore_tab(&mut self, snap: &PanelTabSnapshot) {
        self.start_listing(snap.dir.clone());
        self.filter = snap.filter.clone();
        self.sort_key = snap.sort_key;
        self.sort_asc = snap.sort_asc;
        self.view_mode = snap.view_mode;
        // 空列配置 = Default 快照（restore_tabs 已把面板当前值填入，正常
        // 不会走到），防御性保留现有列。
        if !snap.columns.is_empty() {
            self.columns = snap.columns.clone();
        }
        self.pending_tab_restore = Some(PendingTabRestore {
            selected: snap.selected.iter().cloned().collect(),
            focus_name: snap.focus_name.clone(),
        });
        self.pending_scroll_restore = Some(snap.scroll_offset);
    }

    /// 标签数（恒 ≥1）。
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// 活动标签索引。
    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    /// 标签 i 的目录（活动标签 = 实时 dir，非活动 = 快照 dir）。
    pub fn tab_dir(&self, i: usize) -> &Path {
        if i == self.active_tab {
            &self.dir
        } else {
            &self.tabs[i].dir
        }
    }

    /// Ctrl+T / 「+」：新建标签（复制当前目录与排序/列/视图模式；选中/
    /// 过滤/焦点/滚动/锁定/自定义标题不带入新标签），追加到末尾并切过去。
    pub fn new_tab(&mut self) {
        let current = self.snapshot_tab();
        let mut fresh = current.clone();
        fresh.selected = Vec::new();
        fresh.focus_name = None;
        fresh.filter = String::new();
        fresh.scroll_offset = 0.0;
        fresh.locked = false;
        fresh.custom_title = None;
        self.tabs[self.active_tab] = current;
        self.tabs.push(fresh);
        self.active_tab = self.tabs.len() - 1;
        let snap = self.tabs[self.active_tab].clone();
        self.restore_tab(&snap);
    }

    /// 标签 i 是否锁定。
    pub fn tab_locked(&self, i: usize) -> bool {
        self.tabs.get(i).map(|t| t.locked).unwrap_or(false)
    }

    /// 锁定/解锁标签（右键菜单「锁定」）。
    pub fn set_tab_locked(&mut self, i: usize, locked: bool) {
        if let Some(t) = self.tabs.get_mut(i) {
            t.locked = locked;
        }
    }

    /// 标签 i 的自定义标题（无 = None）。
    pub fn tab_custom_title(&self, i: usize) -> Option<String> {
        self.tabs.get(i).and_then(|t| t.custom_title.clone())
    }

    /// 设置/清除自定义标题（空串按 None 清除）。
    pub fn set_tab_custom_title(&mut self, i: usize, title: Option<String>) {
        if let Some(t) = self.tabs.get_mut(i) {
            t.custom_title = title.filter(|s| !s.trim().is_empty());
        }
    }

    /// 标签标题（未截断）：custom_title > 目录 basename（根目录显示完整
    /// 路径；截断在渲染层 tab_label 统一处理）。
    pub fn tab_title(&self, i: usize) -> String {
        if let Some(title) = self.tab_custom_title(i) {
            return title;
        }
        let dir = self.tab_dir(i);
        dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| dir.display().to_string())
    }

    /// 切换标签：回写当前快照 → 恢复目标 → 重新列举。
    pub fn switch_tab(&mut self, i: usize) {
        if i == self.active_tab || i >= self.tabs.len() {
            return;
        }
        let current = self.snapshot_tab();
        self.tabs[self.active_tab] = current;
        self.active_tab = i;
        let snap = self.tabs[i].clone();
        self.restore_tab(&snap);
    }

    /// 关闭标签：剩 1 个时 no-op（调用方禁用）。关当前标签切到相邻
    /// （优先右邻，末尾取左邻），被关标签不回写；关非当前标签不动面板。
    pub fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        if i < self.active_tab {
            self.active_tab -= 1;
        } else if i == self.active_tab {
            self.active_tab = i.min(self.tabs.len() - 1);
            let snap = self.tabs[self.active_tab].clone();
            self.restore_tab(&snap);
        }
    }

    /// Ctrl+Tab：循环下一个标签。
    pub fn next_tab(&mut self) {
        self.switch_tab((self.active_tab + 1) % self.tabs.len());
    }

    /// Ctrl+Shift+Tab：循环上一个标签。
    pub fn prev_tab(&mut self) {
        self.switch_tab((self.active_tab + self.tabs.len() - 1) % self.tabs.len());
    }

    /// 启动恢复：整组标签目录 + 活动索引（目录由调用方按
    /// fallback_existing_dir 回退；选中/过滤/滚动不持久化）。
    /// 活动索引越界自动 clamp；dirs 为空退化为单空标签。
    pub fn restore_tabs(&mut self, dirs: Vec<PathBuf>, active: usize) {
        self.restore_tabs_full(dirs.into_iter().map(|d| (d, false, None)).collect(), active);
    }

    /// 整组恢复（阶段 AI：带锁定/自定义标题——启动恢复 fm_tabs_* 用；
    /// 标签组应用走 restore_tabs 不恢复锁定）。
    pub fn restore_tabs_full(&mut self, tabs: Vec<(PathBuf, bool, Option<String>)>, active: usize) {
        let mut tabs = tabs;
        if tabs.is_empty() {
            tabs.push((PathBuf::new(), false, None));
        }
        self.tabs = tabs
            .into_iter()
            .map(|(dir, locked, custom_title)| PanelTabSnapshot {
                dir,
                // 排序/列/视图模式不持久化：继承面板当前值（= 启动时恢复的
                // 全局值），切到这些标签时不会被重置回默认。
                sort_key: self.sort_key,
                sort_asc: self.sort_asc,
                view_mode: self.view_mode,
                columns: self.columns.clone(),
                locked,
                custom_title,
                ..Default::default()
            })
            .collect();
        self.active_tab = active.min(self.tabs.len() - 1);
        let dir = self.tabs[self.active_tab].dir.clone();
        self.start_listing(dir);
    }

    /// render_list 消费一次性滚动恢复偏移（标签切换的精确恢复）。
    pub fn take_pending_scroll_restore(&mut self) -> Option<f32> {
        self.pending_scroll_restore.take()
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
            OpenStatus::Ready(Ok(listing)) => {
                self.entries = listing.entries;
                self.listing_truncated = listing.truncated;
                self.entries_version += 1;
                let existing: HashSet<&Path> =
                    self.entries.iter().map(|e| e.path.as_path()).collect();
                if let Some(pending) = self.pending_tab_restore.take() {
                    // 标签恢复：选中按存在性过滤；焦点按文件名定位（行索引在
                    // 内容变化后无意义）；滚动由 pending_scroll_restore 精确
                    // 恢复，不走焦点最小滚动揭示。
                    self.selected = pending.selected;
                    self.selected.retain(|p| existing.contains(p.as_path()));
                    self.focus = pending.focus_name.and_then(|name| {
                        let rows = self.rows();
                        rows.iter()
                            .position(|&i| {
                                self.entries[i].path.file_name()
                                    == Some(std::ffi::OsStr::new(name.as_str()))
                            })
                            .map(|pos| pos + 1)
                    });
                    self.focus_scroll_pending = false;
                    self.anchor = None;
                } else if let Some(path) = self.pending_reveal.take() {
                    // reveal_path 跳转：选中目标并滚动揭示（焦点按全路径定位，
                    // anchor 由 apply_reveal 一并设置）。
                    self.apply_reveal(&path);
                } else {
                    // refresh 路径：丢弃已不存在项的选中态；navigate 路径
                    // selected 已清空，retain 为 no-op。
                    self.selected.retain(|p| existing.contains(p.as_path()));
                    self.focus = None;
                    self.anchor = None;
                }
                self.state = PanelLoadState::Ready;
                self.comments = crate::views::fm_comments::read_comments(&self.dir);
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

    /// 重读当前目录的 descript.ion 注释缓存（编辑写回后调用；阶段 W）。
    pub fn reload_comments(&mut self) {
        self.comments = crate::views::fm_comments::read_comments(&self.dir);
    }

    /// 为当前目录建立 FS 监听；已监听同一路径且递归模式一致则跳过
    /// （避免每帧重建）；fm_watch_recursive 选项变化时按新模式重建。
    fn ensure_watch(&mut self) {
        if self
            .watch
            .as_ref()
            .map(|w| w.path == self.dir && w.recursive == self.watch_recursive)
            .unwrap_or(false)
        {
            return;
        }
        self.watch = try_watch(&self.dir, self.wake_ctx.clone(), self.watch_recursive);
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
            dirs_first: self.dirs_first,
            branch: self.branch_view,
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
            self.dirs_first,
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

    /// TC 勾选语义（fm_space_action = "toggle_select" 的空格 / 无条件 Insert）：
    /// 切换焦点行选中态并下移一行（FocusOnly 不动锚点）；「..」行/无焦点
    /// 不勾选只下移。
    pub fn toggle_focused_selection_and_advance(&mut self) {
        if let Some(row) = self.focus {
            if row > 0 {
                if let Some(path) = self.row_path(row) {
                    if !self.selected.remove(&path) {
                        self.selected.insert(path);
                    }
                }
            }
            self.move_focus(1, FocusMove::FocusOnly);
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
    /// sep i = columns[i] 与 columns[i+1] 之间的分隔线（i ≥ 1 时调整
    /// columns[i] 的宽度，其右侧列块经 col_shift 平移）。
    pub(crate) fn drag_column_sep(&mut self, sep: usize, dx: f32, header_rect: egui::Rect) {
        let total_fixed: f32 = self.columns[1..].iter().map(|(_, w)| w).sum();
        let min_shift = (header_rect.left() + NAME_COL_MIN + total_fixed
            - (header_rect.right() - COL_RIGHT_PAD))
            .min(0.0);
        let shift_room = (self.col_shift + dx).clamp(min_shift, 0.0) - self.col_shift;
        let d = if sep == 0 {
            // 名称列是弹性宽度，没有独立字段，列块平移即名称列缩放。
            shift_room
        } else {
            let Some((_, width)) = self.columns.get_mut(sep) else {
                return;
            };
            let width_room = (*width + dx).clamp(COL_MIN_WIDTH, COL_MAX_WIDTH) - *width;
            let d = if dx > 0.0 {
                width_room.min(shift_room)
            } else {
                width_room.max(shift_room)
            };
            *width += d;
            d
        };
        self.col_shift += d;
    }

    /// 列头右键勾选增删列（阶段 V）：新增固定列追加到列块最右（默认
    /// 宽度）；删除维持「固定列 ≥1」不变式（唯一固定列不可删）。Name
    /// 恒为首列，不可增删（调用方菜单置灰，这里防御）。
    pub fn toggle_column(&mut self, kind: ColumnKind) {
        if kind == ColumnKind::Name {
            return;
        }
        if let Some(pos) = self.columns.iter().position(|(k, _)| *k == kind) {
            if self.columns.len() > 2 {
                self.columns.remove(pos);
            }
        } else {
            self.columns.push((kind, kind.default_width()));
        }
    }

    /// 固定列宽度查询（snapshot 兼容字段 fm_col_size_width/mtime 用）。
    pub fn column_width(&self, kind: ColumnKind) -> Option<f32> {
        self.columns
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, w)| *w)
    }
}

/// 后台线程收集目录条目：单项读取失败（权限等）跳过；metadata 跟随符号
/// 链接（size/mtime/is_dir 取链接目标），失败降级为空值但保留条目。
fn read_dir_entries(path: &Path) -> Result<DirListing, String> {
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
        let is_readonly = meta
            .as_ref()
            .map(|m| m.permissions().readonly())
            .unwrap_or(false);
        let is_system = windows_attr_system(meta.as_ref());
        let mtime = meta.and_then(|m| m.modified().ok());
        entries.push(FsEntry {
            name,
            path,
            is_dir,
            size,
            mtime,
            is_symlink,
            is_hidden,
            is_readonly,
            is_system,
            rel_dir: String::new(),
        });
    }
    Ok(DirListing {
        entries,
        truncated: false,
    })
}

/// 分支视图（Ctrl+B）递归列举：当前目录 + 所有子目录的文件扁平收集，
/// 超 BRANCH_MAX_ENTRIES 截断（truncated 上报）。
fn read_dir_entries_recursive(path: &Path) -> Result<DirListing, String> {
    collect_branch(path, BRANCH_MAX_ENTRIES)
}

/// 递归收集核心（max 参数化便于测试截断）：目录不产出条目（分支视图
/// 无目录行，符号链接目录不跟进防环、按文件条目计）；单项失败跳过；
/// 分支条目的 name 直接存显示名（顶层 = 文件名，子目录 = `rel_dir/name`，
/// 分隔符统一 `/`），排序/过滤/type-ahead/渲染无需特判。
fn collect_branch(path: &Path, max: usize) -> Result<DirListing, String> {
    // 顶层目录不可读 = 错误（同普通列举）；子目录失败跳过。
    std::fs::read_dir(path).map_err(|e| format!("无法读取目录: {e}"))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, String)> = vec![(path.to_path_buf(), String::new())];
    'outer: while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in rd.flatten() {
            let item_path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let ft = item.file_type().ok();
            let is_symlink = ft.map(|t| t.is_symlink()).unwrap_or(false);
            if !is_symlink && ft.map(|t| t.is_dir()).unwrap_or(false) {
                let child_rel = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                stack.push((item_path, child_rel));
                continue;
            }
            if entries.len() >= max {
                truncated = true;
                break 'outer;
            }
            let meta = item.metadata().ok();
            let size = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
            let is_hidden = is_hidden_name(&name) || windows_attr_hidden(meta.as_ref());
            let is_readonly = meta
                .as_ref()
                .map(|m| m.permissions().readonly())
                .unwrap_or(false);
            let is_system = windows_attr_system(meta.as_ref());
            let mtime = meta.and_then(|m| m.modified().ok());
            let display = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            entries.push(FsEntry {
                name: display,
                path: item_path,
                // 分支视图不产出目录行（符号链接目录也按文件计）。
                is_dir: false,
                size,
                mtime,
                is_symlink,
                is_hidden,
                is_readonly,
                is_system,
                rel_dir: rel.clone(),
            });
        }
    }
    Ok(DirListing { entries, truncated })
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

#[cfg(windows)]
fn windows_attr_system(meta: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    meta.map(|m| m.file_attributes() & FILE_ATTRIBUTE_SYSTEM != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn windows_attr_system(_: Option<&std::fs::Metadata>) -> bool {
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
            is_readonly: false,
            is_system: false,
            rel_dir: String::new(),
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
        assert!(try_watch(&missing, None, false).is_none());
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

    /// 递归 watch（fm_watch_recursive，阶段 Z）：子目录内的新建文件也触发
    /// refresh（非递归只看当前层）。子目录新文件不进当前层 entries，以
    /// entries_version 变化为 refresh 证据。
    #[test]
    fn watch_recursive_sees_subdir_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.watch_recursive = true;
        panel.navigate_to(tmp.path().to_path_buf());
        for _ in 0..200 {
            if !panel.poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert!(
            panel.watch.as_ref().is_some_and(|w| w.recursive),
            "watch_recursive = true 时应建递归 watcher"
        );

        let version = panel.entries_version;
        std::fs::write(sub.join("deep-new-file.txt"), b"x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            panel.poll();
            if panel.entries_version != version {
                break;
            }
            assert!(Instant::now() < deadline, "递归 watch 未感知子目录变化");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
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
            is_readonly: false,
            is_system: false,
            rel_dir: String::new(),
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
            is_readonly: false,
            is_system: false,
            rel_dir: String::new(),
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

    /// 分支列举：当前目录 + 所有子目录的文件扁平收集，name = 显示名
    /// （`rel_dir/name`），目录不产出条目，rel_dir 记录相对子目录路径；
    /// 超 max 截断并上报 truncated。
    #[test]
    fn branch_listing_flattens_files_with_display_names() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"b").unwrap();
        std::fs::write(root.join("sub/deep/c.txt"), b"c").unwrap();

        let listing = collect_branch(root, BRANCH_MAX_ENTRIES).unwrap();
        assert!(!listing.truncated);
        assert_eq!(listing.entries.len(), 3);
        assert!(listing.entries.iter().all(|e| !e.is_dir));
        let mut by_name: std::collections::HashMap<&str, &FsEntry> = listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e))
            .collect();
        let top = by_name.remove("a.txt").expect("顶层文件");
        assert_eq!(top.rel_dir, "");
        let sub = by_name.remove("sub/b.txt").expect("子目录文件");
        assert_eq!(sub.rel_dir, "sub");
        let deep = by_name.remove("sub/deep/c.txt").expect("深层文件");
        assert_eq!(deep.rel_dir, "sub/deep");
        assert!(by_name.is_empty(), "目录不得产出条目: {by_name:?}");
        // path 均为全路径（双击/文件操作无需特判）。
        assert!(listing.entries.iter().all(|e| e.path.is_absolute()));

        // 截断：max=2 → 2 条 + truncated。
        let listing = collect_branch(root, 2).unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), 2);
    }

    /// 分支视图开关：进入递归列举（就绪后条目为扁平文件 + 显示名），
    /// navigate/refresh 自动退出回普通列举。
    #[test]
    fn branch_view_toggle_and_auto_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/b.txt"), b"b").unwrap();
        let poll_ready = |panel: &mut FsPanel| {
            for _ in 0..200 {
                if !panel.poll() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(matches!(panel.state, PanelLoadState::Ready));
        };

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(root.to_path_buf());
        poll_ready(&mut panel);
        assert!(!panel.branch_view);
        // 普通列举：只有目录行 sub。
        assert_eq!(panel.entries.len(), 1);
        assert!(panel.entries[0].is_dir);

        // 进入分支：扁平列出 sub/b.txt（显示名带相对路径）。
        panel.toggle_branch_view();
        assert!(panel.branch_view);
        poll_ready(&mut panel);
        assert!(panel.branch_view);
        assert_eq!(panel.entries.len(), 1);
        assert_eq!(panel.entries[0].name, "sub/b.txt");
        assert!(!panel.entries[0].is_dir);
        assert!(!panel.listing_truncated);

        // 退出分支：回普通列举。
        panel.toggle_branch_view();
        assert!(!panel.branch_view);
        poll_ready(&mut panel);
        assert_eq!(panel.entries.len(), 1);
        assert!(panel.entries[0].is_dir);

        // 分支模式下 navigate 到其他目录自动退出分支（start_listing 清标志）。
        panel.enter_branch_view();
        poll_ready(&mut panel);
        assert!(panel.branch_view);
        panel.navigate_to(root.join("sub"));
        assert!(!panel.branch_view);
    }

    /// 搜索注入（inject_entries_branch）：命中集作为分支式展示直接就绪，
    /// reveal_path 退出注入后选中目标；同目录当场选中、跨目录导航后
    /// poll 应用 pending_reveal。
    #[test]
    fn inject_entries_branch_and_reveal_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"b").unwrap();

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(root.to_path_buf());
        poll_until_ready(&mut panel);

        // 注入：直接 Ready、分支标志、显示名条目（同分支视图约定）。
        let mk = |name: &str, path: PathBuf, rel_dir: &str| FsEntry {
            name: name.to_string(),
            path,
            is_dir: false,
            size: Some(1),
            mtime: None,
            is_symlink: false,
            is_hidden: false,
            is_readonly: false,
            is_system: false,
            rel_dir: rel_dir.to_string(),
        };
        panel.inject_entries_branch(vec![
            mk("a.txt", root.join("a.txt"), ""),
            mk("sub/b.txt", root.join("sub/b.txt"), "sub"),
        ]);
        assert!(panel.branch_view);
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert_eq!(panel.entries.len(), 2);
        assert_eq!(panel.entries[1].name, "sub/b.txt");

        // 注入模式下同目录 reveal：退出注入回普通列举，就绪后选中目标。
        panel.reveal_path(root.join("a.txt"));
        assert!(!panel.branch_view);
        poll_until_ready(&mut panel);
        assert!(panel.selected.contains(&root.join("a.txt")));
        // 目录优先排序（sub 在前）→ a.txt 是 UI 行 2（行 0 = 「..」）。
        assert_eq!(panel.focus, Some(2));

        // 普通模式同目录 reveal：不重新列举，当场选中。
        panel.reveal_path(root.join("sub"));
        assert!(matches!(panel.state, PanelLoadState::Ready));
        assert_eq!(panel.selected.len(), 1);
        assert!(panel.selected.contains(&root.join("sub")));
        assert_eq!(panel.focus, Some(1));

        // 跨目录 reveal：从 sub 揭示 root/a.txt（导航 + pending_reveal）。
        panel.navigate_to(root.join("sub"));
        poll_until_ready(&mut panel);
        panel.reveal_path(root.join("a.txt"));
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, root);
        assert!(panel.selected.contains(&root.join("a.txt")));
        assert_eq!(panel.focus, Some(2));
    }

    /// poll 到 Ready 的测试辅助。
    fn poll_until_ready(panel: &mut FsPanel) {
        for _ in 0..200 {
            if !panel.poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(panel.state, PanelLoadState::Ready));
    }

    /// 标签页不变式：切换往返状态保持（选中/焦点按名/过滤恢复；快照在
    /// 切走时回写）；新建标签复制当前目录但状态全新；标签切换不进导航
    /// 历史（go_back 目标不受标签切换影响）。
    #[test]
    fn tab_switch_roundtrip_preserves_state() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(dir_a.join("keep-me.txt"), b"x").unwrap();
        std::fs::write(dir_a.join("other.txt"), b"y").unwrap();
        std::fs::write(dir_b.join("b-file.txt"), b"z").unwrap();

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(dir_a.clone());
        poll_until_ready(&mut panel);
        assert_eq!(panel.tab_count(), 1);
        panel.selected.insert(dir_a.join("keep-me.txt"));
        // 名称升序：keep-me.txt(行 1), other.txt(行 2)。
        panel.focus = Some(2);
        panel.filter = "txt".to_string();

        // Ctrl+T：新标签复制当前目录，状态全新（选中/过滤清空）。
        panel.new_tab();
        poll_until_ready(&mut panel);
        assert_eq!(panel.tab_count(), 2);
        assert_eq!(panel.active_tab(), 1);
        assert_eq!(panel.dir, dir_a);
        assert!(panel.selected.is_empty());
        assert!(panel.filter.is_empty());

        // 新标签导航到 dir_b。
        panel.navigate_to(dir_b.clone());
        poll_until_ready(&mut panel);

        // 切回标签 0：选中/焦点/过滤恢复（焦点按文件名定位）。
        panel.switch_tab(0);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_a);
        assert!(panel.selected.contains(&dir_a.join("keep-me.txt")));
        assert_eq!(panel.filter, "txt");
        assert_eq!(
            panel.focused_entry().map(|e| e.path.clone()),
            Some(dir_a.join("other.txt"))
        );
        // 标签切换不进导航历史：回退仍是 navigate 历史里的上一个目录。
        assert!(panel.can_go_back());

        // 往返：标签 1 恢复为 dir_b 实时状态；再回标签 0 仍是 dir_a。
        panel.switch_tab(1);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_b);
        panel.prev_tab();
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_a);
        assert_eq!(panel.active_tab(), 0);
        // 循环：prev 从 0 绕到末尾。
        panel.prev_tab();
        poll_until_ready(&mut panel);
        assert_eq!(panel.active_tab(), 1);
        assert_eq!(panel.dir, dir_b);
    }

    /// 关闭标签：关非当前标签只收缩列表；关当前标签切相邻（优先右邻，
    /// 末尾取左邻）并恢复其快照；剩 1 个时 no-op。
    #[test]
    fn tab_close_switches_to_neighbor() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        let dir_c = tmp.path().join("c");
        for d in [&dir_a, &dir_b, &dir_c] {
            std::fs::create_dir_all(d).unwrap();
        }

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.navigate_to(dir_a.clone());
        poll_until_ready(&mut panel);
        panel.new_tab();
        poll_until_ready(&mut panel);
        panel.navigate_to(dir_b.clone());
        poll_until_ready(&mut panel);
        panel.new_tab();
        poll_until_ready(&mut panel);
        panel.navigate_to(dir_c.clone());
        poll_until_ready(&mut panel);
        // tabs = [a, b, c]，active = 2。
        assert_eq!(panel.tab_count(), 3);

        // 剩 1 个前关闭非当前标签（i < active）：只收缩，面板不动。
        panel.close_tab(0);
        assert_eq!(panel.tab_count(), 2);
        assert_eq!(panel.active_tab(), 1);
        assert_eq!(panel.dir, dir_c);

        // 关当前标签（末尾）：切到左邻并恢复其快照（dir_b）。
        panel.close_tab(1);
        poll_until_ready(&mut panel);
        assert_eq!(panel.tab_count(), 1);
        assert_eq!(panel.active_tab(), 0);
        assert_eq!(panel.dir, dir_b);

        // 剩 1 个：no-op。
        panel.close_tab(0);
        assert_eq!(panel.tab_count(), 1);
        assert_eq!(panel.dir, dir_b);
    }

    /// 启动恢复：restore_tabs 建整组标签并 clamp 活动索引；空 dirs
    /// 退化为单标签。
    #[test]
    fn restore_tabs_clamps_active_and_lists() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.restore_tabs(vec![dir_a, dir_b.clone()], 5);
        assert_eq!(panel.tab_count(), 2);
        assert_eq!(panel.active_tab(), 1);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_b);
        // 非活动标签目录保留（tab_dir 读取）。
        assert_eq!(panel.tab_dir(0), tmp.path().join("a").as_path());

        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.restore_tabs(Vec::new(), 0);
        assert_eq!(panel.tab_count(), 1);
    }

    /// 锁定标签（阶段 AI）：锁定时 navigate_to 自动改为新开标签（原标签
    /// 目录不动、新标签不继承锁定/标题）；解锁后恢复正常导航。
    #[test]
    fn locked_tab_navigates_open_new_tab() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.restore_tabs(vec![dir_a.clone()], 0);
        poll_until_ready(&mut panel);
        // 未锁定：正常导航（单标签）。
        panel.navigate_to(dir_b.clone());
        assert_eq!(panel.tab_count(), 1);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_b);
        // 锁定后导航：新开标签到目标，原标签保持原目录。
        panel.set_tab_locked(0, true);
        panel.navigate_to(dir_a.clone());
        assert_eq!(panel.tab_count(), 2);
        assert_eq!(panel.active_tab(), 1, "新标签成为活动标签");
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_a);
        assert_eq!(panel.tab_dir(0), dir_b.as_path(), "锁定标签保持原目录");
        assert!(panel.tab_locked(0));
        assert!(!panel.tab_locked(1), "新标签不继承锁定");
        // 新标签（未锁定）导航仍走正常路径。
        panel.navigate_to(dir_b.clone());
        assert_eq!(panel.tab_count(), 2);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_b);
    }

    /// 锁定标签的历史后退同样开新标签（阶段 AI 从简：历史不移动）。
    #[test]
    fn locked_tab_go_back_opens_new_tab() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        let dir_c = tmp.path().join("c");
        for d in [&dir_a, &dir_b, &dir_c] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.restore_tabs(vec![dir_a.clone()], 0);
        poll_until_ready(&mut panel);
        // 初始目录不进历史（restore_tabs 语义）：a→b→c 后历史 = [b, c]。
        panel.navigate_to(dir_b.clone());
        panel.navigate_to(dir_c.clone());
        poll_until_ready(&mut panel);
        panel.set_tab_locked(0, true);
        assert!(panel.go_back());
        assert_eq!(panel.tab_count(), 2);
        poll_until_ready(&mut panel);
        assert_eq!(panel.dir, dir_b, "历史目标开在新标签");
        assert_eq!(panel.tab_dir(0), dir_c.as_path());
        assert!(panel.can_go_back(), "锁定标签的历史不移动");
    }

    /// 自定义标题（阶段 AI）：显示优先级 custom_title > basename；空串
    /// 按清除处理；restore_tabs_full 恢复锁定/标题。
    #[test]
    fn tab_custom_title_and_restore_full() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("alpha");
        std::fs::create_dir_all(&dir_a).unwrap();
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.restore_tabs_full(
            vec![
                (dir_a.clone(), true, Some("工作".to_string())),
                (tmp.path().to_path_buf(), false, None),
            ],
            0,
        );
        assert!(panel.tab_locked(0));
        assert!(!panel.tab_locked(1));
        assert_eq!(panel.tab_title(0), "工作");
        assert_eq!(
            panel.tab_title(1),
            tmp.path().file_name().unwrap().to_string_lossy()
        );
        // 设置/清除标题。
        panel.set_tab_custom_title(1, Some("  ".to_string()));
        assert_eq!(panel.tab_custom_title(1), None, "空白标题按清除");
        panel.set_tab_custom_title(1, Some("临时".to_string()));
        assert_eq!(panel.tab_title(1), "临时");
        // 快照往返保留锁定/标题（切走再切回）。
        panel.switch_tab(1);
        panel.switch_tab(0);
        assert!(panel.tab_locked(0));
        assert_eq!(panel.tab_title(0), "工作");
    }

    /// fm_columns 解析/序列化（阶段 V）：未知 kind 丢弃、宽度 clamp、
    /// 去重、Name 强制首位、固定列不足补默认；空输入 → None（legacy 播种）。
    #[test]
    fn parse_and_serialize_columns() {
        assert!(parse_columns(&[]).is_none());
        // 正常解析 + Name 归首位 + 宽度 clamp + 去重 + 未知丢弃。
        let cols = parse_columns(&[
            "size:120".to_string(),
            "bogus".to_string(),
            "name".to_string(),
            "mtime:9999".to_string(),
            "size:200".to_string(), // 重复丢弃
        ])
        .unwrap();
        assert_eq!(
            cols,
            vec![
                (ColumnKind::Name, 0.0),
                (ColumnKind::Size, 120.0),
                (ColumnKind::Mtime, 400.0),
            ]
        );
        // 往返。
        let items = serialize_columns(&cols);
        assert_eq!(items, ["name", "size:120", "mtime:400"]);
        assert_eq!(parse_columns(&items).unwrap(), cols);
        // 只有 Name（或全未知）→ 补默认大小/时间列。
        let cols = parse_columns(&["name".to_string(), "zzz".to_string()]).unwrap();
        assert_eq!(
            cols,
            vec![
                (ColumnKind::Name, 0.0),
                (ColumnKind::Size, SIZE_COL_WIDTH),
                (ColumnKind::Mtime, MTIME_COL_WIDTH),
            ]
        );
    }

    /// legacy 播种：fm_columns 空时用 fm_col_size_width/mtime 造默认三列。
    #[test]
    fn restore_columns_seeds_from_legacy_widths() {
        let cols = restore_columns(&[], 120.0, 150.0);
        assert_eq!(
            cols,
            vec![
                (ColumnKind::Name, 0.0),
                (ColumnKind::Size, 120.0),
                (ColumnKind::Mtime, 150.0),
            ]
        );
        // 非空 fm_columns 优先（legacy 宽度忽略）。
        let cols = restore_columns(&["ext".to_string()], 120.0, 150.0);
        assert_eq!(
            cols,
            vec![
                (ColumnKind::Name, 0.0),
                (ColumnKind::Ext, ColumnKind::Ext.default_width()),
            ]
        );
    }

    /// 列勾选增删（阶段 V）：Name 不可动；新增追加到列块最右（默认
    /// 宽度）；唯一固定列不可删（不变式：Name + ≥1 固定列）。
    #[test]
    fn toggle_column_maintains_invariants() {
        let mut panel = FsPanel::new(SortKey::Name, true);
        assert_eq!(panel.columns, default_columns());
        panel.toggle_column(ColumnKind::Name);
        assert_eq!(panel.columns, default_columns(), "Name 不可增删");
        panel.toggle_column(ColumnKind::Ext);
        assert_eq!(
            panel.columns,
            vec![
                (ColumnKind::Name, 0.0),
                (ColumnKind::Size, SIZE_COL_WIDTH),
                (ColumnKind::Mtime, MTIME_COL_WIDTH),
                (ColumnKind::Ext, ColumnKind::Ext.default_width()),
            ]
        );
        panel.toggle_column(ColumnKind::Ext);
        assert_eq!(panel.columns, default_columns());
        // 删到剩 1 个固定列后再删 = no-op。
        panel.toggle_column(ColumnKind::Size);
        panel.toggle_column(ColumnKind::Mtime);
        assert_eq!(panel.columns.len(), 2, "唯一固定列不可删");
        assert_eq!(panel.columns[1].0, ColumnKind::Mtime);
    }

    /// 分隔线拖动回归（阶段 V 泛化后默认三列手感必须与旧硬编码一致，
    /// 期望值 = 旧公式手算）：sep0 只动 col_shift；sep1 调 Size 宽 +
    /// shift 吸收；撞限同步停住。
    #[test]
    fn drag_column_sep_default_layout_matches_legacy() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 22.0));
        // min_shift = (0 + 80 + 90 + 110 - (800 - 6)).min(0) = -514。
        let mut panel = FsPanel::new(SortKey::Name, true);
        // sep0 右拖：列块已贴右缘（shift=0 上限），不动。
        panel.drag_column_sep(0, 50.0, rect);
        assert_eq!(panel.col_shift, 0.0);
        // sep0 左拖 50：列块左移 50。
        panel.drag_column_sep(0, -50.0, rect);
        assert_eq!(panel.col_shift, -50.0);
        // sep0 左拖超额：撞 min_shift 停住。
        panel.drag_column_sep(0, -9999.0, rect);
        assert_eq!(panel.col_shift, -514.0);
        panel.col_shift = 0.0;
        // sep1（Size|Mtime）右拖：Size 变宽需列块右移，shift=0 挡住 → 不动。
        panel.drag_column_sep(1, 20.0, rect);
        assert_eq!(panel.columns[1].1, SIZE_COL_WIDTH);
        assert_eq!(panel.col_shift, 0.0);
        // sep1 左拖 20：Size 90→70，列块跟着左移 20。
        panel.drag_column_sep(1, -20.0, rect);
        assert_eq!(panel.columns[1].1, 70.0);
        assert_eq!(panel.col_shift, -20.0);
        // sep1 继续左拖至列宽下限 60 后停住。
        panel.drag_column_sep(1, -100.0, rect);
        assert_eq!(panel.columns[1].1, 60.0);
        assert_eq!(panel.col_shift, -30.0);
        // sep2（Mtime 右缘=行右缘线不存在；sep 最大 = columns.len()-2）。
        // 三列 → sep ∈ {0, 1}；越界 sep 防御 no-op。
        panel.drag_column_sep(9, -10.0, rect);
        assert_eq!(panel.col_shift, -30.0);
    }

    /// 多列布局的 min_shift 按固定列宽合计（泛化点）。
    #[test]
    fn drag_column_sep_multi_column_min_shift() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 22.0));
        let mut panel = FsPanel::new(SortKey::Name, true);
        panel.toggle_column(ColumnKind::Ext); // 固定列 = Size 90 + Mtime 110 + Ext 70
                                              // min_shift = 80 + 270 - 794 = -444。
        panel.drag_column_sep(0, -9999.0, rect);
        assert_eq!(panel.col_shift, -444.0);
        // sep2（Mtime|Ext）左拖调 Mtime 宽。
        panel.col_shift = 0.0;
        panel.drag_column_sep(2, -30.0, rect);
        assert_eq!(panel.columns[2].1, MTIME_COL_WIDTH - 30.0);
        assert_eq!(panel.col_shift, -30.0);
    }

    /// 标签快照携带排序/列/视图模式（阶段 V）；restore_tabs 的标签
    /// 继承面板当前值（不被重置回默认）。
    #[test]
    fn tab_snapshot_carries_sort_columns_view_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut panel = FsPanel::new(SortKey::Mtime, false);
        panel.view_mode = PanelViewMode::Brief;
        panel.toggle_column(ColumnKind::Attr);
        panel.navigate_to(tmp.path().to_path_buf());
        poll_until_ready(&mut panel);
        let snap = panel.snapshot_tab();
        assert_eq!(snap.sort_key, SortKey::Mtime);
        assert!(!snap.sort_asc);
        assert_eq!(snap.view_mode, PanelViewMode::Brief);
        assert_eq!(snap.columns, panel.columns);
        // 恢复：字段生效。
        let mut panel2 = FsPanel::new(SortKey::Name, true);
        panel2.restore_tab(&snap);
        assert_eq!(panel2.sort_key, SortKey::Mtime);
        assert!(!panel2.sort_asc);
        assert_eq!(panel2.view_mode, PanelViewMode::Brief);
        assert_eq!(panel2.columns, snap.columns);
        // restore_tabs 的标签继承面板当前配置。
        panel2.restore_tabs(vec![tmp.path().to_path_buf()], 0);
        let snap2 = panel2.snapshot_tab();
        assert_eq!(snap2.sort_key, SortKey::Mtime);
        assert_eq!(snap2.columns, snap.columns);
    }
}
