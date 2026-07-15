# 媒体播放器体验补齐实施计划（全宽进度条 / OSD / 静音 / 滚轮音量 / 设备选择 / 音量倍速记忆）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把五项播放器基础体验（全宽进度条、OSD、静音、滚轮音量、输出设备选择、音量/倍速全局记忆）落到现有媒体播放实现上。

**Architecture:** 保持现有分层：`rust-reader-media` 封装全部 mpv FFI（新增 mute 属性观察与 audio-device-list 枚举）；`rust-reader-storage` 的 `Settings` 增加三个字段并随 `on_exit` 统一持久化；`rust-reader-app` 负责 UI（两行进度条、工具栏设备框、快捷键、滚轮）与原生 OSD（CATextLayer 叠加在 CAOpenGLLayer 上）。

**Tech Stack:** Rust、egui 0.29、libmpv（`libmpv-sys`）、objc 0.2 / CoreAnimation（CATextLayer）。

## Global Constraints

- 设计依据：`docs/superpowers/specs/2026-07-15-media-player-ux-design.md`（含 §2.1 修订记录，与本文冲突处以修订记录为准）。
- 仅 macOS；非 macOS 的 `player_stub.rs` 必须保持方法签名同步，保证 `cargo check` 通过。
- UI 文本一律中文（专有名词/技术标识符除外）。
- 提交前跑完整流水线：`cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`，全绿才能 commit。
- 每个 Task 一个 commit，commit message 说明改动与涉及 crate。
- 最小改动：不顺手重构无关代码。
- 持久化约定：设置变更只写回 `self.settings`，由 `on_exit` 统一保存（不新增即时写盘点）。
- OSD 约定：文本右上角、约 1 秒、CA 隐式动画淡出；连续触发覆盖旧文本并重置计时。

---

### Task 1: rust-reader-media —— 静音属性 + 输出设备枚举

**Files:**
- Create: `rust-reader-media/src/devices.rs`
- Modify: `rust-reader-media/src/state.rs`（+`muted` 字段）
- Modify: `rust-reader-media/src/player.rs`（observe 7 / `set_muted` / `audio_devices` / `set_audio_device` / `read_audio_device_list`）
- Modify: `rust-reader-media/src/player_stub.rs`（非 macOS 签名同步）
- Modify: `rust-reader-media/src/lib.rs`（导出 devices）

**Interfaces:**
- Consumes: 现有 `MpvPlayer::set_property_string`、`cstring`、`node_string`、`read_track_list` 的 NODE 遍历模式。
- Produces:
  - `PlayerState.muted: bool`
  - `MpvPlayer::set_muted(&self, muted: bool) -> Result<(), MediaError>`
  - `MpvPlayer::audio_devices(&self) -> Vec<AudioDevice>`
  - `MpvPlayer::set_audio_device(&self, name: &str) -> Result<(), MediaError>`
  - `devices::{RawAudioDevice, AudioDevice, parse_audio_devices}`，`AudioDevice::label() -> String`

- [ ] **Step 1: 写 devices.rs（含失败测试先行）**

先写测试再写实现。创建 `rust-reader-media/src/devices.rs`：

```rust
//! Audio output device records (FFI-free) and their parsing logic.
//! Same layering as tracks.rs: player.rs fills RawAudioDevice from mpv
//! nodes; everything downstream stays testable.

/// Intermediate, FFI-free device record.
#[derive(Debug, Clone, PartialEq)]
pub struct RawAudioDevice {
    pub name: String,
    pub description: Option<String>,
}

/// An audio output device as presented to the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDevice {
    /// Value for mpv's `audio-device` property ("auto" = follow the system).
    pub name: String,
    /// Human-readable description; `label()` falls back to `name` when None.
    pub description: Option<String>,
}

impl AudioDevice {
    pub fn label(&self) -> String {
        self.description.clone().unwrap_or_else(|| self.name.clone())
    }
}

/// Drops empty names and duplicates (keeping the first occurrence).
pub fn parse_audio_devices(raw: Vec<RawAudioDevice>) -> Vec<AudioDevice> {
    let mut seen = std::collections::HashSet::new();
    raw.into_iter()
        .filter(|d| !d.name.is_empty())
        .filter(|d| seen.insert(d.name.clone()))
        .map(|d| AudioDevice {
            name: d.name,
            description: d.description,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, desc: Option<&str>) -> RawAudioDevice {
        RawAudioDevice {
            name: name.to_string(),
            description: desc.map(|s| s.to_string()),
        }
    }

    #[test]
    fn parse_drops_empty_names_and_duplicates() {
        let devices = parse_audio_devices(vec![
            raw("coreaudio/AG06", Some("AG06/AG03")),
            raw("", Some("empty")),
            raw("coreaudio/AG06", Some("dup")),
            raw("coreaudio/hdmi", None),
        ]);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "coreaudio/AG06");
        assert_eq!(devices[1].name, "coreaudio/hdmi");
    }

    #[test]
    fn label_falls_back_to_name_without_description() {
        let d = AudioDevice {
            name: "coreaudio/hdmi".into(),
            description: None,
        };
        assert_eq!(d.label(), "coreaudio/hdmi");
        let d2 = AudioDevice {
            name: "coreaudio/AG06".into(),
            description: Some("AG06/AG03".into()),
        };
        assert_eq!(d2.label(), "AG06/AG03");
    }
}
```

- [ ] **Step 2: 导出模块并跑测试确认编译**

`rust-reader-media/src/lib.rs` 在 `pub mod cover;` 之后加（无条件编译，与 `tracks` 一致）：

```rust
pub mod devices;
```

并在 `pub use error::MediaError;` 后加：

```rust
pub use devices::AudioDevice;
```

Run: `cargo test -p rust-reader-media devices`
Expected: PASS（2 个新测试）

- [ ] **Step 3: PlayerState 增加 muted**

`rust-reader-media/src/state.rs` 在 `pub volume: f64,` 后加：

```rust
    pub muted: bool,
```

- [ ] **Step 4: player.rs 增加 mute 观察与设备 API**

