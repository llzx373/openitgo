#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod apply;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod args;
pub mod chapters;
#[cfg(target_os = "macos")]
pub mod cover;
pub mod devices;
pub mod error;
#[cfg(not(target_os = "macos"))]
pub mod cover {
    use crate::error::MediaError;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub fn cover_output_path(covers_dir: &Path, id: &str) -> PathBuf {
        covers_dir.join(format!("{id}.jpg"))
    }

    pub fn generate_cover(
        _input: &Path,
        _output: &Path,
        _timeout: Duration,
    ) -> Result<(), MediaError> {
        Err(MediaError::Init("媒体播放暂仅支持 macOS".to_string()))
    }
}
#[cfg(target_os = "macos")]
pub mod player;
#[cfg(not(target_os = "macos"))]
pub mod player_stub;
#[cfg(not(target_os = "macos"))]
pub use player_stub as player;
#[cfg(target_os = "macos")]
pub mod render;
pub mod state;
pub mod time;
pub mod tracks;

pub use devices::AudioDevice;
pub use error::MediaError;
pub use player::MpvPlayer;
pub use state::PlayerState;
pub use tracks::{TrackInfo, TrackKind};
