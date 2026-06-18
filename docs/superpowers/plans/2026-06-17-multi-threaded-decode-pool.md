> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将图片解码从单后台线程改为多线程解码池，提升加载速度，同时保持 `PageLoader` 对外 API 不变。

**Architecture:** 保留一个 IO 线程读取文件/压缩包；读取到的图片字节通过 `DecodeJob` 提交给一个由 `std::thread::available_parallelism()` 决定大小的解码池。PDF 渲染仍由 IO 线程处理。解码结果仍通过同一个 `LoadResult` 通道返回。

**Tech Stack:** Rust, `crossbeam-channel`, `std::thread`, `image`, `egui`

---

## File Structure

- `rust-reader-app/src/loader.rs`
  - 新增 `DecodeJob`。
  - 将 `PageLoader` 内部从单 worker 改为 IO 线程 + 解码池。
  - 拆分 `load_page` 为读取字节（IO 线程）与 `decode_image_bytes`（池）。
- `TODO.md`
  - 标记 #15 完成。

---

## Task 1: 重构 `load_page` 为读取 + 解码两阶段

**Files:**
- Modify: `rust-reader-app/src/loader.rs:135-158`

- [ ] **Step 1: 新增 `read_page_bytes` 函数**

```rust
fn read_page_bytes(source: &PageSource) -> Result<(Vec<u8>, Option<String>), String> {
    match source {
        PageSource::PdfPage { .. } => Err("PDF should be rendered on IO thread".to_string()),
        PageSource::File(path) => {
            let hint = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            Ok((bytes, hint))
        }
        PageSource::ZipEntry { archive, name } => {
            let hint = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let mut archive =
                zip::ZipArchive::new(file).map_err(|e| format!("invalid zip archive: {e}"))?;
            let mut entry = archive
                .by_name(name)
                .map_err(|e| format!("zip entry not found: {e}"))?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("failed to read zip entry: {e}"))?;
            Ok((bytes, hint))
        }
        PageSource::RarEntry { archive, name } => {
            let hint = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            let bytes = read_rar_entry(archive, name)?;
            Ok((bytes, hint))
        }
    }
}
```

- [ ] **Step 2: 修改 `decode_image` 为 `decode_image_bytes`**

将现有 `decode_image(bytes: &[u8])` 改为接收 `format_hint: Option<&str>`：

```rust
fn decode_image_bytes(bytes: &[u8], format_hint: Option<&str>) -> Result<ColorImage, String> {
    let format = format_hint.and_then(image::ImageFormat::from_extension);
    let image = if let Some(format) = format {
        image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?
    } else {
        image::load_from_memory(bytes).map_err(|e| e.to_string())?
    };
    let image = downsample_if_needed(image);
    let size = [image.width() as _, image.height() as _];
    let rgba = image.to_rgba8();
    let pixels = rgba.as_flat_samples();
    Ok(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()))
}
```

- [ ] **Step 3: 修改 `load_page` 使用新函数**

```rust
fn load_page(source: &PageSource) -> Result<ColorImage, String> {
    match source {
        PageSource::PdfPage {
            document,
            page_number,
        } => return render_pdf_page(document, *page_number),
        _ => {}
    };
    let (bytes, hint) = read_page_bytes(source)?;
    decode_image_bytes(&bytes, hint.as_deref())
}
```

