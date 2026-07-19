use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::models::Comic;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example profile_view -- <path-to-archive>");

    let app = ReaderApp {
        opener: Some(AsyncOpener::<Comic>::open(path.clone(), |p| {
            openitgo_parser::parse(p).map_err(|e| e.to_string())
        })),
        current_view: View::Loading(path),
        ..Default::default()
    };

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
                next_log: Duration::from_secs(10),
            }))
        }),
    )
}

struct ProfileApp {
    app: ReaderApp,
    start: Instant,
    next_log: Duration,
}

impl eframe::App for ProfileApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        // Keep polling while idle so the snapshot timer fires headless.
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        // Print a snapshot every 10 s so we can see progress, but keep running.
        if self.start.elapsed() > self.next_log {
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
            }
            self.next_log += Duration::from_secs(10);
        }
    }
}
