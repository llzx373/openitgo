> **Status:** 已归档。本文档是最初的实现计划，大量细节已被后续重构覆盖，请勿直接执行。
>
> **注意：** 本文档中的 TODO 编号（如 #14-#25）为历史编号，当前对应关系请参见根目录 `TODO.md` 中的「历史 TODO 编号对照表」。计划中的文件结构、依赖版本、API 签名等与当前代码存在较大差异（例如 `rust-reader-app/src/widgets/page_view.rs` 已删除，GPU 路径已演进为 wgpu）。

# Comic Reader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使用 Rust + egui 构建一款支持 LTR/RTL/Webtoon 三种阅读模式的跨平台桌面漫画阅读器。

**Architecture:** 采用 Cargo Workspace 拆分为 core/parser/storage/app 四个 crate，core 负责领域模型与阅读状态机，parser 负责文件解析，storage 负责 JSON 持久化，app 负责 egui UI。

**Tech Stack:** Rust, egui, eframe, image, zip, serde, serde_json, dirs

---

## File Structure

```text
rustReader/
├── Cargo.toml
├── README.md
├── rust-reader-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── models.rs
│       ├── state.rs
│       └── layout.rs
├── rust-reader-parser/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── traits.rs
│       ├── folder.rs
│       ├── zip.rs
│       ├── rar.rs
│       └── pdf.rs
├── rust-reader-storage/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── models.rs
│       └── json_store.rs
└── rust-reader-app/
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── app.rs
        ├── views/
        │   ├── library.rs
        │   ├── reader.rs
        │   └── settings.rs
        └── widgets/
            ├── thumbnail_bar.rs
            └── page_view.rs
```

---

## Dependency Versions

```toml
# workspace Cargo.toml
[workspace]
members = ["rust-reader-core", "rust-reader-parser", "rust-reader-storage", "rust-reader-app"]
resolver = "2"

[workspace.dependencies]
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "webp", "gif", "avif"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
dirs = "5.0"
thiserror = "1.0"
```

```toml
# rust-reader-app/Cargo.toml
[package]
name = "rust-reader-app"
version = "0.1.0"
edition = "2021"

[dependencies]
eframe = { version = "0.29", features = ["default"] }
egui = "0.29"
rust-reader-core = { path = "../rust-reader-core" }
rust-reader-parser = { path = "../rust-reader-parser" }
rust-reader-storage = { path = "../rust-reader-storage" }
```

---

## Task 1: Initialize Cargo Workspace

**Files:**
- Create: `Cargo.toml`
- Create: `rust-reader-core/Cargo.toml`
- Create: `rust-reader-core/src/lib.rs`
- Create: `rust-reader-parser/Cargo.toml`
- Create: `rust-reader-parser/src/lib.rs`
- Create: `rust-reader-storage/Cargo.toml`
- Create: `rust-reader-storage/src/lib.rs`
- Create: `rust-reader-app/Cargo.toml`
- Create: `rust-reader-app/src/main.rs`

- [ ] **Step 1: Write workspace root Cargo.toml**

```toml
[workspace]
members = ["rust-reader-core", "rust-reader-parser", "rust-reader-storage", "rust-reader-app"]
resolver = "2"

[workspace.dependencies]
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "webp", "gif", "avif"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
dirs = "5.0"
thiserror = "1.0"
```

- [ ] **Step 2: Create rust-reader-core crate**

`rust-reader-core/Cargo.toml`:
```toml
[package]
name = "rust-reader-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
```

`rust-reader-core/src/lib.rs`:
```rust
pub mod models;
pub mod state;
pub mod layout;
```

- [ ] **Step 3: Create rust-reader-parser crate**

`rust-reader-parser/Cargo.toml`:
```toml
[package]
name = "rust-reader-parser"
version = "0.1.0"
edition = "2021"

[dependencies]
rust-reader-core = { path = "../rust-reader-core" }
image = { workspace = true }
zip = { version = "2.2", default-features = false, features = ["deflate"] }
thiserror = { workspace = true }
```

`rust-reader-parser/src/lib.rs`:
```rust
pub mod traits;
pub mod folder;
pub mod zip;
pub mod rar;
pub mod pdf;
```

- [ ] **Step 4: Create rust-reader-storage crate**

`rust-reader-storage/Cargo.toml`:
```toml
[package]
name = "rust-reader-storage"
version = "0.1.0"
edition = "2021"

[dependencies]
rust-reader-core = { path = "../rust-reader-core" }
serde = { workspace = true }
serde_json = { workspace = true }
dirs = { workspace = true }
thiserror = { workspace = true }
```

`rust-reader-storage/src/lib.rs`:
```rust
pub mod models;
pub mod json_store;
```

- [ ] **Step 5: Create rust-reader-app crate**

`rust-reader-app/Cargo.toml`:
```toml
[package]
name = "rust-reader-app"
version = "0.1.0"
edition = "2021"

[dependencies]
eframe = { version = "0.29", features = ["default"] }
egui = "0.29"
rust-reader-core = { path = "../rust-reader-core" }
rust-reader-parser = { path = "../rust-reader-parser" }
rust-reader-storage = { path = "../rust-reader-storage" }
```

`rust-reader-app/src/main.rs`:
```rust
fn main() {
    println!("rust-reader-app ready");
}
```

- [ ] **Step 6: Build workspace**

Run:
```bash
cargo check
```

