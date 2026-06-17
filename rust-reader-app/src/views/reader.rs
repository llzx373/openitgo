use rust_reader_core::models::{Comic, PageSource};
use rust_reader_core::state::ReadingState;

#[derive(Default)]
pub struct ReaderView {
    open: Option<OpenReader>,
}

struct OpenReader {
    comic: Comic,
    state: ReadingState,
    texture: Option<egui::TextureHandle>,
    texture_page: Option<usize>,
}

impl ReaderView {
    pub fn open(&mut self, comic: Comic, state: ReadingState) {
        self.open = Some(OpenReader {
            comic,
            state,
            texture: None,
            texture_page: None,
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let Some(reader) = &mut self.open else {
            ui.label("未打开漫画");
            return;
        };

        if reader.texture_page != Some(reader.state.current_page) {
            match load_page_texture(ui.ctx(), &reader.comic, reader.state.current_page) {
                Ok(texture) => {
                    reader.texture = Some(texture);
                    reader.texture_page = Some(reader.state.current_page);
                }
                Err(err) => {
                    reader.texture = None;
                    reader.texture_page = None;
                    ui.label(err);
                    return;
                }
            }
        }

        if let Some(texture) = &reader.texture {
            ui.image(texture);
        } else {
            ui.label("无法加载页面");
        }
    }
}

fn load_page_texture(
    ctx: &egui::Context,
    comic: &Comic,
    page_index: usize,
) -> Result<egui::TextureHandle, String> {
    let volume = comic
        .volumes
        .first()
        .ok_or_else(|| "漫画没有卷".to_string())?;
    let page = volume
        .pages
        .get(page_index)
        .ok_or_else(|| "页面索引越界".to_string())?;

    let label = format!("page_{}", page_index);
    match &page.source {
        PageSource::File(path) => {
            crate::widgets::page_view::load_texture_from_path(ctx, path, &label)
        }
        PageSource::Bytes(bytes) => {
            crate::widgets::page_view::load_texture_from_bytes(ctx, bytes.as_slice(), &label)
        }
        PageSource::PdfRef { .. } => Err("PDF 页面暂不支持".to_string()),
    }
}
