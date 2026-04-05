# OpenComputer 前后端分离架构设计

## Context

OpenComputer 当前以 Tauri 2 桌面应用为唯一形态，后端核心能力与 Tauri 框架存在耦合。目标是将后端解耦为独立的常驻服务，支持：

1. **独立 HTTP/WS 服务** — 无 GUI 也能运行，作为 daemon/service
2. **Tauri 内嵌模式** — 桌面端自动启动本地 HTTP 服务，开箱即用
3. **Tauri 外连模式** — 桌面端作为纯客户端连接远程服务
4. **终端交互** — 通过已有的 ACP stdio 或 HTTP API
5. **Web 前端** — React 前端可作为独立 Web 应用运行
6. **IM 渠道** — 在独立服务模式下正常运行

### 现状评估

**已 Tauri 无关（70-80%）**：agent/, memory/, session/, chat_engine/(引擎本身), provider/, paths.rs, user_config.rs, plan/, cron/, skills/, subagent/, acp/

**耦合点**：
- `globals.rs:18` — `APP_HANDLE: OnceLock<tauri::AppHandle>` 全局单例，~20 处使用
- `commands/` — 172 个 `#[tauri::command]`，19 文件 ~3,951 LOC
- `chat_engine/types.rs:31-73` — `ChannelSink` 和 `ChannelStreamSink` 包装 Tauri IPC
- ~10 个工具文件 — `handle.emit()` 用于 approval/notification/plan 等事件
- `channel/worker/dispatcher.rs:3-4` — `tauri::Emitter` 用于 IM 流式事件
- `app_init.rs:117,175` — 仅 2 处 Tauri 引用（欢迎通知 + spawn）
- 前端 151 个 `invoke()` 调用

---

## 1. Cargo Workspace 结构

```
OpenComputer/
  Cargo.toml                    # [workspace] manifest
  crates/
    oc-core/                    # 零 tauri/axum 依赖，纯业务逻辑
      Cargo.toml
      src/
        lib.rs                  # 模块注册 + re-exports
        state.rs                # ServiceRegistry (DI container)
        event_bus.rs            # EventBus trait + BroadcastEventBus
        init.rs                 # init_service_registry()（从 app_init.rs 迁入）
        agent/                  # ← src-tauri/src/agent/
        chat_engine/            # ← EventSink trait 留此，ChannelSink 移走
        session/                # ← src-tauri/src/session/
        memory/                 # ← src-tauri/src/memory/
        provider/               # ← src-tauri/src/provider/
        tools/                  # ← src-tauri/src/tools/
        channel/                # ← src-tauri/src/channel/
        plan/                   # ← src-tauri/src/plan/
        cron/                   # ← src-tauri/src/cron/
        skills/                 # ← src-tauri/src/skills/
        subagent/               # ← src-tauri/src/subagent/
        acp/                    # ← src-tauri/src/acp/
        acp_control/            # ← src-tauri/src/acp_control/
        system_prompt/          # ← src-tauri/src/system_prompt/
        context_compact/        # ← src-tauri/src/context_compact/
        sandbox.rs              # ← src-tauri/src/sandbox.rs
        docker/                 # ← src-tauri/src/docker/
        dashboard/              # ← src-tauri/src/dashboard/
        logging/                # ← src-tauri/src/logging/
        paths.rs                # ← 不变
        user_config.rs          # ← 不变
        failover.rs             # ← 不变
        oauth.rs                # ← src-tauri/src/oauth.rs
        weather.rs              # ← src-tauri/src/weather.rs
        backup.rs               # ← src-tauri/src/backup.rs
        slash_commands/          # ← src-tauri/src/slash_commands/

    oc-server/                  # HTTP/WebSocket 服务（依赖 oc-core + axum）
      Cargo.toml
      src/
        lib.rs                  # build_router() + start_server()
        config.rs               # ServerConfig (bind_addr, auth, cors)
        auth.rs                 # API Key / Bearer token 认证中间件
        error.rs                # 统一错误类型 → HTTP 状态码
        routes/
          mod.rs
          chat.rs               # POST /api/chat, POST /api/chat/stop
          sessions.rs           # CRUD /api/sessions/*
          providers.rs          # CRUD /api/providers/*
          memory.rs             # /api/memory/*
          config.rs             # /api/config/*
          agents.rs             # /api/agents/*
          skills.rs             # /api/skills/*
          cron.rs               # /api/cron/*
          plan.rs               # /api/plan/*
          channels.rs           # /api/channels/*
          dashboard.rs          # /api/dashboard/*
          logging.rs            # /api/logs/*
          auth.rs               # /api/auth/*
          misc.rs               # /api/misc/*
        ws/
          mod.rs
          chat_stream.rs        # WS /ws/chat/{session_id} — 流式 LLM 输出
          events.rs             # WS /ws/events — 全局事件推送
          sink.rs               # WebSocketSink: impl EventSink

    oc-tauri/                   # 桌面端（依赖 oc-core + oc-server + tauri）
      Cargo.toml                # 包含 tauri 及所有 tauri-plugin-*
      tauri.conf.json
      src/
        lib.rs                  # tauri::Builder + invoke_handler（薄封装）
        main.rs                 # Guardian / Child / ACP / Server 入口
        setup.rs                # app_setup（window, tray, shortcuts）
        commands.rs             # 薄适配器：#[tauri::command] → core service
        event_bridge.rs         # EventBus subscriber → tauri handle.emit()
        sink_adapters.rs        # TauriChannelSink: impl EventSink
        tray.rs
        shortcuts.rs
        permissions.rs          # macOS 权限检查
```

