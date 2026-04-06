# Session 管理对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.13

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| 存储引擎 | SQLite（WAL 模式） | JSONL Transcript 文件 + 云端 API | JSON 文件（按 session key） |
| 会话标识 | UUID string | UUID（本地）+ 云端 session ID | 复合 key（channel:account:chat） |
| 消息模型 | 关系型（sessions + messages 表） | SDK Event 流（SDKMessage） | transcript 事件流 |
| 全文检索 | FTS5 | 无内置 | 无内置 |
| 远程会话 | 无 | Teleport + WebSocket | ACP + CLI session binding |
| 子 Agent 追踪 | subagent_runs 表 | 内存 Task 系统 | spawnedBy / spawnDepth |

## 二、OpenComputer 实现

### 2.1 SQLite 持久化

`src-tauri/src/session/db.rs` 管理核心数据库：

**sessions 表**：
- `id`（UUID PRIMARY KEY）、`title`、`agent_id`、`provider_id`、`provider_name`、`model_id`
- `created_at`、`updated_at`（ISO 8601）
- `context_json`（上下文快照）
- `last_read_message_id`（未读计数基准）
- `is_cron`（是否由定时任务创建）
- `parent_session_id`（子 Agent 关联）
- `plan_mode`（Plan Mode 状态：off/planning/executing）
- `plan_steps`（步骤进度 JSON，用于崩溃恢复）

**messages 表**：
- 6 种角色：`user`、`assistant`、`event`、`tool`、`text_block`、`thinking_block`
- 完整的工具调用字段：`tool_call_id`、`tool_name`、`tool_arguments`、`tool_result`、`tool_duration_ms`
- 附件元数据（`attachments_meta`）、推理努力（`reasoning_effort`）、思考内容（`thinking`）
- TTFT（Time To First Token）记录

**全文检索**：
- FTS5 虚拟表 `messages_fts`，自动同步 user/assistant 消息
- INSERT/DELETE 触发器自动维护索引
- unicode61 分词器支持多语言

**数据库配置**：
- WAL 模式：崩溃安全 + 并发读
- `PRAGMA synchronous=NORMAL`：平衡性能与持久性
- `PRAGMA foreign_keys=ON`：CASCADE 删除

### 2.2 会话恢复

- 会话数据完全持久化在 SQLite 中，重启后自动恢复
- `context_json` 字段保存 LLM 上下文状态，允许继续对话
- `plan_steps` 支持 Plan Mode 崩溃恢复
- `last_read_message_id` 计算未读消息数
- `updated_at DESC` 索引支持快速按时间排序

### 2.3 子 Agent 追踪

**subagent_runs 表**独立追踪子 Agent 运行：
- `run_id`、`parent_session_id`、`child_session_id`
- `parent_agent_id`、`child_agent_id`
- `task`（任务描述）、`status`（spawning/running/done/failed/killed/timeout）
- `result`、`error`、`model_used`
- `depth`（嵌套深度）、`label`（标签）
- `input_tokens`、`output_tokens`（token 消耗追踪）
- `started_at`、`finished_at`、`duration_ms`

**acp_runs 表**追踪 ACP 协议运行：
- 类似 subagent_runs，增加 `backend_id`、`external_session_id`、`pid`

### 2.4 IM 渠道会话关联

`ChannelSessionInfo` 将会话绑定到 IM 渠道：
- `channel_id`、`account_id`、`chat_id`、`chat_type`、`sender_name`
- 支持将 Telegram/WeChat 对话映射到内部会话

## 三、Claude Code 实现

### 3.1 Transcript 文件存储

`src/assistant/sessionHistory.ts` 管理会话历史：

- 通过 Anthropic API 获取会话事件（`/v1/sessions/{id}/events`）
- `SDKMessage` 事件流作为基本存储单元
- 分页加载：`HISTORY_PAGE_SIZE = 100`，支持 `anchor_to_latest` 和 `before_id` 游标
- `HistoryAuthCtx` 封装认证上下文（OAuth token + org UUID），跨页复用

### 3.2 会话恢复（conversationRecovery）

`src/utils/conversationRecovery.ts` 处理会话恢复：

- `deserializeMessages` 反序列化消息状态
- 与 `teleport.tsx` 集成，支持从远程恢复
- 恢复时过滤技能列表注入消息（skill listing injection）
- 恢复时重建附件状态

