//! HTML/JS shell template for the ebook webview renderer.

use super::JsSettings;
use openitgo_storage::models::EbookSettings;

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
h1, h2, h3, h4, h5, h6 {{ margin: 0.8em 0 0.4em; line-height: 1.3; }}
ul, ol {{ margin: 0 0 1em 0; padding-left: 2em; }}
li {{ margin: 0.25em 0; }}
img {{ max-width: 100%; max-height: calc(100vh - var(--margin-v) * 2); height: auto; object-fit: contain; }}
#column-view {{
  display: block;
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
#column-content.column-animate {{
  transition: transform 0.25s ease;
}}
.ebook-search-highlight {{
  background: rgba(255, 215, 0, 0.55);
  color: inherit;
  border-radius: 2px;
}}
.ebook-search-active {{
  background: rgba(255, 140, 0, 0.85);
  color: inherit;
}}
</style>
</head>
<body class="{mode}">
<div id="column-view"><div id="column-content"></div></div>
<div id="ebook-error-layer" style="display:none; position:absolute; top:0; left:0; width:100%; height:100%; z-index:10; background:var(--bg); color:var(--fg); padding:2em; overflow:auto;"></div>
<script>
const columnView = document.getElementById('column-view');
const columnContent = document.getElementById('column-content');
const errorLayer = document.getElementById('ebook-error-layer');
let currentChapter = 0;
let currentChapterHtml = '';
window.ebookChapterCount = {chapter_count};
const RESIZE_DEBOUNCE_MS = 200;
let currentSettings = {{
  mode: '{mode}',
  animate: {animate},
  invert_scroll: {invert_scroll}
}};

// Cache layout results so identical chapter + viewport + setting combinations
// do not trigger a full reflow. The key is derived from everything that
// influences CSS columns sizing.
let layoutCache = {{
  key: null,
  totalPages: 1,
  pageWidth: 0,
  viewShift: 0
}};
let lastLayoutParams = {{ width: 0, height: 0, mode: '' }};

function hashString(s) {{
  let h = 5381;
  for (let i = 0; i < s.length; i++) {{
    h = ((h << 5) + h) + s.charCodeAt(i);
  }}
  return h >>> 0;
}}

function makeLayoutKey() {{
  const viewportW = document.body.clientWidth;
  const viewportH = columnView ? columnView.clientHeight : 0;
  const marginH = getMarginH();
  const marginV = getMarginV();
  const root = document.documentElement;
  const fontSize = parseFloat(getComputedStyle(root).getPropertyValue('--size')) || 16;
  const lineHeight = parseFloat(getComputedStyle(root).getPropertyValue('--line')) || 1.6;
  const gutter = parseFloat(getComputedStyle(root).getPropertyValue('--column-gutter')) || 40;
  const font = getComputedStyle(root).getPropertyValue('--font').trim();
  const mode = isScrollMode() ? 'scroll' : (isDoubleMode() ? 'double' : 'single');
  return [
    currentChapter,
    hashString(currentChapterHtml || ''),
    viewportW,
    viewportH,
    marginH,
    marginV,
    fontSize,
    lineHeight,
    font,
    mode,
    gutter
  ].join('|');
}}

function escapeHtml(s) {{
  return String(s).replace(/[&<>"']/g, function(c) {{
    return {{ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }}[c];
  }});
}}

function showError(title, detail) {{
  if (!errorLayer) return;
  errorLayer.style.display = 'block';
  errorLayer.innerHTML = '<div id="ebook-error" style="max-width:60em; margin:0 auto;">' +
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

function hideError() {{
  if (!errorLayer) return;
  errorLayer.style.display = 'none';
  errorLayer.innerHTML = '';
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

function isScrollMode() {{ return document.body.classList.contains('scroll'); }}
function isDoubleMode() {{ return document.body.classList.contains('double'); }}

let animationTimer = null;

function enableAnimation() {{
  if (columnContent) columnContent.classList.add('column-animate');
}}

function disableAnimation() {{
  if (animationTimer) {{
    clearTimeout(animationTimer);
    animationTimer = null;
  }}
  if (columnContent) columnContent.classList.remove('column-animate');
}}

function scheduleDisableAnimation() {{
  if (animationTimer) clearTimeout(animationTimer);
  animationTimer = setTimeout(() => {{
    animationTimer = null;
    disableAnimation();
  }}, 260);
}}

let paginatorState = {{
  currentSpread: 0,
  totalPages: 1,
  pageWidth: 0,
  viewShift: 0
}};

function getPageCount() {{
  if (isScrollMode()) {{
    if (!columnView) return 1;
    const ch = Math.max(1, columnView.clientHeight);
    return Math.max(1, Math.ceil(columnView.scrollHeight / ch));
  }}
  return (paginatorState && paginatorState.totalPages) || 0;
}}

function scrollStep() {{
  if (!columnView) return 1;
  return Math.max(1, columnView.clientHeight - 2 * getMarginV());
}}

function maxScroll() {{
  if (!columnView) return 0;
  return Math.max(0, columnView.scrollHeight - columnView.clientHeight);
}}

function layout() {{
  if (!columnContent) return;
  if (!columnView) return;
  const viewportW = document.body.clientWidth;
  const viewportH = columnView.clientHeight;
  const layoutMode = isScrollMode() ? 'scroll' : (isDoubleMode() ? 'double' : 'single');
  lastLayoutParams = {{ width: viewportW, height: viewportH, mode: layoutMode }};

  const cacheKey = makeLayoutKey();
  if (layoutCache.key === cacheKey) {{
    paginatorState.totalPages = layoutCache.totalPages;
    paginatorState.pageWidth = layoutCache.pageWidth;
    paginatorState.viewShift = layoutCache.viewShift;
    return;
  }}

  if (viewportW === 0 || viewportH === 0) {{
    paginatorState.totalPages = 1;
    paginatorState.currentSpread = 0;
    paginatorState.pageWidth = 0;
    paginatorState.viewShift = 0;
    layoutCache = {{ key: cacheKey, totalPages: 1, pageWidth: 0, viewShift: 0 }};
    reportPosition();
    return;
  }}

  if (isScrollMode()) {{
    columnContent.style.width = 'auto';
    columnContent.style.paddingLeft = 'var(--margin-h)';
    columnContent.style.paddingRight = 'var(--margin-h)';
    columnContent.style.columnWidth = 'auto';
    columnContent.style.columnCount = '1';
    columnContent.style.columnGap = '0';
    columnContent.style.height = 'auto';
    columnContent.style.minHeight = '100%';
    columnContent.style.transform = 'none';
    paginatorState.totalPages = 1;
    paginatorState.currentSpread = 0;
    paginatorState.pageWidth = 0;
    paginatorState.viewShift = 0;
    layoutCache = {{ key: cacheKey, totalPages: 1, pageWidth: 0, viewShift: 0 }};
    reportPosition();
    return;
  }}

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
    paginatorState.pageWidth = pageW;
    paginatorState.viewShift = viewShift;
    paginatorState.totalPages = Math.max(1, Math.ceil((scrollW - 2 * marginH) / viewShift));
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
    paginatorState.pageWidth = pageW;
    paginatorState.viewShift = pageW;
    paginatorState.totalPages = Math.max(1, Math.ceil(scrollW / pageW));
  }}

  layoutCache = {{
    key: cacheKey,
    totalPages: paginatorState.totalPages,
    pageWidth: paginatorState.pageWidth,
    viewShift: paginatorState.viewShift
  }};

  // Navigation is explicit in callers; this function only measures layout.
}}

// Recompute total pages from the existing layout geometry without forcing a
// full reflow. Used by the resize handler when only the viewport height changed.
function recomputeTotalPages() {{
  if (isScrollMode() || !columnContent) return;
  const scrollW = columnContent.scrollWidth;
  if (isDoubleMode()) {{
    const marginH = getMarginH();
    paginatorState.totalPages = Math.max(1, Math.ceil((scrollW - 2 * marginH) / paginatorState.viewShift));
  }} else {{
    paginatorState.totalPages = Math.max(1, Math.ceil(scrollW / paginatorState.pageWidth));
  }}
}}

function applyTransform() {{
  if (isScrollMode()) {{
    columnContent.style.transform = 'none';
  }} else if (isDoubleMode()) {{
    const offset = paginatorState.currentSpread * paginatorState.viewShift;
    columnContent.style.transform = `translateX(-${{offset}}px)`;
  }} else {{
    const marginH = getMarginH();
    const offset = paginatorState.currentSpread * paginatorState.pageWidth;
    columnContent.style.transform = `translateX(${{-offset + marginH}}px)`;
  }}
}}

function goToSpread(n) {{
  paginatorState.currentSpread = Math.max(0, Math.min(n, getPageCount() - 1));
  applyTransform();
  reportPosition();
}}

function goToPage(pageIndex) {{
  if (isScrollMode()) {{
    const total = getPageCount();
    const clamped = Math.max(0, Math.min(pageIndex, total - 1));
    const ratio = total > 0 ? clamped / total : 0;
    columnView.scrollTop = Math.floor(ratio * maxScroll());
    reportPosition();
    return;
  }}
  const spread = isDoubleMode() ? Math.floor(pageIndex / 2) : pageIndex;
  goToSpread(spread);
}}

function nextPage() {{
  if (isScrollMode()) {{
    const maxScrollVal = maxScroll();
    if (columnView.scrollTop >= maxScrollVal - 1) {{
      if (currentChapter + 1 < window.ebookChapterCount) {{
        loadChapter(currentChapter + 1, 0);
      }}
    }} else {{
      const target = Math.min(maxScrollVal, columnView.scrollTop + scrollStep());
      if (currentSettings.animate) {{
        columnView.scrollTo({{ top: target, behavior: 'smooth' }});
      }} else {{
        columnView.scrollTop = target;
      }}
      reportPosition();
    }}
    return;
  }}
  if (paginatorState.currentSpread + 1 < getPageCount()) {{
    if (currentSettings.animate) enableAnimation();
    goToSpread(paginatorState.currentSpread + 1);
    if (currentSettings.animate) {{
      scheduleDisableAnimation();
    }}
  }} else if (currentChapter + 1 < window.ebookChapterCount) {{
    loadChapter(currentChapter + 1, 0);
  }}
}}

function prevPage() {{
  if (isScrollMode()) {{
    if (columnView.scrollTop <= 0) {{
      if (currentChapter > 0) {{
        // Passing the maximum safe integer as charOffset makes loadChapter's ratio
        // clamp to 1, so we land directly at the bottom of the previous chapter
        // instead of flashing through the top.
        loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER);
      }}
    }} else {{
      const target = Math.max(0, columnView.scrollTop - scrollStep());
      if (currentSettings.animate) {{
        columnView.scrollTo({{ top: target, behavior: 'smooth' }});
      }} else {{
        columnView.scrollTop = target;
      }}
      reportPosition();
    }}
    return;
  }}
  if (paginatorState.currentSpread > 0) {{
    if (currentSettings.animate) enableAnimation();
    goToSpread(paginatorState.currentSpread - 1);
    if (currentSettings.animate) {{
      scheduleDisableAnimation();
    }}
  }} else if (currentChapter > 0) {{
    // Passing the maximum safe integer as charOffset makes loadChapter's ratio
    // clamp to 1, so we land directly on the last page of the previous chapter
    // instead of flashing through page 0.
    loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER);
  }}
}}

