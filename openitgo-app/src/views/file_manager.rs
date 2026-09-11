//! 双栏文件管理器视图（Total Commander 形态）：左右两个 `FsPanel` 各自独立
//! 浏览本地文件系统，Tab 切换焦点栏，单击/Ctrl/Shift 选择，双击分发打开
//! （目录 → 栏内进入；压缩包 → Archive 视图；其余 → open_path 分发）。
//! 单栏模式右半为预览面板（图片/文本/元信息占位，选中即预览）；
//! 双栏模式 F3 对焦点文件弹临时预览窗，复用同一预览管线。
//!
//! 行渲染严格遵循 AGENTS.md 的列布局约定（与 archive.rs 明细列表同范式）：
//! 固定行高 + show_rows 虚拟化、item_spacing.y 归零、行内容 scope 内
//! interact_size.y 压回 ROW_HEIGHT-4、scope 后 advance_cursor_after_rect
//! 钉回行底、右侧固定列（FsPanel::columns 驱动）painter.text 右对齐直绘
//! （禁止 RTL 嵌套）。

use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::archive::{format_mtime, human_size};
use crate::views::file_manager_attr::{apply_to_path, AttrAction, AttrTimestampDialog};
use crate::views::file_manager_checksum::{is_checksum_file_name, ChecksumDialog};
use crate::views::file_manager_dialog::{
    CompressDialog, ConflictDialog, CopyMoveDialog, DeleteDialog, FmDialog, FmDialogOutcome,
    MultiRenameDialog, NewDirDialog, NewFileDialog, RenameDialog, SelectGroupDialog, SplitDialog,
};
use crate::views::file_manager_icons::{sys_icon_kind, SysIconCache, SysIconLookup};
use crate::views::file_manager_panel::{
    fallback_existing_dir, list_drives, ColumnKind, FocusMove, FsPanel, PanelLoadState,
    PanelViewMode, COL_RIGHT_PAD, ROW_HEIGHT,
};
use crate::views::file_manager_rows::{attr_string, lower_ext, FsEntry, SortKey};
use crate::views::file_manager_search::{SearchDialog, SearchUiAction};
use crate::views::file_manager_sync::{SyncDialog, SyncUiAction};
use crate::views::file_manager_thumbs::{
    grid_cols, grid_row_count, grid_row_of, truncate_cell_name, ThumbCache, ThumbKey, ThumbLookup,
    THUMB_CELL_H, THUMB_CELL_W, THUMB_MAX_DIM,
};
use crate::views::file_ops::{
    collect_chunks, create_dir, create_text_file, format_eta, merge_target_base, rename_entry,
    retry_sources, suggest_folder_name, suggest_text_file_name, ConflictMode, FileOpManager,
    FinishedOp, OpKind, OpSpeedMeter,
};
use crate::views::preview_bytes::{
    decode_preview_text, format_hex_line, load_file_preview, PreviewData, PreviewOutcome,
};
use egui_phosphor_icons::{icons, Icon};
use openitgo_parser::archive::archive_kind;
use openitgo_storage::models::{FmBookmarkGroup, FmButton};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// 双栏/预览分隔条宽度（pt）。
const SPLITTER_WIDTH: f32 = 6.0;
/// 栏内容最小宽度（pt）：窗口过小时不再压缩。
const PANEL_MIN_WIDTH: f32 = 120.0;
/// 双栏比例/预览比例的取值范围（与 settings.fm_dual_ratio 的 clamp 一致）。
const RATIO_MIN: f32 = 0.2;
const RATIO_MAX: f32 = 0.8;
/// 表头「名称」文字的左缩进（pt）：与行内容对齐
/// （行 = 6pt shrink + 约 16pt 图标 + 6pt 间距）。
const NAME_HEADER_INDENT: f32 = 6.0 + 16.0 + 6.0;

/// 面板布局：双栏（ratio = 左栏宽占比）/ 单栏（右半为预览面板）。
pub enum PanelLayout {
    Dual { ratio: f32 },
    Single { preview_open: bool },
}

/// 预览内容模式（阶段 R：预览头部 tab，按内容类型自动解析初值可手切）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewMode {
    Text,
    Image,
    /// 二进制/任意字节查看（is_previewable_name 门槛之外类型的默认档）。
    Hex,
}

/// 预览文本编码手动选择（默认 Auto = 自动检测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewEncoding {
    Auto,
    Utf8,
    Gbk,
    ShiftJis,
    Big5,
}

impl PreviewEncoding {
    /// ComboBox 显示名。
    fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动检测",
            Self::Utf8 => "UTF-8",
            Self::Gbk => "GBK",
            Self::ShiftJis => "Shift-JIS",
            Self::Big5 => "Big5",
        }
    }

    /// decode_preview_text 的 label 参数（Auto = None）。
    fn decode_label(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Utf8 => Some("utf-8"),
            Self::Gbk => Some("gbk"),
            Self::ShiftJis => Some("shift-jis"),
            Self::Big5 => Some("big5"),
        }
    }

    const ALL: [Self; 5] = [
        Self::Auto,
        Self::Utf8,
        Self::Gbk,
        Self::ShiftJis,
        Self::Big5,
    ];
}

/// 文本内搜索（纯函数）：返回包含 needle 的行号列表（不区分大小写；
/// needle 为空返回空）。作用于已加载（可能截断）的预览文本。
fn find_text_matches(text: &str, needle: &str) -> Vec<usize> {
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

/// 双击压缩包分发方式（settings.fm_archive_open）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmArchiveOpen {
    /// 进 Archive 视图（默认，设计决策 3）。
    Archive,
    /// 作为漫画打开（跳过启发式分流）。
    Comic,
    /// 鼠标处弹小菜单二选一。
    Ask,
}

impl FmArchiveOpen {
    pub fn from_setting(s: &str) -> Self {
        match s {
            "comic" => Self::Comic,
            "ask" => Self::Ask,
            _ => Self::Archive,
        }
    }
}

/// 鼠标框选模式（settings.fm_rubber_band，阶段 U）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RubberBandMode {
    /// 右键拖动框选（默认；右键单击未超阈值仍弹上下文菜单）。
    Right,
    /// 左键从空白区起拖框选（行/cell 上的左键拖动维持拖放 payload）。
    Left,
    /// 关闭。
    Off,
}

impl RubberBandMode {
    pub fn from_setting(s: &str) -> Self {
        match s {
            "left" => Self::Left,
            "off" => Self::Off,
            _ => Self::Right,
        }
    }
}

/// 行为设置包（阶段 O，settings.fm_* 可选行为）：默认值 = 一期现状行为。
/// app 侧每帧从 settings 构造下发（同 fm_show_hidden 模式——行为设置不进
/// FmStateSnapshot，不参与快照 diff）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FmBehaviorOptions {
    /// 删除前确认（fm_confirm_delete）。
    pub confirm_delete: bool,
    /// 显示隐藏文件（fm_show_hidden）。
    pub show_hidden: bool,
    /// 永久删除（fm_delete_mode = "permanent"）；Shift+Del = 另一档快捷
    /// （生效档位 = delete_permanent XOR shift）。
    pub delete_permanent: bool,
    /// 空格 TC 勾选语义（fm_space_action = "toggle_select"）：切换焦点项
    /// 选中并下移；false = 现状「计算目录大小」。Insert 键无条件勾选下移。
    pub space_toggle_select: bool,
    /// 目录恒排在文件前（fm_dirs_first）。
    pub dirs_first: bool,
    /// 栏间拖放复制前弹确认框（fm_drag_confirm；false = 松开直拷自动改名，
    /// 按住 Shift = 移动）。
    pub drag_confirm: bool,
    /// 双击压缩包分发（fm_archive_open）。
    pub archive_open: FmArchiveOpen,
    /// Esc 不清选中（fm_esc_keep_selection）：Esc 链止于清过滤。
    pub esc_keep_selection: bool,
    /// 列表/网格空白区双击 = 回上级目录（fm_dblclick_blank_up）。
    pub dblclick_blank_up: bool,
    /// 系统真实图标（fm_system_icons）：列表行 16pt / 网格非图片 cell
    /// 32pt 档经 SHGetFileInfoW 取 Shell 图标；false = 字体图标现状。
    pub system_icons: bool,
    /// 过滤框渲染在焦点栏列表底部（fm_filter_bar_bottom）；false = 顶栏右侧。
    pub filter_bar_bottom: bool,
    /// 鼠标框选模式（fm_rubber_band）。
    pub rubber_band: RubberBandMode,
    /// 底部命令行输入条（fm_command_bar，阶段 X）。
    pub command_bar: bool,
    /// 文件操作并发上限（fm_op_threads，阶段 AA）：0 = 不限。
    pub op_threads: usize,
    /// 递归 FS watch（fm_watch_recursive，阶段 Z）：true 时分支视图下
    /// 子目录变化也触发刷新（大目录有性能取舍）。
    pub watch_recursive: bool,
}

impl Default for FmBehaviorOptions {
    /// 默认值 = 一期现状行为（与 settings 侧 serde default 一致）。
    fn default() -> Self {
        Self {
            confirm_delete: true,
            show_hidden: true,
            delete_permanent: false,
            space_toggle_select: false,
            dirs_first: true,
            drag_confirm: true,
            archive_open: FmArchiveOpen::Archive,
            esc_keep_selection: false,
            dblclick_blank_up: true,
            system_icons: true,
            filter_bar_bottom: false,
            rubber_band: RubberBandMode::Right,
            command_bar: true,
            watch_recursive: false,
            op_threads: 2,
        }
    }
}

/// 文件管理器持久化快照（app 侧 diff 后写回 settings.fm_*）。
#[derive(Debug, Clone, PartialEq)]
pub struct FmStateSnapshot {
    /// "dual" | "single"。
    pub layout: String,
    /// 双栏左栏宽占比（Single 期间取切单栏前保存的比例）。
    pub ratio: f32,
    /// 单栏预览面板开关（Dual 期间取切双栏前保存的开关）。
    pub preview_open: bool,
    /// 排序键 "name"|"size"|"mtime"|"ext"|"unsorted"|"attr"：取活动栏
    /// （取舍：双栏各自排序可能不同，settings 只有单值，持久化活动栏的排序）。
    pub sort_key: String,
    pub sort_asc: bool,
    /// 视图模式 "list"|"brief"|"thumbs"：同 sort_key 先例取活动栏（全局单值，
    /// 双栏各自模式可能不同，持久化活动栏的）。
    pub view_mode: String,
    /// 两栏当前目录（字符串；空 = 用户主目录，跟随 resolve_fm_dir 语义）。
    pub dir_left: String,
    pub dir_right: String,
    /// 常用目录书签分组（两栏共享；空分组保留）。
    pub bookmark_groups: Vec<FmBookmarkGroup>,
    /// 保存的过滤方案（两栏共享；过滤框下拉「保存当前过滤」追加）。
    pub saved_filters: Vec<String>,
    /// 两栏标签页目录（活动标签 = 实时目录；只存目录路径，选中/焦点/过滤/
    /// 滚动不持久化）与活动标签索引。旧 settings 无此数据时恢复端回退
    /// dir_left/dir_right 的单标签行为。
    pub tabs_left: Vec<String>,
    pub tabs_right: Vec<String>,
    pub active_tab_left: usize,
    pub active_tab_right: usize,
    /// 大小/时间列宽与列块平移量（≤0，0 = 列块贴右缘）：全局单值取活动栏
    /// （取舍同 sort_key——双栏各自拖过会不同，持久化活动栏的，恢复时
    /// 两栏同用）。阶段 V 起为旧字段兼容层：权威列配置在 `columns`，
    /// Size/Mtime 列存在时取其宽，否则保留旧值。
    pub col_size_width: f32,
    pub col_mtime_width: f32,
    pub col_shift: f32,
    /// 明细列表列配置（阶段 V）：取活动栏（取舍同 sort_key），app 侧序列化
    /// 为 settings.fm_columns，恢复时两栏同用。
    pub columns: Vec<(ColumnKind, f32)>,
    /// 自定义按钮栏（阶段 Y；两栏共享，权威在 settings.fm_button_bar）。
    pub button_bar: Vec<FmButton>,
}

pub struct FileManagerView {
    pub layout: PanelLayout,
    pub panels: [FsPanel; 2],
    /// 焦点栏（0/1）：键盘操作目标；鼠标点击某栏任意处即激活。
    pub active: usize,
    /// 切回双栏时恢复的比例（Single 期间 Dual.ratio 不可见）。
    saved_ratio: f32,
    /// 切回单栏时恢复的预览开关（Dual 期间 Single.preview_open 不可见）。
    saved_preview_open: bool,
    /// 单栏模式预览面板宽度占比（拖动分隔条可调）。
    preview_ratio: f32,
    /// 当前预览目标文件（「选中即预览」：焦点行落到的文件）。
    preview_path: Option<PathBuf>,
    /// 预览目标的条目快照（元信息占位用；目录列举刷新后可能已失效）。
    preview_entry: Option<FsEntry>,
    /// 在途的后台预览读取。
    preview: Option<AsyncOpener<PreviewOutcome>>,
    /// poll 拿到、待 ui() 上传为纹理的图片。
    pending_preview_image: Option<egui::ColorImage>,
    preview_tex: Option<egui::TextureHandle>,
    preview_text: Option<String>,
    preview_note: Option<String>,
    /// 预览原始字节（阶段 R：HEX 查看与编码手动重解码用；读取成功的
    /// 图片/文本/二进制均有，超上限未读为 None）。
    preview_bytes: Option<std::sync::Arc<[u8]>>,
    /// 预览模式（阶段 R：按内容类型自动解析初值，用户可经 tab 手切）。
    preview_mode: PreviewMode,
    /// 文本编码手动选择（Auto = 自动检测；换新目标回 Auto）。
    preview_encoding: PreviewEncoding,
    /// 图片显示层旋转（0..=3 = 0/90/180/270°；不写文件）。
    preview_rotation: u8,
    /// 文本内搜索：输入框内容 / 匹配行号 / 当前匹配序号 / 待揭示行。
    preview_search: String,
    preview_search_matches: Vec<usize>,
    preview_search_cur: usize,
    preview_search_reveal: Option<usize>,
    /// 图片预览「原始尺寸」模式（false = 适应宽度）。
    preview_full_size: bool,
    /// F3 临时预览弹窗开关（双栏模式）。
    preview_window_open: bool,
    /// F3 弹窗最大化（铺满 CentralPanel 区域；egui Window 不支持自定义
    /// 标题栏按钮，最大化态改走全屏 Area 自绘标题行）。
    preview_window_maximized: bool,
    /// 后台文件操作（复制/移动/删除）。
    ops: FileOpManager,
    /// 操作确认对话框；Some 时渲染模态窗口并屏蔽面板键盘。
    dialog: Option<FmDialog>,
    /// 应用内剪贴板（Ctrl+C/X 复制/剪切，Ctrl+V 粘贴到焦点栏）。
    clipboard: Vec<PathBuf>,
    clipboard_cut: bool,
    /// 本次粘贴来自系统剪贴板且带剪切标志：Move 确认后按 Explorer 惯例
    /// 清空系统剪贴板（apply_dialog_outcome 消费并复位）。
    sys_clipboard_cut_pending: bool,
    /// 盘符列表缓存与在途后台枚举（盘符下拉共用；慢速设备不卡 UI）。
    drives: Option<Vec<PathBuf>>,
    drives_rx: Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>,
    /// 行为设置包（阶段 O，含 confirm_delete/show_hidden/delete_permanent
    /// 等；ui() 每帧下发，权威在 settings）。
    options: FmBehaviorOptions,
    /// 双击压缩包 fm_archive_open = "ask" 的待决小菜单（路径 + 弹出位置）。
    archive_ask: Option<(PathBuf, egui::Pos2)>,
    /// 常用目录书签分组（两栏共享，权威走快照写回 settings.fm_bookmark_groups）。
    bookmark_groups: Vec<FmBookmarkGroup>,
    /// 自定义按钮栏（阶段 Y；两栏共享，权威走快照写回 settings.fm_button_bar；
    /// 构造后由 app 经 set_button_bar 喂入，设置页改动经同方法同步休眠视图）。
    button_bar: Vec<FmButton>,
    /// 保存的过滤方案（两栏共享，权威走快照写回 settings.fm_saved_filters；
    /// 构造后经 `set_saved_filters` 注入）。
    saved_filters: Vec<String>,
    /// 过滤会话历史（最近使用在前，去重，上限 8；不落盘）。
    filter_history: Vec<String>,
    /// Ctrl+S 的一次性请求：下一帧 render_filter_bar 聚焦过滤框
    /// （已聚焦则选中全文）。
    filter_focus_request: bool,
    /// 过滤框 Esc（清空 + 交还焦点）已在本帧消费：handle_keyboard 的
    /// Esc 链见到此标记跳过，防同帧双消费。
    filter_esc_handled: bool,
    /// 命令行 Esc（清空 + 交还焦点）已在本帧消费：handle_keyboard 的
    /// Esc 链见此跳过（同 filter_esc_handled 模式）。
    command_esc_handled: bool,
    /// 鼠标框选（阶段 U）进行中的状态（起点/按钮/所属栏/是否已超阈值）；
    /// 当前指针位置每帧从 input 现读。
    band: Option<RubberBand>,
    /// 保存的选择集（会话内，不落盘）：(名称, 路径集)。
    saved_selections: Vec<(String, Vec<PathBuf>)>,
    /// 「保存当前选择…」小对话框（非模态 egui::Window，同 BookmarkGroupDialog
    /// 模式）。
    selection_dialog: Option<SaveSelectionDialog>,
    /// 「编辑注释…」小对话框（阶段 W；非模态 egui::Window，同选择集对话框
    /// 模式）。
    comment_dialog: Option<CommentDialog>,
    /// 「添加/编辑按钮」对话框（阶段 Y 按钮栏；非模态 egui::Window）。
    button_dialog: Option<ButtonDialog>,
    /// 「选择组」对话框上次使用的模式（会话内记忆，不落盘）。
    select_group_pattern: String,
    /// Alt+↓ 的一次性请求：下一帧焦点栏的历史下拉菜单开/关切换
    /// （弹层开关状态在 egui memory，键盘段无法直接触达）。
    history_menu_toggle: bool,
    /// Ctrl+D 的一次性请求：下一帧焦点栏的书签菜单开/关切换（同
    /// history_menu_toggle 机制）。
    bookmarks_menu_toggle: bool,
    /// 命令行输入条（阶段 X）：输入文本 + 会话内历史（去重置顶，上限
    /// `COMMAND_HISTORY_CAP`）。
    command_input: String,
    command_history: Vec<String>,
    /// ↑↓ 历史导航位置（None = 未在导航；0 = 最新一条）。
    command_history_pos: Option<usize>,
    /// Ctrl+P / Ctrl+Enter 的一次性聚焦请求（命令行未聚焦时置位，下一帧
    /// render_command_bar 消费 request_focus）。
    command_focus_request: bool,
    /// 状态栏速度/ETA 估算器（任务 id + EMA 采样器；任务切换重置）。
    op_speed: Option<(u64, OpSpeedMeter)>,
    /// 状态栏驱动器剩余空间缓存（卷 key + 字节 + 查询时刻；30s TTL，
    /// 失败结果同样缓存——None 不每帧重查）。切卷（key 变化）立即重查。
    drive_free: Option<(String, Option<u64>, Instant)>,
    /// Ctrl+Q 对面栏快速预览（双栏；会话内状态，不落盘）：开启时非活动栏
    /// 整栏替换为预览面板，目标 = 活动栏焦点文件，焦点移动跟随。
    quickview_open: bool,
    /// 缩略图缓存（两栏共享；FileManagerView 级持有——纹理与后台 worker
    /// 不随栏/标签切换重建）。
    thumbs: ThumbCache,
    /// 各栏网格上一帧可见范围（cols, 首网格行, 末网格行）：变化即 bump
    /// 缩略图请求代次，worker 丢弃过期请求（快速滚动不解码不可见 cell）。
    thumb_visible: [Option<(usize, usize, usize)>; 2],
    /// 系统真实图标缓存（两栏共享，与 thumbs 同生命周期/同代次模型）。
    sys_icons: SysIconCache,
    /// 外部拖入的落点区域（render_panels 每帧记录；快览替换栏为 None）。
    panel_drop_rects: [Option<egui::Rect>; 2],
    /// 栏内拖放（FmDragPayload）的面包屑段/标签落点 rect（阶段 Z）：
    /// render_breadcrumb / render_tab_bar 每帧重记录（ui() 帧首清空），
    /// poll_inter_panel_dnd 判定悬停高亮与松开复制；(目标目录, rect)。
    breadcrumb_drop_rects: Vec<(PathBuf, egui::Rect)>,
    tab_drop_rects: Vec<(PathBuf, egui::Rect)>,
    /// 文件搜索对话框（Alt+F7；非模态 egui::Window，worker 关闭即取消）。
    search: SearchDialog,
    /// 校验和对话框（阶段 AC；非模态 egui::Window，worker 关闭即取消）。
    checksum: ChecksumDialog,
    /// 同步目录对话框（阶段 AE；非模态 egui::Window，对比 worker 关闭即取消）。
    sync_dialog: SyncDialog,
    /// 「修改属性/时间戳…」对话框（阶段 AC；comment_dialog 同款非模态模式）。
    attr_dialog: Option<AttrTimestampDialog>,
    /// 书签分组小对话框（新建/重命名；非模态 egui::Window，同 SelectGroupDialog
    /// 模式——菜单内联输入与 CloseOnClickOutside 焦点冲突，故走独立窗口）。
    group_dialog: Option<BookmarkGroupDialog>,
    /// 执行期冲突问答（ConflictMode::Ask「逐个询问」）：待答的 worker 询问
    /// + 弹窗状态；Some 时屏蔽面板键盘（同 self.dialog 机制）。
    pending_conflict: Option<(u64, ConflictDialog)>,
    /// 面包屑路径编辑（铅笔按钮）：正在编辑的栏 idx；None = 未编辑。
    breadcrumb_edit: Option<usize>,
    /// 编辑中文本（进入时预填当前目录完整路径）。
    breadcrumb_edit_text: String,
    /// 首帧 request_focus 一次性标志（同 SelectGroupDialog 模式）。
    breadcrumb_edit_focused: bool,
    /// Enter 后路径不存在：红字提示并保持编辑态（文本变化即清）。
    breadcrumb_edit_error: bool,
    /// 任务面板窗口开关（阶段 AA；非模态 egui::Window，列出在途+排队任务）。
    task_panel_open: bool,
    /// 最近一次含失败项的操作报告（阶段 AA 错误汇总窗；None = 无/已关闭）。
    op_error_report: Option<OpErrorReport>,
}

/// 错误汇总窗数据（阶段 AA）：含失败项的操作结束时从 FinishedOp 留存，
/// 「重试失败项」据此重建同参数任务（Copy/Move 用原 dest_dir 与冲突策略，
/// Delete 用原 permanent 档，Compress 用原 dest_zip）。
struct OpErrorReport {
    kind: OpKind,
    /// 工作线程目标（Compress = dest_zip；Copy/Move = dest_dir）。
    dest: Option<PathBuf>,
    dest_dir: Option<PathBuf>,
    conflict: ConflictMode,
    delete_permanent: bool,
    errors: Vec<(PathBuf, String)>,
}

/// 书签分组小对话框状态（新建/重命名共用；非模态 egui::Window——菜单内联
/// 输入与 CloseOnClickOutside 焦点冲突，故走独立窗口）。
/// 「保存当前选择…」小对话框（阶段 U；复用 BookmarkGroupDialog 的单输入
/// 模式）：打开时捕获选择集快照，确认时以输入名存入 saved_selections。
struct SaveSelectionDialog {
    name: String,
    /// 首帧 request_focus 一次性标志（同 BookmarkGroupDialog 模式）。
    focused: bool,
    /// 打开时的选择集快照（对话框存续期间选择可能变化）。
    paths: Vec<PathBuf>,
}

impl SaveSelectionDialog {
    fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            name: String::new(),
            focused: false,
            paths,
        }
    }
}

/// 「编辑注释…」对话框状态（阶段 W）：写回目标 = 所属栏当前目录 +
/// 文件名（descript.ion 按名匹配）。
struct CommentDialog {
    /// 所属栏（写回后 reload_comments）。
    panel: usize,
    /// 条目文件名（descript.ion 的 key）。
    name: String,
    /// 编辑中文本（预填现注释；空 = 删除该条）。
    text: String,
    /// 首帧 request_focus 一次性标志（同 SaveSelectionDialog 模式）。
    focused: bool,
    /// 上一次写回失败的错误（对话框保持打开并显示）。
    error: Option<String>,
}

impl CommentDialog {
    fn new(panel: usize, name: String, existing: Option<&str>) -> Self {
        Self {
            panel,
            name,
            text: existing.unwrap_or_default().to_string(),
            focused: false,
            error: None,
        }
    }
}

struct BookmarkGroupDialog {
    /// None = 新建分组；Some(i) = 重命名第 i 组。
    rename: Option<usize>,
    name: String,
    /// 首帧 request_focus 一次性标志（同 SelectGroupDialog 模式）。
    focused: bool,
}

impl BookmarkGroupDialog {
    fn new_create() -> Self {
        Self {
            rename: None,
            name: String::new(),
            focused: false,
        }
    }

    fn new_rename(group: usize, current: &str) -> Self {
        Self {
            rename: Some(group),
            name: current.to_string(),
            focused: false,
        }
    }
}

/// 「添加/编辑按钮」对话框状态（阶段 Y 按钮栏；非模态 egui::Window，同
/// BookmarkGroupDialog 模式扩展为多字段）。
struct ButtonDialog {
    /// None = 添加；Some(i) = 编辑 button_bar 第 i 个。
    edit: Option<usize>,
    label: String,
    command: String,
    tooltip: String,
    /// 首帧 request_focus 一次性标志（聚焦按钮文字输入框）。
    focused: bool,
}

impl ButtonDialog {
    fn new_create() -> Self {
        Self {
            edit: None,
            label: String::new(),
            command: String::new(),
            tooltip: String::new(),
            focused: false,
        }
    }

    fn new_edit(index: usize, button: &FmButton) -> Self {
        Self {
            edit: Some(index),
            label: button.label.clone(),
            command: button.command.clone(),
            tooltip: button.tooltip.clone(),
            focused: false,
        }
    }
}

/// 帧内意图：行内交互写入，帧尾统一触发回调（避免回调嵌套借用）。
#[derive(Default)]
struct FmIntents {
    back: bool,
    open_path: Option<PathBuf>,
    open_archive: Option<PathBuf>,
    open_as_comic: Option<PathBuf>,
    /// fm_archive_open = "ask"：双击压缩包待弹选择菜单（帧尾转为
    /// self.archive_ask 状态，视图内部消费）。
    archive_ask: Option<PathBuf>,
    /// 右键「解压到另一栏/当前目录…」：(压缩包路径, 目标目录)。
    extract: Option<(PathBuf, PathBuf)>,
    /// 文件操作汇总/错误（完成/取消/失败时上报给 app error_message）。
    op_error: Option<String>,
    /// 删除确认框「不再询问」勾选（false = 仍需确认）。
    confirm_delete_change: Option<bool>,
}

pub struct FmCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    /// 打开分发（图片/电子书/媒体/漫画文件夹等，走 app 的 open_path）。
    pub on_open_path: &'a mut dyn FnMut(PathBuf),
    /// 双击压缩包：进 Archive 视图（open_archive_browser）。
    pub on_open_archive: &'a mut dyn FnMut(PathBuf),
    /// 右键「作为漫画打开」（目录与压缩包可用）。
    pub on_open_as_comic: &'a mut dyn FnMut(PathBuf),
    /// 右键「解压到…」：(压缩包路径, 目标目录)；app 侧弹解压对话框。
    pub on_extract: &'a mut dyn FnMut(PathBuf, PathBuf),
    /// 文件操作完成/取消/失败的汇总消息。
    pub on_op_error: &'a mut dyn FnMut(String),
    /// 删除确认框「不再询问」勾选变化（写回 settings.fm_confirm_delete）。
    pub on_confirm_delete_change: &'a mut dyn FnMut(bool),
}

/// 明细列表各列的 x 坐标单一来源（表头 paint、行列分隔竖线、列宽拖拽
/// 共用），消除各自手算的漂移。名称列（恒首列）宽 = 剩余弹性；固定列
/// 从行右缘往左依次排列（阶段 V：列集合由 FsPanel::columns 驱动）。
#[derive(Debug, Clone, PartialEq)]
struct ColumnLayout {
    /// 名称列右缘（= 名称|首个固定列分隔竖线 x、最左固定列左缘）。
    name_right: f32,
    /// 固定列（从左到右）：(列类型, 列左缘, 文字右锚点)；文字右锚点 =
    /// 列右缘（最右列 = 行右缘内 COL_RIGHT_PAD 处，含 col_shift）。
    fixed: Vec<(ColumnKind, f32, f32)>,
}

/// 由行/表头 rect 的右缘、列块平移量与列配置算出各列坐标。
/// shift ≤ 0：0 = 列块贴右缘（名称列吃满剩余宽度）；<0 = 列块整体左移。
/// columns[0]（Name）的宽度忽略；columns[1..] 从右往左排——最右列锚定
/// `right - COL_RIGHT_PAD + shift`，每列文字右锚点 = 列右缘。
fn column_layout(right: f32, shift: f32, columns: &[(ColumnKind, f32)]) -> ColumnLayout {
    let mut r = right - COL_RIGHT_PAD + shift;
    let mut fixed: Vec<(ColumnKind, f32, f32)> = Vec::with_capacity(columns.len().max(1) - 1);
    for (kind, w) in columns.iter().skip(1).rev() {
        let text_right = r;
        let left = r - w;
        fixed.push((*kind, left, text_right));
        r = left;
    }
    fixed.reverse();
    ColumnLayout {
        name_right: r,
        fixed,
    }
}

/// 行拖出窗口的触发阈值（pt）：位移超过该值且指针出窗才交 OLE
/// DoDragDrop，避免手滑即触发模态拖放（同 Archive 拖出阈值语义）。
const DRAG_OUT_THRESHOLD: f32 = 40.0;

/// 过滤会话历史上限（最近 8 条）。
const FILTER_HISTORY_CAP: usize = 8;

/// 命令行会话历史上限（去重置顶，阶段 X）。
const COMMAND_HISTORY_CAP: usize = 32;

/// 过滤会话历史维护（纯函数）：trim 后为空忽略；去重后置顶，超出 cap
/// 截断尾部。
fn push_history_capped(history: &mut Vec<String>, item: &str, cap: usize) {
    let item = item.trim();
    if item.is_empty() {
        return;
    }
    if let Some(pos) = history.iter().position(|h| h == item) {
        history.remove(pos);
    }
    history.insert(0, item.to_string());
    history.truncate(cap);
}

/// 框选开始阈值（pt）：按下后位移超过该值才进入框选（未超 = 单击语义，
/// 右键维持弹上下文菜单）。
const BAND_THRESHOLD: f32 = 6.0;

/// 鼠标框选进行中状态（阶段 U）。
struct RubberBand {
    /// 所属栏。
    panel: usize,
    /// 触发按钮：Secondary（"right" 模式）/ Primary（"left" 模式，空白起拖）。
    button: egui::PointerButton,
    /// 起点（屏幕坐标）。
    origin: egui::Pos2,
    /// 位移已超 BAND_THRESHOLD（false = 仍是候选，松开不产生框选）。
    active: bool,
}

/// 列表框选命中（纯函数）：rect 为内容坐标（content_y = 指针 y - 视口顶 +
/// 滚动偏移；x 忽略——明细行整行占满宽度）；返回与矩形竖向重叠的 UI 行
/// 索引（含 0 = 「..」行，调用方自行跳过）。
fn rows_in_rect(row_count: usize, pitch: f32, rect: egui::Rect) -> Vec<usize> {
    if row_count == 0 || pitch <= 0.0 {
        return Vec::new();
    }
    let top = rect.min.y.max(0.0);
    // 底边恰好压在行界上时不算命中下一行（减 eps）。
    let bottom = (rect.max.y - 0.01).max(top);
    let content_bottom = row_count as f32 * pitch;
    if top >= content_bottom {
        return Vec::new();
    }
    let first = (top / pitch).floor() as usize;
    let last = ((bottom / pitch).floor() as usize).min(row_count - 1);
    if last < first {
        return Vec::new();
    }
    (first..=last).collect()
}

/// 网格框选命中（纯函数）：rect 内容坐标；返回线性 item 索引（含 0 =
/// 「..」cell），末行未排满与行右空白自动丢弃（i ≥ item_count 或
/// 列 ≥ cols 不进结果）。
fn cells_in_rect(
    item_count: usize,
    cols: usize,
    cell_w: f32,
    cell_h: f32,
    rect: egui::Rect,
) -> Vec<usize> {
    let mut out = Vec::new();
    let cols = cols.max(1);
    if item_count == 0 || cell_w <= 0.0 || cell_h <= 0.0 {
        return out;
    }
    let r0 = (rect.min.y / cell_h).floor().max(0.0) as usize;
    // 底/右边恰好压界时不算命中下一行/列（减 eps）。
    let r1 = ((rect.max.y - 0.01).max(rect.min.y) / cell_h)
        .floor()
        .max(0.0) as usize;
    let c0 = (rect.min.x / cell_w).floor().max(0.0) as usize;
    let c1 = (((rect.max.x - 0.01).max(rect.min.x) / cell_w)
        .floor()
        .max(0.0) as usize)
        .min(cols - 1);
    if c0 >= cols {
        return out;
    }
    for gr in r0..=r1 {
        for c in c0..=c1 {
            let i = gr * cols + c;
            if i < item_count {
                out.push(i);
            }
        }
    }
    out
}

