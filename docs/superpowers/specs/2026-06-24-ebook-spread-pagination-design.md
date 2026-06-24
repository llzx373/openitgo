# 电子书 Spread 分页设计方案

> 状态：已实现（Task 1–14 已落地并通过自动验证；Task 15 手动 GUI 测试需用户在桌面环境确认）

## 背景与问题

当前电子书单页/双页模式使用 CSS 多列布局（`column-width`）把所有页面横向排列在一个 DOM 容器里，通过改变 `scrollLeft` 翻页。随着翻页次数增加，`scrollLeft` 与 `pageWidth × N` 之间的舍入误差会累积，导致视口左侧出现上一页内容的“漏边”，影响阅读体验。

## 目标

- 彻底消除单页/双页模式下的横向漏边问题。
- 保持翻页动画（3D 翻页，背面为下一页水平镜像）。
- 连续滚动模式保持不变，并保留竖直滚动条。
- 不破坏漫画阅读器相关代码。

## 已确认决策

| 问题 | 决策 |
|------|------|
| 单页模式 spread 大小 | 1 页 = 1 个 spread |
| 双页模式 spread 大小 | 2 页 = 1 个 spread |
| 跨章节翻页 | 严格按章节边界，章末“下一页”自动进入下一章第一页 |
| 连续滚动模式 | 保持现有整章长文档，不参与 spread 改造 |
| 切分位置 | JS 端在隐藏容器中让浏览器真实排版后切分 |
| 预加载 | 预加载当前 ±1 spread |
| 跨 spread 选区 | 接受选区在翻页后丢失的权衡 |
| 翻页动画 | 保留 3D 翻页，每次只捕获当前 spread |

## 架构

```text
+-----------------------------------------+
| Rust EbookRenderer                      |
| - 维护 current_chapter / spread / offset|
| - 提供 ebook://reader 壳页面            |
| - 提供 ebook://reader?chapter=N 章节 HTML|
| - 通过 IPC 接收位置更新                 |
+-----------------------------------------+
                    ↑↓ 自定义协议 + IPC
+-----------------------------------------+
| JS 壳页面                               |
| - 加载章节 HTML 到隐藏测量容器          |
| - 按视口尺寸切成 spreads[]              |
| - 渲染当前 spread，预加载相邻 spread    |
| - 处理点击/滚轮/键盘，执行翻页动画      |
| - IPC 上报 chapter / spread / offset    |
+-----------------------------------------+
```

## 核心组件

### 1. 壳页面（`ebook://reader`）

一个轻量、不变的 HTML/JS 容器。负责：
- 接收设置（主题、字体、字号、行高、边距、动画开关、滚轮反转）。
- 加载章节内容。
- 管理 spread 生命周期和用户交互。

### 2. 测量容器（`#measure`）

- `position: absolute; visibility: hidden; pointer-events: none;`
- 与主渲染区 `#spread` 使用完全相同的字体、字号、行高、边距、box-model。
- 只用于让浏览器对整章内容做一次真实排版，从而精确获得每一页（或每一对页）的边界。
- 双页模式下临时把宽度设为 `50%`，使测量宽度与渲染单元宽度一致，避免重排错位。

### 3. Spread 数组

JS 根据测量容器的高度/列宽，把一章切分为 `spreads[]`。每个元素是一段 HTML 片段，对应：
- 单页模式：1 页内容。
- 双页模式：2 页并排内容。

### 4. 主渲染容器（`#spread`）

- 分页模式：只显示当前 `spreads[currentSpread]`，宽度/高度占满视口（含边距），`overflow: hidden`。
- 滚动模式：显示完整章节 HTML，`overflow-y: scroll`。

### 5. 预加载池（`spreadElementCache`）

内存中保留当前 spread 前后各 1 个 spread 的已解析 DOM 元素，非相邻 spread 会被自动清理。翻页时优先从缓存取内容，避免反复解析 HTML。

### 6. Flipper（`#flipper`，3D 翻页动画层）

捕获范围从“整章”缩小到“当前 spread”。
- `front`：当前 spread 的快照。
- `back`：目标 spread 快照的水平镜像。
- 动画方向由目标 spread 与当前 spread 的相对位置决定。
- 章节切换或设置变化时取消未完成的动画，避免状态错乱。
- 动画完成后清理 flipper，主容器已更新为目标 spread。

## 数据流

### 打开章节

1. Rust 调用 `goto_chapter(chapter, offset)`。
2. JS 请求 `ebook://reader?chapter=N` 获取完整章节 HTML。
3. JS 把 HTML 写入隐藏测量容器，等待布局稳定。
4. JS 按视口尺寸切分 `spreads[]`。
5. JS 找到包含 `offset` 的 spread 作为 `currentSpread`。
6. JS 渲染当前 spread，预加载相邻 spread。
7. JS 通过 IPC 上报 `{ chapter, spread, char_offset }`。

