# 全仓库审计报告 - 2026-04-17

> 方法：6 个 Explore 子 agent 并行对 `oc-core` / `oc-server` / `src-tauri` / 前端六大域做只读静态审计，此文档为去重合并结果。
> 基线分支：`claude/codebase-analysis-rkepG`（HEAD = `9302aed`）。
> 全部结论为静态分析推断，**落地修复前需要逐条 Read 对应 `file:line` 复核**，避免误杀。

优先级排序：🔴 严重 Bug → 🟠 中等 Bug → 🟡 性能/稳定 → ℹ️ 设计与功能改进。

- 严重 Bug：10 项
- 中等 Bug：11 项
- 性能/稳定：10 项
- 轻微/设计：5 项

---

## 🔴 严重 Bug（10 项）

### B1. UTF-8 字节切片越界会导致 panic

- **位置**
  - `crates/oc-core/src/context_compact/summarization.rs:249`
  - `crates/oc-core/src/context_compact/truncation.rs:74, 116, 118`
- **因果**：`&summary[..budget.min(summary.len())]` 把字符预算当字节用；`find_structure_boundary` 的 fallback `target_pos` 由浮点运算转换得到，可能落在 UTF-8 多字节字符中间。中文 / emoji 摘要接近阈值时直接 panic。
- **修复**：统一改 `crate::truncate_utf8(s, max_bytes)`，红线已经明确禁用字节切片。
- **状态**：待修复。

### B2. Tier 3 摘要可能拆散 tool_use / tool_result 对

- **位置**：`crates/oc-core/src/context_compact/summarization.rs:57-91`
- **因果**：`find_round_safe_boundary` 在所有剩余消息同属一个 round 时返回 0，导致整轮被跳过摘要或 tool_use 被保留而对应 tool_result 被摘走。模型看到"没有结果的工具调用"，Anthropic 会直接 400 拒绝请求。
- **修复**：round 调整失败时向前搜索下一个 round 边界，而不是静默返回 0。
- **状态**：待修复。

### B3. Tool loop 最后一轮的 tool_results 被静默丢弃

- **位置**：`crates/oc-core/src/agent/providers/anthropic.rs:318-320, 473-477`（其他 3 个 provider 很可能存在对称问题）
- **因果**：循环在最后一轮执行完工具、把 tool_result 压进 history，但此时已到 `max_rounds` 上限，下一轮 API 不再调用，结果永远不会进入模型。用户侧表现为"工具跑了但模型没收到"。
- **修复**：在 round 起始检测 `round >= max_rounds - 1` 时禁止接受新的 tool_use；或至少额外跑一次"只发消息不要工具"的收尾请求把结果送回模型。
- **状态**：待修复。

### B4. Async job 注入的 `mark_injected` 错误被吞

- **位置**：`crates/oc-core/src/async_jobs/spawn.rs:75`；`async_jobs/injection.rs:13-31`
- **因果**：`let _ = mark_injected(...)` 忽略失败。DB 写失败后，下次 `replay_pending_jobs()` 会把同一 job 再注入一遍，对话里出现重复 `<tool-job-result>` 块。
- **修复**：失败时带 backoff 重试，彻底失败再通过 EventBus 报警；不要吞错误。
- **状态**：待修复。

### B5. Query string token 未 URL 解码 → 鉴权绕过 / 误拒

- **位置**：`crates/oc-server/src/middleware.rs:47`
- **因果**：`strip_prefix("token=")` 直接拿原始 query，不做 `percent_decode`。带百分号编码的合法 token 会被拒；若 expected 自身含特殊字符（或双端编码不一致），可构造绕过分支。
- **修复**：改用 `url::form_urlencoded::parse` 或 `percent_encoding::percent_decode_str` 先解码再比对。
- **状态**：待修复。

### B6. API Key 比较非常量时间

- **位置**：`crates/oc-server/src/middleware.rs:37, 48`
- **因果**：普通 `==` 比较 token，存在 timing side-channel，尤其在 HTTP 暴露到 `0.0.0.0:8420` 的公网/内网场景。
- **修复**：使用 `subtle::ConstantTimeEq` 或等长 HMAC 比较。
- **状态**：待修复。

