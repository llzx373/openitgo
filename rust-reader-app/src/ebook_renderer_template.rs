//! HTML/JS shell template for the ebook webview renderer.

use super::JsSettings;
use rust_reader_storage::models::EbookSettings;

pub fn reader_html(settings: &EbookSettings) -> String {
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
const flipper = document.getElementById('flipper');
let currentChapter = 0;
let currentOffset = 0;
let isFlipping = false;
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

// Scaffolding for spread pagination (Tasks 5+).
function splitIntoSpreads() {{
  return [];
}}

// Scaffolding for spread pagination (Tasks 5+).
function goToSpread(index) {{
}}

function isPaginated() {{
  return document.body.classList.contains('paginated') || document.body.classList.contains('double');
}}

function pageWidth() {{
  if (document.body.classList.contains('double')) return content.clientWidth / 2;
  return content.clientWidth;
}}

function totalPages() {{
  if (!isPaginated()) return 1;
  const pw = pageWidth();
  if (pw <= 0) return 1;
  return Math.max(1, Math.round(content.scrollWidth / pw));
}}

function currentPage() {{
  if (!isPaginated()) return 0;
  const pw = pageWidth();
  if (pw <= 0) return 0;
  return Math.max(0, Math.min(totalPages() - 1, Math.round(content.scrollLeft / pw)));
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

async function loadChapter(index, offset) {{
  currentChapter = index || 0;
  currentOffset = offset || 0;
  try {{
    const res = await fetch('ebook://reader?chapter=' + currentChapter);
    const html = await res.text();
    content.innerHTML = html;
    if (offset) {{
      scrollToOffset(offset);
    }} else {{
      content.scrollLeft = 0;
    }}
    reportPosition();
  }} catch (e) {{
    content.innerHTML = '<p>章节加载失败: ' + e + '</p>';
  }}
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
      if (isPaginated()) {{
        content.scrollLeft = rect.left + content.scrollLeft - content.getBoundingClientRect().left;
      }} else {{
        content.scrollTop = rect.top + content.scrollTop - content.getBoundingClientRect().top;
      }}
      break;
    }}
    count += node.length;
  }}
}}

function reportPosition() {{
  if (isPaginated()) {{
    const page = currentPage();
    const pw = pageWidth();
    const rect = content.getBoundingClientRect();
    const pageLeft = rect.left + page * pw;
    let offset = 0;
    const textNodes = [];
    const walk = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, null);
    while (walk.nextNode()) textNodes.push(walk.currentNode);
    for (const node of textNodes) {{
      const r = document.createRange();
      r.selectNode(node);
      const br = r.getBoundingClientRect();
      if (br.left >= pageLeft && br.left < pageLeft + pw && br.top >= rect.top) {{
        break;
      }}
      offset += node.length;
    }}
    sendIpc({{
      type: 'position',
      chapter: currentChapter,
      char_offset: offset,
      page: page,
      total_pages: totalPages()
    }});
  }} else {{
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
      char_offset: offset,
      page: 0,
      total_pages: 1
    }});
  }}
}}

function goToPage(page, animate) {{
  if (!isPaginated()) return;
  const total = totalPages();
  page = Math.max(0, Math.min(total - 1, page));
  if (page === currentPage()) return;
  if (animate && currentSettings.animate) {{
    flipToPage(page);
  }} else {{
    content.scrollTo({{ left: page * pageWidth(), behavior: animate ? 'smooth' : 'auto' }});
  }}
}}

function nextPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop += content.clientHeight * 0.9;
  }} else {{
    goToPage(currentPage() + 1, true);
  }}
}}

function prevPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop -= content.clientHeight * 0.9;
  }} else {{
    goToPage(currentPage() - 1, true);
  }}
}}

function capturePage(page) {{
  const container = document.createElement('div');
  container.className = 'page-capture';
  container.style.width = '100%';
  container.style.height = '100%';
  container.style.position = 'relative';
  container.style.overflow = 'hidden';

  const clone = content.cloneNode(true);
  const pw = pageWidth();
  clone.style.position = 'absolute';
  clone.style.left = -(page * pw) + 'px';
  clone.style.top = '0';
  clone.style.width = content.scrollWidth + 'px';
  clone.style.height = content.clientHeight + 'px';
  clone.style.padding = '0';
  clone.style.margin = '0';
  clone.style.overflow = 'visible';

  container.appendChild(clone);
  return container;
}}

