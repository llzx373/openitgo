use super::{
    comic_end_action_label, ebook_mode_label, ebook_theme_label, fit_label, media_end_action_label,
    mode_label, theme_label, toolbar_mode_label, SettingsTab, SettingsView,
};
use crate::theme::tokens::{size, spacing, text};
use crate::widgets::fluent::{self as fw, ButtonKind};
use egui::{RichText, Ui};
use egui_phosphor_icons::icons;
use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
use openitgo_storage::models::{
    ComicEndAction, EbookTheme, FmButton, MediaEndAction, PasswordBook, Settings, Theme,
    ToolbarDisplayMode,
};

pub(super) fn ui(
    view: &mut SettingsView,
    ui: &mut Ui,
    settings: &mut Settings,
    password_book: &mut PasswordBook,
) -> bool {
    let narrow = ui.available_width() < 720.0;
    if narrow {
        egui::ComboBox::from_id_salt("fluent_settings_navigation")
            .selected_text(tab_label(view.tab))
            .show_ui(ui, |ui| {
                for (tab, label) in SettingsTab::ALL {
                    ui.selectable_value(&mut view.tab, tab, label);
                }
            });
        ui.add_space(spacing::LG);
        content(view, ui, settings, password_book)
    } else {
        egui::Panel::left("fluent_settings_navigation")
            .exact_size(size::SETTINGS_NAV_WIDTH + spacing::XL)
            .show(ui, |ui| {
                ui.add_space(spacing::XS);
                ui.label(
                    RichText::new("设置")
                        .font(egui::FontId::proportional(text::TITLE))
                        .strong(),
                );
                ui.add_space(spacing::LG);
                for (tab, label) in SettingsTab::ALL {
                    if fw::navigation_item(ui, label, view.tab == tab).clicked() {
                        view.tab = tab;
                    }
                }
            });
        egui::CentralPanel::default()
            .show(ui, |ui| content(view, ui, settings, password_book))
            .inner
    }
}

fn content(
    view: &mut SettingsView,
    ui: &mut Ui,
    settings: &mut Settings,
    password_book: &mut PasswordBook,
) -> bool {
    let width = ui.available_width().min(size::SETTINGS_CONTENT_MAX);
    ui.set_width(width);
    ui.label(
        RichText::new(tab_label(view.tab))
            .font(egui::FontId::proportional(text::TITLE))
            .strong(),
    );
    ui.add_space(spacing::LG);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(width);
            match view.tab {
                SettingsTab::Appearance => appearance(view, ui, settings),
                SettingsTab::Comic => comic(ui, settings),
                SettingsTab::Ebook => ebook(ui, settings),
                SettingsTab::Media => media(ui, settings),
                SettingsTab::Archive => {
                    return archive(view, ui, settings, password_book);
                }
                SettingsTab::FileManager => file_manager(view, ui, settings),
                SettingsTab::FileAssoc => file_assoc(view, ui),
                SettingsTab::Performance => performance(ui, settings),
                SettingsTab::Shortcuts => shortcuts(view, ui, settings),
            }
            false
        })
        .inner
}

