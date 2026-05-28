# Hooks 系统

事件 → 可拔插处理器。在工具调用、会话生命周期、上下文压缩、权限决策等关键节点执行用户自定义命令 / HTTP / LLM / 子 Agent，**字段级对齐 Claude Code hooks 协议**——社区脚本 paste 即用。完整设计见 [`docs/plans/hooks-system-design.md`](../plans/hooks-system-design.md)；本文描述**已落地**的能力与契约。

## 能力总览

- **28 事件协议面**：枚举完整（`types.rs::HookEvent`）。其中 **24 个真触发**，4 个（`WorktreeCreate` / `WorktreeRemove` / `TeammateIdle` / `InstructionsLoaded`）协议保留——Hope Agent 无对应概念（instructions 是 per-turn 组装进 system prompt；无 git-worktree / teammate-idle），可配置但不 dispatch，待对应能力落地再 fire。
- **5 种 handler**：`command`（shell 子进程）/ `http`（SSRF-gated POST）/ `mcp_tool`（调 MCP 工具）/ `prompt`（一次性 LLM side-query）/ `agent`（spawn 子 Agent）。
- **四层配置 scope**（全部 UNION，无覆盖优先级）：见下「多 scope」。
- **配置热重载**：`config:changed` 触发全局 registry 重建；project/local 文件按 mtime 失效。
- **JSONL transcript 镜像**：`transcript_path` 指向 `~/.hope-agent/sessions/{id}/transcript.jsonl`，启动期 backfill + 持久化时 live 追加，官方脚本可 `jq`。

### 事件分类

| 类型 | 事件 | 决策语义 |
|------|------|---------|
| **阻断型** | `UserPromptSubmit` / `PreToolUse` / `PreCompact` | `block`/`deny` 真阻断（prompt 不入会话 / 工具被拒 / 跳过压缩）|
| **观察型** | `SessionStart` · `SessionEnd` / `Notification` / `PostToolUse` · `PostToolUseFailure` · `PostToolBatch` / `PostCompact` / `Stop` · `StopFailure` / `SubagentStart` · `SubagentStop` / `TaskCreated` · `TaskCompleted` / `ConfigChange` / `CwdChanged` / `FileChanged` / `PermissionRequest` · `PermissionDenied` / `UserPromptExpansion` / `Elicitation` · `ElicitationResult` | 只注入 `additionalContext` / 通知；`block` 降级为非阻断 + log（`is_observation_only`）|

> `Stop` / `StopFailure` 当前 fire-and-forget 观察型（未实现 block-to-continue）；落地该语义时移出 `is_observation_only`。

## 多 scope（design §4）

| Scope | 位置 | 范围 |
|-------|------|------|
| **user** | `~/.hope-agent/config.json` 的 `hooks` | 全局，编进 `registry::global()` |
| **managed** | `/etc/hope-agent/hooks.json`（Win: `%PROGRAMDATA%\hope-agent\hooks.json`）| 全局（企业下发），合进 `registry::global()` |
| **project** | `<会话工作目录>/.hope-agent/hooks.json` | 随仓库共享，按 cwd 解析（默认关，需开 `allowProjectScope`）|
| **local** | `<会话工作目录>/.hope-agent/hooks.local.json` | git-ignored 开发者私有，按 cwd 解析 |

- **UNION 语义**：所有 scope 命中的 hook 都跑，无覆盖。
- project/local 依赖会话工作目录（`sessions.working_dir`，无 home 回退），在 dispatch 时经 [`scopes::resolve_for_cwd`](../../crates/ha-core/src/hooks/scopes.rs) 合并到全局之上，**per-cwd 缓存**（mtime + 全局 reload generation 失效）；无 project/local 文件时直接返回全局 registry（≤2 次 stat）。
- **project/local 默认关闭**（`hooks_allow_project_scope`，`AppConfig` 字段，默认 `false`）：仓库 check-in 的 hooks 不应因会话 cwd 指向就自动执行 shell / HTTP / LLM / 子 Agent（供应链防护）。`resolve_for_cwd` 在开关为 `false` 时直接返回全局 registry、**绝不读取** project/local 文件；用户在 Settings → Hooks 显式开启才加载。`ha-settings` 技能只读此开关（与 `hooks` 同属 `BLOCKED_UPDATE_CATEGORIES`）。
- 每条 fire 路径（dispatch / `fire_and_forget` / 各 gate）统一走 `scopes::any_handlers_for(event, cwd)`，所以 project-only hook 在 user/managed 为空时也能触发。
- `disable_all_hooks` 主开关关闭**所有** scope。

