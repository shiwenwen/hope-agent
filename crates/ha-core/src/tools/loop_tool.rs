use std::sync::Arc;

use serde_json::{json, Value};

use crate::cron::CronDB;
use crate::loop_control::{
    LoopProgressState, LoopSchedule, LoopSnapshot, LoopState, LoopTriggerKind, LoopWatchKind,
};
use crate::session::SessionDB;

use super::ToolExecContext;

const LOOP_TOOL_METADATA_MAX_BYTES: usize = 16 * 1024;

fn json_string(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

fn error_json(message: impl Into<String>) -> String {
    json_string(json!({
        "ok": false,
        "error": message.into(),
    }))
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn resolve_ctx(ctx: &ToolExecContext) -> Result<(String, Arc<SessionDB>), String> {
    if ctx.incognito {
        return Err(
            "Loop tools are disabled for incognito sessions because loops are durable.".to_string(),
        );
    }
    let session_id = ctx
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "No active session is available for Loop tools.".to_string())?
        .to_string();
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| "Session database is unavailable for Loop tools.".to_string())?;
    Ok((session_id, db))
}

fn resolve_cron_db() -> Result<Arc<CronDB>, String> {
    crate::get_cron_db()
        .cloned()
        .ok_or_else(|| "Cron database is unavailable for Loop tools.".to_string())
}

