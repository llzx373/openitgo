//! 比较内容对话框（阶段 AF，TC「按内容比较」简化版）：右键「比较内容…」
//! （选中集恰为 2 个文件）或「比较两栏焦点文件」（双栏两栏各有焦点文件）
//! 打开非模态 egui::Window。打开即后台比较（worker + 取消，搜索器同款
//! 模式）：二进制快比——大小不同报「不同（大小不等）」，否则 256KB 分块
//! 比对报「相同」或首个差异偏移；两文件都判定为文本（扩展名命中
//! `is_text_extension` 或前 8KB 无 NUL 嗅探）且 ≤64MB（同预览上限）时
//! 额外产出行级 diff（`decode_text_guess` 解码）。行级 diff 为自实现
//! LCS（u16 DP 表 + 回溯，不引依赖）：纯函数 `diff_lines` 单测覆盖；
//! 行数乘积超 4000×4000 回退「文件过大，仅二进制比较」。渲染 = 顶部
//! 结论行 + 单 ScrollArea 内左右对照行（Del 红底/Add 绿底/Same 无色，
//! 空位占位；单滚动区天然联动）。

use crate::views::file_ops::verbatim_path;
use crate::views::preview_bytes::is_text_extension;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 二进制/文本分块大小。
const CHUNK: usize = 256 * 1024;
/// 文本 diff 全文大小上限（同预览上限）。
const TEXT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 文本嗅探前缀（无 NUL 视为文本；扩展名未命中时）。
const SNIFF_BYTES: usize = 8192;
/// diff 行数乘积上限（(n+1)×(m+1)；超出回退仅二进制比较）。
const DIFF_MAX_PRODUCT: u64 = 4001 * 4001;
/// diff 列表行高（pt）。
const ROW_H: f32 = 18.0;

/// 行差异类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    Add,
    Del,
}

/// 一行 diff（行号 1-based；Add 无左号，Del 无右号）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub left_no: Option<usize>,
    pub right_no: Option<usize>,
    pub text: String,
}

