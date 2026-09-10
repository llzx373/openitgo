//! 文件管理器系统真实图标（阶段 S）的支撑模块：缓存 key 纯函数、后台
//! 加载 worker 与容量缓存。渲染接线在 `file_manager.rs`（列表行 16pt /
//! 网格非图片 cell 32pt 档）。
//!
//! 加载器模型仿 `file_manager_thumbs`（ThumbCache）：FileManagerView 持有
//! 一个 `SysIconCache`（两栏共享），渲染时按需 `request`（同 key 去重）；
//! 单 worker 线程经 `platform::file_icons::extract_icon` 取 Shell 图标，
//! 结果经 channel 回报、poll 上传为纹理。可见范围变化 bump 代次，worker
//! 丢弃过期在队请求。
//!
//! key 规则（`sys_icon_key`）：目录 = 单一键；exe/lnk/ico = 完整路径
//! （图标内嵌于文件自身）；其他 = 小写扩展名（无扩展名为空串）。图标不
//! 随 mtime 变化，无有效性维度；容量 256 按插入序逐出最旧。

use crossbeam_channel::{Receiver, Sender};
use egui::ColorImage;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 缓存容量（个），超出按插入序逐出最旧。
const ICON_CACHE_CAP: usize = 256;
/// worker 单次排空的批量上限（防 try_recv 空转）。
const WORKER_BATCH_MAX: usize = 256;

/// 图标缓存 key 的种类（与尺寸档无关，尺寸在 `SysIconKey.large`）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SysIconKind {
    /// 目录：所有目录共享一个系统文件夹图标。
    Dir,
    /// exe/lnk/ico：图标内嵌于文件自身，按完整路径缓存。
    Path(PathBuf),
    /// 普通文件：按小写扩展名缓存（无扩展名为空串）。
    Ext(String),
}

/// 完整缓存 key：种类 + 尺寸档（16pt 列表 / 32pt 网格各存一份）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SysIconKey {
    kind: SysIconKind,
    large: bool,
}

/// 缓存 key 纯函数。exe/lnk/ico 走完整路径；目录归一为 Dir；其余取
/// 小写扩展名（无扩展名 = 空串，共享系统默认文件图标）。
pub(crate) fn sys_icon_kind(path: &Path, is_dir: bool) -> SysIconKind {
    if is_dir {
        return SysIconKind::Dir;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(ext.as_str(), "exe" | "lnk" | "ico") {
        SysIconKind::Path(path.to_path_buf())
    } else {
        SysIconKind::Ext(ext)
    }
}

/// 容量逐出核心：map + 插入序队列；重插同 key 留下旧序项，evict 时
/// 比对 stamp 防误删（与 ThumbStore 同构，无有效性维度）。
struct IconStore<V> {
    map: HashMap<SysIconKey, StoreSlot<V>>,
    order: VecDeque<(u64, SysIconKey)>,
    next_stamp: u64,
    cap: usize,
}

struct StoreSlot<V> {
    stamp: u64,
    value: V,
}

impl<V> IconStore<V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            next_stamp: 0,
            cap,
        }
    }

    fn get(&self, key: &SysIconKey) -> Option<&V> {
        self.map.get(key).map(|s| &s.value)
    }

    /// 插入（同 key 覆盖）；超容量按插入序逐出最旧。
    fn insert(&mut self, key: SysIconKey, value: V) {
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        self.map.insert(key.clone(), StoreSlot { stamp, value });
        self.order.push_back((stamp, key));
        while self.map.len() > self.cap {
            let Some((order_stamp, order_key)) = self.order.pop_front() else {
                break;
            };
            // 序项已被重插覆盖（stamp 不同）时不删现条目。
            if self
                .map
                .get(&order_key)
                .is_some_and(|s| s.stamp == order_stamp)
            {
                self.map.remove(&order_key);
            }
        }
    }
}

