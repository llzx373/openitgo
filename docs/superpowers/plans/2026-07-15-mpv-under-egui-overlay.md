# mpv 视频层下沉到 egui 之下（方案 A）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 mpv 视频层从"egui 之上的原生 NSView"改为"egui 透明 surface 之下的 CA 子层"，让 egui 的菜单、下拉框、弹窗天然悬浮在视频画面之上，不再需要菜单打开时黑屏停放视频的 hack。

**Architecture:** eframe 0.29 原生支持透明 backbuffer：`ViewportBuilder::with_transparent(true)` 使 egui-wgpu 选择 `CompositeAlphaMode::PreMultiplied`，wgpu-hal 随之把 CAMetalLayer 设为 `opaque=NO`；`App::clear_color` 返回全透明后，egui 未绘制的区域透出下层内容。wgpu 的 metal layer 是 winit view 主 layer 的子层（wgpu-hal `addSublayer`），把 mpv 的 CAOpenGLLayer 以 `insertSublayer:atIndex:0` 插入同一 layer 树底部，视频即在 egui 之下合成。鼠标事件仍由 egui 的 NSView 接收（hit-test 按几何不按 alpha），交互路径不变。

**Tech Stack:** Rust, eframe 0.29.1 / egui-wgpu 0.29.1 / wgpu 22.1 / winit 0.30.13, AppKit/CoreAnimation via objc 0.2 (raw `msg_send!`), libmpv (CAOpenGLLayer render path)。

## Global Constraints

- 仅 macOS；非 macOS stub（`rust-reader-app/src/platform.rs` 的 `#[cfg(not(target_os = "macos"))] pub mod macos`）方法签名保持不变：`MpvNativeView::new(parent, bounds, player) -> Result<Self, String>` 与 `set_bounds(&self, bounds)`。
- UI 文本一律中文（专有名词/技术标识符除外）。
- 提交前跑完整流水线：`cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`，全绿才能 commit。
- 每个 Task 一个 commit，commit message 说明改动与涉及 crate。
- 最小改动：不顺手重构无关代码。
- OSD 维持现有 CATextLayer 方案（它随 mpv 层一起下沉，透过 egui 空洞可见）；本计划不把 OSD 迁回 egui。
- 电子书 wry webview 不在本计划范围（wry 内部自行 addSubview，下沉需改 wry 集成层；其菜单遮盖问题维持现状）。
- 不 push；合并/提交只在本地进行。

---

### Task 1: 合成验证 probe（`probe_overlay.rs`）

先验证核心假设：透明 wgpu surface + 下层 CA 层的合成、z-order、窗口 resize 后层级保持。

**Files:**
- Create: `rust-reader-app/examples/probe_overlay.rs`

**Interfaces:**
- Consumes: `eframe::Frame: HasWindowHandle`（`frame.window_handle()` 取 NSView；`views/media.rs:39` 已证明此 trait 实现存在）、`core_graphics::color::CGColor::rgb`。
- Produces: 无可被消费接口（诊断例子）。

- [ ] **Step 1: 写 probe 例子**

