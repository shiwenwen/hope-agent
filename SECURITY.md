# Security Policy

[简体中文](#中文) · [English](#english)

---

## 中文

### 受支持的版本

| 版本 | 状态 |
|---|---|
| 0.1.x | ✅ 接受漏洞报告 |
| < 0.1 | ❌ |

正式发布前的预发版本（pre-release / alpha）不在受支持范围内。

### 报告漏洞

**请勿在公开 issue / discussion / PR 中报告安全漏洞**。

请通过 GitHub Security Advisory 私密提交：

- 入口：仓库主页 → **Security** 标签 → **Report a vulnerability**
- 直链：<https://github.com/shiwenwen/hope-agent/security/advisories/new>

提交后我们会在 **72 小时内首次回复**，确认漏洞性质、严重程度和修复时间线。如果漏洞被确认，会在修复发布同时公开披露并致谢报告者（除非你要求匿名）。

### 漏洞范围

Hope Agent 是一款本地运行的桌面 AI 助手，请重点关注以下方向：

- **凭据泄露**：API Key / OAuth token / 用户私密配置（`~/.hope-agent/credentials/`）从内存、日志、IPC、HTTP 响应、错误消息等通道意外暴露
- **任意命令执行 / 沙箱绕过**：通过工具调用、技能脚本、MCP 服务器、Channel webhook 等渠道绕过权限引擎执行用户未授权的命令
- **SSRF**：HTTP 出站请求绕过 [`security::ssrf::check_url`](crates/ha-core/src/security/ssrf.rs) 触达内网或元数据服务
- **XSS / 提示注入**：会话历史、技能内容、第三方文档触达 webview 时的 DOM 注入
- **签名 / 升级链**：Tauri Updater 验签绕过、updater 私钥相关
- **HTTP/WS 守护进程**：`hope-agent server` 鉴权绕过、未鉴权数据访问
- **本地文件越权**：通过工作目录注入或路径解析问题访问受保护路径
- **依赖供应链**：vendored skills 或 dependencies 中已知 CVE

### 不在范围内

- 用户主动配置的危险操作（如把 Smart 模式 / YOLO 模式开了之后命令执行）—— 这是设计行为
- 第三方 LLM 提供商的内容审核问题
- UI 美观 / 可用性 / 性能问题 —— 请走普通 issue
- 已经在 [`docs/architecture/`](docs/architecture/) 中明确标注的 trade-off

### 隐私与日志

Hope Agent 的[统一日志](crates/ha-core/src/logging.rs) 在写入前用 `redact_sensitive` 脱敏 API Key / OAuth Token / 长 base64 内容。**如果你发现日志泄露了任何凭据 / token / 用户私密配置**，请按上述渠道私密报告。

---

## English

### Supported Versions

| Version | Status |
|---|---|
| 0.1.x | ✅ Accepting vulnerability reports |
| < 0.1 | ❌ |

Pre-release versions (alpha) are not in scope.

### Reporting a Vulnerability

**Please do NOT file security issues in public issues / discussions / PRs.**

Report privately via GitHub Security Advisory:

- Path: repository home → **Security** tab → **Report a vulnerability**
- Direct link: <https://github.com/shiwenwen/hope-agent/security/advisories/new>

We aim to provide an **initial response within 72 hours**, with confirmation of the issue and a remediation timeline. Confirmed vulnerabilities will be publicly disclosed alongside the fix release with credit to the reporter (unless anonymity is requested).

### In Scope

Hope Agent is a locally-running desktop AI assistant. We are most interested in:

- **Credential leakage**: API keys / OAuth tokens / private user config (`~/.hope-agent/credentials/`) leaking via memory, logs, IPC, HTTP responses, error messages
- **Arbitrary command execution / sandbox bypass**: via tool calls, skill scripts, MCP servers, channel webhooks bypassing the permission engine
- **SSRF**: outbound HTTP bypassing [`security::ssrf::check_url`](crates/ha-core/src/security/ssrf.rs) to reach internal networks or metadata services
- **XSS / prompt injection**: DOM injection through session history, skill content, third-party documents reaching the webview
- **Signing / update chain**: Tauri Updater signature verification bypass, updater private key issues
- **HTTP/WS daemon**: `hope-agent server` auth bypass, unauthenticated data access
- **Local file privilege escalation**: working-directory injection or path resolution issues accessing protected paths
- **Dependency supply chain**: known CVEs in vendored skills or dependencies

### Out of Scope

- User-explicitly-enabled dangerous operations (e.g. command execution after enabling Smart / YOLO mode) — this is by design
- Content moderation issues of third-party LLM providers
- UI / UX / performance issues — please file a regular issue
- Trade-offs already documented in [`docs/architecture/`](docs/architecture/)

### Privacy and Logs

Hope Agent's [unified logging](crates/ha-core/src/logging.rs) applies `redact_sensitive` before writing API keys / OAuth tokens / long base64 payloads. **If you find any log leaking credentials / tokens / private user config**, please report via the channel above.