/// 图标查找结果。
pub(crate) enum SysIconLookup {
    /// 未缓存（调用方随后 request）。
    Miss,
    /// 提取失败：已记忆，不再重试（画字体图标）。
    Failed,
    Ready(egui::TextureHandle),
}

enum IconValue {
    Ready(egui::TextureHandle),
    Failed,
}

struct IconRequest {
    key: SysIconKey,
    path: PathBuf,
    is_dir: bool,
    generation: u64,
}

enum IconOutcome {
    Decoded(ColorImage),
    Failed,
    /// 代次过期被丢弃（poll 只清在途标记，不进缓存）。
    Dropped,
}

struct IconResult {
    key: SysIconKey,
    outcome: IconOutcome,
}

/// 系统图标缓存 + 后台提取 worker（两栏共享）。
pub(crate) struct SysIconCache {
    store: IconStore<IconValue>,
    /// 在途请求（key 去重）。
    pending: HashSet<SysIconKey>,
    tx: Sender<IconRequest>,
    rx: Receiver<IconResult>,
    /// worker 侧可见的最新代次（可见范围变化即 bump）。
    latest_gen: Arc<AtomicU64>,
    generation: u64,
}

impl Default for SysIconCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SysIconCache {
    pub(crate) fn new() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<IconRequest>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<IconResult>();
        let latest_gen = Arc::new(AtomicU64::new(0));
        let worker_latest = Arc::clone(&latest_gen);
        std::thread::Builder::new()
            .name("fm-sys-icons".to_string())
            .spawn(move || worker_loop(req_rx, res_tx, worker_latest))
            .expect("spawn fm-sys-icons worker");
        Self {
            store: IconStore::new(ICON_CACHE_CAP),
            pending: HashSet::new(),
            tx: req_tx,
            rx: res_rx,
            latest_gen,
            generation: 0,
        }
    }

    /// 可见范围变化：bump 代次，worker 丢弃更早代次的在队请求。
    pub(crate) fn bump_generation(&mut self) {
        self.generation += 1;
        self.latest_gen.store(self.generation, Ordering::Relaxed);
    }

    /// 查缓存。
    pub(crate) fn lookup(&self, kind: &SysIconKind, large: bool) -> SysIconLookup {
        let key = SysIconKey {
            kind: kind.clone(),
            large,
        };
        match self.store.get(&key) {
            Some(IconValue::Ready(tex)) => SysIconLookup::Ready(tex.clone()),
            Some(IconValue::Failed) => SysIconLookup::Failed,
            None => SysIconLookup::Miss,
        }
    }

    /// 请求提取：已缓存 / 同 key 在途 → no-op。
    pub(crate) fn request(&mut self, path: &Path, is_dir: bool, large: bool) {
        let key = SysIconKey {
            kind: sys_icon_kind(path, is_dir),
            large,
        };
        if self.store.get(&key).is_some() || !self.pending.insert(key.clone()) {
            return;
        }
        let _ = self.tx.send(IconRequest {
            key,
            path: path.to_path_buf(),
            is_dir,
            generation: self.generation,
        });
    }

    /// 有在途请求（调用方据此 request_repaint_after 排空结果）。
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// 排空结果通道：成功上传纹理进缓存；失败记 Failed 防每帧重试；
    /// Dropped 只清在途标记。返回是否有结果落地。
    pub(crate) fn poll(&mut self, ctx: &egui::Context) -> bool {
        let mut landed = false;
        while let Ok(result) = self.rx.try_recv() {
            self.pending.remove(&result.key);
            match result.outcome {
                IconOutcome::Decoded(image) => {
                    let tex = ctx.load_texture(
                        format!("fm-sys-icon-{:?}", result.key),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.store.insert(result.key, IconValue::Ready(tex));
                    landed = true;
                }
                IconOutcome::Failed => {
                    self.store.insert(result.key, IconValue::Failed);
                }
                IconOutcome::Dropped => {}
            }
        }
        landed
    }
}

