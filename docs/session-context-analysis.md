# 会话上下文内存与落盘处理逻辑分析

## 整体架构

```
┌──────────────────────────────────────────────────────┐
│  前端 React State (messages[])                        │
│  - 仅用于 UI 显示                                     │
│  - 只保留 user + assistant 消息                       │
│  - 不参与 LLM API 调用                                │
└────────────┬─────────────────────────────────────────┘
             │ invoke("chat", { message, sessionId })
             ↓
┌──────────────────────────────────────────────────────┐
│  后端 Agent 内存 (conversation_history)               │
│  - Vec<serde_json::Value>                            │
│  - 包含完整上下文：user + assistant + tool_call/result│
│  - 每次 API 调用时发送全部历史                        │
└────────────┬─────────────────────────────────────────┘
             │
             ↓
┌──────────────────────────────────────────────────────┐
│  SQLite 持久化 (SessionDB)                            │
│  - sessions 表 + messages 表                          │
│  - 逐条 append：user / assistant / tool / system      │
└──────────────────────────────────────────────────────┘
```

---

## 1. 内存中的对话历史

**文件**: `src-tauri/src/agent.rs:273-281`

```rust
pub struct AssistantAgent {
    provider: LlmProvider,
    user_agent: String,
    thinking_style: ThinkingStyle,
    conversation_history: std::sync::Mutex<Vec<serde_json::Value>>,
}
```

### 关键行为

- **初始化为空** — 每次 `new()` 时 `conversation_history: std::sync::Mutex::new(Vec::new())`
- **每次 `chat()` 调用流程**：
  1. clone 当前历史
  2. 追加新用户消息
  3. 发送**完整历史**给 LLM API
  4. tool loop 中继续追加 assistant 响应 + tool_call + tool_result
  5. 循环结束后**写回**整个数组

以 Anthropic Provider 为例（`agent.rs:582`）：

```rust
// 1. 读取历史
let mut messages = self.conversation_history.lock().unwrap().clone();

// 2. 追加用户消息
messages.push(json!({ "role": "user", "content": user_content }));

// 3. 发送完整 messages 到 API
let body = json!({
    "model": model,
    "messages": messages,  // ← 完整历史
    "stream": true,
    ...
});

// 4. tool loop 中追加更多消息（assistant 回复 + tool result）
// ... 省略 ...

// 5. 写回
*self.conversation_history.lock().unwrap() = messages;
```

OpenAI Chat / OpenAI Responses / Codex 三种 Provider 同理，都遵循相同模式。

**结论**：在同一个 Agent 实例的生命周期内，对话历史是完整累积的，LLM API 收到的是全部消息。

---

## 2. SQLite 落盘逻辑

### 2.1 数据库结构（`src-tauri/src/session.rs`）

**sessions 表**：

| 字段 | 类型 | 说明 |
|------|------|------|
| id | TEXT PK | UUID |
| title | TEXT | 会话标题（首条消息自动生成） |
| agent_id | TEXT | 所属 Agent |
| provider_name | TEXT | 当前使用的 Provider |
| model_id | TEXT | 当前使用的模型 |
| created_at | TEXT | 创建时间 |
| updated_at | TEXT | 最后更新时间 |

**messages 表**：

| 字段 | 类型 | 说明 |
|------|------|------|
| id | INTEGER PK | 自增 |
| session_id | TEXT FK | 所属会话 |
| role | TEXT | user / assistant / system / tool |
| content | TEXT | 消息内容 |
| timestamp | TEXT | 时间戳 |
| attachments_meta | TEXT | 附件元数据 JSON |
| model_id | TEXT | 生成该消息的模型 |
| input_tokens | INTEGER | 输入 token 数 |
| output_tokens | INTEGER | 输出 token 数 |
| tool_call_id | TEXT | 工具调用 ID |
| tool_name | TEXT | 工具名称 |
| tool_arguments | TEXT | 工具参数 |
| tool_result | TEXT | 工具执行结果 |
| tool_is_error | BOOLEAN | 工具是否出错 |

### 2.2 落盘时序（`src-tauri/src/lib.rs` chat 命令）

