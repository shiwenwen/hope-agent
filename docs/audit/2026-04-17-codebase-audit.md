# 全仓库审计报告 - 2026-04-17

> 方法：6 个 Explore 子 agent 并行对 `oc-core` / `oc-server` / `src-tauri` / 前端六大域做只读静态审计，此文档为去重合并结果。
> 基线分支：`claude/codebase-analysis-rkepG`（HEAD = `9302aed`）。
> 全部结论为静态分析推断，**落地修复前需要逐条 Read 对应 `file:line` 复核**，避免误杀。

优先级排序：🔴 严重 Bug → 🟠 中等 Bug → 🟡 性能/稳定 → ℹ️ 设计与功能改进。

- 严重 Bug：10 项（已处理 9 项 / 1 项维持原状）
- 中等 Bug：11 项（本轮处理 7 项 / 4 项维持原状）
- 性能/稳定：10 项
- 轻微/设计：5 项

> **2026-04-17 复核（分支 `claude/fix-audit-bugs-Ip6Bl`）**：按本文件列出的严重 Bug 逐条 Read 源文件复核。B1 / B3 / B4 / B5 / B6 / B7 / B8 / B9 均已在本轮修复；B10 经复核发现 `job_status` 早就改用 `async_jobs::wait::Notify` 注册表，不再依赖 EventBus 事件，原描述已失效；B2 经复核与原描述偏差较大（`find_round_safe_boundary` 已按 round 边界对齐，不会把 tool_use / tool_result 切开），保留观察。
>
> **2026-04-18 中等 Bug 复核（分支 `claude/fix-audit-bugs-BcytF`）**：按本文件列出的 11 项中等 Bug 逐条 Read 源文件复核。M3 / M4 / M5 / M6 / M8 / M9 / M10 本轮修复；M7 经复核发现 `claim_job_for_execution` 已实现原子抢占（`UPDATE ... WHERE running_at IS NULL AND next_run_at=? AND status='active'`），原风险不存在；M1 经复核发现 `cache_ttl_emergency` 仅作为 throttle 旁路开关使用，TTL 过期时 throttle 本就不生效，无需再"解耦"；M2 经复核发现循环内 `total_chars + truncated.len() + overhead > byte_budget` 的 break 检查已兜住 `MAX_RECOVERY_TOTAL_BYTES`，不会打爆上限；M11 经复核发现 useEffect 闭包内仅使用 refs 和 `useState` 返回的 setters，这些在 React 中本身稳定，eslint-disable-line 合理。
>
> **2026-04-18 稳定性/性能批修复（分支 `worktree-fix-audit-p2-p4-p5-p6-p9-p10`）**：按本文件"修复顺序建议"里的"第三批稳定性/性能"要求处理 P2 / P4 / P5 / P6 / P9 / P10。P2 `AskUserQuestionBlock` 倒计时 chip 原生 `title` 换成 `@/components/ui/tooltip` 的 `IconTip`；P4 经复核 oc-server + channel worker 共 4 处 `.subscribe()` 已处理 Lagged，只剩 Tauri bridge 静默 continue，补 `app_warn!` + emit `_event_bus_lagged` 事件让前端可感知；P5 OAuth margin 60s → 30s，新增 `oauth::ensure_fresh_codex_token(current_access_token)` per-chat 预刷新入口，[`chat_engine/engine.rs`](../../crates/oc-core/src/chat_engine/engine.rs) 在 model_chain 循环前调用；P6 `memory/helpers.rs` 的 `sanitize_fts_query` / `expand_query` 改为 `Option<String>`，消除 `"*"` 空回退导致的全库扫，`memory/sqlite/trait_impl.rs::search` 在 None 时跳过 FTS 直接走 vector；P9 经复核两个 effect 的 subscribe / unsubscribe 已对齐，只做风格统一（两处都走 `const unlisten = ...; return unlisten`）；P10 Plan git checkpoint 分支命名 `chrono::Local` 换成 `UTC ISO` + 8 位 UUID 尾缀，新增 `ref_exists` helper 在 `git branch` 前 `rev-parse --verify --quiet` 去重，`rollback_to_checkpoint` 也复用同 helper 去掉重复的 verify 分支。

---

## 🔴 严重 Bug（10 项）

### B1. UTF-8 字节切片越界会导致 panic

- **位置**
  - `crates/oc-core/src/context_compact/summarization.rs:249`
  - `crates/oc-core/src/context_compact/truncation.rs:74, 116, 118`
