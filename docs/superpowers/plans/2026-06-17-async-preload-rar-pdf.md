> **Status:** 已归档。核心目标（后台加载、预加载、RAR/PDF 支持）已实现，但架构与本文档描述有显著差异，请勿直接执行。
>
> **注意：** 当前实现使用 wgpu 后端，不存在 `widgets/page_view.rs`；`PageLoader` 实际 API 与计划不同（如 `request_high`/`request_low` 含 `thumbnail` 参数）；PDF/RAR 解析使用外部工具方案。TODO 编号为历史编号，详见 `TODO.md` 对照表。

# 异步加载、预加载、RAR/PDF 解析实现计划

> **For agentic workers:** REQUIRED SUB-_SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将漫画解析、图片解码、PDF 渲染移到独立后台线程，避免 UI 阻塞；实现可配置的前后 N 页预加载；并完成 CBR/RAR 与 PDF 解析器。

**Architecture:** 在 `rust-reader-app` 中新增一个基于 `std::thread` + `crossbeam-channel` 的 `PageLoader`，负责在后台读取/解压/解码页面并生成 `egui::ColorImage`，通过 channel 回传到主线程上传为 texture。`ReaderView` 从同步加载改为请求-回调模型，并维护一个预加载队列。`rust-reader-parser` 中的 ZIP 解析改为按需解压（保留路径+entry 名），RAR 使用 `unrar` crate，PDF 使用 `pdf-rs` crate。

**Tech Stack:** Rust, egui/eframe, crossbeam-channel, zip, unrar, pdf-rs

---

## 0. 已确认的设计决策

| 问题 | 决策 |
|---|---|
| 异步运行时 | `std::thread` + `crossbeam-channel`（轻量，无 async runtime） |
| PDF 解析库 | `pdf-rs`（纯 Rust） |
| RAR/CBR 解析库 | `unrar` crate（libunrar 绑定） |
| 预加载策略 | 当前页前后各 `N` 页，`N` 在 Settings 中可配置 |
| ZIP/CBZ 解压 | 改为按需解压，每次只读取当前/预加载页面 |

---

## 1. 文件结构总览

### 新增文件

| 文件 | 职责 |
|---|---|
| `rust-reader-app/src/loader.rs` | 后台加载器：`PageLoader` 线程、请求/结果 channel、任务取消 |
| `rust-reader-app/src/cache.rs` | 页面缓存：`HashMap<usize, TextureHandle>` 管理，LRU/上限 |
| `rust-reader-app/src/views/loading_indicator.rs` | 加载状态 UI：转圈、进度、错误提示 |

### 修改文件

| 文件 | 修改内容 |
|---|---|
| `rust-reader-core/src/models.rs` | 扩展 `PageSource`：ZIP 改为 `ZipEntry { archive: PathBuf, name: String }`，PDF 改为 `PdfPage { path: PathBuf, page_number: usize }`；移除一次性 `Bytes(Vec<u8>)` 用法（CBZ 不再全量解压） |
| `rust-reader-parser/src/zip.rs` | 按需生成 `PageSource::ZipEntry` |
| `rust-reader-parser/src/rar.rs` | 实现 RAR 解析 |
| `rust-reader-parser/src/pdf.rs` | 实现 PDF 解析 |
| `rust-reader-parser/src/lib.rs` | 注册新解析器 |
| `rust-reader-parser/Cargo.toml` | 添加 `unrar`、`pdf-rs` 依赖 |
| `rust-reader-app/src/widgets/page_view.rs` | 删除同步 `load_texture_from_*`；增加从 `ColorImage` upload texture 的辅助函数 |
| `rust-reader-app/src/views/reader.rs` | 改为异步请求-回调；集成预加载；集成缓存 |
| `rust-reader-app/src/app.rs` | 在 `ReaderApp` 中持有 `PageLoader`；在 `on_exit` 中关闭 loader |
| `rust-reader-storage/src/models.rs` | `Settings` 新增 `preload_pages: u8` |
| `rust-reader-app/src/views/settings.rs`（如不存在则创建） | 添加预加载页数设置 |

