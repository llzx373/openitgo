# 媒体播放功能设计（视频 + 音频）

日期：2026-07-14
状态：已批准设计，待写实施计划

## 1. 背景与目标

rustReader 目前支持漫画（ZIP/CBZ、RAR/CBR、PDF、图片文件夹）和电子书
（EPUB/TXT/MOBI/AZW3/Markdown）。本需求让应用能够**打开并播放视频和音频文件**，
播放体验与漫画/电子书一致：从库打开、显示封面、记住播放进度、工具栏/控制条自动隐藏。

## 2. 已确认的决策

| 决策点 | 结论 |
|---|---|
| 媒体类型 | 视频 + 音频 |
| 播放引擎 | 内嵌 libmpv（`libmpv-sys` 直接封装薄安全层） |
| libmpv 来源 | 随 `.app` 打包进 `Contents/Frameworks/`，开箱即用 |
| 渲染集成 | 原生子 `NSView`（`CAOpenGLLayer`）叠加在 egui 窗口上，同电子书 webview 模式 |
| 首版范围 | 完整集成：入库、封面、进度恢复、历史记录 |
| 播放控制 | 全量：播放/暂停、进度拖拽、音量、全屏、倍速、字幕轨/音轨切换 |
| 平台 | 仅 macOS（与现状一致） |

非目标（YAGNI）：播放列表/文件夹剧集连播、音量持久化、在线流、跨平台、
投屏、均衡器、音频可视化。

## 3. 架构总览

新增一个 crate + app 内两个模块：

```
rust-reader-media/          新 crate，隔离所有 unsafe FFI
  src/lib.rs
  src/player.rs             MpvPlayer：句柄、命令、事件泵、属性观察
  src/render.rs             RenderContext：mpv_render_context（OpenGL）
  src/types.rs              轨道列表、播放状态等纯数据结构
  src/time.rs               毫秒 ↔ mm:ss / h:mm:ss 格式化

rust-reader-app/src/platform/macos/mpv_view.rs
                            子 NSView + CAOpenGLLayer 创建、set_bounds、显隐
rust-reader-app/src/views/media.rs
                            MediaView：传输状态、工具栏、seek 条、快捷键
```

职责边界：

- `rust-reader-media` 不知道 egui 的存在；通过回调/共享状态向外通信。
  纯逻辑（轨道解析、时间格式化、状态映射）可单测；FFI 部分靠编译期保证。
- `mpv_view.rs` 只管原生窗口层，不管播放逻辑。
- `media.rs` 只管 UI 与状态同步，通过 `MpvPlayer` 的命令 API 控制播放。

## 4. rust-reader-media crate

### 4.1 MpvPlayer

- `MpvPlayer::new() -> Result<Self, MediaError>`：`mpv_create`，设置
  `vo=libmpv`、`keep-open=yes`、`input-default-bindings=no`（快捷键由 egui 统一处理）。
- 命令 API（每个都是一行 `mpv_command`/`mpv_set_property` 封装）：
  - `load_file(path)`、`play()`、`pause()`、`toggle_pause()`
  - `seek_absolute(ms)`、`set_volume(0..=100)`、`set_speed(0.25..=4.0)`
  - `set_subtitle_track(id)` / `set_audio_track(id)`（`id=None` 表示关闭字幕）
  - `screenshot_to_file(path)`：封面用，见 §9
- 事件泵：单独线程 `mpv_wait_event` 循环，处理：
  - `MPV_EVENT_PROPERTY_CHANGE`：更新 `Arc<Mutex<PlayerState>>`，
    然后调用构造时注入的 `repaint: Box<dyn Fn() + Send>` 回调
    （app 侧传 `egui::Context::request_repaint` 的闭包——与电子书状态栏
    修复（commit b071a7b）同一手法，控制条进度实时刷新）。
  - `MPV_EVENT_FILE_LOADED` / `MPV_EVENT_END_FILE` / `MPV_EVENT_SHUTDOWN`：
    更新状态机的载入/结束/错误标记。
  - `MPV_EVENT_LOG_MESSAGE`（warning 以上）：`eprintln!` 透传。
