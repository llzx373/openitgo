//! HTML/JS shell template for the ebook webview renderer.

use super::JsSettings;
use rust_reader_storage::models::EbookSettings;

pub fn reader_html(settings: &EbookSettings, chapter_count: usize) -> String {
    let js = JsSettings::from(settings);
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<style>
:root {{
  --bg: {bg};
  --fg: {fg};
  --font: {font};
  --size: {size}px;
  --line: {line};
  --margin-h: {margin_h}px;
  --margin-v: {margin_v}px;
  --column-gutter: 40px;
}}
* {{ box-sizing: border-box; }}
html, body {{
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: var(--bg);
  color: var(--fg);
  font-family: var(--font);
  font-size: var(--size);
  line-height: var(--line);
}}
p {{ margin: 0 0 1em 0; text-indent: 2em; }}
img {{ max-width: 100%; max-height: calc(100vh - var(--margin-v) * 2); height: auto; object-fit: contain; }}
#flipper {{
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  perspective: 1500px;
  display: none;
  z-index: 100;
}}
#flipper .sheet {{
  position: absolute;
  top: 0;
  height: 100%;
  transform-style: preserve-3d;
  transition: transform 0.45s ease-in-out;
}}
#flipper .front, #flipper .back {{
  position: absolute;
  width: 100%;
  height: 100%;
  backface-visibility: hidden;
  overflow: hidden;
  background: var(--bg);
}}
#flipper .back {{
  transform: rotateY(180deg) scaleX(-1);
}}
#measure {{
  position: absolute;
  visibility: hidden;
  pointer-events: none;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  padding: var(--margin-v) var(--margin-h);
  box-sizing: border-box;
  overflow: hidden;
}}
#spread {{
  display: none;
  width: 100%;
  height: 100%;
  padding: var(--margin-v) var(--margin-h);
  box-sizing: border-box;
  overflow: hidden;
}}
body.scroll #spread {{
  overflow-y: scroll;
}}
#column-view {{
  display: none;
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: var(--bg);
}}
#column-content {{
  height: 100%;
  padding: var(--margin-v) 0;
  column-fill: auto;
  background: var(--bg);
  color: var(--fg);
  font-family: var(--font);
  font-size: var(--size);
  line-height: var(--line);
}}
#column-content img, #column-content table, #column-content figure, #column-content pre, #column-content blockquote {{
  break-inside: avoid;
  max-width: 100%;
  max-height: calc(100% - 2 * var(--margin-v));
}}
body.scroll #column-view {{
  overflow-x: hidden;
  overflow-y: scroll;
}}
body.scroll #column-content {{
  height: auto;
  min-height: 100%;
  column-count: 1;
  column-width: auto;
  column-gap: 0;
  width: auto;
  padding: var(--margin-v) var(--margin-h);
}}
#column-view.column-animate {{
  transition: transform 0.25s ease;
}}
</style>
</head>
<body class="{mode}">
<div id="measure"></div>
<div id="spread"></div>
<div id="column-view"><div id="column-content"></div></div>
<div id="flipper"></div>
<script>
const measure = document.getElementById('measure');
const spread = document.getElementById('spread');
const flipper = document.getElementById('flipper');
const columnView = document.getElementById('column-view');
const columnContent = document.getElementById('column-content');
window.ebookUseColumns = false;
let currentChapter = 0;
let currentSpread = 0;
let spreads = [];
let currentChapterHtml = '';
window.ebookChapterCount = {chapter_count};
let isFlipping = false;
let pendingFlipTarget = null;
const RESIZE_DEBOUNCE_MS = 200;
let currentSettings = {{
  mode: '{mode}',
  animate: {animate},
  invert_scroll: {invert_scroll}
}};
const SPREAD_SAFETY_PX = 4;
const MAX_SPREADS_PER_CHAPTER = 10000;
function spreadSafety() {{ return SPREAD_SAFETY_PX; }}

function escapeHtml(s) {{
  return String(s).replace(/[&<>"']/g, function(c) {{
    return {{ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }}[c];
  }});
}}

function showError(title, detail) {{
  const spread = document.getElementById('spread');
  if (!spread) return;
  spread.style.display = 'block';
  spread.innerHTML = '<div id="ebook-error" style="padding:2em; color:var(--fg); background:var(--bg); font-family:var(--font); font-size:var(--size);">' +
    '<h2 style="margin-top:0">电子书渲染错误</h2>' +
    '<p><strong>' + escapeHtml(title) + '</strong></p>' +
    '<pre style="white-space:pre-wrap; word-break:break-all; opacity:0.8;">' + escapeHtml(detail) + '</pre>' +
    '</div>';
  try {{
    if (typeof ipc !== 'undefined') {{
      ipc.postMessage(JSON.stringify({{ type: 'error', error: title + ': ' + detail }}));
    }}
  }} catch (e) {{}}
}}

// Prevent anchors and other navigation from reloading the shell.
document.addEventListener('click', function(e) {{
  let el = e.target;
  while (el && el !== document.body) {{
    if (el.tagName === 'A') {{
      e.preventDefault();
      e.stopPropagation();
      return;
    }}
    el = el.parentElement;
  }}
}}, true);
window.addEventListener('beforeunload', function(e) {{
  e.preventDefault();
  e.returnValue = '';
}});

function sendIpc(obj) {{
  const json = JSON.stringify(obj);
  if (typeof window.ipc !== 'undefined' && window.ipc && window.ipc.postMessage) {{
    window.ipc.postMessage(json);
  }} else {{
    setTimeout(() => sendIpc(obj), 10);
  }}
}}

function debugSplit(label, fullPh, maxBottom, count) {{
  sendIpc({{ type: 'debug', label: label, pageHeight: fullPh, maxBottom: Math.round(maxBottom), spreads: count }});
}}

function isScrollMode() {{ return document.body.classList.contains('scroll'); }}
function isDoubleMode() {{ return document.body.classList.contains('double'); }}

// --- CSS columns paginator (Phase 1) ---
function isColumnMode() {{ return window.ebookUseColumns === true; }}

let columnAnimationTimer = null;

function columnEnableAnimation() {{
  if (columnView) columnView.classList.add('column-animate');
}}

function columnDisableAnimation() {{
  if (columnAnimationTimer) {{
    clearTimeout(columnAnimationTimer);
    columnAnimationTimer = null;
  }}
  if (columnView) columnView.classList.remove('column-animate');
}}

function columnScheduleDisableAnimation() {{
  if (columnAnimationTimer) clearTimeout(columnAnimationTimer);
  columnAnimationTimer = setTimeout(() => {{
    columnAnimationTimer = null;
    columnDisableAnimation();
  }}, 260);
}}

let columnState = {{
  currentSpread: 0,
  totalPages: 1,
  pageWidth: 0,
  viewShift: 0
}};

function columnGetPageCount() {{
  if (isScrollMode()) {{
    if (!columnView) return 1;
    const ch = Math.max(1, columnView.clientHeight);
    return Math.max(1, Math.ceil(columnView.scrollHeight / ch));
  }}
  return (columnState && columnState.totalPages) || 0;
}}

function columnScrollStep() {{
  if (!columnView) return 1;
  return Math.max(1, columnView.clientHeight - 2 * getMarginV());
}}

function columnMaxScroll() {{
  if (!columnView) return 0;
  return Math.max(0, columnView.scrollHeight - columnView.clientHeight);
}}

function columnShow() {{
  spread.style.display = 'none';
  columnView.style.display = 'block';
}}

function columnHide() {{
  if (columnView) columnView.style.display = 'none';
}}

function columnLayout() {{
  if (!columnContent) return;
  columnShow();
  if (isScrollMode()) {{
    columnShow();
    columnContent.style.width = 'auto';
    columnContent.style.paddingLeft = 'var(--margin-h)';
    columnContent.style.paddingRight = 'var(--margin-h)';
    columnContent.style.columnWidth = 'auto';
    columnContent.style.columnCount = '1';
    columnContent.style.columnGap = '0';
    columnContent.style.height = 'auto';
    columnContent.style.minHeight = '100%';
    columnView.style.transform = 'none';
    columnState.totalPages = 1;
    columnState.currentSpread = 0;
    columnReportPosition();
    return;
  }}

  const viewportW = document.body.clientWidth;
  const marginH = getMarginH();
  const gutter = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--column-gutter')) || 40;

  if (isDoubleMode()) {{
    const pageW = Math.max(1, Math.floor((viewportW - gutter) / 2));
    const colW = Math.max(1, pageW - 2 * marginH);
    const colGap = gutter + 2 * marginH;
    const viewShift = viewportW + gutter;
    columnContent.style.width = viewportW + 'px';
    columnContent.style.paddingLeft = marginH + 'px';
    columnContent.style.paddingRight = marginH + 'px';
    columnContent.style.columnWidth = colW + 'px';
    columnContent.style.columnGap = colGap + 'px';
    columnContent.style.columnCount = 'auto';
    const scrollW = columnContent.scrollWidth;
    columnState.pageWidth = pageW;
    columnState.viewShift = viewShift;
    columnState.totalPages = Math.max(1, Math.ceil((scrollW - 2 * marginH) / viewShift));
  }} else {{
    // 单页模式下内容宽度等于页宽，水平边距通过 transform 整体偏移实现；
    // 双页模式需要把边距放在每栏内部，因此用 padding 控制，column-gap 只保留栏间距。
    const pageW = Math.max(1, viewportW);
    const colW = Math.max(1, pageW - 2 * marginH);
    columnContent.style.width = colW + 'px';
    columnContent.style.paddingLeft = '0';
    columnContent.style.paddingRight = '0';
    columnContent.style.columnWidth = colW + 'px';
    columnContent.style.columnGap = (2 * marginH) + 'px';
    columnContent.style.columnCount = 'auto';
    const scrollW = columnContent.scrollWidth;
    columnState.pageWidth = pageW;
    columnState.viewShift = pageW;
    columnState.totalPages = Math.max(1, Math.ceil(scrollW / pageW));
  }}

  // Navigation is explicit in callers; this function only measures layout.
}}