Expected: Successful compilation of empty crates.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml rust-reader-core rust-reader-parser rust-reader-storage rust-reader-app
git commit -m "chore: initialize cargo workspace with four crates"
```

---

## Task 2: Core Domain Models

**Files:**
- Create: `rust-reader-core/src/models.rs`
- Test: `rust-reader-core/src/models.rs`

- [ ] **Step 1: Write models module with tests**

`rust-reader-core/src/models.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Comic {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub volumes: Vec<Volume>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Volume {
    pub title: String,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub index: usize,
    pub source: PageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PageSource {
    File(PathBuf),
    Bytes(Vec<u8>),
    PdfRef { document_path: PathBuf, page_number: usize },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReadingMode {
    Ltr,
    Rtl,
    Webtoon,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum FitMode {
    Height,
    Width,
    Page,
    Original,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_source_file() {
        let page = Page {
            index: 0,
            source: PageSource::File(PathBuf::from("page.png")),
        };
        assert!(matches!(page.source, PageSource::File(_)));
    }

    #[test]
    fn test_reading_mode_serialize() {
        let mode = ReadingMode::Rtl;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"Rtl\"");
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p rust-reader-core
```

Expected: Tests pass.

- [ ] **Step 3: Commit**

```bash
git add rust-reader-core/src/models.rs
git commit -m "feat(core): add domain models with serde support"
```

---

## Task 3: Reading State Machine

**Files:**
- Create: `rust-reader-core/src/state.rs`
- Test: `rust-reader-core/src/state.rs`

- [ ] **Step 1: Write ReadingState with navigation logic and tests**

`rust-reader-core/src/state.rs`:
```rust
use crate::models::{FitMode, ReadingMode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadingState {
    pub mode: ReadingMode,
    pub current_page: usize,
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub fit_mode: FitMode,
}

impl ReadingState {
    pub fn new(mode: ReadingMode, total_pages: usize) -> Self {
        Self {
            mode,
            current_page: 0,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            fit_mode: default_fit_mode(mode),
        }
    }

    pub fn next_page(&mut self, total_pages: usize) {
        if self.current_page + 1 < total_pages {
            self.current_page += 1;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn prev_page(&mut self) {
        if self.current_page > 0 {
            self.current_page -= 1;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn go_to_page(&mut self, page: usize, total_pages: usize) {
        if page < total_pages {
            self.current_page = page;
            self.pan = egui::Vec2::ZERO;
        }
    }

    pub fn set_mode(&mut self, mode: ReadingMode, total_pages: usize) {
        self.mode = mode;
        self.fit_mode = default_fit_mode(mode);
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        if self.current_page >= total_pages && total_pages > 0 {
            self.current_page = total_pages - 1;
        }
    }
}

fn default_fit_mode(mode: ReadingMode) -> FitMode {
    match mode {
        ReadingMode::Ltr | ReadingMode::Rtl => FitMode::Height,
        ReadingMode::Webtoon => FitMode::Width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReadingMode;

    #[test]
    fn test_next_page_stops_at_end() {
        let mut state = ReadingState::new(ReadingMode::Ltr, 3);
        state.next_page(3);
        state.next_page(3);
        state.next_page(3);
        assert_eq!(state.current_page, 2);
    }

    #[test]
    fn test_prev_page_stops_at_start() {
        let mut state = ReadingState::new(ReadingMode::Rtl, 3);
        state.prev_page();
        assert_eq!(state.current_page, 0);
    }

    #[test]
    fn test_go_to_page_clamps() {
        let mut state = ReadingState::new(ReadingMode::Webtoon, 5);
        state.go_to_page(10, 5);
        assert_eq!(state.current_page, 0);
        state.go_to_page(2, 5);
        assert_eq!(state.current_page, 2);
    }
}
```

- [ ] **Step 2: Add egui dependency to core**

`rust-reader-core/Cargo.toml`:
```toml
[dependencies]
serde = { workspace = true }
egui = "0.29"
```

- [ ] **Step 3: Run tests**

Run:
```bash
cargo test -p rust-reader-core
```

Expected: Tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-core/src/state.rs rust-reader-core/Cargo.toml
git commit -m "feat(core): add reading state machine with navigation"
```

---

## Task 4: Layout Engine

**Files:**
- Create: `rust-reader-core/src/layout.rs`
- Test: `rust-reader-core/src/layout.rs`

- [ ] **Step 1: Write layout calculations and tests**

`rust-reader-core/src/layout.rs`:
```rust
use crate::models::{FitMode, ReadingMode};
use egui::{Rect, Vec2};

pub struct PageLayout {
    pub rect: Rect,
    pub page_index: usize,
}

pub fn compute_layout(
    mode: ReadingMode,
    viewport_size: Vec2,
    page_sizes: &[Vec2],
    zoom: f32,
) -> Vec<PageLayout> {
    let mut layouts = Vec::new();
    match mode {
        ReadingMode::Ltr | ReadingMode::Rtl => {
            let mut cursor = 0.0;
            let direction = if matches!(mode, ReadingMode::Ltr) { 1.0 } else { -1.0 };
            for (idx, &size) in page_sizes.iter().enumerate() {
                let scaled = scale_to_fit(size, viewport_size, FitMode::Height) * zoom;
                let x = if direction > 0.0 {
                    cursor
                } else {
                    viewport_size.x - cursor - scaled.x
                };
                layouts.push(PageLayout {
                    rect: Rect::from_min_size(egui::pos2(x, (viewport_size.y - scaled.y) / 2.0), scaled),
                    page_index: idx,
                });
                cursor += scaled.x;
            }
        }
        ReadingMode::Webtoon => {
            let mut cursor = 0.0;
            for (idx, &size) in page_sizes.iter().enumerate() {
                let scaled = scale_to_fit(size, viewport_size, FitMode::Width) * zoom;
                layouts.push(PageLayout {
                    rect: Rect::from_min_size(
                        egui::pos2((viewport_size.x - scaled.x) / 2.0, cursor),
                        scaled,
                    ),
                    page_index: idx,
                });
                cursor += scaled.y;
            }
        }
    }
    layouts
}

pub fn scale_to_fit(size: Vec2, viewport: Vec2, fit_mode: FitMode) -> Vec2 {
    match fit_mode {
        FitMode::Original => size,
        FitMode::Page => {
            let scale = (viewport.x / size.x).min(viewport.y / size.y);
            size * scale
        }
        FitMode::Height => {
            let scale = viewport.y / size.y;
            size * scale
        }
        FitMode::Width => {
            let scale = viewport.x / size.x;
            size * scale
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_to_fit_height() {
        let size = Vec2::new(800.0, 1200.0);
        let viewport = Vec2::new(1920.0, 1080.0);
        let result = scale_to_fit(size, viewport, FitMode::Height);
        assert_eq!(result.y, 1080.0);
    }

    #[test]
    fn test_webtoon_layout_stacks_vertically() {
        let sizes = vec![Vec2::new(800.0, 1200.0), Vec2::new(800.0, 1200.0)];
        let viewport = Vec2::new(1000.0, 600.0);
        let layouts = compute_layout(ReadingMode::Webtoon, viewport, &sizes, 1.0);
        assert_eq!(layouts.len(), 2);
        assert!(layouts[1].rect.min.y > layouts[0].rect.min.y);
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p rust-reader-core
```

Expected: Tests pass.

- [ ] **Step 3: Commit**

```bash
git add rust-reader-core/src/layout.rs
git commit -m "feat(core): add layout engine for three reading modes"
```

---

## Task 5: Parser Trait and Folder Parser

**Files:**
- Create: `rust-reader-parser/src/traits.rs`
- Create: `rust-reader-parser/src/folder.rs`
- Test: `rust-reader-parser/src/folder.rs`

- [ ] **Step 1: Write Parser trait with error type**

`rust-reader-parser/src/traits.rs`:
```rust
use rust_reader_core::models::Comic;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid archive: {0}")]
    InvalidArchive(String),
    #[error("Unsupported format")]
    Unsupported,
    #[error("No pages found")]
    NoPages,
}

pub trait Parser: Send + Sync {
    fn supports(path: &Path) -> bool
    where
        Self: Sized;
    fn parse(path: &Path) -> Result<Comic, ParseError>
    where
        Self: Sized;
}
```

- [ ] **Step 2: Write FolderParser with tests**

`rust-reader-parser/src/folder.rs`:
```rust
use crate::traits::{ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::path::{Path, PathBuf};

pub struct FolderParser;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "avif"];

impl Parser for FolderParser {
    fn supports(path: &Path) -> bool {
        path.is_dir()
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_image(p))
            .collect();
        entries.sort();

        if entries.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = entries
            .into_iter()
            .enumerate()
            .map(|(idx, path)| Page {
                index: idx,
                source: PageSource::File(path),
            })
            .collect();

        let title = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(Comic {
            id: title.clone(),
            title,
            path: path.to_path_buf(),
            volumes: vec![Volume { title: "Default".to_string(), pages }],
        })
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_empty_folder_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = FolderParser::parse(tmp.path());
        assert!(matches!(result, Err(ParseError::NoPages)));
    }

    #[test]
    fn test_parse_folder_with_images() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("01.png"), b"fake").unwrap();
        fs::write(tmp.path().join("02.jpg"), b"fake").unwrap();
        let comic = FolderParser::parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 2);
    }
}
```

- [ ] **Step 3: Add tempfile dev dependency**

`rust-reader-parser/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3.14"
```

- [ ] **Step 4: Run tests**

Run:
```bash
cargo test -p rust-reader-parser
```

Expected: Tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust-reader-parser/src/traits.rs rust-reader-parser/src/folder.rs rust-reader-parser/Cargo.toml
git commit -m "feat(parser): add parser trait and folder parser"
```

---

## Task 6: ZIP/CBZ Parser

**Files:**
- Create: `rust-reader-parser/src/zip.rs`
- Test: `rust-reader-parser/src/zip.rs`

- [ ] **Step 1: Write ZipParser with tests**

`rust-reader-parser/src/zip.rs`:
```rust
use crate::traits::{ParseError, Parser};
use rust_reader_core::models::{Comic, Page, PageSource, Volume};
use std::io::{Cursor, Read};
use std::path::Path;

pub struct ZipParser;

impl Parser for ZipParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "cbz"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)
                .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
            if file.is_file() && is_image_name(file.name()) {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                entries.push((file.name().to_string(), bytes));
            }
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        if entries.is_empty() {
            return Err(ParseError::NoPages);
        }

        let pages: Vec<Page> = entries
            .into_iter()
            .enumerate()
            .map(|(idx, (_, bytes))| Page {
                index: idx,
                source: PageSource::Bytes(bytes),
            })
            .collect();

        let title = path.file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(Comic {
            id: title.clone(),
            title,
            path: path.to_path_buf(),
            volumes: vec![Volume { title: "Default".to_string(), pages }],
        })
    }
}

fn is_image_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "avif")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_parse_cbz() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("01.png", options).unwrap();
            zip.write_all(b"fake").unwrap();
            zip.start_file("02.jpg", options).unwrap();
            zip.write_all(b"fake").unwrap();
            zip.finish().unwrap();
        }
        let comic = ZipParser::parse(&path).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p rust-reader-parser
```

Expected: Tests pass.

- [ ] **Step 3: Commit**

```bash
git add rust-reader-parser/src/zip.rs
git commit -m "feat(parser): add zip/cbz parser"
```

---

## Task 7: RAR/CBR and PDF Parser Stubs

**Files:**
- Create: `rust-reader-parser/src/rar.rs`
- Create: `rust-reader-parser/src/pdf.rs`
- Test: `rust-reader-parser/src/lib.rs` integration

- [ ] **Step 1: Write RarParser stub**

`rust-reader-parser/src/rar.rs`:
```rust
use crate::traits::{ParseError, Parser};
use rust_reader_core::models::Comic;
use std::path::Path;

pub struct RarParser;

impl Parser for RarParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "rar" | "cbr"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        // Future: integrate with unrar crate or external unrar binary.
        // For now, report unsupported to avoid pulling in non-free dependencies.
        Err(ParseError::Unsupported)
    }
}
```

- [ ] **Step 2: Write PdfParser stub**

`rust-reader-parser/src/pdf.rs`:
```rust
use crate::traits::{ParseError, Parser};
use rust_reader_core::models::Comic;
use std::path::Path;

pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase() == "pdf")
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        // Future: integrate with pdf-rs or mupdf bindings.
        Err(ParseError::Unsupported)
    }
}
```

- [ ] **Step 3: Add dispatch helper and integration test**

`rust-reader-parser/src/lib.rs`:
```rust
pub mod traits;
pub mod folder;
pub mod zip;
pub mod rar;
pub mod pdf;

use rust_reader_core::models::Comic;
use std::path::Path;
use traits::{ParseError, Parser};

pub fn parse(path: &Path) -> Result<Comic, ParseError> {
    if folder::FolderParser::supports(path) {
        folder::FolderParser::parse(path)
    } else if zip::ZipParser::supports(path) {
        zip::ZipParser::parse(path)
    } else if rar::RarParser::supports(path) {
        rar::RarParser::parse(path)
    } else if pdf::PdfParser::supports(path) {
        pdf::PdfParser::parse(path)
    } else {
        Err(ParseError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_dispatch_folder() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("page.png"), b"fake").unwrap();
        let comic = parse(tmp.path()).unwrap();
        assert_eq!(comic.volumes[0].pages.len(), 1);
    }
}
```

- [ ] **Step 4: Run tests**

Run:
```bash
cargo test -p rust-reader-parser
```

Expected: Tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust-reader-parser/src/rar.rs rust-reader-parser/src/pdf.rs rust-reader-parser/src/lib.rs
git commit -m "feat(parser): add rar/cbr and pdf parser stubs with dispatch"
```

---

## Task 8: Storage Models

**Files:**
- Create: `rust-reader-storage/src/models.rs`
- Test: `rust-reader-storage/src/models.rs`

- [ ] **Step 1: Write storage models with tests**

`rust-reader-storage/src/models.rs`:
```rust
use rust_reader_core::models::{FitMode, ReadingMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub window_size: (f32, f32),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub entries: Vec<LibraryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub comic_id: String,
    pub volume_index: usize,
    pub page_index: usize,
    pub last_read_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub comic_id: String,
    pub volume_index: usize,
    pub page_index: usize,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bookmarks {
    pub items: Vec<Bookmark>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let s = Settings::default();
        assert!(matches!(s.theme, Theme::System));
    }

    #[test]
    fn test_library_serialize() {
        let lib = Library {
            entries: vec![LibraryEntry {
                comic_id: "id".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp"),
                cover_path: None,
            }],
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(json.contains("Test"));
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p rust-reader-storage
```

Expected: Tests pass.

- [ ] **Step 3: Commit**

```bash
git add rust-reader-storage/src/models.rs rust-reader-storage/Cargo.toml
git commit -m "feat(storage): add storage models"
```

---

## Task 9: JSON Store

**Files:**
- Create: `rust-reader-storage/src/json_store.rs`
- Test: `rust-reader-storage/src/json_store.rs`

- [ ] **Step 1: Write JsonStore with tests**

`rust-reader-storage/src/json_store.rs`:
```rust
use crate::models::{Bookmarks, History, Library, Settings};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct JsonStore {
    dir: PathBuf,
}

impl JsonStore {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    pub fn default_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("rust-reader"))
    }

    pub fn ensure_dir(&self) -> Result<(), StorageError> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), StorageError> {
        self.write_json("settings.json", settings)
    }

    pub fn load_settings(&self) -> Result<Settings, StorageError> {
        self.read_json("settings.json")
    }

    pub fn save_library(&self, library: &Library) -> Result<(), StorageError> {
        self.write_json("library.json", library)
    }

    pub fn load_library(&self) -> Result<Library, StorageError> {
        self.read_json("library.json")
    }

    pub fn save_history(&self, history: &History) -> Result<(), StorageError> {
        self.write_json("history.json", history)
    }

    pub fn load_history(&self) -> Result<History, StorageError> {
        self.read_json("history.json")
    }

    pub fn save_bookmarks(&self, bookmarks: &Bookmarks) -> Result<(), StorageError> {
        self.write_json("bookmarks.json", bookmarks)
    }

    pub fn load_bookmarks(&self) -> Result<Bookmarks, StorageError> {
        self.read_json("bookmarks.json")
    }

    fn write_json<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), StorageError> {
        self.ensure_dir()?;
        let path = self.dir.join(name);
        let json = serde_json::to_string_pretty(value)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T, StorageError> {
        let path = self.dir.join(name);
        if !path.exists() {
            return Ok(T::default());
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Settings;

    #[test]
    fn test_roundtrip_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path());
        let settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();
        assert_eq!(settings, loaded);
    }
}
```

- [ ] **Step 2: Add tempfile dev dependency**

`rust-reader-storage/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3.14"
```

- [ ] **Step 3: Run tests**

Run:
```bash
cargo test -p rust-reader-storage
```

Expected: Tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-storage/src/json_store.rs rust-reader-storage/Cargo.toml
git commit -m "feat(storage): add json file store"
```

---

## Task 10: egui App Skeleton

**Files:**
- Create: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-app/src/main.rs`

- [ ] **Step 1: Create basic App struct**

`rust-reader-app/src/app.rs`:
```rust
use rust_reader_core::models::ReadingMode;
use rust_reader_storage::models::Settings;

pub enum View {
    Library,
    Reader,
    Settings,
}

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            settings: Settings {
                default_mode: ReadingMode::Ltr,
                ..Default::default()
            },
        }
    }
}

impl ReaderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("rustReader");
            ui.label("App skeleton loaded");
        });
    }
}
```

- [ ] **Step 2: Wire main.rs**

`rust-reader-app/src/main.rs`:
```rust
mod app;

use app::ReaderApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rustReader",
        options,
        Box::new(|cc| Ok(Box::new(ReaderApp::new(cc)))),
    )
}
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/app.rs rust-reader-app/src/main.rs
git commit -m "feat(app): add egui app skeleton"
```

---

## Task 11: Library View

**Files:**
- Create: `rust-reader-app/src/views/library.rs`
- Create: `rust-reader-app/src/views/mod.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: Create views module**

`rust-reader-app/src/views/mod.rs`:
```rust
pub mod library;
pub mod reader;
pub mod settings;
```

`rust-reader-app/src/views/library.rs`:
```rust
use rust_reader_storage::models::Library;

pub struct LibraryView {
    pub library: Library,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            library: Library::default(),
        }
    }
}

