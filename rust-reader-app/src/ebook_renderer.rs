use rust_reader_core::ebook::{Ebook, EbookReadingMode};
use rust_reader_storage::models::{EbookSettings, EbookTheme};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use wry::{Rect, WebView, WebViewBuilder};

pub struct EbookRenderer {
    webview: WebView,
    state: Arc<Mutex<RendererState>>,
}

struct RendererState {
    ebook: Ebook,
    current_chapter: usize,
    char_offset: usize,
    settings: EbookSettings,
}

#[derive(Debug, serde::Deserialize)]
struct JsToRust {
    #[serde(rename = "type")]
    kind: String,
    chapter: Option<usize>,
    char_offset: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
struct JsSettings {
    mode: String,
    bg: String,
    fg: String,
    font: String,
    size: u32,
    line: f32,
    margin_h: u32,
    margin_v: u32,
}

impl From<&EbookSettings> for JsSettings {
    fn from(s: &EbookSettings) -> Self {
        let (bg, fg) = match s.theme {
            EbookTheme::Light => ("#ffffff".to_string(), "#1a1a1a".to_string()),
            EbookTheme::Dark => ("#1a1a1a".to_string(), "#e8e8e8".to_string()),
            EbookTheme::Sepia => ("#f4ecd8".to_string(), "#5b4636".to_string()),
        };
        Self {
            mode: match s.reading_mode {
                EbookReadingMode::SinglePage => "single paginated".to_string(),
                EbookReadingMode::DoublePage => "double paginated".to_string(),
                EbookReadingMode::Scroll => "scroll".to_string(),
            },
            bg,
            fg,
            font: s.font_family.clone(),
            size: s.font_size,
            line: s.line_height,
            margin_h: s.margin_horizontal,
            margin_v: s.margin_vertical,
        }
    }
}

impl EbookRenderer {
    pub fn new<
        W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle,
    >(
        parent: &W,
        bounds: Rect,
        ebook: Ebook,
        settings: EbookSettings,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            settings,
        }));

        let ipc_state = state.clone();
        let webview = WebViewBuilder::new()
            .with_bounds(bounds)
            .with_custom_protocol("ebook".to_string(), {
                let state = state.clone();
                move |_id, request| handle_ebook_protocol(&state, request)
            })
            .with_ipc_handler(move |request| {
                let body = request.body();
                if let Ok(msg) = serde_json::from_str::<JsToRust>(body) {
                    handle_ipc_message(msg, &ipc_state);
                }
            })
            .with_url("ebook://reader")
            .build_as_child(parent)
            .map_err(|e| e.to_string())?;
        eprintln!("EbookRenderer: webview created");

        Ok(Self { webview, state })
    }

    pub fn set_bounds(&self, bounds: Rect) {
        if let Err(e) = self.webview.set_bounds(bounds) {
            eprintln!("EbookRenderer::set_bounds failed: {e}");
        }
    }

    pub fn apply_settings(&self, settings: &EbookSettings) {
        if let Ok(mut state) = self.state.lock() {
            state.settings = settings.clone();
        }
        let js = format!(
            "applySettings({});",
            serde_json::to_string(&JsSettings::from(settings)).unwrap_or_default()
        );
        if let Err(e) = self.webview.evaluate_script(&js) {
            eprintln!("EbookRenderer::apply_settings failed: {e}");
        }
    }

    pub fn goto_chapter(&self, chapter: usize, offset: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.current_chapter = chapter;
            state.char_offset = offset;
        }
        let js = format!("loadChapter({}, {});", chapter, offset);
        if let Err(e) = self.webview.evaluate_script(&js) {
            eprintln!("EbookRenderer::goto_chapter failed: {e}");
        }
    }

    pub fn next_page(&self) {
        if let Err(e) = self.webview.evaluate_script("nextPage();") {
            eprintln!("EbookRenderer::next_page failed: {e}");
        }
    }

    pub fn prev_page(&self) {
        if let Err(e) = self.webview.evaluate_script("prevPage();") {
            eprintln!("EbookRenderer::prev_page failed: {e}");
        }
    }

    pub fn current_position(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.current_chapter, state.char_offset)
    }
}

fn handle_ipc_message(msg: JsToRust, state: &Arc<Mutex<RendererState>>) {
    if let Ok(mut state) = state.lock() {
        if msg.kind.as_str() == "position" {
            if let Some(chapter) = msg.chapter {
                state.current_chapter = chapter;
            }
            if let Some(offset) = msg.char_offset {
                state.char_offset = offset;
            }
        }
    }
}

