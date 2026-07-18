//! Diagnostic probe for ebook menu parking (#52): opens an ebook, waits for
//! the reader view, then toggles the webview hidden/shown on a timer so the
//! parked state can be screenshot-verified (menu overlay itself is egui and
//! hard to synthesize headlessly).
//! Usage: cargo run -p openitgo-app --example probe_ebook_menu -- <ebook-file>

use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::ebook::Ebook;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example probe_ebook_menu -- <ebook-file>");

    let app = ReaderApp {
        ebook_opener: Some(AsyncOpener::<Ebook>::open(path.clone(), |p| {
            openitgo_parser::parse_ebook(p).map_err(|e| e.to_string())
        })),
        current_view: View::Loading(path),
        ..Default::default()
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenItGo ebook parking probe",
        options,
        Box::new(|_cc| {
            Ok(Box::new(ParkingProbe {
                app,
                start: Instant::now(),
                phase: 0,
            }))
        }),
    )
}

struct ParkingProbe {
    app: ReaderApp,
    start: Instant,
    phase: u8,
}

impl eframe::App for ParkingProbe {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(100));
        if self.app.ebook_view.open.is_none() {
            return;
        }
        match self.phase {
            0 if self.start.elapsed() > Duration::from_secs(3) => {
                self.app.ebook_view.set_webview_hidden(true);
                eprintln!(
                    "[probe] hidden={} — 正文区应为纯色背景（截图验证点 1）",
                    self.app.ebook_view.webview_hidden()
                );
                self.phase = 1;
            }
            1 if self.start.elapsed() > Duration::from_secs(6) => {
                self.app.ebook_view.set_webview_hidden(false);
                eprintln!(
                    "[probe] hidden={} — 正文应恢复显示（截图验证点 2）",
                    self.app.ebook_view.webview_hidden()
                );
                self.phase = 2;
            }
            2 if self.start.elapsed() > Duration::from_secs(9) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }
}