/// 名称显示宽度估算（char-unit：ASCII = 1，其余 = 2；与
/// truncate_cell_name 同一口径）。简表列宽估算用。
fn name_units(name: &str) -> usize {
    name.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// 简表列宽（pt，阶段 V）：最长名称估算宽度（7pt/unit）+ 28pt
/// （6pt 缩进 + 16pt 图标 + 6pt 间距），clamp 120..=300。
fn brief_col_width(max_units: usize) -> f32 {
    (max_units as f32 * 7.0 + 28.0).clamp(120.0, 300.0)
}

/// 简表列数：栏宽 / 列宽，至少 1 列。
fn brief_cols(available_width: f32, cell_w: f32) -> usize {
    (available_width / cell_w).floor().max(1.0) as usize
}

/// 简表 cell 名称单行截断：char-unit 预算 = (cell_w - 30) / 7（30pt =
/// 缩进+图标+间距+右留白），超预算尾部替换为「…」（同
/// truncate_cell_name 的截断风格）。
fn brief_cell_name(name: &str, cell_w: f32) -> String {
    let budget = ((cell_w - 30.0) / 7.0).floor().max(4.0) as usize;
    let mut units = 0;
    for (i, c) in name.chars().enumerate() {
        let w = if c.is_ascii() { 1 } else { 2 };
        if units + w > budget {
            let mut s: String = name.chars().take(i.saturating_sub(1)).collect();
            s.push('…');
            return s;
        }
        units += w;
    }
    name.to_string()
}

/// 栏间拖放复制的 payload：行 drag source 设置，经 egui 全局 dnd 状态
/// 跨栏传递（payload 与 widget Id 无关，栏间 push_id 隔离不影响）。
#[derive(Debug, Clone)]
struct FmDragPayload {
    sources: Vec<PathBuf>,
    src_panel: usize,
}

/// 行拖拽的源集合（Explorer 惯例）：被拖行已在选中集内 → 整个选中集
/// （排序保证确定性），否则仅被拖行自身。「..」上级行不可拖（调用处保证）。
fn drag_sources(selected: &HashSet<PathBuf>, row_path: &Path) -> Vec<PathBuf> {
    if selected.contains(row_path) {
        let mut sources: Vec<PathBuf> = selected.iter().cloned().collect();
        sources.sort();
        sources
    } else {
        vec![row_path.to_path_buf()]
    }
}

/// 悬停信息提示（明细行与网格/简表 cell 共用）：全路径 + 大小，有注释
/// （阶段 W descript.ion）时追加注释行；「..」= 上级目录 / 分支模式 =
/// 退出分支视图提示。
fn row_hover_tip(entry: Option<&FsEntry>, branch: bool, comment: Option<&str>) -> String {
    match entry {
        None => {
            if branch {
                "退出分支视图".to_string()
            } else {
                "上级目录".to_string()
            }
        }
        Some(e) => {
            let mut tip = e.path.display().to_string();
            if !e.is_dir {
                if let Some(size) = e.size {
                    tip.push_str(&format!("\n大小: {}", human_size(size)));
                }
            }
            if let Some(comment) = comment {
                if !comment.is_empty() {
                    tip.push_str(&format!("\n注释: {comment}"));
                }
            }
            tip
        }
    }
}

/// 标签条标题：目录 basename；根目录（如 `C:\`）无 basename 时显示完整
/// 路径。超 20 字符截断 + 省略号（完整路径走悬停 tooltip）。
fn tab_label(dir: &Path) -> String {
    const MAX_CHARS: usize = 20;
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.display().to_string());
    if name.chars().count() > MAX_CHARS {
        let truncated: String = name.chars().take(MAX_CHARS - 1).collect();
        format!("{truncated}…")
    } else {
        name
    }
}

impl FileManagerView {
    /// 从 settings 恢复布局/排序（app 构造时调用）。
    pub fn new(
        layout: &str,
        ratio: f32,
        preview_open: bool,
        sort_key: &str,
        sort_asc: bool,
        bookmark_groups: &[FmBookmarkGroup],
    ) -> Self {
        let layout = if layout == "single" {
            PanelLayout::Single { preview_open }
        } else {
            PanelLayout::Dual { ratio }
        };
        let sort = match sort_key {
            "size" => SortKey::Size,
            "mtime" => SortKey::Mtime,
            "ext" => SortKey::Ext,
            "unsorted" => SortKey::Unsorted,
            "attr" => SortKey::Attr,
            _ => SortKey::Name,
        };
        Self {
            layout,
            panels: [FsPanel::new(sort, sort_asc), FsPanel::new(sort, sort_asc)],
            active: 0,
            saved_ratio: ratio,
            saved_preview_open: preview_open,
            preview_ratio: 0.35,
            preview_path: None,
            preview_entry: None,
            preview: None,
            pending_preview_image: None,
            preview_tex: None,
            preview_text: None,
            preview_note: None,
            preview_bytes: None,
            preview_mode: PreviewMode::Hex,
            preview_encoding: PreviewEncoding::Auto,
            preview_rotation: 0,
            preview_search: String::new(),
            preview_search_matches: Vec::new(),
            preview_search_cur: 0,
            preview_search_reveal: None,
            preview_full_size: false,
            preview_window_open: false,
            preview_window_maximized: false,
            ops: FileOpManager::default(),
            dialog: None,
            clipboard: Vec::new(),
            clipboard_cut: false,
            sys_clipboard_cut_pending: false,
            drives: None,
            drives_rx: None,
            options: FmBehaviorOptions::default(),
            archive_ask: None,
            bookmark_groups: bookmark_groups.to_vec(),
            button_bar: Vec::new(),
            saved_filters: Vec::new(),
            filter_history: Vec::new(),
            filter_focus_request: false,
            filter_esc_handled: false,
            command_esc_handled: false,
            band: None,
            saved_selections: Vec::new(),
            selection_dialog: None,
            comment_dialog: None,
            button_dialog: None,
            select_group_pattern: String::new(),
            history_menu_toggle: false,
            bookmarks_menu_toggle: false,
            command_input: String::new(),
            command_history: Vec::new(),
            command_history_pos: None,
            command_focus_request: false,
            op_speed: None,
            drive_free: None,
            quickview_open: false,
            thumbs: ThumbCache::new(),
            thumb_visible: [None, None],
            sys_icons: SysIconCache::new(),
            panel_drop_rects: [None, None],
            breadcrumb_drop_rects: Vec::new(),
            tab_drop_rects: Vec::new(),
            search: SearchDialog::default(),
            checksum: ChecksumDialog::default(),
            sync_dialog: SyncDialog::default(),
            attr_dialog: None,
            group_dialog: None,
            pending_conflict: None,
            task_panel_open: false,
            op_error_report: None,
            breadcrumb_edit: None,
            breadcrumb_edit_text: String::new(),
            breadcrumb_edit_focused: false,
            breadcrumb_edit_error: false,
        }
    }

    /// 采集当前状态快照（持久化写回用）。目录取面板当前 dir；
    /// 排序取活动栏（settings 只有单值，见 FmStateSnapshot.sort_key 注释）。
    pub fn snapshot(&self) -> FmStateSnapshot {
        let (layout, ratio, preview_open) = match &self.layout {
            PanelLayout::Dual { ratio } => ("dual", *ratio, self.saved_preview_open),
            PanelLayout::Single { preview_open } => ("single", self.saved_ratio, *preview_open),
        };
        let panel = &self.panels[self.active];
        let sort_key = match panel.sort_key {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Mtime => "mtime",
            SortKey::Ext => "ext",
            SortKey::Unsorted => "unsorted",
            SortKey::Attr => "attr",
        };
        // 旧字段兼容层：Size/Mtime 列存在时取其宽（列被删除则回退默认
        // 宽度，不影响新权威字段 columns）。
        let col_size_width = panel
            .column_width(ColumnKind::Size)
            .unwrap_or_else(|| ColumnKind::Size.default_width());
        let col_mtime_width = panel
            .column_width(ColumnKind::Mtime)
            .unwrap_or_else(|| ColumnKind::Mtime.default_width());
        FmStateSnapshot {
            layout: layout.to_string(),
            ratio,
            preview_open,
            sort_key: sort_key.to_string(),
            sort_asc: panel.sort_asc,
            view_mode: panel.view_mode.as_setting().to_string(),
            dir_left: self.panels[0].dir.display().to_string(),
            dir_right: self.panels[1].dir.display().to_string(),
            bookmark_groups: self.bookmark_groups.clone(),
            saved_filters: self.saved_filters.clone(),
            tabs_left: self.panel_tab_dirs(0),
            tabs_right: self.panel_tab_dirs(1),
            active_tab_left: self.panels[0].active_tab(),
            active_tab_right: self.panels[1].active_tab(),
            col_size_width,
            col_mtime_width,
            col_shift: panel.col_shift,
            columns: panel.columns.clone(),
            button_bar: self.button_bar.clone(),
        }
    }

    /// 栏内全部标签的目录字符串（活动标签 = 实时 dir）。
    fn panel_tab_dirs(&self, idx: usize) -> Vec<String> {
        (0..self.panels[idx].tab_count())
            .map(|i| self.panels[idx].tab_dir(i).display().to_string())
            .collect()
    }

    /// 注入保存的过滤方案（构造后由 app 从 settings 喂入）。
    pub fn set_saved_filters(&mut self, filters: &[String]) {
        self.saved_filters = filters.to_vec();
    }

    /// 注入/同步按钮栏（阶段 Y）：构造后由 app 从 settings 喂入；设置页编辑
    /// 时也经本方法同步到休眠视图（同 apply_layout_settings 先例——否则
    /// 下次进入 FM 时 maybe_save_fm_state 会把旧副本写回覆盖设置页改动）。
    pub fn set_button_bar(&mut self, buttons: &[FmButton]) {
        self.button_bar = buttons.to_vec();
    }

    /// 恢复列配置（阶段 V）：全局单值，两栏同用（同 fm_sort_key 先例）。
    /// 调用方负责解析/规范化（panel.rs parse_columns 或 legacy 播种）。
    pub fn set_columns(&mut self, columns: &[(ColumnKind, f32)]) {
        for panel in &mut self.panels {
            panel.columns = columns.to_vec();
        }
    }

    /// 记录过滤串进会话历史（trim 后为空忽略）。
    fn record_filter_history(&mut self, filter: &str) {
        push_history_capped(&mut self.filter_history, filter, FILTER_HISTORY_CAP);
    }

    /// 添加书签到指定分组（两栏共享）；组内已有时 no-op 返回 false。
    pub fn add_bookmark_to_group(&mut self, group: usize, dir: &Path) -> bool {
        let Some(g) = self.bookmark_groups.get_mut(group) else {
            return false;
        };
        let item = dir.display().to_string();
        if g.items.iter().any(|b| Path::new(b) == dir) {
            return false;
        }
        g.items.push(item);
        true
    }

    /// 移除指定分组内的书签；不在组内返回 false。
    pub fn remove_bookmark(&mut self, group: usize, dir: &Path) -> bool {
        let Some(g) = self.bookmark_groups.get_mut(group) else {
            return false;
        };
        let before = g.items.len();
        g.items.retain(|b| Path::new(b) != dir);
        g.items.len() != before
    }

    /// 新建分组；空名/重名 no-op 返回 false。
    pub fn add_group(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.bookmark_groups.iter().any(|g| g.name == name) {
            return false;
        }
        self.bookmark_groups.push(FmBookmarkGroup {
            name: name.to_string(),
            items: Vec::new(),
        });
        true
    }

    /// 重命名分组；空名/与他组重名 no-op 返回 false。
    pub fn rename_group(&mut self, group: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty()
            || self
                .bookmark_groups
                .iter()
                .enumerate()
                .any(|(i, g)| i != group && g.name == name)
        {
            return false;
        }
        let Some(g) = self.bookmark_groups.get_mut(group) else {
            return false;
        };
        g.name = name.to_string();
        true
    }

    /// 删除分组（组内书签一并删除；从简无确认，调用点注释说明）。
    pub fn remove_group(&mut self, group: usize) {
        if group < self.bookmark_groups.len() {
            self.bookmark_groups.remove(group);
        }
    }

