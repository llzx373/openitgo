# OpenItGo 状态机、不变式与转换审计

本文是当前代码的状态机单一记录。范围包括显式 `enum` 状态机，以及由
`Option`、通道、原子标志、代次、快照和对话框共同组成的隐式状态机。这里的
“终态”指业务结果已经确定；对象被移除、线程稍后退出和文件系统收尾是另一件事，
不能混为一谈。

## 1. 已确认问题与修复优先级

| 优先级 | 问题组 | 实际可达的无效状态/转换 | 影响 | 状态 |
|---|---|---|---|---|
| P0 | FM-1：部分完成被误判为完整成功 | 嵌套跳过或目录读/建失败后仍整删 Move 源；已有目录/失败目标进入整体撤销记录 | 源数据或目标已有数据被删除 | 已修复并回归 |
| P1 | FM-2：失败重试改变操作语义 | 子项被提升为顶层源导致目录扁平化；重试丢失过滤与校验选项 | 写入错误位置，或搬运原本被排除的文件 | 已修复并回归 |
| P1 | OPEN-1：旧打开任务覆盖新请求 | 漫画、电子书、分流、密码探测可同时在途；旧密码探测完成后再次 `open_path` | 新内容被旧内容替换，视图与目标路径不一致 | 已修复并回归 |

### FM-1：完成、成功与可撤销必须分开

**可达路径与证据**

- `ConflictMode::Skip` 下移动目录，且目标已有同名子文件：旧实现只在顶层记录
  跳过，递归内部的 `Resolve::Skip` 不置 `drop_source_unsafe`。递归返回 `Ok` 后，
  Move 将整个源目录移入回收站，包括明确跳过的文件。
- `ConflictMode::Overwrite` 下把空目录移动到同名普通文件：旧实现记录
  `create_dir_all` 失败但仍返回 `Ok`，随后删除空源目录。
- Copy 的 `written` 旧判定只检查 `copy_recursive(...).is_ok()`；因此缺失源、子项失败、
  覆盖写和目录合并都会记录整个顶层目标。撤销 Copy 会把该目标整体移入回收站；
  对已有目录的合并而言，这会连同合并前已有数据一起删除。
- 修复前实测：
  `move_skip_nested_conflict_preserves_source` 在“跳过项的源不得被删除”处失败；
  `move_directory_onto_file_failure_preserves_source` 在“目标创建失败时不得删除源目录”
  处失败；`copy_missing_source_does_not_record_undo_target` 与
  `copy_directory_merge_does_not_record_preexisting_target_for_undo` 均发现非空
  `written`。

**修复后的安全规则**

- 任何目录创建、目录读取、目录项读取、文件复制/校验失败或嵌套跳过，都使当前
  顶层 Move 源不可整体删除。
- `copy_recursive` 返回 `Ok` 只表示“未因取消中断”，不表示“无错误”。调用方以
  本顶层源前后的错误数差判定 `clean`。
- 只有 `clean && !dst_existed` 的新目标才进入整体撤销记账。覆盖、合并和部分失败
  不提供整体撤销；这比删除不确定归属的数据更安全。
- Copy/Move/Delete 的错误路径统一为源路径，重试不再把目标路径当成源。

### FM-2：重试是原任务的新实例，不是另一个操作

旧实现直接从 `errors` 取路径作为新顶层源。比如原操作是 `root/ -> dest/`，
`root/a.txt` 冲突后，旧重试会执行 `root/a.txt -> dest/a.txt`，而不是恢复
`root/ -> dest/root/` 的相对结构。旧 UI 还调用 `start_copy/start_move`，把
`verify`、`filter_pattern`、`filter_newer_days` 重置为默认值。

修复后，`FinishedOp`/错误报告保留原顶层源与 `CopyOptions`；错误仅用于筛选相关
顶层源，重试经 `start_copy_opts/start_move_opts` 创建新任务。取消或失败任务的 id
不复用，旧 worker 的迟到结果不能进入新任务。

### OPEN-1：打开流程采用“最后一次请求获胜”

Loading 视图仍显示全局文件菜单，拖放和密码重试也能发起打开，因此多任务在途不是
理论状态。旧实现只替换同类型 `Option`：漫画 opener 与 ebook opener 可以并存，
password probe 完成后还能主动打开旧路径。

