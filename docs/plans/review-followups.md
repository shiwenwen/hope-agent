# Review Followups — 审查决定但本期不改的问题

> 本文档登记**已被 code review 识别、但当期 PR 决定不修**的问题。每条记录的目的是：让债务可见、可检索、可调度，避免下一次有人撞上同一个问题再重新发现。
>
> 登记规则见 [AGENTS.md](../../AGENTS.md) "Review Followups 登记"段。

## 文档使用方式

- **新增一条 Follow-up**：在最下方"Open"段追加一个 `### F-XXX` 子节，编号递增（不复用），按下方"条目模板"填写。一次提交一个原子条目；多个 review 想法分开记。
- **关闭一条**：把整段从 "Open" 移到底部 "Closed" 段，附 commit / PR 链接和关闭日期；不要原地删除（保留可检索的历史）。
- **不强制顺序**：可以打散在多个版本里慢慢清。
- **不当作 backlog**：这里只放"review 决定不改"的；功能 backlog 放别处（issue tracker / 其他 plan）。

## 条目模板

每条 Follow-up 至少包含：

```
### F-XXX 简短标题

- **来源**：YYYY-MM-DD `<功能名>` PR / `/simplify` review / 手动审查
- **现象**：一两句描述当前是什么样
- **为什么留**：当期不修的具体理由（范围 / 优先级 / 依赖 / 风险）
- **改的话要做什么**：列出涉及文件、需要的设计决策、可能的迁移路径
- **影响面**：当前是否有用户可见的 bug / 安全 / 性能问题；如果只是"不优雅"就明说
- **触发时机建议**：什么场景下应该顺手收掉（例如 "下一次动这块代码时" / "做某某独立重构 PR 时"）
```

---

## Open

### F-032 `SessionMeta.plan_mode` 仍是 `String`，应换 `PlanMode` enum

- **来源**：2026-04-30 F-029 收尾 `/simplify` review（quality agent）
- **现象**：[`crates/ha-core/src/session/types.rs:33`](../../crates/ha-core/src/session/types.rs) 把 `plan_mode` 存为 `String`（取值 `"off" | "planning" | "executing"`），与 F-029 之前的 `permission_mode: String` 完全同形。Plan 系统已经有相关 enum（如 `agent/types.rs::PlanAgentMode`），但 `SessionMeta` 上的字段仍走字符串
- **为什么留**：本期 F-029 commit message 显式 scope 为 `permission_mode`，刻意没扩展。Plan domain 改动需要单独 audit DB 列读写、IM channel ensure_conversation、前端 PlanPanel 逻辑等
- **改的话要做什么**：
  1. 在 `permission/mode.rs` 的隔壁（或 `plan/mode.rs`）定义 `PlanMode { Off, Planning, Executing }` enum，加 serde rename_all snake_case + Default + parse_or_default
  2. `SessionMeta.plan_mode: PlanMode`，DB 读用 parse_or_default
  3. 检查所有 `m.plan_mode == "off"` / `"planning"` / `"executing"` 的 stringly compare（grep），改成 enum 匹配
  4. Tauri / HTTP 命令：`set_plan_mode` 之类入参可改成 enum；前端 `PlanMode` union 类型保留不变
- **影响面**：纯整洁。和 F-029 风险等级一致——stringly typed 字段拼写错误会 silently fall through 到默认行为
- **触发时机建议**：下次给 Plan 加新模式（如 `"reviewing"`）时；或独立"session 元数据 stringly-typed 收口"小 PR 一次清掉

---

### F-022 SkillsPanel 三个 Switch handler 失败处理风格不一致

- **来源**：2026-04-29 Skill 自动审核 UI 信号 PR `/simplify` review（reuse agent）
- **现象**：[`src/components/settings/skills-panel/index.tsx`](../../src/components/settings/skills-panel/index.tsx) 在同一个面板里有三个 Switch handler，失败处理风格不齐：
  - `handleSetAutoReviewPromotion`（line ~228）和 `handleSetAutoReviewEnabled`（line ~253）：本期新加，**乐观更新 + 失败 rollback** —— `setAutoReview…(v) → call → catch → setAutoReview…(previous)`，后端写失败 UI 自动滚回旧值
  - `handleSetSkillEnvCheck`（line ~217）：早期代码，**乐观更新 + 不 rollback** —— `setSkillEnvCheck(v) → await call(...)`，后端如果抛异常 UI 已经切到新值但 config 没存，状态永久不一致直到下次 reload
- **为什么留**：本期 PR 主题是"auto-review UI 信号 + 自动激活开关"，`handleSetSkillEnvCheck` 是 pre-existing 不相关代码。AGENTS.md 风格"a bug fix doesn't need surrounding cleanup"——本 PR 给新代码加 rollback 是新代码本应做对的事，去补一个 pre-existing handler 算超范围
- **改的话要做什么**：
  1. 把 `handleSetSkillEnvCheck` 改成同样的 try/catch + rollback：
     ```ts
     async function handleSetSkillEnvCheck(v: boolean) {
       const previous = skillEnvCheck
       setSkillEnvCheck(v)
       try {
         await getTransport().call("set_skill_env_check", { enabled: v })
       } catch (e) {
         logger.error("settings", "SkillsPanel::setSkillEnvCheck", "Failed to update", e)
         setSkillEnvCheck(previous)
       }
     }
     ```
  2. 顺手扫一遍 [`src/components/settings/`](../../src/components/settings/) 下其它 panel 的 Switch handler，是否也有同样的"乐观更新无 rollback"模式（特别是 ToolSettingsPanel / ChatSettingsPanel / PlanSettingsPanel 这类直接 await call 的）；如果范围大可以抽 `useOptimisticToggle(value, setter, callback)` 共用 hook
- **影响面**：纯一致性 + 边缘 bug。后端 `set_skill_env_check` 走 `mutate_config` 几乎不会失败（除非配置文件磁盘异常），实际触发概率低；触发时用户看到 Switch 显示开但实际行为没切换，要刷新或重新点才会自愈
- **触发时机建议**：下一次动 SkillsPanel（加新 Switch / Setting）时顺手收掉；或独立"settings panel optimistic-toggle 一致化"小 PR 把全 settings 目录扫一遍

---

### F-021 `acp/agent.rs` 每 RPC 新建 tokio runtime + Codex token 每 retry 重复 load

