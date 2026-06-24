use crate::ebook_renderer::EbookRenderer;
use rust_reader_core::ebook::Ebook;
use rust_reader_storage::models::EbookSettings;
use wry::Rect;

#[derive(Default)]
pub struct EbookView {
    pub open: Option<OpenEbook>,
    pub show_toc: bool,
}

pub struct OpenEbook {
    pub ebook: Ebook,
    pub renderer: EbookRenderer,
    pub current_chapter: usize,
    pub current_page: usize,
}

impl EbookView {
    /// Opens an ebook. Reserved for the future ebook open flow.
    #[allow(dead_code)]
    pub fn open(
        &mut self,
        parent: &(impl wry::raw_window_handle::HasWindowHandle
              + wry::raw_window_handle::HasDisplayHandle),
        bounds: Rect,
        ebook: Ebook,
        settings: &EbookSettings,
    ) -> Result<(), String> {
        let renderer = EbookRenderer::new(parent, bounds, ebook.clone(), settings.clone())?;
        self.open = Some(OpenEbook {
            ebook,
            renderer,
            current_chapter: 0,
            current_page: 0,
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

    /// Navigates to a chapter.
    pub fn goto_chapter(&mut self, chapter: usize) {
        if let Some(open) = self.open.as_mut() {
            let chapter = chapter.min(open.ebook.total_chapters().saturating_sub(1));
            open.current_chapter = chapter;
            open.renderer.goto_chapter(chapter, 0);
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
            let (chapter, _, page) = open.renderer.current_position();
            open.current_chapter = chapter;
            open.current_page = page;
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        // Reserve the central panel area; the webview is positioned over it.
        ui.allocate_space(ui.available_size());
    }

    /// Renders the table-of-contents side panel. Call this *before* the
    /// central panel so the webview bounds can avoid the panel area.
    pub fn render_toc(&self, ctx: &egui::Context) -> Option<usize> {
        let open = self.open.as_ref()?;
        let mut jump_to = None;
        egui::SidePanel::left("ebook_toc")
            .default_width(240.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("目录");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for chapter in &open.ebook.chapters {
                        let is_current = chapter.index == open.current_chapter;
                        let label = chapter.title.as_deref().unwrap_or("无标题");
                        let response = ui.selectable_label(is_current, label);
                        if response.clicked() {
                            jump_to = Some(chapter.index);
                        }
                    }
                });
            });
        jump_to
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_reader_core::ebook::{Ebook, EbookChapter};
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
}
