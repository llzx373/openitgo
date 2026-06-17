# 漫画阅读器设计文档

- **日期**: 2026-06-17
- **项目**: rustReader
- **技术栈**: Rust + egui
- **状态**: 已确认，待实现计划

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
| 实现方案 | 方案 2：Workspace 多 crate + 同步 I/O |
| 未来规划 | 方案 3：异步后台加载线程池 |

### 2.2 首版功能清单

- [ ] 打开本地漫画：文件夹 / ZIP / RAR / PDF
- [ ] 三种阅读模式：LTR / RTL / Webtoon
- [ ] 缩放：适应高度、适应宽度、整页适配、原始尺寸、自定义缩放
- [ ] 平移：放大时拖拽平移
- [ ] 全屏阅读
- [ ] 翻页：键盘、鼠标点击、滚轮
- [ ] 跳转：页码输入、缩略图栏点击
- [ ] 书架：网格展示、封面缩略图
- [ ] 阅读历史：自动恢复上次阅读位置
- [ ] 书签：添加/删除/跳转
- [ ] 设置持久化：主题、默认模式、窗口状态、快捷键
- [ ] 错误提示：文件打开失败、解码失败等友好弹窗

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
    Bytes(Vec<u8>),
    PdfRef(PdfPageRef),
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
}
```

## 5. 阅读模式与渲染

### 5.1 三种模式布局

| 模式 | 滚动轴 | 翻页方向 | 页面排布 |
|---|---|---|---|
| LTR | 水平 | 左→右 | 单页，从左侧开始；双页模式列入未来规划 |
| RTL | 水平 | 右→左 | 单页，从右侧开始；双页模式列入未来规划 |
| Webtoon | 垂直 | 上→下 | 所有页面纵向无缝拼接 |

### 5.2 渲染流程

1. `Viewport` 维护当前可见区域（逻辑坐标）。
2. `LayoutEngine` 根据 `ReadingMode` 计算每页在逻辑画布上的位置与尺寸。
3. `Renderer` 将逻辑坐标映射到屏幕坐标，仅绘制可见区域内的页面。
4. 解码后的图片以 egui `TextureHandle` 形式缓存在 `TextureCache` 中，按 LRU 策略淘汰。

### 5.3 缩放策略

- 默认：LTR/RTL 使用 `FitMode::Height`，Webtoon 使用 `FitMode::Width`。
- 用户可通过 `Ctrl + 滚轮` 在适配基础上进一步缩放。
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
| `RarParser` | `unrar` crate 或外部 `unrar` | 支持 `.cbr`/`.rar`。 |
| `PdfParser` | `pdf-rs` 或 `mupdf` 绑定 | 提取页面为位图；若库不成熟，首版可降级提示。 |

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
   - 右键菜单：删除记录、在文件夹中显示、刷新缩略图。

2. **阅读器视图（Reader）**
   - 中央：漫画渲染区。
   - 顶部工具栏（可自动隐藏）：模式切换、缩放、页码跳转、全屏。
   - 底部缩略图栏：当前卷所有页面缩略图，点击跳转。
   - 侧边栏（可收起）：目录、书签列表。

3. **设置视图（Settings）**
   - 通用：主题、语言、默认阅读模式、存储位置。
   - 阅读：背景色、翻页动画开关、预加载页数。
   - 快捷键：首版使用默认快捷键，界面预留自定义入口，实际自定义功能列入未来规划。

### 8.2 交互方式

| 操作 | 行为 |
|---|---|
| 左键点击左半边 / 按 ← | LTR 上一页；RTL 下一页 |
| 左键点击右半边 / 按 → | LTR 下一页；RTL 上一页 |
| 滚轮 | LTR/RTL 横向翻页；Webtoon 垂直滚动 |
| 空格 / PgDn | 下一页 / 向下滚动 |
| Esc | 退出全屏 |
| F11 | 切换全屏 |
| Ctrl + O | 打开文件/文件夹 |
| Ctrl + 滚轮 | 缩放 |
| 拖拽 | 放大时平移 |

## 9. 错误处理与测试

### 9.1 错误处理

- 定义统一错误枚举：`FileError`, `ParseError`, `DecodeError`, `IoError`。
- 所有错误最终转换为 `AppError`，通过 egui 弹窗展示。
- 文件打开失败时保留旧状态，不崩溃。

### 9.2 测试策略

- `rust-reader-core`：单元测试覆盖翻页状态机、缩放计算、布局计算。
- `rust-reader-parser`：集成测试，使用最小测试样本（zip/rar/pdf/文件夹）。
- `rust-reader-app`：smoke test 与手动验证为主。

## 10. 未来规划

1. **异步后台加载**：文件解压、图片解码、PDF 渲染放入独立线程池，避免 UI 阻塞。
2. **预加载优化**：当前页前后 N 页提前解码缓存。
3. **Web / WASM 支持**：若后续需要，需将同步 I/O 替换为异步/浏览器 API。
4. **元数据与在线搜索**：漫画信息编辑、封面下载。

## 11. 决策记录

| 决策 | 选项 | 理由 |
|---|---|---|
| 架构 | Workspace 多 crate | 职责清晰，便于测试与扩展 |
| I/O 模型 | 同步 I/O（首版） | 复杂度可控，后续可平滑迁移到异步 |
| UI 框架 | egui | 用户指定，跨平台、即时模式 |
| 持久化格式 | JSON 文件 | 简单、易调试、无需引入数据库 |
| 图片解码 | `image` crate | Rust 生态标准，支持格式广 |
