> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 ZIP 漫画缓存 `ZipArchive` 句柄和条目索引，避免翻页时重复打开/解析压缩包。

**Architecture:** 解析阶段给 `PageSource::ZipEntry` 增加 `index`；`PageLoader` 的 IO 线程维护 `HashMap<PathBuf, ZipArchive<File>>`，读取时按索引命中缓存，失败则清除重试。

**Tech Stack:** Rust, `zip`, `crossbeam-channel`, `rust-reader-core`, `rust-reader-parser`, `rust-reader-app`

---

## File Structure

- `rust-reader-core/src/models.rs`
  - 给 `PageSource::ZipEntry` 增加 `index: usize`。
- `rust-reader-parser/src/zip.rs`
  - 解析时记录并写入 `index`。
  - 更新测试中的构造。
- `rust-reader-app/src/loader.rs`
  - 新增 `ZipCache` 结构。
  - IO 线程读取 `ZipEntry` 时使用缓存和索引。
- `TODO.md`
  - 标记 #16 完成。

---

## Task 1: 给 `PageSource::ZipEntry` 增加 `index`

**Files:**
- Modify: `rust-reader-core/src/models.rs:42-45`
- Test: `rust-reader-core/src/models.rs:80-98`

- [ ] **Step 1: 修改 `PageSource::ZipEntry`**

```rust
ZipEntry {
    archive: PathBuf,
    name: String,
    index: usize,
},
```

- [ ] **Step 2: 修复 `test_page_source_file` 之外的其他测试（如有）**  
当前只有 `File` 测试受影响，无需改动。

- [ ] **Step 3: 运行 core 测试**

Run: `cargo test -p rust-reader-core`
Expected: PASS

- [ ] **Step 4: 提交**

```bash
git add rust-reader-core/src/models.rs
git commit -m "feat(core): add index to PageSource::ZipEntry"
```

---

## Task 2: 在 ZIP 解析时填充 `index`

**Files:**
- Modify: `rust-reader-parser/src/zip.rs:21-47`
- Test: `rust-reader-parser/src/zip.rs:74-129`

- [ ] **Step 1: 重构解析循环，记录索引**

将：

```rust
let mut names: Vec<String> = Vec::new();
for i in 0..archive.len() {
    let entry = archive.by_index(i)?;
    if entry.is_file() && is_image_name(entry.name()) {
        names.push(entry.name().to_string());
    }
}
names.sort();
```

改为：

```rust
let mut entries: Vec<(usize, String)> = Vec::new();
for i in 0..archive.len() {
    let entry = archive
        .by_index(i)
        .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
    if entry.is_file() && is_image_name(entry.name()) {
        entries.push((i, entry.name().to_string()));
    }
}
entries.sort_by(|a, b| a.1.cmp(&b.1));
```

- [ ] **Step 2: 构造 PageSource 时使用 index**

将 `pages` 构造改为：

```rust
let pages: Vec<Page> = entries
    .into_iter()
    .enumerate()
    .map(|(idx, (zip_index, name))| Page {
        index: idx,
        source: PageSource::ZipEntry {
            archive: archive_path.clone(),
            name,
            index: zip_index,
        },
    })
    .collect();
```

- [ ] **Step 3: 更新测试中的 `PageSource::ZipEntry` 构造**

在 `test_parse_cbz` 中，将期望值改为包含 `index: 0`（因为 `01.png` 是 zip 中第一个文件）。

```rust
assert_eq!(
    comic.volumes[0].pages[0].source,
    PageSource::ZipEntry {
        archive: path,
        name: "01.png".to_string(),
        index: 0,
    }
);
```

- [ ] **Step 4: 运行 parser 测试**

Run: `cargo test -p rust-reader-parser`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add rust-reader-parser/src/zip.rs
git commit -m "feat(parser): fill ZipEntry index during zip parsing"
```

---

## Task 3: 在 Loader 中缓存 `ZipArchive` 句柄

**Files:**
- Modify: `rust-reader-app/src/loader.rs`

- [ ] **Step 1: 新增 `ZipCache` 结构**

在 `loader.rs` 中合适位置添加：

```rust
use std::collections::HashMap;

struct ZipCache {
    archives: HashMap<std::path::PathBuf, zip::ZipArchive<std::fs::File>>,
}

impl ZipCache {
    fn new() -> Self {
        Self {
            archives: HashMap::new(),
        }
    }

    fn get_or_open(
        &mut self,
        path: &std::path::Path,
    ) -> Result<&mut zip::ZipArchive<std::fs::File>, String> {
        if !self.archives.contains_key(path) {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            self.archives.insert(path.to_path_buf(), archive);
        }
        self.archives
            .get_mut(path)
            .ok_or_else(|| "zip archive missing from cache".to_string())
    }

