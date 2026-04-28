# Chat Engine 对话引擎架构

> 返回 [文档索引](../README.md) | 更新时间：2026-04-05

## 目录

- [概述](#概述)
- [模块结构](#模块结构)
- [核心类型](#核心类型)
  - [EventSink trait](#eventsink-trait)
  - [ChatEngineParams](#chatengineparams)
  - [ChatEngineResult](#chatengineresult)
  - [CapturedUsage](#capturedusage)
- [请求流程](#请求流程)
- [流式事件协议](#流式事件协议)
- [流式回调处理](#流式回调处理)
- [Stream Broadcast & Reload Recovery](#stream-broadcast--reload-recovery)
- [Failover 集成](#failover-集成)
- [Post-turn Effects](#post-turn-effects)
- [记忆提取门控](#记忆提取门控)
- [集成关系](#集成关系)
- [文件清单](#文件清单)

---

## 概述

Chat Engine 是 Hope Agent 的对话编排入口，统一处理来自四种来源的请求：

| 来源 | EventSink 实现 | 说明 |
|---|---|---|
| UI 聊天（桌面） | `ChannelSink`（Tauri IPC Channel，定义在 src-tauri） | 用户直接交互（桌面模式） |
| UI 聊天（HTTP） | `NoopEventSink`（定义在 ha-core）+ `chat:stream_delta` EventBus | 用户直接交互（HTTP/WS 模式）；浏览器通过 `/ws/events` 接收流 |
| IM Channel | `ChannelStreamSink`（EventBus + mpsc） | Telegram / WeChat 等渠道 |
| Cron 定时任务 | `NoopEventSink` | 定时触发的对话复用同一个 noop sink，最终结果由 Cron delivery 处理 |
| ACP 协议 | stdio 协议输出层 | IDE 直连 |

Chat Engine 本身不持有状态，所有依赖通过 `ChatEngineParams` 注入。调用方（`commands/chat.rs`、`channel/worker.rs` 等）从 `State<AppState>` 或磁盘提取参数，构建 params 后调用 `run_chat_engine()`。

## 模块结构

```
crates/ha-core/src/chat_engine/
├── mod.rs              模块声明和 re-export
├── types.rs            EventSink trait + ChatEngineParams/Result + CapturedUsage
├── context.rs          Agent 构建 + 上下文恢复/保存 + 工具事件持久化 + Channel 中继 + 记忆提取
├── engine.rs           run_chat_engine() 核心引擎
├── persister.rs        StreamPersister：流式增量累积 + flush 到 SessionDB + 工具事件落库
├── stream_broadcast.rs `chat:stream_delta` / `chat:stream_end` / `channel:stream_delta` 事件名 + 广播抽象
└── stream_seq.rs       ChatSource 枚举 + 每会话流序号注册表（重载恢复去重 cursor）
```

## 核心类型

### EventSink trait

抽象事件输出层，解耦引擎与具体输出通道：

```rust
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: &str);
}
```

三种实现：

- **`ChannelSink`**（定义在 `src-tauri/src/commands/chat.rs`）— 包裹 `tauri::ipc::Channel<String>`，用于桌面模式 UI 直连。事件直接推送到 Tauri WebView 前端
- **`NoopEventSink`**（定义在 `crates/ha-core/src/chat_engine/types.rs`）— 丢弃所有事件。HTTP 模式、Cron 定时任务、subagent fork-and-forget 等"没有实时 UI 消费方"的入口共用此 sink；真正的浏览器流式输出由 Chat Engine 的 `chat:stream_delta` EventBus 双写路径推到 `/ws/events`
- **`ChannelStreamSink`**（定义在 `crates/ha-core/src/chat_engine/types.rs`）— 双路输出：(1) 通过 `EventBus` 发布 `channel:stream_delta` 事件推送到前端实时展示；(2) 通过 `mpsc::Sender` 转发到后台任务，驱动 IM 渠道的渐进式消息编辑（如 Telegram 消息实时更新）

### ChatEngineParams

完整的请求参数包，调用方一次性构建：

| 分组 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 基础 | `session_id` | `String` | 会话 ID |
| | `agent_id` | `String` | Agent ID |
| | `message` | `String` | 用户消息 |
| | `attachments` | `Vec<Attachment>` | 多模态附件 |
| | `session_db` | `Arc<SessionDB>` | 会话数据库 |
| 模型链 | `model_chain` | `Vec<ActiveModel>` | 预解析的模型降级链 |
| | `providers` | `Vec<ProviderConfig>` | Provider 配置快照 |
| | `codex_token` | `Option<(String, String)>` | Codex OAuth (access_token, account_id)；允许传 `None`，引擎侧在 `model_chain` 真的命中 Codex 时从磁盘 hydrate + refresh，三个入口（桌面 / HTTP / Channel）行为一致 |
| Agent 配置 | `resolved_temperature` | `Option<f64>` | 三层覆盖后的温度值 |
| | `web_search_enabled` | `bool` | 是否启用网络搜索 |
| | `notification_enabled` | `bool` | 是否启用通知 |
| | `image_gen_config` | `Option<ImageGenConfig>` | 图像生成配置 |
| | `canvas_enabled` | `bool` | 是否启用 Canvas |
| | `compact_config` | `CompactConfig` | 上下文压缩配置 |
| 可选 | `extra_system_context` | `Option<String>` | 额外系统提示词 |
| | `reasoning_effort` | `Option<String>` | 推理强度 |
| | `cancel` | `Arc<AtomicBool>` | 取消信号 |
| | `plan_agent_mode` | `Option<PlanAgentMode>` | Plan Mode 配置 |
| | `plan_mode_allow_paths` | `Option<Vec<String>>` | Plan Mode 路径白名单 |
| | `skill_allowed_tools` | `Vec<String>` | Skill 工具白名单 |
| | `denied_tools` | `Vec<String>` | 调用方执行策略级别的工具黑名单（与 schema 级过滤双重防御） |
| | `subagent_depth` | `u32` | 当前子 agent 嵌套深度，用于工具 schema 过滤与子 spawn 限制 |
| | `steer_run_id` | `Option<String>` | 关联 subagent run id；每轮 tool round 末尾 drain 对应 steer mailbox |
| | `auto_approve_tools` | `bool` | true 时所有工具调用免审批（IM 渠道 auto-approve 模式） |
| | `follow_global_reasoning_effort` | `bool` | Provider 循环是否在 turn 中途重读全局 reasoning effort |
| | `post_turn_effects` | `bool` | 成功响应后是否调度自动标题 / 记忆提取 / 技能审核（subagent 等场景关掉） |
| | `abort_on_cancel` | `bool` | 调用方取消时是否丢弃 partial 响应并返回 Err（区别于持久化为最终 assistant 行） |
| | `persist_final_error_event` | `bool` | engine 是否落自身的最终错误事件（Channel 等已自管的入口设为 false） |
| | `source` | `ChatSource` | 流入口标识，驱动 `/api/server/status` 的 `activeChatCounts` 分类 |
| 输出 | `event_sink` | `Arc<dyn EventSink>` | 事件输出通道 |

### ChatEngineResult

```rust
pub struct ChatEngineResult {
    pub response: String,                  // 最终响应文本
    pub model_used: Option<ActiveModel>,   // 实际使用的模型
    pub agent: Option<AssistantAgent>,     // Agent 实例（UI chat 用于更新 State）
}
```

### CapturedUsage

从流式回调中捕获的 Token 使用量和性能指标：

```rust
struct CapturedUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub model: Option<String>,
    pub ttft_ms: Option<i64>,        // Time To First Token
}
```

## 请求流程

```mermaid
sequenceDiagram
    participant Caller as 调用方
    participant Engine as Chat Engine
    participant Agent as AssistantAgent
    participant DB as SessionDB
    participant Sink as EventSink

    Caller->>Engine: run_chat_engine(params)

    loop 遍历 model_chain
        Engine->>Engine: 1. build_agent_from_snapshot()
        Engine->>Engine: 2. 配置 Agent（温度/工具/Plan Mode 等）
        Engine->>DB: 3. restore_agent_context()（加载 conversation_history）
        Engine->>DB: 4. update_session_model()

        loop 重试循环（MAX_RETRIES=2）
            alt 非首个模型
                Engine->>Sink: emit model_fallback 事件
                Engine->>DB: append_message(Event)
            end

            Engine->>Agent: agent.chat(message, attachments, effort, cancel, on_delta)

            Note over Engine: on_delta 回调实时处理：<br/>- usage → 更新 CapturedUsage<br/>- thinking_delta → 累积 pending_thinking<br/>- text_delta → 累积 pending_text<br/>- tool_call → flush thinking/text → 持久化<br/>- tool_result → 更新工具结果

            alt 成功
                Engine->>Sink: emit usage（含 duration_ms）
                Engine->>DB: flush 剩余 thinking/text blocks
                Engine->>DB: append_message(Assistant + usage 元数据)
                Engine->>DB: save_agent_context()
                Engine->>Sink: emit chat:stream_end
                Engine->>Engine: schedule_memory_extraction_after_turn()
                Engine-->>Caller: Ok(ChatEngineResult)
            else ContextOverflow（首次）
                Engine->>Engine: emergency_compact()
                Engine->>DB: save_agent_context()
                Engine->>Sink: emit context_compacted
                Note over Engine: retry_count++ → 继续重试
            else Terminal 错误
                Engine->>DB: save context + append Event
                Engine-->>Caller: Err(error)
            else Retryable（retry < MAX）
                Note over Engine: 指数退避等待后重试
            else 重试耗尽 / Non-retryable
                Note over Engine: break → 尝试下一个模型
            end
        end
    end

    Engine->>DB: append_message(Event: 全部失败)
    Engine-->>Caller: Err("All models failed")
```

### 7 步详解

1. **初始化** — 从 `model_chain` 构建 Agent，配置温度、工具限制、Plan Mode 等
2. **上下文恢复** — `restore_agent_context()` 从 DB 加载 `context_json`，反序列化为 `Vec<Value>` 设回 Agent
3. **流式执行** — 调用 `agent.chat()` 启动 LLM 请求 + Tool Loop，通过 `on_delta` 回调实时处理
4. **响应持久化** — flush 未完成的 thinking/text blocks，保存 assistant 消息（附带 tokens、model、ttft_ms、duration_ms）
5. **上下文保存** — `save_agent_context()` 将更新后的 conversation_history 序列化存回 DB
6. **记忆提取** — assistant 消息落库后结束可见 stream，再后台调度自动记忆提取，避免 stop 按钮 / sidebar spinner 被后处理任务拖住
7. **错误处理** — 分类错误、决定重试/降级/终止

## 流式事件协议

所有事件通过 `EventSink.send()` 以 JSON 字符串形式推送，前端通过 `type` 字段分发处理：

| type | 字段 | 说明 |
|---|---|---|
| `usage` | `input_tokens, output_tokens, model, ttft_ms, duration_ms` | Token 用量和性能指标 |
| `text_delta` | `text` | 文本增量 |
| `thinking_delta` | `content` | 思考内容增量 |
| `tool_call` | `call_id, name, arguments` | 工具调用发起 |
| `tool_result` | `call_id, result, duration_ms, is_error` | 工具执行结果 |
| `model_fallback` | `model, from_model, provider_id, model_id, reason, attempt, total, error` | 模型降级通知 |
| `context_compacted` | `data` | 上下文压缩完成 |
| `codex_auth_expired` | `error` | Codex OAuth Token 过期 |
| `event` | （通用） | 其他系统事件 |

## 流式回调处理

`on_delta` 闭包在 `agent.chat()` 的流式输出过程中被调用，承担两项职责：

**1. 累积与 flush 机制**

- `pending_text` / `pending_thinking` — 使用 `Arc<Mutex<String>>` 累积增量文本
- 遇到 `tool_call` 事件时，将累积内容 flush 为 `TextBlock` / `ThinkingBlock` 消息写入 DB
- 最终响应成功后，flush 剩余的 pending 内容
- `thinking_start_time` 记录首个 `thinking_delta` 的时间，计算 thinking 总耗时

**2. 工具事件持久化**

`persist_tool_event()` 拦截 `tool_call` 和 `tool_result` 事件：
- `tool_call` → 创建新的 Tool 消息（结果为空）
- `tool_result` → 通过 `call_id` 匹配更新已有 Tool 消息的 result、duration、is_error

## Stream Broadcast & Reload Recovery

每条 stream delta 走「双写」路径：

1. **主路径** — `EventSink.send()` 直接推 per-call sink（桌面 IPC Channel / `NoopEventSink`）
2. **保险路径 / 广播路径** — 同一事件经 `chat_engine::stream_broadcast` 注入序号后，通过 `EventBus` 发 `chat:stream_delta`（带 `{sessionId, seq}`）；HTTP / Tauri 前端订阅 `/ws/events` 或 Tauri 事件总线时统一从这里取流

`stream_seq.rs` 维护按 `(session_id, ChatSource)` 分组的递增序号注册表，并暴露 `begin / end / current_seq` 给重载恢复路径——前端断线重连或刷新时携带最后 seq 作为 cursor，主路径与广播路径共享同一 cursor 去重，互为兜底（IM Channel 的 mpsc 死掉时 Bus 路径接管）。

`ChatSource` 枚举区分 UI / Channel / Cron / Subagent 等入口，决定是否注册到 reload-recovery 索引、是否落 `activeChatCounts`，以及是否触发 `chat:stream_end` 收尾广播；IM 渠道走的 `channel:stream_delta` 与主 chat 流分别走独立事件名互不混淆。

> 历史遗留的 per-session chat WebSocket 路由已于 commit `8860eb23` 移除，所有 stream 现统一走 `/ws/events` 单通道。

## Failover 集成

Chat Engine 内置完整的模型降级和重试逻辑：

```mermaid
flowchart TD
    A[agent.chat() 失败] --> B{classify_error}
    B -->|ContextOverflow| C{首次?}
    C -->|是| D[emergency_compact + 重试]
    C -->|否| E[Terminal: 返回错误]
    B -->|Terminal<br/>Auth/Billing/ModelNotFound| E
    B -->|Retryable<br/>RateLimit/Overloaded/Timeout| F{retry < MAX_RETRIES?}
    F -->|是| G["指数退避等待<br/>delay = min(base * 2^retry, 10s)"]
    G --> H[重试同一模型]
    F -->|否| I[尝试 model_chain 下一模型]
    B -->|Auth + Codex| J[emit codex_auth_expired]
    J --> I

```

**退避参数：**
退避基数 / 上限 / 单模型重试次数已统一外移到 `failover::FailoverPolicy::chat_engine_default()`（见 [failover.md](./failover.md)），engine 内不再自管这三个常量。引擎本地仅保留一个常量 `MAX_COMPACTION_RETRIES = 1`（每模型最多紧急压缩重试 1 次），其它分类、退避、profile 轮换、Codex 强制不轮换等行为全部交给 `failover::executor::execute_with_failover` 配合 `chat_engine_default` policy 决定。

**Codex 特殊处理：** Auth 错误时，如果当前 Provider 是 Codex 类型，额外发送 `codex_auth_expired` 事件通知前端触发重新授权流程。

## Post-turn Effects

成功响应、assistant 消息落库并完成可见 stream 收尾后，若 `ChatEngineParams.post_turn_effects = true`，引擎会在最终 `Ok` 返回前依次调度三组后处理（均为后台 spawn，不阻塞调用方）：

1. **自动会话标题** — `crate::session_title::maybe_schedule_after_success(...)`（源：`crates/ha-core/src/session_title.rs`）按门槛触发 side_query 起标题
2. **自动记忆提取** — `schedule_memory_extraction_after_turn(...)` 走「记忆提取门控」描述的四道 Gate；同时累积本轮 token / message 计入 Agent 维度的 extraction stats
3. **技能审核（auto_review）** — 复用同一轮统计，调用 `skills::author` 的 auto-review 通道对本轮新增/修改的 skill draft 做安全扫描与 promotion 决策

`post_turn_effects=false` 用于 subagent fork-and-forget、cron 子调用等"不该改主会话用户感知状态"的入口，所有三项后处理整体跳过。

> 实现位置参考 `crates/ha-core/src/chat_engine/engine.rs` 中 `post_turn_effects` 分支（约 L451 起）。

## 记忆提取门控

`schedule_memory_extraction_after_turn()` 在每次成功响应后检查门控；满足阈值时通过 `tokio::spawn` 后台执行记忆提取。可见聊天流在最终 assistant 行落库后立即结束，自动提取不会阻塞前端的停止按钮、会话列表转圈或 `POST /chat` 返回：

| 门控 | 条件 | 说明 |
|---|---|---|
| Gate 1 | `auto_extract == true` | 全局或 Agent 级配置 |
| Gate 2 | `manual_memory_saved == false` | 本轮未手动调用 save_memory |
| Gate 3 | 冷却保护 | 距上次提取 ≥ `extract_time_threshold_secs`（默认 300s） |
| Gate 4 | 内容阈值（任一满足） | Token ≥ 阈值（默认 8000）或 消息数 ≥ 阈值（默认 10） |

Gate 3（冷却）和 Gate 4（内容）需同时满足。后台提取调度后重置追踪状态。

**空闲超时兜底**：当阈值提取未触发时（追踪状态未重置），调度延迟任务（默认 30 分钟）。超时后从 DB 加载历史执行最终提取。新建会话时 `create_session()` 调用 `flush_all_idle_extractions()` 立即执行所有待提取。

提取使用的 provider/model 可独立配置（Agent 级 > 全局 > 当前模型），支持用廉价模型做提取以降低成本。

## 集成关系

```mermaid
graph TB
    subgraph ChatEngine["Chat Engine"]
        Provider["Provider<br/>模型构建"]
        Agent["Agent<br/>chat() + Tool Loop"]
        Failover["Failover<br/>分类重试"]
        SessionDB["SessionDB<br/>消息持久化"]
        ContextCompact["Context Compact<br/>上下文压缩"]
        MemoryExtract["Memory Extract<br/>记忆提取"]
        Channel["Channel<br/>消息中继"]
        PlanMode["Plan Mode<br/>工具限制"]
    end

    Provider --- Agent
    Agent --- Failover
    SessionDB --- ContextCompact
    ContextCompact --- MemoryExtract
    Channel --- PlanMode
```

| 模块 | 交互方式 | 说明 |
|---|---|---|
| **SessionDB** | 直接调用 | 消息追加、上下文存取、工具结果更新 |
| **Provider** | `build_agent_from_snapshot()` | 根据 Provider 配置构建 Agent |
| **AssistantAgent** | `agent.chat()` | Tool Loop、流式输出、Side Query |
| **Failover** | `classify_error()` + `retry_delay_ms()` | 错误分类和退避计算 |
| **Context Compact** | `emergency_compact()` | ContextOverflow 时紧急压缩 |
| **Memory Extract** | `run_extraction()` | 自动记忆提取 |
| **Channel** | `relay_to_channel()` | IM Channel 消息中继（context.rs） |
| **Plan Mode** | `plan_agent_mode` + `plan_mode_allow_paths` | 透传到 Agent 限制工具和路径 |

## 文件清单

| 文件 | 职责 |
|---|---|
| `crates/ha-core/src/chat_engine/mod.rs` | 模块声明和 re-export |
| `crates/ha-core/src/chat_engine/types.rs` | EventSink trait、ChatEngineParams、ChatEngineResult、CapturedUsage |
| `crates/ha-core/src/chat_engine/context.rs` | Agent 构建、上下文恢复/保存、工具事件持久化、Channel 中继、记忆提取 |
| `crates/ha-core/src/chat_engine/engine.rs` | `run_chat_engine()` 核心引擎：模型链遍历、重试循环、流式处理、failover |
