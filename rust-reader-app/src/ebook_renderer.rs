use rust_reader_core::ebook::{Ebook, EbookReadingMode};
use rust_reader_storage::models::{EbookSettings, EbookTheme};
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
    current_page: usize,
    total_pages: usize,
    current_spread: usize,
    total_spreads: usize,
    settings: EbookSettings,
}

#[derive(Debug, serde::Deserialize)]
struct JsToRust {
    #[serde(rename = "type")]
    kind: String,
    chapter: Option<usize>,
    char_offset: Option<usize>,
    page: Option<usize>,
    total_pages: Option<usize>,
    spread: Option<usize>,
    total_spreads: Option<usize>,
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
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(RendererState {
            ebook,
            current_chapter: 0,
            char_offset: 0,
            current_page: 0,
            total_pages: 1,
            current_spread: 0,
            total_spreads: 1,
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

    pub fn current_position(&self) -> (usize, usize, usize) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.current_chapter, state.char_offset, state.current_page)
    }

    pub fn current_spread_count(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.total_spreads.max(1)
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
            if let Some(page) = msg.page {
                state.current_page = page;
            }
            if let Some(total) = msg.total_pages {
                state.total_pages = total.max(1);
            }
            if let Some(spread) = msg.spread {
                state.current_spread = spread;
            }
            if let Some(total) = msg.total_spreads {
                state.total_spreads = total.max(1);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
