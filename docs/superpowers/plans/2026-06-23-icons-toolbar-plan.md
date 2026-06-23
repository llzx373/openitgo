> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 rustReader 生成 macOS 标准应用图标、设置运行时窗口图标，并改造阅读器工具栏支持图标/文字混合显示模式。

**Architecture:** 应用图标使用 AI 生成的 `assets/icon_candidates/app_icon_003.jpg`，通过 Python/Pillow 脚本导出为 macOS 标准尺寸 PNG 与 `.icns`；工具栏图标使用 `egui-phosphor` 图标字体，在设置中新增 `ToolbarDisplayMode` 枚举并在 `render_reader_toolbar` 中按模式渲染按钮。

**Tech Stack:** Rust, egui 0.29, eframe 0.29, egui-phosphor 0.7, Python/Pillow, iconutil.

---

## File Structure

| File | Change |
|---|---|
| `assets/icon/16x16.png` ... `1024x1024.png` | 新增应用图标各尺寸 PNG |
| `assets/icon/AppIcon.icns` | 新增 macOS 图标集 |
| `assets/icon/Info.plist.template` | 新增 macOS bundle 图标配置模板 |
| `assets/icon_candidates/preview.html` | 已存在，保留作为历史候选 |
| `rust-reader-app/Cargo.toml` | 新增 `egui-phosphor = "0.7"` |
| `rust-reader-app/src/main.rs` | 加载窗口图标并传给 `NativeOptions` |
| `rust-reader-app/src/app.rs` | 初始化 Phosphor 字体、改造工具栏按钮渲染 |
| `rust-reader-app/src/views/settings.rs` | 增加工具栏显示模式设置 |
| `rust-reader-storage/src/models.rs` | 新增 `ToolbarDisplayMode` 与设置字段 |
| `rust-reader-storage/src/models.rs` (tests) | 补充序列化往返测试 |

---

## Task 1: 从候选图生成 macOS 图标资源

**Files:**
- Create: `assets/icon/generate_icons.py`
- Create: `assets/icon/16x16.png`, `32x32.png`, `128x128.png`, `256x256.png`, `512x512.png`, `1024x1024.png`
- Create: `assets/icon/AppIcon.icns`
- Create: `assets/icon/Info.plist.template`

- [ ] **Step 1: 编写图标导出脚本**

`assets/icon/generate_icons.py`:

```python
#!/usr/bin/env python3
from pathlib import Path
from PIL import Image

src = Path(__file__).parent.parent / "icon_candidates" / "app_icon_003.jpg"
dst_dir = Path(__file__).parent
sizes = [16, 32, 128, 256, 512, 1024]

img = Image.open(src).convert("RGBA")
# The generated image already has rounded corners on a transparent/white background.
# Resize to square and save each size.
for size in sizes:
    resized = img.resize((size, size), Image.Resampling.LANCZOS)
    resized.save(dst_dir / f"{size}x{size}.png", "PNG")

print("Generated PNG icons:", sizes)
```

- [ ] **Step 2: 运行脚本生成 PNG**

Run:

```bash
cd /Users/liu/srcs/rustReader
python3 assets/icon/generate_icons.py
```

Expected: `assets/icon/` 下出现 6 张 PNG。

- [ ] **Step 3: 使用 iconutil 打包 `.icns`**

Run:

```bash
cd /Users/liu/srcs/rustReader/assets/icon
mkdir -p AppIcon.iconset
cp 16x16.png   AppIcon.iconset/icon_16x16.png
cp 32x32.png   AppIcon.iconset/icon_16x16@2x.png
cp 32x32.png   AppIcon.iconset/icon_32x32.png
cp 64x64.png   AppIcon.iconset/icon_32x32@2x.png || sips -z 64 64 128x128.png --out AppIcon.iconset/icon_32x32@2x.png
cp 128x128.png AppIcon.iconset/icon_128x128.png
cp 256x256.png AppIcon.iconset/icon_128x128@2x.png
cp 256x256.png AppIcon.iconset/icon_256x256.png
cp 512x512.png AppIcon.iconset/icon_256x256@2x.png
cp 512x512.png AppIcon.iconset/icon_512x512.png
cp 1024x1024.png AppIcon.iconset/icon_512x512@2x.png
iconutil -c icns AppIcon.iconset -o AppIcon.icns
rm -rf AppIcon.iconset
```

