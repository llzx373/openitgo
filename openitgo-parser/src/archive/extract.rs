//! 通用解压引擎：ZIP 条目级并行，RAR/7z/TAR 单线程流式。
//!
//! 取消约定：cancel 置位时不作为错误返回——调用方经进度通道的
//! `Failed("已取消")` 事件感知取消，`extract_archive` 返回 `Ok(())`。
//! 已完整写出的文件保留，半成品文件删除。

use super::{archive_kind, classify_sevenz_error, sevenz_password, tar_reader, ArchiveKind};
use crate::traits::ParseError;
use crossbeam_channel::Sender;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub password: Option<String>,
    /// 解压线程数（仅 ZIP 生效），0 = 自动（available_parallelism，上限 8）。
    pub threads: usize,
    /// false = 重名自动改名 "name (1).ext"。
    pub overwrite: bool,
}

#[derive(Debug)]
pub enum ExtractProgress {
    Started {
        total_entries: usize,
        total_bytes: u64,
    },
    EntryDone {
        name: String,
        bytes: u64,
    },
    Finished {
        output_dir: PathBuf,
        written: u64,
    },
    Failed(String),
}

/// 解压压缩包到 `output_dir`；`selection` 为 None 时解压全部条目，
/// 否则只解压名字与 [`crate::archive::list_entries`] 精确匹配的条目。
pub fn extract_archive(
    path: &Path,
    output_dir: &Path,
    selection: Option<&[String]>,
    opts: &ExtractOptions,
    progress: Sender<ExtractProgress>,
    cancel: Arc<AtomicBool>,
) -> Result<(), ParseError> {
    std::fs::create_dir_all(output_dir)?;
    match archive_kind(path) {
        Some(ArchiveKind::Zip) => {
            extract_zip(path, output_dir, selection, opts, &progress, &cancel)
        }
        Some(ArchiveKind::Rar) => {
            extract_rar(path, output_dir, selection, opts, &progress, &cancel)
        }
        Some(ArchiveKind::SevenZ) => {
            extract_sevenz(path, output_dir, selection, opts, &progress, &cancel)
        }
        Some(ArchiveKind::Tar) => {
            extract_tar(path, output_dir, selection, opts, &progress, &cancel)
        }
        None => Err(ParseError::Unsupported),
    }
}

fn send_progress(progress: &Sender<ExtractProgress>, event: ExtractProgress) {
    // 接收方可能已关闭（如 UI 已退出），发送失败不视为解压失败。
    let _ = progress.send(event);
}

fn send_failed(progress: &Sender<ExtractProgress>, err: &ParseError) {
    send_progress(progress, ExtractProgress::Failed(err.to_string()));
}

/// 已取消：发 Failed("已取消") 并返回 Ok（见模块头注释的取消约定）。
fn cancelled(progress: &Sender<ExtractProgress>) -> Result<(), ParseError> {
    send_progress(progress, ExtractProgress::Failed("已取消".to_string()));
    Ok(())
}

/// 把归档条目名转成 output_dir 内的安全目标路径。
/// 含 `..`、绝对路径、Windows 盘符的条目返回 None（调用方跳过不写盘）。
/// 统一按 '/' 和 '\\' 两种分隔符切分，Windows/Unix 行为一致。
pub(crate) fn safe_target_path(output_dir: &Path, name: &str) -> Option<PathBuf> {
    let mut rel = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => {
                // 拒绝 Windows 盘符（"C:"）伪装成的组件
                let bytes = part.as_bytes();
                if bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
                    return None;
                }
                rel.push(part);
            }
        }
    }
    if rel.as_os_str().is_empty() {
        return None;
    }
    let target = output_dir.join(&rel);
    // 防御性复核：最终路径不得越出 output_dir
    if target.starts_with(output_dir) {
        Some(target)
    } else {
        None
    }
}

