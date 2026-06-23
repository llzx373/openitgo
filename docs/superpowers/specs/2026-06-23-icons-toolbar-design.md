> **Status:** 已确认，待实现。
>
> **依赖决策：** 工具栏图标使用 Phosphor Icons 字体；应用图标选用候选方案 C；工具栏显示模式仅放在「设置」页。

# 图标与工具栏显示模式设计

## 目标

1. 为 rustReader 生成/准备一套符合 macOS 标准的应用图标，并在运行时设置为窗口图标。
2. 为阅读器工具栏的每个动作添加图标。
3. 支持三种工具栏显示模式：仅图标、仅文字、图标+文字，并在设置中持久化。

## 范围

- 应用图标：生成 macOS 所需的 16/32/128/256/512/1024 px PNG 与 `.icns`，并提供 `.app` bundle 所需的 `Info.plist` 模板。
- 工具栏图标：仅使用图标字体（不引入 PNG/SVG 资源），减少资源维护成本。
- 显示模式：影响 `render_reader_toolbar` 中每个按钮的渲染方式；默认「图标+文字」。

## 应用图标

### 设计选择

选用 AI 生成的候选方案 C：`assets/icon_candidates/app_icon_003.jpg`（玻璃质感花瓣状书页，色彩丰富）。

### macOS 标准处理

- 基础尺寸 1024×1024 px。
- 导出为以下 PNG：`assets/icon/16x16.png`、`32x32.png`、`128x128.png`、`256x256.png`、`512x512.png`、`1024x1024.png`。
- 使用 `iconutil` 或 Python 脚本打包为 `assets/icon/AppIcon.icns`。
- 提供 `assets/icon/Info.plist.template`，包含 `CFBundleIconFile` 等字段，供后续打包 `.app` 使用。

### 运行时窗口图标

在 `rust-reader-app/src/main.rs` 的 `eframe::run_native` 中，通过 `NativeOptions::viewport.icon` 传入 `IconData`：

```rust
let icon = load_icon(include_bytes!("../assets/icon/1024x1024.png"));
let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 720.0])
        .with_icon(icon),
    ..Default::default()
};
```

`load_icon` 将 PNG 解码为 RGBA 字节并构造 `egui::IconData`。

## 工具栏图标

### 图标字体

使用 [Phosphor Icons](https://phosphoricons.com/)（MIT 许可证）：

- 添加依赖 `phosphor-egui = "0.7"`（提供 `PhosphorFonts` 与图标常量）。
- 在 `ReaderApp::new` 中安装字体到 `egui::FontDefinitions`。

### 动作到图标映射

| 动作 | Phosphor 图标 | Unicode |
|---|---|---|
| 返回书架 | House | \u{e13e} |
| 国漫（LTR）| ArrowRight | \u{e12a} |
| 日漫（RTL）| ArrowLeft | \u{e128} |
| 韩漫（Webtoon）| ArrowDown | \u{e126} |
| 双页 | BookOpen | \u{e14f} |
| 缩小 | Minus | \u{e1f4} |
| 放大 | Plus | \u{e25b} |
| 适应宽度 | ArrowsOutHorizontal | \u{e13c} |
| 适应高度 | ArrowsOutVertical | \u{e13d} |
| 自动适应 | FrameCorners | \u{e18e} |
| 上一页 | CaretLeft | \u{e15c} |
| 下一页 | CaretRight | \u{e15e} |
| 添加书签 | Bookmark | \u{e148} |
| 全屏 | ArrowsOutSimple | \u{e13a} |
| 设置 | Gear | \u{e1a1} |
| 隐藏工具栏 | X | \u{e2b3} |

> 若 `phosphor-egui` 版本不同，实际常量以 crate 文档为准；备选可直接使用 TTF 文件 + `egui::FontFamily::Name`。

## 工具栏显示模式

### 数据模型

在 `rust-reader-storage/src/models.rs` 新增枚举与字段：

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarDisplayMode {
    #[default]
    IconAndText,
    IconOnly,
    TextOnly,
}

pub struct Settings {
    // ... existing fields ...
    pub toolbar_display_mode: ToolbarDisplayMode,
}
```

默认值：`IconAndText`。

### 设置 UI

在 `rust-reader-app/src/views/settings.rs` 中增加下拉框：

```rust
ui.label("工具栏显示模式");
egui::ComboBox::from_id_salt("toolbar_display_mode")
    .selected_text(toolbar_mode_label(settings.toolbar_display_mode))
    .show_ui(ui, |ui| {
        ui.selectable_value(&mut settings.toolbar_display_mode, ToolbarDisplayMode::IconAndText, "图标+文字");
        ui.selectable_value(&mut settings.toolbar_display_mode, ToolbarDisplayMode::IconOnly, "仅图标");
        ui.selectable_value(&mut settings.toolbar_display_mode, ToolbarDisplayMode::TextOnly, "仅文字");
    });
```

### 工具栏渲染

在 `rust-reader-app/src/app.rs::render_reader_toolbar` 中，根据 `self.settings.toolbar_display_mode` 生成按钮内容：

- `IconAndText`：`ui.button(format!("{icon} {text}"))`
- `IconOnly`：`ui.button(icon).on_hover_text(text)`
- `TextOnly`：保持现有文字按钮

模式选择器（国漫/日漫/韩漫）在 `IconOnly` 模式下使用图标+悬停文字，当前激活项高亮。

## 错误处理

- 图标字体加载失败：回退到纯文字按钮，记录 `eprintln!` 警告。
- 应用图标 PNG 缺失/解码失败：不设置窗口图标，应用仍可启动。

## 测试计划

- 编译通过：`cargo check --workspace`。
- 设置序列化往返测试：在 `rust-reader-storage` 中验证 `ToolbarDisplayMode` 的 JSON 往返。
- 手动验证：启动应用后工具栏显示图标；切换显示模式后重启仍生效；窗口图标在 macOS Dock/标题栏可见。

## 变更文件

- `rust-reader-app/Cargo.toml`：添加 `phosphor-egui`。
- `rust-reader-app/src/main.rs`：加载窗口图标。
- `rust-reader-app/src/app.rs`：工具栏使用图标、支持显示模式。
- `rust-reader-app/src/views/settings.rs`：增加显示模式设置。
- `rust-reader-storage/src/models.rs`：新增 `ToolbarDisplayMode` 与字段。
- `assets/icon/`：应用图标 PNG 与 `.icns`。
- `assets/icon/Info.plist.template`：macOS bundle 图标配置模板。
- `docs/superpowers/specs/2026-06-23-icons-toolbar-design.md`：本文档。
