//! Inject the Homebrew libmpv link search path on macOS.
//!
//! libmpv-sys emits `cargo:rustc-link-lib=mpv` but Homebrew's /opt/homebrew/lib
//! (or /usr/local/lib on Intel) is not in the default linker search path.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    for dir in ["/opt/homebrew/lib", "/usr/local/lib"] {
        if std::path::Path::new(dir).join("libmpv.dylib").exists() {
            println!("cargo:rustc-link-search=native={dir}");
            return;
        }
    }
    println!(
        "cargo:warning=libmpv.dylib not found in /opt/homebrew/lib or /usr/local/lib; install mpv via Homebrew"
    );
}
