//! Manual smoke test: plays a file for ~5 seconds, printing state changes.
//! Usage: cargo run -p openitgo-media --example probe -- <media-file>

use openitgo_media::player::MpvPlayer;
use std::sync::{Arc, Mutex};

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe <media-file>");
    let last = Arc::new(Mutex::new(String::new()));
    let last2 = last.clone();
    let player = MpvPlayer::new(Box::new(move || {
        // State is read below; repaint fires on every property change.
        let _ = &last2;
    }))
    .expect("mpv init failed");
    player
        .load_file(std::path::Path::new(&path))
        .expect("loadfile failed");
    let state = player.state();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let s = state.lock().unwrap();
        let line = format!(
            "pos={}ms dur={:?} paused={} vol={} speed={} video={} tracks={} err={:?}",
            s.position_ms,
            s.duration_ms,
            s.paused,
            s.volume,
            s.speed,
            s.has_video,
            s.tracks.len(),
            s.error
        );
        drop(s);
        let mut l = last.lock().unwrap();
        if *l != line {
            println!("{line}");
            *l = line;
        }
    }
    println!("probe done");
}
