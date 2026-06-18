> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将漫画页面纹理从 RGBA8 改为 DXT5/BC3 sRGB 压缩上传，减少 GPU 显存占用，不支持时回退到 RGBA8。

**Architecture:** 解码线程用 `texpresso` 把 `ColorImage` 压缩为 DXT5；主线程检测 S3TC 扩展后通过 glow 上传压缩数据并注册为 native texture。缓存层同时保存 managed `TextureHandle` 和 native `TextureId`。

**Tech Stack:** Rust, egui 0.29, eframe 0.29 (Glow), glow, texpresso

---

## File Structure

- `rust-reader-app/Cargo.toml`
  - 新增 `texpresso` 和 `glow` 依赖。
- `rust-reader-app/src/loader.rs`
  - 新增 `LoadedImage`、`CompressedFormat`。
  - 解码后压缩为 DXT5，失败则回退 `ColorImage`。
- `rust-reader-app/src/widgets/page_view.rs`（或新建 `texture_upload.rs`）
  - 新增 `upload_image` / `upload_compressed` 上传函数。
- `rust-reader-app/src/cache.rs`
  - `PageCache` 支持 managed 和 native 两种 texture slot。
- `rust-reader-app/src/views/reader.rs`
  - `update` 接收 `&mut eframe::Frame`，用于检测扩展和上传压缩纹理。
  - 渲染时区分 `TextureSlot`。
- `rust-reader-app/src/app.rs`
  - 调用 `reader_view.update` 时传入 `frame`。
  - 保存 GPU 能力标志。
- `TODO.md`
  - 标记 #17 完成，#18 放弃。

---

## Task 1: 添加依赖

**Files:**
- Modify: `rust-reader-app/Cargo.toml`

- [ ] **Step 1: 添加 `texpresso` 和 `glow`**

```toml
[dependencies]
# ... existing deps ...
texpresso = "2.0"
glow = "0.14"
```

- [ ] **Step 2: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: downloads dependencies, no errors yet

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/Cargo.toml
git commit -m "build(app): add texpresso and glow dependencies"
```

---

## Task 2: 在 Loader 中压缩为 DXT5

**Files:**
- Modify: `rust-reader-app/src/loader.rs`

- [ ] **Step 1: 定义 `LoadedImage` 和 `CompressedFormat`**

在 `LoadResult` 附近添加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressedFormat {
    Dxt5Srgb,
}

pub enum LoadedImage {
    Compressed {
        data: Vec<u8>,
        original_size: [u32; 2],
        gpu_size: [u32; 2],
        format: CompressedFormat,
    },
    Color(ColorImage),
}
```

- [ ] **Step 2: 修改 `LoadResult`**

```rust
pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub image: Result<LoadedImage, String>,
}
```

- [ ] **Step 3: 新增 DXT5 压缩函数**

```rust
fn compress_dxt5(image: image::DynamicImage) -> Result<LoadedImage, String> {
    let original_w = image.width();
    let original_h = image.height();
    let rgba = image.to_rgba8();
    let (gpu_w, gpu_h) = padded_size(original_w, original_h);

    let pixels = if original_w == gpu_w && original_h == gpu_h {
        rgba.into_raw()
    } else {
        pad_rgba(&rgba, original_w, original_h, gpu_w, gpu_h)
    };

    let mut output = vec![0u8; texpresso::Format::Bc3.compressed_size(gpu_w as usize, gpu_h as usize)];
    texpresso::Format::Bc3.compress(
        &pixels,
        gpu_w as usize,
        gpu_h as usize,
        texpresso::Params {
            algorithm: texpresso::Algorithm::ClusterFit,
            weights: texpresso::COLOUR_WEIGHTS_PERCEPTUAL,
            weigh_colour_by_alpha: false,
        },
        &mut output,
    );

    Ok(LoadedImage::Compressed {
        data: output,
        original_size: [original_w, original_h],
        gpu_size: [gpu_w, gpu_h],
        format: CompressedFormat::Dxt5Srgb,
    })
}

fn padded_size(width: u32, height: u32) -> (u32, u32) {
    let pad = |n: u32| ((n + 3) / 4) * 4;
    (pad(width), pad(height))
}

fn pad_rgba(
    src: &image::RgbaImage,
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..src_h {
        for x in 0..src_w {
            let src_pixel = src.get_pixel(x, y).0;
            let dst_idx = ((y * dst_w + x) * 4) as usize;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src_pixel);
        }
    }
    dst
}
```

