# macOS Dock 拖入打开实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 macOS 用户可以把漫画压缩包或文件夹拖到 Dock 图标上，无论应用是否已运行，都能自动导入书库并打开第一个可识别的漫画。

**Architecture:** 在 `eframe` 创建 `NSApplication` 和 delegate 之后，通过 `objc` 运行时向当前 delegate 注入 `application:openFiles:` / `application:openFile:` 方法；收到的路径写入线程安全队列，由 `ReaderApp::update` 每帧取出并复用现有的 `add_folder_to_library` / `open_comic` 逻辑处理。同时更新 `Info.plist` 声明支持的文档类型，使打包后的 `.app` 能被系统识别为可打开这些文件。

**Tech Stack:** Rust, `eframe`/`egui`, `winit`, `objc` crate, macOS AppKit/Cocoa.

---

## 文件结构

- `rust-reader-app/Cargo.toml`：新增 macOS target 依赖 `objc`。
- `rust-reader-app/src/platform.rs`：在现有的 `#[cfg(target_os = "macos")] pub mod macos` 内新增 `pub mod dock_open` 子模块，包含 handler 安装与路径队列。
- `rust-reader-app/src/main.rs`：在 `eframe::run_native` 的 `AppCreator` 闭包中调用安装函数。
- `rust-reader-app/src/app.rs`：在 `ReaderApp::update` 中每帧读取队列并处理。
- `assets/icon/Info.plist.template`：添加 `CFBundleDocumentTypes`，注册支持的压缩包/文件夹类型。

---

### Task 1: 添加 macOS 依赖

**Files:**
- Modify: `rust-reader-app/Cargo.toml`

- [ ] **Step 1: 在 `[target.'cfg(target_os = "macos")'.dependencies]` 下添加 `objc`**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
```

- [ ] **Step 2: 验证依赖能解析**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 依赖下载成功，检查通过（此时没有新代码调用 objc，所以不会报错）。

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/Cargo.toml
git commit -m "deps: add objc for macOS Dock open-file handling"
```

---

### Task 2: 实现 macOS Dock/Finder 打开事件处理器

**Files:**
- Modify: `rust-reader-app/src/platform.rs`

- [ ] **Step 1: 在 `macos` 模块末尾、`#[cfg(not(target_os = "macos"))]` 模块之前，新增 `pub mod dock_open`**

```rust
    pub mod dock_open {
        use std::ffi::{c_char, CStr};
        use std::path::PathBuf;
        use std::sync::Mutex;

        use objc::runtime::{
            class_addMethod, object_getClass, BOOL, Class, Object, Sel, YES,
        };
        use objc::{class, msg_send, sel, sel_impl};

        static OPEN_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

        #[link(name = "Cocoa", kind = "framework")]
        extern "C" {}

        /// 向当前 NSApplication delegate 注入 `application:openFiles:` 与
        /// `application:openFile:`，用于接收 Dock / Finder 拖入或双击打开的文件。
        pub fn install_dock_open_handler() {
            unsafe {
                let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
                if app.is_null() {
                    return;
                }
                let delegate: *mut Object = msg_send![app, delegate];
                if delegate.is_null() {
                    return;
                }
                let cls = object_getClass(delegate) as *mut Class;
                if cls.is_null() {
                    return;
                }

                let open_files_types = b"v@:@:@\0".as_ptr() as *const c_char;
                let _ = class_addMethod(
                    cls,
                    sel!(application:openFiles:),
                    std::mem::transmute(
                        open_files_callback
                            as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
                    ),
                    open_files_types,
                );

                let open_file_types = b"c@:@:@\0".as_ptr() as *const c_char;
                let _ = class_addMethod(
                    cls,
                    sel!(application:openFile:),
                    std::mem::transmute(
                        open_file_callback
                            as extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> BOOL,
                    ),
                    open_file_types,
                );
            }
        }

        /// 取出并清空当前累积的待打开路径。应在主线程每帧调用一次。
        pub fn take_dock_open_paths() -> Vec<PathBuf> {
            let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        }

        extern "C" fn open_files_callback(
            _this: &Object,
            _sel: Sel,
            _app: *mut Object,
            files: *mut Object,
        ) {
            if files.is_null() {
                return;
            }
            let paths = unsafe { collect_paths_from_array(files) };
            if let Ok(mut guard) = OPEN_QUEUE.lock() {
                guard.extend(paths);
            }
        }

        extern "C" fn open_file_callback(
            _this: &Object,
            _sel: Sel,
            _app: *mut Object,
            file: *mut Object,
        ) -> BOOL {
            if file.is_null() {
                return unsafe { objc::runtime::NO };
            }
            if let Some(path) = unsafe { nsstring_to_path(file) } {
                if let Ok(mut guard) = OPEN_QUEUE.lock() {
                    guard.push(path);
                }
                YES
            } else {
                unsafe { objc::runtime::NO }
            }
        }

        unsafe fn collect_paths_from_array(files: *mut Object) -> Vec<PathBuf> {
            let count: usize = msg_send![files, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                let item: *mut Object = msg_send![files, objectAtIndex:i];
                if item.is_null() {
                    continue;
                }
                if let Some(path) = nsstring_to_path(item) {
                    paths.push(path);
                }
            }
            paths
        }

        unsafe fn nsstring_to_path(s: *mut Object) -> Option<PathBuf> {
            let utf8: *const c_char = msg_send![s, UTF8String];
            if utf8.is_null() {
                return None;
            }
            CStr::from_ptr(utf8)
                .to_str()
                .ok()
                .map(PathBuf::from)
        }
    }
```