3a. `MpvPlayer::new` 的属性观察块尾部（id 6 的 `track-list` 之后）加：

```rust
            mpv::mpv_observe_property(
                handle,
                7,
                cstring("mute").as_ptr(),
                mpv::mpv_format_MPV_FORMAT_FLAG,
            );
```

3b. 事件循环 `match userdata` 的 `6 =>` 分支之后加：

```rust
                        7 => {
                            if format == mpv::mpv_format_MPV_FORMAT_FLAG && !data.is_null() {
                                // SAFETY: format guarantees data points to a c_int.
                                s.muted = unsafe { *(data as *mut i32) } != 0;
                                should_repaint = true;
                            }
                        }
```

3c. `set_audio_track` 之后加命令 API：

```rust
    pub fn set_muted(&self, muted: bool) -> Result<(), MediaError> {
        self.set_property_string("mute", if muted { "yes" } else { "no" })
    }

    /// Enumerates mpv's `audio-device-list`. Returns an empty Vec when the
    /// property is unavailable; callers treat that as "auto only".
    pub fn audio_devices(&self) -> Vec<crate::devices::AudioDevice> {
        let name = cstring("audio-device-list");
        // SAFETY: a zeroed node is valid for mpv to fill; freed below after
        // reading.
        let mut node: mpv::mpv_node = unsafe { std::mem::zeroed() };
        // SAFETY: handle is valid; `name` is a valid NUL-terminated string;
        // `node` outlives the call.
        let rc = unsafe {
            mpv::mpv_get_property(
                self.handle,
                name.as_ptr(),
                mpv::mpv_format_MPV_FORMAT_NODE,
                &mut node as *mut _ as *mut std::ffi::c_void,
            )
        };
        if rc < 0 {
            return Vec::new();
        }
        // SAFETY: node is a filled NODE tree owned by us.
        let raw = unsafe { read_audio_device_list(&mut node) };
        // SAFETY: node was filled by mpv_get_property and not yet freed.
        unsafe { mpv::mpv_free_node_contents(&mut node) };
        crate::devices::parse_audio_devices(raw)
    }

    /// `name` is an entry of `audio-device-list`; "auto" follows the system.
    pub fn set_audio_device(&self, name: &str) -> Result<(), MediaError> {
        self.set_property_string("audio-device", name)
    }
```

3d. 文件尾部（`read_track_list` 之后）加 FFI 转换：

```rust
/// Walks an audio-device-list NODE (list of maps with string `name` /
/// `description` keys) into FFI-free records.
/// # Safety: `node` must point to a valid mpv_node owned by the caller.
unsafe fn read_audio_device_list(node: *mut mpv::mpv_node) -> Vec<crate::devices::RawAudioDevice> {
    let mut out = Vec::new();
    // SAFETY: per caller, node points to a valid mpv_node owned by us.
    if node.is_null() || unsafe { (*node).format } != mpv::mpv_format_MPV_FORMAT_NODE_ARRAY {
        return out;
    }
    // SAFETY: format is NODE_ARRAY, so the list union member is valid.
    let list = unsafe { (*node).u.list };
    if list.is_null() {
        return out;
    }
    // SAFETY: values has num entries.
    let (num, values) = unsafe { ((*list).num, (*list).values) };
    for i in 0..num {
        // SAFETY: i < num, so values[i] is a valid mpv_node.
        let entry = unsafe { values.offset(i as isize) };
        if entry.is_null() || unsafe { (*entry).format } != mpv::mpv_format_MPV_FORMAT_NODE_MAP {
            continue;
        }
        // SAFETY: format is NODE_MAP, so the list union member is valid.
        let map = unsafe { (*entry).u.list };
        if map.is_null() {
            continue;
        }
        // SAFETY: keys and values each have num entries.
        let (mnum, mvalues, mkeys) = unsafe { ((*map).num, (*map).values, (*map).keys) };
        let mut device = crate::devices::RawAudioDevice {
            name: String::new(),
            description: None,
        };
        for j in 0..mnum {
            // SAFETY: j < mnum, so keys[j] is a valid NUL-terminated string
            // owned by the node.
            let key = unsafe {
                let k = mkeys.offset(j as isize);
                if k.is_null() || (*k).is_null() {
                    continue;
                }
                std::ffi::CStr::from_ptr(*k).to_string_lossy().into_owned()
            };
            // SAFETY: j < mnum, so values[j] is a valid mpv_node.
            let v = unsafe { mvalues.offset(j as isize) };
            if v.is_null() || unsafe { (*v).format } != mpv::mpv_format_MPV_FORMAT_STRING {
                continue;
            }
            match key.as_str() {
                // SAFETY: format checked STRING above; node_string copies it.
                "name" => device.name = unsafe { node_string(v) },
                "description" => device.description = Some(unsafe { node_string(v) }),
                _ => {}
            }
        }
        if !device.name.is_empty() {
            out.push(device);
        }
    }
    out
}
```

- [ ] **Step 5: player_stub.rs 签名同步**

在 `set_audio_track` 后加：

```rust
    pub fn set_muted(&self, _muted: bool) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn audio_devices(&self) -> Vec<crate::devices::AudioDevice> {
        Vec::new()
    }

    pub fn set_audio_device(&self, _name: &str) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }
```

- [ ] **Step 6: 流水线 + commit**

Run: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全绿（无需 mpv 运行环境的部分只编译；本机已装 mpv，media 测试可跑）

```bash
git add rust-reader-media/
git commit -m "feat(media): mute 属性观察与 set_muted、audio-device-list 枚举与切换"
```

---

### Task 2: rust-reader-storage —— Settings 三个媒体字段

**Files:**
- Modify: `rust-reader-storage/src/models.rs`（字段 + Default + validate + clamp）
- Modify: `rust-reader-storage/src/json_store.rs`（测试）

**Interfaces:**
- Consumes: 现有 `Settings`、`validate`、`clamp` 模式。
- Produces:
  - `Settings.media_volume: f64`（默认 100.0，合法区间 0..=100）
  - `Settings.media_speed: f64`（默认 1.0，合法区间 0.1..=16）
  - `Settings.media_audio_device: String`（默认 `""` = auto）

