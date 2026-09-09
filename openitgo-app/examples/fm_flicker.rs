//! 文件管理器闪烁诊断：进入 FileManager 视图，等两栏 Ready 后强制 5 秒
//! 连续重绘（request_repaint 每帧，模拟真实桌面事件流），逐帧记录两栏
//! 视口高/滚动偏移/fm_dual_ratio/目录快照，检测 A/B 振荡。
//! 正常：所有观测量 5 秒内恒定。振荡：某个量逐帧交替。
//! 用法：`cargo run -p openitgo-app --example fm_flicker -- [初始目录] [single|singlex|dual]`
//! （single=单栏带预览，singlex=单栏无预览，dual=双栏；缺省读用户 settings 布局）
//!
//! 历史：用于定位「打开文件管理器后界面疯狂闪烁」——根因是
//! `FsPanel::poll()` 用 `mem::take` 无条件重置 state 为 Idle，Ready 面板
//! 每帧被打回 Idle，触发 app 侧恢复逻辑每 3 帧重列目录（vp 周期 3 振荡
//! [594,594,0]）。修复见 file_manager_panel.rs 的
//! `poll_preserves_non_loading_state` 回归测试。

use openitgo_app::app::{ReaderApp, View};
use openitgo_app::views::file_manager_panel::PanelLoadState;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    let mut app = ReaderApp::default();
    let mode = std::env::args().nth(2).unwrap_or_default();
    if mode.starts_with("single") {
        app.file_manager_view.layout = openitgo_app::views::file_manager::PanelLayout::Single {
            preview_open: mode == "single",
        };
    } else if mode.starts_with("dual") {
        app.file_manager_view.layout =
            openitgo_app::views::file_manager::PanelLayout::Dual { ratio: 0.5 };
    }
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
        "OpenItGo file manager flicker probe",
        options,
        Box::new(|cc| {
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(FlickerApp {
                app,
                measure_start: None,
                frames: 0,
                last_obs: None,
                changes: 0,
            }))
        }),
    )
}

/// 逐帧观测量（任何一项逐帧变化 = 振荡源）。
#[derive(Debug, Clone, PartialEq)]
struct Obs {
    vp: [f32; 2],
    scroll: [f32; 2],
    ratio: f32,
    layout: String,
    dir_left: String,
    error: Option<String>,
}

struct FlickerApp {
    app: ReaderApp,
    measure_start: Option<Instant>,
    frames: u32,
    last_obs: Option<Obs>,
    changes: u32,
}

impl FlickerApp {
    fn observe(&self) -> Obs {
        let fm = &self.app.file_manager_view;
        let snap = fm.snapshot();
        Obs {
            vp: [
                fm.panels[0].last_viewport_height,
                fm.panels[1].last_viewport_height,
            ],
            scroll: [
                fm.panels[0].last_scroll_offset,
                fm.panels[1].last_scroll_offset,
            ],
            ratio: self.app.settings.fm_dual_ratio,
            layout: snap.layout,
            dir_left: snap.dir_left,
            error: self.app.error_message.clone(),
        }
    }
}

impl eframe::App for FlickerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let ctx = ui.ctx();
        let ready = self
            .app
            .file_manager_view
            .panels
            .iter()
            .all(|p| matches!(p.state, PanelLoadState::Ready));
        match self.measure_start {
            None => {
                if ready {
                    self.measure_start = Some(Instant::now());
                    self.last_obs = Some(self.observe());
                    eprintln!("[flicker] ready, forcing 5s of continuous repaints…");
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Some(start) => {
                self.frames += 1;
                let obs = self.observe();
                if self.frames <= 40 {
                    eprintln!("[flicker] f{} vp={:?}", self.frames, obs.vp);
                }
                if self.last_obs.as_ref() != Some(&obs) {
                    self.changes += 1;
                    if self.changes <= 20 {
                        eprintln!(
                            "[flicker] CHANGE #{} at frame {} (+{:.2}s):\n  prev: {:?}\n  now:  {:?}",
                            self.changes,
                            self.frames,
                            start.elapsed().as_secs_f64(),
                            self.last_obs,
                            obs
                        );
                    }
                    self.last_obs = Some(obs);
                }
                if start.elapsed() >= Duration::from_secs(5) {
                    eprintln!(
                        "[flicker] RESULT: {} frames, {} observable changes in 5.0s",
                        self.frames, self.changes
                    );
                    std::process::exit(0);
                }
                // 强制连续重绘，模拟真实桌面的事件驱动帧流。
                ctx.request_repaint();
            }
        }
    }
}
