# macOS Dock 拖入打开设计

## 目标

让 rustReader 在 macOS 上支持把漫画压缩包或文件夹拖到 Dock 图标打开：

- 无论应用是否已经运行，拖入后都能收到文件路径。
- 收到后自动导入书库并打开第一个可识别的漫画。
- 与现有“拖入窗口”和“菜单打开文件夹”行为保持一致。

## 用户行为

1. 用户把一个或多个文件/文件夹拖到 Dock 上的 rustReader 图标。
2. 如果应用未运行，系统启动应用并把路径通过 `NSApplicationDelegate` 的 `application:openURLs:` 传进来。
3. 如果应用已运行，系统直接把同一委托方法调进来。
4. 应用把文件加入书库；如果第一个拖入项本身就是漫画文件，则直接进入阅读器。

## 技术背景

- `eframe`/`winit` 目前**不会**自动处理 `application:openFile:` / `application:openFiles:` / `application:openURLs:` 事件（见 winit issue #1751）。
- 更关键的是：应用**未运行时**，系统会在 `NSApplication` 设置 delegate 后的极早期就把打开事件派发出去。如果在 `eframe::run_native` 的 `AppCreator` 闭包里才注入 delegate 方法，事件已经错过，系统会弹出 “cannot open files in the Comic Archive format” 错误。
- 因此需要在 delegate 被设置的那一瞬间就向其类注入打开文件方法。

## 方案选择

采用 **方案 A+：通过 objc 运行时 swizzle `-[NSApplication setDelegate:]`，在 winit 设置 delegate 时动态注入打开文件方法**。

原因：

- 不依赖 winit delegate 类的加载时机或类名。
- 在 delegate 对象被赋值的同时完成方法注入，保证事件分发前 delegate 已经具备响应能力。
- 同时注入 `application:openURLs:`（macOS 10.13+ 推荐）、`application:openFiles:` 和 `application:openFile:` 三个方法，覆盖各种派发路径。

## 架构

```text
+----------------------------------+
| macOS (Dock / Finder)            |
|  application:openURLs:           |
+-------------+--------------------+
              |
              v
+-------------+--------------------+
| NSApplication.setDelegate:       |
|  rust_reader_set_delegate()      |
|  -> add_open_methods_to_class()  |
+-------------+--------------------+
              |
              v
+-------------+--------------------+
| platform::macos::dock_open       |
|  - OPEN_QUEUE (Mutex)            |
+-------------+--------------------+
              |
              v
+-------------+--------------------+
| ReaderApp::update()              |
|  - take_dock_open_paths()        |
|  - add_folder_to_library(path)   |
|  - open_comic(path) (如果可解析) |
+----------------------------------+
```

## 新增/修改模块

位置：`rust-reader-app/src/platform.rs` 的 `#[cfg(target_os = "macos")] pub mod macos` 内。

核心 API：

```rust
#[cfg(target_os = "macos")]
pub mod dock_open {
    use std::path::PathBuf;

    /// 尽早 swizzle NSApplication.setDelegate:，使 delegate 被设置时立即
    /// 注入 application:openURLs:/openFiles:/openFile: 方法。
    /// 应在 main() 中、eframe::run_native 之前调用。
    pub fn install_dock_open_handler_early();

    /// 向当前 NSApplication delegate 注入打开文件方法（兜底/热更新场景）。
    pub fn install_dock_open_handler();

    /// 取出当前累积的待打开路径。由 ReaderApp::update 每帧调用。
    pub fn take_dock_open_paths() -> Vec<PathBuf>;
}
```

### 实现要点

1. **Swizzle `-[NSApplication setDelegate:]`**
   - 向 `NSApplication` 类添加 `rustReader_setDelegate:` 方法。
   - 使用 `method_exchangeImplementations` 交换 `setDelegate:` 与 `rustReader_setDelegate:` 的实现。
   - 在 `rust_reader_set_delegate` 中先调用原始 `setDelegate:`，再通过 `object_getClass` 取得新 delegate 的真实类，调用 `add_open_methods_to_class` 注入方法。

2. **注入 delegate 方法**
   - `application:openURLs:`（`v@:@@`）: 接收 `NSArray<NSURL *>`，提取 `-[NSURL path]` 后入队。
   - `application:openFiles:`（`v@:@@`）: 接收 `NSArray<NSString *>`，提取文件系统路径后入队。
   - `application:openFile:`（`c@:@@` 返回 `BOOL`）: 单文件兜底，返回 `YES` 表示已处理。
   - 注入前通过 `class_getInstanceMethod` 检查是否已存在，避免重复添加或打印误报警告。