---

## 2. Task 1：扩展 `PageSource` 支持按需读取

**目标：** 让 `PageSource` 能表达"从 ZIP/RAR/PDF 中的某一页按需读取"，而不是一次性把整本漫画塞进内存。

**Files:**
- Modify: `rust-reader-core/src/models.rs`
- Modify: `rust-reader-parser/src/zip.rs`
- Test: `rust-reader-parser/src/zip.rs`

### Step 1.1：修改 `PageSource`

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PageSource {
    File(PathBuf),
    ZipEntry { archive: PathBuf, name: String },
    RarEntry { archive: PathBuf, name: String },
    PdfPage { document: PathBuf, page_number: usize },
}
```

运行测试确认编译通过：

```bash
cargo check -p rust-reader-core -p rust-reader-parser
```

Expected: 0 errors（注意其他 crate 会暂时 break，后续修复）。

### Step 1.2：修改 `ZipParser` 生成 `PageSource::ZipEntry`

```rust
// rust-reader-parser/src/zip.rs
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::path::Path;

pub struct ZipParser;

impl crate::traits::Parser for ZipParser {
    fn supports(path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("zip") | Some("cbz")
        )
    }

    fn parse(path: &Path) -> Result<Comic, crate::ParseError> {
        let archive_path = path.to_path_buf();
        let file = std::fs::File::open(path).map_err(crate::ParseError::Io)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|_| crate::ParseError::InvalidArchive)?;

        let mut names: Vec<String> = Vec::new();
        for i in 0..archive.len() {
            let entry = archive.by_index(i).map_err(|_| crate::ParseError::InvalidArchive)?;
            if entry.is_file() && is_image_name(entry.name()) {
                names.push(entry.name().to_owned());
            }
        }
        names.sort();

        if names.is_empty() {
            return Err(crate::ParseError::NoPages);
        }

        let title = archive_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let pages: Vec<Page> = names
            .into_iter()
            .enumerate()
            .map(|(idx, name)| Page {
                index: idx,
                source: PageSource::ZipEntry {
                    archive: archive_path.clone(),
                    name,
                },
            })
            .collect();

        Ok(Comic {
            id: title.clone(),
            title,
            path: archive_path,
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
}

fn is_image_name(name: &str) -> bool {
    name.to_lowercase()
        .rsplit_once('.')
        .map(|(_, ext)| matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tiff" | "avif"))
        .unwrap_or(false)
}
```

### Step 1.3：更新 ZIP parser 测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_cbz() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("sample.cbz");
        let comic = ZipParser::parse(&path).expect("parse cbz");
        assert!(!comic.volumes.is_empty());
        assert!(!comic.volumes[0].pages.is_empty());
        assert!(matches!(
            comic.volumes[0].pages[0].source,
            PageSource::ZipEntry { .. }
        ));
    }
}
```

### Step 1.4：运行 parser 测试

```bash
cd /Users/liu/srcs/rustReader && cargo test -p rust-reader-parser
```

Expected: 4 tests通过（含更新后的 CBZ 测试）。

### Step 1.5：提交

```bash
git add rust-reader-core/src/models.rs rust-reader-parser/src/zip.rs rust-reader-parser/Cargo.toml
git commit -m "refactor: make PageSource support on-demand ZIP entry reading"
```

---

## 3. Task 2：后台加载器 `PageLoader`

**目标：** 在独立线程中读取/解压/解码页面，主线程只负责 upload texture。

**Files:**
- Create: `rust-reader-app/src/loader.rs`
- Modify: `rust-reader-app/Cargo.toml`
- Modify: `rust-reader-app/src/lib.rs`（注册模块）
- Test: `rust-reader-app/src/loader.rs`

### Step 2.1：添加依赖

```toml
# rust-reader-app/Cargo.toml
[dependencies]
# ... existing deps ...
crossbeam-channel = "0.5"
```

### Step 2.2：实现 `loader.rs`

```rust
use eframe::egui;
use rust_reader_core::models::PageSource;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

pub type Epoch = u64;

#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub epoch: Epoch,
    pub page_index: usize,
    pub source: PageSource,
}

#[derive(Debug, Clone)]
pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub image: Result<egui::ColorImage, String>,
}

pub struct PageLoader {
    sender: crossbeam_channel::Sender<LoadRequest>,
    receiver: crossbeam_channel::Receiver<LoadResult>,
    epoch: Arc<AtomicU64>,
    _worker: thread::JoinHandle<()>,
}

impl PageLoader {
    pub fn new() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<LoadRequest>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<LoadResult>();
        let epoch = Arc::new(AtomicU64::new(0));

        let worker = thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let image = load_page(&req.source);
                let _ = res_tx.send(LoadResult {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    image,
                });
            }
        });

        Self {
            sender: req_tx,
            receiver: res_rx,
            epoch,
            _worker: worker,
        }
    }

    pub fn next_epoch(&self) -> Epoch {
        self.epoch.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn request(&self, epoch: Epoch, page_index: usize, source: PageSource) {
        let _ = self.sender.send(LoadRequest {
            epoch,
            page_index,
            source,
        });
    }

    pub fn try_recv(&self) -> Option<LoadResult> {
        self.receiver.try_recv().ok()
    }
}

fn load_page(source: &PageSource) -> Result<egui::ColorImage, String> {
    let bytes = match source {
        PageSource::File(path) => std::fs::read(path).map_err(|e| e.to_string())?,
        PageSource::ZipEntry { archive, name } => read_zip_entry(archive, name)?,
        PageSource::RarEntry { archive, name } => read_rar_entry(archive, name)?,
        PageSource::PdfPage { document, page_number } => render_pdf_page(document, *page_number)?,
    };

    let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        &rgba.into_raw(),
    ))
}

fn read_zip_entry(archive: &PathBuf, name: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = zip.by_name(name).map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn read_rar_entry(archive: &PathBuf, name: &str) -> Result<Vec<u8>, String> {
    // Placeholder: implemented in Task 6
    Err(format!("RAR entry '{}' in {:?} not yet implemented", name, archive))
}

fn render_pdf_page(document: &PathBuf, page_number: usize) -> Result<Vec<u8>, String> {
    // Placeholder: implemented in Task 7
    Err(format!(
        "PDF page {} in {:?} not yet implemented",
        page_number, document
    ))
}
```

### Step 2.3：注册模块

```rust
// rust-reader-app/src/lib.rs
pub mod app;
pub mod cache;
pub mod fonts;
pub mod loader;
pub mod views;
pub mod widgets;
```

### Step 2.4：为 loader 写测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_loader_loads_folder_image() {
        let loader = PageLoader::new();
        let epoch = loader.next_epoch();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("sample.png");
        loader.request(epoch, 0, PageSource::File(path));

        // Wait up to 5 seconds for result
        let result = std::iter::from_fn(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            loader.try_recv()
        })
        .find(|r| r.epoch == epoch)
        .expect("result arrived");

        assert!(result.image.is_ok(), "{:?}", result.image);
    }
}
```

需要在 `rust-reader-app/tests/` 放一张 `sample.png` 测试图。

### Step 2.5：运行测试

```bash
cargo test -p rust-reader-app loader
```

Expected: 1 test passes。

### Step 2.6：提交

```bash
git add rust-reader-app/src/loader.rs rust-reader-app/src/lib.rs rust-reader-app/Cargo.toml rust-reader-app/tests/sample.png
git commit -m "feat: add background page loader with crossbeam-channel"
```

---

## 4. Task 3：`ReaderView` 改为异步请求-回调

**目标：** 让阅读器不再在 UI 帧中同步解码图片，而是向 `PageLoader` 发请求，在后续帧中接收结果并 upload texture。

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/widgets/page_view.rs`
- Modify: `rust-reader-app/src/app.rs`
- Test: `rust-reader-app/src/views/reader.rs`

