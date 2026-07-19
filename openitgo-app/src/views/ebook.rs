use crate::ebook_renderer::EbookRenderer;
use openitgo_core::ebook::Ebook;
use openitgo_storage::models::{EbookSettings, EbookTheme};
use wry::Rect;

#[derive(Default)]
pub struct EbookView {
    pub open: Option<OpenEbook>,
    pub show_toc: bool,
}

/// Ebook full-text search bar state. Kept separate from the renderer so the
/// state machine is unit-testable without a WebView.
#[derive(Default)]
pub struct SearchState {
    pub visible: bool,
    pub query: String,
    focus_pending: bool,
}

impl SearchState {
    pub fn open(&mut self) {
        self.visible = true;
        self.focus_pending = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    /// Returns `true` exactly once after `open()` so the UI can focus the
    /// search input on the first frame it appears.
    pub fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_pending)
    }
}

pub struct OpenEbook {
    pub ebook: Ebook,
    pub renderer: EbookRenderer,
    pub current_chapter: usize,
    pub current_spread: usize,
    pub search: SearchState,
    /// 菜单停放状态：true 时 webview 已 set_visible(false) 隐藏。
    pub webview_hidden: bool,
}

impl EbookView {
    pub fn open(
        &mut self,
        ctx: &egui::Context,
        parent: &(impl wry::raw_window_handle::HasWindowHandle
              + wry::raw_window_handle::HasDisplayHandle),
        bounds: Rect,
        ebook: Ebook,
        settings: &EbookSettings,
    ) -> Result<(), String> {
        let renderer = EbookRenderer::new(parent, bounds, ebook.clone(), settings.clone(), ctx)?;
        self.open = Some(OpenEbook {
            ebook,
            renderer,
            current_chapter: 0,
            current_spread: 0,
            search: SearchState::default(),
            webview_hidden: false,
        });
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = None; // Drops the EbookRenderer and its WebView.
    }