    /// 设置页改动默认布局/双栏比例时同步到本视图（此时视图休眠）。
    /// 不同步的话，maybe_save_fm_state 会在下次进入视图时把旧布局写回
    /// settings，覆盖用户在设置页的改动。
    pub fn apply_layout_settings(&mut self, layout: &str, ratio: f32) {
        let ratio = ratio.clamp(RATIO_MIN, RATIO_MAX);
        if layout == "single" {
            if let PanelLayout::Dual { ratio: r } = self.layout {
                self.saved_ratio = r;
            }
            // 单栏只渲染 panels[0]：焦点栏内容保留到左栏（同顶栏切换逻辑）。
            if self.active == 1 {
                self.panels.swap(0, 1);
                self.active = 0;
            }
            self.preview_window_open = false;
            // 单栏无对面栏概念，快览关闭。
            self.quickview_open = false;
            self.layout = PanelLayout::Single {
                preview_open: self.saved_preview_open,
            };
        } else {
            self.saved_ratio = ratio;
            self.layout = PanelLayout::Dual { ratio };
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        callbacks: FmCallbacks<'_>,
        options: FmBehaviorOptions,
    ) {
        self.options = options;
        // show_hidden/dirs_first 权威在 settings（同 confirm_delete），每帧下发；
        // 休眠期间设置页改动在下次进入时经此同步，rows_cache 按
        // RowsKey 自动失效，无需重新 read_dir。
        for panel in &mut self.panels {
            panel.show_hidden = options.show_hidden;
            panel.dirs_first = options.dirs_first;
            // 递归 watch 选项（阶段 Z）：每帧下发，ensure_watch 按
            // (路径, recursive) 跳过/重建。
            panel.watch_recursive = options.watch_recursive;
            // FS watch 回调唤醒 UI 用（首帧注入，后续 no-op）。
            panel.set_wake_ctx(ui.ctx().clone());
        }
        // 文件操作并发上限（阶段 AA，fm_op_threads）：每帧下发，在途任务
        // 不动，调高立即放行排队任务。
        self.ops.set_max_concurrent(options.op_threads);
        let FmCallbacks {
            on_back,
            on_open_path,
            on_open_archive,
            on_open_as_comic,
            on_extract,
            on_op_error,
            on_confirm_delete_change,
        } = callbacks;
        let mut intents = FmIntents::default();

        // 每帧排空两栏的列举结果；Loading 期间主动重绘（egui 空闲不重绘）。
        let mut loading = false;
        let mut drop_preview = false;
        for panel in &mut self.panels {
            let was_loading = matches!(panel.state, PanelLoadState::Loading(_));
            loading |= panel.poll();
            // 目录大小计算在途：主动重绘排空结果（egui 空闲不重绘）。
            loading |= panel.dir_sizes_in_flight();
            // FS watch 事件待去抖：主动重绘推进去抖窗口直到触发 refresh。
            loading |= panel.watch_refresh_pending();
            // 列举完成（navigate/refresh）：预览目标属于该栏且已消失则清预览。
            if was_loading && !matches!(panel.state, PanelLoadState::Loading(_)) {
                if let Some(target) = &self.preview_path {
                    if target.parent() == Some(panel.dir.as_path())
                        && !panel.entries.iter().any(|e| &e.path == target)
                    {
                        drop_preview = true;
                    }
                }
            }
        }
        if drop_preview {
            self.clear_preview();
        }
        if loading {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        self.poll_preview(ui.ctx());
        // 缩略图：排空解码结果；在途期间主动重绘（对齐项目 loader 约定）。
        self.thumbs.poll(ui.ctx());
        if self.thumbs.has_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // 系统图标：同缩略图排空/重绘约定。
        self.sys_icons.poll(ui.ctx());
        if self.sys_icons.has_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // 文件操作：每帧排空进度/完成事件；活动任务期间主动重绘。
        let op_summary = self.ops.poll();
        if op_summary.has_active {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        for finished in op_summary.finished {
            self.on_op_finished(finished, &mut intents);
        }
        // 执行期冲突问答（Ask 模式）：取新询问（一次一窗）；待答询问所属
        // 任务已结束（取消打断等待/异常）时关窗——worker 侧经 cancel 旗标
        // 或 answer_tx 断开兜底按 Cancel 收拢，不会悬挂。
        if self.pending_conflict.is_none() {
            if let Some((id, query)) = self.ops.take_pending_conflict() {
                self.pending_conflict = Some((id, ConflictDialog::new(query)));
            }
        }
        if let Some((id, _)) = &self.pending_conflict {
            if !self.ops.is_active(*id) {
                self.pending_conflict = None;
            }
        }
        let active_op = op_summary.active;
        // 速度/ETA 采样（任务切换重置采样器；暂停期间 done 不变，
        // EMA 自然衰减归零、速率显示消失）。采样间隔由 meter 内部节流。
        match (&active_op, &mut self.op_speed) {
            (Some(op), Some((id, meter))) if *id == op.id => {
                meter.sample(Instant::now(), op.progress.done_bytes);
            }
            (Some(op), _) => {
                let mut meter = OpSpeedMeter::new();
                meter.sample(Instant::now(), op.progress.done_bytes);
                self.op_speed = Some((op.id, meter));
            }
            (None, _) => self.op_speed = None,
        }

        self.render_top_bar(ui, &mut intents);
        // 自定义按钮栏（阶段 Y）：顶栏下方一条。
        self.render_button_bar(ui, &mut intents);
        ui.separator();

        // 底栏：当前栏选中/条目统计 + 操作进度。
        let mut cancel_op: Option<u64> = None;
        let mut toggle_pause: Option<(u64, bool)> = None;
        // 点击进度区打开任务面板（阶段 AA）。
        let mut open_task_panel = false;
        // 进度区在途/排队后缀计数（阶段 AA；+N = 未显示的其余在途）。
        let op_extra_active = self.ops.active_count().saturating_sub(1);
        let op_queued = self.ops.queued_count();
        // 驱动器剩余空间（「剩余 X GB」显示在右侧路径前）：30s 缓存 +
        // 切卷即重查，查询在 panel 借用之前完成（&mut self）。
        let drive_free = self.status_drive_free();
        egui::Panel::bottom("fm_status_bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let panel = &self.panels[self.active];
                let total = panel.entries.len();
                let selected = panel.selected.len();
                ui.label(format!("共 {total} 项"));
                // 分支列举超上限截断提示。
                if panel.listing_truncated {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("结果过多已截断").color(ui.visuals().warn_fg_color),
                    );
                }
                if selected > 0 {
                    // 选中集总大小：文件直接求和；目录仅计入「计算大小」已缓存
                    // 的值，未命中不触发计算（0 字节选中集不显示，避免误导）。
                    let total_size: u64 = panel
                        .entries
                        .iter()
                        .filter(|e| panel.selected.contains(&e.path))
                        .map(|e| {
                            if e.is_dir {
                                panel.dir_sizes.get(&e.path).copied().unwrap_or(0)
                            } else {
                                e.size.unwrap_or(0)
                            }
                        })
                        .sum();
                    ui.separator();
                    let label = if total_size > 0 {
                        format!("已选 {selected} 项 · 合计 {}", human_size(total_size))
                    } else {
                        format!("已选 {selected} 项")
                    };
                    ui.label(label);
                }
                // type-ahead 缓冲（type-to-select）：弱色显示，到期自动消失
                // （主动重绘推进过期判定）。
                if let Some(buf) = panel.type_ahead_buffer() {
                    ui.separator();
                    ui.label(egui::RichText::new(format!("定位: {buf}")).weak());
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                // 过滤激活指示：与「定位:」同区域并列（过滤框在顶栏，此处给
                // 近处反馈）。
                if !panel.filter.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(format!("过滤: {}", panel.filter)).weak());
                }
                // 操作进行中提示文本让位（状态栏宽度有限）。
                if active_op.is_none() {
                    ui.separator();
                    ui.label(egui::RichText::new("Tab 切换栏 · 双击打开 · 右键菜单").weak());
                }
                // 活动文件操作：总进度条 + 当前文件进度 + 速度/ETA +
                // 暂停/继续 + 取消（进度区加宽，提示文本相应让位）。
                if let Some(op) = &active_op {
                    ui.separator();
                    // Ask 模式冲突问答在途：worker 阻塞等答，进度暂停推进。
                    if self.pending_conflict.is_some() {
                        ui.label(
                            egui::RichText::new("等待确认…").color(ui.visuals().warn_fg_color),
                        );
                    }
                    let fraction = op.progress.fraction();
                    let pct = (fraction * 100.0).round() as u32;
                    let bar_text = if op.paused {
                        format!("{} {pct}%（已暂停）", op.kind.verb())
                    } else {
                        format!("{} {pct}%", op.kind.verb())
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(200.0)
                            .text(bar_text),
                    )
                    .interact(egui::Sense::click())
                    .on_hover_text("点击打开任务面板")
                    .clicked()
                    .then(|| open_task_panel = true);
                    // 在途/排队后缀（阶段 AA）：只显示第一个在途任务，
                    // 其余经计数提示（点击进度区打开任务面板看全部）。
                    if op_extra_active > 0 || op_queued > 0 {
                        let mut suffix = String::new();
                        if op_extra_active > 0 {
                            suffix.push_str(&format!("+{op_extra_active} 在途"));
                        }
                        if op_queued > 0 {
                            if !suffix.is_empty() {
                                suffix.push(' ');
                            }
                            suffix.push_str(&format!("+{op_queued} 排队"));
                        }
                        ui.label(egui::RichText::new(format!("（{suffix}）")).weak());
                    }
                    // 当前文件内进度（分块复制维护；cur_total=0 不显示）。
                    if op.progress.cur_total_bytes > 0 {
                        let cur_pct = (op.progress.cur_done_bytes as f64
                            / op.progress.cur_total_bytes as f64
                            * 100.0)
                            .round() as u32;
                        ui.label(egui::RichText::new(format!("当前文件 {cur_pct}%")).weak());
                    }
                    // 速度 / ETA（EMA 平滑；速度 0 或 ETA<2s 不显示剩余时间）。
                    if let Some((_, meter)) = &self.op_speed {
                        if let Some(bps) = meter.speed_bps() {
                            if bps > 0.0 {
                                let mut text = format!("{}/s", human_size(bps as u64));
                                if let Some(eta) =
                                    meter.eta_secs(op.progress.total_bytes, op.progress.done_bytes)
                                {
                                    if eta >= 2.0 {
                                        text.push_str(&format!(" · 剩余 {}", format_eta(eta)));
                                    }
                                }
                                ui.label(egui::RichText::new(text).weak());
                            }
                        }
                    }
                    let current = op
                        .progress
                        .current
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !current.is_empty() {
                        ui.label(egui::RichText::new(current).weak())
                            .on_hover_text(op.progress.current.display().to_string());
                    }
                    // 暂停/继续（阶段 AB 起压缩也支持暂停——parser
                    // create_zip 块边界轮询）。
                    let pause_label = if op.paused { "继续" } else { "暂停" };
                    if ui.add(egui::Button::new(pause_label).small()).clicked() {
                        toggle_pause = Some((op.id, !op.paused));
                    }
                    if ui.small_button("取消").clicked() {
                        cancel_op = Some(op.id);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(panel.dir.display().to_string()).weak());
                    // 驱动器剩余空间（路径前；查询失败/不支持不显示）。
                    if let Some(bytes) = drive_free {
                        ui.separator();
                        ui.label(egui::RichText::new(format!("剩余 {}", human_size(bytes))).weak());
                    }
                });
            });
            ui.add_space(4.0);
        });
        // 命令行输入条（阶段 X，fm_command_bar）：状态栏上方。
        self.render_command_bar(ui, &mut intents);
        if let Some(id) = cancel_op {
            self.ops.cancel(id);
        }
        if let Some((id, paused)) = toggle_pause {
            self.ops.set_paused(id, paused);
        }
        if open_task_panel {
            self.task_panel_open = true;
        }

        // 中央：双栏 + 可拖分隔条 / 单栏 + 预览占位。
        // 面包屑段/标签拖放落点 rect（阶段 Z）在 render_panels 内重记录，
        // 帧首清空（同 panel_drop_rects 模式，只是记录点在渲染内部）。
        self.breadcrumb_drop_rects.clear();
        self.tab_drop_rects.clear();
        let panel_rects = self.render_panels(ui, &mut intents);

        // 外部拖入的落点区域（app 侧 handle_dropped_files 经 panel_rect_at
        // 查询；快览替换栏不算落点——此时该栏显示的是预览面板，同下方的
        // 点击激活跳过逻辑）。
        self.panel_drop_rects = [None, None];
        for (idx, rect) in &panel_rects {
            if self.quickview_open && *idx != self.active {
                continue;
            }
            self.panel_drop_rects[*idx] = Some(*rect);
        }

        // 鼠标点击某栏任意处即激活该栏（快览面板不产生栏切换：
        // 快览恒停在「对面」，点击它激活会把两栏语义搞乱）。
        let (pressed, pos) = ui.ctx().input(|i| {
            (
                i.pointer.primary_pressed() || i.pointer.secondary_pressed(),
                i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
            )
        });
        if pressed {
            if let Some(pos) = pos {
                for (idx, rect) in &panel_rects {
                    if self.quickview_open && *idx != self.active {
                        continue;
                    }
                    if rect.contains(pos) {
                        self.active = *idx;
                    }
                }
            }
        }

        // 栏间拖放复制：悬停高亮落点栏 + 松开弹「复制到…」确认框 + 拖动徽标。
        self.poll_inter_panel_dnd(ui, &panel_rects);
        // 行拖出窗口（Windows OLE；文件本就在盘上，直接 do_drag_drop）。
        self.poll_drag_out_external(ui.ctx(), &mut intents);

        self.handle_keyboard(ui, &mut intents);
        // 「选中即预览」跟随焦点行（键盘/鼠标改动焦点之后统一同步）。
        self.sync_preview_target();
        // F3 临时预览弹窗（双栏模式）。
        self.render_preview_window(ui.ctx());
        // 文件操作确认对话框（复制/移动/删除/重命名/新建文件夹）。
        self.render_dialog(ui.ctx(), &mut intents);
        // 执行期冲突问答弹窗（Ask 模式；worker 阻塞等答）。
        self.render_conflict_dialog(ui.ctx());
        // 文件搜索对话框（非模态 egui::Window）。
        self.render_search_dialog(ui.ctx());
        // 校验和 + 属性/时间戳对话框（阶段 AC；非模态 egui::Window）。
        self.checksum.ui(ui.ctx());
        self.render_attr_dialog(ui.ctx());
        // 同步目录对话框（阶段 AE；非模态 egui::Window）。
        self.render_sync_dialog(ui.ctx());
        // 书签分组小对话框（新建/重命名；非模态 egui::Window）。
        self.render_group_dialog(ui.ctx());
        self.render_selection_dialog(ui.ctx());
        self.render_comment_dialog(ui.ctx());
        self.render_button_dialog(ui.ctx());
        // 任务面板 + 错误汇总窗（阶段 AA；非模态 egui::Window）。
        self.render_task_panel(ui.ctx());
        self.render_op_error_report(ui.ctx());
        // 压缩包 ask 小菜单（fm_archive_open = "ask"；鼠标处弹出二选一）。
        self.render_archive_ask_menu(ui.ctx(), &mut intents);

        // 双击压缩包 ask 意图 → 记录弹出位置转状态（菜单下一帧起渲染）。
        if let Some(path) = intents.archive_ask.take() {
            let pos = ui
                .ctx()
                .pointer_interact_pos()
                .unwrap_or_else(|| ui.ctx().content_rect().center());
            self.archive_ask = Some((path, pos));
        }

        // 帧尾统一外抛回调。
        if intents.back {
            on_back();
        }
        if let Some(path) = intents.open_archive {
            on_open_archive(path);
        }
        if let Some(path) = intents.open_path {
            on_open_path(path);
        }
        if let Some(path) = intents.open_as_comic {
            on_open_as_comic(path);
        }
        if let Some((archive, dest)) = intents.extract {
            on_extract(archive, dest);
        }
        if let Some(msg) = intents.op_error {
            on_op_error(msg);
        }
        if let Some(confirm) = intents.confirm_delete_change {
            on_confirm_delete_change(confirm);
        }
    }

    /// 状态栏「剩余 X GB」数据源：焦点栏目录所在卷的可用字节
    /// （`platform::drive_info`）；会话内 30s 缓存，卷 key 变化（切盘/切栏
    /// 到异卷）立即重查；查询失败同样缓存（None 不每帧系统调用）。
    fn status_drive_free(&mut self) -> Option<u64> {
        const TTL: Duration = Duration::from_secs(30);
        let dir = self.panels[self.active].dir.clone();
        let key = crate::platform::drive_info::volume_key(&dir)?;
        let now = Instant::now();
        if let Some((k, bytes, t)) = &self.drive_free {
            if *k == key && now.duration_since(*t) < TTL {
                return *bytes;
            }
        }
        let bytes = crate::platform::drive_info::free_space(&dir);
        self.drive_free = Some((key, bytes, now));
        bytes
    }

    /// 顶栏：返回书架 / 单双栏切换（单栏时含预览开关）/ 右侧过滤框
    /// （过滤作用于焦点栏）。
    fn render_top_bar(&mut self, ui: &mut egui::Ui, intents: &mut FmIntents) {
        ui.horizontal(|ui| {
            if ui
                .button((icons::HOUSE, " 书架"))
                .on_hover_text("返回书架")
                .clicked()
            {
                intents.back = true;
            }
            ui.separator();
            let dual = matches!(self.layout, PanelLayout::Dual { .. });
            if ui
                .add(egui::Button::new((icons::COLUMNS, " 双栏")).selected(dual))
                .on_hover_text("双栏模式")
                .clicked()
                && !dual
            {
                self.layout = PanelLayout::Dual {
                    ratio: self.saved_ratio,
                };
            }
            if ui
                .add(egui::Button::new((icons::SQUARE_HALF, " 单栏")).selected(!dual))
                .on_hover_text("单栏模式（右侧为预览面板）")
                .clicked()
                && dual
            {
                if let PanelLayout::Dual { ratio } = self.layout {
                    self.saved_ratio = ratio;
                }
                // 保留焦点栏内容到左栏（单栏只渲染 panels[0]）。
                if self.active == 1 {
                    self.panels.swap(0, 1);
                }
                self.active = 0;
                self.preview_window_open = false;
                // 单栏无对面栏概念，快览关闭。
                self.quickview_open = false;
                self.layout = PanelLayout::Single {
                    preview_open: self.saved_preview_open,
                };
            }
            if matches!(self.layout, PanelLayout::Dual { .. })
                && ui
                    .add(egui::Button::new((icons::EYE, " 快览")).selected(self.quickview_open))
                    .on_hover_text("对面栏快速预览（Ctrl+Q）")
                    .clicked()
            {
                self.quickview_open = !self.quickview_open;
                if !self.quickview_open {
                    // 关闭即清预览目标，避免后台继续读取。
                    self.clear_preview();
                }
            }
            // 视图模式三态切换（焦点栏；选中/焦点是线性行索引天然保留，
            // 滚动按焦点行重定位——last_scroll_offset 的行高单位变了，
            // 置 MAX 让 min_scroll_to_reveal 把焦点行钉到视口顶）。
            let mode = self.panels[self.active].view_mode;
            let mode_icon = match mode {
                PanelViewMode::List => icons::LIST,
                PanelViewMode::Brief => icons::ROWS,
                PanelViewMode::Thumbs => icons::SQUARES_FOUR,
            };
            let mode_response = ui
                .add(egui::Button::new((mode_icon, format!(" {}", mode.label()))))
                .on_hover_text("视图模式（焦点栏）：列表 / 简表 / 缩略图");
            egui::Popup::menu(&mode_response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    for m in [
                        PanelViewMode::List,
                        PanelViewMode::Brief,
                        PanelViewMode::Thumbs,
                    ] {
                        if ui.selectable_label(mode == m, m.label()).clicked() {
                            let panel = &mut self.panels[self.active];
                            panel.view_mode = m;
                            if panel.focus.is_some() {
                                panel.focus_scroll_pending = true;
                                panel.last_scroll_offset = f32::MAX;
                            }
                            ui.close();
                        }
                    }
                });
            if let PanelLayout::Single { preview_open } = &mut self.layout {
                ui.separator();
                if ui
                    .add(egui::Button::new((icons::EYE, " 预览")).selected(*preview_open))
                    .on_hover_text("切换预览面板")
                    .clicked()
                {
                    *preview_open = !*preview_open;
                    self.saved_preview_open = *preview_open;
                }
            }
            ui.separator();
            if ui
                .button((icons::MAGNIFYING_GLASS, " 搜索"))
                .on_hover_text("文件搜索（Alt+F7）")
                .clicked()
            {
                self.open_search_dialog();
            }
            // 任务面板（阶段 AA）：列出在途+排队的文件操作；无任务时置灰。
            let has_tasks = self.ops.active_count() + self.ops.queued_count() > 0;
            if ui
                .add_enabled(
                    has_tasks,
                    egui::Button::new((icons::LIST_CHECKS, " 任务")).selected(self.task_panel_open),
                )
                .on_hover_text("文件操作任务面板（在途/排队/暂停/取消）")
                .clicked()
            {
                self.task_panel_open = !self.task_panel_open;
            }
            // 同步目录（阶段 AE）：仅双栏渲染（单栏隐藏），两栏目录不同才可用。
            if matches!(self.layout, PanelLayout::Dual { .. }) {
                let dirs_differ = self.panels[0].dir != self.panels[1].dir;
                if ui
                    .add_enabled(
                        dirs_differ,
                        egui::Button::new((icons::ARROWS_CLOCKWISE, " 同步")),
                    )
                    .on_hover_text("同步两栏目录…")
                    .clicked()
                {
                    let l = self.panels[0].dir.clone();
                    let r = self.panels[1].dir.clone();
                    self.sync_dialog.open_with(l, r);
                }
            }
            // 按钮栏为空的占位提示（阶段 Y）：非空时按钮条在顶栏下方整行
            // 渲染（含末尾「+」），顶栏不再重复占位。
            if self.button_bar.is_empty()
                && ui
                    .button((icons::PLUS, " 添加按钮"))
                    .on_hover_text("添加自定义命令按钮（按钮栏）")
                    .clicked()
            {
                self.button_dialog = Some(ButtonDialog::new_create());
            }
            // 过滤框（焦点栏）：默认顶栏右侧；fm_filter_bar_bottom 时改由
            // render_panel 渲染在焦点栏底部。
            if !self.options.filter_bar_bottom {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.render_filter_bar(ui);
                });
            }
        });
    }

    /// 自定义按钮栏（阶段 Y，fm_button_bar）：顶栏下方一条按钮条，列表为空
    /// 时不渲染、不占垂直空间（空栏的「+ 添加按钮」占位在顶栏，见
    /// render_top_bar）。点击经 spawn_shell_command 在焦点栏目录后台执行
    /// （占位符 %P/%N/%p 由 expand_button_command 展开；启动失败 →
    /// intents.op_error）。悬停显示 tooltip（空 = 展开后的命令）；右键弹
    /// 「编辑…/删除」小菜单；末尾「+」开添加对话框。列表权威在
    /// settings.fm_button_bar，本视图持副本经快照 diff 写回。
    fn render_button_bar(&mut self, ui: &mut egui::Ui, intents: &mut FmIntents) {
        if self.button_bar.is_empty() {
            return;
        }
        // 占位符值与按钮副本先取好（水平闭包内不能再借 self）。
        let focus_dir = self.panels[self.active].dir.display().to_string();
        let focus_name = self.panels[self.active].focused_entry().map(|e| e.name);
        let other_dir = self.panels[1 - self.active].dir.display().to_string();
        let buttons = self.button_bar.clone();
        ui.horizontal(|ui| {
            let mut run_idx = None;
            let mut edit_idx = None;
            let mut delete_idx = None;
            for (i, b) in buttons.iter().enumerate() {
                let expanded = expand_button_command(
                    &b.command,
                    &focus_dir,
                    focus_name.as_deref(),
                    &other_dir,
                );
                let response = ui.button(&b.label);
                let response = if b.tooltip.is_empty() {
                    response.on_hover_text(expanded)
                } else {
                    response.on_hover_text(&b.tooltip)
                };
                if response.clicked() {
                    run_idx = Some(i);
                }
                response.context_menu(|ui| {
                    if ui.button((icons::PENCIL_SIMPLE, " 编辑…")).clicked() {
                        edit_idx = Some(i);
                        ui.close();
                    }
                    if ui.button((icons::TRASH, " 删除")).clicked() {
                        delete_idx = Some(i);
                        ui.close();
                    }
                });
            }
            // 末尾小「+」开添加对话框（空栏时整条不渲染，占位在顶栏）。
            if ui
                .add(egui::Button::new(icons::PLUS.as_str()).small())
                .on_hover_text("添加按钮")
                .clicked()
            {
                self.button_dialog = Some(ButtonDialog::new_create());
            }
            if let Some(i) = edit_idx {
                self.button_dialog = Some(ButtonDialog::new_edit(i, &self.button_bar[i]));
            }
            if let Some(i) = delete_idx {
                self.button_bar.remove(i);
            }
            if let Some(i) = run_idx {
                let cmd = expand_button_command(
                    &self.button_bar[i].command,
                    &focus_dir,
                    focus_name.as_deref(),
                    &other_dir,
                );
                let dir = self.panels[self.active].dir.clone();
                if let Err(e) = spawn_shell_command(&dir, &cmd) {
                    intents.op_error = Some(e);
                }
            }
        });
    }

    /// 过滤条（顶栏右侧 / 栏底部两处调用点共享，位置由
    /// fm_filter_bar_bottom 决定）：输入框（作用于焦点栏）+ 方案/历史
    /// 下拉。Ctrl+S 的一次性请求（filter_focus_request）在此消费：未聚焦
    /// request_focus，已聚焦选中全文。过滤框聚焦时 Esc = 清空并交还焦点
    /// （置 filter_esc_handled 防 handle_keyboard 的 Esc 链同帧双消费）；
    /// 失焦/清空时把非空过滤串记入会话历史。
    fn render_filter_bar(&mut self, ui: &mut egui::Ui) {
        let edit_id = egui::Id::new("fm_filter_edit");
        let active = self.active;
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.panels[active].filter)
                .id(edit_id)
                .hint_text("过滤（焦点栏，支持 *.zip）")
                .desired_width(160.0),
        );
        let funnel = ui
            .add(egui::Button::new(icons::FUNNEL.as_str()).frame(false))
            .on_hover_text("过滤方案 / 最近使用");
        // 菜单动作先收集、闭包后统一应用（闭包内 self 只读借用）。
        let current = self.panels[active].filter.trim().to_string();
        let mut apply: Option<String> = None;
        let mut save_current = false;
        egui::Popup::menu(&funnel)
            .id(egui::Id::new("fm_filter_menu"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(200.0);
                if ui
                    .add_enabled(!current.is_empty(), egui::Button::new("保存当前过滤"))
                    .clicked()
                {
                    save_current = true;
                    ui.close();
                }
                ui.separator();
                if self.saved_filters.is_empty() {
                    ui.label(egui::RichText::new("（无保存的方案）").weak());
                }
                for f in &self.saved_filters {
                    if ui.button(f).clicked() {
                        apply = Some(f.clone());
                        ui.close();
                    }
                }
                if !self.filter_history.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("最近使用").weak());
                    for h in &self.filter_history {
                        if ui.button(h).clicked() {
                            apply = Some(h.clone());
                            ui.close();
                        }
                    }
                }
            });
        if save_current && !current.is_empty() && !self.saved_filters.contains(&current) {
            self.saved_filters.push(current);
        }
        if let Some(f) = apply {
            self.record_filter_history(&f);
            self.panels[active].filter = f;
        }
        // Ctrl+S：聚焦/选中全文。
        if self.filter_focus_request {
            self.filter_focus_request = false;
            if response.has_focus() {
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
                let len = self.panels[active].filter.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(len),
                    )));
                state.store(ui.ctx(), edit_id);
            } else {
                response.request_focus();
            }
        }
        // 聚焦时 Esc：清空并交还焦点（FM 的 Esc 链本帧被
        // egui_wants_keyboard_input 挡住；交还焦点后不再挡，故置
        // filter_esc_handled 防 handle_keyboard 同帧再走 Esc 链）。
        if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            let f = self.panels[active].filter.clone();
            self.record_filter_history(&f);
            self.panels[active].filter.clear();
            response.surrender_focus();
            self.filter_esc_handled = true;
        } else if response.lost_focus() {
            let f = self.panels[active].filter.clone();
            self.record_filter_history(&f);
        }
    }

    /// 命令行输入条（阶段 X，fm_command_bar）：FM 底部、状态栏上方（bottom
    /// panel 后注册者叠在上）。左侧弱色显示焦点栏当前目录作提示前缀，右侧
    /// 单行输入。Enter 执行（shell 后台 spawn，见 spawn_shell_command；
    /// 失败经 intents.op_error 上报）并清空 + 入历史（去重置顶，上限
    /// COMMAND_HISTORY_CAP）；聚焦时 ↑/↓ 逐条回填历史（TC 手感，↓ 越过
    /// 最新回到空白输入），Esc 清空并交还焦点（置 command_esc_handled 防
    /// handle_keyboard 的 Esc 链同帧双消费，同 filter_esc_handled 模式）。
    /// 命令行聚焦期间 FM 面板快捷键由 egui_wants_keyboard_input 统一屏蔽。
    fn render_command_bar(&mut self, ui: &mut egui::Ui, intents: &mut FmIntents) {
        if !self.options.command_bar {
            return;
        }
        egui::Panel::bottom("fm_command_bar").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let edit_id = egui::Id::new("fm_command_edit");
                let dir = self.panels[self.active].dir.display().to_string();
                ui.label(egui::RichText::new(format!("{dir}>")).weak());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.command_input)
                        .id(edit_id)
                        // Enter 不交还焦点（默认 return_key 会 surrender，焦点
                        // 锁滤波被重置，下一帧 ↑/↓ 会被 egui 当成焦点漫游键
                        // 把焦点移走）——Enter 由下面自行处理，执行后焦点保留。
                        .return_key(None)
                        .desired_width(f32::INFINITY),
                );
                // Ctrl+P / Ctrl+Enter 的一次性聚焦请求。已聚焦时不再
                // request_focus——那会重建 FocusWidget、把 TextEdit 刚设置的
                // 焦点锁滤波（方向键归输入框）重置回默认，下一帧 ↑/↓ 就被
                // egui 当成焦点漫游键把焦点移走。
                if self.command_focus_request {
                    self.command_focus_request = false;
                    if !response.has_focus() {
                        response.request_focus();
                    }
                }
                // Esc：清空并交还焦点。egui 在帧首就按焦点锁滤波（TextEdit
                // 的 EventFilter.escape=false）收走了焦点，故须认 lost_focus。
                if ui.input(|i| i.key_pressed(egui::Key::Escape))
                    && (response.has_focus() || response.lost_focus())
                {
                    self.command_input.clear();
                    self.command_history_pos = None;
                    if response.has_focus() {
                        response.surrender_focus();
                    }
                    self.command_esc_handled = true;
                    return;
                }
                if !response.has_focus() {
                    return;
                }
                // Enter 执行。带修饰键的 Enter 不算——Ctrl+Enter = 送焦点项
                // 文件名（本帧由 handle_keyboard 在命令行之后处理，不拦会
                // 边粘贴边执行）。
                if ui.input(|i| {
                    i.key_pressed(egui::Key::Enter)
                        && !i.modifiers.command
                        && !i.modifiers.alt
                        && !i.modifiers.shift
                }) {
                    let cmd = self.command_input.trim().to_string();
                    if !cmd.is_empty() {
                        let dir = self.panels[self.active].dir.clone();
                        match spawn_shell_command(&dir, &cmd) {
                            Ok(()) => {
                                push_history_capped(
                                    &mut self.command_history,
                                    &cmd,
                                    COMMAND_HISTORY_CAP,
                                );
                            }
                            Err(e) => intents.op_error = Some(e),
                        }
                    }
                    self.command_input.clear();
                    self.command_history_pos = None;
                    return;
                }
                // ↑/↓ 历史导航（回填后光标移到末尾——TextEditState 模式同
                // 过滤框 Ctrl+S 选中全文）。
                let len = self.command_history.len();
                let target = if len > 0 && ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    Some(Some(
                        self.command_history_pos.map_or(0, |p| (p + 1).min(len - 1)),
                    ))
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    match self.command_history_pos {
                        Some(0) => Some(None),
                        Some(p) => Some(Some(p - 1)),
                        None => None,
                    }
                } else {
                    None
                };
                let Some(target) = target else { return };
                self.command_history_pos = target;
                self.command_input = match target {
                    Some(p) => self.command_history[p].clone(),
                    None => String::new(),
                };
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
                let end = self.command_input.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(end),
                        egui::text::CCursor::new(end),
                    )));
                state.store(ui.ctx(), edit_id);
            });
            ui.add_space(2.0);
        });
    }

    /// 中央面板区；返回各栏的 rect（点击激活用）。
    fn render_panels(
        &mut self,
        ui: &mut egui::Ui,
        intents: &mut FmIntents,
    ) -> Vec<(usize, egui::Rect)> {
        let total_w = ui.available_width();
        let height = ui.available_height();
        match self.layout {
            PanelLayout::Dual { ratio } => {
                let mut ratio = ratio;
                let left_w = ((total_w - SPLITTER_WIDTH) * ratio).clamp(
                    PANEL_MIN_WIDTH,
                    (total_w - SPLITTER_WIDTH - PANEL_MIN_WIDTH).max(PANEL_MIN_WIDTH),
                );
                let mut rects = Vec::with_capacity(2);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let left = ui.allocate_ui_with_layout(
                        egui::vec2(left_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        // push_id 按栏隔离 widget Id 子树：否则两栏同位置控件的
                        // auto-id 相同（ScrollArea 滚动状态、列宽拖拽、列头点击
                        // 的持久状态被跨栏共享，滚左栏右栏跟着动）。
                        |ui| {
                            ui.push_id(("fm_panel", 0), |ui| {
                                self.render_panel_or_quickview(ui, 0, intents)
                            });
                        },
                    );
                    rects.push((0, left.response.rect));
                    // total_w 为 0（窗口被压扁的退化帧）时除法产生 NaN，
                    // NaN ratio 会传染进后续布局计算。
                    if let Some(delta) = render_splitter(ui, height) {
                        if total_w > 0.0 {
                            ratio = (ratio + delta / total_w).clamp(RATIO_MIN, RATIO_MAX);
                        }
                    }
                    let right_w = ui.available_width();
                    let right = ui.allocate_ui_with_layout(
                        egui::vec2(right_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.push_id(("fm_panel", 1), |ui| {
                                self.render_panel_or_quickview(ui, 1, intents)
                            });
                        },
                    );
                    rects.push((1, right.response.rect));
                });
                self.saved_ratio = ratio;
                self.layout = PanelLayout::Dual { ratio };
                rects
            }
            PanelLayout::Single { preview_open } => {
                let mut rects = Vec::with_capacity(1);
                if preview_open {
                    let mut preview_ratio = self.preview_ratio;
                    let preview_w = (total_w - SPLITTER_WIDTH) * preview_ratio;
                    let panel_w = (total_w - SPLITTER_WIDTH - preview_w).max(PANEL_MIN_WIDTH);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let panel = ui.allocate_ui_with_layout(
                            egui::vec2(panel_w, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.push_id(("fm_panel", 0), |ui| self.render_panel(ui, 0, intents));
                            },
                        );
                        rects.push((0, panel.response.rect));
                        if let Some(delta) = render_splitter(ui, height) {
                            if total_w > 0.0 {
                                preview_ratio =
                                    (preview_ratio - delta / total_w).clamp(RATIO_MIN, 0.6);
                            }
                        }
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.draw_preview_content(ui, false),
                        );
                    });
                    self.preview_ratio = preview_ratio;
                } else {
                    let panel = ui.allocate_ui_with_layout(
                        egui::vec2(total_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.push_id(("fm_panel", 0), |ui| self.render_panel(ui, 0, intents));
                        },
                    );
                    rects.push((0, panel.response.rect));
                }
                rects
            }
        }
    }

    /// 双栏一侧内容：Ctrl+Q 快览开启且本侧为非活动栏时整栏替换为预览面板
    /// （被替换栏对象不列举不渲染，state/selected/滚动原样保留，关闭快览
    /// 后原样恢复）；否则渲染常规栏。
    fn render_panel_or_quickview(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        intents: &mut FmIntents,
    ) {
        if self.quickview_open && self.active != idx {
            self.draw_preview_content(ui, false);
        } else {
            self.render_panel(ui, idx, intents);
        }
    }

    /// 单栏内容：标签条 → 面包屑 → 状态（加载/失败）→ 列头 → 虚拟化明细列表。
    fn render_panel(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        self.render_tab_bar(ui, idx);
        self.render_breadcrumb(ui, idx);
        // fm_filter_bar_bottom：过滤条渲染在焦点栏列表底部——先给内容区
        // 留出高度，再在栏底画过滤条（只作用于焦点栏，切栏即跟随）。
        let bottom_bar = self.options.filter_bar_bottom && idx == self.active;
        if bottom_bar {
            let bar_h = 28.0;
            let content_h = (ui.available_height() - bar_h).max(60.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), content_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.render_panel_body(ui, idx, intents);
                },
            );
            ui.separator();
            self.render_filter_bar(ui);
        } else {
            self.render_panel_body(ui, idx, intents);
        }
    }

    /// 栏内容主体（列表/网格/加载态），render_panel 抽出以配合底部过滤条
    /// 的高度预留。
    fn render_panel_body(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        // 先抽出状态快照，避免 match 借用与臂内 &mut self 冲突。
        enum Phase {
            Idle,
            Loading,
            Failed(String),
            Ready,
        }
        let phase = match &self.panels[idx].state {
            PanelLoadState::Idle => Phase::Idle,
            PanelLoadState::Loading(_) => Phase::Loading,
            PanelLoadState::Failed(e) => Phase::Failed(e.clone()),
            PanelLoadState::Ready => Phase::Ready,
        };
        match phase {
            Phase::Idle => {
                ui.label(egui::RichText::new("正在初始化…").weak());
            }
            Phase::Loading => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new("正在读取目录…").weak());
                });
            }
            Phase::Failed(message) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new(icons::WARNING.as_str()).size(24.0));
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, message);
                    if ui.button((icons::ARROW_CLOCKWISE, " 重试")).clicked() {
                        self.panels[idx].refresh();
                    }
                });
            }
            Phase::Ready => match self.panels[idx].view_mode {
                PanelViewMode::List => {
                    self.render_column_header(ui, idx);
                    self.render_list(ui, idx, intents);
                }
                PanelViewMode::Brief => {
                    self.render_brief(ui, idx, intents);
                }
                PanelViewMode::Thumbs => {
                    self.render_grid(ui, idx, intents);
                }
            },
        }
    }

    /// 标签条（面包屑上方）：标签 = 目录 basename（根目录显示盘符/根名，
    /// 超长截断，悬停全路径）；当前标签高亮，单击切换、中键关闭（剩 1 个
    /// 禁关）、右侧「+」新建（复制当前目录）；过多时横向可滚动。
    fn render_tab_bar(&mut self, ui: &mut egui::Ui, idx: usize) {
        enum TabAction {
            Switch(usize),
            Close(usize),
            New,
        }
        let mut action = None;
        egui::ScrollArea::horizontal()
            .id_salt(("fm_tab_bar", idx))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let count = self.panels[idx].tab_count();
                    let active_tab = self.panels[idx].active_tab();
                    for i in 0..count {
                        let dir = self.panels[idx].tab_dir(i).to_path_buf();
                        let resp = ui
                            .add(egui::Button::new(tab_label(&dir)).selected(i == active_tab))
                            .on_hover_text(dir.display().to_string());
                        // 拖放落点 rect（阶段 Z）：非当前标签才记（当前标签 =
                        // 本栏目录，拖上没有意义）。
                        if i != active_tab {
                            self.tab_drop_rects.push((dir.clone(), resp.rect));
                        }
                        if resp.clicked() {
                            action = Some(TabAction::Switch(i));
                        }
                        if resp.middle_clicked() && count > 1 {
                            action = Some(TabAction::Close(i));
                        }
                    }
                    if ui
                        .add(egui::Button::new(icons::PLUS.as_str()).frame(false))
                        .on_hover_text("新建标签（Ctrl+T）")
                        .clicked()
                    {
                        action = Some(TabAction::New);
                    }
                });
            });
        match action {
            Some(TabAction::Switch(i)) => self.panels[idx].switch_tab(i),
            Some(TabAction::Close(i)) => self.panels[idx].close_tab(i),
            Some(TabAction::New) => self.panels[idx].new_tab(),
            None => {}
        }
    }

    /// 面包屑：盘符下拉（最左）+ 路径分段可点击跳回 + 铅笔按钮路径编辑；
    /// 焦点栏铺淡底色高亮。
    fn render_breadcrumb(&mut self, ui: &mut egui::Ui, idx: usize) {
        let active = self.active == idx;
        let fill = if active {
            ui.visuals().faint_bg_color
        } else {
            egui::Color32::TRANSPARENT
        };
        egui::Frame::new()
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(4, 2))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let back_tip = self.panels[idx]
                        .back_target()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "后退".to_string());
                    if ui
                        .add_enabled(
                            self.panels[idx].can_go_back(),
                            egui::Button::new(icons::CARET_LEFT.as_str()).frame(false),
                        )
                        .on_hover_text(back_tip)
                        .clicked()
                    {
                        self.panels[idx].go_back();
                    }
                    let forward_tip = self.panels[idx]
                        .forward_target()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "前进".to_string());
                    if ui
                        .add_enabled(
                            self.panels[idx].can_go_forward(),
                            egui::Button::new(icons::CARET_RIGHT.as_str()).frame(false),
                        )
                        .on_hover_text(forward_tip)
                        .clicked()
                    {
                        self.panels[idx].go_forward();
                    }
                    self.render_history_button(ui, idx);
                    self.render_drive_switcher(ui, idx);
                    self.render_bookmarks_button(ui, idx);
                    ui.separator();
                    // 路径编辑态：TextEdit 替换分段；Enter 导航 / Esc 还原，
                    // 无效路径红字保持编辑态（egui_wants_keyboard_input 会
                    // 屏蔽面板全局键，Enter/Esc 在此自行检测）。
                    if self.breadcrumb_edit == Some(idx) {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.breadcrumb_edit_text)
                                .desired_width(ui.available_width().max(240.0)),
                        );
                        if !self.breadcrumb_edit_focused {
                            response.request_focus();
                            self.breadcrumb_edit_focused = true;
                        }
                        if response.changed() {
                            self.breadcrumb_edit_error = false;
                        }
                        if self.breadcrumb_edit_error {
                            ui.colored_label(ui.visuals().error_fg_color, "路径不存在");
                        }
                        // 单行输入框 Enter 自动失焦（同 SelectGroupDialog 注释）。
                        let enter =
                            response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape))
                            && (response.has_focus() || response.lost_focus());
                        if enter {
                            let typed = PathBuf::from(self.breadcrumb_edit_text.trim());
                            if typed.is_dir() {
                                // 导航竞态兜底同书签跳转。
                                let target = fallback_existing_dir(typed);
                                self.breadcrumb_edit = None;
                                self.panels[idx].navigate_to(target);
                            } else {
                                self.breadcrumb_edit_error = true;
                                response.request_focus();
                            }
                        } else if esc {
                            self.breadcrumb_edit = None;
                        }
                        return;
                    }
                    let dir = self.panels[idx].dir.clone();
                    let branch = self.panels[idx].branch_view;
                    let segments = breadcrumb_segments(&dir);
                    if segments.is_empty() {
                        ui.label(egui::RichText::new("…").weak());
                        return;
                    }
                    let last = segments.len() - 1;
                    let mut jump: Option<PathBuf> = None;
                    for (i, (path, label)) in segments.iter().enumerate() {
                        if i > 0 {
                            ui.label(egui::RichText::new("›").weak());
                        }
                        let text = if i == last {
                            egui::RichText::new(label).strong()
                        } else {
                            egui::RichText::new(label)
                        };
                        let seg_resp = ui
                            .selectable_label(i == last, text)
                            .on_hover_text(path.display().to_string());
                        // 拖放落点 rect（阶段 Z）：全部段都记（含当前段——
                        // 同目录跳过由 poll_inter_panel_dnd 判定）。
                        self.breadcrumb_drop_rects
                            .push((path.clone(), seg_resp.rect));
                        if seg_resp.clicked() && i != last {
                            jump = Some(path.clone());
                        }
                    }
                    if let Some(path) = jump {
                        self.panels[idx].navigate_to(path);
                    }
                    // 分支视图状态指示（路径后弱色标记）。
                    if branch {
                        ui.label(egui::RichText::new("[分支]").weak());
                    }
                    // 铅笔按钮：进入路径编辑态（预填当前目录完整路径）。
                    if ui
                        .add(egui::Button::new(icons::PENCIL_SIMPLE.as_str()).frame(false))
                        .on_hover_text("编辑路径")
                        .clicked()
                    {
                        self.breadcrumb_edit = Some(idx);
                        self.breadcrumb_edit_text = self.panels[idx].dir.display().to_string();
                        self.breadcrumb_edit_focused = false;
                        self.breadcrumb_edit_error = false;
                    }
                });
            });
    }

    /// 历史下拉按钮（‹ › 旁）：菜单列出目录历史（新→旧，当前项打勾 ✓，
    /// 悬停显示完整路径），点击直跳；Alt+↓ 经 history_menu_toggle 一次性
    /// 请求切换焦点栏菜单的开/关（与点击共用同一 memory 弹层状态）。
    fn render_history_button(&mut self, ui: &mut egui::Ui, idx: usize) {
        let response = ui
            .add(egui::Button::new(icons::CLOCK_COUNTER_CLOCKWISE.as_str()).frame(false))
            .on_hover_text("目录历史（Alt+↓）");
        let kb_toggle = self.history_menu_toggle && self.active == idx;
        if kb_toggle {
            self.history_menu_toggle = false;
        }
        let set = (response.clicked() || kb_toggle).then_some(egui::SetOpenCommand::Toggle);
        egui::Popup::menu(&response)
            .id(egui::Id::new(("fm_history_menu", idx)))
            .open_memory(set)
            .show(|ui| {
                let history = self.panels[idx].history_list();
                if history.is_empty() {
                    ui.label(egui::RichText::new("（无历史）").weak());
                    return;
                }
                ui.set_min_width(320.0);
                let mut jump: Option<usize> = None;
                let count = history.len();
                for (i, (path, is_current)) in history.iter().enumerate() {
                    let display = path.display().to_string();
                    let text = if *is_current {
                        format!("✓ {display}")
                    } else {
                        display.clone()
                    };
                    if ui
                        .selectable_label(*is_current, text)
                        .on_hover_text(&display)
                        .clicked()
                    {
                        // 菜单序 i（新→旧）→ 历史 pos = count - i（1 起）。
                        jump = Some(count - i);
                        ui.close();
                    }
                }
                if let Some(pos) = jump {
                    self.panels[idx].navigate_history_to(pos);
                }
            });
    }

    /// 面包屑最左的盘符下拉：显示该栏当前盘符（如 `C:\`），点开列出可用
    /// 盘符/卷，点击后该栏 navigate_to 盘符根（两栏各自独立）。盘符枚举
    /// 首次打开时后台线程执行（慢速/断开设备不卡 UI），结果缓存于视图。
    fn render_drive_switcher(&mut self, ui: &mut egui::Ui, idx: usize) {
        // 每帧尝试接收后台枚举结果；在途时主动重绘直到就绪。
        if let Some(rx) = &self.drives_rx {
            if let Ok(list) = rx.try_recv() {
                self.drives = Some(list);
                self.drives_rx = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        }
        let label = format!(
            "{} {}",
            icons::HARD_DRIVES.as_str(),
            drive_label(&self.panels[idx].dir)
        );
        ui.menu_button(label, |ui| {
            if self.drives.is_none() {
                // 首次打开：起后台线程枚举（仅一次，结果缓存）。
                if self.drives_rx.is_none() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(list_drives());
                    });
                    self.drives_rx = Some(rx);
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                ui.label(egui::RichText::new("正在扫描盘符…").weak());
                return;
            }
            let drives = self.drives.clone().unwrap_or_default();
            let current = self.panels[idx].dir.clone();
            for d in drives {
                let is_current = current.starts_with(&d) && d.as_os_str() != "/";
                if ui
                    .selectable_label(is_current, d.display().to_string())
                    .clicked()
                {
                    self.panels[idx].navigate_to(d);
                    ui.close();
                }
            }
        });
    }

    /// 面包屑上的书签菜单（两栏共享一份）：顶部「添加当前目录 ▸」（分组子菜单，
    /// 已在组内打勾禁用）+「新建分组…」，下方各分组子菜单（书签点击跳转经
    /// fallback_existing_dir、✕ 移除不收起菜单；组尾重命名/删除分组）。
    /// 动作先收集、闭包内统一应用（迭代分组时 self 只能只读借用）。
    /// Ctrl+D 经 bookmarks_menu_toggle 一次性请求切换焦点栏菜单开/关（与点击
    /// 共用同一 memory 弹层状态，同历史下拉的 Alt+↓ 模式）。
    fn render_bookmarks_button(&mut self, ui: &mut egui::Ui, idx: usize) {
        let response = ui
            .add(egui::Button::new(icons::STAR.as_str()).frame(false))
            .on_hover_text("常用目录书签（Ctrl+D）");
        let kb_toggle = self.bookmarks_menu_toggle && self.active == idx;
        if kb_toggle {
            self.bookmarks_menu_toggle = false;
        }
        let set = (response.clicked() || kb_toggle).then_some(egui::SetOpenCommand::Toggle);
        egui::Popup::menu(&response)
            .id(egui::Id::new(("fm_bookmarks_menu", idx)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .open_memory(set)
            .show(|ui| {
                ui.set_min_width(280.0);
                let dir = self.panels[idx].dir.clone();
                let mut add_to: Option<usize> = None;
                let mut open_create = false;
                let mut open_rename: Option<usize> = None;
                let mut delete_group: Option<usize> = None;
                let mut remove_bm: Option<(usize, PathBuf)> = None;
                let mut jump: Option<PathBuf> = None;
                egui::containers::menu::SubMenuButton::new("添加当前目录").ui(ui, |ui| {
                    if self.bookmark_groups.is_empty() {
                        ui.label(egui::RichText::new("（无分组，请先新建分组）").weak());
                        return;
                    }
                    for (gi, g) in self.bookmark_groups.iter().enumerate() {
                        let already = g.items.iter().any(|b| Path::new(b) == dir);
                        let label = if already {
                            format!("✓ {}", g.name)
                        } else {
                            g.name.clone()
                        };
                        if ui.add_enabled(!already, egui::Button::new(label)).clicked() {
                            add_to = Some(gi);
                        }
                    }
                });
                if ui.button("新建分组…").clicked() {
                    open_create = true;
                }
                ui.separator();
                if self.bookmark_groups.is_empty() {
                    ui.label(egui::RichText::new("（无书签）").weak());
                }
                for (gi, g) in self.bookmark_groups.iter().enumerate() {
                    egui::containers::menu::SubMenuButton::new(g.name.as_str()).ui(ui, |ui| {
                        ui.set_min_width(240.0);
                        if g.items.is_empty() {
                            ui.label(egui::RichText::new("（空分组）").weak());
                        }
                        for bm in &g.items {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(icons::X.as_str())
                                                .frame(false)
                                                .small(),
                                        )
                                        .on_hover_text("移除书签")
                                        .clicked()
                                    {
                                        remove_bm = Some((gi, PathBuf::from(bm)));
                                    }
                                    if ui
                                        .add(
                                            egui::Label::new(bm.as_str())
                                                .truncate()
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(bm)
                                        .clicked()
                                    {
                                        jump = Some(PathBuf::from(bm));
                                    }
                                },
                            );
                        }
                        ui.separator();
                        if ui.button("重命名分组…").clicked() {
                            open_rename = Some(gi);
                        }
                        // 从简无确认：组内书签一并删除。
                        if ui.button("删除分组").clicked() {
                            delete_group = Some(gi);
                        }
                    });
                }
                // 统一应用收集到的动作。
                if let Some(gi) = add_to {
                    self.add_bookmark_to_group(gi, &dir);
                }
                if let Some((gi, bm)) = remove_bm {
                    self.remove_bookmark(gi, &bm);
                }
                if let Some(bm) = jump {
                    let target = fallback_existing_dir(bm);
                    self.panels[idx].navigate_to(target);
                    ui.close();
                }
                if open_create {
                    self.group_dialog = Some(BookmarkGroupDialog::new_create());
                    ui.close();
                }
                if let Some(gi) = open_rename {
                    let current = self.bookmark_groups[gi].name.clone();
                    self.group_dialog = Some(BookmarkGroupDialog::new_rename(gi, &current));
                    ui.close();
                }
                if let Some(gi) = delete_group {
                    self.remove_group(gi);
                    ui.close();
                }
            });
    }

    /// 压缩包 ask 小菜单（fm_archive_open = "ask"）：双击处在鼠标位置弹出
    /// 「Archive 视图打开 / 作为漫画打开」二选一；Esc（handle_keyboard 的
    /// Esc 链首档）/ 点击菜单外关闭。
    fn render_archive_ask_menu(&mut self, ctx: &egui::Context, intents: &mut FmIntents) {
        let Some((path, pos)) = self.archive_ask.clone() else {
            return;
        };
        let mut close = false;
        let area = egui::Area::new(egui::Id::new("fm_archive_ask"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if ui.button("Archive 视图打开").clicked() {
                        intents.open_archive = Some(path.clone());
                        close = true;
                    }
                    if ui.button("作为漫画打开").clicked() {
                        intents.open_as_comic = Some(path.clone());
                        close = true;
                    }
                });
            });
        // 点击菜单外关闭（按钮点击已置 close，不冲突）。
        let outside_click = ctx.input(|i| {
            (i.pointer.primary_pressed() || i.pointer.secondary_pressed())
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p))
        });
        if close || outside_click {
            self.archive_ask = None;
        }
    }

    /// 渲染书签分组对话框（新建/重命名共用；非模态 egui::Window，
    /// 参照 SelectGroupDialog 的 focused/Enter 模式）。
    fn render_group_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.group_dialog.take() else {
            return;
        };
        let title = if dialog.rename.is_some() {
            "重命名分组"
        } else {
            "新建分组"
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancelled = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("分组名：");
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
                    if !dialog.focused {
                        response.request_focus();
                        dialog.focused = true;
                    }
                    // 单行输入框 Enter 自动失焦。
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                });
                let name = dialog.name.trim().to_string();
                let dup = !name.is_empty()
                    && self
                        .bookmark_groups
                        .iter()
                        .enumerate()
                        .any(|(i, g)| Some(i) != dialog.rename && g.name == name);
                let error = if name.is_empty() {
                    Some("名称不能为空")
                } else if dup {
                    Some("已存在同名分组")
                } else {
                    None
                };
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("确定"))
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirm {
            match dialog.rename {
                Some(i) => {
                    self.rename_group(i, &dialog.name);
                }
                None => {
                    self.add_group(&dialog.name);
                }
            }
        } else if !cancelled && open {
            self.group_dialog = Some(dialog);
        }
    }

    /// 任务面板（阶段 AA）：非模态窗口列出全部在途 + 排队任务——动词图标、
    /// 源→目标摘要（首项 + 共 N 项）、进度条（排队任务显示「排队中」）、
    /// 暂停/继续（排队/压缩禁用）与取消。顶栏「任务」按钮与状态栏进度区
    /// 点击开关；无任务时按钮置灰。
    fn render_task_panel(&mut self, ctx: &egui::Context) {
        if !self.task_panel_open {
            return;
        }
        let summaries = self.ops.task_summaries();
        let mut open = self.task_panel_open;
        let mut cancel_id: Option<u64> = None;
        let mut toggle_pause: Option<(u64, bool)> = None;
        egui::Window::new("任务")
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if summaries.is_empty() {
                    ui.label(egui::RichText::new("无在途或排队任务").weak());
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for s in &summaries {
                        let icon = match s.kind {
                            OpKind::Copy => icons::COPY,
                            OpKind::Move => icons::ARROW_RIGHT,
                            OpKind::Delete => icons::TRASH,
                            OpKind::Compress => icons::FILE_ZIP,
                            OpKind::Split => icons::SCISSORS,
                            OpKind::Merge => icons::ARROWS_MERGE,
                        };
                        ui.horizontal(|ui| {
                            ui.label(icon);
                            // 源→目标摘要：首项文件名 + 共 N 项；目标仅
                            // 复制/移动/压缩有。
                            let first = s
                                .first_source
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let mut text = format!("{} {first}", s.kind.verb());
                            if s.source_count > 1 {
                                text.push_str(&format!(" 等 {} 项", s.source_count));
                            }
                            if let Some(dest) = &s.dest_dir {
                                text.push_str(&format!(" → {}", dest.display()));
                            }
                            let full = s
                                .first_source
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            ui.label(text).on_hover_text(full);
                        });
                        ui.horizontal(|ui| {
                            if s.queued {
                                ui.label(egui::RichText::new("排队中").weak());
                            } else {
                                let fraction = s.progress.fraction();
                                let pct = (fraction * 100.0).round() as u32;
                                let bar_text = if s.paused {
                                    format!("{pct}%（已暂停）")
                                } else {
                                    format!("{pct}%")
                                };
                                ui.add(
                                    egui::ProgressBar::new(fraction)
                                        .desired_width(160.0)
                                        .text(bar_text),
                                );
                                // 暂停/继续（阶段 AB 起压缩也支持暂停）。
                                let pause_label = if s.paused { "继续" } else { "暂停" };
                                if ui.add(egui::Button::new(pause_label).small()).clicked() {
                                    toggle_pause = Some((s.id, !s.paused));
                                }
                            }
                            if ui.small_button("取消").clicked() {
                                cancel_id = Some(s.id);
                            }
                        });
                        ui.separator();
                    }
                });
            });
        self.task_panel_open = open;
        if let Some(id) = cancel_id {
            // 在途置旗标；排队任务直接出队（file_ops cancel 内部区分）。
            self.ops.cancel(id);
        }
        if let Some((id, paused)) = toggle_pause {
            self.ops.set_paused(id, paused);
        }
    }

    /// 错误汇总窗（阶段 AA）：操作结束且 errors 非空时弹出（on_op_finished
    /// 留存报告），虚拟化列出全部失败项（路径 + 原因）；「重试失败项」从
    /// 失败项重建同参数任务（源路径仍在才纳入，retry_sources）。
    fn render_op_error_report(&mut self, ctx: &egui::Context) {
        let Some(report) = &self.op_error_report else {
            return;
        };
        let mut open = true;
        let mut retry = false;
        egui::Window::new(format!(
            "{} — {} 项失败",
            report.kind.verb(),
            report.errors.len()
        ))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .open(&mut open)
        .show(ctx, |ui| {
            let row_height = ui.text_style_height(&egui::TextStyle::Body);
            egui::ScrollArea::vertical().max_height(320.0).show_rows(
                ui,
                row_height,
                report.errors.len(),
                |ui, range| {
                    for (path, err) in &report.errors[range] {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(path.display().to_string())
                                    .color(ui.visuals().warn_fg_color),
                            );
                            ui.label(egui::RichText::new(err).weak());
                        });
                    }
                },
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button((icons::ARROW_CLOCKWISE, " 重试失败项"))
                    .on_hover_text("以相同参数重试仍存在的源（Copy/Move 沿用原目标与冲突策略）")
                    .clicked()
                {
                    retry = true;
                }
            });
        });
        if !open {
            self.op_error_report = None;
            return;
        }
        if retry {
            let Some(report) = self.op_error_report.take() else {
                return;
            };
            let sources = retry_sources(&report.errors);
            if sources.is_empty() {
                // 失败源全部消失：无可重试项（报告关闭）。
                return;
            }
            match report.kind {
                OpKind::Copy => {
                    if let Some(dest) = report.dest_dir {
                        self.ops.start_copy(sources, dest, report.conflict);
                    }
                }
                OpKind::Move => {
                    if let Some(dest) = report.dest_dir {
                        self.ops.start_move(sources, dest, report.conflict);
                    }
                }
                OpKind::Delete => {
                    self.ops.start_delete(sources, report.delete_permanent);
                }
                OpKind::Compress => {
                    // Compress 失败走 fatal 不进 errors，理论不到这里；
                    // 兜底用原 dest_zip 重压。
                    if let Some(zip) = report.dest {
                        self.ops.start_compress(sources, zip);
                    }
                }
                // Split/Merge 的失败项是产出文件（分块/合并结果）而非源，
                // retry_sources 语义不适用；且 Split 重建缺 chunk_size——不重试。
                OpKind::Split | OpKind::Merge => {}
            }
        }
    }

    /// 「添加/编辑按钮」对话框（阶段 Y）：三字段（按钮文字/命令/悬停提示），
    /// 确定时 trim 后写回 button_bar（权威经快照 diff 进 settings.fm_button_bar）。
    fn render_button_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.button_dialog.take() else {
            return;
        };
        let title = if dialog.edit.is_some() {
            "编辑按钮"
        } else {
            "添加按钮"
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancelled = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("按钮文字：");
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut dialog.label).desired_width(160.0));
                    if !dialog.focused {
                        response.request_focus();
                        dialog.focused = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("命令：");
                    ui.add(egui::TextEdit::singleline(&mut dialog.command).desired_width(320.0));
                });
                ui.horizontal(|ui| {
                    ui.label("悬停提示：");
                    ui.add(egui::TextEdit::singleline(&mut dialog.tooltip).desired_width(320.0));
                });
                ui.label(
                    egui::RichText::new("占位符：%P = 当前目录，%N = 焦点项名称，%p = 另一栏目录")
                        .weak()
                        .small(),
                );
                let error = if dialog.label.trim().is_empty() {
                    Some("按钮文字不能为空")
                } else if dialog.command.trim().is_empty() {
                    Some("命令不能为空")
                } else {
                    None
                };
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("确定"))
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirm {
            let button = FmButton {
                label: dialog.label.trim().to_string(),
                command: dialog.command.trim().to_string(),
                tooltip: dialog.tooltip.trim().to_string(),
            };
            match dialog.edit {
                Some(i) => {
                    if let Some(slot) = self.button_bar.get_mut(i) {
                        *slot = button;
                    }
                }
                None => self.button_bar.push(button),
            }
        } else if !cancelled && open {
            self.button_dialog = Some(dialog);
        }
    }

    /// 「保存当前选择…」小对话框（阶段 U；同 BookmarkGroupDialog 单输入
    /// 模式）：命名后存入 saved_selections（重名拒绝）。
    fn render_selection_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.selection_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancelled = false;
        egui::Window::new("保存当前选择")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("已捕获 {} 个选中项", dialog.paths.len()));
                ui.horizontal(|ui| {
                    ui.label("名称：");
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
                    if !dialog.focused {
                        response.request_focus();
                        dialog.focused = true;
                    }
                    // 单行输入框 Enter 自动失焦。
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                });
                let name = dialog.name.trim().to_string();
                let dup = !name.is_empty() && self.saved_selections.iter().any(|(n, _)| n == &name);
                let error = if name.is_empty() {
                    Some("名称不能为空")
                } else if dup {
                    Some("已存在同名选择集")
                } else {
                    None
                };
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("确定"))
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirm {
            let name = dialog.name.trim().to_string();
            self.saved_selections.push((name, dialog.paths));
        } else if !cancelled && open {
            self.selection_dialog = Some(dialog);
        }
    }

    /// 「编辑注释…」对话框（阶段 W）：多行输入预填现注释；确定写回
    /// descript.ion（空 = 删除该条）并重读该栏缓存；失败保持打开显示错误。
    /// Ctrl+Enter = 确认（多行框里 Enter 是换行）。
    fn render_comment_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.comment_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancelled = false;
        egui::Window::new(format!("编辑注释 — {}", dialog.name))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("注释（留空 = 删除该条；换行写回时折叠为空格）：");
                let response = ui.add(
                    egui::TextEdit::multiline(&mut dialog.text)
                        .desired_width(320.0)
                        .desired_rows(3),
                );
                if !dialog.focused {
                    response.request_focus();
                    dialog.focused = true;
                }
                if response.has_focus()
                    && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                {
                    confirm = true;
                }
                if let Some(err) = &dialog.error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirm {
            let dir = self.panels[dialog.panel].dir.clone();
            match crate::views::fm_comments::write_comment(&dir, &dialog.name, Some(&dialog.text)) {
                Ok(()) => {
                    self.panels[dialog.panel].reload_comments();
                }
                Err(e) => {
                    dialog.error = Some(e);
                    self.comment_dialog = Some(dialog);
                }
            }
        } else if !cancelled && open {
            self.comment_dialog = Some(dialog);
        }
    }

    /// 列头：名称 + 固定列（FsPanel::columns 驱动，阶段 V），整列格可点击
    /// 切换排序键与升降序（Comment 列无数据来源不可排序），当前键显示
    /// ▲/▼。列坐标取自 column_layout（与行内容/竖线同一来源）。每个列头
    /// 格都可右键：排序键/升降序菜单 + 列勾选（增删固定列，至少保留 1 个）。
    /// 每条分隔竖线各带 6pt 拖拽热区（后注册于列点击格，拖拽优先）。
    fn render_column_header(&mut self, ui: &mut egui::Ui, idx: usize) {
        let (header_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::hover(),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(
            header_rect,
            0.0,
            ui.visuals().widgets.noninteractive.bg_fill,
        );
        let panel = &mut self.panels[idx];
        let layout = column_layout(header_rect.right(), panel.col_shift, &panel.columns);
        let cy = header_rect.center().y;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        // 扩展名排序时名称列头显示「扩展名」——仅当扩展名没有独立列时
        // （有独立列则箭头与标题都落在扩展名列上）。
        let has_ext_col = panel.columns.iter().any(|(k, _)| *k == ColumnKind::Ext);
        // (列 rect, 列类型, 标题, 文字锚点 x, 对齐方式)
        let mut cols: Vec<(egui::Rect, ColumnKind, String, f32, egui::Align2)> =
            Vec::with_capacity(panel.columns.len());
        let name_rect = egui::Rect::from_min_max(
            header_rect.min,
            egui::pos2(layout.name_right, header_rect.max.y),
        );
        let name_label = if panel.sort_key == SortKey::Ext && !has_ext_col {
            "扩展名"
        } else {
            "名称"
        };
        cols.push((
            name_rect,
            ColumnKind::Name,
            name_label.to_string(),
            header_rect.left() + NAME_HEADER_INDENT,
            egui::Align2::LEFT_CENTER,
        ));
        for (i, (kind, left, text_right)) in layout.fixed.iter().enumerate() {
            let right = layout
                .fixed
                .get(i + 1)
                .map(|(_, l, _)| *l)
                .unwrap_or(header_rect.right());
            let rect = egui::Rect::from_min_max(
                egui::pos2(*left, header_rect.min.y),
                egui::pos2(right, header_rect.max.y),
            );
            cols.push((
                rect,
                *kind,
                kind.label().to_string(),
                *text_right,
                egui::Align2::RIGHT_CENTER,
            ));
        }
        let mut clicked: Option<ColumnKind> = None;
        let mut texts: Vec<(egui::Pos2, egui::Align2, String)> = Vec::with_capacity(cols.len());
        for (rect, kind, label, anchor_x, align) in cols {
            let response = ui.interact(
                rect,
                ui.id().with(("fm-header", kind.as_str())),
                egui::Sense::click(),
            );
            if response.clicked() && kind.sort_key().is_some() {
                clicked = Some(kind);
            }
            // 列头右键：排序键/升降序菜单 + 列勾选（整格左击逻辑不变）。
            response.context_menu(|ui| {
                for (k, menu_label) in [
                    (SortKey::Name, "名称"),
                    (SortKey::Ext, "扩展名"),
                    (SortKey::Size, "大小"),
                    (SortKey::Mtime, "修改时间"),
                    (SortKey::Attr, "属性"),
                    (SortKey::Unsorted, "不排序"),
                ] {
                    if ui
                        .selectable_label(panel.sort_key == k, menu_label)
                        .clicked()
                    {
                        panel.toggle_sort(k);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.selectable_label(panel.sort_asc, "升序").clicked() {
                    panel.sort_asc = true;
                    ui.close();
                }
                if ui.selectable_label(!panel.sort_asc, "降序").clicked() {
                    panel.sort_asc = false;
                    ui.close();
                }
                ui.separator();
                // 列勾选：Name 恒在（置灰展示）；固定列至少保留 1 个
                // （唯一固定列的取消勾选置灰）。
                let fixed_count = panel.columns.len() - 1;
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.selectable_label(true, ColumnKind::Name.label());
                });
                for kind in [
                    ColumnKind::Ext,
                    ColumnKind::Size,
                    ColumnKind::Mtime,
                    ColumnKind::Attr,
                    ColumnKind::Comment,
                ] {
                    let present = panel.columns.iter().any(|(k, _)| *k == kind);
                    let removable = present && fixed_count > 1;
                    ui.add_enabled_ui(!present || removable, |ui| {
                        if ui.selectable_label(present, kind.label()).clicked() {
                            panel.toggle_column(kind);
                            ui.close();
                        }
                    });
                }
            });
            if response.hovered() {
                painter.rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
            }
            // 扩展名排序的箭头画在名称列——仅当扩展名没有独立列（见上）。
            let arrow = if kind.sort_key() == Some(panel.sort_key)
                || (kind == ColumnKind::Name && panel.sort_key == SortKey::Ext && !has_ext_col)
            {
                if panel.sort_asc {
                    " ▲"
                } else {
                    " ▼"
                }
            } else {
                ""
            };
            texts.push((egui::pos2(anchor_x, cy), align, format!("{label}{arrow}")));
        }
        // 列宽拖拽热区：每条分隔竖线各 ±3pt，后注册于列点击格使拖拽优先。
        // sep i = columns[i]|columns[i+1] 分隔线 = fixed[i] 的列左缘。
        let sep_xs: Vec<f32> = layout.fixed.iter().map(|(_, left, _)| *left).collect();
        let mut hovered_sep: Option<usize> = None;
        for (i, &x) in sep_xs.iter().enumerate() {
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(x - 3.0, header_rect.top()),
                egui::pos2(x + 3.0, header_rect.bottom()),
            );
            let response = ui.interact(
                drag_rect,
                ui.id().with(("fm-col-sep", i)),
                egui::Sense::drag(),
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                hovered_sep = Some(i);
            }
            if response.dragged() {
                let dx = response.drag_motion().x;
                if dx != 0.0 {
                    panel.drag_column_sep(i, dx, header_rect);
                }
            }
        }
        let line_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        paint_column_separators(&painter, header_rect, line_color, &layout);
        if let Some(i) = hovered_sep {
            let strong = ui.visuals().widgets.active.bg_stroke.color;
            painter.vline(
                sep_xs[i],
                header_rect.y_range(),
                egui::Stroke::new(1.0, strong),
            );
        }
        painter.hline(
            header_rect.x_range(),
            header_rect.bottom(),
            egui::Stroke::new(1.0, line_color),
        );
        for (pos, align, text) in texts {
            painter.text(pos, align, text, font_id.clone(), ui.visuals().text_color());
        }
        if let Some(kind) = clicked {
            if let Some(key) = kind.sort_key() {
                panel.toggle_sort(key);
            }
        }
    }

    /// 明细列表：行 0 = 「..」上级行（恒居行首，根目录禁用），其余来自
    /// `FsPanel::rows()`（目录优先 + 排序 + 过滤）。固定行高 + show_rows 虚拟化；
    /// 键盘移动焦点按 Explorer 最小滚动语义揭示。
    fn render_list(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        let rows = self.panels[idx].rows();
        let row_count = rows.len() + 1;
        // 行间不留缝（Explorer/WinRAR 式紧密列表）：item_spacing.y 归零后
        // 行距 = ROW_HEIGHT。注意 show_rows 在调用时捕获本 ui 的 item_spacing
        // 计算 pitch，行内 advance_cursor_after_rect 也读同一值，三处天然一致。
        ui.spacing_mut().item_spacing.y = 0.0;
        let row_pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let panel = &mut self.panels[idx];
        panel.last_row_pitch = row_pitch;
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        // 标签切换的滚动恢复（精确偏移，优先于焦点揭示——restore 时
        // focus_scroll_pending 已置 false，两者不会同时触发）。
        if let Some(offset) = panel.take_pending_scroll_restore() {
            if offset > 0.0 {
                area = area.vertical_scroll_offset(offset);
            }
        }
        if panel.focus_scroll_pending {
            panel.focus_scroll_pending = false;
            if panel.last_viewport_height > 0.0 {
                if let Some(focus) = panel.focus {
                    if focus < row_count {
                        let new_offset = min_scroll_to_reveal(
                            panel.last_scroll_offset,
                            panel.last_viewport_height,
                            focus as f32 * row_pitch,
                        );
                        if (new_offset - panel.last_scroll_offset).abs() > 0.01 {
                            area = area.vertical_scroll_offset(new_offset);
                        }
                    }
                }
            }
        }
        let active = self.active == idx;
        let output = area.show_rows(ui, ROW_HEIGHT, row_count, |ui, range| {
            for row in range {
                // 行内交互（双击目录/「..」、右键「打开」）可触发 navigate_to /
                // refresh：entries 当场清空、rows 快照即刻失效，继续按旧行索引
                // 渲染剩余行会越界 panic（双击目录闪退的根因）。状态离开 Ready
                // 就停笔，下一帧用新行模型整帧重画。
                if !matches!(self.panels[idx].state, PanelLoadState::Ready) {
                    break;
                }
                self.render_row(ui, idx, &rows, row, active, intents);
            }
        });
        let panel = &mut self.panels[idx];
        panel.last_scroll_offset = output.state.offset.y;
        panel.last_viewport_height = output.inner_rect.height();
        // 双击空白处回上级（fm_dblclick_blank_up）：明细行整行占满宽度，
        // 空白 = 内容底以下的视口区域。
        self.blank_dblclick_up(ui, idx, &output, row_count as f32 * row_pitch, None);
        // 鼠标框选（fm_rubber_band）。
        self.rubber_band(
            ui,
            idx,
            output.inner_rect,
            output.state.offset.y,
            row_count as f32 * row_pitch,
            None,
            row_pitch,
            row_count,
        );
    }

    /// 明细列表一行：整行 allocate 交互 + 斑马纹/高亮/焦点描边 + 列分隔竖线，
    /// 名称列截断、大小/时间列右对齐固定宽。焦点描边仅焦点栏强显示。
    fn render_row(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        rows: &[usize],
        row: usize,
        active: bool,
        intents: &mut FmIntents,
    ) {
        let panel = &mut self.panels[idx];
        let is_parent = row == 0;
        let branch = panel.branch_view;
        // rows 可能是导航前的旧快照（同帧行内双击已清空 entries）：用 get
        // 防御，越界行本帧按「..」样式渲染，下一帧即被新行模型替换。
        let entry: Option<FsEntry> = if is_parent {
            None
        } else {
            rows.get(row - 1)
                .and_then(|&i| panel.entries.get(i).cloned())
        };
        let selected = entry
            .as_ref()
            .is_some_and(|e| panel.selected.contains(&e.path));
        let dir_size = entry
            .as_ref()
            .filter(|e| e.is_dir)
            .and_then(|e| panel.dir_sizes.get(&e.path).copied());
        let focused = panel.focus == Some(row);
        let at_root = panel.is_root();
        // 分支模式下「..」行 = 退出分支视图（根目录也可用）。
        let parent_enabled = branch || !at_root;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        // 斑马纹（全局行号，滚动时条纹不闪动）先铺底，再叠加选中/悬停高亮。
        if row.is_multiple_of(2) {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        }
        if selected {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        if focused {
            // 焦点行描边：活动栏强色，非活动栏弱化（对齐 Archive 视图焦点语义）。
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1.0, ui.visuals().weak_text_color())
            };
            ui.painter()
                .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
        }
        let layout = column_layout(rect.right(), panel.col_shift, &panel.columns);
        paint_column_separators(
            ui.painter(),
            rect,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
            &layout,
        );

        // 行内容：图标 + 名称；右侧固定宽列（FsPanel::columns 驱动，阶段 V；
        // 目录行大小列仅在「计算大小」已算出时显示，未命中留空）。
        let content = rect.shrink2(egui::vec2(6.0, 2.0));
        // 名称列在列块左缘前截断（跟随 col_shift），不与固定列文字叠字。
        let name_right = (layout.name_right - 6.0).max(content.min.x + 20.0);
        let name_rect =
            egui::Rect::from_min_max(content.min, egui::pos2(name_right, content.max.y));
        ui.scope_builder(egui::UiBuilder::new().max_rect(name_rect), |ui| {
            // ui.horizontal 的初始行高取 interact_size.y（全局 28pt，为工具栏
            // 按钮而设），不压回会撑爆 18pt 的内容区：文字随之下沉约 5pt 贴到
            // 条纹下缘，且 scope 结束时列表竖向光标被多推 8pt，实际行距与
            // show_rows 假定的 22pt 逐行漂移。
            ui.spacing_mut().interact_size.y = ROW_HEIGHT - 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match &entry {
                    None => {
                        let text_color = if parent_enabled {
                            ui.visuals().weak_text_color()
                        } else {
                            ui.visuals().widgets.noninteractive.fg_stroke.color
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(icons::ARROW_UP.as_str()).color(text_color),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            egui::Label::new(egui::RichText::new("..").color(text_color))
                                .selectable(false),
                        );
                    }
                    Some(e) => {
                        let mut icon_drawn = false;
                        if self.options.system_icons {
                            let kind = sys_icon_kind(&e.path, e.is_dir);
                            match self.sys_icons.lookup(&kind, false) {
                                SysIconLookup::Ready(tex) => {
                                    ui.add(
                                        egui::Image::new(&tex)
                                            .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                                    );
                                    icon_drawn = true;
                                }
                                SysIconLookup::Miss => {
                                    self.sys_icons.request(&e.path, e.is_dir, false);
                                }
                                SysIconLookup::Failed => {}
                            }
                        }
                        if !icon_drawn {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(entry_icon(e).as_str()).weak(),
                                )
                                .selectable(false),
                            );
                        }
                        // 语义着色（目录 accent / 链接斜体弱档 / 隐藏弱色 /
                        // 选中行回退强对比色，见 entry_name_rich_text）。
                        let name = entry_name_rich_text(&e.name, e, ui.visuals(), selected);
                        ui.add(egui::Label::new(name).truncate().selectable(false));
                    }
                }
                // 右对齐固定列：与表头/竖线共用 column_layout 锚点直接绘制，
                // 天然跟随 col_shift（拖分隔线时一起动）且无间距误差。不走
                // right_to_left 布局——egui 0.35 RTL 嵌套会把文字画到格子
                // 右缘之外且不读 col_shift。
                if let Some(e) = &entry {
                    let painter = ui.painter();
                    let font_id = egui::TextStyle::Body.resolve(ui.style());
                    let col = ui.visuals().weak_text_color();
                    let cy = rect.center().y;
                    for (kind, _, text_right) in &layout.fixed {
                        // 各列文本（None = 该单元格留空，如目录的大小列未算
                        // 出/文件的 Attr 无属性；Comment 列占位恒空）。
                        let (text, color) = match kind {
                            ColumnKind::Name => (None, col),
                            ColumnKind::Comment => {
                                // 注释列（阶段 W）：descript.ion 缓存按文件名
                                // 匹配；无注释留空。弱一档色与大小列区分。
                                (
                                    self.panels[idx].comments.get(&e.name).cloned(),
                                    col.gamma_multiply(0.85),
                                )
                            }
                            ColumnKind::Ext => {
                                // 扩展名列：目录与无扩展名文件留空。
                                let text = if e.is_dir { None } else { lower_ext(&e.name) };
                                (text, col)
                            }
                            ColumnKind::Size => {
                                if !e.is_dir {
                                    (e.size.map(human_size), col)
                                } else {
                                    // 目录大小比文件大小再弱一档，与精确文件
                                    // 大小区分。
                                    (dir_size.map(human_size), col.gamma_multiply(0.6))
                                }
                            }
                            ColumnKind::Mtime => {
                                (Some(format_mtime(system_time_to_unix(e.mtime))), col)
                            }
                            ColumnKind::Attr => {
                                let s = attr_string(e.is_readonly, e.is_hidden, e.is_system);
                                ((!s.is_empty()).then_some(s), col)
                            }
                        };
                        if let Some(text) = text {
                            painter.text(
                                egui::pos2(*text_right, cy),
                                egui::Align2::RIGHT_CENTER,
                                text,
                                font_id.clone(),
                                color,
                            );
                        }
                    }
                }
            });
        });
        // scope_dyn 结束时会用「内容区底 + item_spacing」改写列表竖向光标
        // （内容区比行高矮 4pt，会回退光标、行距偏离 show_rows 的假定），
        // 钉回「行底 + item_spacing」。
        ui.advance_cursor_after_rect(rect);

        // 行 = 栏间拖放的 drag source（「..」上级行不可拖）：拖动开始即设置
        // payload，落点栏判定与 drop 生效在 poll_inter_panel_dnd。
        // 拖拽与单击互斥由 egui 保证（拖动超过阈值后不产生 clicked）。
        if let Some(e) = &entry {
            let sources = drag_sources(&self.panels[idx].selected, &e.path);
            response.dnd_set_drag_payload(FmDragPayload {
                sources,
                src_panel: idx,
            });
        }

        let mods = ui.input(|i| i.modifiers);
        if response.clicked() {
            self.panels[idx].click_row(row, mods.command, mods.shift);
        }
        if response.double_clicked() {
            if is_parent {
                if branch {
                    self.panels[idx].exit_branch_view();
                } else if parent_enabled {
                    self.panels[idx].parent_dir();
                }
            } else {
                self.open_ui_row(idx, rows, row, intents);
            }
        }
        // 「..」行无右键菜单；右键框选已超阈值（band.active）时不弹
        // （egui click 判定自带位移阈值，此门是阈值不一致时的保险）。
        let band_active = self
            .band
            .as_ref()
            .is_some_and(|b| b.panel == idx && b.active);
        if !is_parent && !band_active {
            response.context_menu(|ui| {
                // Explorer 惯例：右键未选中的行先把它单选。
                let Some(e) = &entry else { return };
                if !self.panels[idx].selected.contains(&e.path) {
                    self.panels[idx].click_row(row, false, false);
                }
                self.entry_context_menu(ui, idx, rows, row, e, intents);
            });
        }
        response.on_hover_text(row_hover_tip(
            entry.as_ref(),
            branch,
            entry
                .as_ref()
                .and_then(|e| self.panels[idx].comments.get(&e.name))
                .map(String::as_str),
        ));
    }

    /// 条目右键菜单本体（明细行与网格 cell 共用；调用方负责「先单选」
    /// 前奏与「..」行不弹菜单的门槛）。
    fn entry_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        rows: &[usize],
        row: usize,
        e: &FsEntry,
        intents: &mut FmIntents,
    ) {
        if ui.button((icons::ARROW_SQUARE_OUT, " 打开")).clicked() {
            self.open_ui_row(idx, rows, row, intents);
            ui.close();
        }
        // 「打开方式…」：仅文件行 + Windows（Shell openas 动词）。
        if !e.is_dir
            && crate::platform::shell_verbs::is_supported()
            && ui.button((icons::LIST_BULLETS, " 打开方式…")).clicked()
        {
            if let Err(err) = crate::platform::shell_verbs::show_open_with(&e.path) {
                intents.op_error = Some(err);
            }
            ui.close();
        }
        let comic_openable = e.is_dir || archive_kind(&e.path).is_some();
        if comic_openable && ui.button((icons::BOOK_OPEN, " 作为漫画打开")).clicked() {
            intents.open_as_comic = Some(e.path.clone());
            ui.close();
        }
        let targets = self.op_targets(idx);
        let extract_src = match targets.as_slice() {
            [p] if archive_kind(p).is_some() => Some(p.clone()),
            _ => None,
        };
        let other_dir = match self.layout {
            PanelLayout::Dual { .. } => Some(self.panels[1 - idx].dir.clone()),
            PanelLayout::Single { .. } => None,
        };
        let (extract_label, extract_dest) = match other_dir {
            Some(d) if d != self.panels[idx].dir => ("另一栏", d),
            _ => ("当前目录", self.panels[idx].dir.clone()),
        };
        if ui
            .add_enabled(
                extract_src.is_some(),
                egui::Button::new((icons::EXPORT, format!(" 解压到{extract_label}…"))),
            )
            .clicked()
        {
            if let Some(src) = extract_src {
                intents.extract = Some((src, extract_dest));
            }
            ui.close();
        }
        ui.separator();
        if ui.button((icons::COPY, " 复制路径")).clicked() {
            ui.ctx().copy_text(e.path.display().to_string());
            ui.close();
        }
        if ui.button((icons::ARROW_CLOCKWISE, " 刷新")).clicked() {
            self.panels[idx].refresh();
            ui.close();
        }
        if ui.button((icons::MAGNIFYING_GLASS, " 搜索…")).clicked() {
            self.open_search_dialog();
            ui.close();
        }
        // 「同步目录…」（阶段 AE）：双栏且两栏目录不同才显示（同顶栏按钮）。
        let sync_available = matches!(self.layout, PanelLayout::Dual { .. })
            && self.panels[0].dir != self.panels[1].dir;
        if sync_available && ui.button((icons::ARROWS_CLOCKWISE, " 同步目录…")).clicked() {
            let l = self.panels[0].dir.clone();
            let r = self.panels[1].dir.clone();
            self.sync_dialog.open_with(l, r);
            ui.close();
        }
        let dir_targets: Vec<PathBuf> = self.panels[idx]
            .entries
            .iter()
            .filter(|e| e.is_dir && targets.contains(&e.path))
            .map(|e| e.path.clone())
            .collect();
        if ui
            .add_enabled(
                !dir_targets.is_empty(),
                egui::Button::new((icons::GAUGE, " 计算大小")),
            )
            .clicked()
        {
            self.panels[idx].request_dir_sizes(dir_targets);
            ui.close();
        }
        // 「校验和…」（阶段 AC）：选中集含文件时可用；选中集恰为单个校验
        // 文件（.md5/.sfv/.sha1/.sha256）时直接进验证模式。
        let file_targets: Vec<PathBuf> = self.panels[idx]
            .entries
            .iter()
            .filter(|e| !e.is_dir && targets.contains(&e.path))
            .map(|e| e.path.clone())
            .collect();
        let checksum_file = match targets.as_slice() {
            [p] if p
                .file_name()
                .is_some_and(|n| is_checksum_file_name(&n.to_string_lossy()))
                && p.is_file() =>
            {
                Some(p.clone())
            }
            _ => None,
        };
        if ui
            .add_enabled(
                checksum_file.is_some() || !file_targets.is_empty(),
                egui::Button::new((icons::FINGERPRINT, " 校验和…")),
            )
            .clicked()
        {
            if let Some(p) = checksum_file {
                self.checksum.open_verify(p);
            } else {
                self.checksum.open_compute(file_targets);
            }
            ui.close();
        }
        ui.separator();
        if ui.button((icons::PENCIL_SIMPLE, " 重命名")).clicked() {
            self.dialog = Some(FmDialog::Rename(RenameDialog::new(e.path.clone())));
            ui.close();
        }
        // 编辑注释（阶段 W）：descript.ion 按文件名匹配当前目录——分支视图
        // 子目录项（rel_dir 非空）的注释在其各自目录，这里不接盘（禁用）。
        if ui
            .add_enabled(
                e.rel_dir.is_empty(),
                egui::Button::new((icons::NOTE, " 编辑注释…")),
            )
            .on_hover_text("编辑 descript.ion 注释（Ctrl+Shift+Z）")
            .clicked()
        {
            self.comment_dialog = Some(CommentDialog::new(
                idx,
                e.name.clone(),
                self.panels[idx].comments.get(&e.name).map(String::as_str),
            ));
            ui.close();
        }
        if ui
            .button((icons::PENCIL_SIMPLE_LINE, " 批量重命名…"))
            .clicked()
        {
            self.open_multi_rename_dialog(idx);
            ui.close();
        }
        let dual = matches!(self.layout, PanelLayout::Dual { .. });
        let dest_label = if dual { "另一栏" } else { "当前目录" };
        if ui
            .button((icons::COPY, format!(" 复制到{dest_label}…")))
            .clicked()
        {
            let targets = self.op_targets(idx);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Copy, targets, idx);
            }
            ui.close();
        }
        if ui
            .button((icons::EXPORT, format!(" 移动到{dest_label}…")))
            .clicked()
        {
            let targets = self.op_targets(idx);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Move, targets, idx);
            }
            ui.close();
        }
        if ui.button((icons::PACKAGE, " 压缩为 zip…")).clicked() {
            let targets = self.op_targets(idx);
            if !targets.is_empty() {
                self.open_compress_dialog(targets, idx);
            }
            ui.close();
        }
        // 「分割…」（阶段 AD）：选中集恰为单个文件时可用。
        let split_src = match targets.as_slice() {
            [p] if p.is_file() => Some(p.clone()),
            _ => None,
        };
        if ui
            .add_enabled(
                split_src.is_some(),
                egui::Button::new((icons::SCISSORS, " 分割…")),
            )
            .clicked()
        {
            if let Some(src) = split_src {
                let dest = self.panels[idx].dir.clone();
                self.dialog = Some(FmDialog::Split(SplitDialog::new(src, &dest)));
            }
            ui.close();
        }
        // 「合并…」（阶段 AD）：单个 .001 或一组同目录同前缀 .NNN 分块；
        // 缺号/目标已存在直接报错（intents.op_error），不经对话框。
        let merge_base = merge_target_base(&targets);
        if ui
            .add_enabled(
                merge_base.is_some(),
                egui::Button::new((icons::ARROWS_MERGE, " 合并…")),
            )
            .clicked()
        {
            if let Some((dir, base)) = merge_base {
                match collect_chunks(&dir, &base) {
                    Ok(chunks) => {
                        let dest = dir.join(&base);
                        if dest.exists() {
                            intents.op_error = Some(format!("合并目标已存在：{}", dest.display()));
                        } else {
                            self.ops.start_merge(chunks, dest);
                        }
                    }
                    Err(e) => intents.op_error = Some(e),
                }
            }
            ui.close();
        }
        if ui.button((icons::FOLDER_PLUS, " 新建文件夹")).clicked() {
            let parent = self.panels[idx].dir.clone();
            let suggested = suggest_folder_name(&parent);
            self.dialog = Some(FmDialog::NewDir(NewDirDialog::new(parent, suggested)));
            ui.close();
        }
        if ui.button((icons::FILE_PLUS, " 新建文本文件")).clicked() {
            self.open_new_file_dialog(idx);
            ui.close();
        }
        ui.separator();
        if ui.button((icons::TRASH, " 删除")).clicked() {
            let targets = self.op_targets(idx);
            if !targets.is_empty() {
                // 右键菜单无 Shift 语境，用设置基档。
                let permanent = self.options.delete_permanent;
                if self.options.confirm_delete {
                    self.dialog = Some(FmDialog::Delete(DeleteDialog::new(targets, permanent)));
                } else {
                    self.start_delete(targets, permanent);
                }
            }
            ui.close();
        }
        // 「选择」子菜单（阶段 U）：保存/恢复选择集（会话内，不落盘）。
        ui.separator();
        egui::containers::menu::SubMenuButton::new((icons::SELECTION, " 选择")).ui(ui, |ui| {
            ui.set_min_width(200.0);
            let selected: Vec<PathBuf> = self.panels[idx].selected.iter().cloned().collect();
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new("保存当前选择…"))
                .clicked()
            {
                self.selection_dialog = Some(SaveSelectionDialog::new(selected));
                ui.close();
            }
            ui.separator();
            if self.saved_selections.is_empty() {
                ui.label(egui::RichText::new("（无已存选择集）").weak());
            }
            let mut restore: Option<usize> = None;
            let mut delete: Option<usize> = None;
            for (i, (name, paths)) in self.saved_selections.iter().enumerate() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // ✕ 删除不收起菜单（同书签菜单先例）。
                    if ui
                        .add(egui::Button::new(icons::X.as_str()).frame(false))
                        .clicked()
                    {
                        delete = Some(i);
                    }
                    if ui.button(format!("{name}（{} 项）", paths.len())).clicked() {
                        restore = Some(i);
                        ui.close();
                    }
                });
            }
            if let Some(i) = delete {
                self.saved_selections.remove(i);
            }
            if let Some(i) = restore {
                let paths = self.saved_selections[i].1.clone();
                if let Some(msg) = self.restore_selection(idx, &paths) {
                    intents.op_error = Some(msg);
                }
                ui.close();
            }
        });
        // 「修改属性/时间戳…」（阶段 AC）：应用内可编辑版，区别于末尾的
        // 系统属性框（全平台可用；unix 仅只读位有效）。
        if ui
            .add_enabled(
                !targets.is_empty(),
                egui::Button::new((icons::CLOCK_USER, " 修改属性/时间戳…")),
            )
            .clicked()
        {
            self.attr_dialog = Some(AttrTimestampDialog::new(targets.clone()));
            ui.close();
        }
        // 「属性」：末尾（Explorer 惯例；非 Windows 隐藏）。
        if crate::platform::shell_verbs::is_supported() {
            ui.separator();
            if ui.button((icons::INFO, " 属性")).clicked() {
                if let Err(err) = crate::platform::shell_verbs::show_properties(&e.path) {
                    intents.op_error = Some(err);
                }
                ui.close();
            }
        }
    }

    /// 缩略图网格：cell 176×200pt（160 缩略图区 + 两行名称）；列数 =
    /// 栏宽 / cell 宽（≥1）；show_rows 按网格行虚拟化（一行 = 一排
    /// cell）。焦点/选中仍是线性 UI 行索引（行 0 = 「..」cell），
    /// 过滤/排序/type-ahead/分支视图全部不受影响（网格只是渲染层）。
    fn render_grid(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        let rows = self.panels[idx].rows();
        let item_count = rows.len() + 1;
        let cols = grid_cols(ui.available_width());
        let grid_rows = grid_row_count(item_count, cols);
        ui.spacing_mut().item_spacing.y = 0.0;
        let panel = &mut self.panels[idx];
        panel.last_grid_cols = cols;
        // PgUp/PgDn 步进换算基准：网格行高。
        panel.last_row_pitch = THUMB_CELL_H;
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        // 标签切换的滚动恢复（同 render_list）。
        if let Some(offset) = panel.take_pending_scroll_restore() {
            if offset > 0.0 {
                area = area.vertical_scroll_offset(offset);
            }
        }
        // 焦点揭示：线性行号先换算所在网格行。
        if panel.focus_scroll_pending {
            panel.focus_scroll_pending = false;
            if panel.last_viewport_height > 0.0 {
                if let Some(focus) = panel.focus {
                    if focus < item_count {
                        let new_offset = min_scroll_to_reveal(
                            panel.last_scroll_offset,
                            panel.last_viewport_height,
                            grid_row_of(focus, cols) as f32 * THUMB_CELL_H,
                        );
                        if (new_offset - panel.last_scroll_offset).abs() > 0.01 {
                            area = area.vertical_scroll_offset(new_offset);
                        }
                    }
                }
            }
        }
        let active = self.active == idx;
        let output = area.show_rows(ui, THUMB_CELL_H, grid_rows, |ui, range| {
            for grid_row in range {
                // 同 render_list：行内交互（双击目录/「..」、右键「打开」）
                // 可触发导航当场清空 entries，旧 rows 快照即刻失效，状态
                // 离开 Ready 就停笔。
                if !matches!(self.panels[idx].state, PanelLoadState::Ready) {
                    break;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for col in 0..cols {
                        let row = grid_row * cols + col;
                        if row >= item_count {
                            break;
                        }
                        self.render_grid_cell(ui, idx, &rows, row, active, intents);
                    }
                });
            }
        });
        let panel = &mut self.panels[idx];
        panel.last_scroll_offset = output.state.offset.y;
        panel.last_viewport_height = output.inner_rect.height();
        // 可见范围变化：bump 缩略图请求代次——worker 解码前比对，快速
        // 滚动时过期请求（已不可见 cell）直接丢弃不浪费解码。
        let first = (output.state.offset.y / THUMB_CELL_H).floor().max(0.0) as usize;
        let last =
            ((output.state.offset.y + output.inner_rect.height()) / THUMB_CELL_H).ceil() as usize;
        let visible = (cols, first, last);
        if self.thumb_visible[idx] != Some(visible) {
            self.thumb_visible[idx] = Some(visible);
            self.thumbs.bump_generation();
            self.sys_icons.bump_generation();
        }
        // 双击空白处回上级（fm_dblclick_blank_up）：cell 定宽左排，每行右侧
        // 余量与末行未排满部分都算空白（几何见 GridBlankGeom）。
        let full_rows = item_count / cols;
        let partial = item_count % cols;
        let geom = GridBlankGeom {
            row_pitch: THUMB_CELL_H,
            full_rows,
            full_width: cols as f32 * THUMB_CELL_W,
            last_width: if partial == 0 {
                cols as f32 * THUMB_CELL_W
            } else {
                partial as f32 * THUMB_CELL_W
            },
        };
        self.blank_dblclick_up(
            ui,
            idx,
            &output,
            grid_rows as f32 * THUMB_CELL_H,
            Some(geom),
        );
        // 鼠标框选（fm_rubber_band）。
        self.rubber_band(
            ui,
            idx,
            output.inner_rect,
            output.state.offset.y,
            grid_rows as f32 * THUMB_CELL_H,
            Some((geom, cols, THUMB_CELL_W)),
            THUMB_CELL_H,
            item_count,
        );
    }

    /// 简表（Brief，阶段 V）：多列排布的紧凑名称行（图标 + 单行截断
    /// 名称，行高 = ROW_HEIGHT），行主序——与缩略图网格同一套线性行号/
    /// 键盘步进/框选/空白双击机制（last_grid_cols/GridBlankGeom），仅
    /// cell 几何不同。列宽由可见条目的最长名称字符量估算
    /// （brief_col_width），列数 = 栏宽 / 列宽（brief_cols）。
    fn render_brief(&mut self, ui: &mut egui::Ui, idx: usize, intents: &mut FmIntents) {
        let rows = self.panels[idx].rows();
        let item_count = rows.len() + 1;
        let max_units = rows
            .iter()
            .filter_map(|&i| self.panels[idx].entries.get(i))
            .map(|e| name_units(&e.name))
            .max()
            .unwrap_or(4);
        let cell_w = brief_col_width(max_units);
        let cols = brief_cols(ui.available_width(), cell_w);
        let grid_rows = grid_row_count(item_count, cols);
        ui.spacing_mut().item_spacing.y = 0.0;
        let panel = &mut self.panels[idx];
        panel.last_grid_cols = cols;
        // PgUp/PgDn 步进与焦点滚动定位基准：简表行高。
        panel.last_row_pitch = ROW_HEIGHT;
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        // 标签切换的滚动恢复（同 render_list/render_grid）。
        if let Some(offset) = panel.take_pending_scroll_restore() {
            if offset > 0.0 {
                area = area.vertical_scroll_offset(offset);
            }
        }
        // 焦点揭示：线性行号先换算所在行（同网格）。
        if panel.focus_scroll_pending {
            panel.focus_scroll_pending = false;
            if panel.last_viewport_height > 0.0 {
                if let Some(focus) = panel.focus {
                    if focus < item_count {
                        let new_offset = min_scroll_to_reveal(
                            panel.last_scroll_offset,
                            panel.last_viewport_height,
                            grid_row_of(focus, cols) as f32 * ROW_HEIGHT,
                        );
                        if (new_offset - panel.last_scroll_offset).abs() > 0.01 {
                            area = area.vertical_scroll_offset(new_offset);
                        }
                    }
                }
            }
        }
        let active = self.active == idx;
        let output = area.show_rows(ui, ROW_HEIGHT, grid_rows, |ui, range| {
            for grid_row in range {
                // 同 render_list：行内交互可触发导航当场清空 entries，旧
                // rows 快照即刻失效，状态离开 Ready 就停笔。
                if !matches!(self.panels[idx].state, PanelLoadState::Ready) {
                    break;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for col in 0..cols {
                        let row = grid_row * cols + col;
                        if row >= item_count {
                            break;
                        }
                        self.render_brief_cell(ui, idx, &rows, row, cell_w, active, intents);
                    }
                });
            }
        });
        let panel = &mut self.panels[idx];
        panel.last_scroll_offset = output.state.offset.y;
        panel.last_viewport_height = output.inner_rect.height();
        // 双击空白处回上级与框选：cell 定宽左排，几何同网格（仅 cell
        // 宽/高不同）。
        let full_rows = item_count / cols;
        let partial = item_count % cols;
        let geom = GridBlankGeom {
            row_pitch: ROW_HEIGHT,
            full_rows,
            full_width: cols as f32 * cell_w,
            last_width: if partial == 0 {
                cols as f32 * cell_w
            } else {
                partial as f32 * cell_w
            },
        };
        self.blank_dblclick_up(ui, idx, &output, grid_rows as f32 * ROW_HEIGHT, Some(geom));
        self.rubber_band(
            ui,
            idx,
            output.inner_rect,
            output.state.offset.y,
            grid_rows as f32 * ROW_HEIGHT,
            Some((geom, cols, cell_w)),
            ROW_HEIGHT,
            item_count,
        );
    }

    /// 简表一个 cell：图标（16pt 系统图标或字体图标）+ 单行截断名称；
    /// 选中/悬停/焦点视觉与单击/双击/右键/拖动语义同网格 cell、明细行。
    #[allow(clippy::too_many_arguments)]
    fn render_brief_cell(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        rows: &[usize],
        row: usize,
        cell_w: f32,
        active: bool,
        intents: &mut FmIntents,
    ) {
        let is_parent = row == 0;
        // rows 可能是导航前的旧快照（同帧行内双击已清空 entries）：get 防御。
        let entry: Option<FsEntry> = if is_parent {
            None
        } else {
            rows.get(row - 1)
                .and_then(|&i| self.panels[idx].entries.get(i).cloned())
        };
        let branch = self.panels[idx].branch_view;
        let parent_enabled = branch || !self.panels[idx].is_root();
        let selected = entry
            .as_ref()
            .is_some_and(|e| self.panels[idx].selected.contains(&e.path));
        let focused = self.panels[idx].focus == Some(row);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(cell_w, ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter().clone();
        if selected {
            painter.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        if focused {
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1.0, ui.visuals().weak_text_color())
            };
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
        }

        let cy = rect.center().y;
        let weak = ui.visuals().weak_text_color();
        // 图标：16pt（同明细行），「..」cell 用 ARROW_UP。
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 6.0 + 8.0, cy),
            egui::vec2(16.0, 16.0),
        );
        match &entry {
            None => {
                let color = if parent_enabled {
                    weak
                } else {
                    ui.visuals().widgets.noninteractive.fg_stroke.color
                };
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icons::ARROW_UP.as_str(),
                    egui::FontId::proportional(13.0),
                    color,
                );
            }
            Some(e) => {
                let mut icon_drawn = false;
                if self.options.system_icons {
                    let kind = sys_icon_kind(&e.path, e.is_dir);
                    match self.sys_icons.lookup(&kind, false) {
                        SysIconLookup::Ready(tex) => {
                            painter.image(
                                tex.id(),
                                icon_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            icon_drawn = true;
                        }
                        SysIconLookup::Miss => {
                            self.sys_icons.request(&e.path, e.is_dir, false);
                        }
                        SysIconLookup::Failed => {}
                    }
                }
                if !icon_drawn {
                    painter.text(
                        icon_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        entry_icon(e).as_str(),
                        egui::FontId::proportional(13.0),
                        weak,
                    );
                }
            }
        }
        // 名称：单行截断（brief_cell_name），语义着色与明细行共用
        // entry_name_rich_text；超出 cell 右缘裁剪。
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 6.0 + 16.0 + 4.0, rect.top()),
            egui::pos2(rect.right() - 4.0, rect.bottom()),
        );
        let name_text = match &entry {
            None => {
                let color = if parent_enabled {
                    ui.visuals().text_color()
                } else {
                    ui.visuals().widgets.noninteractive.fg_stroke.color
                };
                egui::RichText::new("..").color(color)
            }
            Some(e) => {
                let truncated = brief_cell_name(&e.name, cell_w);
                entry_name_rich_text(&truncated, e, ui.visuals(), selected)
            }
        };
        let galley = egui::WidgetText::RichText(std::sync::Arc::new(name_text)).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            name_rect.width(),
            egui::FontSelection::Default,
        );
        painter.with_clip_rect(name_rect).galley(
            egui::pos2(name_rect.left(), cy - galley.size().y / 2.0),
            galley,
            ui.visuals().text_color(),
        );

        // 交互与网格 cell/明细行一致：cell = 栏间拖放的 drag source。
        if let Some(e) = &entry {
            let sources = drag_sources(&self.panels[idx].selected, &e.path);
            response.dnd_set_drag_payload(FmDragPayload {
                sources,
                src_panel: idx,
            });
        }
        let mods = ui.input(|i| i.modifiers);
        if response.clicked() {
            self.panels[idx].click_row(row, mods.command, mods.shift);
        }
        if response.double_clicked() {
            if is_parent {
                if branch {
                    self.panels[idx].exit_branch_view();
                } else if parent_enabled {
                    self.panels[idx].parent_dir();
                }
            } else {
                self.open_ui_row(idx, rows, row, intents);
            }
        }
        // 「..」cell 无右键菜单；右键框选已超阈值时不弹（同明细行门控）。
        let band_active = self
            .band
            .as_ref()
            .is_some_and(|b| b.panel == idx && b.active);
        if !is_parent && !band_active {
            response.context_menu(|ui| {
                // Explorer 惯例：右键未选中的项先把它单选。
                let Some(e) = &entry else { return };
                if !self.panels[idx].selected.contains(&e.path) {
                    self.panels[idx].click_row(row, false, false);
                }
                self.entry_context_menu(ui, idx, rows, row, e, intents);
            });
        }
        response.on_hover_text(row_hover_tip(
            entry.as_ref(),
            branch,
            entry
                .as_ref()
                .and_then(|e| self.panels[idx].comments.get(&e.name))
                .map(String::as_str),
        ));
    }

    /// 双击空白处回上级（fm_dblclick_blank_up）：本帧主键双击命中空白区
    /// （判定见 dblclick_hits_blank）即 parent_dir()；命中行/cell 的双击
    /// 由各自行响应消费，几何上不落在空白区，互不冲突。
    fn blank_dblclick_up(
        &mut self,
        ui: &egui::Ui,
        idx: usize,
        output: &egui::scroll_area::ScrollAreaOutput<()>,
        content_height: f32,
        grid: Option<GridBlankGeom>,
    ) {
        if !self.options.dblclick_blank_up {
            return;
        }
        let dbl = ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        });
        if !dbl {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        if dblclick_hits_blank(
            output.inner_rect,
            output.state.offset.y,
            content_height,
            grid,
            pos,
        ) {
            self.panels[idx].parent_dir();
        }
    }

    /// 鼠标框选（fm_rubber_band，阶段 U）：render_list/render_brief/
    /// render_grid 帧尾调用。启动判定——"right" = 右键视口内按下（单击
    /// 未超阈值仍弹上下文菜单，行渲染处另有 band.active 门控保险）；
    /// "left" = 左键空白区按下（行/cell 上左键起拖维持拖放 payload，靠
    /// dblclick_hits_blank 同一几何判定排除）。拖动中只画半透明矩形
    /// （从简：不实时改选中）；松开时按命中行/cell 应用选中：无修饰 =
    /// 替换、Shift = 追加、Ctrl = 切换，**不更新 anchor/focus**（框选是
    /// 区域语义，不参与 Shift+点击锚点区间）。「..」行/cell（索引 0）
    /// 不进选中。grid = Some((几何, 列数, cell 宽)) 时按 cell 命中
    /// （简表/缩略图），None 按整行命中（明细列表）。
    #[allow(clippy::too_many_arguments)]
    fn rubber_band(
        &mut self,
        ui: &egui::Ui,
        idx: usize,
        viewport: egui::Rect,
        scroll_y: f32,
        content_height: f32,
        grid: Option<(GridBlankGeom, usize, f32)>,
        pitch: f32,
        item_count: usize,
    ) {
        let mode = self.options.rubber_band;
        if self.band.is_none() && mode != RubberBandMode::Off {
            let secondary = mode == RubberBandMode::Right
                && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
            let primary = mode == RubberBandMode::Left
                && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            if secondary || primary {
                if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                    let blank_ok = secondary
                        || dblclick_hits_blank(
                            viewport,
                            scroll_y,
                            content_height,
                            grid.map(|(g, _, _)| g),
                            origin,
                        );
                    if viewport.contains(origin) && blank_ok {
                        self.band = Some(RubberBand {
                            panel: idx,
                            button: if secondary {
                                egui::PointerButton::Secondary
                            } else {
                                egui::PointerButton::Primary
                            },
                            origin,
                            active: false,
                        });
                    }
                }
            }
        }
        let Some(band) = &mut self.band else {
            return;
        };
        if band.panel != idx {
            return;
        }
        let button = band.button;
        let (down, released, pos) = ui.input(|i| {
            (
                i.pointer.button_down(button),
                i.pointer.button_released(button),
                i.pointer.latest_pos(),
            )
        });
        // 丢失的松开（拖到窗口外释放等）：清状态防滞留。
        if !down && !released {
            self.band = None;
            return;
        }
        if let Some(p) = pos {
            if !band.active && (p - band.origin).length() > BAND_THRESHOLD {
                band.active = true;
            }
            if band.active && !released {
                let rect = egui::Rect::from_two_pos(band.origin, p);
                let painter = ui.painter().with_clip_rect(viewport);
                let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.25);
                painter.rect_filled(rect, 2.0, fill);
                painter.rect_stroke(
                    rect,
                    2.0,
                    ui.visuals().selection.stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        if released {
            let Some(band) = self.band.take() else { return };
            if !band.active {
                return;
            }
            let Some(p) = pos else { return };
            let to_content =
                |q: egui::Pos2| egui::pos2(q.x - viewport.min.x, q.y - viewport.min.y + scroll_y);
            let rect = egui::Rect::from_two_pos(to_content(band.origin), to_content(p));
            let hits = match grid {
                None => rows_in_rect(item_count, pitch, rect),
                Some((g, cols, cell_w)) => {
                    cells_in_rect(item_count, cols, cell_w, g.row_pitch, rect)
                }
            };
            self.apply_band(idx, hits, ui.input(|i| i.modifiers));
        }
    }

    /// 框选命中应用选中：无修饰 = 替换、Shift = 追加、Ctrl = 切换；
    /// 「..」行（0）跳过；anchor/focus 不动（见 rubber_band 注释）。
    fn apply_band(&mut self, idx: usize, hits: Vec<usize>, mods: egui::Modifiers) {
        let panel = &mut self.panels[idx];
        if !mods.shift && !mods.command {
            panel.selected.clear();
        }
        for r in hits {
            if r == 0 {
                continue;
            }
            if let Some(path) = panel.row_path(r) {
                if mods.command {
                    if !panel.selected.insert(path.clone()) {
                        panel.selected.remove(&path);
                    }
                } else {
                    panel.selected.insert(path);
                }
            }
        }
    }

    /// 恢复选择集到栏 idx（阶段 U）：替换式选中——按路径匹配当前栏可见
    /// 项（已不存在/不在当前目录的忽略，返回提示消息）；焦点设到首个
    /// 命中行并请求滚动揭示。anchor 不动（同框选语义）。
    fn restore_selection(&mut self, idx: usize, paths: &[PathBuf]) -> Option<String> {
        let panel = &mut self.panels[idx];
        panel.selected.clear();
        let rows = panel.rows();
        let mut hits = 0usize;
        let mut first_row = None;
        for (off, &ei) in rows.iter().enumerate() {
            let Some(e) = panel.entries.get(ei) else {
                continue;
            };
            if paths.iter().any(|p| p == &e.path) {
                panel.selected.insert(e.path.clone());
                hits += 1;
                if first_row.is_none() {
                    first_row = Some(off + 1);
                }
            }
        }
        if let Some(r) = first_row {
            panel.focus = Some(r);
            panel.focus_scroll_pending = true;
        }
        let missing = paths.len() - hits;
        (missing > 0).then(|| format!("{missing} 项不在当前目录，已忽略"))
    }

    /// 网格一个 cell：选中/悬停底色 + 焦点描边（同明细行视觉语言），
    /// 居中缩略图/大字体图标 + 底部两行截断名称；单击/双击/右键/拖动
    /// （栏间 dnd payload）语义与明细行一致。
    fn render_grid_cell(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        rows: &[usize],
        row: usize,
        active: bool,
        intents: &mut FmIntents,
    ) {
        let is_parent = row == 0;
        // rows 可能是导航前的旧快照（同帧行内双击已清空 entries）：get 防御。
        let entry: Option<FsEntry> = if is_parent {
            None
        } else {
            rows.get(row - 1)
                .and_then(|&i| self.panels[idx].entries.get(i).cloned())
        };
        let branch = self.panels[idx].branch_view;
        // 分支模式下「..」cell = 退出分支视图（根目录也可用）。
        let parent_enabled = branch || !self.panels[idx].is_root();
        let selected = entry
            .as_ref()
            .is_some_and(|e| self.panels[idx].selected.contains(&e.path));
        let focused = self.panels[idx].focus == Some(row);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(THUMB_CELL_W, THUMB_CELL_H),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
        }
        if focused {
            // 焦点描边：活动栏强色，非活动栏弱化（对齐明细行焦点语义）。
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1.0, ui.visuals().weak_text_color())
            };
            painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
        }

        // 160×160 缩略图区（水平居中）。
        let thumb_side = THUMB_MAX_DIM as f32;
        let thumb_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 6.0 + thumb_side / 2.0),
            egui::vec2(thumb_side, thumb_side),
        );
        let weak = ui.visuals().weak_text_color();
        match &entry {
            None => {
                painter.text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icons::ARROW_UP.as_str(),
                    egui::FontId::proportional(48.0),
                    weak,
                );
            }
            Some(e) if e.is_dir => {
                let mut drawn = false;
                if self.options.system_icons {
                    let kind = sys_icon_kind(&e.path, true);
                    match self.sys_icons.lookup(&kind, true) {
                        SysIconLookup::Ready(tex) => {
                            painter.image(
                                tex.id(),
                                egui::Rect::from_center_size(
                                    thumb_rect.center(),
                                    egui::vec2(32.0, 32.0),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            drawn = true;
                        }
                        SysIconLookup::Miss => self.sys_icons.request(&e.path, true, true),
                        SysIconLookup::Failed => {}
                    }
                }
                if !drawn {
                    painter.text(
                        thumb_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icons::FOLDER.as_str(),
                        egui::FontId::proportional(72.0),
                        weak,
                    );
                }
            }
            Some(e) => {
                // 图片扩展名判别用 is_comic_image_name（额外排除 macOS
                // `._*` AppleDouble 垃圾文件，防无意义解码；
                // is_previewable_name 含文本扩展名，不适用）。
                let is_image = openitgo_parser::traits::is_comic_image_name(&e.name);
                let mut drawn = false;
                if is_image {
                    let key: ThumbKey = (e.mtime, e.size.unwrap_or(0));
                    match self.thumbs.lookup(&e.path, &key) {
                        ThumbLookup::Ready(tex) => {
                            let size = tex.size_vec2();
                            let scale = (thumb_side / size.x).min(thumb_side / size.y).min(1.0);
                            let fit =
                                egui::Rect::from_center_size(thumb_rect.center(), size * scale);
                            painter.image(
                                tex.id(),
                                fit,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            drawn = true;
                        }
                        // 未缓存：请求（可见 cell 才会渲染到这里）+ 占位图标。
                        ThumbLookup::Miss => self.thumbs.request(e.path.clone(), key),
                        // 解码失败/非图片：已记忆，回退大图标。
                        ThumbLookup::Failed => {}
                    }
                }
                if !drawn && self.options.system_icons {
                    let kind = sys_icon_kind(&e.path, false);
                    match self.sys_icons.lookup(&kind, true) {
                        SysIconLookup::Ready(tex) => {
                            painter.image(
                                tex.id(),
                                egui::Rect::from_center_size(
                                    thumb_rect.center(),
                                    egui::vec2(32.0, 32.0),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            drawn = true;
                        }
                        SysIconLookup::Miss => self.sys_icons.request(&e.path, false, true),
                        SysIconLookup::Failed => {}
                    }
                }
                if !drawn {
                    let icon = if is_image {
                        icons::FILE_IMAGE
                    } else {
                        entry_icon(e)
                    };
                    painter.text(
                        thumb_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon.as_str(),
                        egui::FontId::proportional(48.0),
                        weak,
                    );
                }
            }
        }
        // 名称：底部两行区，字符量按两行截断（truncate_cell_name），
        // 像素折行交给 layout wrap；水平居中、超出区域裁剪。语义着色与
        // 明细行共用 entry_name_rich_text（「..」cell 维持现状配色）。
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, thumb_rect.bottom() + 4.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
        );
        let name_text = match &entry {
            None => {
                let color = if parent_enabled {
                    ui.visuals().text_color()
                } else {
                    ui.visuals().widgets.noninteractive.fg_stroke.color
                };
                egui::RichText::new("..").color(color)
            }
            Some(e) => {
                let truncated = truncate_cell_name(&e.name, 24);
                entry_name_rich_text(&truncated, e, ui.visuals(), selected)
            }
        };
        let galley = egui::WidgetText::RichText(std::sync::Arc::new(name_text)).into_galley(
            ui,
            Some(egui::TextWrapMode::Wrap),
            name_rect.width(),
            egui::FontSelection::Default,
        );
        painter.with_clip_rect(name_rect).galley(
            egui::pos2(
                name_rect.center().x - galley.size().x / 2.0,
                name_rect.top(),
            ),
            galley,
            ui.visuals().text_color(),
        );

        // 交互与明细行一致：cell = 栏间拖放的 drag source（「..」不可拖）。
        if let Some(e) = &entry {
            let sources = drag_sources(&self.panels[idx].selected, &e.path);
            response.dnd_set_drag_payload(FmDragPayload {
                sources,
                src_panel: idx,
            });
        }
        let mods = ui.input(|i| i.modifiers);
        if response.clicked() {
            self.panels[idx].click_row(row, mods.command, mods.shift);
        }
        if response.double_clicked() {
            if is_parent {
                if branch {
                    self.panels[idx].exit_branch_view();
                } else if parent_enabled {
                    self.panels[idx].parent_dir();
                }
            } else {
                self.open_ui_row(idx, rows, row, intents);
            }
        }
        // 「..」cell 无右键菜单；右键框选已超阈值时不弹（同明细行门控）。
        let band_active = self
            .band
            .as_ref()
            .is_some_and(|b| b.panel == idx && b.active);
        if !is_parent && !band_active {
            response.context_menu(|ui| {
                // Explorer 惯例：右键未选中的项先把它单选。
                let Some(e) = &entry else { return };
                if !self.panels[idx].selected.contains(&e.path) {
                    self.panels[idx].click_row(row, false, false);
                }
                self.entry_context_menu(ui, idx, rows, row, e, intents);
            });
        }
        response.on_hover_text(row_hover_tip(
            entry.as_ref(),
            branch,
            entry
                .as_ref()
                .and_then(|e| self.panels[idx].comments.get(&e.name))
                .map(String::as_str),
        ));
    }

    /// 打开一行的默认动作（双击/Enter/右键「打开」共用）：「..」= 上级；
    /// 目录 = 栏内进入；压缩包 = Archive 视图；其余 = open_path 分发。
    fn open_ui_row(&mut self, idx: usize, rows: &[usize], row: usize, intents: &mut FmIntents) {
        if row == 0 {
            if self.panels[idx].branch_view {
                self.panels[idx].exit_branch_view();
            } else {
                self.panels[idx].parent_dir();
            }
            return;
        }
        // rows 与 entries 之间存在失配窗口（同帧前面的行已触发导航），用 get 防御。
        let Some(entry) = rows
            .get(row - 1)
            .and_then(|&entry_idx| self.panels[idx].entries.get(entry_idx).cloned())
        else {
            return;
        };
        if entry.is_dir {
            self.panels[idx].navigate_to(entry.path.clone());
        } else if archive_kind(&entry.path).is_some() {
            // 双击压缩包分发按 fm_archive_open：Archive 视图（默认）/
            // 作为漫画打开 / 鼠标处弹小菜单二选一。
            match self.options.archive_open {
                FmArchiveOpen::Archive => intents.open_archive = Some(entry.path),
                FmArchiveOpen::Comic => intents.open_as_comic = Some(entry.path),
                FmArchiveOpen::Ask => intents.archive_ask = Some(entry.path),
            }
        } else {
            intents.open_path = Some(entry.path);
        }
    }

    /// 操作目标：selected 非空用选中集（按栏内顺序），否则用焦点项。
    fn op_targets(&mut self, idx: usize) -> Vec<PathBuf> {
        let panel = &mut self.panels[idx];
        if panel.selected.is_empty() {
            return panel
                .focused_entry()
                .map(|e| vec![e.path])
                .unwrap_or_default();
        }
        panel
            .entries
            .iter()
            .filter(|e| panel.selected.contains(&e.path))
            .map(|e| e.path.clone())
            .collect()
    }

    /// 复制/移动确认框：目标默认非焦点栏目录（单栏模式 = 本栏目录）。
    fn open_copy_move_dialog(&mut self, kind: OpKind, sources: Vec<PathBuf>, from_panel: usize) {
        let dest = match self.layout {
            PanelLayout::Dual { .. } => self.panels[1 - from_panel].dir.clone(),
            PanelLayout::Single { .. } => self.panels[from_panel].dir.clone(),
        };
        self.dialog = Some(FmDialog::CopyMove(CopyMoveDialog::new(
            kind, sources, &dest,
        )));
    }

    /// 压缩确认框：目标目录默认值与复制/移动一致（非焦点栏/本栏目录）。
    fn open_compress_dialog(&mut self, sources: Vec<PathBuf>, from_panel: usize) {
        let dest = match self.layout {
            PanelLayout::Dual { .. } => self.panels[1 - from_panel].dir.clone(),
            PanelLayout::Single { .. } => self.panels[from_panel].dir.clone(),
        };
        self.dialog = Some(FmDialog::Compress(CompressDialog::new(sources, &dest)));
    }

    /// 批量重命名（Ctrl+M）：作用于 op_targets（选中集或焦点项），
    /// is_dir 从栏内 entries 查（查不到回退磁盘探测）。
    fn open_multi_rename_dialog(&mut self, idx: usize) {
        let targets = self.op_targets(idx);
        if targets.is_empty() {
            return;
        }
        let items: Vec<(PathBuf, bool)> = targets
            .into_iter()
            .map(|p| {
                let is_dir = self.panels[idx]
                    .entries
                    .iter()
                    .find(|e| e.path == p)
                    .map(|e| e.is_dir)
                    .unwrap_or_else(|| p.is_dir());
                (p, is_dir)
            })
            .collect();
        self.dialog = Some(FmDialog::MultiRename(MultiRenameDialog::new(items)));
    }

    /// 栏间拖放复制（仅双栏接收）：行 payload 经 egui 全局 dnd 状态传递，
    /// 拖动中画「N 项」光标徽标；指针悬停另一栏（目录不同）时整栏高亮，
    /// 松开弹出既有「复制到…」确认框（dest = 目标栏当前目录，经
    /// open_copy_move_dialog 的既有 dest 计算）。拖到源栏自身或两栏
    /// 同目录时忽略（不高亮、不响应 drop）。
    fn poll_inter_panel_dnd(&mut self, ui: &egui::Ui, panel_rects: &[(usize, egui::Rect)]) {
        let ctx = ui.ctx();
        let Some(payload) = egui::DragAndDrop::payload::<FmDragPayload>(ctx) else {
            return;
        };
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        // 免确认模式（fm_drag_confirm=false）下 Shift = 移动，徽标提示。
        let shift_move = !self.options.drag_confirm && ctx.input(|i| i.modifiers.shift);
        // 光标跟随徽标（「N 项」/「N 项 · 移动」）；松开帧 payload 仍在但按键已抬，徽标消失。
        if pointer_down {
            if let Some(pos) = ctx.pointer_interact_pos() {
                egui::Area::new(egui::Id::new("fm-dnd-badge"))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .fixed_pos(pos + egui::vec2(14.0, 14.0))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            let label = if shift_move {
                                format!("{} 项 · 移动", payload.sources.len())
                            } else {
                                format!("{} 项", payload.sources.len())
                            };
                            ui.label(label);
                        });
                    });
            }
        }
        // 面包屑段/标签落点（阶段 Z，单/双栏通用，优先于栏体落点）：
        // 悬停高亮该段/标签，松开 = 复制到其目录（确认框沿用 fm_drag_confirm；
        // 免确认模式直拷自动改名，不做 Shift 移动——栏体落点才支持）。
        let layer = ui.layer_id();
        let released = ctx.input(|i| i.pointer.primary_released());
        let mut drop_dest: Option<PathBuf> = None;
        if pointer_down || released {
            for (target, rect) in self
                .breadcrumb_drop_rects
                .iter()
                .chain(self.tab_drop_rects.iter())
            {
                // 拖回源栏当前目录无意义（当前段/当前标签天然落在此）。
                if *target == self.panels[payload.src_panel].dir {
                    continue;
                }
                if !ctx.rect_contains_pointer(layer, *rect) {
                    continue;
                }
                if pointer_down {
                    // 高亮落点段/标签：栏体落点同款淡底 + 选中色描边。
                    let painter = ui.painter();
                    painter.rect_filled(*rect, 2.0, ui.visuals().faint_bg_color);
                    painter.rect_stroke(
                        *rect,
                        2.0,
                        egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    drop_dest = Some(target.clone());
                }
            }
        }
        if let Some(dest) = drop_dest {
            egui::DragAndDrop::clear_payload(ctx);
            let sources = payload.sources.clone();
            if self.options.drag_confirm {
                self.dialog = Some(FmDialog::CopyMove(CopyMoveDialog::new(
                    OpKind::Copy,
                    sources,
                    &dest,
                )));
            } else {
                self.ops.start_copy(sources, dest, ConflictMode::AutoRename);
            }
            return;
        }
        if !matches!(self.layout, PanelLayout::Dual { .. }) {
            return;
        }
        let mut drop_sources: Option<Vec<PathBuf>> = None;
        for &(idx, rect) in panel_rects {
            // 快览面板（被替换的非活动栏）不作为落点（不高亮、不响应 drop）。
            if idx == payload.src_panel
                || (self.quickview_open && idx != self.active)
                || !ctx.rect_contains_pointer(layer, rect)
            {
                continue;
            }
            if self.panels[idx].dir == self.panels[payload.src_panel].dir {
                continue;
            }
            if pointer_down {
                // 高亮落点栏：active 栏面包屑同款淡底 + 选中色描边。
                let painter = ui.painter();
                painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
            } else if released {
                drop_sources = Some(payload.sources.clone());
            }
        }
        if let Some(sources) = drop_sources {
            egui::DragAndDrop::clear_payload(ctx);
            if self.options.drag_confirm {
                // 落点恒为 1-src_panel，dest 计算与 F5/菜单「复制到另一栏…」一致。
                self.open_copy_move_dialog(OpKind::Copy, sources, payload.src_panel);
            } else {
                // fm_drag_confirm=false：松开直拷（冲突策略默认自动改名）；
                // 按住 Shift = 移动。确认模式下 Shift 不区分（维持现状）。
                let dest = self.panels[1 - payload.src_panel].dir.clone();
                if shift_move {
                    self.ops.start_move(sources, dest, ConflictMode::AutoRename);
                } else {
                    self.ops.start_copy(sources, dest, ConflictMode::AutoRename);
                }
            }
        }
    }

    /// 外部拖入的落点判定（app 侧 handle_dropped_files 用）：pos 命中某栏
    /// 的落点区域返回栏索引；快览替换栏/顶栏/状态栏不命中。
    pub fn panel_rect_at(&self, pos: egui::Pos2) -> Option<usize> {
        self.panel_drop_rects
            .iter()
            .position(|r| r.is_some_and(|r| r.contains(pos)))
    }

    /// 外部文件拖入某栏：sources = 拖入路径，目标 = 该栏目录，走既有
    /// 复制确认框（与栏间拖放同确认语义）。
    pub fn drop_to_panel(&mut self, paths: Vec<PathBuf>, idx: usize) {
        self.open_copy_move_dialog(OpKind::Copy, paths, idx);
    }

    /// FM 行拖出窗口（Windows OLE）：拖动中位移 >40pt 且指针出窗 → 清
    /// egui payload 后 do_drag_drop（文件本就在盘上无需暂存）。按住 Shift
    /// 拖出允许 MOVE（阶段 Z）：落点实际执行 MOVE 且成功后，源经
    /// file_ops Delete 任务进回收站（保进度/回收站语义）。松开未出窗 =
    /// 栏间拖放现状（egui dnd 插件自行管理 payload，本函数不介入）；
    /// Esc 取消由 egui 内建处理。
    fn poll_drag_out_external(&mut self, ctx: &egui::Context, intents: &mut FmIntents) {
        if !crate::platform::drag_out::is_supported() {
            return;
        }
        let Some(payload) = egui::DragAndDrop::payload::<FmDragPayload>(ctx) else {
            return;
        };
        let (origin, pos, viewport) = ctx.input(|i| {
            (
                i.pointer.press_origin(),
                i.pointer.latest_pos(),
                i.viewport_rect(),
            )
        });
        let moved_far = origin
            .zip(pos)
            .is_some_and(|(o, p)| o.distance(p) > DRAG_OUT_THRESHOLD);
        let left_window = pos.is_none_or(|p| !viewport.contains(p));
        if !(moved_far && left_window) {
            return;
        }
        let sources = payload.sources.clone();
        egui::DragAndDrop::clear_payload(ctx);
        // 模态阻塞（自带消息循环）；DROP/CANCEL 返回后本次拖出都结束。
        // Shift 在进模态前快照（模态内 egui 输入不再更新）。
        let allow_move = ctx.input(|i| i.modifiers.shift);
        match crate::platform::drag_out::do_drag_drop(&sources, allow_move) {
            // 落点执行了 MOVE：源进回收站（Delete 任务带进度；失败会在
            // on_op_finished 汇总上报）。
            Ok(true) => self.start_delete(sources, false),
            Ok(false) => {}
            Err(e) => intents.op_error = Some(e),
        }
    }

    /// 删除（确认框已把关或 fm_confirm_delete=false）：预览目标在被删项中
    /// 先清预览，然后起后台任务。permanent = 物理删除（否则移入回收站）。
    fn start_delete(&mut self, sources: Vec<PathBuf>, permanent: bool) {
        if let Some(tp) = &self.preview_path {
            if sources.contains(tp) {
                self.clear_preview();
            }
        }
        self.ops.start_delete(sources, permanent);
    }

    /// 渲染操作确认对话框；Some(outcome) 时统一执行。
    fn render_dialog(&mut self, ctx: &egui::Context, intents: &mut FmIntents) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        match dialog.ui(ctx) {
            None => self.dialog = Some(dialog),
            Some(outcome) => self.apply_dialog_outcome(outcome, intents),
        }
    }

    /// 打开文件搜索对话框（Alt+F7 / 顶栏按钮 / 右键菜单共用）：
    /// 搜索根 = 焦点栏当前目录。
    fn open_search_dialog(&mut self) {
        let root = self.panels[self.active].dir.clone();
        self.search.open_with(root);
    }

    /// 新建文本文件对话框（Shift+F4 / 右键菜单共用）：父目录 = 指定栏当前目录。
    fn open_new_file_dialog(&mut self, idx: usize) {
        let parent = self.panels[idx].dir.clone();
        let suggested = suggest_text_file_name(&parent);
        self.dialog = Some(FmDialog::NewFile(NewFileDialog::new(parent, suggested)));
    }

    /// 渲染同步目录对话框（阶段 AE）：「开始同步」确认经 file_ops 提交
    /// Copy（Overwrite）任务组与 Delete（回收站）任务——任务队列/进度/
    /// 错误汇总天然生效，完成刷新同既有 on_op_finished 路径。
    fn render_sync_dialog(&mut self, ctx: &egui::Context) {
        let SyncUiAction::Run(plan) = self.sync_dialog.ui(ctx) else {
            return;
        };
        for (sources, dest_dir) in plan.copies {
            self.ops
                .start_copy(sources, dest_dir, ConflictMode::Overwrite);
        }
        if !plan.deletes.is_empty() {
            self.ops.start_delete(plan.deletes, false);
        }
    }

    /// 渲染搜索对话框并消费动作：跳转 = 焦点栏 reveal 并关闭；
    /// 「输送到焦点栏」= 命中集注入焦点栏（分支视图同款）并关闭。
    fn render_search_dialog(&mut self, ctx: &egui::Context) {
        match self.search.ui(ctx) {
            SearchUiAction::None => {}
            SearchUiAction::Reveal(path) => {
                let active = self.active;
                self.panels[active].reveal_path(path);
                self.search.close();
            }
            SearchUiAction::FeedToPanel => {
                let entries = self.search.feed_entries();
                let active = self.active;
                self.panels[active].inject_entries_branch(entries);
                self.search.close();
            }
        }
    }

    /// 渲染「修改属性/时间戳…」对话框（阶段 AC；take/reinsert 模式同    /// render_comment_dialog）：确认经 `apply_to_path` 逐项应用（平台属性位
    /// 与 filetime 时间戳），任一失败回传 error 保持打开；全成功后刷新涉及
    /// 栏（父目录匹配，选中集不动）。
    fn render_attr_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.attr_dialog.take() else {
            return;
        };
        match dialog.ui(ctx) {
            None => self.attr_dialog = Some(dialog),
            Some(AttrAction::Close) => {}
            Some(AttrAction::Apply { bits, mtime, atime }) => {
                let mut errors: Vec<String> = Vec::new();
                for path in dialog.paths() {
                    if let Err(e) = apply_to_path(path, bits, mtime, atime) {
                        errors.push(format!("{}: {e}", path.display()));
                    }
                }
                if errors.is_empty() {
                    for path in dialog.paths() {
                        for panel in &mut self.panels {
                            if path.parent() == Some(panel.dir.as_path()) {
                                panel.refresh();
                            }
                        }
                    }
                } else {
                    dialog.error = Some(errors.join("；"));
                    self.attr_dialog = Some(dialog);
                }
            }
        }
    }

    /// 渲染执行期冲突问答弹窗；用户选择后回发 worker（同 render_dialog
    /// 的 take/reinsert 模式，防嵌套借用）。
    fn render_conflict_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut dialog)) = self.pending_conflict.take() else {
            return;
        };
        match dialog.ui(ctx) {
            Some(answer) => self.ops.answer_conflict(id, answer),
            None => self.pending_conflict = Some((id, dialog)),
        }
    }

    fn apply_dialog_outcome(&mut self, outcome: FmDialogOutcome, intents: &mut FmIntents) {
        let sys_cut_paste = std::mem::take(&mut self.sys_clipboard_cut_pending);
        match outcome {
            FmDialogOutcome::Cancelled => {}
            FmDialogOutcome::ConfirmCopyMove {
                kind,
                sources,
                dest,
                conflict,
                opts,
            } => {
                match kind {
                    OpKind::Copy => {
                        self.ops.start_copy_opts(sources, dest, conflict, opts);
                    }
                    OpKind::Move => {
                        self.ops.start_move_opts(sources, dest, conflict, opts);
                        if sys_cut_paste {
                            // Explorer 惯例：剪切粘贴生效后清空系统剪贴板，
                            // 防同一份「剪切」被重复粘贴。
                            crate::platform::clipboard_files::clear();
                        }
                    }
                    OpKind::Delete | OpKind::Compress | OpKind::Split | OpKind::Merge => {
                        unreachable!("各有专用路径")
                    }
                };
            }
            FmDialogOutcome::ConfirmCompress { sources, dest_zip } => {
                self.ops.start_compress(sources, dest_zip);
            }
            FmDialogOutcome::ConfirmSplit {
                source,
                dest_dir,
                chunk_size,
            } => {
                self.ops.start_split(source, dest_dir, chunk_size);
            }
            FmDialogOutcome::ConfirmDelete {
                sources,
                permanent,
                dont_ask_again,
            } => {
                if dont_ask_again {
                    intents.confirm_delete_change = Some(false);
                }
                self.start_delete(sources, permanent);
            }
            FmDialogOutcome::ConfirmRename { path, new_name } => {
                match rename_entry(&path, &new_name) {
                    Ok(new_path) => self.refresh_panel_of(&new_path),
                    Err(e) => intents.op_error = Some(e),
                }
            }
            FmDialogOutcome::ConfirmNewDir { parent, name } => match create_dir(&parent, &name) {
                Ok(new_path) => self.refresh_panel_of(&new_path),
                Err(e) => intents.op_error = Some(e),
            },
            FmDialogOutcome::ConfirmNewFile { parent, name } => {
                match create_text_file(&parent, &name) {
                    Ok(new_path) => self.refresh_panel_of(&new_path),
                    Err(e) => intents.op_error = Some(e),
                }
            }
            FmDialogOutcome::SelectGroup {
                pattern,
                select,
                files_only,
            } => {
                // 记住上次输入（会话内），作用于焦点栏当前可见行。
                self.select_group_pattern = pattern.clone();
                self.panels[self.active].apply_pattern_selection(&pattern, select, files_only);
            }
            FmDialogOutcome::ConfirmMultiRename { plans } => {
                let mut failures = Vec::new();
                for plan in &plans {
                    match rename_entry(&plan.src, &plan.dst_name) {
                        Ok(new_path) => self.refresh_panel_of(&new_path),
                        Err(e) => failures.push(format!("{}: {e}", plan.src.display())),
                    }
                }
                if !failures.is_empty() {
                    intents.op_error = Some(format!(
                        "批量重命名完成，{} 项失败:\n{}",
                        failures.len(),
                        failures.join("\n")
                    ));
                }
            }
        }
    }

    /// 重命名/新建文件夹完成：刷新所在栏并选中新条目（refresh 保留选中，
    /// 列举完成后新路径仍在选中集内）。
    fn refresh_panel_of(&mut self, new_path: &Path) {
        for panel in &mut self.panels {
            if new_path.parent() == Some(panel.dir.as_path()) {
                panel.selected.insert(new_path.to_path_buf());
                panel.refresh();
            }
        }
    }

    /// 解压完成刷新落点栏：栏目录 == 输出目录（直接解压进目标文件夹）或
    /// 输出目录的父目录（智能/强制建包名子目录）时刷新，保留选中。
    pub fn refresh_extract_dest(&mut self, output_dir: &Path) {
        for panel in &mut self.panels {
            if panel.dir == output_dir || output_dir.parent() == Some(panel.dir.as_path()) {
                panel.refresh();
            }
        }
    }

    /// 操作完成：刷新涉及的两栏（目标栏 + 源栏），汇总错误经回调上报。
    fn on_op_finished(&mut self, op: FinishedOp, intents: &mut FmIntents) {
        for panel in &mut self.panels {
            let involved =
                op.dest_dir.as_ref() == Some(&panel.dir) || op.src_dirs.contains(&panel.dir);
            if involved {
                panel.refresh();
            }
        }
        let verb = op.kind.verb();
        let mut parts = Vec::new();
        if op.cancelled {
            parts.push(format!("{verb}已取消（已处理部分保留）"));
        }
        if let Some(fatal) = &op.fatal {
            parts.push(format!("{verb}失败: {fatal}"));
        }
        if !op.errors.is_empty() {
            // 阶段 AA 错误汇总窗：留存完整失败清单 + 重试参数（toast 只报
            // 前 3 项）。新报告覆盖旧窗。
            self.op_error_report = Some(OpErrorReport {
                kind: op.kind,
                dest: op.dest.clone(),
                dest_dir: op.dest_dir.clone(),
                conflict: op.conflict,
                delete_permanent: op.delete_permanent,
                errors: op.errors.clone(),
            });
            let first: Vec<String> = op
                .errors
                .iter()
                .take(3)
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect();
            parts.push(format!(
                "{verb}完成，{} 项失败:\n{}",
                op.errors.len(),
                first.join("\n")
            ));
        }
        if !parts.is_empty() {
            intents.op_error = Some(parts.join("\n"));
        }
    }

    /// 键盘导航（Explorer/TC 式）：Tab 切换焦点栏（纯 Tab；Shift+Tab 不拦）、
    /// Ctrl+Tab/Ctrl+Shift+Tab 标签循环、Ctrl+T 新建标签、Ctrl+W 关闭当前
    /// 标签（剩 1 个忽略）、↑/↓ 移动焦点并单选（网格模式按列数步进，
    /// 网格专属 ←→ 步进 1）、
    /// Shift+↑/↓ 从 anchor 扩选、Ctrl+↑/↓ 只移焦点、Home/End 跳首/末行、
    /// PgUp/PgDn 整页步进、Enter 打开焦点行、空格计算焦点目录大小、
    /// Backspace 上级、Ctrl+A 全选可见、Ctrl+R 刷新、Alt+←/→ 导航历史、
    /// Alt+↓ 历史下拉开关、Ctrl+D 书签菜单开关、Ctrl+L 选中集目录批量
    /// 计算大小（无选中回退焦点目录）、可打印字符 type-ahead 定位、`*` 反选、
    /// `+`/`-` 弹「选择组」对话框、Ctrl+U 交换两栏、Ctrl+←/→ 栏间目录
    /// 同步、Ctrl+\ 回根目录、Ctrl+Q 对面栏快速预览（单栏 = 预览开关）、
    /// Ctrl+B 分支视图（「..」行/Esc 末级 = 退出分支）、
    /// Ctrl+M 批量重命名、Esc 分级清 type-ahead 缓冲→过滤→选中。
    /// Ctrl+S 聚焦过滤框（已聚焦则选中全文）——提前于
    /// egui_wants_keyboard_input 检查，过滤框聚焦时也可再次触发；
    /// 其余键在过滤框等文本输入占用键盘时不处理。
    /// 文件操作键：F2 重命名 / F5 复制 / F6 移动 / F7 新建文件夹 /
    /// F4 系统「编辑」动词（无关联回退「打开方式…」，目录忽略）/
    /// Shift+F4 新建文本文件 / F8(Delete) 删除（confirm_delete 时先弹确认框；
    /// Shift+Del = 删除方式的另一档快捷）；Alt+F5 压缩为 zip、
    /// Alt+F9 解压对话框（单压缩包选中集）、Alt+F7 文件搜索；
    /// Alt+Enter 焦点项系统属性、Ctrl+Shift+Enter 以管理员身份运行（runas）；
    /// Ctrl+C/X/V 剪贴板。行为开关由 self.options（每帧下发）提供。
    /// （纯功能键分支需排 Alt——key_pressed 不认修饰键，Alt+F4 系统关窗不拦。）
    fn handle_keyboard(&mut self, ui: &egui::Ui, intents: &mut FmIntents) {
        // 对话框打开时屏蔽面板键盘（输入归对话框；冲突问答窗/分组小窗同此机制）。
        if self.dialog.is_some()
            || self.pending_conflict.is_some()
            || self.group_dialog.is_some()
            || self.selection_dialog.is_some()
            || self.comment_dialog.is_some()
            || self.button_dialog.is_some()
        {
            return;
        }
        // Ctrl+S：聚焦过滤框（render_filter_bar 下一帧消费；已聚焦 = 选中
        // 全文）。必须在 egui_wants_keyboard_input 检查之前——过滤框聚焦时
        // 该检查恒 true，不放前面则「已聚焦选中全文」分支永远到不了。
        if ui.input(|i| {
            i.key_pressed(egui::Key::S)
                && i.modifiers.command
                && !i.modifiers.alt
                && !i.modifiers.shift
        }) {
            self.filter_focus_request = true;
        }
        // Ctrl+P / Ctrl+Enter（阶段 X 命令行，TC 手感）：同样在
        // egui_wants_keyboard_input 检查之前——命令行聚焦时该检查恒 true，
        // 且 TextEdit 不为带 Ctrl 的按键产文本，两键不与输入冲突。
        // Ctrl+P = 焦点栏当前路径追加到输入末尾；Ctrl+Enter = 焦点项文件名
        // 追加（Ctrl+Shift+Enter 已是 runas，纯 Ctrl+Enter 空闲）。
        if self.options.command_bar
            && ui.input(|i| {
                i.key_pressed(egui::Key::P)
                    && i.modifiers.command
                    && !i.modifiers.alt
                    && !i.modifiers.shift
            })
        {
            let dir = self.panels[self.active].dir.display().to_string();
            self.command_input.push_str(&dir);
            self.command_focus_request = true;
        }
        if self.options.command_bar
            && ui.input(|i| {
                i.key_pressed(egui::Key::Enter)
                    && i.modifiers.command
                    && !i.modifiers.alt
                    && !i.modifiers.shift
            })
        {
            if let Some(entry) = self.panels[self.active].focused_entry() {
                self.command_input.push_str(&entry.name);
                self.command_focus_request = true;
            }
        }
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        // Tab 切换焦点栏（仅双栏；Shift/Ctrl+Tab 不拦——Ctrl+Tab 是标签
        // 循环，见下；Shift+Tab 留给系统/输入焦点）。
        if matches!(self.layout, PanelLayout::Dual { .. })
            && !mods.shift
            && !mods.command
            && ui.input(|i| i.key_pressed(egui::Key::Tab))
        {
            self.active = 1 - self.active;
        }
        // Shift 优先于 Ctrl（Explorer：Shift+方向 = 扩选，Ctrl+方向 = 只移焦点）。
        let focus_mode = if mods.shift {
            FocusMove::Extend
        } else if mods.command {
            FocusMove::FocusOnly
        } else {
            FocusMove::Select
        };
        let active = self.active;
        // Ctrl+Tab / Ctrl+Shift+Tab：焦点栏标签循环（与纯 Tab 切栏互不干扰）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
            if mods.shift {
                self.panels[active].prev_tab();
            } else {
                self.panels[active].next_tab();
            }
        }
        // Ctrl+T：新建标签（复制当前目录）；Ctrl+W：关闭当前标签
        // （剩 1 个时 close_tab 自身 no-op）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::T)) {
            self.panels[active].new_tab();
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::W)) {
            let tab = self.panels[active].active_tab();
            self.panels[active].close_tab(tab);
        }
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.panels[active].go_back();
        }
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.panels[active].go_forward();
        }
        // Alt+↓：开/关焦点栏的目录历史下拉（一次性请求，面包屑渲染时消费）。
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.history_menu_toggle = true;
        }
        // Ctrl+D：开/关焦点栏的书签菜单（一次性请求，同 Alt+↓ 机制）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::D)) {
            self.bookmarks_menu_toggle = true;
        }
        // Ctrl+L：对选中集内所有目录批量计算大小；选中集无目录时回退焦点目录
        // （同右键「计算大小」入口 request_dir_sizes）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::L)) {
            let panel = &mut self.panels[active];
            let mut dirs: Vec<PathBuf> = panel
                .entries
                .iter()
                .filter(|e| e.is_dir && panel.selected.contains(&e.path))
                .map(|e| e.path.clone())
                .collect();
            if dirs.is_empty() {
                if let Some(e) = panel.focused_entry() {
                    if e.is_dir {
                        dirs.push(e.path);
                    }
                }
            }
            if !dirs.is_empty() {
                panel.request_dir_sizes(dirs);
            }
        }
        // Alt+Enter：焦点项系统「属性」对话框（同 Explorer；非 Windows 无此键位）。
        if mods.alt
            && crate::platform::shell_verbs::is_supported()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            if let Some(entry) = self.panels[active].focused_entry() {
                if let Err(e) = crate::platform::shell_verbs::show_properties(&entry.path) {
                    intents.op_error = Some(e);
                }
            }
        }
        // 鼠标侧键：Extra1 = 后退，Extra2 = 前进（仅本视图；漫画阅读器侧
        // 键翻页在 app.rs 的 View::Reader 分支处理，不冲突）。
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.panels[active].go_back();
        }
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
            self.panels[active].go_forward();
        }
        // 剪贴板：Ctrl+C 复制 / Ctrl+X 剪切 / Ctrl+V 粘贴（经确认框）。
        // Windows 优先写/读系统剪贴板（CF_HDROP，与 Explorer 互通）；
        // 非 Windows 或写失败回退应用内路径列表。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::C)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                if crate::platform::clipboard_files::set_files(&targets, false).is_ok() {
                    // 系统剪贴板接管：清掉应用内副本，防系统剪贴板被其他
                    // 内容覆盖后 Ctrl+V 回退粘贴出陈旧文件列表。
                    self.clipboard.clear();
                    self.clipboard_cut = false;
                } else {
                    self.clipboard = targets;
                    self.clipboard_cut = false;
                }
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::X)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                if crate::platform::clipboard_files::set_files(&targets, true).is_ok() {
                    self.clipboard.clear();
                    self.clipboard_cut = false;
                } else {
                    self.clipboard = targets;
                    self.clipboard_cut = true;
                }
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::V)) {
            if let Some((paths, is_cut)) = crate::platform::clipboard_files::get_files() {
                // 系统剪贴板有文件（含 Explorer/其他程序复制的）：目标 =
                // 焦点栏目录，剪切 → Move 否则 Copy，走既有确认框。
                self.sys_clipboard_cut_pending = is_cut;
                let kind = if is_cut { OpKind::Move } else { OpKind::Copy };
                self.open_copy_move_dialog(kind, paths, active);
            } else if !self.clipboard.is_empty() {
                let kind = if self.clipboard_cut {
                    OpKind::Move
                } else {
                    OpKind::Copy
                };
                self.open_copy_move_dialog(kind, self.clipboard.clone(), active);
            }
        }
        // 文件操作：F2 重命名 / F5 复制 / F6 移动 / F7 新建文件夹 / F8(Del) 删除。
        // Alt+F7 = 文件搜索、Alt+F5 = 压缩为 zip、Alt+F9 = 解压对话框
        // （TC 语义；纯功能键需排 Alt，否则同键双触发。Alt+F4 是系统关窗，不拦）。
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::F7)) {
            self.open_search_dialog();
        }
        // Alt+F5：压缩为 zip 对话框（同右键「压缩为 zip…」）。
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::F5)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.open_compress_dialog(targets, active);
            }
        }
        // Alt+F9：解压对话框（同右键「解压到…」；选中集恰为单个压缩包时可用，
        // 目标目录 = 另一栏（异目录双栏）否则当前目录）。
        if mods.alt && ui.input(|i| i.key_pressed(egui::Key::F9)) {
            let targets = self.op_targets(active);
            let src = match targets.as_slice() {
                [p] if archive_kind(p).is_some() => Some(p.clone()),
                _ => None,
            };
            if let Some(src) = src {
                let dest = match self.layout {
                    PanelLayout::Dual { .. } => {
                        let other = self.panels[1 - active].dir.clone();
                        if other != self.panels[active].dir {
                            other
                        } else {
                            self.panels[active].dir.clone()
                        }
                    }
                    PanelLayout::Single { .. } => self.panels[active].dir.clone(),
                };
                intents.extract = Some((src, dest));
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F2)) {
            if let Some(entry) = self.panels[active].focused_entry() {
                self.dialog = Some(FmDialog::Rename(RenameDialog::new(entry.path)));
            }
        }
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::F5)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Copy, targets, active);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F6)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                self.open_copy_move_dialog(OpKind::Move, targets, active);
            }
        }
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::F7)) {
            let parent = self.panels[active].dir.clone();
            let suggested = suggest_folder_name(&parent);
            self.dialog = Some(FmDialog::NewDir(NewDirDialog::new(parent, suggested)));
        }
        // Shift+F4：新建文本文件（TC 语义）。
        if mods.shift && ui.input(|i| i.key_pressed(egui::Key::F4)) {
            self.open_new_file_dialog(active);
        }
        // F4：焦点文件系统「编辑」动词（无 edit 关联时回退「打开方式…」；
        // 目录忽略；非 Windows 无此键位）。
        if !mods.shift
            && !mods.alt
            && !mods.command
            && crate::platform::shell_verbs::is_supported()
            && ui.input(|i| i.key_pressed(egui::Key::F4))
        {
            if let Some(entry) = self.panels[active].focused_entry() {
                if !entry.is_dir {
                    if let Err(e) = crate::platform::shell_verbs::edit_file(&entry.path) {
                        intents.op_error = Some(e);
                    }
                }
            }
        }
        // F8/Del 删除；Shift+Del = 删除方式的另一档快捷（trash 模式下直删，
        // permanent 模式下进回收站）：生效档位 = 设置档 XOR Shift。
        if ui.input(|i| i.key_pressed(egui::Key::F8) || i.key_pressed(egui::Key::Delete)) {
            let targets = self.op_targets(active);
            if !targets.is_empty() {
                let permanent = self.options.delete_permanent != mods.shift;
                if self.options.confirm_delete {
                    self.dialog = Some(FmDialog::Delete(DeleteDialog::new(targets, permanent)));
                } else {
                    self.start_delete(targets, permanent);
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.panels[active].parent_dir();
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::A)) {
            self.panels[active].select_all_visible();
        }
        // `*` 反选 / `+`「选择组」对话框 / `-` 同框预置取消选择（TC 语义），
        // 其余可打印字符进 type-ahead（type-to-select）缓冲。
        // egui 0.35 的 Key 枚举没有小键盘乘/加/减键，主键盘 `*` 又是 Shift+8，
        // 统一用 Event::Text 捕获——文本事件只在无控件占用键盘时产生
        // （上面 egui_wants_keyboard_input 已挡掉过滤框/对话框输入）。
        // 空格保留给「计算焦点目录大小」，不进 type-ahead。
        let (star, plus, minus, type_chars) = ui.input(|i| {
            let (mut star, mut plus, mut minus) = (false, false, false);
            let mut chars = Vec::new();
            for e in &i.events {
                if let egui::Event::Text(t) = e {
                    for c in t.chars() {
                        match c {
                            '*' => star = true,
                            '+' => plus = true,
                            '-' => minus = true,
                            ' ' => {}
                            c if !c.is_control() => chars.push(c),
                            _ => {}
                        }
                    }
                }
            }
            (star, plus, minus, chars)
        });
        if star {
            self.panels[active].invert_selection();
        }
        if plus {
            self.dialog = Some(FmDialog::SelectGroup(SelectGroupDialog::new(
                self.select_group_pattern.clone(),
                false,
            )));
        }
        if minus {
            self.dialog = Some(FmDialog::SelectGroup(SelectGroupDialog::new(
                self.select_group_pattern.clone(),
                true,
            )));
        }
        // type-to-select：命中即移动焦点并单选（Select 语义同 ↑/↓，
        // 经 focus_row 置最小滚动揭示）；Ctrl/Alt 组合键不产出定位字符。
        if !mods.command && !mods.alt {
            let panel = &mut self.panels[active];
            for c in type_chars {
                if let Some(row) = panel.type_ahead_push(c) {
                    panel.focus_row(row, FocusMove::Select);
                }
            }
        }
        // 栏间快捷键（仅双栏）：Ctrl+U 交换两栏（watcher/loader 随结构体走，
        // active 不变；快照 diff 自然写回 dir_left/dir_right）。
        let dual = matches!(self.layout, PanelLayout::Dual { .. });
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::U)) {
            self.panels.swap(0, 1);
        }
        // Ctrl+→：另一栏跳到本栏焦点目录（焦点非目录则跳本栏当前目录）；
        // Ctrl+←：反向（本栏 ← 另一栏焦点目录/当前目录）。
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            let target = match self.panels[active].focused_entry() {
                Some(e) if e.is_dir => e.path,
                _ => self.panels[active].dir.clone(),
            };
            self.panels[1 - active].navigate_to(target);
        }
        if dual && mods.command && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            let other = 1 - active;
            let target = match self.panels[other].focused_entry() {
                Some(e) if e.is_dir => e.path,
                _ => self.panels[other].dir.clone(),
            };
            self.panels[active].navigate_to(target);
        }
        // Ctrl+\：本栏回根目录（同面包屑段点击根段）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::Backslash)) {
            let panel_dir = self.panels[active].dir.clone();
            if !panel_dir.as_os_str().is_empty() {
                if let Some(root) = panel_dir.ancestors().last() {
                    self.panels[active].navigate_to(root.to_path_buf());
                }
            }
        }
        // Ctrl+B：分支视图开关（当前目录 + 所有子目录文件扁平列出）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::B)) {
            self.panels[active].toggle_branch_view();
        }
        // Ctrl+M：批量重命名（作用于选中集或焦点项）。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::M)) {
            self.open_multi_rename_dialog(active);
        }
        // Ctrl+Shift+Z：编辑焦点项注释（阶段 W；Ctrl+Z 预留给撤销，不占）。
        // 分支视图子目录项同右键菜单门槛（注释写回当前目录 descript.ion）。
        if mods.command && mods.shift && ui.input(|i| i.key_pressed(egui::Key::Z)) {
            if let Some(entry) = self.panels[active].focused_entry() {
                if entry.rel_dir.is_empty() {
                    self.comment_dialog = Some(CommentDialog::new(
                        active,
                        entry.name.clone(),
                        self.panels[active]
                            .comments
                            .get(&entry.name)
                            .map(String::as_str),
                    ));
                }
            }
        }
        // Ctrl+Q：双栏 = 开关对面栏快速预览（快览面板恒渲染非活动栏位置，
        // 目标 = 活动栏焦点文件）；单栏 = 切换预览面板（同顶栏「预览」）。
        // Q 是字母键，但 type-ahead 捕获有 !mods.command 门控，不会抢键。
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::Q)) {
            match &mut self.layout {
                PanelLayout::Dual { .. } => {
                    self.quickview_open = !self.quickview_open;
                    if !self.quickview_open {
                        // 关闭即清预览目标，避免后台继续读取。
                        self.clear_preview();
                    }
                }
                PanelLayout::Single { preview_open } => {
                    *preview_open = !*preview_open;
                    self.saved_preview_open = *preview_open;
                }
            }
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::R)) {
            self.panels[active].refresh();
        }
        // 网格状模式（简表/缩略图）：↑↓ 按列数步进（线性行号换算，列数由
        // render_brief/render_grid 每帧写入 last_grid_cols），←→ 步进 1
        // （列表模式 ←→ 不绑定）；PgUp/PgDn = 可见行数 × 列数。
        let grid_like = self.panels[active].view_mode.is_grid_like();
        let vstep = self.panels[active].last_grid_cols.max(1) as isize;
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            let step = if grid_like { vstep } else { 1 };
            self.panels[active].move_focus(step, focus_mode);
        }
        if !mods.alt && ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            let step = if grid_like { vstep } else { 1 };
            self.panels[active].move_focus(-step, focus_mode);
        }
        if grid_like && !mods.command && !mods.alt {
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                self.panels[active].move_focus(1, focus_mode);
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                self.panels[active].move_focus(-1, focus_mode);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Home)) {
            self.panels[active].move_focus_edge(false, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::End)) {
            self.panels[active].move_focus_edge(true, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
            let rows_step = self.panels[active].page_step();
            let step = if grid_like {
                rows_step * vstep
            } else {
                rows_step
            };
            self.panels[active].move_focus(step, focus_mode);
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
            let rows_step = self.panels[active].page_step();
            let step = if grid_like {
                rows_step * vstep
            } else {
                rows_step
            };
            self.panels[active].move_focus(-step, focus_mode);
        }
        // Enter 打开焦点行；排修饰键——key_pressed 不认修饰键，不排则
        // Alt+Enter（属性）/Ctrl+Shift+Enter（runas）会同帧双触发。
        if !mods.command && !mods.alt && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let rows = self.panels[active].rows();
            if let Some(row) = self.panels[active].focus {
                self.open_ui_row(active, &rows, row, intents);
            }
        }
        // Ctrl+Shift+Enter：以管理员身份运行焦点文件/目录（verb "runas"，
        // 触发 UAC；非 Windows 无此键位）。
        if mods.command
            && mods.shift
            && crate::platform::shell_verbs::is_supported()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            if let Some(entry) = self.panels[active].focused_entry() {
                if let Err(e) = crate::platform::shell_verbs::run_as_admin(&entry.path) {
                    intents.op_error = Some(e);
                }
            }
        }
        // 空格：默认计算焦点目录大小；fm_space_action = "toggle_select" 时
        // 改 TC 勾选语义（切换焦点项选中并下移）。文件/「..」/无焦点忽略。
        if !mods.command
            && !mods.shift
            && !mods.alt
            && ui.input(|i| i.key_pressed(egui::Key::Space))
        {
            if self.options.space_toggle_select {
                self.panels[active].toggle_focused_selection_and_advance();
            } else if let Some(entry) = self.panels[active].focused_entry() {
                if entry.is_dir {
                    self.panels[active].request_dir_sizes(vec![entry.path]);
                }
            }
        }
        // Insert：无条件 TC 勾选语义（与 fm_space_action 设置无关）。
        if ui.input(|i| i.key_pressed(egui::Key::Insert)) {
            self.panels[active].toggle_focused_selection_and_advance();
        }
        // F3：双栏模式对焦点文件弹临时预览窗（再按 F3 / Esc / 关闭按钮关窗）。
        if ui.input(|i| i.key_pressed(egui::Key::F3)) {
            if self.preview_window_open {
                self.preview_window_open = false;
            } else if matches!(self.layout, PanelLayout::Dual { .. }) {
                if let Some(entry) = self.panels[active].focused_entry() {
                    if !entry.is_dir {
                        self.set_preview_target(entry);
                        self.preview_window_open = true;
                    }
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.filter_esc_handled || self.command_esc_handled {
                // 过滤框/命令行 Esc（清空 + 交还焦点）已在各自渲染处消费，
                // 不再走 Esc 链（防同帧双消费）。
                self.filter_esc_handled = false;
                self.command_esc_handled = false;
            } else if self.archive_ask.is_some() {
                // 压缩包 ask 小菜单优先关闭。
                self.archive_ask = None;
            } else if self.preview_window_open {
                self.preview_window_open = false;
            } else {
                let panel = &mut self.panels[active];
                if panel.type_ahead_buffer().is_some() {
                    panel.clear_type_ahead();
                } else if !panel.filter.is_empty() {
                    panel.filter.clear();
                } else if !self.options.esc_keep_selection && !panel.selected.is_empty() {
                    panel.clear_selection();
                } else if !self.options.esc_keep_selection && panel.branch_view {
                    // 分支模式且缓冲/过滤/选中均空：退出分支视图。
                    panel.exit_branch_view();
                }
                // fm_esc_keep_selection = true：Esc 链止于清过滤（选中保留、
                // 分支视图不退出）。
            }
        }
    }

    /// 「选中即预览」：焦点栏焦点行落到文件时更新预览目标（可预览类型后台
    /// 加载，不可预览类型直接显示元信息占位，不读内容）；焦点不在文件上
    /// （目录/「..」/无焦点）时清空预览——预览面板/快览恒在，内容切占位。
    /// 仅单栏预览开 / F3 弹窗开 / 双栏快览开时跟随焦点，其余情况不发起
    /// 后台读取。
    fn sync_preview_target(&mut self) {
        let follows = matches!(self.layout, PanelLayout::Single { preview_open: true })
            || self.preview_window_open
            || (matches!(self.layout, PanelLayout::Dual { .. }) && self.quickview_open);
        if !follows {
            return;
        }
        match self.panels[self.active].focused_entry() {
            Some(entry) if !entry.is_dir => {
                if self.preview_path.as_ref() != Some(&entry.path) {
                    self.set_preview_target(entry);
                }
            }
            _ => self.clear_preview(),
        }
    }

    /// 设置预览目标并发起后台读取（阶段 R 起不再按 is_previewable_name
    /// 门槛分流：所有类型都读取——文本/图片走内容嗅探，其余落 HEX 模式；
    /// 读取上限仍由 loader 的 PREVIEW_MAX_BYTES 把关）。每个新目标重置
    /// 模式/编码/旋转/搜索态（手动编码选择「下一个文件回自动」）。
    fn set_preview_target(&mut self, entry: FsEntry) {
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        self.preview_bytes = None;
        self.preview_mode = PreviewMode::Hex;
        self.preview_encoding = PreviewEncoding::Auto;
        self.preview_rotation = 0;
        self.preview_search.clear();
        self.preview_search_matches = Vec::new();
        self.preview_search_cur = 0;
        self.preview_search_reveal = None;
        self.preview_full_size = false;
        let path = entry.path.clone();
        self.preview = Some(AsyncOpener::open(path, load_file_preview));
        self.preview_path = Some(entry.path.clone());
        self.preview_entry = Some(entry);
    }

    /// 清空预览目标与全部预览内容（焦点离开文件 / 目标消失时）。
    fn clear_preview(&mut self) {
        self.preview_path = None;
        self.preview_entry = None;
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        self.preview_bytes = None;
        self.preview_search.clear();
        self.preview_search_matches = Vec::new();
        self.preview_search_reveal = None;
        self.preview_full_size = false;
    }

    /// 每帧排空预览后台读取；在途时主动重绘（egui 空闲不重绘）。
    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(mut preview) = self.preview.take() else {
            return;
        };
        match preview.poll() {
            OpenStatus::Loading => {
                self.preview = Some(preview);
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            OpenStatus::Ready(result) => self.apply_preview_result(result),
        }
    }

    /// 应用预览后台结果：按分类解析初始模式（图片/文本/HEX），原始字节
    /// 留档供 HEX 查看与编码重解码（Note 带字节时——如图片解码失败——
    /// 也可 HEX）。
    fn apply_preview_result(&mut self, result: Result<PreviewOutcome, String>) {
        match result {
            Ok(outcome) => {
                self.preview_bytes = outcome.bytes;
                match outcome.data {
                    PreviewData::Image(img) => {
                        self.pending_preview_image = Some(img);
                        self.preview_mode = PreviewMode::Image;
                    }
                    PreviewData::Text(text) => {
                        self.preview_text = Some(text);
                        self.preview_mode = PreviewMode::Text;
                    }
                    // 二进制：不再显示「不支持预览」占位，直接落 HEX。
                    PreviewData::Unsupported => self.preview_mode = PreviewMode::Hex,
                    PreviewData::Note(note) => {
                        self.preview_note = Some(note);
                        if self.preview_bytes.is_some() {
                            self.preview_mode = PreviewMode::Hex;
                        }
                    }
                }
                self.refresh_preview_search();
            }
            Err(e) => self.preview_note = Some(e),
        }
    }

    /// 重算文本搜索匹配（搜索词变化 / 新文本加载 / 编码重解码后调用）。
    fn refresh_preview_search(&mut self) {
        self.preview_search_matches = match &self.preview_text {
            Some(text) => find_text_matches(text, &self.preview_search),
            None => Vec::new(),
        };
        self.preview_search_cur = 0;
    }

    /// 跳到下一个/上一个匹配（delta = ±1）：更新当前序号并置待揭示行。
    fn preview_search_step(&mut self, delta: isize) {
        let n = self.preview_search_matches.len();
        if n == 0 {
            return;
        }
        let cur = (self.preview_search_cur as isize + delta).rem_euclid(n as isize) as usize;
        self.preview_search_cur = cur;
        self.preview_search_reveal = Some(self.preview_search_matches[cur]);
    }

    /// 编码手动切换：用已读字节重解码（不重读文件）；解码失败显示说明
    /// 并保留原文本。
    fn preview_redecode(&mut self, enc: PreviewEncoding) {
        self.preview_encoding = enc;
        let Some(bytes) = self.preview_bytes.clone() else {
            return;
        };
        match decode_preview_text(&bytes, enc.decode_label()) {
            Some(text) => {
                self.preview_text = Some(text);
                self.preview_note = None;
                self.refresh_preview_search();
            }
            None => {
                self.preview_note = Some(format!("无法以 {} 解码", enc.label()));
            }
        }
    }

    /// F3 临时预览弹窗（双栏模式）：与单栏预览面板共用绘制代码。
    /// 最大化态铺满 CentralPanel 区域（egui Window 不支持自定义标题栏
    /// 按钮，最大化改走全屏 Area 自绘标题行：还原/关闭）。
    fn render_preview_window(&mut self, ctx: &egui::Context) {
        if !self.preview_window_open {
            return;
        }
        let title = self
            .preview_entry
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "预览".to_string());
        if self.preview_window_maximized {
            let rect = ctx.content_rect();
            let mut open = true;
            let mut restore = false;
            egui::Area::new(egui::Id::new("fm_preview_window_max"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    egui::Frame::window(&ctx.global_style()).show(ui, |ui| {
                        ui.set_min_size(rect.size());
                        ui.set_max_size(rect.size());
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&title).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("✕").clicked() {
                                        open = false;
                                    }
                                    if ui.button("还原").clicked() {
                                        restore = true;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        self.draw_preview_content(ui, false);
                    });
                });
            if restore {
                self.preview_window_maximized = false;
            }
            if !open {
                self.preview_window_open = false;
                self.preview_window_maximized = false;
            }
            return;
        }
        let mut open = true;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_size([420.0, 520.0])
            .open(&mut open)
            .show(ctx, |ui| {
                self.draw_preview_content(ui, true);
            });
        if !open {
            self.preview_window_open = false;
        }
    }

    /// 预览内容绘制（单栏预览面板 / Ctrl+Q 快览 / F3 弹窗共用；阶段 R）：
    /// 名称/大小/时间头部 + 模式 tab（文本/图片/HEX，按内容类型自动初值、
    /// 可手切，可用性按已加载内容门控）；文本模式头部 = 编码下拉（手动
    /// 重解码不重读文件）+ 搜索框（◀▶ 循环 + 计数，搜索激活时切换为行级
    /// 虚拟化视图做高亮/揭示）；图片模式 = 适应宽度/原始尺寸 + 显示层
    /// 旋转 90°（UV 角点轮换，不写文件）；HEX = format_hex_line 虚拟化。
    /// in_popup = F3 弹窗（头部多一个「最大化」按钮）。
    fn draw_preview_content(&mut self, ui: &mut egui::Ui, in_popup: bool) {
        // poll 收到的 ColorImage 在此（有 ctx）惰性上传为纹理。
        if let Some(img) = self.pending_preview_image.take() {
            self.preview_tex = Some(ui.ctx().load_texture(
                "fm-preview",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        let Some(entry) = self.preview_entry.clone() else {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("选中文件以预览").weak());
            return;
        };
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&entry.name).strong());
        let mut meta = entry.size.map(human_size).unwrap_or_default();
        let mtime = format_mtime(system_time_to_unix(entry.mtime));
        if !mtime.is_empty() {
            if !meta.is_empty() {
                meta.push_str(" · ");
            }
            meta.push_str(&mtime);
        }
        if !meta.is_empty() {
            ui.label(egui::RichText::new(meta).weak());
        }

        let has_text = self.preview_text.is_some();
        let has_image = self.preview_tex.is_some();
        let has_bytes = self.preview_bytes.is_some();
        // 当前模式指向不可用 tab（如重新解码失败清掉文本）→ 落回可用档。
        let mode_ok = match self.preview_mode {
            PreviewMode::Text => has_text,
            PreviewMode::Image => has_image,
            PreviewMode::Hex => has_bytes,
        };
        if !mode_ok {
            self.preview_mode = if has_text {
                PreviewMode::Text
            } else if has_image {
                PreviewMode::Image
            } else {
                PreviewMode::Hex
            };
        }
        if has_text || has_image || has_bytes {
            ui.horizontal(|ui| {
                for (mode, label, enabled) in [
                    (PreviewMode::Text, "文本", has_text),
                    (PreviewMode::Image, "图片", has_image),
                    (PreviewMode::Hex, "HEX", has_bytes),
                ] {
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::selectable(self.preview_mode == mode, label)
                                .frame_when_inactive(false),
                        )
                        .clicked()
                    {
                        self.preview_mode = mode;
                    }
                }
                // F3 弹窗：最大化（egui Window 标题栏不支持自定义按钮，
                // 放内容头部；最大化态为全屏 Area 自绘标题行）。
                if in_popup && !self.preview_window_maximized {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("最大化").clicked() {
                            self.preview_window_maximized = true;
                        }
                    });
                }
            });
        }
        // 模式专属头部行。
        match self.preview_mode {
            PreviewMode::Image if has_image => {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(!self.preview_full_size, "适应宽度")
                        .clicked()
                    {
                        self.preview_full_size = false;
                    }
                    if ui
                        .selectable_label(self.preview_full_size, "原始尺寸")
                        .clicked()
                    {
                        self.preview_full_size = true;
                    }
                    ui.separator();
                    if ui
                        .button("旋转 90°")
                        .on_hover_text("仅显示层旋转，不写文件")
                        .clicked()
                    {
                        self.preview_rotation = (self.preview_rotation + 1) % 4;
                    }
                });
            }
            PreviewMode::Text if has_text => {
                ui.horizontal(|ui| {
                    ui.label("编码");
                    let mut selected = None;
                    egui::ComboBox::from_id_salt("fm_preview_encoding")
                        .selected_text(self.preview_encoding.label())
                        .show_ui(ui, |ui| {
                            for enc in PreviewEncoding::ALL {
                                if ui
                                    .selectable_label(self.preview_encoding == enc, enc.label())
                                    .clicked()
                                {
                                    selected = Some(enc);
                                }
                            }
                        });
                    if let Some(enc) = selected {
                        self.preview_redecode(enc);
                    }
                });
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.preview_search)
                            .desired_width(140.0)
                            .hint_text("搜索文本"),
                    );
                    if resp.changed() {
                        self.refresh_preview_search();
                    }
                    let n = self.preview_search_matches.len();
                    if !self.preview_search.is_empty() {
                        if ui
                            .add_enabled(n > 0, egui::Button::new("◀"))
                            .on_hover_text("上一个")
                            .clicked()
                        {
                            self.preview_search_step(-1);
                        }
                        if ui
                            .add_enabled(n > 0, egui::Button::new("▶"))
                            .on_hover_text("下一个")
                            .clicked()
                        {
                            self.preview_search_step(1);
                        }
                        let count = if n == 0 {
                            "无匹配".to_string()
                        } else {
                            format!("第 {}/{} 处", self.preview_search_cur + 1, n)
                        };
                        ui.label(egui::RichText::new(count).weak());
                    }
                });
            }
            _ => {}
        }
        ui.separator();

        match self.preview_mode {
            PreviewMode::Image if has_image => self.draw_preview_image(ui),
            PreviewMode::Text if has_text => self.draw_preview_text(ui),
            PreviewMode::Hex if has_bytes => self.draw_preview_hex(ui),
            _ => {}
        }
        if let Some(note) = &self.preview_note {
            ui.label(egui::RichText::new(note).weak());
        }
        if self.preview.is_some()
            && self.preview_tex.is_none()
            && self.preview_text.is_none()
            && self.preview_note.is_none()
            && self.preview_bytes.is_none()
        {
            ui.label(egui::RichText::new("正在读取…").weak());
        }
    }

    /// 图片模式内容：适应宽度（单滚动）/ 原始尺寸（双向滚动），均经
    /// paint_rotated_image 应用显示层旋转（90° 步进时交换宽高考量）。
    fn draw_preview_image(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = &self.preview_tex else {
            return;
        };
        let tex_id = tex.id();
        let size = tex.size_vec2();
        let k = self.preview_rotation % 4;
        let (dw, dh) = if k % 2 == 1 {
            (size.y, size.x)
        } else {
            (size.x, size.y)
        };
        if self.preview_full_size {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(dw, dh), egui::Sense::hover());
                    paint_rotated_image(ui.painter(), tex_id, rect, k);
                });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width();
                let height = if dw > 0.0 { width * dh / dw } else { width };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                paint_rotated_image(ui.painter(), tex_id, rect, k);
            });
    }

    /// 文本模式内容：无搜索词 = 只读 TextEdit（自动换行、可选中复制，维持
    /// 原手感）；有搜索词 = 行级虚拟化视图（匹配行底色高亮，当前匹配 =
    /// 选中色，◀▶ 跳转后顶对齐揭示）。
    fn draw_preview_text(&mut self, ui: &mut egui::Ui) {
        let Some(text) = &self.preview_text else {
            return;
        };
        if self.preview_search.is_empty() {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // 只读 &str 缓冲（TextBuffer for &str 拒绝修改）：可选中复制。
                    let mut text = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
            return;
        }
        let text = text.clone();
        let matches = self.preview_search_matches.clone();
        let cur_line = matches.get(self.preview_search_cur).copied();
        let reveal = self.preview_search_reveal.take();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        let lines: Vec<&str> = text.lines().collect();
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(line) = reveal {
            area = area.vertical_scroll_offset(line as f32 * row_h);
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let sel = ui.visuals().selection.bg_fill;
        let faint = ui.visuals().faint_bg_color;
        let text_color = ui.visuals().text_color();
        area.show_rows(ui, row_h, lines.len(), |ui, range| {
            for i in range {
                let line = lines.get(i).copied().unwrap_or("");
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_h),
                    egui::Sense::hover(),
                );
                if matches.binary_search(&i).is_ok() {
                    let color = if Some(i) == cur_line { sel } else { faint };
                    ui.painter().rect_filled(rect, 0.0, color);
                }
                ui.painter().text(
                    egui::pos2(rect.min.x + 4.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    line,
                    egui::FontId::monospace(row_h * 0.85),
                    text_color,
                );
            }
        });
    }

    /// HEX 模式内容：16 字节/行虚拟化（行按需 format_hex_line，不物化全表）。
    fn draw_preview_hex(&mut self, ui: &mut egui::Ui) {
        let Some(bytes) = self.preview_bytes.clone() else {
            return;
        };
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        let line_count = bytes.len().div_ceil(16);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, line_count, |ui, range| {
                for line in range {
                    if let Some(text) = format_hex_line(&bytes, line) {
                        ui.monospace(text);
                    }
                }
            });
    }
}

