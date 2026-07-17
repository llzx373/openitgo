use openitgo_core::ebook::{Ebook, EbookReadingMode};
use openitgo_storage::models::{EbookSettings, EbookTheme};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use wry::{Rect, WebView, WebViewBuilder};

#[path = "ebook_renderer_template.rs"]
mod ebook_renderer_template;
use ebook_renderer_template::reader_html;

pub struct EbookRenderer {
    webview: WebView,
    state: Arc<Mutex<RendererState>>,
}

struct RendererState {
    ebook: Ebook,
    current_chapter: usize,
    char_offset: usize,
    current_spread: usize,
    total_spreads: usize,
    settings: EbookSettings,
    search_count: usize,
    search_active: i64,
}

#[derive(Debug, serde::Deserialize)]
struct JsToRust {
    #[serde(rename = "type")]
    kind: String,
    chapter: Option<usize>,
    char_offset: Option<usize>,
    spread: Option<usize>,
    total_spreads: Option<usize>,
    error: Option<String>,
    count: Option<usize>,
    active: Option<i64>,
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
    animate: bool,
    invert_scroll: bool,
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
            animate: s.enable_page_animation,
            invert_scroll: s.invert_scroll,
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
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings,
            search_count: 0,
            search_active: -1,
        }));

        let ipc_state = state.clone();
        let repaint = ctx.clone();
        let webview = WebViewBuilder::new()
            .with_bounds(bounds)
            .with_custom_protocol("ebook".to_string(), {
                let state = state.clone();
                move |_id, request| handle_ebook_protocol(&state, request)
            })
            .with_ipc_handler(move |request| {
                let body = request.body();
                if let Ok(msg) = serde_json::from_str::<JsToRust>(body) {
                    handle_ipc_message(msg, &ipc_state, &repaint);
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
        // Warm the parser cache for the chapters the user is most likely to
        // open next. The JS side also preloads on its own navigation paths;
        // these calls are idempotent and cheap if already done.
        if chapter > 0 {
            self.preload_chapter(chapter - 1);
        }
        self.preload_chapter(chapter + 1);
    }

    /// Ask the webview to fetch and parse a chapter into an inert template.
    /// This is best-effort and does not change the visible page.
    pub fn preload_chapter(&self, chapter: usize) {
        let js = format!("preloadChapter({});", chapter);
        if let Err(e) = self.webview.evaluate_script(&js) {
            eprintln!("EbookRenderer::preload_chapter failed: {e}");
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

    pub fn current_spread_count(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.total_spreads.max(1)
    }

    pub fn current_spread(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.current_spread
    }

    pub fn jump_to_toc(&self, chapter: usize, fragment: Option<&str>) {
        if let Ok(mut state) = self.state.lock() {
            state.current_chapter = chapter;
        }
        let fragment = fragment.unwrap_or("");
        let js = format!(
            "jumpToTocItem({}, {});",
            chapter,
            serde_json::to_string(fragment).unwrap_or_default()
        );
        if let Err(e) = self.webview.evaluate_script(&js) {
            eprintln!("EbookRenderer::jump_to_toc failed: {e}");
        }
    }

    pub fn find_text(&self, query: &str) {
        let js = format!(
            "findText({});",
            serde_json::to_string(query).unwrap_or_default()
        );
        if let Err(e) = self.webview.evaluate_script(&js) {
            eprintln!("EbookRenderer::find_text failed: {e}");
        }
    }

    pub fn find_next(&self) {
        if let Err(e) = self.webview.evaluate_script("findNext();") {
            eprintln!("EbookRenderer::find_next failed: {e}");
        }
    }

    pub fn find_prev(&self) {
        if let Err(e) = self.webview.evaluate_script("findPrev();") {
            eprintln!("EbookRenderer::find_prev failed: {e}");
        }
    }

    pub fn clear_highlights(&self) {
        if let Err(e) = self.webview.evaluate_script("clearHighlights();") {
            eprintln!("EbookRenderer::clear_highlights failed: {e}");
        }
    }

    /// Current search state as reported by the webview: `(match count, active
    /// index)`. `(0, -1)` when no search is active.
    pub fn search_state(&self) -> (usize, i64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.search_count, state.search_active)
    }
}

fn handle_ipc_message(msg: JsToRust, state: &Arc<Mutex<RendererState>>, repaint: &egui::Context) {
    if msg.kind.as_str() == "error" {
        if let Some(err) = msg.error {
            eprintln!("EbookRenderer: JS error: {err}");
        }
        return;
    }
    if msg.kind.as_str() == "debug" {
        eprintln!("EbookRenderer: JS debug: {msg:?}");
        return;
    }
    if msg.kind.as_str() == "search" {
        if let Ok(mut state) = state.lock() {
            state.search_count = msg.count.unwrap_or(0);
            state.search_active = msg.active.unwrap_or(-1);
        }
        repaint.request_repaint();
        return;
    }
    if let Ok(mut state) = state.lock() {
        if msg.kind.as_str() == "position" {
            if let Some(chapter) = msg.chapter {
                state.current_chapter = chapter;
            }
            if let Some(offset) = msg.char_offset {
                state.char_offset = offset;
            }
            if let Some(total) = msg.total_spreads {
                state.total_spreads = total.max(1);
                state.current_spread = state
                    .current_spread
                    .min(state.total_spreads.saturating_sub(1));
            }
            if let Some(spread) = msg.spread {
                state.current_spread = spread.min(state.total_spreads.saturating_sub(1));
            }
            // The webview's position updates arrive asynchronously; the egui
            // event loop is otherwise idle (ControlFlow::Wait), so ask it to
            // repaint so the toolbar/statusbar see the new values immediately.
            repaint.request_repaint();
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
            match openitgo_parser::html::render_chapter_html(&state.ebook, idx) {
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
        let chapter_count = state.ebook.total_chapters();
        return wry::http::Response::builder()
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-cache, no-store, must-revalidate")
            .body(
                reader_html(&state.settings, chapter_count)
                    .into_bytes()
                    .into(),
            )
            .unwrap();
    }

    if let Some(res_path) = decode_res_path(path) {
        match openitgo_parser::html::read_epub_resource(&state.ebook, &res_path) {
            Some((mime, bytes)) => {
                return wry::http::Response::builder()
                    .header("Content-Type", mime)
                    .header("Cache-Control", "no-cache, no-store, must-revalidate")
                    .body(bytes.into())
                    .unwrap();
            }
            None => {
                eprintln!("EbookRenderer: resource not found: {res_path}");
            }
        }
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

/// Extract and percent-decode the archive path from an `ebook://res/<path>`
/// request URI. Returns `None` for non-resource paths.
fn decode_res_path(path: &str) -> Option<String> {
    let raw = path.strip_prefix("/res/")?;
    if raw.is_empty() {
        return None;
    }
    Some(
        percent_encoding::percent_decode_str(raw)
            .decode_utf8()
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| raw.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_res_path() {
        assert_eq!(
            decode_res_path("/res/OEBPS/Images/pic.png"),
            Some("OEBPS/Images/pic.png".to_string())
        );
        assert_eq!(
            decode_res_path("/res/OEBPS/a%20b.png"),
            Some("OEBPS/a b.png".to_string())
        );
        assert_eq!(decode_res_path("/reader"), None);
        assert_eq!(decode_res_path("/res/"), None);
        assert_eq!(decode_res_path("/other/x"), None);
    }

    #[test]
    fn test_js_settings_mode_strings() {
        use openitgo_core::ebook::EbookReadingMode;

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
        use openitgo_storage::models::EbookTheme;

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

    #[test]
    fn test_js_settings_pagination_flags() {
        let settings = EbookSettings {
            enable_page_animation: true,
            invert_scroll: true,
            ..Default::default()
        };
        let js = JsSettings::from(&settings);
        assert!(js.animate);
        assert!(js.invert_scroll);
    }

    #[test]
    fn test_js_to_rust_deserializes_spread_fields() {
        let json =
            r#"{"type":"position","chapter":1,"spread":3,"char_offset":120,"total_spreads":12}"#;
        let msg: JsToRust = serde_json::from_str(json).unwrap();
        assert_eq!(msg.kind, "position");
        assert_eq!(msg.chapter, Some(1));
        assert_eq!(msg.spread, Some(3));
        assert_eq!(msg.char_offset, Some(120));
        assert_eq!(msg.total_spreads, Some(12));
    }

    #[test]
    fn test_reader_html_contains_chapter_count() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 5);
        assert!(html.contains("window.ebookChapterCount = 5"));
    }

    #[test]
    fn test_handle_ipc_message_updates_spread_state() {
        use openitgo_core::ebook::Ebook;
        use openitgo_storage::models::EbookSettings;
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};

        let state = Arc::new(Mutex::new(RendererState {
            ebook: Ebook {
                id: "test".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp/test.epub"),
                authors: Vec::new(),
                language: None,
                resources: Vec::new(),
                spine: Vec::new(),
                chapters: Vec::new(),
            },
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }));
        let msg: JsToRust = serde_json::from_str(
            r#"{"type":"position","chapter":2,"spread":5,"char_offset":120,"total_spreads":10}"#,
        )
        .unwrap();
        handle_ipc_message(msg, &state, &egui::Context::default());
        let s = state.lock().unwrap();
        assert_eq!(s.current_chapter, 2);
        assert_eq!(s.current_spread, 5);
        assert_eq!(s.total_spreads, 10);
        assert_eq!(s.char_offset, 120);
    }

    #[test]
    fn test_handle_ipc_message_clamps_spread_to_total() {
        use openitgo_core::ebook::Ebook;
        use openitgo_storage::models::EbookSettings;
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};

        let state = Arc::new(Mutex::new(RendererState {
            ebook: Ebook {
                id: "test".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp/test.epub"),
                authors: Vec::new(),
                language: None,
                resources: Vec::new(),
                spine: Vec::new(),
                chapters: Vec::new(),
            },
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }));
        let msg: JsToRust =
            serde_json::from_str(r#"{"type":"position","spread":15,"total_spreads":10}"#).unwrap();
        handle_ipc_message(msg, &state, &egui::Context::default());
        let s = state.lock().unwrap();
        assert_eq!(s.current_spread, 9);
        assert_eq!(s.total_spreads, 10);
    }

    #[test]
    fn test_handle_ipc_message_clamps_spread_when_total_shrinks() {
        use openitgo_core::ebook::Ebook;
        use openitgo_storage::models::EbookSettings;
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};

        let state = Arc::new(Mutex::new(RendererState {
            ebook: Ebook {
                id: "test".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp/test.epub"),
                authors: Vec::new(),
                language: None,
                resources: Vec::new(),
                spine: Vec::new(),
                chapters: Vec::new(),
            },
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }));
        let msg: JsToRust =
            serde_json::from_str(r#"{"type":"position","spread":5,"total_spreads":10}"#).unwrap();
        handle_ipc_message(msg, &state, &egui::Context::default());
        let msg: JsToRust =
            serde_json::from_str(r#"{"type":"position","total_spreads":3}"#).unwrap();
        handle_ipc_message(msg, &state, &egui::Context::default());
        let s = state.lock().unwrap();
        assert_eq!(s.current_spread, 2);
        assert_eq!(s.total_spreads, 3);
    }

    #[test]
    fn test_handle_ipc_message_accepts_column_paginator_position() {
        use openitgo_core::ebook::Ebook;
        use openitgo_storage::models::EbookSettings;
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};

        let state = Arc::new(Mutex::new(RendererState {
            ebook: Ebook {
                id: "test".to_string(),
                title: "Test".to_string(),
                path: PathBuf::from("/tmp/test.epub"),
                authors: Vec::new(),
                language: None,
                resources: Vec::new(),
                spine: Vec::new(),
                chapters: Vec::new(),
            },
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }));
        // The column paginator sends the same position shape as the old paginator.
        let msg: JsToRust = serde_json::from_str(
            r#"{"type":"position","chapter":1,"spread":4,"char_offset":200,"total_spreads":12}"#,
        )
        .unwrap();
        handle_ipc_message(msg, &state, &egui::Context::default());
        let s = state.lock().unwrap();
        assert_eq!(s.current_chapter, 1);
        assert_eq!(s.current_spread, 4);
        assert_eq!(s.total_spreads, 12);
        assert_eq!(s.char_offset, 200);
    }

    #[test]
    fn test_reader_html_has_single_page_column_formulas() {
        use openitgo_storage::models::EbookSettings;
        let html = reader_html(&EbookSettings::default(), 3);
        // Single-page branch uses the column content width as the page box.
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

    fn test_state() -> Arc<Mutex<RendererState>> {
        use openitgo_core::ebook::EbookChapter;
        use std::path::PathBuf;
        let ebook = Ebook {
            id: "t".to_string(),
            title: "T".to_string(),
            path: PathBuf::from("/tmp/t.epub"),
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters: vec![EbookChapter {
                index: 0,
                id: "c".to_string(),
                href: "c.xhtml".to_string(),
                title: None,
            }],
        };
        Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            current_spread: 0,
            total_spreads: 1,
            settings: EbookSettings::default(),
            search_count: 0,
            search_active: -1,
        }))
    }

    #[test]
    fn test_ipc_search_message_updates_state() {
        let state = test_state();
        let ctx = egui::Context::default();
        let msg: JsToRust =
            serde_json::from_str(r#"{"type":"search","count":5,"active":2}"#).unwrap();
        handle_ipc_message(msg, &state, &ctx);
        let s = state.lock().unwrap();
        assert_eq!(s.search_count, 5);
        assert_eq!(s.search_active, 2);
    }

    #[test]
    fn test_ipc_search_message_defaults() {
        let state = test_state();
        let ctx = egui::Context::default();
        let msg: JsToRust = serde_json::from_str(r#"{"type":"search"}"#).unwrap();
        handle_ipc_message(msg, &state, &ctx);
        let s = state.lock().unwrap();
        assert_eq!(s.search_count, 0);
        assert_eq!(s.search_active, -1);
    }
}