- **来源**：2026-04-27 chat-engine subagent 收敛 PR `/simplify` review（efficiency agent）
- **现象**：
  - [`crates/ha-core/src/acp/agent.rs::build_agent`](../../crates/ha-core/src/acp/agent.rs) 在每次 RPC 请求里 `tokio::runtime::Builder::new_current_thread().enable_all().build()?` 新建一个 runtime 只为 `block_on` 一次 `try_new_from_provider`。`build_agent` 与 `run_agent_chat` 各自 build 自己的 runtime——同一个 "new session → prompt" 序列会发两次 runtime 分配 / 销毁
  - `run_agent_chat` 的 `model_chain × retry` 循环里每次 attempt 都会跑一次 `try_new_from_provider`，对 Codex 走的是 `oauth::load_fresh_codex_token()`——内部**没有**进程级缓存，每次都是 disk read（可能再叠 token endpoint roundtrip）。N model × M retry 次失败可重试场景下放大很明显
- **为什么留**：
  - 收敛 runtime 需要把 `Runtime` 实例挂到 `AcpAgent` 上，构造 / shutdown 顺序要重排——ACP 入口是 sync stdio 主循环，没有外层 runtime 可借（`Handle::try_current()` / `block_in_place` 都不可行），改动有顺序敏感性
  - `oauth::load_fresh_codex_token` 加 in-memory cache 涉及锁 / TTL 选择 / refresh-when-near-expiry 边界，得跟 `ensure_fresh_codex_token` 已有的"prime 后写盘"路径协调，不是单点替换
  - ACP 是低频调用路径（每个 RPC ~人手速度），实际产线压力低，不阻塞 chat-engine 收敛主目标
- **改的话要做什么**：
  1. 在 [`AcpAgent::new`](../../crates/ha-core/src/acp/agent.rs) 持有 `Arc<tokio::runtime::Runtime>`，`build_agent` / `run_agent_chat` 改成 `&self.rt` 复用；构造在 `new` 里失败也 `Result<Self>` 回报
  2. 在 [`crates/ha-core/src/oauth.rs`](../../crates/ha-core/src/oauth.rs) 加进程级 `OnceCell<Mutex<Option<TokenCache>>>` 缓存，`load_fresh_codex_token` 优先读缓存；写盘路径（`refresh_access_token` / `save_token`）同步 invalidate 缓存。或者在 `AcpAgent::run_agent_chat` 顶部一次性 `load_fresh_codex_token` 然后逐 retry 直接构造 `LlmProvider::Codex { ... }`，绕过外层 `try_new_from_provider`
- **影响面**：纯效率，无可见 bug。runtime 浪费每次 ~ms 级（本地默认 num_workers=1），token reload 在网络抖动期会放大失败 latency。Codex 用户在 ACP 模式失败重试时最容易感知
- **触发时机建议**：下一次动 ACP（新协议字段、prompt routing 改动）或 Codex OAuth 流程（refresh logic / 新 grant）时顺手收掉；或独立 "ACP runtime / OAuth caching" 重构 PR

---

### F-020 `ChatEngineParams` 7 个新 boolean / option 字段应收敛成 `ExecutionMode` 枚举

- **来源**：2026-04-27 chat-engine subagent 收敛 PR `/simplify` review（quality agent）
- **现象**：[`crates/ha-core/src/chat_engine/types.rs::ChatEngineParams`](../../crates/ha-core/src/chat_engine/types.rs) 在本期为统一 subagent / parent injection 路径加了 7 个新字段：`denied_tools`、`subagent_depth`、`steer_run_id`、`follow_global_reasoning_effort`、`post_turn_effects`、`abort_on_cancel`、`persist_final_error_event`。实际只有两个语义轴：
  - **Foreground**（4 处：[`src-tauri/src/commands/chat.rs`](../../src-tauri/src/commands/chat.rs)、[`crates/ha-server/src/routes/chat.rs`](../../crates/ha-server/src/routes/chat.rs)、[`crates/ha-core/src/channel/worker/dispatcher.rs`](../../crates/ha-core/src/channel/worker/dispatcher.rs)、[`crates/ha-core/src/cron/executor.rs`](../../crates/ha-core/src/cron/executor.rs)）— 全部 `follow_global_reasoning_effort: true, post_turn_effects: true, abort_on_cancel: false, persist_final_error_event: true`
  - **Background**（2 处：[`crates/ha-core/src/subagent/spawn.rs`](../../crates/ha-core/src/subagent/spawn.rs)、[`crates/ha-core/src/subagent/injection.rs`](../../crates/ha-core/src/subagent/injection.rs)）— 全部反向：`false, false, true, false`
- **为什么留**：4 个 boolean 完美关联，确实可以收敛成 `enum ExecutionMode { Foreground, Background { abort_on_cancel: bool } }` + `denied_tools / subagent_depth / steer_run_id` 也只在 Background 非默认。但改动要触达 ha-core / ha-server / src-tauri 三个 crate 的 6 个调用点，本期 `/simplify` 已经在做 subagent 收敛 + ChatSource 谓词抽取 + image_gen helper 抽取等多项整理，再叠加 enum 重构会让 PR 进一步膨胀，超出 simplify 单次合理范围
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/chat_engine/types.rs`](../../crates/ha-core/src/chat_engine/types.rs) 新增 `pub enum ExecutionMode { Foreground, Background { abort_on_cancel: bool, denied_tools: Vec<String>, subagent_depth: u32, steer_run_id: Option<String> } }`
  2. 把 `follow_global_reasoning_effort` / `post_turn_effects` / `persist_final_error_event` 三个固定相关字段从 `ChatEngineParams` 删除，由 `mode.is_foreground()` 推导
  3. 给 `ChatEngineParams` 加 `pub fn foreground(...)` / `pub fn background(...)` 构造函数，6 个调用点全部改成 builder 风格
  4. 同步更新 [`docs/architecture/chat-engine.md`](../../docs/architecture/chat-engine.md) 如果有的话
- **影响面**：纯整洁度。当前所有调用点都正确，但 `false / true` 字面量噪声大，新增第 7 个 caller（例如未来 ACP 走 chat_engine）时容易漏字段（编译错保住但语义对不齐 review 才能抓）
- **触发时机建议**：下次有第 7 个 chat_engine 调用点要新加（例如 ACP 改走 `run_chat_engine` 复用主路径），或下次需要再加第 8 个 mode-related boolean / option 字段时一次性收掉；不要单独立 PR

---

### F-019 SSE 解析器在 4 处 LLM / IM stream 重复实现

- **来源**：2026-04-26 F-004 重新核查时分流出来
- **现象**：4 处 `bytes_stream` SSE 解析各自手写 buffer + `find("\n\n")` / `find('\n')` + `event:` / `data:` 拆解，结构相似但实现细节有出入：
  - [`crates/ha-core/src/agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs)（`\n\n` event boundary，多 `data:` 行 join）
  - [`crates/ha-core/src/agent/providers/openai_chat_adapter.rs`](../../crates/ha-core/src/agent/providers/openai_chat_adapter.rs)
  - [`crates/ha-core/src/agent/providers/openai_responses_adapter.rs`](../../crates/ha-core/src/agent/providers/openai_responses_adapter.rs)
  - [`crates/ha-core/src/channel/signal/client.rs`](../../crates/ha-core/src/channel/signal/client.rs)（line-based + 空行 boundary，结构等价）