    pub fn update_bounds(&mut self, bounds: Rect) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.set_bounds(bounds);
        }
    }

    /// 菜单/浮层打开时隐藏 webview（停放方案）：egui 弹层画不进原生
    /// webview 区域，只能把 webview 藏起来，区域由 egui 以阅读背景色填充。
    /// 状态去重：仅可见性变化时才发 wry IPC。
    pub fn set_webview_hidden(&mut self, hidden: bool) {
        if let Some(open) = self.open.as_mut() {
            let Some(new) = visibility_transition(open.webview_hidden, hidden) else {
                return;
            };
            open.webview_hidden = new;
            open.renderer.set_visible(!new);
        }
    }

    #[allow(dead_code)] // bin 内未调用；诊断探针（lib 消费者）与测试使用
    pub fn webview_hidden(&self) -> bool {
        self.open
            .as_ref()
            .map(|o| o.webview_hidden)
            .unwrap_or(false)
    }

    pub fn apply_settings(&mut self, settings: &EbookSettings) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.apply_settings(settings);
        }
    }

    pub fn next_page(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.next_page();
        }
    }

    pub fn prev_page(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.prev_page();
        }
    }

    pub fn toggle_toc(&mut self) {
        self.show_toc = !self.show_toc;
    }

    pub fn search_visible(&self) -> bool {
        self.open
            .as_ref()
            .map(|o| o.search.visible)
            .unwrap_or(false)
    }

    pub fn toggle_search(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.search.toggle();
            if !open.search.visible {
                open.renderer.clear_highlights();
            }
        }
    }

    pub fn close_search(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.search.close();
            open.renderer.clear_highlights();
        }
    }

    pub fn find_next(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.find_next();
        }
    }

    pub fn find_prev(&mut self) {
        if let Some(open) = self.open.as_ref() {
            open.renderer.find_prev();
        }
    }

    fn toc_fragment(href: &str) -> Option<String> {
        href.split_once('#').map(|(_, fragment)| {
            let raw = fragment.to_string();
            match percent_encoding::percent_decode_str(&raw).decode_utf8() {
                Ok(decoded) => decoded.into_owned(),
                Err(_) => raw,
            }
        })
    }

    /// Navigates to a chapter.
    pub fn goto_chapter(&mut self, chapter: usize) {
        if let Some(open) = self.open.as_mut() {
            let chapter = chapter.min(open.ebook.total_chapters().saturating_sub(1));
            open.current_chapter = chapter;
            open.renderer.goto_chapter(chapter, 0);
        }
    }

    /// Navigates to a TOC entry, optionally jumping to a fragment within the chapter.
    pub fn goto_toc(&mut self, chapter: usize, fragment: Option<String>) {
        if let Some(open) = self.open.as_mut() {
            let chapter = chapter.min(open.ebook.total_chapters().saturating_sub(1));
            open.current_chapter = chapter;
            open.renderer.jump_to_toc(chapter, fragment.as_deref());
        }
    }

    pub fn next_chapter(&mut self) {
        if let Some(open) = self.open.as_mut() {
            let chapter =
                (open.current_chapter + 1).min(open.ebook.total_chapters().saturating_sub(1));
            open.current_chapter = chapter;
            open.renderer.goto_chapter(chapter, 0);
        }
    }

    pub fn prev_chapter(&mut self) {
        if let Some(open) = self.open.as_mut() {
            let chapter = open.current_chapter.saturating_sub(1);
            open.current_chapter = chapter;
            open.renderer.goto_chapter(chapter, 0);
        }
    }

    pub fn sync_position(&mut self) {
        if let Some(open) = self.open.as_mut() {
            let (chapter, _) = open.renderer.current_position();
            open.current_chapter = chapter;
            open.current_spread = open.renderer.current_spread();
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        // Reserve the central panel area; the webview is positioned over it.
        ui.allocate_space(ui.available_size());
    }

    /// Renders the table-of-contents side panel. Call this *before* the
    /// central panel so the webview bounds can avoid the panel area.
    pub fn render_toc(&self, ui: &mut egui::Ui) -> Option<(usize, Option<String>)> {
        let open = self.open.as_ref()?;
        let mut jump_to: Option<(usize, Option<String>)> = None;
        egui::Panel::left("ebook_toc")
            .default_size(240.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("目录");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for chapter in &open.ebook.chapters {
                        let is_current = chapter.index == open.current_chapter;
                        let label = chapter.title.as_deref().unwrap_or("无标题");
                        let response = ui.selectable_label(is_current, label);
                        if response.clicked() {
                            let fragment = Self::toc_fragment(&chapter.href);
                            jump_to = Some((chapter.index, fragment));
                        }
                    }
                });
            });
        jump_to
    }
}

/// 状态去重：可见性真的变化才返回 Some(新状态)，否则 None（不发 IPC）。
pub fn visibility_transition(applied_hidden: bool, want_hidden: bool) -> Option<bool> {
    (applied_hidden != want_hidden).then_some(want_hidden)
}

