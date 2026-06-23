> **Status:** 已实现。多线程解码池已落地，实际含缩略图队列与 wgpu 纹理上传路径。
>
> **注意：** 文档目标中的 TODO #15 为历史编号，详见 `TODO.md` 中的「历史 TODO 编号对照表」。

# 多线程解码池设计

## 目标
将漫画阅读器中的图片解码从单后台线程改为多线程解码池（TODO #15），提升大文件、高分辨率图片的加载速度。

## 范围
- 仅池化**图片解码**（`image::load_from_memory` + 降采样）。
- PDF 渲染仍保留在单 IO 线程，不参与池化。
- 文件/压缩包读取仍由单个 IO 线程完成，读取后把原始字节提交给解码池。
- `PageLoader` 对外 API 不变。

## 当前架构
- `PageLoader` 内部有一个后台线程，通过 `crossbeam_channel` 接收 `LoadRequest`（high/low 双通道）。
- 该线程同步完成：读文件/压缩包/PDF → 解码图片 → 发送 `LoadResult`。
- `ReaderView` 每帧调用 `loader.try_recv()`，按 `epoch` 过滤结果，上传到 GPU texture。

## 新架构

### 组件

1. **IO 线程（1 个）**
   - 消费现有的 `high_receiver` / `low_receiver`。
   - 对 `PageSource::PdfPage`：直接调用 `render_pdf_page`，发送 `LoadResult`。
   - 对其他来源：读取原始字节，构造 `DecodeJob`，发送到解码池。

2. **解码池（`num_cpus::get()` 个线程）**
   - 消费 `DecodeJob`。
   - 调用 `decode_image_bytes(bytes, format_hint)` 解码并降采样。
   - 发送 `LoadResult` 到结果通道。

### 数据类型

```rust
struct DecodeJob {
    epoch: Epoch,
    page_index: usize,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}
```

`format_hint` 用于 `image::ImageFormat::from_extension`，帮助 `image` crate 识别无 magic number 的格式。

### 优先级

- IO 线程仍使用 `crossbeam_channel::select!` 优先处理 high 请求。
- 解码池使用单个 FIFO 通道即可；因为 high 请求会先被 IO 线程读取并提交到池，自然排在前面。

### 不变量

- `PageLoader` 公开方法不变：`new`, `next_epoch`, `request_high`, `request_low`, `try_recv`。
- `LoadResult` 不变：`epoch`, `page_index`, `image: Result<ColorImage, String>`。
- `ReaderView` 与 `PageCache` 无需改动。

## 错误处理

- 读取失败：IO 线程直接发送 `Err(...)` 的 `LoadResult`。
- 解码失败：解码池发送 `Err(...)` 的 `LoadResult`。
- 通道关闭：worker 退出循环；`PageLoader` 继续运行，后续请求被忽略。

## 线程数

- 默认 `num_cpus::get()`。
- 不暴露用户设置。

## 测试计划

- 保留并运行现有 `PageLoader` 测试（行为不变）。
- 新增一个并发 smoke 测试：向 `PageLoader` 并发请求多张不同图片，所有结果都能收到且不 panic。
- 保留 `decode_image` / `downsample_if_needed` 单测。

## 变更文件

- `rust-reader-app/src/loader.rs`（主要实现）
- `rust-reader-app/Cargo.toml`（新增 `num_cpus` 依赖）
- `TODO.md`