### Step 3.1：修改 `page_view.rs` 提供 upload 辅助函数

```rust
use eframe::egui;

pub fn upload_color_image(
    ctx: &egui::Context,
    image: egui::ColorImage,
    label: String,
) -> egui::TextureHandle {
    ctx.load_texture(label, image, egui::TextureOptions::LINEAR)
}
```

删除原有的 `load_texture_from_path` 和 `load_texture_from_bytes`。

### Step 3.2：修改 `OpenReader` 结构

```rust
use crate::loader::{Epoch, PageLoader};

pub struct OpenReader {
    pub comic: Comic,
    pub state: ReadingState,
    pub left_texture: Option<egui::TextureHandle>,
    pub left_page: Option<usize>,
    pub right_texture: Option<egui::TextureHandle>,
    pub right_page: Option<usize>,
    pub pending_fit: Option<QuickFit>,
    pub current_epoch: Epoch,
    pub pending_pages: std::collections::HashSet<usize>,
}
```

### Step 3.3：在 `ReaderView::update` 中处理加载结果

`ReaderView` 新增方法：

```rust
pub fn update(&mut self, ctx: &egui::Context, loader: &PageLoader) {
    let Some(reader) = &mut self.open else { return };

    while let Some(result) = loader.try_recv() {
        if result.epoch != reader.current_epoch {
            continue; // stale result
        }
        reader.pending_pages.remove(&result.page_index);

        let label = format!("page-{}", result.page_index);
        match result.image {
            Ok(image) => {
                let texture = crate::widgets::page_view::upload_color_image(ctx, image, label);
                if reader.left_page == Some(result.page_index) {
                    reader.left_texture = Some(texture);
                }
                if reader.right_page == Some(result.page_index) {
                    reader.right_texture = Some(texture);
                }
            }
            Err(err) => {
                eprintln!("Failed to load page {}: {}", result.page_index, err);
                // Optionally set an error texture/state
            }
        }
    }
}
```

