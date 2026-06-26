# 电子书分页长期迁移计划：从 JS 行盒测量到 CSS Columns

> 状态：计划中  
> 目标：废弃当前的 `measure` + `cloneNode` + 行盒测量方案，改用浏览器原生 CSS `columns` 分页，彻底解决跨页重复/截断问题并降低维护成本。

---

## 1. 背景与动机

当前实现（`ebook_renderer_template.rs`）的核心流程是：

1. 把章节 HTML 注入隐藏的 `measure` 容器；
2. 用 `Range.getClientRects()` 收集所有文本行盒；
3. 在 JS 里逐页计算安全切分点；
4. 用 `cloneNode(true)` 复制整章内容，每页通过 `overflow: hidden` + 绝对定位偏移显示其中一段。

这套方案虽然理论上能精确控制分页，但存在以下结构性问题：

- **代码复杂**：`findSafeEnd`、`buildClonedSpread`、`buildDoubleSpread`、`splitSinglePage`、`splitDoublePage` 互相耦合，任何改动都影响多处。
- **浏览器子像素渲染不可控**：即使切分点计算在数学上正确，不同字体、字号、DPI 下仍可能出现 1~2px 的渗漏或截断。
- **对复杂 EPUB 支持差**：图片、表格、浮动元素、EPUB 自带 CSS 很容易破坏行盒假设。
- **性能开销大**：每章都要完整布局一次、克隆一次 DOM，长章节内存和 CPU 压力明显。

业界主流开源阅读器（Readium、Epub.js、Foliate-js）都采用 **CSS Multi-column Layout** 做分页。该方案把“如何断页”交给浏览器，阅读器只负责滚动/定位，工程上更可靠、维护成本更低。

---

## 2. 目标

1. 用 CSS `columns` 替代 JS 行盒测量作为默认分页引擎。
2. 保持现有功能不变：单页、双页、滚动模式、翻页动画、进度同步、目录跳转、字体/边距调整。
3. 彻底消除跨页重复行和最后一行截断问题。
4. 最终删除 `measure` 容器、`collectLineBoxes`、克隆分页相关代码。

---

## 3. 高层架构

```
当前架构：
  章节 HTML
      ↓
  measure 容器（隐藏）
      ↓
  collectLineBoxes → findSafeEnd
      ↓
  buildClonedSpread / buildDoubleSpread
      ↓
  spreads[] 数组（每页一段 HTML）
      ↓
  WebView 逐页渲染

目标架构：
  章节 HTML
      ↓
  注入到一个 column 容器
      ↓
  CSS columns 自动分栏（栏高 = 页高）
      ↓
  通过 transform / scrollLeft 移动视口
      ↓
  WebView 只渲染当前章节一次
```

---

## 4. 关键技术决策

### 4.1 单页模式

容器样式：

```css
#column-view {
  width: 100vw;
  height: calc(100vh - 2 * var(--margin-v));
  column-width: calc(100vw - 2 * var(--margin-h));
  column-gap: 0;
  column-fill: auto;
  overflow: hidden;
}
```

翻页通过 `transform: translateX(-N * pageWidth)` 或 `scrollLeft` 实现。

### 4.2 双页模式

CSS `columns: 2` 默认是“先从上到下、再向右”，不符合书籍的“左右两页连续”体验。推荐方案：

- 容器宽度设为 `2 * pageWidth + gap`；
- 外部视口只显示其中 `2 * pageWidth` 宽的区域；
- 每次翻页移动 `2 * pageWidth`。

这样浏览器仍按栏分页，但用户看到的是连续的左右两页。

### 4.3 滚动模式

滚动模式可复用同一容器，去掉 `column-*` 样式，启用 `overflow-y: scroll`。当前已有类似实现，改动较小。

### 4.4 翻页动画

可选两种策略：

1. **CSS transform 滑动**：直接移动 column 容器。简单、性能好，但双页模式下可能需要额外处理奇偶页。
2. **截图/快照翻页**：和当前 flipper 类似，用 `captureSpreadElement` 把当前视图和下一视图捕获成图片再做 3D 翻转。实现复杂但效果最接近现有体验。

建议先用方案 1，验证基本功能后再评估是否需要方案 2。

### 4.5 进度与跳转

- 总页数 = `scrollWidth / pageWidth`（单页）或 `scrollWidth / (2 * pageWidth)`（双页）。
- 当前页 = `scrollLeft / pageWidth`。
- 保存进度时记录 `chapter` + `scrollLeft`（或页索引）。
- 目录跳转：通过 `Element.scrollIntoView()` 或计算目标元素所在列的 `offsetLeft`。

### 4.6 图片与表格

需要在注入 CSS 中强制：

```css
img, table, figure, pre, blockquote {
  break-inside: avoid;
  max-height: var(--page-height);
}
```

对于超过一页高的图片/表格，浏览器会强制拆分，`break-inside: avoid` 只能尽量保证，无法完全避免。

---

## 5. 迁移阶段

### Phase 0：调研与原型（1~2 周）

- [ ] 在 `target/tmp/` 下创建独立 HTML 原型，验证 CSS columns 在各种 EPUB 内容（纯文本、图片、表格、列表、代码块）上的表现。
- [ ] 确认 wry WebView 对 `columns`、`column-fill`、`break-inside` 的支持情况。
- [ ] 测试单页、双页、滚动三种模式的原型。
- [ ] 输出原型结论报告，确定最终 CSS 策略。

### Phase 1：新分页器骨架（2~3 周）

