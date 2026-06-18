use rust_reader_core::models::{FitMode, ReadingMode};
use rust_reader_storage::models::{Settings, Theme};

#[derive(Default)]
pub struct SettingsView;

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

        ui.checkbox(
            &mut settings.invert_scroll,
            "反转滚轮方向（适用于 macOS 自然滚动）",
        );

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