```
用户发送消息
    │
    ├─ [1] 保存用户消息到 DB（lib.rs:985-987）
    │      let mut user_msg = session::NewMessage::user(&message);
    │      user_msg.attachments_meta = attachments_meta;
    │      db.append_message(&sid, &user_msg);
    │
    ├─ [2] 调用 agent.chat()，流式处理中...
    │      │
    │      ├─ 工具调用结果 → persist_tool_event()（lib.rs:1154-1170）
    │      │   拦截 "tool_result" 类型事件，保存到 DB
    │      │
    │      └─ 模型降级事件 → 保存 system 消息（lib.rs:1085）
    │
    └─ [3] 保存助手最终回复到 DB（lib.rs:1109）
           db.append_message(&sid, &session::NewMessage::assistant(&result));
```

### 2.3 persist_tool_event 函数细节

```rust
fn persist_tool_event(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("tool_result") => {
                let call_id = event.get("call_id")...;
                let result = event.get("result")...;
                let tool_msg = session::NewMessage::tool(call_id, "", "", result, None, false);
                let _ = db.append_message(session_id, &tool_msg);
            }
            _ => {} // 其他事件不持久化
        }
    }
}
```

**注意**：只持久化了 `tool_result`，没有持久化 `tool_call`（工具调用请求本身）。这在后续恢复历史时可能造成数据不完整。

---

## 3. 前端消息管理

### 3.1 发送消息（`src/components/ChatScreen.tsx:234-341`）

```typescript
async function handleSend() {
    // 1. 将用户消息添加到 UI state
    setMessages((prev) => [...prev, { role: "user", content: text }])

    // 2. 添加空的 assistant 消息占位
    setMessages((prev) => [...prev, { role: "assistant", content: "" }])

    // 3. 创建 Channel 监听流式事件
    const onEvent = new Channel<string>()
    onEvent.onmessage = (raw) => {
        // 处理 text_delta / tool_call / tool_result / session_created / model_fallback
    }

    // 4. 调用后端，只传当前消息（不传历史）
    await invoke<string>("chat", {
        message: text,
        attachments,
        sessionId: currentSessionId,  // 会话 ID
        onEvent
    })
}
```

**关键**：前端**只传当前消息**给后端，不传历史。历史的管理完全依赖后端 Agent 实例的 `conversation_history`。

### 3.2 切换会话（`src/components/ChatScreen.tsx:149-172`）

```typescript
async function handleSwitchSession(sessionId: string) {
    const msgs = await invoke<SessionMessage[]>("load_session_messages_cmd", { sessionId })
    const displayMessages: Message[] = []
    for (const msg of msgs) {
        if (msg.role === "user") {
            displayMessages.push({ role: "user", content: msg.content })
        } else if (msg.role === "assistant") {
            displayMessages.push({ role: "assistant", content: msg.content })
        }
    }
    setMessages(displayMessages)      // 仅用于 UI 显示
    setCurrentSessionId(sessionId)
    // ⚠️ 没有通知后端重建 Agent 的 conversation_history
}
```

### 3.3 新建会话（`src/components/ChatScreen.tsx:175-183`）

```typescript
async function handleNewChat(agentId: string) {
    setMessages([])               // 清空 UI
    setCurrentSessionId(null)     // 清空 session ID
    setCurrentAgentId(agentId)
    // ⚠️ 同样没有通知后端重置 Agent
}
```

---

## 4. 核心问题分析

### 问题 1：每次 chat 都可能创建新 Agent，历史丢失

`lib.rs:1048-1055` 中，当 `model_chain` 非空时：

```rust
for (idx, model_ref) in model_chain.iter().enumerate() {
    let agent = match build_agent_for_model(model_ref, &state).await {
        Some(a) => a,    // ← 全新实例，conversation_history 为空
        None => { continue; }
    };
    // ...
    match agent.chat(&message, ...).await {
        Ok(result) => {
            *state.agent.lock().await = Some(agent);  // 缓存到全局
            return Ok(result);
        }
        // ...
    }
}
```

**每次 `chat` 调用都新建 Agent** → `conversation_history` 为空 → **LLM 只收到当前这一条消息**。

只有在 `model_chain` 为空的 fallback 路径（`lib.rs:1019-1038`）才复用缓存的 Agent，此时历史是连续的。

### 问题 2：会话切换时后端没有同步

前端切换会话时只做了 UI 层面的操作，**后端 Agent 的 `conversation_history` 不会被重置或重建**。

