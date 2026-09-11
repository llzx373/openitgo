//! 校验和对话框（阶段 AC，TC「文件 → 校验和」简化版）：右键「校验和…」
//! 打开非模态 egui::Window。两种模式——**计算**：后台 worker 逐文件分块读
//! （256KB）单次过同时算 CRC32/SHA-1/SHA-256（crc32fast/sha1/sha2 均已在
//! 依赖树，显式声明复用；MD5 无依赖不引入，校验文件中 md5 条目标「不支持」），
//! 进度「N/M 个文件」+ 可取消，结果列表点击复制 hex；**验证**：
//! `parse_checksum_file` 解析 md5sum 格式（`<hash> [*]<文件名>`，按 hash
//! 长度分 sha1/sha256/md5）与 sfv 格式（`<文件名> <crc32>`，`;` 注释），
//! 逐项重算比对标 ✓/✗/缺失/不支持。选中集恰为单个校验文件时菜单直接进
//! 验证模式。解析与增量 hash 为纯函数/纯逻辑，单测覆盖；worker 模式同
//! `file_manager_search.rs`（Drop 即置取消旗标）。

use crate::views::file_ops::verbatim_path;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// hash 分块读取大小。
const CHUNK: usize = 256 * 1024;
/// 结果/验证列表行高（pt）。
const ROW_H: f32 = 20.0;

/// 支持的算法（MD5 缺席：依赖树无 md-5，不新引入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Crc32,
    Sha1,
    Sha256,
}

impl ChecksumAlgorithm {
    pub const ALL: [ChecksumAlgorithm; 3] = [
        ChecksumAlgorithm::Crc32,
        ChecksumAlgorithm::Sha1,
        ChecksumAlgorithm::Sha256,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ChecksumAlgorithm::Crc32 => "CRC32",
            ChecksumAlgorithm::Sha1 => "SHA-1",
            ChecksumAlgorithm::Sha256 => "SHA-256",
        }
    }
}

/// 单文件三算法结果（hex 小写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHashes {
    pub crc32: String,
    pub sha1: String,
    pub sha256: String,
}

impl FileHashes {
    pub fn get(&self, algo: ChecksumAlgorithm) -> &str {
        match algo {
            ChecksumAlgorithm::Crc32 => &self.crc32,
            ChecksumAlgorithm::Sha1 => &self.sha1,
            ChecksumAlgorithm::Sha256 => &self.sha256,
        }
    }
}

/// 增量三算法累加器（单次 IO 过同时喂三个 hasher）。
pub struct HashAccumulator {
    crc: crc32fast::Hasher,
    sha1: sha1::Sha1,
    sha256: sha2::Sha256,
}

impl Default for HashAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl HashAccumulator {
    pub fn new() -> Self {
        use sha1::Digest;
        Self {
            crc: crc32fast::Hasher::new(),
            sha1: sha1::Sha1::new(),
            sha256: sha2::Sha256::new(),
        }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        use sha1::Digest;
        self.crc.update(bytes);
        self.sha1.update(bytes);
        self.sha256.update(bytes);
    }

    pub fn finalize(self) -> FileHashes {
        use sha1::Digest;
        FileHashes {
            crc32: format!("{:08x}", self.crc.finalize()),
            sha1: format!("{:x}", self.sha1.finalize()),
            sha256: format!("{:x}", self.sha256.finalize()),
        }
    }
}

/// 校验文件中的一条记录。`algorithm: None` = md5（无算法可用，验证标
/// 「不支持」）；`hash` 保留原文大小写（比对时忽略大小写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumFileEntry {
    pub algorithm: Option<ChecksumAlgorithm>,
    pub hash: String,
    pub filename: String,
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 解析校验文件（纯函数）。逐行自动识别两种格式（空行与 `;`/`#` 开头
/// 注释行跳过；文件名中的空格经「最后 token」判定保留）：
/// - md5sum 族：行首 hex token（32 = md5 / 40 = sha1 / 64 = sha256）加空白
///   加可选 `*` 二进制标记加文件名；
/// - sfv：最后一个空白分隔 token 为 8 位 hex（crc32），其余为文件名。
pub fn parse_checksum_file(text: &str) -> Vec<ChecksumFileEntry> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        // md5sum 族：行首 hex token。
        if let Some(pos) = line.find(char::is_whitespace) {
            let token = &line[..pos];
            let rest = line[pos..].trim_start();
            let filename = rest.strip_prefix('*').unwrap_or(rest);
            if is_hex(token) && !filename.is_empty() {
                let algorithm = match token.len() {
                    32 => Some(None),
                    40 => Some(Some(ChecksumAlgorithm::Sha1)),
                    64 => Some(Some(ChecksumAlgorithm::Sha256)),
                    _ => None,
                };
                if let Some(algorithm) = algorithm {
                    out.push(ChecksumFileEntry {
                        algorithm,
                        hash: token.to_string(),
                        filename: filename.to_string(),
                    });
                    continue;
                }
            }
        }
        // sfv：最后 token 为 8 位 hex。
        if let Some(pos) = line.rfind(char::is_whitespace) {
            let token = &line[pos + 1..];
            let filename = line[..pos].trim();
            if token.len() == 8 && is_hex(token) && !filename.is_empty() {
                out.push(ChecksumFileEntry {
                    algorithm: Some(ChecksumAlgorithm::Crc32),
                    hash: token.to_string(),
                    filename: filename.to_string(),
                });
            }
        }
    }
    out
}

