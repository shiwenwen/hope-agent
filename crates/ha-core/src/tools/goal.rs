use std::sync::Arc;

use serde_json::{json, Value};

use crate::goal::{
    build_goal_completion_report, CloseGoalInput, GoalClosureDecision, GoalSnapshot, GoalState,
};
use crate::session::SessionDB;

use super::ToolExecContext;

const GOAL_EVIDENCE_METADATA_MAX_BYTES: usize = 16 * 1024;

fn json_string(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

fn error_json(message: impl Into<String>) -> String {
    json_string(json!({
        "ok": false,
        "error": message.into(),
    }))
}

fn resolve_ctx(ctx: &ToolExecContext) -> Result<(String, Arc<SessionDB>), String> {
    if ctx.incognito {
        return Err(
            "Goal tools are disabled for incognito sessions because goals are durable.".to_string(),
        );
    }
    let session_id = ctx
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "No active session is available for Goal tools.".to_string())?
        .to_string();
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| "Session database is unavailable for Goal tools.".to_string())?;
    Ok((session_id, db))
}

fn active_goal(ctx: &ToolExecContext) -> Result<(String, Arc<SessionDB>, GoalSnapshot), String> {
    let (session_id, db) = resolve_ctx(ctx)?;
    let snapshot = db
        .active_goal_for_session(&session_id)
        .map_err(|e| format!("Failed to read active goal: {e}"))?
        .ok_or_else(|| "No active goal exists for this session.".to_string())?;
    Ok((session_id, db, snapshot))
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn string_array_arg(args: &Value, key: &str, max: usize) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(max)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn ensure_json_budget(value: &Value, label: &str, max_bytes: usize) -> Result<(), String> {
    let encoded =
        serde_json::to_string(value).map_err(|e| format!("Failed to encode {label}: {e}"))?;
    if encoded.len() > max_bytes {
        return Err(format!("{label} exceeds {max_bytes} bytes"));
    }
    Ok(())
}

fn compact_goal_status(snapshot: &GoalSnapshot) -> Value {
    let goal = &snapshot.goal;
    let required_missing: Vec<Value> = snapshot
        .criteria
        .iter()
        .filter(|criterion| {
            criterion.kind.as_str() == "required" && criterion.status.as_str() != "satisfied"
        })
        .take(12)
        .map(|criterion| {
            json!({
                "id": criterion.id,
                "text": criterion.text,
                "status": criterion.status.as_str(),
                "reason": criterion.reason,
                "evidenceIds": criterion.evidence_ids,
            })
        })
        .collect();
    let latest_events: Vec<Value> = snapshot
        .events
        .iter()
        .rev()
        .take(8)
        .map(|event| {
            json!({
                "seq": event.seq,
                "kind": event.kind,
                "createdAt": event.created_at,
                "summary": event.payload.get("summary").cloned().unwrap_or(Value::Null),
                "status": event.payload.get("status").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    json!({
        "ok": true,
        "goal": {
            "id": goal.id,
            "state": goal.state.as_str(),
            "revision": goal.revision,
            "objective": goal.objective,
            "completionCriteria": goal.completion_criteria,
            "blockedReason": goal.blocked_reason,
            "closureDecision": goal.closure_decision.map(|decision| decision.as_str()),
            "closureReason": goal.closure_reason,
            "updatedAt": goal.updated_at,
        },
        "audit": {
            "stale": snapshot.audit_stale,
            "status": goal.final_evidence.get("status").and_then(Value::as_str),
            "summary": goal.final_summary,
            "missing": goal.final_evidence.get("missing").cloned().unwrap_or(Value::Null),
            "blockers": goal.final_evidence.get("blockers").cloned().unwrap_or(Value::Null),
            "nextEvidenceNeeded": goal.final_evidence.get("nextEvidenceNeeded").cloned().unwrap_or(Value::Null),
        },
        "criteria": {
            "items": snapshot.criteria_items,
            "requiredMissing": required_missing,
        },
        "evidence": {
            "count": snapshot.evidence.len(),
            "latest": snapshot.evidence.iter().rev().take(12).collect::<Vec<_>>(),
        },
        "budget": snapshot.budget,
        "tasks": {
            "total": snapshot.tasks.len(),
            "open": snapshot.tasks.iter().filter(|task| task.status != "completed").count(),
            "completed": snapshot.tasks.iter().filter(|task| task.status == "completed").count(),
        },
        "workflowRuns": {
            "total": snapshot.workflow_runs.len(),
            "open": snapshot.workflow_runs.iter().filter(|run| !run.state.is_terminal()).count(),
        },
        "latestEvents": latest_events,
    })
}

pub(crate) async fn tool_goal_status(_args: &Value, ctx: &ToolExecContext) -> String {
    let (_, _, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    json_string(compact_goal_status(&snapshot))
}

pub(crate) async fn tool_goal_checkpoint(args: &Value, ctx: &ToolExecContext) -> String {
    let summary = match string_arg(args, "summary") {
        Some(value) => value,
        None => return error_json("summary is required"),
    };
    let status = string_arg(args, "status").unwrap_or_else(|| "progress".to_string());
    let (_, db, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let event = match db.append_goal_event(
        &snapshot.goal.id,
        "goal_checkpoint",
        json!({
            "summary": summary,
            "status": status,
            "next": string_arg(args, "next"),
            "evidence": string_array_arg(args, "evidence", 16),
            "confidence": string_arg(args, "confidence"),
            "goalRevision": snapshot.goal.revision,
        }),
    ) {
        Ok(event) => event,
        Err(e) => return error_json(format!("Failed to record goal checkpoint: {e}")),
    };
    json_string(json!({
        "ok": true,
        "goalId": snapshot.goal.id,
        "checkpointSeq": event.seq,
        "state": snapshot.goal.state.as_str(),
    }))
}

pub(crate) async fn tool_goal_record_evidence(args: &Value, ctx: &ToolExecContext) -> String {
    let relation = match string_arg(args, "relation") {
        Some(value) => value,
        None => return error_json("relation is required"),
    };
    if !matches!(
        relation.as_str(),
        "source_cited"
            | "claim_checked"
            | "user_decision"
            | "artifact_reviewed"
            | "data_quality_checked"
            | "citation_audited"
            | "message_draft_approved"
            | "meeting_context_collected"
            | "review_completed"
            | "review_passed"
            | "review_finding"
    ) {
        return error_json(format!(
            "relation is not allowed for general goal evidence: {relation}"
        ));
    }
    let title = match string_arg(args, "title") {
        Some(value) => value,
        None => return error_json("title is required"),
    };
    let summary = match string_arg(args, "summary") {
        Some(value) => value,
        None => return error_json("summary is required"),
    };
    let (_, db, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let criterion_id = string_arg(args, "goalCriterionId");
    let criterion =
        match db.resolve_goal_criterion_binding(&snapshot.goal.id, criterion_id.as_deref()) {
            Ok(value) => value,
            Err(e) => return error_json(format!("Invalid goal criterion binding: {e}")),
        };
    let source_id = string_arg(args, "sourceId")
        .unwrap_or_else(|| format!("goal_evidence_{}", uuid::Uuid::new_v4().simple()));
    let mut metadata = args
        .get("metadata")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    metadata["title"] = json!(title);
    metadata["summary"] = json!(summary);
    metadata["source"] = json!("goal_record_evidence");
    metadata["goalRevision"] = json!(snapshot.goal.revision);
    if let Some(criterion) = criterion {
        metadata["goalCriterionId"] = json!(criterion.id);
        metadata["goalCriterion"] = json!(criterion.text);
        metadata["goalCriterionKind"] = json!(criterion.kind.as_str());
        metadata["goalCriterionRevision"] = json!(criterion.goal_revision);
    }
    if let Err(err) = ensure_json_budget(
        &metadata,
        "goal evidence metadata",
        GOAL_EVIDENCE_METADATA_MAX_BYTES,
    ) {
        return error_json(err);
    }
    let link = match db.link_goal_target(
        &snapshot.goal.id,
        "general",
        &source_id,
        &relation,
        metadata,
    ) {
        Ok(link) => link,
        Err(e) => return error_json(format!("Failed to attach goal evidence: {e}")),
    };
    let refreshed = db
        .goal_snapshot(&snapshot.goal.id, 100)
        .ok()
        .flatten()
        .unwrap_or(snapshot);
    json_string(json!({
        "ok": true,
        "goalId": refreshed.goal.id,
        "evidenceLinkId": link.id,
        "evidenceCount": refreshed.evidence.len(),
        "auditStale": refreshed.audit_stale,
    }))
}

pub(crate) async fn tool_goal_evaluate(args: &Value, ctx: &ToolExecContext) -> String {
    let (_, db, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let _ = db.append_goal_event(
        &snapshot.goal.id,
        "goal_evaluate_requested",
        json!({
            "reason": string_arg(args, "reason"),
            "goalRevision": snapshot.goal.revision,
        }),
    );
    let evaluated = match db.evaluate_goal(&snapshot.goal.id) {
        Ok(snapshot) => snapshot,
        Err(e) => return error_json(format!("Goal evaluation failed: {e}")),
    };
    json_string(json!({
        "ok": true,
        "status": evaluated.goal.final_evidence.get("status").and_then(Value::as_str),
        "state": evaluated.goal.state.as_str(),
        "summary": evaluated.goal.final_summary,
        "auditStale": evaluated.audit_stale,
        "report": build_goal_completion_report(&evaluated, None),
        "missing": evaluated.goal.final_evidence.get("missing").cloned().unwrap_or(Value::Null),
        "blockers": evaluated.goal.final_evidence.get("blockers").cloned().unwrap_or(Value::Null),
        "nextEvidenceNeeded": evaluated.goal.final_evidence.get("nextEvidenceNeeded").cloned().unwrap_or(Value::Null),
    }))
}

pub(crate) async fn tool_goal_finish_request(args: &Value, ctx: &ToolExecContext) -> String {
    let summary = string_arg(args, "summary");
    let follow_up_items = string_array_arg(args, "followUpItems", 20);
    let remaining_risk = string_arg(args, "remainingRisk");
    let (_, db, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let _ = db.append_goal_event(
        &snapshot.goal.id,
        "goal_finish_requested",
        json!({
            "summary": summary,
            "remainingRisk": remaining_risk,
            "followUpItems": follow_up_items,
            "goalRevision": snapshot.goal.revision,
        }),
    );

    let evaluated = if snapshot.goal.state == GoalState::Completed && !snapshot.audit_stale {
        snapshot
    } else {
        match db.evaluate_goal(&snapshot.goal.id) {
            Ok(snapshot) => snapshot,
            Err(e) => return error_json(format!("Goal finish evaluation failed: {e}")),
        }
    };
    let final_status = evaluated
        .goal
        .final_evidence
        .get("status")
        .and_then(Value::as_str);
    if final_status != Some("completed") {
        let _ = db.append_goal_event(
            &evaluated.goal.id,
            "goal_finish_rejected",
            json!({
                "reason": "final_audit_not_completed",
                "status": final_status,
                "missing": evaluated.goal.final_evidence.get("missing").cloned().unwrap_or(Value::Null),
                "blockers": evaluated.goal.final_evidence.get("blockers").cloned().unwrap_or(Value::Null),
            }),
        );
        return json_string(json!({
            "ok": false,
            "status": "not_ready",
            "state": evaluated.goal.state.as_str(),
            "summary": evaluated.goal.final_summary,
            "missing": evaluated.goal.final_evidence.get("missing").cloned().unwrap_or(Value::Null),
            "blockers": evaluated.goal.final_evidence.get("blockers").cloned().unwrap_or(Value::Null),
            "nextEvidenceNeeded": evaluated.goal.final_evidence.get("nextEvidenceNeeded").cloned().unwrap_or(Value::Null),
            "message": "The goal was not closed because the current audit did not pass.",
        }));
    }

    let closed = match db.close_goal(CloseGoalInput {
        goal_id: evaluated.goal.id.clone(),
        decision: GoalClosureDecision::AcceptedV1,
        reason: summary
            .clone()
            .or_else(|| Some("goal_finish_request".to_string())),
        follow_up_items,
    }) {
        Ok(snapshot) => snapshot,
        Err(e) => return error_json(format!("Goal close failed: {e}")),
    };
    let mut report = build_goal_completion_report(&closed, summary.as_deref());
    if remaining_risk.is_some() {
        report.remaining_risk = remaining_risk;
    }
    json_string(json!({
        "ok": true,
        "status": "completed",
        "state": closed.goal.state.as_str(),
        "report": report,
    }))
}

pub(crate) async fn tool_goal_block_request(args: &Value, ctx: &ToolExecContext) -> String {
    let reason = match string_arg(args, "reason") {
        Some(value) => value,
        None => return error_json("reason is required"),
    };
    let attempted = string_array_arg(args, "attempted", 20);
    if attempted.is_empty() {
        return error_json("attempted must include at least one concrete attempt");
    }
    let needed = string_arg(args, "needed");
    let fingerprint = string_arg(args, "fingerprint").unwrap_or_else(|| {
        reason
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    });
    let needs_user_input = bool_arg(args, "needsUserInput");
    let external_state_required = bool_arg(args, "externalStateRequired");
    let (_, db, snapshot) = match active_goal(ctx) {
        Ok(value) => value,
        Err(err) => return error_json(err),
    };
    let previous_same = snapshot
        .events
        .iter()
        .filter(|event| {
            event.kind == "goal_block_requested"
                && event
                    .payload
                    .get("fingerprint")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == fingerprint)
        })
        .count();
    let event = match db.append_goal_event(
        &snapshot.goal.id,
        "goal_block_requested",
        json!({
            "reason": reason,
            "attempted": attempted,
            "needed": needed,
            "fingerprint": fingerprint,
            "needsUserInput": needs_user_input,
            "externalStateRequired": external_state_required,
            "repeatCount": previous_same + 1,
            "goalRevision": snapshot.goal.revision,
        }),
    ) {
        Ok(event) => event,
        Err(e) => return error_json(format!("Failed to record block request: {e}")),
    };
    let should_block = needs_user_input || external_state_required || previous_same + 1 >= 3;
    if !should_block {
        return json_string(json!({
            "ok": true,
            "status": "recorded",
            "state": snapshot.goal.state.as_str(),
            "blockRequestSeq": event.seq,
            "repeatCount": previous_same + 1,
            "requiredRepeatCount": 3,
            "message": "Block request recorded, but the goal remains open. Continue if there is any safe meaningful progress left.",
        }));
    }
    let blocked = match db.transition_goal(&snapshot.goal.id, GoalState::Blocked, Some(&reason)) {
        Ok(snapshot) => snapshot,
        Err(e) => return error_json(format!("Failed to mark goal blocked: {e}")),
    };
    json_string(json!({
        "ok": true,
        "status": "blocked",
        "state": blocked.goal.state.as_str(),
        "blockedReason": blocked.goal.blocked_reason,
        "blockRequestSeq": event.seq,
    }))
}
