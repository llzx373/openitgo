# macOS Dock 拖入打开设计

## 目标

让 rustReader 在 macOS 上支持把漫画压缩包或文件夹拖到 Dock 图标打开：

- 无论应用是否已经运行，拖入后都能收到文件路径。
- 收到后自动导入书库并打开第一个可识别的漫画。
- 与现有“拖入窗口”和“菜单打开文件夹”行为保持一致。

## 用户行为

1. 用户把一个或多个文件/文件夹拖到 Dock 上的 rustReader 图标。
2. 如果应用未运行，系统启动应用并把路径通过 `NSApplicationDelegate` 的 `application:openFiles:` 传进来。
3. 如果应用已运行，系统直接把同一委托方法调进来。
4. 应用把文件加入书库；如果第一个拖入项本身就是漫画文件，则直接进入阅读器。

## 技术背景

- `eframe`/`winit` 目前**不会**自动处理 `application:openFile:` / `application:openFiles:` 事件（见 winit issue #1751）。
- 因此需要自己向 `NSApp.delegate` 注入这两个方法。
- 同时需要在 `Info.plist` 中声明支持的文档类型，否则打包成 `.app` 后系统不会允许拖到 Dock 上。

## 方案选择

采用 **方案 A：通过 objc 运行时给现有 `NSApplicationDelegate` 注入 `application:openFiles:` 方法**。

原因：

- 符合 macOS 原生事件分发路径。
- 应用未运行和已运行时都能收到。
- 实现相对集中，只新增一个 macOS-only 模块。

备选方案 B（Carbon Apple Event）因为 API 已废弃且没有现成 Rust 绑定，不作为首选。

## 架构

```text
+----------------------------------+
| macOS (Dock / Finder)            |
|  application:openFiles:          |
+-------------+--------------------+
              |
              v
+-------------+--------------------+
| platform::macos::dock_open       |
|  - install_dock_open_handler()   |
|  - DOCK_OPEN_QUEUE (Mutex)       |
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

## 新增模块

在 `rust-reader-app/src/platform/macos/` 下新增 `dock_open.rs`（或直接放在 `platform.rs` 的 `macos` 模块里）。

核心 API：

```rust
#[cfg(target_os = "macos")]
pub mod dock_open {
    use std::path::PathBuf;

    /// 安装 Dock/Finder 拖入打开处理器。应在 `eframe` 创建 event loop 之后、
    /// 主窗口事件循环开始之前调用一次。
    pub fn install_dock_open_handler();

    /// 取出当前累积的待打开路径。由 `ReaderApp::update` 每帧调用。
    pub fn take_dock_open_paths() -> Vec<PathBuf>;
}
```

### 实现要点

1. 使用 `cocoa` 和 `objc` crate 取得 `NSApplication::sharedApplication()` 的当前 `delegate`。
2. 通过 `object_getClass` 取得 delegate 的 `Class`。
3. 使用 `class_addMethod` 向该类添加 `application:openFiles:` 方法（selector `sel!(application:openFiles:)`）。
   - 如果 winit 未来自己实现了该方法，可以再改为 `method_exchangeImplementations` 做包装；目前 winit 未实现，所以直接添加即可。
4. 方法内部把 `NSArray<NSString>` 转换成 Rust `Vec<PathBuf>`，存入全局 `Mutex<Vec<PathBuf>>`。
5. 同样添加 `application:openFile:` 作为单文件兜底。

## 与现有代码集成

### 初始化

在 `rust-reader-app/src/main.rs` 的 `eframe::run_native` 创建闭包里调用：

```rust
eframe::run_native(
    "rustReader",
    options,
    Box::new(|cc| {
        #[cfg(target_os = "macos")]
        crate::platform::macos::dock_open::install_dock_open_handler();

        fonts::setup_fonts(&cc.egui_ctx);
        Ok(Box::new(ReaderApp::new(cc)))
    }),
)
```

此时 `eframe`/`winit` 已经创建好 `NSApplication` 并设置好 delegate，注入方法不会破坏现有事件循环。

### 每帧处理

在 `ReaderApp::update` 中，紧接 `handle_dropped_files` 之后：

```rust
self.handle_dropped_files(ctx);
for path in crate::platform::macos::dock_open::take_dock_open_paths() {
    self.add_folder_to_library(path.clone());
}
```

为了保持与窗口拖入行为一致，处理完后再尝试打开第一个路径：

```rust
let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
if let Some(first) = dock_paths.first() {
    if rust_reader_parser::parse(first).is_ok() {
        self.open_comic(first.clone());
    }
}
for path in dock_paths {
    self.add_folder_to_library(path);
}
```

如果第一个拖入项是文件夹，`add_folder_to_library` 会递归扫描并导入；此时保持留在书库界面，让用户看到导入结果。

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
- 从终端 `cargo run` 运行时，Dock 上的图标是原始二进制，不是 `.app` 包，系统通常不会接受 Dock 拖入；但代码层面的处理仍然保留，便于后续打包测试。

## 错误处理

- 如果路径无法解析成漫画，`add_folder_to_library` 已经在 UI 显示错误信息。
- 如果 delegate 为 nil 或注入失败，仅在日志中记录，不阻断程序启动。
- Objective-C 转换路径失败时，忽略该项。

## 依赖

在 `rust-reader-app/Cargo.toml` 的 macOS target 依赖中新增：

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
cocoa = "0.26"
```

（版本以 `cargo` 当前可解析为准。）

## 测试计划

1. 单元测试：新增一个 macOS-only 的转换测试，验证 `NSArray<NSString>` 到 `Vec<PathBuf>` 的辅助函数。
2. 集成测试：
   - 构建 `.app` 后把 `.zip`/`.cbz` 拖到 Dock 图标，确认应用启动并进入阅读器。
   - 应用已运行时拖入文件夹，确认书库新增条目。
   - 拖入多个文件时，确认第一个可解析项被打开，其余被导入。
3. 回归测试：运行 `cargo fmt/check/test/clippy --workspace`。

## 未涉及范围

- 不修改 Windows / Linux 的拖入窗口行为。
- 不实现自定义文件关联安装程序（仅提供 `Info.plist` 模板）。
- 不处理通过 `mailto:` 等非文件 URL 打开的场景。
