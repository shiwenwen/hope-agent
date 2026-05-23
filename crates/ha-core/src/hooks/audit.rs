//! Hook audit logging + overflow file handling (design §7.10 / §8.6 / §16.2).
//!
//! Every dispatch logs one `app_info!("hooks", "dispatch", …)` line. Injected
//! context exceeding the 10 000-char cap is spilled to an overflow file and
//! replaced inline with a pointer.

use std::path::PathBuf;
use std::time::Duration;

use super::types::{HookDecision, HookEvent, HookOutcome};

/// Max characters of hook output injected into the LLM context (design §8.6).
pub const MAX_INJECT_CHARS: usize = 10_000;

fn decision_label(decision: &HookDecision) -> &'static str {
    match decision {
        HookDecision::Allow => "allow",
        HookDecision::Ask => "ask",
        HookDecision::Defer => "defer",
        HookDecision::Block { .. } => "block",
        HookDecision::Deny { .. } => "deny",
    }
}

/// Log one dispatch (always `category="hooks"`, `source="dispatch"`).
pub fn log_dispatch(
    event: HookEvent,
    handler_count: usize,
    outcome: &HookOutcome,
    duration: Duration,
) {
    app_info!(
        "hooks",
        "dispatch",
        "event={} handlers={} decision={} continue={} ctx_blocks={} dur={}ms",
        event.as_str(),
        handler_count,
        decision_label(&outcome.decision),
        outcome.continue_execution,
        outcome.additional_context.len(),
        duration.as_millis()
    );
}

/// Write oversized injected content to
/// `~/.hope-agent/hooks/overflow/{event}-{session}-{ts}.txt`. Returns the path
/// on success.
pub fn write_overflow(event: HookEvent, session_id: &str, content: &str) -> Option<PathBuf> {
    let dir = crate::paths::hooks_dir().ok()?.join("overflow");
    std::fs::create_dir_all(&dir).ok()?;
    let ts = chrono::Utc::now().timestamp_millis();
    let safe_sid: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let path = dir.join(format!("{}-{}-{}.txt", event.as_str(), safe_sid, ts));
    std::fs::write(&path, content).ok()?;
    Some(path)
}
