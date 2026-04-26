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

### F-004 NDJSON 流式解析无统一 helper

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：[`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 用 `bytes_stream` + `buf.iter().position(b'\n')` 手写 NDJSON 切行；同模式在 [`docker/deploy.rs`](../../crates/ha-core/src/docker/deploy.rs)（用 `BufReader::lines()`）、[`mcp/client.rs`](../../crates/ha-core/src/mcp/client.rs)、[`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs)、[`agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs)（SSE 流）等处都有 inline 实现，**仓库目前没有统一的"流式 NDJSON helper"约定**。
- **为什么留**：现有几处实现差异较大（reqwest `bytes_stream` vs `tokio::io::BufReader` vs `mpsc::Receiver<String>`），抽通用 helper 需要先统一 input 抽象，跨模块改动大。本期 `pull_model` 的实现已加 1 MiB 单行上限防御 + 单元测试覆盖，质量上可独立。
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs)（或新建 `util/ndjson.rs`）加 `pub fn ndjson_stream<S>(stream: S, max_line_bytes: usize) -> impl Stream<Item = Result<Value>>`
  2. 把 reqwest `bytes_stream::Stream<Item = Result<Bytes, _>>` 转 `AsyncBufRead`（用 `tokio_util::io::StreamReader`），再用 `lines()` 拆行 + `serde_json::from_str`
  3. 替换 5 处 inline 实现
- **影响面**：当前无 bug，每处实现都正确但重复。
- **触发时机建议**：下一次新增"还需要解析流式 NDJSON / SSE"的接入点时，顺手抽 helper；不必单独立 PR。

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

---

## Closed

> 已修复条目移到此处，附 commit hash + 关闭日期。保留以便后续 grep。

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