impl LibraryView {
    pub fn ui(&mut self, ui: &mut egui::Ui, on_open: &mut dyn FnMut(usize)) {
        ui.heading("书架");
        if self.library.entries.is_empty() {
            ui.label("暂无漫画，请点击“打开”按钮添加。");
            return;
        }
        egui::Grid::new("library_grid").show(ui, |ui| {
            for (idx, entry) in self.library.entries.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.label(&entry.title);
                    if ui.button("打开").clicked() {
                        on_open(idx);
                    }
                });
                ui.end_row();
            }
        });
    }
}
```

- [ ] **Step 2: Integrate into App**

`rust-reader-app/src/app.rs`:
```rust
use crate::views::library::LibraryView;
use rust_reader_core::models::ReadingMode;
use rust_reader_storage::models::Settings;

pub enum View {
    Library,
    Reader,
    Settings,
}

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            settings: Settings {
                default_mode: ReadingMode::Ltr,
                ..Default::default()
            },
            library_view: LibraryView::default(),
        }
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                View::Library => {
                    self.library_view.ui(ui, &mut |_| {
                        // Reader opening implemented in Task 12
                    });
                }
                _ => {
                    ui.label("View not implemented yet");
                }
            }
        });
    }
}
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/views rust-reader-app/src/app.rs
git commit -m "feat(app): add library view scaffold"
```

---

## Task 12: Reader View - Page Rendering

**Files:**
- Create: `rust-reader-app/src/widgets/page_view.rs`
- Create: `rust-reader-app/src/widgets/mod.rs`
- Create: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: Create page view widget**

`rust-reader-app/src/widgets/mod.rs`:
```rust
pub mod page_view;
pub mod thumbnail_bar;
```

`rust-reader-app/src/widgets/page_view.rs`:
```rust
use egui::{ColorImage, TextureHandle, TextureOptions};

