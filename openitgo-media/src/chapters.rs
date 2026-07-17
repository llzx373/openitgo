//! Chapter records (FFI-free) and their parsing logic.
//! Same layering as devices.rs: player.rs fills RawChapter from mpv nodes;
//! everything downstream stays testable.

/// Intermediate, FFI-free chapter record.
#[derive(Debug, Clone, PartialEq)]
pub struct RawChapter {
    pub title: Option<String>,
}

/// Chapter titles for the UI; untitled chapters fall back to `第 N 章`.
pub fn parse_chapters(raw: Vec<RawChapter>) -> Vec<String> {
    raw.into_iter()
        .enumerate()
        .map(|(i, c)| match c.title {
            Some(t) if !t.trim().is_empty() => t,
            _ => format!("第 {} 章", i + 1),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_titles() {
        let raw = vec![
            RawChapter {
                title: Some("序幕".into()),
            },
            RawChapter {
                title: Some("高潮".into()),
            },
        ];
        assert_eq!(parse_chapters(raw), vec!["序幕", "高潮"]);
    }

    #[test]
    fn parse_falls_back_for_missing_or_blank_titles() {
        let raw = vec![
            RawChapter { title: None },
            RawChapter {
                title: Some("  ".into()),
            },
            RawChapter {
                title: Some("终章".into()),
            },
        ];
        assert_eq!(parse_chapters(raw), vec!["第 1 章", "第 2 章", "终章"]);
    }

    #[test]
    fn parse_empty_list() {
        assert!(parse_chapters(Vec::new()).is_empty());
    }
}