/// worker：阻塞收首个请求后排空积压成批，**逆序**处理（新请求优先），
/// 同 key 只留最新一次；代次过期直接回 Dropped 不提取。
fn worker_loop(rx: Receiver<IconRequest>, tx: Sender<IconResult>, latest_gen: Arc<AtomicU64>) {
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
            if !seen.insert(req.key.clone()) {
                continue;
            }
            if req.generation < latest_gen.load(Ordering::Relaxed) {
                if tx
                    .send(IconResult {
                        key: req.key,
                        outcome: IconOutcome::Dropped,
                    })
                    .is_err()
                {
                    return;
                }
                continue;
            }
            let outcome = match crate::platform::file_icons::extract_icon(
                &req.path,
                req.is_dir,
                req.key.large,
            ) {
                Some(image) => IconOutcome::Decoded(image),
                None => IconOutcome::Failed,
            };
            if tx
                .send(IconResult {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_rules() {
        // 目录归一。
        assert_eq!(
            sys_icon_kind(Path::new("C:/foo/bar"), true),
            SysIconKind::Dir
        );
        assert_eq!(sys_icon_kind(Path::new("D:/other"), true), SysIconKind::Dir);
        // exe/lnk/ico 按完整路径。
        assert_eq!(
            sys_icon_kind(Path::new("C:/a.exe"), false),
            SysIconKind::Path(PathBuf::from("C:/a.exe"))
        );
        assert_eq!(
            sys_icon_kind(Path::new("C:/b.LNK"), false),
            SysIconKind::Path(PathBuf::from("C:/b.LNK"))
        );
        // 普通文件按小写扩展名。
        assert_eq!(
            sys_icon_kind(Path::new("C:/a.TXT"), false),
            SysIconKind::Ext("txt".to_string())
        );
        assert_eq!(
            sys_icon_kind(Path::new("C:/b.txt"), false),
            SysIconKind::Ext("txt".to_string())
        );
        // 无扩展名 = 空串。
        assert_eq!(
            sys_icon_kind(Path::new("C:/README"), false),
            SysIconKind::Ext(String::new())
        );
    }

    fn key(name: &str) -> SysIconKey {
        SysIconKey {
            kind: SysIconKind::Ext(name.to_string()),
            large: false,
        }
    }

    #[test]
    fn store_evicts_oldest() {
        let mut store: IconStore<u32> = IconStore::new(2);
        store.insert(key("a"), 10);
        store.insert(key("b"), 20);
        assert_eq!(store.get(&key("a")), Some(&10));
        // 超容量：最旧的 a 被逐出。
        store.insert(key("c"), 30);
        assert_eq!(store.map.len(), 2);
        assert_eq!(store.get(&key("a")), None);
        assert_eq!(store.get(&key("b")), Some(&20));
        assert_eq!(store.get(&key("c")), Some(&30));
    }

    #[test]
    fn store_reinsert_survives_stale_order_entry() {
        let mut store: IconStore<u32> = IconStore::new(2);
        store.insert(key("a"), 10);
        store.insert(key("b"), 20);
        // a 重插：新 stamp，order 里留下旧序项。
        store.insert(key("a"), 11);
        // 触发逐出：旧序项 (a, stamp0) 弹出但现条目是 stamp2，不误删；
        // 真正最旧的有效条目是 b。
        store.insert(key("c"), 30);
        assert_eq!(store.get(&key("a")), Some(&11));
        assert_eq!(store.get(&key("c")), Some(&30));
        assert_eq!(store.get(&key("b")), None);
    }

    #[test]
    fn key_distinguishes_size_tier() {
        let mut store: IconStore<u32> = IconStore::new(4);
        let small = SysIconKey {
            kind: SysIconKind::Dir,
            large: false,
        };
        let large = SysIconKey {
            kind: SysIconKind::Dir,
            large: true,
        };
        store.insert(small.clone(), 16);
        store.insert(large.clone(), 32);
        assert_eq!(store.get(&small), Some(&16));
        assert_eq!(store.get(&large), Some(&32));
    }
}