修复后，所有打开入口先经 `cancel_pending_open` 清除旧漫画/电子书解析、压缩包分流、
密码探测、密码对话框和一次性打开选项。`AsyncOpener::drop` 同时置协作取消标志。
旧 worker 即使稍后才退出，也没有保留在应用中的接收状态，不能再驱动视图转换。

修复前实测：`new_open_request_cancels_superseded_comic_opener` 发现旧 `opener` 仍存在；
`new_open_request_cancels_stale_password_probe` 发现旧探测弹出了密码对话框并覆盖
新 Loading 路径。

## 2. 应用视图与打开流程

代码：`openitgo-app/src/app.rs` 的 `View`、`ReaderApp::open_path`、
`cancel_pending_open` 和各 `poll_*`。

### 状态

- `Library`、`Reader`、`Ebook`、`Settings`、`FileManager`：顶层内容视图。
- `Archive(path)`：压缩包浏览；`path` 必须与 `archive_view.path` 一致。
- `Loading(path)`：当前打开请求的目标；只允许当前请求持有的后台任务写回。
- `previous_view` 是单层 FM 回退落点，不是历史栈；消费或进入 Library/Settings 后清空。

### 不变式

1. 同一时刻最多一个逻辑打开请求可写回；不同类型 opener 不得并行竞争视图。
2. `Loading(path)` 的 `path` 属于最新请求；旧任务完成不得改变它。
3. `pending_open_options` 是一次性负载，成功和失败都消费，新请求必须清除旧负载。
4. 外部条目打开 `pending_external_open` 是独立动作，不属于内容视图代际，不因另开
   漫画/电子书而取消。
5. 阅读器页面结果另受 epoch 保护；视图切换与打开代际是两层不同防护。

### 转换表

| 当前状态 | 事件 | 下一状态/动作 | 守卫 |
|---|---|---|---|
| 任意视图 | 新 `open_path` | 取消全部旧打开任务，再按类型分流 | 新请求获胜 |
| 任意视图 | 打开电子书 | `Loading(path)`，创建 ebook opener | 旧任务已取消 |
| 任意视图 | 打开漫画/图片 | `Loading(path)`，创建 comic opener | 图片附带一次性定位选项 |
| 任意视图 | 打开 zip/cbz/rar/cbr | `Loading(path)`，后台列目录 | 等待启发式分流 |
| 分流完成 | 图片占比达到漫画阈值 | 创建 comic opener，保持 Loading | 复用同一路径 |
| 分流完成 | 文件包 | `Archive(path)`，复用条目结果 | 不重复列目录 |
| 分流失败 | 含密码错误 | 回落 comic opener | 统一密码链处理 |
| Comic Ready(Ok) | 应用每书设置、历史、一次性选项 | `Reader` | 页码经 `ReadingState` 校正 |
| Ebook Ready(Ok) | 创建 WebView 并恢复位置 | `Ebook` | 创建失败转 Library + 错误 |
| Comic/Ebook Ready(Err) | 普通错误 | `Library` + 错误消息 | 当前任务才可达 |
| 密码错误 | 有未试候选 | Loading + password probe | 每路径每会话自动试一次 |
| 密码探测成功 | 写入会话密码 | 对原路径发起新打开请求 | 旧 probe 已取出 |
| 密码探测失败 | 全部候选无效 | Library + 密码对话框 | 不自动循环 |
| Loading | 用户取消 | 取消全部打开任务，转 Library | 不执行迟到结果 |

## 3. `AsyncOpener<T>`：一次性异步任务

代码：`openitgo-app/src/opener.rs`。

显式输出状态为 `Loading` 和 `Ready(Result<T, String>)`；内部还有由 `cached` 与
`cancel` 组成的隐式状态。

| 状态 | 事件 | 下一状态 |
|---|---|---|
| 新建/在途 | 通道暂无结果 | Loading |
| 在途 | worker 发送 Ok/Err | Ready(Ok/Err)，缓存结果 |
| Ready | 再次 `poll` | 返回同一缓存结果，不重复消费 |
| 在途 | sender 异常断开 | Ready(Err("后台读取异常终止"))，缓存 |
| 任意 | Drop | 置 cancel；不再提供 UI 轮询入口 |

