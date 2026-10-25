use openitgo_app::loader::PageLoader;
use openitgo_core::models::PageSource;
use std::time::{Duration, Instant};

fn write_png_image(dir: &std::path::Path, name: &str, width: u32, height: u32) {
    let img = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]));
    img.save(dir.join(name)).unwrap();
}

fn wait_for_full_result(
    loader: &PageLoader,
    epoch: u64,
    page_index: usize,
) -> openitgo_app::loader::LoadResult {
    let start = Instant::now();
    loop {
        if let Some(result) = loader.try_recv() {
            if result.epoch == epoch && result.page_index == page_index && !result.thumbnail {
                return result;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn sample_cbr_sources() -> Vec<PageSource> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../openitgo-parser/tests/sample.cbr");
    let comic = openitgo_parser::parse(&path).unwrap();
    let sources: Vec<_> = comic.volumes[0]
        .pages
        .iter()
        .map(|page| page.source.clone())
        .collect();
    assert_eq!(sources.len(), 2);
    sources
}

#[test]
fn test_loader_concurrent_image_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let count = 5;
    for i in 0..count {
        write_png_image(
            tmp.path(),
            &format!("page{:02}.png", i),
            100 + i as u32,
            200,
        );
    }

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();
    for i in 0..count {
        let source = PageSource::File(tmp.path().join(format!("page{:02}.png", i)));
        assert!(loader.request_high(epoch, i, source));
    }

    let start = Instant::now();
    let mut received = 0;
    while received < count {
        if let Some(result) = loader.try_recv() {
            // Full decodes piggyback a thumbnail result; skip those here.
            if result.thumbnail {
                continue;
            }
            assert_eq!(result.epoch, epoch);
            let image = result.image.expect("image should decode");
            let size = image.original_size();
            assert!(size[0] > 0 && size[1] > 0);
            received += 1;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for loader results");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn test_loader_epoch_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    write_png_image(tmp.path(), "page.png", 50, 50);

    let loader = PageLoader::new();
    let old_epoch = loader.next_epoch();
    let new_epoch = loader.next_epoch();

    let source = PageSource::File(tmp.path().join("page.png"));
    assert!(loader.request_high(old_epoch, 0, source.clone()));
    assert!(loader.request_high(new_epoch, 0, source));

    let start = Instant::now();
    let mut received_old = false;
    let mut received_new = false;
    while !received_old || !received_new {
        if let Some(result) = loader.try_recv() {
            if result.thumbnail {
                continue;
            }
            result.image.expect("both epochs should decode");
            received_old |= result.epoch == old_epoch;
            received_new |= result.epoch == new_epoch;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for loader result");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn test_loader_thumbnail_and_full_share_source() {
    let tmp = tempfile::tempdir().unwrap();
    write_png_image(tmp.path(), "page.png", 400, 400);

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();
    let source = PageSource::File(tmp.path().join("page.png"));

    assert!(loader.request_thumbnail(epoch, 0, source.clone()));
    assert!(loader.request_high(epoch, 0, source));

    let start = Instant::now();
    let mut got_thumbnail = false;
    let mut got_full = false;
    while !got_thumbnail || !got_full {
        if let Some(result) = loader.try_recv() {
            let _ = result.image.expect("image should decode");
            if result.thumbnail {
                got_thumbnail = true;
            } else {
                got_full = true;
            }
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for thumbnail and full results");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn test_loader_zip_thumbnail_and_full_share_one_archive_read() {
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("pages.cbz");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("page.png", options).unwrap();
        let image = image::RgbaImage::from_pixel(400, 400, image::Rgba([0, 128, 255, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
    }

    let loader = PageLoader::new();
    loader.set_perf_enabled(true);
    let epoch = loader.next_epoch();
    let source = PageSource::ZipEntry {
        archive: path,
        name: "page.png".to_string(),
        index: 0,
    };
    assert!(loader.request_thumbnail(epoch, 0, source.clone()));
    assert!(loader.request_high(epoch, 0, source));

    let start = Instant::now();
    let mut got_thumbnail = false;
    let mut got_full = false;
    while !got_thumbnail || !got_full {
        if let Some(result) = loader.try_recv() {
            let _ = result.image.expect("image should decode");
            if result.thumbnail {
                got_thumbnail = true;
            } else {
                got_full = true;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }

    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.zip_entry_reads, 1);
    assert_eq!(snapshot.error_results, 0);
}

#[test]
fn test_loader_prewarm_reuses_raw_bytes_without_decoding() {
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("prewarm.cbz");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("page.png", options).unwrap();
        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 128, 255, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
    }

    let loader = PageLoader::new_with_compress(false, 1);
    loader.set_perf_enabled(true);
    let source = PageSource::ZipEntry {
        archive: path,
        name: "page.png".to_string(),
        index: 0,
    };
    let prewarm_epoch = loader.prewarm(vec![source.clone()]).unwrap();
    let start = Instant::now();
    loop {
        if let Some((epoch, result)) = loader.try_recv_prewarm() {
            assert_eq!(epoch, prewarm_epoch);
            result.expect("prewarm should read source bytes");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(loader.perf_snapshot().decode_full_jobs, 0);

    let epoch = loader.next_epoch();
    assert!(loader.request_high(epoch, 0, source));
    let result = wait_for_full_result(&loader, epoch, 0);
    result.image.expect("prewarmed image should decode");
    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.zip_entry_reads, 1);
    assert_eq!(snapshot.decode_full_jobs, 1);
}

#[test]
fn test_loader_rar_sequential_reads_reuse_archive_and_raw_cache() {
    let sources = sample_cbr_sources();
    let loader = PageLoader::new_with_compress(false, 1);
    loader.set_perf_enabled(true);
    let epoch = loader.next_epoch();

    assert!(loader.request_high(epoch, 0, sources[0].clone()));
    let _ = wait_for_full_result(&loader, epoch, 0);
    assert!(loader.request_high(epoch, 1, sources[1].clone()));
    let _ = wait_for_full_result(&loader, epoch, 1);

    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.rar_archive_opens, 1);
    assert_eq!(snapshot.rar_header_skips, 0);
    assert_eq!(snapshot.rar_entry_reads, 2);
    assert_eq!(snapshot.raw_cache_misses, 2);

    assert!(loader.request_high(epoch, 0, sources[0].clone()));
    let _ = wait_for_full_result(&loader, epoch, 0);
    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.rar_archive_opens, 1);
    assert_eq!(snapshot.rar_entry_reads, 2);
    assert_eq!(snapshot.raw_cache_hits, 1);
}

#[test]
fn test_loader_rar_backward_read_reopens_and_repositions() {
    let sources = sample_cbr_sources();
    let loader = PageLoader::new_with_compress(false, 1);
    loader.set_perf_enabled(true);
    let epoch = loader.next_epoch();

    assert!(loader.request_high(epoch, 1, sources[1].clone()));
    let _ = wait_for_full_result(&loader, epoch, 1);
    assert!(loader.request_high(epoch, 0, sources[0].clone()));
    let _ = wait_for_full_result(&loader, epoch, 0);

    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.rar_archive_opens, 2);
    assert_eq!(snapshot.rar_header_skips, 1);
    assert_eq!(snapshot.rar_entry_reads, 2);
}

#[test]
fn test_loader_rar_wrong_password_does_not_reuse_cursor() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../openitgo-parser/tests/fixtures/encrypted-files-pw123.rar");
    let comic = openitgo_parser::parse_with_password(&path, Some("pw123")).unwrap();
    let source = comic.volumes[0].pages[0].source.clone();
    let loader = PageLoader::new_with_compress(false, 1);
    loader.set_perf_enabled(true);
    let passwords = loader.passwords();
    passwords
        .write()
        .unwrap()
        .insert(path.clone(), "nope".to_string());
    let epoch = loader.next_epoch();

    assert!(loader.request_high(epoch, 0, source.clone()));
    assert!(wait_for_full_result(&loader, epoch, 0).image.is_err());
    assert_eq!(loader.perf_snapshot().rar_archive_opens, 1);

    passwords.write().unwrap().insert(path, "pw123".to_string());
    assert!(loader.request_high(epoch, 0, source));
    let _ = wait_for_full_result(&loader, epoch, 0);
    let snapshot = loader.perf_snapshot();
    assert_eq!(snapshot.rar_archive_opens, 2);
    assert_eq!(snapshot.rar_entry_reads, 1);
}
