use crate::timing;
use crossbeam_channel::{bounded, select, unbounded, Receiver, Sender};
use egui::ColorImage;
use pdf_render::pdf_interpret::pdf_syntax::Pdf;
use pdf_render::pdf_interpret::InterpreterSettings;
use pdf_render::vello_cpu::color::palette::css::WHITE;
use pdf_render::{render, RenderSettings};
use rust_reader_core::models::PageSource;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadPriority {
    High,
    Low,
}

#[derive(Debug, Clone)]
pub enum LoadedImage {
    Compressed {
        data: Vec<u8>,
        original_size: [u32; 2],
        gpu_size: [u32; 2],
    },
    Color(ColorImage),
}

impl LoadedImage {
    #[allow(dead_code)]
    pub fn original_size(&self) -> [u32; 2] {
        match self {
            LoadedImage::Compressed { original_size, .. } => *original_size,
            LoadedImage::Color(img) => [img.size[0] as u32, img.size[1] as u32],
        }
    }

    pub fn size_bytes(&self) -> usize {
        match self {
            LoadedImage::Compressed { data, .. } => data.len(),
            LoadedImage::Color(img) => img.size[0] * img.size[1] * 4,
        }
    }

    pub fn to_color_image(&self) -> Result<ColorImage, String> {
        match self {
            LoadedImage::Compressed {
                data,
                original_size,
                gpu_size,
            } => decompress_dxt5(data, *original_size, *gpu_size),
            LoadedImage::Color(img) => Ok(img.clone()),
        }
    }
}

pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub thumbnail: bool,
    pub dropped: bool,
    pub original_size: [u32; 2],
    pub image: Result<LoadedImage, String>,
}

pub struct LoadRequest {
    pub epoch: Epoch,
    pub page_index: usize,
    pub thumbnail: bool,
    pub source: PageSource,
}

struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    thumbnail: bool,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}

pub struct PageLoader {
    high_sender: Sender<LoadRequest>,
    low_sender: Sender<LoadRequest>,
    receiver: Receiver<LoadResult>,
    epoch: Arc<AtomicU64>,
    compress: Arc<AtomicBool>,
    _io_workers: Vec<thread::JoinHandle<()>>,
    _decode_workers: Vec<thread::JoinHandle<()>>,
}

impl Default for PageLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PageLoader {
    /// Create a loader that returns original ColorImage by default (no compression).
    pub fn new() -> Self {
        Self::new_with_compress(false, 0)
    }