```rust
//! Overlay-compositing probe for the mpv-under-egui architecture: runs a
//! transparent eframe window with a red CALayer inserted at index 0 of the
//! winit view's layer (simulating the video layer), an opaque top bar, a
//! transparent central hole, and a semi-transparent green popup over the
//! hole. After 2s the window resizes once to prove the z-order survives
//! surface reconfiguration. Verify with a screenshot: the hole shows red,
//! the popup blends over red, the bars are fully opaque.
//! Usage: cargo run -p rust-reader-app --example probe_overlay

// objc 0.2's sel_impl macro carries a stale `cfg(feature = "cargo-clippy")`.
#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
mod imp {
    use objc::runtime::{Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use wry::raw_window_handle::RawWindowHandle;

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    struct ProbeApp {
        attached: bool,
        start: std::time::Instant,
        resized: bool,
    }

    impl ProbeApp {
        fn attach_red_layer(&mut self, frame: &eframe::Frame) {
            if self.attached {
                return;
            }
            let Ok(handle) = frame.window_handle() else {
                return;
            };
            let RawWindowHandle::AppKit(h) = handle.as_raw() else {
                return;
            };
            let ns_view = h.ns_view.as_ptr() as *mut Object;
            // SAFETY: all messages run on the UI thread against live AppKit
            // objects (the winit view and freshly allocated layers); selectors
            // match their receivers' classes.
            unsafe {
                let () = msg_send![ns_view, setWantsLayer: YES];
                let parent: *mut Object = msg_send![ns_view, layer];
                let red: *mut Object = msg_send![class!(CALayer), alloc];
                let red: *mut Object = msg_send![red, init];
                let cg = core_graphics::color::CGColor::rgb(1.0, 0.0, 0.0, 1.0)
                    .expect("valid RGB color");
                let () = msg_send![red, setBackgroundColor: cg];
                let bounds: core_graphics::geometry::CGRect = msg_send![parent, bounds];
                let frame_rect = core_graphics::geometry::CGRect::new(
                    &core_graphics::geometry::CGPoint::new(0.0, 60.0),
                    &core_graphics::geometry::CGSize::new(
                        bounds.size.width,
                        bounds.size.height - 120.0,
                    ),
                );
                let () = msg_send![red, setFrame: frame_rect];
                // Below wgpu's CAMetalLayer, which wgpu-hal added as the first
                // sublayer during surface init.
                let () = msg_send![parent, insertSublayer: red atIndex: 0u32];
                let () = msg_send![red, release];
            }
            self.attached = true;
        }
    }

    impl eframe::App for ProbeApp {
        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            [0.0, 0.0, 0.0, 0.0]
        }

        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.attach_red_layer(frame);
            if !self.resized && self.start.elapsed() > std::time::Duration::from_secs(2) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(900.0, 640.0)));
                self.resized = true;
            }
            let bar = egui::Frame::none().fill(egui::Color32::from_rgb(30, 60, 160));
            egui::TopBottomPanel::top("probe_top")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("不透明顶栏（应完全遮挡下层红色）");
                });
            egui::TopBottomPanel::bottom("probe_bottom")
                .frame(bar)
                .show(ctx, |ui| {
                    ui.label("不透明底栏");
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |_ui| {});
            // Semi-transparent popup inside the hole, like a dropdown menu.
            egui::Area::new(egui::Id::new("probe_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(120.0, 120.0))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgba_unmultiplied(0, 255, 0, 128))
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(220.0, 90.0));
                            ui.label("半透明弹层（应混色在红色之上）");
                        });
                });
            ctx.request_repaint();
        }
    }

    pub fn run() {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([700.0, 480.0])
            .with_transparent(true);
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            hardware_acceleration: eframe::HardwareAcceleration::Required,
            ..Default::default()
        };
        let start = std::time::Instant::now();
        eframe::run_native(
            "RUSTREADER_PROBE_OVERLAY",
            options,
            Box::new(|_cc| {
                Ok(Box::new(ProbeApp {
                    attached: false,
                    start,
                    resized: false,
                }))
            }),
        )
        .expect("eframe run_native failed");
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    imp::run();
    #[cfg(not(target_os = "macos"))]
    println!("probe_overlay 仅支持 macOS");
}
```

- [ ] **Step 2: 构建并运行，截取两张图**

```bash
cargo build -p rust-reader-app --example probe_overlay
cargo run -p rust-reader-app --example probe_overlay &
PROBE_PID=$!
sleep 1 && screencapture -x -o /tmp/probe_overlay_1.png   # resize 前
sleep 3 && screencapture -x -o /tmp/probe_overlay_2.png   # resize 后
kill $PROBE_PID
```

- [ ] **Step 3: 读图验证（人工/agent 目视，ReadMediaFile）**

逐条确认：
1. 中央空洞显示红色下层（不是黑色/桌面）。
2. 绿色弹层与红色混色（呈黄绿色），即 egui 弹层合成在下层内容之上。
3. 顶栏/底栏完全不透明，无红色透出。
4. resize 后（图 2）布局正确、红色仍在底层、无错位。
5. 窗口阴影正常，无透明穿帮（窗口外缘看不到桌面渗入）。

若第 1 条失败（红色不可见）：说明 `insertSublayer:atIndex:0` 的 z-order 或透明 backbuffer 未生效——**停止执行本计划**，回报现象，备选方案是 child window 下沉（`NSWindow addChildWindow:ordered:NSWindowBelow`），需重新设计 Task 4。

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/examples/probe_overlay.rs
git commit -m "test(app): 新增 probe_overlay 验证透明 egui 层与下层 CA 层合成"
```

---

### Task 2: 启动透明化（viewport + clear_color）

**Files:**
- Modify: `rust-reader-app/src/main.rs:30`
- Modify: `rust-reader-app/src/app.rs:170`（`impl eframe::App for ReaderApp` 内）

**Interfaces:**
- Consumes: `eframe::App::clear_color(&self, &egui::Visuals) -> [f32; 4]`（eframe 0.29 epi.rs:193）。
- Produces: 无。

- [ ] **Step 1: main.rs 开启透明视口**

```rust
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        // Transparent backbuffer: egui-wgpu then picks
        // CompositeAlphaMode::PreMultiplied and the CAMetalLayer becomes
        // non-opaque, so the video layer below the egui surface (Task 4)
        // shows through unpainted regions.
        .with_transparent(true);