- **为什么留**：抽公共 SSE parser 需要先统一 event 数据结构（`SseEvent { event, data, id, retry }`）+ 决定多 `data:` 行 join、`\r\n`、`:` 注释行、`retry` 字段处理。3 个 LLM provider adapter 是聊天热点路径，重构必须有逐 frame 等价测试兜底，独立 PR 范围。
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs)（或新建 `util/sse.rs`）加 `pub fn sse_event_stream<S>(stream: S, max_buffer_bytes: usize) -> impl Stream<Item = Result<SseEvent>>`
  2. 用 `tokio_util::io::StreamReader` + `AsyncBufReadExt::lines()` 逐行收 `event:` / `data:` / `id:` / `retry:` / 空行 boundary，多 `data:` 按 SSE 规范 `\n` join
  3. 替换 4 处 inline 解析；保留各 caller 自己的 event-name 分支与 payload 反序列化
- **影响面**：纯整洁度，当前无可见 bug；但 SSE spec 边界条件 4 处实现各有遗漏，新增 SSE 接入点时容易再走偏
- **触发时机建议**：下一次新增 SSE 接入点（OpenAI 新流式模式 / 新 IM channel SSE 入站）时顺手抽；或独立 "SSE parser 统一" 重构 PR

---

### F-013 EventBus 事件名常量散落，应有 events 常量模块

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：EventBus 事件名当前混合两种风格：
  - **Rust 常量**：[`crates/ha-core/src/chat_engine/stream_broadcast.rs::EVENT_CHAT_STREAM_DELTA`](../../crates/ha-core/src/chat_engine/stream_broadcast.rs)、[`crates/ha-core/src/docker/mod.rs::EVENT_SEARXNG_DEPLOY_PROGRESS`](../../crates/ha-core/src/docker/mod.rs)、[`crates/ha-core/src/local_llm/mod.rs`](../../crates/ha-core/src/local_llm/mod.rs) 的 `EVENT_LOCAL_LLM_*`
  - **前端独立常量 / 字面量**：前端仍各自维护同值（例如本地小模型进度事件、`useChatStreamReattach.ts` 的 `EVENT_CHAT_STREAM_DELTA`），缺少跨 Rust/TS 的单一来源
- **为什么留**：跨前端（TS）/ 后端（Rust）同步常量需要 codegen 或 wire-format 文档约定，引入新约束。本期把刚碰到的 searxng 升成常量已经是最低成本的"按碰到逐步收"。
- **改的话要做什么**：候选方案：
  - **A**：每个子系统在自己 mod 顶部定义 `pub const EVENT_*: &str = "..."`（已经 chat / searxng 在做）；前端继续维护独立常量但加注释指向 Rust 同名定义。Rust 端集中调用，前端只 listen 时用一次，漂移风险低
  - **B**：用 `build.rs` 生成 TS const 文件，从 Rust 单一来源。需要新增 build pipeline 复杂度
- **影响面**：纯整洁度。事件名漂移会被 watchdog 测试快速发现（事件不到达 → UI 不更新），是 "fail loud" 类型的 bug。
- **触发时机建议**：等再积累 2-3 个新事件名（看 local_llm 之外）时一次性把所有 `local_llm:*` / 其它字面量升成常量；不必单独立 PR。

---

### F-016 LocalModelJobsDB 与 AsyncJobsDB 大量重复

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **现象**：[`crates/ha-core/src/local_model_jobs.rs`](../../crates/ha-core/src/local_model_jobs.rs) 重新实现了与 [`crates/ha-core/src/async_jobs/`](../../crates/ha-core/src/async_jobs/) 几乎一一对应的基础设施：
  - 状态枚举 `LocalModelJobStatus { Running, Cancelling, Completed, Failed, Interrupted, Cancelled }` ↔ `AsyncJobStatus`（多一个 `TimedOut`）
  - `is_terminal()` + `TERMINAL_SQL_LIST`
  - `LocalModelJobsDB::open` 的 PRAGMA WAL/NORMAL + CREATE TABLE 模板
  - `mark_interrupted_running` / `mark_cancelling` 的 lifecycle 逻辑
  - `static CANCELS: Mutex<HashMap<String, CancellationToken>>` 取消注册表（`async_jobs::cancel` 已有）
  - `now_secs()` 时间戳助手（`async_jobs::spawn` 已有）
  - `row_to_job` 行解析模板