    /// Create a loader with explicit compression and decode-thread limits.
    ///
    /// `decode_threads` is the number of background decode workers. `0` means
    /// use the system's reported parallelism (the previous default).
    pub fn new_with_compress(compress: bool, decode_threads: usize) -> Self {
        let (high_sender, high_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) =
            bounded(64);
        let (low_sender, low_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(64);
        // Use an unbounded result channel so completed decode results are never
        // dropped just because the UI thread is temporarily behind.
        let (result_sender, receiver): (Sender<LoadResult>, Receiver<LoadResult>) = unbounded();
        let (high_decode_sender, high_decode_receiver): (Sender<DecodeJob>, Receiver<DecodeJob>) =
            bounded(64);
        let (low_decode_sender, low_decode_receiver): (Sender<DecodeJob>, Receiver<DecodeJob>) =
            bounded(256);

        let compress = Arc::new(AtomicBool::new(compress));

        let worker_count = if decode_threads == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .max(1)
        } else {
            decode_threads.max(1)
        };

        // Spawn multiple IO workers so file reads (especially from archives) can
        // happen in parallel. Each worker owns its own archive cache.
        let io_thread_count = worker_count;
        let mut io_workers = Vec::with_capacity(io_thread_count);
        for _ in 0..io_thread_count {
            let high_receiver = high_receiver.clone();
            let low_receiver = low_receiver.clone();
            let result_sender_for_io = result_sender.clone();
            let high_decode_sender = high_decode_sender.clone();
            let low_decode_sender = low_decode_sender.clone();
            io_workers.push(thread::spawn(move || {
                let mut zip_cache = ZipCache::new();
                let mut high_disconnected = false;
                let mut low_disconnected = false;
                loop {
                    // Drain every pending high-priority IO request before considering
                    // a low-priority one. This keeps rapid page turns responsive even
                    // when an IO worker is busy reading preload entries.
                    while let Ok(req) = high_receiver.try_recv() {
                        process_io_request(
                            req,
                            LoadPriority::High,
                            &result_sender_for_io,
                            &high_decode_sender,
                            &low_decode_sender,
                            &mut zip_cache,
                        );
                    }
                    select! {
                        recv(high_receiver) -> req => {
                            if let Ok(req) = req {
                                process_io_request(
                                    req,
                                    LoadPriority::High,
                                    &result_sender_for_io,
                                    &high_decode_sender,
                                    &low_decode_sender,
                                    &mut zip_cache,
                                );
                            } else {
                                high_disconnected = true;
                                if low_disconnected {
                                    break;
                                }
                            }
                        }
                        recv(low_receiver) -> req => {
                            if let Ok(req) = req {
                                process_io_request(
                                    req,
                                    LoadPriority::Low,
                                    &result_sender_for_io,
                                    &high_decode_sender,
                                    &low_decode_sender,
                                    &mut zip_cache,
                                );
                            } else {
                                low_disconnected = true;
                                if high_disconnected {
                                    break;
                                }
                            }
                        }
                    }
                }
            }));
        }

        timing::log(&format!(
            "PageLoader started with {} IO threads and {} decode workers (compress={})",
            io_thread_count,
            worker_count,
            compress.load(Ordering::Relaxed)
        ));
        let mut decode_workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let high_decode_receiver = high_decode_receiver.clone();
            let low_decode_receiver = low_decode_receiver.clone();
            let result_sender = result_sender.clone();
            let compress = compress.clone();
            decode_workers.push(thread::spawn(move || {
                fn process_job(job: DecodeJob, compress: bool, result_sender: &Sender<LoadResult>) {
                    let result = if job.thumbnail {
                        decode_thumbnail_bytes(&job.bytes, job.format_hint.as_deref()).map(
                            |(thumb, original_size)| LoadResult {
                                epoch: job.epoch,
                                page_index: job.page_index,
                                thumbnail: true,
                                dropped: false,
                                original_size,
                                image: Ok(LoadedImage::Color(thumb)),
                            },
                        )
                    } else {
                        decode_image_bytes(&job.bytes, job.format_hint.as_deref(), compress).map(
                            |image| {
                                let original_size = image.original_size();
                                LoadResult {
                                    epoch: job.epoch,
                                    page_index: job.page_index,
                                    thumbnail: false,
                                    dropped: false,
                                    original_size,
                                    image: Ok(image),
                                }
                            },
                        )
                    };
                    let _ = result_sender.send(result.unwrap_or_else(|e| LoadResult {
                        epoch: job.epoch,
                        page_index: job.page_index,
                        thumbnail: job.thumbnail,
                        dropped: false,
                        original_size: [0, 0],
                        image: Err(e),
                    }));
                }

                let mut high_disconnected = false;
                let mut low_disconnected = false;
                loop {
                    // Always drain pending high-priority jobs first.
                    while let Ok(job) = high_decode_receiver.try_recv() {
                        process_job(job, compress.load(Ordering::Relaxed), &result_sender);
                    }

                    if high_disconnected && low_disconnected {
                        break;
                    }

                    select! {
                        recv(high_decode_receiver) -> job => {
                            if let Ok(job) = job {
                                process_job(job, compress.load(Ordering::Relaxed), &result_sender);
                            } else {
                                high_disconnected = true;
                                if low_disconnected {
                                    break;
                                }
                            }
                        }
                        recv(low_decode_receiver) -> job => {
                            if let Ok(job) = job {
                                process_job(job, compress.load(Ordering::Relaxed), &result_sender);
                            } else {
                                low_disconnected = true;
                                if high_disconnected {
                                    break;
                                }
                            }
                        }
                    }
                }
            }));
        }