### B7. Service install 脚本命令注入

- **位置**：`crates/oc-core/src/service_install.rs:121-125, 270`
- **因果**：`exe_path` / `bind_addr` / `api_key` 未转义直接拼进 systemd `ExecStart` / launchd `ProgramArguments`。用户 home 含空格、`api_key` 含引号就能把参数拆破；恶意配置可注入额外命令到开机自启。
- **修复**：systemd 用 `shlex::try_quote`；launchd plist 用多个独立 `<string>` 元素保持数组结构，不要拼成单串。
- **状态**：待修复。

### B8. ProfileStickyMap 内存泄漏 + 粗暴 clear

- **位置**：`crates/oc-core/src/failover.rs:338-348`
- **因果**：到达 `STICKY_MAX_SESSIONS_PER_PROVIDER=500` 上限时 `sessions.clear()` 清空全部，破坏 session 粘性；长期运行下 session 只增不减（无死 session 清理路径）。
- **修复**：改 LRU（`lru` crate，或 `IndexMap` + 手动 evict 最旧），粘性不受清空冲击。
- **状态**：待修复。

### B9. Project 删除非事务：DB 成功 / FS 残留 / symlink 风险

- **位置**：`crates/oc-core/src/project/db.rs:283-294` + `project/files.rs:184-213`
- **因果**：先 `DELETE projects`（事务内），再 `rm -rf projects/{id}/`（事务外）。commit 后崩溃 → 目录残留成孤儿；更糟：若 `{id}` 被篡改为 symlink，可删到预期外路径。
- **修复**：`canonicalize()` 校验目标必须位于 `~/.opencomputer/projects/` 下；或改为先改名到 `.trash/`、commit 后异步清理。
- **状态**：待修复。

### B10. EventBus broadcast 容量溢出 → 异步 job 完成通知丢失

- **位置**：`crates/oc-core/src/event_bus.rs:27` + `async_jobs` 消费路径
- **因果**：`tokio::broadcast` 滞后消费者会收到 `RecvError::Lagged` 并丢事件。`job_status(block=true)` 只靠 `async_tool_job:completed` + 200ms 兜底轮询；事件丢失时模型必须等到兜底轮询才醒，而且只覆盖 600s 上限，极端场景直接 timeout。
- **修复**：订阅端显式 match `Lagged`，丢失时立即做一次 DB 状态查询；或核心事件改走 mpsc / watch。
- **状态**：待修复。

---

## 🟠 中等 Bug（11 项）

### M1. Cache-TTL 紧急阈值 TTL 过期后失效

- **位置**：`crates/oc-core/src/agent/context.rs:37, 50-78`
- **因果**：`CACHE_TTL_EMERGENCY_RATIO=0.95` 只在 `within_ttl==true` 时才检查；TTL 一过期，`emergency` 恒为 false，高 usage 也不会触发分层保护，直接掉到 Tier 4。
- **修复**：emergency 判断与 TTL 解耦，usage ≥ 95% 永远走应急路径。
- **状态**：待修复。

### M2. Post-compaction 文件恢复预算会被打爆

- **位置**：`crates/oc-core/src/context_compact/recovery.rs:100-116`
- **因果**：5 文件 × 16KB = 80KB，再加 XML 标签 overhead 可超 `MAX_RECOVERY_TOTAL_BYTES=100KB`，循环不动态收紧 `max_file_bytes`。
- **修复**：循环内用剩余预算动态 clamp 单文件上限。
- **状态**：待修复。

### M3. 文件恢复消息插入位置硬编码为索引 1

- **位置**：`crates/oc-core/src/agent/context.rs:283`
- **因果**：`messages.insert(1, recovery_msg)` 假设摘要一定在 0。将来 `apply_summary()` 多塞一条说明就错位。
- **修复**：用 `split.preserved_start_index` 或语义化常量。
- **状态**：待修复。

### M4. Idle memory extract 句柄竞态