fn appearance(view: &mut SettingsView, ui: &mut Ui, settings: &mut Settings) {
    group(ui, "主题", |ui| {
        row(
            ui,
            "主题",
            Some("Windows 主题使用 Fluent 2；跟随系统会自动切换明暗"),
            |ui| {
                egui::ComboBox::from_id_salt("theme")
                    .selected_text(theme_label(settings.theme.clone()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.theme, Theme::System, "跟随系统");
                        ui.selectable_value(&mut settings.theme, Theme::Light, "浅色");
                        ui.selectable_value(&mut settings.theme, Theme::Dark, "深色");
                        ui.selectable_value(&mut settings.theme, Theme::Windows, "Windows");
                        ui.selectable_value(&mut settings.theme, Theme::MacOS, "macOS");
                    });
            },
        );
        separator(ui);
        row(ui, "主题色预览", None, |ui| {
            ui.horizontal(|ui| {
                let dark = crate::theme::dark_visuals();
                let light = crate::theme::light_visuals();
                let windows = crate::theme::windows_visuals();
                let macos = crate::theme::macos_visuals();
                crate::theme::theme_swatch(ui, dark.panel_fill, dark.hyperlink_color)
                    .on_hover_text("深色预览");
                crate::theme::theme_swatch(ui, light.panel_fill, light.hyperlink_color)
                    .on_hover_text("浅色预览");
                crate::theme::theme_swatch(ui, windows.panel_fill, windows.hyperlink_color)
                    .on_hover_text("Windows 预览");
                crate::theme::theme_swatch(ui, macos.panel_fill, macos.hyperlink_color)
                    .on_hover_text("macOS 预览");
            });
        });
    });
    group(ui, "工具栏", |ui| {
        row(ui, "显示模式", None, |ui| {
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
        });
        separator(ui);
        row(ui, "显示工具栏", None, |ui| {
            fw::toggle(ui, &mut settings.show_toolbar, "显示工具栏");
        });
        separator(ui);
        row(ui, "显示状态栏 / 进度条", None, |ui| {
            fw::toggle(ui, &mut settings.show_statusbar, "显示状态栏 / 进度条");
        });
    });
    group(ui, "阅读界面", |ui| {
        row(ui, "阅读背景色", None, |ui| {
            ui.color_edit_button_srgb(&mut settings.background_color);
        });
        separator(ui);
        row(
            ui,
            "阅读栏透明度",
            Some("阅读区背景与工具栏 / 进度条共用；1 = 不透明"),
            |ui| {
                fw::slider(
                    ui,
                    egui::Slider::new(&mut settings.chrome_opacity, 0.2..=1.0).show_value(true),
                );
            },
        );
    });
    let _ = view;
}

fn comic(ui: &mut Ui, settings: &mut Settings) {
    group(ui, "阅读方式", |ui| {
        row(ui, "默认阅读模式", None, |ui| {
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
        });
        separator(ui);
        row(ui, "默认缩放 / 适应", None, |ui| {
            egui::ComboBox::from_id_salt("fit")
                .selected_text(fit_label(settings.default_fit))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.default_fit, FitMode::Height, "适应高度");
                    ui.selectable_value(&mut settings.default_fit, FitMode::Width, "适应宽度");
                    ui.selectable_value(&mut settings.default_fit, FitMode::Page, "适应页面");
                    ui.selectable_value(&mut settings.default_fit, FitMode::Original, "原始大小");
                });
        });
        separator(ui);
        row(ui, "默认双页", None, |ui| {
            fw::toggle(ui, &mut settings.double_page, "默认双页");
        });
        separator(ui);
        row(ui, "打开漫画时自动加入书架", None, |ui| {
            fw::toggle(
                ui,
                &mut settings.auto_add_to_library,
                "打开漫画时自动加入书架",
            );
        });
    });
    group(ui, "翻页", |ui| {
        row(ui, "翻页动画", None, |ui| {
            fw::toggle(ui, &mut settings.enable_page_animation, "漫画翻页动画");
        });
        separator(ui);
        row(
            ui,
            "反转滚轮方向",
            Some("适用于 macOS 自然滚动"),
            |ui| {
                fw::toggle(ui, &mut settings.invert_scroll, "漫画反转滚轮方向");
            },
        );
        separator(ui);
        row(
            ui,
            "滚轮翻页阈值",
            Some("滚一格不翻页就调小，容易误翻就调大"),
            |ui| {
                fw::slider(
                    ui,
                    egui::Slider::new(&mut settings.page_scroll_threshold, 1.0..=40.0)
                        .suffix(" pt"),
                );
            },
        );
        separator(ui);
        row(
            ui,
            "宽页阈值",
            Some("宽高比达到此值的页面在双页模式下独占一页"),
            |ui| {
                fw::slider(
                    ui,
                    egui::Slider::new(&mut settings.wide_page_threshold, 1.0..=2.0).step_by(0.05),
                );
            },
        );
        separator(ui);
        row(ui, "到末页后再翻下一页", None, |ui| {
            egui::ComboBox::from_id_salt("comic_end_action")
                .selected_text(comic_end_action_label(settings.comic_end_action))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut settings.comic_end_action,
                        ComicEndAction::DoNothing,
                        "什么都不做",
                    );
                    ui.selectable_value(
                        &mut settings.comic_end_action,
                        ComicEndAction::WrapToFirst,
                        "回到第一页",
                    );
                    ui.selectable_value(
                        &mut settings.comic_end_action,
                        ComicEndAction::NextSibling,
                        "打开下一个漫画",
                    );
                });
        });
    });
}