### 翻页

1. 用户点击、滚轮或快捷键触发 `nextPage()` / `prevPage()`。
2. JS 检查边界：
   - 当前章节第一页时“上一页”进入上一章最后一页。
   - 当前章节最后一页时“下一页”进入下一章第一页。
3. 如果动画开启，使用 flipper 播放 3D 翻页；否则直接替换内容。
4. 更新 `currentSpread`，渲染新 spread，预加载相邻 spread。
5. IPC 上报新位置。

### 设置变化

1. Rust 调用 `apply_settings(settings)`。
2. JS 更新 CSS 变量和 body class。
3. 由于字体/字号/边距变化会影响分页，JS 在分页模式下重新测量当前章节并重新切分 spread。
4. 通过当前 spread 的累计文本长度计算字符偏移，重新切分后用 `findSpreadForOffset` 恢复到大致相同位置。

### 窗口大小变化

1. `resize` 事件触发后防抖。
2. 重新测量当前章节并切分 spread。
3. 保持当前字符偏移对应的 spread。

## 协议变更

保留现有协议：
- `ebook://reader` → 返回壳页面 HTML。
- `ebook://reader?chapter=N` → 返回第 N 章完整 HTML。

不需要新增 `ebook://spread/...` 协议，因为 spread 内容在 JS 端从完整章节 HTML 中切分。

## IPC 消息格式

保留并扩展现有 `JsToRust`：

```json
{
  "type": "position",
  "chapter": 0,
  "spread": 3,
  "char_offset": 1250,
  "total_spreads": 12
}
```

新增 `spread` 字段表示当前 spread 序号；`total_spreads` 用于状态栏显示总 spread 数。

## Rust 状态扩展

`RendererState` 增加：
- `current_spread: usize`
- `total_spreads: usize`

`EbookRenderer` 增加：
- `current_spread(&self) -> usize`
- `current_spread_count(&self) -> usize`

状态栏/工具栏从 `current_spread` + `total_spreads` 显示“第 X / Y 页”。

## 错误处理

- 章节 HTML 加载失败：在壳页面显示错误信息，IPC 报告失败状态。
- 测量容器异常（如内容为空）：降级为只渲染一个包含全部内容的 spread，避免崩溃。
- 翻页越界：在章节边界自动切换章节；如果已经是第一章/最后一章，停止翻页。
- 设置变化后重新测量失败：保留当前渲染内容，记录错误日志。

## 测试策略

1. **单元测试**：
   - `JsSettings` 正确传递 `animate` / `invert_scroll`。
   - IPC 消息中 `spread` / `total_spreads` 正确反序列化。
   - `EbookRenderer::current_spread_count` 返回正确值。

2. **HTML/JS 测试**：
   - 壳页面包含 `measure`、`spread`、`flipper` 等关键元素。
   - `reader_html` 包含 `splitIntoSpreads`、`goToSpread`、`nextPage`、`prevPage` 等函数。

3. **集成测试（手动）**：
   - 打开一个 EPUB，切换单页/双页，翻页无漏边。
   - 改变字号后边距正常，无漏边。
   - 跨章节翻页正常。
   - 3D 翻页动画正常。
   - 连续滚动模式不受影响。

## 影响范围

- `rust-reader-app/src/ebook_renderer.rs`：扩展 IPC 协议和 `RendererState`，注入章节总数。
- `rust-reader-app/src/ebook_renderer_template.rs`：壳页面 HTML/JS 模板，实现测量、切分、预加载、3D 翻页、输入、resize/设置重测。
- `rust-reader-app/src/views/ebook.rs`：扩展 `OpenEbook` 跟踪 `current_spread`。
- `rust-reader-app/src/app.rs`：状态栏/工具栏显示 spread 页码。
- `rust-reader-storage/src/models.rs`：无需新增字段（复用 `enable_page_animation` / `invert_scroll`）。

## 不做的范围

- 不修改漫画阅读器代码。
- 不改 EPUB/TXT 解析逻辑。
- 不改变书架、历史、书签的数据结构（仍用 `page_index` 表示章节，`char_offset` 表示字符偏移）。

## 实现备注

- 实际代码中 `spreads` 数组存储的是 HTML 字符串，预加载池将其解析为 DOM 元素；`getSpreadElement(index)` 负责按需解析或复用缓存。
- `findSpreadForOffset` 当前使用文本长度累加的近似算法，跨图片或内联元素时可能有 1 spread 漂移，后续可优化为二分查找实际渲染范围。
- 详细实现步骤见 `docs/superpowers/plans/2026-06-24-ebook-spread-pagination.md`。
