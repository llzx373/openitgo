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

        Ok(Some(dynamic_to_loaded_image(
            image::DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(width as u32, height as u32, pixel_data)
                    .ok_or_else(|| "invalid RGBA buffer size".to_string())?,
            ),
            compress,
        )?))
    }

    unsafe fn decode_thumbnail(
        bytes: &[u8],
        compress: bool,
    ) -> Result<Option<LoadedImage>, String> {
        let cf_data = CFData::from_buffer(bytes);

        let max_size_key = CFString::from_static_string("kCGImageSourceThumbnailMaxPixelSize");
        let create_with_transform_key =
            CFString::from_static_string("kCGImageSourceCreateThumbnailWithTransform");
        let should_cache_key = CFString::from_static_string("kCGImageSourceShouldCache");

        let max_size = CFNumber::from(MAX_IMAGE_DIMENSION as i64);
        let one = CFNumber::from(1i32);
        let zero = CFNumber::from(0i32);

        let options = CFDictionary::from_CFType_pairs(&[
            (max_size_key, max_size),
            (create_with_transform_key, one),
            (should_cache_key, zero),
        ]);

        let source = CGImageSourceCreateWithData(
            cf_data.as_concrete_TypeRef(),
            options.as_concrete_TypeRef(),
        );
        if source.is_null() {
            return Err("ImageIO could not create source for thumbnail".to_string());
        }
        let _source_guard = ImageSource(source);

        let image_ref = CGImageSourceCreateImageAtIndex(source, 0, std::ptr::null());
        if image_ref.is_null() {
            return Err("ImageIO could not create thumbnail".to_string());
        }
        let cg_image = CGImage::from_ptr(image_ref);

        let width = cg_image.width();
        let height = cg_image.height();
        let bytes_per_row = width * 4;
        let mut pixel_data = vec![0u8; height * bytes_per_row];

        render_into_buffer(&cg_image, width, height, bytes_per_row, &mut pixel_data)?;

        Ok(Some(dynamic_to_loaded_image(
            image::DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(width as u32, height as u32, pixel_data)
                    .ok_or_else(|| "invalid RGBA buffer size".to_string())?,
            ),
            compress,
        )?))
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
}

#[cfg(not(target_os = "macos"))]
pub mod macos {
    use crate::loader::LoadedImage;

    pub fn decode_image_bytes(
        _bytes: &[u8],
        _compress: bool,
    ) -> Result<Option<LoadedImage>, String> {
        Ok(None)
    }
}