/// 校验文件扩展名门槛（.md5/.sfv/.sha1/.sha256；右键菜单分流用）。
pub fn is_checksum_file_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            e.eq_ignore_ascii_case("md5")
                || e.eq_ignore_ascii_case("sfv")
                || e.eq_ignore_ascii_case("sha1")
                || e.eq_ignore_ascii_case("sha256")
        })
}

/// 分块计算单文件三算法（worker 核心；cancel 命中提前返回 Err("已取消")）。
fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<FileHashes, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(verbatim_path(path)).map_err(|e| format!("无法打开: {e}"))?;
    let mut acc = HashAccumulator::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".to_string());
        }
        let n = f.read(&mut buf).map_err(|e| format!("读取失败: {e}"))?;
        if n == 0 {
            break;
        }
        acc.update(&buf[..n]);
    }
    Ok(acc.finalize())
}

/// worker → UI 事件。
enum ChecksumEvent {
    FileDone {
        index: usize,
        result: Result<FileHashes, String>,
    },
    Finished,
}

/// drain 的一批结果（job index → 计算结果）。
type JobResults = Vec<(usize, Result<FileHashes, String>)>;

/// 后台校验和任务：逐文件计算，进度 = 已完成数（AtomicUsize）。
/// Drop 即置取消（worker 在文件/块边界检查退出，线程自行收尾）。
pub struct ChecksumTask {
    rx: crossbeam_channel::Receiver<ChecksumEvent>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: usize,
}

impl ChecksumTask {
    /// files = (显示名, 路径)；显示名仅 UI 用（worker 不回传）。
    pub fn start(files: Vec<(String, PathBuf)>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let total = files.len();
        let (cancel2, done2) = (cancel.clone(), done.clone());
        std::thread::Builder::new()
            .name("fm-checksum".to_string())
            .spawn(move || {
                for (index, (_, path)) in files.into_iter().enumerate() {
                    if cancel2.load(Ordering::Relaxed) {
                        return;
                    }
                    let result = hash_file(&path, &cancel2);
                    if result.is_ok() {
                        done2.fetch_add(1, Ordering::Relaxed);
                    }
                    if tx.send(ChecksumEvent::FileDone { index, result }).is_err() {
                        return;
                    }
                }
                let _ = tx.send(ChecksumEvent::Finished);
            })
            .ok();
        Self {
            rx,
            cancel,
            done,
            total,
        }
    }

