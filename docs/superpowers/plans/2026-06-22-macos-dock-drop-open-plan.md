# macOS Dock 拖入打开实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 macOS 用户可以把漫画压缩包或文件夹拖到 Dock 图标上，无论应用是否已运行，都能自动导入书库并打开第一个可识别的漫画。

**Architecture:** 通过 objc 运行时 **swizzle `-[NSApplication setDelegate:]`**，在 winit 设置 `NSApplicationDelegate` 的瞬间就向其实际类注入 `application:openURLs:` / `application:openFiles:` / `application:openFile:` 方法；收到的路径写入线程安全队列，由 `ReaderApp::update` 每帧取出并复用现有的 `handle_open_paths` 逻辑处理。同时更新 `Info.plist` 声明支持的文档类型，使打包后的 `.app` 能被系统识别为可打开这些文件。

**Tech Stack:** Rust, `eframe`/`egui`, `winit`, `objc` crate, macOS AppKit/Cocoa.

---

## 文件结构

- `rust-reader-app/Cargo.toml`：macOS target 依赖 `objc`（已存在）。
- `rust-reader-app/src/platform.rs`：在现有的 `#[cfg(target_os = "macos")] pub mod macos` 内新增/更新 `pub mod dock_open` 子模块，包含 swizzle、handler 安装与路径队列。
- `rust-reader-app/src/main.rs`：在 `main()` 最开头调用 `install_dock_open_handler_early()`；在 `eframe::run_native` 的 `AppCreator` 闭包中保留 `install_dock_open_handler()` 作为兜底。
- `rust-reader-app/src/app.rs`：在 `ReaderApp::update` 中每帧读取队列并通过 `handle_open_paths` 处理。
- `assets/icon/Info.plist.template`：添加 `CFBundleDocumentTypes`，注册支持的压缩包/文件夹类型。

---

### Task 1: 确认 macOS 依赖

**Files:**
- 已存在: `rust-reader-app/Cargo.toml`

- [x] **Step 1: 确认 `[target.'cfg(target_os = "macos")'.dependencies]` 下已有 `objc`**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
```

- [x] **Step 2: 验证依赖能解析**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 检查通过。

---

### Task 2: 实现 macOS Dock/Finder 打开事件处理器

**Files:**
- Modify: `rust-reader-app/src/platform.rs`

- [x] **Step 1: 在 `macos` 模块末尾、`#[cfg(not(target_os = "macos"))]` 模块之前，新增/更新 `pub mod dock_open`**

核心结构：

```rust
    pub mod dock_open {
        use std::ffi::{c_char, CStr};
        use std::path::PathBuf;
        use std::sync::Mutex;

        use objc::runtime::{
            class_addMethod, class_getInstanceMethod, class_getName,
            method_exchangeImplementations, object_getClass, Class, Object, Sel, BOOL, YES,
        };
        use objc::{class, msg_send, sel, sel_impl};

        static OPEN_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

        #[link(name = "Cocoa", kind = "framework")]
        extern "C" {}

        /// 尽早 swizzle NSApplication.setDelegate:，使 winit 设置 delegate 时
        /// 立即注入 application:openURLs:/openFiles:/openFile:。
        pub fn install_dock_open_handler_early() {
            unsafe { swizzle_nsapplication_set_delegate() };
        }

        /// 向当前 NSApplication delegate 注入打开文件方法（兜底）。
        pub fn install_dock_open_handler() {
            // 取得当前 NSApplication delegate，调用 add_open_methods_to_class。
        }

        unsafe fn swizzle_nsapplication_set_delegate() { /* ... */ }
        extern "C" fn rust_reader_set_delegate(this: &Object, _sel: Sel, delegate: *mut Object) { /* ... */ }
        unsafe fn add_open_methods_to_class(cls: *mut Class) { /* ... */ }
        extern "C" fn open_urls_callback(...) { /* 入队 */ }
        extern "C" fn open_files_callback(...) { /* 入队 */ }
        extern "C" fn open_file_callback(...) -> BOOL { /* 入队，返回 YES */ }
        pub fn take_dock_open_paths() -> Vec<PathBuf>;
    }
```

实现要点：
- 使用 `method_exchangeImplementations` 交换 `-[NSApplication setDelegate:]`。
- 在 `rust_reader_set_delegate` 中先调用原始 setter，再向 delegate 类添加方法。
- 同时注入 `application:openURLs:`、`application:openFiles:`、`application:openFile:`。
- 路径解析失败时忽略，delegate 为 nil 时跳过。

