use eframe::egui;
use egui_phosphor_icons::icons;
use openitgo_app::theme;
use openitgo_app::widgets::fluent::{self, ButtonKind};
use openitgo_storage::models::Theme;
use std::time::Duration;

#[derive(Default)]
struct GalleryApp {
    initialized: bool,
    dark: bool,
    enabled: bool,
    selected: bool,
    text: String,
    volume: f32,
}

impl eframe::App for GalleryApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.initialized {
            openitgo_app::fonts::setup_fonts(&ctx);
            theme::install(&ctx, &Theme::Windows);
            self.volume = 65.0;
            self.text = "可编辑文本".to_owned();
            self.initialized = true;
        }
        ctx.set_theme(if self.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });

        egui::Panel::top("gallery_header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Fluent 2 控件画廊");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("深色");
                    fluent::toggle(ui, &mut self.dark, "深色主题");
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(960.0);
                ui.add_space(16.0);
                fluent::setting_group(ui, "按钮", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if fluent::button(ui, "主要操作", ButtonKind::Primary).clicked() {
                            self.selected = !self.selected;
                        }
                        fluent::button(ui, "标准按钮", ButtonKind::Standard);
                        fluent::button(ui, "弱化按钮", ButtonKind::Subtle);
                        fluent::icon_button(
                            ui,
                            icons::MAGNIFYING_GLASS,
                            "搜索",
                            ButtonKind::Subtle,
                        )
                        .on_hover_text("搜索");
                        ui.add_enabled_ui(false, |ui| {
                            fluent::button(ui, "禁用按钮", ButtonKind::Standard);
                        });
                    });
                    if self.selected {
                        ui.label("主要操作已触发");
                    }
                });
                ui.add_space(16.0);
                fluent::setting_group(ui, "设置行", |ui| {
                    fluent::setting_row(
                        ui,
                        "启用新外观",
                        Some("使用统一的表面、间距和控件状态"),
                        |ui| {
                            fluent::toggle(ui, &mut self.enabled, "启用新外观");
                        },
                    );
                    ui.separator();
                    fluent::setting_row(
                        ui,
                        "默认音量",
                        Some("拖动滑块调整音量"),
                        |ui| {
                            fluent::slider(ui, egui::Slider::new(&mut self.volume, 0.0..=100.0));
                        },
                    );
                    ui.separator();
                    fluent::setting_row(
                        ui,
                        "显示名称",
                        Some("支持中文与英文混排"),
                        |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(220.0));
                        },
                    );
                    ui.separator();
                    fluent::setting_row(ui, "阅读模式", None, |ui| {
                        egui::ComboBox::from_id_salt("gallery_reading_mode")
                            .selected_text("适应页面")
                            .show_ui(ui, |ui| {
                                let _ = ui.selectable_label(true, "适应页面");
                                let _ = ui.selectable_label(false, "适应宽度");
                                let _ = ui.selectable_label(false, "原始大小");
                            });
                    });
                });
                ui.add_space(16.0);
                fluent::setting_group(ui, "导航", |ui| {
                    ui.set_max_width(240.0);
                    fluent::navigation_item(ui, "外观", true);
                    fluent::navigation_item(ui, "漫画", false);
                    fluent::navigation_item(ui, "文件管理器", false);
                });
                ui.add_space(24.0);
            });
        });
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenItGo Fluent Gallery",
        options,
        Box::new(|_cc| Ok(Box::<GalleryApp>::default())),
    )
}
