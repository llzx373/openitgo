# 媒体播放（视频 + 音频）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 rustReader 能打开并播放视频/音频文件（内嵌 libmpv，原生子视图渲染），完整接入库、封面、进度恢复与历史。

**Architecture:** 新 crate `rust-reader-media` 封装 libmpv FFI（命令、事件泵、属性观察、OpenGL 渲染上下文、无头封面截图）；app 侧 `mpv_view.rs` 提供 macOS 原生子 `NSView`（`CAOpenGLLayer`）叠加渲染，`views/media.rs` 提供 `MediaView` UI 与传输控制。mpv 画面由 `CAOpenGLLayer` 驱动，egui 控制条由事件泵线程经 `egui::Context::request_repaint()` 驱动（commit b071a7b 同款手法）。

**Tech Stack:** Rust workspace、libmpv（Homebrew mpv 0.41，随 .app 打包）、`libmpv-sys 3.1`、`objc 0.2` + `cgl 0.3` + `core-foundation/core-graphics`（沿用现有 macOS 依赖）、eframe/egui 0.29、crossbeam-channel。

## Global Constraints

- 仅 macOS；非 macOS 编译必须保持通过（stub 返回错误）。
- UI 文案中文（技术标识符除外）。
- 每个任务结束必须全绿：`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`。
- 不改动既有漫画/电子书行为；既有测试全部保持通过。
- 漫画/电子书/媒体的 stable id 一律 `rust_reader_parser::stable_comic_id(&path)`。
- 进度复用 `HistoryEntry.char_offset` 存毫秒，`page_index = 0`；不新增存储字段。
- 设计文档：`docs/superpowers/specs/2026-07-14-media-playback-design.md`。
- 与 spec 的两处有意偏差（实现期核实后确认）：① macOS 原生代码用 `objc 0.2`（项目已有依赖）而非 spec §14 的 objc2；② 不引入 `MediaKind` 枚举，扩展名直接映射 `MediaType`，音视频运行时靠 `track-list` 区分。

---

### Task 1: `rust-reader-media` 脚手架 + 时间格式化

**Files:**
- Modify: `Cargo.toml:2`（workspace members）
- Create: `rust-reader-media/Cargo.toml`
- Create: `rust-reader-media/src/lib.rs`
- Create: `rust-reader-media/src/error.rs`
- Create: `rust-reader-media/src/time.rs`

**Interfaces:**
- Produces:
  - `rust_reader_media::MediaError`（enum：`Init(String)`、`Command { code: i32, what: String }`、`Load(String)`，`Display` 中文）
  - `rust_reader_media::time::format_time_ms(ms: u64) -> String`（`< 1h` → `m:ss`，`>= 1h` → `h:mm:ss`）

- [ ] **Step 1: 前置确认 libmpv（需要用户在本机执行，安装到工作目录之外，先征得同意）**

```bash
brew install mpv
ls "$(brew --prefix mpv)/lib/" | grep libmpv        # 期望看到 libmpv.2.dylib
otool -D "$(brew --prefix mpv)/lib/libmpv.2.dylib"  # install name 应为绝对路径
```

Expected: `libmpv.2.dylib` 存在，install name 为绝对路径（开发期可直接加载）。

注意：brew 的 mpv formula（实测 0.41.0_6）**不提供 `libmpv.pc`**，不要用 pkg-config 验证。
同时 `/opt/homebrew/lib` 不在 Rust 默认链接搜索路径，`ld` 直接 `-lmpv` 会报
`library 'mpv' not found`——Task 5 用 build.rs 注入 link-search 解决。

- [ ] **Step 2: workspace 加入新成员**

`Cargo.toml:2` 改为：

```toml
members = ["rust-reader-core", "rust-reader-parser", "rust-reader-storage", "rust-reader-app", "rust-reader-media"]
```

- [ ] **Step 3: 创建 `rust-reader-media/Cargo.toml`（暂不含 libmpv-sys，Task 5 再加）**

```toml
[package]
name = "rust-reader-media"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "libmpv playback core for rustReader"

[dependencies]
thiserror = { workspace = true }
```

- [ ] **Step 4: 写失败测试 — `rust-reader-media/src/time.rs` 末尾 `#[cfg(test)]`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_under_one_hour_as_m_ss() {
        assert_eq!(format_time_ms(0), "0:00");
        assert_eq!(format_time_ms(59_000), "0:59");
        assert_eq!(format_time_ms(60_000), "1:00");
        assert_eq!(format_time_ms(3_599_000), "59:59");
    }

    #[test]
    fn formats_one_hour_and_above_as_h_mm_ss() {
        assert_eq!(format_time_ms(3_600_000), "1:00:00");
        assert_eq!(format_time_ms(7_261_000), "2:01:01");
    }
}
```

- [ ] **Step 5: 运行测试确认失败**

Run: `cargo test -p rust-reader-media`
Expected: 编译失败 `cannot find function format_time_ms in this scope`

- [ ] **Step 6: 实现 `rust-reader-media/src/time.rs`**

```rust
/// Formats a millisecond position as `m:ss` below one hour, `h:mm:ss` otherwise.
pub fn format_time_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}
```

- [ ] **Step 7: 创建 `rust-reader-media/src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("无法初始化播放器：{0}")]
    Init(String),
    #[error("播放器命令失败（{code}）：{what}")]
    Command { code: i32, what: String },
    #[error("无法播放该文件：{0}")]
    Load(String),
}
```

- [ ] **Step 8: 创建 `rust-reader-media/src/lib.rs`**

```rust
pub mod error;
pub mod time;

pub use error::MediaError;
```

- [ ] **Step 9: 运行测试确认通过 + 全量流水线**

Run: `cargo test -p rust-reader-media`（PASS 2 个测试）
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿，既有 150+ 测试全部通过。

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml rust-reader-media/
git commit -m "feat(media): scaffold rust-reader-media crate with MediaError and time formatting"
```

---

### Task 2: `MediaType` 增加 Video/Audio 变体

**Files:**
- Modify: `rust-reader-storage/src/models.rs:193-199`（MediaType 定义）及测试模块

**Interfaces:**
- Consumes: 无
- Produces: `MediaType::Video`、`MediaType::Audio`（serde 为 `video`/`audio`；反序列化缺失字段仍默认 `Comic`）

- [ ] **Step 1: 写失败测试（追加到 `rust-reader-storage/src/models.rs` 的 `mod tests`）**

```rust
#[test]
fn test_media_type_video_audio_roundtrip() {
    let v = serde_json::to_string(&MediaType::Video).unwrap();
    let a = serde_json::to_string(&MediaType::Audio).unwrap();
    assert_eq!(v, "\"video\"");
    assert_eq!(a, "\"audio\"");
    assert_eq!(serde_json::from_str::<MediaType>(&v).unwrap(), MediaType::Video);
    assert_eq!(serde_json::from_str::<MediaType>(&a).unwrap(), MediaType::Audio);
}

#[test]
fn test_library_entry_deserializes_media_type_video() {
    let json = r#"{"comic_id":"id","title":"T","path":"/tmp/v.mp4","cover_path":null,"added_at":0,"media_type":"video"}"#;
    let entry: LibraryEntry = serde_json::from_str(json).unwrap();
    assert_eq!(entry.media_type, MediaType::Video);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-storage media_type`
Expected: FAIL（`Video`/`Audio` 变体不存在，编译错误）

- [ ] **Step 3: 修改 `rust-reader-storage/src/models.rs:195-199`**

```rust
pub enum MediaType {
    #[default]
    Comic,
    Ebook,
    Video,
    Audio,
}
```

- [ ] **Step 4: 运行测试确认通过 + 全量流水线**

Run: `cargo test -p rust-reader-storage`
Expected: PASS（含原有 `missing_media_type_as_comic` 兼容测试）
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add rust-reader-storage/src/models.rs
git commit -m "feat(storage): add Video/Audio variants to MediaType"
```

---

### Task 3: 媒体文件识别与入库分类

**Files:**
- Modify: `rust-reader-app/src/app.rs:27-45`（`is_ebook_file` 旁新增 `is_media_file`，扩展 `media_type_for_path`）
- Modify: `rust-reader-app/src/app.rs:1629-1635`（`add_file_to_library` 增加媒体分支）
- Modify: `rust-reader-app/src/app.rs:1842-1860`（`walk_supported_files` 纳入媒体扩展名）
- Modify: `rust-reader-app/src/views/library.rs:348-352`（`Library` 模式筛选放行 Video/Audio）

**Interfaces:**
- Consumes: `MediaType::Video/Audio`（Task 2）、`rust_reader_parser::stable_comic_id(&Path) -> String`
- Produces:
  - `fn is_media_file(path: &Path) -> bool`
  - `fn media_type_for_path(path: &Path) -> MediaType`（媒体扩展名 → Video/Audio，优先级：ebook > media > comic）
  - `ReaderApp::add_media_to_library(&mut self, path: PathBuf)`

- [ ] **Step 1: 写失败测试（追加到 `rust-reader-app/src/app.rs` 的 `mod tests`）**

```rust
#[test]
fn test_is_media_file_recognizes_video_and_audio() {
    use std::path::Path;
    assert!(is_media_file(Path::new("a.mp4")));
    assert!(is_media_file(Path::new("a.MKV")));
    assert!(is_media_file(Path::new("a.webm")));
    assert!(is_media_file(Path::new("a.mp3")));
    assert!(is_media_file(Path::new("a.flac")));
    assert!(!is_media_file(Path::new("a.epub")));
    assert!(!is_media_file(Path::new("a.cbz")));
    assert!(!is_media_file(Path::new("a")));
}

#[test]
fn test_media_type_for_path_classifies_media() {
    use std::path::Path;
    assert_eq!(media_type_for_path(Path::new("a.mp4")), MediaType::Video);
    assert_eq!(media_type_for_path(Path::new("a.mkv")), MediaType::Video);
    assert_eq!(media_type_for_path(Path::new("a.mp3")), MediaType::Audio);
    assert_eq!(media_type_for_path(Path::new("a.flac")), MediaType::Audio);
    assert_eq!(media_type_for_path(Path::new("a.epub")), MediaType::Ebook);
    assert_eq!(media_type_for_path(Path::new("a.cbz")), MediaType::Comic);
}
```

测试文件头部需补 `use rust_reader_storage::models::MediaType;`（若 tests 模块尚未导入）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app media_file`
Expected: 编译失败（`is_media_file` 未定义）

- [ ] **Step 3: 实现 `is_media_file`（插入 `rust-reader-app/src/app.rs:37` 之后）**

```rust
fn is_media_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                // 视频
                "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "ts" | "m2ts"
                | "mpg" | "mpeg" | "3gp"
                // 音频
                | "mp3" | "flac" | "aac" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "aiff"
                | "ape" | "wma"
            )
        })
        .unwrap_or(false)
}
```

- [ ] **Step 4: 扩展 `media_type_for_path`（替换 `rust-reader-app/src/app.rs:39-45` 函数体）**

```rust
fn media_type_for_path(path: &std::path::Path) -> MediaType {
    if is_ebook_file(path) {
        MediaType::Ebook
    } else if is_media_file(path) {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("mp3" | "flac" | "aac" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "aiff"
            | "ape" | "wma") => MediaType::Audio,
            _ => MediaType::Video,
        }
    } else {
        MediaType::Comic
    }
}
```

- [ ] **Step 5: 实现 `add_media_to_library` 并接入分发**

在 `add_ebook_to_library` 之后新增：

```rust
fn add_media_to_library(&mut self, path: std::path::PathBuf) {
    if self.library_view.library.entries.iter().any(|e| e.path == path) {
        return;
    }
    let added_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("未知媒体")
        .to_string();
    self.library_view
        .library
        .entries
        .push(rust_reader_storage::models::LibraryEntry {
            comic_id: rust_reader_parser::stable_comic_id(&path),
            title,
            path,
            cover_path: None,
            added_at,
            media_type: media_type_for_path(&path),
        });
}
```

把 `add_file_to_library`（`app.rs:1629-1635`）改为：

```rust
fn add_file_to_library(&mut self, path: std::path::PathBuf) {
    if is_ebook_file(&path) {
        self.add_ebook_to_library(path);
    } else if is_media_file(&path) {
        self.add_media_to_library(path);
    } else if let Ok(comic) = rust_reader_parser::parse(&path) {
        self.add_comic_to_library(comic, &path);
    }
}
```

- [ ] **Step 6: `walk_supported_files` 纳入媒体（`app.rs:1856` 附近）**

把：

```rust
} else if is_supported_comic_file(&path) || is_ebook_file(&path) {
```

改为：

```rust
} else if is_supported_comic_file(&path) || is_ebook_file(&path) || is_media_file(&path) {
```

- [ ] **Step 7: 加入库往返测试（追加到 `mod tests`，参照既有 `tmp_dir` 测试写法）**

```rust
#[test]
fn test_add_media_to_library_uses_stable_id_and_media_type() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let video = tmp_dir.path().join("clip.mp4");
    std::fs::write(&video, b"fake").unwrap();
    let mut app = test_app();
    app.add_media_to_library(video.clone());
    let entry = &app.library_view.library.entries[0];
    assert_eq!(entry.media_type, MediaType::Video);
    assert_eq!(entry.title, "clip");
    assert_eq!(entry.comic_id, rust_reader_parser::stable_comic_id(&video));
}
```

（`test_app()` 用 tests 模块里既有的构造方式；若不存在同名 helper，照搬 `app.rs:2011` 附近的构造代码。）

- [ ] **Step 8: 书库筛选纳入媒体条目（`rust-reader-app/src/views/library.rs:348-352`）**

设计文档假设"筛选逻辑自然生效无需改动"，但实际代码 `LibraryMode::Library`
只放行 `MediaType::Comic`，媒体条目会在默认书库模式不可见。把
`filtered_entries` 中的筛选改为：

```rust
let media_ok = match self.mode {
    LibraryMode::Ebooks => e.media_type == MediaType::Ebook,
    LibraryMode::Library => matches!(
        e.media_type,
        MediaType::Comic | MediaType::Video | MediaType::Audio
    ),
    _ => true,
};
```

先写失败测试（追加到 `library.rs` 的 `mod tests`，参照 `test_search_filters_by_title`
的 `LibraryView { ..Default::default() }` 构造方式）：

```rust
#[test]
fn test_library_mode_includes_media_entries() {
    let entry = |id: &str, media_type: MediaType| LibraryEntry {
        comic_id: id.to_string(),
        title: id.to_string(),
        path: PathBuf::from(format!("/{id}")),
        cover_path: None,
        added_at: 0,
        media_type,
    };
    let library = Library {
        entries: vec![
            entry("comic", MediaType::Comic),
            entry("video", MediaType::Video),
            entry("audio", MediaType::Audio),
            entry("ebook", MediaType::Ebook),
        ],
    };
    let view = LibraryView {
        library,
        mode: LibraryMode::Library,
        ..Default::default()
    };
    let filtered = view.filtered_entries(&History::default(), LibrarySort::Title);
    let ids: Vec<&str> = filtered.iter().map(|(_, e)| e.comic_id.as_str()).collect();
    assert_eq!(ids, ["audio", "comic", "video"]);
}
```