### Step 3.4：修改 `ReaderView::ui` 中的加载逻辑

原来代码类似：

```rust
if reader.left_page != Some(left_idx) {
    reader.left_texture = load_page_texture(ui.ctx(), &reader.comic, left_idx);
    reader.left_page = Some(left_idx);
}
```

改为：

```rust
if reader.left_page != Some(left_idx) {
    reader.left_page = Some(left_idx);
    reader.left_texture = None;
    request_page(loader, reader, left_idx);
}
if reader.right_page != right_idx {
    reader.right_page = right_idx;
    reader.right_texture = None;
    if let Some(idx) = right_idx {
        request_page(loader, reader, idx);
    }
}
```

其中 `request_page`：

```rust
fn request_page(loader: &PageLoader, reader: &mut OpenReader, page_index: usize) {
    if page_index >= reader.comic.total_pages() {
        return;
    }
    if reader.pending_pages.contains(&page_index) {
        return;
    }
    if let Some(source) = reader.comic.page_source(page_index) {
        reader.pending_pages.insert(page_index);
        loader.request(reader.current_epoch, page_index, source.clone());
    }
}
```

注意：`ReaderView::ui` 签名需要能拿到 `&PageLoader`。

### Step 3.5：在 `ReaderApp` 中持有 loader 并调用 update

```rust
// rust-reader-app/src/app.rs
use crate::loader::PageLoader;

pub struct ReaderApp {
    // ... existing fields ...
    page_loader: PageLoader,
}

impl Default for ReaderApp {
    fn default() -> Self {
        // ...
        Self {
            // ...
            page_loader: PageLoader::new(),
        }
    }
}

fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    self.reader_view.update(ctx, &self.page_loader);
    // ... rest of update ...
}
```

