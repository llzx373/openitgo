//! mpv 命令与属性参数构造。纯函数，不触碰 libmpv FFI，
//! 因此非 macOS 平台（CI ubuntu job）也能编译与测试。

/// `volume` 属性参数：钳制到 0..=100，保留 1 位小数。
pub fn format_volume_arg(volume: f64) -> String {
    format!("{:.1}", volume.clamp(0.0, 100.0))
}

/// `speed` 属性参数：钳制到 0.1..=16.0，保留 2 位小数。
pub fn format_speed_arg(speed: f64) -> String {
    format!("{:.2}", speed.clamp(0.1, 16.0))
}

/// 绝对 seek 命令参数：`["seek", <秒.3位小数>, "absolute"]`，
/// `exact` 时追加 `"exact"`（与 player.rs 原内联实现逐字一致）。
pub fn seek_abs_args(ms: u64, exact: bool) -> Vec<String> {
    let secs = format!("{:.3}", ms as f64 / 1000.0);
    let mut args = vec!["seek".to_string(), secs, "absolute".to_string()];
    if exact {
        args.push("exact".to_string());
    }
    args
}

/// mpv 布尔属性参数。
pub fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// `loop-file` 属性参数：`inf` 无限循环，`no` 正常 EOF。
pub fn loop_file_arg(enabled: bool) -> &'static str {
    if enabled {
        "inf"
    } else {
        "no"
    }
}

/// `ab-loop-a`/`ab-loop-b` 属性参数：秒（3 位小数）或 `no` 清除。
pub fn ab_loop_arg(secs: Option<f64>) -> String {
    match secs {
        Some(v) => format!("{v:.3}"),
        None => "no".to_string(),
    }
}

/// `sid` 属性参数：轨道 id 或 `no` 关闭字幕。
pub fn sid_arg(id: Option<i64>) -> String {
    match id {
        Some(id) => id.to_string(),
        None => "no".to_string(),
    }
}

/// `background-color` 属性：mpv `#AARRGGBB`，alpha 来自阅读区透明度。
/// 与漫画 `reader_background_fill` 同色，letterbox 相对桌面半透明而非纯黑。
pub fn format_background_color_arg(rgb: [u8; 3], opacity: f32) -> String {
    let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{a:02X}{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_arg_clamps_to_0_100_with_one_decimal() {
        assert_eq!(format_volume_arg(50.0), "50.0");
        assert_eq!(format_volume_arg(-3.0), "0.0");
        assert_eq!(format_volume_arg(250.0), "100.0");
        assert_eq!(format_volume_arg(33.33), "33.3");
    }

    #[test]
    fn speed_arg_clamps_to_0_1_16_with_two_decimals() {
        assert_eq!(format_speed_arg(1.0), "1.00");
        assert_eq!(format_speed_arg(0.05), "0.10");
        assert_eq!(format_speed_arg(20.0), "16.00");
        assert_eq!(format_speed_arg(1.25), "1.25");
    }

    #[test]
    fn seek_abs_args_builds_absolute_seek_with_optional_exact() {
        assert_eq!(
            seek_abs_args(61_500, false),
            vec!["seek", "61.500", "absolute"]
        );
        assert_eq!(
            seek_abs_args(61_500, true),
            vec!["seek", "61.500", "absolute", "exact"]
        );
        assert_eq!(seek_abs_args(0, false), vec!["seek", "0.000", "absolute"]);
    }

    #[test]
    fn yes_no_and_loop_file_sentinels_match_mpv() {
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
        assert_eq!(loop_file_arg(true), "inf");
        assert_eq!(loop_file_arg(false), "no");
    }

    #[test]
    fn ab_loop_arg_formats_seconds_or_no() {
        assert_eq!(ab_loop_arg(Some(12.3456)), "12.346");
        assert_eq!(ab_loop_arg(None), "no");
    }

    #[test]
    fn sid_arg_formats_track_id_or_no() {
        assert_eq!(sid_arg(Some(3)), "3");
        assert_eq!(sid_arg(None), "no");
    }

    #[test]
    fn background_color_arg_is_aarrggbb() {
        assert_eq!(format_background_color_arg([30, 30, 30], 1.0), "#FF1E1E1E");
        assert_eq!(format_background_color_arg([12, 34, 56], 0.5), "#800C2238");
        assert_eq!(format_background_color_arg([255, 0, 0], 0.0), "#00FF0000");
    }
}
