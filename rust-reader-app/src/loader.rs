use crossbeam_channel::{bounded, select, Receiver, Sender};
use egui::ColorImage;
use pdf_render::pdf_interpret::pdf_syntax::Pdf;
use pdf_render::pdf_interpret::InterpreterSettings;
use pdf_render::vello_cpu::color::palette::css::WHITE;
use pdf_render::{render, RenderSettings};
use rust_reader_core::models::PageSource;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

struct ZipCache {
    archives: HashMap<std::path::PathBuf, zip::ZipArchive<std::fs::File>>,
}

impl ZipCache {
    fn new() -> Self {
        Self {
            archives: HashMap::new(),
        }
    }

    fn get_or_open(
        &mut self,
        path: &std::path::Path,
    ) -> Result<&mut zip::ZipArchive<std::fs::File>, String> {
        if !self.archives.contains_key(path) {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            self.archives.insert(path.to_path_buf(), archive);
        }
        self.archives
            .get_mut(path)
            .ok_or_else(|| "zip archive missing from cache".to_string())
    }

    fn remove(&mut self, path: &std::path::Path) {
        self.archives.remove(path);
    }
}

pub type Epoch = u64;

#[allow(dead_code)]
pub enum LoadPriority {
    High,
    Low,
}

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

struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}

pub struct PageLoader {
    high_sender: Sender<LoadRequest>,
    low_sender: Sender<LoadRequest>,
    receiver: Receiver<LoadResult>,
    epoch: Arc<AtomicU64>,
    _io_worker: thread::JoinHandle<()>,
    _decode_workers: Vec<thread::JoinHandle<()>>,
}

impl Default for PageLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PageLoader {
    pub fn new() -> Self {
        let (high_sender, high_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) =
            bounded(64);
        let (low_sender, low_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(64);
        let (result_sender, receiver): (Sender<LoadResult>, Receiver<LoadResult>) = bounded(64);
        let (decode_sender, decode_receiver): (Sender<DecodeJob>, Receiver<DecodeJob>) =
            bounded(64);

        let result_sender_for_io = result_sender.clone();
        let io_worker = thread::spawn(move || {
            let mut zip_cache = ZipCache::new();
            loop {
                if let Ok(req) = high_receiver.try_recv() {
                    process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
                    continue;
                }
                select! {
                    recv(high_receiver) -> req => {
                        if let Ok(req) = req {
                            process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
                        }
                    }
                    recv(low_receiver) -> req => {
                        if let Ok(req) = req {
                            process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
                        }
                    }
                }
            }
        });

        let worker_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);
        let mut decode_workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let decode_receiver = decode_receiver.clone();
            let result_sender = result_sender.clone();
            decode_workers.push(thread::spawn(move || {
                while let Ok(job) = decode_receiver.recv() {
                    let image = decode_image_bytes(&job.bytes, job.format_hint.as_deref());
                    let _ = result_sender.send(LoadResult {
                        epoch: job.epoch,
                        page_index: job.page_index,
                        image,
                    });
                }
            }));
        }

        Self {
            high_sender,
            low_sender,
            receiver,
            epoch: Arc::new(AtomicU64::new(1)),
            _io_worker: io_worker,
            _decode_workers: decode_workers,
        }
    }

    pub fn next_epoch(&self) -> Epoch {
        self.epoch.fetch_add(1, Ordering::SeqCst)
    }

    pub fn request_high(&self, epoch: Epoch, page_index: usize, source: PageSource) {
        // Receiver dropped means PageLoader is shutting down; ignore.
        let _ = self.high_sender.send(LoadRequest {
            epoch,
            page_index,
            source,
        });
    }

    pub fn request_low(&self, epoch: Epoch, page_index: usize, source: PageSource) {
        // Receiver dropped means PageLoader is shutting down; ignore.
        let _ = self.low_sender.send(LoadRequest {
            epoch,
            page_index,
            source,
        });
    }

    #[allow(dead_code)]
    pub fn request(
        &self,
        priority: LoadPriority,
        epoch: Epoch,
        page_index: usize,
        source: PageSource,
    ) {
        match priority {
            LoadPriority::High => self.request_high(epoch, page_index, source),
            LoadPriority::Low => self.request_low(epoch, page_index, source),
        }
    }

    pub fn try_recv(&self) -> Option<LoadResult> {
        self.receiver.try_recv().ok()
    }
}