- [ ] **Step 9: 运行测试确认通过 + 全量流水线**

Run: `cargo test -p rust-reader-app media`
Expected: PASS
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 10: Commit**

```bash
git add rust-reader-app/src/app.rs rust-reader-app/src/views/library.rs
git commit -m "feat(app): recognize media files and import them into the library"
```

---

### Task 4: 轨道解析与播放状态类型

**Files:**
- Create: `rust-reader-media/src/tracks.rs`
- Create: `rust-reader-media/src/state.rs`
- Modify: `rust-reader-media/src/lib.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `rust_reader_media::tracks::{TrackKind, TrackInfo, RawTrack, parse_tracks}`
    - `TrackKind { Video, Audio, Sub }`（Debug/Clone/Copy/PartialEq/Eq）
    - `TrackInfo { id: i64, kind: TrackKind, title: Option<String>, lang: Option<String>, codec: Option<String>, selected: bool }`（Debug/Clone/PartialEq）
    - `RawTrack { id: i64, kind: String, selected: bool, albumart: bool, title: Option<String>, lang: Option<String>, codec: Option<String> }`
    - `parse_tracks(raw: Vec<RawTrack>) -> Vec<TrackInfo>`（丢弃未知 kind；保留 albumart 轨但标记为 Video）
    - `rust_reader_media::tracks::has_real_video(tracks: &[TrackInfo], raw: &[RawTrack]) -> bool`：存在被选中且非 albumart 的视频轨
  - `rust_reader_media::state::PlayerState`（字段见下，derive Default/Debug/Clone）

- [ ] **Step 1: 写失败测试 — `rust-reader-media/src/tracks.rs` 末尾 `#[cfg(test)]`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: i64, kind: &str, selected: bool, albumart: bool) -> RawTrack {
        RawTrack {
            id,
            kind: kind.to_string(),
            selected,
            albumart,
            title: Some(format!("track-{id}")),
            lang: None,
            codec: None,
        }
    }

    #[test]
    fn parse_tracks_maps_kinds_and_drops_unknown() {
        let tracks = parse_tracks(vec![
            raw(1, "video", true, false),
            raw(2, "audio", true, false),
            raw(3, "sub", false, false),
            raw(4, "attachment", false, false),
        ]);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].kind, TrackKind::Video);
        assert_eq!(tracks[1].kind, TrackKind::Audio);
        assert_eq!(tracks[2].kind, TrackKind::Sub);
        assert!(tracks[0].selected);
        assert!(!tracks[2].selected);
    }

    #[test]
    fn has_real_video_ignores_albumart() {
        let art = raw(1, "video", true, true);
        let parsed = parse_tracks(vec![art.clone()]);
        assert!(!has_real_video(&parsed, &[art]));

        let movie = raw(1, "video", true, false);
        let parsed = parse_tracks(vec![movie.clone()]);
        assert!(has_real_video(&parsed, &[movie]));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-media tracks`
Expected: 编译失败

- [ ] **Step 3: 实现 `rust-reader-media/src/tracks.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Sub,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: i64,
    pub kind: TrackKind,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
}

/// Intermediate, FFI-free track record. `player.rs` fills this from mpv nodes;
/// everything downstream stays testable.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTrack {
    pub id: i64,
    pub kind: String,
    pub selected: bool,
    pub albumart: bool,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
}

pub fn parse_tracks(raw: Vec<RawTrack>) -> Vec<TrackInfo> {
    raw.into_iter()
        .filter_map(|t| {
            let kind = match t.kind.as_str() {
                "video" => TrackKind::Video,
                "audio" => TrackKind::Audio,
                "sub" => TrackKind::Sub,
                _ => return None,
            };
            Some(TrackInfo {
                id: t.id,
                kind,
                title: t.title,
                lang: t.lang,
                codec: t.codec,
                selected: t.selected,
            })
        })
        .collect()
}

/// True when a selected, non-albumart video track exists (real video content).
pub fn has_real_video(tracks: &[TrackInfo], raw: &[RawTrack]) -> bool {
    raw.iter()
        .filter(|r| r.kind == "video" && r.selected && !r.albumart)
        .any(|r| tracks.iter().any(|t| t.id == r.id))
}
```

- [ ] **Step 4: 创建 `rust-reader-media/src/state.rs`**

```rust
use crate::tracks::TrackInfo;

#[derive(Debug, Clone, Default)]
pub struct PlayerState {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub paused: bool,
    pub volume: f64,
    pub speed: f64,
    pub tracks: Vec<TrackInfo>,
    pub current_sub: Option<i64>,
    pub current_audio: Option<i64>,
    pub has_video: bool,
    pub loaded: bool,
    pub ended: bool,
    pub error: Option<String>,
}
```

- [ ] **Step 5: 更新 `rust-reader-media/src/lib.rs`**

```rust
pub mod error;
pub mod state;
pub mod time;
pub mod tracks;

pub use error::MediaError;
pub use state::PlayerState;
pub use tracks::{TrackInfo, TrackKind};
```

- [ ] **Step 6: 运行测试确认通过 + 全量流水线**

Run: `cargo test -p rust-reader-media`
Expected: PASS
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 7: Commit**

```bash
git add rust-reader-media/
git commit -m "feat(media): add track parsing and PlayerState types"
```

---

### Task 5: `MpvPlayer` FFI 核心（命令 + 事件泵 + 属性观察）

**Files:**
- Modify: `rust-reader-media/Cargo.toml`
- Create: `rust-reader-media/build.rs`
- Create: `rust-reader-media/src/player.rs`
- Create: `rust-reader-media/examples/probe.rs`
- Modify: `rust-reader-media/src/lib.rs`

**Interfaces:**
- Consumes: Task 1（MediaError）、Task 4（PlayerState/RawTrack/parse_tracks/has_real_video）
- Produces:
  - `rust_reader_media::player::MpvPlayer`，方法：
    `new(repaint: Box<dyn Fn() + Send + Sync>) -> Result<Self, MediaError>`、
    `state(&self) -> Arc<Mutex<PlayerState>>`、
    `load_file(&self, path: &Path) -> Result<(), MediaError>`、
    `cycle_pause(&self)`、`set_paused(&self, bool)`、
    `seek_rel_sec(&self, secs: f64)`、`seek_abs_ms(&self, ms: u64)`、
    `set_volume(&self, volume: f64)`、`set_speed(&self, speed: f64)`、
    `set_sub_track(&self, id: Option<i64>)`、`set_audio_track(&self, id: i64)`、
    （以上除 `state` 外均返回 `Result<(), MediaError>`）、
    `pub(crate) fn handle(&self) -> *mut libmpv_sys::mpv_handle`
  - `PlayerState` 由事件泵线程持续更新；`time-pos`/`duration` 变化时调用 `repaint` 回调

- [ ] **Step 1: `rust-reader-media/Cargo.toml` 追加依赖 + 新建 `rust-reader-media/build.rs`**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
libmpv-sys = "3.1"
```

libmpv-sys 3.1 使用预生成的 bindings + `cargo:rustc-link-lib=mpv`，**不走 pkg-config**
（bindgen feature 才需要头文件与 pkg-config，本项目不开）。但 macOS 上
`/opt/homebrew/lib` 不在默认链接搜索路径，直接链接会报 `ld: library 'mpv' not found`，
因此新建 `rust-reader-media/build.rs`：

```rust
//! Inject the Homebrew libmpv link search path on macOS.
//!
//! libmpv-sys emits `cargo:rustc-link-lib=mpv` but Homebrew's /opt/homebrew/lib
//! (or /usr/local/lib on Intel) is not in the default linker search path.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    for dir in ["/opt/homebrew/lib", "/usr/local/lib"] {
        if std::path::Path::new(dir).join("libmpv.dylib").exists() {
            println!("cargo:rustc-link-search=native={dir}");
            return;
        }
    }
    println!(
        "cargo:warning=libmpv.dylib not found in /opt/homebrew/lib or /usr/local/lib; install mpv via Homebrew"
    );
}
```

（非 macOS 下不链接 libmpv，player 模块整体 `#[cfg(target_os = "macos")]`。）

- [ ] **Step 2: 实现 `rust-reader-media/src/player.rs`**

```rust
//! libmpv handle lifecycle, command API, event pump and property observation.
//!
//! All unsafe FFI is contained in this file. State updates flow into a shared
//! `PlayerState`; every `time-pos`/`duration` change additionally fires the
//! repaint callback injected at construction time so the egui UI refreshes
//! immediately (same fix pattern as commit b071a7b).

use crate::error::MediaError;
use crate::state::PlayerState;
use crate::tracks::{has_real_video, parse_tracks, RawTrack};
use libmpv_sys as mpv;
use std::ffi::CString;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct MpvPlayer {
    handle: *mut mpv::mpv_handle,
    state: Arc<Mutex<PlayerState>>,
}

// mpv handles are safe to command from any thread while the event loop owns
// waiting; libmpv documents concurrent command calls as safe.
unsafe impl Send for MpvPlayer {}
unsafe impl Sync for MpvPlayer {}

fn cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

impl MpvPlayer {
    pub fn new(repaint: Box<dyn Fn() + Send + Sync>) -> Result<Self, MediaError> {
        let handle = unsafe { mpv::mpv_create() };
        if handle.is_null() {
            return Err(MediaError::Init("mpv_create 返回空句柄".to_string()));
        }
        for (k, v) in [
            ("vo", "libmpv"),
            ("keep-open", "yes"),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
        ] {
            let (k, v) = (cstring(k), cstring(v));
            let rc = unsafe { mpv::mpv_set_option_string(handle, k.as_ptr(), v.as_ptr()) };
            if rc < 0 {
                unsafe { mpv::mpv_terminate_destroy(handle) };
                return Err(MediaError::Init(format!("设置 {k:?} 失败: {rc}")));
            }
        }
        let rc = unsafe { mpv::mpv_initialize(handle) };
        if rc < 0 {
            unsafe { mpv::mpv_terminate_destroy(handle) };
            return Err(MediaError::Init(format!("mpv_initialize 失败: {rc}")));
        }
        unsafe {
            let level = cstring("warn");
            mpv::mpv_request_log_messages(handle, level.as_ptr());
            // Property observation: reply_userdata doubles as the property id.
            mpv::mpv_observe_property(handle, 1, cstring("time-pos").as_ptr(), mpv::mpv_format_MPV_FORMAT_DOUBLE);
            mpv::mpv_observe_property(handle, 2, cstring("duration").as_ptr(), mpv::mpv_format_MPV_FORMAT_DOUBLE);
            mpv::mpv_observe_property(handle, 3, cstring("pause").as_ptr(), mpv::mpv_format_MPV_FORMAT_FLAG);
            mpv::mpv_observe_property(handle, 4, cstring("volume").as_ptr(), mpv::mpv_format_MPV_FORMAT_DOUBLE);
            mpv::mpv_observe_property(handle, 5, cstring("speed").as_ptr(), mpv::mpv_format_MPV_FORMAT_DOUBLE);
            mpv::mpv_observe_property(handle, 6, cstring("track-list").as_ptr(), mpv::mpv_format_MPV_FORMAT_NODE);
        }
        let state = Arc::new(Mutex::new(PlayerState {
            volume: 100.0,
            speed: 1.0,
            ..Default::default()
        }));
        let player = Self { handle, state };
        player.spawn_event_thread(repaint);
        Ok(player)
    }

    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        self.state.clone()
    }

    pub(crate) fn handle(&self) -> *mut mpv::mpv_handle {
        self.handle
    }

    fn command(&self, args: &[&str]) -> Result<(), MediaError> {
        let cargs: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut ptrs: Vec<*const std::ffi::c_char> =
            cargs.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe { mpv::mpv_command(self.handle, ptrs.as_mut_ptr()) };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: args.join(" "),
            });
        }
        Ok(())
    }

    fn set_property_string(&self, name: &str, value: &str) -> Result<(), MediaError> {
        let (n, v) = (cstring(name), cstring(value));
        let rc = unsafe { mpv::mpv_set_property_string(self.handle, n.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            return Err(MediaError::Command {
                code: rc,
                what: format!("{name}={value}"),
            });
        }
        Ok(())
    }

    pub fn load_file(&self, path: &Path) -> Result<(), MediaError> {
        let s = path.to_str().ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
        self.command(&["loadfile", s])
    }

    pub fn cycle_pause(&self) -> Result<(), MediaError> {
        self.command(&["cycle", "pause"])
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), MediaError> {
        self.set_property_string("pause", if paused { "yes" } else { "no" })
    }

    pub fn seek_rel_sec(&self, secs: f64) -> Result<(), MediaError> {
        self.command(&["seek", &format!("{secs}")])
    }

    pub fn seek_abs_ms(&self, ms: u64) -> Result<(), MediaError> {
        self.command(&["seek", &format!("{:.3}", ms as f64 / 1000.0), "absolute", "exact"])
    }

    pub fn set_volume(&self, volume: f64) -> Result<(), MediaError> {
        self.set_property_string("volume", &format!("{:.1}", volume.clamp(0.0, 100.0)))
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), MediaError> {
        self.set_property_string("speed", &format!("{:.2}", speed.clamp(0.1, 16.0)))
    }

    pub fn set_sub_track(&self, id: Option<i64>) -> Result<(), MediaError> {
        match id {
            Some(id) => self.set_property_string("sid", &id.to_string()),
            None => self.set_property_string("sid", "no"),
        }
    }

    pub fn set_audio_track(&self, id: i64) -> Result<(), MediaError> {
        self.set_property_string("aid", &id.to_string())
    }

    fn spawn_event_thread(&self, repaint: Box<dyn Fn() + Send + Sync>) {
        let handle = self.handle;
        let state = self.state.clone();
        std::thread::Builder::new()
            .name("mpv-events".to_string())
            .spawn(move || event_loop(handle, state, repaint))
            .expect("failed to spawn mpv event thread");
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        unsafe { mpv::mpv_terminate_destroy(self.handle) };
    }
}

fn event_loop(
    handle: *mut mpv::mpv_handle,
    state: Arc<Mutex<PlayerState>>,
    repaint: Box<dyn Fn() + Send + Sync>,
) {
    loop {
        let event = unsafe { mpv::mpv_wait_event(handle, -1.0) };
        if event.is_null() {
            break;
        }
        let event_id = unsafe { (*event).event_id };
        match event_id {
            mpv::mpv_event_id_MPV_EVENT_SHUTDOWN => break,
            mpv::mpv_event_id_MPV_EVENT_FILE_LOADED => {
                if let Ok(mut s) = state.lock() {
                    s.loaded = true;
                    s.ended = false;
                    s.error = None;
                }
            }
            mpv::mpv_event_id_MPV_EVENT_END_FILE => {
                let reason = unsafe {
                    let data = (*event).data as *mut mpv::mpv_event_end_file;
                    if data.is_null() { 0 } else { (*data).reason }
                };
                if let Ok(mut s) = state.lock() {
                    s.ended = true;
                    if reason == mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR {
                        s.error = Some("无法播放该文件".to_string());
                    }
                }
                repaint();
            }
            mpv::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                let (userdata, format, data) = unsafe {
                    let prop = (*event).data as *mut mpv::mpv_event_property;
                    if prop.is_null() { continue; }
                    ((*prop).reply_userdata, (*prop).format, (*prop).data)
                };
                let mut should_repaint = false;
                if let Ok(mut s) = state.lock() {
                    match userdata {
                        1 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                let secs = unsafe { *(data as *mut f64) };
                                s.position_ms = (secs * 1000.0).max(0.0) as u64;
                                should_repaint = true;
                            }
                        }
                        2 => {
                            s.duration_ms = if format == mpv::mpv_format_MPV_FORMAT_DOUBLE
                                && !data.is_null()
                            {
                                let secs = unsafe { *(data as *mut f64) };
                                Some((secs * 1000.0).max(0.0) as u64)
                            } else {
                                None
                            };
                            should_repaint = true;
                        }
                        3 => {
                            if format == mpv::mpv_format_MPV_FORMAT_FLAG && !data.is_null() {
                                s.paused = unsafe { *(data as *mut i32) } != 0;
                                should_repaint = true;
                            }
                        }
                        4 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                s.volume = unsafe { *(data as *mut f64) };
                                should_repaint = true;
                            }
                        }
                        5 => {
                            if format == mpv::mpv_format_MPV_FORMAT_DOUBLE && !data.is_null() {
                                s.speed = unsafe { *(data as *mut f64) };
                                should_repaint = true;
                            }
                        }
                        6 => {
                            if format == mpv::mpv_format_MPV_FORMAT_NODE && !data.is_null() {
                                let raw = unsafe { read_track_list(data as *mut mpv::mpv_node) };
                                s.tracks = parse_tracks(raw.clone());
                                s.current_sub = s
                                    .tracks
                                    .iter()
                                    .find(|t| t.kind == crate::tracks::TrackKind::Sub && t.selected)
                                    .map(|t| t.id);
                                s.current_audio = s
                                    .tracks
                                    .iter()
                                    .find(|t| t.kind == crate::tracks::TrackKind::Audio && t.selected)
                                    .map(|t| t.id);
                                s.has_video = has_real_video(&s.tracks, &raw);
                                should_repaint = true;
                            }
                        }
                        _ => {}
                    }
                }
                if should_repaint {
                    repaint();
                }
            }
            _ => {}
        }
    }
}

/// Walks an mpv NODE (track-list: list of maps) into FFI-free RawTrack records.
/// # Safety: `node` must point to a valid mpv_node owned by the mpv event.
unsafe fn read_track_list(node: *mut mpv::mpv_node) -> Vec<RawTrack> {
    let mut out = Vec::new();
    if node.is_null() || unsafe { (*node).format } != mpv::mpv_format_MPV_FORMAT_NODE_ARRAY {
        return out;
    }
    let list = unsafe { (*node).u.list };
    if list.is_null() {
        return out;
    }
    let (num, values) = unsafe { ((*list).num, (*list).values) };
    for i in 0..num {
        let entry = unsafe { values.offset(i as isize) };
        if entry.is_null() || unsafe { (*entry).format } != mpv::mpv_format_MPV_FORMAT_NODE_MAP {
            continue;
        }
        let map = unsafe { (*entry).u.list };
        if map.is_null() {
            continue;
        }
        let (mnum, mvalues, mkeys) = unsafe { ((*map).num, (*map).values, (*map).keys) };
        let mut t = RawTrack {
            id: 0,
            kind: String::new(),
            selected: false,
            albumart: false,
            title: None,
            lang: None,
            codec: None,
        };
        for j in 0..mnum {
            let key = unsafe {
                let k = mkeys.offset(j as isize);
                if k.is_null() || (*k).is_null() {
                    continue;
                }
                std::ffi::CStr::from_ptr(*k).to_string_lossy().into_owned()
            };
            let v = unsafe { mvalues.offset(j as isize) };
            if v.is_null() {
                continue;
            }
            let fmt = unsafe { (*v).format };
            match key.as_str() {
                "id" if fmt == mpv::mpv_format_MPV_FORMAT_INT64 => {
                    t.id = unsafe { (*v).u.int64 };
                }
                "type" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    t.kind = unsafe { node_string(v) };
                }
                "selected" if fmt == mpv::mpv_format_MPV_FORMAT_FLAG => {
                    t.selected = unsafe { (*v).u.flag } != 0;
                }
                "albumart" if fmt == mpv::mpv_format_MPV_FORMAT_FLAG => {
                    t.albumart = unsafe { (*v).u.flag } != 0;
                }
                "title" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    t.title = Some(unsafe { node_string(v) });
                }
                "lang" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    t.lang = Some(unsafe { node_string(v) });
                }
                "codec" if fmt == mpv::mpv_format_MPV_FORMAT_STRING => {
                    t.codec = Some(unsafe { node_string(v) });
                }
                _ => {}
            }
        }
        if !t.kind.is_empty() {
            out.push(t);
        }
    }
    out
}

/// # Safety: `v` must be a valid mpv_node with format MPV_FORMAT_STRING.
unsafe fn node_string(v: *mut mpv::mpv_node) -> String {
    let p = unsafe { (*v).u.string };
    if p.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }
}
```

