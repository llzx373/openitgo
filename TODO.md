# OpenItGo 开发 TODO

按优先级分为 P0/P1/P2/P3。逐个实现，遇到不确定状况先询问用户。完整审计报告见 `docs/audit-report.md`（该报告为 2026-06-17 之前的快照，大量条目已实现，当前状态以本 TODO 为准）。

**历史编号说明**：`docs/superpowers/` 下 2026-06-17 系列计划文档引用的是旧 TODO 编号（如 #14 书架搜索/排序、#15 多线程解码池、#16 ZIP 索引缓存、#17 GPU 纹理压缩、#18 放弃项等），本文件已重新编号，对应关系见底部“历史编号对照表”。

## P0 — 影响基础可用性

- [x] 1. 书架封面缩略图
  - [x] 1.1 添加漫画/文件夹时异步提取第一页并生成封面缩略图
  - [x] 1.2 将封面路径写入 `LibraryEntry.cover_path` 并持久化
  - [x] 1.3 在书架 UI 中以卡片/网格形式展示封面 + 标题
  - [x] 1.4 封面缺失时显示占位色块
  - [x] 1.5 历史/书签列表复用封面（后续 P1 统一处理）
- [x] 2. 统一 comic_id 生成
  - [x] 2.1 所有 parser 使用 `stable_comic_id(path)` 生成 ID
  - [x] 2.2 `ensure_in_library` 解析成功后使用 parser 生成的 ID
  - [x] 2.3 启动时迁移旧 library/history/bookmarks 到新的 path-based ID，并重命名封面文件
- [x] 3. 主题设置生效
  - [x] 3.1 在 `app.update` 中根据 `Settings.theme` 调用 `ctx.set_theme`
  - [x] 3.2 支持 System/Dark/Light 切换并即时生效
  - [x] 3.3 跟踪已应用主题避免每帧重复设置
- [x] 4. Webtoon 真正连续滚动
  - [x] 4.1 在 `reader.rs` 中新增 Webtoon 渲染分支
  - [x] 4.2 使用 `layout.rs` 的垂直布局计算每页偏移
  - [x] 4.3 滚轮改为垂直滚动，并自动更新当前页
  - [x] 4.4 键盘翻页后滚动到对应页面顶部
  - [x] 4.5 按需加载可见页缩略图/全图
- [x] 5. FitMode 与设置打通
  - [x] 5.1 删除 `QuickFit`，统一使用 `FitMode`
  - [x] 5.2 `ReaderView::open` 使用 `state.fit_mode` 作为初始适配
  - [x] 5.3 工具栏/快捷键 fit 操作使用 `FitMode`
  - [x] 5.4 打开漫画时从 `settings.default_fit` 设置 `state.fit_mode`
  - [x] 5.5 `apply_pending_fit` 同时更新 `state.fit_mode`

## P1 — 显著提升体验

- [x] 6. 阅读器缩放/平移增强
  - [x] 6.1 Ctrl/Command + 滚轮缩放
  - [x] 6.2 双击切换 Original / Page fit
  - [x] 6.3 窗口 resize 时自动重新 fit
  - [x] 6.4 限制平移边界，避免小图拖出视口
- [x] 7. 缩略图失败提示与重试退避
  - [x] 7.1 缩略图加载失败记录 `thumbnail_errors` 并显示“点击重试”占位
  - [x] 7.2 全图/缩略图错误均使用指数退避重试（最大 30 秒）
  - [x] 7.3 成功加载后自动清除错误状态
- [x] 8. PDF/RAR 缓存
  - [x] 8.1 PDF：在 loader 中缓存已读文件字节或解析后的文档
  - [x] 8.2 RAR：建立 `name -> header position` 索引，避免线性扫描
  - [x] 8.3 关闭漫画或 epoch 变化时释放缓存