fn process_io_request(
    req: LoadRequest,
    result_sender: &Sender<LoadResult>,
    decode_sender: &Sender<DecodeJob>,
    zip_cache: &mut ZipCache,
) {
    match req.source {
        PageSource::PdfPage {
            document,
            page_number,
        } => {
            let image = render_pdf_page(&document, page_number);
            let _ = result_sender.send(LoadResult {
                epoch: req.epoch,
                page_index: req.page_index,
                image,
            });
        }
        _ => match read_page_bytes(&req.source, zip_cache) {
            Ok((bytes, format_hint)) => {
                let _ = decode_sender.send(DecodeJob {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    bytes,
                    format_hint,
                });
            }
            Err(e) => {
                let _ = result_sender.send(LoadResult {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    image: Err(e),
                });
            }
        },
    }
}

fn read_zip_entry(
    zip_cache: &mut ZipCache,
    archive_path: &std::path::Path,
    index: usize,
) -> Result<Vec<u8>, String> {
    let first_attempt = (|| -> Result<Vec<u8>, String> {
        let archive = zip_cache.get_or_open(archive_path)?;
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)
            .map_err(|e| format!("failed to read zip entry: {e}"))?;
        Ok(bytes)
    })();

    match first_attempt {
        Ok(bytes) => Ok(bytes),
        Err(_) => {
            // File may have been modified/deleted externally. Drop cache and retry once.
            zip_cache.remove(archive_path);
            let archive = zip_cache.get_or_open(archive_path)?;
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("failed to read zip entry: {e}"))?;
            Ok(bytes)
        }
    }
}