Expected: 生成 `AppIcon.icns`。

- [ ] **Step 4: 添加 Info.plist 模板**

`assets/icon/Info.plist.template`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIconName</key>
    <string>AppIcon</string>
</dict>
</plist>
```

- [ ] **Step 5: 提交图标资源**

```bash
cd /Users/liu/srcs/rustReader
git add assets/icon/
git commit -m "assets: add macOS app icon set from candidate C"
```

---

## Task 2: 添加 `ToolbarDisplayMode` 设置

**Files:**
- Modify: `rust-reader-storage/src/models.rs`
- Test: `rust-reader-storage/src/models.rs`

- [ ] **Step 1: 定义枚举并加入 `Settings`**

在 `rust-reader-storage/src/models.rs` 中 `LibrarySort` 之后添加：

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarDisplayMode {
    #[default]
    IconAndText,
    IconOnly,
    TextOnly,
}
```

在 `Settings` 结构体中添加字段：

```rust
pub struct Settings {
    // ... existing fields ...
    pub library_sort: LibrarySort,
    pub toolbar_display_mode: ToolbarDisplayMode,
}
```

在 `Default for Settings` 中：

```rust
impl Default for Settings {
    fn default() -> Self {
        Self {
            // ... existing fields ...
            library_sort: LibrarySort::default(),
            toolbar_display_mode: ToolbarDisplayMode::default(),
        }
    }
}
```

- [ ] **Step 2: 更新 settings round-trip 测试**

在 `test_settings_roundtrip_with_background_color` 中，序列化前添加：

```rust
s.toolbar_display_mode = ToolbarDisplayMode::IconOnly;
```

反序列化后断言：

```rust
assert_eq!(loaded.toolbar_display_mode, ToolbarDisplayMode::IconOnly);
```

- [ ] **Step 3: 运行 storage 测试**

Run:

```bash
cargo test -p rust-reader-storage
```

Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add rust-reader-storage/src/models.rs
git commit -m "feat(storage): add ToolbarDisplayMode setting"
```

---

## Task 3: 添加图标字体依赖与初始化

**Files:**
- Modify: `rust-reader-app/Cargo.toml`
- Modify: `rust-reader-app/src/app.rs`（字体安装）
- Modify: `rust-reader-app/src/main.rs`（窗口图标）

- [ ] **Step 1: 添加 `egui-phosphor` 依赖**

`rust-reader-app/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
egui-phosphor = "0.7"
```

- [ ] **Step 2: 在 App 初始化时安装 Phosphor 字体**

在 `rust-reader-app/src/app.rs` 中，找到 `ReaderApp::new` 或创建位置（通常在 `eframe::run_native` 的闭包中）。在拿到 `CreationContext` 后调用：

```rust
use egui_phosphor::add_to_fonts;

fn setup_phosphor_fonts(cc: &eframe::CreationContext<'_>) {
    let mut fonts = egui::FontDefinitions::default();
    add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    cc.egui_ctx.set_fonts(fonts);
}
```

在 `main.rs` 的 `eframe::run_native` 闭包中：

```rust
eframe::run_native(
    "rustReader",
    options,
    Box::new(|cc| {
        setup_phosphor_fonts(cc);
        Ok(Box::new(ReaderApp::new(cc)))
    }),
)
```

- [ ] **Step 3: 加载运行时窗口图标**

`rust-reader-app/src/main.rs`:

```rust
fn load_app_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon/1024x1024.png");
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}
```

在 `main` 中构造 `NativeOptions`：

```rust
fn main() {
    let icon = load_app_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_icon(icon),
        ..Default::default()
    };
    // ... run_native ...
}
```

- [ ] **Step 4: 检查编译**

Run:

```bash
cargo check -p rust-reader-app
```

Expected: no errors。

- [ ] **Step 5: 提交**

```bash
git add rust-reader-app/Cargo.toml rust-reader-app/src/main.rs rust-reader-app/src/app.rs
git commit -m "feat(app): add phosphor icon font and window icon"
```

---

## Task 4: 改造工具栏按钮渲染

**Files:**
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: 添加工具栏按钮辅助函数**

在 `app.rs` 的 `ReaderApp` impl 附近添加：

```rust
use rust_reader_storage::models::ToolbarDisplayMode;