- [x] 9. 书架/历史/书签右键菜单、搜索排序与元数据
  - [x] 9.1 为书架条目添加右键菜单（打开/编辑/删除）
  - [x] 9.2 显示页数、阅读百分比、添加时间
  - [x] 9.3 历史/书签列表也支持右键操作
  - [x] 9.4 书架支持按标题搜索与多种排序（最近阅读/标题/添加时间）
- [x] 10. 进度条悬停缩略图保持比例
  - [x] 10.1 按原图比例缩放预览图
  - [x] 10.2 限制最大尺寸 80×120
- [x] 11. 空书架引导
  - [x] 11.1 空状态显示大大的“打开文件夹”按钮
  - [x] 11.2 显示拖拽提示

## P2 — 功能完善

- [x] 12. 电子书阅读基础功能
  - [x] 12.1 核心模型：`Ebook`、`EbookChapter`、`EbookReadingMode`
  - [x] 12.2 解析器：EPUB、TXT、MOBI/AZW3、Markdown
  - [x] 12.3 设置模型：`EbookSettings`（字体、字号、行间距、主题、阅读模式）
  - [x] 12.4 设置 UI：电子书折叠面板
  - [x] 12.5 `EbookRenderer`：`wry` 子 webview + 自定义 `ebook://` 协议 + Rust/JS 通信
  - [x] 12.6 应用入口：`View::Ebook`、文件分发、菜单栏/工具栏集成、打开最近文件
  - [x] 12.7 修复 `ebook://reader?chapter=N` 协议处理顺序，验证 EPUB 可正常加载
  - [x] 12.8 目录面板：列出章节并支持跳转
  - [x] 12.9 历史与书签：保存/恢复电子书的章节与字符偏移
  - [x] 12.10 书架混排：在书架中显示电子书条目并支持过滤打开
- [x] 13. 书签 note 编辑
  - [x] 12.1 添加书签时允许输入 note
  - [x] 12.2 书签列表支持编辑/保存 note
- [x] 13. 历史单条删除/清空
  - [x] 13.1 历史列表支持单条删除
  - [x] 13.2 提供“清空历史”按钮
- [x] 14. 递归扫描导入
  - [x] 14.1 添加文件夹时可选递归扫描子目录
  - [x] 14.2 支持导入根目录下的多个漫画文件夹/压缩包
- [x] 15. 跨页/宽页检测与显示选项
  - [x] 15.1 检测明显横向长图并自动单页全宽显示
  - [x] 15.2 提供“跨页”或“从右页开始双页”配置
- [x] 16. 动画与当前 zoom/fit 状态一致
  - [x] 16.1 让翻页动画使用当前 zoom/pan/fit 状态
  - [x] 16.2 或提供“关闭动画”开关
- [x] 17. 页面跳转输入框即时生效
  - [x] 17.1 `DragValue` 在失去焦点或回车时跳转
- [x] 18. 鼠标前进/后退键翻页
  - [x] 18.1 在 `app.rs` 中处理额外鼠标按钮
- [x] 26. 电子书 spread 分页改造
  - [x] 26.1 壳页面使用 `#measure` 真实排版并切分 `spreads[]`
  - [x] 26.2 单页/双页模式每次只渲染当前 spread，配合 ±1 预加载
  - [x] 26.3 3D 翻页动画捕获当前 spread 而非整章
  - [x] 26.4 点击/滚轮翻页与跨章节导航
  - [x] 26.5 设置变化与窗口 resize 时重新测量并保留字符偏移
  - [x] 26.6 连续滚动模式使用 `#spread` 显示完整章节
  - [x] 26.7 移除旧的 `column-width` 横向列 CSS

## P3 — 工程精进

- [x] 19. 上传纹理后释放 CPU 端 ColorImage
  - [x] 19.1 `PageCache::get_texture` 上传后将 `image` 字段置空
  - [x] 19.2 压缩模式保留压缩数据
