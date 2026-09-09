//! zip 写出引擎：把本地文件/目录树打包成 zip。
//! 取消约定与 extract.rs 一致：cancel 置位不作为错误返回——调用方经
//! 进度通道的 `Failed("已取消")` 事件感知取消，`create_zip` 返回 `Ok(())`，
//! 半成品 zip 删除。致命错误（dest 不可写、源文件写入期读取失败等）
//! 发 `Failed(错误文本)`、删半成品并返回 Err。
//! 符号链接一律跳过（不跟进目录、不按目标存文件），单项读取失败跳过不计。

use crate::traits::ParseError;
use crossbeam_channel::Sender;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zip::write::SimpleFileOptions;

#[derive(Debug, Clone, Copy)]
pub struct ZipWriteOptions {
    /// true = Deflated 压缩；false = Stored 仅打包。
    pub compress: bool,
}

impl Default for ZipWriteOptions {
    fn default() -> Self {
        Self { compress: true }
    }
}

#[derive(Debug)]
pub enum ZipWriteProgress {
    Started {
        total_files: usize,
        total_bytes: u64,
    },
    EntryDone {
        name: String,
        bytes: u64,
    },
    Finished {
        written: u64,
    },
    Failed(String),
}

/// 预扫描收集到的单个条目。
struct ZipItem {
    disk: PathBuf,
    /// 包内条目名：相对各 source 父目录的路径，'/' 分隔，目录以 '/' 结尾。
    name: String,
    is_dir: bool,
    bytes: u64,
}

/// 把 sources 打包进 dest zip；条目名 = 相对各 source 父目录的路径
/// （多 source 时各自 basename 为根）。进度经 channel 上报。
pub fn create_zip(
    sources: &[PathBuf],
    dest: &Path,
    opts: &ZipWriteOptions,
    progress: Sender<ZipWriteProgress>,
    cancel: Arc<AtomicBool>,
) -> Result<(), ParseError> {
    // 预扫描：先收集全部条目再创建 dest，避免 dest 落在 source 目录内
    // 时把自己打包进去。单项失败容错跳过。
    let mut items: Vec<ZipItem> = Vec::new();
    for src in sources {
        scan_source(src, &mut items);
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    let total_files = items.iter().filter(|i| !i.is_dir).count();
    let total_bytes: u64 = items.iter().map(|i| i.bytes).sum();
    send_progress(
        &progress,
        ZipWriteProgress::Started {
            total_files,
            total_bytes,
        },
    );

    let file = match std::fs::File::create(verbatim_path(dest)) {
        Ok(f) => f,
        Err(e) => {
            let err = ParseError::Io(e);
            send_failed(&progress, &err);
            return Err(err);
        }
    };
    let method = if opts.compress {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    };
    let options = SimpleFileOptions::default().compression_method(method);
    let mut writer = zip::ZipWriter::new(file);

    let mut written = 0u64;
    let result = write_items(
        &mut writer,
        &items,
        options,
        &progress,
        &cancel,
        &mut written,
    );
    let finish = result.and_then(|()| {
        writer
            .finish()
            .map_err(zip_err)
            .map_err(WriteFail::from)
            .map(|_| ())
    });
    // writer 已随 finish/drop 释放句柄，可安全删除半成品。
    match finish {
        Ok(()) => {
            send_progress(&progress, ZipWriteProgress::Finished { written });
            Ok(())
        }
        Err(WriteFail::Cancelled) => {
            let _ = std::fs::remove_file(verbatim_path(dest));
            send_progress(&progress, ZipWriteProgress::Failed("已取消".to_string()));
            Ok(())
        }
        Err(WriteFail::Fatal(e)) => {
            let _ = std::fs::remove_file(verbatim_path(dest));
            send_failed(&progress, &e);
            Err(e)
        }
    }
}

enum WriteFail {
    Cancelled,
    Fatal(ParseError),
}

impl From<ParseError> for WriteFail {
    fn from(e: ParseError) -> Self {
        WriteFail::Fatal(e)
    }
}

fn write_items(
    writer: &mut zip::ZipWriter<std::fs::File>,
    items: &[ZipItem],
    options: SimpleFileOptions,
    progress: &Sender<ZipWriteProgress>,
    cancel: &Arc<AtomicBool>,
    written: &mut u64,
) -> Result<(), WriteFail> {
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            return Err(WriteFail::Cancelled);
        }
        if item.is_dir {
            writer.add_directory(&item.name, options).map_err(zip_err)?;
            continue;
        }
        writer.start_file(&item.name, options).map_err(zip_err)?;
        let mut src = std::fs::File::open(verbatim_path(&item.disk)).map_err(ParseError::Io)?;
        let n = std::io::copy(&mut src, writer).map_err(ParseError::Io)?;
        *written += n;
        send_progress(
            progress,
            ZipWriteProgress::EntryDone {
                name: item.name.clone(),
                bytes: n,
            },
        );
    }
    Ok(())
}

