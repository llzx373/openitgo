#![allow(dead_code)]

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
                EbookReadingMode::SinglePage => "single".to_string(),
                EbookReadingMode::DoublePage => "double".to_string(),
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
                } else {
                    eprintln!("ebook ipc: malformed message: {}", body);
                }
            })
            .with_url("ebook://reader")
            .build_as_child(parent)
            .map_err(|e| e.to_string())?;

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

    if path == "/reader" {
        return wry::http::Response::builder()
            .header("Content-Type", "text/html; charset=utf-8")
            .body(reader_html(&state.settings).into_bytes().into())
            .unwrap();
    }

    if let Some(rest) = path.strip_prefix("/chapter/") {
        if let Ok(idx) = rest.parse::<usize>() {
            match rust_reader_parser::html::render_chapter_html(&state.ebook, idx) {
                Ok(html) => {
                    return wry::http::Response::builder()
                        .header("Content-Type", "application/xhtml+xml; charset=utf-8")
                        .body(html.into_bytes().into())
                        .unwrap();
                }
                Err(e) => {
                    eprintln!("EbookRenderer: failed to render chapter {idx}: {e}");
                }
            }
        }
    }

    wry::http::Response::builder()
        .status(404)
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
    const res = await fetch('ebook://chapter/' + currentChapter);
    const html = await res.text();
    content.innerHTML = html;
    if (offset) {{
      scrollToOffset(offset);
    }}
    reportPosition();
  }} catch (e) {{
    content.innerHTML = '<p>章节加载失败</p>';
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
  window.ipc.postMessage(JSON.stringify({{
    type: 'position',
    chapter: currentChapter,
    char_offset: offset
  }}));
}}

function nextPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop += content.clientHeight * 0.9;
  }} else {{
    content.scrollLeft += content.clientWidth;
  }}
  reportPosition();
}}

function prevPage() {{
  if (document.body.classList.contains('scroll')) {{
    content.scrollTop -= content.clientHeight * 0.9;
  }} else {{
    content.scrollLeft -= content.clientWidth;
  }}
  reportPosition();
}}

window.addEventListener('scroll', reportPosition, true);
window.addEventListener('resize', reportPosition);
applySettings({settings_json});
loadChapter(0, 0);
window.ipc.postMessage(JSON.stringify({{ type: 'ready' }}));
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
        assert!(html.contains("window.ipc.postMessage"));
    }
}