- [ ] **Step 1: 写失败测试**

`rust-reader-storage/src/json_store.rs` 的 `#[cfg(test)] mod tests` 里（`test_settings_load_clamps_invalid_values` 旁边）加：

```rust
    #[test]
    fn test_media_settings_validate_and_clamp() {
        let mut settings = crate::models::Settings::default();
        settings.media_volume = 150.0;
        settings.media_speed = 0.0;
        assert!(settings.validate().is_err());
        settings.clamp();
        assert_eq!(settings.media_volume, 100.0);
        assert_eq!(settings.media_speed, 0.1);
        assert!(settings.validate().is_ok());
        assert!(crate::models::Settings::default().validate().is_ok());
    }
```

Run: `cargo test -p rust-reader-storage test_media_settings`
Expected: FAIL（`no field named media_volume`）

- [ ] **Step 2: 实现字段与校验**

`models.rs` 的 `Settings` 结构体尾部（`pub ebook: EbookSettings,` 后）加：

```rust
    pub media_volume: f64,
    pub media_speed: f64,
    pub media_audio_device: String,
```

`Default` impl 尾部（`ebook: EbookSettings::default(),` 后）加：

```rust
            media_volume: 100.0,
            media_speed: 1.0,
            media_audio_device: String::new(),
```

`validate` 的 `ebook.margin_vertical` 检查之后加：

```rust
        if !(0.0..=100.0).contains(&self.media_volume) {
            return Err(format!(
                "media_volume must be between 0 and 100, got {}",
                self.media_volume
            ));
        }
        if !(0.1..=16.0).contains(&self.media_speed) {
            return Err(format!(
                "media_speed must be between 0.1 and 16, got {}",
                self.media_speed
            ));
        }
```

`clamp` 尾部加：

```rust
        self.media_volume = self.media_volume.clamp(0.0, 100.0);
        self.media_speed = self.media_speed.clamp(0.1, 16.0);
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test -p rust-reader-storage`
Expected: PASS（含旧有序列化兼容测试；`#[serde(default)]` 保证旧 JSON 可加载）

- [ ] **Step 4: 流水线 + commit**

Run: 完整流水线。
```bash
git add rust-reader-storage/
git commit -m "feat(storage): Settings 增加 media_volume/media_speed/media_audio_device"
```

---

### Task 3: rust-reader-app —— 原生 OSD 层（CATextLayer）与 MediaView OSD 状态

**Files:**
- Modify: `rust-reader-app/src/platform/macos/mpv_view.rs`（CATextLayer 叠加）
- Modify: `rust-reader-app/src/views/media.rs`（Osd 状态 + 文案纯函数）
- Modify: `rust-reader-app/src/app.rs`（render_media 调 tick_osd）

**Interfaces:**
- Consumes: `MpvNativeView` 现有结构；egui `Context::request_repaint_after`。
- Produces:
  - `MpvNativeView::set_osd(&self, text: &str)`
  - `MpvNativeView::clear_osd(&self)`
  - `MediaView::show_osd(&mut self, ctx: &egui::Context, text: String)`
  - `MediaView::tick_osd(&mut self)`
  - `media::volume_osd_text(f64) -> String`、`speed_osd_text(f64) -> String`、`mute_osd_text(bool) -> &'static str`

- [ ] **Step 1: 写文案纯函数的失败测试**

`rust-reader-app/src/views/media.rs` 的 tests mod 加：

```rust
    #[test]
    fn osd_texts_format_values() {
        assert_eq!(volume_osd_text(75.0), "音量 75%");
        assert_eq!(speed_osd_text(1.5), "1.5x");
        assert_eq!(mute_osd_text(true), "静音");
        assert_eq!(mute_osd_text(false), "取消静音");
    }
```

Run: `cargo test -p rust-reader-app osd_texts`
Expected: FAIL（函数不存在）

- [ ] **Step 2: media.rs 实现 OSD 状态与文案**

文件头部 `use` 区加 `use std::time::{Duration, Instant};`。

`MediaView` 定义改为：

```rust
const OSD_DURATION: Duration = Duration::from_millis(1000);

struct Osd {
    until: Instant,
}

#[derive(Default)]
pub struct MediaView {
    pub open: Option<OpenMedia>,
    osd: Option<Osd>,
}
```

`impl MediaView` 内加：

```rust
    /// Shows an OSD message for ~1s. CoreAnimation fades the layer in/out
    /// via its implicit opacity animation; we only track the expiry.
    pub fn show_osd(&mut self, ctx: &egui::Context, text: String) {
        if let Some(open) = self.open.as_ref() {
            open.native.set_osd(&text);
            self.osd = Some(Osd {
                until: Instant::now() + OSD_DURATION,
            });
            ctx.request_repaint_after(OSD_DURATION);
        }
    }

    /// Hides an expired OSD; called once per frame from render_media.
    pub fn tick_osd(&mut self) {
        if self.osd.as_ref().is_some_and(|o| Instant::now() >= o.until) {
            if let Some(open) = self.open.as_ref() {
                open.native.clear_osd();
            }
            self.osd = None;
        }
    }
```

文件尾部（`clamp_seek` 后）加文案函数：

```rust
pub fn volume_osd_text(volume: f64) -> String {
    format!("音量 {:.0}%", volume)
}

pub fn speed_osd_text(speed: f64) -> String {
    format!("{speed:.1}x")
}

pub fn mute_osd_text(muted: bool) -> &'static str {
    if muted { "静音" } else { "取消静音" }
}
```

Run: `cargo test -p rust-reader-app osd_texts`
Expected: PASS（但 `set_osd`/`clear_osd` 尚未存在，下一步实现；先注释调用或先实现 Step 3 再跑——执行顺序：先写 Step 3 的 mpv_view 代码，再跑测试）

**注意**：Step 2 与 Step 3 互相依赖编译，实施时按 Step 3 → Step 2 的顺序写代码，最后一起跑测试。

