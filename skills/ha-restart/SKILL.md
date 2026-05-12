---
name: ha-restart
description: "Restart the running Hope Agent process via conversation. Use when the user asks to restart the app / service / daemon — phrases like 'restart Hope Agent', 'restart the app', 'restart the service', '重启', '重启一下', '重新启动', 'reboot the agent', 'kick the daemon'. Also use when a settings change explicitly requires a restart (rare — most settings hot-reload), or when a subsystem appears wedged and the user agrees to a restart. Covers all formfactors: desktop GUI (Tauri guardian respawn), `hope-agent server` installed as launchd/systemd/Task Scheduler, and foreground `hope-agent server` (detached child + self-exit). Always user-confirmed via `ask_user_question`."
always: false
aliases:
  - restart
  - reboot
allowed-tools: [app_restart, ask_user_question]
---

# Hope Agent Restart

Hope Agent runs in one of three modes (desktop GUI / `hope-agent server` / `hope-agent acp`) and each mode has its own restart strategy. The `app_restart` tool picks the right one automatically; this skill is the methodology for invoking it well.

## When to use

Trigger paths:

- **User asks** — "重启" / "restart" / "restart the service" / "reboot the daemon".
- **Settings change requires restart** — `update_settings` returned a hint like `restart_required: true`. Mention that the change is staged and offer to restart.
- **Subsystem wedged** — channel worker not responding, MCP client stuck, exec process registry full. Mention the symptom, suggest a restart, let the user decide.

Do NOT use restart as a debugging crutch when the actual fix is a config change or a logged-out provider. Try the targeted fix first.

## Workflow

### 1. Just call the tool

```
app_restart()
```

That's it. The tool:

1. Detects the runtime mode (desktop / server-with-installed-service / foreground server / acp).
2. Inventories in-flight work (chat turns, async tool jobs, running cron jobs).
3. If anything is in flight, asks the user to acknowledge the interruption first.
4. Asks a final Yes/No confirmation with the route label so the user knows what's about to happen.
5. Hands off to the OS-level supervisor (guardian / launchd / systemd / schtasks) or spawns a detached child.

The model does NOT pre-summarize in-flight work — the tool does that inside the prompt. Don't duplicate it in the chat turn before calling.

### 2. Tell the user what to expect after restart

The new instance comes up automatically in every mode except `acp`. Briefly tell the user:

- **Desktop**: "The window will close and reopen in a couple of seconds."
- **Server (installed)**: "The service will be restarted by the OS supervisor. HTTP clients will need to reconnect."
- **Server (foreground)**: "Your terminal session will exit. The new process is running detached in the background — `hope-agent server status` (or `kill` by PID) controls it from here."

### 3. ACP refusal

When `runtime_role` is `acp`, the tool returns `status: "unsupported_mode"`. Explain to the user: "Restart isn't supported in ACP mode because the IDE owns the stdio pipes. Restart from your IDE / agent host instead — e.g. close and reopen the agent panel, or rerun the `hope-agent acp` command from your IDE's integration."

## Error recovery

| Tool result                          | Meaning                                                | What to do                                                                 |
| ------------------------------------ | ------------------------------------------------------ | -------------------------------------------------------------------------- |
| `cancelled_by_user` (preflight)      | User backed out after seeing in-flight work            | Acknowledge; offer to wait until the work finishes (and remind them later) |
| `cancelled_by_user` (confirmation)   | User backed out at the final Yes/No                    | Drop it; don't re-ask without a new reason                                 |
| `unsupported_mode`                   | ACP / unknown runtime                                  | Explain ACP refusal (see above) and stop                                   |
| `failed` + `route: "desktop"`        | No `AppLifecycleBridge` registered (impossible in prod)| File a bug; offer manual relaunch ("Cmd+Q then reopen Hope Agent")         |
| `failed` + `route: "service"`        | `launchctl` / `systemctl` / `schtasks` returned error  | Show the error; suggest `hope-agent server status` to inspect              |
| `failed` + `route: "respawn"`        | Detached spawn failed (rare)                           | Show the error; suggest `Ctrl-C` and manual restart                        |

## When NOT to call

- During a **plan-mode session** — restart would orphan the plan. Tell the user to `/plan exit` first.
- When the user is **mid-streaming an answer** in another session — the pre-flight will warn them, but you should also acknowledge it in chat before they confirm.
- When **`hope-agent server start` is unsupervised foreground in production** — the detached respawn works, but the user loses the live log stream. Mention this before they confirm.

## Background

Routes:
- **Desktop** → Tauri `AppHandle::exit(42)`; the guardian process (`ha_core::guardian::run_guardian`) catches the exit code and respawns the GUI child.
- **Service** → `launchctl kickstart -k gui/$UID/ai.hopeagent.server` / `systemctl --user restart hope-agent.service` / `schtasks /End` + `/Run`. Same helper the self-update flow uses after a binary swap.
- **Respawn** → spawn a detached `hope-agent server <captured-argv>` child with `setsid` (Unix) / `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` (Windows), then `std::process::exit(0)` after a 200ms grace so the EventBus / tool result has time to flush.
- **Acp** → refused.

The captured argv comes from `ha_core::server_launch_args()` which the `server` binary entrypoint registers at startup. If the operator started the server without options, the respawn uses the same defaults (`127.0.0.1:8420`, no api key).
