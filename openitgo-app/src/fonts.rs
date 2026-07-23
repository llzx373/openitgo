use egui::{FontDefinitions, FontFamily};
use std::path::PathBuf;

/// Candidate system fonts that include CJK glyphs, ordered by platform likelihood.
const CJK_FONT_CANDIDATES: &[&str] = &[
    // macOS
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    // Windows
    "C:/Windows/Fonts/msyh.ttc",
    "C:/Windows/Fonts/simhei.ttf",
    "C:/Windows/Fonts/simsun.ttc",
    "C:/Windows/Fonts/msgothic.ttc",
    "C:/Windows/Fonts/msmincho.ttc",
    // Linux
    "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansSC-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSansJP-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSansKR-Regular.otf",
];

/// Loads the first available system CJK font and installs the Phosphor icon font.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    if let Some(path) = CJK_FONT_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    {
        if let Ok(bytes) = std::fs::read(&path) {
            // CJK metrics often sit high in egui's layout box; nudge glyphs down
            // so button / toolbar labels look vertically centered.
            let cjk = egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
                y_offset_factor: 0.06,
                ..Default::default()
            });
            fonts
                .font_data
                .insert("cjk".to_owned(), std::sync::Arc::new(cjk));
            if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
                proportional.push("cjk".to_owned());
            }
        }
    }

    egui_phosphor_icons::add_fonts(&mut fonts);
    // Allow icon codepoints to render inline with normal text (e.g. "icon + label"
    // strings) by appending the regular icon font as a Proportional fallback.
    // Slight downward nudge so Phosphor glyphs share a visual center with CJK.
    if let Some(data) = fonts.font_data.get_mut("phosphor-icons") {
        let mut owned = (**data).clone();
        owned.tweak = egui::FontTweak {
            y_offset_factor: 0.04,
            ..Default::default()
        };
        *data = std::sync::Arc::new(owned);
    }
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.push("phosphor-icons".to_owned());
    }

    ctx.set_fonts(fonts);
}
