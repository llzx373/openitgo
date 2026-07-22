# OpenItGo 隐藏 Bug 修复计划（验证用例强制）

> **For agentic workers:** 每个 Bug 按 **红灯测试 → 修复 → 绿灯 → 勾选** 顺序执行。未写出并通过「验证用例」章节中的自动化测试，不得勾选该 Bug 完成。
>
> **跟踪：** `TODO.md` #62–#72；本文件为权威勾选清单。
> **替换说明：** 本文件取代同目录旧稿中「验证偏弱」的写法；进度以本文件为准。

**Goal:** 修复 2026-07-22 审查确认的隐藏缺陷；**每个缺陷必须有可重复的自动化验证用例**（优先单元/集成测试；仅 UI/平台无法单测时才允许「半自动 + 明确手工步骤」）。

**原则：**
1. 先写失败测试（或扩展现有测试），再改生产代码。
2. 测试名见各 Bug「验证用例」表；实现时保持同名，便于 CI 与勾选对照。
3. 每完成一个 Bug：`cargo test -p <crate> <test_name过滤>` 通过 → 再跑全仓流水线 → 勾选。
4. 最小改动；UI 中文；不确定先问用户。

**全仓流水线（每个 Bug 完成后）：**
```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

**范围外：** 非 macOS 媒体/打包、播放列表、跨章搜索、标签层级、缩略图 LRU 大改、发版 0.2.0。

---

## 进度总览

| ID | Bug | 主测 crate | 状态 |
|---|---|---|---|
| B1 / #62 | `DefaultHasher` comic_id 跨版本不稳定 | openitgo-parser + app/storage | ☐ |
| B2 / #63 | 历史/书签几乎只在退出落盘 | openitgo-app | ☐ |
| B3 / #64 | 媒体→媒体换片不写进度；续播从中间起 | openitgo-app | ☐ |
| B4 / #65 | 双页 `clamp_page` 越界跳封面 | openitgo-core | ☐ |
| B5 / #66 | Webtoon 残留双页；滚轮累加器跨书残留 | openitgo-core + openitgo-app | ☐ |
| B6 / #67 | `SharedRawCache` 重复插入双计字节 | openitgo-app | ☐ |
| B7 / #68 | PDF 文档缓存无界增长 | openitgo-app | ☐ |
| B8 / #69 | 密码 path 未规范化；空密码仍重试 | openitgo-app | ☐ |
| B9 / #70 | 加密包尺寸探测无密码；导入错误静默 | openitgo-app | ☐ |
| B10 / #71 | 书架进度永远 0% | openitgo-storage + openitgo-app | ☐ |
| B11 / #72 | 文档/CHANGELOG/勾选收尾 | docs | ☐ |

---

## B1 — `stable_comic_id` 改用稳定哈希 + 迁移（TODO #62）

### 现象 / 根因
`openitgo-parser/src/traits.rs` 使用 `std::collections::hash_map::DefaultHasher`，算法随 Rust 版本可变；ID 键控 history/bookmarks/covers/comic_settings/stats。

### 修复要点
- 改用 `blake3`，对 canonicalize 后的路径字节哈希，输出 **16 位 hex**（取 hash 前 8 字节）。
- 启动时按 `path` 重算 ID，改写 library/history/bookmarks/comic_settings/reading_stats，并重命名 `covers/` 与 `covers/bookmarks/` 下文件。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `stable_comic_id_is_deterministic` | `openitgo-parser/src/traits.rs` | 同一已存在文件路径两次调用结果相等 |
| `stable_comic_id_differs_for_different_paths` | 同上 | 临时目录下两个不同文件 ID 不同 |
| `stable_comic_id_uses_canonical_path` | 同上 | 经 symlink（或等价）打开与真实路径得到同一 ID（平台允许时；不支持则 `#[cfg]` 跳过并注明） |
| `stable_comic_id_is_16_hex_chars` | 同上 | 长度 16，且全部为 `0-9a-f` |
| `migrate_comic_ids_rewrites_library_and_history` | `openitgo-app` 或 `openitgo-storage` | 临时 store：写入旧假 ID + 真实 path → 跑迁移 → 条目 `comic_id == stable_comic_id(path)`，history 同步 |
| `migrate_comic_ids_renames_cover_file` | 同上 | 旧 `covers/<old_id>.jpg` 存在 → 迁移后变为 `covers/<new_id>.jpg`，旧文件不存在 |