        Self {
            high_sender,
            low_sender,
            receiver,
            epoch: Arc::new(AtomicU64::new(1)),
            compress,
            _io_workers: io_workers,
            _decode_workers: decode_workers,
        }
    }

    pub fn set_compress(&self, compress: bool) {
        if self
            .compress
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                if old == compress {
                    None
                } else {
                    Some(compress)
                }
            })
            .is_ok()
        {
            timing::log(&format!("PageLoader compress set to {}", compress));
        }
    }

    pub fn next_epoch(&self) -> Epoch {
        self.epoch.fetch_add(1, Ordering::SeqCst)
    }

    pub fn request_high(&self, epoch: Epoch, page_index: usize, source: PageSource) -> bool {
        // Use try_send so the UI thread never blocks waiting for the IO worker.
        self.high_sender
            .try_send(LoadRequest {
                epoch,
                page_index,
                thumbnail: false,
                source,
            })
            .is_ok()
    }

    pub fn request_low(&self, epoch: Epoch, page_index: usize, source: PageSource) -> bool {
        // Use try_send so preload loops cannot block the UI thread.
        self.low_sender
            .try_send(LoadRequest {
                epoch,
                page_index,
                thumbnail: false,
                source,
            })
            .is_ok()
    }

    /// Request a low-priority thumbnail decode in the background.
    pub fn request_thumbnail(&self, epoch: Epoch, page_index: usize, source: PageSource) -> bool {
        self.low_sender
            .try_send(LoadRequest {
                epoch,
                page_index,
                thumbnail: true,
                source,
            })
            .is_ok()
    }

    /// Request a high-priority thumbnail decode (used when the visible page
    /// has no full image yet and we want something on screen quickly).
    pub fn request_thumbnail_high(
        &self,
        epoch: Epoch,
        page_index: usize,
        source: PageSource,
    ) -> bool {
        self.high_sender
            .try_send(LoadRequest {
                epoch,
                page_index,
                thumbnail: true,
                source,
            })
            .is_ok()
    }

    pub fn try_recv(&self) -> Option<LoadResult> {
        self.receiver.try_recv().ok()
    }
}