pub fn load_texture_from_bytes(ctx: &egui::Context, bytes: &[u8]) -> Option<TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?;
    let size = [image.width() as _, image.height() as _];
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.as_flat_samples();
    let color_image = ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
    Some(ctx.load_texture("page", color_image, TextureOptions::default()))
}

pub fn load_texture_from_path(ctx: &egui::Context, path: &std::path::Path) -> Option<TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    load_texture_from_bytes(ctx, &bytes)
}
```

- [ ] **Step 2: Create reader view**

`rust-reader-app/src/views/reader.rs`:
```rust
use rust_reader_core::models::{Comic, ReadingState};

pub struct ReaderView {
    pub comic: Option<Comic>,
    pub state: Option<ReadingState>,
}

impl Default for ReaderView {
    fn default() -> Self {
        Self {
            comic: None,
            state: None,
        }
    }
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState) {
        self.comic = Some(comic);
        self.state = Some(state);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(comic) = &self.comic else {
            ui.label("未打开漫画");
            return;
        };
        let Some(state) = &self.state else {
            return;
        };
        let volume = &comic.volumes[0];
        let page = &volume.pages[state.current_page];
        let texture = match &page.source {
            rust_reader_core::models::PageSource::File(path) => {
                crate::widgets::page_view::load_texture_from_path(ctx, path)
            }
            rust_reader_core::models::PageSource::Bytes(bytes) => {
                crate::widgets::page_view::load_texture_from_bytes(ctx, bytes)
            }
            rust_reader_core::models::PageSource::PdfRef { .. } => None,
        };
        if let Some(texture) = texture {
            ui.image(&texture);
        } else {
            ui.label("无法加载页面");
        }
    }
}
```

- [ ] **Step 3: Integrate reader view into app**

`rust-reader-app/src/app.rs`:
```rust
use crate::views::{library::LibraryView, reader::ReaderView};
use rust_reader_core::models::{ReadingMode, ReadingState};
use rust_reader_storage::models::Settings;

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            settings: Settings {
                default_mode: ReadingMode::Ltr,
                ..Default::default()
            },
            library_view: LibraryView::default(),
            reader_view: ReaderView::default(),
        }
    }
}

impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                View::Library => {
                    let mut open_idx = None;
                    self.library_view.ui(ui, &mut |idx| open_idx = Some(idx));
                    if let Some(idx) = open_idx {
                        if let Some(entry) = self.library_view.library.entries.get(idx) {
                            if let Ok(comic) = rust_reader_parser::parse(&entry.path) {
                                let state = ReadingState::new(self.settings.default_mode, comic.volumes[0].pages.len());
                                self.reader_view.open(comic, state);
                                self.current_view = View::Reader;
                            }
                        }
                    }
                }
                View::Reader => {
                    self.reader_view.ui(ui, ctx);
                }
                View::Settings => {
                    ui.label("设置视图待实现");
                }
            }
        });
    }
}
```

- [ ] **Step 4: Add image dependency to app**

`rust-reader-app/Cargo.toml`:
```toml
[dependencies]
eframe = { version = "0.29", features = ["default"] }
egui = "0.29"
image = { workspace = true }
rust-reader-core = { path = "../rust-reader-core" }
rust-reader-parser = { path = "../rust-reader-parser" }
rust-reader-storage = { path = "../rust-reader-storage" }
```

- [ ] **Step 5: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 6: Commit**

```bash
git add rust-reader-app/src/widgets rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs rust-reader-app/Cargo.toml
git commit -m "feat(app): add reader view with basic page rendering"
```

---

## Task 13: Reading Modes and Navigation

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: Add mode switch and page navigation to reader view**

`rust-reader-app/src/views/reader.rs`:
```rust
impl ReaderView {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(comic) = &self.comic else {
            ui.label("未打开漫画");
            return;
        };
        let total_pages = comic.volumes[0].pages.len();
        let Some(state) = &mut self.state else {
            return;
        };