/// 行级 LCS diff（纯函数；u16 DP 表 + 回溯，调用方保证行数乘积
/// ≤ `DIFF_MAX_PRODUCT`）。平局（下行与右行 LCS 等长）优先 Del，使
/// 修改呈现为「先删后增」相邻块。
pub fn diff_lines(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let mut table = vec![0u16; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1].saturating_add(1)
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine {
                kind: DiffKind::Same,
                left_no: Some(i + 1),
                right_no: Some(j + 1),
                text: a[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            out.push(DiffLine {
                kind: DiffKind::Del,
                left_no: Some(i + 1),
                right_no: None,
                text: a[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffKind::Add,
                left_no: None,
                right_no: Some(j + 1),
                text: b[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: DiffKind::Del,
            left_no: Some(i + 1),
            right_no: None,
            text: a[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: DiffKind::Add,
            left_no: None,
            right_no: Some(j + 1),
            text: b[j].to_string(),
        });
        j += 1;
    }
    out
}

/// 二进制比较结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryVerdict {
    /// 内容完全一致。
    Same,
    /// 大小不同。
    DiffSize,
    /// 大小相同，首个差异字节偏移。
    DiffAt(u64),
    /// IO 失败（比较未完成）。
    Failed(String),
}

impl BinaryVerdict {
    pub fn label(&self) -> String {
        match self {
            BinaryVerdict::Same => "二进制比较：相同".to_string(),
            BinaryVerdict::DiffSize => "二进制比较：不同（大小不等）".to_string(),
            BinaryVerdict::DiffAt(off) => {
                format!("二进制比较：不同（首个差异位于偏移 0x{off:X}）")
            }
            BinaryVerdict::Failed(e) => format!("比较失败: {e}"),
        }
    }
}

/// 文本 diff 结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextDiff {
    /// 行级 diff 完成（行集 + 增/删统计）。
    Lines {
        lines: Vec<DiffLine>,
        adds: usize,
        dels: usize,
    },
    /// 行数乘积超阈值（仅二进制比较）。
    TooManyLines,
    /// 非文本或超 64MB（仅二进制比较）。
    NotText,
}

/// 比较结果（worker → UI）。
#[derive(Debug)]
pub struct CompareResult {
    pub binary: BinaryVerdict,
    pub text: TextDiff,
}

/// 文本判定（worker 内部）：扩展名命中，或前 SNIFF_BYTES 无 NUL。
fn looks_like_text(path: &Path, bytes: &[u8]) -> bool {
    let ext_hit = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_text_extension);
    if ext_hit {
        return true;
    }
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    !head.contains(&0)
}

/// 二进制分块比对（worker 核心）：大小不同 → DiffSize；否则逐块比对
/// 报首个差异偏移；cancel 命中按 Failed("已取消") 收尾。
fn binary_compare(left: &Path, right: &Path, cancel: &AtomicBool) -> BinaryVerdict {
    use std::io::Read;
    let meta_l = match std::fs::metadata(verbatim_path(left)) {
        Ok(m) => m,
        Err(e) => return BinaryVerdict::Failed(format!("{}: {e}", left.display())),
    };
    let meta_r = match std::fs::metadata(verbatim_path(right)) {
        Ok(m) => m,
        Err(e) => return BinaryVerdict::Failed(format!("{}: {e}", right.display())),
    };
    if meta_l.len() != meta_r.len() {
        return BinaryVerdict::DiffSize;
    }
    let mut fl = match std::fs::File::open(verbatim_path(left)) {
        Ok(f) => f,
        Err(e) => return BinaryVerdict::Failed(format!("{}: {e}", left.display())),
    };
    let mut fr = match std::fs::File::open(verbatim_path(right)) {
        Ok(f) => f,
        Err(e) => return BinaryVerdict::Failed(format!("{}: {e}", right.display())),
    };
    let (mut bl, mut br) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    let mut offset = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return BinaryVerdict::Failed("已取消".to_string());
        }
        let nl = match fl.read(&mut bl) {
            Ok(n) => n,
            Err(e) => return BinaryVerdict::Failed(e.to_string()),
        };
        let nr = match fr.read(&mut br) {
            Ok(n) => n,
            Err(e) => return BinaryVerdict::Failed(e.to_string()),
        };
        if nl == 0 && nr == 0 {
            return BinaryVerdict::Same;
        }
        if nl != nr {
            // 大小相同但读到的块长不一致（文件被并发改动）：差异位置即较短块末尾。
            return BinaryVerdict::DiffAt(offset + nl.min(nr) as u64);
        }
        if bl[..nl] != br[..nr] {
            let pos = bl[..nl]
                .iter()
                .zip(br[..nr].iter())
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            return BinaryVerdict::DiffAt(offset + pos as u64);
        }
        offset += nl as u64;
    }
}

/// 文本 diff（worker 核心）：两文件 ≤64MB 且文本判定通过才做；行数
/// 乘积超阈值回退 TooManyLines。读取失败按 NotText 降级（二进制结论
/// 已含 IO 错误时不重复上报）。
fn text_diff(left: &Path, right: &Path, cancel: &AtomicBool) -> TextDiff {
    let Ok(bytes_l) = std::fs::read(verbatim_path(left)) else {
        return TextDiff::NotText;
    };
    if cancel.load(Ordering::Relaxed) {
        return TextDiff::NotText;
    }
    let Ok(bytes_r) = std::fs::read(verbatim_path(right)) else {
        return TextDiff::NotText;
    };
    if !looks_like_text(left, &bytes_l) || !looks_like_text(right, &bytes_r) {
        return TextDiff::NotText;
    }
    let (Some(text_l), Some(text_r)) = (
        openitgo_parser::archive::decode_text_guess(&bytes_l),
        openitgo_parser::archive::decode_text_guess(&bytes_r),
    ) else {
        return TextDiff::NotText;
    };
    let lines_l: Vec<&str> = text_l.lines().collect();
    let lines_r: Vec<&str> = text_r.lines().collect();
    if (lines_l.len() as u64 + 1) * (lines_r.len() as u64 + 1) > DIFF_MAX_PRODUCT {
        return TextDiff::TooManyLines;
    }
    let lines = diff_lines(&lines_l, &lines_r);
    let adds = lines.iter().filter(|l| l.kind == DiffKind::Add).count();
    let dels = lines.iter().filter(|l| l.kind == DiffKind::Del).count();
    TextDiff::Lines { lines, adds, dels }
}

/// worker → UI 事件。
enum CompareEvent {
    Done(Box<CompareResult>),
}

