//! Platform-specific helpers.

#[cfg(target_os = "macos")]
pub mod macos {
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

    unsafe fn create_thumbnail_image(source: CGImageSourceRef, max_dim: usize) -> Option<CGImage> {
        let max_size_key = CFString::from_static_string("kCGImageSourceThumbnailMaxPixelSize");
        let create_if_absent_key =
            CFString::from_static_string("kCGImageSourceCreateThumbnailFromImageIfAbsent");
        let should_cache_key = CFString::from_static_string("kCGImageSourceShouldCache");

        let max_size = CFNumber::from(max_dim as i64);
        let one = CFNumber::from(1i32);
        let zero = CFNumber::from(0i32);

        let options = CFDictionary::from_CFType_pairs(&[
            (max_size_key, max_size),
            (create_if_absent_key, one),
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
        ColorImage { size, pixels: out }
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

    #[allow(unexpected_cfgs)]
    pub mod dock_open {
        use std::ffi::{c_char, CStr};
        use std::path::PathBuf;
        use std::sync::Mutex;

        use objc::runtime::{
            class_addMethod, class_getName, object_getClass, Class, Object, Sel, BOOL, YES,
        };
        use objc::{class, msg_send, sel, sel_impl};

        static OPEN_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

        #[link(name = "Cocoa", kind = "framework")]
        extern "C" {}

        /// 向当前 NSApplication delegate 注入 `application:openFiles:` 与
        /// `application:openFile:`，用于接收 Dock / Finder 拖入或双击打开的文件。
        ///
        /// `application:openFile(s):` 由系统在主线程调用，因此使用普通 `Mutex`
        /// 即可保证线程安全，无需额外同步。
        pub fn install_dock_open_handler() {
            // SAFETY: 所有 Objective-C 消息发送都在主线程执行；`NSApplication` 为
            // AppKit 单例，其 delegate 与 class 在本次运行时有效。
            unsafe {
                let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
                if app.is_null() {
                    eprintln!("warning: install_dock_open_handler: NSApplication is null");
                    return;
                }
                let delegate: *mut Object = msg_send![app, delegate];
                if delegate.is_null() {
                    eprintln!("warning: install_dock_open_handler: NSApplication delegate is null");
                    return;
                }
                let cls = object_getClass(delegate) as *mut Class;
                if cls.is_null() {
                    eprintln!("warning: install_dock_open_handler: delegate class is null");
                    return;
                }

                // SAFETY: 只修改应用自身的 delegate class，绝不修改系统类（NS 前缀）。
                // 若 class_getName 返回空或以 "NS" 开头，则跳过注入并告警。
                let name_ptr = class_getName(cls);
                if name_ptr.is_null() {
                    eprintln!(
                        "warning: install_dock_open_handler: could not get delegate class name"
                    );
                    return;
                }
                if let Ok(name) = CStr::from_ptr(name_ptr).to_str() {
                    if name.is_empty() {
                        eprintln!(
                            "warning: install_dock_open_handler: delegate class name is empty"
                        );
                        return;
                    }
                    if name.starts_with("NS") {
                        eprintln!(
                            "warning: install_dock_open_handler: refusing to inject into system class {}",
                            name
                        );
                        return;
                    }
                }

                let open_files_types = c"v@:@:@".as_ptr() as *const c_char;
                let added_open_files = class_addMethod(
                    cls,
                    sel!(application:openFiles:),
                    // SAFETY: 目标回调签名与 `extern "C" fn(&Object, Sel, *mut Object, *mut Object)`
                    // 完全一致，且为 AppKit 要求的 cdecl/Objective-C method 调用约定。
                    std::mem::transmute::<
                        extern "C" fn(&Object, Sel, *mut Object, *mut Object),
                        unsafe extern "C" fn(),
                    >(open_files_callback),
                    open_files_types,
                );
                if added_open_files == objc::runtime::NO {
                    eprintln!(
                        "warning: install_dock_open_handler: failed to add application:openFiles:"
                    );
                }

                let open_file_types = c"c@:@:@".as_ptr() as *const c_char;
                let added_open_file = class_addMethod(
                    cls,
                    sel!(application:openFile:),
                    // SAFETY: 目标回调签名与 `extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> BOOL`
                    // 完全一致，且为 AppKit 要求的 cdecl/Objective-C method 调用约定。
                    std::mem::transmute::<
                        extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> BOOL,
                        unsafe extern "C" fn(),
                    >(open_file_callback),
                    open_file_types,
                );
                if added_open_file == objc::runtime::NO {
                    eprintln!(
                        "warning: install_dock_open_handler: failed to add application:openFile:"
                    );
                }
            }
        }

        /// 取出并清空当前累积的待打开路径。应在主线程每帧调用一次。
        pub fn take_dock_open_paths() -> Vec<PathBuf> {
            let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        }

        // SAFETY: 该函数作为 Objective-C method 被 AppKit 在主线程调用；`files` 为
        // NSArray 实例，调用者保证其非空且生命周期覆盖本次调用。
        extern "C" fn open_files_callback(
            _this: &Object,
            _sel: Sel,
            _app: *mut Object,
            files: *mut Object,
        ) {
            if files.is_null() {
                return;
            }
            // SAFETY: `files` 为有效的 `NSArray<NSString *>`；迭代期间数组不会被释放。
            let paths = unsafe { collect_paths_from_array(files) };
            let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            guard.extend(paths);
        }

        // SAFETY: 该函数作为 Objective-C method 被 AppKit 在主线程调用；`file` 为
        // NSString 实例，调用者保证其非空且生命周期覆盖本次调用。
        extern "C" fn open_file_callback(
            _this: &Object,
            _sel: Sel,
            _app: *mut Object,
            file: *mut Object,
        ) -> BOOL {
            if file.is_null() {
                return objc::runtime::NO;
            }
            // SAFETY: `file` 为有效的 `NSString`；`fileSystemRepresentation` 返回的指针
            // 在 autorelease pool 释放前有效。
            if let Some(path) = unsafe { nsstring_to_path(file) } {
                let mut guard = OPEN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
                guard.push(path);
                YES
            } else {
                // 返回 NO 表示我们未处理该文件，系统可能会尝试其他 handler。
                objc::runtime::NO
            }
        }

        unsafe fn collect_paths_from_array(files: *mut Object) -> Vec<PathBuf> {
            // SAFETY: `files` 为 `NSArray` 实例，调用 `count` 不会转移所有权。
            let count: usize = msg_send![files, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                // SAFETY: `objectAtIndex:` 返回数组内已保留对象的指针，不会转移所有权。
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
}