`lib.rs` 追加（macOS 为真实实现，其余平台为同名 stub，保证 app 侧跨平台编译）：

```rust
#[cfg(target_os = "macos")]
pub mod player;
#[cfg(not(target_os = "macos"))]
pub mod player_stub;
#[cfg(not(target_os = "macos"))]
pub use player_stub as player;

pub use player::MpvPlayer;
```

并创建 `rust-reader-media/src/player_stub.rs`：

```rust
//! Non-macOS stub so app code compiles unchanged on other platforms.

use crate::error::MediaError;
use crate::state::PlayerState;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct MpvPlayer;

impl MpvPlayer {
    pub fn new(_repaint: Box<dyn Fn() + Send + Sync>) -> Result<Self, MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        Arc::new(Mutex::new(PlayerState::default()))
    }

    pub fn load_file(&self, _path: &Path) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn cycle_pause(&self) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_paused(&self, _paused: bool) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn seek_rel_sec(&self, _secs: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn seek_abs_ms(&self, _ms: u64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_volume(&self, _volume: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_speed(&self, _speed: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_sub_track(&self, _id: Option<i64>) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_audio_track(&self, _id: i64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }
}
```

- [ ] **Step 3: 编译检查（预期需按 libmpv-sys 3.1 的实际生成名微调常量/字段名）**

Run: `cargo check -p rust-reader-media`
Expected: 通过。若报错为枚举/字段命名差异（bindgen 生成名随版本可能有 `mpv_format_MPV_FORMAT_*` 等前缀变化），以 `~/.cargo/registry/src/*/libmpv-sys-3.1.0/src/lib.rs` 中的实际名称为准修正，并在本计划的 Interfaces 中同步记录最终名称。
注意：`mpv_node` 的 union 字段在 libmpv-sys 中为 `u`，其成员名（`list`/`string`/`flag`/`int64`）同样以实际生成文件为准。

- [ ] **Step 4: 创建手动验证 example `rust-reader-media/examples/probe.rs`**

```rust
//! Manual smoke test: plays a file for ~5 seconds, printing state changes.
//! Usage: cargo run -p rust-reader-media --example probe -- <media-file>

use rust_reader_media::player::MpvPlayer;
use std::sync::{Arc, Mutex};

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe <media-file>");
    let last = Arc::new(Mutex::new(String::new()));
    let last2 = last.clone();
    let player = MpvPlayer::new(Box::new(move || {
        // State is read below; repaint fires on every property change.
        let _ = &last2;
    }))
    .expect("mpv init failed");
    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");
    let state = player.state();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let s = state.lock().unwrap();
        let line = format!(
            "pos={}ms dur={:?} paused={} vol={} speed={} video={} tracks={} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.paused,
            s.volume,
            s.speed,
            s.has_video,
            s.tracks.len(),
            s.error
        );
        drop(s);
        let mut l = last.lock().unwrap();
        if *l != line {
            println!("{line}");
            *l = line;
        }
    }
    println!("probe done");
}
```

- [ ] **Step 5: 手动验证**

Run: `cargo run -p rust-reader-media --example probe -- /path/to/some.mp4`
Expected: 输出中 `dur` 变为 `Some(...)`、`pos` 递增、`tracks` ≥ 1、`err=None`；音频文件 `video=false`。
Run: `cargo run -p rust-reader-media --example probe -- /nonexistent.mp4`
Expected: `err=Some("无法播放该文件")` 出现。

- [ ] **Step 6: 全量流水线**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿（本任务无新增单测；FFI 代码由编译与 probe 验证，clippy 的 unsafe 相关警告必须清零而非 allow）。

- [ ] **Step 7: Commit**

```bash
git add rust-reader-media/
git commit -m "feat(media): MpvPlayer command API, event pump and property observation"
```

---

### Task 6: macOS 原生渲染（RenderContext + mpv_view）

**Files:**（复审修订后的最终清单）
- Modify: `rust-reader-media/Cargo.toml`（macOS deps 追加 `cgl = "0.3"`、`libc = "0.2"`）
- Create: `rust-reader-media/src/render.rs`
- Modify: `rust-reader-media/src/lib.rs`（`#[cfg(target_os = "macos")] pub mod render;`）
- Modify: `rust-reader-media/src/player.rs`、`rust-reader-media/src/player_stub.rs`（追加 `stop()`；real 版另加 `RUST_READER_MPV_LOG=1` 门控的 mpv 日志输出，调试用）
- Modify: `rust-reader-app/Cargo.toml`（macOS deps 追加 `libc = "0.2"`、`cgl = "0.3"`）
- Create: `rust-reader-app/src/platform/macos/mpv_view.rs`
- Create: `rust-reader-app/examples/probe_mpv_view.rs`
- Modify: `rust-reader-app/src/platform.rs`（macOS `pub mod macos` 内声明 `pub mod mpv_view;`；非 macOS inline stub）

**Interfaces:**（最终）
- Consumes: Task 5（`MpvPlayer::handle()`）
- Produces:
  - `rust_reader_media::render::RenderContext`：
    `unsafe fn new(player: &MpvPlayer) -> Result<Self, MediaError>`、
    `fn set_update_callback<F: Fn() + Send + Sync + 'static>(&mut self, f: F)`、
    `fn update(&self) -> u64`、
    `fn render(&self, width: i32, height: i32)`、`fn report_swap(&self)`
  - `crate::platform::macos::mpv_view::MpvNativeView`：
    `fn new<W: HasWindowHandle + HasDisplayHandle>(parent: &W, bounds: wry::Rect, player: &MpvPlayer) -> Result<Self, String>`、
    `fn set_bounds(&self, bounds: wry::Rect)`、`fn remove_from_superview(&self)`
  - `MpvPlayer::stop()`（real + stub）
  - 非 macOS：`pub mod mpv_view` stub，`new` 返回 `Err("媒体播放暂仅支持 macOS")`

**最终设计要点（复审修订后，取代草稿约定）:**

1. **预建 CGL 对象贯穿 render 生命周期**（复审 Critical 修复）。草稿在
   `copyCGLPixelFormatForDisplayMask:`/`copyCGLContextForPixelFormat:` 里每次新建
   CGL 对象，且 `RenderContext::new` 直接在 UI 线程调用——UI 线程（wgpu/Metal）没有
   current CGL context，`mpv_render_context_create` 在此条件下确定性段错误。最终形态：
   `MpvNativeView::new` 先用 `CGLChoosePixelFormat`/`CGLCreateContext` 预建一对
   pf/ctx，create/free 都通过 `with_current_context`（CGLLockContext + 保存/恢复原
   context）把预建 ctx 置为 current；pf/ctx 存入 layer 状态，两个 copy 回调返回预建
   对象并 +1 retain（copy 语义转移所有权）。
2. **LayerState + Mutex + dealloc**（复审 Important 修复，拆壳竞态）。草稿用
   `Option<Box<RenderContext>>` + ivar 裸指针，Drop 与 CA render 线程的 draw 之间存在
   UAF 窗口。最终形态：layer ivar `_rsState` 持有 `Box<LayerState>`（
   `Mutex<Option<RenderContext>>` + cgl_pf + cgl_ctx），由 `dealloc` 释放（方法接收者
   在调用期间保活，故任何在途 draw 都能安全解引用）；`drawInCGLContext` 用 `try_lock`，
   抢不到就跳帧（绝不阻塞 CA render 线程）；`Drop` 用 blocking lock 把 render context
   取出（在途 draw 被等完、新 draw 看到 `None` 跳过），再在 CGL current 下
   `update()` + free。
3. **rsDriveUpdate 驱动 `update()`**（调试中发现的架构结论）。草稿让 update 回调直接
   `setNeedsDisplay`，靠 draw 驱动一切；但 `ADVANCED_CONTROL=1` 下每个 update 回调都
   必须以 `mpv_render_context_update()` 应答，否则 vo core 卡死且不可恢复（此后
   free/命令/terminate_destroy 全部挂起），而 CoreAnimation 不给隐藏窗口调度 draw。
   最终形态：update 回调经 `performSelectorOnMainThread` 跳到 layer 的 `rsDriveUpdate`
   selector（主线程），在 mutex + CGL current 下调 `render.update()`，返回 flags 含
   `MPV_RENDER_UPDATE_FRAME(=1)` 时才 `setNeedsDisplay`；`draw_in` 只 `try_lock` +
   `render` + `report_swap`（不再调 `update()`）。advanced 保持 1：它让
   `BLOCK_FOR_TARGET_TIME=0` + `report_swap` 时序生效并启用 direct rendering（实测
   advanced=0 时 free 反而必挂）。
4. **aarch64 编译期断言**：`msg_send![this, bounds]` 经普通 `objc_msgSend` 返回
   CGRect，依赖 arm64 统一 struct-return ABI；x86_64 需 `objc_msgSend_stret`。
   用 `#[cfg(not(target_arch = "aarch64"))] compile_error!` 直接拒绝 Intel 构建，
   而不是静默编出错误代码。
5. **bindings 实际形态**（与草稿注释的出入，已按生成代码修正）：
   `MPV_RENDER_API_TYPE_OPENGL` 是 `&'static [u8; 7]`（`b"opengl\0"`，非 `&CStr`）；
   `mpv_opengl_init_params` 有第三个字段 `extra_exts`；`mpv_render_context_update`
   返回 `u64` flags。`cgl` 0.3.2 未暴露 `CGLRetainContext`/`CGLReleaseContext`，
   mpv_view.rs 自行 extern 声明（`#[link(name = "OpenGL", kind = "framework")]`）。

- [x] **Step 1: `rust-reader-media/src/render.rs`（最终代码，与仓库一致）**

