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
  overflow-y: scroll;
}}
body.scroll #column-content {{
  columns: 1;
  column-width: auto;
  column-gap: 0;
  width: auto;
  padding: var(--margin-v) var(--margin-h);
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

let columnState = {{
  currentPage: 0,
  totalPages: 1,
  pageWidth: 0,
  viewShift: 0
}};

function columnGetPageCount() {{
  return (columnState && columnState.totalPages) || 0;
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
    columnContent.style.width = 'auto';
    columnContent.style.paddingLeft = '0';
    columnContent.style.paddingRight = '0';
    columnContent.style.columnWidth = 'auto';
    columnContent.style.columns = '1';
    columnContent.style.columnGap = '0';
    columnView.style.transform = 'none';
    columnState.totalPages = 1;
    columnState.currentPage = 0;
    columnReportPosition();
    return;
  }}

  const viewportW = document.body.clientWidth;
  const marginH = getMarginH();
  const gutter = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--column-gutter')) || 40;

  if (isDoubleMode()) {{
    const pageW = Math.floor((viewportW - gutter) / 2);
    const colW = pageW - 2 * marginH;
    const cg = gutter + 2 * marginH;
    const viewShift = viewportW + gutter;
    columnContent.style.width = viewportW + 'px';
    columnContent.style.paddingLeft = marginH + 'px';
    columnContent.style.paddingRight = marginH + 'px';
    columnContent.style.columnWidth = colW + 'px';
    columnContent.style.columnGap = cg + 'px';
    columnContent.style.columnCount = 'auto';
    const scrollW = columnContent.scrollWidth;
    columnState.pageWidth = pageW;
    columnState.viewShift = viewShift;
    columnState.totalPages = Math.max(1, Math.ceil((scrollW - 2 * marginH) / viewShift));
  }} else {{
    const pageW = viewportW;
    const colW = pageW - 2 * marginH;
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

  columnState.currentPage = Math.max(0, Math.min(columnState.currentPage, columnGetPageCount() - 1));
  columnGoToPage(columnState.currentPage);
}}

function columnGoToPage(n) {{
  columnState.currentPage = Math.max(0, Math.min(n, columnGetPageCount() - 1));
  if (isScrollMode()) {{
    columnView.style.transform = 'none';
  }} else if (isDoubleMode()) {{
    const offset = columnState.currentPage * columnState.viewShift;
    columnView.style.transform = `translateX(-${{offset}}px)`;
  }} else {{
    const marginH = getMarginH();
    const offset = columnState.currentPage * columnState.pageWidth;
    columnView.style.transform = `translateX(${{-offset + marginH}}px)`;
  }}
  columnReportPosition();
}}

function columnNext() {{
  if (isScrollMode()) {{
    columnView.scrollTop += columnView.clientHeight * 0.9;
    return;
  }}
  if (columnState.currentPage + 1 < columnGetPageCount()) {{
    columnGoToPage(columnState.currentPage + 1);
  }} else if (currentChapter + 1 < window.ebookChapterCount) {{
    loadChapter(currentChapter + 1, 0);
  }}
}}

function columnPrev() {{
  if (isScrollMode()) {{
    columnView.scrollTop -= columnView.clientHeight * 0.9;
    return;
  }}
  if (columnState.currentPage > 0) {{
    columnGoToPage(columnState.currentPage - 1);
  }} else if (currentChapter > 0) {{
    loadChapter(currentChapter - 1, 0).then(() => {{
      columnGoToPage(columnGetPageCount() - 1);
    }});
  }}
}}

function columnComputeCharOffset() {{
  const total = columnContent.textContent.length;
  if (total === 0) return 0;
  if (isScrollMode()) {{
    const ratio = columnView.scrollTop / Math.max(1, columnView.scrollHeight - columnView.clientHeight);
    return Math.floor(total * Math.max(0, Math.min(1, ratio)));
  }}
  const ratio = columnState.currentPage / Math.max(1, columnGetPageCount());
  return Math.floor(total * ratio);
}}

function columnReportPosition() {{
  sendIpc({{
    "type": "position",
    "chapter": currentChapter,
    "spread": columnState.currentPage,
    "char_offset": columnComputeCharOffset(),
    "total_spreads": columnGetPageCount()
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
  if (isColumnMode()) return columnGoToPage(index);
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
      columnLayout();
      return;
    }}
    if (isScrollMode()) {{
      cancelFlip();
      const offset = currentSpreadCharOffset();
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
      const offset = currentSpreadCharOffset();
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
      columnLayout();
      if (typeof charOffset === 'number' && charOffset >= 0 && columnContent.textContent.length > 0) {{
        const ratio = Math.min(1, charOffset / columnContent.textContent.length);
        const targetPage = Math.floor(ratio * (columnGetPageCount() - 1));
        columnGoToPage(targetPage);
      }} else {{
        columnGoToPage(0);
      }}
      return;
    }}
    if (isScrollMode()) {{
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
}}
window.addEventListener('scroll', reportPosition, true);

let resizeTimeout = null;
window.addEventListener('resize', () => {{
  clearTimeout(resizeTimeout);
  resizeTimeout = setTimeout(() => {{
    if (!currentChapterHtml || isScrollMode()) return;
    if (isColumnMode()) {{
      columnLayout();
      return;
    }}
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
        assert!(html.contains("!isScrollMode()"));
        assert!(html.contains("splitIntoSpreads(currentChapterHtml)"));
        assert!(html.contains("goToSpread(currentSpread, false)"));
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
        assert!(html.contains("function columnNext()"));
        assert!(html.contains("function columnPrev()"));
        assert!(html.contains("function columnReportPosition()"));
        assert!(html.contains("function columnComputeCharOffset()"));
        assert!(html.contains("function columnGetPageCount()"));
        // Dispatch hooks in existing functions
        assert!(html.contains("if (isColumnMode()) return columnNext();"));
        assert!(html.contains("if (isColumnMode()) return columnPrev();"));
        assert!(html.contains("if (isColumnMode()) return columnReportPosition();"));
        assert!(html.contains("if (isColumnMode()) return columnGoToPage("));
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
            .split("function columnGoToPage(n)")
            .nth(1)
            .expect("columnGoToPage not found")
            .split("function columnNext")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnView.style.transform = `translateX(${-offset + marginH}px)`;"),
            "single-page transform should follow prototype translateX(-currentPage * pageW + marginH)"
        );
    }

    #[test]
    fn test_reader_html_column_scroll_mode_uses_column_view() {
        use rust_reader_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("columnView.scrollTop += columnView.clientHeight * 0.9;"),
            "columnNext scroll mode should scroll #column-view"
        );
        assert!(
            html.contains("columnView.scrollTop -= columnView.clientHeight * 0.9;"),
            "columnPrev scroll mode should scroll #column-view"
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
            html.contains("function columnGoToPage(n) {"),
            "columnGoToPage should take only the page index"
        );
        assert!(
            !html.contains("function columnGoToPage(n, animate)"),
            "columnGoToPage should not have an unused animate parameter"
        );
        assert!(
            !html.contains("columnGoToPage(0, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentPage, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentPage + 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnState.currentPage - 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(columnGetPageCount() - 1, true)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(targetPage, false)"),
            "columnGoToPage callers should not pass animate"
        );
        assert!(
            !html.contains("columnGoToPage(index, animate || false)"),
            "goToSpread should not forward animate to columnGoToPage"
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
}
