#![allow(dead_code)]

use crate::ebook_renderer::EbookRenderer;
use rust_reader_core::ebook::Ebook;
use rust_reader_storage::models::EbookSettings;
use wry::Rect;

#[derive(Default)]
pub struct EbookView {
    pub open: Option<OpenEbook>,
}

pub struct OpenEbook {
    pub ebook: Ebook,
    pub renderer: EbookRenderer,
    pub current_chapter: usize,
    pub char_offset: usize,
}

impl EbookView {
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
            char_offset: 0,
        });
        Ok(())
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

    pub fn goto_chapter(&mut self, chapter: usize) {
        if let Some(open) = self.open.as_mut() {
            open.current_chapter = chapter;
            open.char_offset = 0;
            open.renderer.goto_chapter(chapter, 0);
        }
    }

    pub fn ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        // Reserve the central panel area; the webview is positioned over it.
        ui.allocate_space(ui.available_size());
    }
}
