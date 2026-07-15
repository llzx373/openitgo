# 媒体播放器体验补齐设计（全宽进度条 / OSD / 静音 / 滚轮音量 / 输出设备选择 / 音量倍速记忆）

日期：2026-07-15
状态：已批准设计，待写实施计划

## 1. 背景与目标

媒体播放功能（`a53d1e2` 合并，`42e44a9` 修复渲染）已能完整播放视频/音频，
但与"简单但完整的播放器"相比缺一批基础体验：

1. 进度条是塞在一行里的窄 Slider，不是经典整宽长条；
2. 滚轮调音量、静音、跳转等操作没有屏幕反馈（全屏隐藏控制条时完全无感）；
3. 没有静音；
4. 滚轮在媒体视图无作用；
5. 音频输出设备不可选（跟随系统默认，实测会路由到 USB 声卡造成"无声"困惑）;
6. 音量/倍速每次打开都重置。

本设计覆盖前五项中与用户确认后的全部内容（第 6 项并入第 5 项的持久化）。

## 2. 已确认的决策

| 决策点 | 结论 |
|---|---|
| OSD 反馈 | 做简洁 OSD：画面右上角浮现文本约 1 秒，末尾淡出 |
| 进度条布局 | 两行式：第一行整宽长条，第二行时间 + 音量控件 |
| 输出设备选择入口 | 仅媒体播放工具栏下拉框，持久化到 Settings |
| 音量/倍速记忆粒度 | 全局记忆（Settings 两个字段） |
| 架构 | 保持现有分层：mpv 交互下沉 rust-reader-media，UI/状态机留 rust-reader-app，持久化进 rust-reader-storage |

非目标（YAGNI）：缓冲区间显示、按文件记忆、设置面板设备项、
输出设备热插拔自动刷新、进度条缩略图预览、OSD 动画以外的任何特效。

## 3. 架构总览

改动分布（无新增 crate / 模块）：

```
rust-reader-media/
  src/player.rs             + set_muted / audio_devices / set_audio_device
                            + observe "mute" 属性
  src/state.rs              PlayerState + muted: bool
  src/devices.rs（新）      RawAudioDevice + parse_audio_devices 纯函数

rust-reader-storage/src/    Settings + media_volume / media_speed /
                            media_audio_device（含 validate clamp）

rust-reader-app/
  src/views/media.rs        MediaView + Osd 状态机 + 滚轮累加器 +
                            toggle_mute / set_audio_device 等转发
  src/app.rs                render_media_seekbar 重写为两行；工具栏 +
                            设备下拉框；键盘 + M；OSD 渲染；打开媒体时
                            应用已存音量/倍速/设备；调整后写回 Settings
```

职责边界不变：rust-reader-media 不知道 egui；所有 mpv 属性/命令经
`MpvPlayer`；egui 侧只读 `PlayerState` 与调命令 API。

## 4. 全宽进度条（两行式）

`render_media_seekbar`（app.rs:746 附近）重写：

- **第一行（整宽长条）**：`egui::Slider::new(&mut ratio, 0.0..=1.0)`
  + `show_value(false)`，宽度 `ui.available_width()`。
  - 悬停：`response.hover_pos()` 换算 `ratio_at_pointer × duration_ms`，
    用 `response.on_hover_text()` 显示目标时间（`format_time_ms`）。
  - 跳转沿用现有逻辑：`changed()` 期间 `seek_to_ratio`（关键帧对齐），
    松手 `drag_stopped` 时精确 seek（media.rs 已有 exact 区分）。
  - 无时长（流/未知）时第一行显示禁用态占位。
- **第二行（信息行）**：左侧 `当前时间 / 总时长`（`--:--` 兜底）；
  右侧静音按钮 + 音量滑块（约 120px，`0..=100`）。
- 音量滑块从原第一行挪到第二行；`format_time_ms` 复用。

时间换算抽出纯函数 `hover_time_at(pos, rect, duration_ms) -> Option<u64>`
（None = 无时长），供单测。

## 5. OSD

`MediaView` 新增：

