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
    /// `None` until the first async audio-device-list reply lands — keeps
    /// "not enumerated yet" distinct from a genuinely empty enumeration.
    pub audio_devices: Option<Vec<AudioDevice>>,
}