可能出现的场景：

```
1. 用户在会话 A 中聊了 5 轮 → Agent 积累了 A 的历史
2. 用户切换到会话 B → 前端显示 B 的消息
3. 用户在会话 B 中发消息 → 后端:
   - model_chain 非空 → 新建 Agent → 只有这 1 条消息（丢失 B 的历史）
   - model_chain 为空 → 复用 Agent → 携带会话 A 的历史（上下文污染）
```

### 问题 3：DB 中的历史无法完整重建为 API 格式

即使我们想从 DB 恢复历史，当前的 `messages` 表存储格式和 LLM API 需要的格式之间存在差异：

- `tool_call`（assistant 发起的工具调用请求）**没有被持久化**到 DB
- 只持久化了 `tool_result`（工具执行结果）
- Anthropic API 要求 assistant 消息中包含 `tool_use` block，然后紧跟 `tool_result` 消息
- 缺少 `tool_call` 信息会导致无法正确重建 API 所需的消息格式

---

## 5. 消息流向总结

```
                    前端                              后端
                ┌──────────┐                    ┌──────────────┐
  用户输入 ──→  │ messages  │ ──invoke("chat")→ │ chat 命令     │
                │ state[]  │    (只传当前消息)   │              │
                └──────────┘                    │  ┌─────────┐ │
                     ↑                          │  │ Agent   │ │
                     │                          │  │ history │ │
                  UI 渲染                       │  └────┬────┘ │
                     │                          │       │      │
                ┌──────────┐                    │       ↓      │
                │ 流式事件  │ ←── Channel ────── │  LLM API    │
                │ text_delta│                    │              │
                │ tool_call │                    │  ┌────────┐ │
                │ tool_result                    │  │ SQLite │ │
                └──────────┘                    │  │ 持久化 │ │
                                                │  └────────┘ │
                                                └──────────────┘

  切换会话时：
  前端 ──invoke("load_session_messages_cmd")→ 后端 SQLite
       ←── 返回消息列表 ──────────────────────┘
       (仅用于 UI 显示，Agent history 没有同步)
```

---

## 6. 修复方向建议

### 方案 A：在 chat 命令中从 DB 恢复历史（推荐）

1. **完善 DB 存储**：在 `persist_tool_event` 中也保存 `tool_call` 事件，或在 assistant 消息中内嵌 tool_call 信息
2. **新增 `session.rs` 方法**：`load_messages_for_api(session_id, provider_type) -> Vec<Value>`，将 DB 中的消息转换为特定 Provider 格式的 messages 数组
3. **在 `chat` 命令中注入**：创建 Agent 后、调用 `agent.chat()` 前，从 DB 加载历史并设置到 `conversation_history`
4. **给 Agent 加方法**：`set_conversation_history(messages: Vec<Value>)`

### 方案 B：前端传递历史（简单但有局限）

1. 前端维护完整的消息历史（包括 tool 消息）
2. `chat` 命令接受 `messages` 参数而非单条 `message`
3. **局限**：tool_call/tool_result 的格式在前端难以准确维护

### 方案 C：Agent 实例按 session 缓存

1. 维护 `HashMap<SessionId, AssistantAgent>` 而非单个 Agent
2. 切换会话时自动切换 Agent 实例
3. 新会话创建新实例，已有会话复用（保持历史连续）
4. **局限**：应用重启后内存中的历史仍然丢失，仍需要 DB 恢复机制

### 推荐组合

**方案 A + C**：
- 用方案 C 避免频繁重建（性能优化）
- 用方案 A 作为兜底，应用重启后从 DB 恢复
- 确保 DB 存储的消息格式足够完整，可以还原为任意 Provider 的 API 格式

---

## 7. 涉及文件清单

| 文件 | 关注点 |
|------|--------|
| `src-tauri/src/agent.rs` | `conversation_history` 的读写逻辑，各 Provider 的消息构建 |
| `src-tauri/src/lib.rs` | `chat` 命令中 Agent 创建/复用逻辑，`persist_tool_event` |
| `src-tauri/src/session.rs` | SQLite 表结构、消息存取方法 |
| `src/components/ChatScreen.tsx` | 前端消息管理、会话切换、`handleSend` |
