//! mpv 事件 → `PlayerState` 的状态迁移，以及异步回复 userdata 路由。
//! 纯函数，不触碰 libmpv FFI，非 macOS 平台（CI ubuntu job）同样可测。

use crate::state::PlayerState;
use crate::tracks::{has_real_video, parse_tracks, RawTrack, TrackKind};

/// reply_userdata for the async `audio-device-list` query; observed
/// properties use 1-9 (different event type, but keep namespaces distinct).
pub const AUDIO_DEVICES_REPLY_USERDATA: u64 = 100;
/// reply_userdata for the async `chapter-list` query.
pub const CHAPTER_LIST_REPLY_USERDATA: u64 = 101;

/// 异步 GET_PROPERTY_REPLY 的分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    AudioDevices,
    ChapterList,
    /// 包括 fire-and-forget 的 userdata 0 与未知值。
    Other,
}

pub fn classify_reply(userdata: u64) -> ReplyKind {
    match userdata {
        AUDIO_DEVICES_REPLY_USERDATA => ReplyKind::AudioDevices,
        CHAPTER_LIST_REPLY_USERDATA => ReplyKind::ChapterList,
        _ => ReplyKind::Other,
    }
}

/// FILE_LOADED：新文件开始播放，重置上一文件的播放状态。
pub fn apply_file_loaded(s: &mut PlayerState) {
    s.loaded = true;
    s.ended = false;
    s.error = None;
    s.chapter = None;
    s.chapters.clear();
}

/// END_FILE：播放结束；`is_error`（mpv reason == ERROR）时记录错误。
pub fn apply_end_file(s: &mut PlayerState, is_error: bool) {
    s.ended = true;
    if is_error {
        s.error = Some("无法播放该文件".to_string());
    }
}

/// time-pos 属性（秒）→ position_ms；负值钳 0。
pub fn apply_time_pos(s: &mut PlayerState, secs: f64) {
    s.position_ms = (secs * 1000.0).max(0.0) as u64;
}

/// duration 属性（秒）→ duration_ms；格式无效（None）时清空。
pub fn apply_duration(s: &mut PlayerState, secs: Option<f64>) {
    s.duration_ms = secs.map(|v| (v * 1000.0).max(0.0) as u64);
}

/// track-list 属性：重建轨道列表并派生当前字幕/音轨与有无真实视频。
pub fn apply_track_list(s: &mut PlayerState, raw: Vec<RawTrack>) {
    s.tracks = parse_tracks(raw.clone());
    s.current_sub = s
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Sub && t.selected)
        .map(|t| t.id);
    s.current_audio = s
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio && t.selected)
        .map(|t| t.id);
    s.has_video = has_real_video(&s.tracks, &raw);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_track(id: i64, kind: &str, selected: bool, albumart: bool) -> RawTrack {
        RawTrack {
            id,
            kind: kind.to_string(),
            selected,
            albumart,
            title: None,
            lang: None,
            codec: None,
        }
    }

    #[test]
    fn classify_reply_routes_known_userdata() {
        assert_eq!(
            classify_reply(AUDIO_DEVICES_REPLY_USERDATA),
            ReplyKind::AudioDevices
        );
        assert_eq!(
            classify_reply(CHAPTER_LIST_REPLY_USERDATA),
            ReplyKind::ChapterList
        );
        assert_eq!(classify_reply(0), ReplyKind::Other);
        assert_eq!(classify_reply(999), ReplyKind::Other);
    }

    #[test]
    fn file_loaded_resets_playback_state() {
        let mut s = PlayerState {
            loaded: false,
            ended: true,
            error: Some("旧错误".to_string()),
            chapter: Some(2),
            chapters: vec!["旧章节".to_string()],
            ..Default::default()
        };
        apply_file_loaded(&mut s);
        assert!(s.loaded);
        assert!(!s.ended);
        assert_eq!(s.error, None);
        assert_eq!(s.chapter, None);
        assert!(s.chapters.is_empty());
    }

    #[test]
    fn end_file_sets_error_only_on_error_reason() {
        let mut s = PlayerState::default();
        apply_end_file(&mut s, false);
        assert!(s.ended);
        assert_eq!(s.error, None);

        let mut s = PlayerState::default();
        apply_end_file(&mut s, true);
        assert!(s.ended);
        assert_eq!(s.error.as_deref(), Some("无法播放该文件"));
    }

    #[test]
    fn time_pos_clamps_negative_and_truncates_to_ms() {
        let mut s = PlayerState::default();
        apply_time_pos(&mut s, 61.5);
        assert_eq!(s.position_ms, 61_500);
        apply_time_pos(&mut s, -2.0);
        assert_eq!(s.position_ms, 0);
    }

    #[test]
    fn duration_none_clears_and_some_converts() {
        let mut s = PlayerState::default();
        apply_duration(&mut s, Some(90.0));
        assert_eq!(s.duration_ms, Some(90_000));
        apply_duration(&mut s, None);
        assert_eq!(s.duration_ms, None);
    }

    #[test]
    fn track_list_derives_current_tracks_and_video_presence() {
        let mut s = PlayerState::default();
        apply_track_list(
            &mut s,
            vec![
                raw_track(1, "video", true, false),
                raw_track(2, "audio", true, false),
                raw_track(3, "sub", false, false),
                raw_track(4, "sub", true, false),
            ],
        );
        assert_eq!(s.current_audio, Some(2));
        assert_eq!(s.current_sub, Some(4));
        assert!(s.has_video);

        // 专辑封面不算真实视频；无选中轨道时派生为 None
        let mut s = PlayerState::default();
        apply_track_list(
            &mut s,
            vec![
                raw_track(1, "video", true, true),
                raw_track(2, "audio", true, false),
            ],
        );
        assert!(!s.has_video);
        assert_eq!(s.current_sub, None);
    }
}
