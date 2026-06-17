//! Unified `job:*` EventBus namespace (R3).
//!
//! Every background-job lifecycle signal goes out under one kind-tagged prefix —
//! `job:{created,updated,progress,completed}` — replacing the old
//! `async_tool_job:*` prefix. The R4 panel subscribes to this single namespace
//! to render every kind (`tool` / `group`) in one place; the `kind` +
//! `session_id` fields let it filter and group without a second lookup.
//!
//! Scope note: the `subagent` kind keeps its existing richer `subagent:*` event
//! stream (spawned / running / completed) rather than double-emitting here — the
//! R4 panel reads subagent rows from that stream plus the `job_status` list
//! (their `background_jobs` projection). `job:*` therefore carries `tool` and
//! `group` lifecycle today.
//!
//! These are best-effort UI signals (no bus ⇒ silently dropped); job correctness
//! never depends on an event being delivered.

use serde_json::json;

use super::types::JobKind;

fn emit(event: &str, payload: serde_json::Value) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(event, payload);
    }
}

/// A new background job has been created (running or queued). Lets the panel show
/// it appear without waiting for the first status change.
pub(crate) fn emit_created(
    job_id: &str,
    kind: JobKind,
    tool: &str,
    status: &str,
    session_id: Option<&str>,
) {
    emit(
        "job:created",
        json!({ "job_id": job_id, "kind": kind.as_str(), "tool": tool, "status": status, "session_id": session_id }),
    );
}

/// A non-terminal status transition (e.g. `running` → `cancelling`).
pub(crate) fn emit_updated(
    job_id: &str,
    kind: JobKind,
    tool: &str,
    status: &str,
    session_id: Option<&str>,
) {
    emit(
        "job:updated",
        json!({ "job_id": job_id, "kind": kind.as_str(), "tool": tool, "status": status, "session_id": session_id }),
    );
}

/// A terminal status (completed / failed / timed_out / cancelled / interrupted).
pub(crate) fn emit_completed(
    job_id: &str,
    kind: JobKind,
    tool: &str,
    status: &str,
    session_id: Option<&str>,
) {
    emit(
        "job:completed",
        json!({ "job_id": job_id, "kind": kind.as_str(), "tool": tool, "status": status, "session_id": session_id }),
    );
}

/// In-flight progress: `current` of `total` units done. Used by `Group` (N of M
/// children settled); other kinds may report bytes/rounds in a later slice.
pub(crate) fn emit_progress(
    job_id: &str,
    kind: JobKind,
    session_id: Option<&str>,
    current: usize,
    total: usize,
) {
    emit(
        "job:progress",
        json!({ "job_id": job_id, "kind": kind.as_str(), "session_id": session_id, "current": current, "total": total }),
    );
}

/// Alarm: a terminal job's `injected` flag could not be persisted after retries,
/// so a restart may re-inject it (duplicate `<task-notification>`). Surfaced for
/// observability — was `async_tool_job:mark_injected_failed`.
pub(crate) fn emit_mark_injected_failed(job_id: &str, error: &str) {
    emit(
        "job:mark_injected_failed",
        json!({ "job_id": job_id, "error": error }),
    );
}
