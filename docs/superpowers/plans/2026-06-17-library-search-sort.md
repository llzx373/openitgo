> **Status:** 已实现。书架搜索、排序功能已与代码一致。
>
> **注意：** 本文档中的 TODO 编号（#14）为历史编号，详见 `TODO.md` 中的「历史 TODO 编号对照表」。后续又在书架中增加了封面补生成、已删除检测、清理按钮、菜单栏入口等功能，这些不在本文档范围内。

# 书架搜索/排序实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在书架页为漫画列表添加搜索过滤与排序功能，排序方式持久化到设置。

**Architecture:** 在 `LibraryEntry` 增加 `added_at` 时间戳；在 `Settings` 增加 `library_sort` 枚举；`LibraryView` 根据搜索词和排序方式生成 `(original_index, &LibraryEntry)` 列表，渲染与回调均使用原始索引。

**Tech Stack:** Rust, egui, serde_json, `rust-reader-storage`, `rust-reader-app`

---

## File Structure

- `rust-reader-storage/src/models.rs`
  - 新增 `added_at: u64` 到 `LibraryEntry`。
  - 新增 `LibrarySort` 枚举并加入 `Settings`。
  - 更新默认值与序列化测试。
- `rust-reader-app/src/views/library.rs`
  - 在 `LibraryView` 增加 `search_query: String`。
  - 在工具栏渲染搜索框和排序下拉框。
  - 实现 `filtered_entries(...)` 并用于 `render_library`。
  - 新增搜索/排序单元测试。
- `rust-reader-app/src/app.rs`
  - `add_folder_to_library` 写入 `added_at`。
  - 把 `settings.library_sort` 传递给 `LibraryView::ui`。
- `TODO.md`
  - 标记 #14 完成。

---

## Task 1: 扩展 `LibraryEntry` 数据模型

**Files:**
- Modify: `rust-reader-storage/src/models.rs:81-87`
- Test: `rust-reader-storage/src/models.rs:120-156`

- [ ] **Step 1: 给 `LibraryEntry` 添加 `added_at`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LibraryEntry {
    pub comic_id: String,
    pub title: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub added_at: u64,
}

impl Default for LibraryEntry {
    fn default() -> Self {
        Self {
            comic_id: String::new(),
            title: String::new(),
            path: PathBuf::new(),
            cover_path: None,
            added_at: 0,
        }
    }
}
```

- [ ] **Step 2: 修复现有测试中的构造**  
将 `test_library_serialize` 中的 `LibraryEntry` 构造补全 `added_at: 0`。

- [ ] **Step 3: 运行 storage 测试**

Run: `cargo test -p rust-reader-storage`
Expected: PASS

- [ ] **Step 4: 提交**

```bash
git add rust-reader-storage/src/models.rs
git commit -m "feat(storage): add added_at to LibraryEntry"
```

---

## Task 2: 添加 `LibrarySort` 并加入 `Settings`

**Files:**
- Modify: `rust-reader-storage/src/models.rs:1-37`
- Test: `rust-reader-storage/src/models.rs:120-156`

- [ ] **Step 1: 定义 `LibrarySort` 枚举**

在 `Theme` 之后添加：

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    LastRead,
    Title,
    Added,
}
```

- [ ] **Step 2: 在 `Settings` 中添加 `library_sort`**

```rust
pub struct Settings {
    pub theme: Theme,
    pub default_mode: ReadingMode,
    pub default_fit: FitMode,
    pub double_page: bool,
    pub cache_size_mb: u32,
    pub window_size: (f32, f32),
    pub show_toolbar: bool,
    pub show_statusbar: bool,
    pub invert_scroll: bool,
    pub background_color: [u8; 3],
    pub shortcuts: Shortcuts,
    pub library_sort: LibrarySort,
}
```

并在 `Default for Settings` 中加入 `library_sort: LibrarySort::default()`。

- [ ] **Step 3: 更新 settings roundtrip 测试**  
在 `test_settings_roundtrip_with_background_color` 中显式设置 `s.library_sort = LibrarySort::Title;`，确保序列化往返包含新字段。

