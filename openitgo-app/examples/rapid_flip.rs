use openitgo_app::loader::PageLoader;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("OPENITGO_LOG", "1");

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example rapid_flip -- <path-to-archive>");

    println!("[rapid] parsing {:?}", path);
    let comic = openitgo_parser::parse(&path).expect("failed to parse archive");
    let total = comic.total_pages();
    println!("[rapid] total pages: {}", total);

    let decode_threads = std::env::var("OPENITGO_DECODE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let loader = PageLoader::new_with_compress(false, decode_threads);
    loader.set_perf_enabled(true);
    let epoch = loader.next_epoch();

    let flip_interval = Duration::from_millis(80);
    let mut request_times = Vec::with_capacity(total);
    let start = Instant::now();
    for page_index in 0..total {
        let source = comic.page_source(page_index).cloned().unwrap();
        if !loader.request_high(epoch, page_index, source) {
            return Err(format!("high-priority queue full at page {page_index}").into());
        }
        request_times.push((page_index, Instant::now()));
        std::thread::sleep(flip_interval);
    }

    let timeout = Duration::from_secs(30);
    let mut received = 0;
    let mut errors = 0;
    let mut latencies = Vec::with_capacity(total);
    while received < total && start.elapsed() < timeout {
        if let Some(result) = loader.try_recv() {
            if result.thumbnail {
                continue;
            }
            if result.epoch == epoch {
                match result.image {
                    Ok(img) => {
                        if let Some(&(_, t)) =
                            request_times.iter().find(|(p, _)| *p == result.page_index)
                        {
                            latencies.push(t.elapsed());
                        }
                        println!(
                            "[rapid] page {} OK size {:?}",
                            result.page_index,
                            img.original_size()
                        );
                    }
                    Err(e) => {
                        println!("[rapid] page {} ERROR: {}", result.page_index, e);
                        errors += 1;
                    }
                }
                received += 1;
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    let mut latency_us: Vec<u64> = latencies
        .iter()
        .map(|latency| latency.as_micros() as u64)
        .collect();
    latency_us.sort_unstable();
    let report = serde_json::json!({
        "scenario": "rapid_flip",
        "decode_threads": decode_threads,
        "source": path,
        "total": total,
        "received": received,
        "errors": errors,
        "timed_out": received < total,
        "latency_samples": latency_us.len(),
        "latency_p50_us": percentile(&latency_us, 50),
        "latency_p95_us": percentile(&latency_us, 95),
        "latency_max_us": latency_us.last().copied().unwrap_or(0),
        "loader": loader.perf_snapshot(),
    });
    write_report(&report)?;
    println!(
        "[rapid] done. total={} received={} errors={}",
        total, received, errors
    );
    if received < total {
        return Err("timed out waiting for all pages".into());
    }
    if errors > 0 {
        return Err(format!("{errors} pages failed to decode").into());
    }
    Ok(())
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * percentile).div_ceil(100).max(1);
    sorted[rank.min(sorted.len()) - 1]
}

fn write_report(report: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(report)?;
    println!("[perf] {json}");
    if let Some(path) = std::env::var_os("OPENITGO_PERF_JSON") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_nearest_rank() {
        assert_eq!(percentile(&[1, 2, 3, 4, 100], 50), 3);
        assert_eq!(percentile(&[1, 2, 3, 4, 100], 95), 100);
        assert_eq!(percentile(&[], 95), 0);
    }
}