fn read_page_bytes(
    source: &PageSource,
    zip_cache: &mut ZipCache,
) -> Result<(Vec<u8>, Option<String>), String> {
    match source {
        PageSource::PdfPage { .. } => Err("PDF should be rendered on IO thread".to_string()),
        PageSource::File(path) => {
            let hint = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            Ok((bytes, hint))
        }
        PageSource::ZipEntry {
            archive,
            name,
            index,
        } => {
            let hint = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            let bytes = read_zip_entry(zip_cache, archive, *index)?;
            Ok((bytes, hint))
        }
        PageSource::RarEntry { archive, name } => {
            let hint = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            let bytes = read_rar_entry(archive, name)?;
            Ok((bytes, hint))
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

const MAX_IMAGE_DIMENSION: u32 = 4096;

fn decode_image_bytes(bytes: &[u8], format_hint: Option<&str>) -> Result<ColorImage, String> {
    let format = format_hint.and_then(image::ImageFormat::from_extension);
    let image = if let Some(format) = format {
        image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?
    } else {
        image::load_from_memory(bytes).map_err(|e| e.to_string())?
    };
    let image = downsample_if_needed(image);
    let size = [image.width() as _, image.height() as _];
    let rgba = image.to_rgba8();
    let pixels = rgba.as_flat_samples();
    Ok(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()))
}

fn downsample_if_needed(image: image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (image.width(), image.height());
    let max = w.max(h);
    if max <= MAX_IMAGE_DIMENSION {
        return image;
    }
    let ratio = MAX_IMAGE_DIMENSION as f32 / max as f32;
    let new_w = (w as f32 * ratio).round() as u32;
    let new_h = (h as f32 * ratio).round() as u32;
    image.resize(
        new_w.max(1),
        new_h.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

const PDF_RENDER_DPI: f32 = 150.0;
const PDF_BASE_DPI: f32 = 72.0;
const PDF_MAX_RENDER_WIDTH: f32 = 2048.0;

/// Render a PDF page directly to an egui [`ColorImage`].
pub fn render_pdf_page(document: &Path, page_number: usize) -> Result<ColorImage, String> {
    let data = std::fs::read(document).map_err(|e| format!("failed to read PDF file: {e}"))?;
    let pdf = Pdf::new(data).map_err(|e| format!("failed to parse PDF: {e:?}"))?;

    let page = pdf
        .pages()
        .get(page_number)
        .ok_or_else(|| format!("page index {page_number} out of bounds"))?;

    let (page_width, _page_height) = page.render_dimensions();
    let dpi_scale = PDF_RENDER_DPI / PDF_BASE_DPI;
    let scale = if page_width > 0.0 {
        (PDF_MAX_RENDER_WIDTH / page_width).min(dpi_scale)
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

    let size = [pixmap.width() as usize, pixmap.height() as usize];
    Ok(ColorImage::from_rgba_premultiplied(
        size,
        pixmap.data_as_u8_slice(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn test_loader_loads_folder_image() {
        assert_loader_loads_image_with_extension("png");
    }

    #[test]
    fn test_loader_loads_bmp_image() {
        assert_loader_loads_image_with_extension("bmp");
    }

    #[test]
    fn test_loader_loads_tiff_image() {
        assert_loader_loads_image_with_extension("tiff");
    }

    fn assert_loader_loads_image_with_extension(ext: &str) {
        let tmp = tempfile::tempdir().unwrap();
        let sample_path = tmp.path().join(format!("sample.{ext}"));

        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]));
        image.save(&sample_path).unwrap();

        let loader = PageLoader::new();
        let epoch = loader.next_epoch();
        loader.request_high(epoch, 0, PageSource::File(PathBuf::from(&sample_path)));

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
        let image = render_pdf_page(&path, 0).unwrap();
        assert!(image.size[0] > 0);
        assert!(image.size[1] > 0);
    }

    #[test]
    fn test_downsample_if_needed_keeps_small_image_unchanged() {
        let img = image::DynamicImage::new_rgba8(100, 200);
        let out = downsample_if_needed(img);
        assert_eq!(out.width(), 100);
        assert_eq!(out.height(), 200);
    }

    #[test]
    fn test_downsample_if_needed_scales_huge_image_to_max_dimension() {
        // Use a very wide image so allocation is small but width exceeds the limit.
        let img = image::DynamicImage::new_rgba8(5_000, 100);
        let out = downsample_if_needed(img);
        assert!(
            out.width() <= MAX_IMAGE_DIMENSION && out.height() <= MAX_IMAGE_DIMENSION,
            "downsampled image should fit within max dimension"
        );
        let ratio = out.width() as f32 / out.height() as f32;
        let expected = 5_000.0 / 100.0;
        assert!(
            (ratio - expected).abs() < 0.1,
            "aspect ratio should be preserved"
        );
    }

    #[test]
    fn test_loader_decodes_multiple_images_concurrently() {
        let tmp = tempfile::tempdir().unwrap();
        let count = 8;
        let mut epochs = Vec::new();
        let loader = PageLoader::new();

        for i in 0..count {
            let path = tmp.path().join(format!("sample_{i}.png"));
            let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([i as u8, 0, 0, 255]));
            image.save(&path).unwrap();
            let epoch = loader.next_epoch();
            epochs.push(epoch);
            loader.request_high(epoch, i, PageSource::File(path));
        }

        let mut received = 0;
        let start = Instant::now();
        while received < count && start.elapsed() < Duration::from_secs(10) {
            if let Some(result) = loader.try_recv() {
                let pos = epochs
                    .iter()
                    .position(|&e| e == result.epoch)
                    .expect("unknown epoch");
                epochs.remove(pos);
                let image = result.image.expect("image should decode");
                assert_eq!(image.size, [64, 64]);
                received += 1;
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert_eq!(received, count, "all concurrent images should decode");
    }

    #[test]
    fn test_loader_reads_multiple_zip_entries_with_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for i in 0..4 {
                zip.start_file(format!("{:02}.png", i), options).unwrap();
                let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([i as u8, 0, 0, 255]));
                let mut buf = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
                    .unwrap();
                zip.write_all(&buf).unwrap();
            }
            zip.finish().unwrap();
        }

        let loader = PageLoader::new();
        let epoch = loader.next_epoch();
        let sources: Vec<_> = (0..4)
            .map(|i| PageSource::ZipEntry {
                archive: path.clone(),
                name: format!("{:02}.png", i),
                index: i,
            })
            .collect();

        for (i, source) in sources.iter().enumerate() {
            loader.request_high(epoch, i, source.clone());
        }

        let mut received = 0;
        let start = Instant::now();
        while received < 4 && start.elapsed() < Duration::from_secs(10) {
            if let Some(result) = loader.try_recv() {
                assert_eq!(result.epoch, epoch);
                let image = result.image.expect("image should decode");
                assert_eq!(image.size, [32, 32]);
                received += 1;
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert_eq!(received, 4);
    }
}
