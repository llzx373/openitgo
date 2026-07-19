# OpenItGo TODO 积压批次实施计划（#58 / #59 轻量部分 / #57）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按优先级完成 `TODO.md` 剩余待办：#58 媒体核心单元测试、#59 轻量跨平台（argv 文件关联 + README 措辞）、#57 依赖升级（PDF 双栈合并、objc2 迁移、egui 升级）；egui 升级前先提交检查点，升级失败则回退。

**Architecture:** 7 个任务顺序执行，每个任务独立交付、独立提交。#58 把 `player.rs` 内联的命令参数构造与事件状态迁移抽成 FFI-free 模块（`args.rs` / `apply.rs`），让 ubuntu CI 也能跑这些测试。#57b 把 parser 的 `pdf 0.9` 迁到 `pdf-syntax`（pdf-render 链的同一解析栈），消除双解析器。#57a 把 `objc 0.2` 迁到 `objc2 0.6`（与 wry 0.55 同代）。#57c 最后做 egui 升级，带检查点/回退协议。

**Tech Stack:** Rust workspace（5 crate）、eframe/egui 0.29、libmpv（macOS）、pdf-syntax 0.5.4、objc2 0.6。

## Global Constraints

- 验证流水线（每个任务完成后必须全绿，逐条执行）：
  ```bash
  cargo fmt --all
  cargo check --workspace
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  ```
- 每个任务结束：更新 `TODO.md` 对应勾选 → commit → push（AGENTS.md 提交策略；失败回退的任务不 push）。
- 最小改动，不做无关重构；UI 文本保持中文；遇到不确定先问用户。
- **范围排除**：#59 重量部分（Windows/Linux 媒体播放实现 + 打包脚本）不在本计划内（用户已确认）；#57 中"跟踪 pdf-render beta 迭代"为持续观察项，无代码任务。
- macOS 专属行为（Dock 打开、视频层合成）CI 覆盖不到，相关任务含手动验证步骤。
- 计划中的行号来自 2026-07-18 代码快照，执行时若漂移以符号搜索为准。

---

### Task 0: 基线确认与 32.4 勾选

**Files:**
- Modify: `TODO.md:185`（32.4 勾选）

**Interfaces:**
- Consumes: 无
- Produces: 绿色基线，供后续任务对照

- [ ] **Step 1: 跑完整验证流水线确认基线绿**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部通过。若不绿，停下来先修基线或询问用户，不要在红基线上开工。

- [ ] **Step 2: 勾选 TODO.md 32.4**

`TODO.md:185` 的 `- [ ] 32.4 评估大章节分段加载` 改为：

```markdown
- [x] 32.4 评估大章节分段加载（结论：不做，见 #55 与 `docs/superpowers/reports/2026-07-17-large-chapter-loading-eval.md`）
```

- [ ] **Step 3: Commit + push**

```bash
git add TODO.md
git commit -m "docs: 勾选 32.4 大章节分段加载评估（结论不做，见 #55）"
git push
```

---

### Task 1: openitgo-media 命令参数构造纯函数化（args.rs，TODO #58 上半）

**Files:**
- Create: `openitgo-media/src/args.rs`
- Modify: `openitgo-media/src/lib.rs`（注册模块）
- Modify: `openitgo-media/src/player.rs`（改用 args 函数 + 补 cstring 测试）

**Interfaces:**
- Consumes: 无（纯新模块）
- Produces（player.rs 与 Task 2 均依赖这些签名）:

```rust
pub fn format_volume_arg(volume: f64) -> String
pub fn format_speed_arg(speed: f64) -> String
pub fn seek_abs_args(ms: u64, exact: bool) -> Vec<String>
pub fn yes_no(b: bool) -> &'static str
pub fn loop_file_arg(enabled: bool) -> &'static str
pub fn ab_loop_arg(secs: Option<f64>) -> String
pub fn sid_arg(id: Option<i64>) -> String
```

- [ ] **Step 1: 创建 `openitgo-media/src/args.rs`（实现 + 测试一次写全）**