function computeCharOffset() {{
  const root = columnContent;
  const total = root ? root.textContent.length : 0;
  if (total === 0) return 0;
  if (isScrollMode()) {{
    const ratio = columnView.scrollTop / Math.max(1, columnView.scrollHeight - columnView.clientHeight);
    return Math.floor(total * Math.max(0, Math.min(1, ratio)));
  }}
  const ratio = paginatorState.currentSpread / Math.max(1, getPageCount());
  return Math.floor(total * ratio);
}}

function reportPosition() {{
  let spread = paginatorState.currentSpread;
  let total = getPageCount();
  if (isScrollMode()) {{
    const ch = Math.max(1, columnView.clientHeight);
    spread = Math.floor(columnView.scrollTop / ch);
    total = getPageCount();
  }}
  sendIpc({{
    "type": "position",
    "chapter": currentChapter,
    "spread": spread,
    "char_offset": computeCharOffset(),
    "total_spreads": total
  }});
}}

let ebookSearchHighlights = [];
let ebookSearchActiveIndex = -1;
let ebookSearchQuery = '';

function getActiveChapterRoot() {{
  return columnContent;
}}

function charOffsetOfElement(el) {{
  const root = getActiveChapterRoot();
  if (!root || !el) return 0;
  try {{
    const range = document.createRange();
    range.selectNodeContents(root);
    range.setEnd(el, 0);
    return range.toString().length;
  }} catch (e) {{
    return 0;
  }}
}}

function resolveTocTarget(target) {{
  if (!target) return null;
  let fragment = target;
  const idx = target.indexOf('#');
  if (idx !== -1) {{
    fragment = target.slice(idx + 1);
  }} else if (
    target.indexOf('/') !== -1 ||
    target.indexOf('\\') !== -1 ||
    /\.(xhtml|html|htm)$/i.test(target)
  ) {{
    // A fragment-less path/URL (e.g. "chapter.xhtml") means chapter start,
    // not an element id, so don't try to resolve it as a fragment.
    return null;
  }}
  if (!fragment) return null;
  let el = document.getElementById(fragment);
  if (!el && columnContent && typeof CSS !== 'undefined' && CSS.escape) {{
    // document.getElementById handles valid XML ids; CSS.escape covers the
    // rare edge cases (leading digits, special characters) when falling back.
    try {{ el = columnContent.querySelector('#' + CSS.escape(fragment)); }} catch (e) {{}}
  }}
  return el;
}}

function goToCharOffset(offset) {{
  const root = columnContent;
  const totalChars = root ? root.textContent.length : 0;
  if (totalChars === 0 || !Number.isFinite(offset)) return;
  const ratio = Math.max(0, Math.min(1, offset / totalChars));
  if (isScrollMode()) {{
    columnView.scrollTop = Math.floor(ratio * maxScroll());
  }} else {{
    const targetSpread = Math.floor(ratio * (getPageCount() - 1));
    goToSpread(targetSpread);
  }}
}}

async function jumpToTocItem(chapter, target) {{
  if (typeof chapter !== 'number') return;
  if (chapter !== currentChapter) {{
    await loadChapter(chapter, 0);
  }}
  if (!target) return;
  const el = resolveTocTarget(target);
  if (!el) {{
    const ratio = parseFloat(target);
    if (!isNaN(ratio) && ratio >= 0 && ratio <= 1) {{
      const root = getActiveChapterRoot();
      const totalChars = root ? root.textContent.length : 0;
      const offset = Math.floor(ratio * totalChars);
      goToCharOffset(offset);
    }}
    return;
  }}
  const offset = charOffsetOfElement(el);
  if (isScrollMode()) {{
    el.scrollIntoView({{ behavior: currentSettings.animate ? 'smooth' : 'auto', block: 'start' }});
  }} else {{
    goToCharOffset(offset);
  }}
}}

function clearHighlights() {{
  for (const mark of ebookSearchHighlights) {{
    if (mark && mark.parentNode) {{
      mark.parentNode.replaceChild(document.createTextNode(mark.textContent), mark);
    }}
  }}
  ebookSearchHighlights = [];
  ebookSearchActiveIndex = -1;
  ebookSearchQuery = '';
}}

function setSearchActiveIndex(index) {{
  if (ebookSearchHighlights.length === 0) return;
  if (ebookSearchActiveIndex >= 0 && ebookSearchActiveIndex < ebookSearchHighlights.length) {{
    ebookSearchHighlights[ebookSearchActiveIndex].classList.remove('ebook-search-active');
  }}
  let idx = index % ebookSearchHighlights.length;
  if (idx < 0) idx += ebookSearchHighlights.length;
  ebookSearchActiveIndex = idx;
  const mark = ebookSearchHighlights[idx];
  mark.classList.add('ebook-search-active');
  if (isScrollMode()) {{
    mark.scrollIntoView({{ behavior: currentSettings.animate ? 'smooth' : 'auto', block: 'center' }});
  }} else {{
    const offset = charOffsetOfElement(mark);
    goToCharOffset(offset);
  }}
  sendIpc({{ type: 'search', count: ebookSearchHighlights.length, active: idx }});
}}

