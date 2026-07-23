use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
use openitgo_storage::models::{EbookTheme, Settings, Theme, ToolbarDisplayMode};
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

        ui.horizontal(|ui| {
            ui.label("滚轮翻页阈值 (pt):");
            ui.add(egui::Slider::new(
                &mut settings.page_scroll_threshold,
                1.0..=40.0,
            ));
            ui.label("（滚一格不翻页就调小，容易误翻就调大）");
        });

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

        ui.horizontal(|ui| {
            ui.label("阅读栏透明度:");
            ui.add(
                egui::Slider::new(&mut settings.chrome_opacity, 0.2..=1.0)
                    .show_value(true)
                    .suffix(""),
            );
            ui.label("（阅读区背景与工具栏 / 进度条共用，透出窗口背后；1=不透明）");
        });

        ui.label("主题");
        egui::ComboBox::from_id_salt("theme")
            .selected_text(theme_label(settings.theme.clone()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.theme, Theme::System, "跟随系统");
                ui.selectable_value(&mut settings.theme, Theme::Light, "浅色");
                ui.selectable_value(&mut settings.theme, Theme::Dark, "深色");
            });

        ui.label("工具栏显示模式");
        egui::ComboBox::from_id_salt("toolbar_display_mode")
            .selected_text(toolbar_mode_label(settings.toolbar_display_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings.toolbar_display_mode,
                    ToolbarDisplayMode::IconAndText,
                    "图标 + 文字",
                );
                ui.selectable_value(
                    &mut settings.toolbar_display_mode,
                    ToolbarDisplayMode::IconOnly,
                    "仅图标",
                );
                ui.selectable_value(
                    &mut settings.toolbar_display_mode,
                    ToolbarDisplayMode::TextOnly,
                    "仅文字",
                );
            });

        ui.separator();
        ui.collapsing("电子书", |ui| {
            self.ebook_settings_ui(ui, settings);
        });

        ui.separator();
        ui.heading("快捷键");
        self.shortcut_editor(ui, &mut settings.shortcuts);
    }

    /// Renders only the ebook-related settings. Used when entering settings from
    /// the ebook reader so the user sees relevant options immediately.
    pub fn ebook_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.heading("电子书设置");
        ui.separator();
        self.ebook_settings_ui(ui, settings);
    }

    fn ebook_settings_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.label("阅读模式");
        egui::ComboBox::from_id_salt("ebook_mode")
            .selected_text(ebook_mode_label(settings.ebook.reading_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings.ebook.reading_mode,
                    EbookReadingMode::SinglePage,
                    "单页",
                );
                ui.selectable_value(
                    &mut settings.ebook.reading_mode,
                    EbookReadingMode::DoublePage,
                    "双页",
                );
                ui.selectable_value(
                    &mut settings.ebook.reading_mode,
                    EbookReadingMode::Scroll,
                    "连续滚动",
                );
            });

        ui.label("字体");
        let current_font = settings.ebook.font_family.clone();
        egui::ComboBox::from_id_salt("ebook_font_family")
            .selected_text(&current_font)
            .show_ui(ui, |ui| {
                const PRESETS: &[&str] = &[
                    "system-ui",
                    "serif",
                    "sans-serif",
                    "monospace",
                    "PingFang SC",
                    "Songti SC",
                    "Kaiti SC",
                    "Hiragino Sans GB",
                ];
                for preset in PRESETS {
                    ui.selectable_value(
                        &mut settings.ebook.font_family,
                        preset.to_string(),
                        *preset,
                    );
                }
                if !PRESETS.contains(&current_font.as_str()) {
                    ui.selectable_value(
                        &mut settings.ebook.font_family,
                        current_font.clone(),
                        current_font.clone(),
                    );
                }
            });

        ui.horizontal(|ui| {
            ui.label("字体大小:");
            ui.add(egui::Slider::new(&mut settings.ebook.font_size, 10..=72));
        });

        ui.horizontal(|ui| {
            ui.label("行间距:");
            ui.add(egui::Slider::new(&mut settings.ebook.line_height, 1.0..=3.0).step_by(0.05));
        });

        ui.horizontal(|ui| {
            ui.label("页边距（水平）:");
            ui.add(egui::Slider::new(
                &mut settings.ebook.margin_horizontal,
                0..=200,
            ));
        });

        ui.horizontal(|ui| {
            ui.label("页边距（垂直）:");
            ui.add(egui::Slider::new(
                &mut settings.ebook.margin_vertical,
                0..=200,
            ));
        });

        ui.label("主题");
        egui::ComboBox::from_id_salt("ebook_theme")
            .selected_text(ebook_theme_label(settings.ebook.theme))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Light, "白天");
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Dark, "夜晚");
                ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Sepia, "羊皮纸");
            });

        ui.checkbox(&mut settings.ebook.enable_page_animation, "翻页动画");
        ui.checkbox(
            &mut settings.ebook.invert_scroll,
            "反转滚轮方向（适用于 macOS 自然滚动）",
        );
    }

    fn shortcut_editor(
        &mut self,
        ui: &mut egui::Ui,
        shortcuts: &mut openitgo_storage::models::Shortcuts,
    ) {
        type ShortcutGetter = fn(&mut openitgo_storage::models::Shortcuts) -> &mut Vec<String>;
        let actions: &[(&str, ShortcutGetter)] = &[
            ("下一页", |s| &mut s.next_page),
            ("上一页", |s| &mut s.prev_page),
            ("向下翻页", |s| &mut s.page_down),
            ("向上翻页", |s| &mut s.page_up),
            ("首页", |s| &mut s.first_page),
            ("末页", |s| &mut s.last_page),
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

fn toolbar_mode_label(mode: ToolbarDisplayMode) -> &'static str {
    match mode {
        ToolbarDisplayMode::IconAndText => "图标 + 文字",
        ToolbarDisplayMode::IconOnly => "仅图标",
        ToolbarDisplayMode::TextOnly => "仅文字",
    }
}

fn ebook_mode_label(mode: EbookReadingMode) -> &'static str {
    match mode {
        EbookReadingMode::SinglePage => "单页",
        EbookReadingMode::DoublePage => "双页",
        EbookReadingMode::Scroll => "连续滚动",
    }
}

fn ebook_theme_label(theme: EbookTheme) -> &'static str {
    match theme {
        EbookTheme::Light => "白天",
        EbookTheme::Dark => "夜晚",
        EbookTheme::Sepia => "羊皮纸",
    }
}