- **为什么留**：`local_model_jobs.rs` 顶部注释明确说"故意与 async_jobs 分离：那些是工具调用结果，本模块是用户可见的安装任务"——确实需要不同的 payload schema 与 UI 语义，但 *基础设施层*（DB scaffold / cancel registry / lifecycle）是可以共享的。统一需要把 async_jobs 的相关基元抽到一个 `crate::async_jobs::scaffolding` 层，工程量大且涉及现有 async_jobs 的回归风险，本期 PR 已经过大不再叠加。
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/async_jobs/` 抽出 `lifecycle.rs`：`CommonJobStatus` enum + `is_terminal` + `TERMINAL_SQL_LIST` + `mark_interrupted_running` 通用模板
  2. 把 `cancel.rs::CANCELS` 和 helper（`register_job_token` / `cancel_job` / `remove_job`）改成 generic by job-id 字符串，让 `local_model_jobs` 直接复用而不是另开一份
  3. `local_model_jobs::LocalModelJobsDB::open` 把 PRAGMA + CREATE 步骤拆出 `init_journal_pragmas(&conn)` helper
  4. `now_secs()` 移到 `crate::time` 或 `crate::util`
- **影响面**：纯整洁度，没有 bug。但现状下任何对 async_jobs 基础设施的改动（如新增 status / 改 cancel 协议 / 调 PRAGMA）都需要在 local_model_jobs 平行复制一份，长期维护成本。
- **触发时机建议**：下一次有人需要再加第三类用户可见后台任务（例如"批量索引项目文件"或"长时间 web search"）时一并抽 scaffolding；或独立 "async_jobs scaffolding 抽出" 重构 PR。

---

### F-018 SQLite 写在 tokio worker 上同步串行成为高频进度场景的瓶颈

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **现象**：[`crates/ha-core/src/local_model_jobs.rs::LocalModelJobsDB`](../../crates/ha-core/src/local_model_jobs.rs) 的 `conn: Mutex<Connection>`（`std::sync::Mutex`）在 pull 进度风暴中由 reqwest stream 回调以同步方式持锁；同一把锁也是 `list_jobs` / `get_job` / `cancel_job` 的读路径锁。多 job 并行时 tokio worker 互相阻塞；本期已加 250 ms / phase-change 节流（`ProgressThrottle`）把帧率压到 ~4 Hz 缓解，但 SQLite IO 仍在 worker 线程上。
- **为什么留**：节流后的 4 Hz 写入 + 100 行上限的 GC 已经远低于会成为瓶颈的水平，本期实测无可见卡顿；改成 `spawn_blocking` 或单线程 writer task 是结构性优化但需要重新设计 read/write 分离与 cancel 路径，工程量与风险与本期收益不匹配。
- **改的话要做什么**：候选两条：
  - **A**：所有 SQL 调用包 `spawn_blocking`，retain `Arc<Mutex<Connection>>` 但避免占 worker
  - **B**：dedicated writer task：`mpsc::UnboundedSender<WriterCmd>` + 独立 thread 持 connection，`update_progress` / `append_log` / `mark_*` 改成发消息；读路径用独立 read-only connection（SQLite WAL 允许并发读）
  - 推荐 B，与 dashboard / session DB 的潜在统一更大
- **影响面**：极端场景（多个并发 GB 级 pull + 大量并发 list_jobs 查询）下可能出现 worker stall；现实中很难触发。
- **触发时机建议**：如果未来要支持"批量预拉模型"（多 job 并行）或观察到 tokio worker stall，再处理。

---

### F-022 Diff 面板缺 Shiki 行级语法高亮 + 大 diff 虚拟列表

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`src/components/chat/diff-panel/UnifiedDiffView.tsx`](../../src/components/chat/diff-panel/UnifiedDiffView.tsx) / [`SplitDiffView.tsx`](../../src/components/chat/diff-panel/SplitDiffView.tsx) 当前直接渲染纯文本 diff 行，没有按 `metadata.language` 做语法高亮。Plan 文件 [`diff-plan-foamy-wave.md`](../../../.claude/plans/diff-plan-foamy-wave.md) 第 16 条原本要求新建 `diff-panel/diffShiki.ts` 复用项目 Shiki 实例。同时大文件 diff（>1000 行）当前一次性渲染所有行，没有虚拟列表
- **为什么留**：Shiki 行级 token 高亮要先解决"对每行单独高亮 vs 对整文件高亮再按行切"的取舍 + Shiki 实例在 streamdown 内的访问方式不直接 + 大文件性能优化（虚拟列表 / `requestIdleCallback` 分批）。本期 MVP 优先把 diff 面板可用打通，纯文本 diff 已能满足"看改了什么"的最低需求
- **改的话要做什么**：
  1. 找到 streamdown / [`src/lib/`](../../src/lib/) 下现有 Shiki 实例并暴露稳定 API（可能需新建 `src/lib/shiki/highlight-line.ts`）
  2. 新建 `src/components/chat/diff-panel/diffShiki.ts`：`highlightLine(text, language) -> React.ReactNode`，fallback 返回 `<span>{text}</span>`
  3. UnifiedDiffView / SplitDiffView 在渲染单行时先经 `highlightLine`
  4. 对 >1000 行 diff 加虚拟列表（建议 `@tanstack/react-virtual`，当前未在依赖里）
- **影响面**：纯视觉。当前 diff 在多语言大文件场景可读性较差（语法着色缺失）；超大 diff（数千行）一次性渲染可能引起卡顿，但项目内 256KB 截断阈值已经把单边压住，实际触发概率低
- **触发时机建议**：下次有用户报告 diff 阅读体验差 / 大 diff 卡顿时；或独立"diff 面板增强 + 虚拟列表"PR

---

### F-023 `file_read` metadata 已 emit 但前端 grouping UI 未实现

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`crates/ha-core/src/tools/read.rs`](../../crates/ha-core/src/tools/read.rs) 已 emit `kind: "file_read", path, lines"` 元数据，前端 [`src/types/chat.ts`](../../src/types/chat.ts) 也定义了 `FileReadMetadata` 类型；但 [`src/components/chat/message/MessageContent.tsx`](../../src/components/chat/message/MessageContent.tsx) 中没有 grouping 逻辑——连续相邻 read 仍每条一行展示，没有折叠成截图样式的"已浏览 N 个文件"。Plan 文件第 12 条原本要求新建 `ToolCallList.tsx` 中间组件做 grouping，落地时被砍
- **为什么留**：grouping 涉及 contentBlocks 循环里识别相邻同类 ToolCall + 维持 callId 顺序 + 展开列表交互。本期主要功能（diff 面板）已实现完整；read 聚合是体验优化，单条 read 显示也能接受
- **改的话要做什么**：
  1. 新建 `src/components/chat/message/FileReadAggregate.tsx`：接收 `paths: string[]`，渲染折叠/展开 UI（展开列出每个 path）
  2. 在 [`MessageContent.tsx`](../../src/components/chat/message/MessageContent.tsx) 渲染 contentBlocks 时，把相邻 `metadata.kind === "file_read"` 的 ToolCall 折成 `<FileReadAggregate>`
  3. 注意保留 callId 顺序，避免 streaming 中途插入新 read 时渲染抖动
  4. i18n key `tool.fileRead.aggregateLabel`（本期 plan 第 19 条已计划但未落实，12 语言需补）
- **影响面**：纯视觉。当前连续 read 多个文件占多行显示，chat 偏冗长；用户已习惯当前样式，无 bug
- **触发时机建议**：下次动 MessageContent / ToolCallBlock 渲染层时一并实现；或独立"chat 工具调用紧凑视图"PR

---

