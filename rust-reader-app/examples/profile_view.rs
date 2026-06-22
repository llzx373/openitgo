use rust_reader_app::app::{ReaderApp, View};
use rust_reader_app::opener::ComicOpener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("RUST_READER_LOG", "1");

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example profile_open -- <path-to-archive>");

    let mut app = ReaderApp::default();
    app.opener = Some(ComicOpener::open(path.clone(), |p| {
        rust_reader_parser::parse(p).map_err(|e| e.to_string())
    }));
    app.current_view = View::Loading(path);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        ..Default::default()
    };

    eframe::run_native(
        "rustReader profile open",
        options,
        Box::new(|_cc| {
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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);

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