```

- [ ] **Step 2: ReaderApp 覆盖 clear_color**

在 `impl eframe::App for ReaderApp` 中（`on_exit` 之前）加：

```rust
    /// Fully transparent: every view paints its own opaque panels, and in
    /// the media view the unpainted central area must let the video layer
    /// below the egui surface show through (see
    /// platform/macos/mpv_view.rs).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
```

- [ ] **Step 3: 跑流水线 + 目视冒烟**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run
```

Expected: 全绿；书架/阅读/设置各视图外观与之前完全一致（所有视图都有不透明面板填充，无透明穿帮）；媒体播放行为与今天相同（mpv 视图仍在上层，本任务不改变层级）。

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/main.rs rust-reader-app/src/app.rs
git commit -m "feat(app): 启用透明 backbuffer（viewport transparent + clear_color 全透明）"
```

---

### Task 3: 媒体中央面板透明化（视觉无变化的预备）

**Files:**
- Modify: `rust-reader-app/src/app.rs:623`（`render_media` 的 CentralPanel）

**Interfaces:**
- Consumes: 无（`MediaView::ui` 已在纯音频/错误时画不透明黑底，无需改动）。
- Produces: 无。

- [ ] **Step 1: CentralPanel 改为透明 Frame**

```rust
        // Transparent frame: the video layer composites below the egui
        // surface (Task 4), so the central area must stay unpainted for the
        // video to show through. Audio-only/error states still get an opaque
        // black fill from MediaView::ui.
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
```

（仅替换原 `egui::CentralPanel::default().show(ctx, |ui| {` 一行及上方注释；闭包内容不变。）

- [ ] **Step 2: 跑流水线 + 目视冒烟**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run
```

Expected: 全绿。此时 mpv 视图仍在 egui 之上且不透明，所以媒体播放外观与今天完全一致（透明面板被上层视频盖住，无可见变化）；纯音频/错误仍为黑底占位。

- [ ] **Step 3: Commit**

```bash
git add rust-reader-app/src/app.rs
git commit -m "refactor(app): 媒体视图中央面板改透明 Frame，为视频层下沉铺路"
```

---

### Task 4: MpvNativeView 裸层重构（层级翻转）

把 mpv 从"NSView 子视图（ egui 之上）"改为"CAOpenGLLayer 直接插入 winit view 的 layer 树底部（egui 之下）"。公开接口不变。

**Files:**
- Modify: `rust-reader-app/src/platform/macos/mpv_view.rs`（模块头注释、struct 字段、`new`、`set_bounds`、`set_osd`、`Drop`）
- 不改: 非 macOS stub（签名未变）；`make_frame`/`osd_frame`/测试不变。

**Interfaces:**
- Consumes: 现有 `make_frame(&wry::Rect) -> CGRect`、`osd_frame(w,h,tw,th) -> CGRect`、`layer_class()`、`LayerState`、`with_current_context`、`RenderContext`。
- Produces（签名不变）:
  - `MpvNativeView::new<W: HasWindowHandle + HasDisplayHandle>(parent: &W, bounds: wry::Rect, player: &MpvPlayer) -> Result<Self, String>`
  - `set_bounds(&self, bounds: wry::Rect)` / `set_osd(&self, text: &str)` / `clear_osd(&self)`

关键背景（实现前必读）：
- wgpu-hal 在 surface 初始化时把 CAMetalLayer `addSublayer` 到 winit view 的主 layer（wgpu-hal-22 metal/surface.rs:123-130）。本任务创建 mpv 层时 metal layer 已存在，`insertSublayer:atIndex:0` 即插到它下面。
- `make_frame` 是 (x,y,w,h) 直通转换（无 y 翻转）：winit view 的坐标系让今天的子视图定位正确，其 layer 坐标系一致，直接复用。**若 Task 4 验证发现视频上下错位**，改用在父 layer `bounds.size.height` 下做 `y' = parent_h - y - h` 的翻转公式（在 `set_bounds`/`new` 里包一层），并在模块注释记录实测结果。
- 裸 CA 层的几何变更默认带 0.25s 隐式动画（旧 NSView 路径由 AppKit 禁用）。所有 frame 变更必须包在 `CATransaction` + `setDisableActions:YES` 里；**OSD 的 opacity 淡入淡出依赖隐式动画，setOpacity 必须留在事务之外**。

- [ ] **Step 1: 重写模块头注释**

把文件顶部关于"NSView overlay 在 egui 之上"的说明改为（保持其余内容真实）：

```rust
//! macOS mpv video layer: a CAOpenGLLayer inserted at index 0 of the winit
//! view's layer tree, i.e. BELOW wgpu's CAMetalLayer. The egui surface is
//! non-opaque (transparent backbuffer, see main.rs), so the video shows
//! through the unpainted central area and egui menus/popups composite above
//! the video. Coordinates are top-left logical (make_frame passes them
//! through; the parent layer's coordinate space matches the flipped winit
//! view). The OSD is a CATextLayer sublayer of the video layer and is
//! visible through the same hole.
```

- [ ] **Step 2: struct 去掉 `view` 字段**

```rust
pub struct MpvNativeView {
    layer: *mut Object,
    osd_layer: *mut Object,
    state: *mut LayerState,
}
```

（同步更新 struct 上方生命周期注释：去掉 NSView 相关描述，说明 `layer` 由我们 alloc/init 持 +1，superlayer 经 insertSublayer 持有一份直到 Drop 中 removeFromSuperlayer。）

- [ ] **Step 3: `new` 的插入段改写**

把 `let (view, layer, osd_layer) = unsafe { ... };` 整段替换为：

```rust
        // SAFETY: all Objective-C messages below run on the UI thread that
        // owns the parent window; every object is a valid, live instance
        // (freshly allocated or the window's content view/layer), and
        // selectors match the receivers' classes.
        let (layer, osd_layer) = unsafe {
            // wgpu's CAMetalLayer is already a sublayer of the winit view's
            // layer (added by wgpu-hal at surface init). Insert the video
            // layer at index 0 so it composites below the transparent egui
            // surface. setWantsLayer first for probe windows without wgpu.
            let () = msg_send![ns_view, setWantsLayer: YES];
            let parent_layer: *mut Object = msg_send![ns_view, layer];
            let layer: *mut Object = msg_send![layer_class(), alloc];
            let layer: *mut Object = msg_send![layer, init];
            (*layer).set_ivar::<usize>("_rsState", state_ptr as usize);
            let () = msg_send![layer, setAsynchronous: YES];
            let () = msg_send![layer, setNeedsDisplayOnBoundsChange: YES];
            // Retina: match the window's backing scale.
            let window: *mut Object = msg_send![ns_view, window];
            let scale: f64 = msg_send![window, backingScaleFactor];
            let () = msg_send![layer, setContentsScale: scale];
            // Bare layers animate geometry changes implicitly; the video must
            // track egui layout exactly, so every frame change goes through a
            // disabled-actions transaction (also in set_bounds/set_osd). The
            // OSD opacity fade lives on osd_layer and is unaffected.
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: YES];
            let () = msg_send![layer, setFrame: make_frame(&bounds)];
            let () = msg_send![parent_layer, insertSublayer: layer atIndex: 0u32];
            let () = msg_send![class!(CATransaction), commit];
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
            (layer, osd_layer)
        };
```

构造函数返回改为 `Ok(Self { layer, osd_layer, state: state_ptr })`；new() 其余部分（GL 上下文、RenderContext、update callback）不变。

- [ ] **Step 4: `set_bounds` / `set_osd` 改用 layer 几何**

```rust
    pub fn set_bounds(&self, bounds: wry::Rect) {
        // SAFETY: self.layer/osd_layer are live objects owned by us.
        unsafe {
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: YES];
            let () = msg_send![self.layer, setFrame: make_frame(&bounds)];
            let () = msg_send![class!(CATransaction), commit];
            let lbounds: core_graphics::geometry::CGRect = msg_send![self.layer, bounds];
            let cur: core_graphics::geometry::CGRect = msg_send![self.osd_layer, frame];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(lbounds.size.width, lbounds.size.height, cur.size.width, cur.size.height)
            ];
        }
    }
```

`set_osd` 整段替换为（几何取自 layer bounds；frame 变更在事务内，opacity 在事务外以保留淡入动画）：

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
            let lbounds: core_graphics::geometry::CGRect = msg_send![self.layer, bounds];
            let () = msg_send![class!(CATransaction), begin];
            let () = msg_send![class!(CATransaction), setDisableActions: YES];
            let () = msg_send![
                self.osd_layer,
                setFrame: osd_frame(
                    lbounds.size.width,
                    lbounds.size.height,
                    text.width + 24.0,
                    text.height + 10.0,
                )
            ];
            let () = msg_send![class!(CATransaction), commit];
            // Outside the transaction: the opacity fade needs the implicit
            // animation that the transaction suppresses.
            let () = msg_send![self.osd_layer, setOpacity: 1.0f32];
        }
    }