- 属性观察（`mpv_observe_property`）：`time-pos`、`duration`、`pause`、
  `volume`、`speed`、`track-list`、`media-title`。
- `PlayerState`：`position_ms`、`duration_ms: Option<u64>`（就绪前为 None，
  UI 显示 `--:--`）、`paused`、`volume`、`speed`、`tracks: Vec<TrackInfo>`、
  `current_sub: Option<i64>`、`current_audio: Option<i64>`、`ended`、`error: Option<String>`。
- `TrackInfo`：`id: i64`、`kind: Video|Audio|Sub`、`title: Option<String>`、
  `lang: Option<String>`、`codec: Option<String>`、`selected: bool`。
  从 `track-list` 的 node 结构解析，解析函数纯逻辑、可单测。

### 4.2 RenderContext

- `RenderContext::new(&player, get_proc_address) -> Result<Self, MediaError>`：
  `mpv_render_context_create`，参数 `MPV_RENDER_PARAM_API_TYPE =
  MPV_RENDER_API_TYPE_OPENGL`、`MPV_RENDER_PARAM_OPENGL_INIT_PARAMS`。
- `set_update_callback`：mpv 需要画新一帧时调注入的回调（app 侧标记
  `CAOpenGLLayer` 需要显示）。这条渲染链路完全在原生层，不经过 egui。
- `render(width, height, flip_y)`：在 `displayLayer` 回调里调用
  `mpv_render_context_render`，FBO 由 `CAOpenGLLayer` 提供。
- `report_swap()`：`mpv_render_context_report_swap`。

### 4.3 错误类型

`MediaError`：`Init(String)`、`Command { code: i32, what: String }`、
`Load(String)`。`Display` 输出中文友好文案，供 app 的 `error_message` 通道使用。

## 5. macOS 子视图渲染集成（mpv_view.rs）

- `MpvNativeView::new(parent: &HasWindowHandle+HasDisplayHandle, bounds: Rect,
  player: &MpvPlayer) -> Result<Self, String>`：
  - 创建 `NSView`，设置 `wantsLayer`，挂一个自定义 `CAOpenGLLayer` 子类
    （`isAsynchronous = true`）。
  - 父窗口通过 `raw_window_handle` 取 `NSWindow`，`contentView.addSubview`。
  - `RenderContext::set_update_callback` 里 `[layer setNeedsDisplay]`；
    `displayLayer` 里绑定 layer FBO、`render()`、`report_swap()`。
- `set_bounds(Rect)` / `set_visible(bool)` / `remove()`：与电子书
  `EbookRenderer::set_bounds` 同模式；egui 每帧用中央面板矩形同步
  （`MediaView::update_bounds`，照搬 `render_ebook` 的结构）。
- 关闭时先 `mpv_render_context_free` 再释放 mpv 句柄，避免野回调。

## 6. MediaView UI 与控制（views/media.rs）

- `MediaView { open: Option<OpenMedia> }`；
  `OpenMedia { player, native_view, state: Arc<Mutex<PlayerState>>, title, path }`。
- 顶部工具栏（复用 `should_show_bar` 自动隐藏）：
  `书架 | 播放/暂停 | -10s | +10s | 倍速 0.5x/1x/1.5x/2x 循环 | 字幕轨下拉 | 音轨下拉 | 全屏`
- 底部控制条：seek 滑条（`duration` 就绪才可拖）、`当前时间 / 总时长`、音量滑条。
- 快捷键（沿用 `shortcuts.rs` 机制）：
  `Space` 播放/暂停、`←/→` ±5s、`↑/↓` 音量 ±5、`f` 全屏、`j/l` ±10s、
  `1/2/3/4` 倍速、`v` 字幕开关、`Esc` 退出全屏或返回书架。
- 音频文件：mpv 无画面输出，子视图区域由 egui 在中央面板画占位
  （封面图或纯黑 + 曲名），控制条不变。检测方式：`track-list` 里没有
  被选中的 video 轨即按音频处理。