/// 后台比较任务。Drop 即置取消。
pub struct CompareTask {
    rx: crossbeam_channel::Receiver<CompareEvent>,
    cancel: Arc<AtomicBool>,
}

impl CompareTask {
    pub fn start(left: PathBuf, right: PathBuf) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel2 = cancel.clone();
        std::thread::Builder::new()
            .name("fm-compare".to_string())
            .spawn(move || {
                // 64MB 上限只约束文本 diff；二进制比较分块读无上限。
                let binary = binary_compare(&left, &right, &cancel2);
                if cancel2.load(Ordering::Relaxed) {
                    return;
                }
                let text = if std::fs::metadata(verbatim_path(&left))
                    .map(|m| m.len() <= TEXT_MAX_BYTES)
                    .unwrap_or(false)
                    && std::fs::metadata(verbatim_path(&right))
                        .map(|m| m.len() <= TEXT_MAX_BYTES)
                        .unwrap_or(false)
                {
                    text_diff(&left, &right, &cancel2)
                } else {
                    TextDiff::NotText
                };
                let _ = tx.send(CompareEvent::Done(Box::new(CompareResult { binary, text })));
            })
            .ok();
        Self { rx, cancel }
    }

    fn drain(&self) -> Option<CompareResult> {
        let mut done = None;
        for ev in self.rx.try_iter() {
            match ev {
                CompareEvent::Done(r) => done = Some(*r),
            }
        }
        done
    }
}

impl Drop for CompareTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 比较对话框状态（FileManagerView 持有；非模态 egui::Window）。
pub struct CompareDialog {
    open: bool,
    left: PathBuf,
    right: PathBuf,
    task: Option<CompareTask>,
    result: Option<CompareResult>,
}

impl Default for CompareDialog {
    fn default() -> Self {
        Self {
            open: false,
            left: PathBuf::new(),
            right: PathBuf::new(),
            task: None,
            result: None,
        }
    }
}

impl CompareDialog {
    /// 打开并开始比较。
    pub fn open_with(&mut self, left: PathBuf, right: PathBuf) {
        self.left = left;
        self.right = right;
        self.open = true;
        self.result = None;
        self.task = Some(CompareTask::start(self.left.clone(), self.right.clone()));
    }