- [ ] **Step 4: 运行 storage 测试**

Run: `cargo test -p rust-reader-storage`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add rust-reader-storage/src/models.rs
git commit -m "feat(storage): add LibrarySort to Settings"
```

---

## Task 3: 在 `app.rs` 写入添加时间

**Files:**
- Modify: `rust-reader-app/src/app.rs:673-696`

- [ ] **Step 1: 在 `add_folder_to_library` 中设置 `added_at`**

将：

```rust
let entry = rust_reader_storage::models::LibraryEntry {
    comic_id: comic.id.clone(),
    title: comic.title.clone(),
    path: path.clone(),
    cover_path: None,
};
```

改为：

```rust
let added_at = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
let entry = rust_reader_storage::models::LibraryEntry {
    comic_id: comic.id.clone(),
    title: comic.title.clone(),
    path: path.clone(),
    cover_path: None,
    added_at,
};
```

- [ ] **Step 2: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(app): set added_at when adding folder to library"
```

---

## Task 4: 在 `LibraryView` 中实现搜索/排序

**Files:**
- Modify: `rust-reader-app/src/views/library.rs:1-140`

- [ ] **Step 1: 引入 `LibrarySort` 并扩展 `LibraryView`**

将 `use` 改为：

```rust
use rust_reader_storage::models::{Bookmarks, History, Library, LibraryEntry, LibrarySort};
```

将 `LibraryView` 改为：

```rust
pub struct LibraryView {
    pub library: Library,
    pub mode: LibraryMode,
    pub search_query: String,
    edit_buffer: Option<(usize, String)>,
    pending_delete: Option<usize>,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            library: Library::default(),
            mode: LibraryMode::Library,
            search_query: String::new(),
            edit_buffer: None,
            pending_delete: None,
        }
    }
}
```

- [ ] **Step 2: 更新 `ui` 签名以接收 `library_sort`**

```rust
pub fn ui(
    &mut self,
    ui: &mut egui::Ui,
    history: &History,
    bookmarks: &Bookmarks,
    library_sort: LibrarySort,
    callbacks: LibraryCallbacks<'_>,
) {
```

- [ ] **Step 3: 在工具栏加入搜索框和排序下拉框**

在 `ui.horizontal` 内部、模式 tabs 之后、`ui.with_layout` 之前加入：

```rust
if self.mode == LibraryMode::Library {
    ui.separator();
    ui.add(
        egui::TextEdit::singleline(&mut self.search_query)
            .hint_text("搜索漫画"),
    );
    egui::ComboBox::from_id_salt("library_sort")
        .selected_text(sort_label(library_sort))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut self.sort, LibrarySort::LastRead, "最近阅读");
            ui.selectable_value(&mut self.sort, LibrarySort::Title, "标题");
            ui.selectable_value(&mut self.sort, LibrarySort::Added, "添加时间");
        });
}
```

> 注意：这里需要把排序状态保存在某个可变位置。因为 `library_sort` 来自 `Settings`，是持久化的，而 `ComboBox` 需要 `&mut LibrarySort`。可以让 `LibraryView::ui` 接收 `&mut LibrarySort`，并在退出时由 `app.rs` 保存 `Settings` 实现持久化。

将 `ui` 签名改为：

```rust
pub fn ui(
    &mut self,
    ui: &mut egui::Ui,
    history: &History,
    bookmarks: &Bookmarks,
    library_sort: &mut LibrarySort,
    callbacks: LibraryCallbacks<'_>,
) {
```

并在工具栏中直接传入 `library_sort`。

- [ ] **Step 4: 实现 `filtered_entries` 和排序辅助函数**

在 `LibraryView` 中新增：

```rust
fn filtered_entries<'a>(
    &'a self,
    history: &'a History,
    sort: LibrarySort,
) -> Vec<(usize, &'a LibraryEntry)> {
    let query = self.search_query.trim().to_lowercase();
    let mut entries: Vec<(usize, &LibraryEntry)> = self
        .library
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| query.is_empty() || e.title.to_lowercase().contains(&query))
        .collect();

    entries.sort_by(|(_, a), (_, b)| match sort {
        LibrarySort::Title => a.title.cmp(&b.title),
        LibrarySort::Added => b.added_at.cmp(&a.added_at),
        LibrarySort::LastRead => {
            let last_a = last_read_at(history, &a.comic_id);
            let last_b = last_read_at(history, &b.comic_id);
            last_b.cmp(&last_a)
        }
    });

    entries
}
```