- [ ] **Step 3: mpv_view.rs 增加 CATextLayer OSD**

3a. `MpvNativeView` 结构体加字段：

```rust
pub struct MpvNativeView {
    view: *mut Object,
    layer: *mut Object,
    osd_layer: *mut Object,
    state: *mut LayerState,
}
```

3b. 文件级注释块（`//!` 头）末尾追加一段：

```rust
//! OSD: a CATextLayer sublayer of the CAOpenGLLayer shows transient text
//! (volume, mute, seeks) at the top-right of the video. egui cannot paint
//! over the native view (that is why overlays park it at zero size), so the
//! OSD lives in the native layer tree; CoreAnimation's implicit opacity
//! animation provides the fade.
```

3c. 增加布局与外观代码（`make_frame` 之后）：

```rust
/// Top-right anchor in the layer's bottom-left-origin coordinate space (our
/// NSView is not flipped — the reason the video needs FLIP_Y). `text_w` is
/// clamped so long device names cannot run off the left edge.
fn osd_frame(
    view_w: f64,
    view_h: f64,
    text_w: f64,
    text_h: f64,
) -> core_graphics::geometry::CGRect {
    const MARGIN: f64 = 16.0;
    let w = text_w.min((view_w - 2.0 * MARGIN).max(0.0));
    let x = (view_w - w - MARGIN).max(MARGIN);
    let y = (view_h - text_h - MARGIN).max(MARGIN);
    core_graphics::geometry::CGRect::new(
        &core_graphics::geometry::CGPoint::new(x, y),
        &core_graphics::geometry::CGSize::new(w, text_h),
    )
}
```

3d. `MpvNativeView::new` 里 `let () = msg_send![ns_view, addSubview: view];` 之后（`(view, layer)` 返回前）加：

```rust
            // OSD text layer: a sublayer of the CAOpenGLLayer, hidden until
            // the first set_osd. Retained by us (+1), released in Drop.
            let osd_layer: *mut Object = msg_send![class!(CATextLayer), alloc];
            let osd_layer: *mut Object = msg_send![osd_layer, init];
            let () = msg_send![osd_layer, setFontSize: 20.0f64];
            let fg: *mut Object = msg_send![class!(NSColor), colorWithRed: 1.0f64 green: 1.0f64 blue: 1.0f64 alpha: 1.0f64];
            let fg_cg: *mut c_void = msg_send![fg, CGColor];
            let () = msg_send![osd_layer, setForegroundColor: fg_cg];
            let bg: *mut Object = msg_send![class!(NSColor), colorWithRed: 0.0f64 green: 0.0f64 blue: 0.0f64 alpha: 0.6f64];
            let bg_cg: *mut c_void = msg_send![bg, CGColor];
            let () = msg_send![osd_layer, setBackgroundColor: bg_cg];
            let () = msg_send![osd_layer, setCornerRadius: 8.0f64];
            let () = msg_send![osd_layer, setContentsScale: scale];
            let () = msg_send![osd_layer, setOpacity: 0.0f32];
            let () = msg_send![layer, addSublayer: osd_layer];
```

返回值改为 `(view, layer, osd_layer)`，结构构造改为：

```rust
        Ok(Self {
            view,
            layer,
            osd_layer,
            state: state_ptr,
        })
```

（`let (view, layer) = unsafe { ... }` 相应改为 `let (view, layer, osd_layer) = unsafe { ... }`。）

3e. 新方法（`set_bounds` 之后）：

```rust
    /// Shows `text` at the top-right of the video. The CATextLayer fades in
    /// via CoreAnimation's implicit opacity animation.
    pub fn set_osd(&self, text: &str) {
        let ctext = std::ffi::CString::new(text)
            .unwrap_or_else(|_| std::ffi::CString::new("").expect("empty CString"));
        // SAFETY: all objects are live instances owned by us, messaged on the
        // UI thread; selectors match the receivers' classes.
        unsafe {
            let ns: *mut Object = msg_send![class!(NSString), alloc];
            let ns: *mut Object = msg_send![ns, initWithUTF8String: ctext.as_ptr()];
            let () = msg_send![self.osd_layer, setString: ns];
            let () = msg_send![ns, release];
            let text: core_graphics::geometry::CGSize =
                msg_send![self.osd_layer, preferredFrameSize];
            let frame: core_graphics::geometry::CGRect = msg_send![self.view, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(
                    frame.size.width,
                    frame.size.height,
                    text.width + 24.0,
                    text.height + 10.0,
                )
            ];
            let () = msg_send![self.osd_layer, setOpacity: 1.0f32];
        }
    }

    /// Fades the OSD out (implicit animation); harmless when already hidden.
    pub fn clear_osd(&self) {
        // SAFETY: self.osd_layer is a live CATextLayer owned by us.
        unsafe {
            let () = msg_send![self.osd_layer, setOpacity: 0.0f32];
        }
    }
```

3f. `set_bounds` 里追加 OSD 重定位（保持右上角锚定）：

```rust
    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.view/osd_layer are live objects owned by us.
        unsafe {
            let () = msg_send![self.view, setFrame: make_frame(&bounds)];
            let frame: core_graphics::geometry::CGRect = msg_send![self.view, frame];
            let cur: core_graphics::geometry::CGRect = msg_send![self.osd_layer, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(frame.size.width, frame.size.height, cur.size.width, cur.size.height)
            ];
        }
    }
```

3g. `Drop` 里在释放 layer/view 之前加：

```rust
        // SAFETY: balanced release for the alloc/init retain in new(). The
        // superlayer also retains it until its own dealloc.
        unsafe {
            let () = msg_send![self.osd_layer, release];
        }
```

- [ ] **Step 4: app.rs 接入 tick_osd**

`render_media` 里 `self.media_view.sync_state();` 之后加：

```rust
        self.media_view.tick_osd();
```

- [ ] **Step 5: 测试 + 流水线 + commit**

Run: `cargo test -p rust-reader-app osd_texts` → PASS；然后完整流水线。