不变式：结果最多消费一次；Ready 不会退回 Loading；重试必须新建 opener；Drop 是协作
取消，不代表 worker 已同步退出。FileManager 搜索、比较、同步、校验和等任务也使用
“Drop 置取消 + 通道结果”的同款模式。

## 4. 漫画阅读状态

代码：`openitgo-core/src/state.rs` 的 `ReadingState`，以及
`openitgo-app/src/views/reader.rs` 的 `OpenReader`。

### 不变式

- `total_pages == 0` 时 `current_page == 0`；否则 `current_page < total_pages`。
- Webtoon 下 `double_page == false`；`is_double_page()` 必须同时考虑模式与标志。
- 普通双页跳转锚点为封面 0 或奇数 spread 锚点；宽页单独展示是导航例外，不能
  把宽页边界导航强行改回普通奇数锚点。
- `rotation` 正常持久化值仅为 0/90/180/270；脏数据打开时回 0。
- 翻页、跳页和模式切换按各自规则清 `pan`；模式切换重置 zoom/fit。
- 当前页、动画两端页和 Webtoon 可见页受缓存淘汰保护。

### 转换表

| 事件 | 转换 |
|---|---|
| `next_page/prev_page` | 在边界内移动一页；双页普通路径按 spread 锚点移动 |
| `next_spread/prev_spread` | 双页下考虑宽页边界；非双页退化为单页 |
| `go_to_page` | 清 pan，并 clamp 到有效页/spread |
| `set_mode(Webtoon)` | 清双页，使用 Width fit，重置 zoom/pan |
| `set_double_page` | Webtoon 强制 false；其他模式设置并校正页码 |
| `rotate_cw` | 0→90→180→270→0；调用方挂 `pending_fit` |
| 打开/挂起恢复 | `bump_epoch`，取消旧加载并清在途/错误状态 |

## 5. 页面加载、重试与代次

代码：`openitgo-app/src/loader.rs`、`views/reader.rs`。

### Loader 请求状态

请求身份是 `(epoch, page_index, thumbnail)`；优先级不是身份的一部分。

| 状态 | 事件 | 下一状态 |
|---|---|---|
| 不存在 | 新请求且队列有容量 | Registered(Low/High) |
| Registered(Low) | 同 key 高优先级请求 | Registered(High)，必要时补发高优先级唤醒 |
| Registered | IO `claim_io` 成功 | IOClaimed；同一控制块只能成功 claim 一次 |
| IOClaimed | 读取成功 | DecodeQueued，按当前优先级选队列 |
| DecodeQueued | 解码完成 | Completed + LoadResult |
| 任意在途 | `cancel_pending` | Cancelled，注册表清空；迟到结果仍可物理到达 |
| 注册/补发 | 有界队列满 | 新请求移除注册；补发保留 `high_wakeup_pending` 供重试 |
| 低优先级 decode 队列满 | 背压 | Completed + `dropped=true`，不记永久页面错误 |

### Reader 接收与重试

- 只有 `result.epoch == current_epoch` 的结果可更新缓存、pending 和错误表。
- `dropped=true` 清 pending 后允许后续预加载重试，不进入错误退避。
- 真正错误进入 `page_errors/thumbnail_errors`，重试间隔为 2、4、8、16、30、30…秒
  上限；成功清除对应错误与退避记录。
- full pending 超过 5 秒可重新请求；这解决结果丢失，不把慢 IO 直接判为永久失败。
- 新 epoch 使旧结果无效。禁止为了“恢复”旧页而清掉新 epoch 的 pending/error 状态。

## 6. Archive 浏览与拖出

代码：`openitgo-app/src/views/archive.rs`。

### 列目录与预览

`ArchiveViewState = Idle | Listing | Ready | NeedPassword | Failed(String)`。

| 当前状态 | 事件 | 下一状态 |
|---|---|---|
| Idle/Ready/Failed/NeedPassword | `open/open_with_password` | 清旧条目与预览，Listing |
| Listing | 列目录成功 | Ready，保存 entries/comment |
| Listing | PasswordRequired/Incorrect | NeedPassword；带密码重试失败标 incorrect |
| Listing | 其他错误 | Failed |
| Ready | F5 | Listing；完成后仅在目录仍存在时恢复 current_dir |
| 任意 | 新包 | 旧 listing/preview/drag 被替换或取消 |