### 依赖关系

```
oc-tauri  ──→  oc-server  ──→  oc-core
    │                              ↑
    └──────────────────────────────┘
            (也直接依赖)
```

**铁律**：`oc-core` 的 Cargo.toml 禁止出现 `tauri` 或 `axum` 依赖。

---

## 2. ServiceRegistry — 替代全局单例

### 当前问题

`globals.rs` 使用 11 个 `OnceLock` 静态变量 + `AppState` 结构体，其中 `APP_HANDLE` 是唯一 Tauri 耦合点。另外 `agent: Mutex<Option<AssistantAgent>>` 和 `chat_cancel: Arc<AtomicBool>` 是全局单例，不支持多客户端并发对话。

### 设计

```rust
// oc-core/src/state.rs

/// Per-session active state — supports concurrent conversations across clients.
pub struct SessionState {
    pub agent: AssistantAgent,
    pub cancel: Arc<AtomicBool>,
    /// Per-session broadcast for streaming events. Multiple WS clients can subscribe
    /// to the same session to observe real-time LLM output.
    pub stream_bus: broadcast::Sender<String>,
    pub last_active: Instant,
}

pub struct ServiceRegistry {
    // ── 数据库 ──
    pub session_db: Arc<SessionDB>,
    pub log_db: Arc<LogDB>,
    pub cron_db: Arc<CronDB>,

    // ── 子系统 ──
    pub memory_backend: Arc<dyn MemoryBackend>,
    pub logger: AppLogger,
    pub subagent_cancels: Arc<SubagentCancelRegistry>,
    pub channel_registry: Arc<ChannelRegistry>,
    pub channel_db: Arc<ChannelDB>,
    pub channel_cancels: Arc<ChannelCancelRegistry>,
    pub acp_manager: Option<Arc<AcpSessionManager>>,

    // ── 多会话并发状态 ──
    pub active_sessions: Mutex<HashMap<String, SessionState>>,
    pub provider_store: Mutex<ProviderStore>,
    pub reasoning_effort: Mutex<String>,
    pub codex_token: Mutex<Option<(String, String)>>,
    pub current_agent_id: Mutex<String>,
    pub auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,

    // ── 事件总线（全局广播，多客户端同步） ──
    pub event_bus: Arc<dyn EventBus>,

    // ── 审批系统 ──
    pub approval_state: ApprovalState,
}
```

**多客户端并发模型**：
- 全局 `event_bus` 使用 `broadcast::channel`，每个客户端 WS 连接调用 `subscribe()` 获得独立 Receiver
- Per-session `stream_bus` 允许多个客户端订阅同一会话的流式输出（如桌面端发起对话，Web 端实时旁观）
- `chat_cancel` 从全局单例改为 per-session `SessionState.cancel`
- Session 过期清理：30 分钟无活动释放 agent 实例，释放内存

`init_service_registry()` 从 `app_init.rs:init_app_state()` 迁入，去掉 2 处 Tauri 引用（L117 欢迎通知移到 oc-tauri setup，L175 `tauri::async_runtime::spawn` 改 `tokio::spawn`）。

