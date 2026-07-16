# Bug 分析与已知问题

## 现象

连续快速翻页时，页面加载越来越慢，最终⏳图标卡住不动，显示"加载中..."但永远不会完成。

## 修复状态

- [x] **Bug 1**：解码任务丢弃后回传错误结果，UI 可清除 pending 状态并 retry
- [x] **Bug 2**：`pending_pages` 增加超时机制，超时后允许重试
- [x] **问题 3**：`PageLoader` 改为多 IO 线程并发读取文件
- [x] **问题 4**：预加载冷却期从 300ms 缩短为 100ms
- [x] **问题 5**：缓存满时通过 `enforce_budget_with_protected` 淘汰旧页后继续预加载

> 注：本文档记录的是 2026-06-17 左右的 bug 快照。当前代码已重构多次，下面提到的文件行号已不再准确，请按函数名阅读。

## 根本原因：解码任务被静默丢弃 + UI 无重试机制（死锁）

### Bug 1：解码任务丢弃后永远不重试（P0，核心死锁）

**位置**：`rust-reader-app/src/loader.rs` 的 `process_io_request` + `rust-reader-app/src/views/reader.rs` 的 `request_page`

**触发路径**：

1. 用户快速翻页，每翻一页调用 `request_page()` → `loader.request_high()`，发送一个 `LoadRequest` 到 `high_sender`（容量 64）
2. IO 线程读取文件，产生 `DecodeJob` 发送到 `high_decode_sender`（容量 64）
3. 当翻页速度 > 解码速度时，decode 通道被填满
4. `process_io_request()` 中的 `try_send` 失败，解码任务被**静默丢弃**：

```rust
if sender.try_send(job).is_err() {
    timing::log(&format!("IO dropped decode job page {}...", req.page_index));
    // 关键：没有向 result_sender 发送错误结果！
}
```

5. UI 侧 `request_page()` 将页面加入 `pending_pages`（在 `request_high` 成功后）：

```rust
if reader.pending_pages.contains_key(&page_index) {
    return; // 页面被标记为"正在加载"，跳过重试
}
```

6. 由于没有 `LoadResult` 返回，`pending_pages` 中的页面**永远不会被清除**
7. **死锁**：UI 认为页面"正在加载"，实际任务已被丢弃，`LoadResult` 永远不会到来

### Bug 2：pending 状态无超时机制（P1）

**位置**：`rust-reader-app/src/views/reader.rs`

早期 `pending_pages` 是 `HashSet<usize>`，没有关联的超时时间。页面只有以下方式离开 pending 状态：
- `LoadResult` 到达（`update()` 中 `pending_pages.remove()`）
- 打开新漫画时 `bump_epoch()` 清空 `pending_pages`

如果 `LoadResult` 因任何原因未到达，页面会永久卡在 pending 状态。

### 问题 3：IO 单线程限制（P2）

**位置**：`rust-reader-app/src/loader.rs`

早期全局只有一个 IO 线程处理所有文件读取。虽然读取后立刻将解码任务派发给多线程 worker，但文件读取本身是串行的。快速翻页产生的并发文件读取请求都在排队。

### 问题 4：预加载冷却期过长（P3）

**位置**：`rust-reader-app/src/views/reader.rs`

早期：

```rust
const PRELOAD_COOLDOWN_AFTER_TURN: Duration = Duration::from_millis(300);
```

翻页后 300ms 内预加载完全停止。连续快速翻页（<300ms/次）时，预加载永远不会触发，所有页面都是冷加载。

### 问题 5：缓存满时预加载直接跳过（P3）

**位置**：`rust-reader-app/src/views/reader.rs`

早期逻辑：

```rust
if reader.cache.total_size_bytes() >= budget {
    return; // 缓存满 = 不预加载
}
```

快速翻页时 LRU 缓存被近期浏览的页面填满，预加载直接停止，而不是淘汰旧页释出空间后继续预取新页。

---

## 修复方向

1. **P0**：`process_io_request()` 在 `try_send` 失败时，应向 `result_sender` 回传错误结果，让 UI 清除 pending 状态
2. **P1**：为 `pending_pages` 添加超时机制（如超 5 秒未返回则移除并允许重试）；对在 `pending_pages` 中但超时的页面，`request_page()` 应强制重新发送
3. **P2**：允许并发文件读取（多 IO 线程 + 共享 raw-bytes cache）
4. **P3**：预加载冷却期恢复时，检查当前页是否已在缓存中，若不在则立即预加载；缓存满时应淘汰旧页腾出空间继续预加载，而非直接跳过

---

## 已知问题（2026-07-15 记录）

以下来自 mpv-under-egui 分支（视频层下沉）开发期间的冒烟取证，均为**基线既有问题**（经 A/B 对照或代码路径分析确认与该分支改动无关）。均未修复，优先级为自评。

### 问题 A：mpv 启动偶发 wedge（DR image 分配卡死）（P1）

- **现象**：启动播放偶发卡死，栈显示卡在 DR image 分配。
- **证据**：Task 4 新二进制与基线二进制 A/B 对照均复现 → 基线既有，与视频层下沉无关。
- **修复线索**：暂无，需进一步诊断（疑与 mpv/CAOpenGLLayer 初始化时序有关）；复现不稳定。
- **发现经过**：Task 5 真机冒烟期间。

### 问题 B：dock-open 队列空闲滞留（P2）

- **现象**：app 空闲时通过 Finder/Dock 投递的文件不立即打开，滞留到下次重绘（动一下窗口/鼠标才打开）。
- **根因线索**：`application:openURLs:` 等回调把文件放入队列，但队列只在 egui `update()` 中排空；egui 空闲时不重绘，`update()` 不被调用。
- **修复线索**：收到 openURLs 时主动唤醒 UI（`egui::Context::request_repaint()` 或经 winit event loop 的 user event）。
- **发现经过**：Task 3 真机冒烟用 dock-open 通道投递测试视频时发现。

### 问题 C：EOF 后空闲 OSD 滞留（P3）

- **现象**：播放结束（EOF）后，画面右上角 OSD 文字滞留超过设计的约 1s，直到下次重绘才消失。
- **根因线索**：OSD 清除（`tick_osd`）由 egui 重绘驱动；播放中 mpv 事件泵持续 `request_repaint()`，EOF 后事件停止、egui 空闲，清理逻辑得不到执行。属事件驱动模型的既有行为。
- **修复线索**：EOF/close 时主动 `clear_osd` 或补一次 `request_repaint()`（一行级修复）。
- **发现经过**：Task 4 真机冒烟；与 media-player-ux 终审遗留 Minor"close() 未清 osd（≤1s 自愈）"同族。
