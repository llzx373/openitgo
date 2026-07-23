//! Platform-specific helpers.

#[cfg(target_os = "macos")]
pub mod macos {
    pub mod mpv_view;

    use crate::loader::{dynamic_to_loaded_image, LoadedImage, MAX_IMAGE_DIMENSION};
    use crate::timing;
    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::context::CGContext;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::image::{CGImage, CGImageAlphaInfo};
    use core_graphics::sys;
    use egui::{Color32, ColorImage};
    use foreign_types::ForeignType;
    use std::os::raw::c_void;

    #[repr(C)]
    struct CGImageSource(c_void);
    type CGImageSourceRef = *mut CGImageSource;

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageSourceCreateWithData(
            data: core_foundation::data::CFDataRef,
            options: core_foundation::dictionary::CFDictionaryRef,
        ) -> CGImageSourceRef;
        fn CGImageSourceCreateImageAtIndex(
            source: CGImageSourceRef,
            index: usize,
            options: core_foundation::dictionary::CFDictionaryRef,
        ) -> sys::CGImageRef;
        fn CGImageSourceCreateThumbnailAtIndex(
            source: CGImageSourceRef,
            index: usize,
            options: core_foundation::dictionary::CFDictionaryRef,
        ) -> sys::CGImageRef;
    }

    struct ImageSource(CGImageSourceRef);

    impl ImageSource {
        unsafe fn from_bytes(bytes: &[u8]) -> Option<Self> {
            let cf_data = CFData::from_buffer(bytes);
            let source =
                CGImageSourceCreateWithData(cf_data.as_concrete_TypeRef(), std::ptr::null());
            if source.is_null() {
                None
            } else {
                Some(Self(source))
            }
        }

        unsafe fn create_image_at_index(&self, index: usize) -> Option<CGImage> {
            let image_ref = CGImageSourceCreateImageAtIndex(self.0, index, std::ptr::null());
            if image_ref.is_null() {
                None
            } else {
                Some(CGImage::from_ptr(image_ref))
            }
        }
    }

    impl Drop for ImageSource {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0 as *const c_void) }
        }
    }

    /// Decode image bytes using macOS ImageIO / Core Graphics.
    ///
    /// Returns `Ok(None)` if ImageIO cannot recognize the data, allowing the
    /// caller to fall back to the pure-Rust `image` crate.
    pub fn decode_image_bytes(bytes: &[u8], compress: bool) -> Result<Option<LoadedImage>, String> {
        timing::log("platform.macos: trying ImageIO decode");

        unsafe {
            let source = ImageSource::from_bytes(bytes)
                .ok_or_else(|| "ImageIO could not create image source".to_string())?;
            let cg_image = source
                .create_image_at_index(0)
                .ok_or_else(|| "ImageIO could not create image".to_string())?;

            let width = cg_image.width();
            let height = cg_image.height();
            if width == 0 || height == 0 {
                return Ok(None);
            }

            // If the image exceeds our maximum dimension, ask ImageIO to produce
            // a downsampled thumbnail directly. This avoids allocating a huge
            // bitmap and then doing a second CPU resize.
            let max_dim = width.max(height);
            if max_dim > MAX_IMAGE_DIMENSION as usize {
                return decode_thumbnail(bytes, compress);
            }

            decode_full_image(&cg_image, compress)
        }
    }

    unsafe fn decode_full_image(
        cg_image: &CGImage,
        compress: bool,
    ) -> Result<Option<LoadedImage>, String> {
        let width = cg_image.width();
        let height = cg_image.height();
        let bytes_per_row = width * 4;
        let mut pixel_data = vec![0u8; height * bytes_per_row];

        render_into_buffer(cg_image, width, height, bytes_per_row, &mut pixel_data)?;

        if compress {
            // Keep the existing image-crate path so we can reuse the DXT5 compressor.
            Ok(Some(dynamic_to_loaded_image(
                image::DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(width as u32, height as u32, pixel_data)
                        .ok_or_else(|| "invalid RGBA buffer size".to_string())?,
                ),
                compress,
            )?))
        } else {
            Ok(Some(LoadedImage::Color(color_image_from_rgba(
                width,
                height,
                &pixel_data,
            ))))
        }
    }

    unsafe fn decode_thumbnail(
        bytes: &[u8],
        compress: bool,
    ) -> Result<Option<LoadedImage>, String> {
        let cf_data = CFData::from_buffer(bytes);

        let source = CGImageSourceCreateWithData(cf_data.as_concrete_TypeRef(), std::ptr::null());
        if source.is_null() {
            return Err("ImageIO could not create source for thumbnail".to_string());
        }
        let _source_guard = ImageSource(source);

        let cg_image = create_thumbnail_image(source, MAX_IMAGE_DIMENSION as usize)
            .ok_or_else(|| "ImageIO could not create thumbnail".to_string())?;

        let width = cg_image.width();
        let height = cg_image.height();
        let bytes_per_row = width * 4;
        let mut pixel_data = vec![0u8; height * bytes_per_row];

        render_into_buffer(&cg_image, width, height, bytes_per_row, &mut pixel_data)?;

        if compress {
            Ok(Some(dynamic_to_loaded_image(
                image::DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(width as u32, height as u32, pixel_data)
                        .ok_or_else(|| "invalid RGBA buffer size".to_string())?,
                ),
                compress,
            )?))
        } else {
            Ok(Some(LoadedImage::Color(color_image_from_rgba(
                width,
                height,
                &pixel_data,
            ))))
        }
    }

    /// Decode a small (256px on the long edge) thumbnail directly via ImageIO.
    /// Uses `CGImageSourceCreateThumbnailAtIndex` so ImageIO itself can downsample
    /// without decoding the full-resolution image first.
    pub fn decode_thumbnail_bytes(bytes: &[u8]) -> Result<Option<ColorImage>, String> {
        use crate::loader::THUMBNAIL_MAX_DIMENSION;
        timing::log("platform.macos: trying ImageIO thumbnail decode");

        unsafe {
            let source = ImageSource::from_bytes(bytes)
                .ok_or_else(|| "ImageIO could not create image source".to_string())?;

            let cg_image = create_thumbnail_image(source.0, THUMBNAIL_MAX_DIMENSION as usize)
                .ok_or_else(|| "ImageIO could not create thumbnail".to_string())?;
            let width = cg_image.width();
            let height = cg_image.height();
            let bytes_per_row = width * 4;
            let mut pixel_data = vec![0u8; height * bytes_per_row];
            render_into_buffer(&cg_image, width, height, bytes_per_row, &mut pixel_data)?;
            timing::log(&format!(
                "platform.macos: thumbnail decode done [{}x{}]",
                width, height
            ));
            Ok(Some(color_image_from_rgba(width, height, &pixel_data)))
        }
    }

    /// Create a thumbnail (or GPU-sized downsample) from the **full** image.
    ///
    /// Must use `kCGImageSourceCreateThumbnailFromImageAlways`, not
    /// `…IfAbsent`. Many JPEG/HEIC files ship a tiny EXIF/HEIF embedded
    /// thumbnail (often ~160×100); with `IfAbsent`, ImageIO returns that
    /// embedded thumb regardless of `max_dim`, which the oversized-page
    /// decode path then treats as the "full" page — postage-stamp rendering
    /// and wrong fit zoom. `Always` forces a fresh downsample from the
    /// primary image to `max_dim`. `WithTransform` applies EXIF orientation.
    unsafe fn create_thumbnail_image(source: CGImageSourceRef, max_dim: usize) -> Option<CGImage> {
        let max_size_key = CFString::from_static_string("kCGImageSourceThumbnailMaxPixelSize");
        let create_always_key =
            CFString::from_static_string("kCGImageSourceCreateThumbnailFromImageAlways");
        let with_transform_key =
            CFString::from_static_string("kCGImageSourceCreateThumbnailWithTransform");
        let should_cache_key = CFString::from_static_string("kCGImageSourceShouldCache");

        let max_size = CFNumber::from(max_dim as i64);
        let always = CFNumber::from(1i32);
        let with_transform = CFNumber::from(1i32);
        let zero = CFNumber::from(0i32);

        let options = CFDictionary::from_CFType_pairs(&[
            (max_size_key, max_size),
            (create_always_key, always),
            (with_transform_key, with_transform),
            (should_cache_key, zero),
        ]);

        let image_ref =
            CGImageSourceCreateThumbnailAtIndex(source, 0, options.as_concrete_TypeRef());
        if image_ref.is_null() {
            None
        } else {
            Some(CGImage::from_ptr(image_ref))
        }
    }

    unsafe fn render_into_buffer(
        cg_image: &CGImage,
        width: usize,
        height: usize,
        bytes_per_row: usize,
        pixel_data: &mut [u8],
    ) -> Result<(), String> {
        let color_space = CGColorSpace::create_device_rgb();
        // CGBitmapContext does not support non-premultiplied alpha directly, so
        // we draw with premultiplied RGBA and then unpremultiply below.
        let bitmap_info = CGImageAlphaInfo::CGImageAlphaPremultipliedLast as u32;

        let context = CGContext::create_bitmap_context(
            Some(pixel_data.as_mut_ptr() as *mut c_void),
            width,
            height,
            8,
            bytes_per_row,
            &color_space,
            bitmap_info,
        );

        let rect = CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(width as f64, height as f64),
        );
        context.draw_image(rect, cg_image);
        unpremultiply_rgba(pixel_data);
        Ok(())
    }

    fn color_image_from_rgba(width: usize, height: usize, pixels: &[u8]) -> ColorImage {
        let size = [width, height];
        let expected = width * height * 4;
        if pixels.len() != expected {
            // Defensive: fall back to the crate helper, which validates dimensions.
            return ColorImage::from_rgba_unmultiplied(size, pixels);
        }
        let mut out = Vec::with_capacity(width * height);
        for chunk in pixels.chunks_exact(4) {
            out.push(Color32::from_rgba_unmultiplied(
                chunk[0], chunk[1], chunk[2], chunk[3],
            ));
        }
        ColorImage {
            size,
            source_size: egui::Vec2::new(width as f32, height as f32),
            pixels: out,
        }
    }

    fn unpremultiply_rgba(pixels: &mut [u8]) {
        for chunk in pixels.chunks_exact_mut(4) {
            let a = chunk[3];
            if a == 0 || a == 255 {
                continue;
            }
            chunk[0] = ((chunk[0] as u16 * 255) / a as u16) as u8;
            chunk[1] = ((chunk[1] as u16 * 255) / a as u16) as u8;
            chunk[2] = ((chunk[2] as u16 * 255) / a as u16) as u8;
        }
    }

    pub mod dock_open {
        use std::ffi::{c_char, CStr};
        use std::path::PathBuf;
        use std::sync::Mutex;

        use objc2::ffi::class_addMethod;
        use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
        use objc2::{class, msg_send, sel};

        static OPEN_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

        fn dock_log(msg: &str) {
            use std::io::Write;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/openitgo-dock.log")
                .and_then(|mut f| writeln!(f, "[{}] {}", now, msg));
        }

        #[link(name = "Cocoa", kind = "framework")]
        extern "C" {}

        /// 尽早通过 swizzle `-[NSApplication setDelegate:]`，
        /// 在 winit 设置 delegate 的瞬间就向其实际类注入打开文件方法。
        ///
        /// 在 `eframe::run_native` 之前调用，这样无论 winit 何时创建 delegate 对象，
        /// 文件打开事件在分发前 delegate 都已经具备响应能力。
        pub fn install_dock_open_handler_early() {
            // SAFETY: 仅交换 `NSApplication` 的 `setDelegate:` 实现，并在设置 delegate
            // 后向其类注入 `application:openFiles:` 等方法，不破坏 AppKit 原有行为。
            unsafe { swizzle_nsapplication_set_delegate() };
        }

        /// 向当前 NSApplication delegate 注入 `application:openFiles:` 与
        /// `application:openFile:`，作为 Apple Event 处理器的补充。
        ///
        /// `application:openFile(s):` 由系统在主线程调用，因此使用普通 `Mutex`
        /// 即可保证线程安全，无需额外同步。
        pub fn install_dock_open_handler() {
            // SAFETY: 所有 Objective-C 消息发送都在主线程执行；`NSApplication` 为
            // AppKit 单例，其 delegate 与 class 在本次运行时有效。
            unsafe {
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                if app.is_null() {
                    eprintln!("warning: install_dock_open_handler: NSApplication is null");
                    return;
                }
                let delegate: *mut AnyObject = msg_send![app, delegate];
                if delegate.is_null() {
                    eprintln!("warning: install_dock_open_handler: NSApplication delegate is null");
                    return;
                }
                let cls = (&*delegate).class();
                add_open_methods_to_class(cls);
            }
        }

        /// Swizzle `-[NSApplication setDelegate:]` so that whenever a delegate is
        /// assigned we immediately inject `application:openFiles:` and friends into
        /// the delegate's class.
        ///
        /// # Safety
        /// Must be called on the main thread before `NSApplication` finishes launching.
        unsafe fn swizzle_nsapplication_set_delegate() {
            let ns_app_cls = match AnyClass::get(c"NSApplication") {
                Some(cls) => cls,
                None => {
                    eprintln!(
                        "warning: swizzle_nsapplication_set_delegate: NSApplication not found"
                    );
                    return;
                }
            };

            let original_sel = sel!(setDelegate:);
            let swizzled_sel = sel!(openItGo_setDelegate:);

            // Add our custom method to NSApplication; if it already exists, log and bail.
            let added = class_addMethod(
                ns_app_cls as *const AnyClass as *mut AnyClass,
                swizzled_sel,
                std::mem::transmute::<extern "C" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                    openitgo_set_delegate,
                ),
                c"v@:@".as_ptr() as *const c_char,
            );
            if !added.as_bool() {
                dock_log("dock_open: setDelegate: swizzle method already exists");
                return;
            }

            let Some(original_method) = ns_app_cls.instance_method(original_sel) else {
                eprintln!("warning: swizzle_nsapplication_set_delegate: setDelegate: not found");
                return;
            };
            let Some(swizzled_method) = ns_app_cls.instance_method(swizzled_sel) else {
                eprintln!(
                    "warning: swizzle_nsapplication_set_delegate: openItGo_setDelegate: not found"
                );
                return;
            };

            // SAFETY: 两个 Method 均指向已存在的方法；仅做实现交换，不改变方法数量。
            unsafe {
                original_method.exchange_implementation(swizzled_method);
            }
            dock_log("dock_open: swizzled NSApplication setDelegate:");
        }

        // SAFETY: 该函数作为 Objective-C method 被 `NSApplication` 调用；
        // `delegate` 为新的 `NSApplicationDelegate` 实例。
        extern "C" fn openitgo_set_delegate(this: &AnyObject, _sel: Sel, delegate: *mut AnyObject) {
            // 通过交换后的 selector 调用原始实现。
            let _: () = unsafe { msg_send![this, openItGo_setDelegate: delegate] };
            if !delegate.is_null() {
                // SAFETY: `class()` 返回 delegate 的真实类；向其注入方法。
                let cls = unsafe { (&*delegate).class() };
                unsafe { add_open_methods_to_class(cls) };
            }
        }

        /// 向指定 Objective-C 类添加 `application:openFiles:` 与
        /// `application:openFile:` 方法。
        ///
        /// # Safety
        /// `cls` 必须是有效的、非系统的 AppKit delegate 类。
        unsafe fn add_open_methods_to_class(cls: &AnyClass) {
            // SAFETY: 只修改应用自身的 delegate class，绝不修改系统类（NS 前缀）。
            // 若类名为空或以 "NS" 开头，则跳过注入并告警。
            let name = cls.name();
            if let Ok(name) = name.to_str() {
                if name.is_empty() {
                    eprintln!("warning: add_open_methods_to_class: delegate class name is empty");
                    return;
                }
                if name.starts_with("NS") {
                    eprintln!(
                        "warning: add_open_methods_to_class: refusing to inject into system class {}",
                        name
                    );
                    return;
                }
            }

            let class_name = name.to_string_lossy();

            // `application:openFiles:`（旧式路径数组）。
            add_delegate_method_if_missing(
                cls,
                sel!(application:openFiles:),
                // SAFETY: 目标回调签名与 `extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject)`
                // 完全一致，且为 AppKit 要求的 cdecl/Objective-C method 调用约定。
                std::mem::transmute::<
                    extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject),
                    Imp,
                >(open_files_callback),
                c"v@:@:@".as_ptr() as *const c_char,
                "application:openFiles:",
                &class_name,
            );

            // `application:openFile:`（旧式单文件，返回 BOOL）。
            add_delegate_method_if_missing(
                cls,
                sel!(application:openFile:),
                // SAFETY: 目标回调签名与 `extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool`
                // 完全一致，且为 AppKit 要求的 cdecl/Objective-C method 调用约定。
                std::mem::transmute::<
                    extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool,
                    Imp,
                >(open_file_callback),
                c"c@:@:@".as_ptr() as *const c_char,
                "application:openFile:",
                &class_name,
            );

            // macOS 10.13+ 推荐使用 `application:openURLs:` 接收文件/文件夹。
            add_delegate_method_if_missing(
                cls,
                sel!(application:openURLs:),
                // SAFETY: 目标回调签名与 `extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject)`
                // 完全一致，且为 AppKit 要求的 cdecl/Objective-C method 调用约定。
                std::mem::transmute::<
                    extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject),
                    Imp,
                >(open_urls_callback),
                c"v@:@:@".as_ptr() as *const c_char,
                "application:openURLs:",
                &class_name,
            );
        }

        /// 如果 delegate 类尚未实现指定 selector，则注入方法。
        ///
        /// # Safety
        /// `cls` 必须是有效的 Objective-C 类；`imp` / `types` 必须匹配 selector 的签名。
        unsafe fn add_delegate_method_if_missing(
            cls: &AnyClass,
            sel: Sel,
            imp: Imp,
            types: *const c_char,
            method_name: &str,
            class_name: &str,
        ) {
            if cls.instance_method(sel).is_some() {
                return;
            }
            let added = class_addMethod(cls as *const AnyClass as *mut AnyClass, sel, imp, types);
            if !added.as_bool() {
                eprintln!(
                    "warning: add_delegate_method_if_missing: failed to add {} on {}",
                    method_name, class_name
                );
            } else {
                dock_log(&format!(
                    "dock_open: installed {} on delegate class {}",
                    method_name, class_name
                ));
            }
        }

        /// 取出并清空当前累积的待打开路径。应在主线程每帧调用一次。
        pub fn take_dock_open_paths() -> Vec<PathBuf> {
            let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        }

        static WAKE_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

        /// 注册用于唤醒 UI 的 egui Context；app 创建时调用一次。
        pub fn set_wake_context(ctx: egui::Context) {
            *WAKE_CTX.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
        }

        /// 空闲时 winit 事件循环睡眠、egui 不重绘，队列只能等下次 `update()`
        /// 才能排空；收到打开事件后必须主动唤醒，否则文件滞留到下次重绘。
        fn wake_ui() {
            let ctx = WAKE_CTX.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        }

        fn enqueue_paths(paths: Vec<PathBuf>) {
            if paths.is_empty() {
                return;
            }
            {
                let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
                for path in paths {
                    if !guard.iter().any(|p| p == &path) {
                        guard.push(path);
                    }
                }
            }
            wake_ui();
        }

        // SAFETY: 该函数作为 Objective-C method 被 AppKit 在主线程调用；`files` 为
        // NSArray 实例，调用者保证其非空且生命周期覆盖本次调用。
        extern "C" fn open_files_callback(
            _this: &AnyObject,
            _sel: Sel,
            _app: *mut AnyObject,
            files: *mut AnyObject,
        ) {
            if files.is_null() {
                return;
            }
            // SAFETY: `files` 为有效的 `NSArray<NSString *>`；迭代期间数组不会被释放。
            let paths = unsafe { collect_paths_from_array(files) };
            if !paths.is_empty() {
                dock_log(&format!(
                    "dock_open: received {} file(s) via application:openFiles:",
                    paths.len()
                ));
            }
            enqueue_paths(paths);
        }

        // SAFETY: 该函数作为 Objective-C method 被 AppKit 在主线程调用；`urls` 为
        // NSArray<NSURL *> 实例，调用者保证其非空且生命周期覆盖本次调用。
        extern "C" fn open_urls_callback(
            _this: &AnyObject,
            _sel: Sel,
            _app: *mut AnyObject,
            urls: *mut AnyObject,
        ) {
            if urls.is_null() {
                return;
            }
            // SAFETY: `urls` 为有效的 `NSArray<NSURL *>`；迭代期间数组不会被释放。
            let paths = unsafe { collect_paths_from_url_array(urls) };
            if !paths.is_empty() {
                dock_log(&format!(
                    "dock_open: received {} file(s) via application:openURLs:",
                    paths.len()
                ));
            }
            enqueue_paths(paths);
        }

        // SAFETY: 该函数作为 Objective-C method 被 AppKit 在主线程调用；`file` 为
        // NSString 实例，调用者保证其非空且生命周期覆盖本次调用。
        extern "C" fn open_file_callback(
            _this: &AnyObject,
            _sel: Sel,
            _app: *mut AnyObject,
            file: *mut AnyObject,
        ) -> Bool {
            if file.is_null() {
                return Bool::NO;
            }
            // SAFETY: `file` 为有效的 `NSString`；`fileSystemRepresentation` 返回的指针
            // 在 autorelease pool 释放前有效。
            if let Some(path) = unsafe { nsstring_to_path(file) } {
                dock_log(&format!(
                    "dock_open: received file via application:openFile: {}",
                    path.display()
                ));
                enqueue_paths(vec![path]);
                Bool::YES
            } else {
                // 返回 NO 表示我们未处理该文件，系统可能会尝试其他 handler。
                Bool::NO
            }
        }

        unsafe fn collect_paths_from_array(files: *mut AnyObject) -> Vec<PathBuf> {
            // SAFETY: `files` 为 `NSArray` 实例，调用 `count` 不会转移所有权。
            let count: usize = msg_send![files, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                // SAFETY: `objectAtIndex:` 返回数组内已保留对象的指针，不会转移所有权。
                let item: *mut AnyObject = msg_send![files, objectAtIndex:i];
                if item.is_null() {
                    continue;
                }
                if let Some(path) = nsstring_to_path(item) {
                    paths.push(path);
                }
            }
            paths
        }

        unsafe fn collect_paths_from_url_array(urls: *mut AnyObject) -> Vec<PathBuf> {
            // SAFETY: `urls` 为 `NSArray<NSURL *>` 实例，调用 `count` 不会转移所有权。
            let count: usize = msg_send![urls, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                // SAFETY: `objectAtIndex:` 返回数组内已保留对象的指针，不会转移所有权。
                let url: *mut AnyObject = msg_send![urls, objectAtIndex:i];
                if url.is_null() {
                    continue;
                }
                // SAFETY: `path` 返回 `NSString`，生命周期覆盖当前 autorelease pool。
                let path_str: *mut AnyObject = msg_send![url, path];
                if let Some(path) = nsstring_to_path(path_str) {
                    paths.push(path);
                }
            }
            paths
        }

        unsafe fn nsstring_to_path(s: *mut AnyObject) -> Option<PathBuf> {
            // SAFETY: `s` 为有效的 `NSString`；`fileSystemRepresentation` 返回以
            // 文件系统编码表示的、以 NUL 结尾的 C 字符串指针。
            let fs: *const c_char = msg_send![s, fileSystemRepresentation];
            if fs.is_null() {
                return None;
            }
            // SAFETY: `fileSystemRepresentation` 保证返回合法且生命周期覆盖当前
            // autorelease pool 的 C 字符串；使用 `CStr` 仅做只读解析，不越界。
            CStr::from_ptr(fs).to_str().ok().map(PathBuf::from)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::loader::MAX_IMAGE_DIMENSION;

        #[test]
        fn oversized_jpeg_with_exif_thumb_decodes_near_max_dim_not_exif_thumb() {
            // Fixture: 4200×100 primary + ~160×4 EXIF thumb. Long edge > 4096
            // forces the ImageIO thumbnail downsample path; with IfAbsent this
            // used to return the EXIF postage stamp.
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/large_jpeg_with_exif_thumb.jpg");
            let bytes = std::fs::read(&path).expect("fixture present");
            let loaded = decode_image_bytes(&bytes, false)
                .expect("decode ok")
                .expect("ImageIO should handle JPEG");
            let [w, h] = loaded.original_size();
            let long = w.max(h);
            assert!(
                long > 1024,
                "decoded {}x{} — looks like EXIF embedded thumb, not a full downsample",
                w,
                h
            );
            assert!(
                long <= MAX_IMAGE_DIMENSION,
                "decoded {}x{} exceeds MAX_IMAGE_DIMENSION",
                w,
                h
            );
            // Aspect roughly preserved (4200:100).
            assert!(
                (w as f32 / h as f32) > 20.0,
                "unexpected aspect {}x{}",
                w,
                h
            );
        }

        #[test]
        fn ui_thumbnail_path_downscales_oversized_jpeg_not_exif_thumb() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/large_jpeg_with_exif_thumb.jpg");
            let bytes = std::fs::read(&path).expect("fixture present");
            let thumb = decode_thumbnail_bytes(&bytes)
                .expect("decode ok")
                .expect("ImageIO should handle JPEG");
            let long = thumb.size[0].max(thumb.size[1]) as u32;
            assert!(
                long > 32,
                "UI thumb {}x{} too small — EXIF thumb leak?",
                thumb.size[0],
                thumb.size[1]
            );
            assert!(
                long <= crate::loader::THUMBNAIL_MAX_DIMENSION,
                "UI thumb {} exceeds THUMBNAIL_MAX_DIMENSION",
                long
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub mod macos {
    use crate::loader::LoadedImage;
    use egui::ColorImage;

    pub fn decode_image_bytes(
        _bytes: &[u8],
        _compress: bool,
    ) -> Result<Option<LoadedImage>, String> {
        Ok(None)
    }

    pub fn decode_thumbnail_bytes(_bytes: &[u8]) -> Result<Option<ColorImage>, String> {
        Ok(None)
    }

    pub mod mpv_view {
        pub struct MpvNativeView;
        impl MpvNativeView {
            pub fn new<
                W: wry::raw_window_handle::HasWindowHandle + wry::raw_window_handle::HasDisplayHandle,
            >(
                _parent: &W,
                _bounds: wry::Rect,
                _player: &openitgo_media::MpvPlayer,
            ) -> Result<Self, String> {
                Err("媒体播放暂仅支持 macOS".to_string())
            }
            pub fn set_bounds(&self, _bounds: wry::Rect) {}
        }
    }
}
