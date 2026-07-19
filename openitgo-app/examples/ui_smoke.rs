use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::models::Comic;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example ui_smoke -- <path-to-archive>");

    let app = ReaderApp {
        opener: Some(AsyncOpener::<Comic>::open(path.clone(), |p| {
            openitgo_parser::parse(p).map_err(|e| e.to_string())
        })),
        current_view: View::Loading(path),
        ..Default::default()
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo UI smoke",
        options,
        Box::new(|cc| {
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(SmokeApp {
                app,
                start: Instant::now(),
            }))
        }),
    )
}

struct SmokeApp {
    app: ReaderApp,
    start: Instant,
}

impl eframe::App for SmokeApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let ctx = ui.ctx();
        // An idle egui app does not repaint, so poll explicitly: without this
        // the loader results are never drained when run headless (no input
        // events), and the smoke would stall before reaching the checks below.
        ctx.request_repaint_after(Duration::from_millis(100));

        if let Some(reader) = self.app.reader_view.open.as_ref() {
            let current = reader.state.current_page;
            if reader.cache.contains_full(current) {
                eprintln!(
                    "[smoke] current page {} is in cache after {:.1} s",
                    current,
                    self.start.elapsed().as_secs_f64()
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        if self.start.elapsed() > Duration::from_secs(30) {
            eprintln!("[smoke] timed out waiting for current page");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