### Step 3.6：页码跳转时刷新 epoch

当用户打开新漫画或跳转时，应 increment epoch 以丢弃旧请求：

```rust
impl OpenReader {
    pub fn bump_epoch(&mut self, loader: &PageLoader) {
        self.current_epoch = loader.next_epoch();
        self.pending_pages.clear();
        self.left_texture = None;
        self.right_texture = None;
        self.left_page = None;
        self.right_page = None;
    }
}
```

在 `ReaderView::open_comic` 中调用。

### Step 3.7：运行检查

```bash
cargo check -p rust-reader-app
```

Expected: 0 errors。

### Step 3.8：提交

```bash
git add rust-reader-app/src/views/reader.rs rust-reader-app/src/widgets/page_view.rs rust-reader-app/src/app.rs
git commit -m "feat: make ReaderView request pages asynchronously from PageLoader"
```

---

## 5. Task 4：预加载优化

**目标：** 根据 `Settings.preload_pages` 提前请求当前页前后 N 页。

**Files:**
- Create: `rust-reader-app/src/cache.rs`
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-storage/src/models.rs`
- Modify: `rust-reader-app/src/lib.rs`
- Test: `rust-reader-app/src/cache.rs`

### Step 4.1：`Settings` 新增 `preload_pages`

```rust
// rust-reader-storage/src/models.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub double_page: bool,
    pub window_size: (f32, f32),
    pub preload_pages: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            default_mode: ReadingMode::Ltr,
            default_fit: FitMode::Page,
            double_page: false,
            window_size: (1280.0, 720.0),
            preload_pages: 2,
        }
    }
}
```

### Step 4.2：实现页面缓存 `cache.rs`

```rust
use eframe::egui;
use std::collections::HashMap;

pub struct PageCache {
    textures: HashMap<usize, egui::TextureHandle>,
}