预览是另一隐式状态机：`preview_entry != preview_requested` 时启动新读取；启动新目标
会丢弃旧 opener 和旧纹理。选中集只存文件名，目录选中态由后代统计派生。

### 拖出

`Potential -> Extracting -> Ready -> OLE DoDragDrop -> 清除`。

- Potential 超过 40pt 或出窗才启动 IO。
- Extracting 成功且暂存非空才进 Ready；失败/空结果清理暂存并报错。
- 在进入模态 DoDragDrop 前松开主键即取消并清理。
- Archive 拖出恒 COPY-only；Ready 尚未出窗时不得启动 OLE。

## 7. 文件面板与目录大小

代码：`openitgo-app/src/views/file_manager_panel.rs`。

`PanelLoadState = Idle | Loading(AsyncOpener) | Ready | Failed(String)`。

| 当前状态 | 事件 | 下一状态/动作 |
|---|---|---|
| Idle | 恢复目录/首次进入 | Loading，清旧条目、选择与缓存 |
| 任意 | navigate/refresh/restore tab | 替换旧 opener，Loading |
| Loading | 无结果 | 保持 Loading 并请求下帧轮询 |
| Loading | Ok | 校正选择/焦点/标签恢复负载，读注释，Ready |
| Loading | Err/断连 | Failed |
| Ready | FS watch 去抖 300ms | refresh → Loading，保留仍存在的选择 |
| Ready/Failed | 无导航的 `poll` | 原样返回；不得经 `mem::take` 打回 Idle |

不变式：只有当前 `dir` 的 opener 存活；目录切换会取消目录大小 worker 并清结果；
`columns[0]` 恒为 Name，固定列至少一列；标签快照按路径/文件名恢复，不恢复已失效的
行号。行内导航可当场清空 entries，渲染访问必须使用防御性索引。

## 8. 解压任务管理器

代码：`openitgo-app/src/extract_manager.rs`。

显式状态：`Queued | Running | Done | Failed | Cancelled`。

| 当前状态 | 事件 | 下一状态 |
|---|---|---|
| 新任务 | Running 数 < 4 | Running，启动 worker |
| 新任务 | Running 数 = 4 | Queued，进入 FIFO |
| Queued | 有空位 | Running，按 FIFO 启动 |
| Queued | 取消 | Cancelled，从 pending 移除，不起线程 |
| Running | Started/EntryDone | Running，更新总量/进度 |
| Running | Finished | Done，汇总实际 output_dir |
| Running | Failed("已取消") | Cancelled |
| Running | Failed(其他)/worker Err | Failed |
| 任意终态 | 迟到事件 | 忽略，不得覆盖终态或重复汇报 |
| 全部无活动 | poll 收尾 | 清除终态任务行 |

不变式：运行数不超过 4；pending 与 Queued 一一对应；Queued 没有 worker；取消运行中
任务只置标志，最终由引擎事件收敛；只有 Done 执行“删除压缩包/打开目标文件夹”。

## 9. 文件操作、冲突问答、重试与撤销

代码：`openitgo-app/src/views/file_ops.rs`、`file_manager.rs`、
`file_manager_undo.rs`。

### 任务状态（隐式）

`Queued -> Running <-> Paused`，Running 可进入 `WaitingConflict`；活动任务的取消为
`CancelRequested -> Finished(cancelled)`，正常/失败结束为 `Finished`。Finished 被 poll
消费后移除；排队取消直接出队。

| 当前状态 | 事件 | 下一状态/动作 |
|---|---|---|
| 提交 | 未达 `max_concurrent` | Running；0 = 不限并发 |
| 提交 | 已达上限 | Queued(FIFO)，不起线程 |
| Running | 完成/断连/取消 | Finished；断连转为 fatal，不能永久卡住 |
| Finished 被消费 | 有空位 | 移除任务并 FIFO 放行 |
| Running + Ask | 遇到冲突 | WaitingConflict，最多一条待答查询/任务 |
| WaitingConflict | Overwrite/Skip/AutoRename | 按回答继续；apply_all 记忆同级策略 |
| WaitingConflict | Cancel/取消/回答通道断开 | Finished(cancelled) |
| Running/Paused | cancel | 块/项边界收拢，清半成品 |
| Finished(errors) | 重试失败项 | 用原顶层源与完整参数创建新 id 任务 |