fn ebook(ui: &mut Ui, settings: &mut Settings) {
    group(ui, "版式", |ui| {
        row(ui, "阅读模式", None, |ui| {
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
        });
        separator(ui);
        row(ui, "字体", None, |ui| {
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
        });
        separator(ui);
        row(ui, "主题", None, |ui| {
            egui::ComboBox::from_id_salt("ebook_theme")
                .selected_text(ebook_theme_label(settings.ebook.theme))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Light, "白天");
                    ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Dark, "夜晚");
                    ui.selectable_value(&mut settings.ebook.theme, EbookTheme::Sepia, "羊皮纸");
                });
        });
    });
    group(ui, "文字与页面", |ui| {
        row(ui, "字体大小", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.ebook.font_size, 10..=72).suffix(" pt"),
            );
        });
        separator(ui);
        row(ui, "行间距", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.ebook.line_height, 1.0..=3.0).step_by(0.05),
            );
        });
        separator(ui);
        row(ui, "水平页边距", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.ebook.margin_horizontal, 0..=200).suffix(" px"),
            );
        });
        separator(ui);
        row(ui, "垂直页边距", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.ebook.margin_vertical, 0..=200).suffix(" px"),
            );
        });
    });
    group(ui, "阅读行为", |ui| {
        row(ui, "翻页动画", None, |ui| {
            fw::toggle(
                ui,
                &mut settings.ebook.enable_page_animation,
                "电子书翻页动画",
            );
        });
        separator(ui);
        row(
            ui,
            "反转滚轮方向",
            Some("适用于 macOS 自然滚动"),
            |ui| {
                fw::toggle(ui, &mut settings.ebook.invert_scroll, "电子书反转滚轮方向");
            },
        );
    });
}

fn media(ui: &mut Ui, settings: &mut Settings) {
    group(ui, "播放", |ui| {
        row(
            ui,
            "播放到结尾",
            Some("自动下一集按同目录自然排序续播"),
            |ui| {
                egui::ComboBox::from_id_salt("media_end_action")
                    .selected_text(media_end_action_label(settings.media_end_action))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut settings.media_end_action,
                            MediaEndAction::Stop,
                            "停止",
                        );
                        ui.selectable_value(
                            &mut settings.media_end_action,
                            MediaEndAction::NextInDir,
                            "自动播放下一集",
                        );
                    });
            },
        );
        separator(ui);
        row(ui, "默认音量", None, |ui| {
            let mut volume = settings.media_volume as f32;
            if fw::slider(ui, egui::Slider::new(&mut volume, 0.0..=100.0)).changed() {
                settings.media_volume = volume as f64;
            }
        });
        separator(ui);
        row(ui, "默认倍速", None, |ui| {
            let mut speed = settings.media_speed as f32;
            if fw::slider(
                ui,
                egui::Slider::new(&mut speed, 0.25..=4.0)
                    .step_by(0.05)
                    .suffix("×"),
            )
            .changed()
            {
                settings.media_speed = speed as f64;
            }
        });
        separator(ui);
        row(
            ui,
            "默认音频输出",
            Some("填写 mpv 设备名；留空为系统默认"),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut settings.media_audio_device)
                        .hint_text("自动")
                        .desired_width(220.0),
                );
            },
        );
    });
}