- **位置**：`crates/oc-core/src/memory_extract.rs:516-524`
- **因果**：先移除句柄、再比对 `expected_updated_at`；窗口内被新消息触发 `schedule_idle_extraction()`，新句柄写回但任务已在跑，可能重复执行提取，浪费 API + 重复记忆。
- **修复**：改 CAS（`dashmap::entry().or_insert_with_key`）或保留句柄直到任务结束再移除。
- **状态**：待修复。

### M5. ask_user 题目级 + 组级超时冲突导致悬挂

- **位置**：`crates/oc-core/src/tools/ask_user_question.rs:142-162` + `channel/worker/ask_user.rs:351-410`
- **因果**：组超时 600s、单题 60s，单题过期后 `BUTTON_PENDING` 没被回收；group timeout 未到，后续按钮回调仍进入已死 question。
- **修复**：handler 顶部检查 `timeout_at`；过期立即清理 `BUTTON_PENDING`。
- **状态**：待修复。

### M6. WeChat 入站媒体 file_id 路径验证弱

- **位置**：`crates/oc-core/src/channel/wechat/media.rs:477-517` + `channel/worker/media.rs:70-114`
- **因果**：只替换 `['/', '\\', ':']`，不处理 `..` 组合；`persist_channel_media_to_session` 没 `canonicalize()` 验证源路径真在 inbound-temp 下。符号链接攻击可复制任意文件进 attachments。
- **修复**：file_id 白名单（UUID 或 `[a-zA-Z0-9_-]+`），拷贝前 `canonicalize()` 校验前缀。
- **状态**：待修复。

### M7. Cron 重启抖动窗口内重复执行

- **位置**：`crates/oc-core/src/cron/scheduler.rs:88`；`cron/db.rs:445-450`
- **因果**：15s 轮询 + `next_run_at <= now`，进程闪退 <15s 重启后同秒 job 可能再跑一次；缺少 `running_at` 原子更新或幂等标记。
- **修复**：`UPDATE ... SET running_at=? WHERE id=? AND running_at IS NULL` 原子占位，拿到更新后才执行。
- **状态**：待修复。

### M8. Replay pending async jobs 非原子 select + dispatch

- **位置**：`crates/oc-core/src/async_jobs/mod.rs:81-96`
- **因果**：列 `injected=0` → dispatch；若多实例或事件重入，同 job 会被重复 dispatch；dispatch 失败无重试记录。
- **修复**：单事务内 `UPDATE ... RETURNING` 或 `UPDATE ... SET dispatching=1 WHERE injected=0` 抢占后再投递。
- **状态**：待修复。

### M9. Plan 状态机缺合法转移校验

- **位置**：`crates/oc-core/src/plan/types.rs:7-44`；`plan/store.rs` 所有 setter
- **因果**：六态间可任意跳转（Completed → Executing 也允许），并发请求可致步骤重复执行或跳过 git checkpoint。
- **修复**：`fn is_valid_transition(from, to) -> bool`，store 写入前强制校验。
- **状态**：待修复。

### M10. Plan 文件名秒级时间戳碰撞

- **位置**：`crates/oc-core/src/plan/file_io.rs:31-35, 61-84`
- **因果**：`%Y%m%d-%H%M%S`，同秒多会话生成同名文件互相覆盖；version 计数器从内存拿、重启归零，覆盖旧版本快照。
- **修复**：加纳秒 / UUID 后缀；版本号启动时从磁盘 `max(version)+1` 初始化。
- **状态**：待修复。

### M11. 前端 `useNotificationListeners` stale closure

- **位置**：`src/components/chat/hooks/useNotificationListeners.ts:200`
- **因果**：useEffect 依赖数组只含 `[reloadSessions]`，闭包内用了 `setMessages` / `setLoading` / `setLoadingSessionIds` 等 7 个状态/ref；新值变更后订阅仍持旧引用，回调作用于过期 state。
- **修复**：补齐依赖，或把所有 setter 改成 `useLatestRef` 化。
- **状态**：待修复。

---

## 🟡 性能 / 稳定性 / 安全隐患（10 项）

### P1. XSS：snippet `<mark>` 白名单反解可被绕过