/// 显示层图片旋转（阶段 R 预览「旋转 90°」）：Mesh 顶点固定、UV 角点
/// 轮换实现顺时针 k×90°，纹理与文件不动。rect 由调用方按旋转后宽高比
/// 分配（k 为奇数时交换宽/高）。
fn paint_rotated_image(painter: &egui::Painter, tex: egui::TextureId, rect: egui::Rect, k: u8) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let uv = [
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ];
    let k = k as usize % 4;
    let mut mesh = egui::Mesh::with_texture(tex);
    for i in 0..4 {
        // 顺时针：显示角 i 采样原图角 (i + 4 - k) % 4。
        mesh.vertices.push(egui::epaint::Vertex {
            pos: corners[i],
            uv: uv[(i + 4 - k) % 4],
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// 竖向分隔条：拖动返回本帧的水平位移（px），hover/拖拽时换光标。
fn render_splitter(ui: &mut egui::Ui, height: f32) -> Option<f32> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SPLITTER_WIDTH, height), egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let color = if response.dragged() || response.hovered() {
        ui.visuals().widgets.active.bg_stroke.color
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0, color),
    );
    if response.dragged() {
        let dx = response.drag_motion().x;
        (dx != 0.0).then_some(dx)
    } else {
        None
    }
}

/// 栏当前目录的根展示名（Windows `C:\`；Unix `/`）；空路径回退 `/`。
/// 复用面包屑首段（Windows Prefix+Root 已合并为 `C:\`）。
fn drive_label(dir: &Path) -> String {
    breadcrumb_segments(dir)
        .first()
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| "/".to_string())
}