        ui.horizontal(|ui| {
            if ui.selectable_label(matches!(state.mode, rust_reader_core::models::ReadingMode::Ltr), "国漫").clicked() {
                state.set_mode(rust_reader_core::models::ReadingMode::Ltr, total_pages);
            }
            if ui.selectable_label(matches!(state.mode, rust_reader_core::models::ReadingMode::Rtl), "日漫").clicked() {
                state.set_mode(rust_reader_core::models::ReadingMode::Rtl, total_pages);
            }
            if ui.selectable_label(matches!(state.mode, rust_reader_core::models::ReadingMode::Webtoon), "韩漫").clicked() {
                state.set_mode(rust_reader_core::models::ReadingMode::Webtoon, total_pages);
            }
        });

        // Page display
        let page = &comic.volumes[0].pages[state.current_page];
        let texture = match &page.source {
            rust_reader_core::models::PageSource::File(path) => {
                crate::widgets::page_view::load_texture_from_path(ctx, path)
            }
            rust_reader_core::models::PageSource::Bytes(bytes) => {
                crate::widgets::page_view::load_texture_from_bytes(ctx, bytes)
            }
            rust_reader_core::models::PageSource::PdfRef { .. } => None,
        };
        if let Some(texture) = texture {
            ui.image(&texture);
        } else {
            ui.label("无法加载页面");
        }

        ui.horizontal(|ui| {
            if ui.button("上一页").clicked() {
                state.prev_page();
            }
            ui.label(format!("{}/{}", state.current_page + 1, total_pages));
            if ui.button("下一页").clicked() {
                state.next_page(total_pages);
            }
        });
    }
}
```

- [ ] **Step 2: Add keyboard input handling in app update**

`rust-reader-app/src/app.rs`:
```rust
impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if matches!(self.current_view, View::Reader) {
            if let Some(state) = self.reader_view.state.as_mut() {
                let total = self.reader_view.comic.as_ref().map(|c| c.volumes[0].pages.len()).unwrap_or(0);
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    match state.mode {
                        rust_reader_core::models::ReadingMode::Ltr => state.next_page(total),
                        rust_reader_core::models::ReadingMode::Rtl => state.prev_page(),
                        rust_reader_core::models::ReadingMode::Webtoon => state.next_page(total),
                    }
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    match state.mode {
                        rust_reader_core::models::ReadingMode::Ltr => state.prev_page(),
                        rust_reader_core::models::ReadingMode::Rtl => state.next_page(total),
                        rust_reader_core::models::ReadingMode::Webtoon => state.prev_page(),
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // ... existing view dispatch
        });
    }
}
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs
git commit -m "feat(app): add reading mode switch and keyboard navigation"
```

---

## Task 14: Zoom, Pan, Fullscreen

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: Add zoom, pan, and fullscreen controls**

`rust-reader-app/src/views/reader.rs`:
```rust
// In ReaderView::ui, before page display:
ui.horizontal(|ui| {
    if ui.button("-").clicked() {
        state.zoom *= 0.9;
    }
    ui.label(format!("{:.0}%", state.zoom * 100.0));
    if ui.button("+").clicked() {
        state.zoom *= 1.1;
    }
    if ui.button("适应").clicked() {
        state.zoom = 1.0;
        state.pan = egui::Vec2::ZERO;
    }
});