## 模块（`crates/ha-core/src/hooks/`）

| 文件 | 职责 |
|------|------|
| `mod.rs` | `HookDispatcher::dispatch`（per-cwd 解析 → 匹配 → 并发执行（catch_unwind 隔离）→ 聚合 → 审计）+ `fire_*` 助手 + `init` |
| `types.rs` | `HookEvent`（28 变体）/ `HookInput`（per-event，flatten common）/ `HookOutput` / `HookOutcome` / `HookDecision` / `PermissionMode` |
| `config.rs` | `HooksConfig`（28×`Vec<HookMatcherGroup>`）+ 5 种 `HookHandlerConfig` + `merge_from`（scope 合并）|
| `scopes.rs` | 多 scope 解析：managed 加载 / 全局合并配置缓存 + generation / per-cwd registry 缓存 / `resolve_for_cwd` / `any_handlers_for` |
| `matcher.rs` | 三语法：wildcard / 精确-或-pipe / regex（无效 regex → never-match）|
| `registry.rs` | 全局 `ArcSwap<HookRegistry>`；`reload_from_config`（user+managed 合并 + 喂 scopes + bump generation）|
| `runner/{mod,command,http,mcp_tool,prompt,agent}.rs` | `HookHandler` trait + 5 种 handler |
| `parse.rs` | exit code + JSON / plaintext → `HookContribution`；`permissionDecision` 仅 PreToolUse |
| `decision.rs` | 多 hook 聚合（`deny > block > defer > ask > allow`，additionalContext 有序拼接 + 10000 字符上限 overflow）|
| `env.rs` | `command` 环境变量（`CLAUDE_PROJECT_DIR` / `HOPE_*` / `CLAUDE_CODE_REMOTE` / `PATH`）|
| `transcript.rs` | JSONL 镜像：`build_line` 共享 + backfill（跳 incognito）+ `append_persisted`（live）|

## 关键契约

- **零 Tauri 依赖**：全在 `ha-core`，desktop / `server` / ACP 三模式共用。
- **统一入口**：业务代码只碰 `HookDispatcher::dispatch` / `fire_*` 助手，读 `HookOutcome`；匹配 / 并发 / 超时 / 去重 / 聚合 / panic 隔离全在内部。
- **PreToolUse 接权限引擎**：name 可见性闸后、权限引擎前触发；`deny` 短路；`updatedInput` 影子参数让引擎重判；`permissionDecision:"allow"` 仅跳软 Ask（保护路径 / 危险命令 / Plan 永远弹窗）；`ask`/`defer` 强制弹窗。
- **UserPromptSubmit 阻断**：在 [`agent::preflight::user_prompt_preflight`](../../crates/ha-core/src/agent/preflight.rs)（四入口统一 chokepoint）触发；`block`/`deny`/`continue:false` → prompt 不持久化为 user 消息（不入 LLM 上下文）、不跑 turn、各入口原生回话 + 落一条 `event` 行；非阻断的 `additionalContext` 暂存 session slot，turn 起始 drain 进 `extra_system_context`。
- **`command` 默认超时 600s**（http 30s / prompt 60s / agent 120s）；stdout/stderr 各截断 1 MiB；exit 2 = 阻断（观察型降级为非阻断 + log）。
- **配置读 `cached_config().hooks`**，user scope 写走 `mutate_config(("hooks", source), …)`（详见 [`config-system.md`](config-system.md)）；project/local/managed 是独立 scope 文件。
- **`ha-settings` 技能只读**：`get_settings` 含 `hooks`（http header 脱敏），写被 `BLOCKED_UPDATE_CATEGORIES` 拦截——hooks 能跑任意命令，可写会让模型给自己装命令执行（特权升级）。

