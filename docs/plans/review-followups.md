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

### F-002 Provider 写入路径未单一化（add_provider 缺 upsert 语义）

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：仓库里有两条创建 Provider 的路径：
  - [`src-tauri/src/commands/provider/crud.rs::add_provider`](../../src-tauri/src/commands/provider/crud.rs)：GUI"添加 Provider"走这条，**无 upsert**，每次都 `store.providers.push()`
  - [`crates/ha-core/src/local_llm/mod.rs::ensure_ollama_provider_with_model`](../../crates/ha-core/src/local_llm/mod.rs)：本地小模型助手走这条，**有 upsert**（按 base_url 去重）
- **为什么留**：
  1. ha-core 不能反向依赖 src-tauri（AGENTS.md "零 Tauri 依赖" 约束），不能直接调用 `add_provider`
  2. 老的 GUI `add_provider` 不 upsert 不算 bug（用户视觉判重）；只有自动化路径才需要 upsert
  3. 统一要下放 Provider CRUD 到 ha-core，跨多个 crate + 改写入语义，独立工作
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/provider/`](../../crates/ha-core/src/provider/) 新建 `crud.rs`，把 Provider 写入逻辑（add / update / delete / reorder / upsert_by_base_url）下放，统一走 `mutate_config(("providers.<op>", source), ...)`
  2. [`src-tauri/src/commands/provider/crud.rs`](../../src-tauri/src/commands/provider/crud.rs) 与 [`crates/ha-server/src/routes/providers.rs`](../../crates/ha-server/src/routes/providers.rs) 改成薄壳直接调用 ha-core 函数
  3. `local_llm::ensure_ollama_provider_with_model` 改用统一的 `upsert_by_base_url` helper
- **影响面**：当前无 bug；未来若 `add_provider` 增加副作用（如自动写 SSRF trusted_hosts、emit 特定事件），新代码不会自动享受到，是漂移风险。
- **触发时机建议**：下一次有人需要"再加一条 Provider 创建路径"时（例如 import / batch onboarding），顺势统一；或独立 "Provider CRUD 单一入口" 重构 PR。

---

### F-003 "local Ollama" 判定逻辑分散在 4 处

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：判断"这是不是本地 Ollama"的代码散落在四个文件，正则/字符串/常量各一份：
  - [`crates/ha-core/src/local_llm/mod.rs::is_local_ollama_url`](../../crates/ha-core/src/local_llm/mod.rs) — 后端 upsert 去重
  - [`src/components/settings/ProviderSettings.tsx::LOCAL_OLLAMA_HOST_RE`](../../src/components/settings/ProviderSettings.tsx) — 前端"是否挂载本地小模型卡片"判定
  - [`crates/ha-core/src/openclaw_import/providers.rs`](../../crates/ha-core/src/openclaw_import/providers.rs) — 写死 `http://127.0.0.1:11434`
  - [`src/components/settings/provider-setup/templates/local.ts`](../../src/components/settings/provider-setup/templates/local.ts) — Provider 模板字段
- **为什么留**：跨前后端、跨 crate 的"小常量统一"价值有限；当前四处行为一致没有 bug。强行抽出 shared 常量需要做 wire-format 同步（前端 TS const 怎么从 Rust 同步），引入新约束。
- **改的话要做什么**：候选方案：
  - **A**：在 [`crates/ha-core/src/provider/local.rs`](../../crates/ha-core/src/provider/local.rs)（新建）放 `pub const LOCAL_OLLAMA_BASE_URL` + `pub fn is_local_ollama_url`，后端三处都用；前端继续维护一个独立常量但加注释指向后端同名定义
  - **B**：把"已知本地后端"做成数据驱动表（Ollama / LM Studio / vLLM / SGLang / LiteLLM），后端暴露 `GET /api/local-llm/known-backends`，前端跟着拉
- **影响面**：纯整洁度，没有 bug。但如果有一天 Ollama 默认端口变了或要支持 `http://[::1]:11434` 之类，要改 4 个地方。
- **触发时机建议**：下一次需要在多处加新的 local backend（例如增加对 LM Studio 的一键安装支持）时统一；或者发现端口/host 变化要改时被动收掉。