- [ ] **Step 4: 修改 `decode_image_bytes` 返回 `LoadedImage`**

```rust
fn decode_image_bytes(bytes: &[u8], format_hint: Option<&str>) -> Result<LoadedImage, String> {
    let format = format_hint.and_then(image::ImageFormat::from_extension);
    let image = if let Some(format) = format {
        image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?
    } else {
        image::load_from_memory(bytes).map_err(|e| e.to_string())?
    };
    let image = downsample_if_needed(image);
    compress_dxt5(image)
}
```

- [ ] **Step 5: 在 PDF 渲染路径回退到 `ColorImage`**

PDF 渲染函数 `render_pdf_page` 仍返回 `ColorImage`。将其包装为 `LoadedImage::Color`：

```rust
fn render_pdf_page(document: &Path, page_number: usize) -> Result<LoadedImage, String> {
    // ... existing render logic ...
    Ok(LoadedImage::Color(ColorImage::from_rgba_premultiplied(size, pixmap.data_as_u8_slice())))
}
```

并更新 `process_io_request` 中 PDF 分支直接发送 `render_pdf_page` 结果。

- [ ] **Step 6: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 7: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "feat(loader): compress decoded images to DXT5"
```

---

## Task 3: 上传模块支持压缩纹理

**Files:**
- Modify or create: `rust-reader-app/src/widgets/page_view.rs`

- [ ] **Step 1: 添加上传函数**

如果 `page_view.rs` 已有 `upload_color_image`，扩展为：

```rust
use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};

pub fn upload_image(
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
    label: &str,
    image: LoadedImage,
    supports_dxt5: bool,
) -> TextureSlot {
    match image {
        LoadedImage::Compressed { data, original_size, gpu_size, .. } if supports_dxt5 => {
            upload_compressed_native(frame, label, data, gpu_size, original_size)
        }
        LoadedImage::Compressed { original_size, .. } => {
            // Should not happen if loader always falls back; fallback to a 1x1 placeholder.
            let color = ColorImage::new([original_size[0] as _, original_size[1] as _], egui::Color32::MAGENTA);
            TextureSlot::Managed(ctx.load_texture(label, color, TextureOptions::LINEAR))
        }
        LoadedImage::Color(image) => {
            TextureSlot::Managed(ctx.load_texture(label, image, TextureOptions::LINEAR))
        }
    }
}

fn upload_compressed_native(
    frame: &mut eframe::Frame,
    label: &str,
    data: Vec<u8>,
    gpu_size: [u32; 2],
    display_size: [u32; 2],
) -> TextureSlot {
    let gl = frame.gl().expect("glow context required");
    let texture = unsafe { gl.create_texture() }.expect("failed to create texture");
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.compressed_tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT,
            gpu_size[0] as i32,
            gpu_size[1] as i32,
            0,
            data.len() as i32,
            &data,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    let id = frame.register_native_texture(texture);
    TextureSlot::Native(id, display_size)
}
```

- [ ] **Step 2: 定义 `TextureSlot`**

放在上传模块中：

```rust
#[derive(Clone)]
pub enum TextureSlot {
    Managed(TextureHandle),
    Native(TextureId, [u32; 2]), // display size
}

impl TextureSlot {
    pub fn size(&self) -> [u32; 2] {
        match self {
            TextureSlot::Managed(h) => h.size(),
            TextureSlot::Native(_, s) => *s,
        }
    }