### 全局访问器迁移

当前工具代码通过 `crate::get_session_db()` 等函数访问全局。迁移后：
- 高频路径（tools）：通过扩展 `ToolExecContext` 注入所需依赖
- 低频路径（channel worker）：通过 `Arc<ServiceRegistry>` 传递

---

## 3. EventBus — 替代 APP_HANDLE 事件发射

### Trait 定义

```rust
// oc-core/src/event_bus.rs

#[derive(Debug, Clone, Serialize)]
pub struct AppEvent {
    pub name: String,
    pub payload: serde_json::Value,
}

pub trait EventBus: Send + Sync + 'static {
    fn emit(&self, name: &str, payload: serde_json::Value);
    fn subscribe(&self) -> broadcast::Receiver<AppEvent>;
}

pub struct BroadcastEventBus {
    tx: broadcast::Sender<AppEvent>,
}
```

### 事件清单（从代码中梳理）

| 事件名 | 来源文件 | 用途 |
|--------|---------|------|
| `approval_required` | tools/approval.rs | 命令需要用户审批 |
| `agent:send_notification` | tools/notification.rs, app_init.rs | 桌面通知 |
| `channel:stream_delta` | chat_engine/types.rs:63 | IM 流式 token |
| `channel:message_update` | channel/worker/dispatcher.rs | IM 会话新消息 |
| `channel:stream_start/end` | channel/worker/dispatcher.rs | IM 流式状态 |
| `subagent_event` | subagent/helpers.rs | 子 Agent 生命周期 |
| `parent_agent_stream` | subagent/helpers.rs | 子 Agent 结果注入 |
| `core_memory_updated` | tools/memory.rs | 记忆变更 |
| `plan_question_request` | tools/plan_question.rs | Plan 向用户提问 |
| `plan_mode_changed` | commands/plan.rs | Plan 状态变更 |
| `cron:run_completed` | cron/executor.rs | 定时任务完成 |
| `acp_control_event` | acp_control/events.rs | ACP 生命周期/流式 |
| `slash:*` | channel/worker/slash.rs | 斜杠命令结果 |

### 迁移方式（机械替换）

```rust
// Before（tools/notification.rs）:
if let Some(handle) = crate::get_app_handle() {
    use tauri::Emitter;
    let _ = handle.emit("agent:send_notification", payload);
}

// After:
registry.event_bus.emit("agent:send_notification", payload);
```

### Tauri 桥接

```rust
// oc-tauri/src/event_bridge.rs
pub async fn bridge_events_to_tauri(
    mut rx: broadcast::Receiver<AppEvent>,
    handle: tauri::AppHandle,
) {
    while let Ok(event) = rx.recv().await {
        let _ = handle.emit(&event.name, event.payload);
    }
}
```

---

## 4. EventSink 拆分

### 当前状态

`chat_engine/types.rs` 定义了 `EventSink` trait（纯 Rust）和两个实现：
- `ChannelSink` — 包装 `tauri::ipc::Channel<String>`
- `ChannelStreamSink` — 通过 `get_app_handle().emit()` + mpsc 转发

### 拆分方案

**oc-core 保留**：`EventSink` trait + `ChannelStreamSink`（去掉 L61-70 的 emit 调用，改为通过 EventBus）

**oc-server 新增**：`WebSocketSink`
```rust
pub struct WebSocketSink {
    tx: mpsc::Sender<String>,
}
impl EventSink for WebSocketSink {
    fn send(&self, event: &str) {
        let _ = self.tx.try_send(event.to_string());
    }
}
```

**oc-tauri 迁入**：`TauriChannelSink`
```rust
pub struct TauriChannelSink {
    pub channel: tauri::ipc::Channel<String>,
}
impl EventSink for TauriChannelSink {
    fn send(&self, event: &str) {
        let _ = self.channel.send(event.to_string());
    }
}
```

---

## 5. HTTP/WebSocket API 设计

### 端点规范