## Handler 通用字段

每个 handler 都可带这些可选字段（`asyncRewake` 仅 `command`）；过滤在 `dispatch_with` 的 build 循环内、去重前完成，不改 `HookHandler` trait：

| 字段 | 作用 |
| --- | --- |
| `timeout` | 单 handler 超时秒（默认值见上：command 600 / http 30 / prompt 60 / agent 120）|
| `if` | 条件执行 `ToolName(pattern)`：**仅** `PreToolUse` / `PostToolUse` / `PostToolUseFailure` 这类带工具参数的事件求值，工具名或模式不匹配则跳过该 handler（其余事件直接跳过，fail-safe）。复用权限引擎的参数提取器 + glob（`*` 贪心、`**` 等价 `*`，不拆 Bash subcommand）；接受 Claude Code 工具别名（`Bash`→`exec`、`Write`→`write`、`Edit`→`edit`、`Read`→`read`、`WebFetch`→`web_fetch`）。例：`exec(rm *)` / `write(src/**)` / `web_fetch(*.github.com)` |
| `once` | 该 handler 每会话只跑一次（per-process 内存去重，按 type + identity，进程重启重置，与 `SessionStart` once 同语义）|
| `statusMessage` | handler 即将运行时在桌面 GUI 弹一句 toast（emit `hook:status` 事件，App 全局监听）。慢 handler（`async` / `agent` / `prompt`）才有感，快 `command` 一闪而过；IM 渠道暂不展示 |
| `asyncRewake` | （仅 `command` + `async`）后台 hook `exit 2` 时，把 stderr 作为 `<hook-async-result>` system-reminder 注入**下一轮对话**（复用 `subagent::injection::inject_and_run_parent`）。**注意：这会让后台 hook 自主发起一轮 LLM（消耗 token）**——需作者显式配 `asyncRewake` 且 hook 主动 `exit 2`，审计 `category=hooks` |
| `shell` / `async` | （`command`）`bash` / `powershell`；`async` = fire-and-forget，不影响决策 |

## 配置示例

user scope（`~/.hope-agent/config.json` 顶层 `hooks` 键）：

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "~/.hope-agent/hooks/guard.sh" } ] }
    ],
    "PostToolUse": [
      { "matcher": "Write|Edit", "hooks": [ { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.hope-agent/hooks/fmt.sh", "async": true } ] }
    ],
    "SessionStart": [
      { "matcher": "startup|resume", "hooks": [ { "type": "command", "command": "~/.hope-agent/hooks/load-context.sh" } ] }
    ]
  }
}
```

project scope（`<仓库>/.hope-agent/hooks.json`，提交进仓库给团队共享，shape 与上面 `hooks` 对象一致）：

```json
{
  "FileChanged": [
    { "matcher": ".*\\.rs$", "hooks": [ { "type": "command", "command": "jq -r .path | xargs rustfmt", "async": true } ] }
  ]
}
```

> 事件数据（如 `FileChanged` 的 `path`）在 stdin 的 hook 输入 JSON 里，用 `jq` 读取；env 只携带 `CLAUDE_PROJECT_DIR` / `HOPE_*` 等通用变量（§11.1）。`matcher` 对 `FileChanged` 匹配文件**绝对路径**，故 `.*\.rs$` 命中所有 `.rs`。

## 已知差异 / 缺口

- `Stop` / `StopFailure` 暂为观察型（无 block-to-continue）。
- `PostToolUse` 拿到的是工具结果预览（超大结果落盘后的 head+tail），非完整内容。
- live transcript 仅在 user/managed scope 有 hook 时追加（避免每消息持久化热路径上的 stat）；project-only 会话退化为 backfill-only（§10.4 允许 drift）。
- `prompt` / `agent` handler 在 dispatch 内跑 LLM / spawn 子 Agent，有成本与延迟——勿挂在 PreToolUse 等热路径。

完整协议差异红线见设计文档 §2.4。