fn performance(ui: &mut Ui, settings: &mut Settings) {
    group(ui, "图片缓存", |ui| {
        row(ui, "缓存大小", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.cache_size_mb, 100..=4096).suffix(" MB"),
            );
        });
        separator(ui);
        row(ui, "真实图片缓存页数", None, |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.real_image_cache_pages, 1..=200),
            );
        });
        separator(ui);
        row(
            ui,
            "DXT5/BC3 纹理压缩",
            Some("使用原生 GPU 格式；打开时 CPU 占用较高"),
            |ui| {
                fw::toggle(ui, &mut settings.compress_images, "DXT5/BC3 纹理压缩");
            },
        );
        separator(ui);
        row(
            ui,
            "解码线程数",
            Some("0 = 自动；重启后生效"),
            |ui| {
                fw::slider(ui, egui::Slider::new(&mut settings.decode_threads, 0..=16));
            },
        );
    });
}

fn file_manager(view: &mut SettingsView, ui: &mut Ui, settings: &mut Settings) {
    group(ui, "删除", |ui| {
        row(
            ui,
            "删除方式",
            Some("Shift+Del 临时使用另一种删除方式"),
            |ui| {
                egui::ComboBox::from_id_salt("fm_delete_mode")
                    .selected_text(if settings.fm_delete_mode == "permanent" {
                        "永久删除"
                    } else {
                        "移入回收站"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut settings.fm_delete_mode,
                            "trash".to_string(),
                            "移入回收站",
                        );
                        ui.selectable_value(
                            &mut settings.fm_delete_mode,
                            "permanent".to_string(),
                            "永久删除（无法恢复）",
                        );
                    });
            },
        );
        separator(ui);
        row(ui, "删除前确认", None, |ui| {
            fw::toggle(ui, &mut settings.fm_confirm_delete, "删除前确认");
        });
    });
    group(ui, "显示与布局", |ui| {
        row(ui, "显示隐藏文件", None, |ui| {
            fw::toggle(ui, &mut settings.fm_show_hidden, "显示隐藏文件");
        });
        separator(ui);
        row(ui, "目录排在文件前", None, |ui| {
            fw::toggle(ui, &mut settings.fm_dirs_first, "目录排在文件前");
        });
        separator(ui);
        row(
            ui,
            "显示系统真实图标",
            Some("关闭后使用内置字体图标"),
            |ui| {
                fw::toggle(ui, &mut settings.fm_system_icons, "显示系统真实图标");
            },
        );
        separator(ui);
        row(ui, "过滤框显示在栏底部", None, |ui| {
            fw::toggle(ui, &mut settings.fm_filter_bar_bottom, "过滤框显示在栏底部");
        });
        separator(ui);
        row(ui, "显示命令行输入条", None, |ui| {
            fw::toggle(ui, &mut settings.fm_command_bar, "显示命令行输入条");
        });
        separator(ui);
        row(
            ui,
            "递归监听子目录变化",
            Some("大目录或网络盘可能产生额外开销"),
            |ui| {
                fw::toggle(ui, &mut settings.fm_watch_recursive, "递归监听子目录变化");
            },
        );
        separator(ui);
        row(
            ui,
            "文件操作并发数",
            Some("0 = 不限；超出的任务排队等待"),
            |ui| {
                ui.add(egui::DragValue::new(&mut settings.fm_op_threads).range(0..=8));
            },
        );
        separator(ui);
        row(
            ui,
            "列表密度",
            Some("舒适模式增加行高；切换后保留当前焦点和选择"),
            |ui| {
                egui::ComboBox::from_id_salt("fm_density")
                    .selected_text(if settings.fm_density == "comfortable" {
                        "舒适"
                    } else {
                        "紧凑"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.fm_density, "compact".to_owned(), "紧凑");
                        ui.selectable_value(
                            &mut settings.fm_density,
                            "comfortable".to_owned(),
                            "舒适",
                        );
                    });
            },
        );
        separator(ui);
        row(ui, "默认布局", None, |ui| {
            egui::ComboBox::from_id_salt("fm_layout")
                .selected_text(if settings.fm_layout == "single" {
                    "单栏"
                } else {
                    "双栏"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.fm_layout, "dual".to_string(), "双栏");
                    ui.selectable_value(
                        &mut settings.fm_layout,
                        "single".to_string(),
                        "单栏（含预览面板）",
                    );
                });
        });
        separator(ui);
        row(ui, "双栏比例", Some("左栏宽度占比"), |ui| {
            fw::slider(
                ui,
                egui::Slider::new(&mut settings.fm_dual_ratio, 0.2..=0.8).step_by(0.01),
            );
        });
    });
    group(ui, "交互行为", |ui| {
        row(
            ui,
            "空格键",
            Some("Insert 键始终为勾选并下移"),
            |ui| {
                egui::ComboBox::from_id_salt("fm_space_action")
                    .selected_text(if settings.fm_space_action == "toggle_select" {
                        "勾选焦点项并下移"
                    } else {
                        "计算目录大小"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut settings.fm_space_action,
                            "dir_size".to_string(),
                            "计算目录大小",
                        );
                        ui.selectable_value(
                            &mut settings.fm_space_action,
                            "toggle_select".to_string(),
                            "勾选焦点项并下移",
                        );
                    });
            },
        );
        separator(ui);
        row(
            ui,
            "栏间拖放复制前确认",
            Some("拖动时按住 Shift 为移动"),
            |ui| {
                fw::toggle(ui, &mut settings.fm_drag_confirm, "栏间拖放复制前确认");
            },
        );
        separator(ui);
        row(ui, "Esc 保留选中", None, |ui| {
            fw::toggle(ui, &mut settings.fm_esc_keep_selection, "Esc 保留选中");
        });
        separator(ui);
        row(ui, "双击空白处回上级目录", None, |ui| {
            fw::toggle(
                ui,
                &mut settings.fm_dblclick_blank_up,
                "双击空白处回上级目录",
            );
        });
        separator(ui);
        row(ui, "鼠标框选", None, |ui| {
            egui::ComboBox::from_id_salt("fm_rubber_band")
                .selected_text(match settings.fm_rubber_band.as_str() {
                    "left" => "左键（空白区起拖）",
                    "off" => "关闭",
                    _ => "右键拖动",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut settings.fm_rubber_band,
                        "right".to_string(),
                        "右键拖动",
                    );
                    ui.selectable_value(
                        &mut settings.fm_rubber_band,
                        "left".to_string(),
                        "左键（空白区起拖）",
                    );
                    ui.selectable_value(&mut settings.fm_rubber_band, "off".to_string(), "关闭");
                });
        });
        separator(ui);
        row(ui, "双击压缩包", None, |ui| {
            egui::ComboBox::from_id_salt("fm_archive_open")
                .selected_text(match settings.fm_archive_open.as_str() {
                    "comic" => "作为漫画打开",
                    "ask" => "每次询问",
                    _ => "压缩包浏览视图",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut settings.fm_archive_open,
                        "archive".to_string(),
                        "压缩包浏览视图",
                    );
                    ui.selectable_value(
                        &mut settings.fm_archive_open,
                        "comic".to_string(),
                        "作为漫画打开",
                    );
                    ui.selectable_value(
                        &mut settings.fm_archive_open,
                        "ask".to_string(),
                        "每次询问",
                    );
                });
        });
    });
    group(ui, "自定义按钮栏", |ui| {
        button_bar(view, ui, settings);
    });
}