fn toolbar_button(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    match mode {
        ToolbarDisplayMode::IconOnly => ui.button(icon).on_hover_text(text),
        ToolbarDisplayMode::TextOnly => ui.button(text),
        ToolbarDisplayMode::IconAndText => ui.button(format!("{} {}", icon, text)),
    }
}

fn toolbar_selectable(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    active: bool,
    mode: ToolbarDisplayMode,
) -> egui::Response {
    let label = match mode {
        ToolbarDisplayMode::IconOnly => egui::WidgetText::from(icon),
        ToolbarDisplayMode::TextOnly => egui::WidgetText::from(text),
        ToolbarDisplayMode::IconAndText => egui::WidgetText::from(format!("{} {}", icon, text)),
    };
    ui.selectable_label(active, label).on_hover_text(text)
}
```

- [ ] **Step 2: 替换 `render_reader_toolbar` 中的按钮**

引入图标常量（使用 `egui_phosphor::regular` 中的 Unicode 字符串）：

```rust
use egui_phosphor::regular;
```

在 `render_reader_toolbar` 开头读取模式：

```rust
let mode = self.settings.toolbar_display_mode;
```

替换各按钮为：

```rust
if toolbar_button(ui, regular::HOUSE, "书架", mode).clicked() {
    self.current_view = View::Library;
}

// 模式选择器
let modes = [
    (ReadingMode::Ltr, regular::ARROW_RIGHT, "国漫"),
    (ReadingMode::Rtl, regular::ARROW_LEFT, "日漫"),
    (ReadingMode::Webtoon, regular::ARROW_DOWN, "韩漫"),
];
for (m, icon, label) in modes {
    if toolbar_selectable(ui, icon, label, mode == m, mode).clicked() {
        if let Some(reader) = self.reader_view.open.as_mut() {
            reader.state.set_mode(m, total_pages);
        }
    }
}

// 双页
if mode != ReadingMode::Webtoon {
    let double_page = ...;
    if toolbar_selectable(ui, regular::BOOK_OPEN, "双页", double_page, mode).clicked() {
        // ... existing logic ...
    }
}

// 缩放/适应
if toolbar_button(ui, regular::MINUS, "", mode).clicked() { reader.zoom_out(); }
// zoom label 保持
if toolbar_button(ui, regular::PLUS, "", mode).clicked() { reader.zoom_in(); }
if toolbar_button(ui, regular::ARROWS_OUT_HORIZONTAL, "适应宽度", mode).clicked() { ... }
if toolbar_button(ui, regular::ARROWS_OUT_VERTICAL, "适应高度", mode).clicked() { ... }
if toolbar_button(ui, regular::FRAME_CORNERS, "自动适应", mode).clicked() { ... }

// 翻页
if toolbar_button(ui, regular::CARET_LEFT, "上一页", mode).clicked() { self.reader_prev_page(); }
// page input / total label 保持
if toolbar_button(ui, regular::CARET_RIGHT, "下一页", mode).clicked() { self.reader_next_page(); }

if toolbar_button(ui, regular::BOOKMARK, "添加书签", mode).clicked() { self.add_bookmark(current_page); }
if toolbar_button(ui, regular::ARROWS_OUT_SIMPLE, "全屏", mode).clicked() { self.toggle_fullscreen(ctx); }
if toolbar_button(ui, regular::GEAR, "设置", mode).clicked() { self.current_view = View::Settings; }

