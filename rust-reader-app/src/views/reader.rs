use rust_reader_core::models::{Comic, PageSource, ReadingMode};
use rust_reader_core::state::ReadingState;

#[derive(Default)]
pub struct ReaderView {
    pub open: Option<OpenReader>,
}

pub struct OpenReader {
    pub comic: Comic,
    pub state: ReadingState,
    pub texture: Option<egui::TextureHandle>,
    pub texture_page: Option<usize>,
}

impl OpenReader {
    pub fn total_pages(&self) -> usize {
        self.comic
            .volumes
            .first()
            .map(|v| v.pages.len())
            .unwrap_or(0)
    }
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

        let total_pages = reader.total_pages();

        if total_pages == 0 {
            ui.label("此漫画没有页面");
            return;
        }

        let modes = [
            (ReadingMode::Ltr, "国漫"),
            (ReadingMode::Rtl, "日漫"),
            (ReadingMode::Webtoon, "韩漫"),
        ];
        ui.horizontal(|ui| {
            for (mode, label) in modes {
                if ui
                    .selectable_label(reader.state.mode == mode, label)
                    .clicked()
                {
                    reader.state.set_mode(mode, total_pages);
                }
            }
        });

        ui.horizontal(|ui| {
            if ui.button("-").clicked() {
                reader.state.zoom *= 0.9;
            }
            ui.label(format!("{:.0}%", reader.state.zoom * 100.0));
            if ui.button("+").clicked() {
                reader.state.zoom *= 1.1;
            }
            if ui.button("适应").clicked() {
                reader.state.zoom = 1.0;
                reader.state.pan = rust_reader_core::state::Vec2::ZERO;
            }
        });

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

        let response = ui.interact(
            ui.max_rect(),
            ui.id().with("reader_drag"),
            egui::Sense::drag(),
        );
        if response.dragged() {
            reader.state.pan.x += response.drag_delta().x;
            reader.state.pan.y += response.drag_delta().y;
        }

        if let Some(texture) = &reader.texture {
            ui.image(texture);
        } else {
            ui.label("无法加载页面");
        }

        ui.horizontal(|ui| {
            if ui.button("上一页").clicked() {
                reader.state.prev_page();
            }
            ui.label(format!("{}/{}", reader.state.current_page + 1, total_pages));
            if ui.button("下一页").clicked() {
                reader.state.next_page(total_pages);
            }
        });
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