/// 电子书主题的阅读背景色（与 ebook_renderer.rs JsSettings 的 bg 值一致）。
pub fn ebook_theme_bg(theme: EbookTheme) -> egui::Color32 {
    match theme {
        EbookTheme::Light => egui::Color32::from_rgb(0xff, 0xff, 0xff),
        EbookTheme::Dark => egui::Color32::from_rgb(0x1a, 0x1a, 0x1a),
        EbookTheme::Sepia => egui::Color32::from_rgb(0xf4, 0xec, 0xd8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openitgo_core::ebook::{Ebook, EbookChapter};
    use std::path::PathBuf;

    fn sample_ebook() -> Ebook {
        Ebook {
            id: "test".to_string(),
            title: "Test Book".to_string(),
            path: PathBuf::from("/tmp/test.epub"),
            authors: Vec::new(),
            language: None,
            resources: Vec::new(),
            spine: Vec::new(),
            chapters: vec![
                EbookChapter {
                    index: 0,
                    id: "ch1".to_string(),
                    href: "ch1.xhtml".to_string(),
                    title: Some("第一章".to_string()),
                },
                EbookChapter {
                    index: 1,
                    id: "ch2".to_string(),
                    href: "ch2.xhtml".to_string(),
                    title: Some("第二章".to_string()),
                },
            ],
        }
    }

    #[test]
    fn test_toggle_toc() {
        let mut view = EbookView::default();
        assert!(!view.show_toc);
        view.toggle_toc();
        assert!(view.show_toc);
        view.toggle_toc();
        assert!(!view.show_toc);
    }

    #[test]
    fn test_current_chapter_label() {
        let ebook = sample_ebook();
        let chapter = &ebook.chapters[1];
        assert_eq!(chapter.index, 1);
        assert_eq!(chapter.title.as_deref(), Some("第二章"));
    }

    #[test]
    fn test_open_ebook_has_current_spread_field() {
        // Compile-time check that OpenEbook exposes a pub current_spread: usize field.
        fn assert_field_exists(renderer: EbookRenderer) -> usize {
            let ebook = sample_ebook();
            let mut open = OpenEbook {
                ebook,
                renderer,
                current_chapter: 0,
                current_spread: 0,
                search: SearchState::default(),
                webview_hidden: false,
            };
            open.current_spread = 7;
            open.current_spread
        }
        let _ = assert_field_exists;
    }

    #[test]
    fn test_toc_fragment_extracts_fragment() {
        assert_eq!(
            EbookView::toc_fragment("chapter.xhtml#section1"),
            Some("section1".to_string())
        );
        assert_eq!(
            EbookView::toc_fragment("chapter.xhtml#top"),
            Some("top".to_string())
        );
    }

    #[test]
    fn test_toc_fragment_returns_none_without_fragment() {
        assert_eq!(EbookView::toc_fragment("chapter.xhtml"), None);
        assert_eq!(EbookView::toc_fragment("path/to/chapter.xhtml"), None);
    }

    #[test]
    fn test_toc_fragment_url_decodes_fragment() {
        assert_eq!(
            EbookView::toc_fragment("chapter.xhtml#section%201"),
            Some("section 1".to_string())
        );
        assert_eq!(
            EbookView::toc_fragment("chapter.xhtml#section%C3%A9"),
            Some("sectioné".to_string())
        );
    }

    #[test]
    fn test_toc_fragment_decode_failure_fallback() {
        // A lone percent sign is not a valid percent-encoding sequence.
        assert_eq!(
            EbookView::toc_fragment("chapter.xhtml#section%"),
            Some("section%".to_string())
        );
    }

    #[test]
    fn test_search_state_open_close_toggle() {
        let mut s = SearchState::default();
        assert!(!s.visible);
        s.toggle();
        assert!(s.visible);
        assert!(s.take_focus_request());
        assert!(!s.take_focus_request());
        s.query = "test".to_string();
        s.close();
        assert!(!s.visible);
        assert!(s.query.is_empty());
    }

    #[test]
    fn test_ebook_view_search_methods_without_open_book() {
        let mut view = EbookView::default();
        assert!(!view.search_visible());
        view.toggle_search();
        view.close_search();
        view.find_next();
        view.find_prev();
        assert!(!view.search_visible());
    }

    #[test]
    fn test_visibility_transition_only_fires_on_change() {
        assert_eq!(visibility_transition(false, true), Some(true));
        assert_eq!(visibility_transition(true, false), Some(false));
        assert_eq!(visibility_transition(false, false), None);
        assert_eq!(visibility_transition(true, true), None);
    }

    #[test]
    fn test_ebook_theme_bg_matches_js_settings() {
        // 与 ebook_renderer.rs JsSettings 的 bg 值保持一致
        assert_eq!(
            ebook_theme_bg(EbookTheme::Light),
            egui::Color32::from_rgb(0xff, 0xff, 0xff)
        );
        assert_eq!(
            ebook_theme_bg(EbookTheme::Dark),
            egui::Color32::from_rgb(0x1a, 0x1a, 0x1a)
        );
        assert_eq!(
            ebook_theme_bg(EbookTheme::Sepia),
            egui::Color32::from_rgb(0xf4, 0xec, 0xd8)
        );
    }

    #[test]
    fn test_set_webview_hidden_without_open_book_is_noop() {
        let mut view = EbookView::default();
        view.set_webview_hidden(true);
        assert!(!view.webview_hidden());
        view.set_webview_hidden(false);
        assert!(!view.webview_hidden());
    }
}