fn handle_ebook_protocol(
    state: &Arc<Mutex<RendererState>>,
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<Cow<'static, [u8]>> {
    let path = request.uri().path();

    let state = match state.lock() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("EbookRenderer: mutex poisoned in protocol handler: {e}");
            return wry::http::Response::builder()
                .status(500)
                .body(Vec::new().into())
                .unwrap();
        }
    };

    let query = request.uri().query().unwrap_or("");
    if let Some(ch) = query.strip_prefix("chapter=") {
        if let Ok(idx) = ch.parse::<usize>() {
            eprintln!("EbookRenderer: serving chapter {idx}");
            match rust_reader_parser::html::render_chapter_html(&state.ebook, idx) {
                Ok(html) => {
                    return wry::http::Response::builder()
                        .header("Content-Type", "application/xhtml+xml; charset=utf-8")
                        .header("Cache-Control", "no-cache, no-store, must-revalidate")
                        .body(html.into_bytes().into())
                        .unwrap();
                }
                Err(e) => {
                    eprintln!("EbookRenderer: failed to render chapter {idx}: {e}");
                }
            }
        }
    }

    if path == "/reader" || path == "/" || path.is_empty() {
        return wry::http::Response::builder()
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-cache, no-store, must-revalidate")
            .body(reader_html(&state.settings).into_bytes().into())
            .unwrap();
    }

    // Return an empty 200 response for unknown resource requests instead of a
    // 404, which can be treated as a navigation error by WebKit and trigger a
    // reload of the shell page.
    wry::http::Response::builder()
        .status(200)
        .header("Content-Type", "text/plain")
        .header("Cache-Control", "no-cache, no-store, must-revalidate")
        .body(Vec::new().into())
        .unwrap()
}

fn reader_html(settings: &EbookSettings) -> String {
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
}}
body.double #content {{
  column-width: calc((100vw - var(--margin-h) * 2) / 2);
}}
body.scroll #content {{
  overflow-y: auto;
}}
p {{ margin: 0 0 1em 0; text-indent: 2em; }}
img {{ max-width: 100%; height: auto; }}
</style>
</head>
<body class="{mode}">
<div id="content"></div>
<script>
const content = document.getElementById('content');
let currentChapter = 0;
let currentOffset = 0;

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

function applySettings(json) {{
  const s = typeof json === 'string' ? JSON.parse(json) : json;
  const root = document.documentElement;
  root.style.setProperty('--bg', s.bg);
  root.style.setProperty('--fg', s.fg);
  root.style.setProperty('--font', s.font);
  root.style.setProperty('--size', s.size + 'px');
  root.style.setProperty('--line', s.line);
  root.style.setProperty('--margin-h', s.margin_h + 'px');
  root.style.setProperty('--margin-v', s.margin_v + 'px');
  document.body.className = s.mode;
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
      if (document.body.classList.contains('paginated') || document.body.classList.contains('double')) {{
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
    char_offset: offset
  }});
}}

function pageDelta() {{
  return document.body.classList.contains('double') ? content.clientWidth / 2 : content.clientWidth;
}}

function nextPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop += content.clientHeight * 0.9;
  }} else {{
    content.scrollLeft += pageDelta();
  }}
  reportPosition();
}}

function prevPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop -= content.clientHeight * 0.9;
  }} else {{
    content.scrollLeft -= pageDelta();
  }}
  reportPosition();
}}

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
        settings_json = serde_json::to_string(&js).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_html_contains_required_functions() {
        let settings = EbookSettings::default();
        let html = reader_html(&settings);
        assert!(!html.is_empty());
        assert!(html.contains("function loadChapter"));
        assert!(html.contains("function applySettings"));
        assert!(html.contains("function nextPage"));
        assert!(html.contains("function prevPage"));
        assert!(html.contains("function reportPosition"));
        assert!(html.contains("function sendIpc"));
        assert!(html.contains("window.ipc.postMessage"));
    }

    #[test]
    fn test_reader_html_includes_css_variables() {
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
    }

    #[test]
    fn test_js_settings_mode_strings() {
        use rust_reader_core::ebook::EbookReadingMode;

        let settings = EbookSettings {
            reading_mode: EbookReadingMode::SinglePage,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.mode, "single paginated");

        let settings = EbookSettings {
            reading_mode: EbookReadingMode::DoublePage,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.mode, "double paginated");

        let settings = EbookSettings {
            reading_mode: EbookReadingMode::Scroll,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.mode, "scroll");
    }

    #[test]
    fn test_js_settings_theme_colors() {
        use rust_reader_storage::models::EbookTheme;

        let settings = EbookSettings {
            theme: EbookTheme::Light,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.bg, "#ffffff");
        assert_eq!(js.fg, "#1a1a1a");

        let settings = EbookSettings {
            theme: EbookTheme::Dark,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.bg, "#1a1a1a");
        assert_eq!(js.fg, "#e8e8e8");

        let settings = EbookSettings {
            theme: EbookTheme::Sepia,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert_eq!(js.bg, "#f4ecd8");
        assert_eq!(js.fg, "#5b4636");
    }
}