    /// (已完成, 总数)。
    pub fn progress(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 排空事件；返回 (本批结果, 是否已结束)。
    fn drain(&mut self) -> (JobResults, bool) {
        let mut results = JobResults::new();
        let mut finished = false;
        for ev in self.rx.try_iter() {
            match ev {
                ChecksumEvent::FileDone { index, result } => results.push((index, result)),
                ChecksumEvent::Finished => finished = true,
            }
        }
        (results, finished)
    }
}

impl Drop for ChecksumTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 验证行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
enum VerifyStatus {
    Pending,
    Ok,
    Mismatch,
    Missing,
    Unsupported,
    Error(String),
}

struct VerifyRow {
    filename: String,
    expected: String,
    algorithm: Option<ChecksumAlgorithm>,
    status: VerifyStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogMode {
    Compute,
    Verify,
}

/// 校验和对话框状态（FileManagerView 持有；非模态 egui::Window）。
pub struct ChecksumDialog {
    open: bool,
    mode: DialogMode,
    // 计算模式：显示名与结果槽（index 对齐 task jobs）。
    files: Vec<String>,
    results: Vec<Option<Result<FileHashes, String>>>,
    algo_sel: ChecksumAlgorithm,
    // 验证模式：校验文件路径输入 + 行集 + job index → 行 index 映射。
    verify_path: String,
    verify_rows: Vec<VerifyRow>,
    job_rows: Vec<usize>,
    verify_error: Option<String>,
    task: Option<ChecksumTask>,
}

impl Default for ChecksumDialog {
    fn default() -> Self {
        Self {
            open: false,
            mode: DialogMode::Compute,
            files: Vec::new(),
            results: Vec::new(),
            algo_sel: ChecksumAlgorithm::Crc32,
            verify_path: String::new(),
            verify_rows: Vec::new(),
            job_rows: Vec::new(),
            verify_error: None,
            task: None,
        }
    }
}

impl ChecksumDialog {
    /// 计算模式打开（右键「校验和…」，文件目标集）。
    pub fn open_compute(&mut self, files: Vec<PathBuf>) {
        self.open = true;
        self.mode = DialogMode::Compute;
        self.files = files
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect();
        self.results = vec![None; files.len()];
        self.task = (!files.is_empty())
            .then(|| ChecksumTask::start(self.files.iter().cloned().zip(files).collect()));
    }

    /// 验证模式打开（选中集恰为校验文件，或对话框内输入路径）。
    pub fn open_verify(&mut self, path: PathBuf) {
        self.open = true;
        self.mode = DialogMode::Verify;
        self.verify_path = path.display().to_string();
        self.load_verify();
    }

    /// 关闭并取消在途任务（窗口 X 直接走 ui() 内的 task 清理）。
    #[allow(dead_code)]
    pub fn close(&mut self) {
        self.open = false;
        self.task = None;
    }

    /// 读取并解析校验文件，逐项派生初始状态并起后台重算。
    fn load_verify(&mut self) {
        self.task = None;
        self.verify_rows.clear();
        self.job_rows.clear();
        self.verify_error = None;
        let path = PathBuf::from(self.verify_path.trim());
        let bytes = match std::fs::read(verbatim_path(&path)) {
            Ok(b) => b,
            Err(e) => {
                self.verify_error = Some(format!("无法读取校验文件: {e}"));
                return;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let entries = parse_checksum_file(&text);
        if entries.is_empty() {
            self.verify_error = Some("未解析到任何校验记录（支持 md5sum 与 sfv 格式）".to_string());
            return;
        }
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut jobs: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries {
            let status = match entry.algorithm {
                None => VerifyStatus::Unsupported,
                Some(_) => {
                    let full = parent.join(&entry.filename);
                    if verbatim_path(&full).is_file() {
                        self.job_rows.push(self.verify_rows.len());
                        jobs.push((entry.filename.clone(), full));
                        VerifyStatus::Pending
                    } else {
                        VerifyStatus::Missing
                    }
                }
            };
            self.verify_rows.push(VerifyRow {
                filename: entry.filename,
                expected: entry.hash,
                algorithm: entry.algorithm,
                status,
            });
        }
        self.task = (!jobs.is_empty()).then(|| ChecksumTask::start(jobs));
    }

    /// 渲染（open 时）。任务在途时主动重绘排空结果。
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        // 排空 worker 事件（中间批次也要落地，Finished 只负责收尾）。
        if let Some(task) = &mut self.task {
            let (results, finished) = task.drain();
            self.apply_results(results);
            if finished {
                self.task = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut open = self.open;
        egui::Window::new("校验和")
            .collapsible(false)
            .resizable(true)
            .default_size([600.0, 420.0])
            .open(&mut open)
            .show(ctx, |ui| {
                self.ui_body(ui);
            });
        self.open = open;
        if !open {
            self.task = None;
        }
    }

    fn apply_results(&mut self, results: JobResults) {
        match self.mode {
            DialogMode::Compute => {
                for (index, result) in results {
                    if let Some(slot) = self.results.get_mut(index) {
                        *slot = Some(result);
                    }
                }
            }
            DialogMode::Verify => {
                for (index, result) in results {
                    let Some(&row) = self.job_rows.get(index) else {
                        continue;
                    };
                    let Some(vr) = self.verify_rows.get_mut(row) else {
                        continue;
                    };
                    vr.status = match result {
                        Err(e) => VerifyStatus::Error(e),
                        Ok(hashes) => {
                            let actual = vr
                                .algorithm
                                .map(|a| hashes.get(a).to_string())
                                .unwrap_or_default();
                            if actual.eq_ignore_ascii_case(&vr.expected) {
                                VerifyStatus::Ok
                            } else {
                                VerifyStatus::Mismatch
                            }
                        }
                    };
                }
            }
        }
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) {
        // 模式切换 + 状态行。
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, DialogMode::Compute, "计算");
            ui.selectable_value(&mut self.mode, DialogMode::Verify, "验证");
            ui.separator();
            if let Some(task) = &self.task {
                let (done, total) = task.progress();
                ui.label(format!("{done}/{total} 个文件"));
                if ui.button("停止").clicked() {
                    self.task = None;
                }
            }
        });
        ui.separator();
        match self.mode {
            DialogMode::Compute => self.ui_compute(ui),
            DialogMode::Verify => self.ui_verify(ui),
        }
    }