并在模块级添加：

```rust
fn last_read_at(history: &History, comic_id: &str) -> u64 {
    history
        .entries
        .iter()
        .filter(|h| h.comic_id == comic_id)
        .map(|h| h.last_read_at)
        .max()
        .unwrap_or(0)
}

fn sort_label(sort: LibrarySort) -> &'static str {
    match sort {
        LibrarySort::LastRead => "最近阅读",
        LibrarySort::Title => "标题",
        LibrarySort::Added => "添加时间",
    }
}
```

- [ ] **Step 5: 在 `render_library` 中使用过滤后的列表**

将 `render_library` 改为接收 `history: &History` 和 `sort: LibrarySort`，并在方法开头：

```rust
let entries = self.filtered_entries(history, sort);
if entries.is_empty() {
    ui.label("没有匹配的漫画。");
    return;
}
```

循环改为：

```rust
for (original_idx, entry) in entries {
    // ... 使用 original_idx 替代 idx
}
```

所有回调、编辑/删除确认都使用 `original_idx`。

- [ ] **Step 6: 更新 `match self.mode` 调用**

```rust
match self.mode {
    LibraryMode::Library => self.render_library(ui, history, *library_sort, callbacks),
    LibraryMode::History => self.render_history(ui, history, callbacks),
    LibraryMode::Bookmarks => self.render_bookmarks(ui, bookmarks, callbacks),
}
```

- [ ] **Step 7: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 8: 提交**

```bash
git add rust-reader-app/src/views/library.rs
git commit -m "feat(library): add search and sort UI with filtered view"
```

---

## Task 5: 在 `app.rs` 传入可变的 `library_sort`

**Files:**
- Modify: `rust-reader-app/src/app.rs:157-169`

- [ ] **Step 1: 修改 `self.library_view.ui` 调用**

将调用改为：

```rust
self.library_view.ui(
    ui,
    &self.history,
    &self.bookmarks,
    &mut self.settings.library_sort,
    LibraryCallbacks {
        on_open_library: &mut |idx| open_idx = Some(idx),
        on_open_path: &mut |path| open_path = Some(path),
        on_add: &mut || add_requested = true,
        on_delete_bookmark: &mut |idx| delete_bookmark_idx = Some(idx),
        on_update_title: &mut |idx, title| update_title = Some((idx, title)),
        on_delete_library: &mut |idx| delete_library_idx = Some(idx),
    },
);
```

- [ ] **Step 2: 检查编译**

Run: `cargo check -p rust-reader-app`
Expected: no errors

- [ ] **Step 3: 提交**

```bash
git add rust-reader-app/src/app.rs
git commit -m "feat(app): wire settings.library_sort into LibraryView"
```

---

## Task 6: 为搜索/排序添加单元测试

**Files:**
- Modify: `rust-reader-app/src/views/library.rs:229-266`

- [ ] **Step 1: 写搜索过滤测试**

```rust
#[test]
fn test_search_filters_by_title_case_insensitive() {
    let mut view = LibraryView::default();
    view.library = Library {
        entries: vec![
            LibraryEntry {
                comic_id: "a".to_string(),
                title: "Alpha Comic".to_string(),
                path: PathBuf::from("/a"),
                cover_path: None,
                added_at: 1,
            },
            LibraryEntry {
                comic_id: "b".to_string(),
                title: "Beta Comic".to_string(),
                path: PathBuf::from("/b"),
                cover_path: None,
                added_at: 2,
            },
        ],
    };
    view.search_query = "alpha".to_string();
    let filtered = view.filtered_entries(&History::default(), LibrarySort::Title);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].1.title, "Alpha Comic");
}
```

- [ ] **Step 2: 写按标题排序测试**