function columnApplyTransform() {{
  if (isScrollMode()) {{
    columnView.style.transform = 'none';
  }} else if (isDoubleMode()) {{
    const offset = columnState.currentSpread * columnState.viewShift;
    columnView.style.transform = `translateX(-${{offset}}px)`;
  }} else {{
    const marginH = getMarginH();
    const offset = columnState.currentSpread * columnState.pageWidth;
    columnView.style.transform = `translateX(${{-offset + marginH}}px)`;
  }}
}}

function columnGoToSpread(n) {{
  columnState.currentSpread = Math.max(0, Math.min(n, columnGetPageCount() - 1));
  columnApplyTransform();
  columnReportPosition();
}}

function columnGoToPage(pageIndex) {{
  if (isScrollMode()) {{
    const total = columnGetPageCount();
    const clamped = Math.max(0, Math.min(pageIndex, total - 1));
    const ratio = total > 0 ? clamped / total : 0;
    columnView.scrollTop = Math.floor(ratio * columnMaxScroll());
    columnReportPosition();
    return;
  }}
  const spread = isDoubleMode() ? Math.floor(pageIndex / 2) : pageIndex;
  columnGoToSpread(spread);
}}

function columnNext() {{
  if (isScrollMode()) {{
    const maxScroll = columnMaxScroll();
    if (columnView.scrollTop >= maxScroll - 1) {{
      if (currentChapter + 1 < window.ebookChapterCount) {{
        loadChapter(currentChapter + 1, 0);
      }}
    }} else {{
      const target = Math.min(maxScroll, columnView.scrollTop + columnScrollStep());
      if (currentSettings.animate) {{
        columnView.scrollTo({{ top: target, behavior: 'smooth' }});
      }} else {{
        columnView.scrollTop = target;
      }}
      columnReportPosition();
    }}
    return;
  }}
  if (columnState.currentSpread + 1 < columnGetPageCount()) {{
    if (currentSettings.animate) columnEnableAnimation();
    columnGoToSpread(columnState.currentSpread + 1);
    if (currentSettings.animate) {{
      columnScheduleDisableAnimation();
    }}
  }} else if (currentChapter + 1 < window.ebookChapterCount) {{
    loadChapter(currentChapter + 1, 0);
  }}
}}

function columnPrev() {{
  if (isScrollMode()) {{
    if (columnView.scrollTop <= 0) {{
      if (currentChapter > 0) {{
        // Passing the maximum safe integer as charOffset makes loadChapter's ratio
        // clamp to 1, so we land directly at the bottom of the previous chapter
        // instead of flashing through the top.
        loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER);
      }}
    }} else {{
      const target = Math.max(0, columnView.scrollTop - columnScrollStep());
      if (currentSettings.animate) {{
        columnView.scrollTo({{ top: target, behavior: 'smooth' }});
      }} else {{
        columnView.scrollTop = target;
      }}
      columnReportPosition();
    }}
    return;
  }}
  if (columnState.currentSpread > 0) {{
    if (currentSettings.animate) columnEnableAnimation();
    columnGoToSpread(columnState.currentSpread - 1);
    if (currentSettings.animate) {{
      columnScheduleDisableAnimation();
    }}
  }} else if (currentChapter > 0) {{
    // Passing the maximum safe integer as charOffset makes loadChapter's ratio
    // clamp to 1, so we land directly on the last page of the previous chapter
    // instead of flashing through page 0.
    loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER);
  }}
}}

function columnComputeCharOffset() {{
  const total = columnContent.textContent.length;
  if (total === 0) return 0;
  if (isScrollMode()) {{
    const ratio = columnView.scrollTop / Math.max(1, columnView.scrollHeight - columnView.clientHeight);
    return Math.floor(total * Math.max(0, Math.min(1, ratio)));
  }}
  const ratio = columnState.currentSpread / Math.max(1, columnGetPageCount());
  return Math.floor(total * ratio);
}}

function columnReportPosition() {{
  let spread = columnState.currentSpread;
  let total = columnGetPageCount();
  if (isScrollMode()) {{
    const ch = Math.max(1, columnView.clientHeight);
    spread = Math.floor(columnView.scrollTop / ch);
    total = columnGetPageCount();
  }}
  sendIpc({{
    "type": "position",
    "chapter": currentChapter,
    "spread": spread,
    "char_offset": columnComputeCharOffset(),
    "total_spreads": total
  }});
}}

function pageHeight() {{
  // measure.clientHeight 包含 padding，实际排版内容区要去掉上下 margin-v。
  // 注意：document 还没完成 layout 时这个值可能是 0 或负数，需要调用方判断。
  return measure.clientHeight - 2 * getMarginV();
}}

function getMarginH() {{
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--margin-h')) || 0;
}}

function getMarginV() {{
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--margin-v')) || 0;
}}

function ancestorLi(node) {{
  let el = node.parentElement;
  while (el && el !== measure) {{
    if (el.tagName === 'LI') return el;
    el = el.parentElement;
  }}
  return null;
}}

function blockAncestor(el) {{
  while (el && el !== measure) {{
    const display = window.getComputedStyle(el).display;
    if (display.startsWith('block') || display === 'list-item' || display === 'table-cell' || display === 'flex' || display === 'grid') return el;
    el = el.parentElement;
  }}
  return measure;
}}

function lineHeightForElement(el) {{
  const style = window.getComputedStyle(el);
  const lh = parseFloat(style.lineHeight);
  if (lh > 0) return lh;
  const size = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--size')) || 16;
  const line = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--line')) || 1.5;
  return line * size;
}}

function collectLineBoxes(root) {{
  const boxes = [];
  const rootRect = root.getBoundingClientRect();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
  while (walker.nextNode()) {{
    const node = walker.currentNode;
    const parent = node.parentElement;
    if (!parent || parent.tagName === 'SCRIPT' || parent.tagName === 'STYLE') continue;
    if ((node.textContent || '').trim().length === 0) continue;
    const li = ancestorLi(node);
    const liTop = li ? li.getBoundingClientRect().top - rootRect.top : null;
    const block = blockAncestor(parent);
    const lineHeight = lineHeightForElement(block);
    const range = document.createRange();
    range.selectNode(node);
    for (const r of range.getClientRects()) {{
      if (r.width <= 0 || r.height <= 0) continue;
      const leading = Math.max(0, (lineHeight - r.height) * 0.5);
      boxes.push({{
        top: r.top - rootRect.top,
        bottom: r.bottom - rootRect.top,
        lineTop: r.top - rootRect.top - leading,
        lineBottom: r.bottom - rootRect.top + leading,
        left: r.left - rootRect.left,
        right: r.right - rootRect.left,
        liTop: liTop
      }});
    }}
  }}
  for (const el of root.querySelectorAll('img, hr, svg, canvas')) {{
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;
    const li = ancestorLi(el);
    const liTop = li ? li.getBoundingClientRect().top - rootRect.top : null;
    boxes.push({{
      top: r.top - rootRect.top,
      bottom: r.bottom - rootRect.top,
      lineTop: r.top - rootRect.top,
      lineBottom: r.bottom - rootRect.top,
      left: r.left - rootRect.left,
      right: r.right - rootRect.left,
      atomic: true,
      liTop: liTop
    }});
  }}
  boxes.sort((a, b) => a.lineTop - b.lineTop || a.left - b.left);
  return boxes;
}}

function findSafeEnd(boxes, start, target) {{
  let safeEnd = target;
  const n = boxes.length;
  let i = 0;
  while (i < n && boxes[i].lineTop <= start) i++;
  let j = i;
  while (j < n && boxes[j].lineTop <= target) {{
    if (boxes[j].lineBottom > target) {{
      const lineTop = boxes[j].lineTop;
      const liTop = boxes[j].liTop;
      // 激进策略：如果最后一行有可能被截断，就连上一行一起放到下一页。
      // 这里把切分点回退到跨越目标行的上一行的 lineTop。
      let candidate = lineTop;
      let k = j - 1;
      while (k >= i && boxes[k].lineTop === lineTop) k--;
      if (k >= i) {{
        candidate = boxes[k].lineTop;
      }}
      if (liTop !== null && liTop > start && liTop <= target) {{
        candidate = liTop;
      }}
      if (candidate > start) {{
        safeEnd = Math.min(safeEnd, candidate);
      }}
    }}
    j++;
  }}
  // 切分点必须落在整数像素上，否则 CSS 对 cell 高度的取整会让下一页的第一行
  // 在上一页底部露出一个像素条，造成同一行在相邻两页重复出现。
  return Math.floor(safeEnd);
}}

function buildClonedSpread(start, end) {{
  const safety = spreadSafety();
  const ph = end - start;
  const cell = document.createElement('div');
  cell.style.position = 'relative';
  cell.style.overflow = 'hidden';
  cell.style.height = ph + 'px';
  const clone = measure.cloneNode(true);
  clone.removeAttribute('id');
  clone.style.position = 'absolute';
  // measure 有顶部 padding，克隆节点没有；需要按 start 在 measure 坐标系中的位置
  // 进行偏移，使当前页第一行的 line box 顶部对齐到 cell 顶部。
  const marginV = getMarginV();
  const offset = start - marginV;
  clone.style.top = -offset + 'px';
  clone.style.width = '100%';
  cell.appendChild(clone);
  // 在页面四周留出一小条安全区，让轻微超出 line box 的字形也能显示出来。
  const wrapper = document.createElement('div');
  wrapper.style.height = (ph + 2 * safety) + 'px';
  wrapper.style.paddingTop = safety + 'px';
  wrapper.style.paddingBottom = safety + 'px';
  wrapper.style.boxSizing = 'border-box';
  wrapper.appendChild(cell);
  return wrapper.outerHTML;
}}