### F-024 DiffPanel 与 CanvasPanel 互斥未实现

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`src/components/chat/ChatScreen.tsx`](../../src/components/chat/ChatScreen.tsx) 在 useEffect 中实现了"DiffPanel 打开自动关 PlanPanel"互斥，但**没有**与 CanvasPanel 的互斥。三面板同时打开会导致主 chat 区被挤压到不可用宽度。Plan 文件第 18 条原本要求三面板互斥
- **为什么留**：CanvasPanel 自管 visibility（不像 PlanPanel 暴露 `setShowPanel` API），改动需要先重构 CanvasPanel 的 state ownership。本期落地时为避免冲击 CanvasPanel 现有契约，权衡后只做了 DiffPanel ↔ PlanPanel
- **改的话要做什么**：
  1. CanvasPanel 把 `showPanel` state 提到 ChatScreen 上层管理（或暴露 imperative `onClose` ref）
  2. ChatScreen 的 useEffect 加入第三向互斥：openDiff → close PlanPanel + close CanvasPanel；openCanvas → close DiffPanel + close PlanPanel；openPlan → close DiffPanel + close CanvasPanel
  3. 或更优：抽 `useExclusivePanel(panelId)` hook 统一管理三面板 mutex（PlanPanel / CanvasPanel / DiffPanel 注册到同 registry）
- **影响面**：可见 bug。极端场景（用户 Plan + Canvas + Diff 全打开）chat 主区被挤压；日常使用罕见
- **触发时机建议**：下次动 CanvasPanel state ownership / 新增第四个 side panel 时一并收掉；或独立"side panel mutex 统一"重构 PR

---

### F-025 `commands/permission.rs` 与 `routes/permission.rs` 镜像重复

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse + quality 双 agent）
- **现象**：[`src-tauri/src/commands/permission.rs`](../../src-tauri/src/commands/permission.rs) 和 [`crates/ha-server/src/routes/permission.rs`](../../crates/ha-server/src/routes/permission.rs) 是 byte-for-byte 镜像，仅 wrapper 类型不同（`Result<T, CmdError>` vs `Result<Json<T>, AppError>`）。两份独立维护：
  - `PatternListPayload` / `SetPatternsBody` / `GlobalYoloStatus` 三个 struct 定义重复
  - 12 个 endpoint 一一对应，业务逻辑同（都调 `protected_paths::current_patterns()` 等）
  - `mutate_config(("permission.smart", "settings-ui"|"http"), …)` 仅 source 标签不同
- **为什么留**：Phase 3 范围是"前端 UI + 后端 file IO + commands/routes"打通，时间紧；这是项目级"Tauri ↔ HTTP 双暴露"的通用模式，单独动一个域意义有限——MCP / config / agents 等其它子系统也都有同样的镜像样板，应当作为"统一模式"独立 PR 推
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/permission/` 新建 `api.rs` 模块，把所有 payload 结构体（`PatternListPayload` / `SetPatternsBody` / `GlobalYoloStatus`）+ thin worker functions（`get_protected_paths_inner() -> PatternListPayload` 等）集中
  2. `commands/permission.rs` 退化成 `#[tauri::command]` 包装：`fn get_protected_paths() -> Result<PatternListPayload, CmdError> { permission::api::get_protected_paths().map_err(Into::into) }`
  3. `routes/permission.rs` 同样退化成 `Json(...)` 包装
  4. `mutate_config` 的 source 标签作为参数传入 worker function：`api::set_smart_mode_config(cfg, "settings-ui")` vs `api::set_smart_mode_config(cfg, "http")`
  5. 顺手考虑把这个模式抽成跨域的 `crate::transport_shim::tauri!()` / `axum_route!()` 宏（或代码生成），扩到 mcp / agents / config 等 4-5 个有同样镜像的子系统
- **影响面**：纯重构，无功能变化。当前 ~200 行重复代码 / 12 个新增 endpoint 的双倍维护成本；未来加新 endpoint 必须记得改两处
- **触发时机建议**：等下一个新增"Tauri ↔ HTTP 双暴露"endpoint 的 PR 时累积痛感再做；或立项"transport-shim 通用化"独立重构 PR，一次清掉 mcp / agents / config / permission 4 个域的镜像

---

