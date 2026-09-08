//! 压缩包浏览视图：资源管理器式三栏布局（左栏目录树 / 中栏面包屑+明细
//! 列表 / 右栏预览面板），操作手感对齐 WinRAR/资源管理器：单击选中、
//! Ctrl/Shift 多选、列头排序、Backspace/Enter/方向键键盘导航、右键菜单。
//! 后台列出全部条目（含目录与非图片）；状态机
//! Idle → Listing → Ready / NeedPassword / Failed。

use crate::app::{PASSWORD_INCORRECT_MARKER, PASSWORD_REQUIRED_MARKER};
use crate::opener::{AsyncOpener, OpenStatus};
use crate::views::archive_tree::{
    breadcrumb_paths, build_dir_rows, list_rows, ListRow, SortKey, TreeRow,
};
use egui_phosphor_icons::{icons, Icon};
use openitgo_parser::archive::{list_entries, read_comment, read_entry, ArchiveEntry};
use openitgo_parser::traits::ParseError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 预览读取上限：超过该大小的条目不读取（防爆内存）。
const PREVIEW_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 文本嗅探只取条目内容的前 256KB。
const TEXT_SNIFF_BYTES: usize = 256 * 1024;
/// 文本预览展示的字符数上限。
const TEXT_PREVIEW_MAX_CHARS: usize = 64_000;
/// 明细列表行高（pt），虚拟化滚动要求固定行高。
const ROW_HEIGHT: f32 = 22.0;
/// 明细列表「大小」「压缩后」「时间」列的默认宽度（pt），右对齐。
const SIZE_COL_WIDTH: f32 = 90.0;
const PACKED_COL_WIDTH: f32 = 90.0;
const MTIME_COL_WIDTH: f32 = 110.0;
/// 列宽拖拽的取值范围（pt）。
const COL_MIN_WIDTH: f32 = 60.0;
const COL_MAX_WIDTH: f32 = 400.0;
/// 列内容右缘内边距（pt）：列锚点在右缘内 6pt 处，表头与行内容共用。
const COL_RIGHT_PAD: f32 = 6.0;
/// 表头「名称」文字的左缩进（pt）：与行内容对齐
/// （行 = 6pt shrink + 约 16pt 图标 + 6pt 间距）。
const NAME_HEADER_INDENT: f32 = 6.0 + 16.0 + 6.0;

/// 名称/大小/压缩后/时间四列的 x 坐标单一来源（表头 paint、行列分隔竖线、
/// 列宽拖拽共用），消除各自手算的漂移。名称列宽 = 剩余弹性。
#[derive(Debug, Clone, Copy)]
struct ColumnLayout {
    /// 名称列右缘（= 名称|大小分隔竖线 x、大小列左缘）。
    size_left: f32,
    /// 大小列右缘（= 大小|压缩后分隔竖线 x、压缩后列左缘），大小文字右锚点。
    packed_left: f32,
    /// 压缩后列右缘（= 压缩后|时间分隔竖线 x、时间列左缘），压缩后文字右锚点。
    mtime_left: f32,
    /// 时间文字右锚点（行右缘内 COL_RIGHT_PAD 处）。
    content_right: f32,
}