```rust
//! mpv 命令与属性参数构造。纯函数，不触碰 libmpv FFI，
//! 因此非 macOS 平台（CI ubuntu job）也能编译与测试。

/// `volume` 属性参数：钳制到 0..=100，保留 1 位小数。
pub fn format_volume_arg(volume: f64) -> String {
    format!("{:.1}", volume.clamp(0.0, 100.0))
}

/// `speed` 属性参数：钳制到 0.1..=16.0，保留 2 位小数。
pub fn format_speed_arg(speed: f64) -> String {
    format!("{:.2}", speed.clamp(0.1, 16.0))
}

/// 绝对 seek 命令参数：`["seek", <秒.3位小数>, "absolute"]`，
/// `exact` 时追加 `"exact"`（与 player.rs 原内联实现逐字一致）。
pub fn seek_abs_args(ms: u64, exact: bool) -> Vec<String> {
    let secs = format!("{:.3}", ms as f64 / 1000.0);
    let mut args = vec!["seek".to_string(), secs, "absolute".to_string()];
    if exact {
        args.push("exact".to_string());
    }
    args
}

/// mpv 布尔属性参数。
pub fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// `loop-file` 属性参数：`inf` 无限循环，`no` 正常 EOF。
pub fn loop_file_arg(enabled: bool) -> &'static str {
    if enabled { "inf" } else { "no" }
}

/// `ab-loop-a`/`ab-loop-b` 属性参数：秒（3 位小数）或 `no` 清除。
pub fn ab_loop_arg(secs: Option<f64>) -> String {
    match secs {
        Some(v) => format!("{v:.3}"),
        None => "no".to_string(),
    }
}

/// `sid` 属性参数：轨道 id 或 `no` 关闭字幕。
pub fn sid_arg(id: Option<i64>) -> String {
    match id {
        Some(id) => id.to_string(),
        None => "no".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_arg_clamps_to_0_100_with_one_decimal() {
        assert_eq!(format_volume_arg(50.0), "50.0");
        assert_eq!(format_volume_arg(-3.0), "0.0");
        assert_eq!(format_volume_arg(250.0), "100.0");
        assert_eq!(format_volume_arg(33.33), "33.3");
    }

    #[test]
    fn speed_arg_clamps_to_0_1_16_with_two_decimals() {
        assert_eq!(format_speed_arg(1.0), "1.00");
        assert_eq!(format_speed_arg(0.05), "0.10");
        assert_eq!(format_speed_arg(20.0), "16.00");
        assert_eq!(format_speed_arg(1.25), "1.25");
    }

    #[test]
    fn seek_abs_args_builds_absolute_seek_with_optional_exact() {
        assert_eq!(seek_abs_args(61_500, false), vec!["seek", "61.500", "absolute"]);
        assert_eq!(
            seek_abs_args(61_500, true),
            vec!["seek", "61.500", "absolute", "exact"]
        );
        assert_eq!(seek_abs_args(0, false), vec!["seek", "0.000", "absolute"]);
    }

    #[test]
    fn yes_no_and_loop_file_sentinels_match_mpv() {
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
        assert_eq!(loop_file_arg(true), "inf");
        assert_eq!(loop_file_arg(false), "no");
    }

    #[test]
    fn ab_loop_arg_formats_seconds_or_no() {
        assert_eq!(ab_loop_arg(Some(12.3456)), "12.346");
        assert_eq!(ab_loop_arg(None), "no");
    }

    #[test]
    fn sid_arg_formats_track_id_or_no() {
        assert_eq!(sid_arg(Some(3)), "3");
        assert_eq!(sid_arg(None), "no");
    }
}
```

- [ ] **Step 2: 注册模块**

`openitgo-media/src/lib.rs` 在其他模块声明处加（注意 cfg_attr：这些函数在非 macOS 上只被测试使用，避免 dead_code 警告）：

```rust
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod args;
```

- [ ] **Step 3: 跑测试确认新模块通过**

Run: `cargo test -p openitgo-media`
Expected: 新增 6 个测试全 PASS（macOS 上原有测试也全过）。

- [ ] **Step 4: player.rs 改用 args 函数**

`player.rs` 顶部加 `use crate::args;`，然后逐点替换（行为必须与现状逐字一致）：

1. `set_paused`（player.rs:237-239）：`if paused { "yes" } else { "no" }` → `args::yes_no(paused)`
2. `seek_abs_ms`（player.rs:248-255）整段改为：

```rust
pub fn seek_abs_ms(&self, ms: u64, exact: bool) -> Result<(), MediaError> {
    let argv = args::seek_abs_args(ms, exact);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    self.command(&refs)
}
```

3. `set_volume`（player.rs:257-259）：`&format!("{:.1}", volume.clamp(0.0, 100.0))` → `&args::format_volume_arg(volume)`
4. `set_speed`（player.rs:261-263）：→ `&args::format_speed_arg(speed)`
5. `set_sub_track`（player.rs:265-270）：match 两分支 → `&args::sid_arg(id)`（整个 match 收敛为一行调用）
6. `set_muted`（player.rs:296-298）：→ `args::yes_no(muted)`
7. `set_loop_file`（player.rs:335-337）：`if enabled { "inf" } else { "no" }` → `args::loop_file_arg(enabled)`
8. `set_ab_loop_a` / `set_ab_loop_b`（player.rs:349-362）：两个 match → `&args::ab_loop_arg(secs)`

- [ ] **Step 5: player.rs 补 cstring 测试（文件末尾新增）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstring_passes_through_normal_strings() {
        assert_eq!(cstring("hello").to_str().unwrap(), "hello");
    }

    #[test]
    fn cstring_falls_back_to_empty_on_interior_nul() {
        assert_eq!(cstring("a\0b").to_str().unwrap(), "");
    }
}
```

注意：player.rs 整体被 `#[cfg(target_os = "macos")]` 门控，这两个测试只在 macOS 跑，属预期。

- [ ] **Step 6: 验证 + 收尾**

Run: `cargo test -p openitgo-media`，然后执行 Global Constraints 的完整流水线。
TODO.md 不勾选（#58 还差 Task 2）。
Commit: `refactor(media): 命令参数构造抽为纯函数 args.rs 并补单测（#58）`

---

### Task 2: openitgo-media 事件状态迁移纯函数化（apply.rs，TODO #58 下半）

**Files:**
- Create: `openitgo-media/src/apply.rs`
- Modify: `openitgo-media/src/lib.rs`（注册模块）
- Modify: `openitgo-media/src/player.rs`（event_loop 改用 apply 函数，userdata 常量移入 apply.rs）
- Modify: `openitgo-media/src/state.rs`（补 Default 测试）
- Modify: `AGENTS.md`（media 模块说明同步）

