> **Status:** 已实现。设计与当前代码基本一致。
>
> **注意：** 文档目标中的 TODO #14 为历史编号，详见 `TODO.md` 中的「历史 TODO 编号对照表」。

# 书架搜索/排序设计

## 目标
为 rustReader 的书架页增加搜索过滤和排序能力（TODO #14）。

## 范围
- 仅影响「漫画」标签页；历史/书签保持现有展示。
- 搜索词为会话级，不持久化。
- 排序方式持久化到 `Settings`。

## 数据模型变更

### `LibraryEntry`
在 `rust-reader-storage/src/models.rs` 中新增字段：

```rust
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64, // 新增：添加时的 Unix 秒
}
```

- 新增条目时写入 `SystemTime::now()` 的 Unix 秒。
- 旧数据反序列化缺失该字段时，默认 `0`。

### `LibrarySort`
新增枚举并加入 `Settings`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    LastRead,
    Title,
    Added,
}
```

- `LastRead`：按 `HistoryEntry.last_read_at` 的最大值降序，无历史为 `0`。
- `Title`：按 `LibraryEntry.title` 字母序升序。
- `Added`：按 `LibraryEntry.added_at` 降序。

## UI 变更

在 `rust-reader-app/src/views/library.rs` 顶部工具栏加入：

1. 搜索单行输入框（hint：「搜索漫画」）。
2. 排序下拉框（最近阅读 / 标题 / 添加时间）。

控件仅在 `LibraryMode::Library` 下显示。使用现有 egui 模式（`TextEdit::singleline`、`ComboBox::from_id_salt`）。

## 过滤与排序逻辑

`LibraryView` 提供：

```rust
fn filtered_entries(
    &self,
    history: &History,
    sort: LibrarySort,
) -> Vec<(usize, &LibraryEntry)>
```

- 先用 `search_query` 忽略大小写过滤 `title`。
- 再按 `sort` 排序。
- 返回 `(original_index, entry)`，渲染与回调都使用 `original_index`。

## 回调索引处理

`render_library` 遍历 `filtered_entries`：
- 编辑/删除/打开时传递 `original_index`。
- `app.rs` 中的回调无需修改，因为收到的已经是原始索引。

## 测试计划

在 `rust-reader-app/src/views/library.rs` 新增：

1. `test_search_filters_by_title_case_insensitive`
2. `test_sort_by_title`
3. `test_sort_by_added_time`
4. `test_sort_by_last_read_uses_history`

验证 `cargo clippy --workspace -- -D warnings && cargo test --workspace` 通过。

## 变更文件

- `rust-reader-storage/src/models.rs`
- `rust-reader-app/src/views/library.rs`
- `rust-reader-app/src/app.rs`
- `TODO.md`
