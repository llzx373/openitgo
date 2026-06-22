use rust_reader_app::loader::PageLoader;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    // Force diagnostic logging from the app crate.
    std::env::set_var("RUST_READER_LOG", "1");

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example diagnose_hang -- <path-to-archive>");

    println!("[diagnose] parsing {:?}", path);
    let start = Instant::now();
    let comic = rust_reader_parser::parse(&path).expect("failed to parse archive");
    println!(
        "[diagnose] parsed in {:.1} ms, total pages: {}",
        start.elapsed().as_secs_f64() * 1000.0,
        comic.total_pages()
    );

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();

    for page_index in [0, 1, 2] {
        let Some(source) = comic.page_source(page_index).cloned() else {
            continue;
        };
        println!("[diagnose] requesting page {}", page_index);
        loader.request_high(epoch, page_index, source);
    }

    let timeout = Duration::from_secs(60);
    let start = Instant::now();
    let mut received = 0;
    while received < 3 && start.elapsed() < timeout {
        if let Some(result) = loader.try_recv() {
            println!(
                "[diagnose] result page {} epoch {}: {:?}",
                result.page_index,
                result.epoch,
                result.image.as_ref().map(|i| i.original_size())
            );
            received += 1;
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    println!(
        "[diagnose] finished, received {}/3 in {:.1} s",
        received,
        start.elapsed().as_secs_f64()
    );
}