**Interfaces:**
- Consumes: `crate::tracks::{parse_tracks, has_real_video, RawTrack, TrackKind}`（已存在，`tracks.rs:31-57`）
- Produces（player.rs event_loop 依赖）:

```rust
pub const AUDIO_DEVICES_REPLY_USERDATA: u64 = 100;
pub const CHAPTER_LIST_REPLY_USERDATA: u64 = 101;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind { AudioDevices, ChapterList, Other }

pub fn classify_reply(userdata: u64) -> ReplyKind
pub fn apply_file_loaded(s: &mut PlayerState)
pub fn apply_end_file(s: &mut PlayerState, is_error: bool)
pub fn apply_time_pos(s: &mut PlayerState, secs: f64)
pub fn apply_duration(s: &mut PlayerState, secs: Option<f64>)
pub fn apply_track_list(s: &mut PlayerState, raw: Vec<RawTrack>)
```

- [ ] **Step 1: 创建 `openitgo-media/src/apply.rs`（实现 + 测试一次写全）**

```rust
//! mpv 事件 → `PlayerState` 的状态迁移，以及异步回复 userdata 路由。
//! 纯函数，不触碰 libmpv FFI，非 macOS 平台（CI ubuntu job）同样可测。

use crate::state::PlayerState;
use crate::tracks::{has_real_video, parse_tracks, RawTrack, TrackKind};

/// reply_userdata for the async `audio-device-list` query; observed
/// properties use 1-9 (different event type, but keep namespaces distinct).
pub const AUDIO_DEVICES_REPLY_USERDATA: u64 = 100;
/// reply_userdata for the async `chapter-list` query.
pub const CHAPTER_LIST_REPLY_USERDATA: u64 = 101;

/// 异步 GET_PROPERTY_REPLY 的分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    AudioDevices,
    ChapterList,
    /// 包括 fire-and-forget 的 userdata 0 与未知值。
    Other,
}

pub fn classify_reply(userdata: u64) -> ReplyKind {
    match userdata {
        AUDIO_DEVICES_REPLY_USERDATA => ReplyKind::AudioDevices,
        CHAPTER_LIST_REPLY_USERDATA => ReplyKind::ChapterList,
        _ => ReplyKind::Other,
    }
}

/// FILE_LOADED：新文件开始播放，重置上一文件的播放状态。
pub fn apply_file_loaded(s: &mut PlayerState) {
    s.loaded = true;
    s.ended = false;
    s.error = None;
    s.chapter = None;
    s.chapters.clear();
}

/// END_FILE：播放结束；`is_error`（mpv reason == ERROR）时记录错误。
pub fn apply_end_file(s: &mut PlayerState, is_error: bool) {
    s.ended = true;
    if is_error {
        s.error = Some("无法播放该文件".to_string());
    }
}

/// time-pos 属性（秒）→ position_ms；负值钳 0。
pub fn apply_time_pos(s: &mut PlayerState, secs: f64) {
    s.position_ms = (secs * 1000.0).max(0.0) as u64;
}

/// duration 属性（秒）→ duration_ms；格式无效（None）时清空。
pub fn apply_duration(s: &mut PlayerState, secs: Option<f64>) {
    s.duration_ms = secs.map(|v| (v * 1000.0).max(0.0) as u64);
}

/// track-list 属性：重建轨道列表并派生当前字幕/音轨与有无真实视频。
pub fn apply_track_list(s: &mut PlayerState, raw: Vec<RawTrack>) {
    s.tracks = parse_tracks(raw.clone());
    s.current_sub = s
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Sub && t.selected)
        .map(|t| t.id);
    s.current_audio = s
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio && t.selected)
        .map(|t| t.id);
    s.has_video = has_real_video(&s.tracks, &raw);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_track(id: i64, kind: &str, selected: bool, albumart: bool) -> RawTrack {
        RawTrack {
            id,
            kind: kind.to_string(),
            selected,
            albumart,
            title: None,
            lang: None,
            codec: None,
        }
    }

    #[test]
    fn classify_reply_routes_known_userdata() {
        assert_eq!(
            classify_reply(AUDIO_DEVICES_REPLY_USERDATA),
            ReplyKind::AudioDevices
        );
        assert_eq!(
            classify_reply(CHAPTER_LIST_REPLY_USERDATA),
            ReplyKind::ChapterList
        );
        assert_eq!(classify_reply(0), ReplyKind::Other);
        assert_eq!(classify_reply(999), ReplyKind::Other);
    }

    #[test]
    fn file_loaded_resets_playback_state() {
        let mut s = PlayerState {
            loaded: false,
            ended: true,
            error: Some("旧错误".to_string()),
            chapter: Some(2),
            chapters: vec!["旧章节".to_string()],
            ..Default::default()
        };
        apply_file_loaded(&mut s);
        assert!(s.loaded);
        assert!(!s.ended);
        assert_eq!(s.error, None);
        assert_eq!(s.chapter, None);
        assert!(s.chapters.is_empty());
    }

    #[test]
    fn end_file_sets_error_only_on_error_reason() {
        let mut s = PlayerState::default();
        apply_end_file(&mut s, false);
        assert!(s.ended);
        assert_eq!(s.error, None);

        let mut s = PlayerState::default();
        apply_end_file(&mut s, true);
        assert!(s.ended);
        assert_eq!(s.error.as_deref(), Some("无法播放该文件"));
    }

    #[test]
    fn time_pos_clamps_negative_and_truncates_to_ms() {
        let mut s = PlayerState::default();
        apply_time_pos(&mut s, 61.5);
        assert_eq!(s.position_ms, 61_500);
        apply_time_pos(&mut s, -2.0);
        assert_eq!(s.position_ms, 0);
    }

    #[test]
    fn duration_none_clears_and_some_converts() {
        let mut s = PlayerState::default();
        apply_duration(&mut s, Some(90.0));
        assert_eq!(s.duration_ms, Some(90_000));
        apply_duration(&mut s, None);
        assert_eq!(s.duration_ms, None);
    }

    #[test]
    fn track_list_derives_current_tracks_and_video_presence() {
        let mut s = PlayerState::default();
        apply_track_list(
            &mut s,
            vec![
                raw_track(1, "video", true, false),
                raw_track(2, "audio", true, false),
                raw_track(3, "sub", false, false),
                raw_track(4, "sub", true, false),
            ],
        );
        assert_eq!(s.current_audio, Some(2));
        assert_eq!(s.current_sub, Some(4));
        assert!(s.has_video);

        // 专辑封面不算真实视频；无选中轨道时派生为 None
        let mut s = PlayerState::default();
        apply_track_list(
            &mut s,
            vec![
                raw_track(1, "video", true, true),
                raw_track(2, "audio", true, false),
            ],
        );
        assert!(!s.has_video);
        assert_eq!(s.current_sub, None);
    }
}
```