```rust
//! mpv OpenGL render context. Rendering happens inside a CAOpenGLLayer
//! (app side); this type only owns the mpv render context and its update
//! callback. Mirrors mpv's examples/libmpv/cocoa/cocoabasic.m.
//!
//! Threading rules (per libmpv render.h): the render context must be created
//! before the mpv handle is destroyed and freed before it; the update
//! callback may fire from arbitrary mpv threads and must not call any mpv
//! API — app code must hop to the render thread and call
//! `mpv_render_context_update`/`render` there.

use crate::error::MediaError;
use crate::player::MpvPlayer;
use libmpv_sys as mpv;
use std::ffi::c_void;

pub struct RenderContext {
    ctx: *mut mpv::mpv_render_context,
    // Owns the closure passed to mpv's update callback; the raw pointer given
    // to mpv aliases this box, so it must stay alive until the callback is
    // unset in Drop.
    update_cb: Option<Box<Box<dyn Fn() + Send + Sync>>>,
}

// The render context is only driven from the layer's draw callback (render
// thread) plus the update callback hop; ownership can move between threads.
unsafe impl Send for RenderContext {}

// SAFETY: called by libmpv with `name` being a valid NUL-terminated GL
// function name; dlsym on RTLD_DEFAULT is safe for any symbol name.
unsafe extern "C" fn get_proc_address(
    _ctx: *mut c_void,
    name: *const std::ffi::c_char,
) -> *mut c_void {
    libc::dlsym(libc::RTLD_DEFAULT, name)
}

extern "C" fn update_trampoline(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: `ctx` is the pointer we registered in `set_update_callback`,
    // pointing at a live `Box<dyn Fn() + Send + Sync>`; the callback is unset
    // before the box is dropped. libmpv forbids calling mpv APIs from this
    // callback — we only invoke user code that hops threads.
    let cb = unsafe { &*(ctx as *const Box<dyn Fn() + Send + Sync>) };
    cb();
}

impl RenderContext {
    /// Creates an mpv render context for the OpenGL backend.
    ///
    /// # Safety
    /// The caller must guarantee an OpenGL-capable environment
    /// (macOS CAOpenGLLayer with a current CGL context) and that the returned
    /// context is dropped before `player` is destroyed.
    pub unsafe fn new(player: &MpvPlayer) -> Result<Self, MediaError> {
        let mut init = mpv::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx: std::ptr::null_mut(),
            extra_exts: std::ptr::null(),
        };
        let api = c_api_type();
        // Advanced control ON: it makes mpv_render_context_update() a hard
        // requirement after each update callback, but it also makes
        // BLOCK_FOR_TARGET_TIME=0 + report_swap timing effective and enables
        // direct rendering. The app guarantees sustained update() calls via
        // the layer's main-thread drive selector (see mpv_view.rs); letting
        // callbacks go unanswered wedges the vo core, after which free,
        // commands and terminate_destroy all hang.
        let advanced: i32 = 1;
        let mut params = [
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_ADVANCED_CONTROL,
                data: &advanced as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let mut ctx: *mut mpv::mpv_render_context = std::ptr::null_mut();
        // SAFETY: player.handle() is a valid, initialized mpv handle; `params`
        // is a valid INVALID-terminated array whose data pointers stay valid
        // for the duration of the call (render.h only requires that).
        let rc = unsafe {
            mpv::mpv_render_context_create(&mut ctx, player.handle(), params.as_mut_ptr())
        };
        if rc < 0 || ctx.is_null() {
            return Err(MediaError::Init(format!(
                "mpv_render_context_create 失败: {rc}"
            )));
        }
        Ok(Self {
            ctx,
            update_cb: None,
        })
    }

    /// Registers `f` as the frame-available callback. mpv may invoke it from
    /// arbitrary threads; it must not call mpv APIs directly.
    pub fn set_update_callback<F: Fn() + Send + Sync + 'static>(&mut self, f: F) {
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(f));
        let ptr = Box::into_raw(boxed);
        // SAFETY: self.ctx is a valid render context; `ptr` stays valid until
        // the callback is reset (Drop) because we re-box it into `update_cb`
        // below.
        unsafe {
            mpv::mpv_render_context_set_update_callback(
                self.ctx,
                Some(update_trampoline),
                ptr as *mut c_void,
            );
        }
        // Reclaim the raw pointer into owned storage; dropping any previous
        // closure is safe because mpv no longer references it after the reset
        // above.
        self.update_cb = Some(unsafe { Box::from_raw(ptr) });
    }

    /// Must be called once after each update callback fired, on a thread with
    /// the GL context current (a hard requirement because we create the
    /// context with MPV_RENDER_PARAM_ADVANCED_CONTROL=1 — letting callbacks
    /// go unanswered wedges the vo core, and free/commands/terminate then
    /// hang). Returns the raw mpv_render_update_flag bitset.
    pub fn update(&self) -> u64 {
        // SAFETY: self.ctx is valid; the caller guarantees a current GL
        // context on this thread and serialization with other mpv_render_*
        // calls (MpvNativeView's mutex).
        unsafe { mpv::mpv_render_context_update(self.ctx) }
    }

    /// Renders into the currently bound framebuffer (CAOpenGLLayer FBO 0).
    /// Must be called on the render thread with the CGL context current.
    pub fn render(&self, width: i32, height: i32) {
        let mut fbo = mpv::mpv_opengl_fbo {
            fbo: 0,
            w: width,
            h: height,
            internal_format: 0,
        };
        let flip: i32 = 0; // CAOpenGLLayer is already upright.
        let block: i32 = 0; // Never block the layer's display callback.
        let mut params = [
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo as *mut _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: &flip as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: &block as *const _ as *mut c_void,
            },
            mpv::mpv_render_param {
                type_: mpv::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: self.ctx is valid; `params` is a valid INVALID-terminated
        // array living for the call; the caller guarantees a current GL
        // context on this thread.
        unsafe { mpv::mpv_render_context_render(self.ctx, params.as_mut_ptr()) };
    }

    /// Tells mpv a swap happened; call once after each `render` for timing.
    pub fn report_swap(&self) {
        // SAFETY: self.ctx is valid; report_swap is thread-safe with a
        // current context and ignored when no video is active.
        unsafe { mpv::mpv_render_context_report_swap(self.ctx) };
    }
}

fn c_api_type() -> *mut std::ffi::c_char {
    // Bindings expose this as `&'static [u8; 7]` (b"opengl\0"), not `&CStr`;
    // the cast drops const for the C API, which only reads it during create.
    mpv::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut std::ffi::c_char
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: self.ctx is a valid render context owned by us. Unsetting
        // the callback before free guarantees mpv cannot touch `update_cb`
        // afterwards; the context is freed before the player handle per the
        // caller's lifetime contract.
        unsafe {
            mpv::mpv_render_context_set_update_callback(self.ctx, None, std::ptr::null_mut());
            mpv::mpv_render_context_free(self.ctx);
        }
    }
}
```

- [x] **Step 2: 编译检查 render.rs**

Run: `cargo check -p rust-reader-media`
Expected: 通过。`lib.rs` 追加：

```rust
#[cfg(target_os = "macos")]
pub mod render;
```

- [x] **Step 3: `rust-reader-app/Cargo.toml` 依赖调整**

`[dependencies]` 通用段追加（`state`/`tracks`/`time`/stub 在非 macOS 也要用）：

```toml
rust-reader-media = { path = "../rust-reader-media" }
```

`[target.'cfg(target_os = "macos")'.dependencies]` 段追加：

```toml
libc = "0.2"
cgl = "0.3"
```

另：`rust-reader-media/src/player.rs` 与 `player_stub.rs` 追加 `stop()`（暂停并回到
文件开头，供 MediaView 的"停止"键）；real 版 player 增加 `RUST_READER_MPV_LOG=1`
门控的 mpv 日志打印（排查 AO/解码问题用，默认关闭）。

- [x] **Step 4: 创建 `rust-reader-app/src/platform/macos/mpv_view.rs`（最终代码，与仓库一致）**

```rust
//! macOS native overlay hosting libmpv's CAOpenGLLayer, mirroring how the
//! ebook webview is overlaid on the egui window. Coordinates are top-left
//! logical points (winit's content view is flipped), same as wry child views.
//!
//! GL context lifecycle: `MpvNativeView` pre-builds one CGLPixelFormat and
//! one CGLContext. The same context is current for
//! `mpv_render_context_create`, every `drawInCGLContext` and the final
//! `mpv_render_context_free`, satisfying render.h's "same context" rule —
//! the app (wgpu/Metal) has no current GL context on the UI thread, and
//! creating the mpv render context without one segfaults. The layer's
//! `copyCGLPixelFormatForDisplayMask:`/`copyCGLContextForPixelFormat:` return
//! these pre-built objects with +1 retain (copy semantics).
//!
//! Teardown: the layer owns a `Box<LayerState>` (ivar `_rsState`, freed in
//! `dealloc`) holding `Mutex<Option<RenderContext>>`. Draws `try_lock` and
//! skip a frame on contention, so the CA render thread never blocks;
//! `Drop` takes the render context out under a blocking lock, which both
//! gates new draws (they see `None`) and waits out any in-flight draw
//! (render.h: only one mpv_render_* call at a time).
//!
//! Update drive: with MPV_RENDER_PARAM_ADVANCED_CONTROL=1 every update
//! callback must be answered by `mpv_render_context_update()` or the vo core
//! wedges (and free/commands/terminate then hang). CoreAnimation stops
//! scheduling draws for hidden windows, so the update callback hops to the
//! main thread's `rsDriveUpdate` selector, which answers `update()` under
//! the mutex with the CGL context current — draws stay optional.

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`;
// same allow as platform.rs's dock_open module.
#![allow(unexpected_cfgs)]
// The MediaView consumer lands in Task 7; until then the bin target (main.rs)
// sees this API as unused (the lib target exposes it via `pub mod platform`).
#![allow(dead_code)]

// `msg_send![this, bounds]` returns CGRect through plain objc_msgSend; the
// arm64 ABI handles struct returns uniformly, x86_64 would need
// objc_msgSend_stret. Fail the build instead of silently miscompiling.
#[cfg(not(target_arch = "aarch64"))]
compile_error!(
    "mpv_view assumes the arm64 objc_msgSend struct-return ABI (CGRect); Intel macOS needs stret handling"
);

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use rust_reader_media::render::RenderContext;
use std::ffi::c_void;
use std::sync::Mutex;

#[link(name = "OpenGL", kind = "framework")]
extern "C" {
    // Not exposed by the cgl crate (0.3.2), declared in <OpenGL/CGLContext.h>.
    fn CGLRetainContext(ctx: cgl::CGLContextObj) -> cgl::CGLContextObj;
    fn CGLReleaseContext(ctx: cgl::CGLContextObj);
}

/// Shared between the layer's draw callback and `MpvNativeView::drop`.
/// Owned by the layer via the `_rsState` ivar and freed in `dealloc`, so any
/// in-flight draw (the receiver stays alive for its whole method call) always
/// finds the pointee valid.
struct LayerState {
    /// Serializes mpv_render_* calls; `None` once teardown has taken the
    /// render context out.
    render: Mutex<Option<RenderContext>>,
    cgl_pf: cgl::CGLPixelFormatObj,
    cgl_ctx: cgl::CGLContextObj,
}

pub struct MpvNativeView {
    view: *mut Object,
    layer: *mut Object,
    state: *mut LayerState,
}

// The raw NSView/CALayer pointers are only touched from the UI thread that
// owns this value; moving ownership between threads does not alias them.
unsafe impl Send for MpvNativeView {}

fn layer_class() -> &'static Class {
    use std::sync::OnceLock;
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let superclass = Class::get("CAOpenGLLayer").expect("CAOpenGLLayer missing");
        let mut decl =
            ClassDecl::new("RustReaderMpvLayer", superclass).expect("failed to declare layer");
        decl.add_ivar::<usize>("_rsState");
        // SAFETY: each selector matches the CAOpenGLLayer delegate method
        // signature we register; the fn pointers use the C ABI and the types
        // are layout-compatible with the Objective-C declarations.
        unsafe {
            decl.add_method(
                sel!(copyCGLPixelFormatForDisplayMask:),
                copy_pixel_format as extern "C" fn(&Object, Sel, u32) -> *mut c_void,
            );
            decl.add_method(
                sel!(copyCGLContextForPixelFormat:),
                copy_context as extern "C" fn(&Object, Sel, *mut c_void) -> *mut c_void,
            );
            decl.add_method(
                sel!(canDrawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                can_draw
                    as extern "C" fn(
                        &Object,
                        Sel,
                        *mut c_void,
                        *mut c_void,
                        f64,
                        *const c_void,
                    ) -> BOOL,
            );
            decl.add_method(
                sel!(drawInCGLContext:pixelFormat:forLayerTime:displayTime:),
                draw_in
                    as extern "C" fn(&Object, Sel, *mut c_void, *mut c_void, f64, *const c_void),
            );
            decl.add_method(
                sel!(rsDriveUpdate),
                drive_update as extern "C" fn(&Object, Sel),
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&Object, Sel));
        }
        decl.register()
    })
}

/// Reads the `_rsState` ivar. Returns `None` only after `dealloc` has run
/// (which cannot race a live method call on the layer).
fn state_from_ivar(this: &Object) -> Option<&LayerState> {
    // SAFETY: `this` is a RustReaderMpvLayer instance; the ivar was declared
    // with usize layout.
    let ptr: usize = unsafe { *this.get_ivar("_rsState") };
    if ptr == 0 {
        return None;
    }
    // SAFETY: ptr came from Box::into_raw in MpvNativeView::new and is freed
    // only in dealloc; the layer (method receiver) outlives this call, so the
    // pointee is valid for its duration.
    Some(unsafe { &*(ptr as *const LayerState) })
}

/// Runs `f` with `ctx` as the current CGL context, restoring the previous
/// one. The CGL lock serializes with CoreAnimation's own use of the same
/// context on its render thread.
fn with_current_context<R>(ctx: cgl::CGLContextObj, f: impl FnOnce() -> R) -> R {
    // SAFETY: ctx is a valid CGL context owned by us.
    unsafe {
        cgl::CGLLockContext(ctx);
        let prev = cgl::CGLGetCurrentContext();
        cgl::CGLSetCurrentContext(ctx);
        let r = f();
        cgl::CGLSetCurrentContext(prev);
        cgl::CGLUnlockContext(ctx);
        r
    }
}

fn create_pixel_format() -> Option<cgl::CGLPixelFormatObj> {
    use cgl::{
        kCGLPFAAccelerated, kCGLPFADoubleBuffer, kCGLPFANoRecovery, CGLChoosePixelFormat,
        CGLPixelFormatAttribute,
    };
    // Default (legacy) GL profile is enough for libmpv.
    let attrs: [CGLPixelFormatAttribute; 4] = [
        kCGLPFAAccelerated,
        kCGLPFANoRecovery,
        kCGLPFADoubleBuffer,
        0,
    ];
    let mut pf: cgl::CGLPixelFormatObj = std::ptr::null_mut();
    let mut npix: i32 = 0;
    // SAFETY: attrs is a valid 0-terminated attribute array; pf/npix are valid
    // out-pointers that outlive the call.
    unsafe {
        CGLChoosePixelFormat(attrs.as_ptr(), &mut pf, &mut npix);
    }
    if pf.is_null() {
        None
    } else {
        Some(pf)
    }
}

extern "C" fn copy_pixel_format(this: &Object, _sel: Sel, _mask: u32) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { cgl::CGLRetainPixelFormat(state.cgl_pf) },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn copy_context(this: &Object, _sel: Sel, _pf: *mut c_void) -> *mut c_void {
    match state_from_ivar(this) {
        // +1 retain: copy semantics transfer ownership of the returned object
        // to the layer.
        Some(state) => unsafe { CGLRetainContext(state.cgl_ctx) },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn can_draw(
    _this: &Object,
    _sel: Sel,
    _ctx: *mut c_void,
    _pf: *mut c_void,
    _t: f64,
    _ts: *const c_void,
) -> BOOL {
    YES
}

extern "C" fn draw_in(
    this: &Object,
    _sel: Sel,
    _ctx: *mut c_void,
    _pf: *mut c_void,
    _t: f64,
    _ts: *const c_void,
) {
    let Some(state) = state_from_ivar(this) else {
        return;
    };
    // Never block the CA render thread: a contended lock means teardown is
    // freeing the render context — skip this frame.
    let Ok(guard) = state.render.try_lock() else {
        return;
    };
    let Some(render) = guard.as_ref() else {
        return;
    };
    // CoreAnimation has already made `_ctx` (our pre-built context, handed
    // out by copy_context) current on its render thread, as render.h requires.
    // SAFETY: `this` is a valid CALayer; `bounds`/`contentsScale` are plain
    // getters that don't transfer ownership.
    let bounds: core_graphics::geometry::CGRect = unsafe { msg_send![this, bounds] };
    let scale: f64 = unsafe { msg_send![this, contentsScale] };
    let w = (bounds.size.width * scale) as i32;
    let h = (bounds.size.height * scale) as i32;
    if w > 0 && h > 0 {
        render.render(w, h);
        render.report_swap();
    }
}

/// Runs on the main thread (via performSelectorOnMainThread from the mpv
/// update callback). Answers every update callback with
/// `mpv_render_context_update()` — a hard requirement with advanced control,
/// independent of whether CoreAnimation schedules a draw (hidden windows).
extern "C" fn drive_update(this: &Object, _sel: Sel) {
    let Some(state) = state_from_ivar(this) else {
        return;
    };
    // Blocking lock: renders are short (BLOCK_FOR_TARGET_TIME=0), and every
    // callback must be answered to keep the vo core from wedging.
    let guard = state.render.lock().unwrap_or_else(|e| e.into_inner());
    let Some(render) = guard.as_ref() else {
        return;
    };
    let flags = with_current_context(state.cgl_ctx, || render.update());
    // MPV_RENDER_UPDATE_FRAME from render.h (bit values are ABI-stable).
    const MPV_RENDER_UPDATE_FRAME: u64 = 1;
    if flags & MPV_RENDER_UPDATE_FRAME != 0 {
        // SAFETY: `this` is a live layer (performSelector retained it);
        // setNeedsDisplay takes no arguments.
        unsafe {
            let () = msg_send![this, setNeedsDisplay];
        }
    }
}

extern "C" fn dealloc(this: &Object, _sel: Sel) {
    // SAFETY: `this` is a RustReaderMpvLayer; the ivar was declared as usize.
    let ptr: usize = unsafe { *this.get_ivar("_rsState") };
    if ptr != 0 {
        // SAFETY: ptr came from Box::into_raw in MpvNativeView::new; dealloc
        // runs at most once, so the box is reclaimed exactly once. No draw can
        // be in-flight: a method receiver stays alive for its whole call.
        let state = unsafe { Box::from_raw(ptr as *mut LayerState) };
        // Defensive: if the view was leaked without Drop (e.g. mem::forget),
        // free the render context here with the CGL context current.
        if let Ok(mut guard) = state.render.lock() {
            if let Some(render) = guard.take() {
                with_current_context(state.cgl_ctx, || drop(render));
            }
        }
        // SAFETY: balanced release of the base references created in new().
        unsafe {
            CGLReleaseContext(state.cgl_ctx);
            cgl::CGLReleasePixelFormat(state.cgl_pf);
        }
    }
    // SAFETY: forwards to CAOpenGLLayer's dealloc, required by ObjC rules.
    unsafe {
        let superclass = this
            .class()
            .superclass()
            .expect("CAOpenGLLayer superclass missing");
        msg_send![super(this, superclass), dealloc]
    }
}

impl MpvNativeView {
    pub fn new<
        W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle,
    >(
        parent: &W,
        bounds: wry::Rect,
        player: &rust_reader_media::MpvPlayer,
    ) -> Result<Self, String> {
        use wry::raw_window_handle::RawWindowHandle;
        let handle = parent
            .window_handle()
            .map_err(|e| format!("无法获取窗口句柄: {e:?}"))?;
        let ns_view = match handle.as_raw() {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut Object,
            _ => return Err("媒体播放暂仅支持 macOS".to_string()),
        };
        // Pre-build the GL objects; the same CGL context backs the mpv render
        // context for its whole lifetime (create/render/free). The UI thread
        // normally has no current GL context, and mpv_render_context_create
        // requires one.
        let pf = create_pixel_format().ok_or("CGLChoosePixelFormat 失败".to_string())?;
        let mut ctx: cgl::CGLContextObj = std::ptr::null_mut();
        // SAFETY: pf is a valid pixel format object created above; ctx is a
        // valid out-pointer.
        unsafe {
            cgl::CGLCreateContext(pf, std::ptr::null_mut(), &mut ctx);
        }
        if ctx.is_null() {
            // SAFETY: balanced release of pf created above.
            unsafe { cgl::CGLReleasePixelFormat(pf) };
            return Err("CGLCreateContext 失败".to_string());
        }
        // SAFETY: ctx is current inside with_current_context (render.h
        // requirement); the render context is stored in LayerState and freed
        // before the player per MpvNativeView's lifetime contract.
        let render = with_current_context(ctx, || unsafe { RenderContext::new(player) });
        let render = match render {
            Ok(render) => render,
            Err(e) => {
                // SAFETY: balanced release of the objects created above.
                unsafe {
                    CGLReleaseContext(ctx);
                    cgl::CGLReleasePixelFormat(pf);
                }
                return Err(e.to_string());
            }
        };
        let state = Box::new(LayerState {
            render: Mutex::new(Some(render)),
            cgl_pf: pf,
            cgl_ctx: ctx,
        });
        let state_ptr = Box::into_raw(state);
        // SAFETY: all Objective-C messages below run on the UI thread that
        // owns the parent window; every object is a valid, live instance
        // (freshly allocated or the window's content view), and selectors
        // match the receivers' classes.
        let (view, layer) = unsafe {
            let frame = make_frame(&bounds);
            let view: *mut Object = msg_send![class!(NSView), alloc];
            let view: *mut Object = msg_send![view, initWithFrame: frame];
            let () = msg_send![view, setWantsLayer: YES];
            let layer: *mut Object = msg_send![layer_class(), alloc];
            let layer: *mut Object = msg_send![layer, init];
            (*layer).set_ivar::<usize>("_rsState", state_ptr as usize);
            let () = msg_send![layer, setAsynchronous: YES];
            let () = msg_send![layer, setNeedsDisplayOnBoundsChange: YES];
            // Retina: match the window's backing scale.
            let window: *mut Object = msg_send![ns_view, window];
            let scale: f64 = msg_send![window, backingScaleFactor];
            let () = msg_send![layer, setContentsScale: scale];
            let () = msg_send![view, setLayer: layer];
            let () = msg_send![ns_view, addSubview: view];
            (view, layer)
        };
        let layer_addr = layer as usize;
        // SAFETY: state_ptr is a live Box<LayerState> (owned by the layer);
        // locking is uncontended here — the view was just attached and Drop
        // has not run.
        let mut guard = unsafe { &*state_ptr }
            .render
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(render) = guard.as_mut() {
            render.set_update_callback(move || {
                // mpv calls this from arbitrary threads; hop to the main
                // thread, where rsDriveUpdate answers update() and schedules
                // a draw. `layer` stays alive: the callback is unset (via
                // RenderContext::drop, in MpvNativeView::drop) before the
                // layer is released, and performSelectorOnMainThread retains
                // the receiver until delivery.
                let layer = layer_addr as *mut Object;
                // SAFETY: per the lifetime note above, layer is valid;
                // rsDriveUpdate takes no arguments and transfers nothing.
                unsafe {
                    let () = msg_send![
                        layer,
                        performSelectorOnMainThread: sel!(rsDriveUpdate)
                        withObject: std::ptr::null_mut::<Object>()
                        waitUntilDone: NO
                    ];
                }
            });
        }
        drop(guard);
        Ok(Self {
            view,
            layer,
            state: state_ptr,
        })
    }

    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.view is a live NSView owned by us; setFrame: is a plain
        // setter that copies the rect.
        unsafe {
            let () = msg_send![self.view, setFrame: make_frame(&bounds)];
        }
    }

    pub fn remove_from_superview(&self) {
        // SAFETY: self.view is a live NSView owned by us.
        unsafe {
            let () = msg_send![self.view, removeFromSuperview];
        }
    }
}

impl Drop for MpvNativeView {
    fn drop(&mut self) {
        // SAFETY: self.view is a live NSView owned by us.
        unsafe {
            let () = msg_send![self.view, removeFromSuperview];
        }
        // Take the render context out under the lock: in-flight draws either
        // hold it (we wait them out) or will see None and skip. After this,
        // no draw can touch mpv again.
        //
        // SAFETY: self.state points at the layer-owned Box<LayerState>; the
        // layer is still alive (we release it below), so the pointee is valid.
        let render = {
            let state = unsafe { &*self.state };
            let mut guard = state.render.lock().unwrap_or_else(|e| e.into_inner());
            let render = guard.take();
            drop(guard);
            render
        };
        if let Some(render) = render {
            // Answer any still-pending update callback before free (harmless
            // if none), then free with the same CGL context current — both
            // render.h requirements.
            //
            // SAFETY: self.state is still valid (see above).
            let ctx = unsafe { &*self.state }.cgl_ctx;
            with_current_context(ctx, || {
                render.update();
                drop(render);
            });
        }
        // SAFETY: balanced release for the alloc/init retains. The layer's
        // dealloc reclaims the Box<LayerState> and the base CGL references;
        // the view also retained the layer via setLayer, keeping everything
        // valid until here.
        unsafe {
            let () = msg_send![self.layer, release];
            let () = msg_send![self.view, release];
        }
    }
}

fn make_frame(bounds: &wry::Rect) -> core_graphics::geometry::CGRect {
    use wry::dpi::{Position, Size};
    let (x, y) = match bounds.position {
        Position::Logical(p) => (p.x, p.y),
        Position::Physical(p) => (p.x as f64, p.y as f64),
    };
    let (w, h) = match bounds.size {
        Size::Logical(s) => (s.width, s.height),
        Size::Physical(s) => (s.width as f64, s.height as f64),
    };
    core_graphics::geometry::CGRect::new(
        &core_graphics::geometry::CGPoint::new(x, y),
        &core_graphics::geometry::CGSize::new(w, h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wry::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

    #[test]
    fn make_frame_logical_passthrough() {
        let rect = wry::Rect {
            position: LogicalPosition::new(10.0, 20.0).into(),
            size: LogicalSize::new(640.0, 480.0).into(),
        };
        let frame = make_frame(&rect);
        assert_eq!(frame.origin.x, 10.0);
        assert_eq!(frame.origin.y, 20.0);
        assert_eq!(frame.size.width, 640.0);
        assert_eq!(frame.size.height, 480.0);
    }

    #[test]
    fn make_frame_physical_widens_to_f64() {
        let rect = wry::Rect {
            position: PhysicalPosition::new(3, 4).into(),
            size: PhysicalSize::new(100u32, 50u32).into(),
        };
        let frame = make_frame(&rect);
        assert_eq!(frame.origin.x, 3.0);
        assert_eq!(frame.origin.y, 4.0);
        assert_eq!(frame.size.width, 100.0);
        assert_eq!(frame.size.height, 50.0);
    }
}
```