/// 面包屑分段：根（`C:\` 或 `/`）+ 各级目录名，每段带其完整路径。
fn breadcrumb_segments(path: &Path) -> Vec<(PathBuf, String)> {
    let mut segments: Vec<(PathBuf, String)> = Vec::new();
    let mut cur = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(prefix) => {
                cur.push(prefix.as_os_str());
                segments.push((
                    cur.clone(),
                    prefix.as_os_str().to_string_lossy().into_owned(),
                ));
            }
            Component::RootDir => {
                cur.push(comp.as_os_str());
                match segments.last_mut() {
                    // Windows：`C:` + 根 → 显示 `C:\`。
                    Some(last) => {
                        last.0 = cur.clone();
                        last.1.push('\\');
                    }
                    None => segments.push((cur.clone(), "/".to_string())),
                }
            }
            Component::Normal(name) => {
                cur.push(name);
                segments.push((cur.clone(), name.to_string_lossy().into_owned()));
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    segments
}

/// 明细列表行的类型图标（按扩展名，大小写不敏感）；目录行恒用 FOLDER。
/// 名称语义着色（明细行与网格共用；固定规则，不做用户配色）：
/// 目录 = accent 色（hyperlink_color，主题派生深浅均可读；egui/epaint 无
/// 字重支持——strong() 仅为更强颜色、未注册 bold 字族，故以 accent 色
/// 承担目录强调）；符号链接 = 斜体 + 弱一档；隐藏条目 = 弱档 0.6（同
/// dir_sizes 先例）；普通文件 = 默认文字色。选中行颜色统一回退
/// text_color()（选中底色上保持对比度——现状选中行即此色；斜体字形
/// 保留）。颜色全部经 visuals 派生，不硬编码色值。
fn entry_name_rich_text(
    name: &str,
    e: &FsEntry,
    visuals: &egui::Visuals,
    selected: bool,
) -> egui::RichText {
    let mut text = egui::RichText::new(name);
    if e.is_symlink {
        text = text.italics();
    }
    let base = if e.is_dir {
        visuals.hyperlink_color
    } else if e.is_symlink {
        visuals.weak_text_color()
    } else {
        visuals.text_color()
    };
    let color = if selected {
        visuals.text_color()
    } else if e.is_hidden {
        base.gamma_multiply(0.6)
    } else {
        base
    };
    text.color(color)
}

fn entry_icon(entry: &FsEntry) -> Icon {
    if entry.is_dir {
        return icons::FOLDER;
    }
    if archive_kind(&entry.path).is_some() {
        return icons::FILE_ARCHIVE;
    }
    let ext = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        e if openitgo_parser::traits::is_image_extension(e) => icons::FILE_IMAGE,
        "txt" | "md" | "log" => icons::FILE_TEXT,
        "pdf" => icons::FILE_PDF,
        "mp4" | "mkv" | "avi" | "mov" | "webm" => icons::FILE_VIDEO,
        "mp3" | "flac" | "aac" | "ogg" | "wav" => icons::FILE_AUDIO,
        _ => icons::FILE,
    }
}

