use crate::platform::file_assoc::{self, AssocState, ExtAssoc};
use egui_phosphor_icons::icons;
use openitgo_core::ebook::EbookReadingMode;
use openitgo_core::models::{FitMode, ReadingMode};
use openitgo_storage::models::{
    ComicEndAction, EbookTheme, MediaEndAction, PasswordBook, PasswordBookEntry, Settings, Theme,
    ToolbarDisplayMode,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Comic,
    Ebook,
    Media,
    Archive,
    FileManager,
    FileAssoc,
    Performance,
    Shortcuts,
}

impl SettingsTab {
    const ALL: [(SettingsTab, &'static str); 9] = [
        (SettingsTab::Appearance, "外观"),
        (SettingsTab::Comic, "漫画"),
        (SettingsTab::Ebook, "电子书"),
        (SettingsTab::Media, "媒体"),
        (SettingsTab::Archive, "压缩包"),
        (SettingsTab::FileManager, "文件管理器"),
        (SettingsTab::FileAssoc, "文件关联"),
        (SettingsTab::Performance, "性能"),
        (SettingsTab::Shortcuts, "快捷键"),
    ];
}

#[derive(Default)]
pub struct SettingsView {
    pub tab: SettingsTab,
    shortcut_add_buffer: HashMap<&'static str, String>,
    /// 密码本列表中切换为明文显示的行索引。
    password_show: HashSet<usize>,
    /// 密码本「添加」输入框：密码。
    password_add_input: String,
    /// 密码本「添加」输入框：可选备注。
    password_add_note: String,
    /// 文件关联状态缓存（None = 未加载，首次进入该 tab 时惰性查询）。
    file_assoc: Option<Vec<ExtAssoc>>,
    /// 文件关联操作结果 / 错误提示。
    assoc_status: Option<String>,
}

impl SettingsView {
    pub fn focus_tab(&mut self, tab: SettingsTab) {
        self.tab = tab;
    }