```rust
#[test]
fn test_sort_by_title() {
    let mut view = LibraryView::default();
    view.library = Library {
        entries: vec![
            LibraryEntry {
                comic_id: "b".to_string(),
                title: "Beta".to_string(),
                path: PathBuf::from("/b"),
                cover_path: None,
                added_at: 0,
            },
            LibraryEntry {
                comic_id: "a".to_string(),
                title: "Alpha".to_string(),
                path: PathBuf::from("/a"),
                cover_path: None,
                added_at: 0,
            },
        ],
    };
    let sorted = view.filtered_entries(&History::default(), LibrarySort::Title);
    assert_eq!(sorted[0].1.title, "Alpha");
    assert_eq!(sorted[1].1.title, "Beta");
}
```

- [ ] **Step 3: 写按添加时间排序测试**

```rust
#[test]
fn test_sort_by_added_time() {
    let mut view = LibraryView::default();
    view.library = Library {
        entries: vec![
            LibraryEntry {
                comic_id: "old".to_string(),
                title: "Old".to_string(),
                path: PathBuf::from("/old"),
                cover_path: None,
                added_at: 100,
            },
            LibraryEntry {
                comic_id: "new".to_string(),
                title: "New".to_string(),
                path: PathBuf::from("/new"),
                cover_path: None,
                added_at: 200,
            },
        ],
    };
    let sorted = view.filtered_entries(&History::default(), LibrarySort::Added);
    assert_eq!(sorted[0].1.title, "New");
    assert_eq!(sorted[1].1.title, "Old");
}
```

- [ ] **Step 4: 写按最近阅读排序测试**

```rust
#[test]
fn test_sort_by_last_read_uses_history() {
    let mut view = LibraryView::default();
    view.library = Library {
        entries: vec![
            LibraryEntry {
                comic_id: "recent".to_string(),
                title: "Recent".to_string(),
                path: PathBuf::from("/recent"),
                cover_path: None,
                added_at: 0,
            },
            LibraryEntry {
                comic_id: "old".to_string(),
                title: "Old Read".to_string(),
                path: PathBuf::from("/old"),
                cover_path: None,
                added_at: 0,
            },
        ],
    };
    let history = History {
        entries: vec![
            HistoryEntry {
                comic_id: "old".to_string(),
                volume_index: 0,
                page_index: 0,
                last_read_at: 100,
            },
            HistoryEntry {
                comic_id: "recent".to_string(),
                volume_index: 0,
                page_index: 0,
                last_read_at: 300,
            },
        ],
    };
    let sorted = view.filtered_entries(&history, LibrarySort::LastRead);
    assert_eq!(sorted[0].1.comic_id, "recent");
    assert_eq!(sorted[1].1.comic_id, "old");
}
```

- [ ] **Step 5: 运行 app 测试**

Run: `cargo test -p rust-reader-app`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add rust-reader-app/src/views/library.rs
git commit -m "test(library): add search and sort tests"
```

---

## Task 7: 全量验证与 TODO 更新

**Files:**
- Modify: `TODO.md`

- [ ] **Step 1: 运行完整检查**

Run: `cargo fmt -- --check && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all PASS

- [ ] **Step 2: 更新 TODO.md**

将 `- [ ] 14. 书架搜索/排序` 改为 `- [x]`。

- [ ] **Step 3: 提交并推送**

```bash
git add TODO.md
git commit -m "chore: mark #14 library search/sort as done"
git push
```

---

## Self-Review

- **Spec coverage:**
  - `added_at` 时间戳：Task 1 + Task 3
  - `LibrarySort` 枚举与持久化：Task 2 + Task 5
  - 搜索框/排序下拉 UI：Task 4
  - 过滤与排序逻辑：Task 4
  - 回调使用原始索引：Task 4
  - 单元测试：Task 6
  - TODO 更新：Task 7
- **Placeholder scan:** 无 TBD/占位符。
- **Type consistency：** `LibrarySort` 名称在模型、视图、计划中一致；`filtered_entries` 签名使用 `&History` 与 `LibrarySort`。