### 步骤

- [ ] **Step 1:** 先写上表测试（此时 ID 仍为 DefaultHasher 时，迁移测试可先只测「重算函数」；算法切换后全部绿灯）
- [ ] **Step 2:** 实现 blake3 `stable_comic_id`
- [ ] **Step 3:** 实现启动迁移并让迁移测试通过
- [ ] **Step 4:** 全仓流水线；勾选本 B1 与 `TODO.md` #62；commit

```bash
cargo test -p openitgo-parser stable_comic_id
# 迁移测试所在 crate：
cargo test -p openitgo-app migrate_comic_ids
```

---

## B2 — 历史/书签及时落盘（TODO #63）

### 现象 / 根因
`record_*_history` 只改内存；`save_history`/`save_bookmarks` 主要在 `on_exit`。崩溃丢本会话进度。

### 修复要点
- `history_dirty` / `bookmarks_dirty` + `persist_history_bookmarks()`。
- 离开 Reader/Ebook/Media、书签增删改：**立即** flush。
- 阅读中：脏且距上次 ≥ 30s 再 flush（可用可注入的时钟/`Instant` 便于测）。
- `on_exit` 保存 `Err` → 设置 `error_message`（勿再 `let _ =` 吞掉）。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `persist_history_on_leave_reader_writes_store` | `openitgo-app/src/app.rs` | 临时 JsonStore：打开漫画态 `record_reader_history` + 模拟离开视图触发 persist → 磁盘 `load_history` 含对应 `page_index` |
| `persist_bookmarks_on_add_writes_store` | 同上 | `add_bookmark`（或等价）后磁盘 bookmarks 含新条目 |
| `history_dirty_throttle_skips_flush_within_30s` | 同上 | 两次 record 间隔 &lt; 30s 且非「离开视图」路径 → 第二次不增加写次数（用计数包装或比较 mtime/内容版本）；离开视图路径不受节流 |
| `on_exit_save_failure_sets_error_message` | 同上 | store 目录只读或注入失败 → `error_message` 含「历史」或「书签」类中文提示（若难造只读，可测 `persist` 返回 Err 时 app 赋值逻辑） |

### 步骤

- [ ] **Step 1:** 写验证用例（红）
- [ ] **Step 2:** 实现 dirty/flush/错误提示（绿）
- [ ] **Step 3:** 全仓流水线；勾选 #63；commit

```bash
cargo test -p openitgo-app persist_history
cargo test -p openitgo-app persist_bookmarks
cargo test -p openitgo-app history_dirty_throttle
```

---

## B3 — 媒体换片写历史 + 自动续播从头（TODO #64）

### 现象 / 根因
`record_media_history` 仅在离开 `View::Media` 时调用；`poll_media_open` / `maybe_auto_next_media` 换片不写。Auto-next 仍按 history `char_offset` 续播，可能从半集中间起。

### 修复要点
- `poll_media_open` 在 `media_view.open` 之前：若已有打开媒体且 path 不同 → `record_media_history`（+ flush）。
- auto-next：record 当前集后，打开下一集时 **resume_ms = None / 0**（强制开头）。
- 手动打开同一文件仍可续播（行为不变）。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `record_media_history_before_switching_file` | `openitgo-app` | 构造 Media 打开态（可用 stub/`OpenMedia` 最小字段或抽纯函数 `should_record_before_media_open(old, new) + apply`）；切到新 path 后 history 中旧 path 的 `char_offset` 等于切换前 `position_ms` |
| `auto_next_forces_resume_from_start` | 同上 | 纯函数或 `poll` 前置逻辑：`force_start == true` 时传给 open 的 resume 为 `None`；即使 history 有中间进度 |
| `manual_open_still_resumes_from_history` | 同上 | `force_start == false` 时仍读取 history 续播 |

若完整 `MediaView::open` 依赖 mpv/macOS：把「是否 record / resume 参数」抽成无 FFI 纯函数再测（与 `openitgo-media` args/apply 风格一致）。

### 步骤

