//! `schedule_wakeup` tool — agent self-scheduled, one-shot wake-back (R10).
//!
//! The agent calls this to be re-entered into the **current session** after a
//! delay, then ends its turn. At fire time a `<wakeup>` message is injected
//! through the shared injection pipeline (idle-gated, like a background-job
//! completion) and a fresh parent turn runs carrying the agent's note.
//!
//! This is NOT cron (user-configured, periodic, possibly a separate session) —
//! it is a one-shot "self-pause then resume here" primitive. See
//! `crate::wakeup` for the lifecycle and cross-process model.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::execution::ToolExecContext;
use crate::wakeup;

pub async fn tool_schedule_wakeup(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("schedule_wakeup requires an active session"))?;
    let agent_id = ctx
        .agent_id
        .as_deref()
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);

    // Only top-level sessions may schedule wakeups. A subagent / forked child
    // session is a transient worker: it has no future user-facing turns and is
    // never torn down via the session-cleanup watcher, so a wakeup would later
    // fire a ghost, billed parent turn into a long-dormant child session that
    // nobody is watching. Subagent runs carry `subagent_depth > 0` (fast path);
    // any other child carries a `parent_session_id` (covers forks etc.).
    if ctx.subagent_depth > 0 {
        return Err(anyhow!(
            "schedule_wakeup is not available inside a subagent run — it would resume a transient child session"
        ));
    }
    if let Some(db) = crate::get_session_db() {
        if let Ok(Some(meta)) = db.get_session(session_id) {
            if meta.parent_session_id.is_some() {
                return Err(anyhow!(
                    "schedule_wakeup is only available for top-level sessions, not child / forked sessions"
                ));
            }
        }
    }

    let delay_secs = args
        .get("delay_secs")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("schedule_wakeup: missing required integer `delay_secs`"))?;
    if delay_secs <= 0 {
        return Err(anyhow!(
            "schedule_wakeup: `delay_secs` must be a positive number of seconds"
        ));
    }
    let note = args
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match wakeup::schedule(session_id, agent_id, delay_secs, note, ctx.incognito) {
        Ok(outcome) => Ok(json!({
            "scheduled": true,
            "wakeup_id": outcome.id,
            "delay_secs": outcome.delay_secs,
            "fire_at_unix": outcome.fire_at,
            "message": format!(
                "Wakeup scheduled. End your turn now — you'll be woken back into this \
                 session in ~{}s to continue. (Clamped to [{}, {}]s.)",
                outcome.delay_secs,
                wakeup::MIN_DELAY_SECS,
                wakeup::max_delay_secs()
            )
        })
        .to_string()),
        Err(e) => Err(anyhow!("schedule_wakeup: {}", e)),
    }
}

pub fn get_schedule_wakeup_tool() -> super::definitions::ToolDefinition {
    super::definitions::ToolDefinition {
        name: super::TOOL_SCHEDULE_WAKEUP.into(),
        description: "Schedule a one-shot wake-up that re-enters THIS session after a delay, then \
            end your turn. Use when you must wait on something the runtime can't notify you about \
            — an external CI run, a remote queue, a rate-limit cooldown, or a 'check again later' — \
            instead of busy-polling with job_status or stalling the turn. At fire time you receive a \
            `<wakeup>` message carrying your `note` and run a fresh turn to continue. This is NOT \
            cron (that's user-configured & periodic); this is one-shot and continues the current \
            context. `delay_secs` is clamped to a configured range (default [10, 86400]s; the 10s \
            floor is fixed). Limited to a configured number of pending wakeups per session (default 5)."
            .into(),
        tier: super::definitions::ToolTier::Core {
            subclass: super::definitions::CoreSubclass::Meta,
        },
        // Benign control-flow primitive (schedules a self-wakeup; no external
        // side effect) → internal so it never prompts for approval, like
        // `job_status`. `internal` governs approval, not model visibility.
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "delay_secs": {
                    "type": "integer",
                    "description": "Seconds from now to wake back up. Clamped to [10, 86400]. Pick a cadence that matches what you're waiting for — for a CI run that takes ~8 minutes, ~480, not 30."
                },
                "note": {
                    "type": "string",
                    "description": "A note to your future self: what you're waiting for and what to do when woken. This is injected back verbatim so you can resume without re-deriving context."
                }
            },
            "required": ["delay_secs"],
            "additionalProperties": false
        }),
    }
}