```
# ── 聊天 ──
POST   /api/chat                        # 发起对话（事件通过 WS 推送）
POST   /api/chat/stop                   # 停止当前对话
POST   /api/chat/approval/{request_id}  # 审批响应
POST   /api/chat/attachment             # 上传附件（multipart）
GET    /api/chat/system-prompt          # 获取系统提示词
GET    /api/chat/tools                  # 获取工具列表

# ── 会话 ──
POST   /api/sessions                    # 创建会话
GET    /api/sessions                    # 列表（?agent_id=&limit=&offset=）
GET    /api/sessions/{id}               # 获取会话详情
DELETE /api/sessions/{id}               # 删除会话
PATCH  /api/sessions/{id}               # 重命名
GET    /api/sessions/{id}/messages      # 加载消息

# ── Provider ──
GET    /api/providers                   # 列表
POST   /api/providers                   # 添加
PUT    /api/providers/{id}              # 更新
DELETE /api/providers/{id}              # 删除
POST   /api/providers/test              # 测试连接
GET    /api/providers/active-model      # 当前活跃模型

# ── 记忆 ──
POST   /api/memory                      # 添加
PUT    /api/memory/{id}                 # 更新
DELETE /api/memory/{id}                 # 删除
GET    /api/memory                      # 列表
POST   /api/memory/search              # 语义搜索

# ── 配置 ──
GET/PUT /api/config/{section}           # 各子配置（web-search, proxy, compact...）

# ── Agent ──
GET    /api/agents                      # 列表
GET    /api/agents/{id}                 # 获取配置
PUT    /api/agents/{id}                 # 保存配置

# ── Skills / Cron / Plan / Dashboard / Channel / Logging / Auth ──
# 同样按 RESTful 映射，省略详细列表

# ── WebSocket ──
WS     /ws/chat/{session_id}            # 每会话流式推送（LLM token 输出）
WS     /ws/events                       # 全局事件推送（approval, channel 更新等）
```

### 流式协议

WebSocket 传输的 JSON 事件格式与现有前端 `useStreamEventHandler.ts` 解析的格式完全一致：

```json
{"type": "text", "text": "Hello"}
{"type": "thinking", "text": "..."}
{"type": "tool_call_start", "toolCallId": "tc_1", "name": "exec", "arguments": "..."}
{"type": "tool_call_end", "toolCallId": "tc_1", "result": "..."}
{"type": "usage", "input_tokens": 1234, "output_tokens": 567}
{"type": "done"}
```

### 服务入口

```rust
// oc-server/src/lib.rs
pub async fn start_server(config: ServerConfig, registry: Arc<ServiceRegistry>) -> Result<()> {
    let app = build_router(registry, &config);
    let listener = TcpListener::bind(&config.bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

---

## 6. 前端 Transport 抽象层

### 接口定义

```typescript
// src/lib/transport.ts
export interface Transport {
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  openChatStream(sessionId: string, onEvent: (event: string) => void): ChatStream;
  listen(eventName: string, handler: (payload: unknown) => void): () => void;
}
```

### 环境自动检测

```typescript
// src/lib/transport-provider.ts
export function getTransport(): Transport {
  if (window.__TAURI_INTERNALS__) {
    return new TauriTransport();
  }
  const serverUrl = import.meta.env.VITE_SERVER_URL || "http://localhost:8420";
  return new HttpTransport(serverUrl);
}
```

### 迁移方式

所有 151 个 `invoke()` 调用统一替换：

```typescript
// Before:
import { invoke } from "@tauri-apps/api/core";
const result = await invoke("list_sessions_cmd", { agentId });

// After:
import { getTransport } from "@/lib/transport-provider";
const result = await getTransport().call("list_sessions_cmd", { agentId });
```

`HttpTransport.call()` 内部维护命令名到 REST 端点的映射表。

### ChatStream 迁移

```typescript
// Before (useChatStream.ts):
const onEvent = new Channel<string>();
onEvent.onmessage = (raw) => handleStreamEvent(JSON.parse(raw));
await invoke("chat", { message, onEvent, ... });