/// 名称列与「大小」「修改时间」列之间的淡竖线（表头与数据行共用；
/// 坐标取自 column_layout，与表头列区间一致）。
fn paint_column_separators(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    layout: &ColumnLayout,
) {
    let stroke = egui::Stroke::new(1.0, color);
    for (_, left, _) in &layout.fixed {
        painter.vline(*left, rect.y_range(), stroke);
    }
}

/// 键盘焦点的最小滚动（Explorer 语义）：焦点行已在视口内则偏移不变；
/// 上方越界贴顶（offset = 行顶），下方越界贴底（offset = 行底 - 视口高）。
fn min_scroll_to_reveal(offset: f32, viewport_h: f32, row_top: f32) -> f32 {
    let row_bottom = row_top + ROW_HEIGHT;
    if row_top < offset {
        row_top
    } else if row_bottom > offset + viewport_h {
        row_bottom - viewport_h
    } else {
        offset
    }
}

/// SystemTime → Unix 秒（本地时间格式化在 archive::format_mtime 里做）。
fn system_time_to_unix(t: Option<std::time::SystemTime>) -> Option<i64> {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// 命令行执行（阶段 X）：Windows `cmd /c` / 其余 `sh -c`，工作目录 =
/// 焦点栏目录；后台 spawn——不捕获输出（命令自己的控制台/终端可见）、
/// 不阻塞 UI、不等待退出。
fn spawn_shell_command(dir: &Path, input: &str) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/c");
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg("-c");
        c
    };
    cmd.arg(input).current_dir(dir);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("命令启动失败: {e}"))
}

