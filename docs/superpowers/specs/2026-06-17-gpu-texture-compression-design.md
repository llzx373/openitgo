> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将漫画页面的 GPU 纹理从 RGBA8 改为 DXT5/BC3 sRGB 压缩，减少显存占用。

**Architecture:** 解码线程在返回结果前用 `texpresso` 把 `ColorImage` 压缩成 DXT5；主线程检测 `GL_EXT_texture_compression_s3tc` 后，通过 glow 的 `compressed_tex_image_2d` + `register_native_texture` 上传压缩数据。不支持时回退到未压缩 `ColorImage`。缓存层统一保存 `TextureHandle` 或 `TextureId`，淘汰时删除 native texture。

**Tech Stack:** Rust, egui 0.29, eframe 0.29 (Glow), glow, texpresso

---

## 范围

- 仅针对 Glow 后端（当前默认）。
- 压缩格式：DXT5/BC3 sRGB（8 bpp）。
- 运行时检测 S3TC 扩展，不支持则回退。
- 图片尺寸补齐到 4 的倍数；显示时通过 UV 裁剪隐藏补齐区域。

## 数据类型

### `LoadedImage`（新增）

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

### `LoadResult` 修改

```rust
pub struct LoadResult {
    pub epoch: Epoch,
    pub page_index: usize,
    pub image: Result<LoadedImage, String>,
}
```

## 压缩阶段

在 `rust-reader-app/src/loader.rs` 的 `decode_image_bytes` 中：

1. 解码得到 `image::DynamicImage`，降采样。
2. 转换为 RGBA8。
3. 补齐宽高到 4 的倍数（复制边缘像素或填黑）。
4. 调用 `texpresso::Format::Bc3.encode(...)` 得到压缩字节。
5. 返回 `LoadedImage::Compressed { ... }`。

注意：压缩在 #15 的解码池线程中执行，不阻塞 UI。

## 扩展检测

在 `rust-reader-app/src/app.rs` 中：

- 在 `ReaderApp::new` 或第一次进入 Reader 时，通过 `frame.gl()` 获取 `glow::Context`。
- 调用 `gl.supported_extensions()` 检查是否包含 `"GL_EXT_texture_compression_s3tc"`。
- 将结果保存到 `ReaderApp.gpu_supports_dxt5`。

## 上传阶段

在 `rust-reader-app/src/widgets/page_view.rs`（或新建 `texture_upload.rs`）中：

### 未压缩路径（回退）

```rust
ctx.load_texture(label, image, TextureOptions::LINEAR)
```

### 压缩路径

```rust
let gl = frame.gl().expect("glow context");
let texture = unsafe { gl.create_texture() }?;
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
        data,
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
}
let id = frame.register_native_texture(texture);
```

## 缓存与渲染

### `PageCache` 修改

```rust
enum TextureSlot {
    Managed(egui::TextureHandle),
    Native(egui::TextureId, [u32; 2]), // display_size
}

struct CacheEntry {
    slot: TextureSlot,
    size_bytes: usize,
    last_used: Instant,
}
```

- `insert_managed(handle, size_bytes)`
- `insert_native(id, display_size, size_bytes)`
- `get(&self, idx) -> Option<(&TextureSlot, [u32;2])>`
- `enforce_budget` 淘汰时，若淘汰 `Native` 则调用 `gl.delete_texture`。

### Reader 渲染

`ReaderView::update` 接收 `&mut eframe::Frame` 用于上传/检测扩展。

渲染时：
- `Managed`：直接 `ui.image(handle)`。
- `Native(id, display_size)`：
  ```rust
  let uv_max = egui::vec2(
      display_size[0] as f32 / gpu_size[0] as f32,
      display_size[1] as f32 / gpu_size[1] as f32,
  );
  egui::Image::new((id, display_size))
      .uv(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max.to_pos2()))
  ```

## 预算计算

- 压缩：`gpu_size.0 * gpu_size.1` 字节（8 bpp）。
- 未压缩：`w * h * 4` 字节。

## 错误处理

- 压缩失败：记录错误并回退到未压缩 `ColorImage`。
- 上传失败：显示错误占位图，缓存不插入。

## 测试计划

- `loader` 测试：压缩输出大小 = `ceil(w/4)*ceil(h/4)*16`。
- `cache` 测试：native texture 插入/淘汰不 panic（使用 mock TextureId）。
- 全量 `cargo test --workspace` 通过。

## 变更文件

- `rust-reader-app/Cargo.toml`
- `rust-reader-app/src/loader.rs`
- `rust-reader-app/src/cache.rs`
- `rust-reader-app/src/widgets/page_view.rs`（或新增 `texture_upload.rs`）
- `rust-reader-app/src/views/reader.rs`
- `rust-reader-app/src/app.rs`
- `TODO.md`
