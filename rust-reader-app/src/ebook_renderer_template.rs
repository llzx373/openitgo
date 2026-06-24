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
#content {{
  width: 100%;
  height: 100%;
  padding: var(--margin-v) var(--margin-h);
  box-sizing: border-box;
}}
body.paginated #content {{
  column-width: calc(100vw - var(--margin-h) * 2);
  column-gap: 0;
  column-fill: auto;
  overflow: hidden;
  scroll-snap-type: x mandatory;
  scroll-behavior: smooth;
}}
body.paginated.no-anim #content {{
  scroll-behavior: auto;
}}
body.double #content {{
  column-width: calc((100vw - var(--margin-h) * 2) / 2);
}}
body.scroll #content {{
  overflow-y: auto;
}}
body.paginated #content > *,
body.paginated #content img {{
  break-inside: avoid;
}}
p {{ margin: 0 0 1em 0; text-indent: 2em; }}
img {{ max-width: 100%; height: auto; }}
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
</style>
</head>
<body class="{mode}">
<div id="content"></div>
<div id="measure"></div>
<div id="spread"></div>
<div id="flipper"></div>
<script>
const content = document.getElementById('content');
const measure = document.getElementById('measure');
const spread = document.getElementById('spread');
const flipper = document.getElementById('flipper');
let currentChapter = 0;
let currentSpread = 0;
let spreads = [];
let currentChapterHtml = '';
window.ebookChapterCount = {chapter_count};
let isFlipping = false;
let pendingFlipTarget = null;
let currentSettings = {{
  mode: '{mode}',
  animate: {animate},
  invert_scroll: {invert_scroll}
}};

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

function pageHeight() {{
  return measure.clientHeight;
}}

function splitSinglePage(html) {{
  measure.innerHTML = html;
  const ph = pageHeight();
  if (!ph || ph <= 0) {{
    measure.innerHTML = '';
    return [html];
  }}
  const totalHeight = measure.scrollHeight;
  const spreads = [];
  for (let y = 0; y < totalHeight; y += ph) {{
    const clone = measure.cloneNode(true);
    clone.removeAttribute('id');
    const wrapper = document.createElement('div');
    wrapper.style.position = 'relative';
    wrapper.style.overflow = 'hidden';
    wrapper.style.height = ph + 'px';
    const inner = clone;
    inner.style.position = 'absolute';
    inner.style.top = -y + 'px';
    inner.style.width = '100%';
    wrapper.appendChild(inner);
    spreads.push(wrapper.outerHTML);
  }}
  measure.innerHTML = '';
  return spreads;
}}

function splitDoublePage(html) {{
  const originalWidth = measure.style.width;
  measure.style.width = '50%';
  measure.innerHTML = html;
  const ph = pageHeight();
  if (!ph || ph <= 0) {{
    measure.innerHTML = '';
    measure.style.width = originalWidth;
    return [html];
  }}
  const totalHeight = measure.scrollHeight;
  const spreads = [];
  for (let y = 0; y < totalHeight; y += ph * 2) {{
    const wrapper = document.createElement('div');
    wrapper.style.display = 'flex';
    wrapper.style.width = '100%';
    wrapper.style.height = ph + 'px';
    wrapper.style.overflow = 'hidden';
    for (let col = 0; col < 2; col++) {{
      const pageY = y + col * ph;
      if (pageY >= totalHeight) break;
      const cell = document.createElement('div');
      cell.style.flex = '1';
      cell.style.height = ph + 'px';
      cell.style.overflow = 'hidden';
      cell.style.position = 'relative';
      const clone = measure.cloneNode(true);
      clone.removeAttribute('id');
      clone.style.position = 'absolute';
      clone.style.top = -pageY + 'px';
      clone.style.width = '100%';
      cell.appendChild(clone);
      wrapper.appendChild(cell);
    }}
    spreads.push(wrapper.outerHTML);
  }}
  measure.innerHTML = '';
  measure.style.width = originalWidth;
  return spreads;
}}