    pub fn id(&self) -> TextureId {
        match self {
            TextureSlot::Managed(h) => h.id(),
            TextureSlot::Native(id, _) => *id,
        }
    }
}
```

- [ ] **Step 3: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: may require adjusting imports; fix until no errors

- [ ] **Step 4: 提交**

```bash
git add rust-reader-app/src/widgets/page_view.rs
git commit -m "feat(texture): upload DXT5 compressed textures via glow"
```

---

## Task 4: 改造缓存层

**Files:**
- Modify: `rust-reader-app/src/cache.rs`

- [ ] **Step 1: 使用 `TextureSlot` 替换 `TextureHandle`**

```rust
use crate::widgets::page_view::TextureSlot;

struct CacheEntry {
    slot: TextureSlot,
    size_bytes: usize,
    last_used: Instant,
}

pub struct PageCache {
    entries: HashMap<usize, CacheEntry>,
    total_size_bytes: usize,
}
```

- [ ] **Step 2: 修改 insert/get**

```rust
impl PageCache {
    pub fn insert(&mut self, page_index: usize, slot: TextureSlot) {
        let size_bytes = match &slot {
            TextureSlot::Managed(h) => h.size()[0] * h.size()[1] * 4,
            TextureSlot::Native(_, size) => {
                let padded = ((size[0] + 3) / 4) * 4;
                padded * ((size[1] + 3) / 4) * 4
            }
        };
        if let Some(old) = self.entries.remove(&page_index) {
            self.total_size_bytes -= old.size_bytes;
        }
        self.total_size_bytes += size_bytes;
        self.entries.insert(
            page_index,
            CacheEntry {
                slot,
                size_bytes,
                last_used: Instant::now(),
            },
        );
    }

    pub fn get(&mut self, page_index: usize) -> Option<&TextureSlot> {
        self.entries.get_mut(&page_index).map(|e| {
            e.last_used = Instant::now();
            &e.slot
        })
    }
}
```

- [ ] **Step 3: 淘汰时删除 native texture**

`enforce_budget` 需要一个 `glow::Context` 引用（或一个删除回调）来删除 native texture。为简化，先在 `enforce_budget` 签名中加入 `gl: &glow::Context`：

```rust
pub fn enforce_budget(&mut self, budget_bytes: usize, gl: &glow::Context) {
    while self.total_size_bytes > budget_bytes && self.entries.len() > 1 {
        let lru = self.entries.iter().min_by_key(|(_, e)| e.last_used).map(|(k, _)| *k);
        if let Some(idx) = lru {
            if let Some(entry) = self.entries.remove(&idx) {
                if let TextureSlot::Native(id, _) = entry.slot {
                    unsafe { gl.delete_texture(id) };
                }
                self.total_size_bytes -= entry.size_bytes;
            }
        }
    }
}
```

> 注意：`TextureId::User` 内层类型是 `u64`；`glow::Context::delete_texture` 接收 `TextureId`。需确认 glow 支持。如果不支持，使用 `gl.delete_texture(native_id)` 可能需要从 `TextureId` 提取。egui 的 `TextureId` 是 `Copy`，可 pattern match：`if let egui::TextureId::User(native) = id { gl.delete_texture(native); }`。

- [ ] **Step 4: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 5: 提交**

```bash
git add rust-reader-app/src/cache.rs
git commit -m "feat(cache): store managed and native texture slots"
```

---

## Task 5: Reader 集成与渲染

**Files:**
- Modify: `rust-reader-app/src/views/reader.rs`
- Modify: `rust-reader-app/src/app.rs`

- [ ] **Step 1: 修改 `ReaderView::update` 签名**

```rust
pub fn update(
    &mut self,
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
    loader: &PageLoader,
    cache_size_mb: u32,
    supports_dxt5: bool,
)
```

- [ ] **Step 2: 在 update 中使用 `upload_image`**

把现有的 `upload_color_image(...)` 调用替换为：

```rust
let slot = upload_image(ctx, frame, &label, image, supports_dxt5);
reader.cache.insert(page_index, slot);
```

- [ ] **Step 3: 修改 `enforce_cache_budget` 调用**

传入 `frame.gl().expect(...)`：

```rust
reader.cache.enforce_budget(cache_size_bytes, frame.gl().unwrap());
```

- [ ] **Step 4: 修改渲染逻辑**

把原来使用 `TextureHandle` 的地方改为 `TextureSlot`：

```rust
fn render_page(ui: &mut egui::Ui, slot: &TextureSlot, fit_size: egui::Vec2) {
    match slot {
        TextureSlot::Managed(handle) => {
            ui.image((handle.id(), fit_size));
        }
        TextureSlot::Native(id, display_size) => {
            let (gpu_w, gpu_h) = padded_size(display_size[0], display_size[1]);
            let uv_max = egui::vec2(
                display_size[0] as f32 / gpu_w as f32,
                display_size[1] as f32 / gpu_h as f32,
            );
            ui.add(
                egui::Image::new((*id, fit_size))
                    .uv(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max.to_pos2())),
            );
        }
    }
}
```

- [ ] **Step 5: 修改 `app.rs` 调用**

```rust
let gl = _frame.gl();
let supports_dxt5 = gl.map_or(false, |gl| {
    gl.supported_extensions().contains("GL_EXT_texture_compression_s3tc")
});
self.reader_view.update(
    ctx,
    _frame,
    &self.page_loader,
    self.settings.cache_size_mb,
    supports_dxt5,
);
```

- [ ] **Step 6: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 7: 提交**

```bash
git add rust-reader-app/src/views/reader.rs rust-reader-app/src/app.rs
git commit -m "feat(reader): render DXT5 compressed textures"
```

---

## Task 6: 测试

**Files:**
- Modify: `rust-reader-app/src/loader.rs` test module

- [ ] **Step 1: 添加压缩输出大小测试**

```rust
#[test]
fn test_dxt5_compressed_size() {
    let img = image::DynamicImage::new_rgba8(64, 64);
    let loaded = compress_dxt5(img).expect("compression should succeed");
    match loaded {
        LoadedImage::Compressed { data, gpu_size, .. } => {
            assert_eq!(gpu_size, [64, 64]);
            let expected = (64 / 4) * (64 / 4) * 16;
            assert_eq!(data.len(), expected);
        }
        LoadedImage::Color(_) => panic!("expected compressed"),
    }
}