    fn ui_compute(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("算法:");
            egui::ComboBox::from_id_salt("fm-checksum-algo")
                .selected_text(self.algo_sel.label())
                .show_ui(ui, |ui| {
                    for algo in ChecksumAlgorithm::ALL {
                        ui.selectable_value(&mut self.algo_sel, algo, algo.label());
                    }
                });
            ui.label(egui::RichText::new("点击行复制该算法的校验和到剪贴板").weak());
        });
        ui.add_space(4.0);
        if self.files.is_empty() {
            ui.label(egui::RichText::new("（无文件——从右键菜单对文件「校验和…」打开）").weak());
            return;
        }
        let running = self.task.is_some();
        let mut copy: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, self.files.len(), |ui, range| {
                for row in range {
                    let Some(name) = self.files.get(row) else {
                        continue;
                    };
                    let width = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                    }
                    let text_color = ui.visuals().text_color();
                    let error_color = ui.visuals().error_fg_color;
                    let slot = self.results.get(row).and_then(|s| s.as_ref());
                    let (hex, hex_color) = match slot {
                        None if running => ("…".to_string(), text_color),
                        None => ("—".to_string(), text_color),
                        Some(Ok(h)) => (h.get(self.algo_sel).to_string(), text_color),
                        Some(Err(e)) => (format!("错误: {e}"), error_color),
                    };
                    // 名称列（裁剪防画进 hex 列）。
                    let hex_width = match self.algo_sel {
                        ChecksumAlgorithm::Crc32 => 80.0,
                        ChecksumAlgorithm::Sha1 => 300.0,
                        ChecksumAlgorithm::Sha256 => 460.0,
                    };
                    let name_rect = egui::Rect::from_min_max(
                        rect.min + egui::vec2(4.0, 0.0),
                        egui::pos2(rect.right() - hex_width - 8.0, rect.max.y),
                    );
                    ui.painter().with_clip_rect(name_rect).text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    ui.painter().text(
                        egui::pos2(rect.right() - 4.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        &hex,
                        egui::FontId::monospace(12.0),
                        hex_color,
                    );
                    if resp.clicked() {
                        if let Some(Ok(h)) = slot {
                            copy = Some(h.get(self.algo_sel).to_string());
                        }
                    }
                }
            });
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
        }
    }

    fn ui_verify(&mut self, ui: &mut egui::Ui) {
        let mut load = false;
        ui.horizontal(|ui| {
            ui.label("校验文件:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.verify_path)
                    .hint_text(".md5 / .sfv / .sha1 / .sha256")
                    .desired_width(320.0),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                load = true;
            }
            if ui.button("加载").clicked() {
                load = true;
            }
        });
        if load {
            self.load_verify();
        }
        if let Some(err) = &self.verify_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        if !self.verify_rows.is_empty() {
            let ok = self
                .verify_rows
                .iter()
                .filter(|r| r.status == VerifyStatus::Ok)
                .count();
            ui.label(format!("共 {} 条 · 通过 {ok} 条", self.verify_rows.len()));
        }
        ui.add_space(4.0);
        let ok_color = egui::Color32::from_rgb(0x4c, 0xc3, 0x8a);
        let err_color = ui.visuals().error_fg_color;
        let weak_color = ui.visuals().weak_text_color();
        let text_color = ui.visuals().text_color();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, self.verify_rows.len(), |ui, range| {
                for row in range {
                    let Some(vr) = self.verify_rows.get(row) else {
                        continue;
                    };
                    let width = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::hover());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                    }
                    let (mark, mark_color, tip) = match &vr.status {
                        VerifyStatus::Pending => ("…", weak_color, "计算中"),
                        VerifyStatus::Ok => ("✓", ok_color, "校验通过"),
                        VerifyStatus::Mismatch => ("✗", err_color, "校验不一致"),
                        VerifyStatus::Missing => ("?", weak_color, "文件缺失"),
                        VerifyStatus::Unsupported => ("—", weak_color, "MD5 算法不可用，无法验证"),
                        VerifyStatus::Error(e) => ("!", err_color, e.as_str()),
                    };
                    let name_rect = egui::Rect::from_min_max(
                        rect.min + egui::vec2(4.0, 0.0),
                        egui::pos2(rect.right() - 24.0, rect.max.y),
                    );
                    let display = format!("{}  {}", vr.filename, vr.expected);
                    ui.painter().with_clip_rect(name_rect).text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        display,
                        egui::FontId::monospace(12.0),
                        text_color,
                    );
                    ui.painter().text(
                        egui::pos2(rect.right() - 8.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        mark,
                        egui::FontId::proportional(13.0),
                        mark_color,
                    );
                    resp.on_hover_text(tip);
                }
            });
        // 底部提示（无记录时）。
        if self.verify_rows.is_empty() && self.verify_error.is_none() {
            ui.label(egui::RichText::new("选择 .md5/.sfv/.sha1/.sha256 校验文件后「加载」").weak());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_md5sum_style_lines() {
        let text = "\
d41d8cd98f00b204e9800998ecf8427e  empty.txt
a9993e364706816aba3e25717850c26c9cd0d89d *abc.bin
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  abc.dat
";
        let entries = parse_checksum_file(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].algorithm, None); // md5：无算法
        assert_eq!(entries[0].filename, "empty.txt");
        assert_eq!(entries[0].hash, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(entries[1].algorithm, Some(ChecksumAlgorithm::Sha1));
        assert_eq!(entries[1].filename, "abc.bin"); // `*` 二进制标记剥离
        assert_eq!(entries[2].algorithm, Some(ChecksumAlgorithm::Sha256));
    }

    #[test]
    fn parse_sfv_lines_with_comments_and_spaces() {
        let text = "\
; generated by test
file one.zip 0A12B34C

another file.rar deadbeef
# not a comment in sfv but skipped anyway
garbage line without hash
";
        let entries = parse_checksum_file(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].algorithm, Some(ChecksumAlgorithm::Crc32));
        assert_eq!(entries[0].filename, "file one.zip");
        assert_eq!(entries[0].hash, "0A12B34C"); // 大小写保留
        assert_eq!(entries[1].filename, "another file.rar");
        assert_eq!(entries[1].hash, "deadbeef");
    }

    #[test]
    fn parse_skips_malformed_lines() {
        assert!(parse_checksum_file("").is_empty());
        assert!(parse_checksum_file(";;;").is_empty());
        assert!(parse_checksum_file("abc 短 hex").is_empty());
        // 行首 8 位 hex + 文件名：不是 md5sum 格式，末 token 非 hex，跳过。
        assert!(parse_checksum_file("abcd1234 file.bin").is_empty());
    }

    #[test]
    fn checksum_file_name_detection() {
        assert!(is_checksum_file_name("a.md5"));
        assert!(is_checksum_file_name("B.SFV"));
        assert!(is_checksum_file_name("c.sha256"));
        assert!(!is_checksum_file_name("d.zip"));
        assert!(!is_checksum_file_name("md5"));
    }

    #[test]
    fn hash_accumulator_known_vectors() {
        let mut acc = HashAccumulator::new();
        acc.update(b"abc");
        let h = acc.finalize();
        assert_eq!(h.crc32, "352441c2");
        assert_eq!(h.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 空输入向量。
        let h = HashAccumulator::new().finalize();
        assert_eq!(h.crc32, "00000000");
        assert_eq!(h.sha1, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            h.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hash_file_matches_accumulator() {
        let dir = std::env::temp_dir().join(format!("openitgo-sum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.bin");
        // 跨块边界（>CHUNK 一部分即可，用小数据 + 重复保证确定性）。
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &data).unwrap();
        let cancel = AtomicBool::new(false);
        let got = hash_file(&file, &cancel).expect("hash ok");
        let mut acc = HashAccumulator::new();
        acc.update(&data);
        assert_eq!(got, acc.finalize());
        // 取消旗标预先置位：hash_file 第一轮即放弃。
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(hash_file(&file, &cancel).unwrap_err(), "已取消");
        std::fs::remove_file(&file).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