### F-026 ApprovalTab `APPROVAL_OPTIN_GROUPS` 17 工具 9 分组硬编码于 TS

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse + quality）
- **现象**：[`src/components/settings/agent-panel/tabs/ApprovalTab.tsx:21-67`](../../src/components/settings/agent-panel/tabs/ApprovalTab.tsx#L21-L67) 把 17 个可勾选审批的工具按 9 个分组（shell / browser / settings / outbound / paid / spawn / network / crossSession / settingsRead）硬编码在 TS 常量里。后端工具注册表（[`tools/definitions/`](../../crates/ha-core/src/tools/definitions/)）虽然已有 `ToolTier` 元数据，但**没有"是否出现在用户审批勾选清单 + 归属哪个分组"的字段**——所以 UI 这份清单只能写在 TS 端。
  漂移风险：今天 Rust 加新工具 `send_email` 进 Tier 2，UI 不会自动显示在审批清单里，必须有人记得去改 ApprovalTab。TS 编译器无法捕获这个不一致
- **为什么留**：Phase 3 simplify 不重构 schema。要做需要：
  1. `ToolDefinition` 加 `approval_opt_in: bool` + `approval_group: Option<&'static str>` 两字段
  2. 53 个工具定义文件每个填这俩字段（含归类决策）
  3. `list_builtin_tools` payload 透传字段
  4. ApprovalTab 改数据驱动（动态分组渲染 + 与现有 i18n key 对齐）
  跨 53 个文件的注释决策不属于"清理 review"范围
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/tools/definitions/types.rs::ToolDefinition` 加 metadata：
     ```rust
     /// 是否出现在 Agent「自定义工具审批」勾选清单里。Tier 2/3 中的部分工具开启。
     pub approval_opt_in: bool,
     /// 审批清单 UI 的分组标签 (i18n key 后缀)。
     pub approval_group: Option<&'static str>,
     ```
  2. 在每个工具的 `register_*_tool()` 函数里设置这两个字段（按当前 ApprovalTab.tsx 的 17/9 分组对照填）
  3. `list_builtin_tools` 添加这两个字段到 payload；`commands/chat.rs::list_builtin_tools` + 对应 HTTP 路由同步
  4. ApprovalTab 改成 `useMemo(() => groupBy(builtinTools.filter(t => t.approval_opt_in), t => t.approval_group))`，删除 `APPROVAL_OPTIN_GROUPS` 常量
  5. 更新 [`docs/architecture/tool-system.md`](../../docs/architecture/tool-system.md) 描述新元数据字段
- **影响面**：纯一致性，无可见 bug。当前 17 个工具固定，添加新 Tier 2/3 工具时如果忘改 TS，用户在 Agent 设置里看不到该工具但功能层面仍然工作（不会崩溃，只是无法 opt-in 审批）
- **触发时机建议**：下次新增可审批工具（Tier 2/3）时如果意识到要同时改两处，就顺手把这套 metadata schema 搭起来；或独立"tool definition metadata 扩展"重构 PR

---

### F-027 9 个语言 `settings.approvalPanel` block 是英文 verbatim fallback

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse agent）
- **现象**：Phase 3.4 新增的 `settings.approvalPanel.*` 文案块（~50 keys）只有 `zh.json` / `en.json` / `zh-TW.json`（部分）有原生翻译；剩下 9 个语言（`ar` / `es` / `ja` / `ko` / `ms` / `pt` / `ru` / `tr` / `vi`）通过 `node -e` 脚本批量 deep-clone 英文 block 写入 —— 这些 locale 的"权限"设置面板会渲染英文标签 / 描述 / 提示。同样的情况也部分发生在 `settings.agentApproval.*`（Phase 3.3）和 `approval.reasons.*`（Phase 3.5），但 zh / en / zh-TW / ja / ko 都已精修
- **为什么留**：英文 fallback 不会让 UI 崩溃 / 不会丢功能；Anthropic 内部不是翻译团队 —— 用机器翻译质量参差不如等母语审稿。提交时 `pnpm i18n:check ✓` 因为 key 数量已对齐，仅文案语言不对
- **改的话要做什么**：
  1. 收集需要翻译的 key 集合：从 `en.json` 提 `settings.approvalPanel.*`、`settings.agentApproval.*`（zh-TW 已部分精修，但 ja / ko 也只有部分）、`approval.reasons.*` 在非 zh / en / zh-TW 的 locale 里全部
  2. 翻译团队 / 母语志愿者按 locale 校对（约 ~70 keys × 9 语言 = 630 条）
  3. 提交时把 zh-TW / ja / ko 的部分英文 fallback 一起替换掉
  4. 顺带清查仓库里其它"批量 deep-clone 英文"的 i18n debt：grep `settings.*` 中相同字符串在多个非 en locale 里完全一致的 key
- **影响面**：UX bug for 9 个语言用户。Settings 中相关 panel 看英文不会崩溃，但显著降低非英语 / 非中文用户的体验
- **触发时机建议**：等收到非英 / 非中文用户反馈，或翻译团队 / 志愿者主动认领；不阻塞功能 PR

---

## Closed

> 已修复条目移到此处，附 commit hash + 关闭日期。保留以便后续 grep。

### F-004 NDJSON 流式解析无统一 helper

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / rejected on second look，不实现
- **修复方式**：实现前先核对 5 个候选站点，发现登记前提错误：实际 NDJSON 只有 [`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 一处，其它 4 处均不属于：
  - [`docker/deploy.rs`](../../crates/ha-core/src/docker/deploy.rs) — `docker pull` 纯文本 stdout 转 log
  - [`mcp/client.rs`](../../crates/ha-core/src/mcp/client.rs) — MCP server stderr 纯文本 tail（rate-limit + truncate）
  - [`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs) — 子进程 stdout/stderr 纯文本转 `mpsc::Receiver<String>`
  - [`agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs) — **SSE**（`event:` / `data:` / `\n\n` boundary），不是 NDJSON

  抽 helper 只有一个消费者 (`pull_model`)，且本期已经自带 `MAX_PULL_LINE_BYTES` + 严格末帧 + 单测覆盖，新增一层间接零收益（典型的 premature abstraction）。SSE 那侧的真重复另开 [F-019](#f-019-sse-解析器在-4-处-llm--im-stream-重复实现) 登记。

---

### F-017 旧 `local_llm:install_progress` / `local_llm:pull_progress` / `local_embedding:pull_progress` 事件路径已无前端监听

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **关闭**：2026-04-26
- **修复方式**：grep 全仓库确认前端 100% 已切到 `local_model_job:*` 事件总线、外部消费面零调用后，删除旧路径所有源码与文档。具体：
  - **ha-core**：删除 `EVENT_LOCAL_LLM_INSTALL_PROGRESS` / `EVENT_LOCAL_LLM_PULL_PROGRESS` / `EVENT_LOCAL_EMBEDDING_PULL_PROGRESS` 三个常量；删除非 cancellable 包装函数 `local_llm::install_ollama_via_script` / `local_llm::pull_and_activate` / `local_embedding::pull_and_activate`；windows stub 合并到 `install_ollama_via_script_cancellable`；`*_cancellable` 版本仅保留给 `local_model_jobs` 调用
  - **ha-server**：[`routes/local_llm.rs`](../../crates/ha-server/src/routes/local_llm.rs) / [`routes/local_embedding.rs`](../../crates/ha-server/src/routes/local_embedding.rs) 删 `install` / `pull` handler 与对应 imports，砍到只剩硬件 / Ollama 状态 / 模型目录探测；[`router 注册`](../../crates/ha-server/src/lib.rs) 去掉 `/local-llm/install` / `/local-llm/pull` / `/local-embedding/pull` 三条路由
  - **src-tauri**：[`commands/local_llm.rs`](../../src-tauri/src/commands/local_llm.rs) / [`commands/local_embedding.rs`](../../src-tauri/src/commands/local_embedding.rs) 删 `local_llm_install_ollama` / `local_llm_pull_and_activate` / `local_embedding_pull_and_activate` 三条命令；[`invoke_handler!`](../../src-tauri/src/lib.rs) 注册表去三行
  - **前端**：[`src/lib/transport-http.ts`](../../src/lib/transport-http.ts) COMMAND_MAP 删除三条路由映射
  - **文档**：[`docs/architecture/api-reference.md`](../../docs/architecture/api-reference.md) 事件表用 `local_model_job:*` 替换，新增「Local model background jobs」表与 8 条 routes / 同时把 Local LLM assistant 表收敛到 5 条探测命令；[`docs/architecture/transport-modes.md`](../../docs/architecture/transport-modes.md) 事件矩阵同步替换；[`AGENTS.md`](../../AGENTS.md) 「本地 LLM 助手」段把"进度走 EventBus"改成"后台任务统一接口"；docker.rs / docker command shim 内残留的旧函数引用注释一并清理
  - 验证：`cargo check -p ha-core -p ha-server` / `cargo check -p hope-agent` / `pnpm typecheck` 全绿
- **影响面**：dead-code 移除，无 runtime 行为变更。

---

### F-003 "local Ollama" 判定逻辑分散在 4 处

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-002 + F-003 修复
- **修复方式**：新增 [`crates/ha-core/src/provider/local.rs`](../../crates/ha-core/src/provider/local.rs) 维护 known local backends catalog（Ollama / LiteLLM / vLLM / LM Studio / SGLang）与 host+port 匹配逻辑，`local_llm::OLLAMA_BASE_URL` 改为复用 `LOCAL_OLLAMA_BASE_URL`。新增 Tauri `local_llm_known_backends` 与 HTTP `GET /api/local-llm/known-backends`，前端 [`provider-detection.ts`](../../src/components/settings/local-llm/provider-detection.ts) 改为消费后端 catalog，不再维护 `LOCAL_OLLAMA_HOST_RE`。ProviderSettings / TemplateGrid 均使用同一 catalog 判定是否展示本地小模型助手。

---

### F-002 Provider 写入路径未单一化（add_provider 缺 upsert 语义）

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-002 + F-003 修复
- **修复方式**：新增 [`crates/ha-core/src/provider/crud.rs`](../../crates/ha-core/src/provider/crud.rs) 作为 Provider 写入单一入口，集中 add / update / delete / reorder / set active / add-and-activate / batch add / Codex ensure / local backend upsert。GUI、HTTP、onboarding、Codex auth/restore/logout、OpenClaw import、CLI onboarding、IM slash active-model 切换和 local LLM 注册路径均改走 ha-core helper。普通 `add_provider` 继续追加并生成新 id；本地模型助手单独通过 known backend upsert 去重。

---

### F-015 `src/components/settings/` 大批原生 `<button>` / `<input>` / `<textarea>` 未走 shadcn

- **来源**：2026-04-26 焦点轮廓视觉降噪手动审查
- **关闭**：2026-04-26 / branch `worktree-settings-shadcn-migration`
- **修复方式**：把 `src/components/settings/` 下 50+ 个文件里所有原生 `<button>`（116 处）/ `<input>`（5 处非 file/checkbox 类型）/ `<textarea>`（2 处）/ `<input type="range">`（2 处）/ `<input type="checkbox">`（4 处）系统替换成 shadcn 等价组件：`<Button>` 各 variant（ghost / outline / secondary / icon）、`<Input>`、`<Textarea>`、`<Slider>`、`<Switch>`。图标按钮统一走 `size="icon"`；原本"看起来像按钮但其实是文字链"的内联点击点（如 SearxngDocker 端口、profile 自定义重置）改 `variant="ghost"` + 行内 className override 保留 baseline + underline。涉及 40+ 文件，主要包括 ProviderEditPage / ProviderSettings / ContextCompactPanel / GlobalModelPanel / AgentEditView / PersonalityTab / CapabilitiesTab / ModelTab / AgentListView / AvatarCropDialog / DangerousModeSection / ProfileForm / MemoryListView / MemoryFormView / EmbeddingModelSection / SkillListView / SkillDetailView / ModelEditor / AddAccountDialog / AllowlistTagInput 等。新代码若再写原生 `<button>` / `<input>` / `<textarea>` 由 code review 打回。`src/index.css` 全局 focus-visible fallback 仍然保留作为防御层。

---

### F-009 EventBus 桥接闭包样板在多处重复

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-009 修复
- **修复方式**：在 [`crates/ha-core/src/event_bus.rs`](../../crates/ha-core/src/event_bus.rs) 新增 `EventBusProgressExt::emit_progress`，把 typed progress callback 统一桥接到 EventBus JSON payload。为保留 `EventBus` 的 object-safe 形状（仓库大量使用 `Arc<dyn EventBus>`），实现采用 `Arc<B: EventBus + ?Sized>` 扩展 trait，而不是直接在 `EventBus` 本体加泛型方法。local LLM install / pull、SearXNG deploy、local embedding pull 的 ha-server route 与 Tauri command 均已切换到 helper，事件名与 payload contract 不变。

---

### F-012 `useChatStream.ts::onEvent` 嵌套 try/catch + 多重 if 应 flatten

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-012 修复
- **修复方式**：[`useChatStream.ts`](../../src/components/chat/hooks/useChatStream.ts) 的 `onEvent` 现在拆为 `handleSessionCreated`、`shouldDropStreamEvent`、`dispatchStreamEvent`、`appendRawStreamText` 等本地 helper；保留 `__pending__` cache rename、loading session 更新、`_oc_seq` cursor 去重、ended stream 丢弃与 raw fallback 行为。

---

### F-005 前端字节/容量格式化在 6+ 处重复

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-005 修复
- **修复方式**：新增 [`src/lib/format.ts`](../../src/lib/format.ts) 统一 `formatBytes`、`formatBytesFromMb`、`formatGbFromMb`；替换 dashboard、BrowserPanel、FileCard、log panel、SkillDetailView、本地 LLM / embedding 卡片、project 上传与 logo 限制错误文案里的重复容量格式化，并新增 [`src/lib/format.test.ts`](../../src/lib/format.test.ts) 覆盖单位转换。

---

### F-014 `docs/architecture/` 缺中心化 transport mode 文档

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-014 修复
- **修复方式**：新增 [`docs/architecture/transport-modes.md`](../architecture/transport-modes.md)，集中说明 Tauri / HTTP / ACP 三种入口、`getTransport()` 选择逻辑、`Transport` 方法矩阵、`chat:stream_delta` 双写与 reattach 角色、`/ws/events` EventBus 桥、主要 EventBus 事件目录，以及 `startChat` 不是通用 `streamCall` 的决策记录。同步回填 [`docs/README.md`](../README.md) 索引。

---

### F-010 HTTP `startChat` 用合成 `session_created` 事件 vs 显式 return shape 的取舍

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-010 修复
- **修复方式**：保留 [`src/lib/transport-http.ts::startChat`](../../src/lib/transport-http.ts) 合成 `session_created` 的现有合约，让 [`useChatStream.ts`](../../src/components/chat/hooks/useChatStream.ts) 继续用同一条 `onEvent` 路径完成 `__pending__` cache rename，避免把 HTTP 特例泄漏到 hook。经核实前端已不再消费 `/ws/chat/{session_id}`，HTTP 流式输出完整走 `/ws/events` 上的 `chat:stream_delta`；因此删除 [`crates/ha-server/src/ws/chat_stream.rs`](../../crates/ha-server/src/ws/chat_stream.rs)、`ChatStreamRegistry`、`WsSink` 和 `/ws/chat/{session_id}` 路由，ha-server 改用 `NoopSink` 依赖 Chat Engine 的 EventBus 双写路径。同步更新架构文档中旧的 `openChatStream` / `/ws/chat` 描述。

---

### F-006 Ollama pull 流提前结束时仍会激活模型

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 现在会在流结束时解析残留 buffer 中无换行的最后一帧；若最终状态不是 `success`，或最后残留帧是截断/非法 JSON，则返回 `Err`，阻止后续 provider 注册与 active model 切换。新增单元测试覆盖 final success 有换行、final success 无换行、early EOF、truncated final frame。

---

### F-007 Ollama 安装成功后进度弹窗不会关闭

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`InstallProgressDialog`](../../src/components/settings/local-llm/InstallProgressDialog.tsx) 增加受控 `onOpenChange`，运行中拦截关闭，完成/错误态允许关闭；[`LocalLlmAssistantCard.tsx::installOllama`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 在一键安装并启动成功后展示完成态约 800ms，然后自动关闭弹窗并刷新 Ollama 状态。

---

### F-008 HTTP 模式下手动下载 Ollama 按钮无效

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`LocalLlmAssistantCard.tsx::openDownloadPage`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 现在会检查 `open_url` 返回值；当 HTTP/server 模式返回 `{ ok: false }` 时主动 fallback 到 `window.open("https://ollama.com/download")`，Tauri 原生打开失败时也继续走同一 fallback。

---

### F-011 短期 EventBus 订阅 + `try/finally off()` 模式应抽 `withEventListener` helper

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：新增 [`src/lib/transport-events.ts::withEventListener`](../../src/lib/transport-events.ts)，封装"订阅事件 → 执行长任务 → finally 取消订阅"模式；本地小模型 install / pull 与 SearXNG deploy 三个调用点已切换到该 helper。

---

### F-001 Tauri 命令错误类型未统一

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / branch `worktree-tauri-cmd-error-unify`
- **修复方式**：新增 [`src-tauri/src/commands/error.rs`](../../src-tauri/src/commands/error.rs) 定义 `CmdError(pub String)`，挂 `impl<E: Into<anyhow::Error>> From<E>` + `impl Serialize`（输出纯字符串，IPC wire 与原 `Result<T, String>` 等价）；把 `src-tauri/src/commands/` 下 31 个文件的命令签名统一改成 `Result<T, CmdError>`，291 处 `.map_err(|e| e.to_string())?` 删成 `?`，剩余 `.map_err(|e| format!(...))` 改为 `CmdError::msg(format!(...))`，`Err("..".to_string())` / `.ok_or_else(|| "..".to_string())` 等串字面量误差类全部走 `CmdError::msg(..)`。`tauri_wrappers.rs` 不属于"命令尾巴 boilerplate"范畴，保持 `Result<T, String>` 不动。前端零变化。

---

### F-030 `ResolveContext { ... }` 14 字段构造在 execution.rs / exec.rs 重复

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `59a36ab5`
- **修复方式**：新增 [`tools::execution::resolve_tool_permission`](../../crates/ha-core/src/tools/execution.rs) `pub(super)` async helper，统一构造 `permission::engine::ResolveContext` + 跑 `resolve_async` + 保留 "Smart 才 cached_config" hot-path 优化。两处 caller（`tools/execution.rs` 主 dispatch、`tools/exec.rs` exec 命令前置审批）从 11 行字段构造塌缩到 1 行 helper 调用。新增字段时只需改 helper 一处。

---

### F-031 `permission::judge::cache_key` JSON 序列化不规范化对象键序

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `2eefc428`
- **修复方式**：[`permission::judge`](../../crates/ha-core/src/permission/judge.rs) 把 `args.to_string().hash(...)` 替换成新的 `hash_value_canonical(args, hasher)` 递归哈希器：对象内按键排序后逐对哈希，数组按位置哈希，每个 `Value` 变体加 1 字节 tag 防跨变体冲突（null vs ""）。同语义但键序不同的 args 现在产生相同 cache key，避免冗余的 ~5s 判官 LLM 调用。新增 3 条单测：键序不变性 / 嵌套对象递归 / null/string/array/object 互不冲突。

---

### F-028 `permission::judge` cache 与 `agent::active_memory` cache 模式重复

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（reuse agent）
- **关闭**：2026-04-30 / commit `67c7e1f2`
- **修复方式**：新增 [`crate::ttl_cache::TtlCache<K, V>`](../../crates/ha-core/src/ttl_cache.rs)：TTL 在 `get` 时传入（让 `cache_ttl_secs` 配置即时生效）、溢出时 LRU-by-age 单条 evict（O(n) 但 n ≤ cap）、`get` 命中过期项 lazy 移除、无后台 sweep。`permission::judge` 退化为 `OnceLock<TtlCache<u64, JudgeResponse>>` 删除自带 60 行 cache helper；`agent::active_memory` 把 `Mutex<HashMap<...>>` 字段换成 `TtlCache` 删除手写 evict-oldest 12 行。新增 5 条 ttl_cache 单测；代码净减 ~125 行重复，新增 helper 170 行（含完整 doc + 单测）。

---

### F-029 `SessionMeta.permission_mode` 仍是 `String`，应换 `SessionMode` enum

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `0dcddf5a`
- **修复方式**：[`session::types::SessionMeta`](../../crates/ha-core/src/session/types.rs) 的 `permission_mode: String` 改成 `permission_mode: SessionMode`（已带 Default impl + snake_case serde rename）。前端 `SessionMode` union / DB TEXT 列 / JSON 编码完全不变，仅 Rust 内部强类型化。`SessionMode::parse_or_default` 仅在 DB row→struct 边界用一次（[`session/db.rs::row_to_session_meta`](../../crates/ha-core/src/session/db.rs)），消费方（`agent/config.rs` system_prompt 构造、`agent/mod.rs` ToolExecContext 构造）改成 `.map(|m| m.permission_mode)` 直接拷贝 enum (Copy)；`update_session_permission_mode` 参数改成 `SessionMode`，4 处 caller 删掉 `.as_str()` 包装。awareness 测试 fixture `"default".into()` 同步改成 `SessionMode::Default`。ha-core 771 / ha-server 18 单测全绿。
