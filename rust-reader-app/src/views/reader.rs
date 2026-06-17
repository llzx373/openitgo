use rust_reader_core::models::Comic;
use rust_reader_core::state::ReadingState;

#[derive(Default)]
pub struct ReaderView {
    pub comic: Option<Comic>,
    pub state: Option<ReadingState>,
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState) {
        self.comic = Some(comic);
        self.state = Some(state);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(comic) = &self.comic else {
            ui.label("未打开漫画");
            return;
        };
        let Some(state) = &self.state else {
            return;
        };
        let volume = &comic.volumes[0];
        let page = &volume.pages[state.current_page];
        let texture = match &page.source {
            rust_reader_core::models::PageSource::File(path) => {
                crate::widgets::page_view::load_texture_from_path(ctx, path)
            }
            rust_reader_core::models::PageSource::Bytes(bytes) => {
                crate::widgets::page_view::load_texture_from_bytes(ctx, bytes.as_slice())
            }
            rust_reader_core::models::PageSource::PdfRef { .. } => None,
        };
        if let Some(texture) = texture {
            ui.image(&texture);
        } else {
            ui.label("无法加载页面");
        }
    }
}