- [x] **Step 5: 在 `rust-reader-app/src/platform.rs` 的 macOS `pub mod macos` 内声明**

```rust
pub mod mpv_view;
```

非 macOS 的 `pub mod macos` 内加 inline stub：

```rust
pub mod mpv_view {
    pub struct MpvNativeView;
    impl MpvNativeView {
        pub fn new<W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle>(
            _parent: &W,
            _bounds: wry::Rect,
            _player: &rust_reader_media::MpvPlayer,
        ) -> Result<Self, String> {
            Err("媒体播放暂仅支持 macOS".to_string())
        }
        pub fn set_bounds(&self, _bounds: wry::Rect) {}
        pub fn remove_from_superview(&self) {}
    }
}
```

- [x] **Step 6: 创建 `rust-reader-app/examples/probe_mpv_view.rs` 并验收（最终代码，与仓库一致）**

probe 模拟真实 app 环境：创建线程（如同 wgpu/Metal UI 线程）**无 current CGL
context**、offscreen 窗口、主 runloop 泵驱动 `rsDriveUpdate`。修复前在此路径上
`mpv_render_context_create` 确定性段错误。

```rust
//! macOS-only probe for MpvNativeView, simulating the real app environment:
//! the creating thread (like the wgpu/Metal UI thread) has NO current CGL
//! context. Verifies that view + mpv render context creation still succeeds
//! (a pre-fix version segfaulted inside mpv_render_context_create here) and
//! that `has_video` flips to true once a video file loads.
//! Usage: cargo run -p rust-reader-app --example probe_mpv_view -- <video-file>

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`.
#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
fn main() {
    use objc::runtime::{Object, NO};
    use objc::{class, msg_send, sel, sel_impl};
    use rust_reader_app::platform::macos::mpv_view::MpvNativeView;
    use rust_reader_media::MpvPlayer;
    use std::ffi::c_void;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::raw_window_handle::{
        AppKitDisplayHandle, AppKitWindowHandle, DisplayHandle, HandleError, RawDisplayHandle,
        RawWindowHandle, WindowHandle,
    };

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_mpv_view <video-file>");

    // Offscreen window + content view; no event loop is needed for creation.
    // SAFETY: all messages go to valid AppKit objects on the main thread;
    // selectors match the receivers' classes.
    let (content_view, _window) = unsafe {
        let _app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let rect = core_graphics::geometry::CGRect::new(
            &core_graphics::geometry::CGPoint::new(0.0, 0.0),
            &core_graphics::geometry::CGSize::new(640.0, 480.0),
        );
        let window: *mut Object = msg_send![class!(NSWindow), alloc];
        let style: usize = 1 << 1; // NSWindowStyleMaskClosable
        let window: *mut Object = msg_send![window,
            initWithContentRect: rect
            styleMask: style
            backing: 2usize // NSBackingStoreBuffered
            defer: NO
        ];
        let content_view: *mut Object = msg_send![window, contentView];
        // No orderFront: the window stays offscreen on purpose — the
        // rsDriveUpdate selector answers mpv's update callbacks on the main
        // run loop independently of CoreAnimation draws, so playback health
        // must not depend on visibility.
        (content_view, window)
    };
    assert!(!content_view.is_null(), "NSWindow contentView is null");

    struct Parent(*mut Object);
    impl wry::raw_window_handle::HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let view = std::ptr::NonNull::new(self.0 as *mut c_void)
                .expect("content view pointer is non-null");
            // SAFETY: the handle borrows a live NSView that outlives it.
            Ok(unsafe {
                WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(view)))
            })
        }
    }
    impl wry::raw_window_handle::HasDisplayHandle for Parent {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            // SAFETY: the AppKit display handle carries no pointers.
            Ok(unsafe {
                DisplayHandle::borrow_raw(RawDisplayHandle::AppKit(AppKitDisplayHandle::new()))
            })
        }
    }
    let parent = Parent(content_view);

    // The crux: this thread must have no current CGL context, exactly like
    // the app's wgpu/Metal UI thread.
    // SAFETY: plain getter, no preconditions.
    assert!(
        unsafe { cgl::CGLGetCurrentContext() }.is_null(),
        "probe must start with no current CGL context"
    );

    let player = MpvPlayer::new(Box::new(|| {})).expect("mpv init failed");
    let bounds = wry::Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(640.0, 480.0).into(),
    };
    let view = MpvNativeView::new(&parent, bounds, &player).expect("MpvNativeView::new failed");
    println!("MpvNativeView created with no pre-set current CGL context");
    // SAFETY: plain getter, no preconditions.
    assert!(
        unsafe { cgl::CGLGetCurrentContext() }.is_null(),
        "current CGL context must be restored after creation"
    );

    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");
    let state = player.state();
    let mut saw_video = false;
    for _ in 0..50 {
        // Pump the main runloop for ~100ms so the queued setNeedsDisplay
        // messages and CA commits run and the layer actually draws (each
        // draw calls mpv update()/render() on the CA render thread).
        // SAFETY: runUntilDate: on the main run loop from the main thread;
        // NSDate factory returns an autoreleased object.
        unsafe {
            let run_loop: *mut Object = msg_send![class!(NSRunLoop), mainRunLoop];
            let date: *mut Object = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.1f64];
            let () = msg_send![run_loop, runUntilDate: date];
        }
        let s = state.lock().unwrap();
        saw_video |= s.has_video;
        println!(
            "pos={}ms dur={:?} video={} tracks={} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.has_video,
            s.tracks.len(),
            s.error
        );
    }
    println!("probe_mpv_view done: has_video_seen={saw_video}");
    // Teardown order: view (frees the render context) before player.
    drop(view);
    drop(player);
    println!("teardown clean");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("probe_mpv_view is macOS-only");
}
```

Run: `cargo run -q -p rust-reader-app --example probe_mpv_view -- target/tmp/probe/test_noaudio.mp4`
Expected: 打印 `MpvNativeView created with no pre-set current CGL context`，pos 持续
推进、`has_video_seen=true`、`teardown clean`，exit=0。

测试文件（不入 git）：

```bash
ffmpeg -y -f lavfi -i testsrc=duration=8:size=320x240:rate=15 -an -c:v libx264 target/tmp/probe/test_noaudio.mp4
```

**必须用无音频文件**（`-an`），原因见下方"环境注意"。

- [x] **Step 7: 全量流水线**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿（clippy 覆盖所有 examples，包括两个 probe）。

- [x] **Step 8: Commits**

```bash
git commit -m "feat(media): OpenGL render context and macOS CAOpenGLLayer overlay"   # 869f47d
git commit -m "fix(media): pre-built CGL context for render lifecycle, teardown race guard"  # ea6a55f
```

**环境注意（本机 AG06/AG03 USB 声卡，与本任务代码无关）:** mpv 的 CoreAudio AO 在该
USB 声卡上初始化挂起，导致带音频的 mp4 卡死 playloop（pos=0，随后 free /
terminate_destroy 全部 hang）；afplay 播放同文件正常。无 render context 的旧 probe
（Task 5）对同一文件同样复现，证明与 Task 6 改动无关。所有 probe 一律使用 `-an`
生成的无音频文件。

---

### Task 7: `MediaView` 最小播放（打开 mp4 出画面）

**Files:**
- Create: `rust-reader-app/src/views/media.rs`
- Modify: `rust-reader-app/src/views/mod.rs`（按既有模块声明方式追加 `pub mod media;`）
- Modify: `rust-reader-app/src/app.rs`（`View` 枚举、结构体字段、`update`、渲染分发、按键分发、`open_path`）
- Modify: `rust-reader-app/src/app.rs:2011`（测试构造函数补字段）

**Interfaces:**
- Consumes: Task 5（`MpvPlayer`）、Task 6（`MpvNativeView`）、`rust_reader_media::PlayerState`
- Produces:
  - `crate::views::media::MediaView { pub open: Option<OpenMedia> }`
  - `OpenMedia { pub path: PathBuf, pub title: String, pub player: MpvPlayer, native: MpvNativeView, pub state: Arc<Mutex<PlayerState>>, pub last: PlayerState, pub pending_resume_ms: Option<u64> }`
  - `MediaView::open(ctx: &egui::Context, parent, bounds: wry::Rect, path: PathBuf, resume_ms: Option<u64>) -> Result<(), String>`、`close()`、`update_bounds(wry::Rect)`、`sync_state()`、`ui(ctx, ui)`、`toggle_pause()`、`seek_rel(secs: f64)`
  - `View::Media`
  - `ReaderApp::open_media(&mut self, path: PathBuf)`（设置 `pending_media_open`）、`poll_media_open(ctx, frame)`

- [ ] **Step 1: 创建 `rust-reader-app/src/views/media.rs`**

```rust
use rust_reader_media::{MpvPlayer, PlayerState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct MediaView {
    pub open: Option<OpenMedia>,
}

pub struct OpenMedia {
    pub path: PathBuf,
    pub title: String,
    pub player: MpvPlayer,
    native: crate::platform::macos::mpv_view::MpvNativeView,
    pub state: Arc<Mutex<PlayerState>>,
    pub last: PlayerState,
    pub pending_resume_ms: Option<u64>,
}

impl MediaView {
    pub fn open(
        &mut self,
        ctx: &egui::Context,
        parent: &(impl wry::raw_window_handle::HasWindowHandle
              + wry::raw_window_handle::HasDisplayHandle),
        bounds: wry::Rect,
        path: PathBuf,
        resume_ms: Option<u64>,
    ) -> Result<(), String> {
        let ctx2 = ctx.clone();
        let player = MpvPlayer::new(Box::new(move || ctx2.request_repaint()))
            .map_err(|e| e.to_string())?;
        let native = crate::platform::macos::mpv_view::MpvNativeView::new(parent, bounds, &player)?;
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("未知媒体")
            .to_string();
        player.load_file(&path).map_err(|e| e.to_string())?;
        let state = player.state();
        self.open = Some(OpenMedia {
            path,
            title,
            player,
            native,
            state,
            last: PlayerState::default(),
            pending_resume_ms: resume_ms,
        });
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = None; // Drops native view, render context and player.
    }

    pub fn update_bounds(&mut self, bounds: wry::Rect) {
        if let Some(open) = self.open.as_ref() {
            open.native.set_bounds(bounds);
        }
    }

    /// Copies the latest player state into `last` for UI reads; applies a
    /// pending resume once the duration is known.
    pub fn sync_state(&mut self) {
        if let Some(open) = self.open.as_mut() {
            if let Ok(s) = open.state.lock() {
                open.last = s.clone();
            }
            if let Some(ms) = open.pending_resume_ms {
                if let Some(dur) = open.last.duration_ms {
                    if should_resume(ms, dur) {
                        let _ = open.player.seek_abs_ms(ms);
                    }
                    open.pending_resume_ms = None;
                }
            }
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.allocate_space(ui.available_size());
    }

    pub fn toggle_pause(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.cycle_pause();
        }
    }

    pub fn seek_rel(&mut self, secs: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.seek_rel_sec(secs);
        }
    }
}

