use std::{collections::HashSet, sync::Arc};

use crate::goal::{
    CloseGoalInput, CreateGoalInput, GoalClosureDecision, GoalSnapshot, GoalState, UpdateGoalInput,
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
    let criteria = if goal.completion_criteria.trim().is_empty() {
        "_No explicit completion criteria yet._".to_string()
    } else {
        goal.completion_criteria
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let workflows = snapshot.workflow_runs.len();
    let tasks_total = snapshot.tasks.len();
    let tasks_done = snapshot
        .tasks
        .iter()
        .filter(|task| task.status == "completed")
        .count();
    let final_summary = goal
        .final_summary
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("No final audit yet.");
    let closure = goal
        .closure_decision
        .map(|decision| format!("\nClosure decision: `{}`", decision.as_str()))
        .unwrap_or_else(|| "\nClosure decision: `pending_user_acceptance`".to_string());
    let blocked = goal
        .blocked_reason
        .as_deref()
        .map(|reason| format!("\nBlocked reason: `{reason}`"))
        .unwrap_or_default();

    format!(
        "## Goal `{}`\n\nState: **{}** · revision: **{}** · workflows: **{}** · tasks: **{}/{}**{}{}\n\n**Objective**\n{}\n\n**Completion criteria**\n{}\n\n**Final audit**\n{}\n\nUse `/goal evaluate` to run the conservative final audit, `/goal accept` to accept v1 closure, `/goal strict` to require stricter evidence, `/goal pause|resume|clear` to control it.",
        short_id(&goal.id),
        state,
        goal.revision,
        workflows,
        tasks_done,
        tasks_total,
        closure,
        blocked,
        goal.objective,
        criteria,
        final_summary,
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

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
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