    /// 渲染（open 时）。比较在途时主动重绘。
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if let Some(task) = &self.task {
            if let Some(result) = task.drain() {
                self.result = Some(result);
                self.task = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut open = self.open;
        egui::Window::new("比较内容")
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 520.0])
            .open(&mut open)
            .show(ctx, |ui| {
                self.ui_body(ui);
            });
        self.open = open;
        if !open {
            self.task = None;
        }
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) {
        // 文件对。
        ui.horizontal(|ui| {
            ui.label("左：");
            ui.label(egui::RichText::new(self.left.display().to_string()).strong());
        });
        ui.horizontal(|ui| {
            ui.label("右：");
            ui.label(egui::RichText::new(self.right.display().to_string()).strong());
        });
        if self.task.is_some() {
            ui.label("比较中…");
            return;
        }
        let Some(result) = &self.result else {
            return;
        };
        // 结论行 + 差异统计。
        ui.horizontal(|ui| {
            let same = matches!(result.binary, BinaryVerdict::Same);
            let color = if same {
                egui::Color32::from_rgb(0x4c, 0xc3, 0x8a)
            } else {
                ui.visuals().error_fg_color
            };
            ui.colored_label(color, result.binary.label());
            if let TextDiff::Lines { adds, dels, .. } = &result.text {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(0x4c, 0xc3, 0x8a),
                    format!("+{adds}"),
                );
                ui.colored_label(ui.visuals().error_fg_color, format!("−{dels}"));
            }
        });
        ui.separator();
        match &result.text {
            TextDiff::NotText => {
                ui.label(egui::RichText::new("非文本文件或超过 64MB，仅二进制比较").weak());
            }
            TextDiff::TooManyLines => {
                ui.label(egui::RichText::new("文件过大，仅二进制比较").weak());
            }
            TextDiff::Lines { lines, .. } => {
                self.ui_diff_lines(ui, lines);
            }
        }
    }

    /// 左右对照 diff 列表：单 ScrollArea 逐行两列（滚动天然联动）；
    /// Del 左红底右占位，Add 左占位右绿底，Same 无色。
    fn ui_diff_lines(&self, ui: &mut egui::Ui, lines: &[DiffLine]) {
        let del_bg = egui::Color32::from_rgb(0x5a, 0x2d, 0x2d);
        let add_bg = egui::Color32::from_rgb(0x2d, 0x4a, 0x2d);
        let text_color = ui.visuals().text_color();
        let no_color = ui.visuals().weak_text_color();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, lines.len(), |ui, range| {
                let half = (ui.available_width() - 8.0) / 2.0;
                for row in range {
                    let Some(line) = lines.get(row) else {
                        continue;
                    };
                    let (rect, _resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ROW_H),
                        egui::Sense::hover(),
                    );
                    let left_rect = egui::Rect::from_min_size(rect.min, egui::vec2(half, ROW_H));
                    let right_rect = egui::Rect::from_min_size(
                        rect.min + egui::vec2(half + 8.0, 0.0),
                        egui::vec2(half, ROW_H),
                    );
                    // 左半：Same/Del 显内容（Del 红底），Add 占位。
                    match line.kind {
                        DiffKind::Add => {}
                        DiffKind::Same => {
                            paint_cell(
                                ui,
                                left_rect,
                                line.left_no,
                                &line.text,
                                None,
                                no_color,
                                text_color,
                            );
                        }
                        DiffKind::Del => {
                            paint_cell(
                                ui,
                                left_rect,
                                line.left_no,
                                &line.text,
                                Some(del_bg),
                                no_color,
                                text_color,
                            );
                        }
                    }
                    // 右半：Same/Add 显内容（Add 绿底），Del 占位。
                    match line.kind {
                        DiffKind::Del => {}
                        DiffKind::Same => {
                            paint_cell(
                                ui,
                                right_rect,
                                line.right_no,
                                &line.text,
                                None,
                                no_color,
                                text_color,
                            );
                        }
                        DiffKind::Add => {
                            paint_cell(
                                ui,
                                right_rect,
                                line.right_no,
                                &line.text,
                                Some(add_bg),
                                no_color,
                                text_color,
                            );
                        }
                    }
                }
            });
    }
}