3. **路径队列与消费**
   - 使用 `Mutex<Vec<PathBuf>>` 作为 `OPEN_QUEUE`。
   - delegate 回调将路径推入队列；`ReaderApp::update` 每帧调用 `take_dock_open_paths()` 取出并处理。

## 与现有代码集成

### 初始化

在 `rust-reader-app/src/main.rs` 的 `main()` 最开头调用：

```rust
fn main() -> eframe::Result<()> {
    #[cfg(target_os = "macos")]
    crate::platform::macos::dock_open::install_dock_open_handler_early();

    // ... viewport / options ...

    eframe::run_native(
        "rustReader",
        options,
        Box::new(|cc| {
            fonts::setup_fonts(&cc.egui_ctx);

            #[cfg(target_os = "macos")]
            crate::platform::macos::dock_open::install_dock_open_handler();

            Ok(Box::new(ReaderApp::new(cc)))
        }),
    )
}
```

`install_dock_open_handler_early()` 负责在 delegate 被设置的第一时间注入方法；`install_dock_open_handler()` 作为兜底，在 eframe 创建好应用后再检查一次当前 delegate。

### 每帧处理

在 `ReaderApp::update` 中，紧接 `handle_dropped_files` 之后：

```rust
self.handle_dropped_files(ctx);

#[cfg(target_os = "macos")]
{
    let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
    if !dock_paths.is_empty() {
        self.handle_open_paths(dock_paths);
    }
}
```

`handle_open_paths` 会：

- 把所有路径加入书库（文件会被解析，文件夹会被递归扫描）。
- 如果第一个路径是可解析的漫画文件，直接进入阅读器。

## Info.plist 声明

更新 `assets/icon/Info.plist.template`，声明 rustReader 可以打开以下类型：

- 压缩包：`public.zip-archive`、`com.rarlab.rar-archive`
- 扩展名：`cbz`、`zip`、`rar`、`cbr`、`pdf`
- 文件夹：`public.folder`

示例片段：

```xml
<key>CFBundleDocumentTypes</key>
<array>
    <dict>
        <key>CFBundleTypeName</key>
        <string>Comic Archive</string>
        <key>CFBundleTypeRole</key>
        <string>Viewer</string>
        <key>LSHandlerRank</key>
        <string>Alternate</string>
        <key>LSItemContentTypes</key>
        <array>
            <string>public.zip-archive</string>
            <string>com.rarlab.rar-archive</string>
            <string>public.folder</string>
        </array>
        <key>CFBundleTypeExtensions</key>
        <array>
            <string>cbz</string>
            <string>zip</string>
            <string>rar</string>
            <string>cbr</string>
            <string>pdf</string>
        </array>
    </dict>
</array>
```

说明：

- 只有在打包成 `.app` 并使用该 `Info.plist` 时，系统才会允许把对应文件拖到 Dock 图标上。
- 打包脚本 `scripts/package-macos.sh` 会基于该模板生成最终的 `Info.plist` 并签名。

## 错误处理

- 如果路径无法解析成漫画，`add_folder_to_library` 已经在 UI 显示错误信息。
- 如果 delegate 为 nil 或注入失败，仅在日志中记录，不阻断程序启动。
- Objective-C 转换路径失败时，忽略该项。
- 队列中去重：同一事件可能同时触发多个 delegate 方法，入队时检查是否已存在，避免重复打开。

## 依赖

在 `rust-reader-app/Cargo.toml` 的 macOS target 依赖中已有：

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
```

## 测试计划

1. 集成测试：
   - 构建 `.app` 后把 `.zip`/`.cbz` 拖到 Dock 图标，确认应用**未运行**时启动并进入阅读器。
   - 应用已运行时拖入文件夹，确认书库新增条目。
   - 拖入多个文件时，确认第一个可解析项被打开，其余被导入。
2. 回归测试：运行 `cargo fmt/check/test/clippy --workspace`。

## 未涉及范围

- 不修改 Windows / Linux 的拖入窗口行为。
- 不实现自定义文件关联安装程序（仅提供 `Info.plist` 模板）。
- 不处理通过 `mailto:` 等非文件 URL 打开的场景。