- [ ] **Step 2: 注册模块**

`openitgo-media/src/lib.rs` 加：

```rust
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod apply;
```

- [ ] **Step 3: 跑测试确认新模块通过**

Run: `cargo test -p openitgo-media`
Expected: 新增 6 个测试全 PASS。

- [ ] **Step 4: player.rs event_loop 改用 apply 函数**

顶部 import 改为 `use crate::apply::{self, AUDIO_DEVICES_REPLY_USERDATA, CHAPTER_LIST_REPLY_USERDATA};`，删除 `player.rs:45-49` 的两个本地常量定义（注释含义已移入 apply.rs）。逐点替换（行号来自探索快照，执行时按符号定位）：

1. FILE_LOADED 重置块（player.rs:468-474）→ `apply::apply_file_loaded(&mut s);`
2. END_FILE 块（player.rs:490-507）：FFI 边界先算 `let is_error = (*event).error == mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR;`（按现有 reason 读取代码的实际写法），然后 `apply::apply_end_file(&mut s, is_error);`；原有 repaint 调用保留不动。
3. time-pos（player.rs:565-571）→ `apply::apply_time_pos(&mut s, secs);`（secs 的读取/判空留在原地）
4. duration（player.rs:573-581）→ `apply::apply_duration(&mut s, secs_opt);`（secs_opt: Option<f64> 由现有"格式无效/空指针"判定逻辑得出）
5. track-list（player.rs:605-629）→ `apply::apply_track_list(&mut s, raw);`（替换 parse_tracks + 两个 find + has_real_video 四行内联）
6. GET_PROPERTY_REPLY 分发（player.rs:509-546）：userdata 的 `if/else if` 改为 `match apply::classify_reply(userdata)`，`ReplyKind::Other` 分支保持原忽略行为。

- [ ] **Step 5: state.rs 补 Default 测试（文件末尾新增）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_idle() {
        let s = PlayerState::default();
        assert!(!s.loaded);
        assert!(!s.ended);
        assert!(s.error.is_none());
        assert!(s.duration_ms.is_none());
        assert!(s.audio_devices.is_none());
        assert!(s.chapters.is_empty());
        assert!(s.tracks.is_empty());
    }
}
```

- [ ] **Step 6: AGENTS.md 同步**

`AGENTS.md` 两处：
1. Repository Layout 的 `openitgo-media/` 条目末尾补一句：`args.rs`/`apply.rs` 为 FFI-free 纯函数模块（命令参数构造、事件状态迁移），ubuntu CI 可测。
2. "MpvPlayer observe/userdata 分配" 条目：`userdata 100/101` 常量位置由 player.rs 改为 `apply.rs`（`AUDIO_DEVICES_REPLY_USERDATA`/`CHAPTER_LIST_REPLY_USERDATA`），分配规则不变。

- [ ] **Step 7: 验证 + 收尾**

Run: `cargo test -p openitgo-media`，然后完整流水线。
TODO.md 勾选 58。
Commit: `refactor(media): 事件状态迁移抽为纯函数 apply.rs 并补单测（#58）`

---

### Task 3: argv 文件关联打开 + README 平台措辞（TODO #59 轻量部分）

**Files:**
- Modify: `openitgo-app/src/app.rs:415-424`（ReaderApp::new + 新增自由函数）
- Modify: `openitgo-app/src/app.rs` 现有 `#[cfg(test)] mod tests`（约 :4494，追加测试）
- Modify: `README.md:3`

**Interfaces:**
- Consumes: `ReaderApp::open_path`（app.rs:3149-3157，私有但同模块可调）
- Produces:

```rust
fn initial_open_path(
    env_open: Option<std::path::PathBuf>,
    arg1: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf>
```

- [ ] **Step 1: 新增 `initial_open_path` 并接入 ReaderApp::new**

在 `ReaderApp::new` 附近（app.rs:414 前）加自由函数：

