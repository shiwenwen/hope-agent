use serde_json::json;

use super::super::TOOL_APP_RESTART;
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

pub fn get_app_restart_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APP_RESTART.into(),
        description: "Restart the running Hope Agent process. Use when the user asks to restart \
the app / service, after a settings change that requires a restart, or to recover from a \
stuck subsystem.\n\n\
Routing (picked automatically by the current runtime mode):\n\
- Desktop GUI → triggers `app.exit(42)`; the guardian process respawns the GUI child.\n\
- `hope-agent server` with launchd / systemd / Task Scheduler installed → asks the OS supervisor \
to restart the service (`launchctl kickstart -k` / `systemctl --user restart` / `schtasks /End` + `/Run`).\n\
- `hope-agent server` running foreground (no installed service) → spawns a detached child with \
the original launch argv and self-exits.\n\
- `hope-agent acp` → refused. The IDE owns the stdio lifetime; restart there means the IDE \
needs to re-spawn the backend itself.\n\n\
Two mandatory confirmation gates:\n\
1. **Pre-flight** — if any chat turns / async tool jobs / running cron jobs are in flight, the \
tool first asks the user to acknowledge the interruption. Skipped when nothing is running.\n\
2. **Confirmation** — a Yes/No dialog naming the route (so the user sees exactly what will run).\n\n\
Neither Plan mode nor `--dangerously-skip-all-approvals` suppresses these gates. After the user \
confirms, the tool returns immediately; the process is expected to die within a few hundred ms. \
For server / desktop modes a fresh process comes back up automatically; for foreground server the \
detached child takes over the bind socket.\n\n\
The `action` field is reserved for future verbs (stop, start) — today only `restart` (or omitting \
the field entirely) is accepted."
            .into(),
        // Meta tier: same shape as `app_update`. Always-eager schema since
        // this is a low-frequency but high-value capability the model should
        // discover without `tool_search`.
        tier: ToolTier::Core {
            subclass: CoreSubclass::Meta,
        },
        // `internal: false` so the standard permission flow runs *around*
        // the call; the tool then layers its own ask_user_question gates
        // inside. internal=true would skip the permission engine entirely
        // and we want any "no app_restart in this agent" capability filter
        // to still apply.
        internal: false,
        // Restart is inherently process-scoped — no point in two
        // concurrent calls; the second would race on `exit(42)`.
        concurrent_safe: false,
        // Returns within ~1s of confirmation; even the foreground respawn
        // path completes the spawn synchronously and then schedules a 200ms
        // self-exit. No background job needed.
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["restart"],
                    "description": "Reserved for future verbs (today only `restart` is recognized; omit to default)."
                }
            },
            "additionalProperties": false
        }),
    }
}