fn button_bar(view: &mut SettingsView, ui: &mut Ui, settings: &mut Settings) {
    ui.label(
        RichText::new("占位符：%P = 当前目录、%N = 焦点项名称、%p = 另一栏目录")
            .small()
            .weak(),
    );
    let mut delete_idx = None;
    let mut swap_idx = None;
    let count = settings.fm_button_bar.len();
    for (i, button) in settings.fm_button_bar.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .add_enabled_ui(i > 0, |ui| {
                    fw::icon_button(ui, icons::CARET_UP, "上移按钮", ButtonKind::Subtle)
                })
                .inner
                .clicked()
            {
                swap_idx = Some(i - 1);
            }
            if ui
                .add_enabled_ui(i + 1 < count, |ui| {
                    fw::icon_button(ui, icons::CARET_DOWN, "下移按钮", ButtonKind::Subtle)
                })
                .inner
                .clicked()
            {
                swap_idx = Some(i);
            }
            ui.label(&button.label);
            ui.label(RichText::new(&button.command).weak().small());
            if fw::icon_button(ui, icons::X, "删除按钮", ButtonKind::Subtle).clicked() {
                delete_idx = Some(i);
            }
        });
    }
    if let Some(i) = swap_idx {
        settings.fm_button_bar.swap(i, i + 1);
    }
    if let Some(i) = delete_idx {
        settings.fm_button_bar.remove(i);
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut view.button_add_label)
                .hint_text("按钮文字")
                .desired_width(100.0),
        );
        ui.add(
            egui::TextEdit::singleline(&mut view.button_add_command)
                .hint_text("命令")
                .desired_width(220.0),
        );
        ui.add(
            egui::TextEdit::singleline(&mut view.button_add_tooltip)
                .hint_text("悬停提示")
                .desired_width(140.0),
        );
        let ready =
            !view.button_add_label.trim().is_empty() && !view.button_add_command.trim().is_empty();
        ui.add_enabled_ui(ready, |ui| {
            if fw::button(ui, "添加按钮", ButtonKind::Primary).clicked() {
                settings.fm_button_bar.push(FmButton {
                    label: view.button_add_label.trim().to_string(),
                    command: view.button_add_command.trim().to_string(),
                    tooltip: view.button_add_tooltip.trim().to_string(),
                });
                view.button_add_label.clear();
                view.button_add_command.clear();
                view.button_add_tooltip.clear();
            }
        });
    });
}

