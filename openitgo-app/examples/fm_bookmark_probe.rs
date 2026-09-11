//! FM 书签跳转诊断探针：加载真实 settings（含真实书签分组），进入
//! FileManager 视图，等两栏 Ready 后取第一个目录书签，先导航到父目录，
//! 再按书签菜单相同的路径 navigate_to 跳转；分别截取 Loading 瞬间、
//! Ready 之后、以及合成 Ctrl+D 打开书签菜单三张图到 target/，
//! 然后 std::process::exit（绕开 on_exit 的 settings 落盘，不污染真实
//! FM 状态）。30 秒超时。
//!
//! 用法：`cargo run -p openitgo-app --example fm_bookmark_probe`

use openitgo_app::app::{ReaderApp, View};
use openitgo_app::views::file_manager_panel::{fallback_existing_dir, PanelLoadState};
use std::path::PathBuf;
use std::time::{Duration, Instant};

enum Phase {
    WaitInit,
    WaitAway,
    ShotLoading,
    WaitShotLoading,
    WaitReady,
    Settle(u32),
    WaitShotReady,
    OpenMenu(u32),
    WaitShotMenu,
    ClickPress,
    ClickRelease,
    SubSettle(u32),
    WaitShotSub,
    Done,
}

fn save_png(image: &egui::ColorImage, name: &str) {
    let path = format!("{}/../target/{name}", env!("CARGO_MANIFEST_DIR"));
    let img = image::RgbaImage::from_raw(
        image.width() as u32,
        image.height() as u32,
        image.as_raw().to_vec(),
    )
    .expect("screenshot buffer");
    img.save(&path).expect("save screenshot");
    eprintln!("[probe] screenshot saved to {path}");
}

fn request_shot(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
}

fn take_shot(ctx: &egui::Context) -> Option<egui::ColorImage> {
    let events = ctx.input(|i| i.events.clone());
    events.iter().find_map(|ev| {
        if let egui::Event::Screenshot { image, .. } = ev {
            Some((**image).clone())
        } else {
            None
        }
    })
}

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");

    let mut app = ReaderApp::default();
    app.current_view = View::FileManager;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenItGo fm bookmark probe",
        options,
        Box::new(|cc| {
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(ProbeApp {
                app,
                start: Instant::now(),
                phase: Phase::WaitInit,
                bookmark: None,
            }))
        }),
    )
}

struct ProbeApp {
    app: ReaderApp,
    start: Instant,
    phase: Phase,
    bookmark: Option<PathBuf>,
}

impl ProbeApp {
    fn panel_ready(&self) -> bool {
        matches!(
            self.app.file_manager_view.panels[0].state,
            PanelLoadState::Ready
        )
    }

    fn dump_panel(&mut self) {
        let p = &mut self.app.file_manager_view.panels[0];
        let rows = p.rows();
        eprintln!("[probe] ---- panel state after bookmark jump ----");
        eprintln!("[probe] dir            = {:?}", p.dir);
        eprintln!("[probe] entries.len()  = {}", p.entries.len());
        eprintln!("[probe] rows().len()   = {}", rows.len());
        eprintln!("[probe] filter         = {:?}", p.filter);
        eprintln!("[probe] view_mode      = {:?}", p.view_mode);
        eprintln!("[probe] col_shift      = {}", p.col_shift);
        eprintln!("[probe] columns        = {:?}", p.columns);
        for (i, e) in p.entries.iter().take(5).enumerate() {
            eprintln!(
                "[probe] entry[{i}]       name={:?} is_dir={} hidden={} size={:?}",
                e.name, e.is_dir, e.is_hidden, e.size
            );
        }
    }
}

