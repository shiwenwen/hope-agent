//! `peek_sessions` — a deferred tool that lets the model actively inspect
//! other sessions without waiting for a turn-start refresh.
//!
//! The schema is minimal: an optional free-form query filter and an optional
//! limit. It always reads from the live `SessionDB` + `RecapDb` (no caching),
//! so the model gets current data.

use serde_json::{json, Value};

use super::config::CrossSessionConfig;
use crate::tools::ToolDefinition;

/// Construct the `peek_sessions` tool definition. Registered as `deferred=true`
/// so it only shows up via `tool_search` unless explicitly loaded.
pub fn peek_sessions_schema() -> ToolDefinition {
    ToolDefinition {
        name: crate::tools::TOOL_PEEK_SESSIONS.into(),
        description: "Inspect what the user is doing in other sessions right now. \
Returns a compact markdown list of peer sessions (title, agent, kind, relative time, \
goal/summary). Use this when the user references \"the other thing\", \"last time\", \
or you suspect cross-session context matters. Always read-only.".into(),
        internal: true,
        deferred: true,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional substring filter matched against session title/goal."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max entries to return. Default 6, max 20.",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Execute the tool. `current_session_id` is pulled from the tool execution
/// context so we can exclude the caller's own session.
pub fn run_peek_sessions(
    args: &Value,
    current_session_id: Option<&str>,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::to_lowercase);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(6)
        .clamp(1, 20) as usize;

    let session_db = crate::get_session_db().ok_or_else(|| "session DB unavailable".to_string())?;
    let cfg_global = crate::config::cached_config().cross_session.clone();
    // Relax scope for peek: ignore the global enabled flag and type exclusions —
    // the model is actively asking, so give it everything visible.
    let cfg = CrossSessionConfig {
        enabled: true,
        exclude_cron: false,
        exclude_channel: false,
        exclude_subagents: false,
        max_sessions: limit,
        ..cfg_global
    };
    let current = current_session_id.unwrap_or("");
    let mut snap = super::collect::collect_entries(&session_db, &cfg, current)
        .map_err(|e| format!("collect_entries failed: {}", e))?;

    if let Some(q) = query {
        snap.entries.retain(|e| {
            e.title.to_lowercase().contains(&q)
                || e.underlying_goal
                    .as_deref()
                    .map(|g| g.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || e.brief_summary
                    .as_deref()
                    .map(|s| s.to_lowercase().contains(&q))
                    .unwrap_or(false)
        });
    }

    if snap.entries.is_empty() {
        return Ok("No peer sessions match.".into());
    }

    let md = super::render::render_markdown(&snap, cfg.max_chars)
        .unwrap_or_else(|| "No peer sessions available.".into());
    Ok(md)
}