- [ ] **Step 2: 运行 macOS 编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 编译通过，无 objc 类型/链接错误。

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/platform.rs
git commit -m "feat(macos): add Dock/Finder open-file handler via objc runtime"
```

---

### Task 3: 在应用启动时安装 handler

**Files:**
- Modify: `rust-reader-app/src/main.rs`

- [ ] **Step 1: 在 `eframe::run_native` 的 `AppCreator` 闭包中，在 `fonts::setup_fonts` 之后、`ReaderApp::new` 之前调用安装函数**

```rust
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
```

- [ ] **Step 2: 运行编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 通过。

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/main.rs
git commit -m "feat(macos): install Dock open-file handler on startup"
```

---

### Task 4: 在主循环中消费收到的路径

**Files:**
- Modify: `rust-reader-app/src/app.rs`（`ReaderApp::update` 方法）

- [ ] **Step 1: 在 `handle_dropped_files(ctx);` 之后、`poll_opener(ctx);` 之前，插入 Dock 路径处理**

```rust
        self.handle_dropped_files(ctx);

        #[cfg(target_os = "macos")]
        {
            let dock_paths = crate::platform::macos::dock_open::take_dock_open_paths();
            if let Some(first) = dock_paths.first() {
                if rust_reader_parser::parse(first).is_ok() {
                    self.open_comic(first.clone());
                }
            }
            for path in dock_paths {
                self.add_folder_to_library(path);
            }
        }

        self.poll_opener(ctx);
```

说明：
- 第一个路径如果本身就是可解析的漫画文件，直接进入阅读器。
- 所有路径都会调用 `add_folder_to_library`：文件会被解析并加入书库；文件夹会被递归扫描导入。

- [ ] **Step 2: 运行编译检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check -p rust-reader-app
```

Expected: 通过。

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(macos): consume Dock-dropped files in update loop"
```

---

### Task 5: 更新 Info.plist 以声明可打开的文件类型

**Files:**
- Modify: `assets/icon/Info.plist.template`

- [ ] **Step 1: 在 `<dict>` 内添加 `CFBundleDocumentTypes`**

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

- [ ] **Step 2: 验证 plist 格式**

Run:
```bash
plutil -lint /Users/liu/srcs/rustReader/assets/icon/Info.plist.template
```

Expected: `OK`。

- [ ] **Step 3: 提交**

```bash
git add assets/icon/Info.plist.template
git commit -m "chore(macos): register supported document types in Info.plist"
```

---

### Task 6: 全量验证与测试

- [ ] **Step 1: 格式化代码**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo fmt --all
```

Expected: 成功，无输出。

- [ ] **Step 2: 运行 workspace 检查**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo check --workspace
```

Expected: 无错误。

- [ ] **Step 3: 运行测试**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo test --workspace
```

Expected: 全部通过。

- [ ] **Step 4: 运行 clippy**

Run:
```bash
cd /Users/liu/srcs/rustReader && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 无警告。

- [ ] **Step 5: 手动集成测试（macOS 真机）**

1. 构建发布版：
   ```bash
   cargo build --release -p rust-reader-app
   ```
2. 把生成的二进制打包成 `.app`（可使用现有 `assets/icon/Info.plist.template` + 图标资源），或直接用已存在的打包脚本。
3. 把一个 `.zip`/`.cbz` 文件拖到 Dock 上的 rustReader 图标。
4. 验证：
   - 应用未运行时：应用启动并进入阅读器。
   - 应用已运行时：文件被打开或导入书库。
5. 把一个包含图片的文件夹拖到 Dock 图标，验证文件夹被递归扫描导入书库。

- [ ] **Step 6: 提交验证后的最终变更（如有）**

```bash
git add -A
git commit -m "style: cargo fmt after macOS Dock drop-to-open implementation"
```

---

## 实现后检查清单

- [ ] `cargo fmt --all` 通过。
- [ ] `cargo check --workspace` 通过。
- [ ] `cargo test --workspace` 通过。
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- [ ] 在 macOS 上验证 `.app` 打包后 Dock 拖入可打开压缩包。
- [ ] 在 macOS 上验证应用运行时 Dock 拖入可导入文件夹。