/// 按钮栏命令占位符展开（阶段 Y）：`%P` = 焦点栏当前目录、`%N` = 焦点项
/// 名称（无焦点 = 空串）、`%p` = 另一栏目录。不提供转义，未知占位符原样
/// 保留（replace 天然如此）。
fn expand_button_command(
    cmd: &str,
    focus_dir: &str,
    focus_name: Option<&str>,
    other_dir: &str,
) -> String {
    cmd.replace("%P", focus_dir)
        .replace("%N", focus_name.unwrap_or(""))
        .replace("%p", other_dir)
}

/// 网格空白区几何（双击回上级判定用）：cell 定宽左排，每行右侧余量与
/// 末行未排满部分都算空白。
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridBlankGeom {
    /// 网格行高（cell 高）。
    row_pitch: f32,
    /// 排满的完整网格行数。
    full_rows: usize,
    /// 完整行的内容宽度（cols × cell 宽）。
    full_width: f32,
    /// 末行实际占用宽度（末行排满时 = full_width）。
    last_width: f32,
}

/// 空白区双击命中判定（纯函数）：指针在视口内且落在内容区之外——内容底
/// 以下任意位置；网格模式（grid = Some）还包括每行右侧未占用部分。
/// 内容坐标 = 指针位置 - 视口左上 + 竖向滚动偏移。
fn dblclick_hits_blank(
    viewport: egui::Rect,
    scroll_y: f32,
    content_height: f32,
    grid: Option<GridBlankGeom>,
    pos: egui::Pos2,
) -> bool {
    if !viewport.contains(pos) {
        return false;
    }
    let content_y = pos.y - viewport.min.y + scroll_y;
    if content_y >= content_height {
        return true;
    }
    if let Some(g) = grid {
        let row = (content_y / g.row_pitch).floor().max(0.0) as usize;
        let filled = if row < g.full_rows {
            g.full_width
        } else {
            g.last_width
        };
        let content_x = pos.x - viewport.min.x;
        if content_x >= filled {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_history_capped_dedupes_and_truncates() {
        let mut h: Vec<String> = Vec::new();
        push_history_capped(&mut h, "  *.zip ", 3);
        push_history_capped(&mut h, "漫画", 3);
        assert_eq!(h, ["漫画", "*.zip"]);
        // 空串忽略。
        push_history_capped(&mut h, "   ", 3);
        assert_eq!(h, ["漫画", "*.zip"]);
        // 重复置顶。
        push_history_capped(&mut h, "*.zip", 3);
        assert_eq!(h, ["*.zip", "漫画"]);
        // 超 cap 截尾。
        push_history_capped(&mut h, "a", 3);
        push_history_capped(&mut h, "b", 3);
        assert_eq!(h, ["b", "a", "*.zip"]);
    }

    #[test]
    fn rows_in_rect_hits_vertical_overlap() {
        let pitch = 22.0;
        // 覆盖行 1..=2（内容坐标 y 22..66）。
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 22.5), egui::pos2(100.0, 60.0));
        assert_eq!(rows_in_rect(5, pitch, r), vec![1, 2]);
        // 底边恰好压行界（y=66 = 行 3 顶）不命中行 3。
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 22.5), egui::pos2(100.0, 66.0));
        assert_eq!(rows_in_rect(5, pitch, r), vec![1, 2]);
        // 顶边压行界（y=22 = 行 1 顶）命中行 1。
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 22.0), egui::pos2(100.0, 23.0));
        assert_eq!(rows_in_rect(5, pitch, r), vec![1]);
        // 完全在内容之下 = 空；越界底自动钳到末行。
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 200.0), egui::pos2(100.0, 300.0));
        assert!(rows_in_rect(5, pitch, r).is_empty());
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 90.0), egui::pos2(100.0, 500.0));
        assert_eq!(rows_in_rect(5, pitch, r), vec![4]);
        // 反拖（min>max 已由 from_two_pos 规范化，这里直接验证退化矩形）。
        assert!(rows_in_rect(0, pitch, r).is_empty());
        assert!(rows_in_rect(5, 0.0, r).is_empty());
    }

    #[test]
    fn cells_in_rect_hits_grid_region() {
        // 3 列 × 176×200，7 项（末行 1 格）。
        let (cols, w, h, n) = (3, 176.0, 200.0, 7);
        // 左上角 2×2。
        let r = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(200.0, 210.0));
        assert_eq!(cells_in_rect(n, cols, w, h, r), vec![0, 1, 3, 4]);
        // 末行未排满：item 6 存在、item 7/8 丢弃。
        let r = egui::Rect::from_min_max(egui::pos2(0.0, 400.0), egui::pos2(528.0, 600.0));
        assert_eq!(cells_in_rect(n, cols, w, h, r), vec![6]);
        // 行右空白（x ≥ 528）不命中任何 cell。
        let r = egui::Rect::from_min_max(egui::pos2(530.0, 10.0), egui::pos2(700.0, 190.0));
        assert!(cells_in_rect(n, cols, w, h, r).is_empty());
        // 右边压界（x=352 = 列 2 左缘）不命中列 2。
        let r = egui::Rect::from_min_max(egui::pos2(180.0, 10.0), egui::pos2(352.0, 190.0));
        assert_eq!(cells_in_rect(n, cols, w, h, r), vec![1]);
    }

    #[test]
    fn breadcrumb_segments_windows_style() {
        // 不依赖真实文件系统，纯路径解析（Windows 前缀在非 Windows 上
        // 走 Normal 分支，此处只验证分段语义不验证盘符）。
        let segs = breadcrumb_segments(Path::new("/a/b/c"));
        let labels: Vec<&str> = segs.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["/", "a", "b", "c"]);
        assert_eq!(segs[2].0, PathBuf::from("/a/b"));
    }

    #[test]
    fn drive_label_takes_root_segment() {
        assert_eq!(drive_label(Path::new("/a/b")), "/");
        assert_eq!(drive_label(Path::new("")), "/");
        #[cfg(windows)]
        assert_eq!(drive_label(Path::new(r"C:\foo\bar")), r"C:\");
    }

    #[test]
    fn min_scroll_only_when_out_of_view() {
        // 已在视口内：不动
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 150.0), 100.0);
        // 上方越界：贴顶
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 50.0), 50.0);
        // 下方越界：贴底
        assert_eq!(min_scroll_to_reveal(100.0, 200.0, 280.0), 102.0);
    }

    #[test]
    fn drag_sources_selected_set_or_single_row() {
        let a = PathBuf::from("/x/a");
        let b = PathBuf::from("/x/b");
        let c = PathBuf::from("/x/c");
        let selected = HashSet::from([b.clone(), a.clone()]);
        // 拖选中行 → 整个选中集（排序后确定）。
        assert_eq!(drag_sources(&selected, &a), vec![a.clone(), b.clone()]);
        // 拖未选中行 → 仅该行自身。
        assert_eq!(drag_sources(&selected, &c), vec![c.clone()]);
    }

    #[test]
    fn find_text_matches_case_insensitive_line_indices() {
        let text = "第一行 Alpha\nsecond line\n第三个 ALPHA 行\n\nlast";
        assert_eq!(find_text_matches(text, "alpha"), [0, 2]);
        assert_eq!(find_text_matches(text, "ALPHA"), [0, 2]);
        assert_eq!(find_text_matches(text, "第三"), [2]);
        assert!(find_text_matches(text, "").is_empty());
        assert!(find_text_matches(text, "不存在").is_empty());
        assert_eq!(find_text_matches(text, "last"), [4]);
    }

    #[test]
    fn dblclick_blank_hit_detection() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 400.0));
        // 列表模式（grid = None）：内容底以下 = 空白，其余不是。
        assert!(dblclick_hits_blank(
            viewport,
            0.0,
            300.0,
            None,
            egui::pos2(250.0, 350.0)
        ));
        assert!(!dblclick_hits_blank(
            viewport,
            0.0,
            300.0,
            None,
            egui::pos2(250.0, 100.0)
        ));
        // 视口外不命中。
        assert!(!dblclick_hits_blank(
            viewport,
            0.0,
            300.0,
            None,
            egui::pos2(600.0, 350.0)
        ));
        // 滚动偏移参与内容坐标换算：内容高 600，scroll 0 时 y=350 仍在内容内；
        // 滚下 300 后同一屏幕位置对应内容 y=650，越过内容底 = 空白。
        assert!(!dblclick_hits_blank(
            viewport,
            0.0,
            600.0,
            None,
            egui::pos2(250.0, 350.0)
        ));
        assert!(dblclick_hits_blank(
            viewport,
            300.0,
            600.0,
            None,
            egui::pos2(250.0, 350.0)
        ));
        // 内容高于视口（滚到底后无空白区）。
        assert!(!dblclick_hits_blank(
            viewport,
            200.0,
            600.0,
            None,
            egui::pos2(250.0, 399.0)
        ));

        // 网格模式：3 列 × 176 宽，7 项 → 末行 1 格（宽 176）。
        let geom = GridBlankGeom {
            row_pitch: 200.0,
            full_rows: 2,
            full_width: 528.0,
            last_width: 176.0,
        };
        let h = 3.0 * 200.0;
        // 完整行右侧余量 = 空白（视口宽 500 < full_width 528 时该分支按几何仍判；
        // 用宽视口验证典型形态）。
        let wide = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 700.0));
        assert!(dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(geom),
            egui::pos2(600.0, 50.0)
        ));
        assert!(!dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(geom),
            egui::pos2(100.0, 50.0)
        ));
        // 末行：未排满部分（x ≥ 176）= 空白，已占用格不是。
        assert!(dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(geom),
            egui::pos2(200.0, 450.0)
        ));
        assert!(!dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(geom),
            egui::pos2(100.0, 450.0)
        ));
        // 末行以下 = 空白。
        assert!(dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(geom),
            egui::pos2(100.0, 650.0)
        ));
        // 末行排满（partial == 0 → last_width = full_width）：行内右余量仍空白。
        let full = GridBlankGeom {
            last_width: 528.0,
            ..geom
        };
        assert!(!dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(full),
            egui::pos2(400.0, 450.0)
        ));
        assert!(dblclick_hits_blank(
            wide,
            0.0,
            h,
            Some(full),
            egui::pos2(600.0, 450.0)
        ));
    }

    #[test]
    fn bookmark_groups_add_dedup_remove_and_snapshot() {
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        let a = PathBuf::from("/a");
        let b = PathBuf::from("/b");

        // 分组管理：空名/重名 no-op。
        assert!(!view.add_group("  "));
        assert!(view.add_group("常用"));
        assert!(!view.add_group("常用"));
        assert!(view.add_group("下载"));

        assert!(view.add_bookmark_to_group(0, &a));
        assert!(view.add_bookmark_to_group(0, &b));
        // 组内去重：重复添加 no-op；越界组 no-op。
        assert!(!view.add_bookmark_to_group(0, &a));
        assert!(!view.add_bookmark_to_group(9, &a));
        let groups = &view.snapshot().bookmark_groups;
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "常用");
        assert_eq!(groups[0].items, ["/a".to_string(), "/b".to_string()]);
        assert!(groups[1].items.is_empty());

        assert!(view.remove_bookmark(0, &a));
        assert!(!view.remove_bookmark(0, &a));
        assert_eq!(view.snapshot().bookmark_groups[0].items, ["/b".to_string()]);

        // 重命名（与他组重名拒绝）+ 删除分组。
        assert!(!view.rename_group(0, "下载"));
        assert!(view.rename_group(0, "收藏"));
        assert_eq!(view.snapshot().bookmark_groups[0].name, "收藏");
        view.remove_group(1);
        assert_eq!(view.snapshot().bookmark_groups.len(), 1);
    }

    #[test]
    fn snapshot_restores_bookmark_groups_from_constructor() {
        let saved = vec![
            FmBookmarkGroup {
                name: "常用".to_string(),
                items: vec!["/a".to_string(), "/b".to_string()],
            },
            FmBookmarkGroup {
                name: "空组".to_string(),
                items: Vec::new(),
            },
        ];
        let view = FileManagerView::new("dual", 0.5, false, "name", true, &saved);
        assert_eq!(view.snapshot().bookmark_groups, saved);
    }

    // ---- 无头 egui 测试基座：注入输入事件驱动 FileManagerView 真实渲染帧 ----

    /// 跑一帧真实渲染（含行交互/意图分发），events 为本帧注入的输入。
    fn headless_frame(
        ctx: &egui::Context,
        view: &mut FileManagerView,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        // InputState.modifiers 取自 RawInput.modifiers（不从 Key 事件推导），
        // 这里从注入事件回填，使带修饰键的快捷键测试生效。
        let modifiers = events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(time),
            modifiers,
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            view.ui(
                ui,
                FmCallbacks {
                    on_back: &mut || {},
                    on_open_path: &mut |_| {},
                    on_open_archive: &mut |_| {},
                    on_open_as_comic: &mut |_| {},
                    on_extract: &mut |_, _| {},
                    on_op_error: &mut |_| {},
                    on_confirm_delete_change: &mut |_| {},
                },
                // 测试基座：确认框关、其余默认（= 现状行为）。
                FmBehaviorOptions {
                    confirm_delete: false,
                    ..Default::default()
                },
            );
        });
    }

    fn primary_click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = egui::PointerButton::Primary;
        vec![
            egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    /// navigate 并 poll 到 Ready（AsyncOpener 后台线程）。
    fn navigate_ready(panel: &mut FsPanel, dir: &Path) {
        panel.navigate_to(dir.to_path_buf());
        for _ in 0..400 {
            if !panel.poll() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("目录列举未在限时内完成");
    }

    /// epaint 0.35 对未注册的字体族直接 panic：FM 行/按钮用磷图标字体
    /// （fonts.rs 在 lib crate root，bin 测试目标够不到，此处内联最小版）。
    fn setup_test_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor_icons::add_fonts(&mut fonts);
        ctx.set_fonts(fonts);
    }

    /// 回归（双击目录闪退）：双击目录行同帧 navigate_to 清空 entries，
    /// render_list 的循环若继续按导航前 rows 快照渲染后续行，render_row
    /// 里 entries[i] 越界 panic。修复后状态离开 Ready 即停笔。
    #[test]
    fn double_click_dir_mid_frame_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        for d in ["d1", "d2", "d3"] {
            std::fs::create_dir(tmp.path().join(d)).unwrap();
        }
        for f in ["f1", "f2", "f3"] {
            std::fs::write(tmp.path().join(f), b"x").unwrap();
        }
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧

        // 自上而下单击扫描，定位 UI 行 1（第一个目录 d1）的 y。
        // y 从 95 起跳过顶栏/面包屑/列头（面包屑单击会导航、污染扫描）。
        let mut row1_pos = None;
        let mut y = 95.0;
        while y < 500.0 {
            t += 1.0; // 间隔超过 max_double_click_delay，避免连击计数干扰
            let pos = egui::pos2(200.0, y);
            headless_frame(&ctx, &mut view, t, primary_click_events(pos));
            if view.panels[0].focus == Some(1) {
                row1_pos = Some(pos);
                break;
            }
            y += 5.0;
        }
        let pos = row1_pos.expect("未能定位到行 1（d1）");
        // 扫描期间的点击可能改动选中态；重置回被测目录（布局不变，y 仍有效）。
        navigate_ready(&mut view.panels[0], tmp.path());
        headless_frame(&ctx, &mut view, t, vec![]);

        // 双击 d1 → navigate_to 同帧清 entries：修复前下一行渲染即越界 panic。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        t += 0.1;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        for _ in 0..20 {
            t += 1.0;
            view.panels[0].poll();
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert!(view.panels[0].dir.ends_with("d1"));
        assert!(matches!(view.panels[0].state, PanelLoadState::Ready));
    }

    /// 回归（双栏滚动串扰）：两栏 ScrollArea 曾共享 auto-id，滚动状态
    /// 互相跟随。修复后每栏 push_id 隔离，滚左栏右栏不动、反之亦然。
    #[test]
    fn dual_panels_scroll_independently() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..200 {
            std::fs::write(tmp.path().join(format!("f{i:03}")), b"x").unwrap();
        }
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        navigate_ready(&mut view.panels[1], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧

        let wheel_down = |pos: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -5.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ]
        };

        // 滚左栏：右栏不应跟随。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, wheel_down(egui::pos2(300.0, 400.0)));
        let left_after = view.panels[0].last_scroll_offset;
        assert!(left_after > 0.0, "滚轮应滚动左栏");
        assert_eq!(
            view.panels[1].last_scroll_offset, 0.0,
            "右栏不应跟随左栏滚动"
        );

        // 滚右栏：左栏保持不动。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, wheel_down(egui::pos2(980.0, 400.0)));
        assert!(view.panels[1].last_scroll_offset > 0.0, "滚轮应滚动右栏");
        assert_eq!(
            view.panels[0].last_scroll_offset, left_after,
            "左栏不应跟随右栏滚动"
        );
    }

    /// 栏间拖放：从左栏拖行到右栏松开 → payload 设置/传递 → 弹既有
    /// 「复制到…」确认框（不直拷）；拖到源栏自身松开则无事发生。
    #[test]
    fn drag_row_to_other_panel_opens_copy_dialog() {
        let tmp_left = tempfile::tempdir().unwrap();
        let tmp_right = tempfile::tempdir().unwrap();
        std::fs::write(tmp_left.path().join("a.txt"), b"x").unwrap();
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp_left.path());
        navigate_ready(&mut view.panels[1], tmp_right.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.txt");
        assert_eq!(view.active, 0, "行定位应落在左栏");

        let button = egui::PointerButton::Primary;
        let mods = egui::Modifiers::NONE;
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: mods,
        };
        // 按下后移动超过拖拽阈值（6pt）→ drag_started → payload 设置。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![press(pos, true)]);
        t += 0.1;
        let mid = egui::pos2(pos.x + 30.0, pos.y + 10.0);
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(mid)]);
        assert!(
            egui::DragAndDrop::has_payload_of_type::<FmDragPayload>(&ctx),
            "拖动行应设置 dnd payload"
        );

        // 拖到右栏松开 → 弹复制确认框，payload 被取走。
        t += 0.1;
        let drop_pos = egui::pos2(1000.0, 400.0);
        headless_frame(
            &ctx,
            &mut view,
            t,
            vec![egui::Event::PointerMoved(drop_pos), press(drop_pos, false)],
        );
        assert!(
            matches!(view.dialog, Some(FmDialog::CopyMove(_))),
            "drop 到另一栏应弹「复制到…」确认框"
        );
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));

        // 关对话框；再拖到源栏自身松开 → 不弹框。
        view.dialog = None;
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![press(pos, true)]);
        t += 0.1;
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(mid)]);
        t += 0.1;
        headless_frame(&ctx, &mut view, t, vec![press(mid, false)]);
        assert!(view.dialog.is_none(), "拖回源栏不应弹框");
    }

    /// 阶段 Z：拖到面包屑段松开 = 复制到该段目录（沿用 fm_drag_confirm
    /// 确认框）；面包屑落点在单栏也接收（优先于栏体落点、Dual 早退之前）。
    #[test]
    fn drag_row_to_breadcrumb_segment_opens_copy_dialog() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a.txt"), b"x").unwrap();
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], &sub);
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]); // 布局帧

        // 面包屑落点 rect 记录：末段 = 当前目录，且含父目录段。
        assert_eq!(
            view.breadcrumb_drop_rects.last().map(|(p, _)| p),
            Some(&sub),
            "面包屑末段应为当前目录"
        );
        let seg_rect = view
            .breadcrumb_drop_rects
            .iter()
            .find(|(p, _)| *p == tmp.path())
            .map(|(_, r)| *r)
            .expect("面包屑应含父目录段");

        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.txt");
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // 起拖（按下 + 位移超过拖拽阈值）。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![press(pos, true)]);
        t += 0.1;
        let mid = egui::pos2(pos.x + 30.0, pos.y + 10.0);
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(mid)]);
        assert!(egui::DragAndDrop::has_payload_of_type::<FmDragPayload>(
            &ctx
        ));

        // 按住悬停父目录段：只高亮，不弹框。
        let seg_pos = seg_rect.center();
        t += 0.1;
        headless_frame(&ctx, &mut view, t, vec![egui::Event::PointerMoved(seg_pos)]);
        assert!(view.dialog.is_none(), "悬停落点段不应弹框");

        // 松开 → 弹「复制到 tmp」确认框，payload 清空。
        t += 0.1;
        headless_frame(
            &ctx,
            &mut view,
            t,
            vec![egui::Event::PointerMoved(seg_pos), press(seg_pos, false)],
        );
        assert!(
            matches!(view.dialog, Some(FmDialog::CopyMove(_))),
            "drop 到面包屑段应弹「复制到…」确认框"
        );
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
    }

    /// 阶段 Z：标签落点 rect 只记录非当前标签；当前目录天然被 poll 跳过。
    #[test]
    fn tab_drop_rects_track_inactive_tabs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        headless_frame(&ctx, &mut view, 0.0, vec![]); // 布局帧
        assert!(view.tab_drop_rects.is_empty(), "单标签时不应有标签落点");

        // 新开标签（复制当前目录并切过去）：原标签成为落点。
        view.panels[0].new_tab();
        headless_frame(&ctx, &mut view, 1.0, vec![]);
        assert_eq!(view.tab_drop_rects.len(), 1, "只记录非当前标签");
        assert_eq!(view.tab_drop_rects[0].0, tmp.path());
    }

    // ---- 预览链路回归（查看预览崩溃）----

    fn key_events(key: egui::Key) -> Vec<egui::Event> {
        key_events_mod(key, egui::Modifiers::NONE)
    }

    fn key_events_mod(key: egui::Key, mods: egui::Modifiers) -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: mods,
            },
        ]
    }

    /// 2x2 红 PNG 字节。
    fn test_png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 单击扫描定位「指定文件名的焦点行」（跳过顶栏/面包屑/列头）。
    fn locate_file_row(
        ctx: &egui::Context,
        view: &mut FileManagerView,
        t: &mut f64,
        file_name: &str,
    ) -> egui::Pos2 {
        let mut y = 95.0;
        while y < 500.0 {
            *t += 1.0;
            let pos = egui::pos2(200.0, y);
            headless_frame(ctx, view, *t, primary_click_events(pos));
            let hit = view.panels[view.active]
                .focused_entry()
                .is_some_and(|e| e.name == file_name);
            if hit {
                return pos;
            }
            y += 5.0;
        }
        panic!("未能定位到文件行 {file_name}");
    }

    /// 单栏预览：选中图片行 → 后台加载 → 纹理上传渲染若干帧；
    /// 再切「原始尺寸」模式渲染（独立双向滚动区路径）。
    #[test]
    fn preview_image_load_and_render() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "hello").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");

        // 单击图片行 → 焦点 → sync_preview_target 发起后台读取。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        assert!(view.preview.is_some(), "应发起预览读取");
        // 排空加载并渲染（含纹理上传帧）。
        for _ in 0..30 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
            if view.preview_tex.is_some() {
                break;
            }
        }
        assert!(view.preview_tex.is_some(), "图片预览应已上传纹理");
        // 原始尺寸模式（ScrollArea::both 路径）。
        view.preview_full_size = true;
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
    }

    /// 竞态：预览在途时导航离开（clear_preview 先行），结果晚到不应 panic。
    #[test]
    fn preview_navigate_away_while_loading() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.txt"), "x").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        assert!(view.preview.is_some());
        // 加载在途时直接导航进子目录（焦点清空 → clear_preview）。
        view.panels[0].navigate_to(sub.clone());
        for _ in 0..30 {
            t += 1.0;
            view.panels[0].poll();
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert_eq!(view.panels[0].dir, sub);
        assert!(matches!(view.panels[0].state, PanelLoadState::Ready));
    }

    /// 损坏图片（扩展名 png、内容垃圾）→ Note 路径渲染。
    #[test]
    fn preview_corrupt_image_note() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), b"not a png at all").unwrap();
        let mut view = FileManagerView::new("single", 0.5, true, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        for _ in 0..30 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
            if view.preview_note.is_some() {
                break;
            }
        }
        assert!(view.preview_note.is_some(), "损坏图片应走 Note 说明");
    }

    /// F3 弹窗（双栏）：打开 → 移动焦点换内容 → Esc 关闭，全程渲染。
    #[test]
    fn preview_f3_window_open_switch_close() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), test_png_bytes()).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("c.bin"), [0xFF, 0xFE, 0x00]).unwrap();
        let mut view = FileManagerView::new("dual", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        navigate_ready(&mut view.panels[1], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        let pos = locate_file_row(&ctx, &mut view, &mut t, "a.png");
        // 焦点落到 a.png 后开 F3 弹窗。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, primary_click_events(pos));
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::F3));
        assert!(view.preview_window_open, "F3 应打开预览弹窗");
        // 加载 + 渲染若干帧（弹窗与面板同帧共存）。
        for _ in 0..10 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        // ↓ 移焦点到 b.txt（弹窗内容切换），再 ↓ 到 c.bin（不支持类型）。
        for _ in 0..2 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, key_events(egui::Key::ArrowDown));
            for _ in 0..5 {
                t += 1.0;
                headless_frame(&ctx, &mut view, t, vec![]);
            }
        }
        // Esc 关弹窗。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::Escape));
        assert!(!view.preview_window_open, "Esc 应关闭弹窗");
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
    }

    /// 列布局泛化回归（阶段 V）：默认三列（Name+Size:90+Mtime:110）的
    /// 坐标必须与旧硬编码公式逐项等价——content_right = right-6+shift；
    /// Mtime (mtime_left, content_right)；Size (size_left, mtime_left)；
    /// name_right = size_left。
    #[test]
    fn column_layout_default_matches_legacy_formula() {
        let columns = [
            (ColumnKind::Name, 0.0),
            (ColumnKind::Size, 90.0),
            (ColumnKind::Mtime, 110.0),
        ];
        let layout = column_layout(800.0, 0.0, &columns);
        assert_eq!(layout.name_right, 594.0);
        assert_eq!(
            layout.fixed,
            vec![
                (ColumnKind::Size, 594.0, 684.0),
                (ColumnKind::Mtime, 684.0, 794.0)
            ]
        );
        let layout = column_layout(800.0, -30.0, &columns);
        assert_eq!(layout.name_right, 564.0);
        assert_eq!(
            layout.fixed,
            vec![
                (ColumnKind::Size, 564.0, 654.0),
                (ColumnKind::Mtime, 654.0, 764.0)
            ]
        );
        // 多列：从右往左排，最右列锚定行右缘内 COL_RIGHT_PAD。
        let columns = [
            (ColumnKind::Name, 0.0),
            (ColumnKind::Ext, 70.0),
            (ColumnKind::Size, 90.0),
            (ColumnKind::Mtime, 110.0),
        ];
        let layout = column_layout(800.0, 0.0, &columns);
        assert_eq!(layout.name_right, 524.0);
        assert_eq!(
            layout.fixed,
            vec![
                (ColumnKind::Ext, 524.0, 594.0),
                (ColumnKind::Size, 594.0, 684.0),
                (ColumnKind::Mtime, 684.0, 794.0),
            ]
        );
    }

    /// 简表几何（阶段 V）：列宽 = 最长名称 char-unit × 7 + 28，
    /// clamp 120..=300；列数 = 栏宽/列宽 ≥1；名称按 char-unit 预算截断。
    #[test]
    fn brief_geometry_and_name_truncation() {
        assert_eq!(brief_col_width(0), 120.0);
        assert_eq!(brief_col_width(13), 120.0, "13*7+28=119 → 下限 120");
        assert_eq!(brief_col_width(20), 168.0);
        assert_eq!(brief_col_width(100), 300.0, "上限 300");
        assert_eq!(brief_cols(500.0, 120.0), 4);
        assert_eq!(brief_cols(100.0, 120.0), 1);
        // 截断：cell_w=120 → 预算 (120-30)/7 = 12 units。
        assert_eq!(brief_cell_name("abcdefghij", 120.0), "abcdefghij");
        assert_eq!(brief_cell_name("abcdefghijklmnop", 120.0), "abcdefghijk…");
        // 中文 = 2 units：6 字 = 12 恰好放下；7 字截断（超预算时退一格）。
        assert_eq!(brief_cell_name("简表测试名称", 120.0), "简表测试名称");
        assert_eq!(brief_cell_name("简表测试名称啊", 120.0), "简表测试名…");
        assert_eq!(name_units("ab简"), 4);
    }

    /// Brief 模式 + 自定义列的无头渲染冒烟（阶段 V）：简表渲染若干帧不
    /// panic、last_grid_cols 落位、↓ 按列数步进；列勾选加 Ext/Attr/
    /// Comment 后列表模式（列头 + 行直绘）渲染不 panic，快照携带列配置。
    #[test]
    fn brief_mode_and_custom_columns_render_headless() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(tmp.path().join(format!("file-{i:02}.txt")), b"x").unwrap();
        }
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        // 简表渲染：多列排布，列数写回 last_grid_cols。
        view.panels[0].view_mode = PanelViewMode::Brief;
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        let cols = view.panels[0].last_grid_cols;
        assert!(cols > 1, "简表应多列排布，got {cols}");
        // ↓ 按列数步进（网格状模式键盘语义）。
        view.panels[0].focus = Some(0);
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::ArrowDown));
        assert_eq!(view.panels[0].focus, Some(cols.min(20)));
        // 自定义列：加 Ext/Attr/Comment，列表模式渲染（列头 + 行直绘）。
        view.panels[0].view_mode = PanelViewMode::List;
        for k in [ColumnKind::Ext, ColumnKind::Attr, ColumnKind::Comment] {
            view.panels[0].toggle_column(k);
        }
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert_eq!(view.panels[0].columns.len(), 6);
        assert_eq!(view.snapshot().columns.len(), 6);
        // 删列后再渲染。
        view.panels[0].toggle_column(ColumnKind::Comment);
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        assert_eq!(view.panels[0].columns.len(), 5);
    }

    /// 注释（阶段 W）无头冒烟：列举就绪后 descript.ion 进缓存；Comment
    /// 列渲染不 panic；编辑对话框打开渲染不 panic。
    #[test]
    fn comments_render_headless() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        std::fs::write(tmp.path().join("b.txt"), b"x").unwrap();
        std::fs::write(
            tmp.path().join("descript.ion"),
            "a.txt 这是注释\n".as_bytes(),
        )
        .unwrap();
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        assert_eq!(
            view.panels[0].comments.get("a.txt").map(String::as_str),
            Some("这是注释")
        );
        // Comment 列渲染。
        view.panels[0].toggle_column(ColumnKind::Comment);
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        // 编辑对话框打开渲染。
        view.comment_dialog = Some(CommentDialog::new(
            0,
            "a.txt".to_string(),
            view.panels[0].comments.get("a.txt").map(String::as_str),
        ));
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert!(view.comment_dialog.is_some(), "未确认/取消时对话框应保持");
    }

    /// 命令行输入条（阶段 X）无头冒烟：Ctrl+P 送当前路径、Ctrl+Enter 送
    /// 焦点项文件名、Enter 执行清空入历史、↑/↓ 历史回填、Esc 清空交还焦点。
    #[test]
    fn command_bar_headless() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let mut t = 0.0;
        headless_frame(&ctx, &mut view, t, vec![]);

        // Ctrl+P：焦点栏当前路径追加到输入末尾（下一帧命令行获得焦点）。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events_mod(egui::Key::P, ctrl));
        let dir_str = tmp.path().display().to_string();
        assert_eq!(view.command_input, dir_str);
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![]);

        // 焦点落到文件行后 Ctrl+Enter：追加文件名。
        view.panels[0].focus = Some(1);
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events_mod(egui::Key::Enter, ctrl));
        assert_eq!(view.command_input, format!("{dir_str}a.txt"));

        // Enter 执行（无害命令）：清空输入 + 入历史，执行后仍留在命令行。
        view.command_input = if cfg!(windows) {
            "exit 0".to_string()
        } else {
            "true".to_string()
        };
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::Enter));
        assert!(view.command_input.is_empty());
        assert_eq!(view.command_history.len(), 1);
        assert_eq!(view.command_history_pos, None);

        // ↑ 回填最新历史；↓ 越过最新回空白。
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::ArrowUp));
        assert_eq!(view.command_input, view.command_history[0]);
        assert_eq!(view.command_history_pos, Some(0));
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::ArrowDown));
        assert!(view.command_input.is_empty());
        assert_eq!(view.command_history_pos, None);

        // Esc：清空并交还焦点。command_esc_handled 是帧内标记（当帧即被
        // handle_keyboard 的 Esc 链消费清零），故改验语义结果：焦点已交还、
        // 且 Esc 链未双消费（预置的选中集不被清空）。
        let sel = tmp.path().join("a.txt");
        view.panels[0].selected.insert(sel.clone());
        view.command_input = "abc".to_string();
        t += 1.0;
        headless_frame(&ctx, &mut view, t, key_events(egui::Key::Escape));
        assert!(view.command_input.is_empty());
        assert!(ctx.memory(|m| m.focused()).is_none(), "Esc 应交还焦点");
        assert!(
            view.panels[0].selected.contains(&sel),
            "Esc 被命令行消费后不应再走 Esc 链清空选中集"
        );
    }

    /// spawn_shell_command：无害命令可后台启动（不等待退出）。
    #[test]
    fn spawn_shell_command_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let input = if cfg!(windows) { "exit 0" } else { "true" };
        spawn_shell_command(tmp.path(), input).unwrap();
    }

    /// 按钮栏占位符展开（阶段 Y）：%P/%N/%p 各自替换，无焦点时 %N 为空，
    /// 未知占位符原样保留。
    #[test]
    fn expand_button_command_replaces_placeholders() {
        assert_eq!(
            expand_button_command("echo %P %N %p", "C:\\dir", Some("a.txt"), "D:\\other"),
            "echo C:\\dir a.txt D:\\other"
        );
        assert_eq!(
            expand_button_command("echo %N", "C:\\dir", None, "D:\\other"),
            "echo "
        );
        // 未知占位符/字面 % 原样保留；大小写敏感（%P 与 %p 不同）。
        assert_eq!(
            expand_button_command("echo 100% %X %P%p", "A", None, "B"),
            "echo 100% %X AB"
        );
    }

    /// 按钮栏快照往返（阶段 Y）：set_button_bar 注入 → snapshot 取回。
    #[test]
    fn button_bar_snapshot_roundtrip() {
        let buttons = vec![
            FmButton {
                label: "终端".to_string(),
                command: "cmd".to_string(),
                tooltip: String::new(),
            },
            FmButton {
                label: "记事本".to_string(),
                command: "notepad %N".to_string(),
                tooltip: "打开焦点文件".to_string(),
            },
        ];
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        assert!(view.snapshot().button_bar.is_empty());
        view.set_button_bar(&buttons);
        assert_eq!(view.snapshot().button_bar, buttons);
    }

    /// 按钮栏无头冒烟（阶段 Y）：按钮渲染 + 点击执行无害命令 +
    /// 添加对话框渲染不 panic。
    #[test]
    fn button_bar_render_headless() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let mut view = FileManagerView::new("single", 0.5, false, "name", true, &[]);
        navigate_ready(&mut view.panels[0], tmp.path());
        view.set_button_bar(&[FmButton {
            label: "无害".to_string(),
            command: if cfg!(windows) {
                "exit 0".to_string()
            } else {
                "true".to_string()
            },
            tooltip: String::new(),
        }]);
        let ctx = egui::Context::default();
        setup_test_fonts(&ctx);
        let mut t = 0.0;
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        // 自上而下扫描单击，命中按钮行即执行（历史不进命令行——按钮栏
        // 与命令行历史无关；这里验证点击链路不 panic、视图状态健在）。
        let mut y = 30.0;
        while y < 120.0 {
            t += 1.0;
            headless_frame(
                &ctx,
                &mut view,
                t,
                primary_click_events(egui::pos2(400.0, y)),
            );
            y += 4.0;
        }
        assert_eq!(view.button_bar.len(), 1, "扫描点击不应改动按钮列表");
        // 添加对话框渲染。
        view.button_dialog = Some(ButtonDialog::new_create());
        for _ in 0..3 {
            t += 1.0;
            headless_frame(&ctx, &mut view, t, vec![]);
        }
        assert!(view.button_dialog.is_some(), "未确认/取消时对话框应保持");
        // 编辑对话框预填渲染。
        view.button_dialog = Some(ButtonDialog::new_edit(0, &view.button_bar[0]));
        t += 1.0;
        headless_frame(&ctx, &mut view, t, vec![]);
        assert!(view.button_dialog.is_some());
    }
}