fn process_io_request(
    req: LoadRequest,
    priority: LoadPriority,
    result_sender: &Sender<LoadResult>,
    high_decode_sender: &Sender<DecodeJob>,
    low_decode_sender: &Sender<DecodeJob>,
    zip_cache: &mut ZipCache,
) {
    let priority_label = match priority {
        LoadPriority::High => "high",
        LoadPriority::Low => "low",
    };
    let kind = if req.thumbnail { "thumbnail" } else { "full" };
    timing::log(&format!(
        "IO request page {} (epoch {}) priority {} kind {}",
        req.page_index, req.epoch, priority_label, kind
    ));
    match req.source {
        PageSource::PdfPage {
            document,
            page_number,
        } => {
            let result = if req.thumbnail {
                render_pdf_page(&document, page_number).and_then(|image| {
                    make_thumbnail_from_loaded(image).map(|(thumb, original)| LoadResult {
                        epoch: req.epoch,
                        page_index: req.page_index,
                        thumbnail: true,
                        dropped: false,
                        original_size: original,
                        image: Ok(LoadedImage::Color(thumb)),
                    })
                })
            } else {
                render_pdf_page(&document, page_number).map(|image| {
                    let original_size = image.original_size();
                    LoadResult {
                        epoch: req.epoch,
                        page_index: req.page_index,
                        thumbnail: false,
                        dropped: false,
                        original_size,
                        image: Ok(image),
                    }
                })
            };
            let _ = result_sender.send(result.unwrap_or_else(|e| LoadResult {
                epoch: req.epoch,
                page_index: req.page_index,
                thumbnail: req.thumbnail,
                dropped: false,
                original_size: [0, 0],
                image: Err(e),
            }));
        }
        _ => match read_page_bytes(&req.source, zip_cache) {
            Ok((bytes, format_hint)) => {
                let job = DecodeJob {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    thumbnail: req.thumbnail,
                    bytes,
                    format_hint,
                };
                let sender = match priority {
                    LoadPriority::High => high_decode_sender,
                    LoadPriority::Low => low_decode_sender,
                };
                if sender.try_send(job).is_err() {
                    timing::log(&format!(
                        "IO dropped decode job page {} ({} queue full)",
                        req.page_index, priority_label
                    ));
                    // High-priority failures are real errors the UI should surface.
                    // Low-priority (preload/thumbnail) failures are transient backpressure;
                    // tell the UI to drop the pending marker so it can retry later.
                    let dropped = priority == LoadPriority::Low;
                    let _ = result_sender.send(LoadResult {
                        epoch: req.epoch,
                        page_index: req.page_index,
                        thumbnail: req.thumbnail,
                        dropped,
                        original_size: [0, 0],
                        image: Err(format!(
                            "{} decode queue full for page {}",
                            priority_label, req.page_index
                        )),
                    });
                }
            }
            Err(e) => {
                let _ = result_sender.send(LoadResult {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    thumbnail: req.thumbnail,
                    dropped: false,
                    original_size: [0, 0],
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
    let first_attempt = timing::time("read_zip_entry (cached)", || -> Result<Vec<u8>, String> {
        let archive = zip_cache.get_or_open(archive_path)?;
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)
            .map_err(|e| format!("failed to read zip entry: {e}"))?;
        Ok(bytes)
    });

    match first_attempt {
        Ok(bytes) => {
            timing::log(&format!(
                "read_zip_entry index {} -> {} bytes",
                index,
                bytes.len()
            ));
            Ok(bytes)
        }
        Err(_) => {
            // File may have been modified/deleted externally. Drop cache and retry once.
            timing::log("read_zip_entry cache miss/mismatch, retrying");
            zip_cache.remove(archive_path);
            let bytes = timing::time("read_zip_entry (retry)", || -> Result<Vec<u8>, String> {
                let archive = zip_cache.get_or_open(archive_path)?;
                let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes)
                    .map_err(|e| format!("failed to read zip entry: {e}"))?;
                Ok(bytes)
            })?;
            timing::log(&format!(
                "read_zip_entry retry index {} -> {} bytes",
                index,
                bytes.len()
            ));
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

pub(crate) const MAX_IMAGE_DIMENSION: u32 = 4096;

fn compress_dxt5(image: image::DynamicImage) -> Result<LoadedImage, String> {
    let original_w = image.width();
    let original_h = image.height();
    timing::log(&format!(
        "compress_dxt5 start: {}x{}",
        original_w, original_h
    ));
    let rgba = image.to_rgba8();
    let (gpu_w, gpu_h) = dxt5_padded_size(original_w, original_h);

    let pixels = if original_w == gpu_w && original_h == gpu_h {
        rgba.into_raw()
    } else {
        pad_rgba(&rgba, original_w, original_h, gpu_w, gpu_h)
    };

    let mut output =
        vec![0u8; texpresso::Format::Bc3.compressed_size(gpu_w as usize, gpu_h as usize)];
    // RangeFit is much faster than ClusterFit/IterativeClusterFit and is good
    // enough for comic pages. ClusterFit was taking ~18 s for a 1920x1530 image.
    texpresso::Format::Bc3.compress(
        &pixels,
        gpu_w as usize,
        gpu_h as usize,
        texpresso::Params {
            algorithm: texpresso::Algorithm::RangeFit,
            weights: texpresso::COLOUR_WEIGHTS_PERCEPTUAL,
            weigh_colour_by_alpha: false,
        },
        &mut output,
    );
    timing::log(&format!(
        "compress_dxt5 done: {}x{} -> {} bytes",
        original_w,
        original_h,
        output.len()
    ));

    Ok(LoadedImage::Compressed {
        data: output,
        original_size: [original_w, original_h],
        gpu_size: [gpu_w, gpu_h],
    })
}

pub fn dxt5_padded_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(4) * 4, height.div_ceil(4) * 4)
}

pub fn decompress_dxt5(
    data: &[u8],
    original_size: [u32; 2],
    gpu_size: [u32; 2],
) -> Result<ColorImage, String> {
    timing::log(&format!(
        "decompress_dxt5 start: {:?} -> {:?}",
        original_size, gpu_size
    ));
    let [gpu_w, gpu_h] = gpu_size;
    let blocks_x = gpu_w / 4;
    let blocks_y = gpu_h / 4;
    let expected = (blocks_x * blocks_y * 16) as usize;
    if data.len() != expected {
        return Err(format!(
            "DXT5 data size mismatch: {} != {}",
            data.len(),
            expected
        ));
    }

    let result = timing::time("decompress_dxt5 (decode+crop)", || {
        let mut rgba = vec![0u8; (gpu_w * gpu_h * 4) as usize];
        let pitch = gpu_w as usize * 4;

        for by in 0..blocks_y {
            for bx in 0..blocks_x {
                let block_idx = ((by * blocks_x + bx) * 16) as usize;
                let src = &data[block_idx..block_idx + 16];
                let base = (by * 4) as usize * pitch + (bx * 4) as usize * 4;
                bcdec_rs::bc3(src, &mut rgba[base..], pitch);
            }
        }

        let [orig_w, orig_h] = original_size;
        if orig_w == gpu_w && orig_h == gpu_h {
            Ok(ColorImage::from_rgba_unmultiplied(
                [gpu_w as usize, gpu_h as usize],
                &rgba,
            ))
        } else {
            let mut cropped = vec![0u8; (orig_w * orig_h * 4) as usize];
            for y in 0..orig_h {
                let src_start = (y * gpu_w * 4) as usize;
                let dst_start = (y * orig_w * 4) as usize;
                cropped[dst_start..dst_start + (orig_w * 4) as usize]
                    .copy_from_slice(&rgba[src_start..src_start + (orig_w * 4) as usize]);
            }
            Ok(ColorImage::from_rgba_unmultiplied(
                [orig_w as usize, orig_h as usize],
                &cropped,
            ))
        }
    });
    timing::log("decompress_dxt5 done");
    result
}

fn pad_rgba(src: &image::RgbaImage, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..src_h {
        for x in 0..src_w {
            let src_pixel = src.get_pixel(x, y).0;
            let dst_idx = ((y * dst_w + x) * 4) as usize;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src_pixel);
        }
    }
    dst
}

fn decode_image_bytes(
    bytes: &[u8],
    format_hint: Option<&str>,
    compress: bool,
) -> Result<LoadedImage, String> {
    timing::log(&format!(
        "decode_image_bytes start: hint={:?} bytes={} compress={}",
        format_hint,
        bytes.len(),
        compress
    ));

    // On macOS, try the system's ImageIO/Core Graphics decoder first. It is
    // significantly faster than the pure-Rust `image` crate for many formats
    // (especially WebP and JPEG) and handles misnamed files automatically.
    #[cfg(target_os = "macos")]
    match crate::platform::macos::decode_image_bytes(bytes, compress) {
        Ok(Some(image)) => {
            timing::log(&format!(
                "decode_image_bytes done via ImageIO: {:?}",
                image.original_size()
            ));
            return Ok(image);
        }
        Ok(None) => {
            timing::log("decode_image_bytes ImageIO declined, falling back to image crate");
        }
        Err(err) => {
            timing::log(&format!(
                "decode_image_bytes ImageIO failed ({}), falling back to image crate",
                err
            ));
        }
    }

    // Prefer magic-byte detection over the filename extension, because many
    // archives contain misnamed files (e.g. PNG data with a .webp extension).
    let magic_format = image::guess_format(bytes).ok();
    let ext_format = format_hint.and_then(image::ImageFormat::from_extension);
    timing::log(&format!(
        "decode_image_bytes detected formats: magic={:?} ext={:?}",
        magic_format, ext_format
    ));

    let image = timing::time("decode_image_bytes (image crate)", || {
        if let Some(format) = magic_format {
            image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())
        } else if let Some(format) = ext_format {
            match image::load_from_memory_with_format(bytes, format) {
                Ok(img) => Ok(img),
                Err(first_err) => {
                    // Fall back to auto-detection from magic bytes.
                    image::load_from_memory(bytes).map_err(|fallback_err| {
                        format!(
                            "hint {:?} decode failed ({}); auto-detect also failed ({})",
                            format_hint, first_err, fallback_err
                        )
                    })
                }
            }
        } else {
            image::load_from_memory(bytes).map_err(|e| e.to_string())
        }
    })?;
    let image = timing::time("decode_image_bytes (downsample)", || {
        downsample_if_needed(image)
    });
    let result = dynamic_to_loaded_image(image, compress);
    timing::log(&format!(
        "decode_image_bytes done: {:?}",
        result.as_ref().map(|i| i.original_size())
    ));
    result
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

/// Maximum pixel size of the long edge for page thumbnails.
pub(crate) const THUMBNAIL_MAX_DIMENSION: u32 = 256;

/// Resize an already-decoded dynamic image to thumbnail dimensions while
/// preserving aspect ratio.
fn make_thumbnail(image: image::DynamicImage) -> (ColorImage, [u32; 2]) {
    let original_size = [image.width(), image.height()];
    let (w, h) = (image.width(), image.height());
    let max = w.max(h);
    let thumb = if max <= THUMBNAIL_MAX_DIMENSION {
        image
    } else {
        let ratio = THUMBNAIL_MAX_DIMENSION as f32 / max as f32;
        let new_w = (w as f32 * ratio).round() as u32;
        let new_h = (h as f32 * ratio).round() as u32;
        image.resize(
            new_w.max(1),
            new_h.max(1),
            image::imageops::FilterType::Lanczos3,
        )
    };
    let rgba = thumb.to_rgba8();
    (
        ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            &rgba.into_raw(),
        ),
        original_size,
    )
}

/// Build a thumbnail from any loaded image (Color or Compressed).
fn make_thumbnail_from_loaded(loaded: LoadedImage) -> Result<(ColorImage, [u32; 2]), String> {
    let color = loaded.to_color_image()?;
    let (w, h) = (color.size[0] as u32, color.size[1] as u32);
    let bytes: Vec<u8> = color
        .pixels
        .iter()
        .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
        .collect();
    let rgba_image = image::RgbaImage::from_raw(w, h, bytes)
        .ok_or_else(|| "invalid RGBA buffer for thumbnail".to_string())?;
    Ok(make_thumbnail(image::DynamicImage::ImageRgba8(rgba_image)))
}

/// Decode raw bytes into a thumbnail ColorImage plus the original page size.
fn decode_thumbnail_bytes(
    bytes: &[u8],
    format_hint: Option<&str>,
) -> Result<(ColorImage, [u32; 2]), String> {
    let loaded = decode_image_bytes(bytes, format_hint, false)?;
    make_thumbnail_from_loaded(loaded)
}

/// Convert an already-decoded dynamic image into the app's internal format.
///
/// This is shared between the native macOS decoder and the `image`-crate
/// fallback path. It does **not** downsample; callers should downsample first
/// if needed.
pub(crate) fn dynamic_to_loaded_image(
    image: image::DynamicImage,
    compress: bool,
) -> Result<LoadedImage, String> {
    let (w, h) = (image.width(), image.height());
    if compress {
        compress_dxt5(image)
    } else {
        let rgba = image.to_rgba8();
        Ok(LoadedImage::Color(ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            &rgba.into_raw(),
        )))
    }
}

const PDF_RENDER_DPI: f32 = 150.0;
const PDF_BASE_DPI: f32 = 72.0;
const PDF_MAX_RENDER_WIDTH: f32 = 2048.0;

/// Render a PDF page directly to an egui [`ColorImage`].
pub fn render_pdf_page(document: &Path, page_number: usize) -> Result<LoadedImage, String> {
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
    Ok(LoadedImage::Color(ColorImage::from_rgba_premultiplied(
        size,
        pixmap.data_as_u8_slice(),
    )))
}

#[cfg(test)]
fn assert_loaded_image_size(image: LoadedImage, expected: [usize; 2]) {
    match image {
        LoadedImage::Compressed { original_size, .. } => {
            assert_eq!(
                [original_size[0] as usize, original_size[1] as usize],
                expected
            );
        }
        LoadedImage::Color(color) => {
            assert_eq!(color.size, expected);
        }
    }
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
        let loaded_image = result.image.expect("expected image to load successfully");
        assert_loaded_image_size(loaded_image, [64, 64]);
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
        match image {
            LoadedImage::Color(color) => {
                assert!(color.size[0] > 0);
                assert!(color.size[1] > 0);
            }
            _ => panic!("PDF should produce a Color image"),
        }
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
                assert_loaded_image_size(image, [64, 64]);
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
                assert_loaded_image_size(image, [32, 32]);
                received += 1;
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert_eq!(received, 4);
    }

    #[test]
    fn test_loader_decodes_zip_entry_with_wrong_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cbz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // Write PNG bytes but name the entry with a .webp extension.
            zip.start_file("page01.webp", options).unwrap();
            let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 255]));
            let mut buf = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
                .unwrap();
            zip.write_all(&buf).unwrap();
            zip.finish().unwrap();
        }

        let loader = PageLoader::new();
        let epoch = loader.next_epoch();
        loader.request_high(
            epoch,
            0,
            PageSource::ZipEntry {
                archive: path,
                name: "page01.webp".to_string(),
                index: 0,
            },
        );

        let result = wait_for_result(&loader, epoch, 0, Duration::from_secs(5));
        let image = result.image.expect("misnamed PNG should decode");
        assert_loaded_image_size(image, [32, 32]);
    }

    #[test]
    fn test_dxt5_padded_size() {
        assert_eq!(dxt5_padded_size(4, 4), (4, 4));
        assert_eq!(dxt5_padded_size(1, 1), (4, 4));
        assert_eq!(dxt5_padded_size(5, 7), (8, 8));
        assert_eq!(dxt5_padded_size(8, 9), (8, 12));
    }

    #[test]
    fn test_compress_dxt5_output_size() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            5,
            7,
            image::Rgba([255, 0, 0, 255]),
        ));
        let loaded = compress_dxt5(img).expect("compression should succeed");
        match loaded {
            LoadedImage::Compressed {
                data,
                original_size,
                gpu_size,
            } => {
                assert_eq!(original_size, [5, 7]);
                let (expected_w, expected_h) = dxt5_padded_size(5, 7);
                assert_eq!(gpu_size, [expected_w, expected_h]);
                let expected = texpresso::Format::Bc3
                    .compressed_size(gpu_size[0] as usize, gpu_size[1] as usize);
                assert_eq!(data.len(), expected);
            }
            LoadedImage::Color(_) => panic!("expected compressed image"),
        }
    }

    #[test]
    fn test_dxt5_roundtrip() {
        let original = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 128, 64, 255]));
        let loaded = compress_dxt5(image::DynamicImage::ImageRgba8(original.clone()))
            .expect("compression should succeed");
        let color = loaded
            .to_color_image()
            .expect("decompression should succeed");
        assert_eq!(color.size, [8, 8]);
        // DXT5 is lossy; verify the pixel is close to the original.
        let pixel = color.pixels[0];
        assert_eq!(pixel.r(), 255);
        assert!((128i32 - pixel.g() as i32).abs() <= 2);
        assert!((64i32 - pixel.b() as i32).abs() <= 2);
        assert_eq!(pixel.a(), 255);
    }

    #[test]
    fn test_decode_image_bytes_falls_back_for_wrong_extension() {
        // Create a PNG image but tell the decoder the extension is "webp".
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 128, 255, 255]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let loaded = decode_image_bytes(&bytes, Some("webp"), false)
            .expect("PNG content with .webp hint should decode via magic-byte fallback");
        assert_eq!(loaded.original_size(), [4, 4]);

        // And the reverse: WebP content with a .png hint.
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::WebP,
        )
        .unwrap();
        let loaded = decode_image_bytes(&bytes, Some("png"), false)
            .expect("WebP content with .png hint should decode via magic-byte fallback");
        assert_eq!(loaded.original_size(), [4, 4]);
    }

    #[test]
    fn test_io_sends_error_when_decode_queue_full() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sample.png");
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        img.save(&path).unwrap();

        let (result_sender, result_receiver) = unbounded();
        // A zero-capacity decode queue guarantees try_send will fail.
        let (high_decode_sender, _high_decode_receiver) = bounded(0);
        let (low_decode_sender, _low_decode_receiver) = bounded(0);
        let mut zip_cache = ZipCache::new();

        process_io_request(
            LoadRequest {
                epoch: 1,
                page_index: 7,
                thumbnail: false,
                source: PageSource::File(path),
            },
            LoadPriority::High,
            &result_sender,
            &high_decode_sender,
            &low_decode_sender,
            &mut zip_cache,
        );

        let result = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("expected an error result when decode queue is full");
        assert_eq!(result.epoch, 1);
        assert_eq!(result.page_index, 7);
        assert!(
            result.image.is_err(),
            "result should be an error, got {:?}",
            result.image
        );
    }
}
