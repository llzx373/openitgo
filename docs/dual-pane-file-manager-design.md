# 双栏文件管理器（Total Commander 形态）需求与设计文档

> 状态：已实施（一期）。本文档保留需求与设计原貌；实现与设计的出入见文末「实施偏差记录」。
> 调研基线：`openitgo-app/src/views/archive.rs`（3251 行）、`views/archive_tree.rs`（679 行纯函数）、
> `app.rs` 视图分发（`View` 枚举 app.rs:670）。

## 1. 背景与目标

OpenItGo 目前有一个 Archive 视图（`View::Archive`），但它是**压缩包内部内容**的浏览器：
数据源是 `list_entries()` 返回的 `ArchiveEntry` 纯路径字符串模型，不能浏览本地文件系统。
代码库中不存在通用的本地目录浏览模型（`read_dir` 仅用于续播/兄弟漫画等特定用途）。

本需求新增一个 **Total Commander 形态的双栏文件管理器**：

- **双栏模式**：左右两个独立的文件面板，各自浏览不同目录，Tab 切换焦点栏，
  栏间执行复制/移动等操作（TC 的核心手感）。
- **单栏模式**：只保留一个文件面板，另一栏的区域变为**预览面板**，
  显示当前选中文件的内容（图片/文本），类似 Explorer 预览窗格。

非目标（本期不做）：

- 网络位置（FTP/SMB/WebDAV）。
- 文件搜索、内容比较、批量重命名等 TC 高级功能。
- 取代现有 Archive 视图——两者并存，职责不同（见 §8）。

## 2. 功能需求

按优先级分 P0（首版必须）/ P1（首版应尽量）/ P2（后续迭代）。

### 2.1 面板与导航

| # | 优先级 | 需求 |
|---|--------|------|
| F1 | P0 | 双栏布局：左右两个文件面板，可拖动分隔条调整比例（默认 50/50） |
| F2 | P0 | 单栏模式：一个文件面板 + 预览区域；单/双栏可一键切换，状态持久化 |
| F3 | P0 | 每栏独立状态：当前目录、选中集、焦点行、排序、滚动位置、历史（前进/后退） |
| F4 | P0 | 焦点栏概念：高亮标识活动栏，Tab 切换；鼠标点击某栏即激活该栏 |
| F5 | P0 | 目录导航：双击/Enter 进入目录；Backspace/「..」行回上级；面包屑或可编辑路径栏 |
| F6 | P0 | 排序：名称/大小/修改时间，点列头切换升降序；目录恒排在文件前 |
| F7 | P1 | 过滤框：按子串过滤当前目录条目（复用 Archive 视图的过滤交互） |
| F8 | P1 | 导航历史：每栏前进/后退（Alt+←/→）；「..」与 Backspace 语义同 Explorer |
| F9 | P2 | 驱动器栏 / 常用目录书签（Windows 盘符列表） |

### 2.2 选择与交互

| # | 优先级 | 需求 |
|---|--------|------|
| F10 | P0 | Explorer/TC 式选择：单击单选、Ctrl 增减选、Shift 范围选、Ctrl+A 全选 |
| F11 | P0 | 键盘导航：↑↓/Home/End/PgUp/PgDn，焦点行最小滚动揭示（复用 `min_scroll_to_reveal`） |
| F12 | P0 | 右键菜单：打开 / 打开方式 / 复制 / 移动 / 删除 / 重命名 / 新建文件夹 / 刷新 / 属性（大小等） |
| F13 | P1 | 栏间拖放：从左栏拖选中标到右栏 = 复制（Windows 已有 OLE drag_out 经验，但栏内拖放用 egui 内部拖拽即可） |
| F14 | P1 | 选中即预览：单栏模式下焦点落到可预览文件自动更新预览（复用 Archive 视图「选中即预览」语义） |

### 2.3 文件操作

TC 的核心价值在文件操作，首版需覆盖基本集：

