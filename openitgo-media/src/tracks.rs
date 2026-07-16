#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Sub,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: i64,
    pub kind: TrackKind,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
}

/// Intermediate, FFI-free track record. `player.rs` fills this from mpv nodes;
/// everything downstream stays testable.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTrack {
    pub id: i64,
    pub kind: String,
    pub selected: bool,
    pub albumart: bool,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
}

pub fn parse_tracks(raw: Vec<RawTrack>) -> Vec<TrackInfo> {
    raw.into_iter()
        .filter_map(|t| {
            let kind = match t.kind.as_str() {
                "video" => TrackKind::Video,
                "audio" => TrackKind::Audio,
                "sub" => TrackKind::Sub,
                _ => return None,
            };
            Some(TrackInfo {
                id: t.id,
                kind,
                title: t.title,
                lang: t.lang,
                codec: t.codec,
                selected: t.selected,
            })
        })
        .collect()
}

/// True when a selected, non-albumart video track exists (real video content).
pub fn has_real_video(tracks: &[TrackInfo], raw: &[RawTrack]) -> bool {
    raw.iter()
        .filter(|r| r.kind == "video" && r.selected && !r.albumart)
        .any(|r| tracks.iter().any(|t| t.id == r.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: i64, kind: &str, selected: bool, albumart: bool) -> RawTrack {
        RawTrack {
            id,
            kind: kind.to_string(),
            selected,
            albumart,
            title: Some(format!("track-{id}")),
            lang: None,
            codec: None,
        }
    }

    #[test]
    fn parse_tracks_maps_kinds_and_drops_unknown() {
        let tracks = parse_tracks(vec![
            raw(1, "video", true, false),
            raw(2, "audio", true, false),
            raw(3, "sub", false, false),
            raw(4, "attachment", false, false),
        ]);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].kind, TrackKind::Video);
        assert_eq!(tracks[1].kind, TrackKind::Audio);
        assert_eq!(tracks[2].kind, TrackKind::Sub);
        assert!(tracks[0].selected);
        assert!(!tracks[2].selected);
    }

    #[test]
    fn has_real_video_ignores_albumart() {
        let art = raw(1, "video", true, true);
        let parsed = parse_tracks(vec![art.clone()]);
        assert!(!has_real_video(&parsed, &[art]));

        let movie = raw(1, "video", true, false);
        let parsed = parse_tracks(vec![movie.clone()]);
        assert!(has_real_video(&parsed, &[movie]));
    }
}