/// 由行/表头 rect 的右缘与三列宽度算出各列坐标。
fn column_layout(right: f32, size_w: f32, packed_w: f32, mtime_w: f32) -> ColumnLayout {
    let content_right = right - COL_RIGHT_PAD;
    let mtime_left = content_right - mtime_w;
    let packed_left = mtime_left - packed_w;
    let size_left = packed_left - size_w;
    ColumnLayout {
        size_left,
        packed_left,
        mtime_left,
        content_right,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArchiveViewState {
    #[default]
    Idle,
    Listing,
    Ready,
    /// 列表遇到 PasswordRequired/Incorrect；点击「输入密码」走密码对话框。
    NeedPassword,
    Failed(String),
}

/// 明细列表行的身份（选择锚点/焦点用）：目录用归一化 full_path，
/// 文件用原始 entry.name。
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowKey {
    Dir(String),
    File(String),
}

/// 拖出解压的触发阈值（pt）：指针距起点超过该距离或已出窗才开始后台解压，
/// 避免 egui 6pt 拖动阈值下手滑即触发整批解压 IO。
const DRAG_OUT_THRESHOLD: f32 = 40.0;

/// Potential → Extracting 的触发判定：指针出窗（pos None 或越出视口）
/// 或距按下起点超过 DRAG_OUT_THRESHOLD。
fn should_begin_extract(origin: egui::Pos2, pos: Option<egui::Pos2>, viewport: egui::Rect) -> bool {
    match pos {
        None => true,
        Some(p) => !viewport.contains(p) || p.distance(origin) > DRAG_OUT_THRESHOLD,
    }
}

/// 拖动幽灵的文案。
fn ghost_text(supported: bool, preparing: bool, count: usize) -> String {
    if !supported {
        "当前平台不支持拖出".to_string()
    } else if preparing {
        format!("⇪ {count} 个文件 · 正在准备拖出…")
    } else {
        format!("⇪ {count} 个文件 · 拖到窗口外解压")
    }
}

/// 拖动幽灵：指针旁的带底色小卡片（Order::Foreground，不拦截交互）。
/// 指针出窗（latest_pos None）时不画。
fn paint_drag_ghost(ctx: &egui::Context, text: &str) {
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    egui::Area::new(egui::Id::new("archive-drag-ghost"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(pos + egui::vec2(12.0, 16.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(text);
            });
        });
}

/// 拖出解压状态机：Potential（按下刚拖动，等阈值/出窗才解压）→
/// Extracting（后台解压进本次专属暂存目录）→ Ready（出窗即 OLE DoDragDrop）。
/// 任何时刻主键松开且还没进 DoDragDrop = 取消。
enum DragOutState {
    Potential {
        names: Vec<String>,
        origin: egui::Pos2,
    },
    Extracting {
        staging: PathBuf,
        names: Vec<String>,
        task: AsyncOpener<()>,
    },
    Ready {
        staging: PathBuf,
        files: Vec<PathBuf>,
    },
}

impl DragOutState {
    /// 本次拖出的暂存目录（取消时 best-effort 清理用）。
    fn staging(&self) -> Option<&Path> {
        match self {
            DragOutState::Extracting { staging, .. } | DragOutState::Ready { staging, .. } => {
                Some(staging)
            }
            DragOutState::Potential { .. } => None,
        }
    }
}

/// 后台线程产出的预览内容。
#[derive(Debug, Clone)]
enum PreviewData {
    Image(egui::ColorImage),
    Text(String),
    Unsupported,
    /// 附带说明（过大/解码失败等）。
    Note(String),
}

pub struct ArchiveView {
    pub path: Option<PathBuf>,
    pub entries: Vec<ArchiveEntry>,
    /// 选中的文件条目名（目录行选中态派生自其后代文件）。
    pub selected: HashSet<String>,
    pub filter: String,
    pub state: ArchiveViewState,
    /// open_with_password 使用的密码；列表成功后由 app 取走记入密码本。
    pub tried_password: Option<String>,
    /// 带密码列目录仍遇密码错误：app 据此以 incorrect 复现密码对话框。
    pub password_failed: bool,
    /// ZIP 注释（其他格式恒 None）；列目录时顺带读出。
    pub comment: Option<String>,
    listing: Option<AsyncOpener<(Vec<ArchiveEntry>, Option<String>)>>,
    /// 目录树中折叠的目录 full_path（`/` 分隔归一化路径）。
    pub collapsed: HashSet<String>,
    /// 中栏当前目录（`/` 分隔归一化路径）；None = 根目录。
    pub current_dir: Option<String>,
    /// 「全部文件」扁平模式：忽略 current_dir 展示全包文件。
    pub flat_all: bool,
    /// Shift 范围选锚点。
    anchor: Option<RowKey>,
    /// 键盘焦点行（方向键移动、Enter 打开）。
    focus: Option<RowKey>,
    /// 键盘移动焦点后置位，下一帧按 Explorer 最小滚动语义揭示焦点行。
    focus_scroll_pending: bool,
    /// 树跟随当前目录：enter_dir/go_up/show_all_files 置位，下一帧渲染目录树时
    /// 自动展开 current_dir 的全部祖先并滚动揭示该行（「全部文件」模式揭示根行）。
    tree_reveal_pending: bool,
    /// 上一帧明细列表的滚动偏移与视口高度（最小滚动计算的基准；0 = 未知）。
    last_scroll_offset: f32,
    last_viewport_height: f32,
    /// 明细列表排序（默认名称升序）。
    sort_key: SortKey,
    sort_asc: bool,
    /// 大小/压缩后/时间列宽（pt，分隔竖线拖拽可调，clamp 60..=400）；
    /// 跨包保留（clear_entries_state 不重置），会话内有效，不落盘。
    col_width_size: f32,
    col_width_packed: f32,
    col_width_mtime: f32,
    /// 右侧预览面板开关。
    pub preview_open: bool,
    /// 当前预览目标条目名（单击文件条目设置）。
    preview_entry: Option<String>,
    /// 已为该条目发起过后台读取（避免每帧重复 spawn）。
    preview_requested: Option<String>,
    preview: Option<AsyncOpener<PreviewData>>,
    /// poll 拿到、待 ui() 上传为纹理的图片。
    pending_preview_image: Option<egui::ColorImage>,
    preview_tex: Option<egui::TextureHandle>,
    preview_text: Option<String>,
    preview_note: Option<String>,
    /// 会话密码（app 每帧从会话密码表写入），预览读取加密条目用。
    pub preview_password: Option<String>,
    /// 拖出状态机（Potential → Extracting → Ready，见 DragOutState）。
    drag_out: Option<DragOutState>,
    /// 拖出暂存目录递增计数（temp_root("drag")/<计数>/，同包多次拖出不互相覆盖）。
    drag_out_counter: u64,
    /// 拖出错误（app render_archive 取走写入 error_message）。
    pub drag_error: Option<String>,
}

impl Default for ArchiveView {
    fn default() -> Self {
        Self {
            path: None,
            entries: Vec::new(),
            selected: HashSet::new(),
            filter: String::new(),
            state: ArchiveViewState::Idle,
            tried_password: None,
            password_failed: false,
            comment: None,
            listing: None,
            collapsed: HashSet::new(),
            current_dir: None,
            flat_all: false,
            anchor: None,
            focus: None,
            focus_scroll_pending: false,
            tree_reveal_pending: false,
            last_scroll_offset: 0.0,
            last_viewport_height: 0.0,
            sort_key: SortKey::Name,
            sort_asc: true,
            col_width_size: SIZE_COL_WIDTH,
            col_width_packed: PACKED_COL_WIDTH,
            col_width_mtime: MTIME_COL_WIDTH,
            preview_open: false,
            preview_entry: None,
            preview_requested: None,
            preview: None,
            pending_preview_image: None,
            preview_tex: None,
            preview_text: None,
            preview_note: None,
            preview_password: None,
            drag_out: None,
            drag_out_counter: 0,
            drag_error: None,
        }
    }
}

pub struct ArchiveCallbacks<'a> {
    pub on_back: &'a mut dyn FnMut(),
    /// 「作为漫画打开」：仅当包本身是支持的漫画格式（zip/cbz/rar/cbr）时展示。
    pub on_open_as_comic: &'a mut dyn FnMut(),
    /// 双击图片条目（漫画格式包）：以漫画打开并定位到该页。
    pub on_open_entry_as_comic: &'a mut dyn FnMut(String),
    /// 双击其他条目（或非漫画格式包的图片）：临时解压后用系统程序打开。
    pub on_open_entry_external: &'a mut dyn FnMut(String),
    pub on_extract_all: &'a mut dyn FnMut(),
    pub on_extract_selected: &'a mut dyn FnMut(Vec<String>),
    /// NeedPassword 状态下点击「输入密码」。
    pub on_need_password: &'a mut dyn FnMut(),
}

impl ArchiveView {
    /// 清空条目相关状态（选择/过滤/折叠/当前目录/预览/排序），供 open* 系列复用。
    fn clear_entries_state(&mut self) {
        self.entries.clear();
        self.selected.clear();
        self.filter.clear();
        self.collapsed.clear();
        self.current_dir = None;
        self.flat_all = false;
        self.anchor = None;
        self.focus = None;
        self.focus_scroll_pending = false;
        self.tree_reveal_pending = false;
        self.last_scroll_offset = 0.0;
        self.last_viewport_height = 0.0;
        self.sort_key = SortKey::Name;
        self.sort_asc = true;
        self.comment = None;
        self.preview_entry = None;
        self.preview_requested = None;
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        self.cancel_drag_out();
        self.drag_error = None;
    }

    /// 后台列出条目（不带密码；加密包落入 NeedPassword 状态）。
    pub fn open(&mut self, path: PathBuf) {
        self.path = Some(path.clone());
        self.clear_entries_state();
        self.state = ArchiveViewState::Listing;
        self.tried_password = None;
        self.password_failed = false;
        self.listing = Some(AsyncOpener::open(path, |p| {
            // 列目录顺带读 ZIP 注释（其他格式 read_comment 恒 None）。
            list_entries(p, None)
                .map(|entries| (entries, read_comment(p)))
                .map_err(|e| match e {
                    ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                    ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                    other => other.to_string(),
                })
        }));
    }

    /// 带密码重跑列目录（密码对话框确认后调用）。
    pub fn open_with_password(&mut self, path: PathBuf, password: String) {
        self.path = Some(path.clone());
        self.clear_entries_state();
        self.state = ArchiveViewState::Listing;
        self.password_failed = false;
        self.tried_password = Some(password.clone());
        self.listing = Some(AsyncOpener::open(path, move |p| {
            list_entries(p, Some(&password))
                .map(|entries| (entries, read_comment(p)))
                .map_err(|e| match e {
                    ParseError::PasswordRequired => PASSWORD_REQUIRED_MARKER.to_string(),
                    ParseError::PasswordIncorrect => PASSWORD_INCORRECT_MARKER.to_string(),
                    other => other.to_string(),
                })
        }));
    }

    /// 直接以已列好的条目进入 Ready（启发式分流复用列目录结果，不重复列出）。
    pub fn open_with_entries(&mut self, path: PathBuf, entries: Vec<ArchiveEntry>) {
        self.path = Some(path);
        self.clear_entries_state();
        self.entries = entries;
        self.state = ArchiveViewState::Ready;
        self.tried_password = None;
        self.password_failed = false;
        self.listing = None;
    }

    /// 每帧排空列表与预览结果；视图不处于前台时结果留在通道里，
    /// 下次 open 直接替换。
    pub fn poll(&mut self) {
        if let Some(mut listing) = self.listing.take() {
            match listing.poll() {
                OpenStatus::Loading => self.listing = Some(listing),
                OpenStatus::Ready(result) => self.apply_listing_result(result),
            }
        }
        self.poll_preview();
        self.poll_drag_out();
    }

    /// 拖出的条目集：拖动项在多选集合中 → 整个选中集（包内顺序），否则单条目。
    pub(crate) fn drag_entry_set(selected_in_order: &[String], dragged: &str) -> Vec<String> {
        if selected_in_order.len() > 1 && selected_in_order.iter().any(|n| n == dragged) {
            selected_in_order.to_vec()
        } else {
            vec![dragged.to_string()]
        }
    }

    /// 拖动起始（render_list_row 的 drag_started）：进入 Potential，
    /// 只锁定条目集与按下起点，不做任何 IO（越过阈值/出窗才开始解压）。
    fn begin_drag_potential(&mut self, names: Vec<String>, origin: egui::Pos2) {
        if names.is_empty() || self.drag_out.is_some() {
            return;
        }
        self.drag_out = Some(DragOutState::Potential { names, origin });
    }

    /// Potential 越过阈值/出窗：后台把条目集按包内相对路径解压进
    /// 本次专属暂存目录 temp_root("drag")/<计数>/。
    fn begin_drag_extract(&mut self, names: Vec<String>) {
        let Some(archive) = self.path.clone() else {
            self.drag_out = None;
            return;
        };
        self.drag_out_counter += 1;
        let staging =
            crate::temp_open::temp_root("drag", &archive).join(self.drag_out_counter.to_string());
        let password = self.preview_password.clone();
        let staging_in_task = staging.clone();
        let names_in_task = names.clone();
        let task = AsyncOpener::open(archive.clone(), move |_p| {
            for n in &names_in_task {
                crate::temp_open::extract_entry_to_staging(
                    &staging_in_task,
                    &archive,
                    n,
                    password.as_deref(),
                )
                .map_err(|e| format!("{n}: {e}"))?;
            }
            Ok(())
        });
        self.drag_out = Some(DragOutState::Extracting {
            staging,
            names,
            task,
        });
    }

    /// 取消本次拖出：清空状态；暂存目录已建则 best-effort 删除（不再白留 24h）。
    fn cancel_drag_out(&mut self) {
        if let Some(state) = self.drag_out.take() {
            if let Some(dir) = state.staging() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    /// 排空拖出后台解压：成功 → 顶层负载进 Ready，失败 → drag_error + 清理暂存目录。
    fn poll_drag_out(&mut self) {
        let taken = match self.drag_out.take() {
            Some(s @ DragOutState::Extracting { .. }) => s,
            other => {
                self.drag_out = other;
                return;
            }
        };
        let DragOutState::Extracting {
            staging,
            names,
            mut task,
        } = taken
        else {
            unreachable!()
        };
        match task.poll() {
            OpenStatus::Loading => {
                self.drag_out = Some(DragOutState::Extracting {
                    staging,
                    names,
                    task,
                });
            }
            OpenStatus::Ready(Ok(())) => {
                // HDROP 负载 = 暂存目录顶层项：拖单个目录时负载即该文件夹本身。
                let files = crate::temp_open::staging_payload(&staging);
                if files.is_empty() {
                    self.drag_error = Some("拖出解压失败: 暂存目录为空".to_string());
                    let _ = std::fs::remove_dir_all(&staging);
                } else {
                    self.drag_out = Some(DragOutState::Ready { staging, files });
                }
            }
            OpenStatus::Ready(Err(e)) => {
                self.drag_error = Some(format!("拖出解压失败: {e}"));
                let _ = std::fs::remove_dir_all(&staging);
            }
        }
    }

    /// 拖出主流程（每帧）：Potential 画幽灵并等阈值/出窗 → Extracting 等后台
    /// 解压 → Ready 出窗即 OLE DoDragDrop（模态阻塞，自带消息循环）。
    /// 主键松开且还没进 DoDragDrop = 取消并清理暂存目录。
    fn update_drag_out(&mut self, ctx: &egui::Context) {
        if self.drag_out.is_none() {
            return;
        }
        let supported = crate::platform::drag_out::is_supported();
        let (primary_down, pos, viewport) = ctx.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.latest_pos(),
                i.viewport_rect(),
            )
        });
        if !primary_down {
            self.cancel_drag_out();
            return;
        }
        let browsing_ready = self.state == ArchiveViewState::Ready;
        match self.drag_out.take() {
            Some(DragOutState::Potential { names, origin }) => {
                if supported && should_begin_extract(origin, pos, viewport) {
                    if browsing_ready {
                        paint_drag_ghost(ctx, &ghost_text(true, true, names.len()));
                    }
                    self.begin_drag_extract(names);
                } else {
                    if browsing_ready {
                        paint_drag_ghost(ctx, &ghost_text(supported, false, names.len()));
                    }
                    self.drag_out = Some(DragOutState::Potential { names, origin });
                }
            }
            Some(DragOutState::Extracting {
                staging,
                names,
                task,
            }) => {
                if browsing_ready {
                    paint_drag_ghost(ctx, &ghost_text(supported, true, names.len()));
                }
                self.drag_out = Some(DragOutState::Extracting {
                    staging,
                    names,
                    task,
                });
                // 持续重绘以便空闲时 poll_drag_out 排空解压结果。
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Some(DragOutState::Ready { staging, files }) => {
                let left_window = pos.is_none_or(|p| !viewport.contains(p));
                if left_window {
                    // 模态阻塞；DROP/CANCEL 返回后本次拖出都结束。暂存目录不删
                    // （落点可能还在读，交给 24h clean_stale 兜底）。
                    if let Err(e) = crate::platform::drag_out::do_drag_drop(&files) {
                        self.drag_error = Some(e);
                    }
                } else {
                    self.drag_out = Some(DragOutState::Ready { staging, files });
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
            None => {}
        }
    }

    fn apply_listing_result(
        &mut self,
        result: Result<(Vec<ArchiveEntry>, Option<String>), String>,
    ) {
        match result {
            Ok((entries, comment)) => {
                self.entries = entries;
                self.comment = comment;
                self.state = ArchiveViewState::Ready;
                self.password_failed = false;
            }
            Err(e) if e == PASSWORD_REQUIRED_MARKER || e == PASSWORD_INCORRECT_MARKER => {
                // 带密码尝试后仍失败 = 密码错误，对话框以 incorrect 复现。
                self.password_failed = self.tried_password.is_some();
                self.state = ArchiveViewState::NeedPassword;
            }
            Err(e) => self.state = ArchiveViewState::Failed(e),
        }
    }

    /// 预览目标变化时发起后台读取，并排空预览结果。
    fn poll_preview(&mut self) {
        if self.preview_open && self.preview_entry != self.preview_requested {
            self.start_preview();
        }
        let Some(mut preview) = self.preview.take() else {
            return;
        };
        match preview.poll() {
            OpenStatus::Loading => self.preview = Some(preview),
            OpenStatus::Ready(result) => self.apply_preview_result(result),
        }
    }

    /// 为 preview_entry 发起后台读取；目标变化时丢弃旧纹理与旧结果。
    fn start_preview(&mut self) {
        self.preview_requested = self.preview_entry.clone();
        self.preview = None;
        self.pending_preview_image = None;
        self.preview_tex = None;
        self.preview_text = None;
        self.preview_note = None;
        let (Some(path), Some(name)) = (self.path.clone(), self.preview_entry.clone()) else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| !e.is_dir && e.name == name) else {
            return;
        };
        let size = entry.size;
        let password = self.preview_password.clone();
        self.preview = Some(AsyncOpener::open(path, move |p| {
            load_preview(p, &name, size, password.as_deref())
        }));
    }

    fn apply_preview_result(&mut self, result: Result<PreviewData, String>) {
        match result {
            Ok(PreviewData::Image(img)) => self.pending_preview_image = Some(img),
            Ok(PreviewData::Text(text)) => self.preview_text = Some(text),
            Ok(PreviewData::Unsupported) => {
                self.preview_note = Some("不支持预览该类型".to_string());
            }
            Ok(PreviewData::Note(note)) => self.preview_note = Some(note),
            Err(e) => self.preview_note = Some(e),
        }
    }

    /// 目录的全部后代文件条目名（原始名；`/` 与 `\\` 都算边界，
    /// 先剥掉目录路径尾部斜杠再加边界匹配，`a` 不会误中 `ab/`）。
    fn descendant_file_names(&self, dir_full_path: &str) -> Vec<String> {
        let dir = dir_full_path.trim_end_matches(['/', '\\']);
        let prefix_slash = format!("{dir}/");
        let prefix_backslash = format!("{dir}\\");
        self.entries
            .iter()
            .filter(|e| {
                !e.is_dir
                    && (e.name.starts_with(&prefix_slash) || e.name.starts_with(&prefix_backslash))
            })
            .map(|e| e.name.clone())
            .collect()
    }

    /// 目录勾选态：有后代文件且全部被选中。
    fn dir_all_selected(&self, dir_full_path: &str) -> bool {
        let names = self.descendant_file_names(dir_full_path);
        !names.is_empty() && names.iter().all(|n| self.selected.contains(n))
    }

    /// 目录勾选级联：勾选/取消勾选其全部后代文件条目。
    fn cascade_set(&mut self, dir_full_path: &str, checked: bool) {
        for name in self.descendant_file_names(dir_full_path) {
            if checked {
                self.selected.insert(name);
            } else {
                self.selected.remove(&name);
            }
        }
    }

    /// 中栏当前行模型（目录优先 + 排序 + 过滤，见 archive_tree::list_rows）。
    fn rows(&self) -> Vec<ListRow> {
        list_rows(
            &self.entries,
            self.current_dir.as_deref(),
            self.flat_all,
            &self.filter,
            self.sort_key,
            self.sort_asc,
        )
    }

    /// 行的身份键（选择锚点/焦点/高亮判定用）。
    fn row_key(&self, row: &ListRow) -> RowKey {
        match row {
            ListRow::Dir { full_path, .. } => RowKey::Dir(full_path.clone()),
            ListRow::File { idx } => RowKey::File(self.entries[*idx].name.clone()),
        }
    }

    /// 行的选中态：文件查 selected，目录看后代文件是否全选。
    fn row_selected(&self, key: &RowKey) -> bool {
        match key {
            RowKey::Dir(dir) => self.dir_all_selected(dir),
            RowKey::File(name) => self.selected.contains(name),
        }
    }

    /// 选中/取消一行：目录行级联到全部后代文件。
    fn set_row_selected(&mut self, key: &RowKey, on: bool) {
        match key {
            RowKey::Dir(dir) => self.cascade_set(dir, on),
            RowKey::File(name) => {
                if on {
                    self.selected.insert(name.clone());
                } else {
                    self.selected.remove(name);
                }
            }
        }
    }

    /// Explorer 式点击选择：无修饰 = 单选；Ctrl = 切换；Shift = 以 anchor
    /// 到目标的行序区间替换式选中（Ctrl+Shift 追加；无锚点退化为普通点击）。
    /// 任何点击都更新 anchor 与 focus。
    fn click_row(&mut self, key: RowKey, ctrl: bool, shift: bool) {
        if shift {
            if let Some(anchor) = self.anchor.clone() {
                let keys: Vec<RowKey> = self.rows().iter().map(|r| self.row_key(r)).collect();
                if let (Some(a), Some(t)) = (
                    keys.iter().position(|k| *k == anchor),
                    keys.iter().position(|k| *k == key),
                ) {
                    if !ctrl {
                        self.selected.clear();
                    }
                    let (lo, hi) = if a <= t { (a, t) } else { (t, a) };
                    for k in &keys[lo..=hi] {
                        self.set_row_selected(k, true);
                    }
                    self.focus = Some(key);
                    return;
                }
            }
            // 无锚点或锚点已不可见：退化为普通点击。
        }
        if ctrl {
            let on = !self.row_selected(&key);
            self.set_row_selected(&key, on);
        } else {
            self.selected.clear();
            self.set_row_selected(&key, true);
        }
        self.anchor = Some(key.clone());
        self.focus = Some(key);
    }

    /// 全选当前可见行（目录行级联进文件）。
    fn select_all_visible(&mut self) {
        let keys: Vec<RowKey> = self.rows().iter().map(|r| self.row_key(r)).collect();
        for k in keys {
            self.set_row_selected(&k, true);
        }
    }

    /// 清空选中与锚点/焦点（导航切换目录、Esc 时用）。
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.focus = None;
    }

    /// 进入目录（树/面包屑/双击目录行/右键「进入」共用）。
    fn enter_dir(&mut self, full_path: String) {
        self.flat_all = false;
        self.current_dir = Some(full_path);
        self.clear_selection();
        self.tree_reveal_pending = true;
    }

    /// 切到「全部文件」扁平模式（左栏特殊根节点/面包屑）。
    fn show_all_files(&mut self) {
        self.flat_all = true;
        self.current_dir = None;
        self.clear_selection();
        self.tree_reveal_pending = true;
    }

    /// Backspace 上级：扁平模式 → 根目录；目录 → 截掉末段；根目录 no-op。
    fn go_up(&mut self) {
        if self.flat_all {
            self.flat_all = false;
            self.current_dir = None;
            self.clear_selection();
            self.tree_reveal_pending = true;
            return;
        }
        let Some(dir) = self.current_dir.clone() else {
            return;
        };
        let dir = dir.trim_end_matches(['/', '\\']);
        self.current_dir = dir.rfind(['/', '\\']).map(|i| dir[..i].to_string());
        self.clear_selection();
        self.tree_reveal_pending = true;
    }

    /// ↑/↓ 移动焦点：无焦点时选中首行/末行；有焦点按行序步进并单选。
    fn move_focus(&mut self, delta: isize) {
        let keys: Vec<RowKey> = self.rows().iter().map(|r| self.row_key(r)).collect();
        if keys.is_empty() {
            return;
        }
        let cur = self
            .focus
            .as_ref()
            .and_then(|f| keys.iter().position(|k| k == f));
        let next = match cur {
            None if delta >= 0 => 0,
            None => keys.len() - 1,
            Some(i) => (i as isize + delta).clamp(0, keys.len() as isize - 1) as usize,
        };
        let key = keys[next].clone();
        self.selected.clear();
        self.set_row_selected(&key, true);
        self.anchor = Some(key.clone());
        self.focus = Some(key.clone());
        // 预览跟随键盘：文件行同步预览目标（与鼠标单击一致），目录行不动。
        if let RowKey::File(name) = &key {
            self.preview_entry = Some(name.clone());
        }
        self.focus_scroll_pending = true;
    }

    /// 列头点击排序：同键切换升/降，换键回到升序。
    fn toggle_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
    }

    /// 当前列宽下的列坐标（表头、数据行、竖线共用同一来源）。
    fn layout(&self, right: f32) -> ColumnLayout {
        column_layout(
            right,
            self.col_width_size,
            self.col_width_packed,
            self.col_width_mtime,
        )
    }

    /// 选中文件条目数与总大小（解压后字节）。
    fn selected_stats(&self) -> (usize, u64) {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && self.selected.contains(&e.name))
            .fold((0, 0), |(n, bytes), e| (n + 1, bytes + e.size))
    }

    /// 选中的条目名，按包内顺序排列。
    fn selected_names_in_order(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, callbacks: ArchiveCallbacks<'_>) {
        let ArchiveCallbacks {
            on_back,
            on_open_as_comic,
            on_open_entry_as_comic,
            on_open_entry_external,
            on_extract_all,
            on_extract_selected,
            on_need_password,
        } = callbacks;

        // 拖出：每帧驱动状态机（幽灵/解压/出窗发起 OLE DoDragDrop）。
        self.update_drag_out(ui.ctx());

        let openable_comic = self
            .path
            .as_deref()
            .is_some_and(crate::app::is_supported_comic_file);

        // 顶栏（WinRAR 工具栏位）：导航 / 打开 / 解压 / 预览 + 右侧过滤框。
        ui.horizontal(|ui| {
            if ui
                .button((icons::HOUSE, " 书架"))
                .on_hover_text("返回书架")
                .clicked()
            {
                on_back();
            }
            ui.separator();
            if openable_comic
                && ui
                    .button((icons::BOOK_OPEN, " 作为漫画打开"))
                    .on_hover_text("用漫画阅读器打开该压缩包")
                    .clicked()
            {
                on_open_as_comic();
            }
            if self.state == ArchiveViewState::Ready {
                ui.separator();
                if ui
                    .button((icons::EXPORT, " 解压全部"))
                    .on_hover_text("选择目标位置并解压全部条目")
                    .clicked()
                {
                    on_extract_all();
                }
                let (selected_count, _) = self.selected_stats();
                let selected_button = egui::Button::new(format!("解压选中 ({selected_count})"));
                if ui
                    .add_enabled(selected_count > 0, selected_button)
                    .on_hover_text("解压当前选中的条目（Ctrl+单击 / Shift+单击 多选）")
                    .clicked()
                {
                    on_extract_selected(self.selected_names_in_order());
                }
                ui.separator();
                if ui
                    .add(egui::Button::new((icons::EYE, " 预览")).selected(self.preview_open))
                    .on_hover_text("切换右侧预览面板")
                    .clicked()
                {
                    self.preview_open = !self.preview_open;
                }
            }
            if let Some(path) = &self.path {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.label(egui::RichText::new(name).strong())
                    .on_hover_text(path.display().to_string());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("过滤条目")
                        .desired_width(160.0),
                );
            });
        });
        ui.separator();

        let ready = self.state == ArchiveViewState::Ready;
        if ready {
            // 底栏：纯状态栏（按钮都在顶栏）。
            egui::Panel::bottom("archive_bottom_bar").show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let file_count = self.entries.iter().filter(|e| !e.is_dir).count();
                    let total_bytes: u64 = self
                        .entries
                        .iter()
                        .filter(|e| !e.is_dir)
                        .map(|e| e.size)
                        .sum();
                    ui.label(format!(
                        "共 {file_count} 个文件 · 总大小 {}",
                        human_size(total_bytes)
                    ));
                    let (selected_count, selected_bytes) = self.selected_stats();
                    // 无选中时不显示选中统计（避免恒在的「已选 0 项 · 0 B」噪音）。
                    if selected_count > 0 {
                        ui.separator();
                        ui.label(format!(
                            "已选 {selected_count} 项 · {}",
                            human_size(selected_bytes)
                        ));
                    }
                    ui.separator();
                    let hint = if crate::platform::drag_out::is_supported() {
                        "双击打开 · 右键菜单 · 按住拖出窗口解压"
                    } else {
                        "双击打开 · 右键菜单"
                    };
                    ui.label(egui::RichText::new(hint).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let place = if self.flat_all {
                            "全部文件".to_string()
                        } else {
                            self.current_dir
                                .clone()
                                .unwrap_or_else(|| "根目录".to_string())
                        };
                        ui.label(egui::RichText::new(place).weak());
                    });
                });
                ui.add_space(4.0);
            });
            if self.preview_open {
                egui::Panel::right("archive_preview")
                    .default_size(280.0)
                    .show(ui, |ui| {
                        self.render_preview(ui);
                    });
            }
            egui::Panel::left("archive_tree_pane")
                .default_size(180.0)
                .show(ui, |ui| {
                    self.render_dir_pane(ui);
                });
            // 预览在途时主动重绘以排空后台读取结果。
            if self.preview.is_some() {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            self.handle_keyboard(
                ui,
                openable_comic,
                on_open_entry_as_comic,
                on_open_entry_external,
            );
        }

        match &self.state {
            ArchiveViewState::Idle => {}
            ArchiveViewState::Listing => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("⏳").size(24.0));
                    ui.label("正在读取压缩包…");
                });
            }
            ArchiveViewState::NeedPassword => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::LOCK.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("该压缩包已加密，需要密码").size(16.0));
                    ui.label(egui::RichText::new("输入密码后可浏览条目并解压").weak());
                    ui.add_space(8.0);
                    if ui.button((icons::KEY, " 输入密码")).clicked() {
                        on_need_password();
                    }
                });
            }
            ArchiveViewState::Failed(message) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new(icons::WARNING.as_str()).size(32.0));
                    ui.add_space(8.0);
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("无法读取压缩包: {message}"),
                    );
                });
            }
            ArchiveViewState::Ready => {
                self.render_comment_bar(ui);
                self.render_breadcrumb(ui);
                self.render_file_list(
                    ui,
                    openable_comic,
                    on_open_entry_as_comic,
                    on_open_entry_external,
                    on_extract_all,
                    on_extract_selected,
                );
            }
        }
    }

    /// 键盘导航（WinRAR/资源管理器式）：Backspace 上级、Enter 打开焦点行、
    /// ↑/↓ 移动焦点并单选、Ctrl+A 全选可见、Esc 清过滤或清空选中。
    /// 过滤框等文本输入占用键盘时不处理。
    fn handle_keyboard(
        &mut self,
        ui: &egui::Ui,
        openable_comic: bool,
        on_open_entry_as_comic: &mut dyn FnMut(String),
        on_open_entry_external: &mut dyn FnMut(String),
    ) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.go_up();
        }
        if mods.command && ui.input(|i| i.key_pressed(egui::Key::A)) {
            self.select_all_visible();
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.move_focus(1);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.move_focus(-1);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(key) = self.focus.clone() {
                self.open_row(
                    key,
                    openable_comic,
                    on_open_entry_as_comic,
                    on_open_entry_external,
                );
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if !self.filter.is_empty() {
                self.filter.clear();
            } else {
                self.clear_selection();
            }
        }
    }

    /// 打开一行的默认动作（双击/Enter/右键「打开」共用）：
    /// 目录 = 进入；文件 = 漫画格式包的图片进漫画链路，其余外部打开。
    fn open_row(
        &mut self,
        key: RowKey,
        openable_comic: bool,
        on_open_entry_as_comic: &mut dyn FnMut(String),
        on_open_entry_external: &mut dyn FnMut(String),
    ) {
        match key {
            RowKey::Dir(dir) => self.enter_dir(dir),
            RowKey::File(name) => {
                if openable_comic && openitgo_parser::traits::is_comic_image_name(&name) {
                    on_open_entry_as_comic(name);
                } else {
                    on_open_entry_external(name);
                }
            }
        }
    }

    /// 左栏目录树（纯导航）：特殊根节点「全部文件」+ 仅目录节点。
    /// 整行可点（Explorer 式，行右侧空白同样有效），横向滚动容纳长目录名；
    /// 行内手画 缩进 → 折叠三角（独立热区，只切折叠不进入）→ 图标 → 名称；
    /// 当前目录高亮铺整行，行高与明细列表同为 ROW_HEIGHT。
    /// tree_reveal_pending 置位时（导航变更）先统一展开 current_dir 的全部
    /// 祖先（祖先展开会影响行序列，故在建行之前处理），再滚动揭示命中行。
    fn render_dir_pane(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut reveal_all_files = false;
                let mut reveal_current = false;
                if self.tree_reveal_pending {
                    self.tree_reveal_pending = false;
                    if self.flat_all {
                        reveal_all_files = true;
                    } else if let Some(dir) = self.current_dir.clone() {
                        for path in breadcrumb_paths(&dir) {
                            self.collapsed.remove(&path);
                        }
                        reveal_current = true;
                    }
                }
                let rows = build_dir_rows(&self.entries, &self.collapsed);
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                let painter = ui.painter().clone();
                let text_w = |s: &str| {
                    painter
                        .layout_no_wrap(s.to_string(), font_id.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                };
                // 行宽 = 视口宽与最宽内容行（缩进+三角+图标+间距+名称）的较大者，
                // 内容超出视口时由横向滚动揭示。
                let content_w = |depth: usize, has_triangle: bool, name: &str| {
                    let triangle = if has_triangle { 16.0 } else { 0.0 };
                    4.0 + depth as f32 * 16.0 + triangle + 16.0 + 6.0 + text_w(name) + 8.0
                };
                let mut width = content_w(0, false, "全部文件");
                for row in &rows {
                    width = width.max(content_w(row.depth, true, &row.name));
                }
                let width = width.max(ui.clip_rect().width());
                self.dir_all_files_row(ui, width, reveal_all_files);
                for row in &rows {
                    self.dir_pane_row(ui, row, width, reveal_current);
                }
            });
    }

    /// 「全部文件」特殊根行：整行可点，高亮扁平模式。
    fn dir_all_files_row(&mut self, ui: &mut egui::Ui, width: f32, reveal: bool) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
        if reveal {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let painter = ui.painter().clone();
        if self.flat_all {
            painter.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        let cy = rect.center().y;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        painter.text(
            egui::pos2(rect.left() + 4.0, cy),
            egui::Align2::LEFT_CENTER,
            icons::FILES.as_str(),
            font_id.clone(),
            ui.visuals().weak_text_color(),
        );
        painter.text(
            egui::pos2(rect.left() + 4.0 + 16.0 + 6.0, cy),
            egui::Align2::LEFT_CENTER,
            "全部文件",
            font_id,
            ui.visuals().text_color(),
        );
        if response.clicked() {
            self.show_all_files();
        }
    }

    /// 目录树行：整行可点（单击设为当前目录，高亮铺整行）；行内手画
    /// 缩进 → 折叠三角（独立小热区，只切折叠不进入）→ 文件夹图标 → 名称。
    fn dir_pane_row(&mut self, ui: &mut egui::Ui, row: &TreeRow, width: f32, reveal: bool) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
        let is_current =
            !self.flat_all && self.current_dir.as_deref() == Some(row.full_path.as_str());
        if reveal && is_current {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let painter = ui.painter().clone();
        if is_current {
            painter.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        let cy = rect.center().y;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let mut x = rect.left() + 4.0 + row.depth as f32 * 16.0;
        if row.has_children {
            // 折叠三角区域单独一个点击热区（后注册，覆盖在整行热区之上）。
            let tri_rect =
                egui::Rect::from_center_size(egui::pos2(x + 8.0, cy), egui::vec2(16.0, ROW_HEIGHT));
            let tri_response = ui.interact(
                tri_rect,
                ui.id().with(("tree-collapse", &row.full_path)),
                egui::Sense::click(),
            );
            let is_collapsed = self.collapsed.contains(&row.full_path);
            let glyph = if is_collapsed { "▸" } else { "▾" };
            let color = if tri_response.hovered() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            painter.text(
                tri_rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                font_id.clone(),
                color,
            );
            if tri_response.on_hover_text("展开/折叠").clicked() {
                if is_collapsed {
                    self.collapsed.remove(&row.full_path);
                } else {
                    self.collapsed.insert(row.full_path.clone());
                }
            }
        }
        // 无子目录的行也占三角位，图标跨行对齐。
        x += 16.0;
        painter.text(
            egui::pos2(x, cy),
            egui::Align2::LEFT_CENTER,
            icons::FOLDER.as_str(),
            font_id.clone(),
            ui.visuals().weak_text_color(),
        );
        painter.text(
            egui::pos2(x + 16.0 + 6.0, cy),
            egui::Align2::LEFT_CENTER,
            &row.name,
            font_id,
            ui.visuals().text_color(),
        );
        let clicked = response.clicked();
        response.on_hover_text(&row.full_path);
        if clicked {
            self.enter_dir(row.full_path.clone());
        }
    }

    /// 面包屑上方的 ZIP 注释栏：可折叠（默认收起，标题为首行），
    /// 弱色全文 + Tooltip 显示全文；无注释不渲染。
    fn render_comment_bar(&mut self, ui: &mut egui::Ui) {
        let Some(comment) = self.comment.clone() else {
            return;
        };
        let first_line: String = comment
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect();
        egui::CollapsingHeader::new(format!("{} 注释: {first_line}", icons::NOTE.as_str()))
            .default_open(false)
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&comment).weak()).wrap());
            })
            .header_response
            .on_hover_text(comment);
    }

    /// 面包屑：过滤激活时显示「<过滤词>（N 个匹配）」（列表已是全包匹配，
    /// 显示路径段会自相矛盾）；扁平模式显示「全部文件」；否则
    /// 「根目录 / dir1 / dir2」，每段可点击跳回。
    fn render_breadcrumb(&mut self, ui: &mut egui::Ui) {
        let needle = self.filter.trim();
        if !needle.is_empty() {
            let matches = self
                .rows()
                .iter()
                .filter(|r| matches!(r, ListRow::File { .. }))
                .count();
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} {needle}（{matches} 个匹配）",
                        icons::MAGNIFYING_GLASS.as_str()
                    ))
                    .strong(),
                );
            });
            ui.separator();
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if self.flat_all {
                ui.label(egui::RichText::new("全部文件").strong());
            } else {
                if ui
                    .selectable_label(self.current_dir.is_none(), "根目录")
                    .clicked()
                {
                    self.current_dir = None;
                    self.clear_selection();
                }
                if let Some(dir) = self.current_dir.clone() {
                    for path in breadcrumb_paths(&dir) {
                        let label = path.rsplit('/').next().unwrap_or(&path).to_string();
                        ui.label(egui::RichText::new("/").weak());
                        let is_current = path == dir;
                        if ui.selectable_label(is_current, label).clicked() {
                            self.enter_dir(path);
                        }
                    }
                }
            }
        });
        ui.separator();
    }

    /// 列头：名称 / 大小 / 压缩后 / 时间，整列格可点击切换排序键与升降序，
    /// 当前键显示 ▲/▼。列坐标取自 column_layout（与行内容/竖线同一来源）；
    /// 名称列左对齐并带图标占位缩进（与行内名称 x 对齐），右侧三列右对齐。
    /// 整行铺淡底色 + hover 列高亮 + 列间竖线 + 底部描边（WinRAR 式表头）。
    /// 三条分隔竖线各带 6pt 拖拽热区（后注册于列点击格，拖拽优先），
    /// 拖动调整右侧列宽（clamp COL_MIN..=COL_MAX），hover 显示横向调整光标。
    fn render_column_header(&mut self, ui: &mut egui::Ui) {
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
        let layout = self.layout(header_rect.right());
        let cy = header_rect.center().y;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let name_rect = egui::Rect::from_min_max(
            header_rect.min,
            egui::pos2(layout.size_left, header_rect.max.y),
        );
        let size_rect = egui::Rect::from_min_max(
            egui::pos2(layout.size_left, header_rect.min.y),
            egui::pos2(layout.packed_left, header_rect.max.y),
        );
        let packed_rect = egui::Rect::from_min_max(
            egui::pos2(layout.packed_left, header_rect.min.y),
            egui::pos2(layout.mtime_left, header_rect.max.y),
        );
        let mtime_rect = egui::Rect::from_min_max(
            egui::pos2(layout.mtime_left, header_rect.min.y),
            header_rect.max,
        );
        // (列 rect, 排序键, 标题, 文字锚点 x, 对齐方式)
        let cols: [(egui::Rect, SortKey, &str, f32, egui::Align2); 4] = [
            (
                name_rect,
                SortKey::Name,
                "名称",
                header_rect.left() + NAME_HEADER_INDENT,
                egui::Align2::LEFT_CENTER,
            ),
            (
                size_rect,
                SortKey::Size,
                "大小",
                layout.packed_left,
                egui::Align2::RIGHT_CENTER,
            ),
            (
                packed_rect,
                SortKey::Packed,
                "压缩后",
                layout.mtime_left,
                egui::Align2::RIGHT_CENTER,
            ),
            (
                mtime_rect,
                SortKey::Modified,
                "时间",
                layout.content_right,
                egui::Align2::RIGHT_CENTER,
            ),
        ];
        let mut clicked: Option<SortKey> = None;
        let mut texts: Vec<(egui::Pos2, egui::Align2, String)> = Vec::with_capacity(4);
        for (rect, key, label, anchor_x, align) in cols {
            let response = ui.interact(
                rect,
                ui.id().with(("archive-header", label)),
                egui::Sense::click(),
            );
            if response.clicked() {
                clicked = Some(key);
            }
            if response.hovered() {
                painter.rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
            }
            let arrow = if self.sort_key == key {
                if self.sort_asc {
                    " ▲"
                } else {
                    " ▼"
                }
            } else {
                ""
            };
            texts.push((egui::pos2(anchor_x, cy), align, format!("{label}{arrow}")));
        }
        // 列宽拖拽热区：三条分隔竖线各 ±3pt，后注册于列点击格使拖拽优先。
        // 拖动竖线调整其右侧列宽（右移 = 变窄），hover/拖拽时换光标并加深竖线。
        let sep_xs = [layout.size_left, layout.packed_left, layout.mtime_left];
        let mut hovered_sep: Option<usize> = None;
        for (i, &x) in sep_xs.iter().enumerate() {
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(x - 3.0, header_rect.top()),
                egui::pos2(x + 3.0, header_rect.bottom()),
            );
            let response = ui.interact(
                drag_rect,
                ui.id().with(("archive-col-sep", i)),
                egui::Sense::drag(),
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                hovered_sep = Some(i);
            }
            if response.dragged() {
                let dx = response.drag_motion().x;
                if dx != 0.0 {
                    let width = match i {
                        0 => &mut self.col_width_size,
                        1 => &mut self.col_width_packed,
                        _ => &mut self.col_width_mtime,
                    };
                    *width = (*width - dx).clamp(COL_MIN_WIDTH, COL_MAX_WIDTH);
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
        if let Some(key) = clicked {
            self.toggle_sort(key);
        }
    }

    /// 中栏明细列表：行模型见 archive_tree::list_rows（目录优先 + 排序 +
    /// 过滤）。整行交互：单击选择（Ctrl/Shift 多选）、双击/Enter 打开、
    /// 右键菜单、按住拖出窗口解压；固定行高 + show_rows 虚拟化。
    fn render_file_list(
        &mut self,
        ui: &mut egui::Ui,
        openable_comic: bool,
        on_open_entry_as_comic: &mut dyn FnMut(String),
        on_open_entry_external: &mut dyn FnMut(String),
        on_extract_all: &mut dyn FnMut(),
        on_extract_selected: &mut dyn FnMut(Vec<String>),
    ) {
        self.render_column_header(ui);
        let rows = self.rows();
        if rows.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(egui::RichText::new("没有匹配的条目").weak());
            });
            return;
        }
        let flat = self.flat_all || !self.filter.trim().is_empty();
        // 滚动条恒显：内容溢出与否行区宽度恒定，表头与行内容/竖线恒对齐。
        // 键盘移动焦点：show_rows 只渲染可见行，按 Explorer 最小滚动语义
        // 仅当焦点行越出视口时把偏移调到行上缘/下缘贴边。
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        if self.focus_scroll_pending {
            self.focus_scroll_pending = false;
            if self.last_viewport_height > 0.0 {
                if let Some(focus) = &self.focus {
                    if let Some(i) = rows.iter().position(|r| self.row_key(r) == *focus) {
                        let new_offset = min_scroll_to_reveal(
                            self.last_scroll_offset,
                            self.last_viewport_height,
                            i as f32 * ROW_HEIGHT,
                        );
                        if (new_offset - self.last_scroll_offset).abs() > 0.01 {
                            area = area.vertical_scroll_offset(new_offset);
                        }
                    }
                }
            }
        }
        // 每帧意图：行交互/右键菜单设置，帧尾统一外抛（避免回调嵌套借用）。
        let mut open_key: Option<RowKey> = None;
        let mut extract_selected = false;
        let mut extract_all = false;
        let mut preview_name: Option<String> = None;
        let output = area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            let first = range.start;
            for (offset, row) in rows[range].iter().enumerate() {
                self.render_list_row(
                    ui,
                    row,
                    first + offset,
                    flat,
                    &mut open_key,
                    &mut extract_selected,
                    &mut extract_all,
                    &mut preview_name,
                );
            }
        });
        self.last_scroll_offset = output.state.offset.y;
        self.last_viewport_height = output.inner_rect.height();
        if let Some(name) = preview_name {
            self.preview_open = true;
            self.preview_entry = Some(name);
        }
        if let Some(key) = open_key {
            self.open_row(
                key,
                openable_comic,
                on_open_entry_as_comic,
                on_open_entry_external,
            );
        }
        if extract_selected {
            on_extract_selected(self.selected_names_in_order());
        }
        if extract_all {
            on_extract_all();
        }
    }

    /// 明细列表一行：整行 allocate 交互 + 斑马纹/高亮/焦点描边 + 列分隔竖线，
    /// 名称列截断、大小/压缩后列右对齐固定宽。目录行：双击进入、拖动 = 后代文件集。
    #[allow(clippy::too_many_arguments)]
    fn render_list_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &ListRow,
        row_index: usize,
        flat: bool,
        open_key: &mut Option<RowKey>,
        extract_selected: &mut bool,
        extract_all: &mut bool,
        preview_name: &mut Option<String>,
    ) {
        let key = self.row_key(row);
        let selected = self.row_selected(&key);
        let focused = self.focus.as_ref() == Some(&key);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        // 斑马纹（全局行号，滚动时条纹不闪动）先铺底，再叠加选中/悬停高亮。
        if row_index.is_multiple_of(2) {
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
            ui.painter().rect_stroke(
                rect,
                2.0,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
        let layout = self.layout(rect.right());
        paint_column_separators(
            ui.painter(),
            rect,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
            &layout,
        );

        // 行内容：图标 + 名称（目录模式显示 basename，扁平/过滤显示全路径），
        // 右侧固定宽的大小/压缩后/时间列（目录行留空）。
        let file = match row {
            ListRow::File { idx } => Some(&self.entries[*idx]),
            ListRow::Dir { .. } => None,
        };
        let content = rect.shrink2(egui::vec2(6.0, 2.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match row {
                    ListRow::Dir { name, .. } => {
                        ui.label(egui::RichText::new(icons::FOLDER.as_str()).weak());
                        ui.add(egui::Label::new(name).truncate());
                    }
                    ListRow::File { .. } => {
                        let name = &file.expect("file row").name;
                        ui.label(egui::RichText::new(entry_icon(name).as_str()).weak());
                        let display = if flat {
                            name.as_str()
                        } else {
                            name.rsplit(['/', '\\']).next().unwrap_or(name)
                        };
                        ui.add(egui::Label::new(display).truncate());
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(self.col_width_mtime, ROW_HEIGHT - 4.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if let Some(e) = file {
                                ui.label(egui::RichText::new(format_mtime(e.mtime)).weak());
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(self.col_width_packed, ROW_HEIGHT - 4.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if let Some(packed) = file.and_then(|e| e.compressed_size) {
                                ui.label(egui::RichText::new(human_size(packed)).weak());
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(self.col_width_size, ROW_HEIGHT - 4.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if let Some(e) = file {
                                ui.label(egui::RichText::new(human_size(e.size)).weak());
                            }
                        },
                    );
                });
            });
        });

        let mods = ui.input(|i| i.modifiers);
        if response.clicked() {
            self.click_row(key.clone(), mods.command, mods.shift);
            if let RowKey::File(name) = &key {
                self.preview_entry = Some(name.clone());
            }
        }
        if response.double_clicked() {
            *open_key = Some(key.clone());
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            let names = match &key {
                RowKey::File(name) => Self::drag_entry_set(&self.selected_names_in_order(), name),
                RowKey::Dir(dir) => self.descendant_file_names(dir),
            };
            let origin = ui.input(|i| {
                i.pointer
                    .press_origin()
                    .or_else(|| i.pointer.latest_pos())
                    .unwrap_or_default()
            });
            self.begin_drag_potential(names, origin);
        }
        response.context_menu(|ui| {
            // Explorer 惯例：右键未选中的行先把它单选。
            if !self.row_selected(&key) {
                self.click_row(key.clone(), false, false);
            }
            match &key {
                RowKey::Dir(_) => {
                    if ui.button((icons::FOLDER_OPEN, " 进入")).clicked() {
                        *open_key = Some(key.clone());
                        ui.close();
                    }
                }
                RowKey::File(_) => {
                    if ui.button((icons::ARROW_SQUARE_OUT, " 打开")).clicked() {
                        *open_key = Some(key.clone());
                        ui.close();
                    }
                    if ui.button((icons::EYE, " 预览")).clicked() {
                        if let RowKey::File(name) = &key {
                            *preview_name = Some(name.clone());
                        }
                        ui.close();
                    }
                }
            }
            ui.separator();
            let (count, _) = self.selected_stats();
            if ui
                .add_enabled(
                    count > 0,
                    egui::Button::new((icons::EXPORT, format!(" 解压选中到… ({count})"))),
                )
                .clicked()
            {
                *extract_selected = true;
                ui.close();
            }
            if matches!(&key, RowKey::Dir(_)) && ui.button((icons::EXPORT, " 解压全部…")).clicked()
            {
                *extract_all = true;
                ui.close();
            }
            ui.separator();
            if ui.button((icons::CHECK_SQUARE, " 全选")).clicked() {
                self.select_all_visible();
                ui.close();
            }
            if ui.button((icons::X, " 清空选中")).clicked() {
                self.clear_selection();
                ui.close();
            }
        });
        // 悬停信息提示（被截断名称的完整信息）：目录 = 完整路径；
        // 文件 = 全路径 + 大小 + 压缩后。
        let tip = match &key {
            RowKey::Dir(dir) => dir.clone(),
            RowKey::File(name) => {
                let mut tip = name.clone();
                if let Some(entry) = self.entries.iter().find(|e| !e.is_dir && e.name == *name) {
                    tip.push_str(&format!("\n大小: {}", human_size(entry.size)));
                    if let Some(packed) = entry.compressed_size {
                        tip.push_str(&format!("\n压缩后: {}", human_size(packed)));
                    }
                }
                tip
            }
        };
        response.on_hover_text(tip);
    }

    /// 右侧预览面板内容：条目名/大小 + 图片纹理 / 只读文本 / 说明。
    fn render_preview(&mut self, ui: &mut egui::Ui) {
        // poll 收到的 ColorImage 在此（有 ctx）惰性上传为纹理。
        if let Some(img) = self.pending_preview_image.take() {
            self.preview_tex = Some(ui.ctx().load_texture(
                "archive-preview",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Some(name) = self.preview_entry.clone() else {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("单击条目以预览").weak());
                    return;
                };
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&name).strong());
                if let Some(entry) = self.entries.iter().find(|e| e.name == name) {
                    ui.label(egui::RichText::new(human_size(entry.size)).weak());
                }
                ui.separator();
                if let Some(tex) = &self.preview_tex {
                    let width = ui.available_width();
                    let size = tex.size_vec2();
                    let height = if size.x > 0.0 {
                        width * size.y / size.x
                    } else {
                        width
                    };
                    ui.image(egui::load::SizedTexture::new(
                        tex.id(),
                        egui::vec2(width, height),
                    ));
                } else if let Some(text) = &self.preview_text {
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
                }
                if let Some(note) = &self.preview_note {
                    ui.label(egui::RichText::new(note).weak());
                }
                if self.preview.is_some()
                    && self.preview_tex.is_none()
                    && self.preview_text.is_none()
                    && self.preview_note.is_none()
                {
                    ui.label(egui::RichText::new("正在读取…").weak());
                }
            });
    }
}