- [x] 20. protected_page_indices 改为 HashSet
- [x] 21. SharedRawCache 锁粒度优化
  - [x] 21.1 缩小 Mutex 持有范围
  - [x] 21.2 评估使用并发缓存结构（如 dashmap）
- [x] 22. 设置 JSON 原子写 + 备份 + 校验
  - [x] 22.1 写入临时文件后 rename
  - [x] 22.2 写入前备份旧文件
  - [x] 22.3 加载错误时提示用户而非静默 fallback
  - [x] 22.4 对 `decode_threads`、`cache_size_mb` 等做范围校验
- [x] 23. 历史记录同时保存 comic_id 与 path
  - [x] 23.1 修改 `HistoryEntry` 结构
  - [x] 23.2 匹配时优先 comic_id，找不到则按 path 兜底
- [x] 24. 项目文档与工程化
  - [x] 24.1 添加 LICENSE 文件（MIT）
  - [x] 24.2 添加 AGENTS.md
  - [x] 24.3 添加 GitHub Actions CI（fmt/clippy/test）
  - [x] 24.4 添加 CHANGELOG.md
  - [x] 24.5 清理/更新 docs/ 中过时的设计文档
- [x] 25. 增加非 GUI 集成测试
  - [x] 25.1 parser 多格式 round-trip 测试
  - [x] 25.2 storage 文件 I/O 测试
  - [x] 25.3 loader 并发与缓存行为测试

## 历史编号对照表

| 旧编号（2026-06-17 计划） | 当前编号 / 位置 |
|---|---|
| #14 书架搜索/排序 | P1-9 |
| #15 多线程解码池 | P0-4 / P3-21 |
| #16 ZIP 索引缓存 | P1-8 |
| #17 GPU 纹理压缩 | P3-19（上传后释放 CPU 端 ColorImage）/ `compress_images` 设置 |
| #18 放弃项 | 已在后续重构中重新规划 |
| #19 上传纹理后释放 CPU 端 ColorImage | P3-19 |
| #20 protected_page_indices 改为 HashSet | P3-20 |
| #21 SharedRawCache 锁粒度优化 | P3-21 |
| #22 设置 JSON 原子写 + 备份 + 校验 | P2-22 |
| #23 历史记录同时保存 comic_id 与 path | P2-23 |
| #24 项目文档与工程化 | P2-24 |
| #25 增加非 GUI 集成测试 | P2-25 |

## P2 — 电子书分页迁移到 CSS Columns（长期计划）

详细计划见 `docs/superpowers/plans/2026-06-26-migrate-ebook-to-css-columns.md`。

- [x] 27. Phase 0：原型验证
  - [x] 27.1 创建 CSS columns 单页/双页原型（`target/tmp/ebook-columns-proto.html`）
  - [x] 27.2 用多段文本 + 图片 + 表格样本验证无跨页截断/重复
  - [x] 27.3 在 wry WebView 中验证 `column-fill`、`break-inside` 行为
  - [x] 27.4 输出原型结论，确定最终 CSS 策略
- [x] 28. Phase 1：新分页器骨架
  - [x] 28.1 在 `ebook_renderer_template.rs` 中新增 `columnPaginator` 模块
  - [x] 28.2 添加 feature flag `window.ebookUseColumns` 实现新旧方案并存
  - [x] 28.3 实现 column 容器渲染、单页/双页/滚动切换、IPC 位置上报
  - [x] 28.4 Rust 侧 `ebook_renderer.rs` 兼容新 IPC 消息
- [x] 29. Phase 2：功能对齐
  - [x] 29.1 单页模式完整可用
  - [x] 29.2 双页模式完整可用
  - [x] 29.3 滚动模式完整可用
  - [x] 29.4 翻页动画（先 transform 滑动，后续评估 3D 翻转）
  - [x] 29.5 进度保存/恢复（resize、设置变更后回到近似位置）
  - [x] 29.6 目录跳转、搜索高亮
  - [x] 29.7 字体/字号/行高/边距调整实时生效
