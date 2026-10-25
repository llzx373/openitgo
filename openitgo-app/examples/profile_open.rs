use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::models::Comic;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");
    std::env::set_var("OPENITGO_PERF", "1");

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example profile_open -- <path-to-archive>");

    let mut app = ReaderApp::default();
    app.page_loader.set_perf_enabled(true);
    app.opener = Some(AsyncOpener::<Comic>::open(path.clone(), |p| {
        openitgo_parser::parse(p).map_err(|e| e.to_string())
    }));
    app.current_view = View::Loading(path);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo profile open",
        options,
        Box::new(|cc| {
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(ProfileApp {
                app,
                start: Instant::now(),
                reported: false,
            }))
        }),
    )
}

struct ProfileApp {
    app: ReaderApp,
    start: Instant,
    reported: bool,
}

impl eframe::App for ProfileApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        // Keep polling while idle so the snapshot timer fires headless.
        ui.ctx().request_repaint_after(Duration::from_millis(100));

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
                let report = serde_json::json!({
                    "scenario": "profile_open",
                    "source": reader.comic.path,
                    "elapsed_ms": self.start.elapsed().as_millis() as u64,
                    "total": total,
                    "thumbnails": thumbs,
                    "full_images": fulls,
                    "reader": reader.perf_snapshot(),
                    "loader": self.app.page_loader.perf_snapshot(),
                });
                write_report(&report).expect("performance report should be written");
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
