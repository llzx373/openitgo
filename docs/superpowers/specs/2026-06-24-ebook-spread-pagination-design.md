# 电子书 Spread 分页设计方案

> 状态：已评审通过，待写入实现计划

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

### 2. 测量容器（`div.measure`）

- `position: absolute; visibility: hidden; pointer-events: none;`
- 与主渲染区使用完全相同的字体、字号、行高、边距、列宽参数。
- 只用于让浏览器对整章内容做一次真实排版，从而精确获得每一页（或每一对页）的边界。
- 测量完成后即可从 DOM 中移除，内存占用是临时的。

### 3. Spread 数组

JS 根据测量容器的高度/列宽，把一章切分为 `spreads[]`。每个元素是一段 HTML 片段，对应：
- 单页模式：1 页内容。
- 双页模式：2 页并排内容。

### 4. 主渲染容器（`div.spread`）

只显示当前 `spreads[currentSpread]`。宽度占满视口，高度也占满视口（减去边距）。

### 5. 预加载池

内存中保留：
- 上一 spread（用于“上一页”动画）
- 当前 spread
- 下一 spread（用于“下一页”动画）

翻页时优先从预加载池取内容，避免白屏。

### 6. Flipper（3D 翻页动画层）

与当前实现类似，但捕获范围从“整章”缩小到“当前 spread”。
- `front`：当前 spread 的快照。
- `back`：下一 spread 快照的水平镜像。
- 动画完成后清理 flipper，主容器已更新为下一 spread。

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
3. 由于字体/字号/边距变化会影响分页，JS 需要重新测量当前章节并重新切分 spread。
4. 尽量保持当前阅读位置（按字符偏移或 spread 序号）不变。

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

- `rust-reader-app/src/ebook_renderer.rs`：重写壳页面 HTML/JS，扩展 IPC 协议和状态。
- `rust-reader-app/src/views/ebook.rs`：扩展 `OpenEbook` 跟踪 `current_spread`。
- `rust-reader-app/src/app.rs`：状态栏/工具栏显示 spread 页码；跨章节翻页逻辑。
- `rust-reader-storage/src/models.rs`：无需新增字段（复用 `enable_page_animation` / `invert_scroll`）。

## 不做的范围

- 不修改漫画阅读器代码。
- 不改 EPUB/TXT 解析逻辑。
- 不改变书架、历史、书签的数据结构（仍用 `page_index` 表示章节，`char_offset` 表示字符偏移）。