function findText(query) {{
  clearHighlights();
  ebookSearchQuery = query || '';
  if (!query) {{
    sendIpc({{ type: 'search', count: 0, active: -1 }});
    return;
  }}
  const root = getActiveChapterRoot();
  if (!root) return;
  const lowerQuery = query.toLowerCase();
  while (true) {{
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
    let found = false;
    while (walker.nextNode()) {{
      const node = walker.currentNode;
      const parent = node.parentElement;
      if (!parent || parent.classList.contains('ebook-search-highlight') || parent.tagName === 'SCRIPT' || parent.tagName === 'STYLE') continue;
      const text = node.textContent;
      const lower = text.toLowerCase();
      const pos = lower.indexOf(lowerQuery);
      if (pos === -1) continue;
      const before = text.slice(0, pos);
      const matchText = text.slice(pos, pos + query.length);
      const after = text.slice(pos + query.length);
      const fragment = document.createDocumentFragment();
      if (before) fragment.appendChild(document.createTextNode(before));
      const mark = document.createElement('mark');
      mark.className = 'ebook-search-highlight';
      mark.textContent = matchText;
      fragment.appendChild(mark);
      let afterNode = null;
      if (after) {{
        afterNode = document.createTextNode(after);
        fragment.appendChild(afterNode);
      }}
      parent.replaceChild(fragment, node);
      ebookSearchHighlights.push(mark);
      found = true;
      break;
    }}
    if (!found) break;
  }}
  if (ebookSearchHighlights.length > 0) {{
    setSearchActiveIndex(0);
  }} else {{
    sendIpc({{ type: 'search', count: 0, active: -1 }});
  }}
}}

// Re-run the active search after a relayout (settings change, resize, chapter
// switch) replaced the chapter DOM and dropped all highlight marks.
function restoreSearchAfterLayout() {{
  if (!ebookSearchQuery) return;
  findText(ebookSearchQuery);
}}

function findNext() {{
  if (ebookSearchHighlights.length === 0) return;
  setSearchActiveIndex(ebookSearchActiveIndex + 1);
}}

function findPrev() {{
  if (ebookSearchHighlights.length === 0) return;
  setSearchActiveIndex(ebookSearchActiveIndex - 1);
}}

function getMarginH() {{
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--margin-h')) || 0;
}}

function getMarginV() {{
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--margin-v')) || 0;
}}

function applySettings(json) {{
  const s = typeof json === 'string' ? JSON.parse(json) : json;
  currentSettings = s;
  // Settings that affect CSS columns changed; force a fresh layout even if the
  // computed key happens to collide.
  layoutCache.key = null;
  // NOTE: re-layout replaces the chapter DOM, so any search highlights are
  // discarded. Callers that need persistent search results must re-issue
  // findText after the layout settles.
  // Capture the approximate character position before layout changes, so we
  // can restore the closest page afterwards.
  let savedCharOffset = 0;
  if (isScrollMode()) {{
    const totalChars = columnContent.textContent.length;
    const maxScrollVal = maxScroll();
    if (totalChars > 0 && maxScrollVal > 0) {{
      const ratio = columnView.scrollTop / maxScrollVal;
      savedCharOffset = Math.floor(totalChars * Math.max(0, Math.min(1, ratio)));
    }}
  }} else {{
    savedCharOffset = computeCharOffset();
  }}
  const root = document.documentElement;
  root.style.setProperty('--bg', s.bg);
  root.style.setProperty('--fg', s.fg);
  root.style.setProperty('--font', s.font);
  root.style.setProperty('--size', s.size + 'px');
  root.style.setProperty('--line', s.line);
  root.style.setProperty('--margin-h', s.margin_h + 'px');
  root.style.setProperty('--margin-v', s.margin_v + 'px');
  document.body.className = s.mode;
  // 设置变化可能导致分页改变，重新布局并恢复进度
  if (currentChapterHtml) {{
    columnContent.innerHTML = currentChapterHtml;
    hideError();
    layout();
    const totalChars = columnContent.textContent.length;
    if (isScrollMode()) {{
      if (totalChars > 0 && savedCharOffset > 0) {{
        const ratio = savedCharOffset / totalChars;
        columnView.scrollTop = Math.floor(ratio * maxScroll());
      }}
      reportPosition();
    }} else {{
      let targetSpread = 0;
      if (totalChars > 0 && savedCharOffset > 0) {{
        const ratio = savedCharOffset / totalChars;
        targetSpread = Math.floor(ratio * (getPageCount() - 1));
      }}
      goToSpread(targetSpread);
    }}
    restoreSearchAfterLayout();
  }}
}}

// Lightweight adjacent-chapter preload. The fetched HTML is parsed into an
// inert <template> so it warms the Rust EPUB parser / HTML parse cache without
// affecting the visible column layout.
async function preloadChapter(index) {{
  if (typeof index !== 'number') return;
  if (index < 0 || index >= window.ebookChapterCount) return;
  const id = 'preload-chapter-' + index;
  if (document.getElementById(id)) return;
  try {{
    const res = await fetch('ebook://reader?chapter=' + index);
    const html = await res.text();
    const template = document.createElement('template');
    template.id = id;
    template.innerHTML = html;
    document.body.appendChild(template);
  }} catch (e) {{
    // Preloading is best-effort; failures should not affect the current chapter.
  }}
}}

// Remove stale preloaded chapter templates, keeping only the adjacent chapters
// around keepCenter so the DOM does not grow unbounded.
function cleanupPreloaded(keepCenter) {{
  if (!Number.isFinite(keepCenter)) return;
  const minIndex = Math.max(0, keepCenter - 1);
  const maxIndex = Math.min(window.ebookChapterCount - 1, keepCenter + 1);
  const templates = document.querySelectorAll('template[id^="preload-chapter-"]');
  for (const template of templates) {{
    const match = template.id.match(/^preload-chapter-(\d+)$/);
    if (!match) continue;
    const index = parseInt(match[1], 10);
    if (index < minIndex || index > maxIndex) {{
      template.remove();
    }}
  }}
}}