- [ ] **Step 4: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 5: 运行现有 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "refactor(loader): split read and decode into two stages"
```

---

## Task 2: 引入 `DecodeJob` 并改造 `PageLoader` 为多线程

**Files:**
- Modify: `rust-reader-app/src/loader.rs:13-124`

- [ ] **Step 1: 新增 `DecodeJob` 类型**

在 `LoadResult` 之后添加：

```rust
struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}
```

- [ ] **Step 2: 修改 `PageLoader` 字段**

```rust
pub struct PageLoader {
    high_sender: Sender<LoadRequest>,
    low_sender: Sender<LoadRequest>,
    receiver: Receiver<LoadResult>,
    epoch: Arc<AtomicU64>,
    _io_worker: thread::JoinHandle<()>,
    _decode_workers: Vec<thread::JoinHandle<()>>,
}
```

- [ ] **Step 3: 修改 `PageLoader::new`**

```rust
pub fn new() -> Self {
    let (high_sender, high_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(64);
    let (low_sender, low_receiver): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(64);
    let (result_sender, receiver): (Sender<LoadResult>, Receiver<LoadResult>) = bounded(64);
    let (decode_sender, decode_receiver): (Sender<DecodeJob>, Receiver<DecodeJob>) = bounded(64);

    let result_sender_for_io = result_sender.clone();
    let io_worker = thread::spawn(move || loop {
        if let Ok(req) = high_receiver.try_recv() {
            process_io_request(req, &result_sender_for_io, &decode_sender);
            continue;
        }
        select! {
            recv(high_receiver) -> req => {
                if let Ok(req) = req {
                    process_io_request(req, &result_sender_for_io, &decode_sender);
                }
            }
            recv(low_receiver) -> req => {
                if let Ok(req) = req {
                    process_io_request(req, &result_sender_for_io, &decode_sender);
                }
            }
        }
    });

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);
    let mut decode_workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let decode_receiver = decode_receiver.clone();
        let result_sender = result_sender.clone();
        decode_workers.push(thread::spawn(move || {
            while let Ok(job) = decode_receiver.recv() {
                let image = decode_image_bytes(&job.bytes, job.format_hint.as_deref());
                let _ = result_sender.send(LoadResult {
                    epoch: job.epoch,
                    page_index: job.page_index,
                    image,
                });
            }
        }));
    }

    Self {
        high_sender,
        low_sender,
        receiver,
        epoch: Arc::new(AtomicU64::new(1)),
        _io_worker: io_worker,
        _decode_workers: decode_workers,
    }
}
```

- [ ] **Step 4: 新增 `process_io_request`**

```rust
fn process_io_request(
    req: LoadRequest,
    result_sender: &Sender<LoadResult>,
    decode_sender: &Sender<DecodeJob>,
) {
    match req.source {
        PageSource::PdfPage {
            document,
            page_number,
        } => {
            let image = render_pdf_page(&document, page_number);
            let _ = result_sender.send(LoadResult {
                epoch: req.epoch,
                page_index: req.page_index,
                image,
            });
        }
        _ => match read_page_bytes(&req.source) {
            Ok((bytes, format_hint)) => {
                let _ = decode_sender.send(DecodeJob {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    bytes,
                    format_hint,
                });
            }
            Err(e) => {
                let _ = result_sender.send(LoadResult {
                    epoch: req.epoch,
                    page_index: req.page_index,
                    image: Err(e),
                });
            }
        },
    }
}
```

- [ ] **Step 5: 删除旧的 `process_request` 和 `load_page`（如果已无用）**

`load_page` 在拆分后仍被 `process_io_request` 间接使用？不，拆分后 `process_io_request` 直接调用 `read_page_bytes` 和 `render_pdf_page`。因此删除 `load_page` 和 `process_request`。

- [ ] **Step 6: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 7: 运行现有 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 8: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "feat(loader): add multi-threaded decode pool"
```

---

## Task 3: 新增并发解码 smoke 测试

**Files:**
- Modify: `rust-reader-app/src/loader.rs` test module

- [ ] **Step 1: 添加并发解码测试**

在 `mod tests` 中新增：

```rust
#[test]
fn test_loader_decodes_multiple_images_concurrently() {
    let tmp = tempfile::tempdir().unwrap();
    let count = 8;
    let mut epochs = Vec::new();
    let loader = PageLoader::new();

    for i in 0..count {
        let path = tmp.path().join(format!("sample_{i}.png"));
        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([i as u8, 0, 0, 255]));
        image.save(&path).unwrap();
        let epoch = loader.next_epoch();
        epochs.push(epoch);
        loader.request_high(epoch, i, PageSource::File(path));
    }

    let mut received = 0;
    let start = Instant::now();
    while received < count && start.elapsed() < Duration::from_secs(10) {
        if let Some(result) = loader.try_recv() {
            let pos = epochs.iter().position(|&e| e == result.epoch).expect("unknown epoch");
            epochs.remove(pos);
            let image = result.image.expect("image should decode");
            assert_eq!(image.size, [64, 64]);
            received += 1;
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(received, count, "all concurrent images should decode");
}
```

- [ ] **Step 2: 运行 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "test(loader): add concurrent decode smoke test"
```

---

## Task 4: 全量验证与 TODO 更新

**Files:**
- Modify: `TODO.md`

- [ ] **Step 1: 运行完整检查**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all PASS

- [ ] **Step 2: 更新 TODO.md**

将 `- [ ] 15. 多线程解码池` 改为 `- [x]`。

- [ ] **Step 3: 提交并推送**

```bash
git add TODO.md
git commit -m "chore: mark #15 multi-threaded decode pool as done"
git push
```

---

## Self-Review

- **Spec coverage:**
  - 图片解码池化：Task 2
  - IO 与解码分离：Task 1
  - PDF 留在 IO 线程：Task 2 `process_io_request`
  - 线程数 = CPU 核心数：Task 2 `available_parallelism`
  - API 不变：Task 2 保留 `PageLoader` 公开方法
  - 测试：Task 3
- **Placeholder scan：** 无 TBD/占位符。
- **Type consistency：** `DecodeJob` 字段、`process_io_request` 参数与 `PageLoader::new` 一致。