```rust
struct Osd { text: String, until: std::time::Instant }
const OSD_DURATION: Duration = Duration::from_millis(1000);
```

- `MediaView::osd(&mut self, text: impl Into<String>)`：设置文本与截止时间，
  并请求 egui 重绘。
- 触发点与文本：
  - 滚轮音量 / ↑↓ 键 / 拖音量滑块：`音量 75%`
  - 静音切换：`静音` / `取消静音`
  - ←/→/J/L 跳转：`-5s` / `+5s` / `-10s` / `+10s`
  - 倍速（1-4 键与工具栏按钮）：`1.5x`
  - 输出设备切换：`输出: <设备描述>`
- 渲染：在媒体 CentralPanel 内用 `egui::Area`（`Order::Foreground`）
  锚定面板 rect 右上角（留 12px 边距），半透明圆角底 + 文本；
  剩余不足 300ms 时按剩余比例淡出 alpha；到期不重绘。
  位置在 CentralPanel 内计算，天然避开顶部工具栏与底部进度条。
- 连续触发覆盖旧文本并重置计时。

状态机抽纯函数（`osd_alpha(now, until) -> f32`）供单测。

## 6. 静音

- rust-reader-media：
  - `PlayerState` + `muted: bool`（默认 false）；
  - `MpvPlayer::new` 增加 `mpv_observe_property(handle, 7, "mute", MPV_FORMAT_FLAG)`；
  - 事件循环 `7 =>` 分支写 `s.muted` 并触发 repaint（复用现有模式）；
  - `MpvPlayer::set_muted(bool)`（`set_property_string mute yes/no`）。
- app：
  - 底栏静音按钮：图标/文字随 `muted` 切换（静音时滑块 `enabled(false)` 灰显，
    不改动已存音量值）；
  - 键盘 `M`：`player.set_muted(!muted)`，OSD 反馈；
  - `MediaView::toggle_mute()` 转发。

## 7. 滚轮音量

- 在媒体 CentralPanel 区域捕获滚轮：`ui.input(|i| i.smooth_scroll_delta.y)`
  仅在 `View::Media` 且指针在 CentralPanel 内时消费。
- 累加器（MediaView 字段 `scroll_acc: f32`）：`acc += delta.y`，
  `steps = trunc(acc / 25.0)`，`acc -= steps * 25.0`，按 `steps`
  调 `adjust_volume(steps × 5.0)`（余数保留，反向滚动自然抵消，
  触控板平滑小步滚动不会一下跳爆）。阈值常量
  `SCROLL_VOLUME_STEP_PX: f32 = 25.0`。
- 每次有效调整触发 OSD（`音量 N%`）。
- 翻页类视图的滚轮逻辑不受影响（分支在 `View::Media` 内）。

累加器抽纯函数（`accumulate_scroll(acc, delta) -> (new_acc, steps)`）供单测。

## 8. 音频输出设备选择

- rust-reader-media：
  - `devices.rs`（新）：`RawAudioDevice { name: String, description: String }`
    + `parse_audio_devices(raw: Vec<RawAudioDevice>) -> Vec<AudioDevice>`
    （排序/去重等纯逻辑，供单测；与 tracks.rs 的 RawTrack/parse 分层一致）。
  - `MpvPlayer::audio_devices()`：`get_property("audio-device-list")`
    （`MPV_FORMAT_NODE`），FFI 转换函数仿照 `read_track_list`；
    失败/为空返回空 Vec。
  - `MpvPlayer::set_audio_device(name: &str)`：
    `set_property_string("audio-device", name)`；`"auto"` 表示跟随系统。
- app：
  - 媒体工具栏加 `egui::ComboBox`：首项"自动"，之后为设备描述
    （`description`，无则用 `name`）；当前值来自
    `Settings.media_audio_device`（空串 = auto）。
  - 进入媒体视图 / 打开文件时枚举一次（不做热插拔监听）。
  - 选择后：`set_audio_device` + 写回 Settings 保存 + OSD `输出: <描述>`。
  - 打开媒体应用已存设备时，若设备已不在列表中：回退 `"auto"`、
    更新 Settings，不报错（设备可能只是没插）。