| # | 优先级 | 需求 | 默认目标 |
|---|--------|------|----------|
| F15 | P0 | 复制（F5 / Ctrl+C→Ctrl+V / 右键） | 非焦点栏的当前目录 |
| F16 | P0 | 移动（F6 / Ctrl+X→Ctrl+V） | 同上 |
| F17 | P0 | 删除（F8 / Del，移回收站，复用 `trash` crate 先例；防误删确认对话框见决策 4） | — |
| F18 | P0 | 重命名（F2）、新建文件夹（F7） | 栏内 |
| F19 | P0 | 操作前确认对话框：显示源→目标、冲突处理（覆盖/跳过/自动改名，对齐 `extract_overwrite` 语义） | — |
| F20 | P0 | 后台执行 + 进度/取消：大量文件复制不冻结 UI，走 `AsyncOpener` 式后台线程 + 每帧 poll | — |
| F21 | P1 | 错误汇总：部分失败（权限/占用）不中断整批，结束时汇总报告 | — |

### 2.4 与阅读器能力的集成

| # | 优先级 | 需求 |
|---|--------|------|
| F22 | P0 | 双击分发：图片→漫画链路、电子书/媒体→对应视图、文件夹→进入目录 |
| F23 | P0 | 右键「作为漫画打开」对文件夹/压缩包可用 |
| F24 | P0 | 双击压缩包 → 进入现有 Archive 视图（`open_archive_browser`，不做面板内进入式浏览） |
| F25 | P2 | 右键「解压到另一栏」（复用 `ExtractManager`）；「压缩选中为 zip」依赖 parser 侧写 zip 能力，需调研 |

### 2.5 单栏模式（预览形态）

| # | 优先级 | 需求 |
|---|--------|------|
| F26 | P0 | 单栏时右半区域为预览面板：图片（适应宽度/原始尺寸切换）、文本（Monospace、编码嗅探、上限截断）、不支持的类型显示元信息占位 |
| F27 | P0 | 预览内容上限与 Archive 视图一致：>64MB 不读、文本前 256KB 嗅探、64k 字符截断 |
| F28 | P1 | 预览面板可关闭/调整宽度；单栏模式下视频/音频文件显示元信息 + 「播放」按钮（直接播放走 `open_path`，不做内嵌播放） |

## 3. 关键设计决策（已确认 2025）

1. **入口形态**：新建顶级 `View::FileManager`，从库视图顶栏/菜单进入。不合并进 Archive 视图
   （两者状态机差异大：密码/异步列包/漫画分流 vs 面板导航/文件操作）；行模型/选择/预览等
   纯函数抽出复用（见 §5）。
2. **文件操作范围**：首版含复制/移动/删除/重命名/新建文件夹全集（F15–F21 全部 P0）。
3. **压缩包点击行为**：文件管理器中双击压缩包 → **进入现有 Archive 视图**（`open_archive_browser`），
   不做面板内进入式浏览。原 F24（面板数据源切换）从路线图中移除。
4. **删除语义**：一律移回收站（`trash` crate），不提供物理删除。删除确认对话框为**防误删**
   显式确认：列出待删项（超过若干项时显示前 N 项 + 计数），需点击「移入回收站」确认；
   提供「不再询问」选项（写回 settings `fm_confirm_delete=false`，设置页可改回）。
5. **持久化**：单/双栏模式、栏宽比例、排序、**每栏当前目录**均持久化（对齐 TC 记住目录的行为）。
   启动时若持久化目录已不存在，逐级回退到最近存在祖先，最终回退用户主目录。
6. **平台范围**：文件操作与浏览跨平台（Win/macOS）；栏间拖放首版仅应用内（非 OLE 拖出）。
7. **不改变** `open_path` 对普通文件夹的现有打开行为；文件管理器仅从显式入口进入。

## 4. 交互设计

### 4.1 双栏布局

