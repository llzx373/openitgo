//! Non-macOS stub so app code compiles unchanged on other platforms.

use crate::error::MediaError;
use crate::state::PlayerState;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct MpvPlayer;

impl MpvPlayer {
    pub fn new(_repaint: Box<dyn Fn() + Send + Sync>) -> Result<Self, MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        Arc::new(Mutex::new(PlayerState::default()))
    }

    pub fn load_file(&self, _path: &Path) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn stop(&self) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn cycle_pause(&self) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_paused(&self, _paused: bool) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn seek_rel_sec(&self, _secs: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn seek_abs_ms(&self, _ms: u64, _exact: bool) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_volume(&self, _volume: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_speed(&self, _speed: f64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_sub_track(&self, _id: Option<i64>) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }

    pub fn set_audio_track(&self, _id: i64) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }
}