/// 后台读取并分类预览内容：图片解码、文本（UTF-8 直读，否则 chardetng
/// 识别 GBK 等编码，嗅探前 256KB）、其余不支持；超过 64MB 的条目不读取。
fn load_preview(
    path: &Path,
    entry_name: &str,
    size: u64,
    password: Option<&str>,
) -> Result<PreviewData, String> {
    if size > PREVIEW_MAX_BYTES {
        return Ok(PreviewData::Note("文件过大，不预览".to_string()));
    }
    let bytes = read_entry(path, entry_name, password).map_err(|e| e.to_string())?;
    Ok(classify_preview_bytes(entry_name, &bytes))
}

/// 按扩展名与内容嗅探分类已读出的字节（纯函数，便于单测）。
fn classify_preview_bytes(name: &str, bytes: &[u8]) -> PreviewData {
    let is_image = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(openitgo_parser::traits::is_image_extension);
    if is_image {
        return match image::load_from_memory(bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                PreviewData::Image(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
            }
            Err(e) => PreviewData::Note(format!("无法解码图片: {e}")),
        };
    }
    let prefix = &bytes[..bytes.len().min(TEXT_SNIFF_BYTES)];
    match openitgo_parser::archive::decode_text_guess(prefix) {
        Some(s) if !s.contains('\0') => {
            let mut truncated: String = s.chars().take(TEXT_PREVIEW_MAX_CHARS).collect();
            if s.chars().count() > TEXT_PREVIEW_MAX_CHARS {
                truncated.push_str("\n…（内容过长，已截断）");
            }
            PreviewData::Text(truncated)
        }
        _ => PreviewData::Unsupported,
    }
}

