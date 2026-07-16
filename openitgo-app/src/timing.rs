use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static INIT: std::sync::Once = std::sync::Once::new();

fn enabled() -> bool {
    INIT.call_once(|| {
        let enabled = std::env::var_os("OPENITGO_LOG").is_some()
            || std::env::var_os("OPENITGO_TIMING").is_some();
        ENABLED.store(enabled, Ordering::Relaxed);
    });
    ENABLED.load(Ordering::Relaxed)
}

pub fn time<F: FnOnce() -> R, R>(label: &str, f: F) -> R {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    eprintln!(
        "[openitgo] {}: {:.1} ms",
        label,
        elapsed.as_secs_f64() * 1000.0
    );
    result
}

pub fn log(label: &str) {
    if enabled() {
        eprintln!("[openitgo] {}", label);
    }
}

pub fn log_if_slow<F: FnOnce() -> R, R>(label: &str, threshold: Duration, f: F) -> R {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    if elapsed > threshold {
        eprintln!(
            "[openitgo] {}: {:.1} ms",
            label,
            elapsed.as_secs_f64() * 1000.0
        );
    }
    result
}