- [x] **Step 2: 运行 macOS 编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 编译通过，无 objc 类型/链接错误。

- [x] **Step 3: 提交**

```bash
git add rust-reader-app/src/platform.rs
git commit -m "feat(macos): swizzle NSApplication setDelegate: to inject Dock open-file handlers"
```

---

### Task 3: 在应用启动时安装 handler

**Files:**
- Modify: `rust-reader-app/src/main.rs`

- [x] **Step 1: 在 `main()` 最开头调用 `install_dock_open_handler_early()`，闭包中保留兜底安装**

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

说明：
- `install_dock_open_handler_early()` 必须在 `eframe::run_native` 之前调用，才能赶上 winit 设置 delegate 的瞬间。
- 闭包中的 `install_dock_open_handler()` 用于非标准启动路径或 delegate 被重新设置时的兜底。

- [x] **Step 2: 运行编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 通过。

- [x] **Step 3: 提交**

```bash
git add rust-reader-app/src/main.rs
git commit -m "feat(macos): install Dock open-file handler before eframe startup"
```

---

### Task 4: 在主循环中消费收到的路径

**Files:**
- Modify: `rust-reader-app/src/app.rs`（`ReaderApp::update` 方法）

- [x] **Step 1: 在 `handle_dropped_files(ctx);` 之后插入 Dock 路径处理**

```rust
        self.handle_dropped_files(ctx);

        #[cfg(target_os = "macos")]
        {
            let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
            if !dock_paths.is_empty() {
                self.handle_open_paths(dock_paths);
            }
        }

        self.poll_opener(ctx);
```

说明：
- `handle_open_paths` 会先把所有路径加入书库，再尝试打开第一个可解析的漫画文件。
- 与窗口拖入行为保持一致。

- [x] **Step 2: 运行编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 通过。

- [x] **Step 3: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(macos): consume Dock-dropped files in update loop"
```

---

### Task 5: 更新 Info.plist 以声明可打开的文件类型

**Files:**
- Modify: `assets/icon/Info.plist.template`

- [x] **Step 1: 在 `<dict>` 内添加 `CFBundleDocumentTypes`**

最终 `Info.plist.template` 应类似：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIconName</key>
    <string>AppIcon</string>
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
</dict>
</plist>
```

- [x] **Step 2: 验证 plist 格式**

Run:
```bash
plutil -lint /Users/liu/srcs/rustReader/assets/icon/Info.plist.template
```

Expected: `OK`。

- [x] **Step 3: 提交**

```bash
git add assets/icon/Info.plist.template
git commit -m "chore(macos): register supported document types in Info.plist"
```

---

### Task 6: 全量验证与测试

- [x] **Step 1: 格式化代码**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo fmt --all
```

Expected: 成功，无输出。

- [x] **Step 2: 运行 workspace 检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check --workspace
```

Expected: 无错误。

- [x] **Step 3: 运行测试**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo test --workspace
```

Expected: 全部通过。

- [x] **Step 4: 运行 clippy**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 无警告。

- [x] **Step 5: 手动集成测试（macOS 真机）**

1. 构建并打包：
   ```bash
   bash scripts/package-macos.sh
   ```
2. 把生成的 `target/release/bundle/rustReader.app` 复制到 `/Applications/`。
3. 验证：
   - 应用**未运行**时，把 `.zip`/`.cbz` 拖到 Dock 图标：应用启动并进入阅读器。
   - 应用**已运行**时，把 `.zip`/文件夹拖到 Dock 图标：文件被打开或导入书库。
   - 普通启动（不带文件）：应用正常启动，无错误弹窗。

- [x] **Step 6: 提交验证后的最终变更（如有）**

```bash
git add -A
git commit -m "style: cargo fmt after macOS Dock drop-to-open implementation"
```

---

## 实现后检查清单

- [x] `cargo fmt --all` 通过。
- [x] `cargo check --workspace` 通过。
- [x] `cargo test --workspace` 通过。
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- [x] 在 macOS 上验证 `.app` 打包后 Dock 拖入可打开压缩包（应用未运行）。
- [x] 在 macOS 上验证应用运行时 Dock 拖入可导入文件夹。
- [x] 在 macOS 上验证普通启动（不带文件）正常。