// Detect drag to pan when zoomed in
let response = ui.interact(ui.max_rect(), ui.id().with("reader_drag"), egui::Sense::drag());
if response.dragged() {
    state.pan += response.drag_delta();
}
```

- [ ] **Step 2: Add fullscreen shortcut in app**

`rust-reader-app/src/app.rs`:
```rust
impl eframe::App for ReaderApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            frame.set_fullscreen(!frame.info().window_info.fullscreen);
        }
        // ... rest
    }
}
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs
git commit -m "feat(app): add zoom, pan, and fullscreen controls"
```

---

## Task 15: Page Navigator Bar

**Files:**
- Create: `rust-reader-app/src/widgets/page_navigator.rs`
- Modify: `rust-reader-app/src/views/reader.rs`

- [ ] **Step 1: Implement page navigator widget**

`rust-reader-app/src/widgets/page_navigator.rs`:
```rust
use rust_reader_core::models::Comic;

pub fn page_navigator(
    ui: &mut egui::Ui,
    comic: &Comic,
    current_page: usize,
    on_select: &mut dyn FnMut(usize),
) {
    if comic.volumes.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        for (idx, _page) in comic.volumes[0].pages.iter().enumerate() {
            let selected = idx == current_page;
            let label = (idx + 1).to_string();
            if ui.selectable_label(selected, label).clicked() {
                on_select(idx);
            }
        }
    });
}
```

- [ ] **Step 2: Integrate into reader view**

`rust-reader-app/src/views/reader.rs`:
```rust
use crate::widgets::page_navigator::page_navigator;

// In ReaderView::ui, after page display:
let current_page = reader.state.current_page;
let total_pages = reader.total_pages();
let comic = &reader.comic;
let state = &mut reader.state;
let texture_page = &mut reader.texture_page;
page_navigator(ui, comic, current_page, &mut |idx| {
    state.go_to_page(idx, total_pages);
    *texture_page = None;
});
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/widgets/page_navigator.rs rust-reader-app/src/views/reader.rs
git commit -m "feat(app): add page navigator bar"
```

---

## Task 16: History and Bookmarks

**Files:**
- Modify: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-app/src/views/reader.rs`
- Create: `rust-reader-app/src/views/settings.rs`

- [ ] **Step 1: Add storage to app and save history on close**

`rust-reader-app/src/app.rs`:
```rust
use rust_reader_storage::{json_store::JsonStore, models::{Bookmarks, History, Library}};

pub struct ReaderApp {
    pub current_view: View,
    pub settings: Settings,
    pub library_view: LibraryView,
    pub reader_view: ReaderView,
    pub store: JsonStore,
    pub history: History,
    pub bookmarks: Bookmarks,
}

impl Default for ReaderApp {
    fn default() -> Self {
        let store = JsonStore::new(JsonStore::default_dir().unwrap_or_else(|| PathBuf::from(".")));
        let settings = store.load_settings().unwrap_or_default();
        let library = store.load_library().unwrap_or_default();
        let history = store.load_history().unwrap_or_default();
        let bookmarks = store.load_bookmarks().unwrap_or_default();
        let mut library_view = LibraryView::default();
        library_view.library = library;
        Self {
            current_view: View::Library,
            settings,
            library_view,
            reader_view: ReaderView::default(),
            store,
            history,
            bookmarks,
        }
    }
}

impl eframe::App for ReaderApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.store.save_settings(&self.settings);
        let _ = self.store.save_library(&self.library_view.library);
        let _ = self.store.save_history(&self.history);
        let _ = self.store.save_bookmarks(&self.bookmarks);
    }
    // ...
}
```

- [ ] **Step 2: Update history when opening comic**

`rust-reader-app/src/app.rs`:
```rust
if let Some(idx) = open_idx {
    if let Some(entry) = self.library_view.library.entries.get(idx).cloned() {
        if let Ok(comic) = rust_reader_parser::parse(&entry.path) {
            let total = comic.volumes[0].pages.len();
            let mut state = ReadingState::new(self.settings.default_mode, total);
            // Restore history
            if let Some(h) = self.history.entries.iter().find(|h| h.comic_id == entry.comic_id) {
                state.go_to_page(h.page_index, total);
            }
            self.reader_view.open(comic, state);
            self.current_view = View::Reader;
        }
    }
}
```

- [ ] **Step 3: Add bookmark button in reader view**