- **因果**：`&summary[..budget.min(summary.len())]` 把字符预算当字节用；`find_structure_boundary` 的 fallback `target_pos` 由浮点运算转换得到，可能落在 UTF-8 多字节字符中间。中文 / emoji 摘要接近阈值时直接 panic。
- **修复**：统一改 `crate::truncate_utf8(s, max_bytes)`，红线已经明确禁用字节切片。
- **状态**：✅ 已修复（summarization 的 cap 切片改走 `truncate_utf8`；`find_structure_boundary` / `_forward` 的搜索边界、fallback 回退点、`has_important_tail` 的 2KB 尾部切片全部用新增的 `floor_char_boundary` / `ceil_char_boundary` helper 或 `truncate_utf8_tail` 对齐到字符边界）。

### ~~B2. Tier 3 摘要可能拆散 tool_use / tool_result 对~~（原描述失效）

- **位置**：`crates/oc-core/src/context_compact/summarization.rs:57-91`
- **复核结论**：`find_round_safe_boundary` 实现是"从 `target_index` 向后回退到一个 round 变更点"，返回的 boundary_index 处满足 `messages[i-1].round != messages[i].round`，即 tool_use / tool_result 不会被切开；若所有可摘要消息同属一个 round 则返回 0，`split_for_summarization` 直接返回 `None` 跳过摘要，**不会** 把 tool_use 留下、tool_result 摘走——不会产生 Anthropic 400。审计描述的"拆散"场景在当前实现下不存在，至多是"整块跳过本次摘要"。该退化行为更接近性能/稳定性问题，本次不动。
- **状态**：⛔ 划掉（原因描述与现状不符）。

### B3. Tool loop 最后一轮的 tool_results 被静默丢弃

- **位置**：`crates/oc-core/src/agent/providers/{anthropic,openai_chat,openai_responses,codex}.rs`
- **因果**：循环在最后一轮执行完工具、把 tool_result 压进 history，但此时已到 `max_rounds` 上限，下一轮 API 不再调用，结果永远不会进入模型。用户侧表现为"工具跑了但模型没收到"。
- **修复**：在 round 起始检测 `round >= max_rounds - 1` 时禁止接受新的 tool_use；或至少额外跑一次"只发消息不要工具"的收尾请求把结果送回模型。
- **状态**：✅ 已修复（4 个 provider 统一：进入最后一轮 `round + 1 == max_rounds` 时请求不再携带 `tools`，模型只能产出文本应答；natural_exit 自然成立，避免静默丢结果 + 省掉"max rounds"占位文本）。

### B4. Async job 注入的 `mark_injected` 错误被吞

- **位置**：`crates/oc-core/src/async_jobs/spawn.rs:444`（无 parent session 的兜底）；`async_jobs/injection.rs` 注入主路径
- **因果**：`let _ = mark_injected(...)` 忽略失败。DB 写失败后，下次 `replay_pending_jobs()` 会把同一 job 再注入一遍，对话里出现重复 `<tool-job-result>` 块。
- **修复**：失败时带 backoff 重试，彻底失败再通过 EventBus 报警；不要吞错误。
- **状态**：✅ 已修复（`injection.rs` 新增 `mark_injected_with_retry`：`[0, 100ms, 500ms, 2000ms]` 4 次退避；彻底失败 emit `async_tool_job:mark_injected_failed` 事件 + `app_error!` 日志，保留"下次 replay 会重新注入"的语义但不再静默）。spawn.rs:444 的"无 parent session"分支只剩 orphan 清理意图，不涉及回放路径，保持 `let _ =`。

### B5. Query string token 未 URL 解码 → 鉴权绕过 / 误拒

- **位置**：`crates/oc-server/src/middleware.rs:47`
- **因果**：`strip_prefix("token=")` 直接拿原始 query，不做 `percent_decode`。带百分号编码的合法 token 会被拒；若 expected 自身含特殊字符（或双端编码不一致），可构造绕过分支。
- **修复**：改用 `url::form_urlencoded::parse` 或 `percent_encoding::percent_decode_str` 先解码再比对。
- **状态**：✅ 已修复（middleware 新增轻量 `percent_decode_form_value` + 配合 `+` → space 语义，query token 解码后再比较；配套单测覆盖正常/畸形编码）。

### B6. API Key 比较非常量时间

