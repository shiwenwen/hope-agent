# Hooks 系统

事件 → 可拔插处理器。在工具调用、会话生命周期、上下文压缩等关键节点执行用户自定义命令，**字段级对齐 Claude Code hooks 协议**——社区脚本 paste 即用。完整设计与 28 事件路线见 [`docs/plans/hooks-system-design.md`](../plans/hooks-system-design.md)；本文只描述**当前已落地**的能力与契约。

## 当前能力（首个里程碑）

- **6 个观察型事件**（均不阻断，只注入 `additionalContext`）：`SessionStart`（startup / resume / compact）、`SessionEnd`（clear / logout）、`Notification`（permission_prompt）、`PostToolUse`、`PostToolUseFailure`、`PostCompact`。
- **`command` handler**：shell 子进程，stdin 收完整 hook 输入 JSON，按 exit code + stdout JSON 双通道返回。
- **配置热重载**：改 `config.json` 的 `hooks` 字段后，下一次事件已用新配置，无需重启。
- **JSONL transcript 镜像**：`transcript_path` 指向 `~/.hope-agent/sessions/{id}/transcript.jsonl`，官方脚本可 `jq` 读取。

> 阻断型事件（`PreToolUse` / `UserPromptSubmit` / `Stop` / `PreCompact` …）、`http`/`mcp_tool`/`prompt`/`agent` handler、多 scope 配置（project / local / managed）、GUI 面板与 `ha-settings` 技能集成尚未落地，见设计文档 §18 的分阶段计划。

## 模块（`crates/ha-core/src/hooks/`）

| 文件 | 职责 |
|------|------|
| `mod.rs` | `HookDispatcher::dispatch`（匹配 → 并发执行 → 聚合 → 审计）+ `fire_and_forget` / `fire_notification` / `fire_session_end` + `init` |
| `types.rs` | `HookEvent`（28 变体）/ `HookInput`（per-event，flatten common）/ `HookOutput` / `HookOutcome` / `HookDecision` / `PermissionMode` |
| `config.rs` | `HooksConfig`（`AppConfig.hooks`）+ 5 种 `HookHandlerConfig` |
| `matcher.rs` | 三语法判别：wildcard / 精确-或-pipe / regex（无效 regex → never-match） |
| `registry.rs` | 全局 `ArcSwap<HookRegistry>` + `reload_from_config` 热重载 |
| `runner/{mod,command}.rs` | `HookHandler` trait + `command` 子进程 spawner |
| `parse.rs` | exit code + JSON / plaintext → `HookContribution` |
| `decision.rs` | 多 hook 聚合（`deny > block > defer > ask > allow`，additionalContext 有序拼接） |
| `env.rs` | `command` 环境变量组装（`CLAUDE_PROJECT_DIR` / `HOPE_*` / `CLAUDE_CODE_REMOTE` / `PATH`） |
| `audit.rs` | `category="hooks"` 审计日志 + overflow 文件（10 000 字符注入上限） |
| `transcript.rs` | JSONL 镜像写入 + 历史会话 backfill |

## 关键契约

- **零 Tauri 依赖**：全在 `ha-core`，desktop / `server` / ACP 三模式共用。
- **`additionalContext` 注入两路**：`SessionStart(startup/resume)` 走 `extra_system_context`（turn 级，跨 failover 存活）；`PostToolUse`/`PostToolUseFailure` append 到工具结果；`PostCompact` / `SessionStart(compact)` / `Notification` 走 agent 的 `pending_hook_context` → 下一轮 reminder suffix。
- **`command` 默认超时 600s**；stdout/stderr 各截断 1 MiB；exit 2 = 阻断（观察型事件降级为非阻断 + log）。
- **配置读写走 config contract**：读 `cached_config().hooks`，写 `mutate_config(("hooks", source), …)`（详见 [`config-system.md`](config-system.md)）。本期仅 user scope（`~/.hope-agent/config.json`）。
- **四入口统一 preflight**：Tauri / HTTP / IM / ACP 的 user message 均经 [`agent::preflight::user_prompt_preflight`](../../crates/ha-core/src/agent/preflight.rs)（当前透传，`UserPromptSubmit` 落地后在此接 block）。

## 配置示例（`~/.hope-agent/config.json` 顶层 `hooks` 键）

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.hope-agent/hooks/fmt.sh", "async": true }
        ]
      }
    ],
    "SessionStart": [
      { "matcher": "startup|resume", "hooks": [ { "type": "command", "command": "~/.hope-agent/hooks/load-context.sh" } ] }
    ]
  }
}
```

## 已知差异 / 缺口（本期）

- `permission_mode` 仅近似（plan allow-list 非空 → `plan`，否则 `default`；YOLO / Smart 未区分）。
- `Notification` 的 `idle_prompt` / `auth_success`、`SessionEnd` 的 app-shutdown、多 session fan-out 尚未接入。
- `PostToolUse` 拿到的是工具结果预览（超大结果落盘后的 head+tail），非完整内容。

完整协议差异红线见设计文档 §2.4。