- **位置**：`src/components/chat/sidebar/SearchResultItem.tsx:133-135`
- **因果**：FTS5 `snippet()` 若消息原文含 `<mark onclick=...>`，先 escape 再"白名单反解 `<mark>`" 的实现会把 `&lt;mark onclick&gt;` 恢复成可执行标签；sanitize 粒度是标签名而非属性。
- **修复**：不要反解 HTML，直接用 React 元素按 FTS 分界符（`\u0002` / `\u0003`）切片包 `<span class="bg-primary/30">`。
- **状态**：待修复。

### P2. AskUserQuestionBlock 用了原生 `title=""`

- **位置**：`src/components/chat/ask-user/AskUserQuestionBlock.tsx:252`
- **因果**：违反红线——必须用 `@/components/ui/tooltip`。
- **修复**：换 `<IconTip label={...}>`。
- **状态**：待修复。

### P3. `tauri.conf.json` CSP 仍为 `null`

- **位置**：`src-tauri/tauri.conf.json:30`
- **因果**：红线里列为禁止项。Streamdown 渲染任意 Markdown + 外部图片，一旦出现 XSS 无兜底。
- **修复**：至少 `default-src 'self'; img-src 'self' data: https:; script-src 'self'`，按需渐进放开。
- **状态**：待修复。

### P4. Broadcast slow consumer 未处理 `Lagged`

- **位置**：`crates/oc-core/src/event_bus.rs:17, 40-42`
- **因果**：订阅方 hang / panic / 消费慢，滞后 receiver 不被主动回收；同一 channel 其他消费者会因 lag 错过事件。无消费者心跳、无超时关门。
- **修复**：订阅循环显式 match `RecvError::Lagged` 并立即 re-sync；后台扫描过期订阅配合 keep-alive ping 强制 drop。
- **状态**：待修复。

### P5. OAuth 刷新 margin 偏大且被动

- **位置**：`crates/oc-core/src/oauth.rs:64-76`；Codex provider
- **因果**：60s margin 仍可能在网络抖动中发送即过期 token；Codex 纯被动靠 Auth error 重试，首次失败会让用户看到一次可见错误。
- **修复**：margin 降到 30s，Codex 发请求前主动探测 + 异步 `refresh_token` 续期。
- **状态**：待修复。

### P6. FTS5 `sanitize_fts_query` 空回退 `"*"` 扫全库

- **位置**：`crates/oc-core/src/memory/helpers.rs:5-30`
- **因果**：空查询或全被过滤的 query 回退匹配所有记录，大库（10 万+）直接全表扫 + snippet 计算 → 卡顿或阻塞 SQLite。
- **修复**：空查询直接返回空结果；或强制 `LIMIT` + `ORDER BY rowid DESC LIMIT 50`。
- **状态**：待修复。

### P7. Docker SearXNG / 外部请求大响应无体量上限 + SSRF

- **位置**：`crates/oc-core/src/tools/web_search/`、`tools/image_generate/`、`url_preview.rs`
- **因果**：`reqwest` 默认无 body 大小限制，恶意 / 异常上游可返回超大响应打爆内存；目标 URL 未过滤内网段（`127.0.0.0/8`、`10.0.0.0/8`、`169.254.0.0/16`）。
- **修复**：`response.bytes_stream()` + 手动累计 cap（如 10MB），超出立即丢弃；拒绝内网目标。
- **状态**：待修复。

### P8. Log 中可能落 `?token=` / `Authorization: Bearer`

- **位置**：`crates/oc-server/src/lib.rs:51` + 未确认的 axum access log 链路
- **因果**：非桌面模式 binding 到 `0.0.0.0` 时，query string 出现在标准 HTTP 日志 / 反代日志。红线是"API Key 不得出现在任何日志"。
- **修复**：自定义 `on_request` 裁剪 query 或只保留 path；优先 header 路径，query 仅给 WS 不得已用。
- **状态**：待修复。

### P9. `useNotificationListeners` 首个 effect `[]` 依赖永久订阅