function flipToPage(targetPage) {{
  if (isFlipping) return;
  const fromPage = currentPage();
  const direction = targetPage > fromPage ? 1 : -1;
  if (targetPage === fromPage) return;
  isFlipping = true;

  const sheet = document.createElement('div');
  sheet.className = 'sheet';
  const isDouble = document.body.classList.contains('double');
  sheet.style.left = isDouble ? '50%' : '0';
  sheet.style.width = isDouble ? '50%' : '100%';

  const front = document.createElement('div');
  front.className = 'front';
  front.appendChild(capturePage(fromPage));
  const back = document.createElement('div');
  back.className = 'back';
  back.appendChild(capturePage(targetPage));

  sheet.appendChild(front);
  sheet.appendChild(back);
  flipper.innerHTML = '';
  flipper.appendChild(sheet);
  flipper.style.display = 'block';

  content.scrollLeft = targetPage * pageWidth();

  requestAnimationFrame(() => {{
    sheet.style.transform = direction > 0 ? 'rotateY(-180deg)' : 'rotateY(180deg)';
  }});

  setTimeout(() => {{
    flipper.style.display = 'none';
    flipper.innerHTML = '';
    isFlipping = false;
    reportPosition();
  }}, 450);
}}

function onWheel(e) {{
  if (!isPaginated()) return;
  e.preventDefault();
  const delta = currentSettings.invert_scroll ? -e.deltaY : e.deltaY;
  if (delta > 0 || e.deltaX > 0) {{
    nextPage();
  }} else if (delta < 0 || e.deltaX < 0) {{
    prevPage();
  }}
}}

function onClick(e) {{
  if (!isPaginated()) return;
  if (window.getSelection().toString().length > 0) return;
  const rect = content.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width / 2) {{
    prevPage();
  }} else {{
    nextPage();
  }}
}}

content.addEventListener('wheel', onWheel, {{ passive: false }});
content.addEventListener('click', onClick);
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
        settings_json = serde_json::to_string(&js).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_storage::models::EbookSettings;

    #[test]
    fn test_reader_html_contains_spread_containers() {
        let html = reader_html(&EbookSettings::default());
        assert!(html.contains("id=\"measure\""));
        assert!(html.contains("id=\"spread\""));
        assert!(html.contains("function splitIntoSpreads"));
        assert!(html.contains("function goToSpread"));
    }

    #[test]
    fn test_reader_html_contains_required_functions() {
        let settings = EbookSettings::default();
        let html = reader_html(&settings);
        assert!(!html.is_empty());
        assert!(html.contains("function loadChapter"));
        assert!(html.contains("function applySettings"));
        assert!(html.contains("function nextPage"));
        assert!(html.contains("function prevPage"));
        assert!(html.contains("function goToPage"));
        assert!(html.contains("function reportPosition"));
        assert!(html.contains("function sendIpc"));
        assert!(html.contains("function onWheel"));
        assert!(html.contains("function onClick"));
        assert!(html.contains("window.ipc.postMessage"));
    }

    #[test]
    fn test_measure_and_spread_share_box_model() {
        let settings = EbookSettings::default();
        let html = reader_html(&settings);
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
    fn test_reader_html_includes_pagination_css() {
        let settings = EbookSettings {
            font_size: 20,
            line_height: 1.8,
            margin_horizontal: 32,
            margin_vertical: 40,
            ..Default::default()
        };
        let html = reader_html(&settings);
        assert!(html.contains("--size: 20px"));
        assert!(html.contains("--line: 1.8"));
        assert!(html.contains("--margin-h: 32px"));
        assert!(html.contains("--margin-v: 40px"));
        assert!(html.contains("scroll-snap-type: x mandatory"));
        assert!(html.contains("break-inside: avoid"));
    }
}