```bash
git add rust-reader-app/src/platform/macos/mpv_view.rs rust-reader-app/src/views/media.rs rust-reader-app/src/app.rs
git commit -m "feat(app): 原生 CATextLayer OSD 层与 MediaView OSD 状态机"
```

---

### Task 4: rust-reader-app —— 两行式全宽进度条

**Files:**
- Modify: `rust-reader-app/src/views/media.rs`（`hover_time_at`、`seek_to_ratio_exact`）
- Modify: `rust-reader-app/src/app.rs`（`render_media_seekbar` 重写）

**Interfaces:**
- Consumes: Task 1 的 `PlayerState.muted`；现有 `seek_to_ratio`、`clamp_seek`。
- Produces:
  - `media::hover_time_at(pointer_x: f32, bar_rect: egui::Rect, duration_ms: Option<u64>) -> Option<u64>`
  - `MediaView::seek_to_ratio_exact(&mut self, ratio: f64)`
  - `ReaderApp::set_media_volume(&mut self, ctx, v: f64)`、`toggle_media_mute(&mut self, ctx)`（Task 5 复用）

- [ ] **Step 1: 写 hover_time_at 失败测试**

media.rs tests mod 加：

```rust
    #[test]
    fn hover_time_maps_pointer_to_ratio_of_duration() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(200.0, 16.0));
        assert_eq!(hover_time_at(100.0, rect, Some(60_000)), Some(0));
        assert_eq!(hover_time_at(200.0, rect, Some(60_000)), Some(30_000));
        assert_eq!(hover_time_at(300.0, rect, Some(60_000)), Some(60_000));
        // 指针越界时 clamp
        assert_eq!(hover_time_at(400.0, rect, Some(60_000)), Some(60_000));
        assert_eq!(hover_time_at(200.0, rect, None), None);
    }
```

Run: `cargo test -p rust-reader-app hover_time`
Expected: FAIL（函数不存在）

- [ ] **Step 2: media.rs 实现两个函数**

```rust
/// Maps a pointer x position on the seek bar to a time for the hover
/// tooltip; None when the duration is unknown or the bar has no width.
pub fn hover_time_at(
    pointer_x: f32,
    bar_rect: egui::Rect,
    duration_ms: Option<u64>,
) -> Option<u64> {
    let dur = duration_ms?;
    if bar_rect.width() <= 0.0 {
        return None;
    }
    let ratio = ((pointer_x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
    Some((dur as f64 * ratio as f64) as u64)
}
```

`impl MediaView` 内（`seek_to_ratio` 之后）加：

```rust
    /// Exact seek for when the slider drag ends (release); interactive drags
    /// use the keyframe-aligned `seek_to_ratio` instead.
    pub fn seek_to_ratio_exact(&mut self, ratio: f64) {
        if let Some(open) = self.open.as_ref() {
            if let Some(dur) = open.last.duration_ms {
                let target = clamp_seek((dur as f64 * ratio.clamp(0.0, 1.0)) as i64, dur);
                let _ = open.player.seek_abs_ms(target, true);
            }
        }
    }

    /// Returns the new muted state for OSD text; None when nothing is open.
    pub fn toggle_mute(&mut self) -> Option<bool> {
        let open = self.open.as_ref()?;
        let muted = !open.last.muted;
        let _ = open.player.set_muted(muted);
        Some(muted)
    }
```

同时把 `adjust_volume` 改为返回应用后的目标值（持久化与 OSD 需要）：

```rust
    /// Returns the applied (clamped) target volume for persistence/OSD.
    pub fn adjust_volume(&mut self, delta: f64) -> Option<f64> {
        let open = self.open.as_ref()?;
        let target = (open.last.volume + delta).clamp(0.0, 100.0);
        let _ = open.player.set_volume(target);
        Some(target)
    }

    /// Returns the applied target speed for persistence/OSD.
    pub fn cycle_speed(&mut self) -> Option<f64> {
        let open = self.open.as_ref()?;
        let target = next_speed(open.last.speed);
        let _ = open.player.set_speed(target);
        Some(target)
    }
```

（`Option` 是 `#[must_use]`，旧调用点必须同步处理，否则 clippy `-D warnings` 失败。本 Task 内先把三处旧调用点改为丢弃返回值，Task 5 再替换为 helper 调用：
- `render_media_toolbar` 的 `self.media_view.cycle_speed();` → `let _ = self.media_view.cycle_speed();`
- 键盘分支的 `self.media_view.adjust_volume(5.0);` / `(-5.0)` → `let _ = ...`）

- [ ] **Step 3: app.rs 增加两个媒体 helper**

`impl ReaderApp` 内（`render_media_seekbar` 之前）加：

```rust
    fn set_media_volume(&mut self, ctx: &egui::Context, v: f64) {
        let v = v.clamp(0.0, 100.0);
        self.media_view.set_volume(v);
        self.settings.media_volume = v;
        self.media_view
            .show_osd(ctx, crate::views::media::volume_osd_text(v));
    }

    fn toggle_media_mute(&mut self, ctx: &egui::Context) {
        if let Some(muted) = self.media_view.toggle_mute() {
            self.media_view
                .show_osd(ctx, crate::views::media::mute_osd_text(muted).to_string());
        }
    }
```

- [ ] **Step 4: render_media_seekbar 重写为两行**

整体替换现有函数：