// 隐藏工具栏（仅图标，保持简洁）
ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
    if ui.button(regular::X).on_hover_text("隐藏工具栏").clicked() {
        self.settings.show_toolbar = false;
    }
});
```

> 注：`regular::ARROWS_OUT_HORIZONTAL` / `ARROWS_OUT_VERTICAL` / `FRAME_CORNERS` 等常量需以 `egui-phosphor` 0.7 实际导出为准；若不存在，使用相近图标名（如 `ARROWS_OUT`、`SCAN`）替换。

- [ ] **Step 3: 检查编译**

Run:

```bash
cargo check -p rust-reader-app
```

Expected: no errors。

- [ ] **Step 4: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(toolbar): render reader toolbar with phosphor icons"
```

---

## Task 5: 在设置页添加显示模式选项

**Files:**
- Modify: `rust-reader-app/src/views/settings.rs`

- [ ] **Step 1: 引入枚举并添加下拉框**

```rust
use rust_reader_storage::models::{Settings, Theme, ToolbarDisplayMode};
```

在设置 UI 中合适位置（建议在「主题」之前或「默认缩放」之后）添加：

```rust
ui.label("工具栏显示模式");
egui::ComboBox::from_id_salt("toolbar_display_mode")
    .selected_text(toolbar_mode_label(settings.toolbar_display_mode))
    .show_ui(ui, |ui| {
        ui.selectable_value(
            &mut settings.toolbar_display_mode,
            ToolbarDisplayMode::IconAndText,
            "图标 + 文字",
        );
        ui.selectable_value(
            &mut settings.toolbar_display_mode,
            ToolbarDisplayMode::IconOnly,
            "仅图标",
        );
        ui.selectable_value(
            &mut settings.toolbar_display_mode,
            ToolbarDisplayMode::TextOnly,
            "仅文字",
        );
    });
```

添加辅助函数：

```rust
fn toolbar_mode_label(mode: ToolbarDisplayMode) -> &'static str {
    match mode {
        ToolbarDisplayMode::IconAndText => "图标 + 文字",
        ToolbarDisplayMode::IconOnly => "仅图标",
        ToolbarDisplayMode::TextOnly => "仅文字",
    }
}
```

- [ ] **Step 2: 检查编译**

Run:

```bash
cargo check -p rust-reader-app
```

Expected: no errors。

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/views/settings.rs
git commit -m "feat(settings): add toolbar display mode selector"
```

---

## Task 6: 全量验证与文档更新

**Files:**
- Modify: `docs/superpowers/specs/2026-06-23-icons-toolbar-design.md`（如实现与计划有偏差则更新）
- Modify: `CHANGELOG.md`

- [ ] **Step 1: 运行完整检查**

Run:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all PASS。

- [ ] **Step 2: 更新 CHANGELOG.md**

在 Unreleased 下添加：

```markdown
- [新增] 应用图标与工具栏图标（Phosphor Icons）
- [新增] 工具栏显示模式：图标+文字 / 仅图标 / 仅文字
```

- [ ] **Step 3: 提交并推送**

```bash
git add CHANGELOG.md docs/superpowers/specs/2026-06-23-icons-toolbar-design.md
git commit -m "docs: update changelog for icons and toolbar display mode"
git push
```

---

## Self-Review

- **Spec coverage:**
  - 应用图标资源生成：Task 1
  - 运行时窗口图标：Task 3
  - 工具栏图标字体：Task 3 + Task 4
  - 显示模式设置与持久化：Task 2 + Task 5
  - 工具栏按模式渲染：Task 4
  - 测试：Task 2、Task 6
- **Placeholder scan:** 无 TBD；图标常量名以实际 crate 为准的提示已说明。
- **Type consistency:** `ToolbarDisplayMode` 名称在 storage/models、settings view、app toolbar 中一致。