```
┌──────────────────────────────────────────────────────┐
│ 顶栏: 返回书架 | 单/双栏切换 | 视图模式 | 过滤框        │
├───────────────────────────┬──────────────────────────┤
│ [C:\manga\] 面包屑/路径栏  │ [D:\downloads\]          │
│ ┌────────────────────────┐│┌────────────────────────┐│
││ 名称        大小  时间   │││ 名称        大小  时间  ││
││ ..                       │││ ..                      ││
││ 📁 One Piece             │││ 📁 tmp                  ││
││ 📄 vol01.cbz   350MB …   │◄││ 📄 setup.exe            ││  ← 分隔条可拖
││ ▓▓ focus 行（活动栏描边） │││                         ││
│ └────────────────────────┘│└────────────────────────┘│
├───────────────────────────┴──────────────────────────┤
│ 状态栏: 选中 3 项 / 共 142 项 | 空闲空间 | 操作进度     │
└──────────────────────────────────────────────────────┘
```

- 活动栏视觉：标题栏（路径栏）高亮 + 焦点行描边，对齐 Archive 视图焦点语义。
- 列布局：复用 `ColumnLayout` 锚点 + `col_shift` 方案（archive.rs:44/59），列头点击排序。
  **注意保留 AGENTS.md 记录的坑**：行内容 scope 内 `interact_size.y` 压回 `ROW_HEIGHT-4`，
  scope 后 `advance_cursor_after_rect`；右三列用 `painter.text` 右对齐直绘，不用 RTL 嵌套。

### 4.2 单栏布局

```
┌──────────────────────────────────────────────────────┐
│ 顶栏（同上，预览开关）                                 │
├──────────────────────────────────┬───────────────────┤
│ 文件面板（同双栏的左栏）          │ 预览面板           │
│                                  │ （图片/文本/元信息）│
├──────────────────────────────────┴───────────────────┤
│ 状态栏                                                │
└──────────────────────────────────────────────────────┘
```

### 4.3 键盘映射（对齐 TC + 现有 Archive 视图约定）

| 键 | 双栏模式 | 单栏模式 |
|----|----------|----------|
| Tab | 切换焦点栏 | 焦点在列表/预览间切换（或无操作） |
| ↑↓/Home/End/PgUp/PgDn | 栏内导航（Select/Extend/FocusOnly） | 同左 |
| Enter | 打开/进入 | 同左 |
| Backspace / ← | 上级目录 | 同左 |
| F2 / F5 / F6 / F7 / F8(Del) | 重命名/复制/移动/新建文件夹/删除 | 同左（目标栏=自身） |
| F3 | 预览选中文件（双栏模式下弹出或临时切单栏） | — |
| Ctrl+A / Esc | 全选 / 清过滤清选中 | 同左 |
| F5 刷新冲突 | **注意**：Archive 视图 F5 是重列，TC 的 F5 是复制。本视图 F5=复制，刷新改 Ctrl+R | 同左 |
| Alt+←/→ | 导航历史前进/后退 | 同左 |

### 4.4 文件操作确认对话框

对齐 `ExtractDialogState`（extract_dialog.rs）模式：统一入口状态结构 + `ui(ctx) -> bool`。

- 复制/移动：显示源 N 项 → 目标路径（可编辑，默认非焦点栏目录），冲突策略下拉
  （询问/覆盖/跳过/自动改名，默认值来自 settings）。
- 删除：列出前若干项名称 + 「移入回收站」说明 + 「不再询问」选项（决策 4）。
- 进度：复用状态栏区域显示进度条 + 取消按钮（`Arc<AtomicBool>` 约定同 `extract_archive`）。

## 5. 架构设计

### 5.1 模块划分（新增文件均在 `openitgo-app/src/`）

```
views/file_manager.rs        # 视图状态 + ui() 入口 + 双/单栏布局
views/file_manager_panel.rs  # 单面板：FsPanelState（目录、条目、选择、排序、历史）
views/file_ops.rs            # 复制/移动/删除/重命名 后台任务 + 进度/取消/冲突处理
views/file_manager_dialog.rs # 操作确认对话框（对齐 extract_dialog.rs 模式）
```

视图结构沿用一个 struct 持有全部状态、app.rs 持实例的模式（同 `ArchiveView`）：