```rust
    fn render_media_seekbar(&mut self, ctx: &egui::Context) {
        let (pos, dur, volume, muted) = self
            .media_view
            .open
            .as_ref()
            .map(|o| (o.last.position_ms, o.last.duration_ms, o.last.volume, o.last.muted))
            .unwrap_or((0, None, 100.0, false));
        egui::TopBottomPanel::bottom("media_seekbar").show(ctx, |ui| {
            ui.vertical(|ui| {
                // Row 1: full-width seek bar with a hover-time tooltip.
                match dur {
                    Some(d) if d > 0 => {
                        let mut ratio = pos as f32 / d as f32;
                        let slider = egui::Slider::new(&mut ratio, 0.0..=1.0).show_value(false);
                        let width = ui.available_width();
                        let response = ui.add_sized([width, 16.0], slider);
                        if let Some(hover) = response.hover_pos() {
                            if let Some(t) =
                                crate::views::media::hover_time_at(hover.x, response.rect, dur)
                            {
                                response.on_hover_text(rust_reader_media::time::format_time_ms(t));
                            }
                        }
                        if response.drag_stopped() {
                            self.media_view.seek_to_ratio_exact(ratio as f64);
                        } else if response.changed() {
                            self.media_view.seek_to_ratio(ratio as f64);
                        }
                    }
                    _ => {
                        ui.add_enabled(
                            false,
                            egui::Slider::new(&mut 0.0f32, 0.0..=1.0).show_value(false),
                        );
                    }
                }
                // Row 2: time display on the left; mute + volume on the right.
                ui.horizontal(|ui| {
                    let dur_text = dur
                        .map(rust_reader_media::time::format_time_ms)
                        .unwrap_or_else(|| "--:--".to_string());
                    ui.label(format!(
                        "{} / {}",
                        rust_reader_media::time::format_time_ms(pos),
                        dur_text
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut vol = volume as f32;
                        if ui
                            .add_enabled(
                                !muted,
                                egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false),
                            )
                            .changed()
                        {
                            self.set_media_volume(ctx, vol as f64);
                        }
                        if ui
                            .button(if muted { "取消静音" } else { "静音" })
                            .clicked()
                        {
                            self.toggle_media_mute(ctx);
                        }
                    });
                });
            });
        });
    }
```

- [ ] **Step 5: 测试 + 流水线 + commit**

Run: `cargo test -p rust-reader-app hover_time` → PASS；完整流水线。

```bash
git add rust-reader-app/src/views/media.rs rust-reader-app/src/app.rs
git commit -m "feat(app): 两行式全宽进度条（悬停时间、松手精确跳转、静音按钮）"
```

---

### Task 5: rust-reader-app —— 滚轮音量 + 快捷键 OSD 接线

**Files:**
- Modify: `rust-reader-app/src/views/media.rs`（`scroll_acc` 字段、`accumulate_scroll`）
- Modify: `rust-reader-app/src/app.rs`（键盘分支、工具栏按钮、CentralPanel 滚轮）

**Interfaces:**
- Consumes: Task 3 的 `show_osd`、Task 4 的 `set_media_volume`/`toggle_media_mute`/`adjust_volume`/`cycle_speed` 返回值。
- Produces:
  - `media::SCROLL_VOLUME_STEP_PX: f32 = 25.0`
  - `media::accumulate_scroll(acc: f32, delta: f32) -> (f32, i32)`
  - `ReaderApp::adjust_media_volume(&mut self, ctx, delta: f64)`
  - `ReaderApp::seek_media_rel(&mut self, ctx, secs: f64)`
  - `ReaderApp::set_media_speed(&mut self, ctx, speed: f64)`

- [ ] **Step 1: 写 accumulate_scroll 失败测试**

media.rs tests mod 加：

```rust
    #[test]
    fn accumulate_scroll_steps_and_keeps_remainder() {
        let (acc, steps) = accumulate_scroll(0.0, 30.0);
        assert_eq!(steps, 1);
        assert!((acc - 5.0).abs() < 1e-6);
        let (acc, steps) = accumulate_scroll(acc, 20.0);
        assert_eq!(steps, 1);
        assert!((acc - 0.0).abs() < 1e-6);
        // 反向滚动抵消累计值
        let (acc, steps) = accumulate_scroll(10.0, -10.0);
        assert_eq!(steps, 0);
        assert!((acc - 0.0).abs() < 1e-6);
        let (_acc, steps) = accumulate_scroll(0.0, -60.0);
        assert_eq!(steps, -2);
    }
```

Run: `cargo test -p rust-reader-app accumulate_scroll`
Expected: FAIL（函数不存在）

- [ ] **Step 2: media.rs 实现**

`MediaView` 结构体加字段（`osd: Option<Osd>,` 后）：

```rust
    pub scroll_acc: f32,
```

文件尾部加：

```rust
/// Pixels of scroll delta that make one 5% volume step.
pub const SCROLL_VOLUME_STEP_PX: f32 = 25.0;

/// Accumulates scroll delta into volume steps. Returns the new accumulator
/// and how many 5% steps to apply (sign = direction). The remainder is kept,
/// so reverse scrolling cancels partial steps and smooth trackpad scrolling
/// cannot jump the volume in one event.
pub fn accumulate_scroll(acc: f32, delta: f32) -> (f32, i32) {
    let acc = acc + delta;
    let steps = (acc / SCROLL_VOLUME_STEP_PX).trunc() as i32;
    (acc - steps as f32 * SCROLL_VOLUME_STEP_PX, steps)
}
```

- [ ] **Step 3: app.rs 三个 helper**

```rust
    fn adjust_media_volume(&mut self, ctx: &egui::Context, delta: f64) {
        if let Some(v) = self.media_view.adjust_volume(delta) {
            self.settings.media_volume = v;
            self.media_view
                .show_osd(ctx, crate::views::media::volume_osd_text(v));
        }
    }

    fn seek_media_rel(&mut self, ctx: &egui::Context, secs: f64) {
        self.media_view.seek_rel(secs);
        self.media_view.show_osd(ctx, format!("{:+}s", secs as i32));
    }

    fn set_media_speed(&mut self, ctx: &egui::Context, speed: f64) {
        self.media_view.set_speed(speed);
        self.settings.media_speed = speed;
        self.media_view
            .show_osd(ctx, crate::views::media::speed_osd_text(speed));
    }
```

- [ ] **Step 4: 键盘分支接线（`View::Media` arm）**

- `ArrowUp/ArrowDown` 两处的 `self.media_view.adjust_volume(±5.0);` 改为 `self.adjust_media_volume(ctx, ±5.0);`
- 数字键 1-4 的 `self.media_view.set_speed(speed);` 改为 `self.set_media_speed(ctx, speed);`
- `M` 键新增（放在 `V` 键之后）：