- [ ] **Step 1:** 抽纯函数或可测钩子 + 写红灯测试
- [ ] **Step 2:** 接线 `poll_media_open` / `maybe_auto_next_media`
- [ ] **Step 3:** 全仓流水线；勾选 #64；commit

```bash
cargo test -p openitgo-app record_media_history_before
cargo test -p openitgo-app auto_next_forces
cargo test -p openitgo-app manual_open_still_resumes
```

---

## B4 — 双页末页 clamp 跳封面（TODO #65）

### 现象 / 根因
`ReadingState::clamp_page`：双页下 `current_page > last_anchor` 时设为 `0`。

### 修复要点
越界应对齐到 `last_anchor`，封面 `0` 保持不变；偶页对齐逻辑保留。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `go_to_page_past_last_anchor_clamps_to_last_spread` | `openitgo-core/src/state.rs` | 双页 `total=7`：`go_to_page(6, 7)` → `current_page == 5`（**不是 0**） |
| `go_to_page_last_anchor_unchanged` | 同上 | `go_to_page(5, 7)` → `5` |
| `go_to_page_cover_stays_zero` | 同上 | `go_to_page(0, 7)` → `0` |
| `go_to_page_even_total_last_page` | 同上 | 双页 `total=6`：`go_to_page(5, 6)` → `5` |
| `go_to_page_aligns_even_index_to_odd_anchor` | 同上 | 保留现有：`go_to_page(4, 10)` → `3`（回归，防修坏） |

### 步骤

- [ ] **Step 1:** 添加上表测试（前两条在修复前应失败）
- [ ] **Step 2:** 修 `clamp_page`
- [ ] **Step 3:** `cargo test -p openitgo-core`；全仓流水线；勾选 #65；commit

```bash
cargo test -p openitgo-core go_to_page_past_last_anchor
cargo test -p openitgo-core go_to_page
```

---

## B5 — Webtoon 双页残留 + 滚轮累加器（TODO #66）

### 现象 / 根因
`set_mode(Webtoon)` 不关 `double_page` 标志；切回 LTR 后 `is_double_page()` 可能为 true。`page_scroll_acc` 换书/切模式不清零。

### 修复要点
- `set_mode`：若 `mode.is_webtoon()` 则 `double_page = false`。
- `open_comic` / 应用侧切模式：`page_scroll_acc = 0.0`。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `set_mode_to_webtoon_clears_double_page_flag` | `openitgo-core/src/state.rs` | LTR+双页 → `set_mode(Webtoon)` → `double_page == false` 且 `!is_double_page()` |
| `webtoon_then_ltr_does_not_restore_double_page` | 同上 | 上序后再 `set_mode(Ltr)` → `!is_double_page()`（除非用户再次打开双页） |
| `open_comic_resets_page_scroll_acc` | `openitgo-app/src/app.rs` | `page_scroll_acc = 99.0` → `open_comic`（或提取的 reset 辅助）后 `== 0.0` |
| `set_reading_mode_resets_page_scroll_acc` | 同上 | 应用层切换阅读模式后累加器为 0 |

### 步骤

- [ ] **Step 1:** 红灯测试
- [ ] **Step 2:** 修 core + app
- [ ] **Step 3:** 全仓流水线；勾选 #66；commit

```bash
cargo test -p openitgo-core set_mode_to_webtoon
cargo test -p openitgo-core webtoon_then_ltr
cargo test -p openitgo-app page_scroll_acc
```

---

## B6 — SharedRawCache 重复插入双计（TODO #67）

### 现象 / 根因
`SharedRawCache::insert` 不移除旧 key 就 `bytes += len` 且 `order.push`。

### 修复要点
插入前若存在：减旧字节、从 `order` 去掉旧项，再写入新值。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `shared_raw_cache_reinsert_same_key_does_not_double_count` | `openitgo-app/src/loader.rs` | 同 key 先插 100 字节再插 100 字节 → 内部 `bytes == 100`（需 `#[cfg(test)]` 暴露 `bytes()` 或测淘汰行为） |
| `shared_raw_cache_reinsert_replaces_content` | 同上 | 同 key 先 `b"aaa"` 再 `b"bbbb"` → `get` 为后者 |
| `shared_raw_cache_reinsert_then_evict_accounts_correctly` | 同上 | 小 `max_bytes`：重复插入后仍能按预算淘汰，不出现「假满」导致无法插入新 key |