- **位置**：`crates/oc-server/src/middleware.rs:37, 48`
- **因果**：普通 `==` 比较 token，存在 timing side-channel，尤其在 HTTP 暴露到 `0.0.0.0:8420` 的公网/内网场景。
- **修复**：使用 `subtle::ConstantTimeEq` 或等长 HMAC 比较。
- **状态**：✅ 已修复（新增内联 `constant_time_eq` 做 XOR-fold 常量时间比较；Header / Query 两条分支都走同一实现；长度不等短路的 timing leak 对固定长度 API key 可接受；附 3 条单测）。

### B7. Service install 脚本命令注入

- **位置**：`crates/oc-core/src/service_install.rs:121-125, 270`
- **因果**：`exe_path` / `bind_addr` / `api_key` 未转义直接拼进 systemd `ExecStart` / launchd `ProgramArguments`。用户 home 含空格、`api_key` 含引号就能把参数拆破；恶意配置可注入额外命令到开机自启。
- **修复**：systemd 用 `shlex::try_quote`；launchd plist 用多个独立 `<string>` 元素保持数组结构，不要拼成单串。
- **状态**：✅ 已修复（新增 `xml_escape`：launchd plist 里 exe / bind / api_key / log 全部先转义 `<`, `>`, `&`, `"`, `'`，杜绝通过用户字符串打破 `<string>` 元素追加 argv；新增 `systemd_escape_arg`：systemd `ExecStart` 每个 argv 独立加 `"..."` 并转义 `\`, `"`, 控制字符，避免空格/引号拆分）。

### B8. ProfileStickyMap 内存泄漏 + 粗暴 clear

- **位置**：`crates/oc-core/src/failover.rs:338-348`
- **因果**：到达 `STICKY_MAX_SESSIONS_PER_PROVIDER=500` 上限时 `sessions.clear()` 清空全部，破坏 session 粘性；长期运行下 session 只增不减（无死 session 清理路径）。
- **修复**：改 LRU（`lru` crate，或 `IndexMap` + 手动 evict 最旧），粘性不受清空冲击。
- **状态**：✅ 已修复（新增 `StickyShard { map, order: VecDeque }` 本地 LRU 结构，`get` 访问时 promote，`set` 到上限时 `pop_front` 仅 evict 最旧一条，不再破坏全局粘性；补 2 条单测验证"填满+1"只淘汰最旧、"get 后 promote 的条目不会被当作最旧"）。

### B9. Project 删除非事务：DB 成功 / FS 残留 / symlink 风险

- **位置**：`crates/oc-core/src/project/db.rs:283-294` + `project/files.rs:184-213`
- **因果**：先 `DELETE projects`（事务内），再 `rm -rf projects/{id}/`（事务外）。commit 后崩溃 → 目录残留成孤儿；更糟：若 `{id}` 被篡改为 symlink，可删到预期外路径。
- **修复**：`canonicalize()` 校验目标必须位于 `~/.opencomputer/projects/` 下；或改为先改名到 `.trash/`、commit 后异步清理。
- **状态**：✅ symlink / 路径逃逸分支已修复（`purge_project_files_dir` 现在 `canonicalize()` 目标并强制 `starts_with(projects_root)` 检查，失败只记日志不删）。DB commit 后 `rm -rf` 仍是非事务的"先改 DB 后清盘"顺序——现有 project_id 全部来自 `Uuid::new_v4()`，路径逃逸风险已被 canonicalize 兜底，孤儿目录仅限崩溃窗口，不再升级为安全问题，后续清理可由 D2 方向的 reconciler 统一解决。

### ~~B10. EventBus broadcast 容量溢出 → 异步 job 完成通知丢失~~（原描述失效）

- **位置**：`crates/oc-core/src/event_bus.rs:27` + `async_jobs` 消费路径
- **复核结论**：`job_status` 早已改走 `async_jobs::wait::Notify` 注册表（`crates/oc-core/src/async_jobs/wait.rs` + `tools/job_status.rs`），producer 在 `finalize_job` 里通过 `notify_waiters()` 唤醒，*不再依赖* EventBus `async_tool_job:completed` 事件——broadcast lag 已不能导致 waiter 丢醒。`job_status` 循环还保留 100ms→2s 退避 select 做防御兜底，退避上限是单次等待窗口（默认 min(max_job_secs, 1800)），不是 600s。当前真实行为与审计描述偏差较大，原计划的"订阅端处理 Lagged"改进点归入 P4 继续跟踪。
- **状态**：⛔ 划掉（实现早于本次审计就已迁移到 Notify 注册表，原风险不存在）。