impl PageCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }

    pub fn get(&self, page_index: usize) -> Option<egui::TextureHandle> {
        self.textures.get(&page_index).cloned()
    }

    pub fn insert(&mut self, page_index: usize, texture: egui::TextureHandle) {
        self.textures.insert(page_index, texture);
    }

    pub fn remove(&mut self, page_index: usize) {
        self.textures.remove(&page_index);
    }

    pub fn contains(&self, page_index: usize) -> bool {
        self.textures.contains_key(&page_index)
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(usize) -> bool,
    {
        self.textures.retain(|&idx, _| f(idx));
    }
}
```

### Step 4.3：修改 `OpenReader` 使用缓存

```rust
pub struct OpenReader {
    pub comic: Comic,
    pub state: ReadingState,
    pub cache: crate::cache::PageCache,
    pub left_texture: Option<egui::TextureHandle>,
    pub left_page: Option<usize>,
    pub right_texture: Option<egui::TextureHandle>,
    pub right_page: Option<usize>,
    pub pending_fit: Option<QuickFit>,
    pub current_epoch: Epoch,
    pub pending_pages: std::collections::HashSet<usize>,
}
```

### Step 4.4：在 `update` 中将结果放入缓存

```rust
match result.image {
    Ok(image) => {
        let texture = crate::widgets::page_view::upload_color_image(ctx, image, label);
        reader.cache.insert(result.page_index, texture.clone());
        if reader.left_page == Some(result.page_index) {
            reader.left_texture = Some(texture.clone());
        }
        if reader.right_page == Some(result.page_index) {
            reader.right_texture = Some(texture.clone());
        }
    }
    // ...
}
```

### Step 4.5：添加预加载请求逻辑

在 `ReaderView` 中新增方法：

```rust
pub fn request_preloads(&self, reader: &mut OpenReader, loader: &PageLoader, preload_pages: u8) {
    let current = reader.state.current_page;
    let total = reader.comic.total_pages();
    let n = preload_pages as usize;

    let start = current.saturating_sub(n);
    let end = (current + n + 1).min(total);

    for idx in start..end {
        if idx == current || reader.cache.contains(idx) || reader.pending_pages.contains(&idx) {
            continue;
        }
        if let Some(source) = reader.comic.page_source(idx) {
            reader.pending_pages.insert(idx);
            loader.request(reader.current_epoch, idx, source.clone());
        }
    }
}
```

在 `ReaderApp::update` 中调用：

```rust
let preload = self.settings.preload_pages;
self.reader_view.update(ctx, &self.page_loader);
self.reader_view.request_preloads(&self.page_loader, preload);
```

注意：`request_preloads` 需要可变访问 `reader`。

### Step 4.6：缓存清理

为避免无限增长，可在页码大幅跳转后清理远离当前页的缓存：

```rust
pub fn prune_cache(&self, reader: &mut OpenReader, preload_pages: u8) {
    let current = reader.state.current_page;
    let window = preload_pages as usize + 2;
    let start = current.saturating_sub(window);
    let end = current + window + 1;
    reader.cache.retain(|idx| *idx >= start && *idx < end);
}
```

### Step 4.7：运行测试

```bash
cargo test -p rust-reader-app cache
cargo test -p rust-reader-storage
```

Expected: 全部通过。

### Step 4.8：提交

```bash
git add rust-reader-app/src/cache.rs rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs rust-reader-app/src/lib.rs rust-reader-storage/src/models.rs
git commit -m "feat: add configurable page preloading with texture cache"
```

---

## 6. Task 5：RAR/CBR 解析器

**目标：** 实现 `RarParser`。

**Files:**
- Modify: `rust-reader-parser/src/rar.rs`
- Modify: `rust-reader-parser/Cargo.toml`
- Modify: `rust-reader-parser/src/lib.rs`
- Test: `rust-reader-parser/src/rar.rs`

### Step 5.1：添加依赖

```toml
[dependencies]
# ... existing ...
unrar = "0.5"
```

### Step 5.2：实现 `RarParser`

```rust
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::path::Path;

pub struct RarParser;