- [ ] 在 `ebook_renderer_template.rs` 中新增 `columnPaginator.js` 模块（或内联函数）。
- [ ] 新增 feature flag，例如 `window.ebookUseColumns = true/false`，允许新旧方案并存。
- [ ] 实现：
  - 章节 HTML 注入 column 容器；
  - 根据设置应用 `columns: 1` / `columns: 2` / 滚动模式；
  - 暴露 `goToPage(index)`、`next()`、`prev()`、`getPageCount()`；
  - 通过 IPC 上报 `position`。
- [ ] Rust 侧 `ebook_renderer.rs` 增加对新 IPC 消息的兼容。

### Phase 2：功能对齐（3~4 周）

- [ ] 单页模式完整可用。
- [ ] 双页模式完整可用。
- [ ] 滚动模式完整可用。
- [ ] 翻页动画（先实现 transform 滑动，后续评估是否需要 3D 翻转）。
- [ ] 进度保存/恢复（resize、设置变更后仍能回到大致位置）。
- [ ] 目录跳转、搜索高亮。
- [ ] 字体、字号、行高、边距调整实时生效。

### Phase 3：测试与边缘情况（2~3 周）

- [ ] 收集 10~20 本不同类型 EPUB（小说、技术书、漫画混排、图文书）。
- [ ] 针对每本测试：单页、双页、滚动、字体放大、窗口缩放。
- [ ] 处理发现的问题：图片溢出、表格截断、特殊 CSS 冲突等。
- [ ] 添加自动化模板测试，断言 column 相关 CSS 和函数存在。

### Phase 4：清理旧代码（1~2 周）

- [ ] 删除 `measure` 容器和相关样式。
- [ ] 删除 `collectLineBoxes`、`findSafeEnd`、`blockAncestor`、`ancestorLi`。
- [ ] 删除 `buildClonedSpread`、`buildDoubleSpread`、`splitSinglePage`、`splitDoublePage`。
- [ ] 删除 `flipper` 3D 翻转相关代码（如果确定不再使用）。
- [ ] 删除旧的 spread 缓存逻辑（`spreadElementCache`）。
- [ ] 更新 `ebook_renderer_template.rs` 中的 Rust 测试。
- [ ] 更新 `AGENTS.md` 和 `CHANGELOG.md`。

### Phase 5：优化（持续）

- [ ] 相邻章节预加载。
- [ ] 缓存页数计算结果。
- [ ] 减少 resize 时的重新布局。
- [ ] 针对大章节优化内存（考虑分段加载）。

---

## 6. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| CSS columns 在不同 WebView 上表现不一致 | 分页结果有差异 | Phase 0 充分原型测试；保留 feature flag 可回退 |
| 某些 EPUB 自带 CSS 与 columns 冲突 | 布局错乱 | 注入 ReadiumCSS 风格的规范化样式；必要时对冲突样式做白名单/黑名单处理 |
| 图片/表格超过一页高 | 仍被截断 | `break-inside: avoid` 尽量保持完整；超大内容允许拆分并记录为已知限制 |
| 翻页动画质量下降 | 用户体验 | 先实现 transform 滑动；如不满足再投入 3D 翻转 |
| 进度/书签失效 | 数据兼容性 | 新进度格式用 `{ chapter, scrollLeft }`，旧格式兼容转换一个版本 |
| 长章节内存占用增加 | 性能 | Phase 5 考虑虚拟化或分段加载；初始版本可接受 |

---

## 7. 成功标准

- [ ] 在 10 本以上真实 EPUB 上翻页，无跨页重复行。
- [ ] 在 10 本以上真实 EPUB 上翻页，无最后一行截断。
- [ ] 单页/双页/滚动三种模式切换不崩溃、不丢进度。
- [ ] 窗口 resize、字体调整后 500ms 内重新分页完成。
- [ ] `cargo test --workspace` 全部通过。
- [ ] 代码中不再包含 `measure`、`collectLineBoxes`、`buildClonedSpread`。

---

## 8. 时间线估算

| 阶段 | 预计时间 | 产出 |
|---|---|---|
| Phase 0 | 1~2 周 | 原型与可行性报告 |
| Phase 1 | 2~3 周 | 新旧方案并存的 column paginator |
| Phase 2 | 3~4 周 | 功能完整的新分页器 |
| Phase 3 | 2~3 周 | 测试报告与 bug 修复 |
| Phase 4 | 1~2 周 | 旧代码清理完成 |
| Phase 5 | 持续 | 性能与体验优化 |

**总计：约 9~14 周完成核心迁移**（视实际投入时间而定）。

---

## 9. 废弃 方案A 后的当前代码状态

- `findSafeEnd` 已恢复为原始激进策略（待后续整个模块被删除）。
- 通用稳定性修复保留：
  - `MAX_SPREADS_PER_CHAPTER` 防止死循环；
  - `splitSinglePage` / `splitDoublePage` 的 `end <= start` 保护；
  - `debugSplit` 调试辅助；
  - `pageHeight()` 提前返回 0/负数的保护。
- 新增测试保留：
  - `test_reader_html_single_page_breaks_when_end_does_not_advance`
  - `test_reader_html_double_page_breaks_when_right_end_does_not_advance`

这些修复与分页引擎无关，迁移过程中仍可复用或参考。

---

## 10. 下一步行动

1. 审查并批准本计划。
2. 启动 Phase 0：创建一个最小 CSS columns 原型，用真实 EPUB 章节内容验证可行性。
3. 根据原型结果调整 Phase 1 的技术决策。
