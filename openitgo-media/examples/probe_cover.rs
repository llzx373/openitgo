//! Manual smoke test: generates a media cover via the headless image VO.
//! Usage: cargo run -p openitgo-media --example probe_cover -- <media-file> [out.jpg]

fn main() {
    let mut args = std::env::args_os().skip(1).map(std::path::PathBuf::from);
    let input = args
        .next()
        .expect("usage: probe_cover <media-file> [out.jpg]");
    let output = args
        .next()
        .unwrap_or_else(|| std::env::temp_dir().join("openitgo-probe-cover.jpg"));
    match openitgo_media::cover::generate_cover(&input, &output, std::time::Duration::from_secs(15))
    {
        Ok(()) => println!("cover written: {}", output.display()),
        Err(e) => {
            eprintln!("cover failed: {e}");
            std::process::exit(1);
        }
    }
}