- **位置**：`src/components/chat/hooks/useNotificationListeners.ts:35-40`
- **因果**：空依赖永远挂一份监听；组件快速 mount/unmount 时若 unsubscribe 不对称就会累积。
- **修复**：确认 `return () => unsub()` 完全对齐 subscribe；否则会话频繁切换逐步耗内存。
- **状态**：待修复。

### P10. Plan git 分支命名用 `Local` 时间 + 非原子

- **位置**：`crates/oc-core/src/plan/git.rs:30-54`
- **因果**：本地时区 + 秒级时间戳，不同设备 / 并发易重名；`git branch` 非原子，回滚可能打到错 checkpoint。
- **修复**：UTC + 短 UUID；创建前 `git rev-parse --verify` 去重。
- **状态**：待修复。

---

## ℹ️ 轻微 / 设计改进（5 项）

### D1. 记忆查询 IN 子句用 `format!` 拼接

- **位置**：`crates/oc-core/src/memory/sqlite/trait_impl.rs:165-166, 685-687, 702-704`；`crates/oc-core/src/logging/db.rs:133, 147`
- **因果**：当前占位符方案是安全的，但可读性差、后续维护者改错就变 SQL 注入。
- **修复**：提供 `repeat_vars(n)` helper，或改 `rarray` 绑定。
- **状态**：建议改进。

### D2. Project 记忆清理跨 `memory.db` 非事务

- **位置**：`crates/oc-core/src/project/db.rs` + `memory/` 跨库清理路径
- **因果**：Project 删除后跨 `memory.db` 清理失败只留"不可达孤儿"（现有注释承认的状态）。
- **修复**：新增启动时 reconciler：扫 `memory.db` 里 project scope 但 project 不存在的记录并清理。
- **状态**：建议改进。

### D3. `search_session_messages_cmd` 结果按 `messageId` 排序

- **位置**：`crates/oc-core/src/session/` 搜索接口
- **因果**：`messageId` 在 session 迁移 / 导入场景可能不是严格递增，会导致搜索结果与时间轴错位。
- **修复**：改用 `created_at` 排序。
- **状态**：建议改进。

### D4. i18n sync 无 CI hook

- **位置**：`scripts/sync-i18n.mjs`
- **因果**：工具存在但未见 CI 校验，PR 合入后其他 10 种语言缺 key 无法阻断。
- **修复**：GH Action / pre-commit 跑 `sync-i18n.mjs --check`。
- **状态**：建议改进。

### D5. Tool loop 并发组的共享状态

- **位置**：`crates/oc-core/src/tools/` + `ToolExecContext`
- **因果**：只读工具并行执行前提是"只读"，但 `ToolExecContext` 多工具共享同一可变引用（审批 state、日志 buffer）；将来加"只读但写日志"的工具可能竞争。
- **修复**：`ToolExecContext` 可变字段内部 `Mutex`，或改 channel 回传。
- **状态**：建议改进。

---

## 📊 总计

| 档次 | 数量 | 集中区域 |
|---|---|---|
| 🔴 严重 | 10 | `context_compact` UTF-8 切片、tool loop 终止、async_jobs 注入、server 鉴权、service install 命令注入 |
| 🟠 中等 | 11 | cache-TTL 逻辑、memory_extract 竞态、plan 状态机、cron 重放、前端 stale closure |
| 🟡 性能/稳定 | 10 | XSS、CSP、broadcast lag、SSRF、日志 token |
| ℹ️ 轻微/设计 | 5 | SQL 拼接习惯、孤儿清理、排序字段、CI 校验、工具上下文共享 |

**修复顺序建议**

1. 第一批（用户可触发即时故障）：B1、B3、B5 + B6、B7
2. 第二批（长尾数据/稳定性）：B2、B4、B9、B10、M4、M7、M8
3. 第三批（安全加固批）：P1、P3、P7、P8
4. 其余按迭代节奏推进。

## 复核建议

- 每条修复前务必 Read 对应 `file:line` 确认当前实际代码（sub-agent 报告基于静态分析，可能因近期改动失真）
- 可将每条转为独立 issue / PR，便于并行修复与 code review
- 对 UTF-8 切片类问题可一次性全局 grep `&[a-zA-Z_]+\[\.\.` 做批量治理
