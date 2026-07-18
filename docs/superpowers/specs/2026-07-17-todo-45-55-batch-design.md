# TODO #45–#55 批次设计文档

日期：2026-07-17
状态：已批准（用户于 2026-07-17 批准）
范围：TODO.md "新一轮待办" 中 #45、#46、#47、#49、#51、#52、#53、#54、#55（#48/#50 已完成）

## 背景与已确认的范围决策

上一批次（#43/#44/#48/#50/#56/#60）已在 main 串行完成并推送（c07d88b..c2f01fc）。
本批次 9 项经用户确认的范围：

- **#45 媒体杂项**：5 个子项全做（截图、AB 循环、mpv 章节导航、循环播放、倍速微调）。
- **#49 书架增强**：3 个子项全做（分组/标签、阅读统计、书签缩略图），拆 3 个任务串行。
- **#52 电子书菜单遮盖**：采用**停放方案**——菜单/弹层打开时隐藏 wry webview，
  区域以阅读背景色的 egui 面板填充，关闭即恢复；用户接受开菜单瞬间正文区变纯色的代价。
  原生子窗口方案（egui 多视口）因 0.29 不成熟、风险高被否决。
- **#53 排版**：修超高图片跨页拆分与宽表格溢出两项（具体可验证），
  `column-*`/`position:fixed` 冲突尽力清洗；**#54** 准备可勾选验收清单由用户真机走查
  （本质是人工 QA，无法自动化代劳）。
- **#46** 密码仅会话内缓存、**不持久化**。
- **#47** 旋转随 `ComicReadingSettings` 每书持久化（#48 机制的自然延伸）。
- **#55** 纯评估：实测后出做/不做结论报告。

## 执行结构

方案 A（用户批准）：一份 spec + 一份实现计划，10 个任务 SDD 串行，main 直提，
每个任务 commit 前跑完整流水线（fmt/check/test/clippy -D warnings）。
TODO/CHANGELOG 由控制器统一收尾。

## 已核实的技术事实

- zip 锁定版本 2.4.2：`by_index_decrypt` / `by_name_decrypt` 存在（`read.rs:999,1083`），
  密码错误返回 `ZipError::InvalidPassword`；`aes-crypto` feature 存在（纯 Rust，无系统依赖）。
- unrar 0.5.8：`Archive::with_password(file, password)`（`archive.rs:61`）。
- 倍速档位在 `openitgo-app/src/views/media.rs:419`（`[0.5, 1.0, 1.5, 2.0]` 循环）；
  数字键 1-4 直选在 `app.rs:2095`。
- `LibraryEntry`（models.rs:274）无标签字段；书签列表（library.rs:439-）为纯文本网格，无图。
- `ComicReadingSettings`（models.rs:349）为 Copy 结构，`#[serde(default)]` 需确认逐字段兼容
  （加字段时验证旧 comic_settings.json 反序列化）。
- 帮助菜单在 `app.rs:1647`；字幕下拉模式在 `app.rs:740`（#44 刚扩展过，可照抄）。
- mpv 侧尚无 chapter/loop/screenshot/ab-loop 封装；`PlayerState` 观察属性模式可照抄
  `sub-delay`（Task #44 新增，id 8，下一可用 id 9 起）。

## 任务设计

### Task 1：#51 快捷键一览面板

- 帮助菜单（app.rs:1647）新增"快捷键一览"，打开一个 egui 窗口（`show_shortcuts: bool` 状态）。
- 内容两部分：可配置动作（遍历 `Settings.shortcuts` 各字段的当前键位，动作名中文）；
  硬编码键（阅读器：点击左右半边翻页、Ctrl+滚轮缩放、双击切 fit、鼠标侧键；
  媒体：Space、←/→、↑/↓、M、F、Z/X、数字 1-4、[ ]（本批次新增）、Home/End 为可配置不重复列）。
- 面板只读展示，不重复设置面板的编辑功能。
- 测试：动作名/键位列表生成为纯函数（输入 `&Shortcuts` 输出行列表），单测覆盖。

### Task 2：#45 媒体杂项（5 分步，一个 commit）

1. **倍速微调**：`[`/`]` 键与倍速菜单项 ±0.25 步进（clamp 0.1–16，与
   `settings.media_speed` 校验一致），OSD `倍速 1.25x`；原四档循环与数字键直选保留。
   `next_speed` 循环逻辑不动。
2. **循环播放**：菜单开关项（勾选态），mpv `loop-file` 属性 `inf`/`no`（async set）；
   状态存 app 侧 bool，不持久化；OSD `循环播放 开/关`。
3. **截图**：菜单"截图"→ mpv `screenshot-to-file <path>`（async）；保存目录
   `dirs::picture_dir()` 下 `OpenItGo/`（不存在则创建，fallback 配置目录）；文件名
   `<标题>-<yyyyMMdd-HHmmss>.png`；OSD `已保存截图：<路径>`，失败写 `error_message`。