---

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

### F-005 前端字节/容量格式化在 6+ 处重复

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：前端"把 MB / bytes 格式化成 'X.X GB'"的小函数散落在至少 6 个文件，本期新增 [`LocalLlmAssistantCard.tsx::formatGb` + `formatSize`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 是第 7 处。其它已知点：
  - [`src/components/settings/dashboard/types.ts`](../../src/components/dashboard/types.ts)
  - [`src/components/settings/BrowserPanel.tsx`](../../src/components/settings/BrowserPanel.tsx)
  - [`src/components/chat/message/FileCard.tsx`](../../src/components/chat/message/FileCard.tsx)
  - [`src/components/log-panel/constants.ts`](../../src/components/log-panel/constants.ts)
  - `SystemMetricsSection.tsx`
- **为什么留**：每处函数都很短（3-5 行），抽到 `src/lib/format.ts` 价值有限；存量债，本期不展开。
- **改的话要做什么**：
  1. 在 [`src/lib/format.ts`](../../src/lib/format.ts)（新建）加 `formatBytes(bytes: number, opts?)` + `formatBytesFromMb(mb: number)`
  2. 全仓 grep `\.toFixed.*GB|toFixed.*MB` 替换调用点
- **影响面**：纯整洁度。语义差异极小，不会引入 bug。
- **触发时机建议**：下一次新加 file-size / capacity 显示组件时顺手抽；或者独立"前端 utility 整理"小 PR 一次清掉。

---

### F-009 EventBus 桥接闭包样板在 4 处重复

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：把 ha-core 长任务的 progress callback 桥接到 EventBus 的样板（"取 bus → 闭包 emit"）目前在 4 处复制：
  - [`crates/ha-server/src/routes/local_llm.rs::install_ollama`](../../crates/ha-server/src/routes/local_llm.rs) + `pull`
  - [`crates/ha-server/src/routes/searxng.rs::deploy`](../../crates/ha-server/src/routes/searxng.rs)
  - [`src-tauri/src/commands/local_llm.rs::local_llm_install_ollama`](../../src-tauri/src/commands/local_llm.rs) + `local_llm_pull_and_activate`
  - [`src-tauri/src/commands/docker.rs::searxng_docker_deploy`](../../src-tauri/src/commands/docker.rs)
- **为什么留**：跨 ha-server / src-tauri 两个 crate 抽 helper 涉及 trait 设计选择（free function vs `EventBus` trait method 默认实现）；本期 PR 的 scope 只是"消除前端裸 Channel"，再展开会扩大 diff。
- **改的话要做什么**：在 [`crates/ha-core/src/event_bus.rs`](../../crates/ha-core/src/event_bus.rs) 给 `EventBus` trait 加默认方法 `fn emit_progress<T: Serialize>(&self, name: &str) -> impl Fn(&T) + Send + Sync` 返回桥接闭包；4 个调用点都改成 `bus.emit_progress(EVENT_*_PROGRESS)`，省掉 `move |p| bus.emit(NAME, json!(p))` 一行。
- **影响面**：纯整洁度，0 行为变化。
- **触发时机建议**：下次新增第 5 个 long-running command（例如 model fine-tune progress）时顺势抽；或独立 "EventBus helper" 小 PR。

---

### F-010 HTTP `startChat` 用合成 `session_created` 事件 vs 显式 return shape 的取舍

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：[`src/lib/transport-http.ts::startChat`](../../src/lib/transport-http.ts) 在 HTTP 模式下，POST `/api/chat` 返回后**手动合成**一个 `{type:"session_created", session_id:...}` 事件喂给 `onEvent` 回调，目的是让 [`useChatStream.ts`](../../src/components/chat/hooks/useChatStream.ts) 内部 `__pending__` cache key 替换逻辑统一走 onEvent 分支。语义上接口"看起来 generic"但 HTTP 实现行为隐式特化，签名留下"似 stream 实非 stream"的疑义。**还要顺便核实** [`crates/ha-server/src/ws/chat_stream.rs`](../../crates/ha-server/src/ws/chat_stream.rs) 的 `/ws/chat/{session_id}` 路由：前端 `openChatStream` 已删，server 端 `WsSink` 仍 broadcast 到这个路由，可能成为死路径——若 reattach `/ws/events` 能完整覆盖，可一并清理。
- **为什么留**：换成"显式 return `{sessionId, response}` 让调用方自己改 cache"会把 transport 抽象的好处折损（hook 要自己 if (isHttp) 分支）；当前文档已显式说明 HTTP 模式仅合成 `session_created`，合约是诚实的。死路径核实涉及 axum router 注册顺序梳理，独立小工作。
- **改的话要做什么**：(a) 评估是否换成"`startChat` 直接 return `ChatResponse`，cache rename 由 hook 自己做"。(b) 验证 `/ws/chat/{id}` 路由的所有消费者是否都已切到 `/ws/events`，无消费者则删 [`ws/chat_stream.rs`](../../crates/ha-server/src/ws/chat_stream.rs) + lib.rs 路由注册。
- **影响面**：当前无 bug，无性能差异；属于架构清晰度问题。
- **触发时机建议**：HTTP `startChat` 出现第二种"必须前置交付"的事件（例如 chat 命令同步阶段 error）时回头重设计；或独立 "chat stream 路径清理" PR。

---

### F-012 `useChatStream.ts::onEvent` 嵌套 try/catch + 多重 if 应 flatten

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：[`useChatStream.ts:307-359`](../../src/components/chat/hooks/useChatStream.ts) 的 `onEvent` 闭包内部 try/catch 包嵌套 `event.type === "session_created" && event.session_id` 早返回 + `streamId && endedStreamIdsRef.current.get(sid) === streamId` 早返回 + `_oc_seq` dedup + `handleStreamEvent(...)` dispatch + catch 兜底文本拼接，单函数 ~50 行。
- **为什么留**：本期 PR 主题是"切换调用方 API"，没动 onEvent 内部逻辑；按 AGENTS.md "review 决定不改的清理登记到 followups"。
- **改的话要做什么**：拆成几个 named handler：`handleSessionCreated`、`handleStreamDelta`、`fallbackTextAppend`，主 `onEvent` 退化为 `try { dispatch(JSON.parse(raw)) } catch { fallbackTextAppend(raw) }`。
- **影响面**：纯可读性。
- **触发时机建议**：下次有人为了别的事真要动这段逻辑时顺手收掉。

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

### F-014 `docs/architecture/` 缺中心化 transport mode 文档

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：[`useChatStreamReattach.ts:55-66`](../../src/components/chat/hooks/useChatStreamReattach.ts) 的 docstring 是仓库**首次**正式文字化"Tauri 模式 vs HTTP 模式行为差异"。其它地方对 transport 模式的判断散落在 [`isTauriMode()`](../../src/lib/transport.ts) 调用点 + 两个 transport adapter 实现 + `transport-provider.ts` 选 adapter 逻辑，没有架构级综述。新人接手或调试 transport 相关 bug 必须读多个源才能拼出全图。
- **为什么留**：架构文档需要先把"打算保留的" vs "打算简化掉的"区分清楚（参见 F-010 关于 `/ws/chat/{id}` 死路径），再写权威文档；现在写容易立刻过时。
- **改的话要做什么**：在 [`docs/architecture/`](../README.md) 新建 `transport-modes.md`，覆盖：(a) 三种运行模式的事件流向图；(b) 每个 Transport 方法在两种模式下的实现路径；(c) `chat:stream_delta` 双写架构 + reattach 角色（Tauri 兜底 vs HTTP 主路径）；(d) 列出所有 EventBus 事件名 + 用途；(e) 决策记录 "为什么 startChat 不是 streamCall 通用原语"。回填到 `docs/README.md` 索引。
- **影响面**：纯文档债。无功能影响。
- **触发时机建议**：F-010 决策落地（startChat 合约 / `/ws/chat/{id}` 死路径处置）后再写，避免文档与代码不同步。

---

## Closed

> 已修复条目移到此处，附 commit hash + 关闭日期。保留以便后续 grep。

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
