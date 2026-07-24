use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
use openitgo_storage::models::{
    ComicEndAction, EbookTheme, MediaEndAction, Settings, Theme, ToolbarDisplayMode,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Comic,
    Ebook,
    Media,
    Performance,
    Shortcuts,
}

impl SettingsTab {
    const ALL: [(SettingsTab, &'static str); 6] = [
        (SettingsTab::Appearance, "外观"),
        (SettingsTab::Comic, "漫画"),
        (SettingsTab::Ebook, "电子书"),
        (SettingsTab::Media, "媒体"),
        (SettingsTab::Performance, "性能"),
        (SettingsTab::Shortcuts, "快捷键"),
    ];
}

#[derive(Default)]
pub struct SettingsView {
    pub tab: SettingsTab,
    shortcut_add_buffer: HashMap<&'static str, String>,
}

impl SettingsView {
    pub fn focus_tab(&mut self, tab: SettingsTab) {
        self.tab = tab;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.heading(egui::RichText::new("设置").size(22.0).strong());
        ui.add_space(10.0);

        self.tab = crate::theme::tabbed_page(ui, &SettingsTab::ALL, self.tab, |ui, tab| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match tab {
                        SettingsTab::Appearance => self.appearance_ui(ui, settings),
                        SettingsTab::Comic => self.comic_ui(ui, settings),
                        SettingsTab::Ebook => self.ebook_settings_ui(ui, settings),
                        SettingsTab::Media => self.media_ui(ui, settings),
                        SettingsTab::Performance => self.performance_ui(ui, settings),
                        SettingsTab::Shortcuts => self.shortcut_editor(ui, &mut settings.shortcuts),
                    }
                });
        });
    }

    fn appearance_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.horizontal(|ui| {
            ui.label("主题");
            egui::ComboBox::from_id_salt("theme")
                .selected_text(theme_label(settings.theme.clone()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.theme, Theme::System, "跟随系统");
                    ui.selectable_value(&mut settings.theme, Theme::Light, "浅色");
                    ui.selectable_value(&mut settings.theme, Theme::Dark, "深色");
                });
            ui.add_space(8.0);
            let dark = crate::theme::dark_visuals();
            let light = crate::theme::light_visuals();
            crate::theme::theme_swatch(ui, dark.panel_fill, dark.hyperlink_color)
                .on_hover_text("深色预览");
            crate::theme::theme_swatch(ui, light.panel_fill, light.hyperlink_color)
                .on_hover_text("浅色预览");
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

        ui.checkbox(&mut settings.show_toolbar, "显示工具栏");
        ui.checkbox(&mut settings.show_statusbar, "显示状态栏 / 进度条");

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
        });
        hint(
            ui,
            "阅读区背景与工具栏 / 进度条共用，透出窗口背后；1 = 不透明",
        );
    }

    fn comic_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
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

        ui.checkbox(&mut settings.double_page, "默认双页");

        ui.label("默认缩放 / 适应");
        egui::ComboBox::from_id_salt("fit")
            .selected_text(fit_label(settings.default_fit))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.default_fit, FitMode::Height, "适应高度");
                ui.selectable_value(&mut settings.default_fit, FitMode::Width, "适应宽度");
                ui.selectable_value(&mut settings.default_fit, FitMode::Page, "适应页面");
                ui.selectable_value(&mut settings.default_fit, FitMode::Original, "原始大小");
            });

        ui.checkbox(&mut settings.enable_page_animation, "翻页动画");
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
        });
        hint(ui, "滚一格不翻页就调小，容易误翻就调大");

        ui.horizontal(|ui| {
            ui.label("宽页阈值（宽高比）:");
            ui.add(egui::Slider::new(&mut settings.wide_page_threshold, 1.0..=2.0).step_by(0.05));
        });

        ui.label("到末页后再翻下一页");
        egui::ComboBox::from_id_salt("comic_end_action")
            .selected_text(comic_end_action_label(settings.comic_end_action))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings.comic_end_action,
                    ComicEndAction::DoNothing,
                    comic_end_action_label(ComicEndAction::DoNothing),
                );
                ui.selectable_value(
                    &mut settings.comic_end_action,
                    ComicEndAction::WrapToFirst,
                    comic_end_action_label(ComicEndAction::WrapToFirst),
                );
                ui.selectable_value(
                    &mut settings.comic_end_action,
                    ComicEndAction::NextSibling,
                    comic_end_action_label(ComicEndAction::NextSibling),
                );
            });
        hint(
            ui,
            "「打开下一个」：当前是压缩包/PDF 则找同目录下一个漫画文件；当前是图片文件夹则找同级下一个文件夹",
        );
    }

    fn media_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        ui.label("播放到结尾");
        egui::ComboBox::from_id_salt("media_end_action")
            .selected_text(media_end_action_label(settings.media_end_action))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut settings.media_end_action,
                    MediaEndAction::Stop,
                    media_end_action_label(MediaEndAction::Stop),
                );
                ui.selectable_value(
                    &mut settings.media_end_action,
                    MediaEndAction::NextInDir,
                    media_end_action_label(MediaEndAction::NextInDir),
                );
            });
        hint(
            ui,
            "「自动下一集」按同目录自然排序续播；开启循环播放时不会触发",
        );

        ui.horizontal(|ui| {
            ui.label("默认音量:");
            let mut vol = settings.media_volume as f32;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=100.0)).changed() {
                settings.media_volume = vol as f64;
            }
        });

        ui.horizontal(|ui| {
            ui.label("默认倍速:");
            let mut speed = settings.media_speed as f32;
            if ui
                .add(egui::Slider::new(&mut speed, 0.25..=4.0).step_by(0.05))
                .changed()
            {
                settings.media_speed = speed as f64;
            }
        });

        ui.horizontal(|ui| {
            ui.label("默认音频输出:");
            ui.add(
                egui::TextEdit::singleline(&mut settings.media_audio_device)
                    .hint_text("空 = 自动")
                    .desired_width(220.0),
            );
        });
        hint(
            ui,
            "填写 mpv 设备名；留空为系统默认。无效设备会在打开媒体时回退到自动。",
        );
    }

    fn performance_ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
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
            &mut settings.compress_images,
            "DXT5 纹理压缩（节省显存，但打开时 CPU 占用高）",
        );

        ui.horizontal(|ui| {
            ui.label("解码线程数:");
            ui.add(egui::Slider::new(&mut settings.decode_threads, 0..=16).text("0=自动"));
        });
        hint(ui, "解码线程数重启后生效");
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

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().size(12.5));
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

fn comic_end_action_label(action: ComicEndAction) -> &'static str {
    match action {
        ComicEndAction::DoNothing => "什么都不做",
        ComicEndAction::WrapToFirst => "回到第一页",
        ComicEndAction::NextSibling => "打开下一个漫画",
    }
}

fn media_end_action_label(action: MediaEndAction) -> &'static str {
    match action {
        MediaEndAction::Stop => "停止",
        MediaEndAction::NextInDir => "自动播放下一集",
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
