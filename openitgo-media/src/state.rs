use crate::devices::AudioDevice;
use crate::tracks::TrackInfo;

#[derive(Debug, Clone, Default)]
pub struct PlayerState {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub tracks: Vec<TrackInfo>,
    pub current_sub: Option<i64>,
    pub current_audio: Option<i64>,
    pub has_video: bool,
    pub loaded: bool,
    pub ended: bool,
    pub error: Option<String>,
    /// 字幕延迟（秒），来自 mpv `sub-delay` 属性观察；默认 0.0。
    pub sub_delay: f64,
    /// `None` until the first async audio-device-list reply lands — keeps
    /// "not enumerated yet" distinct from a genuinely empty enumeration.
    pub audio_devices: Option<Vec<AudioDevice>>,
    /// 当前章节索引（mpv `chapter` 属性观察，id 9）；无章节文件为 None。
    pub chapter: Option<i64>,
    /// 章节标题列表（异步 `chapter-list` 回复填充，userdata 101；空 = 无章节）。
    pub chapters: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_idle() {
        let s = PlayerState::default();
        assert!(!s.loaded);
        assert!(!s.ended);
        assert!(s.error.is_none());
        assert!(s.duration_ms.is_none());
        assert!(s.audio_devices.is_none());
        assert!(s.chapters.is_empty());
        assert!(s.tracks.is_empty());
    }
}
