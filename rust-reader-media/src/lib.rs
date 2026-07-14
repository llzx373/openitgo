pub mod error;
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

pub use error::MediaError;
pub use player::MpvPlayer;
pub use state::PlayerState;
pub use tracks::{TrackInfo, TrackKind};