/// overwrite=false 时若目标已存在，自动改名为 "name (1).ext"。
fn uniquify(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for i in 1..1000u32 {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

fn resolve_threads(threads: usize) -> usize {
    let auto = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    match threads {
        0 => auto.min(8),
        n => n.clamp(1, 8),
    }
}

fn selection_set(selection: Option<&[String]>) -> Option<HashSet<&str>> {
    selection.map(|s| s.iter().map(String::as_str).collect())
}

/// 汇总 selection 过滤后的条目数/字节数并发 Started 事件。
fn send_started<'a>(
    progress: &Sender<ExtractProgress>,
    entries: impl Iterator<Item = (&'a str, bool, u64)>,
    selected: &Option<HashSet<&str>>,
) {
    let mut total_entries = 0usize;
    let mut total_bytes = 0u64;
    for (name, _, size) in entries {
        if let Some(sel) = selected {
            if !sel.contains(name) {
                continue;
            }
        }
        total_entries += 1;
        total_bytes += size;
    }
    send_progress(
        progress,
        ExtractProgress::Started {
            total_entries,
            total_bytes,
        },
    );
}

// ---------------------------------------------------------------------------
// ZIP：条目级并行
// ---------------------------------------------------------------------------

struct ZipTask {
    index: usize,
    name: String,
    is_dir: bool,
    encrypted: bool,
}

fn extract_zip(
    path: &Path,
    output_dir: &Path,
    selection: Option<&[String]>,
    opts: &ExtractOptions,
    progress: &Sender<ExtractProgress>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ParseError> {
    let selected = selection_set(selection);

    // 主线程预扫描：收集任务清单，并在派活前验证密码（尽早报错，
    // 不要解压一半才失败）。
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
    let mut tasks: Vec<ZipTask> = Vec::with_capacity(archive.len());
    let mut first_encrypted_index: Option<usize> = None;
    let mut total_bytes = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| ParseError::InvalidArchive(e.to_string()))?;
        let name = entry.name().to_string();
        if let Some(sel) = &selected {
            if !sel.contains(name.as_str()) {
                continue;
            }
        }
        if entry.encrypted() && first_encrypted_index.is_none() {
            first_encrypted_index = Some(i);
        }
        total_bytes += entry.size();
        tasks.push(ZipTask {
            index: i,
            name,
            is_dir: entry.is_dir(),
            encrypted: entry.encrypted(),
        });
    }

    if let Some(idx) = first_encrypted_index {
        let pw = opts
            .password
            .as_deref()
            .ok_or(ParseError::PasswordRequired)?;
        match archive.by_index_decrypt(idx, pw.as_bytes()) {
            Ok(_) => {}
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ParseError::PasswordIncorrect);
            }
            Err(e) => return Err(ParseError::InvalidArchive(e.to_string())),
        }
    }

    let total_entries = tasks.len();
    send_progress(
        progress,
        ExtractProgress::Started {
            total_entries,
            total_bytes,
        },
    );

    let threads = resolve_threads(opts.threads).min(tasks.len().max(1));
    let (task_tx, task_rx) = crossbeam_channel::unbounded::<ZipTask>();
    for task in tasks {
        let _ = task_tx.send(task);
    }
    drop(task_tx);

    // worker 共享的第一个错误；出错即置 cancel 让其余 worker 尽快收工。
    let first_error: Arc<Mutex<Option<ParseError>>> = Arc::new(Mutex::new(None));
    let written_total = Arc::new(AtomicU64::new(0));
    let password = opts.password.clone();

    std::thread::scope(|scope| {
        for _ in 0..threads {
            let rx = task_rx.clone();
            let progress = progress.clone();
            let cancel = cancel.clone();
            let first_error = first_error.clone();
            let written_total = written_total.clone();
            let password = password.clone();
            scope.spawn(move || {
                // 每个 worker 开自己的文件句柄 + ZipArchive，互不共享。
                let archive = std::fs::File::open(path)
                    .map_err(ParseError::Io)
                    .and_then(|f| {
                        zip::ZipArchive::new(f)
                            .map_err(|e| ParseError::InvalidArchive(e.to_string()))
                    });
                let mut archive = match archive {
                    Ok(a) => a,
                    Err(e) => {
                        *first_error.lock().unwrap() = Some(e);
                        cancel.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                while let Ok(task) = rx.recv() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    match extract_zip_task(
                        &mut archive,
                        &task,
                        output_dir,
                        password.as_deref(),
                        opts.overwrite,
                    ) {
                        Ok(written) => {
                            written_total.fetch_add(written, Ordering::Relaxed);
                            send_progress(
                                &progress,
                                ExtractProgress::EntryDone {
                                    name: task.name.clone(),
                                    bytes: written,
                                },
                            );
                        }
                        Err(e) => {
                            *first_error.lock().unwrap() = Some(e);
                            cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            });
        }
    });

    if let Some(e) = first_error.lock().unwrap().take() {
        send_failed(progress, &e);
        return Err(e);
    }
    if cancel.load(Ordering::Relaxed) {
        return cancelled(progress);
    }
    send_progress(
        progress,
        ExtractProgress::Finished {
            output_dir: output_dir.to_path_buf(),
            written: written_total.load(Ordering::Relaxed),
        },
    );
    Ok(())
}

fn extract_zip_task(
    archive: &mut zip::ZipArchive<std::fs::File>,
    task: &ZipTask,
    output_dir: &Path,
    password: Option<&str>,
    overwrite: bool,
) -> Result<u64, ParseError> {
    // 穿越条目：拒绝写盘，静默跳过（仍计 EntryDone，bytes=0 由调用方发）。
    let Some(target) = safe_target_path(output_dir, &task.name) else {
        return Ok(0);
    };
    if task.is_dir {
        std::fs::create_dir_all(&target)?;
        return Ok(0);
    }
    let target = if overwrite { target } else { uniquify(target) };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut src = if task.encrypted {
        let pw = password.ok_or(ParseError::PasswordRequired)?;
        match archive.by_index_decrypt(task.index, pw.as_bytes()) {
            Ok(f) => f,
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ParseError::PasswordIncorrect);
            }
            Err(e) => return Err(ParseError::InvalidArchive(e.to_string())),
        }
    } else {
        archive
            .by_index(task.index)
            .map_err(|e| ParseError::InvalidArchive(e.to_string()))?
    };
    // 流式写盘，不把整条目读进内存；写失败时删除半成品文件。
    let mut dst = std::fs::File::create(&target)?;
    match std::io::copy(&mut src, &mut dst) {
        Ok(n) => Ok(n),
        Err(e) => {
            drop(dst);
            let _ = std::fs::remove_file(&target);
            Err(ParseError::Io(e))
        }
    }
}

// ---------------------------------------------------------------------------
// RAR：单线程顺序流
// ---------------------------------------------------------------------------

fn extract_rar(
    path: &Path,
    output_dir: &Path,
    selection: Option<&[String]>,
    opts: &ExtractOptions,
    progress: &Sender<ExtractProgress>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ParseError> {
    let selected = selection_set(selection);
    let password = opts.password.as_deref();

    // 先列表拿总量（数据加密包列表可读，密码在读条目时暴露）。
    let entries = match super::list_entries(path, password) {
        Ok(e) => e,
        Err(e) => {
            send_failed(progress, &e);
            return Err(e);
        }
    };
    send_started(
        progress,
        entries.iter().map(|e| (e.name.as_str(), e.is_dir, e.size)),
        &selected,
    );

    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    let mut archive = match builder.open_for_processing() {
        Ok(a) => a,
        Err(e) => {
            let err = crate::rar::classify_rar_error(e);
            send_failed(progress, &err);
            return Err(err);
        }
    };

    let mut written_total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return cancelled(progress);
        }
        let maybe_entry = match archive.read_header() {
            Ok(e) => e,
            Err(e) => {
                let err = crate::rar::classify_rar_error(e);
                send_failed(progress, &err);
                return Err(err);
            }
        };
        let Some(entry) = maybe_entry else { break };
        let header = entry.entry();
        let name = header.filename.to_string_lossy().to_string();
        let wanted = selected
            .as_ref()
            .is_none_or(|sel| sel.contains(name.as_str()));
        let target = if wanted {
            safe_target_path(output_dir, &name)
        } else {
            None
        };
        let is_file = header.is_file();

        // 写盘文件条目：选中 + 是文件 + 路径安全，三者齐备才 read
        let file_target = if wanted && is_file {
            target.clone()
        } else {
            None
        };
        if let Some(target) = file_target {
            match entry.read() {
                Ok((data, next)) => {
                    archive = next;
                    let target = if opts.overwrite {
                        target
                    } else {
                        uniquify(target)
                    };
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = data.len() as u64;
                    if let Err(e) = std::fs::write(&target, &data) {
                        let _ = std::fs::remove_file(&target);
                        let err = ParseError::Io(e);
                        send_failed(progress, &err);
                        return Err(err);
                    }
                    written_total += bytes;
                    send_progress(progress, ExtractProgress::EntryDone { name, bytes });
                }
                Err(e) => {
                    // 带密码时 CRC BadData → 密码错误（与 parse_rar 规则一致）；
                    // 第一条命中条目即暴露密码问题，尽早失败。
                    let err = if password.is_some() && e.code == unrar::error::Code::BadData {
                        ParseError::PasswordIncorrect
                    } else {
                        crate::rar::classify_rar_error(e)
                    };
                    send_failed(progress, &err);
                    return Err(err);
                }
            }
        } else {
            match entry.skip() {
                Ok(next) => archive = next,
                Err(e) => {
                    let err = crate::rar::classify_rar_error(e);
                    send_failed(progress, &err);
                    return Err(err);
                }
            }
            // 目录条目：skip 之后补建目录（穿越条目除外）。
            if wanted && !is_file {
                if let Some(target) = target {
                    std::fs::create_dir_all(&target)?;
                    send_progress(progress, ExtractProgress::EntryDone { name, bytes: 0 });
                }
            }
        }
    }

    send_progress(
        progress,
        ExtractProgress::Finished {
            output_dir: output_dir.to_path_buf(),
            written: written_total,
        },
    );
    Ok(())
}