```rust
// views/file_manager.rs（示意，非最终实现）
pub struct FileManagerView {
    layout: PanelLayout,            // Dual{ratio} | Single{preview_open, preview_width}
    panels: [FsPanel; 2],           // 双栏两个面板；单栏只用 [0]，[1] 区域画预览
    active: usize,                  // 焦点栏 0/1
    preview: PreviewState,          // 复用 Archive 预览的状态形状
    ops: FileOpManager,             // 后台文件操作
    dialog: Option<FileOpDialog>,
}
```

### 5.2 面板数据模型

Archive 视图的 `ArchiveEntry` 是纯路径字符串模型，不能直接表示本地 FS。新建：

```rust
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,       // 目录为 None（首版不算目录大小）
    pub mtime: Option<SystemTime>,
    pub is_symlink: bool,
}

pub struct FsPanel {
    pub dir: PathBuf,
    pub entries: Vec<FsEntry>,
    pub state: PanelLoadState,   // Idle/Loading/Ready/Failed — 异步 read_dir
    pub selected: HashSet<PathBuf>,   // 与 Archive 一致：只存文件还是含目录？——见下
    pub focus: Option<usize>, anchor: Option<usize>,
    pub sort_key: SortKey, sort_asc: bool,
    pub filter: String,
    pub history: Vec<PathBuf>, history_pos: usize,
    pub rows_cache: Option<(RowsKey, Vec<usize>)>,  // 版本化行缓存，同 Archive
}
```

**选择模型差异**：本地 FS 管理器里目录也是一等操作对象（复制整个文件夹），所以
`selected` 直接存 `PathBuf`（含目录），**不采用** Archive 的「只存文件、目录态派生」模型——
那个模型是为压缩包扁平条目设计的。`click_row`/`move_focus` 的 Explorer 语义平移过来，
以索引行为单位（比字符串 key 简单）。

**行模型纯函数**：参照 `archive_tree.rs` 建 `file_manager_rows.rs`（无 egui 依赖、可单测）：
`list_rows(entries, filter, sort, asc)`（目录恒前、Parent 恒行首）、`natural_cmp` 复用 app.rs 现有实现。

### 5.3 异步与刷新

- 目录列举：`read_dir` 对大目录（数万文件）也会卡 UI，走 `AsyncOpener`（opener.rs:11）
  后台列举 + 每帧 poll + `request_repaint_after(100ms)` 排空（egui 空闲不重绘的既有约定）。
- 首版**不做**文件系统 watch 自动刷新；手动刷新 Ctrl+R + 操作完成后自动刷新涉及的两栏。
  （notify crate 监听可作 P2。）

### 5.4 文件操作引擎（`file_ops.rs`）

```
FileOpManager
  ├── start_copy(sources, dest, conflict) -> op_id
  ├── start_move(...)      # 同盘 rename 快速路径，跨盘 copy+delete
  ├── start_delete(sources) # trash::delete_all
  ├── cancel(op_id)
  └── poll() -> OpSummary  # 每帧 app 侧调用，进度进状态栏
```

- 后台线程递归遍历源（目录展开），逐项 copy/rename/trash，进度经 channel 上报
  （字节数 + 文件计数），`Arc<AtomicBool>` 取消；取消时清理半成品（同 extract 约定）。
- 冲突处理：操作线程遇到冲突且策略=询问时，经 channel 反向请求 UI 弹对话框
  （覆盖/跳过/改名/全部应用），UI 回传决定。这是本设计里最复杂的交互，首版可简化为
  「操作前一次性扫描冲突，对话框里定策略，执行中不再问」。
- 移动 = 同设备 `fs::rename` 优先，失败（跨盘）回退 copy+trash 原文件。
- 半成品目标文件在取消/失败时删除；复制目录中途失败保留已复制部分并在汇总中列出。

### 5.5 预览复用