```rust
/// 启动时要打开的路径：优先 `OPENITGO_OPEN` 环境变量，其次第一个命令行参数
/// （Windows/Linux 文件关联经 argv 传入；macOS 走 Apple Event，argv 无路径，
/// 偶发的 `-psn_*` 等系统参数会被 `exists()` 检查天然过滤）。
fn initial_open_path(
    env_open: Option<std::path::PathBuf>,
    arg1: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    [env_open, arg1].into_iter().flatten().find(|p| p.exists())
}
```

`ReaderApp::new`（app.rs:415-424）改为：

```rust
pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
    let mut app = Self::default();
    let env_open = std::env::var("OPENITGO_OPEN").ok().map(std::path::PathBuf::from);
    let arg1 = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if let Some(path) = initial_open_path(env_open, arg1) {
        app.open_path(path);
    }
    app
}
```

- [ ] **Step 2: 在 app.rs 现有 tests mod 中追加测试**

```rust
#[test]
fn initial_open_path_prefers_env_over_argv() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("env.cbz");
    let arg_file = dir.path().join("arg.cbz");
    std::fs::write(&env_file, b"x").unwrap();
    std::fs::write(&arg_file, b"x").unwrap();
    assert_eq!(
        initial_open_path(Some(env_file.clone()), Some(arg_file)),
        Some(env_file)
    );
}

#[test]
fn initial_open_path_falls_back_to_argv_and_skips_missing() {
    let dir = tempfile::tempdir().unwrap();
    let arg_file = dir.path().join("arg.cbz");
    std::fs::write(&arg_file, b"x").unwrap();
    assert_eq!(
        initial_open_path(None, Some(arg_file.clone())),
        Some(arg_file)
    );
    assert_eq!(
        initial_open_path(None, Some(std::path::PathBuf::from("/nonexistent/xx.cbz"))),
        None
    );
    assert_eq!(initial_open_path(None, None), None);
}

#[cfg(unix)]
#[test]
fn initial_open_path_accepts_non_utf8_argv() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("comic.cbz");
    std::fs::write(&file, b"x").unwrap();
    // 非 UTF-8 字节序列构成的 argv 不再 panic（回归：env::args() 会 panic）
    let raw = std::ffi::OsString::from_vec(b"\xff\xfe.cbz".to_vec());
    assert_eq!(initial_open_path(None, Some(file.clone())), Some(file));
    assert_eq!(
        initial_open_path(None, Some(std::path::PathBuf::from(raw))),
        None
    );
}
```

（`tempfile` 已在 openitgo-app 的 dev-dependencies；参数为修订后的 `Option<PathBuf>`，第 3 个测试是任务尾部"修订"注所述 args_os 审查修复补的非 UTF-8 argv 回归（`#[cfg(unix)]`）。）

- [ ] **Step 3: 跑测试**

Run: `cargo test -p openitgo-app initial_open_path`
Expected: 3 个新测试 PASS。

- [ ] **Step 4: README.md 措辞对齐**

`README.md:3` 删除无限定词"跨平台"，改为（该行其余内容保留）：

```markdown
一款使用 Rust + egui + wgpu 构建的桌面漫画/小说阅读器（完整支持 macOS；Windows/Linux 可编译运行，文件关联打开已支持，媒体播放暂仅 macOS），支持国漫（左→右）…
```

（"…"处保留原文后半句；只改平台限定部分。）

- [ ] **Step 5: 验证 + 收尾**

完整流水线。TODO.md 勾选 59 并标注：`（轻量部分完成；非 macOS 媒体播放 + 打包脚本出范围，未做）`。
Commit: `feat(app): 非 macOS 平台启动时从 argv 打开文件，README 平台措辞对齐（#59）`

> 修订（2026-07-18）：argv 读取由 env::args() 改为 env::args_os()（审查发现：非 UTF-8 参数 panic，用户裁决），initial_open_path 签名相应改为 Option<PathBuf>。

---

### Task 4: parser PDF 栈合并到 pdf-syntax（TODO #57 PDF 子项）

**Files:**
- Modify: `openitgo-parser/Cargo.toml`（`pdf = "0.9"` → `pdf-syntax = "0.5"`）
- Modify: `openitgo-parser/src/pdf.rs`（重写 parse，删 map_pdf_error）
- Modify: `openitgo-parser/tests/integration.rs`（新增损坏 PDF 用例）
- Modify: `openitgo-parser/Cargo.toml` `[dev-dependencies]` 加 `tempfile = "3.14"`（若无）

**Interfaces:**
- Consumes: `pdf_syntax::Pdf`（0.5.4，`Pdf::new(data: impl Into<PdfData>) -> Result<Pdf, LoadPdfError>`；`From<Vec<u8>> for PdfData` 已确认存在；`pdf.pages().len()` 经 `Deref<Target=[Page]>` 可用）
- Produces: `PdfParser` 公开接口完全不变（`Parser` trait 实现，调用方零改动）

背景：parser 只用 `pdf 0.9` 的 `num_pages()` 一个调用（pdf.rs:20），却拖入 21 个依赖；pdf-render 链已有 pdf-syntax 0.5.4 解析栈。`LoadPdfError` 只 derive Debug（无 Display），错误串必须用 `format!("{e:?}")`。

- [ ] **Step 1: 确认 parser 内 `pdf` crate 只有 pdf.rs 一个使用点**

