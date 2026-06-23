> **Status:** 已实现。ZIP 压缩包索引缓存已落地。
>
> **注意：** 文档目标中的 TODO #16 为历史编号，详见 `TODO.md` 中的「历史 TODO 编号对照表」。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 ZIP 漫画缓存 `ZipArchive` 句柄和条目索引，避免翻页时重复打开/解析压缩包。

**Architecture:** 解析阶段记录每个图片条目在 `ZipArchive` 中的索引并写入 `PageSource::ZipEntry.index`；`PageLoader` 的 IO 线程维护一个 `HashMap<PathBuf, ZipArchive<File>>` 缓存，读取时优先按索引命中缓存，未命中则打开并缓存。RAR 不在本次范围内。

**Tech Stack:** Rust, `zip`, `rust-reader-core`, `rust-reader-parser`, `rust-reader-app`

---

## 范围

- 仅处理 `PageSource::ZipEntry`。
- RAR、文件夹、PDF 保持现状。
- 缓存无淘汰，应用退出时释放。

## 数据模型变更

在 `rust-reader-core/src/models.rs` 中：

```rust
pub enum PageSource {
    File(PathBuf),
    ZipEntry {
        archive: PathBuf,
        name: String,
        index: usize, // 新增
    },
    RarEntry {
        archive: PathBuf,
        name: String,
    },
    PdfPage {
        document: PathBuf,
        page_number: usize,
    },
}
```

## Parser 变更

`rust-reader-parser/src/zip.rs`：
- 打开 `ZipArchive` 后，遍历条目时记录满足 `is_image_name` 的条目的原始索引 `i`。
- 构造 `PageSource::ZipEntry { archive, name, index: i }`。

## Loader 变更

`rust-reader-app/src/loader.rs`：
- 新增内部类型 `ZipCache`（或 `ArchiveCache`）：

```rust
struct ZipCache {
    archives: HashMap<PathBuf, zip::ZipArchive<std::fs::File>>,
}

impl ZipCache {
    fn get_or_open(&mut self, path: &Path) -> Result<&mut zip::ZipArchive<std::fs::File>, String> {
        if !self.archives.contains_key(path) {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            self.archives.insert(path.to_path_buf(), archive);
        }
        self.archives
            .get_mut(path)
            .ok_or_else(|| "zip archive missing from cache".to_string())
    }
}
```

- `read_page_bytes` 对 `PageSource::ZipEntry { archive, index, .. }`：
  - 从缓存取 archive；用 `archive.by_index(*index)` 读取条目。
  - 若读取失败，从缓存移除该路径，重新打开并重试一次。

## 线程安全

- `ZipArchive<File>` 不是 `Send`，因此缓存必须由 IO 线程独占。
- 读取仍在 IO 线程完成，解码再提交给 #15 的解码池。这与现有架构一致。

## 错误处理

- 压缩包在缓存期间被外部修改/删除：清除缓存条目后重试一次，仍失败返回错误。
- 索引越界：返回错误，不从缓存移除（属于数据损坏）。

## 测试计划

- Parser：验证 `zip.rs` 生成的 `PageSource::ZipEntry` 包含正确 `index`。
- Loader：连续读取同一 ZIP 多个页面成功；模拟句柄失效后重试成功。
- 全量 `cargo test --workspace` 通过。

## 变更文件

- `rust-reader-core/src/models.rs`
- `rust-reader-parser/src/zip.rs`
- `rust-reader-parser/src/lib.rs`（如需要同步导出/使用）
- `rust-reader-app/src/loader.rs`
- `TODO.md`
