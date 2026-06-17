use crossbeam_channel::{bounded, Receiver, Sender};
use egui::ColorImage;
use pdf_render::pdf_interpret::pdf_syntax::Pdf;
use pdf_render::pdf_interpret::InterpreterSettings;
use pdf_render::vello_cpu::color::palette::css::WHITE;
use pdf_render::{render, RenderSettings};
use rust_reader_core::models::PageSource;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

pub type Epoch = u64;

pub struct LoadRequest {
    pub epoch: Epoch,
    pub page_index: usize,
    pub source: PageSource,
}

pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub image: Result<ColorImage, String>,
}

pub struct PageLoader {
    request_tx: Sender<LoadRequest>,
    result_rx: Receiver<LoadResult>,
    epoch: AtomicU64,
}

impl Default for PageLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PageLoader {
    pub fn new() -> Self {
        let (request_tx, request_rx): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(64);
        let (result_tx, result_rx): (Sender<LoadResult>, Receiver<LoadResult>) = bounded(64);

        thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                let image = load_page(&request.source);
                // Receiver dropped means PageLoader is shutting down; ignore.
                let _ = result_tx.send(LoadResult {
                    epoch: request.epoch,
                    page_index: request.page_index,
                    image,
                });
            }
        });

        Self {
            request_tx,
            result_rx,
            epoch: AtomicU64::new(1),
        }
    }

    pub fn next_epoch(&self) -> Epoch {
        self.epoch.fetch_add(1, Ordering::SeqCst)
    }

    pub fn request(&self, epoch: Epoch, page_index: usize, source: PageSource) {
        // Receiver dropped means PageLoader is shutting down; ignore.
        let _ = self.request_tx.send(LoadRequest {
            epoch,
            page_index,
            source,
        });
    }

    pub fn try_recv(&self) -> Option<LoadResult> {
        self.result_rx.try_recv().ok()
    }
}

fn load_page(source: &PageSource) -> Result<ColorImage, String> {
    match source {
        PageSource::PdfPage {
            document,
            page_number,
        } => {
            let bytes = render_pdf_page(document, *page_number)?;
            decode_image(&bytes)
        }
        _ => {
            let bytes = match source {
                PageSource::File(path) => std::fs::read(path).map_err(|e| e.to_string())?,
                PageSource::ZipEntry { archive, name } => {
                    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                    let mut archive = zip::ZipArchive::new(file)
                        .map_err(|e| format!("invalid zip archive: {e}"))?;
                    let mut entry = archive
                        .by_name(name)
                        .map_err(|e| format!("zip entry not found: {e}"))?;
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut bytes)
                        .map_err(|e| format!("failed to read zip entry: {e}"))?;
                    bytes
                }
                PageSource::RarEntry { archive, name } => read_rar_entry(archive, name)?,
                PageSource::PdfPage { .. } => unreachable!(),
            };

            decode_image(&bytes)
        }
    }
}

fn read_rar_entry(archive_path: &Path, name: &str) -> Result<Vec<u8>, String> {
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|e| e.to_string())?;

    while let Some(entry) = archive.read_header().map_err(|e| e.to_string())? {
        if entry.entry().filename.to_string_lossy() == name {
            let (bytes, _archive) = entry.read().map_err(|e| e.to_string())?;
            return Ok(bytes);
        }
        archive = entry.skip().map_err(|e| e.to_string())?;
    }

    Err(format!("rar entry not found: {name}"))
}

fn decode_image(bytes: &[u8]) -> Result<ColorImage, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let size = [image.width() as _, image.height() as _];
    let rgba = image.to_rgba8();
    let pixels = rgba.as_flat_samples();
    Ok(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()))
}

/// Render a PDF page to PNG-encoded RGBA bytes.
pub fn render_pdf_page(document: &Path, page_number: usize) -> Result<Vec<u8>, String> {
    let data = std::fs::read(document).map_err(|e| format!("failed to read PDF file: {e}"))?;
    let pdf = Pdf::new(data).map_err(|e| format!("failed to parse PDF: {e:?}"))?;

    let page = pdf
        .pages()
        .get(page_number)
        .ok_or_else(|| format!("page index {page_number} out of bounds"))?;

    let (page_width, _page_height) = page.render_dimensions();
    let max_width = 2048.0_f32;
    let dpi_scale = 150.0_f32 / 72.0_f32;
    let scale = if page_width > 0.0 {
        (max_width / page_width).min(dpi_scale)
    } else {
        1.0
    };

    let pixmap = render(
        page,
        &InterpreterSettings::default(),
        &RenderSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
            ..Default::default()
        },
    );

    pixmap
        .into_png()
        .map_err(|e| format!("failed to encode PDF page as PNG: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn test_loader_loads_folder_image() {
        let tmp = tempfile::tempdir().unwrap();
        let sample_path = tmp.path().join("sample.png");

        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]));
        image.save(&sample_path).unwrap();

        let loader = PageLoader::new();
        let epoch = loader.next_epoch();
        loader.request(epoch, 0, PageSource::File(PathBuf::from(&sample_path)));

        let result = wait_for_result(&loader, epoch, 0, Duration::from_secs(5));
        let color_image = result.image.expect("expected image to load successfully");
        assert_eq!(color_image.size, [64, 64]);
    }

    fn wait_for_result(
        loader: &PageLoader,
        expected_epoch: u64,
        expected_page_index: usize,
        timeout: Duration,
    ) -> LoadResult {
        let start = Instant::now();
        loop {
            if let Some(result) = loader.try_recv() {
                if result.epoch == expected_epoch && result.page_index == expected_page_index {
                    return result;
                }
            }
            if start.elapsed() > timeout {
                panic!("timed out waiting for load result");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn test_render_pdf_page_produces_non_empty_image() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../rust-reader-parser/tests/sample.pdf");
        let path = path.canonicalize().expect("sample.pdf should exist");
        let bytes = render_pdf_page(&path, 0).unwrap();
        assert!(!bytes.is_empty());

        let image = image::load_from_memory(&bytes).expect("PNG should decode");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
    }
}