```

- [ ] **Step 5: `Drop` 改 removeFromSuperlayer + 注释**

```rust
        // SAFETY: self.layer is a live CAOpenGLLayer owned by us; removing it
        // from the superlayer drops the superlayer's retain, leaving our
        // alloc/init retain (+1) balanced by the release below.
        unsafe {
            let () = msg_send![self.layer, removeFromSuperlayer];
        }
```

`self.view` 的 release 段删除；osd_layer 与 layer 的 release 保持不变（更新其上方 SAFETY 注释：由"view 经 setLayer 持有"改为"superlayer 经 insertSublayer 持有，已在上面解除"）。

- [ ] **Step 6: 跑流水线 + probes + 真机验证**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p rust-reader-app --example probe_visible -- <某个视频文件> 5
cargo run
```

Expected: 全绿（mpv_view 的 make_frame/osd_frame 测试不受影响）。probe_visible 正常出画面（裸层在 probe 的裸窗口 layer 树下同样工作）。真机打开视频：画面位置正确、无上下颠倒、OSD 右上角可见并淡出；菜单栏菜单与字幕/音轨/输出下拉框**悬浮在视频之上**；滚轮音量/快捷键正常（事件仍由 egui 接收）。若视频上下错位，按任务开头背景说明应用 y 翻转公式。

