//! 文件管理器缩略图视图（TC thumbnail view）的支撑模块：网格布局纯函数、
//! 缩略图后台加载器与容量缓存。渲染/交互在 `file_manager.rs`
//! （render_grid/render_grid_cell）。
//!
//! 加载器模型：FileManagerView 持有一个 `ThumbCache`（两栏共享），渲染
//! 可见 cell 时按需 `request`（同 path 去重）；单 worker 线程解码
//! （>64MB 跳过，与预览上限一致；webp 优先走 libwebp 缩放解码，失败回退
//! image crate），结果经 channel 回报、poll 上传为纹理。快速滚动时
//! 「代次 generation」丢弃过期请求：可见范围变化即 bump，worker 解码前
//! 比对，过期请求只回 Dropped 清在途标记，不浪费解码。
//!
//! 缓存失效：条目 (mtime, len) 作为有效性 key，变了自动重载；容量 500
//! 张按插入序逐出最旧（VecDeque + stamp 防重载后旧序误删）。

use crate::views::preview_bytes::PREVIEW_MAX_BYTES;
use crate::webp_thumb::decode_webp_thumbnail;
use crossbeam_channel::{Receiver, Sender};
use egui::ColorImage;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

/// 缩略图最长边（px）。
pub(crate) const THUMB_MAX_DIM: u32 = 160;
/// 网格 cell 尺寸（pt）：160 缩略图区 + 两行名称 + 边距。
pub(crate) const THUMB_CELL_W: f32 = 176.0;
pub(crate) const THUMB_CELL_H: f32 = 200.0;
/// 缓存容量（张），超出按插入序逐出最旧。
const THUMB_CACHE_CAP: usize = 500;
/// worker 单次排空的批量上限（防 try_recv 空转）。
const WORKER_BATCH_MAX: usize = 256;

/// 缓存有效性 key：条目修改时间 + 长度（变了自动失效重载）。
pub(crate) type ThumbKey = (Option<SystemTime>, u64);

/// 网格列数：栏宽 / cell 宽，至少 1 列。
pub(crate) fn grid_cols(available_width: f32) -> usize {
    (available_width / THUMB_CELL_W).floor().max(1.0) as usize
}

/// 网格行数（item_count = 线性 UI 行数，含「..」cell）。
pub(crate) fn grid_row_count(item_count: usize, cols: usize) -> usize {
    item_count.div_ceil(cols.max(1))
}

/// 线性 UI 行号 → 所在网格行（焦点滚动揭示/PgUp/PgDn 步进换算用）。
pub(crate) fn grid_row_of(linear_row: usize, cols: usize) -> usize {
    linear_row / cols.max(1)
}

/// cell 名称两行截断：按显示宽度估算（ASCII = 1 单位，其余 = 2），
/// 两行放不下时尾部替换为「…」。渲染侧再按像素宽度折行，这里只保证
/// 字符量上限（横向折行由 egui wrap 完成）。
pub(crate) fn truncate_cell_name(name: &str, line_units: usize) -> String {
    let budget = line_units * 2;
    let mut units = 0;
    for (i, c) in name.chars().enumerate() {
        let w = if c.is_ascii() { 1 } else { 2 };
        if units + w > budget {
            let mut s: String = name.chars().take(i.saturating_sub(1)).collect();
            s.push('…');
            return s;
        }
        units += w;
    }
    name.to_string()
}

/// 容量逐出核心（与值类型解耦便于单测）：map + 插入序队列。重载同
/// 路径会产生重复序项，evict 时比对 stamp，过期序项跳过不误删。
struct ThumbStore<V> {
    map: HashMap<PathBuf, StoreSlot<V>>,
    order: VecDeque<(u64, PathBuf)>,
    next_stamp: u64,
    cap: usize,
}

struct StoreSlot<V> {
    stamp: u64,
    key: ThumbKey,
    value: V,
}

impl<V> ThumbStore<V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            next_stamp: 0,
            cap,
        }
    }

    /// 命中且 key 匹配返回 value；key 不匹配（mtime/len 变了）删除旧
    /// 条目返回 None（调用方随之重请求）。
    fn get_valid(&mut self, path: &Path, key: &ThumbKey) -> Option<&V> {
        if let Some(slot) = self.map.get(path) {
            if &slot.key != key {
                self.map.remove(path);
                return None;
            }
        }
        self.map.get(path).map(|s| &s.value)
    }

    /// 插入（同路径覆盖）；超容量按插入序逐出最旧。
    fn insert(&mut self, path: PathBuf, key: ThumbKey, value: V) {
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        self.map
            .insert(path.clone(), StoreSlot { stamp, key, value });
        self.order.push_back((stamp, path));
        while self.map.len() > self.cap {
            let Some((order_stamp, order_path)) = self.order.pop_front() else {
                break;
            };
            // 序项已被重载覆盖（stamp 不同）时不删现条目。
            if self
                .map
                .get(&order_path)
                .is_some_and(|s| s.stamp == order_stamp)
            {
                self.map.remove(&order_path);
            }
        }
    }
}