- [x] 30. Phase 3：测试与边缘情况
  - [x] 30.1 收集 10~20 本不同类型 EPUB 作为测试集
  - [x] 30.2 单页/双页/滚动 + 字体放大 + 窗口缩放全面测试
  - [x] 30.3 处理图片溢出、表格截断、特殊 CSS 冲突
  - [x] 30.4 添加 column 分页相关模板测试
- [x] 31. Phase 4：清理旧代码
  - [x] 31.1 删除 `measure` 容器和行盒测量相关样式
  - [x] 31.2 删除 `collectLineBoxes`、`findSafeEnd`、`buildClonedSpread`、`buildDoubleSpread`
  - [x] 31.3 删除 `splitSinglePage`、`splitDoublePage`、`flipper`（如不再使用）
  - [x] 31.4 更新 Rust 测试与文档
- [x] 32. Phase 5：优化
  - [x] 32.1 相邻章节预加载
  - [x] 32.2 缓存页数计算结果
  - [x] 32.3 减少 resize 时重新布局开销
  - [ ] 32.4 评估大章节分段加载

## P2 — 媒体播放（内嵌 libmpv）

详细设计/计划见 `docs/superpowers/plans/`（2026-07 系列）。

- [x] 33. 媒体播放基础
  - [x] 33.1 `openitgo-media`：libmpv 命令封装、事件泵、属性观察、OpenGL 渲染上下文
  - [x] 33.2 macOS 视频层：`CAOpenGLLayer` + `drawInCGLContext`（drawable FBO 绑定查询与 `FLIP_Y` 修正，修复有进度无画面）
  - [x] 33.3 打开/播放视频与音频，书架集成与封面生成（无头 mpv 截取视频 10% 帧、音频专辑封面）
  - [x] 33.4 播放进度毫秒级持久化与续播（复用历史记录 `char_offset`）
  - [x] 33.5 修复退出播放时间歇性段错误（`MpvPlayer::drop` 先 join 事件线程再销毁 handle）
- [x] 34. 播放控制与 OSD
  - [x] 34.1 播放/暂停、±5s/±10s 跳转、两行式全宽进度条（悬停预览目标时间、关键帧对齐拖动、松手精确跳转）
  - [x] 34.2 倍速、字幕轨切换/关闭、音轨切换、音频输出设备选择、全屏
  - [x] 34.3 音量/静音/滚轮音量与画面右上角 `CATextLayer` OSD 反馈
  - [x] 34.4 音量/倍速/输出设备全局记忆（`media_volume` / `media_speed` / `media_audio_device`）
- [x] 35. 视频层下沉到 egui 之下（方案 A）
  - [x] 35.1 全窗口透明 backbuffer（`with_transparent(true)` + `clear_color` 全透明）
  - [x] 35.2 媒体中央面板透明化（`Frame::none()`）
  - [x] 35.3 `MpvNativeView` 裸层重构：视频层插到 winit view 的 `CAMetalLayer` 的 superlayer 中、`insertSublayer:below:` 锚定
  - [x] 35.4 移除菜单停放 hack：菜单栏菜单与字幕/音轨/输出下拉框直接悬浮在视频之上

## 历史已完成项

- [x] 修复双页模式下右页无右键/拖拽响应
- [x] 解码/加载失败时给出用户可见的错误提示（占位图 + 文字）
- [x] 打开大文件时显示 loading 状态（解析阶段目前是同步的）
- [x] 验证并确保退出时阅读位置能正确恢复
- [x] 全屏时自动隐藏工具栏/进度条
- [x] 书签列表和历史列表 UI
- [x] 书架支持删除/编辑条目
- [x] 图片降采样，避免超高分辨率图撑爆内存
- [x] 点击屏幕左右半边翻页
- [x] macOS Dock 拖入打开（含应用未运行时通过 Finder / Dock 打开压缩包）