Run: `grep -rn "pdf::" openitgo-parser/src --include="*.rs" | grep -v "pdf_syntax" | grep -v "^openitgo-parser/src/pdf.rs"`
Expected: 无输出。若有输出，停下来评估遗漏的使用点。

- [ ] **Step 2: 改写 `openitgo-parser/src/pdf.rs`**

```rust
use crate::traits::{ParseError, Parser};
use openitgo_core::models::{Comic, Page, PageSource, Volume};
use pdf_syntax::Pdf;
use std::path::Path;

pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    fn parse(path: &Path) -> Result<Comic, ParseError> {
        let data = std::fs::read(path).map_err(ParseError::Io)?;
        // LoadPdfError 只实现 Debug（无 Display），用 {:?} 记录。
        let pdf = Pdf::new(data).map_err(|e| ParseError::InvalidArchive(format!("{e:?}")))?;

        let num_pages = pdf.pages().len();
        if num_pages == 0 {
            return Err(ParseError::NoPages);
        }

        let document = path.to_path_buf();
        let pages: Vec<Page> = (0..num_pages)
            .map(|page_number| Page {
                index: page_number,
                source: PageSource::PdfPage {
                    document: document.clone(),
                    page_number,
                },
            })
            .collect();

        let title = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(Comic {
            id: crate::stable_comic_id(path),
            title,
            path: document,
            volumes: vec![Volume {
                title: "Default".to_string(),
                pages,
            }],
        })
    }
}
```

（文件尾部现有 `#[cfg(test)] mod tests` 原样保留；`map_pdf_error` 整个删除。）

- [ ] **Step 3: Cargo.toml 换依赖**

`openitgo-parser/Cargo.toml`：`pdf = "0.9"` 改为 `pdf-syntax = "0.5"`（与 Cargo.lock 中 pdf-render 链的 0.5.4 统一）。`[dev-dependencies]` 若无则加：

```toml
[dev-dependencies]
tempfile = "3.14"
```

- [ ] **Step 4: 编译 + 跑 parser 测试**

Run: `cargo test -p openitgo-parser`
Expected: `test_parse_pdf`、`test_parse_sample_pdf` 等原有测试不改一字全部 PASS（页数语义与 pdf 0.9 一致）。

- [ ] **Step 5: 新增损坏 PDF 集成测试**

`openitgo-parser/tests/integration.rs` 追加（import 风格沿用该文件现状）：

```rust
#[test]
fn test_parse_corrupt_pdf_returns_invalid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.pdf");
    std::fs::write(&path, b"definitely not a pdf").unwrap();
    let err = parse(&path).unwrap_err();
    assert!(matches!(err, ParseError::InvalidArchive(_)));
}
```

Run: `cargo test -p openitgo-parser --test integration`
Expected: 新测试 PASS（若实际返回 `NoPages` 而非 `InvalidArchive`，说明 pdf-syntax 对该输入走了空文档路径，按实际行为调整断言并在测试中注释说明）。

- [ ] **Step 6: 确认 pdf 0.9 移出依赖树**

Run: `grep -A2 'name = "pdf"$' Cargo.lock`
Expected: 无输出（`pdf 0.9.1` 已消失）；`cargo tree -p openitgo-parser | grep pdf` 只剩 `pdf-syntax`。

- [ ] **Step 7: 验证 + 收尾**

完整流水线。TODO.md 勾选 57 中"合并双 PDF 栈"子句（57 整项等 Task 5/6 后再整体勾选，本步在 57 描述行内对该子句做标注，如 `（PDF 栈已合并）`）。
Commit: `refactor(parser): PDF 页数解析从 pdf 0.9 迁到 pdf-syntax，消除双解析栈（#57）`

---

### Task 5: objc 0.2 → objc2 0.6 迁移（TODO #57 objc 子项）

**Files:**
- Modify: `openitgo-app/Cargo.toml:35`（删 `objc = "0.2"`，加 objc2 系依赖）
- Modify: `openitgo-app/src/platform.rs:285-683`（dock_open 内联模块）
- Modify: `openitgo-app/src/platform/macos/mpv_view.rs`（ClassDecl 子类化 + 59 处 msg_send!）
- Modify: `openitgo-app/examples/probe_visible.rs`、`probe_mpv_view.rs`、`probe_video_overlay.rs`（机械替换）

**Interfaces:**
- Consumes: Cargo.lock 已有的 objc2 0.6.4 / objc2-foundation 0.3.2 / objc2-app-kit 0.3.2（wry 0.55 链带入）；objc2-quartz-core 0.3.x（可能需新增直接依赖）
- Produces: `dock_open` 与 `MpvNativeView` 公开接口完全不变（`main.rs`、`app.rs` 调用方零改动）

背景：objc 0.2 已无人维护，objc2 是生态主线。迁移收益：删除 `mpv_view.rs:46-52` 的 aarch64-only `compile_error!` 守卫（objc2 的 `msg_send!` 按 Encode 签名自动处理 stret，Intel macOS 解锁）。swizzle `setDelegate:` 的根本动机（winit 自建 delegate）在 objc2 下依旧成立，**保留 runtime 级 swizzle 方案**，不改为 define_class 自定义 delegate。

- [ ] **Step 1: objc2 API 可用性盘点（决定绑定层还是裸 runtime）**

在 `~/.cargo/registry/src/index.crates.io-*/` 下查 objc2-0.6.x、objc2-foundation-0.3.x、objc2-app-kit-0.3.x、objc2-quartz-core-0.3.x 源码，逐项确认并记录结论到 commit message：