    /// 返回密码本是否有变更（变更即由调用方落盘）。
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
        password_book: &mut PasswordBook,
    ) -> bool {
        ui.heading(egui::RichText::new("设置").size(22.0).strong());
        ui.add_space(10.0);

        let mut book_changed = false;
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
                        SettingsTab::Archive => {
                            book_changed = self.archive_ui(ui, settings, password_book);
                        }
                        SettingsTab::FileManager => Self::file_manager_ui(ui, settings),
                        SettingsTab::FileAssoc => self.file_assoc_ui(ui),
                        SettingsTab::Performance => self.performance_ui(ui, settings),
                        SettingsTab::Shortcuts => self.shortcut_editor(ui, &mut settings.shortcuts),
                    }
                });
        });
        book_changed
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

    /// 文件管理器 tab：删除 / 显示与布局 / 交互行为（阶段 O 可选行为包，
    /// 默认值 = 一期现状行为）。
    fn file_manager_ui(ui: &mut egui::Ui, settings: &mut Settings) {
        ui.label(egui::RichText::new("删除").strong());
        ui.horizontal(|ui| {
            ui.label("删除方式");
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
        });
        hint(
            ui,
            "Shift+Del 为另一档快捷（回收站模式下直删，永久删除模式下进回收站）",
        );
        ui.checkbox(&mut settings.fm_confirm_delete, "删除前确认");
        hint(ui, "关闭后删除不再弹确认框");

        ui.add_space(8.0);
        ui.label(egui::RichText::new("显示与布局").strong());
        ui.checkbox(&mut settings.fm_show_hidden, "显示隐藏文件");
        hint(ui, "隐藏文件指以 . 开头的文件及带系统隐藏属性的文件");
        ui.checkbox(&mut settings.fm_dirs_first, "目录排在文件前");
        hint(ui, "关闭后目录与文件混排、统一按排序键排序");
        ui.horizontal(|ui| {
            ui.label("默认布局");
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
        hint(ui, "改动立即同步到文件管理器视图，下次进入即生效");
        ui.horizontal(|ui| {
            ui.label("双栏比例（左栏宽度）:");
            ui.add(egui::Slider::new(&mut settings.fm_dual_ratio, 0.2..=0.8).step_by(0.01));
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("交互行为").strong());
        ui.horizontal(|ui| {
            ui.label("空格键");
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
        });
        hint(ui, "Insert 键无条件为「勾选并下移」，与此设置无关");
        ui.checkbox(&mut settings.fm_drag_confirm, "栏间拖放复制前确认");
        hint(
            ui,
            "关闭后松开直接复制（自动改名冲突策略），拖动时按住 Shift 为移动",
        );
        ui.checkbox(&mut settings.fm_esc_keep_selection, "Esc 保留选中");
        hint(ui, "开启后 Esc 只关闭弹层、清字母定位与过滤，不再清除选中");
        ui.checkbox(&mut settings.fm_dblclick_blank_up, "双击空白处回上级目录");
        hint(ui, "列表/网格视图中双击未占用区域 = 回上级（同 Backspace）");
        ui.horizontal(|ui| {
            ui.label("双击压缩包");
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
    }

    /// 压缩包 tab：解压目录/线程/覆盖 + 密码本管理。返回密码本是否有变更。
    fn archive_ui(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
        book: &mut PasswordBook,
    ) -> bool {
        let mut book_changed = false;

        ui.label("解压目录");
        ui.horizontal(|ui| {
            let display = if settings.extract_dir.is_empty() {
                "默认（压缩包同目录；内容无单一顶层目录时自动建包名子目录）".to_string()
            } else {
                settings.extract_dir.clone()
            };
            ui.add(egui::Label::new(egui::RichText::new(display).weak()).truncate());
            if ui.button("浏览…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    settings.extract_dir = dir.display().to_string();
                }
            }
            if ui.button("清空").clicked() {
                settings.extract_dir.clear();
            }
        });
        hint(
            ui,
            "留空则解压到压缩包同目录；包内无单一顶层目录时自动放入以包名命名的子目录（重名自动加序号）",
        );

        ui.horizontal(|ui| {
            ui.label("解压线程数:");
            ui.add(egui::DragValue::new(&mut settings.extract_threads).range(0..=32));
        });
        hint(ui, "0 = 自动；仅 ZIP 按条目并行解压，RAR/7z/TAR 始终单线程");

        ui.checkbox(&mut settings.extract_overwrite, "同名文件覆盖");
        hint(ui, "关闭时同名文件自动改名为 \"name (1).ext\"");

        ui.add_space(4.0);
        ui.separator();
        ui.label(egui::RichText::new("密码本").strong());
        let total = book.entries.len();
        let builtin = book.entries.iter().filter(|e| e.builtin).count();
        hint(
            ui,
            &format!("共 {total} 条（内置 {builtin} 条）；打开加密压缩包时按使用频率自动尝试"),
        );

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.password_add_input)
                    .hint_text("新密码")
                    .desired_width(160.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.password_add_note)
                    .hint_text("备注（可选）")
                    .desired_width(140.0),
            );
            if ui.button("添加").clicked() {
                let pw = self.password_add_input.trim().to_string();
                if !pw.is_empty() && !book.entries.iter().any(|e| e.password == pw) {
                    book.entries.push(PasswordBookEntry {
                        password: pw,
                        builtin: false,
                        note: self.password_add_note.trim().to_string(),
                        use_count: 0,
                        last_used_unix: 0,
                    });
                    book_changed = true;
                }
                self.password_add_input.clear();
                self.password_add_note.clear();
            }
        });

        egui::ScrollArea::vertical()
            .max_height(240.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut remove_idx: Option<usize> = None;
                for (idx, entry) in book.entries.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let shown = self.password_show.contains(&idx);
                        let text = if shown {
                            entry.password.clone()
                        } else {
                            "***".to_string()
                        };
                        ui.add(egui::Label::new(egui::RichText::new(text).monospace()).truncate());
                        let eye = if shown { icons::EYE_SLASH } else { icons::EYE };
                        if ui.button(eye).on_hover_text("显示 / 隐藏密码").clicked() {
                            if shown {
                                self.password_show.remove(&idx);
                            } else {
                                self.password_show.insert(idx);
                            }
                        }
                        if entry.builtin {
                            ui.label(egui::RichText::new("内置").weak().size(11.0));
                        }
                        if !entry.note.is_empty() {
                            ui.label(egui::RichText::new(&entry.note).weak());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(icons::TRASH).on_hover_text("删除").clicked() {
                                remove_idx = Some(idx);
                            }
                            ui.label(
                                egui::RichText::new(format!("用过 {} 次", entry.use_count))
                                    .weak()
                                    .size(11.0),
                            );
                        });
                    });
                }
                if let Some(idx) = remove_idx {
                    book.entries.remove(idx);
                    // 行索引位移，重置展开态。
                    self.password_show.clear();
                    book_changed = true;
                }
            });

        if ui
            .button("恢复内置密码")
            .on_hover_text("把被删除的内置常见密码补回密码本")
            .clicked()
        {
            for entry in PasswordBook::builtin_defaults() {
                if !book.entries.iter().any(|e| e.password == entry.password) {
                    book.entries.push(entry);
                    book_changed = true;
                }
            }
        }

        book_changed
    }

    /// 文件关联 tab：按组列出扩展名勾选，批量注册 / 注销。
    fn file_assoc_ui(&mut self, ui: &mut egui::Ui) {
        if !cfg!(target_os = "windows") {
            hint(ui, "文件关联仅支持 Windows");
            return;
        }

        // 惰性加载：首次进入该 tab 查询；加载失败停在错误态等「重试」。
        if self.file_assoc.is_none() && self.assoc_status.is_none() {
            match file_assoc::query_status() {
                Ok(list) => self.file_assoc = Some(list),
                Err(e) => self.assoc_status = Some(format!("查询关联状态失败：{e}")),
            }
        }

        if self.file_assoc.is_none() {
            if let Some(err) = self.assoc_status.clone() {
                ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(0xd0, 0x50, 0x50)));
            }
            if ui.button("重试").clicked() {
                self.assoc_status = None;
            }
            return;
        }

        /// 底部按钮触发的待执行操作（借用的列表释放后再执行）。
        enum Pending {
            None,
            Register(Vec<&'static str>),
            Unregister(Vec<&'static str>),
            OpenSystem,
        }
        let mut pending = Pending::None;

        {
            let assocs = self.file_assoc.as_mut().expect("checked above");
            for (group, label, _) in file_assoc::EXT_GROUPS {
                let mut items: Vec<&mut ExtAssoc> =
                    assocs.iter_mut().filter(|a| a.group == *group).collect();
                if items.is_empty() {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(*label).strong());
                    if ui.small_button("全选").clicked() {
                        for item in items.iter_mut() {
                            item.selected = true;
                        }
                    }
                    if ui.small_button("全不选").clicked() {
                        for item in items.iter_mut() {
                            item.selected = false;
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for item in items.iter_mut() {
                        ui.checkbox(&mut item.selected, format!(".{}", item.ext));
                        let (text, color) = match &item.state {
                            AssocState::Ours => {
                                ("已关联", egui::Color32::from_rgb(0x4c, 0xaf, 0x50))
                            }
                            AssocState::Other(_) => ("其他程序", ui.visuals().weak_text_color()),
                            AssocState::None => ("未关联", egui::Color32::DARK_GRAY),
                        };
                        let resp = ui.label(egui::RichText::new(text).color(color).size(11.0));
                        if let AssocState::Other(progid) = &item.state {
                            resp.on_hover_text(format!("当前默认程序：{progid}"));
                        }
                        ui.add_space(6.0);
                    }
                });
                ui.add_space(4.0);
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("关联选中").clicked() {
                    pending = Pending::Register(
                        assocs
                            .iter()
                            .filter(|a| a.selected)
                            .map(|a| a.ext)
                            .collect(),
                    );
                }
                if ui.button("取消关联选中").clicked() {
                    pending = Pending::Unregister(
                        assocs
                            .iter()
                            .filter(|a| a.selected)
                            .map(|a| a.ext)
                            .collect(),
                    );
                }
                if ui.button("打开系统默认应用设置").clicked() {
                    pending = Pending::OpenSystem;
                }
            });
        }

        match pending {
            Pending::None => {}
            Pending::Register(exts) => self.apply_assoc_op(true, exts),
            Pending::Unregister(exts) => self.apply_assoc_op(false, exts),
            Pending::OpenSystem => match file_assoc::open_default_apps_settings() {
                Ok(()) => self.assoc_status = None,
                Err(e) => self.assoc_status = Some(e),
            },
        }

        if let Some(status) = &self.assoc_status {
            ui.label(egui::RichText::new(status).weak());
        }

        hint(
            ui,
            "若扩展名已被其他程序设为默认，Windows 10/11 需在系统设置中更改（上方按钮直达）；关联后也会出现在右键「打开方式」列表。",
        );
        hint(ui, "便携版移动程序位置后，请重新关联。");
    }

    /// 批量注册 / 注销勾选的扩展名，成功后刷新状态缓存。
    fn apply_assoc_op(&mut self, register: bool, exts: Vec<&'static str>) {
        if exts.is_empty() {
            self.assoc_status = Some("未勾选任何扩展名".to_string());
            return;
        }
        let result = if register {
            file_assoc::register(&exts)
        } else {
            file_assoc::unregister(&exts)
        };
        match result {
            Ok(n) => {
                self.assoc_status = Some(if register {
                    format!("已关联 {n} 个扩展名")
                } else {
                    format!("已取消关联 {n} 个扩展名")
                });
                match file_assoc::query_status() {
                    Ok(list) => self.file_assoc = Some(list),
                    Err(e) => self.assoc_status = Some(format!("刷新关联状态失败：{e}")),
                }
            }
            Err(e) => {
                self.assoc_status = Some(if register {
                    format!("关联失败：{e}")
                } else {
                    format!("取消关联失败：{e}")
                });
            }
        }
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