4. **AB 循环**：菜单"设置 A 点"/"设置 B 点"/"取消 AB 循环"（或一个三态按钮 +
   快捷键 A）→ mpv `ab-loop-a`/`ab-loop-b` 属性（当前秒数或 `no`，async set）；
   OSD 反馈（`A 点 01:23`）；进度条标记出范围。
5. **章节导航**：观察 `chapter`（i64）入 `PlayerState`；在 `FILE_LOADED` 与
   `chapter` 变化时经 `get_property_async`（node，参照 `audio-device-list` 的
   解析模式，另分配一个 reply userdata id）拉取 `chapter-list` 入
   `PlayerState.chapters: Vec<String>`（标题列表）；菜单"上一章/下一章"
   （`["add", "chapter", "-1" / "1"]` async），`chapters` 为空时禁用；
   OSD 章节标题。
- 测试：倍速步进纯函数、截图路径/文件名生成纯函数、AB 状态机纯函数；
  FFI 层不测（#58 另立）。

### Task 3：#46 加密压缩包密码

- parser：
  - Cargo.toml：zip features 加 `"aes-crypto"`。
  - 新增错误变体 `ParseError::PasswordRequired` 与 `ParseError::PasswordIncorrect`
    （或一个 `Encrypted { needs_password: bool }`，实现时选更贴合现有错误枚举的形式）。
  - ZIP：读取加密条目时先用无密码路径探测，命中加密/InvalidPassword 后要求密码，
    走 `by_index_decrypt`；密码错误映射 `InvalidPassword` → `PasswordIncorrect`。
  - RAR：`Archive::with_password(file, password)`；无密码打开加密包时 unrar 的
    错误/空列表行为需在实现时实测分类（unrar 0.5 对加密头返回 OpenError 的哪种）。
  - `parse` 增加带密码入口（如 `parse_with_password(path, Option<&str>)`，
    原 `parse(path)` 保持签名转调 None）。
- app：
  - 打开流程捕获 PasswordRequired/Incorrect → 弹 egui 密码对话框
    （`password` 掩码输入框），确认后带密码重试；取消则放弃打开。
  - 会话缓存 `HashMap<PathBuf, String>`（App 字段，不落盘）；同一文件重开/封面生成
    先查缓存。
  - 书架批量导入遇加密文件：逐个弹同一对话框；用户取消则跳过该文件并汇总提示。
- 测试：构造加密 ZIP fixture（测试内用 zip crate 写加密包）验证
  Required/Incorrect/正确密码三分支；RAR 无法构造则分类逻辑单测 + 人工走查项。

### Task 4：#47 图片旋转

- core：`ReadingState` 加 `rotation: u16`（0/90/180/270），方法 `rotate_cw()`（+90 取模 360）。
- 渲染：reader 绘制时对纹理做 90° 步进旋转（egui UV/Shape 旋转，参照现有
  fit 计算的宽高互换）；宽页检测与 double-page 布局用旋转后的有效宽高。
- 入口：工具栏 + 阅读菜单"旋转 90°"；显示当前角度。
- 持久化：`ComicReadingSettings` 加 `rotation: u16`（serde default 0；实现期由 u8 改 u16——270 超出 u8 范围，见实现记录）；
  打开应用与快照保存沿用 #48 机制（apply 顺序：mode → double_page → fit → rotation）。
- 测试：角度步进/取模、旋转后宽高比互换对宽页判定影响、旧 comic_settings.json
  无 rotation 字段反序列化兼容。

### Task 5：#49a 书架标签

- storage：`LibraryEntry` 加 `tags: Vec<String>`（`#[serde(default)]`，旧文件兼容）。
- UI：书架条目右键菜单加"编辑标签…"（对话框，逗号分隔输入）；
  书架顶部在类型过滤旁加标签过滤（全部标签去重后的 selectable chips，
  单选即够，多选 YAGNI）；搜索框同时匹配标签。
- 测试：标签过滤/搜索匹配纯函数；旧 library.json 无 tags 反序列化。

### Task 6：#49b 阅读统计

- storage：`reading_stats.json`：`HashMap<String /*comic_id*/, ReadingStat>`，
  `ReadingStat { total_seconds: u64, first_read_at: u64, last_read_at: u64 }`；
  原子写/备份沿用 json_store 模式。
- 采集：阅读视图（漫画/电子书/媒体均算）打开期间每 30s 累计一次增量并落盘；
  退出/换书时补落最后一次。App 侧记录进入时间戳。
- 展示：书架新 tab 或菜单入口"阅读统计"：总时长、条目数、每书时长排行
  （附标题、格式化 `X 小时 Y 分`）。不回填历史数据（从启用起累计）。
- 测试：增量聚合纯函数、格式化函数、stats 文件 round-trip/缺失/损坏。

### Task 7：#49c 书签缩略图

- 创建书签时：将当前页缩略图（复用封面生成的缩放尺寸，如 80×120 约束内）
  存 `covers/bookmarks/<comic_id>-p<page>.jpg`（电子书书签不生成，回退封面）。
- 书签列表行首显示缩略图（egui 纹理加载走现有 cover_loader 通道扩展或
  独立小 loader，实现时选侵入小的）；找不到页缩略图 → 封面 → 占位色块。