```rust
                if ctx.input(|i| i.key_pressed(egui::Key::M)) {
                    self.toggle_media_mute(ctx);
                }
```

- `ArrowRight`/`ArrowLeft`/`J`/`L` 四处的 `self.media_view.seek_rel(±x.0);` 改为 `self.seek_media_rel(ctx, ±x.0);`

- [ ] **Step 5: 工具栏按钮接线（render_media_toolbar）**

- `-10s` / `+10s` 按钮的 `self.media_view.seek_rel(±10.0);` 改为 `self.seek_media_rel(ctx, ±10.0);`
- 倍速按钮改为：

```rust
                if ui.button(format!("{:.1}x", speed)).clicked() {
                    if let Some(target) = self.media_view.cycle_speed() {
                        self.settings.media_speed = target;
                        self.media_view
                            .show_osd(ctx, crate::views::media::speed_osd_text(target));
                    }
                }
```

- [ ] **Step 6: CentralPanel 滚轮音量（render_media）**

`egui::CentralPanel::default().show(ctx, |ui| {` 闭包内、`let rect = ui.max_rect();` 之后加：

```rust
            // Scroll-wheel volume over the video area.
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 && ui.rect_contains_pointer(rect) {
                let (acc, steps) =
                    crate::views::media::accumulate_scroll(self.media_view.scroll_acc, scroll);
                self.media_view.scroll_acc = acc;
                if steps != 0 {
                    self.adjust_media_volume(ctx, steps as f64 * 5.0);
                }
            }
```

- [ ] **Step 7: 测试 + 流水线 + commit**

Run: `cargo test -p rust-reader-app accumulate_scroll` → PASS；完整流水线。

```bash
git add rust-reader-app/src/views/media.rs rust-reader-app/src/app.rs
git commit -m "feat(app): 滚轮音量与快捷键/工具栏 OSD 接线，音量倍速写回设置"
```

---

### Task 6: rust-reader-app —— 输出设备选择与启动应用

**Files:**
- Modify: `rust-reader-app/src/views/media.rs`（`audio_devices` 字段 + 三个方法）
- Modify: `rust-reader-app/src/app.rs`（工具栏设备框、poll_media_open 应用）

**Interfaces:**
- Consumes: Task 1 的 `MpvPlayer::audio_devices/set_audio_device`、`AudioDevice::label`；Task 2 的 `Settings.media_audio_device`。
- Produces:
  - `MediaView::refresh_audio_devices(&mut self)`
  - `MediaView::apply_startup_settings(&mut self, volume: f64, speed: f64, audio_device: &str) -> bool`（false = 已存设备不存在，已回退 auto）
  - `MediaView::set_audio_device(&mut self, name: &str) -> Result<(), MediaError>`
  - `ReaderApp::set_media_audio_device(&mut self, ctx, name: String, label: String)`

- [ ] **Step 1: media.rs 实现**

`OpenMedia` 结构体加字段（`pending_resume_ms` 后）：

```rust
    pub audio_devices: Vec<rust_reader_media::AudioDevice>,
```

`MediaView::open` 的 `OpenMedia { ... }` 构造加：

```rust
            audio_devices: Vec::new(),
```

`impl MediaView` 加：

```rust
    /// Re-enumerates output devices for the toolbar ComboBox.
    pub fn refresh_audio_devices(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.audio_devices = open.player.audio_devices();
        }
    }

    /// Applies persisted preferences after a successful open. Returns false
    /// when the saved device no longer exists and "auto" was applied instead
    /// (caller clears the stale setting).
    pub fn apply_startup_settings(&mut self, volume: f64, speed: f64, audio_device: &str) -> bool {
        let Some(open) = self.open.as_mut() else {
            return true;
        };
        let _ = open.player.set_volume(volume);
        let _ = open.player.set_speed(speed);
        if audio_device.is_empty() {
            return true;
        }
        let exists = open.audio_devices.iter().any(|d| d.name == audio_device);
        let target = if exists { audio_device } else { "auto" };
        let _ = open.player.set_audio_device(target);
        exists
    }

    /// Empty `name` selects "auto" (follow the system default device).
    pub fn set_audio_device(&mut self, name: &str) -> Result<(), rust_reader_media::MediaError> {
        let Some(open) = self.open.as_ref() else {
            return Ok(());
        };
        let name = if name.is_empty() { "auto" } else { name };
        open.player.set_audio_device(name)
    }
```

- [ ] **Step 2: app.rs poll_media_open 应用启动设置**

`Ok(()) =>` 分支改为：

```rust
            Ok(()) => {
                self.media_view.refresh_audio_devices();
                let device_ok = self.media_view.apply_startup_settings(
                    self.settings.media_volume,
                    self.settings.media_speed,
                    &self.settings.media_audio_device,
                );
                if !device_ok {
                    // 保存的设备已拔出：回退 auto 并更新设置。
                    self.settings.media_audio_device.clear();
                }
                self.current_view = View::Media;
                self.error_message = None;
            }
```

- [ ] **Step 3: app.rs 设备切换 helper**

```rust
    fn set_media_audio_device(&mut self, ctx: &egui::Context, name: String, label: String) {
        match self.media_view.set_audio_device(&name) {
            Ok(()) => {
                self.settings.media_audio_device = name;
                self.media_view.show_osd(ctx, format!("输出: {label}"));
            }
            Err(e) => {
                self.error_message = Some(format!("无法切换音频输出设备: {e}"));
            }
        }
    }
```

- [ ] **Step 4: 工具栏设备下拉框（render_media_toolbar）**

函数开头（`let (title, tracks, ...) = ...` 之后）加：

```rust
        let devices: Vec<(String, String)> = self
            .media_view
            .open
            .as_ref()
            .map(|o| {
                o.audio_devices
                    .iter()
                    .map(|d| (d.name.clone(), d.label()))
                    .collect()
            })
            .unwrap_or_default();
        let current_device = self.settings.media_audio_device.clone();
```