## 9. 音量/倍速全局记忆

- rust-reader-storage：`Settings` 新增
  - `media_volume: f64`（默认 100.0，validate clamp 0..=100）
  - `media_speed: f64`（默认 1.0，validate clamp 0.1..=16，
    与 `MpvPlayer::set_speed` 范围一致）
  - `media_audio_device: String`（默认 `""` = auto，无需 clamp）
  校验失败沿用现有模式：clamp 后经 `error_message` 告知。
- app：
  - 打开媒体文件后（`poll_media_open` 成功路径）：依次应用
    `set_volume(settings.media_volume)`、`set_speed(settings.media_speed)`、
    `set_audio_device(...)`（非 auto 时）。
  - 用户调整音量/倍速/设备时写回 `self.settings` 并立即保存
    （复用现有 settings 保存调用点模式）。

## 10. 数据流

音量/静音/倍速（与现有属性链路一致，无新链路）：

```
UI 操作 → MediaView 转发 → MpvPlayer 命令 → mpv
→ observe_property 事件 → PlayerState 更新 + request_repaint → UI 刷新
```

OSD 不经过 mpv：命令发起侧同步设置，1 秒计时本地管理。

设备选择：UI ComboBox → `set_audio_device` + Settings 保存（即时生效，
无回读属性；mpv 不提供稳定的当前设备回读，以 Settings 为准）。

## 11. 错误处理

- `audio_devices()` 枚举失败或为空 → ComboBox 只显示"自动"，不报错。
- `set_audio_device` 失败 → `error_message` 提示，Settings 不写回。
- 已存设备不存在 → 静默回退 auto（见 §8）。
- mute/音量/倍速命令失败 → 与现有一致：命令层返回 `MediaError`，
  UI 层忽略或经 `error_message`；不阻塞播放。

## 12. 测试策略

rust-reader-media：
- `parse_audio_devices`：空列表、缺 description 字段回退 name、去重。
- （FFI 层 `read_audio_device_list` 不测，同 `read_track_list` 现状。）

rust-reader-storage：
- Settings validate：`media_volume` 越界 clamp、`media_speed` 越界 clamp、
  默认值。

rust-reader-app：
- `hover_time_at`：指针位置 → ratio → 毫秒换算、无时长 None。
- `accumulate_scroll`：累计 25px 出 1 步、反向滚动抵消累计值、步进后余数保留、平滑小步累加不跳变。
- `osd_alpha`：1 秒内 alpha=1、最后 300ms 线性淡出、过期 0。
- Settings 应用顺序（若可抽纯函数则测，否则留给手工验证）。

手工验证清单（写进实施计划）：
- 两行进度条整宽显示、悬停时间正确、拖动关键帧/松手精确；
- 滚轮音量 + OSD、M 静音 + OSD、滑块灰显；
- 设备下拉切换可听（本机多输出设备）、回退 auto；
- 重启应用后音量/倍速/设备保持；
- 全屏下 OSD 位置不与控制条重叠。

## 13. 文档更新

- README：媒体播放小节补充静音/滚轮音量/设备选择/记忆；快捷键表 + `M`。
- CHANGELOG：Unreleased Added 五条。
- AGENTS.md：Settings notable fields + `media_volume` / `media_speed` /
  `media_audio_device`；Media playback 要点 + OSD 与设备选择。

## 14. 里程碑拆分建议（供 writing-plans 参考）

1. rust-reader-media：mute（属性观察 + set_muted）+ devices 模块。
2. rust-reader-storage：三个 Settings 字段 + validate。
3. rust-reader-app：两行进度条 + 悬停时间。
4. rust-reader-app：OSD 状态机与渲染，接入音量/静音/跳转/倍速。
5. rust-reader-app：静音按钮 + M 键 + 滚轮音量。
6. rust-reader-app：设备下拉框 + 三个记忆字段的应用与写回。
7. 文档 + 手工验证。