/// 预扫描单个 source：目录递归展开（不跟进符号链接），单项失败跳过。
fn scan_source(src: &Path, items: &mut Vec<ZipItem>) {
    let Ok(meta) = std::fs::symlink_metadata(verbatim_path(src)) else {
        return;
    };
    if meta.is_symlink() {
        return;
    }
    let Some(parent) = src.parent() else { return };
    if meta.is_file() {
        if let Some(name) = entry_name(parent, src, false) {
            items.push(ZipItem {
                disk: src.to_path_buf(),
                name,
                is_dir: false,
                bytes: meta.len(),
            });
        }
        return;
    }
    if !meta.is_dir() {
        return;
    }
    if let Some(name) = entry_name(parent, src, true) {
        items.push(ZipItem {
            disk: src.to_path_buf(),
            name,
            is_dir: true,
            bytes: 0,
        });
    }
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(verbatim_path(&dir)) else {
            continue;
        };
        // 排序只为最终条目序确定性（收尾还会按名整排一次）。
        let mut children: Vec<_> = rd.flatten().collect();
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let p = child.path();
            let Ok(ft) = child.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if let Some(name) = entry_name(parent, &p, true) {
                    items.push(ZipItem {
                        disk: p.clone(),
                        name,
                        is_dir: true,
                        bytes: 0,
                    });
                }
                stack.push(p);
            } else if ft.is_file() {
                let bytes = child.metadata().map(|m| m.len()).unwrap_or(0);
                if let Some(name) = entry_name(parent, &p, false) {
                    items.push(ZipItem {
                        disk: p,
                        name,
                        is_dir: false,
                        bytes,
                    });
                }
            }
        }
    }
}

/// 包内条目名：相对 source 父目录的组件经 lossy 转字符串后 '/' 拼接；
/// 拒绝空/`.`/`..` 组件（防绝对路径与路径穿越），目录名以 '/' 结尾。
fn entry_name(parent: &Path, p: &Path, is_dir: bool) -> Option<String> {
    let rel = p.strip_prefix(parent).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for comp in rel.components() {
        let Component::Normal(os) = comp else {
            return None;
        };
        let s = os.to_string_lossy();
        if s.is_empty() || s == "." || s == ".." {
            return None;
        }
        parts.push(s.into_owned());
    }
    if parts.is_empty() {
        return None;
    }
    let mut name = parts.join("/");
    if is_dir {
        name.push('/');
    }
    Some(name)
}

fn zip_err(e: zip::result::ZipError) -> ParseError {
    match e {
        zip::result::ZipError::Io(io) => ParseError::Io(io),
        other => ParseError::InvalidArchive(other.to_string()),
    }
}

fn send_progress(progress: &Sender<ZipWriteProgress>, event: ZipWriteProgress) {
    // 接收方可能已关闭（如 UI 已退出），发送失败不视为压缩失败。
    let _ = progress.send(event);
}

fn send_failed(progress: &Sender<ZipWriteProgress>, err: &ParseError) {
    send_progress(progress, ZipWriteProgress::Failed(err.to_string()));
}