### 步骤

- [ ] **Step 1:** 红灯测试（当前实现下 double_count 必失败）
- [ ] **Step 2:** 修 `insert`
- [ ] **Step 3:** 全仓流水线；勾选 #67；commit

```bash
cargo test -p openitgo-app shared_raw_cache_reinsert
```

---

## B7 — PDF 文档缓存有界（TODO #68）

### 现象 / 根因
`PdfDocumentCache` 无淘汰，多开大 PDF 内存涨。

### 修复要点
按总字节 LRU（建议默认上限 256 MiB）或最多 N 本；超限淘汰最久未用。可选：epoch bump 清非当前 path。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `pdf_document_cache_evicts_when_over_budget` | `openitgo-app/src/loader.rs` | 构造小 max（如 10 字节）：依次 insert A(6)、B(6) → A 被淘汰，`get(A)` 为 None，`get(B)` 有值 |
| `pdf_document_cache_get_refreshes_lru_order` | 同上 | insert A、B 后 `get(A)`，再 insert 需淘汰时淘汰 B 而非 A |
| `pdf_document_cache_rejects_or_skips_single_item_over_max` | 同上 | 单文件 &gt; max → 不插入或不驻留（与 raw cache 行为对齐并文档化） |

### 步骤

- [ ] **Step 1:** 红灯测试
- [ ] **Step 2:** 实现有界缓存
- [ ] **Step 3:** 全仓流水线；勾选 #68；commit

```bash
cargo test -p openitgo-app pdf_document_cache
```

---

## B8 — 密码 key 规范化 + 空密码不重试（TODO #69）

### 现象 / 根因
密码 `HashMap` 用原始 `PathBuf`；空字符串仍 `open_path`/`retry_password_import`。

### 修复要点
- `password_key(path) = canonicalize.unwrap_or(path)`。
- confirm 且 trim 为空：不打开、可提示；对话框可保留或清空 incorrect。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `password_key_canonicalizes_existing_path` | `openitgo-app` | 临时文件：相对路径与 canonicalize 绝对路径 → 同一 key |
| `empty_password_confirm_does_not_open_path` | 同上 | 抽纯函数 `PasswordDialogAction`：空 input + confirm → `None`/KeepDialog，**不**产生 Open/RetryImport |
| `non_empty_password_confirm_stores_and_retries` | 同上 | 非空 → StorePassword + Open/Retry |

UI `render_password_dialog` 若难测：把分支抽成：

```rust
fn password_dialog_on_confirm(input: &str, path: &Path, is_import: bool) -> PasswordConfirmOutcome
```

只测该函数即可满足「验证用例」要求。

### 步骤

- [ ] **Step 1:** 红灯测试 + 抽纯函数
- [ ] **Step 2:** 接线 UI
- [ ] **Step 3:** 全仓流水线；勾选 #69；commit

```bash
cargo test -p openitgo-app password_key
cargo test -p openitgo-app empty_password_confirm
cargo test -p openitgo-app non_empty_password_confirm
```

---

## B9 — 加密尺寸探测 + 导入错误汇总（TODO #70）

### 现象 / 根因
`sync_page_dimensions` 对 Zip/Rar 不带密码；`add_file_to_library` 的 `Err(_)` 吞掉。

### 修复要点
- 尺寸探测接受 `Option<&str>` 密码（或读 `SharedPasswords`）。
- 导入累计 `failed_imports`，结束时中文汇总进 `error_message`。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `sync_page_dimensions_zip_encrypted_with_password` | `openitgo-app`（reader 或 loader 测模块） | 真实小 PNG 加密进 zip（非 fake 无头）：有密码返回 `Some([w,h])`；无密码 `None` 或 Err 路径 |
| `sync_page_dimensions_zip_encrypted_wrong_password_returns_none` | 同上 | 错密码不 panic，返回 None |
| `import_failure_summary_counts_non_password_errors` | `openitgo-app` | 纯函数：输入若干 Result → 汇总字符串含跳过数量；密码类错误不计入该汇总（或单独计数，与产品文案一致） |

若「整图加密 zip + image crate 读头」过重：允许用未加密 zip 测「密码参数被传入」的 mock/spy；但至少有一条加密+正确密码的集成测。

