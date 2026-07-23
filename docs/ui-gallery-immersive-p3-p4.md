# Gallery Immersive — P3 / P4 备忘录

> 状态：P3 / P4 已落地（2026-07-23）。P0–P2 见 CHANGELOG。
> 方向：Gallery Immersive（暗色优先、暖琥珀强调色、内容优先）。

## P3 — 设置页分组与密度（已完成）

设置页用一体式 Tab 面板（`tabbed_page`：Tab 栏与内容无缝连接）：外观 / 漫画 / 电子书 / 媒体 / 性能 / 快捷键；
Tab 等宽固定尺寸，hover 只变色不变宽；说明文字 `weak` 12.5pt；主题旁 Dark/Light 色块预览。
从电子书进入时默认选中「电子书」Tab。

落点：`openitgo-app/src/views/settings.rs`，`theme::{tabbed_page,theme_swatch}`。

## P4 — 电子书 CSS 与壳层对齐 + 微交互（已完成）

1. **色值**：`theme::EbookPalette` 统一 Dark/Light（贴近 panel `#141416` / `#EEEFF1`）与
   Sepia；`JsSettings`、webview CSS `--bg/--fg/--accent`、`ebook_theme_bg` 同源。
2. **微交互**：书架封面 hover 琥珀描边 + 轻提亮；标签 chip hover fill；
   空历史 / 空书签复用空状态插画语气。
3. **明确不做**：工具栏 Area 浮层盖在正文上（透明度应对窗口清屏，见 `chrome_opacity`）。

落点：`theme/mod.rs`、`ebook_renderer.rs`、`ebook_renderer_template.rs`、
`views/ebook.rs`、`views/library.rs`。

---

## 相关已完成（备查）

| 阶段 | 内容 | 主要落点 |
|------|------|----------|
| P0 | Gallery tokens + `Visuals`/`Style` | `openitgo-app/src/theme/mod.rs`，`ReaderApp::apply_theme` |
| P1 | 分段 Tab、圆角封面卡、chip、空状态 | `views/library.rs` |
| P2 | 三视图 chrome frame + Phosphor 统一 | `app.rs` 各 toolbar/statusbar |
| + | `chrome_opacity` 阅读栏/阅读区背景共用 | `settings` + Panel chrome |
| P3 | 设置分组与密度 | `views/settings.rs` |
| P4 | 电子书色值对齐 + 微交互 | `EbookPalette` + library hover |