音轨 ComboBox 之后（`ui.with_layout(right_to_left, ...)` 之前）加：

```rust
                ui.separator();
                let current_label = devices
                    .iter()
                    .find(|(n, _)| *n == current_device)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_else(|| "自动".to_string());
                egui::ComboBox::from_label("输出")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current_device.is_empty(), "自动").clicked() {
                            self.set_media_audio_device(ctx, String::new(), "自动".to_string());
                        }
                        for (name, label) in &devices {
                            if ui.selectable_label(current_device == *name, label).clicked() {
                                self.set_media_audio_device(ctx, name.clone(), label.clone());
                            }
                        }
                    });
```

- [ ] **Step 5: 流水线 + commit**

无新纯函数可单测（FFI 枚举与启动应用靠手工验证，见 Task 7）。完整流水线。

```bash
git add rust-reader-app/src/views/media.rs rust-reader-app/src/app.rs
git commit -m "feat(app): 音频输出设备选择下拉框与启动时应用媒体偏好"
```

---

### Task 7: 文档 + 手工验证

**Files:**
- Modify: `README.md`、`CHANGELOG.md`、`AGENTS.md`

**Interfaces:**
- Consumes: Task 1-6 的全部产出。

- [ ] **Step 1: README**

媒体播放小节：
- "工具栏控制"一条改为：播放/暂停、±10s 跳转、0.5x/1x/1.5x/2x 倍速、字幕轨切换/关闭、音轨切换、**音频输出设备选择**、全屏
- 新增条目：
  - `- 音量控制：底栏滑块、滚轮（视频区滚动，每格 ±5%）、M 键静音；调整时画面右上角显示 OSD 反馈`
  - `- 偏好记忆：音量、倍速与输出设备全局记忆（写入设置，重启后生效）；保存的设备拔出时回退"自动"`
- "已知环境限制：音频输出跟随 macOS 系统默认输出设备，暂不支持应用内切换"一条**删除**（已被设备选择取代）
- 快捷键表加一行：`| M | 静音 / 取消静音 |`；↑/↓ 行描述改为"音量 ±5（OSD 显示）"

- [ ] **Step 2: CHANGELOG**

`[Unreleased]` → `### Added` 追加：

```markdown
- 媒体播放：两行式全宽进度条（悬停显示目标时间，拖动关键帧对齐、松手精确跳转）。
- 媒体播放：画面右上角 OSD 反馈（音量、静音、快进快退、倍速、输出设备切换），CATextLayer 原生叠加约 1 秒淡出。
- 媒体播放：静音（底栏按钮 + M 键，静音时音量滑块灰显）与滚轮音量（视频区滚动，25px 一格 ±5%）。
- 媒体播放：音频输出设备选择（工具栏下拉框，自动 + mpv 枚举设备），保存的设备不存在时回退自动。
- 媒体播放：音量、倍速与输出设备全局记忆（`media_volume` / `media_speed` / `media_audio_device` 设置项）。
```

- [ ] **Step 3: AGENTS.md**

- Settings notable fields 列表加 `media_volume`、`media_speed`、`media_audio_device`。
- Media playback 要点追加一段：

```markdown
- **Media OSD**: transient feedback (volume, mute, seeks, speed, device
  switches) renders in a CATextLayer sublayer of the CAOpenGLLayer
  (`MpvNativeView::set_osd/clear_osd`) — egui cannot paint over the native
  video view. CoreAnimation's implicit opacity animation provides the fade;
  Rust only tracks the 1s expiry (`MediaView::show_osd/tick_osd`).
- **Media preferences**: volume/speed/audio-device are persisted globally in
  `Settings` and applied by `MediaView::apply_startup_settings` after open;
  a missing saved device falls back to "auto".
```

- [ ] **Step 4: 手工验证清单（执行人逐项过）**

准备一个含音频的视频文件与一个 mp3：
1. 进度条整宽显示；悬停各位置 tooltip 时间正确；拖动时画面按关键帧跳、松手后位置精确。
2. 视频区滚轮：音量增减，OSD 显示"音量 N%"；方向正确（上滚加音量，若反向则报告翻转）。
3. M 键/静音按钮：OSD"静音/取消静音"，滑块灰显，再按恢复。
4. ←/→/J/L 与工具栏 ±10s：OSD 显示 ±5s/±10s。
5. 1-4 与倍速按钮：OSD 显示倍速。
6. 输出下拉框列出本机设备（含 AG06/AG03 与显示器音频）；切换后声音从对应设备出；"输出: xxx" OSD 显示。
7. 调音量/倍速/设备后退出应用重开：三者保持。
8. 拔掉已保存的 USB 设备后打开媒体：自动回退、无报错。
9. 全屏下 OSD 在视频右上角可见，不与控制条重叠。
10. mp3：占位封面 + OSD/进度条同样工作。

- [ ] **Step 5: 完整流水线 + commit**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
git add README.md CHANGELOG.md AGENTS.md
git commit -m "docs: 媒体播放器体验补齐（进度条/OSD/静音/滚轮音量/设备选择/偏好记忆）"
```

---

## 自审记录

- **Spec 覆盖**：§4→Task 4，§5→Task 3（原生 OSD，见 §2.1 修订 1），§6→Task 1+4+5，§7→Task 5，§8→Task 1+6，§9→Task 2+5+6，§12 测试→各 Task 测试步 + Task 7 手工清单（osd_alpha 已按修订取消），§13 文档→Task 7。
- **类型一致性**：`show_osd(ctx, String)`、`set_media_volume(ctx, f64)`、`toggle_mute() -> Option<bool>`、`adjust_volume() -> Option<f64>`、`cycle_speed() -> Option<f64>`、`apply_startup_settings(...) -> bool` 在产出与消费任务间签名一致。
- **依赖顺序**：Task 1/2 可并行；Task 3 依赖 Task 1（无，独立）→ 实际仅 Task 5/6 依赖前序；按 1→2→3→4→5→6→7 顺序执行即可全部满足。