fn archive(
    view: &mut SettingsView,
    ui: &mut Ui,
    settings: &mut Settings,
    password_book: &mut PasswordBook,
) -> bool {
    let mut changed = false;
    fw::setting_group(ui, "解压与密码本", |ui| {
        changed = view.archive_ui(ui, settings, password_book);
    });
    ui.add_space(spacing::LG);
    changed
}

fn file_assoc(view: &mut SettingsView, ui: &mut Ui) {
    group(ui, "Windows 文件关联", |ui| {
        view.file_assoc_ui(ui);
    });
}

fn shortcuts(view: &mut SettingsView, ui: &mut Ui, settings: &mut Settings) {
    group(ui, "键盘快捷键", |ui| {
        ui.label(
            RichText::new("点击已有快捷键可移除；在输入框中填写新按键后添加")
                .small()
                .weak(),
        );
        view.shortcut_editor(ui, &mut settings.shortcuts);
    });
}

fn group(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    fw::setting_group(ui, title, add_contents);
    ui.add_space(spacing::LG);
}

fn row(ui: &mut Ui, title: &str, description: Option<&str>, add_contents: impl FnOnce(&mut Ui)) {
    fw::setting_row(ui, title, description, add_contents);
}

fn separator(ui: &mut Ui) {
    ui.add_space(spacing::XS);
    ui.separator();
    ui.add_space(spacing::XS);
}

fn tab_label(tab: SettingsTab) -> &'static str {
    SettingsTab::ALL
        .iter()
        .find_map(|(candidate, label)| (*candidate == tab).then_some(*label))
        .unwrap_or("设置")
}