// After:
const transport = getTransport();
const stream = transport.openChatStream(sessionId, (raw) => {
  handleStreamEvent(JSON.parse(raw));
});
await transport.call("chat", { message, sessionId, ... });
```

---

## 7. 多入口模式 + 统一保活

### main.rs 入口

```
opencomputer                           # 桌面端（Guardian → Tauri + 内嵌 HTTP）
opencomputer server                    # HTTP 服务（Guardian → HTTP daemon）
opencomputer server --bind 0.0.0.0:8420 --api-key KEY
opencomputer server --no-guardian      # 关闭 Guardian（交给 systemd/launchd）
opencomputer server install            # 注册为系统服务（macOS launchd / Linux systemd）
opencomputer server uninstall          # 卸载系统服务
opencomputer server status             # 检查服务状态
opencomputer server stop               # 停止服务
opencomputer acp                       # 已有的 stdio JSON-RPC
```

### Guardian 统一保活

Guardian 从 Tauri 专属提取为 `oc-core/src/guardian.rs` 通用模块，桌面端和服务端共用。

**退出码约定**：

| Exit Code | 含义 | Guardian 行为 |
|-----------|------|--------------|
| 0 | 用户主动退出（关窗口 / stop 命令） | 不重启 |
| 42 | 请求重启（self-fix 后） | 立即重启，不计入 crash |
| 1 | 一般错误 | 重启 + 记录 crash |
| 128+N | 信号杀死 (SIGSEGV/SIGKILL 等) | 重启 + 记录 crash |

**SIGINT/SIGTERM**：signal handler 设置 `should_exit` flag → 优雅退出 → Guardian 不重启。

**异常场景全覆盖**：
- panic → `catch_unwind` → 非 0 exit → Guardian 重启
- OOM 被杀 → SIGKILL → exit 137 → Guardian 重启
- segfault → SIGSEGV → exit 139 → Guardian 重启
- 连续崩溃 → 到阈值 → backup + self-diagnosis + auto-fix → 重启
- 达到 max crashes → Guardian 退出（如有 systemd/launchd 还会被再次拉起）

### 系统服务注册

**macOS** — `~/Library/LaunchAgents/com.opencomputer.server.plist`：
- `KeepAlive = true` + `RunAtLoad = true`
- 使用 `--no-guardian` 避免双层重启

**Linux** — `~/.config/systemd/user/opencomputer.service`：
- `Restart=on-failure` + `RestartSec=3`

**双保险**：平台负责进程保活，Guardian 负责应用级自修复（crash journal + diagnosis）。

### PID 文件 + 健康检查

- `~/.opencomputer/server.pid` 写入进程 PID
- `GET /api/health` → `{"status": "ok", "uptime": ..., "version": "..."}`
- `opencomputer server status` 读 PID + HTTP 健康检查
- `opencomputer server stop` 读 PID 发 SIGTERM

### Tauri 内嵌模式

Tauri app_setup() 中：
1. 构建 `ServiceRegistry`（纯 Rust）
2. 启动 EventBus → Tauri 桥接
3. 启动内嵌 HTTP 服务（localhost 随机端口）
4. 前端通过 Tauri IPC 或 HTTP 访问均可

### Tauri 外连模式

用户在设置中配置远程服务地址。前端 Transport 切换到 HttpTransport，本地不启动核心服务。

---

## 8. ToolExecContext 扩展

```rust
// oc-core/src/tools/execution.rs
pub struct ToolExecContext {
    // 现有字段保留
    pub context_window_tokens: Option<u32>,
    pub used_tokens: Option<u32>,
    pub home_dir: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub subagent_depth: u32,
    pub require_approval: Vec<String>,
    pub force_sandbox: bool,
    pub plan_mode_allow_paths: Vec<String>,
    pub plan_mode_allowed_tools: Vec<String>,

    // 新增：注入的依赖
    pub session_db: Arc<SessionDB>,
    pub memory_backend: Arc<dyn MemoryBackend>,
    pub cron_db: Arc<CronDB>,
    pub logger: AppLogger,
    pub event_bus: Arc<dyn EventBus>,
    pub subagent_cancels: Arc<SubagentCancelRegistry>,
}
```

替代工具代码中的 `crate::get_session_db()` 等全局访问。

---

## 9. 验证方案

1. **Tauri 桌面端回归测试** — `npm run tauri dev`，验证 172 个命令全部正常、流式输出正常、IM 渠道正常
2. **独立服务测试** — `opencomputer server`，用 curl/Postman 测试 REST API，用 wscat 测试 WebSocket 流式
3. **Web 前端测试** — `npm run dev` + `VITE_SERVER_URL=http://localhost:8420`，验证所有功能通过 HTTP/WS 正常工作
4. **IM 渠道测试** — 在独立服务模式下启动 Telegram 渠道，验证消息收发
5. **编译检查** — `cargo check -p oc-core` 确认零 tauri 依赖
