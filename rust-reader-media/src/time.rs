/// Formats a millisecond position as `m:ss` below one hour, `h:mm:ss` otherwise.
pub fn format_time_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_under_one_hour_as_m_ss() {
        assert_eq!(format_time_ms(0), "0:00");
        assert_eq!(format_time_ms(59_000), "0:59");
        assert_eq!(format_time_ms(60_000), "1:00");
        assert_eq!(format_time_ms(3_599_000), "59:59");
    }

    #[test]
    fn formats_one_hour_and_above_as_h_mm_ss() {
        assert_eq!(format_time_ms(3_600_000), "1:00:00");
        assert_eq!(format_time_ms(7_261_000), "2:01:01");
    }
}
