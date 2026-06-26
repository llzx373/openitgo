# 方案B：动态遮罩防止电子书跨页漏行

> 目标：消除前一页底部露出下一页第一行顶部 1~2 像素的问题，同时不误伤当前页最后一行的降部（如 g/j/p/q/y）。

## 原理

当前切分算法在 `findSafeEnd` 中，当某行盒跨越 target 边界时，把切分点设在跨页行的 `lineTop`，`Math.floor` 后作为 `end`。**测量容器中最后可见行的 lineBottom 与 end 之间的空隙 = end - lastLineBottom**。

泄漏发生在空隙极小（<2px）时，浏览器子像素渲染会让下一页第一行的顶部几像素渗透到当前页 cell 的底部。

在 cell 底部加一个高度动态计算的遮罩层，只覆盖空隙范围 + 2px 余量：

```
遮罩高度 = clamp(2, end - lastLineBottom + 2, 8)
```

- `end - lastLineBottom`：页底空隙的精确像素值
- `+2`：额外 2px 容差
- `clamp(2, ..., 8)`：最少 2px，最多 8px（防异常值）

遮罩颜色与背景色相同，置于 cell 最上层（z-index），覆盖泄漏内容。

## 改哪些文件

只改 1 个文件：`rust-reader-app/src/ebook_renderer_template.rs`

## 改哪些函数

### 1. `findSafeEnd`（行 263–284）—— 返回值扩展

**现状**：返回 `Math.floor(safeEnd)`，只返回切分点。

**改动**：查完跨页行后，向前找一个完全在 target 之前的行盒作为"最后可见行"。返回 `{ end: Number, lastLineBottom: Number }`。

**伪代码**：

```javascript
function findSafeEnd(boxes, start, target) {
  let safeEnd = target;
  const n = boxes.length;
  let i = 0;
  while (i < n && boxes[i].lineTop <= start) i++;
  let j = i;
  let lastLineBottom = 0;          // 新增：最后可见行的 lineBottom
  let foundCrossing = false;
  while (j < n && boxes[j].lineTop <= target) {
    if (boxes[j].lineBottom > target) {
      const lineTop = boxes[j].lineTop;
      if (lineTop > start) {
        safeEnd = Math.min(safeEnd, lineTop);
      }
      // j-1 是最后可见行（如果存在）
      if (j > i && boxes[j - 1].lineBottom) {
        lastLineBottom = boxes[j - 1].lineBottom;
      }
      foundCrossing = true;
      break;
    }
    lastLineBottom = boxes[j].lineBottom;  // 每进一步更新最后一个可见行
    j++;
  }
  const end = Math.floor(safeEnd);
  // 如果没找到跨页行（整段都在一页内），lastLineBottom 取最后一行
  if (!foundCrossing && j > 0) {
    lastLineBottom = boxes[j - 1].lineBottom;
  }
  return { end: end, lastLineBottom: lastLineBottom };
}
```

### 2. `buildClonedSpread`（行 286–311）—— 加遮罩

**现状**：签名 `buildClonedSpread(start, end)`

**改动**：新增 `lastLineBottom` 参数，在 cell 底部加遮罩 div。

**伪代码**：

```javascript
function buildClonedSpread(start, end, lastLineBottom) {
  const safety = spreadSafety();
  const ph = end - start;
  const cell = document.createElement('div');
  cell.style.position = 'relative';
  cell.style.overflow = 'hidden';
  cell.style.height = ph + 'px';
  const clone = measure.cloneNode(true);
  clone.removeAttribute('id');
  clone.style.position = 'absolute';
  const marginV = getMarginV();
  const offset = start - marginV;
  clone.style.top = -offset + 'px';
  clone.style.width = '100%';
  cell.appendChild(clone);

  // ---- 新增：底部遮罩 ----
  if (lastLineBottom > 0 && end > lastLineBottom) {
    const gap = end - lastLineBottom;         // 页底空隙
    const maskH = Math.max(2, Math.min(8, gap + 2));  // 夹在 [2, 8] 之间
    const mask = document.createElement('div');
    mask.style.cssText =
      'position:absolute;bottom:0;left:0;right:0;' +
      'height:' + maskH + 'px;' +
      'background:var(--bg);' +
      'z-index:10;pointer-events:none;';
    cell.appendChild(mask);
  }
  // ---- 新增结束 ----

  const wrapper = document.createElement('div');
  wrapper.style.height = (ph + 2 * safety) + 'px';
  wrapper.style.paddingTop = safety + 'px';
  wrapper.style.paddingBottom = safety + 'px';
  wrapper.style.boxSizing = 'border-box';
  wrapper.appendChild(cell);
  return wrapper.outerHTML;
}
```

### 3. `buildDoubleSpread`（行 313–342）—— 加遮罩

**现状**：签名 `buildDoubleSpread(leftStart, leftEnd, rightEnd, ph)`

**改动**：新增 `leftLastLineBottom` 和 `rightLastLineBottom` 两个参数。内部 `makeCell` 也接受 `lastLineBottom`，在每个 cell 底部加遮罩。