function buildDoubleSpread(leftStart, leftEnd, rightEnd, ph) {{
  const safety = spreadSafety();
  const wrapper = document.createElement('div');
  wrapper.style.display = 'flex';
  wrapper.style.width = '100%';
  wrapper.style.height = (ph + 2 * safety) + 'px';
  wrapper.style.paddingTop = safety + 'px';
  wrapper.style.paddingBottom = safety + 'px';
  wrapper.style.boxSizing = 'border-box';
  function makeCell(start, end) {{
    const cell = document.createElement('div');
    cell.style.flex = '1';
    cell.style.height = ph + 'px';
    cell.style.overflow = 'hidden';
    cell.style.position = 'relative';
    const clone = measure.cloneNode(true);
    clone.removeAttribute('id');
    clone.style.position = 'absolute';
    // 见 buildClonedSpread 中的说明：按 measure 坐标系偏移。
    const marginV = getMarginV();
    const offset = start - marginV;
    clone.style.top = -offset + 'px';
    clone.style.width = '100%';
    cell.appendChild(clone);
    return cell;
  }}
  wrapper.appendChild(makeCell(leftStart, leftEnd));
  wrapper.appendChild(makeCell(leftEnd, rightEnd));
  return wrapper.outerHTML;
}}

function splitSinglePage(html) {{
  measure.innerHTML = html;
  const fullPh = pageHeight();
  if (!fullPh || fullPh <= 0) {{
    measure.innerHTML = '';
    return [html];
  }}
  const ph = Math.max(1, Math.floor(fullPh - 2 * spreadSafety()));
  const boxes = collectLineBoxes(measure);
  if (boxes.length === 0) {{
    measure.innerHTML = '';
    return [html];
  }}
  const maxBottom = boxes.reduce((m, b) => Math.max(m, b.lineBottom), 0);
  const marginV = getMarginV();
  const spreads = [];
  let start = Math.floor(marginV);
  while (start < maxBottom) {{
    const target = start + ph;
    let end = findSafeEnd(boxes, start, target);
    if (end <= start) end = target;
    if (end > maxBottom) end = Math.floor(maxBottom);
    if (end <= start) break;
    spreads.push(buildClonedSpread(start, end));
    if (spreads.length > MAX_SPREADS_PER_CHAPTER) {{
      measure.innerHTML = '';
      throw new Error('分页数量异常：' + spreads.length + '，可能进入了死循环');
    }}
    start = end;
  }}
  measure.innerHTML = '';
  return spreads.length > 0 ? spreads : [html];
}}

function splitDoublePage(html) {{
  const originalWidth = measure.style.width;
  const marginH = getMarginH();
  measure.style.width = (document.body.clientWidth / 2 + marginH) + 'px';
  measure.innerHTML = html;
  const fullPh = pageHeight();
  if (!fullPh || fullPh <= 0) {{
    measure.innerHTML = '';
    measure.style.width = originalWidth;
    return [html];
  }}
  const ph = Math.max(1, Math.floor(fullPh - 2 * spreadSafety()));
  const boxes = collectLineBoxes(measure);
  if (boxes.length === 0) {{
    measure.innerHTML = '';
    measure.style.width = originalWidth;
    return [html];
  }}
  const maxBottom = boxes.reduce((m, b) => Math.max(m, b.lineBottom), 0);
  const marginV = getMarginV();
  const spreads = [];
  let start = Math.floor(marginV);
  while (start < maxBottom) {{
    let leftEnd = findSafeEnd(boxes, start, start + ph);
    // 激进策略可能把左页内容全部挤到右页，导致左页空白；至少要放满一页目标高度。
    if (leftEnd <= start) leftEnd = Math.min(start + ph, Math.floor(maxBottom));
    let rightEnd = findSafeEnd(boxes, leftEnd, leftEnd + ph);
    if (rightEnd <= start) rightEnd = Math.min(start + ph * 2, Math.floor(maxBottom));
    if (rightEnd > maxBottom) rightEnd = Math.floor(maxBottom);
    if (rightEnd <= start) break;
    spreads.push(buildDoubleSpread(start, leftEnd, rightEnd, ph));
    if (spreads.length > MAX_SPREADS_PER_CHAPTER) {{
      measure.innerHTML = '';
      measure.style.width = originalWidth;
      throw new Error('分页数量异常：' + spreads.length + '，可能进入了死循环');
    }}
    start = rightEnd;
  }}
  measure.innerHTML = '';
  measure.style.width = originalWidth;
  return spreads.length > 0 ? spreads : [html];
}}

function splitIntoSpreads(html) {{
  try {{
    if (isScrollMode()) return [html];
    const spreads = isDoubleMode() ? splitDoublePage(html) : splitSinglePage(html);
    if (!Array.isArray(spreads) || spreads.length === 0) {{
      throw new Error('分页结果为空');
    }}
    return spreads;
  }} catch (err) {{
    showError('章节分页失败', err.message);
    return [html];
  }}
}}

function goToSpread(index, animate) {{
  if (isColumnMode()) {{
    // columnGoToSpread 已经按 spread 索引工作，无需 spread -> page -> spread 转换。
    return columnGoToSpread(index);
  }}
  if (spreads.length === 0) return;
  const target = Math.max(0, Math.min(spreads.length - 1, index));
  if (target === currentSpread) {{
    renderSpread(target);
    reportPosition();
    return;
  }}
  preloadAdjacent();
  if (animate && currentSettings.animate) {{
    flipToSpread(target);
  }} else {{
    currentSpread = target;
    renderSpread(currentSpread);
    reportPosition();
  }}
}}

let spreadElementCache = {{}};

function createSpreadElement(html) {{
  const el = document.createElement('div');
  el.innerHTML = html;
  return el.firstElementChild || el;
}}

function preloadAdjacent() {{
  const indices = [currentSpread - 1, currentSpread + 1];
  for (const idx of indices) {{
    if (idx >= 0 && idx < spreads.length) {{
      if (!spreadElementCache[idx]) {{
        spreadElementCache[idx] = createSpreadElement(spreads[idx]);
      }}
    }}
  }}
  // Prune cached spreads that are no longer adjacent
  for (const key of Object.keys(spreadElementCache)) {{
    const k = parseInt(key, 10);
    if (Math.abs(k - currentSpread) > 1) {{
      delete spreadElementCache[k];
    }}
  }}
}}

function getSpreadElement(index) {{
  if (index < 0 || index >= spreads.length) {{
    return document.createElement('div');
  }}
  if (spreadElementCache[index]) return spreadElementCache[index];
  return createSpreadElement(spreads[index]);
}}

function renderSpread(index) {{
  spread.innerHTML = '';
  spread.appendChild(getSpreadElement(index));
  spread.style.display = 'block';
  columnHide();
}}

function currentSpreadCharOffset() {{
  let offset = 0;
  for (let i = 0; i < currentSpread && i < spreads.length; i++) {{
    offset += textLength(spreads[i]);
  }}
  return offset;
}}

// Approximate: find the spread whose cumulative text length contains the offset.
function findSpreadForOffset(offset) {{
  if (spreads.length === 0) return 0;
  let count = 0;
  for (let i = 0; i < spreads.length; i++) {{
    count += textLength(spreads[i]);
    if (count >= offset) return i;
  }}
  return spreads.length - 1;
}}

function scrollToOffset(offset) {{
  const textNodes = [];
  const walk = document.createTreeWalker(spread, NodeFilter.SHOW_TEXT, null);
  while (walk.nextNode()) textNodes.push(walk.currentNode);
  let count = 0;
  for (const node of textNodes) {{
    if (count + node.length >= offset) {{
      const range = document.createRange();
      range.setStart(node, offset - count);
      const rect = range.getBoundingClientRect();
      spread.scrollTop = rect.top + spread.scrollTop - spread.getBoundingClientRect().top;
      break;
    }}
    count += node.length;
  }}
}}

function captureSpreadElement(index) {{
  const el = getSpreadElement(index).cloneNode(true);
  const container = document.createElement('div');
  container.style.width = '100%';
  container.style.height = '100%';
  container.style.overflow = 'hidden';
  container.appendChild(el);
  return container;
}}

function flipToSpread(targetIndex) {{
  if (isFlipping) {{
    pendingFlipTarget = targetIndex;
    return;
  }}
  isFlipping = true;
  const chapterAtStart = currentChapter;
  const direction = targetIndex > currentSpread ? 1 : -1;

  const sheet = document.createElement('div');
  sheet.className = 'sheet';
  sheet.style.left = '0';
  sheet.style.width = '100%';

  const front = document.createElement('div');
  front.className = 'front';
  front.appendChild(captureSpreadElement(currentSpread));

  const back = document.createElement('div');
  back.className = 'back';
  back.appendChild(captureSpreadElement(targetIndex));

  sheet.appendChild(front);
  sheet.appendChild(back);
  flipper.innerHTML = '';
  flipper.appendChild(sheet);
  flipper.style.display = 'block';

  renderSpread(targetIndex);

  requestAnimationFrame(() => {{
    sheet.style.transform = direction > 0 ? 'rotateY(-180deg)' : 'rotateY(180deg)';
  }});

  setTimeout(() => {{
    if (currentChapter === chapterAtStart) {{
      currentSpread = targetIndex;
      reportPosition();
    }}
    flipper.style.display = 'none';
    flipper.innerHTML = '';
    isFlipping = false;
    if (pendingFlipTarget !== null && currentChapter === chapterAtStart) {{
      const t = pendingFlipTarget;
      pendingFlipTarget = null;
      goToSpread(t, true);
    }}
  }}, 450);
}}

