use std::{collections::HashSet, sync::Arc};

use crate::goal::{
    CloseGoalInput, CreateGoalInput, GoalClosureDecision, GoalCriterionKind, GoalCriterionStatus,
    GoalSnapshot, GoalState, UpdateGoalInput,
};
use crate::plan::{self, PlanModeState, TransitionOutcome};
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use serde_json::Value;

pub async fn handle_goal(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let trimmed = args.trim();
    if trimmed.is_empty() || matches!(trimmed, "status" | "show") {
        return render_active_goal(session_db, sid);
    }

    match parse_goal_request(trimmed) {
        GoalRequest::Show => render_active_goal(session_db, sid),
        GoalRequest::Help => Ok(display_only(goal_usage())),
        GoalRequest::Transition(command) => transition_active_goal(session_db, sid, command),
        GoalRequest::Upsert(raw) => {
            exit_plan_for_goal_upsert(sid).await?;
            upsert_goal(session_db, sid, raw)?;
            let result = CommandResult {
                content: "Goal updated. Starting a normal model turn for the active goal."
                    .to_string(),
                action: Some(CommandAction::PassThrough {
                    message: raw.trim().to_string(),
                }),
            };
            Ok(result)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalCommand {
    Pause,
    Resume,
    Clear,
    Evaluate,
    Accept,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalRequest<'a> {
    Show,
    Help,
    Transition(GoalCommand),
    Upsert(&'a str),
}

fn parse_goal_request(trimmed: &str) -> GoalRequest<'_> {
    match trimmed {
        "" | "status" | "show" => GoalRequest::Show,
        "help" => GoalRequest::Help,
        "pause" => GoalRequest::Transition(GoalCommand::Pause),
        "resume" => GoalRequest::Transition(GoalCommand::Resume),
        "clear" | "cancel" => GoalRequest::Transition(GoalCommand::Clear),
        "evaluate" | "audit" => GoalRequest::Transition(GoalCommand::Evaluate),
        "accept" | "close" | "done" => GoalRequest::Transition(GoalCommand::Accept),
        "strict" | "needs-strict-evidence" | "needs_strict_evidence" => {
            GoalRequest::Transition(GoalCommand::Strict)
        }
        objective => GoalRequest::Upsert(objective),
    }
}

async fn exit_plan_for_goal_upsert(sid: &str) -> Result<(), String> {
    if plan::get_plan_state(sid).await == PlanModeState::Off {
        return Ok(());
    }
    match plan::transition_state(sid, PlanModeState::Off, "slash_goal_upsert").await {
        Ok(TransitionOutcome::Applied) => Ok(()),
        Ok(TransitionOutcome::Rejected) => {
            Err("Cannot create or update a goal while plan mode is active.".to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn upsert_goal(session_db: &Arc<SessionDB>, sid: &str, raw: &str) -> Result<CommandResult, String> {
    let (objective, completion_criteria) = parse_goal_create_args(raw);
    if objective.trim().is_empty() && completion_criteria.trim().is_empty() {
        return Err(goal_usage());
    }

    if let Some(snapshot) = session_db
        .active_goal_for_session(sid)
        .map_err(|e| e.to_string())?
    {
        let next = session_db
            .update_goal(UpdateGoalInput {
                goal_id: snapshot.goal.id,
                objective: (!objective.trim().is_empty()).then_some(objective),
                completion_criteria: (!completion_criteria.trim().is_empty())
                    .then_some(completion_criteria),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
            })
            .map_err(|e| e.to_string())?;
        return Ok(display_only(goal_upsert_summary(&next)));
    }

    if objective.trim().is_empty() {
        return Err(goal_usage());
    }
    let snapshot = session_db
        .create_goal(CreateGoalInput {
            session_id: sid.to_string(),
            objective,
            completion_criteria,
            domain: None,
            workflow_template_id: None,
            workflow_template_version: None,
            workflow_task_type: None,
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })
        .map_err(|e| e.to_string())?;
    Ok(display_only(goal_upsert_summary(&snapshot)))
}

fn transition_active_goal(
    session_db: &Arc<SessionDB>,
    sid: &str,
    command: GoalCommand,
) -> Result<CommandResult, String> {
    let snapshot = session_db
        .active_goal_for_session(sid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "No active goal for this session. Use `/goal <objective> --criteria <criteria>`."
                .to_string()
        })?;
    let next = match command {
        GoalCommand::Pause => session_db.pause_goal(&snapshot.goal.id),
        GoalCommand::Resume => session_db.resume_goal(&snapshot.goal.id),
        GoalCommand::Clear => session_db.clear_goal(&snapshot.goal.id),
        GoalCommand::Evaluate => session_db.evaluate_goal(&snapshot.goal.id),
        GoalCommand::Accept => session_db.close_goal(CloseGoalInput {
            goal_id: snapshot.goal.id,
            decision: GoalClosureDecision::AcceptedV1,
            reason: Some("User accepted the current audit and remaining risk.".to_string()),
            follow_up_items: final_audit_follow_up_texts(&snapshot.goal.final_evidence),
        }),
        GoalCommand::Strict => session_db.close_goal(CloseGoalInput {
            goal_id: snapshot.goal.id,
            decision: GoalClosureDecision::NeedsStrictEvidence,
            reason: Some("User requested stricter evidence before closing the goal.".to_string()),
            follow_up_items: Vec::new(),
        }),
    }
    .map_err(|e| e.to_string())?;
    Ok(display_only(render_goal_snapshot(&next)))
}

fn final_audit_follow_up_texts(final_evidence: &Value) -> Vec<String> {
    let Some(items) = final_evidence
        .get("followUpItems")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut texts = Vec::new();
    for item in items {
        let text = item
            .as_str()
            .or_else(|| item.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let Some(text) = text else {
            continue;
        };
        let key = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if seen.insert(key) {
            texts.push(text.to_string());
        }
    }
    texts
}

fn render_active_goal(session_db: &Arc<SessionDB>, sid: &str) -> Result<CommandResult, String> {
    match session_db.active_goal_for_session(sid).map_err(|e| e.to_string())? {
        Some(snapshot) => Ok(display_only(render_goal_snapshot(&snapshot))),
        None => Ok(display_only(
            "No active goal for this session.\n\nUse `/goal <objective> --criteria <completion criteria>` to create one.",
        )),
    }
}

fn parse_goal_create_args(raw: &str) -> (String, String) {
    let markers = [
        "--criteria",
        "criteria:",
        "completion criteria:",
        "完成标准：",
        "完成标准:",
    ];
    let lower = raw.to_lowercase();
    for marker in markers {
        let needle = marker.to_lowercase();
        if let Some(index) = lower.find(&needle) {
            let objective = raw[..index].trim().trim_matches('-').trim().to_string();
            let criteria = raw[index + marker.len()..]
                .trim()
                .trim_start_matches(':')
                .trim()
                .to_string();
            return (objective, criteria);
        }
    }
    (raw.trim().to_string(), String::new())
}

fn render_goal_snapshot(snapshot: &GoalSnapshot) -> String {
    let goal = &snapshot.goal;
    let state = goal_state_label(goal.state);
    let workflows = snapshot.workflow_runs.len();
    let tasks_total = snapshot.tasks.len();
    let tasks_done = snapshot
        .tasks
        .iter()
        .filter(|task| task.status == "completed")
        .count();
    let required_total = snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.kind == GoalCriterionKind::Required)
        .count();
    let required_done = snapshot
        .criteria
        .iter()
        .filter(|criterion| {
            criterion.kind == GoalCriterionKind::Required
                && criterion.status == GoalCriterionStatus::Satisfied
        })
        .count();
    let required_progress = if required_total > 0 {
        format!(" · required {required_done}/{required_total}")
    } else {
        String::new()
    };
    let usage = format!(
        "elapsed {} · {} tokens · {} turns",
        format_duration_secs(snapshot.budget.elapsed_secs),
        format_count(snapshot.budget.tokens_used),
        format_count(snapshot.budget.turns_used)
    );
    let evidence = snapshot.evidence.len();
    let closure = goal
        .closure_decision
        .map(|decision| decision.as_str())
        .unwrap_or("pending");
    let audit = render_goal_audit_summary(snapshot);
    let criteria = render_goal_criteria_summary(snapshot);
    let blocked = goal
        .blocked_reason
        .as_deref()
        .filter(|reason| !reason.trim().is_empty())
        .map(|reason| format!("\n\nBlocked: `{reason}`"))
        .unwrap_or_default();

    format!(
        "## Active Goal\n\nState: **{}** · r{}{} · {}\nWorkflows: {} · tasks: {}/{} · evidence: {} · closure: `{}`{}\n\n**Objective**\n{}\n\n**Progress**\n{}\n\n**Latest evaluator**\n{}",
        state,
        goal.revision,
        required_progress,
        usage,
        workflows,
        tasks_done,
        tasks_total,
        evidence,
        closure,
        blocked,
        goal.objective,
        criteria,
        audit,
    )
}

fn goal_upsert_summary(snapshot: &GoalSnapshot) -> String {
    format!(
        "Goal is active: {}",
        snapshot.goal.objective.replace('\n', " ")
    )
}

fn goal_state_label(state: GoalState) -> &'static str {
    match state {
        GoalState::Active => "active",
        GoalState::Paused => "paused",
        GoalState::Evaluating => "evaluating",
        GoalState::Completed => "completed",
        GoalState::Failed => "failed",
        GoalState::Cancelled => "cancelled",
        GoalState::Blocked => "blocked",
    }
}

fn render_goal_criteria_summary(snapshot: &GoalSnapshot) -> String {
    if snapshot.criteria.is_empty() {
        if snapshot.goal.completion_criteria.trim().is_empty() {
            return "- No explicit completion criteria yet.".to_string();
        }
        return snapshot
            .goal
            .completion_criteria
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(6)
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut lines = Vec::new();
    for criterion in snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.kind == GoalCriterionKind::Required)
    {
        let marker = match criterion.status {
            GoalCriterionStatus::Satisfied => "[done]",
            GoalCriterionStatus::Missing => "[missing]",
            GoalCriterionStatus::Blocked => "[blocked]",
        };
        let reason = criterion
            .reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .map(|reason| format!(" — {reason}"))
            .unwrap_or_default();
        lines.push(format!("- {marker} {}{}", criterion.text, reason));
        if lines.len() >= 6 {
            break;
        }
    }
    if lines.is_empty() {
        lines.push(
            "- No required criteria; optional/follow-up items do not block closure.".to_string(),
        );
    }
    let hidden = snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.kind == GoalCriterionKind::Required)
        .count()
        .saturating_sub(lines.len());
    if hidden > 0 {
        lines.push(format!("- ... {hidden} more required criteria"));
    }
    lines.join("\n")
}

fn render_goal_audit_summary(snapshot: &GoalSnapshot) -> String {
    let audit = latest_evaluator_for_display(snapshot);
    let status = audit.get("status").and_then(Value::as_str);
    if status.is_none() && snapshot.goal.final_summary.is_none() {
        return "No final audit yet.".to_string();
    }

    let mut lines = Vec::new();
    if let Some(status) = status {
        lines.push(format!("- Status: `{status}`"));
    }
    if snapshot.audit_stale {
        lines.push("- Audit is stale after newer goal evidence or revision changes.".to_string());
    }
    if let Some(summary) = snapshot
        .goal
        .final_summary
        .as_deref()
        .filter(|_| std::ptr::eq(audit, &snapshot.goal.final_evidence))
        .or_else(|| audit.get("summary").and_then(Value::as_str))
        .filter(|summary| !summary.trim().is_empty())
    {
        lines.push(format!("- Reason: {summary}"));
    }
    if let Some(blocked_reason) = audit
        .get("blockedReason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
    {
        lines.push(format!("- Blocked reason: `{blocked_reason}`"));
    }
    let missing = json_string_items(audit.get("missing"));
    if !missing.is_empty() {
        lines.push(format!(
            "- Missing: {}",
            missing
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let blockers = json_string_items(audit.get("blockers"));
    if !blockers.is_empty() {
        lines.push(format!(
            "- Blockers: {}",
            blockers
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let next = json_string_items(audit.get("nextEvidenceNeeded"));
    if !next.is_empty() {
        lines.push(format!(
            "- Next evidence: {}",
            next.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
        ));
    }
    if lines.is_empty() {
        "No final audit yet.".to_string()
    } else {
        lines.join("\n")
    }
}

fn latest_evaluator_for_display(snapshot: &GoalSnapshot) -> &Value {
    if snapshot
        .goal
        .last_evaluator_result
        .as_object()
        .is_some_and(|object| !object.is_empty())
    {
        &snapshot.goal.last_evaluator_result
    } else {
        &snapshot.goal.final_evidence
    }
}

fn json_string_items(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("text").and_then(Value::as_str))
                .or_else(|| item.get("summary").and_then(Value::as_str))
                .or_else(|| item.get("reason").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn format_duration_secs(secs: i64) -> String {
    let secs = secs.max(0);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn format_count(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    let mut formatted: String = out.chars().rev().collect();
    if negative {
        formatted.insert(0, '-');
    }
    formatted
}

fn goal_usage() -> String {
    [
        "## Goal commands",
        "",
        "- `/goal <objective> --criteria <completion criteria>`: create or update the active goal",
        "- `/goal`: show the active goal",
        "- `/goal status`: show the active goal",
        "- `/goal pause`: pause the active goal",
        "- `/goal resume`: resume the active/blocked goal",
        "- `/goal evaluate`: run final audit from linked workflow/task/validation evidence",
        "- `/goal accept`: accept the current audit and close the goal as v1",
        "- `/goal strict`: keep the goal blocked until stricter evidence is produced",
        "- `/goal clear`: cancel the active goal",
        "",
        "Control words only act as commands when they are the whole argument; longer text is treated as the goal objective.",
    ]
    .join("\n")
}

fn display_only(content: impl Into<String>) -> CommandResult {
    CommandResult {
        content: content.into(),
        action: Some(CommandAction::DisplayOnly),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn control_words_only_apply_as_exact_goal_commands() {
        assert_eq!(
            parse_goal_request("pause"),
            GoalRequest::Transition(GoalCommand::Pause)
        );
        assert_eq!(
            parse_goal_request("pause react upgrade"),
            GoalRequest::Upsert("pause react upgrade")
        );
        assert_eq!(
            parse_goal_request("update react upgrade"),
            GoalRequest::Upsert("update react upgrade")
        );
        assert_eq!(
            parse_goal_request("set react upgrade"),
            GoalRequest::Upsert("set react upgrade")
        );
    }

    #[test]
    fn goal_arg_parser_keeps_objective_and_criteria_simple() {
        assert_eq!(
            parse_goal_create_args("ship goal mode --criteria typecheck passes"),
            ("ship goal mode".to_string(), "typecheck passes".to_string())
        );
        assert_eq!(
            parse_goal_create_args("status should render as objective"),
            (
                "status should render as objective".to_string(),
                String::new()
            )
        );
    }

    #[test]
    fn goal_status_format_helpers_are_user_readable() {
        assert_eq!(format_duration_secs(0), "0s");
        assert_eq!(format_duration_secs(65), "1m 5s");
        assert_eq!(format_duration_secs(3661), "1h 1m");
        assert_eq!(format_count(1_173_488), "1,173,488");
    }

    #[test]
    fn goal_status_card_is_concise_and_usage_oriented() {
        let snapshot = GoalSnapshot {
            goal: crate::goal::Goal {
                id: "goal_123".to_string(),
                session_id: "session_123".to_string(),
                objective: "Ship Goal v3".to_string(),
                completion_criteria: "[required] typecheck passes".to_string(),
                revision: 2,
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                state: GoalState::Active,
                mode_snapshot: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
                created_at: "2026-07-08T00:00:00Z".to_string(),
                updated_at: "2026-07-08T00:01:00Z".to_string(),
                completed_at: None,
                final_summary: Some("Need one more verification.".to_string()),
                final_evidence: json!({
                    "status": "blocked",
                    "missing": ["typecheck evidence"],
                    "nextEvidenceNeeded": ["run focused check"]
                }),
                blocked_reason: None,
                last_evaluator_result: json!({
                    "evaluatorKind": "post_turn",
                    "status": "blocked",
                    "summary": "Latest post-turn check needs one more pass.",
                    "missing": ["latest evaluator evidence"]
                }),
                closure_decision: None,
                closure_reason: None,
                closed_at: None,
                follow_up_items: Vec::new(),
            },
            links: Vec::new(),
            events: Vec::new(),
            audit_stale: false,
            criteria_items: Vec::new(),
            criteria: vec![crate::goal::GoalCriterionAudit {
                id: "crit_1".to_string(),
                text: "typecheck passes".to_string(),
                kind: GoalCriterionKind::Required,
                status: GoalCriterionStatus::Missing,
                evidence_ids: Vec::new(),
                reason: Some("typecheck evidence".to_string()),
            }],
            evidence: Vec::new(),
            timeline: Vec::new(),
            budget: crate::goal::GoalBudgetSnapshot {
                tokens_used: 1_173_488,
                elapsed_secs: 3340,
                turns_used: 7,
                ..Default::default()
            },
            workflow_runs: Vec::new(),
            tasks: Vec::new(),
            grader_runs: Vec::new(),
        };

        let rendered = render_goal_snapshot(&snapshot);

        assert!(rendered.contains("Active Goal"));
        assert!(rendered.contains("55m 40s"));
        assert!(rendered.contains("1,173,488 tokens"));
        assert!(rendered.contains("required 0/1"));
        assert!(rendered.contains("Latest post-turn check needs one more pass."));
        assert!(rendered.contains("latest evaluator evidence"));
        assert!(!rendered.contains("/goal evaluate"));
        assert!(!rendered.contains("pending_user_acceptance"));
    }

    #[test]
    fn slash_accept_extracts_final_audit_follow_ups() {
        let audit = json!({
            "followUpItems": [
                { "text": "manual GUI smoke" },
                "export roadmap note",
                { "text": " manual   GUI smoke " },
                { "text": "" },
                { "id": "missing-text" }
            ]
        });

        assert_eq!(
            final_audit_follow_up_texts(&audit),
            vec![
                "manual GUI smoke".to_string(),
                "export roadmap note".to_string()
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slash_goal_upsert_exits_plan_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("session db"));
        let session = db.create_session("ha-main").expect("session");

        plan::set_plan_state(&session.id, PlanModeState::Planning).await;

        let result = handle_goal(&db, Some(&session.id), "Ship Goal slash parity")
            .await
            .expect("slash goal upsert");

        assert_eq!(plan::get_plan_state(&session.id).await, PlanModeState::Off);
        assert!(matches!(
            result.action,
            Some(CommandAction::PassThrough { .. })
        ));
        assert!(db
            .active_goal_for_session(&session.id)
            .expect("active goal")
            .is_some());
    }
}