/// 缩略图查找结果。
pub(crate) enum ThumbLookup {
    /// 未缓存（调用方随后 request）。
    Miss,
    /// 解码失败/非图片：已记忆，不再重试（画大字体图标）。
    Failed,
    Ready(egui::TextureHandle),
}

enum ThumbValue {
    Ready(egui::TextureHandle),
    Failed,
}

struct ThumbRequest {
    path: PathBuf,
    key: ThumbKey,
    generation: u64,
}

enum ThumbOutcome {
    Decoded(ColorImage),
    Failed,
    /// 代次过期被丢弃（不解码；poll 只清在途标记，不进缓存）。
    Dropped,
}

struct ThumbResult {
    path: PathBuf,
    key: ThumbKey,
    outcome: ThumbOutcome,
}

/// 缩略图缓存 + 后台解码 worker（两栏共享）。
pub(crate) struct ThumbCache {
    store: ThumbStore<ThumbValue>,
    /// 在途请求（path → key）：同 path 去重；key 变化允许重发。
    pending: HashMap<PathBuf, ThumbKey>,
    tx: Sender<ThumbRequest>,
    rx: Receiver<ThumbResult>,
    /// worker 侧可见的最新代次（可见范围变化即 bump）。
    latest_gen: Arc<AtomicU64>,
    generation: u64,
}

