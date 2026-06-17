use egui::{FontData, FontDefinitions, FontFamily};
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

/// Attempts to load the first available system CJK font and merge it into egui's
/// default proportional font family.
pub fn load_cjk_font(ctx: &egui::Context) {
    let path = match CJK_FONT_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    {
        Some(p) => p,
        None => return,
    };

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return,
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_owned(), FontData::from_owned(bytes));

    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.push("cjk".to_owned());
    }
    if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
        // Keep monospace unchanged unless it fails to render CJK later.
        let _ = monospace;
    }

    ctx.set_fonts(fonts);
}