function cancelFlip() {{
  flipper.style.display = 'none';
  flipper.innerHTML = '';
  isFlipping = false;
  pendingFlipTarget = null;
}}

function applySettings(json) {{
  const s = typeof json === 'string' ? JSON.parse(json) : json;
  currentSettings = s;
  // Capture the approximate character position before the paginator or
  // layout changes, so we can restore the closest page afterwards.
  const wasColumn = isColumnMode();
  const wasScroll = isScrollMode();
  let savedCharOffset = 0;
  // When leaving scroll mode, approximate the character offset from the
  // scroll ratio so the paginated layout can land on the closest page.
  if (wasScroll) {{
    const scrollEl = wasColumn ? columnView : spread;
    const textEl = wasColumn ? columnContent : spread;
    const totalChars = textEl.textContent.length;
    if (totalChars > 0 && scrollEl.scrollHeight > 0) {{
      const ratio = scrollEl.scrollTop / scrollEl.scrollHeight;
      savedCharOffset = Math.floor(totalChars * Math.max(0, Math.min(1, ratio)));
    }}
  }} else if (wasColumn) {{
    savedCharOffset = columnComputeCharOffset();
  }} else {{
    savedCharOffset = currentSpreadCharOffset();
  }}
  window.ebookUseColumns = !!s.use_columns;
  const root = document.documentElement;
  root.style.setProperty('--bg', s.bg);
  root.style.setProperty('--fg', s.fg);
  root.style.setProperty('--font', s.font);
  root.style.setProperty('--size', s.size + 'px');
  root.style.setProperty('--line', s.line);
  root.style.setProperty('--margin-h', s.margin_h + 'px');
  root.style.setProperty('--margin-v', s.margin_v + 'px');
  document.body.className = s.mode;
  // 设置变化可能导致分页改变，重新切分
  if (currentChapterHtml) {{
    if (isColumnMode()) {{
      cancelFlip();
      // Make sure the column paginator has content when switching to it.
      columnContent.innerHTML = currentChapterHtml;
      // Measure first so columnGetPageCount() is valid before restoring progress.
      columnLayout();
      const totalChars = columnContent.textContent.length;
      if (isScrollMode()) {{
        if (totalChars > 0 && savedCharOffset > 0) {{
          const ratio = savedCharOffset / totalChars;
          columnView.scrollTop = Math.floor(ratio * columnMaxScroll());
        }}
        columnReportPosition();
      }} else {{
        let targetSpread = 0;
        if (totalChars > 0 && savedCharOffset > 0) {{
          const ratio = savedCharOffset / totalChars;
          targetSpread = Math.floor(ratio * (columnGetPageCount() - 1));
        }}
        columnGoToSpread(targetSpread);
      }}
      return;
    }}
    if (isScrollMode()) {{
      cancelFlip();
      columnHide();
      const offset = savedCharOffset;
      spread.innerHTML = currentChapterHtml;
      spread.style.display = 'block';
      spreads = [];
      currentSpread = 0;
      if (offset > 0) {{
        scrollToOffset(offset);
      }}
      reportPosition();
    }} else {{
      cancelFlip();
      const offset = savedCharOffset;
      spreads = splitIntoSpreads(currentChapterHtml);
      debugSplit('applySettings', pageHeight(), measure.getBoundingClientRect().height, spreads.length);
      currentSpread = findSpreadForOffset(offset);
      goToSpread(currentSpread, false);
    }}
  }}
}}

async function loadChapter(index, charOffset) {{
  index = index ?? 0;
  currentChapter = index;
  pendingFlipTarget = null;
  spreadElementCache = {{}}; // clear stale cache
  try {{
    const res = await fetch('ebook://reader?chapter=' + currentChapter);
    currentChapterHtml = await res.text();
    if (isColumnMode()) {{
      columnContent.innerHTML = currentChapterHtml;
      // Measure first so columnGetPageCount() is valid before restoring progress.
      columnLayout();
      const totalChars = columnContent.textContent.length;
      if (isScrollMode()) {{
        if (typeof charOffset === 'number' && charOffset >= 0 && totalChars > 0) {{
          const ratio = Math.min(1, charOffset / totalChars);
          columnView.scrollTop = Math.floor(ratio * columnMaxScroll());
        }}
        columnReportPosition();
      }} else {{
        let targetSpread = 0;
        if (typeof charOffset === 'number' && charOffset >= 0 && totalChars > 0) {{
          const ratio = Math.min(1, charOffset / totalChars);
          targetSpread = Math.floor(ratio * (columnGetPageCount() - 1));
        }}
        columnGoToSpread(targetSpread);
      }}
      return;
    }}
    if (isScrollMode()) {{
      columnHide();
      spread.innerHTML = currentChapterHtml;
      spread.style.display = 'block';
      if (charOffset) {{
        scrollToOffset(charOffset);
      }}
      spreads = [];
      currentSpread = 0;
      reportPosition();
    }} else {{
      spreads = splitIntoSpreads(currentChapterHtml);
      debugSplit('loadChapter', pageHeight(), measure.getBoundingClientRect().height, spreads.length);
      if (typeof charOffset === 'number' && charOffset >= 0) {{
        currentSpread = findSpreadForOffset(charOffset);
      }} else {{
        currentSpread = 0;
      }}
      goToSpread(currentSpread, false);
    }}
  }} catch (e) {{
    spread.innerHTML = '<p>章节加载失败: ' + e + '</p>';
    spread.style.display = 'block';
  }}
}}

function reportPosition() {{
  if (isColumnMode()) return columnReportPosition();
  let offset = 0;
  if (!isScrollMode() && spreads.length > 0 && currentSpread < spreads.length) {{
    // Approximate character offset by summing text lengths of preceding spreads.
    for (let i = 0; i < currentSpread; i++) {{
      offset += textLength(spreads[i]);
    }}
    sendIpc({{
      "type": "position",
      "chapter": currentChapter,
      "spread": currentSpread,
      "char_offset": offset,
      "total_spreads": spreads.length
    }});
  }} else {{
    // Scroll mode fallback: use #spread's visible text start.
    const rect = spread.getBoundingClientRect();
    let offset = 0;
    const textNodes = [];
    const walk = document.createTreeWalker(spread, NodeFilter.SHOW_TEXT, null);
    while (walk.nextNode()) textNodes.push(walk.currentNode);
    for (const node of textNodes) {{
      const r = document.createRange();
      r.selectNode(node);
      const br = r.getBoundingClientRect();
      if (br.left >= rect.left && br.top >= rect.top) {{
        break;
      }}
      offset += node.length;
    }}
    sendIpc({{
      "type": "position",
      "chapter": currentChapter,
      "spread": 0,
      "char_offset": offset,
      "total_spreads": 1
    }});
  }}
}}

function textLength(html) {{
  const div = document.createElement('div');
  div.innerHTML = html;
  return div.textContent.length;
}}

function nextPage() {{
  if (isColumnMode()) return columnNext();
  if (isScrollMode()) {{
    spread.scrollTop += spread.clientHeight * 0.9;
    return;
  }}
  if (currentSpread + 1 < spreads.length) {{
    goToSpread(currentSpread + 1, true);
  }} else if (currentChapter + 1 < window.ebookChapterCount) {{
    loadChapter(currentChapter + 1, 0);
  }}
}}

function prevPage() {{
  if (isColumnMode()) return columnPrev();
  if (isScrollMode()) {{
    spread.scrollTop -= spread.clientHeight * 0.9;
    return;
  }}
  if (currentSpread > 0) {{
    goToSpread(currentSpread - 1, true);
  }} else if (currentChapter > 0) {{
    loadChapter(currentChapter - 1, 0).then(() => {{
      goToSpread(spreads.length - 1, true);
    }});
  }}
}}

function onWheel(e) {{
  if (isScrollMode()) return;
  e.preventDefault();
  const deltaY = currentSettings.invert_scroll ? -e.deltaY : e.deltaY;
  const deltaX = currentSettings.invert_scroll ? -e.deltaX : e.deltaX;
  if (deltaY > 0 || deltaX > 0) {{
    nextPage();
  }} else if (deltaY < 0 || deltaX < 0) {{
    prevPage();
  }}
}}

function onClick(e) {{
  if (isScrollMode()) return;
  const sel = window.getSelection();
  if (sel && sel.toString().length > 0) return;
  const container = isColumnMode() ? columnView : spread;
  const rect = container.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width / 2) {{
    prevPage();
  }} else {{
    nextPage();
  }}
}}

spread.addEventListener('wheel', onWheel, {{ passive: false }});
spread.addEventListener('click', onClick);
if (columnView) {{
  columnView.addEventListener('wheel', onWheel, {{ passive: false }});
  columnView.addEventListener('click', onClick);
  columnView.addEventListener('scroll', columnReportPosition, {{ passive: true }});
}}
window.addEventListener('scroll', (e) => {{
  // The column-view already has its own scroll listener; ignore its events
  // at the window level to avoid duplicate position reports.
  if (columnView && e.target === columnView) return;
  reportPosition();
}}, true);

let resizeTimeout = null;
window.addEventListener('resize', () => {{
  clearTimeout(resizeTimeout);
  resizeTimeout = setTimeout(() => {{
    if (!currentChapterHtml) return;
    if (isColumnMode()) {{
      let savedScrollRatio = 0;
      if (isScrollMode()) {{
        const maxScroll = columnMaxScroll();
        savedScrollRatio = maxScroll > 0 ? columnView.scrollTop / maxScroll : 0;
      }}
      columnLayout();
      if (isScrollMode()) {{
        columnView.scrollTop = Math.floor(savedScrollRatio * columnMaxScroll());
      }} else {{
        columnGoToSpread(columnState.currentSpread);
      }}
      return;
    }}
    if (isScrollMode()) return;
    cancelFlip();
    const offset = currentSpreadCharOffset();
    spreads = splitIntoSpreads(currentChapterHtml);
    currentSpread = findSpreadForOffset(offset);
    goToSpread(currentSpread, false);
  }}, RESIZE_DEBOUNCE_MS);
}});

