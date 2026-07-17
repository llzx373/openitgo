# Bug 分析与已知问题

> 归档说明：本文档原路径为 `docs/bug.md`，其中记录的问题均已修复，2026-07-17 归档留存于此。

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

**位置**：`openitgo-app/src/loader.rs` 的 `process_io_request` + `openitgo-app/src/views/reader.rs` 的 `request_page`

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

**位置**：`openitgo-app/src/views/reader.rs`

早期 `pending_pages` 是 `HashSet<usize>`，没有关联的超时时间。页面只有以下方式离开 pending 状态：
- `LoadResult` 到达（`update()` 中 `pending_pages.remove()`）
- 打开新漫画时 `bump_epoch()` 清空 `pending_pages`

如果 `LoadResult` 因任何原因未到达，页面会永久卡在 pending 状态。

### 问题 3：IO 单线程限制（P2）

**位置**：`openitgo-app/src/loader.rs`

早期全局只有一个 IO 线程处理所有文件读取。虽然读取后立刻将解码任务派发给多线程 worker，但文件读取本身是串行的。快速翻页产生的并发文件读取请求都在排队。

### 问题 4：预加载冷却期过长（P3）

**位置**：`openitgo-app/src/views/reader.rs`

早期：

```rust
const PRELOAD_COOLDOWN_AFTER_TURN: Duration = Duration::from_millis(300);
```

翻页后 300ms 内预加载完全停止。连续快速翻页（<300ms/次）时，预加载永远不会触发，所有页面都是冷加载。

### 问题 5：缓存满时预加载直接跳过（P3）

**位置**：`openitgo-app/src/views/reader.rs`

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

以下来自 mpv-under-egui 分支（视频层下沉）开发期间的冒烟取证，均为**基线既有问题**（经 A/B 对照或代码路径分析确认与该分支改动无关）。优先级为自评。三个问题均已于 2026-07-16 修复。

### 问题 A：mpv 启动偶发 wedge（DR image 分配卡死）（P1）—— 已修复（2026-07-16）

- **现象**：启动播放偶发卡死，栈显示卡在 DR image 分配。同一根因在播放中拖入新视频时**必现**（2026-07-16 用户报告，实机复现：CPU 停滞、窗口永久冻结）。
- **根因**（sample 抓栈 + mpv 日志互证，`target/wedgetest/` 留存取证）：四方循环等待——
  1. 主线程在 `poll_media_open` 里发**同步** mpv 调用（`refresh_audio_devices` 的 `mpv_get_property` 等），阻塞在 `mp_dispatch_lock`，等 core 播放线程处理；
  2. core 线程在 `write_video` 里等解码线程产出第一帧；
  3. 解码线程在 `get_buffer2 → dr_helper_get_image → mp_dispatch_run` 里等 **DR 图像分配**，该请求只能由调用 `mpv_render_context_update()` 的线程（即主线程 `rsDriveUpdate`）服务；
  4. 主线程阻塞在第 1 步，永远跑不到 `rsDriveUpdate` → 闭环死锁。
  启动时该竞态窗口窄（mpv 常以 "DR path suspected slow, disabling" 降级躲过），表现为偶发；拖入新文件时 `refresh_audio_devices`/`apply_startup_settings` 一串同步调用与新文件首帧 DR 分配重叠，窗口变宽，几乎必现。
- **修复**：UI 线程的 mpv 调用全部改为异步 API——`mpv_command_async`（loadfile/seek/pause/stop）、`mpv_set_property_async`（音量/倍速/静音/轨道/输出设备）、`mpv_get_property_async`（`audio-device-list`，回复由事件泵解析进 `PlayerState::audio_devices`）。主线程永不阻塞在 core 分派上，DR 分配随时可被服务，环路拆除。启动音频设备改为延迟应用：枚举回复到达后经 `pending_startup_device` + `startup_device_target` 校验，设备已拔出则回退 auto 并由 `take_startup_device_invalid` 通知 app 清除设置。
- **验证**：原卡死场景（A 播放中投递 B）连续 2 次不再冻结，CPU 持续上升，日志确认 B 的 DR 分配完成、`VO: [libmpv] 1280x720` 正常起播。
- **发现经过**：Task 5 真机冒烟期间（启动偶发）；2026-07-16 拖放场景稳定复现后确诊。

### 问题 B：dock-open 队列空闲滞留（P2）—— 已修复（2026-07-16）

- **现象**：app 空闲时通过 Finder/Dock 投递的文件不立即打开，滞留到下次重绘（动一下窗口/鼠标才打开）。
- **根因**：`application:openURLs:` 等回调把文件放入 `OPEN_QUEUE`，但队列只在 egui `update()` 中排空（`app.rs` 的 `take_dock_open_paths`）；egui 空闲时 winit 事件循环睡眠、不重绘，`update()` 不被调用。
- **修复**：`dock_open` 模块新增 `set_wake_context`（app 创建时注册 `egui::Context`），三个回调收文件后经 `enqueue_paths` 统一入队并 `request_repaint()` 唤醒事件循环（`openitgo-app/src/platform.rs`、`main.rs`）。
- **验证**：A/B 对照冒烟。基线（/Applications 旧包）：回调收到文件后 CPU 持平（6s 内 0:00.33→0:00.43），文件滞留，bug 复现；新二进制：投递后 1s 内媒体开始解码（5s 内 CPU 0:00.92→0:02.36），无需任何输入事件。

### 问题 C：EOF 后空闲 OSD 滞留（P3）—— 已修复（2026-07-16）

- **现象**：播放结束（EOF）后，画面右上角 OSD 文字滞留超过设计的约 1s，直到下次重绘才消失。
- **根因**（比原记录更深一层）：`show_osd` 本就调用了 `request_repaint_after(1s)`，但 egui 0.29 对非零延迟会**减去预测帧时长**（`context.rs` 中 `delay.saturating_sub(predicted_frame_time)`，约 16.7ms）且**只触发一帧**——到期帧总是在 `until` 之前约 17ms 触发，此时 `tick_osd` 的 `now >= until` 判断不成立、不清除，之后不再有任何帧被预约。播放中 mpv 事件泵的持续 `request_repaint()` 掩盖了这一点；EOF 后事件停止、egui 空闲，OSD 便滞留到下次用户输入。
- **修复**：`tick_osd(ctx)` 在 OSD 未到期时按剩余时间重新 `request_repaint_after`（收敛一两帧后即清除）；同时 `MediaView::close()` 清除 `self.osd`，杜绝残留文本漏进下次打开的媒体（修复终审遗留的"close() 未清 osd"同族问题）。
- **验证**：新增单元测试 `tick_osd_keeps_unexpired_osd_and_clears_expired`、`close_clears_osd_state`；早触发机制经 egui 0.29.1 源码确认。