/// 单个 diff 单元格：行号（右对齐弱色）+ 文本（monospace 裁剪）+ 可选底色。
fn paint_cell(
    ui: &egui::Ui,
    rect: egui::Rect,
    line_no: Option<usize>,
    text: &str,
    bg: Option<egui::Color32>,
    no_color: egui::Color32,
    text_color: egui::Color32,
) {
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let no_rect = egui::Rect::from_min_size(rect.min, egui::vec2(44.0, rect.height()));
    if let Some(no) = line_no {
        ui.painter().text(
            egui::pos2(no_rect.right() - 2.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            no.to_string(),
            egui::FontId::monospace(11.0),
            no_color,
        );
    }
    let text_rect =
        egui::Rect::from_min_max(egui::pos2(no_rect.right() + 4.0, rect.min.y), rect.max);
    ui.painter().with_clip_rect(text_rect).text(
        text_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::monospace(12.0),
        text_color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<DiffKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn diff_identical() {
        let a = vec!["x", "y", "z"];
        let lines = diff_lines(&a, &a);
        assert_eq!(kinds(&lines), vec![DiffKind::Same; 3]);
        assert_eq!(lines[1].left_no, Some(2));
        assert_eq!(lines[1].right_no, Some(2));
    }

    #[test]
    fn diff_add_del_change() {
        // 增：b 多一行。
        let lines = diff_lines(&["a", "c"], &["a", "b", "c"]);
        assert_eq!(
            kinds(&lines),
            vec![DiffKind::Same, DiffKind::Add, DiffKind::Same]
        );
        assert_eq!(lines[1].text, "b");
        assert_eq!(lines[1].right_no, Some(2));
        assert_eq!(lines[1].left_no, None);
        // 删：a 多一行。
        let lines = diff_lines(&["a", "b", "c"], &["a", "c"]);
        assert_eq!(
            kinds(&lines),
            vec![DiffKind::Same, DiffKind::Del, DiffKind::Same]
        );
        assert_eq!(lines[1].left_no, Some(2));
        assert_eq!(lines[1].right_no, None);
        // 改：呈现为先删后增相邻块。
        let lines = diff_lines(&["a", "old", "c"], &["a", "new", "c"]);
        let k = kinds(&lines);
        assert_eq!(k[0], DiffKind::Same);
        assert!(k.contains(&DiffKind::Del));
        assert!(k.contains(&DiffKind::Add));
        assert_eq!(k.last(), Some(&DiffKind::Same));
        let del_pos = k.iter().position(|x| *x == DiffKind::Del).unwrap();
        let add_pos = k.iter().position(|x| *x == DiffKind::Add).unwrap();
        assert!(del_pos < add_pos, "修改应呈现先删后增");
    }

    #[test]
    fn diff_empty_inputs() {
        // 双空：无行。
        assert!(diff_lines(&[], &[]).is_empty());
        // 左空：全 Add。
        let lines = diff_lines(&[], &["a", "b"]);
        assert_eq!(kinds(&lines), vec![DiffKind::Add; 2]);
        // 右空：全 Del。
        let lines = diff_lines(&["a", "b"], &[]);
        assert_eq!(kinds(&lines), vec![DiffKind::Del; 2]);
    }

    #[test]
    fn diff_large_all_different() {
        // 大输入（阈值内 worst case 附近）：全不同 → n 删 + m 增。
        let a: Vec<String> = (0..2000).map(|i| format!("a{i}")).collect();
        let b: Vec<String> = (0..2000).map(|i| format!("b{i}")).collect();
        let av: Vec<&str> = a.iter().map(String::as_str).collect();
        let bv: Vec<&str> = b.iter().map(String::as_str).collect();
        let lines = diff_lines(&av, &bv);
        assert_eq!(
            lines.iter().filter(|l| l.kind == DiffKind::Del).count(),
            2000
        );
        assert_eq!(
            lines.iter().filter(|l| l.kind == DiffKind::Add).count(),
            2000
        );
        assert!(!lines.iter().any(|l| l.kind == DiffKind::Same));
    }

    #[test]
    fn binary_compare_verdicts() {
        let t = tempfile::tempdir().unwrap();
        let f1 = t.path().join("f1.bin");
        let f2 = t.path().join("f2.bin");
        let cancel = AtomicBool::new(false);
        std::fs::write(&f1, b"hello world").unwrap();
        std::fs::write(&f2, b"hello world").unwrap();
        assert_eq!(binary_compare(&f1, &f2, &cancel), BinaryVerdict::Same);
        // 大小不同。
        std::fs::write(&f2, b"hello").unwrap();
        assert_eq!(binary_compare(&f1, &f2, &cancel), BinaryVerdict::DiffSize);
        // 同大小首差异偏移（跨块验证用小数据 + 明确偏移）。
        let data1: Vec<u8> = (0..300_000u32).map(|i| (i % 256) as u8).collect();
        let mut data2 = data1.clone();
        data2[299_999] ^= 0xFF;
        std::fs::write(&f1, &data1).unwrap();
        std::fs::write(&f2, &data2).unwrap();
        assert_eq!(
            binary_compare(&f1, &f2, &cancel),
            BinaryVerdict::DiffAt(299_999)
        );
    }

    #[test]
    fn text_diff_detection_and_outcome() {
        let t = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // 文本（扩展名命中）。
        let a = t.path().join("a.txt");
        let b = t.path().join("b.txt");
        std::fs::write(&a, "line1\nline2\n").unwrap();
        std::fs::write(&b, "line1\nchanged\nline3\n").unwrap();
        match text_diff(&a, &b, &cancel) {
            TextDiff::Lines { adds, dels, lines } => {
                assert_eq!((adds, dels), (2, 1));
                assert!(lines.iter().any(|l| l.kind == DiffKind::Same));
            }
            other => panic!("应为 Lines: {other:?}"),
        }
        // 无扩展名但内容无 NUL → 嗅探为文本。
        let c = t.path().join("c_data");
        std::fs::write(&c, "plain text content\n").unwrap();
        assert!(matches!(text_diff(&a, &c, &cancel), TextDiff::Lines { .. }));
        // 含 NUL → 非文本。
        let d = t.path().join("d.bin");
        std::fs::write(&d, b"ab\0cd").unwrap();
        assert_eq!(text_diff(&a, &d, &cancel), TextDiff::NotText);
    }
}