### 3.3 Teleport（远程仓库会话桥接）

`src/utils/teleport/` 实现远程会话传输：

**api.ts**：
- `CCR_BYOC_BETA` header 标识远程会话协议版本
- `axiosGetWithRetry`：指数退避重试（2s/4s/8s/16s，最多 4 次）
- `isTransientNetworkError`：区分 5xx 瞬态错误 vs 4xx 永久错误
- `prepareApiRequest`：准备 OAuth headers + org UUID
- `sendEventToRemoteSession`：向远程会话发送事件

**gitBundle.ts**：
- Git 仓库打包传输，支持远程环境克隆
- 用于 Teleport 场景下的代码同步

**environments.ts**：
- 远程环境选择逻辑
- 支持多环境（dev/staging/prod）

### 3.4 远程会话（RemoteSessionManager）

`src/remote/RemoteSessionManager.ts` 管理远程 CCR 会话：

**RemoteSessionConfig**：
- `sessionId`、`getAccessToken`（动态获取）、`orgUuid`
- `hasInitialPrompt`：是否带初始 prompt 创建
- `viewerOnly`：纯查看模式（不发送中断、不更新标题、禁用 60s 重连超时）

**RemoteSessionCallbacks**：
- `onMessage`：SDKMessage 接收
- `onPermissionRequest`：CCR 权限请求（工具审批）
- `onPermissionCancelled`：权限请求取消
- `onConnected` / `onDisconnected`：连接状态

**SessionsWebSocket**（`src/remote/SessionsWebSocket.ts`）：
- WebSocket 连接到 `wss://api.anthropic.com/v1/sessions/ws/{id}/subscribe`
- 认证流：连接 -> 发送 auth message（OAuth token）-> 接收事件流
- 重连策略：`MAX_RECONNECT_ATTEMPTS = 5`，`RECONNECT_DELAY_MS = 2000`
- 心跳：`PING_INTERVAL_MS = 30000`
- 永久关闭码：4003（unauthorized）立即停止重连
- 4001（session not found）特殊处理：最多重试 3 次（compaction 期间可能瞬态 404）
- 支持 `onReconnecting` 回调区分瞬态断开和永久关闭
- 兼容 globalThis.WebSocket 和 ws 库（`WebSocketLike` 接口）

## 四、OpenClaw 实现

### 4.1 复合 Session Key

`src/config/sessions/types.ts` 定义丰富的 `SessionEntry`：

**会话标识**：
- `sessionId`（UUID）+ 复合 session key（`channel:account:chat` 格式）
- `session-id-resolution.ts` 支持模糊匹配和歧义消解（`none`/`ambiguous`/`selected`）
- `normalizeMainKey` 和 `toAgentRequestSessionKey` 标准化 key 格式

**生命周期管理**：
- `SessionLifecycleEvent`：`sessionKey`、`reason`、`parentSessionKey`、`label`、`displayName`
- 事件驱动的监听器系统（`onSessionLifecycleEvent`）
- `transcript-events.ts`：会话 transcript 更新事件系统

### 4.2 丰富的会话状态

`SessionEntry` 包含极其丰富的运行时状态：

- **模型配置**：`model`、`modelProvider`、`providerOverride`、`modelOverride`
- **认证**：`authProfileOverride`、`authProfileOverrideSource`（auto/user）
- **Token 追踪**：`inputTokens`、`outputTokens`、`totalTokens`、`totalTokensFresh`、`cacheRead`、`cacheWrite`
- **成本**：`estimatedCostUsd`
- **压缩**：`compactionCount`、`contextTokens`
- **消息队列**：`queueMode`（steer/followup/collect/interrupt 等 8 种）、`queueDebounceMs`、`queueCap`、`queueDrop`
- **中止控制**：`abortCutoffMessageSid`、`abortCutoffTimestamp`
- **记忆管理**：`memoryFlushAt`、`memoryFlushCompactionCount`、`memoryFlushContextHash`

### 4.3 子 Agent/Sandbox 支持

- `spawnedBy`：父会话 key
- `spawnedWorkspaceDir`：继承的工作目录
- `spawnDepth`：嵌套深度（0=main, 1=sub, 2=sub-sub）
- `subagentRole`：orchestrator / leaf
- `subagentControlScope`：children / none
- `forkedFromParent`：是否已从父 transcript fork

