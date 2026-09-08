//! 文件管理器冒烟：进入 FileManager 视图，等两栏目录列举完成（Ready）后
//! 打印成功并退出；30 秒超时。用法：
//! `cargo run -p openitgo-app --example fm_smoke -- [初始目录]`

use openitgo_app::app::{ReaderApp, View};
use openitgo_app::views::file_manager_panel::PanelLoadState;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");

    let mut app = ReaderApp::default();
    // 可选路径参数作为两栏初始目录（经 settings 恢复通道进入）。
    if let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) {
        if dir.is_dir() {
            app.settings.fm_dir_left = dir.display().to_string();
            app.settings.fm_dir_right = dir.display().to_string();
        }
    }
    app.current_view = View::FileManager;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo file manager smoke",
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
        // 空闲 egui 不重绘：主动轮询排空后台列举结果，否则冒烟会卡住。
        ctx.request_repaint_after(Duration::from_millis(100));

        let panels = &self.app.file_manager_view.panels;
        if panels
            .iter()
            .all(|p| matches!(p.state, PanelLoadState::Ready))
        {
            let counts: Vec<usize> = panels.iter().map(|p| p.entries.len()).collect();
            eprintln!(
                "[smoke] both panels ready after {:.1} s (entries: {} / {})",
                self.start.elapsed().as_secs_f64(),
                counts[0],
                counts[1]
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.start.elapsed() > Duration::from_secs(30) {
            let states: Vec<&str> = panels
                .iter()
                .map(|p| match &p.state {
                    PanelLoadState::Idle => "Idle",
                    PanelLoadState::Loading(_) => "Loading",
                    PanelLoadState::Ready => "Ready",
                    PanelLoadState::Failed(_) => "Failed",
                })
                .collect();
            eprintln!(
                "[smoke] timed out waiting for panels (states: {} / {})",
                states[0], states[1]
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
