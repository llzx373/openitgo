# Gallery Immersive — P3 / P4 备忘录

> 状态：未开始。P0–P2 已落地（tokens + Visuals、书架、统一沉浸 chrome）。
> 方向：Gallery Immersive（暗色优先、暖琥珀强调色、内容优先）。

## P3 — 设置页分组与密度

**目标：** 设置从「平铺长表单」变成可读、可扫的分组界面，密度与壳层 tokens 一致。

### 建议改动

1. **分区结构**（左侧分类或顶部大分组，二选一；优先顶部 Collapsing / 卡片分区以降低导航状态成本）：
   - 外观：主题、工具栏显示模式、背景色、菜单栏相关
   - 漫画阅读：默认模式 / 双页 / 适应 / 动画 / 滚轮阈值
   - 电子书：字体、字号、行距、边距、主题（可保留现有 ebook 专用设置入口）
   - 媒体：音量、倍速、默认音频设备
   - 性能：缓存、解码线程、压缩
   - 快捷键：现有编辑器迁入该分区
2. **视觉**：
   - 分区标题用 muted caption + 细分隔，勿堆重边框卡片
   - 控件对齐（标签列宽一致），说明文字用 `weak` + 12–12.5pt
   - 可选：主题切换旁放 Dark/Light 色块预览（读 `theme::dark_visuals` / `light_visuals` 的 panel/accent）
3. **落点文件**：`openitgo-app/src/views/settings.rs`；必要时抽 `theme` 辅助组件。
4. **不做**：不要为此引入新持久化字段；不要重做快捷键数据模型。

### 验收

- [ ] 设置页一屏内能认出各大分区
- [ ] 亮/暗主题下对比度可读
- [ ] 现有设置项行为与持久化不变

---

## P4 — 电子书 CSS 与壳层对齐 + 微交互

**目标：** 电子书 webview 阅读主题与 egui Gallery 壳层不再「两套皮肤」；补充克制的 hover / 进度反馈。

### 建议改动

1. **色值对齐**（`ebook_renderer.rs` / `ebook_renderer_template.rs`）：
   - Dark：背景贴近 `theme` panel `#141416`，正文 `#F0EEE8`，链接/强调用 `ACCENT_DARK`
   - Light：背景贴近 cool gallery `#EEEFF1`（避免暖奶油 `#F4F1EA`）
   - Sepia：保留独立羊皮纸气质，但圆角/间距语气与壳层一致
   - `ebook_theme_bg()`（egui 停放填充）与 JS `--bg` 同源，避免菜单停放时色差
2. **微交互（egui，克制）**：
   - 书架封面 hover：轻微提亮或 1px accent 描边（勿阴影堆叠）
   - 标签 chip / 分段 Tab：已有选中态，可补 hover fill
   - 漫画进度条：已用 accent；可考虑 hover 时加高 2px（可选）
3. **可选增强（若时间允许）**：
   - 漫画/媒体工具栏改为真正的 `Area` 浮层叠在内容上（电子书因 wry 命中穿透仍须占位 Panel）
   - 空历史 / 空书签复用书架空状态插画语气
4. **不做**：不做炫技动画、紫渐变、重阴影；不重写 wry 阅读器架构。

### 验收

- [ ] 电子书 Dark/Light 与壳层并排时色温接近
- [ ] 菜单停放填充与正文背景无明显跳变
- [ ] hover 反馈可感知但不抢内容

---

## 相关已完成（备查）

| 阶段 | 内容 | 主要落点 |
|------|------|----------|
| P0 | Gallery tokens + `Visuals`/`Style` | `openitgo-app/src/theme/mod.rs`，`ReaderApp::apply_theme` |
| P1 | 分段 Tab、圆角封面卡、chip、空状态 | `views/library.rs` |
| P2 | 三视图 chrome frame + Phosphor 统一 | `app.rs` 各 toolbar/statusbar |
