use rust_reader_app::loader::{LoadResult, PageLoader};
use rust_reader_core::models::PageSource;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn test_loader_loads_folder_image() {
    let tmp = tempfile::tempdir().unwrap();
    let sample_path = tmp.path().join("sample.png");

    let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]));
    image.save(&sample_path).unwrap();

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();
    loader.request(epoch, 0, PageSource::File(PathBuf::from(&sample_path)));

    let result = wait_for_result(&loader, epoch, 0, Duration::from_secs(5));
    let color_image = result.image.expect("expected image to load successfully");
    assert_eq!(color_image.size, [64, 64]);
}

fn wait_for_result(
    loader: &PageLoader,
    expected_epoch: u64,
    expected_page_index: usize,
    timeout: Duration,
) -> LoadResult {
    let start = Instant::now();
    loop {
        if let Some(result) = loader.try_recv() {
            if result.epoch == expected_epoch && result.page_index == expected_page_index {
                return result;
            }
        }
        if start.elapsed() > timeout {
            panic!("timed out waiting for load result");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