/// Only resume when the saved position is not within the last 3 seconds,
/// so "reopen at the end" does not flash-finish the file.
pub fn should_resume(position_ms: u64, duration_ms: u64) -> bool {
    position_ms > 0 && position_ms + 3000 < duration_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_resume_skips_positions_near_the_end() {
        assert!(!should_resume(0, 100_000));
        assert!(should_resume(50_000, 100_000));
        assert!(!should_resume(98_000, 100_000));
        assert!(!should_resume(100_000, 100_000));
    }
}
```

- [ ] **Step 2: `views/mod.rs` 追加 `pub mod media;`（按文件内既有顺序）**

先 Read `rust-reader-app/src/views/mod.rs` 确认声明风格，再追加。

- [ ] **Step 3: app.rs 接线**

`View` 枚举（`app.rs:186`）加 `Media`。
`ReaderApp` 结构体加字段（并在 `new()` 与 tests 构造函数 `app.rs:2011` 同步初始化）：

```rust
pub media_view: MediaView,
pub pending_media_open: Option<PathBuf>,
```

`update()`（`app.rs:130`）在 ebook 迁移块之后追加：

```rust
if matches!(self.last_view, View::Media) && !matches!(self.current_view, View::Media) {
    self.media_view.close();
}
```

渲染 `match`（`app.rs:175` 附近）加：

```rust
View::Media => self.render_media(ctx),
```

`open_path`（`app.rs:1743`）加媒体分支：

```rust
fn open_path(&mut self, path: std::path::PathBuf) {
    if is_ebook_file(&path) {
        self.open_ebook(path);
    } else if is_media_file(&path) {
        self.open_media(path);
    } else {
        self.open_comic(path);
    }
}
```

新增：

```rust
fn open_media(&mut self, path: std::path::PathBuf) {
    crate::timing::log(&format!("open_media {:?}", path));
    self.pending_media_open = Some(path);
}