function splitIntoSpreads(html) {{
  if (isScrollMode()) return [html];
  if (isDoubleMode()) return splitDoublePage(html);
  return splitSinglePage(html);
}}

function goToSpread(index, animate) {{
  if (spreads.length === 0) return;
  const target = Math.max(0, Math.min(spreads.length - 1, index));
  if (target === currentSpread) {{
    renderSpread(target);
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
  content.style.display = 'none';
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
  const walk = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, null);
  while (walk.nextNode()) textNodes.push(walk.currentNode);
  let count = 0;
  for (const node of textNodes) {{
    if (count + node.length >= offset) {{
      const range = document.createRange();
      range.setStart(node, offset - count);
      const rect = range.getBoundingClientRect();
      content.scrollTop = rect.top + content.scrollTop - content.getBoundingClientRect().top;
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

function applySettings(json) {{
  const s = typeof json === 'string' ? JSON.parse(json) : json;
  currentSettings = s;
  const root = document.documentElement;
  root.style.setProperty('--bg', s.bg);
  root.style.setProperty('--fg', s.fg);
  root.style.setProperty('--font', s.font);
  root.style.setProperty('--size', s.size + 'px');
  root.style.setProperty('--line', s.line);
  root.style.setProperty('--margin-h', s.margin_h + 'px');
  root.style.setProperty('--margin-v', s.margin_v + 'px');
  document.body.className = s.mode;
  if (s.animate) {{
    document.body.classList.remove('no-anim');
  }} else {{
    document.body.classList.add('no-anim');
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
    if (isScrollMode()) {{
      content.innerHTML = currentChapterHtml;
      content.style.display = 'block';
      spread.style.display = 'none';
      if (charOffset) {{
        scrollToOffset(charOffset);
      }}
      spreads = [];
      currentSpread = 0;
      reportPosition();
    }} else {{
      spreads = splitIntoSpreads(currentChapterHtml);
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
  let offset = 0;
  if (!isScrollMode() && spreads.length > 0 && currentSpread < spreads.length) {{
    // Approximate character offset by summing text lengths of preceding spreads.
    for (let i = 0; i < currentSpread; i++) {{
      offset += textLength(spreads[i]);
    }}
    sendIpc({{
      type: 'position',
      chapter: currentChapter,
      spread: currentSpread,
      char_offset: offset,
      total_spreads: spreads.length
    }});
  }} else {{
    // Scroll mode fallback: use #content's visible text start.
    const rect = content.getBoundingClientRect();
    let offset = 0;
    const textNodes = [];
    const walk = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, null);
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
      type: 'position',
      chapter: currentChapter,
      spread: 0,
      char_offset: offset,
      total_spreads: 1
    }});
  }}
}}

function textLength(html) {{
  const div = document.createElement('div');
  div.innerHTML = html;
  return div.textContent.length;
}}

function nextPage() {{
  if (isScrollMode()) {{
    content.scrollTop += content.clientHeight * 0.9;
    return;
  }}
  if (currentSpread + 1 < spreads.length) {{
    goToSpread(currentSpread + 1, true);
  }} else if (currentChapter + 1 < window.ebookChapterCount) {{
    loadChapter(currentChapter + 1, 0);
  }}
}}

function prevPage() {{
  if (isScrollMode()) {{
    content.scrollTop -= content.clientHeight * 0.9;
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
  const rect = spread.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width / 2) {{
    prevPage();
  }} else {{
    nextPage();
  }}
}}

spread.addEventListener('wheel', onWheel, {{ passive: false }});
spread.addEventListener('click', onClick);
window.addEventListener('scroll', reportPosition, true);
window.addEventListener('resize', reportPosition);
applySettings({settings_json});
loadChapter(0, 0);
sendIpc({{ type: 'ready' }});
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
}