fn resolve_loop_for_session(
    db: &SessionDB,
    session_id: &str,
    loop_id: Option<&str>,
    prefer_dynamic: bool,
    active_only: bool,
) -> Result<LoopSchedule, String> {
    let schedules = db
        .list_loop_schedules_for_session(session_id, 200)
        .map_err(|e| format!("Failed to list loops: {e}"))?;
    let matches: Vec<LoopSchedule> = if let Some(loop_id) = loop_id {
        schedules
            .into_iter()
            .filter(|schedule| schedule.id == loop_id || schedule.id.starts_with(loop_id))
            .collect()
    } else {
        let mut candidates: Vec<LoopSchedule> = schedules
            .into_iter()
            .filter(|schedule| {
                if active_only {
                    schedule.state == LoopState::Active
                } else {
                    !schedule.state.is_terminal()
                }
            })
            .collect();
        if prefer_dynamic {
            let dynamic: Vec<LoopSchedule> = candidates
                .iter()
                .filter(|schedule| schedule.trigger_kind == LoopTriggerKind::Dynamic)
                .cloned()
                .collect();
            if !dynamic.is_empty() {
                candidates = dynamic;
            }
        }
        candidates
    };

    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err("No matching loop exists for this session.".to_string()),
        _ => Err(format!(
            "Multiple loops match; pass a longer loopId. Matches: {}",
            matches
                .iter()
                .take(8)
                .map(|schedule| short_id(&schedule.id))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn compact_schedule(schedule: &LoopSchedule) -> Value {
    json!({
        "id": schedule.id,
        "state": schedule.state.as_str(),
        "triggerKind": schedule.trigger_kind.as_str(),
        "executionStrategy": schedule.execution_strategy.as_str(),
        "runCount": schedule.run_count,
        "maxRuns": schedule.max_runs,
        "nextRunAt": schedule.next_run_at,
        "cronStatus": schedule.cron_status,
        "progressState": schedule.progress_state.map(|state| state.as_str()),
        "progressSummary": schedule.progress_summary,
        "noProgressStreak": schedule.no_progress_streak,
        "failureStreak": schedule.failure_streak,
        "blockedReason": schedule.blocked_reason,
        "promptPreview": truncate(&schedule.prompt, 240),
        "goalId": schedule.goal_id,
        "goalCriterionId": schedule.goal_criterion_id,
        "updatedAt": schedule.updated_at,
    })
}

fn compact_snapshot(snapshot: &LoopSnapshot) -> Value {
    let runs: Vec<Value> = snapshot
        .runs
        .iter()
        .take(8)
        .map(|run| {
            json!({
                "id": run.id,
                "seq": run.seq,
                "state": run.state.as_str(),
                "progressState": run.progress_state.map(|state| state.as_str()),
                "schedulingDecision": run.scheduling_decision,
                "summary": run.result_summary,
                "error": run.error,
                "noProgressReason": run.no_progress_reason,
                "dynamicDecision": run.trace.get("dynamicDecision").cloned().unwrap_or(Value::Null),
                "startedAt": run.started_at,
                "finishedAt": run.finished_at,
            })
        })
        .collect();
    json!({
        "schedule": compact_schedule(&snapshot.schedule),
        "runs": runs,
        "watches": snapshot.watches.iter().map(|watch| json!({
            "id": watch.id,
            "kind": watch.kind.as_str(),
            "spec": watch.spec,
            "active": watch.active,
            "generation": watch.generation,
            "lastEventAt": watch.last_event_at,
            "failureCount": watch.failure_count,
            "lastError": watch.last_error,
            "monitorJobId": watch.monitor_job_id,
        })).collect::<Vec<_>>(),
    })
}

fn metadata_arg(args: &Value) -> Result<Value, String> {
    let metadata = args.get("metadata").cloned().unwrap_or_else(|| json!({}));
    let encoded =
        serde_json::to_string(&metadata).map_err(|e| format!("Failed to encode metadata: {e}"))?;
    if encoded.len() > LOOP_TOOL_METADATA_MAX_BYTES {
        return Err(format!(
            "metadata exceeds {LOOP_TOOL_METADATA_MAX_BYTES} bytes"
        ));
    }
    Ok(metadata)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn truncate(input: &str, max: usize) -> &str {
    if input.len() <= max {
        return input;
    }
    let mut end = max;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

pub(crate) async fn tool_loop_status(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let loop_id = string_arg(args, "loopId");
    if let Some(loop_id) = loop_id.as_deref() {
        let schedule = match resolve_loop_for_session(&db, &session_id, Some(loop_id), false, false)
        {
            Ok(schedule) => schedule,
            Err(err) => return error_json(err),
        };
        let snapshot = match crate::get_cron_db() {
            Some(cron_db) => db.loop_snapshot_with_cron(cron_db, &schedule.id, 8),
            None => db.loop_snapshot(&schedule.id, 8),
        };
        let snapshot = match snapshot {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return error_json("Loop not found."),
            Err(e) => return error_json(format!("Failed to read loop: {e}")),
        };
        return json_string(json!({
            "ok": true,
            "loop": compact_snapshot(&snapshot),
        }));
    }

    let schedules = match crate::get_cron_db() {
        Some(cron_db) => db.list_loop_schedules_for_session_with_cron(cron_db, &session_id, 50),
        None => db.list_loop_schedules_for_session(&session_id, 50),
    };
    let schedules = match schedules {
        Ok(items) => items,
        Err(e) => return error_json(format!("Failed to list loops: {e}")),
    };
    json_string(json!({
        "ok": true,
        "loops": schedules.iter().map(compact_schedule).collect::<Vec<_>>(),
    }))
}

pub(crate) async fn tool_loop_reschedule(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let cron_db = match resolve_cron_db() {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let delay_secs = match args.get("delaySecs").and_then(Value::as_i64) {
        Some(value) if value > 0 => value,
        _ => return error_json("delaySecs is required and must be a positive integer."),
    };
    let reason = match string_arg(args, "reason") {
        Some(value) => value,
        None => return error_json("reason is required."),
    };
    let loop_id = string_arg(args, "loopId");
    let schedule = match resolve_loop_for_session(&db, &session_id, loop_id.as_deref(), true, true)
    {
        Ok(schedule) => schedule,
        Err(err) => return error_json(err),
    };
    match db.record_loop_tool_reschedule(&cron_db, &schedule.id, delay_secs, &reason) {
        Ok((schedule, next_run_at)) => json_string(json!({
            "ok": true,
            "loop": compact_schedule(&schedule),
            "decision": {
                "action": "reschedule",
                "delaySecs": delay_secs.clamp(60, 3600),
                "reason": reason,
                "nextRunAt": next_run_at.or(schedule.next_run_at),
            }
        })),
        Err(e) => error_json(format!("Failed to reschedule loop: {e}")),
    }
}

pub(crate) async fn tool_loop_stop(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let cron_db = match resolve_cron_db() {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let reason = match string_arg(args, "reason") {
        Some(value) => value,
        None => return error_json("reason is required."),
    };
    let outcome = string_arg(args, "outcome").unwrap_or_else(|| "completed".to_string());
    let completed = match outcome.as_str() {
        "completed" => true,
        "blocked" => false,
        other => {
            return error_json(format!(
                "Invalid outcome `{other}`; use completed or blocked."
            ))
        }
    };
    let loop_id = string_arg(args, "loopId");
    let schedule =
        match resolve_loop_for_session(&db, &session_id, loop_id.as_deref(), false, false) {
            Ok(schedule) => schedule,
            Err(err) => return error_json(err),
        };
    match db.record_loop_tool_stop(&cron_db, &schedule.id, completed, &reason) {
        Ok(schedule) => json_string(json!({
            "ok": true,
            "loop": compact_schedule(&schedule),
            "decision": {
                "action": if completed { "completed" } else { "blocked" },
                "reason": reason,
            }
        })),
        Err(e) => error_json(format!("Failed to stop loop: {e}")),
    }
}

pub(crate) async fn tool_loop_record_progress(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let summary = match string_arg(args, "summary") {
        Some(value) => value,
        None => return error_json("summary is required."),
    };
    let state = string_arg(args, "state")
        .as_deref()
        .and_then(LoopProgressState::from_str)
        .unwrap_or(LoopProgressState::WeakProgress);
    let reason = string_arg(args, "reason");
    let metadata = match metadata_arg(args) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let loop_id = string_arg(args, "loopId");
    let schedule =
        match resolve_loop_for_session(&db, &session_id, loop_id.as_deref(), false, false) {
            Ok(schedule) => schedule,
            Err(err) => return error_json(err),
        };
    match db.record_loop_tool_progress(&schedule.id, state, &summary, reason.as_deref(), metadata) {
        Ok(schedule) => json_string(json!({
            "ok": true,
            "loop": compact_schedule(&schedule),
            "recorded": {
                "state": state.as_str(),
                "summary": summary,
                "reason": reason,
            }
        })),
        Err(e) => error_json(format!("Failed to record loop progress: {e}")),
    }
}

pub(crate) async fn tool_loop_watch(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let kind = match string_arg(args, "kind")
        .as_deref()
        .and_then(LoopWatchKind::from_str)
    {
        Some(value) => value,
        None => return error_json(
            "kind is required and must be app_event, job, subagent, file, command, or websocket.",
        ),
    };
    let mut spec = args.get("spec").cloned().unwrap_or_else(|| json!({}));
    if kind == LoopWatchKind::File {
        let Some(raw_path) = spec.get("path").and_then(Value::as_str) else {
            return error_json("file watch requires spec.path.");
        };
        spec["path"] = Value::String(ctx.resolve_path(raw_path));
    }
    let loop_id = string_arg(args, "loopId");
    let schedule = match resolve_loop_for_session(&db, &session_id, loop_id.as_deref(), true, true)
    {
        Ok(schedule) => schedule,
        Err(err) => return error_json(err),
    };
    match db.upsert_loop_watch(&schedule.id, kind, &spec) {
        Ok(watch) => {
            let cron_db = match resolve_cron_db() {
                Ok(value) => value,
                Err(err) => return error_json(err),
            };
            if let Err(err) =
                crate::loop_control::start_loop_monitor_adapter(db.clone(), cron_db, &watch).await
            {
                let _ = db.record_loop_monitor_error(&watch.id, &err.to_string());
                return error_json(format!(
                    "Loop watch was persisted but its monitor could not start; the fallback remains active: {err}"
                ));
            }
            json_string(json!({
                "ok": true,
                "loopId": schedule.id,
                "watch": {
                    "id": watch.id,
                    "kind": watch.kind.as_str(),
                    "spec": watch.spec,
                    "active": watch.active,
                    "generation": watch.generation,
                },
                "fallbackAt": schedule.next_run_at,
            }))
        }
        Err(e) => error_json(format!("Failed to attach loop watch: {e}")),
    }
}

pub(crate) async fn tool_loop_unwatch(args: &Value, ctx: &ToolExecContext) -> String {
    let (session_id, db) = match resolve_ctx(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let watch_id = match string_arg(args, "watchId") {
        Some(value) => value,
        None => return error_json("watchId is required."),
    };
    let loop_id = string_arg(args, "loopId");
    let schedule = match resolve_loop_for_session(&db, &session_id, loop_id.as_deref(), true, false)
    {
        Ok(schedule) => schedule,
        Err(err) => return error_json(err),
    };
    match db.remove_loop_watch(&schedule.id, &watch_id) {
        Ok(watch) => json_string(json!({
            "ok": true,
            "loopId": schedule.id,
            "watch": {
                "id": watch.id,
                "active": watch.active,
                "generation": watch.generation,
            }
        })),
        Err(e) => error_json(format!("Failed to remove loop watch: {e}")),
    }
}