applySettings({settings_json});
loadChapter(0, 0);
sendIpc({{ "type": "ready" }});
</script>
</body>
</html>"#,
        bg = js.bg,
        fg = js.fg,
        font = js.font,
        size = js.size,
        line = js.line,
        margin_h = js.margin_h,
        margin_v = js.margin_v,
        mode = js.mode,
        animate = js.animate,
        invert_scroll = js.invert_scroll,
        chapter_count = chapter_count,
        settings_json = serde_json::to_string(&js).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_storage::models::EbookSettings;

    #[test]
    fn test_reader_html_contains_spread_containers() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("id=\"measure\""));
        assert!(html.contains("id=\"spread\""));
        assert!(html.contains("function splitIntoSpreads"));
        assert!(html.contains("function goToSpread"));
    }

    #[test]
    fn test_reader_html_contains_single_page_split_logic() {
        use rust_reader_storage::models::EbookSettings;
        let settings = EbookSettings {
            reading_mode: rust_reader_core::ebook::EbookReadingMode::SinglePage,
            ..Default::default()
        };
        let html = reader_html(&settings, 1);
        assert!(html.contains("pageHeight"));
        assert!(html.contains("splitSinglePage"));
    }

    #[test]
    fn test_reader_html_contains_double_page_split_logic() {
        use rust_reader_core::ebook::EbookReadingMode;
        use rust_reader_storage::models::EbookSettings;
        let settings = EbookSettings {
            reading_mode: EbookReadingMode::DoublePage,
            ..Default::default()
        };
        let html = reader_html(&settings, 1);
        assert!(html.contains("splitDoublePage"));
    }

    #[test]
    fn test_reader_html_uses_line_box_pagination() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function collectLineBoxes"));
        assert!(html.contains("function findSafeEnd"));
        assert!(html.contains("function buildClonedSpread"));
        assert!(html.contains("function buildDoubleSpread"));
        assert!(html.contains("getClientRects"));
        assert!(html.contains("function ancestorLi"));
        assert!(html.contains("function blockAncestor"));
        assert!(html.contains("liTop"));
        assert!(html.contains("lineTop"));
        assert!(html.contains("lineBottom"));
        assert!(html.contains("function findSafeEnd(boxes, start, target)"));
        assert!(html.contains("function showError"));
        assert!(html.contains("function splitIntoSpreads"));
        assert!(html.contains("SPREAD_SAFETY_PX"));
        assert!(html.contains("paddingTop = safety"));
        assert!(html.contains("paddingBottom = safety"));
        assert!(
            !html.contains("candidate = candidate - buffer"),
            "line-box pagination should not subtract an extra buffer from the break point"
        );
    }

    #[test]
    fn test_reader_html_contains_chapter_navigation_functions() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function loadChapter"));
        assert!(html.contains("function goToSpread"));
        assert!(html.contains("window.ebookChapterCount"));
    }

    #[test]
    fn test_reader_html_contains_render_spread_helpers() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function renderSpread"));
        assert!(html.contains("function findSpreadForOffset"));
    }

    #[test]
    fn test_reader_html_uses_find_spread_for_offset() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function findSpreadForOffset"));
        assert!(html.contains("findSpreadForOffset(charOffset)"));
    }

    #[test]
    fn test_reader_html_contains_required_functions() {
        let settings = EbookSettings::default();
        let html = reader_html(&settings, 1);
        assert!(!html.is_empty());
        assert!(html.contains("function loadChapter"));
        assert!(html.contains("function applySettings"));
        assert!(html.contains("function nextPage"));
        assert!(html.contains("function prevPage"));
        assert!(html.contains("function reportPosition"));
        assert!(html.contains("function sendIpc"));
        assert!(html.contains("function onWheel"));
        assert!(html.contains("function onClick"));
        assert!(html.contains("addEventListener('wheel', onWheel, { passive: false })"));
        assert!(html.contains("addEventListener('click', onClick)"));
        assert!(html.contains("window.ipc.postMessage"));
    }

    #[test]
    fn test_measure_and_spread_share_box_model() {
        let settings = EbookSettings::default();
        let html = reader_html(&settings, 1);
        let measure_rule = html
            .split("#measure {")
            .nth(1)
            .unwrap()
            .split('}')
            .next()
            .unwrap();
        let spread_rule = html
            .split("#spread {")
            .nth(1)
            .unwrap()
            .split('}')
            .next()
            .unwrap();
        for rule in &[measure_rule, spread_rule] {
            assert!(
                rule.contains("padding: var(--margin-v) var(--margin-h)"),
                "missing padding: {}",
                rule
            );
            assert!(
                rule.contains("box-sizing: border-box"),
                "missing box-sizing: {}",
                rule
            );
            assert!(
                rule.contains("overflow: hidden"),
                "missing overflow: {}",
                rule
            );
        }
    }

    #[test]
    fn test_reader_html_includes_spread_styles() {
        let settings = EbookSettings {
            font_size: 20,
            line_height: 1.8,
            margin_horizontal: 32,
            margin_vertical: 40,
            ..Default::default()
        };
        let html = reader_html(&settings, 1);
        assert!(html.contains("--size: 20px"));
        assert!(html.contains("--line: 1.8"));
        assert!(html.contains("--margin-h: 32px"));
        assert!(html.contains("--margin-v: 40px"));
        assert!(html.contains("#measure"));
        assert!(html.contains("#spread"));
        assert!(html.contains("function splitSinglePage"));
        assert!(html.contains("function splitDoublePage"));
        assert!(html.contains("object-fit: contain"));
        assert!(html.contains("max-height: calc(100vh - var(--margin-v) * 2)"));
    }

    #[test]
    fn test_reader_html_contains_preload_logic() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function preloadAdjacent"));
        assert!(html.contains("spreadElementCache"));
    }

    #[test]
    fn test_reader_html_contains_flipper_and_flip_function() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("id=\"flipper\""));
        assert!(html.contains("function flipToSpread"));
    }

    #[test]
    fn test_reader_html_contains_resize_handler() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function applySettings"));
        assert!(html.contains("clearTimeout(resizeTimeout)"));
        assert!(html.contains("setTimeout"));
        assert!(html.contains("splitIntoSpreads(currentChapterHtml)"));
        assert!(html.contains("goToSpread(currentSpread, false)"));

        let handler_body = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("\n  }});")
            .next()
            .expect("resize handler end not found");
        // Must not short-circuit scroll mode at the very top; otherwise the
        // CSS-columns resize path can never preserve the scroll ratio.
        assert!(
            !handler_body.contains("if (!currentChapterHtml || isScrollMode()) return;"),
            "resize handler should not return early for scroll mode at the top"
        );
        assert!(
            handler_body.contains("if (!currentChapterHtml) return;"),
            "resize handler should only guard on missing chapter content"
        );
        // Column paginator branch must preserve and restore scroll ratio in scroll mode.
        let column_branch = handler_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("resize handler column branch not found")
            .split("\n      return;")
            .next()
            .expect("resize handler column branch end not found");
        assert!(
            column_branch.contains(
                "savedScrollRatio = maxScroll > 0 ? columnView.scrollTop / maxScroll : 0;"
            ),
            "column resize should capture the scroll ratio in scroll mode"
        );
        assert!(
            column_branch.contains(
                "columnView.scrollTop = Math.floor(savedScrollRatio * columnMaxScroll());"
            ),
            "column resize should restore the scroll ratio in scroll mode"
        );
        assert!(
            column_branch.contains("columnGoToSpread(columnState.currentSpread);"),
            "column resize should re-navigate to the current spread in paginated mode"
        );
        // Old-paginator scroll mode reflows naturally, so it returns after the column branch.
        assert!(
            handler_body.contains("if (isScrollMode()) return;"),
            "resize handler should skip old-paginator split/re-layout in scroll mode"
        );
    }

    #[test]
    fn test_reader_html_scroll_mode_shows_vertical_scrollbar() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("body.scroll #spread"));
        assert!(html.contains("overflow-y: scroll"));
        assert!(html.contains("spread.innerHTML = currentChapterHtml"));
        assert!(html.contains("spread.scrollTop +="));
    }

    #[test]
    fn test_reader_html_scroll_mode_hides_column_view_in_apply_settings() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(2)
            .expect("applySettings old-paginator scroll branch not found")
            .split("\n    } else {{")
            .next()
            .expect("applySettings scroll branch end not found");
        assert!(
            scroll_branch.contains("columnHide();"),
            "applySettings old-paginator scroll branch must hide the column view so #spread is visible"
        );
    }

    #[test]
    fn test_reader_html_scroll_mode_hides_column_view_in_load_chapter() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(2)
            .expect("loadChapter old-paginator scroll branch not found")
            .split("\n    } else {{")
            .next()
            .expect("loadChapter scroll branch end not found");
        assert!(
            scroll_branch.contains("columnHide();"),
            "loadChapter old-paginator scroll branch must hide the column view so #spread is visible"
        );
    }

    #[test]
    fn test_reader_html_reports_spread_position() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("\"type\": \"position\""));
        assert!(html.contains("\"chapter\":"));
        assert!(html.contains("\"spread\":"));
        assert!(html.contains("\"char_offset\":"));
        assert!(html.contains("\"total_spreads\":"));
        assert!(html.contains("\"type\": \"ready\""));
    }

    #[test]
    fn test_reader_html_single_page_breaks_when_end_does_not_advance() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        // splitSinglePage must stop if clamping end to maxBottom yields no progress,
        // otherwise chapters whose tail is smaller than one page can loop forever.
        let fn_body = html
            .split("function splitSinglePage(html)")
            .nth(1)
            .expect("splitSinglePage not found")
            .split("function splitDoublePage")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("if (end > maxBottom) end = Math.floor(maxBottom);"),
            "end should be clamped to maxBottom"
        );
        assert!(
            fn_body.contains("if (end <= start) break;"),
            "splitSinglePage must break when end cannot advance"
        );
    }

    #[test]
    fn test_reader_html_double_page_breaks_when_right_end_does_not_advance() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function splitDoublePage(html)")
            .nth(1)
            .expect("splitDoublePage not found")
            .split("function splitIntoSpreads")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("if (rightEnd > maxBottom) rightEnd = Math.floor(maxBottom);"),
            "rightEnd should be clamped to maxBottom"
        );
        assert!(
            fn_body.contains("if (rightEnd <= start) break;"),
            "splitDoublePage must break when rightEnd cannot advance"
        );
        assert!(
            fn_body.contains("if (rightEnd <= start) rightEnd = Math.min(start + ph * 2, Math.floor(maxBottom));"),
            "rightEnd fallback should also be clamped to maxBottom"
        );
    }

    #[test]
    fn test_reader_html_contains_column_paginator_skeleton() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        // Feature flag and container
        assert!(html.contains("window.ebookUseColumns"));
        assert!(html.contains("id=\"column-view\""));
        assert!(html.contains("id=\"column-content\""));
        // Core paginator functions
        assert!(html.contains("function isColumnMode()"));
        assert!(html.contains("function columnLayout()"));
        assert!(html.contains("function columnGoToPage("));
        assert!(html.contains("function columnGoToSpread("));
        assert!(html.contains("function columnNext()"));
        assert!(html.contains("function columnPrev()"));
        assert!(html.contains("function columnReportPosition()"));
        assert!(html.contains("function columnComputeCharOffset()"));
        assert!(html.contains("function columnGetPageCount()"));
        // Dispatch hooks in existing functions
        assert!(html.contains("if (isColumnMode()) return columnNext();"));
        assert!(html.contains("if (isColumnMode()) return columnPrev();"));
        assert!(html.contains("if (isColumnMode()) return columnReportPosition();"));
        // goToSpread delegates directly to columnGoToSpread without spread -> page -> spread conversion.
        assert!(html.contains("return columnGoToSpread(index);"));
        // Internal reads should go through the getter for a consistent external API
        assert!(html.contains("columnGetPageCount()"));
    }

    #[test]
    fn test_reader_html_column_layout_uses_column_count_longhand() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnGoToPage")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnContent.style.columnCount = 'auto';"),
            "columnLayout should set columnCount longhand to keep columnWidth/columnGap"
        );
        assert!(
            !fn_body.contains("columnContent.style.columns = 'auto';"),
            "columnLayout should not use the columns shorthand which resets longhands"
        );
    }

    #[test]
    fn test_reader_html_column_single_page_transform_matches_prototype() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnApplyTransform()")
            .nth(1)
            .expect("columnApplyTransform not found")
            .split("function columnGoToSpread")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnView.style.transform = `translateX(${-offset + marginH}px)`;"),
            "single-page transform should follow prototype translateX(-currentSpread * pageW + marginH)"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_mode_uses_column_view() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("function columnScrollStep()"),
            "column scroll mode should expose a scroll-step helper"
        );
        assert!(
            html.contains("columnView.clientHeight - 2 * getMarginV()"),
            "columnScrollStep should derive the step from the viewport height"
        );
        let next_body = html
            .split("function columnNext()")
            .nth(1)
            .expect("columnNext not found")
            .split("function columnPrev")
            .next()
            .unwrap();
        assert!(
            next_body.contains(
                "const target = Math.min(maxScroll, columnView.scrollTop + columnScrollStep());"
            ),
            "columnNext scroll mode should compute a clamped downward scroll target"
        );
        assert!(
            next_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "columnNext scroll mode should support smooth scrolling"
        );
        assert!(
            next_body.contains("columnView.scrollTop = target;"),
            "columnNext scroll mode should fall back to direct scrollTop assignment"
        );
        let prev_body = html
            .split("function columnPrev()")
            .nth(1)
            .expect("columnPrev not found")
            .split("function columnComputeCharOffset")
            .next()
            .unwrap();
        assert!(
            prev_body
                .contains("const target = Math.max(0, columnView.scrollTop - columnScrollStep());"),
            "columnPrev scroll mode should compute a clamped upward scroll target"
        );
        assert!(
            prev_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "columnPrev scroll mode should support smooth scrolling"
        );
        assert!(
            prev_body.contains("columnView.scrollTop = target;"),
            "columnPrev scroll mode should fall back to direct scrollTop assignment"
        );
        let compute_body = html
            .split("function columnComputeCharOffset()")
            .nth(1)
            .expect("columnComputeCharOffset not found")
            .split("function columnReportPosition")
            .next()
            .unwrap();
        assert!(
            compute_body.contains("columnView.scrollTop"),
            "columnComputeCharOffset should read #column-view scroll position"
        );
        assert!(
            compute_body.contains("columnView.scrollHeight"),
            "columnComputeCharOffset should read #column-view scroll height"
        );
        assert!(
            !compute_body.contains("columnContent.scrollTop"),
            "columnComputeCharOffset should not read #column-content scroll position"
        );
    }

    #[test]
    fn test_reader_html_column_goto_page_does_not_take_animate() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("function columnGoToPage(pageIndex) {"),
            "columnGoToPage should take only the page index"
        );
        assert!(
            !html.contains("function columnGoToPage(pageIndex, animate)"),
            "columnGoToPage should not have an unused animate parameter"
        );
        assert!(
            !html.contains("columnGoToPage(0, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentSpread, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentSpread + 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentSpread - 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnGetPageCount() - 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(targetSpread, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(index, animate || false)"),
            "goToSpread should not forward animate to columnGoToPage"
        );
        // Internal spread navigation should use the spread helper, not page helper.
        assert!(html.contains("function columnGoToSpread(n) {"));
        assert!(html.contains("columnGoToSpread(columnState.currentSpread + 1)"));
        assert!(html.contains("columnGoToSpread(columnState.currentSpread - 1)"));
        assert!(html.contains("columnGoToSpread(targetSpread)"));
    }

    #[test]
    fn test_reader_html_column_next_crosses_chapter_boundary() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function columnNext()")
            .nth(1)
            .expect("columnNext not found")
            .split("function columnPrev")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnGoToSpread(columnState.currentSpread + 1)"),
            "columnNext should advance to the next spread within a chapter"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter + 1, 0)"),
            "columnNext should load the next chapter when on the last spread"
        );
    }

    #[test]
    fn test_reader_html_column_prev_crosses_chapter_boundary() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function columnPrev()")
            .nth(1)
            .expect("columnPrev not found")
            .split("function columnComputeCharOffset")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnGoToSpread(columnState.currentSpread - 1)"),
            "columnPrev should go to the previous spread within a chapter"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER)"),
            "columnPrev should load the previous chapter and clamp to the last spread"
        );
        assert!(
            !fn_body.contains("loadChapter(currentChapter - 1, 0)"),
            "columnPrev should not load the previous chapter at offset 0"
        );
        assert!(
            !fn_body.contains(
                ".then(() => {\n      columnGoToSpread(columnGetPageCount() - 1);\n    });"
            ),
            "columnPrev should not flash through spread 0 before jumping to the last spread"
        );
    }

    #[test]
    fn test_reader_html_column_layout_guards_non_positive_widths() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnGoToPage")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("Math.max(1, Math.floor((viewportW - gutter) / 2))"),
            "double-page page width should be guarded against non-positive values"
        );
        assert!(
            fn_body.contains("Math.max(1, pageW - 2 * marginH)"),
            "column width should be guarded against non-positive values"
        );
        assert!(
            fn_body.contains("Math.max(1, viewportW)"),
            "single-page viewport width should be guarded against non-positive values"
        );
        assert!(
            fn_body.contains("columnState.totalPages = Math.max(1,"),
            "totalPages should always be at least 1"
        );
    }

    #[test]
    fn test_reader_html_column_double_branch_guards_narrow_viewport() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnApplyTransform")
            .next()
            .unwrap();
        let double_branch = layout_body
            .split("if (isDoubleMode()) {")
            .nth(1)
            .expect("double branch not found")
            .split("\n  } else {{")
            .next()
            .expect("double branch end not found");
        assert!(
            double_branch.contains("const pageW = Math.max(1, Math.floor((viewportW - gutter) / 2));"),
            "double-page pageW must be guarded so a narrow viewport cannot produce 0 or negative width"
        );
        assert!(
            double_branch.contains("const colW = Math.max(1, pageW - 2 * marginH);"),
            "double-page colW must be guarded so large margins cannot produce 0 or negative width"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_preserves_scroll_offset() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("scrollEl.scrollTop / scrollEl.scrollHeight"),
            "applySettings should compute scroll ratio from the active scroll container"
        );
        assert!(
            fn_body.contains("Math.floor(totalChars * Math.max(0, Math.min(1, ratio)))"),
            "applySettings should convert scroll ratio to a character offset"
        );
    }

    #[test]
    fn test_reader_html_column_helpers_reuse_existing_scroll_double_helpers() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            !html.contains("function columnIsScrollMode()"),
            "should reuse existing isScrollMode helper"
        );
        assert!(
            !html.contains("function columnIsDoubleMode()"),
            "should reuse existing isDoubleMode helper"
        );
        assert!(
            html.contains("if (isScrollMode()) {"),
            "column paginator should call isScrollMode"
        );
        assert!(
            html.contains("} else if (isDoubleMode()) {"),
            "column paginator should call isDoubleMode"
        );
    }

    #[test]
    fn test_reader_html_contains_column_css_rules() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("--column-gutter:"));
        assert!(html.contains("#column-view {"));
        assert!(html.contains("#column-content {"));
        assert!(html.contains("column-fill: auto"));
        assert!(html.contains("break-inside: avoid"));
        assert!(html.contains("body.scroll #column-view"));
        assert!(html.contains("body.scroll #column-content"));
    }

    #[test]
    fn test_reader_html_column_flag_defaults_to_false() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        // The flag is initialized false and is only flipped by applySettings when
        // the Rust side sends use_columns: true.
        assert!(html.contains("window.ebookUseColumns = false;"));
        assert!(html.contains("window.ebookUseColumns = !!s.use_columns;"));
    }

    #[test]
    fn test_reader_html_old_paginator_still_present_when_columns_disabled() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        // Default HTML must keep all existing line-box pagination code intact.
        assert!(html.contains("function collectLineBoxes"));
        assert!(html.contains("function findSafeEnd"));
        assert!(html.contains("function buildClonedSpread"));
        assert!(html.contains("function buildDoubleSpread"));
        assert!(html.contains("function splitSinglePage"));
        assert!(html.contains("function splitDoublePage"));
        assert!(html.contains("function splitIntoSpreads"));
        assert!(html.contains("function goToSpread"));
        assert!(html.contains("getClientRects"));
    }

    #[test]
    fn test_reader_html_column_layout_does_not_auto_navigate() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnGoToPage")
            .next()
            .unwrap();
        assert!(
            !fn_body.contains("columnGoToPage("),
            "columnLayout should only measure layout, not navigate"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_layout_before_target_page() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let apply_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        let column_branch = apply_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("applySettings column branch not found")
            .split("\n      return;")
            .next()
            .expect("applySettings column branch end not found");
        let layout_pos = column_branch
            .find("columnLayout(")
            .expect("columnLayout call not found in applySettings");
        let target_pos = column_branch
            .find("targetSpread = Math.floor(ratio * (columnGetPageCount() - 1));")
            .expect("targetSpread computation not found in applySettings");
        assert!(
            layout_pos < target_pos,
            "applySettings must call columnLayout before computing targetSpread from columnGetPageCount()"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_layout_before_target_page() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let load_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        let column_branch = load_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("loadChapter column branch not found")
            .split("\n      return;")
            .next()
            .expect("loadChapter column branch end not found");
        let layout_pos = column_branch
            .find("columnLayout(")
            .expect("columnLayout call not found in loadChapter");
        let target_pos = column_branch
            .find("targetSpread = Math.floor(ratio * (columnGetPageCount() - 1));")
            .expect("targetSpread computation not found in loadChapter");
        assert!(
            layout_pos < target_pos,
            "loadChapter must call columnLayout before computing targetSpread from columnGetPageCount()"
        );
    }

    #[test]
    fn test_reader_html_column_double_page_layout_matches_prototype() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnApplyTransform")
            .next()
            .unwrap();
        // Isolate the double-page branch.
        let double_branch = layout_body
            .split("if (isDoubleMode()) {")
            .nth(1)
            .expect("double branch not found")
            .split("\n  } else {{")
            .next()
            .expect("double branch end not found");
        assert!(
            double_branch
                .contains("const pageW = Math.max(1, Math.floor((viewportW - gutter) / 2));"),
            "double-page pageW should follow prototype floor((viewportW - gutter) / 2)"
        );
        assert!(
            double_branch.contains("const colW = Math.max(1, pageW - 2 * marginH);"),
            "double-page colW should be pageW - 2 * marginH"
        );
        assert!(
            double_branch.contains("const colGap = gutter + 2 * marginH;"),
            "double-page column-gap should be gutter + 2 * marginH"
        );
        assert!(
            double_branch.contains("columnContent.style.width = viewportW + 'px';"),
            "double-page #column-content width should equal viewportW"
        );
        assert!(
            double_branch.contains("columnContent.style.paddingLeft = marginH + 'px';"),
            "double-page #column-content left padding should equal marginH"
        );
        assert!(
            double_branch.contains("columnContent.style.paddingRight = marginH + 'px';"),
            "double-page #column-content right padding should equal marginH"
        );
        assert!(
            double_branch.contains("columnContent.style.columnCount = 'auto';"),
            "double-page columnCount should be auto"
        );
    }

    #[test]
    fn test_reader_html_column_double_page_transform_step() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnApplyTransform")
            .next()
            .unwrap();
        let double_branch = layout_body
            .split("if (isDoubleMode()) {")
            .nth(1)
            .expect("double branch not found")
            .split("\n  } else {{")
            .next()
            .expect("double branch end not found");
        assert!(
            double_branch.contains("const viewShift = viewportW + gutter;"),
            "double-page transform step should be viewportW + gutter"
        );
        let transform_body = html
            .split("function columnApplyTransform()")
            .nth(1)
            .expect("columnApplyTransform not found")
            .split("function columnGoToSpread")
            .next()
            .unwrap();
        let double_transform = transform_body
            .split("} else if (isDoubleMode()) {")
            .nth(1)
            .expect("double transform branch not found")
            .split("\n  }} else {{")
            .next()
            .expect("double transform branch end not found");
        assert!(
            double_transform.contains("columnState.currentSpread * columnState.viewShift"),
            "double-page transform should use currentSpread * viewShift"
        );
        assert!(
            double_transform.contains("columnView.style.transform = `translateX(-${offset}px)`;"),
            "double-page transform should translateX by negative offset"
        );
    }

    #[test]
    fn test_reader_html_column_double_page_total_spread_formula() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnApplyTransform")
            .next()
            .unwrap();
        let double_branch = layout_body
            .split("if (isDoubleMode()) {")
            .nth(1)
            .expect("double branch not found")
            .split("\n  } else {{")
            .next()
            .expect("double branch end not found");
        assert!(
            double_branch.contains("Math.ceil((scrollW - 2 * marginH) / viewShift)"),
            "double-page total spreads should use ceil((scrollW - 2 * marginH) / viewShift)"
        );
    }

    #[test]
    fn test_reader_html_column_goto_page_maps_page_index_to_spread() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnGoToPage(pageIndex)")
            .nth(1)
            .expect("columnGoToPage not found")
            .split("function columnNext")
            .next()
            .unwrap();
        assert!(
            fn_body
                .contains("const spread = isDoubleMode() ? Math.floor(pageIndex / 2) : pageIndex;"),
            "columnGoToPage should map a page index to the containing spread"
        );
        assert!(
            fn_body.contains("columnGoToSpread(spread);"),
            "columnGoToPage should delegate to columnGoToSpread"
        );
    }

    #[test]
    fn test_reader_html_column_go_to_spread_delegates_directly() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function goToSpread(index, animate)")
            .nth(1)
            .expect("goToSpread not found")
            .split("if (spreads.length === 0) return;")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("return columnGoToSpread(index);"),
            "goToSpread should delegate directly to columnGoToSpread in column mode"
        );
        assert!(
            !fn_body.contains("const pageIndex = isDoubleMode() ? index * 2 : index;"),
            "goToSpread should not convert spread index to page index in column mode"
        );
        assert!(
            !fn_body.contains("return columnGoToPage(pageIndex);"),
            "goToSpread should not route through columnGoToPage in column mode"
        );
    }

    #[test]
    fn test_reader_html_column_report_position_uses_spread() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnReportPosition()")
            .nth(1)
            .expect("columnReportPosition not found")
            .split("function pageHeight")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("let total = columnGetPageCount();"),
            "columnReportPosition should resolve the total spread count"
        );
        assert!(
            fn_body.contains("\"total_spreads\": total"),
            "columnReportPosition should report the resolved total spread count"
        );
        assert!(
            fn_body.contains("let spread = columnState.currentSpread;"),
            "columnReportPosition should start from the current spread index"
        );
        assert!(
            fn_body.contains("columnView.scrollTop / ch"),
            "columnReportPosition should derive the scroll page from #column-view in scroll mode"
        );
        assert!(
            fn_body.contains("\"spread\": spread,"),
            "columnReportPosition should report the resolved spread index"
        );
    }

    #[test]
    fn test_reader_html_column_layout_covers_single_and_double_branches() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnLayout()")
            .nth(1)
            .expect("columnLayout not found")
            .split("function columnApplyTransform")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("if (isDoubleMode()) {"),
            "columnLayout should branch for double-page mode"
        );
        assert!(
            fn_body.contains("} else {"),
            "columnLayout should have a fallback single-page branch"
        );
        assert!(
            fn_body.contains("const pageW = Math.max(1, viewportW);"),
            "single-page branch should use viewportW as pageW"
        );
        assert!(
            fn_body.contains("Math.ceil(scrollW / pageW)"),
            "single-page branch should compute total pages from scrollW / pageW"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_mode_css_rules() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let css = html
            .split("body.scroll #column-content {")
            .nth(1)
            .expect("scroll column-content rule not found")
            .split('}')
            .next()
            .unwrap();
        assert!(
            css.contains("column-count: 1") || css.contains("columns: 1"),
            "scroll mode must disable CSS columns"
        );
        assert!(
            css.contains("height: auto"),
            "scroll mode column-content should grow with content"
        );
        let view_css = html
            .split("body.scroll #column-view {")
            .nth(1)
            .expect("scroll column-view rule not found")
            .split('}')
            .next()
            .unwrap();
        assert!(
            view_css.contains("overflow-y: scroll"),
            "scroll mode #column-view must be the scroll container"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_next_clamps_and_crosses_chapter() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function columnNext()")
            .nth(1)
            .expect("columnNext not found")
            .split("function columnPrev")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnMaxScroll()"),
            "columnNext scroll mode should clamp against the maximum scroll"
        );
        assert!(
            fn_body.contains(
                "const target = Math.min(maxScroll, columnView.scrollTop + columnScrollStep());"
            ),
            "columnNext scroll mode should compute a clamped target scroll position"
        );
        assert!(
            fn_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "columnNext scroll mode should smooth-scroll when animation is enabled"
        );
        assert!(
            fn_body.contains("columnView.scrollTop = target;"),
            "columnNext scroll mode should set scrollTop directly when animation is disabled"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter + 1, 0)"),
            "columnNext scroll mode should load the next chapter at the bottom"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_prev_clamps_and_crosses_chapter() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function columnPrev()")
            .nth(1)
            .expect("columnPrev not found")
            .split("function columnComputeCharOffset")
            .next()
            .unwrap();
        assert!(
            fn_body
                .contains("const target = Math.max(0, columnView.scrollTop - columnScrollStep());"),
            "columnPrev scroll mode should compute a clamped target scroll position"
        );
        assert!(
            fn_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "columnPrev scroll mode should smooth-scroll when animation is enabled"
        );
        assert!(
            fn_body.contains("columnView.scrollTop = target;"),
            "columnPrev scroll mode should set scrollTop directly when animation is disabled"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER)"),
            "columnPrev scroll mode should load the previous chapter at the top"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_goto_page_maps_index_to_ratio() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnGoToPage(pageIndex)")
            .nth(1)
            .expect("columnGoToPage not found")
            .split("function columnNext")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("columnGoToPage scroll branch not found")
            .split("\n  }} else {{")
            .next()
            .expect("columnGoToPage scroll branch end not found");
        assert!(
            scroll_branch.contains("columnGetPageCount()"),
            "columnGoToPage scroll mode should read total scroll pages"
        );
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * columnMaxScroll());"),
            "columnGoToPage scroll mode should set scrollTop from the page ratio"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_position_reporting_uses_column_view() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function columnReportPosition()")
            .nth(1)
            .expect("columnReportPosition not found")
            .split("function pageHeight")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("columnReportPosition scroll branch not found")
            .split("\n  }}")
            .next()
            .expect("columnReportPosition scroll branch end not found");
        assert!(
            scroll_branch.contains("columnView.clientHeight"),
            "scroll position reporting should use #column-view viewport height"
        );
        assert!(
            scroll_branch.contains("columnView.scrollTop"),
            "scroll position reporting should use #column-view scroll position"
        );
        assert!(
            scroll_branch.contains("Math.floor(columnView.scrollTop / ch)"),
            "scroll position reporting should derive the scroll page from #column-view"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_apply_settings_restores_ratio() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        let column_branch = fn_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("applySettings column branch not found")
            .split("\n      return;")
            .next()
            .expect("applySettings column branch end not found");
        let scroll_branch = column_branch
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("applySettings column scroll branch not found")
            .split("\n      } else {")
            .next()
            .expect("applySettings column scroll branch end not found");
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * columnMaxScroll());"),
            "applySettings scroll mode should restore #column-view scroll position from saved ratio"
        );
        assert!(
            scroll_branch.contains("columnReportPosition();"),
            "applySettings scroll mode should report position after restoring scroll"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_load_chapter_restores_offset() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        let column_branch = fn_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("loadChapter column branch not found")
            .split("\n      return;")
            .next()
            .unwrap();
        let scroll_branch = column_branch
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("loadChapter column scroll branch not found")
            .split("\n      } else {")
            .next()
            .expect("loadChapter column scroll branch end not found");
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * columnMaxScroll());"),
            "loadChapter scroll mode should scroll #column-view to the approximate offset"
        );
        assert!(
            scroll_branch.contains("columnReportPosition();"),
            "loadChapter scroll mode should report position after scrolling"
        );
    }

    #[test]
    fn test_reader_html_column_view_has_scroll_listener() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains(
                "columnView.addEventListener('scroll', columnReportPosition, { passive: true });"
            ),
            "#column-view should report position while scrolling"
        );
    }

    #[test]
    fn test_reader_html_window_scroll_listener_ignores_column_view() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let listener = html
            .split("window.addEventListener('scroll', (e) => {")
            .nth(1)
            .expect("window scroll listener not found")
            .split("\n}}, true);")
            .next()
            .expect("window scroll listener end not found");
        assert!(
            listener.contains("if (columnView && e.target === columnView) return;"),
            "window scroll listener must ignore #column-view scroll events to avoid duplicate position reports"
        );
        assert!(
            listener.contains("reportPosition();"),
            "window scroll listener should still report position for other scroll targets"
        );
    }

    #[test]
    fn test_reader_html_column_animation_helpers_present() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("#column-view.column-animate {"),
            "column paginator should define a CSS class for animated transforms"
        );
        assert!(
            html.contains("transition: transform 0.25s ease"),
            "column animation CSS should transition the transform property"
        );
        assert!(
            html.contains("let columnAnimationTimer = null;"),
            "column paginator should track a shared animation timer"
        );
        assert!(
            html.contains("function columnEnableAnimation()"),
            "column paginator should expose columnEnableAnimation"
        );
        assert!(
            html.contains("function columnDisableAnimation()"),
            "column paginator should expose columnDisableAnimation"
        );
        assert!(
            html.contains("function columnScheduleDisableAnimation()"),
            "column paginator should expose columnScheduleDisableAnimation"
        );
        assert!(
            html.contains("columnView.classList.add('column-animate')"),
            "columnEnableAnimation should add the animate class"
        );
        assert!(
            html.contains("columnView.classList.remove('column-animate')"),
            "columnDisableAnimation should remove the animate class"
        );
        assert!(
            html.contains("columnAnimationTimer = setTimeout"),
            "columnScheduleDisableAnimation should schedule a timer"
        );
        assert!(
            html.contains("clearTimeout(columnAnimationTimer)"),
            "column animation helpers should cancel any pending timer"
        );
    }

    #[test]
    fn test_reader_html_column_animation_toggled_by_settings() {
        use rust_reader_storage::models::EbookSettings;
        let on = EbookSettings {
            enable_page_animation: true,
            ..Default::default()
        };
        let off = EbookSettings {
            enable_page_animation: false,
            ..Default::default()
        };
        let html_on = reader_html(&on, 1);
        let html_off = reader_html(&off, 1);
        assert!(
            html_on.contains("animate: true"),
            "enabled setting should render animate: true"
        );
        assert!(
            html_off.contains("animate: false"),
            "disabled setting should render animate: false"
        );
        for html in &[html_on, html_off] {
            let next_body = html
                .split("function columnNext()")
                .nth(1)
                .expect("columnNext not found")
                .split("function columnPrev")
                .next()
                .unwrap();
            assert!(
                next_body.contains("if (currentSettings.animate) columnEnableAnimation();"),
                "columnNext should guard animation on currentSettings.animate"
            );
            assert!(
                next_body.contains("columnScheduleDisableAnimation();"),
                "columnNext should schedule disabling animation after the transition duration"
            );
        }
    }

    #[test]
    fn test_reader_html_column_paginated_animation_branches() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let next_body = html
            .split("function columnNext()")
            .nth(1)
            .expect("columnNext not found")
            .split("function columnPrev")
            .next()
            .unwrap();
        assert!(
            next_body.contains("if (currentSettings.animate) columnEnableAnimation();"),
            "columnNext paginated branch should enable animation before moving"
        );
        assert!(
            next_body.contains("columnGoToSpread(columnState.currentSpread + 1);"),
            "columnNext paginated branch should advance one spread"
        );
        assert!(
            next_body.contains("columnScheduleDisableAnimation();"),
            "columnNext paginated branch should schedule disabling animation after the transition"
        );

        let prev_body = html
            .split("function columnPrev()")
            .nth(1)
            .expect("columnPrev not found")
            .split("function columnComputeCharOffset")
            .next()
            .unwrap();
        assert!(
            prev_body.contains("if (currentSettings.animate) columnEnableAnimation();"),
            "columnPrev paginated branch should enable animation before moving"
        );
        assert!(
            prev_body.contains("columnGoToSpread(columnState.currentSpread - 1);"),
            "columnPrev paginated branch should go back one spread"
        );
        assert!(
            prev_body.contains("columnScheduleDisableAnimation();"),
            "columnPrev paginated branch should schedule disabling animation after the transition"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_column_uses_text_content_length_for_ratio() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let load_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        let column_branch = load_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("loadChapter column branch not found")
            .split("\n      return;")
            .next()
            .expect("loadChapter column branch end not found");
        assert!(
            column_branch.contains("const totalChars = columnContent.textContent.length;"),
            "loadChapter column mode should measure total characters from columnContent"
        );
        assert!(
            column_branch.contains("const ratio = Math.min(1, charOffset / totalChars);"),
            "loadChapter column mode should compute the character ratio"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_preserves_old_paginator_offset_when_enabling_columns() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let apply_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        // When leaving the old paginator (not column, not scroll), the saved offset is the
        // current spread's cumulative character offset.
        assert!(
            apply_body.contains("savedCharOffset = currentSpreadCharOffset();"),
            "applySettings should capture the old paginator's char offset"
        );
        let column_branch = apply_body
            .split("if (isColumnMode()) {")
            .nth(1)
            .expect("applySettings column branch not found")
            .split("\n      return;")
            .next()
            .expect("applySettings column branch end not found");
        assert!(
            column_branch.contains("const ratio = savedCharOffset / totalChars;"),
            "applySettings column mode should restore position from the saved ratio"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_preserves_column_offset_when_disabling_columns() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let apply_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        // When leaving column mode (but not scroll mode), save the column-computed offset.
        assert!(
            apply_body.contains("savedCharOffset = columnComputeCharOffset();"),
            "applySettings should capture the column paginator's char offset"
        );
        // The old paginator then resumes from that offset.
        assert!(
            apply_body.contains("currentSpread = findSpreadForOffset(offset);"),
            "applySettings old-paginator branch should resume from the saved offset"
        );
    }
}
