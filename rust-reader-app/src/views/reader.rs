use crate::widgets::thumbnail_bar::thumbnail_bar;
use rust_reader_core::models::{Comic, PageSource, ReadingMode};
use rust_reader_core::state::{ReadingState, Vec2};

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 5.0;

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
                reader.state.zoom = reader.state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            }
            ui.label(format!("{:.0}%", reader.state.zoom * 100.0));
            if ui.button("+").clicked() {
                reader.state.zoom *= 1.1;
                reader.state.zoom = reader.state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            }
            if ui.button("适应").clicked() {
                reader.state.zoom = 1.0;
                reader.state.pan = Vec2::ZERO;
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

        if let Some(texture) = &reader.texture {
            let texture_size = texture.size_vec2();
            let scaled_size = texture_size * reader.state.zoom;
            let available = ui.available_rect_before_wrap();
            let half_size = scaled_size / 2.0;
            let max_pan_x = (available.width() / 2.0 + half_size.x).max(0.0);
            let max_pan_y = (available.height() / 2.0 + half_size.y).max(0.0);
            reader.state.pan.x = reader.state.pan.x.clamp(-max_pan_x, max_pan_x);
            reader.state.pan.y = reader.state.pan.y.clamp(-max_pan_y, max_pan_y);
            let center = available.center();
            let top_left = egui::pos2(
                center.x - scaled_size.x / 2.0 + reader.state.pan.x,
                center.y - scaled_size.y / 2.0 + reader.state.pan.y,
            );
            let image_rect = egui::Rect::from_min_size(top_left, scaled_size);
            let response = ui.put(
                image_rect,
                egui::Image::new(texture)
                    .fit_to_exact_size(scaled_size)
                    .sense(egui::Sense::drag()),
            );
            if response.dragged() {
                let delta = response.drag_delta();
                reader.state.pan += Vec2::new(delta.x, delta.y);
            }
        } else {
            ui.label("无法加载页面");
        }

        let current_page = reader.state.current_page;
        let total_pages = reader.total_pages();
        let comic = &reader.comic;
        let state = &mut reader.state;
        let texture_page = &mut reader.texture_page;
        thumbnail_bar(ui, comic, current_page, &mut |idx| {
            state.go_to_page(idx, total_pages);
            // Force texture refresh on next frame
            *texture_page = None;
        });

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