- `objc2::runtime` 是否导出 `class_addMethod` / `method_exchangeImplementations` / `class_getInstanceMethod` / `class_getName` / `object_getClass` / `AnyClass` / `AnyObject` / `Sel` / `Bool` / `Imp`
- `objc2::{class!, msg_send!, sel!}` 宏
- `objc2_foundation::{NSString, NSURL, NSArray}`、`objc2_app_kit::{NSApplication, NSColor}`
- `objc2_quartz_core` 是否绑定 `CATransaction` / `CATextLayer` / `CAOpenGLLayer`（CAOpenGLLayer 已 deprecated，绑定可能不全）

**判定规则**：缺失项一律回退 `objc2::runtime` 裸函数 + `msg_send!`（配 `class!`），不引入第三代 crate、不手写 `#[link] extern`。

- [ ] **Step 2: Cargo.toml 换依赖**

`openitgo-app/Cargo.toml` macOS target 依赖：删 `objc = "0.2"`，按 Step 1 结论加（示例）：

```toml
objc2 = "0.6"
objc2-foundation = "0.3"
objc2-app-kit = "0.3"
objc2-quartz-core = "0.3"   # 若 Step 1 判定 quartz 绑定不可用则不加
```

Run: `cargo check -p openitgo-app` 确认依赖解析（此时代码未改，预期只剩 objc 相关报错，先把版本解析跑通：`cargo update -p objc2 --dry-run` 或直接看 `cargo tree | grep objc2` 无 0.5/0.6 混用新增）。

- [ ] **Step 3: 迁移 dock_open（platform.rs:285-683）**

机械映射（逐点）：

- `objc::runtime::{class_addMethod, ...}` → `objc2::runtime` 同名项；`Object` → `AnyObject`、`Class` → `AnyClass`、`BOOL` → `Bool`（`YES` → `Bool::YES`、`NO` → `Bool::NO`）
- `objc::{class, msg_send, sel, sel_impl}` → `objc2::{class, msg_send, sel}`（sel_impl 不再需要；文件顶部 `#[allow(unexpected_cfgs)]` 若是为 objc 0.2 sel_impl 的陈旧 cfg 而加，一并删除）
- IMP 回调签名按 objc2 0.6 `class_addMethod` 的约定调整（`extern "C" fn(&AnyObject, Sel, ...)` 或裸指针形态，以编译器为准）；类型串 `c"v@:@"` 等保持不变
- 防御逻辑（NS 前缀类拒注入 platform.rs:443-449）、三个回调解析 NSArray/NSURL 入 OPEN_QUEUE 的逻辑**逐行保留**，只换类型与宏
- 行为红线：swizzle 时机（`install_dock_open_handler_early` 必须先于 `run_native`）与 `wake_ui` 唤醒链路不动

- [ ] **Step 4: 迁移 mpv_view.rs**

- `objc::declare::ClassDecl` → `objc2::define::ClassBuilder` 或 `define_class!` 宏（取改动小者）；6 个 `add_method` 对应迁移；`_rsState` ivar 保留 `usize` 裸存取或改类型化 ivar（取改动小者）
- `msg_send![super(this, superclass), dealloc]` → objc2 等价 super 写法
- 59 处 `msg_send!` 机械替换；CAOpenGLLayer/CATextLayer/CATransaction/NSColor/NSString 按 Step 1 结论走绑定或 `class!` + 裸消息
- 删除 `mpv_view.rs:46-52` 的 `compile_error!` aarch64 守卫
- 保留 `#[cfg(test)] mod tests`（5 个纯几何测试，不触 objc，应无需改动）

- [ ] **Step 5: 迁移 3 个 probe examples**

imports 与 `msg_send!`/`class!` 机械替换（27 处）；逻辑不动。

- [ ] **Step 6: 编译与自动化验证**

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
rustup target list --installed | grep -q x86_64-apple-darwin && cargo check --target x86_64-apple-darwin -p openitgo-app || echo "x86_64 target 未安装，跳过（可选：rustup target add x86_64-apple-darwin）"
```

Expected: 全绿；x86_64 交叉 check 通过（stret 守卫删除的验证）。

- [ ] **Step 7: 手动验证（CI 覆盖不到，必须做）**

1. `cargo run -p openitgo-app --example probe_visible` — 裸窗口正常显示
2. `cargo run -p openitgo-app --example probe_video_overlay -- <视频文件>` — 视频层合成正常（截图确认）
3. Dock 打开实测：`touch /tmp/openitgo-dock.log`，从 Finder 拖一个 .cbz 到 Dock 图标（应用未运行与已运行各一次），确认打开且日志有记录

- [ ] **Step 8: 阻塞回退（仅当卡死时）**

若 Step 1 发现关键 API 缺口无法绕过，或 Step 3/4 同一编译错误经 3 次不同方案仍无法解决：`git checkout -- openitgo-app Cargo.lock`，TODO.md 57 objc 子项标注阻塞原因，跳到 Task 6（egui 升级后 winit/objc2 版本联动可能改变结论）。

- [ ] **Step 9: 收尾**

TODO.md 57 objc 子句标注 `（已完成）`。AGENTS.md 相关条目（Dock open、media playback 中涉及 objc 的描述）同步为 objc2 措辞。
Commit: `refactor(app): objc 0.2 迁移到 objc2 0.6，解除 aarch64-only 限制（#57）`

