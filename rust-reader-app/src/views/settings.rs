use rust_reader_core::models::{FitMode, ReadingMode};
use rust_reader_storage::models::{Settings, Theme};
use std::collections::HashMap;

#[derive(Default)]
pub struct SettingsView {
    shortcut_add_buffer: HashMap<&'static str, String>,
}

impl SettingsView {
    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.heading("设置");

        ui.label("默认阅读模式");
        egui::ComboBox::from_id_salt("mode")
            .selected_text(mode_label(settings.default_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings.default_mode,
                    ReadingMode::Ltr,
                    "国漫（左→右）",
                );
                ui.selectable_value(
                    &mut settings.default_mode,
                    ReadingMode::Rtl,
                    "日漫（右→左）",
                );
                ui.selectable_value(
                    &mut settings.default_mode,
                    ReadingMode::Webtoon,
                    "韩漫（上→下）",
                );
            });

        ui.horizontal(|ui| {
            ui.label("缓存大小 (MB):");
            ui.add(egui::Slider::new(&mut settings.cache_size_mb, 100..=4096));
        });

        ui.horizontal(|ui| {
            ui.label("真实图片缓存页数:");
            ui.add(egui::Slider::new(
                &mut settings.real_image_cache_pages,
                1..=200,
            ));
        });

        ui.checkbox(
            &mut settings.invert_scroll,
            "反转滚轮方向（适用于 macOS 自然滚动）",
        );

        ui.checkbox(
            &mut settings.compress_images,
            "DXT5 纹理压缩（节省显存，但打开时 CPU 占用高）",
        );

        ui.horizontal(|ui| {
            ui.label("解码线程数:");
            ui.add(egui::Slider::new(&mut settings.decode_threads, 0..=16).text("0=自动"));
            ui.label("（重启后生效）");
        });

        ui.horizontal(|ui| {
            ui.label("宽页阈值（宽高比）:");
            ui.add(egui::Slider::new(&mut settings.wide_page_threshold, 1.0..=2.0).step_by(0.05));
        });

        ui.checkbox(&mut settings.enable_page_animation, "翻页动画");

        ui.label("默认缩放/适应");
        egui::ComboBox::from_id_salt("fit")
            .selected_text(fit_label(settings.default_fit))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.default_fit, FitMode::Height, "适应高度");
                ui.selectable_value(&mut settings.default_fit, FitMode::Width, "适应宽度");
                ui.selectable_value(&mut settings.default_fit, FitMode::Page, "适应页面");
                ui.selectable_value(&mut settings.default_fit, FitMode::Original, "原始大小");
            });

        ui.horizontal(|ui| {
            ui.label("阅读背景色:");
            ui.color_edit_button_srgb(&mut settings.background_color);
        });

        ui.label("主题");
        egui::ComboBox::from_id_salt("theme")
            .selected_text(theme_label(settings.theme.clone()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.theme, Theme::System, "跟随系统");
                ui.selectable_value(&mut settings.theme, Theme::Light, "浅色");
                ui.selectable_value(&mut settings.theme, Theme::Dark, "深色");
            });

        ui.separator();
        ui.heading("快捷键");
        self.shortcut_editor(ui, &mut settings.shortcuts);
    }

    fn shortcut_editor(
        &mut self,
        ui: &mut egui::Ui,
        shortcuts: &mut rust_reader_storage::models::Shortcuts,
    ) {
        type ShortcutGetter = fn(&mut rust_reader_storage::models::Shortcuts) -> &mut Vec<String>;
        let actions: &[(&str, ShortcutGetter)] = &[
            ("下一页", |s| &mut s.next_page),
            ("上一页", |s| &mut s.prev_page),
            ("向下翻页", |s| &mut s.page_down),
            ("向上翻页", |s| &mut s.page_up),
            ("全屏", |s| &mut s.fullscreen),
            ("适应页面", |s| &mut s.fit_page),
            ("适应宽度", |s| &mut s.fit_width),
            ("适应高度", |s| &mut s.fit_height),
            ("放大", |s| &mut s.zoom_in),
            ("缩小", |s| &mut s.zoom_out),
            ("返回书架", |s| &mut s.back_to_library),
        ];
        for &(label, getter) in actions {
            let bindings = getter(shortcuts);
            ui.horizontal(|ui| {
                ui.label(label);
                for i in (0..bindings.len()).rev() {
                    let key = &bindings[i];
                    if ui.button(format!("{} ✕", key)).clicked() {
                        bindings.remove(i);
                    }
                }
                let buffer = self.shortcut_add_buffer.entry(label).or_default();
                ui.add(egui::TextEdit::singleline(buffer).hint_text("添加按键"));
                if ui.button("+").clicked() && !buffer.trim().is_empty() {
                    bindings.push(buffer.trim().to_string());
                    buffer.clear();
                }
            });
        }
    }
}

fn fit_label(fit: FitMode) -> &'static str {
    match fit {
        FitMode::Height => "适应高度",
        FitMode::Width => "适应宽度",
        FitMode::Page => "适应页面",
        FitMode::Original => "原始大小",
    }
}

fn mode_label(mode: ReadingMode) -> &'static str {
    match mode {
        ReadingMode::Ltr => "国漫（左→右）",
        ReadingMode::Rtl => "日漫（右→左）",
        ReadingMode::Webtoon => "韩漫（上→下）",
    }
}

fn theme_label(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "跟随系统",
        Theme::Light => "浅色",
        Theme::Dark => "深色",
    }
}