- 删除书签/删除书籍时清理对应缩略图文件。
- 测试：缩略图路径生成纯函数、清理逻辑（临时目录）。

### Task 8：#52 电子书菜单遮盖停放

- `menu_overlay_open(ctx)` 为真且当前视图 `View::Ebook` 时：调用 wry 0.55 的
  `WebView::set_visible(false)`（已核实存在，`wry-0.55.1/src/lib.rs:2154`）隐藏 webview，
  该区域由 egui 侧以当前电子书主题背景色填充；菜单关闭后 `set_visible(true)` 恢复。
- 注意与现有 media 视图 `menu_overlay_open` 用法（全屏工具栏保活）互不干扰；
  每帧调用 set_visible 需做状态去重（记录已应用可见性，避免重复 IPC）。
- 验证：截图取证（菜单打开时菜单文字像素可见、正文区为背景色而非 webview 内容），
  参照 probe 系列方法写 `probe_ebook_menu.rs` 诊断示例或手动走查项（实现时评估）。

### Task 9：#53 排版修复（高图/宽表格/样式冲突）

- 超高图片跨页拆分：`render_chapter_html` 注入的 CSS 给 `img` 加
  `max-height: 100vh`（按视口）约束 + `break-inside: avoid`；超过一屏的图允许
  缩放而非拆分（拆分位图在 HTML 内不可行，缩放是正解）；模板测试断言注入规则。
- 宽表格溢出：`table { max-width: 100%; } td/th { overflow-wrap: anywhere; }`；
  仍超宽的 `pre` 加横向滚动（`overflow-x: auto`）。
- 样式冲突清洗：sanitize 阶段对 EPUB 自带 `column-*` 与 `position: fixed/absolute`
  的 `style` 属性做剥离（白名单式清洗，记录被剥离计数供调试）；
  已注入的书籍 CSS（#38/#39 引入）加 scope 前缀防与分页器 CSS 冲突——实现时先评估
  现状冲突面，能小则小。
- 测试：合成 HTML fixture（含超高 img / 宽 table / 冲突 style）经
  `render_chapter_html` 后断言改写结果；模板测试照旧。

### Task 10：#54 验收清单 + #55 大章节评估（一个任务，两份文档）

- #54：按 `docs/superpowers/reports/2026-06-26-css-columns-test-plan.md` 的
  6 类书 × 9 项操作生成可勾选验收清单（markdown，存入 reports/，
  文件名 `2026-07-17-css-columns-acceptance-checklist.md`），
  内嵌 #53 修复项与 #52 停放项为对应验证点；交用户真机走查勾选。
- #55：找/造大章节样本（≥500KB HTML 或 ≥3000 段落），测量首次布局耗时、
  resize 重排耗时、内存占用（Activity Monitor / 内存仪表），写评估报告
  `2026-07-17-large-chapter-loading-eval.md`：给出分段加载做/不做的结论
  （阈值依据：布局 >1s 或内存明显异常则建议做，并附建议方案要点）。

## 数据模型变更汇总

| 文件 | 变更 | 兼容性 |
|---|---|---|
| `settings.json` | 无 | — |
| `library.json` | `LibraryEntry.tags: Vec<String>` | `#[serde(default)]` |
| `comic_settings.json` | `ComicReadingSettings.rotation: u16` | `#[serde(default)]`（需逐字段确认） |
| `reading_stats.json` | 新增 | 新文件 |
| `covers/bookmarks/` | 新目录 | 新目录 |
| 密码缓存 | App 内存 `HashMap<PathBuf, String>` | 不持久化 |

## 风险与缓解

- **#52 停放闪烁**：用户已接受；缓解为填充阅读背景色而非黑屏。
- **#46 unrar 加密头错误分类**：unrar 0.5 对加密包的错误形态实现时实测；
  缓解为分类逻辑独立纯函数 + 保守映射（宁可误报需要密码，不可静默失败）。
- **#53 书籍 CSS scope**：全量清洗可能误伤正常排版；限定只剥离 `column-*` 与
  `position:*`，计数可观测。
- **#49b 精度**：应用退出丢失最后 <30s 增量，可接受；崩溃丢一次增量，可接受。
- **zip aes-crypto 依赖**：纯 Rust（aes/hmac/sha1/pbkdf2），无系统库，CI 无影响。

## 全局约束（每个任务 brief 复用）

- 提交前必须通过完整验证流水线：`cargo fmt --all`、`cargo check --workspace`、
  `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`。
- UI 文本中文（专名/技术标识符除外）。
- 最小改动；公开接口变更同步测试；涉及 AGENTS.md 已记录机制时同步更新。
- 不修改 TODO.md 与 CHANGELOG.md（控制器统一收尾）。
- mpv 命令/属性一律 async API。
- commit message 中文 conventional commits，总结改动与涉及 crate。

## 测试策略

每任务 TDD；纯函数优先（倍速步进、截图路径、AB 状态机、密码错误分类、角度数学、
标签过滤、统计聚合、HTML 改写断言）。UI 路径无法 headless 验证的，列入人工走查清单，
与 #54 验收清单合并交付用户。
