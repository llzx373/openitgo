# rustReader

一款使用 Rust + egui 构建的跨平台漫画阅读器，支持国漫（左→右）、日漫（右→左）和韩漫/Webtoon（长条从上到下）三种阅读模式。

## 功能

- 打开本地图片文件夹、CBZ/ZIP、CBR/RAR、PDF
- 三种阅读模式切换
- 缩放、平移、全屏
- 书架、阅读历史、书签
- 缩略图导航
- 异步后台加载：文件解压、图片解码、PDF 渲染在独立线程进行，避免 UI 阻塞
- 基于大小的 LRU 缓存：按设定的内存预算（100 MB - 4 GB，默认 1 GB）缓存已解码页面，自动淘汰最少使用的页面

## 运行

```bash
cargo run -p rust-reader-app
```

## 测试

```bash
cargo test
```