/// 名称列与「大小」「压缩后」「时间」列之间的淡竖线（表头与数据行共用；
/// 坐标取自 column_layout，与表头列区间一致）。
fn paint_column_separators(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    layout: &ColumnLayout,
) {
    let stroke = egui::Stroke::new(1.0, color);
    for x in [layout.size_left, layout.packed_left, layout.mtime_left] {
        painter.vline(x, rect.y_range(), stroke);
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

/// 明细列表文件行的类型图标（按扩展名，大小写不敏感）；目录行恒用 FOLDER。
fn entry_icon(name: &str) -> Icon {
    let ext = Path::new(name)
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
        "zip" | "cbz" | "rar" | "cbr" | "7z" | "tar" | "gz" | "xz" | "zst" | "bz2" => {
            icons::FILE_ARCHIVE
        }
        _ => icons::FILE,
    }
}

/// 人类可读大小：B / KB / MB / GB。
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// 条目修改时间显示：`YYYY-MM-DD HH:MM`（本地时区），None/非法时间戳 → 空串。
pub(crate) fn format_mtime(mtime: Option<i64>) -> String {
    let Some(ts) = mtime else {
        return String::new();
    };
    // 本地时区获取失败（极少见）回退 UTC。
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    format_mtime_with_offset(ts, offset)
}

/// format_mtime 的纯函数核心（固定 offset，便于单测）。
fn format_mtime_with_offset(ts: i64, offset: time::UtcOffset) -> String {
    let Ok(dt) = time::OffsetDateTime::from_unix_timestamp(ts) else {
        return String::new();
    };
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
    dt.to_offset(offset)
        .format(&format)
        .unwrap_or_else(|_| String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            name: name.to_string(),
            is_dir,
            size,
            compressed_size: None,
            mtime: None,
        }
    }

    #[test]
    fn apply_listing_result_state_transitions() {
        let mut view = ArchiveView::default();
        view.apply_listing_result(Ok((vec![entry("a.txt", false, 1)], None)));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert_eq!(view.entries.len(), 1);

        view.apply_listing_result(Err(PASSWORD_REQUIRED_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);

        view.apply_listing_result(Err("IO error: x".to_string()));
        assert_eq!(
            view.state,
            ArchiveViewState::Failed("IO error: x".to_string())
        );
    }

    #[test]
    fn apply_listing_result_tracks_password_attempt_outcome() {
        let mut view = ArchiveView::default();
        // 无密码尝试时的密码错误：不触发 incorrect 复现。
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(!view.password_failed);
        // 带密码尝试后仍失败：password_failed 置位，tried_password 保留。
        view.tried_password = Some("pw".to_string());
        view.apply_listing_result(Err(PASSWORD_INCORRECT_MARKER.to_string()));
        assert_eq!(view.state, ArchiveViewState::NeedPassword);
        assert!(view.password_failed);
        // 带密码尝试成功：Ready 且 password_failed 清除。
        view.apply_listing_result(Ok((vec![entry("a.txt", false, 1)], None)));
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert!(!view.password_failed);
        assert_eq!(view.tried_password.as_deref(), Some("pw"));
    }

    #[test]
    fn rows_respect_root_filter_and_current_dir() {
        let mut view = ArchiveView {
            entries: vec![
                entry("Dir/", true, 0),
                entry("Page01.PNG", false, 10),
                entry("notes.md", false, 5),
                entry("Dir/inner.png", false, 3),
            ],
            ..Default::default()
        };
        // 默认根目录：目录行在前 + 顶层文件。
        assert_eq!(
            view.rows(),
            vec![
                ListRow::Dir {
                    full_path: "Dir".to_string(),
                    name: "Dir".to_string()
                },
                ListRow::File { idx: 2 },
                ListRow::File { idx: 1 },
            ]
        );
        // 过滤激活：忽略 current_dir 全包匹配（大小写不敏感）。
        view.filter = " PAGE ".to_string();
        assert_eq!(view.rows(), vec![ListRow::File { idx: 1 }]);
        // 进入目录：仅直接子文件。
        view.filter.clear();
        view.current_dir = Some("Dir".to_string());
        assert_eq!(view.rows(), vec![ListRow::File { idx: 3 }]);
        // 全部文件扁平模式：全包文件（按显示名自然序：inner < notes < Page01），无目录行。
        view.flat_all = true;
        assert_eq!(
            view.rows(),
            vec![
                ListRow::File { idx: 3 },
                ListRow::File { idx: 2 },
                ListRow::File { idx: 1 },
            ]
        );
    }

    #[test]
    fn click_row_plain_ctrl_and_dir_cascade() {
        let mut view = ArchiveView {
            entries: vec![
                entry("a/b.png", false, 1),
                entry("a/c.png", false, 1),
                entry("top.png", false, 1),
            ],
            ..Default::default()
        };
        // 普通点击目录行：级联选中其全部后代文件。
        view.click_row(RowKey::Dir("a".to_string()), false, false);
        assert_eq!(view.selected.len(), 2);
        assert!(view.selected.contains("a/b.png"));
        // 普通点击文件行：替换为单选。
        view.click_row(RowKey::File("top.png".to_string()), false, false);
        assert_eq!(view.selected.len(), 1);
        assert!(view.selected.contains("top.png"));
        // Ctrl+点击追加；Ctrl+点击已选目录行整体取消。
        view.click_row(RowKey::Dir("a".to_string()), true, false);
        assert_eq!(view.selected.len(), 3);
        view.click_row(RowKey::Dir("a".to_string()), true, false);
        assert_eq!(view.selected.len(), 1);
        assert!(view.selected.contains("top.png"));
    }

    #[test]
    fn click_row_shift_selects_row_range() {
        let mut view = ArchiveView {
            entries: vec![
                entry("d/f.png", false, 1),
                entry("a.png", false, 1),
                entry("b.png", false, 1),
                entry("c.png", false, 1),
            ],
            ..Default::default()
        };
        // 根目录行序：Dir(d), a.png, b.png, c.png。
        view.click_row(RowKey::File("a.png".to_string()), false, false);
        view.click_row(RowKey::File("c.png".to_string()), false, true);
        // Shift 范围含中间的 b.png（不含目录行 d 的内容? 含：区间内目录行也级联）。
        assert!(view.selected.contains("a.png"));
        assert!(view.selected.contains("b.png"));
        assert!(view.selected.contains("c.png"));
        // 反向范围同样覆盖。
        view.click_row(RowKey::File("c.png".to_string()), false, false);
        view.click_row(RowKey::Dir("d".to_string()), false, true);
        assert!(view.selected.contains("d/f.png"));
        assert!(view.selected.contains("c.png"));
        assert!(view.selected.contains("b.png"));
        assert!(view.selected.contains("a.png"));
        // Ctrl+Shift 为追加：保留已选再叠加新区间。
        view.click_row(RowKey::File("a.png".to_string()), false, false);
        view.selected.clear();
        view.selected.insert("d/f.png".to_string());
        view.click_row(RowKey::File("b.png".to_string()), true, true);
        assert!(view.selected.contains("d/f.png"));
        assert!(view.selected.contains("a.png"));
        assert!(view.selected.contains("b.png"));
    }

    #[test]
    fn select_all_visible_cascades_dirs_and_skips_nothing() {
        let mut view = ArchiveView {
            entries: vec![
                entry("dir/", true, 0),
                entry("dir/b.png", false, 9),
                entry("a.txt", false, 7),
            ],
            ..Default::default()
        };
        view.select_all_visible();
        assert_eq!(view.selected.len(), 2);
        assert!(!view.selected.contains("dir/"));
        // 过滤后全选只选可见文件。
        view.selected.clear();
        view.filter = "png".to_string();
        view.select_all_visible();
        assert_eq!(view.selected.len(), 1);
        assert!(view.selected.contains("dir/b.png"));
    }

    #[test]
    fn go_up_climbs_and_flat_returns_to_root() {
        let mut view = ArchiveView {
            current_dir: Some("a/b".to_string()),
            ..Default::default()
        };
        view.go_up();
        assert_eq!(view.current_dir.as_deref(), Some("a"));
        assert!(view.tree_reveal_pending);
        view.tree_reveal_pending = false;
        view.go_up();
        assert_eq!(view.current_dir, None);
        assert!(view.tree_reveal_pending);
        // 根目录再向上为 no-op（不置揭示标记）。
        view.tree_reveal_pending = false;
        view.go_up();
        assert_eq!(view.current_dir, None);
        assert!(!view.tree_reveal_pending);
        // 扁平模式向上回到根目录。
        view.flat_all = true;
        view.go_up();
        assert!(!view.flat_all);
        assert_eq!(view.current_dir, None);
        assert!(view.tree_reveal_pending);
    }

    #[test]
    fn enter_dir_and_show_all_files_set_tree_reveal() {
        let mut view = ArchiveView::default();
        assert!(!view.tree_reveal_pending);
        view.enter_dir("a/b".to_string());
        assert!(view.tree_reveal_pending);
        view.tree_reveal_pending = false;
        view.show_all_files();
        assert!(view.tree_reveal_pending);
        assert!(view.flat_all);
        assert_eq!(view.current_dir, None);
    }

    #[test]
    fn move_focus_walks_rows_and_selects_single() {
        let mut view = ArchiveView {
            entries: vec![
                entry("d/f.png", false, 1),
                entry("a.png", false, 1),
                entry("b.png", false, 1),
            ],
            ..Default::default()
        };
        // 无焦点时向下 = 首行（目录行，级联选中；预览目标不动）。
        view.move_focus(1);
        assert_eq!(view.focus, Some(RowKey::Dir("d".to_string())));
        assert!(view.selected.contains("d/f.png"));
        assert!(view.focus_scroll_pending);
        assert_eq!(view.preview_entry, None);
        view.focus_scroll_pending = false;
        // 文件行：预览跟随键盘焦点（与鼠标单击一致）。
        view.move_focus(1);
        assert_eq!(view.focus, Some(RowKey::File("a.png".to_string())));
        assert_eq!(view.selected.len(), 1);
        assert_eq!(view.preview_entry.as_deref(), Some("a.png"));
        // 末行钳位。
        view.move_focus(5);
        assert_eq!(view.focus, Some(RowKey::File("b.png".to_string())));
        assert_eq!(view.preview_entry.as_deref(), Some("b.png"));
        // 顶部再向上停在首行（目录行不改预览目标）。
        view.move_focus(-10);
        assert_eq!(view.focus, Some(RowKey::Dir("d".to_string())));
        assert_eq!(view.preview_entry.as_deref(), Some("b.png"));
    }

    #[test]
    fn toggle_sort_flips_or_switches_key() {
        let mut view = ArchiveView::default();
        assert_eq!(view.sort_key, SortKey::Name);
        assert!(view.sort_asc);
        view.toggle_sort(SortKey::Name);
        assert!(!view.sort_asc);
        view.toggle_sort(SortKey::Size);
        assert_eq!(view.sort_key, SortKey::Size);
        assert!(view.sort_asc);
    }

    #[test]
    fn selected_stats_and_order() {
        let mut view = ArchiveView {
            entries: vec![
                entry("z.png", false, 3),
                entry("dir/", true, 0),
                entry("a.png", false, 4),
            ],
            ..Default::default()
        };
        view.selected.insert("a.png".to_string());
        view.selected.insert("z.png".to_string());
        assert_eq!(view.selected_stats(), (2, 7));
        // 按包内顺序而非插入顺序。
        assert_eq!(view.selected_names_in_order(), vec!["z.png", "a.png"]);
    }

    #[test]
    fn min_scroll_to_reveal_keeps_visible_row() {
        // 行完全在视口内（110..132 ⊂ 100..300）：偏移不变。
        assert!((min_scroll_to_reveal(100.0, 200.0, 110.0) - 100.0).abs() < 1e-6);
        // 恰好贴顶/贴底的边界行也不动。
        assert!((min_scroll_to_reveal(100.0, 200.0, 100.0) - 100.0).abs() < 1e-6);
        assert!((min_scroll_to_reveal(100.0, 200.0, 300.0 - ROW_HEIGHT) - 100.0).abs() < 1e-6);
    }

    #[test]
    fn min_scroll_to_reveal_snaps_row_above_to_top() {
        // 行顶 44 < offset 100：新 offset = 行顶。
        assert!((min_scroll_to_reveal(100.0, 200.0, 44.0) - 44.0).abs() < 1e-6);
    }

    #[test]
    fn min_scroll_to_reveal_snaps_row_below_to_bottom() {
        // 行底 330+22=352 > 100+200：新 offset = 352-200，行底贴视口底。
        let new = min_scroll_to_reveal(100.0, 200.0, 330.0);
        assert!((new - (330.0 + ROW_HEIGHT - 200.0)).abs() < 1e-6);
        // 行更高不越界：offset=0 时第 0 行恒在视口内。
        assert!(min_scroll_to_reveal(0.0, 200.0, 0.0).abs() < 1e-6);
    }

    #[test]
    fn human_size_formats_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn cascade_set_selects_descendants_with_boundary() {
        let mut view = ArchiveView {
            entries: vec![
                entry("a/b.png", false, 1),
                entry("a/sub/c.png", false, 1),
                entry("ab/d.png", false, 1),
                entry("a.txt", false, 1),
            ],
            ..Default::default()
        };
        view.cascade_set("a", true);
        assert!(view.selected.contains("a/b.png"));
        assert!(view.selected.contains("a/sub/c.png"));
        // 边界：`a` 不得误中 `ab/` 与同名文件 `a.txt`。
        assert!(!view.selected.contains("ab/d.png"));
        assert!(!view.selected.contains("a.txt"));
        assert!(view.dir_all_selected("a"));
        view.cascade_set("a", false);
        assert!(view.selected.is_empty());
        assert!(!view.dir_all_selected("a"));
    }

    #[test]
    fn cascade_set_accepts_backslash_and_trailing_slash_paths() {
        let mut view = ArchiveView {
            entries: vec![entry("a\\b.png", false, 1), entry("a/c.png", false, 1)],
            ..Default::default()
        };
        // 尾部斜杠剥掉后照常匹配；反斜杠条目也命中。
        view.cascade_set("a/", true);
        assert_eq!(view.selected.len(), 2);
    }

    #[test]
    fn open_with_entries_sets_ready_and_resets_state() {
        let mut view = ArchiveView {
            selected: HashSet::from(["old.png".to_string()]),
            filter: "old".to_string(),
            collapsed: HashSet::from(["d".to_string()]),
            current_dir: Some("d".to_string()),
            preview_entry: Some("old.png".to_string()),
            preview_text: Some("txt".to_string()),
            tried_password: Some("pw".to_string()),
            password_failed: true,
            tree_reveal_pending: true,
            state: ArchiveViewState::Failed("x".to_string()),
            ..Default::default()
        };
        view.open_with_entries(
            PathBuf::from("/tmp/pack.zip"),
            vec![entry("a.png", false, 1)],
        );
        assert_eq!(view.state, ArchiveViewState::Ready);
        assert_eq!(view.entries.len(), 1);
        assert!(view.selected.is_empty());
        assert!(view.filter.is_empty());
        assert!(view.collapsed.is_empty());
        assert_eq!(view.current_dir, None);
        assert_eq!(view.preview_entry, None);
        assert_eq!(view.preview_text, None);
        assert_eq!(view.tried_password, None);
        assert!(!view.password_failed);
        assert!(!view.tree_reveal_pending);
        assert!(view.listing.is_none());
        assert_eq!(view.path.as_deref(), Some(Path::new("/tmp/pack.zip")));
    }

    #[test]
    fn poll_preview_spawns_only_for_open_panel_and_new_target() {
        let mut view = ArchiveView {
            path: Some(PathBuf::from("/tmp/pack.zip")),
            entries: vec![entry("a.txt", false, 1)],
            state: ArchiveViewState::Ready,
            preview_entry: Some("a.txt".to_string()),
            ..Default::default()
        };
        // 面板关闭时不发起读取。
        view.poll();
        assert!(view.preview.is_none());
        assert_eq!(view.preview_requested, None);
        // 面板打开后发起读取并记录目标。
        view.preview_open = true;
        view.poll();
        assert!(view.preview.is_some());
        assert_eq!(view.preview_requested.as_deref(), Some("a.txt"));
    }

    #[test]
    fn classify_preview_bytes_decodes_image() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        match classify_preview_bytes("p.png", &buf.into_inner()) {
            PreviewData::Image(ci) => assert_eq!(ci.size, [2, 2]),
            other => panic!("expected Image, got {other:?}"),
        }
        // 扩展名是图片但内容不可解码 → 说明。
        assert!(matches!(
            classify_preview_bytes("p.png", b"not a png"),
            PreviewData::Note(_)
        ));
    }

    #[test]
    fn classify_preview_bytes_text_and_unsupported() {
        match classify_preview_bytes("notes.txt", "你好 world".as_bytes()) {
            PreviewData::Text(t) => assert_eq!(t, "你好 world"),
            other => panic!("expected Text, got {other:?}"),
        }
        // 含 NUL → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", b"ab\0cd"),
            PreviewData::Unsupported
        ));
        // 起始即非法 UTF-8 且无法识别 → 不支持。
        assert!(matches!(
            classify_preview_bytes("a.bin", &[0xFF, 0xFE, 0x00]),
            PreviewData::Unsupported
        ));
        // GBK 编码的中文文本经 chardetng 识别后可预览
        // （"你好，世界！这是一段用于编码识别的中文测试文本。" 的 GBK 字节；
        // 统计识别需要较长样本）。
        let gbk: &[u8] = &[
            0xC4, 0xE3, 0xBA, 0xC3, 0xA3, 0xAC, 0xCA, 0xC0, 0xBD, 0xE7, 0xA3, 0xA1, 0xD5, 0xE2,
            0xCA, 0xC7, 0xD2, 0xBB, 0xB6, 0xCE, 0xD3, 0xC3, 0xD3, 0xDA, 0xB1, 0xE0, 0xC2, 0xEB,
            0xCA, 0xB6, 0xB1, 0xF0, 0xB5, 0xC4, 0xD6, 0xD0, 0xCE, 0xC4, 0xB2, 0xE2, 0xCA, 0xD4,
            0xCE, 0xC4, 0xB1, 0xBE, 0xA1, 0xA3,
        ];
        match classify_preview_bytes("a.txt", gbk) {
            PreviewData::Text(t) => {
                assert_eq!(t, "你好，世界！这是一段用于编码识别的中文测试文本。")
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_preview_bytes_truncates_long_text() {
        let long = "x".repeat(TEXT_PREVIEW_MAX_CHARS + 10_000);
        match classify_preview_bytes("a.txt", long.as_bytes()) {
            PreviewData::Text(t) => {
                assert!(t.ends_with("（内容过长，已截断）"));
                assert!(t.chars().count() < long.chars().count());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn entry_icon_by_extension() {
        // Icon 无 PartialEq，按码位字符串比较。
        let icon_str = |name: &str| entry_icon(name).as_str().to_string();
        assert_eq!(icon_str("p01.png"), icons::FILE_IMAGE.as_str());
        assert_eq!(icon_str("dir/p02.JPG"), icons::FILE_IMAGE.as_str());
        assert_eq!(icon_str("notes.txt"), icons::FILE_TEXT.as_str());
        assert_eq!(icon_str("README.md"), icons::FILE_TEXT.as_str());
        assert_eq!(icon_str("doc.pdf"), icons::FILE_PDF.as_str());
        assert_eq!(icon_str("ep01.mkv"), icons::FILE_VIDEO.as_str());
        assert_eq!(icon_str("song.flac"), icons::FILE_AUDIO.as_str());
        assert_eq!(icon_str("pack.tar.gz"), icons::FILE_ARCHIVE.as_str());
        assert_eq!(icon_str("noext"), icons::FILE.as_str());
    }

    #[test]
    fn drag_entry_set_prefers_whole_multi_selection() {
        let selected = vec![
            "a.txt".to_string(),
            "b.txt".to_string(),
            "c.txt".to_string(),
        ];
        // 拖动项在多选集合中 → 整个选中集（保持包内顺序）。
        assert_eq!(ArchiveView::drag_entry_set(&selected, "b.txt"), selected);
        // 拖动项不在选中集 → 仅单条目。
        assert_eq!(
            ArchiveView::drag_entry_set(&selected, "z.txt"),
            vec!["z.txt".to_string()]
        );
        // 单选集合 → 单条目。
        let single = vec!["a.txt".to_string()];
        assert_eq!(
            ArchiveView::drag_entry_set(&single, "a.txt"),
            vec!["a.txt".to_string()]
        );
        // 空选中 → 单条目。
        assert_eq!(
            ArchiveView::drag_entry_set(&[], "a.txt"),
            vec!["a.txt".to_string()]
        );
    }

    #[test]
    fn should_begin_extract_threshold_and_window_exit() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let origin = egui::pos2(100.0, 100.0);
        // 指针出窗（无位置或越出视口）→ 立即开始解压。
        assert!(should_begin_extract(origin, None, viewport));
        assert!(should_begin_extract(
            origin,
            Some(egui::pos2(-1.0, 100.0)),
            viewport
        ));
        assert!(should_begin_extract(
            origin,
            Some(egui::pos2(100.0, 601.0)),
            viewport
        ));
        // 窗内未超阈值 → 不开始（手滑不触发 IO）。
        assert!(!should_begin_extract(
            origin,
            Some(egui::pos2(120.0, 110.0)),
            viewport
        ));
        // 窗内超过 40pt → 开始。
        assert!(should_begin_extract(
            origin,
            Some(egui::pos2(150.0, 100.0)),
            viewport
        ));
    }

    #[test]
    fn ghost_text_by_state_and_platform() {
        assert_eq!(ghost_text(false, false, 3), "当前平台不支持拖出");
        assert_eq!(ghost_text(false, true, 3), "当前平台不支持拖出");
        assert_eq!(ghost_text(true, false, 3), "⇪ 3 个文件 · 拖到窗口外解压");
        assert_eq!(ghost_text(true, true, 1), "⇪ 1 个文件 · 正在准备拖出…");
    }

    #[test]
    fn format_mtime_with_offset_fixed_ts_and_offset() {
        // 2023-11-14 22:13:20 UTC。
        let ts = 1_700_000_000;
        assert_eq!(
            format_mtime_with_offset(ts, time::UtcOffset::UTC),
            "2023-11-14 22:13"
        );
        // UTC+8：跨日进位到次日 06:13。
        let plus8 = time::UtcOffset::from_hms(8, 0, 0).unwrap();
        assert_eq!(format_mtime_with_offset(ts, plus8), "2023-11-15 06:13");
        // 负偏移：UTC-5 → 当日 17:13。
        let minus5 = time::UtcOffset::from_hms(-5, 0, 0).unwrap();
        assert_eq!(format_mtime_with_offset(ts, minus5), "2023-11-14 17:13");
    }

    #[test]
    fn format_mtime_none_and_invalid_ts_are_empty() {
        assert_eq!(format_mtime(None), "");
        // 超出 OffsetDateTime 可表示范围的时间戳 → 空串。
        assert_eq!(format_mtime_with_offset(i64::MAX, time::UtcOffset::UTC), "");
    }
}