- 预览数据源从「压缩包条目」泛化为 trait 或枚举：
  `PreviewSource::ArchiveEntry{archive, name, password} | PreviewSource::File(PathBuf)`。
  `classify_preview_bytes`（archive.rs:2193）与大小门槛逻辑原样复用——它本就操作字节流。
- 预览状态（`preview_tex/preview_text/preview_note`、`pending_preview_image`）抽成
  共享的 `PreviewState`，Archive 视图与文件管理器共用；抽取放二期也行——首版允许
  文件管理器先复制一份小的预览实现，避免动 Archive 视图引入回归。

### 5.6 视图接入点（app.rs 改动清单预览）

- `View` 枚举加 `FileManager` 变体（app.rs:670）。
- `ReaderApp::ui` 分发加 `render_file_manager`（app.rs:648-656 旁）。
- 视图离开钩子（app.rs:587-602）：文件管理器不写历史记录，离开无特殊清理。
- `sync_window_title`（app.rs:3726）：`文件管理器 - OpenItGo` 或 `{dir} - OpenItGo`。
- 入口：库视图顶栏加「文件管理器」按钮 + 文件菜单项；`open_path` 对普通文件夹的现有
  打开行为**保持不变**（决策 7），文件管理器只从显式入口进入。

### 5.7 持久化

`Settings`（openitgo-storage/src/models.rs:8）新增（`#[serde(default)]` 向后兼容）：

```rust
pub fm_layout: String,          // "dual" | "single"，默认 "dual"
pub fm_dual_ratio: f32,         // 0.2–0.8，默认 0.5
pub fm_preview_open: bool,      // 单栏模式预览开关，默认 true
pub fm_sort_key: String,        // "name"|"size"|"mtime"
pub fm_sort_asc: bool,
pub fm_confirm_delete: bool,    // 删除前确认，默认 true
pub fm_dir_left: String,        // 左栏持久化目录（空 = 用户主目录）
pub fm_dir_right: String,       // 右栏持久化目录
```

validate/clamp 同现有字段。面板目录在视图关闭/切换时写回；启动加载时目录不存在则
逐级回退到最近存在祖先，最终回退用户主目录（决策 5）。

## 6. 错误与边界情况