**伪代码**：

```javascript
function buildDoubleSpread(leftStart, leftEnd, rightEnd, ph,
                           leftLastLineBottom, rightLastLineBottom) {
  const safety = spreadSafety();
  const wrapper = document.createElement('div');
  wrapper.style.display = 'flex';
  wrapper.style.width = '100%';
  wrapper.style.height = (ph + 2 * safety) + 'px';
  wrapper.style.paddingTop = safety + 'px';
  wrapper.style.paddingBottom = safety + 'px';
  wrapper.style.boxSizing = 'border-box';

  function makeCell(start, end, lastLineBottom) {
    const cell = document.createElement('div');
    cell.style.flex = '1';
    cell.style.height = ph + 'px';
    cell.style.overflow = 'hidden';
    cell.style.position = 'relative';
    const clone = measure.cloneNode(true);
    clone.removeAttribute('id');
    clone.style.position = 'absolute';
    const marginV = getMarginV();
    const offset = start - marginV;
    clone.style.top = -offset + 'px';
    clone.style.width = '100%';
    cell.appendChild(clone);

    // 同 buildClonedSpread 的遮罩逻辑
    if (lastLineBottom > 0 && end > lastLineBottom) {
      const gap = end - lastLineBottom;
      const maskH = Math.max(2, Math.min(8, gap + 2));
      const mask = document.createElement('div');
      mask.style.cssText =
        'position:absolute;bottom:0;left:0;right:0;' +
        'height:' + maskH + 'px;' +
        'background:var(--bg);' +
        'z-index:10;pointer-events:none;';
      cell.appendChild(mask);
    }

    return cell;
  }

  wrapper.appendChild(makeCell(leftStart, leftEnd, leftLastLineBottom));
  wrapper.appendChild(makeCell(leftEnd, rightEnd, rightLastLineBottom));
  return wrapper.outerHTML;
}
```

### 4. `splitSinglePage`（行 344–376）—— 调用方适配

**现状**：第 363 行 `let end = findSafeEnd(...)`，第 367 行 `buildClonedSpread(start, end)`。

**改动**：解构 `findSafeEnd` 返回值，传递 `lastLineBottom`。

```javascript
// 第 363 行，原来：
let end = findSafeEnd(boxes, start, target);
// 改为：
const safe = findSafeEnd(boxes, start, target);
let end = safe.end;

// 第 364-366 行的 end 判断不变
// 第 367 行，原来：
spreads.push(buildClonedSpread(start, end));
// 改为：
spreads.push(buildClonedSpread(start, end, safe.lastLineBottom));
```

### 5. `splitDoublePage`（行 378–419）—— 调用方适配

**现状**：第 401 行 `leftEnd = findSafeEnd(...)`，第 404 行 `rightEnd = findSafeEnd(...)`，第 408 行 `buildDoubleSpread(...)`。

**改动**：同理，提取并传递 `lastLineBottom`。

```javascript
// 第 401 行
const leftSafe = findSafeEnd(boxes, start, start + ph);
let leftEnd = leftSafe.end;
// leftEnd 的 fallback 判断（403 行）保持不变

// 第 404 行
const rightSafe = findSafeEnd(boxes, leftEnd, leftEnd + ph);
let rightEnd = rightSafe.end;
// rightEnd 的 fallback 判断（405 行）保持不变

// 第 408 行，原来：
spreads.push(buildDoubleSpread(start, leftEnd, rightEnd, ph));
// 改为：
spreads.push(buildDoubleSpread(start, leftEnd, rightEnd, ph,
               leftSafe.lastLineBottom, rightSafe.lastLineBottom));
```

## 需要更新的测试

`ebook_renderer_template.rs` 行 801–1070 的 Rust 端模板测试。需要确认以下断言仍然通过：

1. `test_reader_html_uses_line_box_pagination`（行 839）：确认新增的遮罩相关字符串（如 `z-index:10;pointer-events:none` 或 `bottom:0;left:0;right:0`）出现在模板中
2. 所有现有测试不应因为接口变更而失败（模板渲染函数 `reader_html` 本身不变，测试是对生成的 HTML 字符串做 `assert!(html.contains(...))`）

## 验证方式

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

并手动打开一个 EPUB，翻页检查底部是否还有泄漏痕迹。

## 边界情况

| 情况 | 行为 |
|------|------|
| `lastLineBottom = 0`（第一行就跨页） | 遮罩高度 = 2px（最小值），略保守但无害 |
| 章节最后一段跨页 | `findSafeEnd` 中 `j > i` 检查保证不取到无效的 boxes[j-1] |
| `end - lastLineBottom` 很大（>20px，大段空白） | 遮罩高度限制为 8px，不会遮盖过多 |
| boxes 为空 | `splitSinglePage` 已在 353 行提前返回 `[html]`，不走到 build 逻辑 |
| 无跨页行（整段一页放得下） | `findSafeEnd` 返回 `{ end: Math.floor(target), lastLineBottom: boxes[last].lineBottom }`，遮罩按正常逻辑计算 |