不变式：在途数不含 Queued；Queued 没有线程/进度；同一任务最多一条待答冲突；终态
结果只消费一次；id 单调不复用。

### 完成与撤销不变式

1. `completed`（未取消地跑完）与 `clean`（本顶层源无新增错误）不同。
2. 非过滤 Move 只有在没有 `drop_source_unsafe` 时才可整删源。
3. `written` 只包含完整成功、目标原本不存在、且 Move 源已移除的顶层项。
4. 覆盖写、目录合并、部分失败、Delete、Compress、Split、Merge 不做整体撤销。
5. Undo 执行前再次检查：Trash 目标仍存在；RenameBack 当前位置存在且原位置空闲。
6. 不可撤销操作不清空既有撤销栈；撤销栈上限 32，满时丢弃最旧项。

## 10. 电子书 WebView 状态

代码：`openitgo-app/src/ebook_renderer.rs`、`ebook_renderer_template.rs`。

Rust 侧保存章节、字符偏移、spread 和设置；JS 侧维护分页 DOM、章节请求和搜索状态。
`chapterLoadGeneration` 是章节导航代次，`chapterFetches` 按章去重。

| 事件 | 转换/守卫 |
|---|---|
| `loadChapter` | 增加 generation；只有最新 generation 的响应可替换 DOM |
| 同章重复请求 | 复用 `chapterFetches` 中的 Promise |
| `applySettings` | 重排并保留字符位置；动画值不变时 Rust 不下发 |
| 分页位置 IPC | clamp spread，并唤醒 egui 重绘 |
| resize | 防抖后按滚动比例或 spread 恢复位置 |
| 菜单/覆盖层打开 | 隐藏原生 WebView，关闭后恢复 |

不变式：旧章节响应不得覆盖新章节；位置恢复在首次章节加载前确定；transform 只作用于
`#column-content`；Windows 页面内 URL 使用相对形式。

## 11. 其他隐式状态与持久化

- FM 缩略图/系统图标：`Miss -> Pending -> Ready|Failed`；可见范围变化 bump generation，
  过期队列项回 `Dropped`，只清 pending，不污染缓存。失败值用于阻止每帧重试；文件
  key（mtime/len）变化允许缩略图重载。
- FM 搜索：`None -> Running -> Done(truncated)`；Drop/关闭置取消。结果批量追加，命中
  达上限后截断；旧任务不能复用为新查询。
- 比较/同步/校验和：`None -> Running -> result(s) -> Finished`；Drop 置取消。校验和
  验证行另有 `Pending|Ok|Mismatch|Missing|Unsupported|Error`。
- 设置/历史/书签/FM 快照：`Clean -> Dirty -> Persisted/Failed`。30 秒节流只适用于阅读
  中历史；离开视图和退出必须绕过节流。保存失败仍更新每书设置快照，避免每帧刷屏，
  但错误必须可见。
- 所有持久化值在恢复时 clamp/规范化；磁盘中的非法枚举或页码不应成为运行时非法状态。

## 12. 回归测试索引与验证边界

| 问题 | 回归测试 |
|---|---|
| OPEN-1 | `new_open_request_cancels_superseded_comic_opener`；`new_open_request_cancels_stale_password_probe` |
| FM-1 | `copy_missing_source_does_not_record_undo_target`；`copy_directory_merge_does_not_record_preexisting_target_for_undo`；`move_skip_nested_conflict_preserves_source`；`move_directory_onto_file_failure_preserves_source` |
| FM-2 | `retry_nested_failure_uses_original_top_level_source`；`retry_sources_filters_missing_and_dedups`；`finished_copy_keeps_original_sources_and_retry_options` |

两个 Move 回归在 Windows 走真实回收站实现，验证的不只是标志位。其余并发/取消路径
由 loader、reader、file_ops、extract_manager 既有测试覆盖，包括优先级升级后队列满重试、
pending 超时、旧 epoch 丢弃、冲突等待取消、队列 FIFO 和迟到终态事件。

本文只把实际代码可达且由失败测试或直接确定路径确认的问题列为已确认 bug。线程
资源耗尽、第三方库未知 panic 等理论风险不在此清单中冒充实测结论。