impl Default for ThumbCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbCache {
    pub(crate) fn new() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<ThumbRequest>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<ThumbResult>();
        let latest_gen = Arc::new(AtomicU64::new(0));
        let worker_latest = Arc::clone(&latest_gen);
        std::thread::Builder::new()
            .name("fm-thumbs".to_string())
            .spawn(move || worker_loop(req_rx, res_tx, worker_latest))
            .expect("spawn fm-thumbs worker");
        Self {
            store: ThumbStore::new(THUMB_CACHE_CAP),
            pending: HashMap::new(),
            tx: req_tx,
            rx: res_rx,
            latest_gen,
            generation: 0,
        }
    }

    /// 可见范围变化：bump 代次（请求随之携带 self.generation），
    /// worker 丢弃更早代次的在队请求。
    pub(crate) fn bump_generation(&mut self) {
        self.generation += 1;
        self.latest_gen.store(self.generation, Ordering::Relaxed);
    }

    /// 查缓存：key（mtime/len）不匹配视为 Miss（旧条目已删，随后
    /// request 会重载）。
    pub(crate) fn lookup(&mut self, path: &Path, key: &ThumbKey) -> ThumbLookup {
        match self.store.get_valid(path, key) {
            Some(ThumbValue::Ready(tex)) => ThumbLookup::Ready(tex.clone()),
            Some(ThumbValue::Failed) => ThumbLookup::Failed,
            None => ThumbLookup::Miss,
        }
    }

    /// 请求解码：已缓存 / 同 key 在途 → no-op。
    pub(crate) fn request(&mut self, path: PathBuf, key: ThumbKey) {
        if self.store.get_valid(&path, &key).is_some() {
            return;
        }
        if self.pending.get(&path) == Some(&key) {
            return;
        }
        self.pending.insert(path.clone(), key);
        let _ = self.tx.send(ThumbRequest {
            path,
            key,
            generation: self.generation,
        });
    }

    /// 有在途请求（调用方据此 request_repaint_after 排空结果）。
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// 排空结果通道：解码成功上传纹理进缓存；失败记 Failed 防每帧
    /// 重试；Dropped 只清在途标记。返回是否有结果落地。
    pub(crate) fn poll(&mut self, ctx: &egui::Context) -> bool {
        let mut landed = false;
        while let Ok(result) = self.rx.try_recv() {
            self.pending.remove(&result.path);
            match result.outcome {
                ThumbOutcome::Decoded(image) => {
                    let tex = ctx.load_texture(
                        format!("fm-thumb-{}", result.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.store
                        .insert(result.path, result.key, ThumbValue::Ready(tex));
                    landed = true;
                }
                ThumbOutcome::Failed => {
                    self.store
                        .insert(result.path, result.key, ThumbValue::Failed);
                }
                ThumbOutcome::Dropped => {}
            }
        }
        landed
    }
}

/// worker：阻塞收首个请求后排空积压成批，**逆序**处理（新请求优先），
/// 同路径只留最新一次；代次过期直接回 Dropped 不解码。
fn worker_loop(rx: Receiver<ThumbRequest>, tx: Sender<ThumbResult>, latest_gen: Arc<AtomicU64>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(req) = rx.try_recv() {
            batch.push(req);
            if batch.len() >= WORKER_BATCH_MAX {
                break;
            }
        }
        let mut seen = HashSet::new();
        for req in batch.into_iter().rev() {
            if !seen.insert(req.path.clone()) {
                continue;
            }
            if req.generation < latest_gen.load(Ordering::Relaxed) {
                if tx
                    .send(ThumbResult {
                        path: req.path,
                        key: req.key,
                        outcome: ThumbOutcome::Dropped,
                    })
                    .is_err()
                {
                    return;
                }
                continue;
            }
            let outcome = match decode_thumb(&req.path) {
                Some(image) => ThumbOutcome::Decoded(image),
                None => ThumbOutcome::Failed,
            };
            if tx
                .send(ThumbResult {
                    path: req.path,
                    key: req.key,
                    outcome,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// 后台解码：>64MB 跳过（与预览上限一致）；webp 优先 libwebp 缩放解码
/// （失败回退 image crate），其余 image crate 读图后 thumbnail 等比缩到
/// 160px 内。任何失败返回 None（调用方记忆 Failed，回退大字体图标）。
fn decode_thumb(path: &Path) -> Option<ColorImage> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > PREVIEW_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
    {
        if let Some((image, _)) = decode_webp_thumbnail(&bytes, THUMB_MAX_DIM) {
            return Some(image);
        }
    }
    let img = image::load_from_memory(&bytes).ok()?;
    let thumb = img.thumbnail(THUMB_MAX_DIM, THUMB_MAX_DIM);
    let rgba = thumb.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        rgba.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_cols_at_least_one_and_floors() {
        assert_eq!(grid_cols(0.0), 1);
        assert_eq!(grid_cols(THUMB_CELL_W - 1.0), 1);
        assert_eq!(grid_cols(THUMB_CELL_W), 1);
        assert_eq!(grid_cols(THUMB_CELL_W * 2.0 + 10.0), 2);
        assert_eq!(grid_cols(THUMB_CELL_W * 5.0 - 1.0), 4);
    }

    #[test]
    fn grid_row_math() {
        assert_eq!(grid_row_count(0, 3), 0);
        assert_eq!(grid_row_count(1, 3), 1);
        assert_eq!(grid_row_count(3, 3), 1);
        assert_eq!(grid_row_count(4, 3), 2);
        assert_eq!(grid_row_of(0, 4), 0);
        assert_eq!(grid_row_of(3, 4), 0);
        assert_eq!(grid_row_of(4, 4), 1);
        assert_eq!(grid_row_of(9, 4), 2);
    }

    #[test]
    fn truncate_cell_name_two_line_budget() {
        // 短名原样。
        assert_eq!(truncate_cell_name("abc.txt", 24), "abc.txt");
        // ASCII：预算 48 单位，超出截断 + 省略号。
        let long = "a".repeat(60);
        let out = truncate_cell_name(&long, 24);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 48);
        // CJK 按 2 单位计：24 个汉字 = 48 单位整好放下，25 个触发截断。
        let cjk = "漫".repeat(24);
        assert_eq!(truncate_cell_name(&cjk, 24), cjk);
        let cjk_over = "漫".repeat(25);
        assert!(truncate_cell_name(&cjk_over, 24).ends_with('…'));
    }

    fn key(n: u64) -> ThumbKey {
        (None, n)
    }

    #[test]
    fn store_evicts_oldest_and_respects_key() {
        let mut store: ThumbStore<u32> = ThumbStore::new(2);
        store.insert(PathBuf::from("a"), key(1), 10);
        store.insert(PathBuf::from("b"), key(1), 20);
        assert_eq!(store.get_valid(Path::new("a"), &key(1)), Some(&10));
        // 超容量：最旧的 a 被逐出。
        store.insert(PathBuf::from("c"), key(1), 30);
        assert_eq!(store.map.len(), 2);
        assert_eq!(store.get_valid(Path::new("a"), &key(1)), None);
        assert_eq!(store.get_valid(Path::new("b"), &key(1)), Some(&20));
        assert_eq!(store.get_valid(Path::new("c"), &key(1)), Some(&30));
        // key 不匹配（mtime/len 变了）= 失效。
        assert_eq!(store.get_valid(Path::new("b"), &key(2)), None);
        assert_eq!(store.map.len(), 1);
    }

    #[test]
    fn store_reinsert_survives_stale_order_entry() {
        let mut store: ThumbStore<u32> = ThumbStore::new(2);
        store.insert(PathBuf::from("a"), key(1), 10);
        store.insert(PathBuf::from("b"), key(1), 20);
        // a 重载（key 变化）：新 stamp，order 里留下旧序项。
        store.insert(PathBuf::from("a"), key(2), 11);
        // 触发逐出：旧序项 (a, stamp0) 先弹出，但 a 的现条目是 stamp2，
        // 比对不跳过误删；真正最旧的有效条目是 b。
        store.insert(PathBuf::from("c"), key(1), 30);
        assert_eq!(store.get_valid(Path::new("a"), &key(2)), Some(&11));
        assert_eq!(store.get_valid(Path::new("c"), &key(1)), Some(&30));
        assert_eq!(store.get_valid(Path::new("b"), &key(1)), None);
    }
}
