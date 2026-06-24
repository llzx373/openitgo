use rust_reader_core::ebook::{Ebook, EbookChapter, EbookReadingMode};
use rust_reader_storage::models::{EbookSettings, EbookTheme};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use wry::{Rect, WebView, WebViewBuilder};

#[derive(Debug, Clone, PartialEq)]
pub enum EbookIpcMessage {
    PositionReport { chapter: usize, char_offset: usize },
    PageCount(usize),
    Ready,
}

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
    page_count: Option<usize>,
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
                }
            })
            .with_url("ebook://reader")
            .build_as_child(parent)
            .map_err(|e| e.to_string())?;

        Ok(Self { webview, state })
    }

    pub fn set_bounds(&self, bounds: Rect) {
        let _ = self.webview.set_bounds(bounds);
    }

    pub fn apply_settings(&self, settings: &EbookSettings) {
        if let Ok(mut state) = self.state.lock() {
            state.settings = settings.clone();
        }
        let js = format!(
            "applySettings({});",
            serde_json::to_string(&JsSettings::from(settings)).unwrap_or_default()
        );
        let _ = self.webview.evaluate_script(&js);
    }

    pub fn goto_chapter(&self, chapter: usize, offset: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.current_chapter = chapter;
            state.char_offset = offset;
        }
        let js = format!("loadChapter({}, {});", chapter, offset);
        let _ = self.webview.evaluate_script(&js);
    }

    pub fn next_page(&self) {
        let _ = self.webview.evaluate_script("nextPage();");
    }

    pub fn prev_page(&self) {
        let _ = self.webview.evaluate_script("prevPage();");
    }
}

fn handle_ipc_message(msg: JsToRust, state: &Arc<Mutex<RendererState>>) {
    if let Ok(mut state) = state.lock() {
        match msg.kind.as_str() {
            "position" => {
                if let Some(chapter) = msg.chapter {
                    state.current_chapter = chapter;
                }
                if let Some(offset) = msg.char_offset {
                    state.char_offset = offset;
                }
            }
            "ready" | "pagecount" => {}
            _ => {}
        }
    }
}

fn handle_ebook_protocol(
    state: &Arc<Mutex<RendererState>>,
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    let state = state.lock().unwrap();

    if path == "/reader" {
        return wry::http::Response::builder()
            .header("Content-Type", "text/html; charset=utf-8")
            .body(reader_html(&state.settings).into_bytes().into())
            .unwrap();
    }

    if let Some(rest) = path.strip_prefix("/chapter/") {
        if let Ok(idx) = rest.parse::<usize>() {
            if let Some(chapter) = state.ebook.chapter_source(idx) {
                if let Some(html) = load_chapter_html(&state.ebook, chapter) {
                    return wry::http::Response::builder()
                        .header("Content-Type", "application/xhtml+xml; charset=utf-8")
                        .body(html.into_bytes().into())
                        .unwrap();
                }
            }
        }
    }

    wry::http::Response::builder()
        .status(404)
        .body(Vec::new().into())
        .unwrap()
}

fn load_chapter_html(ebook: &Ebook, chapter: &EbookChapter) -> Option<String> {
    if is_text_like_path(&ebook.path) {
        let text = std::fs::read_to_string(&ebook.path).ok()?;
        let raw_chapters = if is_markdown_path(&ebook.path) {
            split_markdown_chapters(&text)
        } else {
            txt_split_chapters(&text)
        };
        let (_, body) = raw_chapters.get(chapter.index)?.clone();
        let html = if is_markdown_path(&ebook.path) {
            markdown_to_html(&body)
        } else {
            plain_text_to_html(&body)
        };
        Some(format!("<div class=\"chapter\">{}</div>", html))
    } else if is_mobi_path(&ebook.path) {
        let mobi = mobi::Mobi::from_path(&ebook.path).ok()?;
        let text = mobi.content_as_string_lossy();
        let words: Vec<&str> = text.split_whitespace().collect();
        let chunk = words.chunks(3000).nth(chapter.index)?;
        Some(format!(
            "<div class=\"chapter\">{}</div>",
            plain_text_to_html(&chunk.join(" "))
        ))
    } else {
        let mut doc = epub::doc::EpubDoc::new(&ebook.path).ok()?;
        doc.set_current_chapter(chapter.index);
        let (bytes, _mime) = doc.get_current()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn plain_text_to_html(body: &str) -> String {
    let escaped = escape_html(body);
    escaped
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", p.replace('\n', "<br>")))
        .collect::<String>()
}

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let parser = Parser::new_ext(md, Options::all());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn is_text_like_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "txt" | "md"))
        .unwrap_or(false)
}

fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn is_mobi_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mobi" | "azw" | "azw3"))
        .unwrap_or(false)
}

fn split_markdown_chapters(text: &str) -> Vec<(Option<String>, String)> {
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
            if !body.is_empty() {
                chapters.push((title.take(), body.join("\n")));
                body.clear();
            }
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if !body.is_empty() || title.is_some() {
        chapters.push((title.take(), body.join("\n")));
    }

    if chapters.is_empty() {
        const CHAPTER_WORDS: usize = 3000;
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
}

fn txt_split_chapters(text: &str) -> Vec<(Option<String>, String)> {
    const DEFAULT_CHAPTER_WORDS: usize = 3000;
    let lines: Vec<&str> = text.lines().collect();
    let mut chapters: Vec<(Option<String>, String)> = Vec::new();
    let mut title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    let is_heading = |line: &str| {
        if line.is_empty() {
            return false;
        }
        if line.starts_with('#') {
            return true;
        }
        let lower = line.to_ascii_lowercase();
        lower.starts_with("chapter ") || (lower.starts_with('第') && lower.contains('章'))
    };

    for line in lines {
        let trimmed = line.trim();
        if is_heading(trimmed) {
            if !body.is_empty() {
                chapters.push((title.take(), body.join("\n")));
                body.clear();
            }
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if !body.is_empty() || title.is_some() {
        chapters.push((title.take(), body.join("\n")));
    }
    if chapters.is_empty() {
        let words: Vec<&str> = text.split_whitespace().collect();
        for (idx, chunk) in words.chunks(DEFAULT_CHAPTER_WORDS).enumerate() {
            chapters.push((Some(format!("第 {} 章", idx + 1)), chunk.join("")));
        }
    }
    chapters
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
    fn test_escape_html() {
        assert_eq!(
            escape_html("<script>alert(\"x\");</script>"),
            "&lt;script&gt;alert(&quot;x&quot;);&lt;/script&gt;"
        );
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_markdown_to_html() {
        let html = markdown_to_html("# Hello\n\nworld");
        assert!(html.contains("Hello"));
        assert!(html.contains("world"));
    }

    #[test]
    fn test_is_text_like_path() {
        assert!(is_text_like_path(std::path::Path::new("book.txt")));
        assert!(is_text_like_path(std::path::Path::new("notes.MD")));
        assert!(!is_text_like_path(std::path::Path::new("book.epub")));
    }

    #[test]
    fn test_is_mobi_path() {
        assert!(is_mobi_path(std::path::Path::new("book.mobi")));
        assert!(is_mobi_path(std::path::Path::new("book.AZW3")));
        assert!(!is_mobi_path(std::path::Path::new("book.epub")));
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
        assert!(html.contains("function reportPosition"));
        assert!(html.contains("window.ipc.postMessage"));
    }
}
