# 漫画阅读器设计文档

- **日期**: 2026-06-17
- **项目**: rustReader
- **技术栈**: Rust + egui
- **状态**: 已实现

## 1. 背景与目标

构建一款跨平台桌面漫画阅读器，支持三种主流阅读方向：

- **国漫**: 从左到右（LTR）翻页
- **日漫**: 从右到左（RTL）翻页
- **韩漫/Webtoon**: 长条从上到下垂直滚动

首版目标为"标准体验"：打开漫画、三种阅读模式、缩放/平移/全屏、书架、阅读历史、书签、缩略图导航、键盘快捷键、设置持久化。

## 2. 需求摘要

### 2.1 已确认需求

| 维度 | 选择 |
|---|---|
| 输入格式 | 本地图片文件夹、CBZ/ZIP、CBR/RAR、PDF |
| 目标平台 | Windows / macOS / Linux |
| 功能范围 | 标准体验（书架、历史、书签、缩略图导航等） |
| 图片格式 | `image` crate 支持的所有格式（JPG/PNG/WebP/GIF/AVIF 等），开启全 features |
| 实现方案 | 方案 3：Workspace 多 crate + 异步后台加载（单工作线程 + channel） |

### 2.2 首版功能清单

- [x] 打开本地漫画：文件夹 / ZIP / RAR / PDF
- [x] 三种阅读模式：LTR / RTL / Webtoon
- [x] 缩放：适应高度、适应宽度、整页适配、原始尺寸、自定义缩放
- [x] 平移：放大时拖拽平移
- [x] 全屏阅读
- [x] 翻页：键盘、鼠标点击、滚轮
- [x] 跳转：页码输入、缩略图栏点击
- [x] 书架：网格展示、封面缩略图
- [x] 阅读历史：自动恢复上次阅读位置
- [x] 书签：添加/删除/跳转
- [x] 设置持久化：主题、默认模式、窗口状态、快捷键
- [x] 错误提示：文件打开失败、解码失败等友好弹窗
- [x] 异步后台加载
- [x] 预加载优化
- [x] CBR/RAR 完整解析
- [x] PDF 完整解析

## 3. 架构设计

采用 Cargo Workspace，拆分为 4 个 crate：

```text
rustReader/
├── rust-reader-core      # 领域模型、阅读状态机、布局计算
├── rust-reader-parser    # 文件/压缩包/PDF 解析
├── rust-reader-storage   # 配置、历史、书签、书架元数据持久化
└── rust-reader-app       # egui UI、主循环、事件路由
```

### 3.1 依赖关系

```text
rust-reader-app
    ├── rust-reader-core
    ├── rust-reader-parser
    └── rust-reader-storage

rust-reader-parser ──► rust-reader-core
rust-reader-storage ──► rust-reader-core
```

`parser` 与 `storage` 互不依赖，边界清晰。

## 4. 核心数据模型

```rust
pub struct Comic {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub volumes: Vec<Volume>,
}

pub struct Volume {
    pub title: String,
    pub pages: Vec<Page>,
}

pub struct Page {
    pub index: usize,
    pub source: PageSource,
}

pub enum PageSource {
    File(PathBuf),
    ZipEntry { archive: PathBuf, name: String },
    RarEntry { archive: PathBuf, name: String },
    PdfPage { document: PathBuf, page_number: usize },
}

pub enum ReadingMode {
    Ltr,     // 国漫：左→右
    Rtl,     // 日漫：右→左
    Webtoon, // 韩漫：长条从上到下
}

pub enum FitMode {
    Height,   // 高度撑满
    Width,    // 宽度撑满
    Page,     // 整页适配
    Original, // 原始尺寸
}

pub struct ReadingState {
    pub mode: ReadingMode,
    pub current_page: usize,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit_mode: FitMode,
    pub double_page: bool, // 仅对 LTR/RTL 有效
}
```

## 5. 阅读模式与渲染

### 5.1 三种模式布局

| 模式 | 滚动轴 | 翻页方向 | 页面排布 |
|---|---|---|---|
| LTR | 水平 | 左→右 | 单页或双页跨页，从左侧开始 |
| RTL | 水平 | 右→左 | 单页或双页跨页，从右侧开始 |
| Webtoon | 垂直 | 上→下 | 所有页面纵向无缝拼接 |

### 5.2 渲染流程

1. `ReaderView` 维护当前可见的左/右页面纹理句柄。
2. 当显示页索引发生变化时，`ReaderView::ui` 向 `PageLoader` 提交**高优先级**解码请求；当前页会尽快被后台线程处理并回传。
3. `ReaderView::request_preloads` 以当前页为中心向外提交**低优先级**解码请求，结果存入 `PageCache`。
4. `PageLoader` 使用两条 channel：高优先级队列给当前页，低优先级队列给预加载页。工作线程始终先 drain 高优先级请求，再处理低优先级请求。
5. 解码后的图片以 egui `TextureHandle` 形式缓存在 `PageCache` 中；`PageCache` 按内存预算（100 MB - 4 GB，默认 1 GB）维护，使用 LRU 策略淘汰最少访问的页面。
6. 页面未加载完成时，渲染区显示 spinner + "加载中..." 占位符。

### 5.3 缩放策略

- **适应宽度**：图片宽度撑满窗口。
- **适应高度**：图片高度撑满窗口。
- **自动适应**：整张图片完整显示在窗口内（默认）。
- **原始尺寸**：按图片原始像素尺寸 1:1 显示。
- **+ / - 按钮**：在适配基础上进一步缩放。
- 放大后支持拖拽平移。