    fn remove(&mut self, path: &std::path::Path) {
        self.archives.remove(path);
    }
}
```

- [ ] **Step 2: 在 `PageLoader` 中持有 `ZipCache`**

由于 `ZipArchive<File>` 不是 `Send`，缓存必须由 IO 线程独占。最简单的方式是把缓存放在 IO 线程的闭包里，不作为 `PageLoader` 字段。

修改 `PageLoader::new` 的 IO worker 闭包：

```rust
let io_worker = thread::spawn(move || {
    let mut zip_cache = ZipCache::new();
    loop {
        if let Ok(req) = high_receiver.try_recv() {
            process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
            continue;
        }
        select! {
            recv(high_receiver) -> req => {
                if let Ok(req) = req {
                    process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
                }
            }
            recv(low_receiver) -> req => {
                if let Ok(req) = req {
                    process_io_request(req, &result_sender_for_io, &decode_sender, &mut zip_cache);
                }
            }
        }
    }
});
```

- [ ] **Step 3: 更新 `process_io_request` 签名**

```rust
fn process_io_request(
    req: LoadRequest,
    result_sender: &Sender<LoadResult>,
    decode_sender: &Sender<DecodeJob>,
    zip_cache: &mut ZipCache,
)
```

- [ ] **Step 4: 更新 `read_page_bytes` 使用缓存和索引**

```rust
PageSource::ZipEntry { archive, name, index } => {
    let hint = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());
    let bytes = read_zip_entry(zip_cache, archive, *index)?;
    Ok((bytes, hint))
}
```

- [ ] **Step 5: 新增 `read_zip_entry` 辅助函数**

```rust
fn read_zip_entry(
    zip_cache: &mut ZipCache,
    archive_path: &std::path::Path,
    index: usize,
) -> Result<Vec<u8>, String> {
    match zip_cache
        .get_or_open(archive_path)
        .and_then(|a| a.by_index(index).map_err(|e| e.to_string()))
    {
        Ok(mut entry) => {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("failed to read zip entry: {e}"))?;
            Ok(bytes)
        }
        Err(e) => {
            // 可能是文件被外部修改/删除，清除缓存后重试一次
            zip_cache.remove(archive_path);
            let mut archive = zip_cache.get_or_open(archive_path)?;
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("failed to read zip entry: {e}"))?;
            Ok(bytes)
        }
    }
}
```

- [ ] **Step 6: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 7: 运行 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 8: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "feat(loader): cache open ZipArchive handles by path"
```

---

## Task 4: 新增 loader 缓存测试

**Files:**
- Modify: `rust-reader-app/src/loader.rs` test module

- [ ] **Step 1: 添加 ZIP 多页读取测试**

```rust
#[test]
fn test_loader_reads_multiple_zip_entries_with_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.cbz");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for i in 0..4 {
            zip.start_file(format!("{:02}.png", i), options).unwrap();
            let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([i as u8, 0, 0, 255]));
            let mut buf = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
                .unwrap();
            zip.write_all(&buf).unwrap();
        }
        zip.finish().unwrap();
    }

    let loader = PageLoader::new();
    let epoch = loader.next_epoch();
    let sources: Vec<_> = (0..4)
        .map(|i| PageSource::ZipEntry {
            archive: path.clone(),
            name: format!("{:02}.png", i),
            index: i,
        })
        .collect();

    for (i, source) in sources.iter().enumerate() {
        loader.request_high(epoch, i, source.clone());
    }

    let mut received = 0;
    let start = Instant::now();
    while received < 4 && start.elapsed() < Duration::from_secs(10) {
        if let Some(result) = loader.try_recv() {
            assert_eq!(result.epoch, epoch);
            let image = result.image.expect("image should decode");
            assert_eq!(image.size, [32, 32]);
            received += 1;
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(received, 4);
}
```

- [ ] **Step 2: 运行 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "test(loader): verify zip archive handle cache"
```

---

## Task 5: 全量验证与 TODO 更新

**Files:**
- Modify: `TODO.md`

- [ ] **Step 1: 运行完整检查**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all PASS

- [ ] **Step 2: 更新 TODO.md**

将 `- [ ] 16. 压缩包索引缓存` 改为 `- [x]`。

- [ ] **Step 3: 提交并推送**

```bash
git add TODO.md
git commit -m "chore: mark #16 archive index cache as done"
git push
```

---

## Self-Review

- **Spec coverage:**
  - `PageSource::ZipEntry.index`：Task 1
  - parser 填充 index：Task 2
  - loader 缓存 ZipArchive：Task 3
  - 缓存失效/重试：Task 3 `read_zip_entry`
  - 测试：Task 4
  - TODO 更新：Task 5
- **Placeholder scan：** 无 TBD。
- **Type consistency：** `PageSource::ZipEntry` 三字段、`read_zip_entry` 签名、`process_io_request` 签名一致。