- 权限拒绝目录：进入时显示栏内错误条（不回弹上级），列出可读部分。
- 目标路径不存在/不可写：操作前检查，对话框标红。
- 操作期间源被外部修改/删除：逐项容错，汇总报告。
- 长路径（Windows >260 字符）：复制时用 `\\?\` 前缀（确认 `fs::copy` 行为，必要时自实现）。
- 符号链接：默认复制链接指向的内容还是链接本身？默认**复制目标内容**（同 Explorer），
  删除符号链接只删链接。
- 隐藏文件：默认显示（TC 默认显示），设置项可关（P2）。
- 拖放冲突：egui 内部栏间拖放与 Windows OLE 拖出（`drag_out`）手势需区分——出窗才走 OLE。

## 7. 分期计划

| 期 | 内容 | 验收 |
|----|------|------|
| 一期 | 双栏浏览 + 单/双栏切换 + 选择/键盘/排序/过滤 + 打开分发（压缩包→Archive 视图）+ 单栏预览 + 复制/移动/删除/重命名/新建文件夹（后台+确认对话框+防误删）+ 面板状态持久化 | 全流程手测 + `file_manager_rows`/`file_ops` 单测；`cargo fmt/check/test/clippy` 全绿 |
| 二期 | 栏间拖放、导航历史 UI、解压到另一栏（F25 前半）、预览实现抽取共享、FS watch 刷新 | — |
| 三期 | 驱动器栏/书签、目录大小计算、压缩为 zip、隐藏文件开关 | — |

## 8. 与现有 Archive 视图的关系

两者并存，不合并：

- **Archive 视图**：压缩包内容浏览器 + 解压工作台，状态机围绕密码/异步列包/漫画分流。
- **文件管理器**：本地 FS 双栏管理，状态机围绕面板导航/文件操作。

共享层：`natural_cmp`、`human_size`/`format_mtime`、`classify_preview_bytes`、行距/列布局
绘制约定、`AsyncOpener` 后台模式、`trash` 删除、`ExtractManager`（二期「解压到另一栏」复用）。
Archive 视图本身**不改行为**，仅可能因共享代码抽取而被小幅重构（二期）。

## 9. 风险

1. **文件操作的安全性**：误删/误覆盖是最大风险。缓解：回收站删除、操作前确认、冲突策略显式化、
   取消清理半成品。
2. **egui 中长列表 + 双栏性能**：复用 `show_rows` 虚拟化与行缓存，预期无新问题。
3. **范围膨胀**：面板内压缩包浏览（原 F24）已移除，双击压缩包直接进 Archive 视图（决策 3）。
4. **Windows 专有行为**（长路径、盘符、OLE 拖出）：一期核心跨平台，Windows 细节单列任务验证。

## 10. 实施偏差记录（一期实现 vs 本文档）

一期（阶段一至五）已落地。以下各点实现与本设计文档有意或无意出入，以此节为准：

1. **冲突「询问」语义**：§5（243 行）曾设计「执行线程遇冲突经 channel 反向请求 UI」，
   并预留「首版可简化为操作前一次性定策略」。实现采用后者：`ConflictMode::Ask` 保留在
   引擎枚举中，但执行期不再询问——执行时遇到扫描后新出现的冲突且模式为 Ask/Skip 时
   按 Skip 记入 errors 汇总。UI 对话框的冲突下拉默认「自动改名」。
2. **目录↔目录冲突恒合并**：顶层源目录与目标目录同名时不应用文件冲突策略、不整删
   目标目录，直接合并进入（递归内部按文件逐项应用策略）。文档 F19 的「覆盖/跳过/
   自动改名」实际只作用于文件冲突。
3. **取消语义细化**：取消发生在某文件写入开始前时不动已存在的同名目标；写入中取消
   才删除半成品。已完整复制/移动的项保留并计入进度。
4. **排序持久化取活动栏**：settings `fm_sort_key`/`fm_sort_asc` 是单值，双栏各自排序
   可能不同，`FileManagerView::snapshot()` 只持久化活动栏的排序（`FmStateSnapshot`
   注释有说明）。
5. **F2 重命名预填无 basename 选区**：重命名对话框预填完整文件名但未能选中
   「不含扩展名的主干部分」——egui 无公开文本选区 API，一期接受全不选中。
6. **设置页默认值与运行态的关系**：`fm_layout`/`fm_dual_ratio` 在运行中实际等价于
   「上次使用的布局」——`maybe_save_fm_state` 在文件管理器视图内每帧 diff 写回
   settings（不自行落盘，退出时统一 `save_settings`）。设置页「文件管理器」tab 改动
   这两个值时经 `apply_layout_settings` 同步到休眠视图，防止下次进入时被写回覆盖。
7. **`fm_preview_open` 的暂存**：`PanelLayout::Dual` 期间单栏预览开关不可见，由
   `FileManagerView::saved_preview_open` 暂存往返（同 `saved_ratio` 模式）。
8. **预览共享层提前实施**：「选中即预览」的字节分类/加载已抽为 `views/preview_bytes.rs`
   （Archive 视图与文件管理器共用）——文档把「预览实现抽取共享」排在二期，实际在一期
   阶段三完成。
9. **Windows 长路径**：manifest 未声明 `longPathAware`，file_ops 所有文件系统调用经
   `verbatim_path` 在 >240 字符时加 `\\?\` 前缀（UNC 转 `\\?\UNC\`）；`trash::delete`
   回收站删除不在此列（trash crate 自管路径）。
10. **剪贴板为应用内剪贴板**：Ctrl+C/X/V 只记录路径列表于会话内存，不写系统剪贴板；
    跨进程复制文件为一期外能力。
11. **未实施条目**：栏间拖放复制（F13）、导航历史 UI、解压到另一栏、FS watch 刷新等
    二期条目，以及驱动器栏/书签、目录大小计算等三期条目，均维持未做。