- [ ] **Step 7: Commit**

```bash
git add rust-reader-app/src/platform/macos/mpv_view.rs
git commit -m "refactor(app): MpvNativeView 改为裸 CAOpenGLLayer 插入 layer 树底部（视频层下沉到 egui 之下）"
```

---

### Task 5: 移除菜单停放 hack

**Files:**
- Modify: `rust-reader-app/src/app.rs`（`render_media` 的 bounds 条件与注释）

**Interfaces:**
- Consumes: 现有 `menu_overlay_open(ctx)`（保留，继续用于全屏下工具栏保持可见）。
- Produces: 无。

- [ ] **Step 1: bounds 条件去掉 `!menu_open`**

```rust
            // Audio-only or decode error: park the native layer at zero size
            // so the egui placeholder painted by MediaView::ui shows instead
            // of the video. Menus need no parking: the egui surface
            // composites above the video layer now.
            let bounds = if matches!(overlay, MediaOverlay::None) {
```

- [ ] **Step 2: 更新 menu_open 的注释**

`render_media` 顶部 `let menu_open = menu_overlay_open(ctx);` 的注释改为只提工具栏保持：

```rust
        // While a menu/dropdown is open, keep the toolbar up (otherwise the
        // dropdown self-dismisses in fullscreen when the pointer leaves the
        // top edge).
        let menu_open = menu_overlay_open(ctx);
```

- [ ] **Step 3: 跑流水线 + 真机验证**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run
```

Expected: 全绿。打开菜单/下拉框时视频**不再黑屏让位**，菜单直接悬浮在画面上；全屏下工具栏在菜单打开期间仍保持可见。

- [ ] **Step 4: Commit**

```bash
git add rust-reader-app/src/app.rs
git commit -m "refactor(app): 移除菜单打开时的视频停放（层级翻转后菜单天然在视频之上）"
```

---

### Task 6: 文档 + 手工验证清单

**Files:**
- Modify: `AGENTS.md`（Media playback / Media menus/popups 两段）
- Modify: `CHANGELOG.md`（Unreleased → Changed/Fixed）
- Modify: `README.md`（如有描述菜单行为的旧句则更新）

**Interfaces:**
- Consumes: Task 1-5 全部产出。
- Produces: 无。

- [ ] **Step 1: AGENTS.md 更新**

Media playback 段开头改为：

```markdown
- **Media playback** renders mpv video through a CAOpenGLLayer inserted at
  index 0 of the winit view's layer tree — BELOW wgpu's CAMetalLayer
  (`rust-reader-app/src/platform/macos/mpv_view.rs`). The app runs with a
  transparent backbuffer (`with_transparent(true)` + `clear_color` returning
  zero alpha) and the media view's CentralPanel uses a transparent frame, so
  the video shows through the unpainted central area while egui menus,
  dropdowns and popups composite above it. Hit-testing is unaffected (the
  egui NSView still receives all events). Bare-layer geometry changes must go
  through a `CATransaction` with disabled actions; the OSD opacity fade
  relies on implicit animation and must stay outside such transactions.
  Playback progress is persisted in `HistoryEntry.char_offset` (milliseconds).
  Inside `drawInCGLContext`, CoreAnimation binds its own drawable FBO
  (observed: 1/2, alternating — never 0); the draw must query
  `GL_FRAMEBUFFER_BINDING` and pass it to `RenderContext::render`, because
  rendering to FBO 0 leaves the layer's drawable untouched and composites
  fully transparent. `FLIP_Y` must be 1 for this drawable. Audio output
  defaults to the system device (`auto`) and can be switched at runtime
  (see Media preferences below).