async function loadChapter(index, charOffset) {{
  index = index ?? 0;
  currentChapter = index;
  cleanupPreloaded(currentChapter);
  // A new chapter always needs a fresh layout.
  layoutCache.key = null;
  try {{
    const res = await fetch('ebook://reader?chapter=' + currentChapter);
    currentChapterHtml = await res.text();
    columnContent.innerHTML = currentChapterHtml;
    hideError();
    // Measure first so getPageCount() is valid before restoring progress.
    layout();
    const totalChars = columnContent.textContent.length;
    if (isScrollMode()) {{
      if (typeof charOffset === 'number' && charOffset >= 0 && totalChars > 0) {{
        const ratio = Math.min(1, charOffset / totalChars);
        columnView.scrollTop = Math.floor(ratio * maxScroll());
      }}
      reportPosition();
    }} else {{
      let targetSpread = 0;
      if (typeof charOffset === 'number' && charOffset >= 0 && totalChars > 0) {{
        const ratio = Math.min(1, charOffset / totalChars);
        targetSpread = Math.floor(ratio * (getPageCount() - 1));
      }}
      goToSpread(targetSpread);
    }}
    preloadChapter(currentChapter - 1);
    preloadChapter(currentChapter + 1);
    restoreSearchAfterLayout();
  }} catch (e) {{
    columnContent.innerHTML = '<p>章节加载失败: ' + e + '</p>';
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
  const rect = columnView.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width / 2) {{
    prevPage();
  }} else {{
    nextPage();
  }}
}}

if (columnView) {{
  columnView.addEventListener('wheel', onWheel, {{ passive: false }});
  columnView.addEventListener('click', onClick);
  columnView.addEventListener('scroll', reportPosition, {{ passive: true }});
}}

let resizeTimeout = null;
window.addEventListener('resize', () => {{
  clearTimeout(resizeTimeout);
  resizeTimeout = setTimeout(() => {{
    if (!currentChapterHtml) return;
    const viewportW = document.body.clientWidth;
    const viewportH = columnView ? columnView.clientHeight : 0;
    const mode = isScrollMode() ? 'scroll' : (isDoubleMode() ? 'double' : 'single');
    const sameWidth = viewportW === lastLayoutParams.width;
    const sameMode = mode === lastLayoutParams.mode;

    // If width and mode are unchanged, the existing layout geometry is still
    // valid. In scroll mode the browser handles height changes itself; in
    // paginated mode only the total page count can change with height.
    if (sameWidth && sameMode) {{
      if (mode === 'scroll') return;
      if (viewportH === lastLayoutParams.height) return;
      recomputeTotalPages();
      goToSpread(paginatorState.currentSpread);
      restoreSearchAfterLayout();
      return;
    }}

    let savedScrollRatio = 0;
    if (isScrollMode()) {{
      const maxScrollVal = maxScroll();
      savedScrollRatio = maxScrollVal > 0 ? columnView.scrollTop / maxScrollVal : 0;
    }}
    layout();
    if (isScrollMode()) {{
      columnView.scrollTop = Math.floor(savedScrollRatio * maxScroll());
    }} else {{
      goToSpread(paginatorState.currentSpread);
    }}
    restoreSearchAfterLayout();
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
    use openitgo_storage::models::EbookSettings;

    #[test]
    fn test_reader_html_contains_column_containers() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("id=\"column-view\""));
        assert!(html.contains("id=\"column-content\""));
        assert!(!html.contains("id=\"measure\""));
        assert!(!html.contains("id=\"spread\""));
        assert!(!html.contains("id=\"flipper\""));
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
    fn test_reader_html_uses_css_columns_paginator() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(html.contains("function layout()"));
        assert!(html.contains("function applyTransform()"));
        assert!(html.contains("function goToSpread("));
        assert!(html.contains("function goToPage("));
        assert!(html.contains("function getPageCount()"));
        assert!(html.contains("function computeCharOffset()"));
        assert!(html.contains("function goToCharOffset("));
        assert!(html.contains("column-fill: auto"));
        assert!(html.contains("break-inside: avoid"));
    }

    #[test]
    fn test_reader_html_no_old_paginator_code() {
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(!html.contains("function collectLineBoxes"));
        assert!(!html.contains("function findSafeEnd"));
        assert!(!html.contains("function blockAncestor"));
        assert!(!html.contains("function ancestorLi"));
        assert!(!html.contains("function buildClonedSpread"));
        assert!(!html.contains("function buildDoubleSpread"));
        assert!(!html.contains("function splitSinglePage"));
        assert!(!html.contains("function splitDoublePage"));
        assert!(!html.contains("function splitIntoSpreads"));
        assert!(!html.contains("spreadElementCache"));
        assert!(!html.contains("function flipToSpread"));
        assert!(!html.contains("function captureSpreadElement"));
        assert!(!html.contains("function cancelFlip"));
        assert!(!html.contains("function renderSpread"));
        assert!(!html.contains("function findSpreadForOffset"));
        assert!(!html.contains("function scrollToOffset"));
        assert!(!html.contains("function preloadAdjacent"));
        assert!(!html.contains("window.ebookUseColumns"));
        assert!(!html.contains("function isColumnMode()"));
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
    fn test_reader_html_contains_chapter_count() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 5);
        assert!(html.contains("window.ebookChapterCount = 5"));
    }

    #[test]
    fn test_reader_html_contains_column_css_rules() {
        use openitgo_storage::models::EbookSettings;
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
    fn test_reader_html_layout_guards_non_positive_widths() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
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
            fn_body.contains("paginatorState.totalPages = Math.max(1,"),
            "totalPages should always be at least 1"
        );
    }

    #[test]
    fn test_reader_html_layout_guards_zero_viewport() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
            .next()
            .expect("layout end not found");
        assert!(
            fn_body.contains("const viewportW = document.body.clientWidth;"),
            "layout should read the viewport width"
        );
        assert!(
            fn_body.contains("const viewportH = columnView.clientHeight;"),
            "layout should read the viewport height"
        );
        assert!(
            fn_body.contains("if (viewportW === 0 || viewportH === 0)"),
            "layout should guard against a zero-size viewport"
        );
        assert!(
            fn_body.contains("paginatorState.totalPages = 1;"),
            "layout should keep totalPages valid when the viewport is zero"
        );
        assert!(
            fn_body.contains("reportPosition();"),
            "layout should report position even when the viewport is zero"
        );
    }

    #[test]
    fn test_reader_html_layout_covers_single_and_double_branches() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("if (isDoubleMode()) {"),
            "layout should branch for double-page mode"
        );
        assert!(
            fn_body.contains("} else {"),
            "layout should have a fallback single-page branch"
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
    fn test_reader_html_layout_does_not_auto_navigate() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
            .next()
            .unwrap();
        assert!(
            !fn_body.contains("goToPage("),
            "layout should only measure layout, not navigate"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_preserves_scroll_offset() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("columnView.scrollTop / maxScrollVal"),
            "applySettings should compute scroll ratio using maxScroll as the denominator"
        );
        assert!(
            fn_body.contains("Math.floor(totalChars * Math.max(0, Math.min(1, ratio)))"),
            "applySettings should convert scroll ratio to a character offset"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_layout_before_target_page() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let apply_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        let layout_pos = apply_body
            .find("layout()")
            .expect("layout call not found in applySettings");
        let target_pos = apply_body
            .find("targetSpread = Math.floor(ratio * (getPageCount() - 1));")
            .expect("targetSpread computation not found in applySettings");
        assert!(
            layout_pos < target_pos,
            "applySettings must call layout before computing targetSpread from getPageCount()"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_layout_before_target_page() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let load_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        let layout_pos = load_body
            .find("layout()")
            .expect("layout call not found in loadChapter");
        let target_pos = load_body
            .find("targetSpread = Math.floor(ratio * (getPageCount() - 1));")
            .expect("targetSpread computation not found in loadChapter");
        assert!(
            layout_pos < target_pos,
            "loadChapter must call layout before computing targetSpread from getPageCount()"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_uses_text_content_length_for_ratio() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let load_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function reportPosition()")
            .next()
            .unwrap();
        assert!(
            load_body.contains("const totalChars = columnContent.textContent.length;"),
            "loadChapter should measure total characters from columnContent"
        );
        assert!(
            load_body.contains("const ratio = Math.min(1, charOffset / totalChars);"),
            "loadChapter should compute the character ratio"
        );
    }

    #[test]
    fn test_reader_html_double_page_layout_matches_prototype() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
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
    fn test_reader_html_double_page_transform_step() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
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
            .split("function applyTransform()")
            .nth(1)
            .expect("applyTransform not found")
            .split("function goToSpread")
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
            double_transform.contains("paginatorState.currentSpread * paginatorState.viewShift"),
            "double-page transform should use currentSpread * viewShift"
        );
        assert!(
            double_transform
                .contains("columnContent.style.transform = `translateX(-${offset}px)`;"),
            "double-page transform should translateX by negative offset"
        );
    }

    #[test]
    fn test_reader_html_transform_targets_content_not_view() {
        // #column-view 是 click/wheel 事件监听容器（onClick 还依赖它的
        // getBoundingClientRect 划分左右翻页区）。对它做 translateX 会把事件
        // 接收区域整体移出视口，表现为每章第二页起无法翻页；位移必须作用于
        // 内层 #column-content。
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            !html.contains("columnView.style.transform"),
            "transform must never be applied to #column-view (the click/wheel event container)"
        );
        assert!(
            !html.contains("columnView.classList"),
            "the column-animate class must be toggled on #column-content, not #column-view"
        );
        let transform_body = html
            .split("function applyTransform()")
            .nth(1)
            .expect("applyTransform not found")
            .split("function goToSpread")
            .next()
            .unwrap();
        assert!(
            transform_body.contains("columnContent.style.transform"),
            "applyTransform should translate the inner #column-content"
        );
    }

    #[test]
    fn test_reader_html_double_page_total_spread_formula() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let layout_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function applyTransform")
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
    fn test_reader_html_single_page_column_formulas() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 3);
        assert!(
            html.contains("columnContent.style.width = colW + 'px'"),
            "single-page column-content width should be colW"
        );
        assert!(
            html.contains("columnContent.style.columnWidth = colW + 'px'"),
            "single-page column-width should equal colW"
        );
        assert!(
            html.contains("columnContent.style.columnGap = (2 * marginH) + 'px'"),
            "single-page column-gap should be 2 * marginH"
        );
        assert!(
            html.contains("columnContent.style.columnCount = 'auto'"),
            "single-page column-count should be auto"
        );
        assert!(
            html.contains("Math.max(1, Math.ceil(scrollW / pageW))"),
            "single-page total pages should be ceil(scrollW / viewportW)"
        );
        assert!(
            html.contains("translateX(${-offset + marginH}px)"),
            "single-page transform should be translateX(-currentPage * viewportW + marginH)"
        );
    }

    #[test]
    fn test_reader_html_goto_page_maps_page_index_to_spread() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function goToPage(pageIndex)")
            .nth(1)
            .expect("goToPage not found")
            .split("function nextPage")
            .next()
            .unwrap();
        assert!(
            fn_body
                .contains("const spread = isDoubleMode() ? Math.floor(pageIndex / 2) : pageIndex;"),
            "goToPage should map a page index to the containing spread"
        );
        assert!(
            fn_body.contains("goToSpread(spread);"),
            "goToPage should delegate to goToSpread"
        );
    }

    #[test]
    fn test_reader_html_report_position_uses_spread() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function reportPosition()")
            .nth(1)
            .expect("reportPosition not found")
            .split("function getActiveChapterRoot")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("let total = getPageCount();"),
            "reportPosition should resolve the total spread count"
        );
        assert!(
            fn_body.contains("\"total_spreads\": total"),
            "reportPosition should report the resolved total spread count"
        );
        assert!(
            fn_body.contains("let spread = paginatorState.currentSpread;"),
            "reportPosition should start from the current spread index"
        );
        assert!(
            fn_body.contains("columnView.scrollTop / ch"),
            "reportPosition should derive the scroll page from #column-view in scroll mode"
        );
        assert!(
            fn_body.contains("\"spread\": spread,"),
            "reportPosition should report the resolved spread index"
        );
    }

    #[test]
    fn test_reader_html_scroll_mode_uses_column_view() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("function scrollStep()"),
            "scroll mode should expose a scroll-step helper"
        );
        assert!(
            html.contains("columnView.clientHeight - 2 * getMarginV()"),
            "scrollStep should derive the step from the viewport height"
        );
        let next_body = html
            .split("function nextPage()")
            .nth(1)
            .expect("nextPage not found")
            .split("function prevPage")
            .next()
            .unwrap();
        assert!(
            next_body.contains(
                "const target = Math.min(maxScrollVal, columnView.scrollTop + scrollStep());"
            ),
            "nextPage scroll mode should compute a clamped downward scroll target"
        );
        assert!(
            next_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "nextPage scroll mode should support smooth scrolling"
        );
        assert!(
            next_body.contains("columnView.scrollTop = target;"),
            "nextPage scroll mode should fall back to direct scrollTop assignment"
        );
        let prev_body = html
            .split("function prevPage()")
            .nth(1)
            .expect("prevPage not found")
            .split("function computeCharOffset")
            .next()
            .unwrap();
        assert!(
            prev_body.contains("const target = Math.max(0, columnView.scrollTop - scrollStep());"),
            "prevPage scroll mode should compute a clamped upward scroll target"
        );
        assert!(
            prev_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "prevPage scroll mode should support smooth scrolling"
        );
        assert!(
            prev_body.contains("columnView.scrollTop = target;"),
            "prevPage scroll mode should fall back to direct scrollTop assignment"
        );
        let compute_body = html
            .split("function computeCharOffset()")
            .nth(1)
            .expect("computeCharOffset not found")
            .split("function reportPosition")
            .next()
            .unwrap();
        assert!(
            compute_body.contains("columnView.scrollTop"),
            "computeCharOffset should read #column-view scroll position"
        );
        assert!(
            compute_body.contains("columnView.scrollHeight"),
            "computeCharOffset should read #column-view scroll height"
        );
        assert!(
            !compute_body.contains("columnContent.scrollTop"),
            "computeCharOffset should not read #column-content scroll position"
        );
    }

    #[test]
    fn test_reader_html_scroll_goto_page_maps_index_to_ratio() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function goToPage(pageIndex)")
            .nth(1)
            .expect("goToPage not found")
            .split("function nextPage")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("goToPage scroll branch not found")
            .split("\n  }} else {{")
            .next()
            .expect("goToPage scroll branch end not found");
        assert!(
            scroll_branch.contains("getPageCount()"),
            "goToPage scroll mode should read total scroll pages"
        );
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * maxScroll());"),
            "goToPage scroll mode should set scrollTop from the page ratio"
        );
    }

    #[test]
    fn test_reader_html_scroll_position_reporting_uses_column_view() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function reportPosition()")
            .nth(1)
            .expect("reportPosition not found")
            .split("function getActiveChapterRoot")
            .next()
            .unwrap();
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(1)
            .expect("reportPosition scroll branch not found")
            .split("\n  }}")
            .next()
            .expect("reportPosition scroll branch end not found");
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
    fn test_reader_html_scroll_apply_settings_restores_ratio() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .unwrap();
        // The first scroll branch saves the ratio; the second restores it.
        let scroll_branch = fn_body
            .split("if (isScrollMode()) {")
            .nth(2)
            .expect("applySettings restoring scroll branch not found")
            .split("\n    } else {{")
            .next()
            .expect("applySettings scroll branch end not found");
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * maxScroll());"),
            "applySettings scroll mode should restore #column-view scroll position from saved ratio"
        );
        assert!(
            scroll_branch.contains("reportPosition();"),
            "applySettings scroll mode should report position after restoring scroll"
        );
    }

    #[test]
    fn test_reader_html_scroll_load_chapter_restores_offset() {
        use openitgo_storage::models::EbookSettings;
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
            .nth(1)
            .expect("loadChapter scroll branch not found")
            .split("\n    } else {{")
            .next()
            .expect("loadChapter scroll branch end not found");
        assert!(
            scroll_branch.contains("columnView.scrollTop = Math.floor(ratio * maxScroll());"),
            "loadChapter scroll mode should scroll #column-view to the approximate offset"
        );
        assert!(
            scroll_branch.contains("reportPosition();"),
            "loadChapter scroll mode should report position after scrolling"
        );
    }

    #[test]
    fn test_reader_html_column_view_has_scroll_listener() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains(
                "columnView.addEventListener('scroll', reportPosition, { passive: true });"
            ),
            "#column-view should report position while scrolling"
        );
    }

    #[test]
    fn test_reader_html_animation_helpers_present() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("#column-content.column-animate {"),
            "column paginator should define a CSS class for animated transforms"
        );
        assert!(
            html.contains("transition: transform 0.25s ease"),
            "column animation CSS should transition the transform property"
        );
        assert!(
            html.contains("let animationTimer = null;"),
            "paginator should track a shared animation timer"
        );
        assert!(
            html.contains("function enableAnimation()"),
            "paginator should expose enableAnimation"
        );
        assert!(
            html.contains("function disableAnimation()"),
            "paginator should expose disableAnimation"
        );
        assert!(
            html.contains("function scheduleDisableAnimation()"),
            "paginator should expose scheduleDisableAnimation"
        );
        assert!(
            html.contains("columnContent.classList.add('column-animate')"),
            "enableAnimation should add the animate class to #column-content (the transformed element)"
        );
        assert!(
            html.contains("columnContent.classList.remove('column-animate')"),
            "disableAnimation should remove the animate class from #column-content"
        );
        assert!(
            html.contains("animationTimer = setTimeout"),
            "scheduleDisableAnimation should schedule a timer"
        );
        assert!(
            html.contains("clearTimeout(animationTimer)"),
            "animation helpers should cancel any pending timer"
        );
    }

    #[test]
    fn test_reader_html_animation_toggled_by_settings() {
        use openitgo_storage::models::EbookSettings;
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
                .split("function nextPage()")
                .nth(1)
                .expect("nextPage not found")
                .split("function prevPage")
                .next()
                .unwrap();
            assert!(
                next_body.contains("if (currentSettings.animate) enableAnimation();"),
                "nextPage should guard animation on currentSettings.animate"
            );
            assert!(
                next_body.contains("scheduleDisableAnimation();"),
                "nextPage should schedule disabling animation after the transition duration"
            );
        }
    }

    #[test]
    fn test_reader_html_paginated_animation_branches() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let next_body = html
            .split("function nextPage()")
            .nth(1)
            .expect("nextPage not found")
            .split("function prevPage")
            .next()
            .unwrap();
        assert!(
            next_body.contains("if (currentSettings.animate) enableAnimation();"),
            "nextPage paginated branch should enable animation before moving"
        );
        assert!(
            next_body.contains("goToSpread(paginatorState.currentSpread + 1);"),
            "nextPage paginated branch should advance one spread"
        );
        assert!(
            next_body.contains("scheduleDisableAnimation();"),
            "nextPage paginated branch should schedule disabling animation after the transition"
        );

        let prev_body = html
            .split("function prevPage()")
            .nth(1)
            .expect("prevPage not found")
            .split("function computeCharOffset")
            .next()
            .unwrap();
        assert!(
            prev_body.contains("if (currentSettings.animate) enableAnimation();"),
            "prevPage paginated branch should enable animation before moving"
        );
        assert!(
            prev_body.contains("goToSpread(paginatorState.currentSpread - 1);"),
            "prevPage paginated branch should go back one spread"
        );
        assert!(
            prev_body.contains("scheduleDisableAnimation();"),
            "prevPage paginated branch should schedule disabling animation after the transition"
        );
    }

    #[test]
    fn test_reader_html_next_crosses_chapter_boundary() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function nextPage()")
            .nth(1)
            .expect("nextPage not found")
            .split("function prevPage")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("goToSpread(paginatorState.currentSpread + 1)"),
            "nextPage should advance to the next spread within a chapter"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter + 1, 0)"),
            "nextPage should load the next chapter when on the last spread"
        );
    }

    #[test]
    fn test_reader_html_prev_crosses_chapter_boundary() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function prevPage()")
            .nth(1)
            .expect("prevPage not found")
            .split("function computeCharOffset")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("goToSpread(paginatorState.currentSpread - 1)"),
            "prevPage should go to the previous spread within a chapter"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER)"),
            "prevPage should load the previous chapter and clamp to the last spread"
        );
        assert!(
            !fn_body.contains("loadChapter(currentChapter - 1, 0)"),
            "prevPage should not load the previous chapter at offset 0"
        );
    }

    #[test]
    fn test_reader_html_scroll_next_clamps_and_crosses_chapter() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function nextPage()")
            .nth(1)
            .expect("nextPage not found")
            .split("function prevPage")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("maxScroll()"),
            "nextPage scroll mode should clamp against the maximum scroll"
        );
        assert!(
            fn_body.contains(
                "const target = Math.min(maxScrollVal, columnView.scrollTop + scrollStep());"
            ),
            "nextPage scroll mode should compute a clamped target scroll position"
        );
        assert!(
            fn_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "nextPage scroll mode should smooth-scroll when animation is enabled"
        );
        assert!(
            fn_body.contains("columnView.scrollTop = target;"),
            "nextPage scroll mode should set scrollTop directly when animation is disabled"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter + 1, 0)"),
            "nextPage scroll mode should load the next chapter at the bottom"
        );
    }

    #[test]
    fn test_reader_html_scroll_prev_clamps_and_crosses_chapter() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 2);
        let fn_body = html
            .split("function prevPage()")
            .nth(1)
            .expect("prevPage not found")
            .split("function computeCharOffset")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("const target = Math.max(0, columnView.scrollTop - scrollStep());"),
            "prevPage scroll mode should compute a clamped target scroll position"
        );
        assert!(
            fn_body.contains("columnView.scrollTo({ top: target, behavior: 'smooth' });"),
            "prevPage scroll mode should smooth-scroll when animation is enabled"
        );
        assert!(
            fn_body.contains("columnView.scrollTop = target;"),
            "prevPage scroll mode should set scrollTop directly when animation is disabled"
        );
        assert!(
            fn_body.contains("loadChapter(currentChapter - 1, Number.MAX_SAFE_INTEGER)"),
            "prevPage scroll mode should load the previous chapter at the top"
        );
    }

    #[test]
    fn test_reader_html_contains_toc_jump_and_search_helpers() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("function jumpToTocItem"),
            "jumpToTocItem should exist"
        );
        assert!(
            html.contains("function resolveTocTarget"),
            "resolveTocTarget should exist"
        );
        assert!(
            html.contains("function goToCharOffset"),
            "goToCharOffset should exist"
        );
        assert!(
            html.contains("function charOffsetOfElement"),
            "charOffsetOfElement should exist"
        );
        assert!(html.contains("function findText"), "findText should exist");
        assert!(html.contains("function findNext"), "findNext should exist");
        assert!(html.contains("function findPrev"), "findPrev should exist");
        assert!(
            html.contains("function clearHighlights"),
            "clearHighlights should exist"
        );
        assert!(
            html.contains("function setSearchActiveIndex"),
            "setSearchActiveIndex should exist"
        );
        assert!(
            html.contains(".ebook-search-highlight {"),
            "search highlight CSS should exist"
        );
        assert!(
            html.contains(".ebook-search-active {"),
            "active search highlight CSS should exist"
        );
    }

    #[test]
    fn test_reader_html_jump_to_toc_item_uses_column_paginator() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function jumpToTocItem(chapter, target)")
            .nth(1)
            .expect("jumpToTocItem not found")
            .split("function clearHighlights")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("goToCharOffset(offset)"),
            "jumpToTocItem should use goToCharOffset in paginated mode"
        );
        assert!(
            fn_body.contains(
                "el.scrollIntoView({ behavior: currentSettings.animate ? 'smooth' : 'auto', block: 'start' })"
            ),
            "jumpToTocItem should scroll the element into view in scroll mode"
        );
        assert!(
            !fn_body.contains("goToSpread(findSpreadForOffset(offset), false)"),
            "jumpToTocItem should not fall back to the old paginator"
        );
    }

    #[test]
    fn test_reader_html_resolve_toc_target_rejects_fragmentless_href() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function resolveTocTarget(target)")
            .nth(1)
            .expect("resolveTocTarget not found")
            .split("function goToCharOffset")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("idx !== -1"),
            "resolveTocTarget should detect inputs with a fragment"
        );
        assert!(
            fn_body.contains("fragment-less path/URL"),
            "resolveTocTarget should document fragment-less href behavior"
        );
        assert!(
            fn_body.contains("return null;"),
            "resolveTocTarget should bail out for non-fragment targets"
        );
        assert!(
            fn_body.contains("if (!fragment) return null;"),
            "resolveTocTarget should return null for empty fragments"
        );
    }

    #[test]
    fn test_reader_html_find_text_highlights_in_column_content() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function findText(query)")
            .nth(1)
            .expect("findText not found")
            .split("function findNext")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("clearHighlights()"),
            "findText should clear existing highlights first"
        );
        assert!(
            fn_body.contains("getActiveChapterRoot()"),
            "findText should search the active chapter root"
        );
        assert!(
            fn_body.contains("ebook-search-highlight"),
            "findText should create highlight marks"
        );
        assert!(
            fn_body.contains("setSearchActiveIndex(0)"),
            "findText should activate the first match"
        );
    }

    #[test]
    fn test_reader_html_find_next_prev_cycles_active_highlight() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let next_body = html
            .split("function findNext()")
            .nth(1)
            .expect("findNext not found")
            .split("function findPrev")
            .next()
            .unwrap();
        assert!(
            next_body.contains("setSearchActiveIndex(ebookSearchActiveIndex + 1)"),
            "findNext should advance the active highlight"
        );
        let prev_body = html
            .split("function findPrev()")
            .nth(1)
            .expect("findPrev not found")
            .split("function getMarginH")
            .next()
            .unwrap();
        assert!(
            prev_body.contains("setSearchActiveIndex(ebookSearchActiveIndex - 1)"),
            "findPrev should move the active highlight backwards"
        );
    }

    #[test]
    fn test_reader_html_search_navigation_uses_column_paginator() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function setSearchActiveIndex(index)")
            .nth(1)
            .expect("setSearchActiveIndex not found")
            .split("function findText")
            .next()
            .unwrap();
        assert!(
            fn_body.contains("goToCharOffset(offset)"),
            "setSearchActiveIndex should jump to the match spread in paginated mode"
        );
        assert!(
            fn_body.contains(
                "mark.scrollIntoView({ behavior: currentSettings.animate ? 'smooth' : 'auto', block: 'center' })"
            ),
            "setSearchActiveIndex should scroll matches into view in scroll mode"
        );
    }

    #[test]
    fn test_reader_html_break_inside_avoid_on_elements() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("#column-content img, #column-content table, #column-content figure, #column-content pre, #column-content blockquote"),
            "column content should target images, tables, figures, pre and blockquotes"
        );
        assert!(
            html.contains("break-inside: avoid"),
            "column content should prevent breaks inside figures, tables and blocks"
        );
    }

    #[test]
    fn test_reader_html_headings_lists_paragraphs_styled_consistently() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let css = html
            .split("<style>")
            .nth(1)
            .expect("style block not found")
            .split("</style>")
            .next()
            .expect("style block end not found");
        assert!(
            css.contains("h1, h2, h3, h4, h5, h6"),
            "headings should share a common rule"
        );
        assert!(css.contains("ul, ol"), "lists should share a common rule");
        assert!(
            css.contains("p {"),
            "paragraphs should be explicitly styled"
        );
        assert!(
            css.contains("text-indent: 2em"),
            "paragraphs should have consistent indentation"
        );
        assert!(
            css.contains("margin: 0.25em 0"),
            "list items should have consistent vertical spacing"
        );
    }

    #[test]
    fn test_reader_html_image_constraints() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let global_img = html
            .split("img {")
            .nth(1)
            .expect("global img rule not found")
            .split('}')
            .next()
            .expect("global img rule end not found");
        assert!(
            global_img.contains("max-width: 100%"),
            "global img should constrain width"
        );
        assert!(
            global_img.contains("max-height:"),
            "global img should constrain height"
        );
        let column_img = html
            .split("#column-content img,")
            .nth(1)
            .expect("column img rule not found")
            .split('}')
            .next()
            .expect("column img rule end not found");
        assert!(
            column_img.contains("max-width: 100%"),
            "column img should constrain width"
        );
        assert!(
            column_img.contains("max-height:"),
            "column img should constrain height inside the column"
        );
    }

    #[test]
    fn test_reader_html_compute_char_offset_returns_zero_for_empty_chapter() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function computeCharOffset()")
            .nth(1)
            .expect("computeCharOffset not found")
            .split("function reportPosition")
            .next()
            .expect("computeCharOffset end not found");
        assert!(
            fn_body.contains("const root = columnContent;"),
            "computeCharOffset should read from columnContent safely"
        );
        assert!(
            fn_body.contains("const total = root ? root.textContent.length : 0;"),
            "computeCharOffset should treat a missing root as zero-length"
        );
        assert!(
            fn_body.contains("if (total === 0) return 0;"),
            "computeCharOffset should return 0 for empty chapters"
        );
    }

    #[test]
    fn test_reader_html_go_to_char_offset_clamps_ratio() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function goToCharOffset(offset)")
            .nth(1)
            .expect("goToCharOffset not found")
            .split("async function jumpToTocItem")
            .next()
            .expect("goToCharOffset end not found");
        assert!(
            fn_body.contains("const ratio = Math.max(0, Math.min(1, offset / totalChars));"),
            "goToCharOffset should clamp the ratio to [0, 1]"
        );
        assert!(
            fn_body.contains("!Number.isFinite(offset)"),
            "goToCharOffset should reject non-finite offsets"
        );
    }

    #[test]
    fn test_reader_html_resize_handler_preserves_scroll_ratio() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let handler_body = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("\n  }});")
            .next()
            .expect("resize handler end not found");
        assert!(
            handler_body.contains("if (!currentChapterHtml) return;"),
            "resize handler should guard on missing chapter content"
        );
        assert!(
            handler_body.contains(
                "savedScrollRatio = maxScrollVal > 0 ? columnView.scrollTop / maxScrollVal : 0;"
            ),
            "resize handler should capture the scroll ratio in scroll mode"
        );
        assert!(
            handler_body
                .contains("columnView.scrollTop = Math.floor(savedScrollRatio * maxScroll());"),
            "resize handler should restore the scroll ratio in scroll mode"
        );
        assert!(
            handler_body.contains("goToSpread(paginatorState.currentSpread);"),
            "resize handler should re-navigate to the current spread in paginated mode"
        );
    }

    #[test]
    fn test_reader_html_layout_cache_exists() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("let layoutCache ="),
            "template should declare a layout cache"
        );
        assert!(
            html.contains("function makeLayoutKey()"),
            "template should expose a cache key builder"
        );
        assert!(
            html.contains("function hashString(s)"),
            "template should hash the chapter HTML for the cache key"
        );
        assert!(
            html.contains("let lastLayoutParams ="),
            "template should track the last layout dimensions and mode"
        );
    }

    #[test]
    fn test_reader_html_layout_uses_cache_hit() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function layout()")
            .nth(1)
            .expect("layout not found")
            .split("function recomputeTotalPages")
            .next()
            .expect("layout end not found");
        assert!(
            fn_body.contains("const cacheKey = makeLayoutKey();"),
            "layout should compute a cache key"
        );
        assert!(
            fn_body.contains("if (layoutCache.key === cacheKey)"),
            "layout should check the cache before reflowing"
        );
        assert!(
            fn_body.contains("paginatorState.totalPages = layoutCache.totalPages;"),
            "layout should restore cached totalPages on a hit"
        );
        assert!(
            fn_body.contains("paginatorState.pageWidth = layoutCache.pageWidth;"),
            "layout should restore cached pageWidth on a hit"
        );
        assert!(
            fn_body.contains("paginatorState.viewShift = layoutCache.viewShift;"),
            "layout should restore cached viewShift on a hit"
        );
        assert!(
            fn_body.contains("layoutCache = {"),
            "layout should store results in the cache after measuring"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_invalidates_layout_cache() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .expect("applySettings end not found");
        assert!(
            fn_body.contains("layoutCache.key = null;"),
            "applySettings should invalidate the layout cache when settings change"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_invalidates_layout_cache() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter end not found");
        assert!(
            fn_body.contains("layoutCache.key = null;"),
            "loadChapter should invalidate the layout cache for a new chapter"
        );
    }

    #[test]
    fn test_reader_html_resize_handler_skips_unchanged_layout() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let handler_body = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("\n  }});")
            .next()
            .expect("resize handler end not found");
        assert!(
            handler_body.contains("const sameWidth = viewportW === lastLayoutParams.width;"),
            "resize handler should compare width to the last layout"
        );
        assert!(
            handler_body.contains("const sameMode = mode === lastLayoutParams.mode;"),
            "resize handler should compare mode to the last layout"
        );
        assert!(
            handler_body.contains("if (sameWidth && sameMode)"),
            "resize handler should skip layout when width and mode are unchanged"
        );
        assert!(
            handler_body.contains("if (mode === 'scroll') return;"),
            "resize handler should skip layout for height-only changes in scroll mode"
        );
    }

    #[test]
    fn test_reader_html_resize_handler_recomputes_pages_on_height_change() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let handler_body = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("\n  }});")
            .next()
            .expect("resize handler end not found");
        assert!(
            handler_body.contains("if (viewportH === lastLayoutParams.height) return;"),
            "resize handler should detect height-only changes"
        );
        assert!(
            handler_body.contains("recomputeTotalPages();"),
            "resize handler should recompute total pages without a full reflow"
        );
        assert!(
            handler_body.contains("goToSpread(paginatorState.currentSpread);"),
            "resize handler should preserve the current spread after recomputing pages"
        );
    }

    #[test]
    fn test_reader_html_recompute_total_pages_preserves_geometry() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function recomputeTotalPages()")
            .nth(1)
            .expect("recomputeTotalPages not found")
            .split("function applyTransform")
            .next()
            .expect("recomputeTotalPages end not found");
        assert!(
            fn_body.contains("const scrollW = columnContent.scrollWidth;"),
            "recomputeTotalPages should read the current scroll width"
        );
        assert!(
            fn_body.contains("paginatorState.viewShift"),
            "recomputeTotalPages should reuse the cached viewShift"
        );
        assert!(
            fn_body.contains("paginatorState.pageWidth"),
            "recomputeTotalPages should reuse the cached pageWidth"
        );
    }

    #[test]
    fn test_reader_html_has_preload_chapter() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 3);
        assert!(
            html.contains("async function preloadChapter(index)"),
            "template should expose preloadChapter"
        );
        let load_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter end not found");
        assert!(
            load_body.contains("preloadChapter(currentChapter - 1);"),
            "loadChapter should preload the previous chapter"
        );
        assert!(
            load_body.contains("preloadChapter(currentChapter + 1);"),
            "loadChapter should preload the next chapter"
        );
    }

    #[test]
    fn test_reader_html_preload_chapter_uses_inert_template() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function preloadChapter(index)")
            .nth(1)
            .expect("preloadChapter not found")
            .split("async function loadChapter")
            .next()
            .expect("preloadChapter end not found");
        assert!(
            fn_body.contains("document.createElement('template')"),
            "preloadChapter should parse into an inert template"
        );
        assert!(
            fn_body.contains("ebook://reader?chapter="),
            "preloadChapter should fetch from the ebook protocol"
        );
        assert!(
            !fn_body.contains("columnContent.innerHTML = html"),
            "preloadChapter must not replace the visible chapter content"
        );
    }

    #[test]
    fn test_reader_html_layout_key_includes_font() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function makeLayoutKey()")
            .nth(1)
            .expect("makeLayoutKey not found")
            .split("function escapeHtml")
            .next()
            .expect("makeLayoutKey end not found");
        assert!(
            fn_body.contains("getPropertyValue('--font')"),
            "makeLayoutKey should read the --font CSS variable"
        );
        assert!(
            fn_body.contains("const font"),
            "makeLayoutKey should store the font value"
        );
        assert!(
            fn_body.contains("font,"),
            "makeLayoutKey should include the font in the returned key array"
        );
    }

    #[test]
    fn test_reader_html_preload_cleanup_exists() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 5);
        assert!(
            html.contains("function cleanupPreloaded(keepCenter)"),
            "template should expose cleanupPreloaded"
        );
        let fn_body = html
            .split("function cleanupPreloaded(keepCenter)")
            .nth(1)
            .expect("cleanupPreloaded not found")
            .split("async function loadChapter")
            .next()
            .expect("cleanupPreloaded end not found");
        assert!(
            fn_body.contains("template[id^=\"preload-chapter-\"]"),
            "cleanupPreloaded should select preloaded chapter templates"
        );
        assert!(
            fn_body.contains("keepCenter - 1"),
            "cleanupPreloaded should keep the previous chapter"
        );
        assert!(
            fn_body.contains("keepCenter + 1"),
            "cleanupPreloaded should keep the next chapter"
        );
        assert!(
            fn_body.contains("template.remove();"),
            "cleanupPreloaded should remove stale templates"
        );
        assert!(
            fn_body.contains("Number.isFinite(keepCenter)"),
            "cleanupPreloaded should guard keepCenter as a finite number"
        );
        assert!(
            fn_body.contains("window.ebookChapterCount - 1"),
            "cleanupPreloaded should respect the chapter count bounds"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_calls_cleanup() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter end not found");
        let assign_pos = fn_body
            .find("currentChapter = index;")
            .expect("currentChapter assignment not found in loadChapter");
        let cleanup_pos = fn_body
            .find("cleanupPreloaded(currentChapter);")
            .expect("cleanupPreloaded call not found in loadChapter");
        assert!(
            assign_pos < cleanup_pos,
            "loadChapter must set currentChapter before calling cleanupPreloaded"
        );
    }

    #[test]
    fn test_reader_html_show_error_uses_overlay() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function showError(title, detail)")
            .nth(1)
            .expect("showError not found")
            .split("function hideError")
            .next()
            .expect("showError end not found");
        assert!(
            html.contains("id=\"ebook-error-layer\""),
            "reader shell should declare a dedicated error overlay"
        );
        assert!(
            fn_body.contains("errorLayer.style.display = 'block';"),
            "showError should make the overlay visible"
        );
        assert!(
            fn_body.contains("errorLayer.innerHTML ="),
            "showError should write into the overlay"
        );
        assert!(
            !fn_body.contains("columnView.innerHTML"),
            "showError must not replace the column view content"
        );
    }

    #[test]
    fn test_reader_html_hide_error_defined() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        assert!(
            html.contains("function hideError()"),
            "template should expose a hideError helper"
        );
        let fn_body = html
            .split("function hideError()")
            .nth(1)
            .expect("hideError not found")
            .split("// Prevent anchors")
            .next()
            .expect("hideError end not found");
        assert!(
            fn_body.contains("errorLayer.style.display = 'none';"),
            "hideError should hide the overlay"
        );
        assert!(
            fn_body.contains("errorLayer.innerHTML = '';"),
            "hideError should clear the overlay content"
        );
    }

    #[test]
    fn test_reader_html_load_chapter_hides_error() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter end not found");
        assert!(
            fn_body.contains("columnContent.innerHTML = currentChapterHtml;"),
            "loadChapter should set the chapter content"
        );
        let content_pos = fn_body
            .find("columnContent.innerHTML = currentChapterHtml;")
            .expect("content assignment not found");
        let hide_pos = fn_body
            .find("hideError();")
            .expect("hideError call not found in loadChapter");
        assert!(
            content_pos < hide_pos,
            "loadChapter should hide the error overlay after a successful render"
        );
    }

    #[test]
    fn test_reader_html_apply_settings_hides_error() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("function loadChapter")
            .next()
            .expect("applySettings end not found");
        let content_pos = fn_body
            .find("columnContent.innerHTML = currentChapterHtml;")
            .expect("content assignment not found in applySettings");
        let hide_pos = fn_body
            .find("hideError();")
            .expect("hideError call not found in applySettings");
        assert!(
            content_pos < hide_pos,
            "applySettings should hide the error overlay after a successful render"
        );
    }

    #[test]
    fn test_reader_html_resolve_toc_target_keeps_fragment_with_dot() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function resolveTocTarget(target)")
            .nth(1)
            .expect("resolveTocTarget not found")
            .split("function goToCharOffset")
            .next()
            .expect("resolveTocTarget end not found");
        assert!(
            !fn_body.contains("/[\\\\/\\\\.]/"),
            "resolveTocTarget must not treat every dot as a path separator"
        );
        assert!(
            fn_body.contains("getElementById(fragment)"),
            "resolveTocTarget should look up the fragment as an element id"
        );
    }

    #[test]
    fn test_reader_html_resolve_toc_target_uses_css_escape() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 1);
        let fn_body = html
            .split("function resolveTocTarget(target)")
            .nth(1)
            .expect("resolveTocTarget not found")
            .split("function goToCharOffset")
            .next()
            .expect("resolveTocTarget end not found");
        assert!(
            fn_body.contains("CSS.escape(fragment)"),
            "resolveTocTarget should escape the fallback selector with CSS.escape"
        );
        assert!(
            fn_body.contains("querySelector('#' + CSS.escape(fragment))"),
            "resolveTocTarget should build a safe id selector"
        );
    }

    #[test]
    fn test_search_reports_ipc_and_replays_after_relayout() {
        let html = reader_html(&EbookSettings::default(), 3);

        let set_active = html
            .split("function setSearchActiveIndex(index)")
            .nth(1)
            .expect("setSearchActiveIndex not found")
            .split("function findText")
            .next()
            .expect("setSearchActiveIndex body not found");
        assert!(
            set_active.contains("type: 'search'"),
            "setSearchActiveIndex should report search state via IPC"
        );

        let find_text = html
            .split("function findText(query)")
            .nth(1)
            .expect("findText not found")
            .split("function findNext")
            .next()
            .expect("findText body not found");
        assert!(
            find_text.contains("count: 0, active: -1"),
            "findText should report zero matches via IPC"
        );
        assert_eq!(
            find_text.matches("count: 0, active: -1").count(),
            2,
            "findText should reset the count both on empty query and on no matches"
        );

        assert!(
            html.contains("function restoreSearchAfterLayout"),
            "restoreSearchAfterLayout should exist"
        );

        let apply_settings = html
            .split("function applySettings(json)")
            .nth(1)
            .expect("applySettings not found")
            .split("// Lightweight adjacent-chapter preload")
            .next()
            .expect("applySettings body not found");
        assert!(
            apply_settings.contains("restoreSearchAfterLayout()"),
            "applySettings should replay search after relayout"
        );

        let load_chapter = html
            .split("async function loadChapter(index, charOffset)")
            .nth(1)
            .expect("loadChapter not found")
            .split("function onWheel")
            .next()
            .expect("loadChapter body not found");
        assert!(
            load_chapter.contains("restoreSearchAfterLayout()"),
            "loadChapter should replay search in the new chapter"
        );

        let resize = html
            .split("window.addEventListener('resize', () => {")
            .nth(1)
            .expect("resize handler not found")
            .split("loadChapter(0, 0);")
            .next()
            .expect("resize handler end not found");
        assert!(
            resize.contains("restoreSearchAfterLayout()"),
            "resize handler should replay search after relayout"
        );
    }
}
