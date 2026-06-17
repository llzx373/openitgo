use crossbeam_channel::{bounded, Receiver, Sender};
use egui::ColorImage;
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
    let bytes = match source {
        PageSource::File(path) => std::fs::read(path).map_err(|e| e.to_string())?,
        PageSource::ZipEntry { archive, name } => {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let mut archive =
                zip::ZipArchive::new(file).map_err(|e| format!("invalid zip archive: {e}"))?;
            let mut entry = archive
                .by_name(name)
                .map_err(|e| format!("zip entry not found: {e}"))?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("failed to read zip entry: {e}"))?;
            bytes
        }
        PageSource::RarEntry { archive, name } => read_rar_entry(archive, name)?,
        PageSource::PdfPage { .. } => return Err("PDF not yet implemented".to_string()),
    };

    decode_image(&bytes)
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
}
