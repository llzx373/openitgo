use openitgo_app::loader::PageLoader;
use openitgo_core::models::PageSource;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    std::env::set_var("OPENITGO_LOG", "1");

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example flip_through -- <path-to-archive>");

    println!("[flip] parsing {:?}", path);
    let comic = openitgo_parser::parse(&path).expect("failed to parse archive");
    let total = comic.total_pages();
    println!("[flip] total pages: {}", total);

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();

    let mut errors = 0;
    let mut timeouts = 0;
    for page_index in 0..total {
        let source = comic.page_source(page_index).cloned().unwrap();
        if let PageSource::ZipEntry {
            ref name, index, ..
        } = source
        {
            println!(
                "[flip] requesting page {} (zip index {}): {}",
                page_index, index, name
            );
        } else {
            println!("[flip] requesting page {}", page_index);
        }
        loader.request_high(epoch, page_index, source);

        let timeout = Duration::from_secs(30);
        let start = Instant::now();
        let mut got = false;
        while start.elapsed() < timeout {
            if let Some(result) = loader.try_recv() {
                if result.epoch == epoch && result.page_index == page_index {
                    match result.image {
                        Ok(img) => {
                            println!(
                                "[flip] page {} OK size {:?}",
                                page_index,
                                img.original_size()
                            );
                        }
                        Err(e) => {
                            println!("[flip] page {} ERROR: {}", page_index, e);
                            errors += 1;
                        }
                    }
                    got = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !got {
            println!("[flip] page {} TIMEOUT", page_index);
            timeouts += 1;
        }
    }

    println!(
        "[flip] done. total={} errors={} timeouts={}",
        total, errors, timeouts
    );
}
