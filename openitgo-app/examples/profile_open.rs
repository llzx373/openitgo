use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::models::{Comic, ReadingMode};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");
    std::env::set_var("OPENITGO_PERF", "1");

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example profile_open -- <path-to-archive>");

    let decode_threads = std::env::var("OPENITGO_DECODE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut app = ReaderApp::default();
    app.settings.compress_images = std::env::var_os("OPENITGO_COMPRESS").is_some();
    app.page_loader = openitgo_app::loader::PageLoader::new_with_compress(
        app.settings.compress_images,
        decode_threads,
    );
    app.page_loader.set_perf_enabled(true);
    app.opener = Some(AsyncOpener::<Comic>::open(path.clone(), |p| {
        openitgo_parser::parse(p).map_err(|e| e.to_string())
    }));
    app.current_view = View::Loading(path);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: openitgo_app::cache::wgpu_options_with_bc3(),
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo profile open",
        options,
        Box::new(|cc| {
            openitgo_app::cache::install_wgpu_render_state(
                &cc.egui_ctx,
                cc.wgpu_render_state.clone(),
            );
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(ProfileApp {
                app,
                start: Instant::now(),
                reported: false,
                decode_threads,
                test_suspend: std::env::var_os("OPENITGO_TEST_SUSPEND").is_some(),
                suspend_before: None,
                webtoon: std::env::var_os("OPENITGO_WEBTOON").is_some(),
            }))
        }),
    )
}

struct ProfileApp {
    app: ReaderApp,
    start: Instant,
    reported: bool,
    decode_threads: usize,
    webtoon: bool,
    test_suspend: bool,
    suspend_before: Option<openitgo_app::loader::LoaderPerfSnapshot>,
}

impl eframe::App for ProfileApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        // Keep polling while idle so the snapshot timer fires headless.
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        if self.webtoon {
            if let Some(reader) = self.app.reader_view.open.as_mut() {
                if reader.state.mode != ReadingMode::Webtoon {
                    let total = reader.total_pages();
                    reader.state.set_mode(ReadingMode::Webtoon, total);
                }
            }
        }

        if self.test_suspend
            && self.suspend_before.is_none()
            && self.start.elapsed() > Duration::from_secs(2)
        {
            self.app.current_view = View::Library;
            self.suspend_before = Some(self.app.page_loader.perf_snapshot());
        }

        if !self.reported && self.start.elapsed() > Duration::from_secs(10) {
            self.reported = true;
            if let Some(reader) = self.app.reader_view.open.as_ref() {
                let total = reader.total_pages();
                let thumbs = (0..total)
                    .filter(|&i| reader.cache.contains_thumbnail(i))
                    .count();
                let fulls = (0..total)
                    .filter(|&i| reader.cache.contains_full(i))
                    .count();
                eprintln!(
                    "[profile] {} pages, {} thumbnails, {} full images after {:.1} s",
                    total,
                    thumbs,
                    fulls,
                    self.start.elapsed().as_secs_f64()
                );
                let suspend_after = self
                    .suspend_before
                    .map(|_| self.app.page_loader.perf_snapshot());
                let report = serde_json::json!({
                    "scenario": "profile_open",
                    "decode_threads": self.decode_threads,
                    "source": reader.comic.path,
                    "elapsed_ms": self.start.elapsed().as_millis() as u64,
                    "total": total,
                    "thumbnails": thumbs,
                    "full_images": fulls,
                    "cache_bytes": reader.cache.total_size_bytes(),
                    "thumbnail_cache_bytes": reader.cache.thumbnail_total_size_bytes(),
                    "reader": reader.perf_snapshot(),
                    "loader": self.app.page_loader.perf_snapshot(),
                    "suspend_before": self.suspend_before,
                    "suspend_after": suspend_after,
                });
                write_report(&report).expect("performance report should be written");
                if self.webtoon {
                    assert!(
                        reader
                            .webtoon_visible_pages
                            .iter()
                            .all(|&page| reader.cache.contains_full(page)),
                        "visible Webtoon pages are not fully cached"
                    );
                }
                if let (Some(before), Some(after)) = (self.suspend_before, suspend_after) {
                    assert_eq!(after, before, "Reader work continued after suspension");
                }
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn write_report(report: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(report)?;
    println!("[perf] {json}");
    if let Some(path) = std::env::var_os("OPENITGO_PERF_JSON") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)?;
    }
    Ok(())
}