`rust-reader-app/src/views/reader.rs`:
```rust
// In ui(), add a button:
if ui.button("添加书签").clicked() {
    on_bookmark(state.current_page);
}
```

Change signature to accept `on_bookmark: &mut dyn FnMut(usize)`.

- [ ] **Step 4: Implement settings view scaffold**

`rust-reader-app/src/views/settings.rs`:
```rust
use rust_reader_core::models::ReadingMode;
use rust_reader_storage::models::{Settings, Theme};

pub struct SettingsView;

impl SettingsView {
    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.heading("设置");

        ui.label("默认阅读模式");
        egui::ComboBox::from_id_salt("mode")
            .selected_text(mode_label(settings.default_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.default_mode, ReadingMode::Ltr, "国漫（左→右）");
                ui.selectable_value(&mut settings.default_mode, ReadingMode::Rtl, "日漫（右→左）");
                ui.selectable_value(&mut settings.default_mode, ReadingMode::Webtoon, "韩漫（上→下）");
            });

        ui.label("主题");
        egui::ComboBox::from_id_salt("theme")
            .selected_text(theme_label(settings.theme))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.theme, Theme::System, "跟随系统");
                ui.selectable_value(&mut settings.theme, Theme::Light, "浅色");
                ui.selectable_value(&mut settings.theme, Theme::Dark, "深色");
            });
    }
}

fn mode_label(mode: ReadingMode) -> &'static str {
    match mode {
        ReadingMode::Ltr => "国漫（左→右）",
        ReadingMode::Rtl => "日漫（右→左）",
        ReadingMode::Webtoon => "韩漫（上→下）",
    }
}

fn theme_label(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "跟随系统",
        Theme::Light => "浅色",
        Theme::Dark => "深色",
    }
}
```

- [ ] **Step 5: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 6: Commit**

```bash
git add rust-reader-app/src/app.rs rust-reader-app/src/views/reader.rs rust-reader-app/src/views/settings.rs
git commit -m "feat(app): add history, bookmarks, and settings scaffold"
```

---

## Task 17: Open File Dialog and Library Management

**Files:**
- Modify: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-app/src/views/library.rs`

- [ ] **Step 1: Add open folder/file buttons to library view**

`rust-reader-app/src/views/library.rs`:
```rust
impl LibraryView {
    pub fn ui(&mut self, ui: &mut egui::Ui, on_open: &mut dyn FnMut(usize), on_add: &mut dyn FnMut()) {
        ui.horizontal(|ui| {
            if ui.button("打开文件夹").clicked() {
                on_add();
            }
        });
        // ... existing grid
    }
}
```

- [ ] **Step 2: Use rfd to open file dialog in app**

Add to `rust-reader-app/Cargo.toml`:
```toml
rfd = "0.15"
```

`rust-reader-app/src/app.rs`:
```rust
// In update(), before CentralPanel:
if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.ctrl) {
    if let Some(path) = rfd::FileDialog::new().pick_folder() {
        if let Ok(comic) = rust_reader_parser::parse(&path) {
            let entry = rust_reader_storage::models::LibraryEntry {
                comic_id: comic.id.clone(),
                title: comic.title.clone(),
                path: path.clone(),
                cover_path: None,
            };
            if !self.library_view.library.entries.iter().any(|e| e.path == path) {
                self.library_view.library.entries.push(entry);
            }
        }
    }
}
```

- [ ] **Step 3: Build app**

Run:
```bash
cargo check -p rust-reader-app
```

Expected: Successful compilation.

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/app.rs rust-reader-app/src/views/library.rs rust-reader-app/Cargo.toml
git commit -m "feat(app): add file dialog and library management"
```

---

## Task 18: Integration, README, and Final Polish

**Files:**
- Modify: `README.md`
- Modify: `rust-reader-app/src/app.rs`
- Modify: `rust-reader-app/src/views/reader.rs`

- [ ] **Step 1: Write README**

`README.md`:
```markdown
# rustReader

一款使用 Rust + egui 构建的跨平台漫画阅读器，支持国漫（左→右）、日漫（右→左）和韩漫/Webtoon（长条从上到下）三种阅读模式。

## 功能

- 打开本地图片文件夹、CBZ/ZIP、CBR/RAR（待完整实现）、PDF（待完整实现）
- 三种阅读模式切换
- 缩放、平移、全屏
- 书架、阅读历史、书签
- 缩略图导航

## 运行

```bash
cargo run -p rust-reader-app
```

## 测试

```bash
cargo test
```
```

- [ ] **Step 2: Run all tests**

Run:
```bash
cargo test
```

Expected: All workspace tests pass.

- [ ] **Step 3: Run app smoke test**

Run:
```bash
cargo run -p rust-reader-app
```

Expected: App window opens without crash. Close manually.

- [ ] **Step 4: Final commit**

```bash
git add README.md
git commit -m "docs: add README and finalize app"
```

---

## Self-Review Checklist

- [ ] Spec coverage: 每个设计文档中的需求都有对应任务。
- [ ] Placeholder scan: 无 "TBD"、"TODO"、"implement later"。
- [ ] Type consistency: `ReadingState`, `Comic`, `PageSource` 等类型在 core/app/parser 中一致。
- [ ] Testability: core/parser/storage 都有单元测试，app 可手动 smoke test。

## Known Limitations

- RAR/CBR 和 PDF 解析为首版 stub，返回 `Unsupported` 错误。
- 双页模式、快捷键自定义、预加载、异步加载列入未来规划。