fn poll_media_open(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
    let Some(path) = self.pending_media_open.take() else {
        return;
    };
    let screen = ctx.screen_rect();
    let bounds = wry::Rect {
        position: wry::dpi::LogicalPosition::new(screen.min.x, screen.min.y).into(),
        size: wry::dpi::LogicalSize::new(screen.width(), screen.height()).into(),
    };
    match self.media_view.open(ctx, frame, bounds, path, None) {
        Ok(()) => {
            self.current_view = View::Media;
            self.error_message = None;
        }
        Err(e) => {
            self.error_message = Some(format!("无法打开媒体文件: {}", e));
            self.current_view = View::Library;
        }
    }
}
```

`update()` 中 `self.poll_ebook_opener(ctx, frame);` 之后加 `self.poll_media_open(ctx, frame);`。

- [ ] **Step 4: `render_media`（仿 `render_ebook`，先不做工具栏/控制条）**

```rust
fn render_media(&mut self, ctx: &egui::Context) {
    if self.media_view.open.is_none() {
        self.current_view = View::Library;
        return;
    }
    self.media_view.sync_state();
    egui::CentralPanel::default().show(ctx, |ui| {
        let rect = ui.max_rect();
        let bounds = wry::Rect {
            position: wry::dpi::LogicalPosition::new(rect.min.x, rect.min.y).into(),
            size: wry::dpi::LogicalSize::new(rect.width(), rect.height()).into(),
        };
        self.media_view.update_bounds(bounds);
        self.media_view.ui(ctx, ui);
    });
}
```

- [ ] **Step 5: 按键分发（`app.rs:1371` 的 `View::Ebook` 分支后追加）**

```rust
View::Media => {
    if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
        self.media_view.toggle_pause();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
        self.media_view.seek_rel(5.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
        self.media_view.seek_rel(-5.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        self.current_view = View::Library;
    }
}
```

- [ ] **Step 6: 编译 + 既有测试 + 流水线**

Run: `cargo test -p rust-reader-app media`
Expected: `should_resume` 测试 PASS，既有测试全过。
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 7: 手动验证（出画面里程碑）**

Run: `cargo run -p rust-reader-app`，打开一个 mp4（拖入或从库打开）。
Expected: 视频画面 + 声音；`Space` 暂停/继续；`←/→` 跳转；`Esc` 回书架；切到其他视图再回来不崩溃。音频文件（mp3）能出声（画面为黑）。

- [ ] **Step 8: Commit**

```bash
git add rust-reader-app/
git commit -m "feat(app): MediaView minimal playback with native mpv overlay"
```

---

### Task 8: 控制 UI 全量（工具栏 + seek 条 + 倍速 + 轨道 + 全屏 + 音频占位）

**Files:**
- Modify: `rust-reader-app/src/views/media.rs`
- Modify: `rust-reader-app/src/app.rs`（`render_media` 加 bars、按键分发扩展）

**Interfaces:**
- Consumes: Task 7（MediaView 全套）、`should_show_bar`（`app.rs:1408`）、`toggle_fullscreen`（`app.rs:1403`）、`format_time_ms`（Task 1）
- Produces:
  - `MediaView::seek_to_ratio(ratio: f64)`、`set_volume(v: f64)`、`cycle_speed()`、`set_speed(s: f64)`、`cycle_sub()`、`set_sub(id: Option<i64>)`、`set_audio(id: i64)`、`volume_up/down(delta)`
  - `pub fn next_speed(current: f64) -> f64`（0.5→1→1.5→2→0.5 循环，容差 0.01 匹配）
  - `pub fn clamp_seek(position_ms: i64, duration_ms: u64) -> u64`
  - `pub fn track_label(t: &TrackInfo, index: usize) -> String`（如 `#2 中文 [chi]`）
  - 音频占位：`MediaView::ui` 在 `!last.has_video` 时用 egui 画黑底 + 曲名（中央面板内），原生视图 `set_bounds` 到零尺寸

- [ ] **Step 1: 写失败测试（`views/media.rs` 的 `mod tests` 追加）**

```rust
#[test]
fn next_speed_cycles_through_options() {
    assert_eq!(next_speed(0.5), 1.0);
    assert_eq!(next_speed(1.0), 1.5);
    assert_eq!(next_speed(1.5), 2.0);
    assert_eq!(next_speed(2.0), 0.5);
    assert_eq!(next_speed(1.25), 0.5); // unknown -> restart cycle
}

#[test]
fn clamp_seek_bounds_to_duration() {
    assert_eq!(clamp_seek(-500, 10_000), 0);
    assert_eq!(clamp_seek(5_000, 10_000), 5_000);
    assert_eq!(clamp_seek(99_999, 10_000), 10_000);
}

#[test]
fn track_label_prefers_title_and_lang() {
    use rust_reader_media::tracks::{TrackInfo, TrackKind};
    let t = TrackInfo {
        id: 3,
        kind: TrackKind::Sub,
        title: Some("简中".into()),
        lang: Some("zh".into()),
        codec: None,
        selected: false,
    };
    assert_eq!(track_label(&t, 1), "#2 简中 [zh]");
    let t2 = TrackInfo { title: None, lang: None, ..t.clone() };
    assert_eq!(track_label(&t2, 0), "#1 轨道 3");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-app media`
Expected: 编译失败（函数未定义）

- [ ] **Step 3: 实现纯函数与传输方法（`views/media.rs` 追加）**

```rust
use rust_reader_media::tracks::{TrackInfo, TrackKind};

pub fn next_speed(current: f64) -> f64 {
    const OPTIONS: [f64; 4] = [0.5, 1.0, 1.5, 2.0];
    for (i, s) in OPTIONS.iter().enumerate() {
        if (current - s).abs() < 0.01 {
            return OPTIONS[(i + 1) % OPTIONS.len()];
        }
    }
    OPTIONS[0]
}

pub fn clamp_seek(position_ms: i64, duration_ms: u64) -> u64 {
    position_ms.clamp(0, duration_ms as i64) as u64
}

pub fn track_label(t: &TrackInfo, index: usize) -> String {
    let base = t.title.clone().unwrap_or_else(|| format!("轨道 {}", t.id));
    match &t.lang {
        Some(lang) => format!("#{} {} [{}]", index + 1, base, lang),
        None => format!("#{} {}", index + 1, base),
    }
}

impl MediaView {
    pub fn seek_to_ratio(&mut self, ratio: f64) {
        if let Some(open) = self.open.as_ref() {
            if let Some(dur) = open.last.duration_ms {
                let target = clamp_seek((dur as f64 * ratio.clamp(0.0, 1.0)) as i64, dur);
                let _ = open.player.seek_abs_ms(target);
            }
        }
    }

    pub fn set_volume(&mut self, v: f64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_volume(v);
        }
    }

    pub fn adjust_volume(&mut self, delta: f64) {
        let v = self.open.as_ref().map(|o| o.last.volume + delta);
        if let Some(v) = v {
            self.set_volume(v);
        }
    }

    pub fn cycle_speed(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_speed(next_speed(open.last.speed));
        }
    }

    pub fn set_sub(&mut self, id: Option<i64>) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_sub_track(id);
        }
    }

    pub fn cycle_sub(&mut self) {
        if let Some(open) = self.open.as_ref() {
            let subs: Vec<i64> = open
                .last
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Sub)
                .map(|t| t.id)
                .collect();
            if subs.is_empty() {
                return;
            }
            // Order: current -> next sub -> ... -> off -> first sub
            let next = match open.last.current_sub {
                None => Some(subs[0]),
                Some(cur) => {
                    let pos = subs.iter().position(|id| *id == cur);
                    match pos {
                        Some(i) if i + 1 < subs.len() => Some(subs[i + 1]),
                        _ => None,
                    }
                }
            };
            let _ = open.player.set_sub_track(next);
        }
    }

    pub fn set_audio(&mut self, id: i64) {
        if let Some(open) = self.open.as_ref() {
            let _ = open.player.set_audio_track(id);
        }
    }
}
```

`OpenMedia` 调用播放器前先 `sync_state`，保证 `last` 新鲜（各 transport 方法开头保持现状即可，`sync_state` 每帧由 `render_media` 调用）。

- [ ] **Step 4: 音频占位（修改 `MediaView::ui`）**

```rust
pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    ui.allocate_space(ui.available_size());
    let has_video = self.open.as_ref().map(|o| o.last.has_video).unwrap_or(true);
    if !has_video {
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        let title = self
            .open
            .as_ref()
            .map(|o| o.title.clone())
            .unwrap_or_default();
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            title,
            egui::FontId::proportional(24.0),
            egui::Color32::WHITE,
        );
    }
}
```

并在 `render_media`（app.rs）中：当 `!media_view.open.last.has_video` 时把传给 `update_bounds` 的矩形改为零尺寸（`wry::Rect` width/height = 0），避免原生黑层覆盖占位文字；`has_video` 为 true 时恢复正常矩形。

- [ ] **Step 5: `render_media` 加入工具栏与 seek 条（仿 `render_ebook` 的 `should_show_bar` 结构）**

```rust
fn render_media(&mut self, ctx: &egui::Context) {
    if self.media_view.open.is_none() {
        self.current_view = View::Library;
        return;
    }
    self.media_view.sync_state();

    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
    let screen_size = ctx.screen_rect().size();
    let show_toolbar = Self::should_show_bar(
        self.settings.show_toolbar, fullscreen, mouse_pos, screen_size, BarEdge::Top,
    );
    let show_seekbar = Self::should_show_bar(
        self.settings.show_statusbar, fullscreen, mouse_pos, screen_size, BarEdge::Bottom,
    );
    if show_toolbar {
        self.render_media_toolbar(ctx);
    }
    if show_seekbar {
        self.render_media_seekbar(ctx);
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        let rect = ui.max_rect();
        let has_video = self
            .media_view
            .open
            .as_ref()
            .map(|o| o.last.has_video)
            .unwrap_or(true);
        let bounds = if has_video {
            wry::Rect {
                position: wry::dpi::LogicalPosition::new(rect.min.x, rect.min.y).into(),
                size: wry::dpi::LogicalSize::new(rect.width(), rect.height()).into(),
            }
        } else {
            wry::Rect {
                position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
                size: wry::dpi::LogicalSize::new(0.0, 0.0).into(),
            }
        };
        self.media_view.update_bounds(bounds);
        self.media_view.ui(ctx, ui);
    });
}
```

- [ ] **Step 6: `render_media_toolbar` 与 `render_media_seekbar`**

```rust
fn render_media_toolbar(&mut self, ctx: &egui::Context) {
    let (title, tracks, current_sub, current_audio, speed, paused) = self
        .media_view
        .open
        .as_ref()
        .map(|o| {
            (
                o.title.clone(),
                o.last.tracks.clone(),
                o.last.current_sub,
                o.last.current_audio,
                o.last.speed,
                o.last.paused,
            )
        })
        .unwrap_or_default();
    egui::TopBottomPanel::top("media_toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("书架").clicked() {
                self.current_view = View::Library;
            }
            ui.separator();
            if ui.button(if paused { "播放" } else { "暂停" }).clicked() {
                self.media_view.toggle_pause();
            }
            if ui.button("-10s").clicked() {
                self.media_view.seek_rel(-10.0);
            }
            if ui.button("+10s").clicked() {
                self.media_view.seek_rel(10.0);
            }
            ui.separator();
            if ui.button(format!("{:.1}x", speed)).clicked() {
                self.media_view.cycle_speed();
            }
            ui.separator();
            let subs: Vec<(i64, String)> = tracks
                .iter()
                .filter(|t| t.kind == rust_reader_media::TrackKind::Sub)
                .enumerate()
                .map(|(i, t)| (t.id, crate::views::media::track_label(t, i)))
                .collect();
            egui::ComboBox::from_label("字幕")
                .selected_text(
                    current_sub
                        .and_then(|id| subs.iter().find(|(sid, _)| *sid == id))
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| "关闭".to_string()),
                )
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current_sub.is_none(), "关闭").clicked() {
                        self.media_view.set_sub(None);
                    }
                    for (id, label) in &subs {
                        if ui
                            .selectable_label(current_sub == Some(*id), label)
                            .clicked()
                        {
                            self.media_view.set_sub(Some(*id));
                        }
                    }
                });
            let audios: Vec<(i64, String)> = tracks
                .iter()
                .filter(|t| t.kind == rust_reader_media::TrackKind::Audio)
                .enumerate()
                .map(|(i, t)| (t.id, crate::views::media::track_label(t, i)))
                .collect();
            if audios.len() > 1 {
                egui::ComboBox::from_label("音轨")
                    .selected_text(
                        current_audio
                            .and_then(|id| audios.iter().find(|(aid, _)| *aid == id))
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, label) in &audios {
                            if ui
                                .selectable_label(current_audio == Some(*id), label)
                                .clicked()
                            {
                                self.media_view.set_audio(*id);
                            }
                        }
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("全屏").clicked() {
                    self.toggle_fullscreen(ctx);
                }
                ui.label(title);
            });
        });
    });
}

fn render_media_seekbar(&mut self, ctx: &egui::Context) {
    let (pos, dur, volume) = self
        .media_view
        .open
        .as_ref()
        .map(|o| (o.last.position_ms, o.last.duration_ms, o.last.volume))
        .unwrap_or((0, None, 100.0));
    egui::TopBottomPanel::bottom("media_seekbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(rust_reader_media::time::format_time_ms(pos));
            match dur {
                Some(dur) if dur > 0 => {
                    let mut ratio = pos as f32 / dur as f32;
                    let slider = egui::Slider::new(&mut ratio, 0.0..=1.0).show_value(false);
                    let width = (ui.available_width() - 220.0).max(60.0);
                    if ui.add_sized([width, 16.0], slider).changed() {
                        self.media_view.seek_to_ratio(ratio as f64);
                    }
                }
                _ => {
                    ui.label("--:--");
                }
            }
            ui.label(
                dur.map(rust_reader_media::time::format_time_ms)
                    .unwrap_or_else(|| "--:--".to_string()),
            );
            let mut vol = volume as f32;
            if ui
                .add_sized([100.0, 16.0], egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false))
                .changed()
            {
                self.media_view.set_volume(vol as f64);
            }
        });
    });
}
```

`unwrap_or_default()` 用于 toolbar 元组需要给元组类型加 Default（6 元组均为 Default 类型，可直接用）。

- [ ] **Step 7: 按键分发扩展（替换 Task 7 的 `View::Media` 分支）**

```rust
View::Media => {
    if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
        self.media_view.toggle_pause();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
        self.media_view.seek_rel(5.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
        self.media_view.seek_rel(-5.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::J)) {
        self.media_view.seek_rel(-10.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::L)) {
        self.media_view.seek_rel(10.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        self.media_view.adjust_volume(5.0);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        self.media_view.adjust_volume(-5.0);
    }
    for (key, speed) in [
        (egui::Key::Num1, 0.5),
        (egui::Key::Num2, 1.0),
        (egui::Key::Num3, 1.5),
        (egui::Key::Num4, 2.0),
    ] {
        if ctx.input(|i| i.key_pressed(key)) {
            if let Some(open) = self.media_view.open.as_ref() {
                let _ = open.player.set_speed(speed);
            }
        }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::V)) {
        self.media_view.cycle_sub();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::F)) {
        self.toggle_fullscreen(ctx);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if fullscreen {
            self.toggle_fullscreen(ctx);
        } else {
            self.current_view = View::Library;
        }
    }
}
```

- [ ] **Step 8: 运行测试确认通过 + 全量流水线**

Run: `cargo test -p rust-reader-app media`
Expected: PASS（next_speed / clamp_seek / track_label / should_resume）
Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 9: 手动验证**

打开含内嵌字幕与多音轨的 mkv：
Expected: 工具栏播放/暂停、±10s、倍速按钮循环、字幕下拉可选/可关、音轨下拉切换；底部 seek 可拖、时间走动、音量条生效；`F` 全屏进出；全屏时鼠标移到上/下边缘 bars 浮现；mp3 显示黑底曲名占位。

- [ ] **Step 10: Commit**

```bash
git add rust-reader-app/
git commit -m "feat(app): full media controls (seekbar, speed, tracks, fullscreen, audio placeholder)"
```

---

### Task 9: 封面生成 + 进度历史

**Files:**
- Create: `rust-reader-media/src/cover.rs`
- Modify: `rust-reader-media/src/lib.rs`
- Modify: `rust-reader-app/src/app.rs`（`on_exit`、视图迁移、`request_cover_for_library_entry`、新增 `record_media_history` / `poll_media_covers` / 字段）

**Interfaces:**
- Consumes: Task 5（MpvPlayer 内部复用 `mpv_set_option_string`）、Task 7（`should_resume`、`pending_resume_ms`）
- Produces:
  - `rust_reader_media::cover::generate_cover(input: &Path, output: &Path, timeout: Duration) -> Result<(), MediaError>`（无头 `vo=image` mpv 实例，`--frames=1 --start=10%`，阻塞）
  - `ReaderApp::record_media_history()`（`char_offset = position_ms`、`page_index = 0`）
  - `ReaderApp::request_media_cover(entry_idx)`、`poll_media_covers()`
  - 打开时从 `history` 计算 `resume_ms` 传入 `MediaView::open`

- [ ] **Step 1: 写失败测试（`rust-reader-media/src/cover.rs` 末尾）**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn cover_output_path_uses_covers_dir_and_id() {
        let p = cover_output_path(&PathBuf::from("/data/covers"), "abc123");
        assert_eq!(p, PathBuf::from("/data/covers/abc123.png"));
    }

    #[test]
    fn generate_cover_reports_missing_input() {
        let err = generate_cover(
            std::path::Path::new("/definitely/not/here.mp4"),
            std::path::Path::new("/tmp/out.png"),
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(matches!(err, MediaError::Load(_)));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p rust-reader-media cover`
Expected: 编译失败

- [ ] **Step 3: 实现 `rust-reader-media/src/cover.rs`**

```rust
//! Headless cover generation via a dedicated mpv instance using the `image`
//! video output (mpv >= 0.36). Blocking; call from a worker thread.

use crate::error::MediaError;
use libmpv_sys as mpv;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn cover_output_path(covers_dir: &Path, id: &str) -> PathBuf {
    covers_dir.join(format!("{id}.png"))
}

fn cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

pub fn generate_cover(input: &Path, output: &Path, timeout: Duration) -> Result<(), MediaError> {
    if !input.exists() {
        return Err(MediaError::Load("文件不存在".to_string()));
    }
    let handle = unsafe { mpv::mpv_create() };
    if handle.is_null() {
        return Err(MediaError::Init("mpv_create 返回空句柄".into()));
    }
    let result = (|| {
        for (k, v) in [
            ("vo", "image"),
            ("vo-image-format", "png"),
            ("frames", "1"),
            ("start", "10%"),
            ("terminal", "no"),
            ("aid", "no"),
        ] {
            let rc = unsafe {
                mpv::mpv_set_option_string(handle, cstring(k).as_ptr(), cstring(v).as_ptr())
            };
            if rc < 0 {
                return Err(MediaError::Init(format!("设置 {k} 失败: {rc}")));
            }
        }
        let rc = unsafe { mpv::mpv_initialize(handle) };
        if rc < 0 {
            return Err(MediaError::Init(format!("mpv_initialize 失败: {rc}")));
        }
        let out = output.to_string_lossy().to_string();
        // image VO writes <outfile>-000001.png style names; use vo-image-outfile template.
        let rc = unsafe {
            mpv::mpv_set_option_string(
                handle,
                cstring("vo-image-outfile").as_ptr(),
                cstring(&out).as_ptr(),
            )
        };
        if rc < 0 {
            return Err(MediaError::Init(format!("设置输出路径失败: {rc}")));
        }
        let src = input
            .to_str()
            .ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
        let args = [cstring("loadfile"), cstring(src)];
        let mut ptrs = [args[0].as_ptr(), args[1].as_ptr(), std::ptr::null()];
        let rc = unsafe { mpv::mpv_command(handle, ptrs.as_mut_ptr()) };
        if rc < 0 {
            return Err(MediaError::Load(format!("loadfile 失败: {rc}")));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() > deadline {
                return Err(MediaError::Load("封面生成超时".into()));
            }
            let ev = unsafe { mpv::mpv_wait_event(handle, 0.5) };
            if ev.is_null() {
                continue;
            }
            let id = unsafe { (*ev).event_id };
            if id == mpv::mpv_event_id_MPV_EVENT_SHUTDOWN {
                break;
            }
            if id == mpv::mpv_event_id_MPV_EVENT_END_FILE {
                let reason = unsafe {
                    let d = (*ev).data as *mut mpv::mpv_event_end_file;
                    if d.is_null() { 0 } else { (*d).reason }
                };
                if reason == mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR {
                    return Err(MediaError::Load("解码失败".into()));
                }
                break;
            }
        }
        // vo-image-outfile 可能原样写入，也可能追加帧序号；两种都兜住。
        let produced = find_produced_image(output)?;
        if produced != output {
            std::fs::rename(&produced, output)
                .map_err(|e| MediaError::Load(format!("封面写入失败: {e}")))?;
        }
        Ok(())
    })();
    unsafe { mpv::mpv_terminate_destroy(handle) };
    result
}

fn find_produced_image(template: &Path) -> Result<PathBuf, MediaError> {
    if template.exists() {
        return Ok(template.to_path_buf());
    }
    let dir = template.parent().unwrap_or_else(|| Path::new("."));
    let stem = template.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| MediaError::Load(format!("封面目录不可读: {e}")))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(stem) && n.ends_with(".png"))
                .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| MediaError::Load("未生成封面图像".into()))
}
```

`lib.rs` 追加（非 macOS 给同名 stub，app 侧 `request_media_cover` 直接报错兜底）：

```rust
#[cfg(target_os = "macos")]
pub mod cover;