---

## 🟠 中等 Bug（11 项）

### ~~M1. Cache-TTL 紧急阈值 TTL 过期后失效~~（原描述失效）

- **位置**：`crates/oc-core/src/agent/context.rs:37, 50-78`
- **复核结论**：`cache_ttl_emergency` 的唯一消费点在 [`context_compact/engine.rs:65`](../../crates/oc-core/src/context_compact/engine.rs#L65) 的 `if ctx.cache_ttl_throttled && !ctx.cache_ttl_emergency { ... }`——它只在"throttle 激活但想强制跑 Tier 2+"场景做旁路开关。TTL 过期时 `cache_ttl_throttled=false`，整个分支不成立，`compact_if_needed` 以正常 ratio 运行 Tier 0/1/2/3，根本不会掉到 Tier 4（Tier 4 `emergency_compact` 仅由 ContextOverflow API 错误触发）。"高 usage 也不会触发分层保护"的前提不成立。
- **状态**：⛔ 划掉（原描述失效，当前逻辑正确）。

### ~~M2. Post-compaction 文件恢复预算会被打爆~~（原描述失效）

- **位置**：`crates/oc-core/src/context_compact/recovery.rs:100-116`
- **复核结论**：[`recovery.rs:54`](../../crates/oc-core/src/context_compact/recovery.rs#L54) 先把 `byte_budget = ((tokens_freed * 4) / 10).min(MAX_RECOVERY_TOTAL_BYTES)` 夹到 100KB 上限；循环体内 [line 111](../../crates/oc-core/src/context_compact/recovery.rs#L111) `if total_chars + truncated.len() + overhead > byte_budget { break; }` 已经兜住总量。XML overhead 每文件 `path.len() + 40` ≈ 100–300 bytes，5 文件总 overhead < 1KB，远不会把 80KB 数据打到 100KB 以上。"动态 clamp 单文件上限"至多是减少一次磁盘 I/O 的优化，不是 correctness bug。
- **状态**：⛔ 划掉（budget break 检查已兜底，上限不会被超过）。

### M3. 文件恢复消息插入位置硬编码为索引 1

- **位置**：`crates/oc-core/src/agent/context.rs:283`
- **因果**：`messages.insert(1, recovery_msg)` 假设摘要一定在 0。将来 `apply_summary()` 多塞一条说明就错位。
- **修复**：用 `split.preserved_start_index` 或语义化常量。
- **状态**：✅ 已修复（新增 `context_compact::POST_SUMMARY_INSERT_INDEX = 1` 常量并用 `.min(messages.len())` 兜底；插入点命名化以便后续改动不静默错位）。

### M4. Idle memory extract 句柄竞态

- **位置**：`crates/oc-core/src/memory_extract.rs:516-524`
- **因果**：先移除句柄、再比对 `expected_updated_at`；窗口内被新消息触发 `schedule_idle_extraction()`，新句柄写回但任务已在跑，可能重复执行提取，浪费 API + 重复记忆。
- **修复**：改 CAS（`dashmap::entry().or_insert_with_key`）或保留句柄直到任务结束再移除。
- **状态**：✅ 已修复（`run_idle_extraction` 入口改为 "先比对 `updated_at` 再 remove"——handle 三元组第三位存 `expected_updated_at`，不匹配时保留 map 中的新 handle，彻底消除"任务已在跑但新 schedule 注册的 handle 被误删"的窗口，把实现对齐到原有注释声明的语义）。

### M5. ask_user 题目级 + 组级超时冲突导致悬挂

- **位置**：`crates/oc-core/src/tools/ask_user_question.rs:142-162` + `channel/worker/ask_user.rs:351-410`
- **因果**：组超时 600s、单题 60s，单题过期后 `BUTTON_PENDING` 没被回收；group timeout 未到，后续按钮回调仍进入已死 question。
- **修复**：handler 顶部检查 `timeout_at`；过期立即清理 `BUTTON_PENDING`。
- **状态**：✅ 已修复（`channel/worker/ask_user.rs` 新增 `drop_if_expired()` helper 在 `handle_ask_user_callback` 入口按 `timeout_at` 淘汰 BUTTON_PENDING；`try_handle_ask_user_reply` 的 TEXT_PENDING 分支对 `entry.retain(|p| ...)` 同步清理。当前工具层已取 `per_q_max|global_default` 作统一组超时，但该防御兜底在 tool-side 清理延迟/会话消失等边缘情况下仍能防止过期组被按钮/文本回调重新激活）。

### M6. WeChat 入站媒体 file_id 路径验证弱

- **位置**：`crates/oc-core/src/channel/wechat/media.rs:477-517` + `channel/worker/media.rs:70-114`
- **因果**：只替换 `['/', '\\', ':']`，不处理 `..` 组合；`persist_channel_media_to_session` 没 `canonicalize()` 验证源路径真在 inbound-temp 下。符号链接攻击可复制任意文件进 attachments。
- **修复**：file_id 白名单（UUID 或 `[a-zA-Z0-9_-]+`），拷贝前 `canonicalize()` 校验前缀。
- **状态**：✅ 已修复（新增 `sanitize_file_id` 严格白名单 `[A-Za-z0-9_-]`，其他字符全部替换为 `_` 再 UTF-8 截断 64 字节；拷贝前 `src.canonicalize()` + `inbound_root.canonicalize()` + `starts_with` 前缀校验，解析失败或越界直接 warn 返回 None，不会把符号链接指向的任意文件复制进 session attachments）。

### ~~M7. Cron 重启抖动窗口内重复执行~~（原描述失效）

- **位置**：`crates/oc-core/src/cron/scheduler.rs:88`；`cron/db.rs:445-450`
- **复核结论**：`cron::scheduler` 调度循环已经用 [`claim_job_for_execution()`](../../crates/oc-core/src/cron/db.rs#L536) 做原子抢占——SQL 是 `UPDATE cron_jobs SET running_at=?, next_run_at=? WHERE id=? AND next_run_at=? AND status='active' AND running_at IS NULL`，只有抢到 `running_at IS NULL` 的行才 spawn `execute_job`；同时还维护 `tick_running: AtomicBool` 防止同进程 tick 重叠。15s 轮询 + 原子 claim 已经覆盖"重启抖动窗口同秒再跑"的风险。
- **状态**：⛔ 划掉（原子抢占已实现，本次复核确认）。

### M8. Replay pending async jobs 非原子 select + dispatch

- **位置**：`crates/oc-core/src/async_jobs/mod.rs:81-96`
- **因果**：列 `injected=0` → dispatch；若多实例或事件重入，同 job 会被重复 dispatch；dispatch 失败无重试记录。
- **修复**：单事务内 `UPDATE ... RETURNING` 或 `UPDATE ... SET dispatching=1 WHERE injected=0` 抢占后再投递。
- **状态**：✅ 已修复（`async_jobs/injection.rs` 引入进程级 `DISPATCHING: OnceLock<Mutex<HashSet<String>>>` 注册表 + `try_claim_dispatch`/`release_dispatch`；`dispatch_injection` 在 spawn 前先 insert job_id，重复调用直接 skip，dispatch 线程用 `DispatchGuard` RAII 无论成功/失败/runtime 构建异常都释放 slot。跨进程（desktop + server 同时跑）的 DB 级 claim 需要新列，留待后续；当前进程级互斥已覆盖 startup replay + EventBus 重入 两大常见竞争源）。

### M9. Plan 状态机缺合法转移校验

- **位置**：`crates/oc-core/src/plan/types.rs:7-44`；`plan/store.rs` 所有 setter
- **因果**：六态间可任意跳转（Completed → Executing 也允许），并发请求可致步骤重复执行或跳过 git checkpoint。
- **修复**：`fn is_valid_transition(from, to) -> bool`，store 写入前强制校验。
- **状态**：✅ 已修复（`PlanModeState::is_valid_transition` 列出所有合法迁移：Planning↔Review、Review→Executing、Executing→Paused/Completed、Paused→Executing/Planning、Completed→Planning、同态始终允许，`Off` 作为逃生阀双向开放；`set_plan_state` 在 `get_mut` 分支内先调该 helper，非法迁移记 warn 日志后 return，不再让"Completed → Executing"静默改写重新执行步骤）。

### M10. Plan 文件名秒级时间戳碰撞

- **位置**：`crates/oc-core/src/plan/file_io.rs:31-35, 61-84`
- **因果**：`%Y%m%d-%H%M%S`，同秒多会话生成同名文件互相覆盖；version 计数器从内存拿、重启归零，覆盖旧版本快照。
- **修复**：加纳秒 / UUID 后缀；版本号启动时从磁盘 `max(version)+1` 初始化。
- **状态**：✅ 已修复（`plan_file_path` / `result_file_path` 改用 UTC `%Y%m%dT%H%M%SZ` + `timestamp_subsec_nanos` 9 位零填充后缀，彻底消除同秒跨会话碰撞；`save_plan_file` 新增 `max_disk_version()` 扫描 `{stem}-v{N}.md` 找历史最大版本号，`current_version = mem_version.max(max_disk + 1)` 保证重启后首次备份不会覆盖已有 `plan-xxx-v1.md`）。

### ~~M11. 前端 `useNotificationListeners` stale closure~~（原描述失效）

- **位置**：`src/components/chat/hooks/useNotificationListeners.ts:200`
- **复核结论**：useEffect 闭包内用到的非 `reloadSessions` 项分别是 refs（`currentSessionIdRef` / `loadingSessionsRef` / `sessionCacheRef`）和 React `useState` 返回的 setters（`setMessages` / `setLoading` / `setLoadingSessionIds`）。refs 的身份 identity 永不变；setters 在 React 里官方承诺跨渲染稳定。这些都不需要进依赖数组，闭包也不会持旧引用——`.current` 总是读到最新 ref 值。eslint-disable-line 注释合理，没有真实 stale closure 风险。
- **状态**：⛔ 划掉（refs + setState 均稳定，原风险不存在）。

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
- **状态**：✅ 已修复（[`AskUserQuestionBlock.tsx`](../../src/components/chat/ask-user/AskUserQuestionBlock.tsx) 倒计时 chip 包进 `@/components/ui/tooltip` 的 `IconTip`，走项目统一 tooltip 组件；红线"必须用 ui/tooltip 不用原生 title"恢复）。

### P3. `tauri.conf.json` CSP 仍为 `null`

- **位置**：`src-tauri/tauri.conf.json:30`
- **因果**：红线里列为禁止项。Streamdown 渲染任意 Markdown + 外部图片，一旦出现 XSS 无兜底。
- **修复**：至少 `default-src 'self'; img-src 'self' data: https:; script-src 'self'`，按需渐进放开。
- **状态**：待修复。

### P4. Broadcast slow consumer 未处理 `Lagged`

- **位置**：`crates/oc-core/src/event_bus.rs:17, 40-42`
- **因果**：订阅方 hang / panic / 消费慢，滞后 receiver 不被主动回收；同一 channel 其他消费者会因 lag 错过事件。无消费者心跳、无超时关门。
- **修复**：订阅循环显式 match `RecvError::Lagged` 并立即 re-sync；后台扫描过期订阅配合 keep-alive ping 强制 drop。
- **状态**：✅ 已修复（复核 5 处订阅点：`oc-server/ws/chat_stream.rs` / `oc-server/ws/events.rs` / `channel/worker/approval.rs` / `channel/worker/ask_user.rs` 共 4 处 `.subscribe()` 已实现 Lagged 处理；唯一漏网的 [`src-tauri/src/setup.rs`](../../src-tauri/src/setup.rs) 的 EventBus → Tauri frontend bridge 原本 `Err(RecvError::Lagged(_)) => continue` 静默，现改为 `app_warn!` + `app_handle.emit("_event_bus_lagged", { missed: n })`，前端能感知到有事件被丢弃。背后的 broadcast 容量仍是 256，正常负载下不会触发）。

### P5. OAuth 刷新 margin 偏大且被动

- **位置**：`crates/oc-core/src/oauth.rs:64-76`；Codex provider
- **因果**：60s margin 仍可能在网络抖动中发送即过期 token；Codex 纯被动靠 Auth error 重试，首次失败会让用户看到一次可见错误。
- **修复**：margin 降到 30s，Codex 发请求前主动探测 + 异步 `refresh_token` 续期。
- **状态**：✅ 已修复（[`oauth.rs`](../../crates/oc-core/src/oauth.rs) 引入 `REFRESH_MARGIN_MS = 30_000` 常量，`is_token_expired` 从硬编码 60 000 ms 改引用常量；新增 `ensure_fresh_codex_token(current_access_token: &str) -> Option<(String, String)>`：in-memory token 与 disk 一致且未过期时返回 `None` 让调用方跳过无谓工作，disk 更新或 near-expiry 时就地 refresh 并返回新 `(access_token, account_id)` 对。[`chat_engine/engine.rs`](../../crates/oc-core/src/chat_engine/engine.rs) 在进入 `model_chain` 循环前调用，只在持有 Codex token 时触发；服务端 / 桌面 / ACP 三种运行模式共享该路径）。

### P6. FTS5 `sanitize_fts_query` 空回退 `"*"` 扫全库

- **位置**：`crates/oc-core/src/memory/helpers.rs:5-30`
- **因果**：空查询或全被过滤的 query 回退匹配所有记录，大库（10 万+）直接全表扫 + snippet 计算 → 卡顿或阻塞 SQLite。
- **修复**：空查询直接返回空结果；或强制 `LIMIT` + `ORDER BY rowid DESC LIMIT 50`。
- **状态**：✅ 已修复（[`memory/helpers.rs`](../../crates/oc-core/src/memory/helpers.rs) `sanitize_fts_query` / `expand_query` 返回类型从 `String` 改为 `Option<String>`，全部被 stopword 过滤 / 空白输入下返回 `None`。[`memory/sqlite/trait_impl.rs:217`](../../crates/oc-core/src/memory/sqlite/trait_impl.rs#L217) 的 `search` 从无条件 `expand_query` 改为 `if let Some(fts_query) = ... { FTS step } else { 跳过 }`，None 时跳过 FTS5 MATCH 扫描直接走 vector 分支，hybrid RRF 自然吸收空 FTS 结果。`"*"` 全库扫退化路径彻底消失）。`session/db.rs` 的 `sanitize_fts_query` 本地副本已 early-return 空结果，不受影响。

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
- **状态**：✅ 已修复（复核 [`useNotificationListeners.ts`](../../src/components/chat/hooks/useNotificationListeners.ts) 两个 effect：Tauri transport 的 `listen` 内部已有 `cancelled` 守卫、HTTP transport 的 `listen` 同步解绑，subscribe / unsubscribe 实际上已对齐。只做风格统一——把第一个 `return getTransport().listen(...)` 的隐式 return-unsub 写法改为 `const unlisten = ...; return unlisten`，与第二个 effect 一致，让未来阅读者一眼看清 cleanup 就是 unlisten）。

### P10. Plan git 分支命名用 `Local` 时间 + 非原子

- **位置**：`crates/oc-core/src/plan/git.rs:30-54`
- **因果**：本地时区 + 秒级时间戳，不同设备 / 并发易重名；`git branch` 非原子，回滚可能打到错 checkpoint。
- **修复**：UTC + 短 UUID；创建前 `git rev-parse --verify` 去重。
- **状态**：✅ 已修复（[`plan/git.rs`](../../crates/oc-core/src/plan/git.rs) 分支模板 `opencomputer/checkpoint-{short_id}-{ts}-{uuid8}`，`ts` 从 `chrono::Local::now().format("%Y%m%d%H%M%S")` 改成 `chrono::Utc::now().format("%Y%m%dT%H%M%SZ")`，外加 8 位 `uuid::Uuid::new_v4().simple()` 尾缀消除跨设备 / 并发碰撞。新增 `ref_exists(git_root, rev)` helper 走 `rev-parse --verify --quiet`，`create_git_checkpoint` 在 `git branch` 之前先查，已存在就 warn 后 return None；`rollback_to_checkpoint` 的原 inline verify 分支也收敛到同一 helper，消除重复代码）。

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

| 档次 | 数量 | 本轮处理 | 集中区域 |
|---|---|---|---|
| 🔴 严重 | 10 | 8 ✅ / 2 ⛔ | `context_compact` UTF-8 切片、tool loop 终止、async_jobs 注入、server 鉴权、service install 命令注入 |
| 🟠 中等 | 11 | 7 ✅ / 4 ⛔ | recovery 插入点、idle extract 竞态、ask_user 超时、WeChat 路径、async job claim、plan state machine、plan 文件名 |
| 🟡 性能/稳定 | 10 | – | XSS、CSP、broadcast lag、SSRF、日志 token |
| ℹ️ 轻微/设计 | 5 | – | SQL 拼接习惯、孤儿清理、排序字段、CI 校验、工具上下文共享 |

**2026-04-17 修复批次（分支 `claude/fix-audit-bugs-Ip6Bl`）**

| ID | 处理结果 |
|---|---|
| B1 UTF-8 切片 | ✅ 切换到 `truncate_utf8` / `truncate_utf8_tail` + 新增 `floor_char_boundary` / `ceil_char_boundary` helper |
| ~~B2 Tier 3 切分~~ | ⛔ 复核后确认 round 边界逻辑已保证 tool_use/tool_result 不分离，原描述失效 |
| B3 Tool loop 末轮 | ✅ 4 个 provider 最后一轮移除 `tools`，模型被迫出文本 |
| B4 mark_injected | ✅ 注入路径改走 4 次退避 + EventBus 报警 |
| B5 Query 解码 | ✅ 新增 `percent_decode_form_value` + 单测 |
| B6 常量时间 | ✅ 新增 `constant_time_eq` + 单测 |
| B7 Service install | ✅ launchd XML 转义 + systemd ExecStart 单 arg 引号 |
| B8 ProfileSticky LRU | ✅ `StickyShard` 本地 LRU，上限只驱逐最旧 + 2 条单测 |
| B9 Project 删除 | ✅ `purge_project_files_dir` canonicalize 校验前缀 |
| ~~B10 broadcast 丢事件~~ | ⛔ `job_status` 已改用 `Notify` 注册表，不再依赖 broadcast |

**2026-04-18 中等 Bug 修复批次（分支 `claude/fix-audit-bugs-BcytF`）**

| ID | 处理结果 |
|---|---|
| ~~M1 Cache-TTL emergency~~ | ⛔ `cache_ttl_emergency` 仅作为 throttle 旁路开关，TTL 过期时 throttle 本就失效，无需解耦 |
| ~~M2 Recovery budget~~ | ⛔ 循环内 break 检查已兜住 `MAX_RECOVERY_TOTAL_BYTES`，budget 不会被超过 |
| M3 Recovery insert index | ✅ 引入 `POST_SUMMARY_INSERT_INDEX` 常量 + `.min(len)` 兜底 |
| M4 Idle extract CAS | ✅ `run_idle_extraction` 入口改为"匹配 `expected_updated_at` 才移除 handle" |
| M5 ask_user 超时 | ✅ button 回调前 `drop_if_expired`，text 回调前按 `timeout_at` retain |
| M6 WeChat 路径 | ✅ file_id 严格白名单 + `canonicalize()` 前缀校验 |
| ~~M7 Cron 原子抢占~~ | ⛔ `claim_job_for_execution` 已原子 UPDATE，本轮复核确认 |
| M8 Async replay claim | ✅ 进程级 `DISPATCHING` HashSet + `DispatchGuard` RAII 释放 |
| M9 Plan state | ✅ `PlanModeState::is_valid_transition` + `set_plan_state` 写入前强制校验 |
| M10 Plan 文件名 | ✅ UTC + 纳秒后缀消除同秒碰撞；版本号 `max(mem, disk+1)` 重启安全 |
| ~~M11 stale closure~~ | ⛔ refs 与 setState 均稳定，eslint-disable-line 合理 |

**2026-04-18 稳定性/性能批修复（分支 `worktree-fix-audit-p2-p4-p5-p6-p9-p10`）**

| ID | 处理结果 |
|---|---|
| P2 AskUserQuestionBlock `title` | ✅ 倒计时 chip 用 `@/components/ui/tooltip::IconTip` 替换原生 `title` |
| P4 EventBus 订阅 Lagged | ✅ 复核 4 处订阅点已处理 Lagged，补齐 Tauri bridge 第 5 处 + emit `_event_bus_lagged` 给前端 |
| P5 OAuth 预刷新 | ✅ margin 60s → 30s，新增 `oauth::ensure_fresh_codex_token`，engine 进入 model_chain 前 per-chat 预刷新 |
| P6 FTS5 空回退 | ✅ `sanitize_fts_query` / `expand_query` 返回 `Option<String>`，`search` 在 None 时跳过 FTS step |
| P9 Notification listeners | ✅ 两个 effect 的 subscribe/unsubscribe 对齐，统一 `const unlisten = ...; return unlisten` 写法 |
| P10 Plan git checkpoint | ✅ UTC + 8 位 UUID 尾缀 + `ref_exists` 去重，`rollback_to_checkpoint` 复用 helper 消除 dup |

**修复顺序建议（剩余批次）**

1. 第二批（安全加固批）：P1、P3、P7、P8 已在独立分支落地 —— 等待 PR 合入后本文件会整合
2. 其余设计类（D1–D5）按迭代节奏推进。

## 复核建议

- 每条修复前务必 Read 对应 `file:line` 确认当前实际代码（sub-agent 报告基于静态分析，可能因近期改动失真）
- 可将每条转为独立 issue / PR，便于并行修复与 code review
- 对 UTF-8 切片类问题可一次性全局 grep `&[a-zA-Z_]+\[\.\.` 做批量治理