- 状态轮询：每帧 `MediaView::sync_state()` 读共享状态（非阻塞 `lock()`，
  读不到就沿用上一帧）。重绘由 §4.1 的 `request_repaint` 回调驱动。

## 7. 打开分发与格式

`rust-reader-app/src/app.rs`：

```rust
fn is_media_file(path: &Path) -> bool        // 扩展名判断
fn media_kind_for_path(path: &Path) -> MediaKind // Video | Audio
fn open_media(&mut self, path: PathBuf)      // 直接打开，无需 AsyncOpener（mpv 载入很快）
```

`open_path` 分发顺序：`is_ebook_file` → `is_media_file` → 漫画。

视频扩展名：`mp4 m4v mkv webm avi mov wmv flv ts m2ts mpg mpeg 3gp`
音频扩展名：`mp3 flac aac m4a ogg oga opus wav aiff ape wma`

（mp4/m4a 等容器既可能纯音频也可能含视频：先按视频打开，`track-list`
就绪后按 §6 的规则切换为音频占位显示。）

## 8. 库集成

`rust-reader-storage/src/models.rs`：

```rust
pub enum MediaType {
    #[default]
    Comic,
    Ebook,
    Video,
    Audio,
}
```

- `serde(rename_all = "snake_case")` 下新变体序列化为 `video` / `audio`；
  缺失字段默认 `Comic` 不变，老 JSON 完全兼容。
- `media_type_for_path` 增加媒体分支；库扫描/筛选/排序逻辑
  （`filter_by_media_type`、`library_sort`）自然生效，无需改动。
- 库卡片：视频/音频条目显示封面（§9），标题用文件名（去扩展名）。
  `media-title` 就绪后可在详情处展示，首版不做。

## 9. 封面生成

- 视频：打开时若 `covers/` 中尚无封面，用 `screenshot_to_file` 跳到
  `duration * 10%` 处截一帧，缩放后存入 `covers/`（复用现有封面目录、
  命名规则与"缺失即补"逻辑；媒体条目的 id 同样由
  `rust_reader_parser::stable_comic_id` 从路径生成）。放后台线程
  （`cover_loader` 同款），不阻塞打开；库里先显示占位，生成完自动刷新。
  入库扫描不主动生成封面，只在打开/库内展示缺封面时触发。
- 音频：mpv 读到内嵌封面（`albumart`）就 `screenshot-to-file` 导出当前帧；
  没有就用构建时生成的静态占位（音符图标）。
- 时长未知（流/损坏文件）时退化为第 1 秒截图；再失败则占位。

## 10. 进度与历史

- 复用 `HistoryEntry`：`char_offset = Some(position_ms)`，`page_index = 0`。
  与电子书复用 `char_offset` 存字符偏移同一套路，不动存储结构。
- 触发点：`record_media_history()` 在暂停、切出媒体视图、关闭文件、
  退出应用时调用（照搬 `record_ebook_history` 的挂接点）。
- 打开时：查到历史且 `position_ms < duration_ms - 3000` 才恢复
  （快到结尾不恢复，避免"打开即结束"）。

## 11. 重绘驱动（两条独立链路）

| 链路 | 触发 | 机制 |
|---|---|---|
| mpv 画面 | 解码出新帧 | `update_callback` → `setNeedsDisplay` → `displayLayer` 内 `mpv_render_context_render` |
| egui 控制条 | 属性变化（time-pos 等） | 事件泵线程 → 共享状态 → `egui::Context::request_repaint()` |

第二条是电子书状态栏延迟 bug（commit b071a7b）的同款修复，设计阶段
就纳入，不再遗留"慢一拍"问题。

## 12. 错误处理

- libmpv 加载失败 / `mpv_create` 失败：`open_media` 返回错误 →
  `error_message = "无法初始化播放器：…"`，回库视图。
- `loadfile` 失败（`MPV_EVENT_END_FILE` 带 error）：`"无法播放该文件：…"`。
- 截图失败：静默，用占位封面。
- 所有用户可见文案中文（遵循 AGENTS.md）。

