use egui::Key;
use openitgo_storage::models::Shortcuts;

pub fn key_from_name(name: &str) -> Option<Key> {
    match name {
        "ArrowRight" => Some(Key::ArrowRight),
        "ArrowLeft" => Some(Key::ArrowLeft),
        "ArrowUp" => Some(Key::ArrowUp),
        "ArrowDown" => Some(Key::ArrowDown),
        "PageDown" => Some(Key::PageDown),
        "PageUp" => Some(Key::PageUp),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "Space" => Some(Key::Space),
        "Enter" => Some(Key::Enter),
        "Escape" => Some(Key::Escape),
        "F11" => Some(Key::F11),
        "Plus" => Some(Key::Plus),
        "Equals" => Some(Key::Equals),
        "Minus" => Some(Key::Minus),
        "Num0" => Some(Key::Num0),
        "Num1" => Some(Key::Num1),
        "Num2" => Some(Key::Num2),
        "Num3" => Some(Key::Num3),
        "Num4" => Some(Key::Num4),
        "Num5" => Some(Key::Num5),
        "Num6" => Some(Key::Num6),
        "Num7" => Some(Key::Num7),
        "Num8" => Some(Key::Num8),
        "Num9" => Some(Key::Num9),
        "A" => Some(Key::A),
        "B" => Some(Key::B),
        "C" => Some(Key::C),
        "D" => Some(Key::D),
        "E" => Some(Key::E),
        "F" => Some(Key::F),
        "G" => Some(Key::G),
        "H" => Some(Key::H),
        "I" => Some(Key::I),
        "J" => Some(Key::J),
        "K" => Some(Key::K),
        "L" => Some(Key::L),
        "M" => Some(Key::M),
        "N" => Some(Key::N),
        "O" => Some(Key::O),
        "P" => Some(Key::P),
        "Q" => Some(Key::Q),
        "R" => Some(Key::R),
        "S" => Some(Key::S),
        "T" => Some(Key::T),
        "U" => Some(Key::U),
        "V" => Some(Key::V),
        "W" => Some(Key::W),
        "X" => Some(Key::X),
        "Y" => Some(Key::Y),
        "Z" => Some(Key::Z),
        _ => None,
    }
}

pub fn is_shortcut_pressed(ctx: &egui::Context, bindings: &[String]) -> bool {
    bindings.iter().any(|name| {
        key_from_name(name)
            .map(|key| ctx.input(|i| i.key_pressed(key)))
            .unwrap_or(false)
    })
}

/// 快捷键一览面板的"可自定义"分区：动作名（中文，与设置面板一致）+ 当前键位。
pub fn configurable_shortcut_rows(s: &Shortcuts) -> Vec<(&'static str, String)> {
    let join = |v: &[String]| v.join(" / ");
    vec![
        ("下一页", join(&s.next_page)),
        ("上一页", join(&s.prev_page)),
        ("向下翻页", join(&s.page_down)),
        ("向上翻页", join(&s.page_up)),
        ("首页", join(&s.first_page)),
        ("末页", join(&s.last_page)),
        ("全屏", join(&s.fullscreen)),
        ("适应页面", join(&s.fit_page)),
        ("适应宽度", join(&s.fit_width)),
        ("适应高度", join(&s.fit_height)),
        ("放大", join(&s.zoom_in)),
        ("缩小", join(&s.zoom_out)),
        ("返回书架", join(&s.back_to_library)),
    ]
}

/// 快捷键一览面板的"内置"分区（阅读器），键位硬编码在 app.rs/reader.rs。
pub fn hardcoded_reader_rows() -> Vec<(&'static str, &'static str)> {
    vec![
        ("点击画面左 / 右半边", "上一页 / 下一页"),
        ("双击画面", "切换 原始大小 / 自动适应"),
        ("Ctrl + 滚轮", "缩放"),
        ("鼠标侧键", "上一页 / 下一页"),
    ]
}

/// 快捷键一览面板的"内置"分区（媒体），键位硬编码在 app.rs 的
/// `handle_global_input` View::Media 分支。
pub fn hardcoded_media_rows() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Space", "播放 / 暂停"),
        ("← / →", "快退 / 快进 5 秒"),
        ("↑ / ↓", "音量 + / -"),
        ("1 - 4", "倍速直选（0.5x / 1.0x / 1.5x / 2.0x）"),
        ("Z / X", "字幕延迟 -0.1s / +0.1s"),
        ("M", "静音 / 取消静音"),
        ("F", "全屏"),
        ("[ / ]", "倍速微调 -0.25 / +0.25"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_from_name_roundtrip() {
        assert_eq!(key_from_name("Space"), Some(Key::Space));
        assert_eq!(key_from_name("ArrowRight"), Some(Key::ArrowRight));
        assert_eq!(key_from_name("UnknownKey"), None);
    }

    #[test]
    fn test_key_from_name_known_keys() {
        assert_eq!(key_from_name("Escape"), Some(Key::Escape));
        assert_eq!(key_from_name("F11"), Some(Key::F11));
    }

    #[test]
    fn test_key_from_name_home_end() {
        assert_eq!(key_from_name("Home"), Some(Key::Home));
        assert_eq!(key_from_name("End"), Some(Key::End));
    }

    #[test]
    fn test_configurable_shortcut_rows_defaults() {
        let rows = configurable_shortcut_rows(&openitgo_storage::models::Shortcuts::default());
        let expected: Vec<(&'static str, String)> = vec![
            ("下一页", "ArrowRight".to_string()),
            ("上一页", "ArrowLeft".to_string()),
            ("向下翻页", "PageDown / Space".to_string()),
            ("向上翻页", "PageUp".to_string()),
            ("首页", "Home".to_string()),
            ("末页", "End".to_string()),
            ("全屏", "F11".to_string()),
            ("适应页面", "Num0".to_string()),
            ("适应宽度", "W".to_string()),
            ("适应高度", "H".to_string()),
            ("放大", "Plus / Equals".to_string()),
            ("缩小", "Minus".to_string()),
            ("返回书架", "Escape".to_string()),
        ];
        assert_eq!(rows, expected);
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_configurable_shortcut_rows_reflects_custom_bindings() {
        let mut s = openitgo_storage::models::Shortcuts::default();
        s.next_page = vec!["J".to_string()];
        let rows = configurable_shortcut_rows(&s);
        assert_eq!(rows[0], ("下一页", "J".to_string()));
    }

    #[test]
    fn test_hardcoded_rows_cover_reader_and_media() {
        let reader = hardcoded_reader_rows();
        assert_eq!(
            reader,
            vec![
                ("点击画面左 / 右半边", "上一页 / 下一页"),
                ("双击画面", "切换 原始大小 / 自动适应"),
                ("Ctrl + 滚轮", "缩放"),
                ("鼠标侧键", "上一页 / 下一页"),
            ]
        );
        let media = hardcoded_media_rows();
        assert_eq!(
            media,
            vec![
                ("Space", "播放 / 暂停"),
                ("← / →", "快退 / 快进 5 秒"),
                ("↑ / ↓", "音量 + / -"),
                ("1 - 4", "倍速直选（0.5x / 1.0x / 1.5x / 2.0x）"),
                ("Z / X", "字幕延迟 -0.1s / +0.1s"),
                ("M", "静音 / 取消静音"),
                ("F", "全屏"),
                ("[ / ]", "倍速微调 -0.25 / +0.25"),
            ]
        );
    }
}
