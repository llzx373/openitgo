# rustReader

一款使用 Rust + egui 构建的跨平台漫画阅读器，支持国漫（左→右）、日漫（右→左）和韩漫/Webtoon（长条从上到下）三种阅读模式。

## 功能

- 打开本地图片文件夹、CBZ/ZIP、CBR/RAR（待完整实现）、PDF（待完整实现）
- 三种阅读模式切换
- 缩放、平移、全屏
- 书架、阅读历史、书签
- 缩略图导航

## 运行

```bash
cargo run -p rust-reader-app
```

## 测试

```bash
cargo test
```