#[cfg(not(target_os = "macos"))]
pub mod cover {
    use crate::error::MediaError;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub fn cover_output_path(covers_dir: &Path, id: &str) -> PathBuf {
        covers_dir.join(format!("{id}.png"))
    }

    pub fn generate_cover(
        _input: &Path,
        _output: &Path,
        _timeout: Duration,
    ) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p rust-reader-media cover`
Expected: PASS（`generate_cover_reports_missing_input` 在不存在文件上立即返回；`cover_output_path` 纯函数）
手动抽查：`cargo test -p rust-reader-media cover -- --ignored` 不需要；改为手动：
准备一个 5 秒 mp4，临时 `examples/probe` 不适用，直接在 Task 9 Step 8 手动验证里统一看效果。

- [ ] **Step 5: app.rs — `record_media_history`（仿 `record_ebook_history`，`app.rs:1462`）**

```rust
fn record_media_history(&mut self) {
    if let Some(open) = self.media_view.open.as_ref() {
        let media_id = rust_reader_parser::stable_comic_id(&open.path);
        let path = open.path.clone();
        let position_ms = open.last.position_ms;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(entry) = self
            .history
            .entries
            .iter_mut()
            .find(|h| history_matches(h, &media_id, &path))
        {
            entry.comic_id = media_id;
            entry.path = path;
            entry.page_index = 0;
            entry.char_offset = Some(position_ms as usize);
            entry.last_read_at = now;
        } else {
            self.history.entries.push(HistoryEntry {
                comic_id: media_id,
                path,
                volume_index: 0,
                page_index: 0,
                char_offset: Some(position_ms as usize),
                last_read_at: now,
            });
        }
    }
}
```

`on_exit`（`app.rs:122`）加 `self.record_media_history();`；`update()` 的 Media 迁移块（Task 7 加的）改为：

```rust
if matches!(self.last_view, View::Media) && !matches!(self.current_view, View::Media) {
    self.record_media_history();
    self.media_view.close();
}
```

- [ ] **Step 6: 打开时计算 resume（修改 `poll_media_open`）**

```rust
let resume_ms = self
    .history
    .entries
    .iter()
    .find(|h| history_matches(h, &rust_reader_parser::stable_comic_id(&path), &path))
    .and_then(|h| h.char_offset.map(|ms| ms as u64));
match self.media_view.open(ctx, frame, bounds, path, resume_ms) { ... }
```

- [ ] **Step 7: 封面请求与回收**

`ReaderApp` 加字段（`new()` 与 tests 构造函数同步）：

```rust
pub media_cover_tx: crossbeam_channel::Sender<(String, std::path::PathBuf)>,
pub media_cover_rx: crossbeam_channel::Receiver<(String, std::path::PathBuf)>,
```

`new()` 里：`let (media_cover_tx, media_cover_rx) = crossbeam_channel::unbounded();`

`request_cover_for_library_entry`（`app.rs:1680`）在函数开头插入：

```rust
let Some(entry) = self.library_view.library.entries.get(idx) else {
    return;
};
if matches!(
    entry.media_type,
    rust_reader_storage::models::MediaType::Video | rust_reader_storage::models::MediaType::Audio
) {
    self.request_media_cover(idx);
    return;
}
```

（原有 `let Some(entry) = ... else { return }` 保留其后续逻辑，注意去重。）

新增：

```rust
fn request_media_cover(&mut self, idx: usize) {
    let Some(entry) = self.library_view.library.entries.get(idx) else {
        return;
    };
    let input = entry.path.clone();
    let id = entry.comic_id.clone();
    if !input.exists() {
        return;
    }
    if !self.requested_cover_ids.insert(id.clone()) {
        return;
    }
    let covers_dir = self.covers_dir();
    std::fs::create_dir_all(&covers_dir).ok();
    let out = rust_reader_media::cover::cover_output_path(&covers_dir, &id);
    if out.exists() {
        // 封面已在磁盘上（上次生成过）：直接回填路径，无需重新生成。
        if let Some(entry) = self.library_view.library.entries.get_mut(idx) {
            entry.cover_path = Some(out);
        }
        return;
    }
    let tx = self.media_cover_tx.clone();
    std::thread::spawn(move || {
        if rust_reader_media::cover::generate_cover(
            &input,
            &out,
            std::time::Duration::from_secs(15),
        )
        .is_ok()
        {
            let _ = tx.send((id, out));
        }
    });
}

fn poll_media_covers(&mut self) {
    while let Ok((id, path)) = self.media_cover_rx.try_recv() {
        if let Some(entry) = self
            .library_view
            .library
            .entries
            .iter_mut()
            .find(|e| e.comic_id == id)
        {
            entry.cover_path = Some(path);
        }
    }
}
```

`update()` 中 `self.poll_cover_results();` 之后加 `self.poll_media_covers();`。

- [ ] **Step 8: 测试（历史数据契约，追加 `mod tests`）**

```rust
#[test]
fn test_media_history_entry_contract() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let video = tmp_dir.path().join("clip.mp4");
    std::fs::write(&video, b"fake").unwrap();
    let mut app = test_app();
    let media_id = rust_reader_parser::stable_comic_id(&video);
    app.history.entries.push(HistoryEntry {
        comic_id: media_id.clone(),
        path: video.clone(),
        volume_index: 0,
        page_index: 0,
        char_offset: Some(42_000),
        last_read_at: 1,
    });
    let entry = app
        .history
        .entries
        .iter()
        .find(|h| history_matches(h, &media_id, &video))
        .unwrap();
    assert_eq!(entry.char_offset, Some(42_000));
    assert_eq!(entry.page_index, 0);
}
```

（`record_media_history` 本身依赖真实播放器，归入手动验证；这里锁定
"媒体进度 = `char_offset` 毫秒 + `page_index = 0`"的数据契约。）

- [ ] **Step 9: 全量流水线**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。

- [ ] **Step 10: 手动验证**

- 库里新加入的 mp4 在打开过一次后，`covers/<id>.png` 生成，库卡片显示封面。
- 播到一半回书架，再打开同一文件：从离开位置继续（偏差 < 2s）。
- 播到距结尾 < 3s 回书架，再打开：从头开始。
- mp3（带专辑封面）生成封面；无封面的音频保持占位不报错。

- [ ] **Step 11: Commit**

```bash
git add rust-reader-media/ rust-reader-app/
git commit -m "feat(app): media covers via headless mpv and playback progress history"
```

---

### Task 10: 打包脚本 + 文档 + 最终验证

**Files:**
- Modify: `scripts/package-macos.sh`
- Modify: `README.md`、`AGENTS.md`、`CHANGELOG.md`

**Interfaces:**
- Consumes: 全部前序任务
- Produces: `bundle_mpv()` 打包函数；文档中的新 crate 职责与 brew 前置说明

- [ ] **Step 1: `scripts/package-macos.sh` 增加 `bundle_mpv`（在 codesign 之前调用）**

在 `info_plist` 生成之后、`echo "Signing..."` 之前插入：

```bash
frameworks_dir="${contents_dir}/Frameworks"
mkdir -p "${frameworks_dir}"

echo "Bundling libmpv and its Homebrew dependencies..."
mpv_prefix="$(brew --prefix mpv)"
libmpv="$(ls "${mpv_prefix}"/lib/libmpv.*.dylib | head -1)"
if [[ -z "${libmpv}" ]]; then
    echo "Error: libmpv not found. Run: brew install mpv" >&2
    exit 1
fi

# Recursively collect Homebrew dylib dependencies (excluding system libs).
collect_deps() {
    local lib="$1"
    otool -L "${lib}" | awk 'NR>1 {print $1}' | while read -r dep; do
        case "${dep}" in
            /opt/homebrew/*|/usr/local/*)
                if [[ ! -f "${frameworks_dir}/$(basename "${dep}")" ]]; then
                    cp "${dep}" "${frameworks_dir}/"
                    collect_deps "${dep}"
                fi
                ;;
        esac
    done
}

cp "${libmpv}" "${frameworks_dir}/"
collect_deps "${libmpv}"

# Rewrite install names to @rpath and add rpath to the executable.
for dylib in "${frameworks_dir}"/*.dylib; do
    name="$(basename "${dylib}")"
    install_name_tool -id "@rpath/${name}" "${dylib}" 2>/dev/null || true
    otool -L "${dylib}" | awk 'NR>1 {print $1}' | while read -r dep; do
        case "${dep}" in
            /opt/homebrew/*|/usr/local/*)
                install_name_tool -change "${dep}" "@rpath/$(basename "${dep}")" "${dylib}" || true
                ;;
        esac
    done
done
install_name_tool -add_rpath "@executable_path/../Frameworks" "${macos_dir}/${app_name}"
# The main binary links libmpv directly; rewrite that reference too.
otool -L "${macos_dir}/${app_name}" | awk 'NR>1 {print $1}' | while read -r dep; do
    case "${dep}" in
        /opt/homebrew/*|/usr/local/*)
            install_name_tool -change "${dep}" "@rpath/$(basename "${dep}")" "${macos_dir}/${app_name}" || true
            ;;
    esac
done
```

并把签名行改为逐文件签名后再整体签名：

```bash
echo "Signing bundled dylibs and app bundle..."
for dylib in "${frameworks_dir}"/*.dylib; do
    codesign --force --sign - "${dylib}" >/dev/null
done
codesign --force --deep --sign - "${app_bundle}" >/dev/null
```

- [ ] **Step 2: 验证打包产物**

Run: `scripts/package-macos.sh`
Expected: 构建成功，`target/release/bundle/rustReader.app/Contents/Frameworks/` 含 `libmpv.2.dylib` 及依赖。
Run: `DYLD_PRINT_LIBRARIES=1 target/release/bundle/rustReader.app/Contents/MacOS/rustReader 2>&1 | grep -c /opt/homebrew`
Expected: `0`（无任何 Homebrew 路径加载）。随后手动打开一个 mp4 确认打包内可播放。

- [ ] **Step 3: README.md 更新**

- 功能列表加：视频/音频播放（内嵌 libmpv，支持 mp4/mkv/webm/avi/mov + mp3/flac/aac/m4a/ogg/wav/opus 等）。
- 新增"前置依赖"小节：`brew install mpv`（构建与开发机需要；打包产物自带 libmpv，终端用户无需安装）。

- [ ] **Step 4: AGENTS.md 更新**

- Repository Layout 加 `rust-reader-media/` 一行：libmpv 封装（命令、事件泵、属性观察、OpenGL 渲染上下文、无头封面）。
- Key Architectural Notes 加两条：
  - **Media playback**：mpv 画面经 `CAOpenGLLayer` 原生叠加（`platform/macos/mpv_view.rs`），egui 控制条经事件泵线程 + `egui::Context::request_repaint()` 刷新；进度复用 `HistoryEntry.char_offset`（毫秒）。
  - **Packaging**：`scripts/package-macos.sh` 的 `bundle_mpv` 会把 libmpv 及 Homebrew 依赖拷入 `Contents/Frameworks` 并改写 `@rpath`。

- [ ] **Step 5: CHANGELOG.md 更新**

追加条目（按文件既有格式）：媒体播放（libmpv 内嵌）：视频/音频打开与播放、全量控制、库集成、封面、进度恢复；电子书状态栏实时刷新修复。

- [ ] **Step 6: 最终全量流水线 + 完整手动清单**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿。
手动清单（全部通过才可提交）：
- mp4 / mkv（内嵌字幕 + 多音轨）/ webm / mp3 / flac 各打开一个；
- 播放/暂停、seek（键盘 + 拖拽）、倍速（按钮 + 数字键）、字幕轨切换/关闭、音轨切换、音量、全屏进出、`Esc` 行为；
- 进度恢复（中途 / 结尾两种）；
- 封面生成（视频 10% 帧、音频专辑封面、无封面占位）；
- 漫画与电子书打开/翻页/状态栏回归抽查（既有功能不被破坏）。

- [ ] **Step 7: Commit**

```bash
git add scripts/package-macos.sh README.md AGENTS.md CHANGELOG.md
git commit -m "feat(package): bundle libmpv into .app and document media playback"
```

---

## Self-Review 记录（计划作者自查）

- **Spec 覆盖**：§3 架构→Task 1/4/5/6/7；§4 MpvPlayer→Task 5；§5 渲染→Task 6；§6 UI→Task 7/8；§7 分发→Task 3/7；§8 库→Task 2/3/9；§9 封面→Task 9；§10 进度→Task 7/9；§11 重绘→Task 5（repaint 回调）+ Task 6（update callback）；§12 错误→各任务 error_message/Result；§13 打包→Task 10；§14 依赖→Task 1/5/6；§15 测试→各任务 TDD + Task 10 手动清单；§16 文档→Task 10。
- **类型一致性**：`MpvPlayer`/`PlayerState`/`TrackInfo`/`RawTrack`/`RenderContext`/`MpvNativeView`/`MediaView`/`OpenMedia` 的方法名与字段在 Task 4-9 间一致；`should_resume(ms, dur) -> bool`、`next_speed`、`clamp_seek`、`track_label`、`cover_output_path`、`generate_cover` 签名跨任务一致。`wry::Rect` 字段已按 wry 0.55 实际的 `dpi::Position`/`Size` 枚举编写。
- **跨平台编译**：`rust-reader-media` 的 `player`/`cover` 在非 macOS 有同名 stub（Task 5/9），`mpv_view` 在 `platform.rs` 非 macOS 分支有 stub（Task 6），app 侧代码无需 cfg。
- **已修正的初稿问题**：Task 6 曾残留所有权草稿代码（已替换为 `Option<Box<RenderContext>>` + ivar 裸指针的单一最终形态）；`request_media_cover` 借用冲突（先克隆 path/id 再 `get_mut`）；封面已存在时回填 `cover_path`；`find_produced_image` 兜底 mpv 原样写入 outfile 的情况；清除了若干会触发 clippy `-D warnings` 的未使用导入。
- **风险点**（已在任务内标注）：libmpv-sys 3.1 的 bindgen 生成名（事件/格式常量、union 字段）以实际 crate 源码为准微调，Task 5 Step 3 有明确验证步骤。