---

### Task 6: egui/eframe 升级 + 检查点/回退协议（TODO #57 egui 子项）

**Files:**
- Modify: `openitgo-app/Cargo.toml:10-12`（eframe/egui/egui-phosphor）
- Modify: 编译错误波及的源码（范围由升级结果决定）
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: Task 0-5 全部完成并已提交
- Produces: egui/eframe 最新稳定版；或一份带回退原因记录的阻塞结论

背景：egui 0.29 → 上游最新（TODO 写 0.33+，执行时以 crates.io 实际最新稳定版为准）。高风险联动点：wgpu 大版本（透明 backbuffer/CAMetalLayer 路径）、winit 大版本（dock_open swizzle 依赖 winit 设 delegate 的时机）、Slider API（media seek bar 的 `slider_width` override workaround）、egui-phosphor 版本配套。**用户明确要求：升级前先提交；始终无法成功则回退代码。**

- [ ] **Step 1: 检查点提交（硬性前置）**

```bash
git status --porcelain
```

工作树必须干净；不干净则先把未提交改动按归属提交。然后：

```bash
git rev-parse HEAD   # 记录为 CHECKPOINT_SHA
git push             # 确保检查点在远端也有
```

- [ ] **Step 2: 确定目标版本**

```bash
cargo search eframe --limit 3
cargo search egui-phosphor --limit 3
```

记录 eframe/egui 最新稳定版与 egui-phosphor 的配套版本（其发布与 egui 主版本对齐）。**决策点**：若 egui-phosphor 没有匹配新 egui 的版本，停下来问用户（可选：图标字体换方案或暂缓升级）。

- [ ] **Step 3: bump 版本并首次编译**

`openitgo-app/Cargo.toml` 改 eframe/egui/egui-phosphor 版本 →

```bash
cargo update -p eframe -p egui -p egui-phosphor
cargo check --workspace 2>&1 | tee /tmp/egui-upgrade-check.log
```

- [ ] **Step 4: 按编译错误逐一修复**

已知高风险点（按此顺序先扫一遍存量代码，再处理编译器报出的其余错误）：

1. `main.rs:30-46`：`ViewportBuilder` / `NativeOptions` / `with_transparent` / `Renderer::Wgpu` API 变化
2. 透明 backbuffer 路径：`clear_color` 全透明返回值相关 API（AGENTS.md "Media playback" 条目）
3. Slider：media seek bar 的 `ui.spacing_mut().slider_width` override（egui 0.29 workaround）——新版若 `add_sized` 生效则删 workaround 并实测拖动行为
4. winit 联动：dock_open swizzle 的 `install_dock_open_handler_early` 时机假设（winit 设 delegate 的时点）——若 winit 大版本变化，必须重做 Dock 实测
5. accesskit / 字体 / style / spacing API 杂项

**修复纪律**：同一错误经 3 次不同方案仍无法解决 → 触发 Step 7 回退；每修一类错误跑一次 `cargo check --workspace` 收敛错误数。

- [ ] **Step 5: 自动化验收**

完整流水线（Global Constraints 四条）全绿。

- [ ] **Step 6: 手动冒烟（必须全过，任一不过且无修复路径 → Step 7）**

1. `cargo run -p openitgo-app --example ui_smoke -- <漫画路径>` — 30 秒内加载当前页并自动退出
2. 打开一本漫画手动翻页/缩放/双页切换正常
3. `cargo run -p openitgo-app --example probe_video_overlay -- <视频文件>` — 视频层在 egui 之下合成正常
4. 打开一个视频文件实测：播放/进度条拖动/OSD/菜单悬浮
5. `cargo run -p openitgo-app --example probe_ebook_menu -- <epub路径>` — 电子书菜单停放正常
6. Dock 拖入打开实测（winit 联动验证）

- [ ] **Step 7: 失败回退（用户已授权）**

触发条件（任一）：同一编译错误 3 次尝试无解；wgpu/winit 联动需要重写平台层的阻塞性破坏；Step 6 任一手动冒烟失败且无修复路径。

```bash
git reset --hard <CHECKPOINT_SHA>   # 工作树与 Cargo.lock 一并回到升级前
```

注意：升级工作不落地任何 commit，回退即丢弃；若中途误提交，用 `git reset --hard <CHECKPOINT_SHA>` 同样覆盖（不 push，无需 force-push）。
然后在 TODO.md 57 egui 子句标注阻塞原因与下次再试条件，Commit + push 该标注。

- [ ] **Step 8: 成功收尾**

TODO.md 勾选 57 整项；AGENTS.md 中 egui 版本相关描述（如 Slider workaround 条目）同步。
Commit: `chore(app): 升级 egui/eframe 至 <版本>（#57）`，push。

---

### Task 7: 收尾核对

**Files:**
- Modify: `TODO.md`（勾选核对）
- Modify: `CHANGELOG.md`

- [ ] **Step 1: TODO.md 勾选核对**

确认 32.4 / 57 / 58 / 59 状态与本批次实际结果一致（被回退/阻塞的子项保留未勾选并带原因标注）。

- [ ] **Step 2: CHANGELOG.md 增补**

在 Unreleased（或新版本段）记录本批次：media 纯函数测试模块、argv 文件关联、PDF 单栈化、objc2 迁移、egui 升级（或阻塞结论）。

- [ ] **Step 3: 最终流水线 + push**

完整流水线全绿后 commit + push。