```

Media menus/popups 段改为：

```markdown
- **Media menus/popups**: with the video layer below the transparent egui
  surface, egui overlays (menu-bar menus, the 字幕/音轨/输出 dropdowns)
  naturally render above the video. `menu_overlay_open(ctx)` (visible
  `Order::Middle`/`Order::Foreground` areas) is still used to keep the media
  toolbar from auto-hiding in fullscreen while a menu is open. The media
  seek bar needs a scoped `ui.spacing_mut().slider_width` override: egui 0.29
  `Slider` always allocates `slider_width` (100px) and ignores `add_sized`.
  The diagnostic examples `probe_overlay.rs` (transparent-compositing proof)
  and `probe_visible.rs` (real video compositing) verify the layering.
```

Media OSD 段：把"egui cannot paint over the native video view"一句改为"the CATextLayer lives inside the video layer below egui and shows through the transparent central area"。

- [ ] **Step 2: CHANGELOG 更新**

Unreleased → `### Changed` 追加：

```markdown
- 媒体播放：视频层从 egui 之上的原生 NSView 改为 egui 透明 surface 之下的 CA 子层（透明 backbuffer 合成）；菜单栏菜单与字幕/音轨/输出下拉框现在直接悬浮在视频画面之上，打开菜单时视频不再黑屏让位。
```

- [ ] **Step 3: README 检查**

搜索 README 中"菜单""遮盖""让位"等描述，如有与旧行为相关的句子按新行为更新；没有则不改。

- [ ] **Step 4: 手工验证清单（执行人逐项过，准备一个有音频的视频与一个 mp3）**

1. 打开视频：画面正常、位置正确（不被上下工具栏遮挡）、无上下颠倒、无缩放错位。
2. 菜单栏菜单与字幕/音轨/输出下拉框：悬浮在视频之上，视频不黑屏；选择生效。
3. 滚轮音量、M 键静音、↑/↓、←/→/J/L、1-4：OSD 右上角显示并淡出。
4. mp3：黑色占位 + 标题 + OSD 正常；进度条可拖。
5. 全屏：视频铺满；菜单可用且工具栏不自动消失；ESC/按钮退出全屏正常。
6. 窗口缩放与拖动：视频跟随无残影无错位；有条件的话在不同 scale 的显示器间拖动窗口验证清晰度。
7. 书架/阅读/电子书/设置各视图外观与之前一致，无透明穿帮（看不到桌面渗入）。
8. 媒体打开→关闭→再打开循环 5 次：无崩溃、无残留画面。

- [ ] **Step 5: 跑流水线 + Commit**

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
git add AGENTS.md CHANGELOG.md README.md
git commit -m "docs: 视频层下沉架构（透明 backbuffer 合成）文档更新"
```

---

## 风险与回滚

- **最大风险点**是 Task 1 的合成假设与 Task 4 的坐标系。Task 1 失败则整体改走 child window 方案（重新设计 Task 4）；Task 4 坐标错位有计划内备选公式。
- Task 2/3 落地后 app 行为与今天一致（mpv 仍在上层），Task 4 是唯一的行为翻转点；任一任务出问题可单独 `git revert`，不影响其他任务。
- `with_transparent(true)` 让 NSWindow 变为非不透明：所有视图都有不透明面板填充，理论上无穿帮；Task 6 清单第 7 条兜底验证。窗口阴影由 window server 按内容 alpha 计算，保持正常（Task 1 清单第 5 条先验证）。
