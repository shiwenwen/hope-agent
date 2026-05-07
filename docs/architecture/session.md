# Session 会话系统架构

> 返回 [文档索引](../README.md) | 更新时间：2026-04-28

## 目录

- [概述](#概述)
- [数据模型](#数据模型)
  - [SessionMeta](#sessionmeta)
  - [SessionMessage](#sessionmessage)
  - [MessageRole](#messagerole)
  - [NewMessage Builder](#newmessage-builder)
- [SQLite Schema](#sqlite-schema)
- [核心 API](#核心-api)
  - [会话管理](#会话管理)
  - [消息 CRUD](#消息-crud)
  - [元数据更新](#元数据更新)
  - [上下文持久化](#上下文持久化)
  - [Plan Mode 崩溃恢复](#plan-mode-崩溃恢复)
  - [已读状态](#已读状态)
  - [全文搜索](#全文搜索)
  - [Subagent 运行记录](#subagent-运行记录)
  - [ACP 运行记录](#acp-运行记录)
- [会话生命周期](#会话生命周期)
- [无痕会话（Incognito）](#无痕会话incognito)
- [会话级工作目录](#会话级工作目录)
- [会话级 Awareness 与 Agent 切换](#会话级-awareness-与-agent-切换)
- [自动会话标题](#自动会话标题)
- [特殊设计](#特殊设计)
- [关联文档](#关联文档)
- [文件清单](#文件清单)

---

## 概述

Session 模块是 Hope Agent 的会话与消息持久化系统，基于 SQLite WAL 模式实现高并发读写。所有对话数据（会话元信息、消息内容、工具调用记录、Agent 上下文快照、子 Agent 运行记录、ACP 运行记录）统一存储在 `~/.hope-agent/sessions.db`。

核心职责：

1. **会话生命周期管理** — 创建、列表（分页）、删除、元数据更新
2. **消息持久化** — user / assistant / tool / event / text_block / thinking_block 六种角色
3. **上下文快照** — conversation_history JSON 序列化存储，支持跨重启恢复
4. **全文搜索** — FTS5 虚拟表 + unicode61 分词器，自动触发器同步
5. **未读追踪** — 基于 `last_read_message_id` 水位线的轻量已读/未读机制
6. **子系统记录** — Subagent 运行和 ACP 运行的完整 CRUD

## 数据模型

### SessionMeta

会话元信息，用于列表展示和路由。源：[crates/ha-core/src/session/types.rs:11](../../crates/ha-core/src/session/types.rs)。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `title` | `Option<String>` | 会话标题 |
| `title_source` | `String` | 标题来源：`"manual"` / `"first_message"` / `"llm"`（默认 manual） |
| `agent_id` | `String` | 关联 Agent ID，默认 `"default"` |
| `provider_id` | `Option<String>` | 当前使用的 Provider ID |
| `provider_name` | `Option<String>` | Provider 显示名称 |
| `model_id` | `Option<String>` | 当前使用的模型 ID |
| `created_at` | `String` | RFC 3339 创建时间 |
| `updated_at` | `String` | RFC 3339 最后更新时间 |
| `message_count` | `i64` | 消息总数（子查询计算） |
| `unread_count` | `i64` | 未读消息数（子查询计算） |
| `pending_interaction_count` | `i64` | 待用户交互数（pending 工具审批 + ask_user 组数）；由 command/route 层填充，`list_sessions_paged` 不计算 |
| `is_cron` | `bool` | 是否为定时任务创建的会话 |
| `parent_session_id` | `Option<String>` | 父会话 ID（子 Agent 会话） |
| `plan_mode` | `String` | Plan Mode 状态：`"off"` / `"planning"` / `"executing"` |
| `tool_permission_mode` | `String` | 工具审批模式：`"auto"` / `"ask_every_time"` / `"full_approve"`（默认 auto） |
| `project_id` | `Option<String>` | 所属项目 ID；项目作用域记忆/文件在该项目内全部会话间共享 |
| `channel_info` | `Option<ChannelSessionInfo>` | IM Channel 关联信息（LEFT JOIN channel_conversations） |
| `incognito` | `bool` | 无痕模式开关：true 时不注入被动记忆/awareness、不做自动记忆提取，且关闭即焚 |
| `working_dir` | `Option<String>` | 会话级工作目录绝对路径（注入到 system prompt 当默认目录）；server 模式下指 server 机器路径 |

### SessionMessage

单条消息记录，涵盖所有消息类型的超集字段。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `i64` | 自增主键 |
| `session_id` | `String` | 所属会话 ID |
| `role` | `MessageRole` | 消息角色 |
| `content` | `String` | 消息内容 |
| `timestamp` | `String` | RFC 3339 时间戳 |
| `attachments_meta` | `Option<String>` | 附件元信息 JSON（User 消息） |
| `model` | `Option<String>` | 响应模型名（Assistant 消息） |
| `tokens_in` / `tokens_out` | `Option<i64>` | Token 用量统计 |
| `tokens_in_last` | `Option<i64>` | 末轮输入 token（`ChatUsage::last_input_tokens`，用于压缩判定） |
| `tokens_cache_creation` | `Option<i64>` | Anthropic prompt cache 写入 token |
| `tokens_cache_read` | `Option<i64>` | prompt cache 读命中 token（OpenAI: `cached_tokens`） |
| `reasoning_effort` | `Option<String>` | 推理强度设置 |
| `ttft_ms` | `Option<i64>` | Time To First Token（毫秒） |
| `tool_call_id` | `Option<String>` | 工具调用 ID |
| `tool_name` | `Option<String>` | 工具名称 |
| `tool_arguments` | `Option<String>` | 工具参数 JSON |
| `tool_result` | `Option<String>` | 工具执行结果 |
| `tool_duration_ms` | `Option<i64>` | 工具执行耗时（毫秒） |
| `is_error` | `Option<bool>` | 工具是否执行失败 |
| `thinking` | `Option<String>` | 旧路径思考内容（Assistant 消息内联）；新路径用独立 `ThinkingBlock` 行存储以保持工具调用前后顺序，详见下方 `MessageRole` |
| `source` | `Option<String>` | 触发该 turn 的入口（`ChatSource::as_str()` lowercase：`desktop` / `http` / `channel` / `subagent` / `parentinjection`）。NULL 视作 `desktop`（保守，不破坏老会话已有未读）。Unread badge / GUI→IM 镜像引用前缀都按此字段分流 |

### MessageRole

6 种消息角色枚举：

```
User          — 用户输入
Assistant     — 模型响应
Event         — 系统事件（错误通知、模型降级等）
Tool          — 工具调用及结果
TextBlock     — 中间文本块（工具调用前的文本输出，保持顺序）
ThinkingBlock — 中间思考块（工具调用前的思考输出，保持多轮思考顺序）
```

`TextBlock` 和 `ThinkingBlock` 是流式输出中的中间态消息。当模型在输出过程中穿插工具调用时，引擎会将已累积的文本/思考 flush 为独立消息，确保 UI 展示顺序与模型输出顺序一致。

### NewMessage Builder

`NewMessage` 提供 6 个便捷构造函数，统一设置时间戳和角色：

| 构造函数 | 角色 | 说明 |
|---|---|---|
| `NewMessage::user(content)` | User | 简单用户消息 |
| `NewMessage::assistant(content)` | Assistant | 模型响应 |
| `NewMessage::tool(call_id, name, args, result, duration, is_error)` | Tool | 工具调用记录 |
| `NewMessage::text_block(content)` | TextBlock | 中间文本块 |
| `NewMessage::thinking_block(content)` | ThinkingBlock | 中间思考块 |
| `NewMessage::thinking_block_with_duration(content, duration_ms)` | ThinkingBlock | 带耗时的思考块 |
| `NewMessage::event(content)` | Event | 系统事件 |

## SQLite Schema

数据库在 `SessionDB::open()` 时自动创建表和索引，并通过渐进式 Migration 添加新列。

### 主表

实际 DDL 由 `CREATE TABLE` + 多次 `ALTER TABLE ADD COLUMN` 组合构成（`db.rs::open` 末尾的渐进 migration）。下列是合并后的"逻辑视图"：

```sql
-- 会话表（合并所有迁移列）
CREATE TABLE sessions (
    id                       TEXT PRIMARY KEY,
    title                    TEXT,
    title_source             TEXT NOT NULL DEFAULT 'manual',  -- manual / first_message / llm
    agent_id                 TEXT NOT NULL DEFAULT 'default',
    provider_id              TEXT,
    provider_name            TEXT,
    model_id                 TEXT,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    context_json             TEXT,                            -- Agent conversation_history 快照
    last_read_message_id     INTEGER DEFAULT 0,
    is_cron                  INTEGER NOT NULL DEFAULT 0,
    parent_session_id        TEXT,
    plan_mode                TEXT DEFAULT 'off',
    plan_steps               TEXT,                            -- Plan 步骤进度 JSON（崩溃恢复）
    tool_permission_mode     TEXT NOT NULL DEFAULT 'auto',    -- 工具审批模式
    project_id               TEXT,                            -- 所属项目（外部表 projects）
    awareness_config_json    TEXT,                            -- per-session awareness override
    incognito                INTEGER NOT NULL DEFAULT 0,      -- 无痕模式
    working_dir              TEXT                             -- 会话级工作目录
);

-- 消息表
CREATE TABLE messages (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id               TEXT NOT NULL,
    role                     TEXT NOT NULL,                   -- user/assistant/event/tool/text_block/thinking_block
    content                  TEXT NOT NULL DEFAULT '',
    timestamp                TEXT NOT NULL,
    attachments_meta         TEXT,
    model                    TEXT,
    tokens_in                INTEGER,
    tokens_out               INTEGER,
    reasoning_effort         TEXT,
    tool_call_id             TEXT,
    tool_name                TEXT,
    tool_arguments           TEXT,
    tool_result              TEXT,
    tool_duration_ms         INTEGER,
    is_error                 INTEGER DEFAULT 0,
    thinking                 TEXT,                            -- 旧路径内联思考；新路径用 thinking_block 行
    ttft_ms                  INTEGER,
    tokens_in_last           INTEGER,
    tokens_cache_creation    INTEGER,
    tokens_cache_read        INTEGER,
    tool_metadata            TEXT,                            -- JSON: 工具结构化副输出（diff/before-after）
    stream_status            TEXT,                            -- streaming/completed/orphaned，NULL 视为 completed
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
```

`stream_status` 取值：

- `streaming` — 当前正在被一个活跃的 `StreamPersister` 节流写入，placeholder 行
- `completed` — 流式块已 finalize，content 是最终内容
- `orphaned` — 启动扫尾把上次崩溃残留的 `streaming` 行批量改成此状态；前端按"上次未完成"渲染
- `NULL` — 旧版本数据库残留，所有 reader 视为 `completed`

详见 [`chat-engine.md` Round-level Persistence & Crash Recovery](chat-engine.md#round-level-persistence--crash-recovery)。

### 索引

```sql
CREATE INDEX idx_messages_session_id  ON messages(session_id);
CREATE INDEX idx_sessions_agent_id    ON sessions(agent_id);
CREATE INDEX idx_sessions_updated_at  ON sessions(updated_at DESC);
CREATE INDEX idx_sessions_project_id  ON sessions(project_id);
-- 部分索引：仅覆盖 streaming 行，让 mark_orphaned_streaming_rows() 启动扫尾走 O(streaming-count)
CREATE INDEX idx_messages_stream_active
  ON messages(session_id, stream_status) WHERE stream_status = 'streaming';
```

### FTS5 全文搜索

```sql
-- 虚拟表（unicode61 分词器支持 CJK）
CREATE VIRTUAL TABLE messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id',
    tokenize='unicode61'
);

-- 自动同步触发器（仅索引 user/assistant 非空消息）
CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages
WHEN new.role IN ('user', 'assistant') AND length(new.content) > 0
BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages
WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
END;
```

### 子 Agent / ACP 运行表

```sql
-- 子 Agent 运行记录
CREATE TABLE subagent_runs (
    run_id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL,
    parent_agent_id TEXT NOT NULL,
    child_agent_id TEXT NOT NULL,
    child_session_id TEXT NOT NULL,
    task TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'spawning',  -- spawning/running/completed/error
    result TEXT,
    error TEXT,
    depth INTEGER NOT NULL DEFAULT 1,
    model_used TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER,
    label TEXT,
    attachment_count INTEGER DEFAULT 0,
    input_tokens INTEGER,
    output_tokens INTEGER
);

-- ACP 运行记录
CREATE TABLE acp_runs (
    run_id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL,
    backend_id TEXT NOT NULL,
    external_session_id TEXT,
    task TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'starting',  -- starting/running/completed/error/timeout/killed
    result TEXT,
    error TEXT,
    model_used TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    label TEXT,
    pid INTEGER
);
```

## 核心 API

### 会话管理

| 方法 | 说明 |
|---|---|
| `create_session(agent_id)` | 创建新会话，返回 `SessionMeta` |
| `create_session_with_parent(agent_id, parent_id)` | 创建子 Agent 会话 |
| `create_session_with_project(agent_id, project_id)` | 创建项目作用域会话；`project_id` 非空时强制 `incognito=false`（互斥防御） |
| `get_session(session_id)` | 获取单个会话元信息（含 Channel LEFT JOIN） |
| `list_sessions(agent_id)` | 列出所有会话（按 updated_at DESC） |
| `list_sessions_paged(agent_id, project_filter, limit, offset, active_session_id)` | 分页列表，返回 `(Vec<SessionMeta>, total_count)`。**5 参签名**：见下方 `ProjectFilter` 与 incognito 例外 |
| `delete_session(session_id)` | 删除会话：① CASCADE 删消息 → ② 清理 `plans/{id}.md` 与 `attachments/{id}/` → ③ `cleanup_session_orphan_tables` 单独事务清四张无 FK 表（见下） |
| `purge_session_if_incognito(session_id)` | 仅当会话是无痕态时硬删；前端切走当前无痕会话时调用，实现"关闭即焚" |
| `purge_orphan_incognito_sessions()` | 启动期兜底清理：遍历 `incognito = 1` 且 `updated_at < (now - 60s)` 的会话调用 `delete_session`；防御 crash / SIGKILL / 物理断电后残留 |

**`ProjectFilter<'a>` 枚举**（`db.rs:2426`）：

| 变体 | 语义 |
|---|---|
| `All` | 不按项目过滤 |
| `Unassigned` | 仅 `project_id IS NULL`（不属于任何项目） |
| `InProject(&str)` | 仅属于指定项目的会话 |

**`active_session_id` 例外参数**：默认情况下 `incognito = 0` 强制过滤无痕会话出列表（"无痕"语义）。但用户当前正在打开的那个无痕会话仍需出现在 sidebar，该参数提供例外：`Some(sid)` 时 WHERE 子句变为 `(s.incognito = 0 OR s.id = ?)`。`None` 时严格过滤。

**`delete_session` 关联表清理协议**（`cleanup_session_orphan_tables`，`db.rs:1633`）：

```rust
// 单独事务，按以下顺序删四张无 FK CASCADE 的关联表：
DELETE FROM session_skill_activation WHERE session_id = ?;
DELETE FROM learning_events           WHERE session_id = ?;
DELETE FROM subagent_runs             WHERE parent_session_id = ?;
DELETE FROM acp_runs                  WHERE parent_session_id = ?;
```

失败仅 `app_warn!` 不向上传播——保证主删 `sessions` 行成功后即使关联表清理失败也不阻塞用户。该 contract 在 [AGENTS.md](../../AGENTS.md) 列为强制要求。

### 消息 CRUD

| 方法 | 说明 |
|---|---|
| `append_message(session_id, msg)` | 追加消息并更新 `updated_at`，返回消息 ID |
| `load_session_messages(session_id)` | 加载全部消息（ASC 排序） |
| `load_session_messages_latest(session_id, limit)` | 加载最新 N 条消息 + 总数（首屏加载） |
| `load_session_messages_before(session_id, before_id, limit)` | 向上翻页加载（scroll up） |
| `update_tool_result(session_id, call_id, result, duration, is_error)` | 更新工具执行结果（按 call_id 匹配） |

### 元数据更新

| 方法 | 说明 |
|---|---|
| `update_session_title(session_id, title)` | 更新会话标题（不改 `title_source`） |
| `update_session_title_with_source(session_id, title, source)` | 更新标题并设置来源标签 |
| `update_session_title_if_source(session_id, title, expected_source)` | 仅当当前 `title_source` 等于 `expected_source` 时更新（防止 LLM 标题覆盖用户手改） |
| `update_session_model(session_id, provider_id, provider_name, model_id)` | 更新当前模型信息 |
| `mark_session_cron(session_id)` | 标记为 Cron 会话 |
| `update_session_plan_mode(session_id, plan_mode)` | 更新 Plan Mode 状态 |
| `update_session_tool_permission_mode(session_id, mode)` | 更新工具审批模式 |
| `update_session_incognito(session_id, bool)` | 切换无痕态；`project_id IS NOT NULL` 或 `channel_info IS NOT NULL` 时直接 `Err`（互斥防御） |
| `update_session_working_dir(session_id, Option<&str>)` | 设置/清空会话级工作目录；空串当清空 |
| `update_session_agent(session_id, agent_id)` | 切换会话 Agent；SQL 层强制 `message_count == 0`，非空会话直接拒绝（防止改动一半切 Agent 造成上下文错乱） |

### 上下文持久化

| 方法 | 说明 |
|---|---|
| `save_context(session_id, context_json)` | 保存 Agent 的 `conversation_history` JSON |
| `load_context(session_id)` | 加载上下文 JSON（无则返回 None） |

上下文以 `Vec<serde_json::Value>` 序列化为 JSON 字符串，存储在 `sessions.context_json` 列。Chat Engine 在每次请求开始时调用 `restore_agent_context()` 恢复，请求结束后调用 `save_agent_context()` 持久化。

### Plan Mode 崩溃恢复

| 方法 | 说明 |
|---|---|
| `save_plan_steps(session_id, steps_json)` | 持久化 Plan 步骤进度 JSON |
| `load_plan_steps(session_id)` | 加载 Plan 步骤进度 |

Plan 执行过程中，步骤进度实时写入 `sessions.plan_steps` 列。应用崩溃重启后可从此列恢复执行进度，避免重复执行已完成步骤。

### 已读状态

基于 `last_read_message_id` 水位线机制：未读数 = `messages.id > last_read_message_id AND role = 'assistant' AND COALESCE(m.source, 'desktop') != 'channel'` 的记录数（SQL 子查询计算）。只计最终 `assistant` 行——一轮 turn 中的 `tool` / `text_block` / `thinking_block` / `event` 行都是同一个回复的组件或系统事件，不应重复计入徽标（否则单个问题触发 10 次工具调用就会显示 20+ 未读）。

**`source = 'channel'` 排除契约**：IM 入站消息触发的 assistant 回复，IM 用户在 IM 那侧已经看过了，桌面端不应再红点提醒。`COALESCE(m.source, 'desktop')` 让老行（NULL）保守计为桌面来源，避免升级时把存量真未读静默清空。`SESSION_META_SELECT` + `count_unread_messages` 等所有 unread SQL 必须保持一致。

| 方法 | 说明 |
|---|---|
| `mark_session_read(session_id)` | 标记单个会话已读 |
| `mark_session_read_batch(session_ids)` | 批量标记已读 |
| `mark_all_sessions_read()` | 全部标记已读 |

### 全文搜索

```rust
pub fn search_messages(
    query: &str,
    agent_id: Option<&str>,
    session_id: Option<&str>,            // None = 全局搜索；Some = 单会话搜索（Cmd+F）
    types: Option<&[SessionTypeFilter]>, // 会话类型筛选（regular/cron/subagent/channel）
    limit: usize,
) -> Vec<SessionSearchResult>
```

- 使用 FTS5 `MATCH` 语法，每个 token 用双引号包裹实现精确匹配（`sanitize_fts_query`）
- **incognito 过滤的双路径语义**：
  - `session_id = None`（**全局 FTS** 路径，sidebar 搜索框）：强制 `s.incognito = 0`，无痕会话内容**不会**被全局搜索到
  - `session_id = Some(sid)`（**会话内 Cmd+F** 路径）：**不应用** incognito 过滤；用户既然已经在该无痕会话里，允许搜本会话内容
- `SessionTypeFilter` 枚举：`Regular` / `Cron` / `Subagent` / `Channel`，用于 sidebar 类型 chips 筛选
- snippet 用 STX/ETX（U+0002/U+0003）作为 mark 边界——不可能在用户文本里出现，前端按字符 split 后白名单包回 `<mark>...</mark>`，避免 HTML escape/unescape 攻击面（用户写 `<mark onclick=...>` 不会被反解出来）
- 返回 `SessionSearchResult`：包含 `message_id` / `session_id` / `session_title` / `agent_id` / `message_role` / `content_snippet` / `timestamp` / `relevance_rank` / `is_cron` / `parent_session_id` / `channel_type` / `channel_chat_type`

### 命中后的滚动定位

```rust
pub fn load_session_messages_around(session_id, target_message_id, before_count, after_count)
```

加载目标消息前后窗口（默认 40/20），前端据此把搜索命中的消息加载到上下文里 + 滚动到位置 + pulse 高亮。`Cmd+F` 与全局搜索结果跳转复用同一路径。

### Subagent 运行记录

| 方法 | 说明 |
|---|---|
| `insert_subagent_run(run)` | 插入运行记录 |
| `update_subagent_status(run_id, status, result, error, model, duration)` | 更新状态 |
| `set_subagent_finished_at(run_id, finished_at)` | 设置完成时间 |
| `get_subagent_run(run_id)` | 获取单条记录 |
| `list_subagent_runs(parent_session_id)` | 列出父会话的全部运行 |
| `list_active_subagent_runs(parent_session_id)` | 列出活跃运行（spawning/running） |
| `count_active_subagent_runs(parent_session_id)` | 计数活跃运行 |
| `cleanup_orphan_subagent_runs()` | 启动时清理孤儿运行（标记为 error） |

### ACP 运行记录

`acp_runs` 表含 `pid INTEGER` 列存储后端进程 PID，`status` 取值 `starting/running/completed/error/timeout/killed`。

| 方法 | 说明 |
|---|---|
| `insert_acp_run(run_id, parent_session_id, backend_id, task, label)` | 插入运行记录 |
| `update_acp_run_status(run_id, status, pid, external_session_id)` | 更新状态、进程 PID、外部 session ID |
| `finish_acp_run(run_id, status, result, error, input_tokens, output_tokens)` | 完成运行（自动计算 duration） |
| `get_acp_run(run_id)` | 获取单条记录 |
| `list_acp_runs(parent_session_id)` | 列出父会话的全部运行 |

## 会话生命周期

```mermaid
stateDiagram-v2
    [*] --> 创建: create_session()
    创建 --> 活跃: 首条消息 append
    活跃 --> 活跃: 持续对话
    活跃 --> PlanMode: update_session_plan_mode("planning")
    PlanMode --> 活跃: update_session_plan_mode("off")
    活跃 --> 删除: delete_session()
    PlanMode --> 删除: delete_session()
    删除 --> [*]

    note right of 活跃
        每轮对话：
        1. restore context
        2. append user msg
        3. stream + persist tools
        4. append assistant msg
        5. save context
    end note
```

## 无痕会话（Incognito）

`sessions.incognito` 是无痕态的**单一真相源**。无痕会话除关闭被动 AI 行为外，关闭即焚——不进侧边栏列表、不进全局 FTS、不进 Dashboard 统计。

### 关闭的被动行为
- 不注入 Memory（包括 Active Memory）
- 不注入 Awareness suffix（`agent/mod.rs:704-710` 在 `refresh_awareness_suffix` 入口短路）
- 不跑 inline / idle / flush-before-compact 自动记忆提取
- 不参与跨会话 Awareness 候选采集

### 不进列表/统计的过滤路径
| 过滤位置 | 默认 | 例外 |
|---|---|---|
| `list_sessions_paged` | WHERE `incognito = 0` | `active_session_id = Some(sid)` 时该 sid 出列表 |
| `search_messages`（global） | WHERE `s.incognito = 0` | 无 |
| `search_messages`（in-session, Cmd+F） | 不过滤 | — |
| `dashboard::filters::build_session_filter` | WHERE `incognito = 0` | 无 |

### 关闭即焚

| 触发点 | 行为 |
|---|---|
| 前端切走当前无痕会话（`handleSwitchSession` / `handleNewChat` / `handleNewChatInProject`） | 调 `purge_session_if_incognito` 硬删，不留任何记录 |
| 启动期 | `purge_orphan_incognito_sessions` 兜底：扫描 `incognito = 1 AND updated_at < (now - 60s)` 的残留会话调用 `delete_session`（防御 crash / SIGKILL / 物理断电）。60s cutoff 是 defense-in-depth：即使 `runtime_lock` 选举失败导致两个进程并存，也不会误杀对方刚创建的活跃会话 |

### 与 Project / IM Channel 互斥

无痕会话不能进项目、不能绑定 IM channel：

- 前端 `IncognitoToggle` 在 `project_id != null` 或 `channel_info != null` 时灰化 + tooltip 解释
- 后端 `update_session_incognito` 对 `project_id.is_some()` / `channel_info.is_some()` 直接 `Err`
- `create_session_with_project` 在 `project_id` 存在时强制 `incognito = false`
- `channel/db.rs::ensure_conversation` 入口防御式清零（IM 路径创建的会话强制 `incognito = false`）

## 会话级工作目录

`sessions.working_dir` 持久化用户为该对话指定的绝对路径。`system_prompt::build` 在 Project 段之后、Memory 段之前插入 `# Working Directory` 段告诉模型默认操作目录。详细合并规则见 [Project 系统](project.md)，本节只覆盖会话侧入口。

### 写入校验

`update_session_working_dir(session_id, Option<&str>)` 走 `crate::util::canonicalize_working_dir`：
- 空串当清空
- 非空字符串先 `canonicalize` + `is_dir` 校验，不通过返回 `Err`
- 校验通过后写 `working_dir` 列

### 桌面 vs HTTP 选择路径

| 模式 | 前端入口 | 行为 |
|---|---|---|
| 桌面（Tauri） | `WorkingDirectoryButton` | 调 `@tauri-apps/plugin-dialog` 的 `open({ directory: true })` 弹原生目录选择 |
| HTTP/Server | `WorkingDirectoryButton` + `ServerDirectoryBrowser` Dialog | 走 axum `GET /api/filesystem/list-dir`（单级 + Bearer token） |

### 与 project_id / incognito 正交

会话级 working_dir 与项目级、与无痕模式独立：

- 项目内会话仍可单独设会话级 working_dir，按 [`session.helpers::effective_session_working_dir`](../../crates/ha-core/src/session/helpers.rs) 合并：`session.working_dir > project.working_dir > 不注入`，**lazy resolve**（不复制项目快照——改项目工作目录立即对未单独设置的项目内已有会话生效）
- 无痕会话也可设会话级 working_dir（与无痕语义不冲突——只是工具默认 cwd 提示）

## 会话级 Awareness 与 Agent 切换

### awareness_config_json

per-session override 列。当不为空时，`refresh_awareness_suffix` 用该会话的覆盖配置；为空则用 Agent 级配置。`incognito = 1` 时整个 refresh 路径短路（见上节）。

前端 `AwarenessToggle` 在 `incognito` 时 `disabled`，但**不**改写 `awareness_config_json`——关闭无痕后 awareness 配置自动恢复原状。

### Agent 切换 + message_count == 0 校验

`update_session_agent(session_id, agent_id)` 在 SQL 层校验 `message_count == 0`，对非空会话直接拒绝改动：

```sql
UPDATE sessions SET agent_id = ?
WHERE id = ?
  AND (SELECT COUNT(*) FROM messages WHERE session_id = ?) = 0;
```

前端 `AgentSwitcher` dropdown 在 `messages.length > 0` 时 disabled 是 UX 防御；DB 层是真实的 contract（防止从 ACP / IM 等绕过 UI 的写路径走漏）。

## 自动会话标题

`title_source` 三态机器：`manual` / `first_message` / `llm`。源：[crates/ha-core/src/session_title.rs](../../crates/ha-core/src/session_title.rs)。

| 来源 | 触发 | 覆盖关系 |
|---|---|---|
| `manual` | 用户手改标题 | 终态，不会被自动覆盖 |
| `first_message` | `ensure_first_message_title()` 在首条用户消息时取一行（≤50 字符自动 truncate） | 仅当 `title IS NULL AND message_count <= 1` 时设置 |
| `llm` | `maybe_schedule_after_success` 在 `post_turn_effects=true` 收尾时调度 LLM 起标题 | **仅当 `title_source == 'first_message'` 时**才覆盖（用 `update_session_title_if_source` 语义性 CAS）；保护用户手改不被覆盖 |

### LLM 触发条件（`maybe_schedule_after_success`）
- `AppConfig.session_title.enabled == true`
- `meta.incognito == false`
- `meta.title_source == 'first_message'`
- 选模型：`session_title.{provider_id, model_id}` 配置 > 当前 chat 模型

LLM 路径走独立线程 + 独立 tokio 运行时（不阻塞 chat 流），失败仅 `app_warn!`，不影响主流程。

## 特殊设计

### 消息持久化流程

```mermaid
flowchart TD
    A["append_message(session_id, msg)"] --> B["INSERT INTO messages"]
    B --> C["UPDATE sessions SET updated_at = now()"]
    C --> D{"role IN ('user', 'assistant')<br/>AND length(content) > 0?"}
    D -- 是 --> E["FTS 触发器:<br/>INSERT INTO messages_fts"]
    D -- 否 --> F["跳过 FTS 索引"]
    E --> G["前端刷新 unread_count:<br/>messages.id > last_read_message_id<br/>AND role = 'assistant'"]
    F --> G

```

### 消息顺序保持（TextBlock / ThinkingBlock）

LLM 流式输出中，模型可能在输出文本的中途发起工具调用。为保持 UI 展示顺序与模型实际输出顺序一致，Chat Engine 在遇到 `tool_call` 事件时将已累积的 text / thinking 内容 flush 为独立的 `TextBlock` / `ThinkingBlock` 消息，插入到 tool_call 消息之前。

### 未读追踪

使用水位线方案而非逐条标记，避免大量 UPDATE。`last_read_message_id` 记录用户最后阅读到的消息 ID，未读数通过子查询实时计算。

### 会话删除清理

`delete_session()` 除了 CASCADE 删除消息外，还清理：
- `~/.hope-agent/plans/{session_id}.md` — Plan 文件
- `~/.hope-agent/attachments/{session_id}/` — 附件目录

### FTS 索引容错

- 仅索引 `user` 和 `assistant` 角色的非空消息（tool/event 等不索引）
- 删除时 WHEN 条件与插入一致，避免删除未索引消息导致 FTS 损坏
- `open()` 时执行 `FTS rebuild` 修复可能的已有损坏
- `delete_session()` 失败时尝试 rebuild FTS 后重试

### Channel 集成

会话列表通过 `LEFT JOIN channel_conversations` 获取 IM Channel 关联信息，前端据此展示渠道标识（Telegram / WeChat 等图标和发送者名称）。

### auto_title 实现

`auto_title()`（[helpers.rs:9](../../crates/ha-core/src/session/helpers.rs#L9)）从首条用户消息生成 fallback 标题：取第一行，≤50 字符直接使用，超过则截断到 47 字符 + `"..."`。使用字符计数（非字节长度）正确处理 CJK 和 emoji。`ensure_first_message_title` 在首消息插入后立即调用并写 `title_source = 'first_message'`，LLM 起标题路径再决定是否覆盖（见上方"自动会话标题"）。

## 关联文档

- [Chat Engine](chat-engine.md) — 对话引擎，Session 的主要调用方
- [Plan Mode](plan-mode.md) — Plan Mode 状态管理与步骤持久化
- [Subagent](subagent.md) — 子 Agent 系统，使用 subagent_runs 表

## 文件清单

| 文件 | 职责 |
|---|---|
| `crates/ha-core/src/session/mod.rs` | 模块声明和 re-export |
| `crates/ha-core/src/session/types.rs` | SessionMeta, SessionMessage, MessageRole, NewMessage 类型定义 |
| `crates/ha-core/src/session/db.rs` | SessionDB 核心实现（open, CRUD, FTS, 已读, Migration, ProjectFilter, SessionTypeFilter） |
| `crates/ha-core/src/session/helpers.rs` | auto_title / ensure_first_message_title / effective_session_working_dir / is_session_incognito / cleanup_orphan_incognito 等辅助函数 |
| `crates/ha-core/src/session/subagent_db.rs` | Subagent 运行记录 CRUD |
| `crates/ha-core/src/session/acp_db.rs` | ACP 运行记录 CRUD |
| `crates/ha-core/src/session_title.rs` | 自动会话标题：常量 / SessionTitleConfig / `maybe_schedule_after_success` LLM 调度 |
| `src-tauri/src/commands/session.rs` | Tauri 命令层 |
| `crates/ha-server/src/routes/sessions.rs` | HTTP 路由层（REST API） |
