//! Media smoke: opens a media file with the full app UI (HWND child window
//! hosting mpv on Windows, CAOpenGLLayer on macOS) and exits once playback
//! position advances past 200ms (or after 30s timeout / on playback error).
//! Usage: cargo run -p openitgo-app --example media_smoke -- <media-file>

use openitgo_app::app::{PendingMediaOpen, ReaderApp};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example media_smoke -- <media-file>");

    let mut app = ReaderApp::default();
    app.pending_media_open = Some(PendingMediaOpen {
        path,
        force_start: true,
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo media smoke",
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
        // An idle egui app does not repaint, so poll explicitly (see ui_smoke).
        ctx.request_repaint_after(Duration::from_millis(100));

        if let Some(open) = self.app.media_view.open.as_ref() {
            if let Some(err) = open.last.error.as_ref() {
                eprintln!("[media smoke] playback error: {err}");
                std::process::exit(1);
            }
            if open.last.position_ms > 200 {
                eprintln!(
                    "[media smoke] playing: pos={}ms dur={:?} video={} after {:.1} s",
                    open.last.position_ms,
                    open.last.duration_ms,
                    open.last.has_video,
                    self.start.elapsed().as_secs_f64()
                );
                std::process::exit(0);
            }
        }

        if self.start.elapsed() > Duration::from_secs(30) {
            eprintln!("[media smoke] timed out waiting for playback");
            std::process::exit(1);
        }
    }
}