### 4.4 ACP 会话元数据

`SessionAcpMeta` 管理 ACP 协议集成：
- `backend`、`agent`、`runtimeSessionName`
- `identity`：ACP 身份状态（pending/resolved）
- `mode`：persistent / oneshot
- `runtimeOptions`：runtimeMode、model、cwd、permissionProfile、timeoutSeconds
- `state`：idle / running / error

### 4.5 CLI Session Binding

- `cliSessionBindings`：将 CLI 会话绑定到 OpenClaw 会话
- `extraSystemPromptHash`、`mcpConfigHash`：变更检测
- 支持多 CLI 实例绑定到同一 OpenClaw 会话

### 4.6 会话重置

- `DEFAULT_RESET_TRIGGERS`：`/new`、`/reset` 触发会话重置
- `DEFAULT_IDLE_MINUTES = 0`：无自动超时（可配置）

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| 持久化引擎 | SQLite WAL | JSONL + Cloud API | JSON 文件 |
| 全文检索 | FTS5（自动同步） | 无 | 无 |
| 消息角色类型 | 6 种 | SDKMessage 类型系统 | 自定义事件类型 |
| Token 追踪 | 每消息记录 | 每会话聚合 | 每会话聚合 |
| TTFT 记录 | 有（per-message） | 无 | 无 |
| 工具调用持久化 | 完整（参数+结果+耗时） | 事件流中 | 无独立表 |
| 未读计数 | 有 | 无 | 无 |
| 子 Agent 追踪 | 独立表（完整生命周期） | 内存 Task（非持久化） | session entry 字段 |
| 远程会话 | 无 | Teleport + WebSocket + CCR | ACP + CLI binding |
| 会话恢复 | SQLite 持久化 | API 事件重放 | 文件 + lifecycle event |
| Plan Mode 持久化 | 有（状态+步骤） | 无 | 无 |
| IM 渠道映射 | ChannelSessionInfo | 无 | 复合 session key 原生支持 |
| 消息队列模式 | 无 | 无 | 8 种模式 |
| 会话中止控制 | 无 | 无 | cutoff 时间戳 |
| ACP 集成 | acp_runs 表 | 无 | 完整 ACP 元数据 |
| 崩溃恢复 | SQLite WAL + plan_steps | conversationRecovery | 文件持久化 |
| 会话模糊搜索 | FTS5 全文 | 无 | session-id-resolution 歧义消解 |
| 多实例绑定 | 无 | 无 | cliSessionBindings |
| Fallback 通知 | event 消息 | model fallback 状态 | fallbackNotice 字段 |

## 六、差距分析与建议

### 6.1 核心差距

1. **无远程会话能力**：OpenComputer 仅支持本地会话，缺乏跨设备/跨环境协作
2. **无消息队列管理**：IM 渠道场景下缺乏消息排队、去抖、丢弃策略
3. **无会话中止精确控制**：缺乏 cutoff 时间戳机制，中止后可能重放旧消息
4. **子 Agent 缺乏运行时状态聚合**：虽有 subagent_runs 表但缺少 token 实时聚合到父会话

### 6.2 优势

1. **SQLite + FTS5**：关系型存储 + 全文检索是三者中最强的查询能力
2. **TTFT 追踪**：唯一在消息级别记录首 token 延迟的实现
3. **Plan Mode 崩溃恢复**：步骤级持久化确保长任务不丢失进度
4. **工具调用完整记录**：参数、结果、耗时全部持久化，Dashboard 可直接聚合

### 6.3 建议

**P0 - 短期**：
- 增加会话中止控制（cutoff 时间戳），避免 IM 渠道场景下中止后重放
- 子 Agent 运行时 token 聚合到父会话，Dashboard 展示完整成本

**P1 - 中期**：
- 消息队列模式（参考 OpenClaw 的 8 种模式），支持 IM 渠道高频消息场景
- 会话导出/导入功能，支持跨设备迁移

**P2 - 远期**：
- 探索远程会话能力：通过 ACP 协议支持远程环境的会话桥接
- 会话模板系统：预设上下文/工具/Agent 配置，一键创建特定场景会话