/// Windows 长路径（>MAX_PATH=260）支持：manifest 未声明 longPathAware，
/// 需加 `\\?\`（verbatim）前缀；与 app 侧 file_ops 同款约定。
/// 非 Windows 恒等。
#[cfg(windows)]
fn verbatim_path(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    if !p.is_absolute()
        || s.starts_with(r"\\?\")
        || s.starts_with(r"\\.\")
        || s.chars().count() <= 240
    {
        return p.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    PathBuf::from(format!(r"\\?\{s}"))
}

#[cfg(not(windows))]
fn verbatim_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{list_entries, read_entry};

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn write_file(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn run_zip(sources: &[PathBuf], dest: &Path) -> Vec<ZipWriteProgress> {
        let (tx, rx) = crossbeam_channel::unbounded();
        create_zip(sources, dest, &ZipWriteOptions::default(), tx, no_cancel()).unwrap();
        rx.try_iter().collect()
    }

    #[test]
    fn roundtrip_nested_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("comic");
        write_file(&root.join("a.txt"), b"hello a");
        write_file(&root.join("sub/b.png"), b"png-bytes");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        let dest = tmp.path().join("out.zip");

        let events = run_zip(&[root], &dest);

        let entries = list_entries(&dest, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "comic/",
                "comic/a.txt",
                "comic/empty/",
                "comic/sub/",
                "comic/sub/b.png"
            ]
        );
        assert!(entries[0].is_dir && entries[2].is_dir && entries[3].is_dir);
        assert_eq!(read_entry(&dest, "comic/a.txt", None).unwrap(), b"hello a");
        assert_eq!(
            read_entry(&dest, "comic/sub/b.png", None).unwrap(),
            b"png-bytes"
        );
        assert!(matches!(
            events.first(),
            Some(ZipWriteProgress::Started {
                total_files: 2,
                total_bytes: 16,
            })
        ));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ZipWriteProgress::EntryDone { .. }))
                .count(),
            2
        );
        assert!(matches!(
            events.last(),
            Some(ZipWriteProgress::Finished { written: 16 })
        ));
    }

    #[test]
    fn multi_source_uses_each_basename_as_root() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("single.txt");
        write_file(&file, b"one");
        let dir = tmp.path().join("dir");
        write_file(&dir.join("inner.txt"), b"two");
        let dest = tmp.path().join("out.zip");

        run_zip(&[file, dir], &dest);

        let entries = list_entries(&dest, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dir/", "dir/inner.txt", "single.txt"]);
        assert_eq!(read_entry(&dest, "dir/inner.txt", None).unwrap(), b"two");
        assert_eq!(read_entry(&dest, "single.txt", None).unwrap(), b"one");
    }

    #[test]
    fn cancel_cleans_partial_zip() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("a.txt"), b"data");
        let dest = tmp.path().join("out.zip");

        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(true));
        // 取消约定：返回 Ok，经 Failed("已取消") 感知，半成品删除。
        create_zip(
            &[tmp.path().join("a.txt")],
            &dest,
            &ZipWriteOptions::default(),
            tx,
            cancel,
        )
        .unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(
            events.last(),
            Some(ZipWriteProgress::Failed(msg)) if msg == "已取消"
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn deflated_compresses_repetitive_content() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.txt");
        write_file(&big, &b"aaaaaaaaaaaaaaaa".repeat(4096));
        let dest = tmp.path().join("out.zip");

        run_zip(std::slice::from_ref(&big), &dest);

        let entries = list_entries(&dest, None).unwrap();
        let e = &entries[0];
        assert_eq!(e.size, 65536);
        assert!(e.compressed_size.unwrap() < e.size);
        // compress=false → Stored 不压缩
        let dest2 = tmp.path().join("stored.zip");
        let (tx, _rx) = crossbeam_channel::unbounded();
        create_zip(
            &[big],
            &dest2,
            &ZipWriteOptions { compress: false },
            tx,
            no_cancel(),
        )
        .unwrap();
        let e2 = &list_entries(&dest2, None).unwrap()[0];
        assert_eq!(e2.compressed_size.unwrap(), e2.size);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        write_file(&root.join("real/x.txt"), b"1234");
        std::os::unix::fs::symlink(root.join("real"), root.join("linkdir")).unwrap();
        std::os::unix::fs::symlink(root.join("real/x.txt"), root.join("linkfile")).unwrap();
        let dest = tmp.path().join("out.zip");

        run_zip(&[root], &dest);

        let entries = list_entries(&dest, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["root/", "root/real/", "root/real/x.txt"]);
    }
}
