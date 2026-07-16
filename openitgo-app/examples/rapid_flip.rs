use openitgo_app::loader::PageLoader;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    std::env::set_var("OPENITGO_LOG", "1");

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example rapid_flip -- <path-to-archive>");

    println!("[rapid] parsing {:?}", path);
    let comic = openitgo_parser::parse(&path).expect("failed to parse archive");
    let total = comic.total_pages();
    println!("[rapid] total pages: {}", total);

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();

    // Simulate rapid flipping: request pages quickly, without waiting for each.
    let flip_interval = Duration::from_millis(80);
    let mut request_times = Vec::with_capacity(total);
    let start = Instant::now();
    for page_index in 0..total {
        let source = comic.page_source(page_index).cloned().unwrap();
        loader.request_high(epoch, page_index, source);
        request_times.push((page_index, Instant::now()));
        std::thread::sleep(flip_interval);
    }

    // Drain all results.
    let timeout = Duration::from_secs(30);
    let mut received = 0;
    let mut errors = 0;
    let mut latencies = Vec::with_capacity(total);
    while received < total && start.elapsed() < timeout {
        if let Some(result) = loader.try_recv() {
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

    if !latencies.is_empty() {
        let avg = latencies.iter().sum::<Duration>() / latencies.len() as u32;
        let max = latencies.iter().max().unwrap();
        println!("[rapid] latencies: avg={:.1?} max={:.1?}", avg, max);
    }
    println!(
        "[rapid] done. total={} received={} errors={}",
        total, received, errors
    );
}