## 13. libmpv 打包（scripts/package-macos.sh 扩展）

1. 开发/构建机前置：`brew install mpv`（README、AGENTS.md 记录）。
2. 打包脚本新增 `bundle_mpv()`：
   - `brew --prefix mpv` 定位 `libmpv.2.dylib`，拷入 `Contents/Frameworks/`；
   - `otool -L` 递归收集 `/opt/homebrew` 下的依赖 dylib（libav*、libass、
     libplacebo、liblua、libuchardet 等，预计 30~60 个）一并拷入；
   - `install_name_tool -change` 把所有 `/opt/homebrew/...` 引用改写为
     `@rpath/<name>`；`install_name_tool -id @rpath/<name>` 修正自身；
   - 可执行文件加 `install_name_tool -add_rpath @executable_path/../Frameworks`；
   - 对 Frameworks 内全部 dylib `codesign --force --sign -`（跟随现有
     签名策略），保证 Gatekeeper 不拒载。
3. 验证：`DYLD_PRINT_LIBRARIES=1` 启动打包产物，确认不从 `/opt/homebrew`
   加载任何库；包体预计 +50~80MB。

## 14. 依赖

- workspace 成员新增 `rust-reader-media`。
- `rust-reader-media`：`libmpv-sys`（pkg-config 找库）、`thiserror`。
  安全封装在本 crate 内手写薄层，不依赖 `libmpv2`，避免被上游绑死。
- `rust-reader-app`：macOS 侧 `objc2` / `objc2-app-kit` / `objc2-foundation`
  / `objc2-core-graphics`（与 wry 现有 objc 生态版本对齐，按 Cargo.lock
  里已有版本选）。

## 15. 测试策略

沿用"纯逻辑可测、IO/FFI 隔离"的现有风格。

纯逻辑单测：
- 扩展名分类：`is_media_file` / `media_kind_for_path` / `media_type_for_path`
  （含大小写、无扩展名、mp4 按视频打开）。
- 时间格式化：ms → `mm:ss` / `h:mm:ss`，边界（0、59s、60s、1h、负值钳制）。
- 轨道列表解析：`track-list` node → `Vec<TrackInfo>`，含无字幕轨、
  多音轨、选中项识别。
- `MediaType` 序列化兼容：新变体 round-trip；缺失字段默认 `Comic`。
- 进度历史往返：`char_offset` 毫秒写入/读出；接近结尾不恢复的判断。
- seek 钳制：`[0, duration]` 边界。

手动验证清单（FFI/原生层，写进 PR 描述）：
- 打开 mp4/mkv/webm/mp3/flac 各一个；含内嵌字幕与多音轨的 mkv。
- 播放/暂停、seek、倍速、切字幕轨/音轨、音量、全屏进出。
- 进度恢复：播到一半退出重开；播到结尾重开不恢复。
- 封面：视频 10% 截图、音频内嵌封面、无封面占位。
- 打包产物在 `DYLD_PRINT_LIBRARIES` 下无 `/opt/homebrew` 加载。

流水线照旧全绿：`cargo fmt --all`、`cargo check --workspace`、
`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`。

## 16. 文档更新

- README：支持格式清单、`brew install mpv` 前置依赖。
- AGENTS.md：`rust-reader-media` 职责、mpv 渲染/重绘双链路、打包流程。
- CHANGELOG：新增媒体播放条目。

## 17. 里程碑拆分建议（供 writing-plans 参考）

1. `rust-reader-media` crate + `MpvPlayer` 命令/事件/状态（可在无 UI 下单测纯逻辑）。
2. `mpv_view.rs` 原生渲染 + `MediaView` 最小打开播放（能放 mp4）。
3. 控制 UI 全量（工具栏、seek 条、快捷键、倍速、轨道切换）。
4. 库集成：`MediaType` 变体、分发、封面生成、进度恢复与历史。
5. 打包脚本 `bundle_mpv()` + 文档更新 + 全量验证。