### 步骤

- [ ] **Step 1:** 红灯测试
- [ ] **Step 2:** 修探测与导入汇总
- [ ] **Step 3:** 全仓流水线；勾选 #70；commit

```bash
cargo test -p openitgo-app sync_page_dimensions_zip_encrypted
cargo test -p openitgo-app import_failure_summary
```

---

## B10 — 书架真实进度（TODO #71）

### 现象 / 根因
`library_progress` 恒返回 `Progress::default()`（0%）；`LibraryEntry` 无总页数。

### 修复要点
- `LibraryEntry.page_count: Option<usize>`（serde default 兼容旧 JSON）。
- 打开/导入成功写入并 `save_library`。
- `library_progress(history, entry)`：`read = page_index+1` 或既有约定，`total = page_count`。

### 验证用例（必须全部通过）

| 测试名 | 位置 | 断言 |
|---|---|---|
| `library_entry_page_count_roundtrip_json` | `openitgo-storage` | 含 `page_count: Some(12)` 序列化再反序列化相等；缺字段旧 JSON → `None`/`0` 不炸 |
| `library_progress_uses_page_count_and_history` | `openitgo-app/src/views/library.rs` | history `page_index=2`，`page_count=10` → 进度为 0.3 或产品约定的「已读页/总页」（在测试注释写清公式） |
| `library_progress_zero_when_no_page_count` | 同上 | `page_count` 空 → 0%（或隐藏，与实现一致） |
| `open_comic_updates_library_page_count` | `openitgo-app` | 库中已有条目，opener 成功后该条目 `page_count == comic.total_pages` |

### 步骤

- [ ] **Step 1:** storage 模型测试
- [ ] **Step 2:** `library_progress` 测试 + 实现
- [ ] **Step 3:** 打开路径更新 page_count
- [ ] **Step 4:** 全仓流水线；勾选 #71；commit

```bash
cargo test -p openitgo-storage page_count
cargo test -p openitgo-app library_progress
cargo test -p openitgo-app open_comic_updates_library_page_count
```

---

## B11 — 文档收尾（TODO #72）

### 验证用例（文档门禁）

| 检查项 | 断言 |
|---|---|
| `CHANGELOG.md` `[Unreleased]` | 含本批次 Fixed/Added 条目，覆盖 B1–B10 |
| `TODO.md` | #62–#72 全部 `[x]` |
| 本文件进度总览 | 全部 ☑ |
| `docs/superpowers/README.md` | 本计划状态改为「已实现」 |
| 流水线 | 全绿 |

### 步骤

- [ ] **Step 1:** 更新 CHANGELOG / README 索引 / TODO
- [ ] **Step 2:** 全仓流水线
- [ ] **Step 3:** 勾选 B11 与 #72；commit `docs: 勾选隐藏 bug 修复批次 #62–#72`

---

## 推荐执行顺序

```
B4 → B6 → B5 → B1 → B2 → B3 → B7 → B8 → B9 → B10 → B11
```

说明：B4/B6 范围小、纯单测，适合先建立「红→绿→勾选」节奏；B1 迁移面较大放中间；B11 最后。

可并行：B4∥B6；B5∥B7；B8∥B10（注意 `LibraryEntry` 合并冲突）。

---

## 手工冒烟（批次结束，不替代自动化）

- [ ] 旧配置目录启动后书仍对得上历史/封面
- [ ] 阅读中强杀进程，再开进度接近杀前（允许 ≤30s）
- [ ] 视频 A→不回书架→B→再开 A，A 进度保留
- [ ] 自动续播下一集从 0 起，OSD 正常
- [ ] 奇数页双页拖到最后，停在末 spread
- [ ] 加密 CBZ 密码后首屏比例合理；空密码确定不空转
- [ ] 书架卡片进度非恒 0%

---

## 明确不修（无验证任务）

| 项 | 原因 |
|---|---|
| 缩略图全局 LRU | 需单独缓存设计 |
| 电子书同章多书签 | 产品决策 |
| 媒体 async 命令失败提示 | 另开可观测性批次 |
| RAR BadData/密码歧义 | 库语义限制 |
| CSS columns #54 矩阵 | 用户手工验收 |