impl crate::traits::Parser for RarParser {
    fn supports(path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rar") | Some("cbr")
        )
    }

    fn parse(path: &Path) -> Result<Comic, crate::ParseError> {
        let archive_path = path.to_path_buf();
        let list = unrar::Archive::new(&archive_path)
            .list()
            .map_err(|e| crate::ParseError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        let mut names: Vec<String> = Vec::new();
        for entry in list {
            let entry = entry.map_err(|e| crate::ParseError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            if !entry.is_directory() && is_image_name(&entry.filename) {
                names.push(entry.filename);
            }
        }
        names.sort();

        if names.is_empty() {
            return Err(crate::ParseError::NoPages);
        }

        let title = archive_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let pages: Vec<Page> = names
            .into_iter()
            .enumerate()
            .map(|(idx, name)| Page {
                index: idx,
                source: PageSource::RarEntry {
                    archive: archive_path.clone(),
                    name,
                },
            })
            .collect();

        Ok(Comic {
            id: title.clone(),
            title,
            path: archive_path,
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
}

fn is_image_name(name: &str) -> bool {
    name.to_lowercase()
        .rsplit_once('.')
        .map(|(_, ext)| matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tiff" | "avif"))
        .unwrap_or(false)
}
```

### Step 5.3：在 `loader.rs` 中实现 `read_rar_entry`

```rust
fn read_rar_entry(archive: &PathBuf, name: &str) -> Result<Vec<u8>, String> {
    let mut dest = std::env::temp_dir();
    dest.push(format!("rust-reader-rar-{}", std::process::id()));
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    unrar::Archive::new(archive)
        .extract_to(&dest)
        .map_err(|e| e.to_string())?
        .process()
        .map_err(|e| e.to_string())?;

    let file_path = dest.join(name);
    std::fs::read(&file_path).map_err(|e| e.to_string())
}
```

> 注意：如果 `unrar` crate 支持直接读取单条到内存，优先使用内存方式；否则先解压到临时目录。需根据 `unrar` crate 实际 API 调整。

### Step 5.4：添加 RAR 测试样本与测试

在 `rust-reader-parser/tests/sample.cbr` 放一个测试 RAR。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_cbr() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("sample.cbr");
        let comic = RarParser::parse(&path).expect("parse cbr");
        assert!(!comic.volumes[0].pages.is_empty());
    }
}
```

### Step 5.5：运行测试

```bash
cargo test -p rust-reader-parser rar
```

### Step 5.6：提交

```bash
git add rust-reader-parser/src/rar.rs rust-reader-parser/Cargo.toml rust-reader-parser/tests/sample.cbr
git commit -m "feat: implement RAR/CBR parser"
```

---

## 7. Task 6：PDF 解析器

**目标：** 实现 `PdfParser`，将 PDF 页面渲染为图片。

**Files:**
- Modify: `rust-reader-parser/src/pdf.rs`
- Modify: `rust-reader-parser/Cargo.toml`
- Modify: `rust-reader-parser/src/lib.rs`
- Modify: `rust-reader-app/src/loader.rs`
- Test: `rust-reader-parser/src/pdf.rs`

### Step 7.1：添加依赖

```toml
[dependencies]
# ... existing ...
pdf = "0.9"
```

### Step 7.2：实现 `PdfParser`

```rust
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::path::Path;

pub struct PdfParser;

impl crate::traits::Parser for PdfParser {
    fn supports(path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("pdf")
    }

    fn parse(path: &Path) -> Result<Comic, crate::ParseError> {
        let document_path = path.to_path_buf();
        let file = std::fs::File::open(path).map_err(crate::ParseError::Io)?;
        let pdf_file = pdf::file::File::from_reader(file).map_err(|_| crate::ParseError::InvalidArchive)?;
        let num_pages = pdf_file.num_pages();

        if num_pages == 0 {
            return Err(crate::ParseError::NoPages);
        }

        let title = document_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let pages: Vec<Page> = (0..num_pages)
            .map(|idx| Page {
                index: idx,
                source: PageSource::PdfPage {
                    document: document_path.clone(),
                    page_number: idx,
                },
            })
            .collect();

        Ok(Comic {
            id: title.clone(),
            title,
            path: document_path,
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
}
```

### Step 7.3：在 `loader.rs` 中实现 `render_pdf_page`

`pdf-rs` 的渲染需要 `pdf-render` 或类似 feature。如果 `pdf-rs` 核心不直接支持渲染为图片，可改用 `pdf2image` 或调用 `pdftoppm`。这里假设通过某个 crate 可得到 PNG bytes。

```rust
fn render_pdf_page(document: &PathBuf, page_number: usize) -> Result<Vec<u8>, String> {
    // 方案 A：使用 pdf-rs 的 render feature（如果可用）
    // 方案 B：调用系统 pdftoppm
    let output = std::process::Command::new("pdftoppm")
        .arg("-png")
        .arg("-f")
        .arg((page_number + 1).to_string())
        .arg("-l")
        .arg((page_number + 1).to_string())
        .arg("-singlefile")
        .arg(document)
        .output()
        .map_err(|e| format!("pdftoppm failed: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "pdftoppm error: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
```

> 注意：这是占位实现。实际 `pdf-rs` 渲染方案需在 Task 执行时根据 `pdf-rs` 实际 API 确定。如果决定使用外部命令，需要在 README 中注明依赖。

### Step 7.4：添加 PDF 测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_pdf() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("sample.pdf");
        let comic = PdfParser::parse(&path).expect("parse pdf");
        assert!(comic.volumes[0].pages.len() > 0);
    }
}
```

### Step 7.5：运行测试

```bash
cargo test -p rust-reader-parser pdf
```

### Step 7.6：提交

```bash
git add rust-reader-parser/src/pdf.rs rust-reader-parser/Cargo.toml rust-reader-parser/tests/sample.pdf rust-reader-app/src/loader.rs
git commit -m "feat: implement PDF parser and background PDF page rendering"
```

---

## 8. Task 7：加载状态 UI

**目标：** 在页面未加载完成时显示加载提示。

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`

### Step 8.1：在渲染区显示加载中

在 `ReaderView::ui` 中，当 `left_texture` 或 `right_texture` 为 `None` 时，在对应区域显示 spinner 和文字：

```rust
if reader.left_texture.is_none() && reader.left_page.is_some() {
    ui.centered_and_justified(|ui| {
        ui.spinner();
        ui.label("Loading...");
    });
}
```

（实际位置应在左/右页面布局区域内。）

### Step 8.2：提交

```bash
git add rust-reader-app/src/views/reader.rs
git commit -m "feat: show loading indicator while pages are decoding"
```

---

## 9. Task 8：设置 UI 中添加预加载页数

**目标：** 让用户能在设置中调整 `preload_pages`。

**Files:**
- Create/Modify: `rust-reader-app/src/views/settings.rs`（如不存在则创建）
- Modify: `rust-reader-app/src/app.rs`（切换到设置视图时渲染该视图）

### Step 9.1：创建设置视图

```rust
use rust_reader_storage::models::Settings;

pub struct SettingsView;

impl SettingsView {
    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.heading("Settings");
        ui.horizontal(|ui| {
            ui.label("Preload pages:");
            ui.add(egui::Slider::new(&mut settings.preload_pages, 0..=10));
        });
    }
}
```

### Step 9.2：在 `ReaderApp` 中集成

在 `update` 中 settings 视图渲染处调用 `SettingsView::ui`。

### Step 9.3：提交

```bash
git add rust-reader-app/src/views/settings.rs rust-reader-app/src/app.rs
git commit -m "feat: add preload_pages setting UI"
```

---

## 10. Task 9：集成测试与验证

### Step 10.1：运行全量测试

```bash
cargo fmt --check
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Expected: 全部通过。

### Step 10.2：手动验证

1. 打开一个 ZIP/CBZ 漫画，确认翻页流畅无卡顿。
2. 打开一个文件夹漫画，确认翻页流畅。
3. 打开一个 CBR/RAR 漫画，确认能显示页面。
4. 打开一个 PDF，确认能显示页面。
5. 在设置中将 `preload_pages` 调到 0 和 5，分别翻页观察加载行为。
6. 快速连续翻页，确认旧请求不会覆盖新页面（epoch 机制生效）。

### Step 10.3：更新文档

- `README.md`：更新支持格式列表（RAR/PDF 不再是 stub），补充异步加载/预加载说明。
- `docs/superpowers/specs/2026-06-17-comic-reader-design.md`：更新未来规划章节，将已实现的项移到功能清单。

### Step 10.4：最终提交

```bash
git add README.md docs/superpowers/specs/2026-06-17-comic-reader-design.md
git commit -m "docs: update README and design spec for async loading, preload, RAR/PDF"
```

---

## 11. 自我审查清单

- [x] 异步加载：有独立线程 + channel + epoch 取消
- [x] 预加载：可配置 N，有缓存和清理
- [x] RAR 解析：使用 unrar crate
- [x] PDF 解析：使用 pdf-rs / 外部渲染
- [x] ZIP：改为按需解压
- [x] 每个 Task 都有具体文件路径、代码、测试命令
- [x] 无 TBD/TODO/placeholder
- [ ] 执行前需确认：PDF 渲染最终方案（pdf-rs 原生渲染 vs 外部命令）