## 6. 文件解析层

统一接口：

```rust
pub trait Parser: Send + Sync {
    fn supports(path: &Path) -> bool;
    fn parse(path: &Path) -> Result<Comic, ParseError>;
}
```

| 解析器 | 依赖 | 说明 |
|---|---|---|
| `FolderParser` | `std::fs`, `image` | 按文件名排序读取图片。 |
| `ZipParser` | `zip` crate | 支持 `.cbz`/`.zip`。 |
| `RarParser` | `unrar` crate | 支持 `.cbr`/`.rar`。 |
| `PdfParser` | `pdf-rs` 或 `mupdf` 绑定 | 提取页面为位图，首版已完整支持。 |

解析结果仅保留页面引用，不一次性解码所有图片。

## 7. 持久化层

默认存储在用户配置目录 `dirs::config_dir()/rust-reader/` 下：

| 文件 | 内容 |
|---|---|
| `settings.json` | 主题、默认阅读模式、默认缩放、窗口大小、快捷键 |
| `library.json` | 书架列表（路径、标题、封面缩略图路径） |
| `history.json` | 每本漫画最后阅读的卷和页码、阅读时间 |
| `bookmarks.json` | 每本漫画的书签列表（卷、页、备注） |

使用 `serde` + `serde_json` 序列化。

## 8. UI 设计

### 8.1 三大视图

1. **书架视图（Library）**
   - 网格展示漫画封面缩略图。
   - 点击打开并恢复阅读进度。
   - 支持 Ctrl+O 或「打开文件夹」按钮添加漫画。

2. **阅读器视图（Reader）**
   - 中央：漫画渲染区（单页或双页跨页）。
   - 顶部工具栏：返回书架、模式切换、单/双页切换、缩放、页码跳转、添加书签、全屏、设置。
   - 底部状态栏：页码、阅读模式、缩放比例、单/双页状态、快捷键提示。
   - 底部页面导航条：当前卷所有页面缩略图，点击跳转。
   - 右键菜单：下一页、上一页、首页、末页、添加书签、全屏、返回书架。

3. **设置视图（Settings）**
   - 通用：主题、默认阅读模式、窗口状态。
   - 阅读：缓存大小（100 MB - 4 GB，默认 1 GB）。
   - 快捷键：首版使用默认快捷键，界面预留自定义入口。

### 8.2 交互方式

| 操作 | 行为 |
|---|---|
| 拖拽文件/文件夹到窗口 | 打开漫画 |
| 按 ← | LTR 上一页；RTL 下一页 |
| 按 → | LTR 下一页；RTL 上一页 |
| 滚轮 | LTR/RTL 横向翻页；Webtoon 垂直滚动 |
| 空格 / PgDn | 下一页 / 向下滚动 |
| Esc | 退出全屏或返回书架 |
| F11 / F | 切换全屏 |
| Ctrl + O | 打开文件夹并加入书架 |
| +/- / 0 | 缩放 / 自动适应 |
| 拖拽 | 放大时平移 |
| 右键 | 页面上下文菜单 |
| 双击 | 切换全屏 |

## 9. 错误处理与测试

### 9.1 错误处理

- 定义统一错误枚举：`FileError`, `ParseError`, `DecodeError`, `IoError`。
- 所有错误最终转换为 `AppError`，通过 egui 弹窗展示。
- 文件打开失败时保留旧状态，不崩溃。

### 9.2 测试策略

- `rust-reader-core`：单元测试覆盖翻页状态机、缩放计算、布局计算。
- `rust-reader-parser`：集成测试，使用最小测试样本（zip/rar/pdf/文件夹）。
- `rust-reader-app`：smoke test 与手动验证为主。

## 10. 规划

### 10.1 已实现

- [x] **异步后台加载**：文件解压、图片解码、PDF 渲染通过 channel 提交到单后台工作线程，避免 UI 阻塞。
- [x] **当前页优先加载**：当前显示页使用高优先级 channel，预加载页使用低优先级 channel，确保翻页时当前页尽快显示。
- [x] **基于大小的 LRU 缓存/预加载**：按内存预算（默认 1 GB，可调 100 MB - 4 GB）缓存解码后的页面，并自动淘汰最少使用的页面。
- [x] **CBR/RAR 与 PDF 完整解析**：首版已完整支持 `.cbr`/`.rar` 和 `.pdf`。
- [x] **双页模式**：国漫/日漫支持两页并排显示。

### 10.2 未来规划

1. **Web / WASM 支持**：若后续需要，需将同步 I/O 替换为异步/浏览器 API。
2. **元数据与在线搜索**：漫画信息编辑、封面下载。

## 11. 决策记录

| 决策 | 选项 | 理由 |
|---|---|---|
| 架构 | Workspace 多 crate | 职责清晰，便于测试与扩展 |
| I/O 模型 | 异步后台加载（单工作线程 + 双 channel） | 避免 UI 阻塞；高优先级 channel 保证当前页优先解码，低优先级 channel 用于背景预加载 |
| UI 框架 | egui | 用户指定，跨平台、即时模式 |
| 持久化格式 | JSON 文件 | 简单、易调试、无需引入数据库 |
| 图片解码 | `image` crate | Rust 生态标准，支持格式广 |