/// 7z 内容 CRC 校验失败（错密码解出乱码过不了 CRC）会以
/// io::Error 包着 ChecksumVerificationFailed 的形式从读流里冒出来；
/// 带密码时归一为 PasswordIncorrect（与 RAR BadData 规则同理）。
pub(crate) fn classify_sevenz_io_error(e: std::io::Error, had_password: bool) -> ParseError {
    if had_password {
        if let Some(inner) = e.get_ref() {
            if let Some(se) = inner.downcast_ref::<sevenz_rust2::Error>() {
                if matches!(
                    se,
                    sevenz_rust2::Error::ChecksumVerificationFailed
                        | sevenz_rust2::Error::MaybeBadPassword(_)
                ) {
                    return ParseError::PasswordIncorrect;
                }
            }
        }
    }
    ParseError::Io(e)
}

// ---------------------------------------------------------------------------
// 7z：单线程（LZMA2 块内部可多线程，由 sevenz-rust2 自管）
// ---------------------------------------------------------------------------

fn extract_sevenz(
    path: &Path,
    output_dir: &Path,
    selection: Option<&[String]>,
    opts: &ExtractOptions,
    progress: &Sender<ExtractProgress>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ParseError> {
    let had_password = opts.password.is_some();
    let selected: Option<HashSet<&str>> = selection_set(selection);

    let entries = match super::list_entries(path, opts.password.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            send_failed(progress, &e);
            return Err(e);
        }
    };
    send_started(
        progress,
        entries.iter().map(|e| (e.name.as_str(), e.is_dir, e.size)),
        &selected,
    );

    let mut reader =
        match sevenz_rust2::ArchiveReader::open(path, sevenz_password(opts.password.as_deref())) {
            Ok(r) => r,
            Err(e) => {
                let err = classify_sevenz_error(e, had_password);
                send_failed(progress, &err);
                return Err(err);
            }
        };

    let mut written_total = 0u64;
    // 闭包内无法返回 ParseError：先暂存到 our_error，再以 Err 中止迭代，
    // 迭代返回后优先还原 our_error。
    let mut our_error: Option<ParseError> = None;
    let cancel_clone = cancel.clone();
    let iter_result = reader.for_each_entries(|entry, src| {
        if cancel_clone.load(Ordering::Relaxed) {
            return Ok(false); // 停止迭代
        }
        let name = entry.name.clone();
        if let Some(sel) = &selected {
            if !sel.contains(name.as_str()) {
                return Ok(true);
            }
        }
        let Some(target) = safe_target_path(output_dir, &name) else {
            return Ok(true); // 穿越条目跳过
        };
        if entry.is_directory {
            if let Err(e) = std::fs::create_dir_all(&target) {
                our_error = Some(ParseError::Io(e));
                return Err(sevenz_rust2::Error::Other("create dir failed".into()));
            }
            send_progress(progress, ExtractProgress::EntryDone { name, bytes: 0 });
            return Ok(true);
        }
        let target = if opts.overwrite {
            target
        } else {
            uniquify(target)
        };
        let write_result = (|| -> std::io::Result<u64> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut dst = std::fs::File::create(&target)?;
            std::io::copy(src, &mut dst)
        })();
        match write_result {
            Ok(n) => {
                written_total += n;
                send_progress(progress, ExtractProgress::EntryDone { name, bytes: n });
                Ok(true)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&target); // 删除半成品
                our_error = Some(classify_sevenz_io_error(e, had_password));
                Err(sevenz_rust2::Error::Other("write entry failed".into()))
            }
        }
    });

    if cancel.load(Ordering::Relaxed) {
        return cancelled(progress);
    }
    if let Some(err) = our_error {
        send_failed(progress, &err);
        return Err(err);
    }
    // 解码侧错误（含密码错误：内容加密包第一条加密条目解码即暴露）
    if let Err(e) = iter_result {
        let err = classify_sevenz_error(e, had_password);
        send_failed(progress, &err);
        return Err(err);
    }

    send_progress(
        progress,
        ExtractProgress::Finished {
            output_dir: output_dir.to_path_buf(),
            written: written_total,
        },
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// TAR：单遍流式
// ---------------------------------------------------------------------------

fn extract_tar(
    path: &Path,
    output_dir: &Path,
    selection: Option<&[String]>,
    opts: &ExtractOptions,
    progress: &Sender<ExtractProgress>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ParseError> {
    let selected = selection_set(selection);

    // 先单遍列表拿总量（TAR 是流式格式，需要解压两遍；包大时略贵但实现简单）。
    let entries = match super::list_entries(path, None) {
        Ok(e) => e,
        Err(e) => {
            send_failed(progress, &e);
            return Err(e);
        }
    };
    send_started(
        progress,
        entries.iter().map(|e| (e.name.as_str(), e.is_dir, e.size)),
        &selected,
    );

    let reader = tar_reader(path)?;
    let mut archive = tar::Archive::new(reader);
    let mut written_total = 0u64;

    let entries_iter = match archive.entries() {
        Ok(it) => it,
        Err(e) => {
            let err = ParseError::Io(e);
            send_failed(progress, &err);
            return Err(err);
        }
    };
    for item in entries_iter {
        if cancel.load(Ordering::Relaxed) {
            return cancelled(progress);
        }
        let mut entry = match item {
            Ok(e) => e,
            Err(e) => {
                let err = ParseError::Io(e);
                send_failed(progress, &err);
                return Err(err);
            }
        };
        let name = String::from_utf8_lossy(entry.path_bytes().as_ref()).to_string();
        if let Some(sel) = &selected {
            if !sel.contains(name.as_str()) {
                continue;
            }
        }
        let Some(target) = safe_target_path(output_dir, &name) else {
            continue; // 穿越条目跳过
        };
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)?;
            send_progress(progress, ExtractProgress::EntryDone { name, bytes: 0 });
            continue;
        }
        let target = if opts.overwrite {
            target
        } else {
            uniquify(target)
        };
        let write_result = (|| -> std::io::Result<u64> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut dst = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut dst)
        })();
        match write_result {
            Ok(n) => {
                written_total += n;
                send_progress(progress, ExtractProgress::EntryDone { name, bytes: n });
            }
            Err(e) => {
                let _ = std::fs::remove_file(&target); // 删除半成品
                let err = ParseError::Io(e);
                send_failed(progress, &err);
                return Err(err);
            }
        }
    }

    send_progress(
        progress,
        ExtractProgress::Finished {
            output_dir: output_dir.to_path_buf(),
            written: written_total,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::tests::{
        write_encrypted_7z, write_encrypted_zip, write_test_7z, write_test_tar_gz, write_test_zip,
    };
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn default_opts() -> ExtractOptions {
        ExtractOptions {
            password: None,
            threads: 0,
            overwrite: false,
        }
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// 收集一次解压的全部进度事件。
    fn collect_events(rx: crossbeam_channel::Receiver<ExtractProgress>) -> Vec<ExtractProgress> {
        rx.try_iter().collect()
    }

    #[test]
    fn extract_zip_all_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        write_test_zip(&zip_path);
        let out = tmp.path().join("out");

        let (tx, rx) = crossbeam_channel::unbounded();
        extract_archive(&zip_path, &out, None, &default_opts(), tx, no_cancel()).unwrap();

        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello a");
        assert_eq!(std::fs::read(out.join("sub/b.png")).unwrap(), b"png-bytes");
        assert_eq!(std::fs::read(out.join("notes.md")).unwrap(), b"# notes");
        assert!(out.join("sub").is_dir());

        let events = collect_events(rx);
        assert!(matches!(
            events.first(),
            Some(ExtractProgress::Started {
                total_entries: 4,
                total_bytes: 23,
            })
        ));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ExtractProgress::EntryDone { .. }))
                .count(),
            4
        );
        assert!(matches!(
            events.last(),
            Some(ExtractProgress::Finished { written: 23, .. })
        ));
    }

    #[test]
    fn extract_zip_selection_only() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        write_test_zip(&zip_path);
        let out = tmp.path().join("out");

        let (tx, _rx) = crossbeam_channel::unbounded();
        let selection = vec!["a.txt".to_string()];
        extract_archive(
            &zip_path,
            &out,
            Some(&selection),
            &default_opts(),
            tx,
            no_cancel(),
        )
        .unwrap();

        assert!(out.join("a.txt").exists());
        assert!(!out.join("notes.md").exists());
        assert!(!out.join("sub/b.png").exists());
    }

    #[test]
    fn extract_zip_rename_on_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        write_test_zip(&zip_path);
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("a.txt"), b"existing").unwrap();

        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&zip_path, &out, None, &default_opts(), tx, no_cancel()).unwrap();

        // 原文件不动，新条目改名为 "a (1).txt"
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"existing");
        assert_eq!(std::fs::read(out.join("a (1).txt")).unwrap(), b"hello a");
    }

    #[test]
    fn extract_zip_overwrite_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        write_test_zip(&zip_path);
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("a.txt"), b"existing").unwrap();

        let opts = ExtractOptions {
            overwrite: true,
            ..default_opts()
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&zip_path, &out, None, &opts, tx, no_cancel()).unwrap();
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello a");
    }

    /// 写含路径穿越条目的 zip。
    fn write_traversal_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("../evil.txt", options).unwrap();
        zip.write_all(b"evil").unwrap();
        zip.start_file("sub/../../evil2.txt", options).unwrap();
        zip.write_all(b"evil2").unwrap();
        zip.start_file("C:/abs/evil3.txt", options).unwrap();
        zip.write_all(b"evil3").unwrap();
        zip.start_file("good.txt", options).unwrap();
        zip.write_all(b"good").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn extract_zip_rejects_traversal_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        write_traversal_zip(&zip_path);
        let out = tmp.path().join("out");

        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&zip_path, &out, None, &default_opts(), tx, no_cancel()).unwrap();

        assert!(out.join("good.txt").exists());
        assert!(!tmp.path().join("evil.txt").exists());
        assert!(!tmp.path().join("evil2.txt").exists());
        assert!(!out.join("sub/../../evil2.txt").exists());
    }

    #[test]
    fn safe_target_path_unit() {
        let base = Path::new("/out");
        assert_eq!(
            safe_target_path(base, "a/b.txt"),
            Some(base.join("a/b.txt"))
        );
        assert_eq!(safe_target_path(base, "./a.txt"), Some(base.join("a.txt")));
        assert_eq!(safe_target_path(base, "../evil"), None);
        assert_eq!(safe_target_path(base, "a/../../evil"), None);
        assert_eq!(
            safe_target_path(base, "/abs/path"),
            Some(base.join("abs/path"))
        );
        assert_eq!(safe_target_path(base, "C:\\windows\\x"), None);
        assert_eq!(safe_target_path(base, "c:/x"), None);
        assert_eq!(safe_target_path(base, ""), None);
    }

    #[test]
    fn extract_encrypted_zip_password_states() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("enc.zip");
        write_encrypted_zip(&zip_path, "s3cret");

        // 无密码 → PasswordRequired
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &zip_path,
                &tmp.path().join("o1"),
                None,
                &default_opts(),
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordRequired)
        ));

        // 错密码 → PasswordIncorrect
        let opts = ExtractOptions {
            password: Some("nope".to_string()),
            ..default_opts()
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &zip_path,
                &tmp.path().join("o2"),
                None,
                &opts,
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordIncorrect)
        ));

        // 对密码 → 正常解压
        let opts = ExtractOptions {
            password: Some("s3cret".to_string()),
            ..default_opts()
        };
        let out = tmp.path().join("o3");
        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&zip_path, &out, None, &opts, tx, no_cancel()).unwrap();
        assert_eq!(
            std::fs::read(out.join("secret.txt")).unwrap(),
            b"top secret"
        );
    }

    #[test]
    fn extract_zip_cancelled_before_start() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        write_test_zip(&zip_path);
        let out = tmp.path().join("out");

        let cancel = Arc::new(AtomicBool::new(true));
        let (tx, rx) = crossbeam_channel::unbounded();
        // 取消不是错误：返回 Ok，经 Failed("已取消") 事件通知
        extract_archive(&zip_path, &out, None, &default_opts(), tx, cancel).unwrap();
        assert!(!out.join("a.txt").exists());
        let events = collect_events(rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, ExtractProgress::Failed(msg) if msg == "已取消")));
    }

    #[test]
    fn extract_rar_header_encrypted_password_states() {
        let rar =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted-header-pw123.rar");
        let tmp = tempfile::tempdir().unwrap();

        // 无密码 → PasswordRequired（列表阶段即失败）
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &rar,
                &tmp.path().join("o1"),
                None,
                &default_opts(),
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordRequired)
        ));

        // 错密码 → PasswordIncorrect
        let opts = ExtractOptions {
            password: Some("nope".to_string()),
            ..default_opts()
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(&rar, &tmp.path().join("o2"), None, &opts, tx, no_cancel()),
            Err(ParseError::PasswordIncorrect)
        ));

        // 对密码 → 正常解压
        let opts = ExtractOptions {
            password: Some("pw123".to_string()),
            ..default_opts()
        };
        let out = tmp.path().join("o3");
        let (tx, rx) = crossbeam_channel::unbounded();
        extract_archive(&rar, &out, None, &opts, tx, no_cancel()).unwrap();
        let events = collect_events(rx);
        assert!(events.iter().any(|e| matches!(
            e,
            ExtractProgress::Finished { written, .. } if *written > 0
        )));
    }

    #[test]
    fn extract_rar_data_encrypted_password_states() {
        let rar =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted-files-pw123.rar");
        let tmp = tempfile::tempdir().unwrap();

        // 无密码：列表成功，读第一条目即暴露 MissingPassword
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &rar,
                &tmp.path().join("o1"),
                None,
                &default_opts(),
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordRequired)
        ));

        // 错密码：CRC BadData → PasswordIncorrect
        let opts = ExtractOptions {
            password: Some("nope".to_string()),
            ..default_opts()
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(&rar, &tmp.path().join("o2"), None, &opts, tx, no_cancel()),
            Err(ParseError::PasswordIncorrect)
        ));

        // 对密码 → 正常解压
        let opts = ExtractOptions {
            password: Some("pw123".to_string()),
            ..default_opts()
        };
        let out = tmp.path().join("o3");
        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&rar, &out, None, &opts, tx, no_cancel()).unwrap();
        assert!(std::fs::read_dir(&out).unwrap().next().is_some());
    }

    #[test]
    fn extract_7z_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let sz_path = tmp.path().join("test.7z");
        write_test_7z(&sz_path);
        let out = tmp.path().join("out");

        let (tx, rx) = crossbeam_channel::unbounded();
        extract_archive(&sz_path, &out, None, &default_opts(), tx, no_cancel()).unwrap();
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello a");
        assert_eq!(std::fs::read(out.join("sub/b.png")).unwrap(), b"png-bytes");
        assert!(out.join("sub").is_dir());
        let events = collect_events(rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, ExtractProgress::Finished { .. })));
    }

    #[test]
    fn extract_encrypted_7z_password_states() {
        let tmp = tempfile::tempdir().unwrap();
        let sz_path = tmp.path().join("enc.7z");
        write_encrypted_7z(&sz_path, "pw123");

        // 无密码：列表成功（头部未加密），解码时暴露 PasswordRequired
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &sz_path,
                &tmp.path().join("o1"),
                None,
                &default_opts(),
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordRequired)
        ));

        // 错密码 → PasswordIncorrect
        let opts = ExtractOptions {
            password: Some("nope".to_string()),
            ..default_opts()
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                &sz_path,
                &tmp.path().join("o2"),
                None,
                &opts,
                tx,
                no_cancel()
            ),
            Err(ParseError::PasswordIncorrect)
        ));

        // 对密码 → 正常解压
        let opts = ExtractOptions {
            password: Some("pw123".to_string()),
            ..default_opts()
        };
        let out = tmp.path().join("o3");
        let (tx, _rx) = crossbeam_channel::unbounded();
        extract_archive(&sz_path, &out, None, &opts, tx, no_cancel()).unwrap();
        assert_eq!(
            std::fs::read(out.join("secret.txt")).unwrap(),
            b"top secret"
        );
    }

    #[test]
    fn extract_tar_gz_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("test.tar.gz");
        write_test_tar_gz(&tar_path);
        let out = tmp.path().join("out");

        let (tx, rx) = crossbeam_channel::unbounded();
        extract_archive(&tar_path, &out, None, &default_opts(), tx, no_cancel()).unwrap();
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello a");
        assert_eq!(std::fs::read(out.join("sub/b.png")).unwrap(), b"png-bytes");
        assert!(out.join("sub").is_dir());
        let events = collect_events(rx);
        assert!(matches!(
            events.last(),
            Some(ExtractProgress::Finished { written: 16, .. })
        ));
    }

    #[test]
    fn extract_tar_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("test.tar.gz");
        write_test_tar_gz(&tar_path);
        let out = tmp.path().join("out");

        let (tx, _rx) = crossbeam_channel::unbounded();
        let selection = vec!["sub/b.png".to_string()];
        extract_archive(
            &tar_path,
            &out,
            Some(&selection),
            &default_opts(),
            tx,
            no_cancel(),
        )
        .unwrap();
        assert!(!out.join("a.txt").exists());
        assert_eq!(std::fs::read(out.join("sub/b.png")).unwrap(), b"png-bytes");
    }

    #[test]
    fn extract_unsupported_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(
            extract_archive(
                Path::new("a.pdf"),
                tmp.path(),
                None,
                &default_opts(),
                tx,
                no_cancel()
            ),
            Err(ParseError::Unsupported)
        ));
    }
}