impl eframe::App for ProbeApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // 打开书签菜单阶段：在 app.ui 之前请求（同 Ctrl+D 通道）。
        if matches!(self.phase, Phase::OpenMenu(4)) {
            self.app.file_manager_view.debug_open_bookmarks_menu();
        }
        // 合成点击「常用 ▸」必须在 app.ui 之前注入事件（帧内事件当帧
        // 消费，帧尾注入会被下一帧的 OS 输入覆盖）。
        if matches!(self.phase, Phase::ClickPress | Phase::ClickRelease) {
            let pressed = matches!(self.phase, Phase::ClickPress);
            let screen = ui.ctx().content_rect();
            // 菜单第 4 行（常用）中心 ≈ 降采样图 (100, 147) / (1000, 526)。
            let pos = egui::pos2(
                100.0 / 1000.0 * screen.width(),
                147.0 / 526.0 * screen.height(),
            );
            eprintln!("[probe] synthetic click at {pos:?} pressed={pressed}");
            ui.ctx().input_mut(|i| {
                i.events.push(egui::Event::PointerMoved(pos));
                i.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            });
            self.phase = if pressed {
                Phase::ClickRelease
            } else {
                Phase::SubSettle(4)
            };
        }
        self.app.ui(ui, frame);
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(100));

        match self.phase {
            Phase::WaitInit => {
                let ready = self
                    .app
                    .file_manager_view
                    .panels
                    .iter()
                    .all(|p| matches!(p.state, PanelLoadState::Ready));
                if ready {
                    let groups = &self.app.settings.fm_bookmark_groups;
                    for g in groups {
                        eprintln!("[probe] bookmark group {:?}: {:?}", g.name, g.items);
                    }
                    let bm = groups
                        .iter()
                        .flat_map(|g| g.items.iter())
                        .map(PathBuf::from)
                        .find(|p| p.is_dir());
                    match bm {
                        Some(bm) => {
                            let away = bm
                                .parent()
                                .map(PathBuf::from)
                                .filter(|p| p.is_dir())
                                .unwrap_or_else(|| PathBuf::from("C:\\"));
                            eprintln!("[probe] jumping to bookmark: {bm:?} (away: {away:?})");
                            self.app.file_manager_view.panels[0].navigate_to(away);
                            self.bookmark = Some(bm);
                            self.phase = Phase::WaitAway;
                        }
                        None => {
                            eprintln!("[probe] no dir bookmark found, abort");
                            std::process::exit(1);
                        }
                    }
                }
            }
            Phase::WaitAway => {
                if self.panel_ready() {
                    let bm = self.bookmark.clone().expect("bookmark");
                    let target = fallback_existing_dir(bm);
                    self.app.file_manager_view.panels[0].navigate_to(target);
                    self.phase = Phase::ShotLoading;
                }
            }
            Phase::ShotLoading => {
                // 跳转后第一帧（Loading 中）：截「瞬间」画面。
                request_shot(&ctx);
                self.phase = Phase::WaitShotLoading;
            }
            Phase::WaitShotLoading => {
                if let Some(img) = take_shot(&ctx) {
                    save_png(&img, "fm_probe_loading.png");
                    self.phase = Phase::WaitReady;
                }
            }
            Phase::WaitReady => {
                if self.panel_ready() {
                    self.dump_panel();
                    self.phase = Phase::Settle(10);
                }
            }
            Phase::Settle(n) => {
                if n == 0 {
                    request_shot(&ctx);
                    self.phase = Phase::WaitShotReady;
                } else {
                    self.phase = Phase::Settle(n - 1);
                }
            }
            Phase::WaitShotReady => {
                if let Some(img) = take_shot(&ctx) {
                    save_png(&img, "fm_probe_ready.png");
                    self.phase = Phase::OpenMenu(5);
                }
            }
            Phase::OpenMenu(n) => {
                if n == 0 {
                    request_shot(&ctx);
                    self.phase = Phase::WaitShotMenu;
                } else {
                    self.phase = Phase::OpenMenu(n - 1);
                }
            }
            Phase::WaitShotMenu => {
                if let Some(img) = take_shot(&ctx) {
                    save_png(&img, "fm_probe_menu.png");
                    self.phase = Phase::ClickPress;
                }
            }
            Phase::ClickPress | Phase::ClickRelease => {}
            Phase::SubSettle(n) => {
                if n == 0 {
                    request_shot(&ctx);
                    self.phase = Phase::WaitShotSub;
                } else {
                    self.phase = Phase::SubSettle(n - 1);
                }
            }
            Phase::WaitShotSub => {
                if let Some(img) = take_shot(&ctx) {
                    save_png(&img, "fm_probe_submenu.png");
                    self.phase = Phase::Done;
                    std::process::exit(0);
                }
            }
            Phase::Done => {}
        }

        if self.start.elapsed() > Duration::from_secs(30) {
            eprintln!("[probe] timeout");
            std::process::exit(1);
        }
    }
}
