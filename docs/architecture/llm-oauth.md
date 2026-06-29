# 主 LLM OAuth（ChatGPT / Codex 登录）

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/oauth.rs`](../../crates/ha-core/src/oauth.rs)、[`src-tauri/src/commands/auth.rs`](../../src-tauri/src/commands/auth.rs)、[`src-tauri/src/cli_auth.rs`](../../src-tauri/src/cli_auth.rs)、[`crates/ha-server/src/routes/auth.rs`](../../crates/ha-server/src/routes/auth.rs)

## 概述

主 LLM 的四种 Provider 里**唯一对接 OAuth 的是 Codex（ChatGPT 账号登录）**——其余 Anthropic / OpenAIChat / OpenAIResponses 全部走 API Key。本子系统负责 Codex 这一条 OAuth 链路：PKCE + S256 的 loopback 浏览器流程登录、token 落 `~/.hope-agent/credentials/auth.json`、请求前按需刷新；登出时各 shell（Tauri / HTTP / CLI）先 `delete_providers_by_api_type(Codex)` 删配置里的 Codex provider 行、再 `clear_token` 删 token 文件。

关键边界：**它与 MCP 客户端的 OAuth（[`mcp.md`](mcp.md)，模块 `mcp/oauth.rs`）完全独立、互不共用实现**。两者都是「OAuth 2.1 + PKCE」，但分属两套源码（`oauth.rs` vs `mcp/oauth.rs`）、两套凭据存储（`auth.json` 单文件 vs `credentials/mcp/{id}.json` 一服务器一文件），不要把任何一方的约定套到另一方。Provider 系统侧对 Codex 的鉴权（Bearer + `chatgpt-account-id` header、不参与 failover 轮换）见 [`provider-system.md`](provider-system.md)。

提供四类入口：

- **桌面登录**：Tauri 命令面（`start_codex_auth` → 浏览器授权 → `finalize_codex_auth`）
- **HTTP 登录**：headless server 镜像同一套流程（`/api/auth/codex/*`）
- **CLI 一次性登录**：`hope-agent auth codex login/status/logout` 子命令
- **请求前刷新**：chat / side-query 调用 LLM 前经 `ensure_fresh_codex_token` / `load_fresh_codex_token` 保证 access_token 新鲜

## 模块结构

核心全在 `crates/ha-core/src/oauth.rs`（零 Tauri 依赖）。薄壳分布：

| 文件 | 职责 |
|---|---|
| [`oauth.rs`](../../crates/ha-core/src/oauth.rs) | 子系统真相源：流程编排 / PKCE / token 读写 / 按需刷新 / 失败归类锚点 |
| [`commands/auth.rs`](../../src-tauri/src/commands/auth.rs) | Tauri 命令面（unmasked，桌面本机信任域） |
| [`routes/auth.rs`](../../crates/ha-server/src/routes/auth.rs) | HTTP 路由面（`/api/auth/codex/*`），进程级 `OnceLock<AuthResult>` slot |
| [`cli_auth.rs`](../../src-tauri/src/cli_auth.rs) | 终端子命令 `login_codex` / `run_codex_login` / `run_codex_logout`，新建独立 tokio runtime 跑流程 |
| [`cli_onboarding/`](../../src-tauri/src/cli_onboarding) | 首启引导（`steps/provider.rs`）经 `cli_auth::login_codex(CodexLoginOptions::default())` 复用 CLI 登录流程（默认 `open_browser=true`） |

## OAuth 端点与常量

端点 / 客户端常量在 `oauth.rs` 顶部硬编码；`REFRESH_MARGIN_MS` 单独定义在过期判定附近：

| 常量 | 值 |
|---|---|
| `AUTH_URL` | `https://auth.openai.com/oauth/authorize` |
| `TOKEN_URL` | `https://auth.openai.com/oauth/token` |
| `CLIENT_ID` | `app_EMoamEEZ73f0CkXaXp7hrann` |
| `SCOPES` | `openid profile email offline_access` |
| `REDIRECT_PORT` | `1455`（loopback 回调端口） |
| `REDIRECT_URI` | `http://localhost:1455/auth/callback` |
| `REFRESH_MARGIN_MS` | `30_000`（过期前 30s 视为「需刷新」） |

`originator=hope-agent` 不是命名常量，而是授权 URL `format!` 模板里的内联字面量。

授权 URL 固定带 `response_type=code` / `code_challenge_method=S256` / `id_token_add_organizations=true` / `codex_cli_simplified_flow=true` / `originator=hope-agent`。

## 核心数据结构

- **`TokenData`**（`oauth.rs`）：OAuth 凭据载体，serde 落 `auth.json`。字段 `access_token` + 可选 `refresh_token` / `expires_in` / `token_type` / `account_id` / `expires_at`。`expires_at` 是换 token 时由 `expires_in` 算出的绝对毫秒时间戳，是过期判定的真相源。
- **`AuthStatus`**（`oauth.rs`）：前端轮询用的认证状态，`authenticated: bool` + 可选 `error`；Tauri `check_auth_status` 与 HTTP `GET /auth/codex/status` 共用。
- **`JwtPayload` / `JwtAuth`**（`oauth.rs`）：解 JWT access_token 第二段（payload），取自定义 claim `https://api.openai.com/auth`（`JwtPayload`），内层 `JwtAuth` 承载 `chatgpt_account_id`。
- **`AuthResult`**（`routes/auth.rs`）：类型别名 `Arc<TokioMutex<Option<anyhow::Result<TokenData>>>>`，HTTP 侧 `start`/`finalize` 跨请求共享的进程级 `OnceLock` slot。
- **`SetCodexModelBody`**（`routes/auth.rs`）：`POST /auth/codex/models` 请求体（`model: String`）。

## 数据流：登录（PKCE + S256 loopback）

```
start_oauth_flow(open_browser=true)
  └─ start_oauth_flow_with_auth_url
       ├─ generate_code_verifier()    # 32 字节随机 → URL_SAFE_NO_PAD
       ├─ generate_code_challenge()   # SHA256(verifier) → base64url（S256）
       ├─ 拼 auth_url（带 challenge + state + originator）
       ├─ spawn_blocking → run_callback_server(state, verifier)
       │     ├─ tiny_http 绑 127.0.0.1:1455（loopback only）
       │     ├─ 校验 state 参数（CSRF），不符即拒
       │     ├─ 取 code、回 HTML 成功页
       │     ├─ exchange_code_for_token(code, verifier)
       │     │     └─ POST TOKEN_URL（grant_type=authorization_code + code_verifier）
       │     │          → 解 TokenData、extract_account_id 填 account_id、算 expires_at
       │     └─ 300s（5 分钟）超时
       ├─ 结果写入共享 Arc<Mutex<Option<Result<TokenData>>>>
       └─ open_browser=true 时打开系统浏览器
```

要点：

- **PKCE 编排**：`generate_code_verifier` 产 32 字节随机 verifier（`URL_SAFE_NO_PAD`），`generate_code_challenge` 对其 SHA256 后 base64url 作 challenge，URL 标 `code_challenge_method=S256`。换 token 时把 verifier 原文回传 `TOKEN_URL`。
- **`open_browser` 二态**：`start_oauth_flow` 是桌面 / HTTP 入口（`open_browser=true`，起 callback server 并打开浏览器）；`start_oauth_flow_with_auth_url(open_browser=false)` 返回 `auth_url` 供 CLI / headless onboarding 自行打印 URL（`cli_auth` + `cli_onboarding` 共用）。
- **回调 server**：`run_callback_server` 在 `spawn_blocking` 内用 `tiny_http` 绑 `127.0.0.1:1455`（注释明确「never exposed externally」），校验 `state`、取 `code`、回成功 HTML 页，再调 `exchange_code_for_token`；5 分钟无回调即超时返错。
- **account_id 提取**：`extract_account_id`（`pub`，被 Tauri / HTTP / CLI 三 shell 的 finalize/restore/status 复用）解 JWT access_token payload 取 `chatgpt_account_id`，作为后续请求的 `chatgpt-account-id` header 来源（见 [`provider-system.md`](provider-system.md)）。

## 数据流：过期判定与按需刷新

过期判定 `is_token_expired`：按 `expires_at` 减 `REFRESH_MARGIN_MS`（30s margin）判过期；**无 `expires_at` 视为有效**（不强制刷新）。

两条按需刷新路径，均在 LLM 请求前调用（调用方上下文见 [`chat-engine.md`](chat-engine.md) / [`side-query.md`](side-query.md)）：

- **`load_fresh_codex_token() -> (access_token, account_id)`**：读盘；未过期直接返回；过期则用 `refresh_token` 调 `refresh_access_token` 刷新。**失败归类契约（红线）**：其错误消息必须内嵌字面量 `authentication`，否则 [`failover`](failover.md) 的 `classify_error` 不会归到 `FailoverReason::Auth`（`oauth.rs` 内有单测锁此）。
- **`ensure_fresh_codex_token(current_access_token) -> Option<(access_token, account_id)>`**：对比内存中的 `current_access_token` 与磁盘 token——盘上已被别的进程刷新（未过期但与内存不一致）则**直接采纳磁盘 token**（不发 HTTP）、临近/已过期则 HTTP `refresh_access_token` 刷新，二者都返回新值；内存值已是最新未过期则返 `None`（沿用内存值）。用于 chat turn 中途避免重复刷新。
- **`refresh_access_token(refresh_token) -> TokenData`**：底层 POST `TOKEN_URL`（`grant_type=refresh_token`），刷新成功后 `save_token` 落盘。

## 数据流：登出（破坏性）

登出由三 shell 编排（Tauri `logout_codex` / HTTP `logout_codex` / CLI `run_codex_logout`），两步：

1. `delete_providers_by_api_type(Codex)`——删 `config.json` 里的 Codex provider 行（token 文件与 provider 行是两套存储，重新登录会经 `ensure_codex_provider_persisted` 重建）。**此步在 shell 里调，不在 `clear_token` 内**
2. `clear_token`——删 `auth.json` 文件 + `fire_session_end("", "logout")` 发一次 SessionEnd hook（app-global 代表事件，非 per-session fan-out）

注：`clear_token` 自身只做「删 token 文件 + fire SessionEnd」；provider 行删除是各 shell 在 `clear_token` 之外另调的。

## 持久化

| 存储 | 内容 |
|---|---|
| `~/.hope-agent/credentials/auth.json` | **唯一 token 持久化文件**，路径 `paths.rs::auth_path()` = `credentials_dir().join("auth.json")`；serde_json pretty **明文**（`save_token` 用 `std::fs::write` 直写） |
| 进程内共享 slot | Tauri 用 `AppState.auth_result`（`Arc<Mutex<Option<anyhow::Result<TokenData>>>>`）；HTTP 用 `routes/auth.rs` 的 `OnceLock<AuthResult>`。`finalize` 从 slot `take()` 取 token |
| `AppState.codex_token` | `Arc<Mutex<Option<(access_token, account_id)>>>`，桌面侧 in-memory 缓存（`finalize` / `try_restore_session` 写入；`set_codex_model` 只读它重建 agent，不写） |
| `config.json` Codex provider 行 | 经 `provider::ensure_codex_provider_persisted` 落库 / `delete_providers_by_api_type(Codex)` 删除，与 token 文件是**两套独立存储** |

## 对外接口面（双 transport 对齐）

Tauri ↔ HTTP 镜像同一套能力（对齐表见 [`api-reference.md`](api-reference.md)）：

| 能力 | Tauri 命令 | HTTP 路由 |
|---|---|---|
| 起登录流程 | `start_codex_auth` | `POST /api/auth/codex/start` |
| 完成登录（取 token） | `finalize_codex_auth` | `POST /api/auth/codex/finalize` |
| 查认证状态 | `check_auth_status` | `GET /api/auth/codex/status` |
| 登出 | `logout_codex` | `POST /api/auth/codex/logout` |
| 列可选模型 | `get_codex_models` | `GET /api/auth/codex/models` |
| 设当前模型 | `set_codex_model` | `POST /api/auth/codex/models` |
| 启动恢复会话 | `try_restore_session` | `POST /api/auth/session/restore` |

桌面侧另有 `set_reasoning_effort` / `get_current_settings` / `initialize_agent` 等与 Codex 设置相邻的命令（非 OAuth 核心）。

CLI 一次性入口（`cli_auth.rs`，新建独立 tokio runtime）：

- `hope-agent auth codex login` → `login_codex` / `run_codex_login`（打印 auth URL + 等回调）
- `hope-agent auth codex status`
- `hope-agent auth codex logout` → `run_codex_logout`（走 `clear_token`）

完整 CLI 参考（含 loopback 端口转发说明、与 MCP OAuth 独立性）见 [`cli.md`](cli.md)。

## Hooks 埋点

- **`auth_success` Notification**：只从 **OAuth 流程完成站点** fire（`start_oauth_flow_with_auth_url` 内拿到 token 后）。**刻意不从 `save_token` fire**——`save_token` 也跑在 silent refresh 上，从那里发会误报「登录成功」。
- **`logout` SessionEnd**：经 `clear_token` fire 一次（app-global 代表事件）。

## 安全红线

- **token 禁入日志**（[AGENTS.md](../../AGENTS.md) 安全红线）：`oauth.rs` 日志只记 account_id 提取失败 / 刷新成功失败等元信息，**从不打印 `access_token` / `refresh_token`**。
- **loopback 隔离 + CSRF**：回调 server 仅绑 `127.0.0.1:1455`（never exposed externally）+ `state` 参数校验，5 分钟超时。
- **登出是破坏性的**：登出 shell 先 `delete_providers_by_api_type(Codex)` 删配置里的 Codex provider 行、再 `clear_token` 删 `auth.json`，重新登录会经 `ensure_codex_provider_persisted` 重建。
- **失败归类锚点**：`load_fresh_codex_token` 错误消息必须内嵌字面量 `authentication`，否则 `failover::classify_error` 不返 `FailoverReason::Auth`（`oauth.rs` 内单测锁此）。
- **Codex 不参与 failover profile 轮换**（[AGENTS.md](../../AGENTS.md) 红线 + [`provider-system.md`](provider-system.md)）：凭据失败直接走标准失败路径，不在 profile 间轮换。
- **`auth_success` 来源约束**：见上「Hooks 埋点」——只从 OAuth 流程完成站点 fire，不从 `save_token` fire。

### 已知缺口（技术债）

`save_token` 用 `std::fs::write` 明文直写 `auth.json`，**未走 `platform::write_secure_file`**——既不原子也不强制 `0600`，与 MCP 凭据（已走 `write_secure_file`）不一致，待安全收尾统一。该缺口已在 [`platform.md`](platform.md) 「已知缺口」与 [`security.md`](security.md) 登记。

## 跨子系统

| 子系统 | 关系 |
|---|---|
| [Provider 系统](provider-system.md) | Codex 是唯一 OAuth provider；§4.4「Codex OAuth API」详述 Bearer + `chatgpt-account-id` header、`ensure_codex_provider_persisted`、不参与轮换 |
| [MCP 客户端](mcp.md) | MCP 自有 `mcp/oauth.rs`（OAuth 2.1 + PKCE），与本子系统**物理隔离、不共用实现** |
| [Failover](failover.md) | `load_fresh_codex_token` 错误消息内嵌 `authentication` → `classify_error` 归 `Auth` |
| [Chat Engine](chat-engine.md) / [Side Query](side-query.md) | LLM 请求前调 `ensure_fresh_codex_token` / `load_fresh_codex_token` 保证 token 新鲜 |
| [CLI](cli.md) | `auth codex login/status/logout` 子命令 + loopback 端口转发 + 与 MCP OAuth 独立性 |
| [安全](security.md) / [平台](platform.md) | token 路径 `auth.json` + 登出必 `clear_token` + `save_token` 未走 `write_secure_file` 缺口 |
| [API 参考](api-reference.md) | Tauri ↔ HTTP 七项命令 / 路由对齐表 |
| Hooks | OAuth 完成 fire `auth_success` Notification；登出 fire `logout` SessionEnd |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/oauth.rs`](../../crates/ha-core/src/oauth.rs) | 子系统真相源：PKCE 流程 / `TokenData` / 刷新 / 失败归类锚点 |
| [`src-tauri/src/commands/auth.rs`](../../src-tauri/src/commands/auth.rs) | Tauri 命令面（7 命令，unmasked） |
| [`crates/ha-server/src/routes/auth.rs`](../../crates/ha-server/src/routes/auth.rs) | HTTP 路由面（`/api/auth/codex/*` + `OnceLock<AuthResult>`） |
| [`src-tauri/src/cli_auth.rs`](../../src-tauri/src/cli_auth.rs) | 终端 `auth codex` 子命令 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs) | `auth_path()` = `credentials_dir()/auth.json` |