#[test]
fn test_dxt5_pads_to_multiple_of_four() {
    let img = image::DynamicImage::new_rgba8(65, 65);
    let loaded = compress_dxt5(img).expect("compression should succeed");
    match loaded {
        LoadedImage::Compressed { gpu_size, .. } => {
            assert_eq!(gpu_size, [68, 68]);
        }
        LoadedImage::Color(_) => panic!("expected compressed"),
    }
}
```

- [ ] **Step 2: 运行 loader 测试**

Run: `cargo test -p rust-reader-app loader`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/loader.rs
git commit -m "test(loader): verify DXT5 compression size and padding"
```

---

## Task 7: 全量验证与 TODO 更新

**Files:**
- Modify: `TODO.md`

- [ ] **Step 1: 运行完整检查**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all PASS

- [ ] **Step 2: 更新 TODO.md**

- 将 `- [ ] 17. GPU 纹理压缩` 改为 `- [x]`。
- 将 `- [ ] 18. 元数据与在线搜索` 改为 `- [ ] 18. 元数据与在线搜索（已放弃）` 或删除该项。

- [ ] **Step 3: 提交并推送**

```bash
git add TODO.md
git commit -m "chore: mark #17 done and abandon #18"
git push
```

---

## Self-Review

- **Spec coverage:**
  - 依赖：Task 1
  - Loader 压缩：Task 2
  - 压缩上传：Task 3
  - 缓存改造：Task 4
  - Reader 集成/渲染：Task 5
  - 测试：Task 6
  - TODO：Task 7
- **Placeholder scan：** 无 TBD。
- **Type consistency：** `LoadedImage`、`TextureSlot`、`CacheEntry` 字段一致。
