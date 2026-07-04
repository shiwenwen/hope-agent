use std::sync::Arc;

use crate::execution_mode::ExecutionMode;
use crate::goal::Goal;
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use crate::workflow::{WorkflowOp, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState};
use crate::workflow_mode::WorkflowMode;

const WORKFLOW_LIST_LIMIT: usize = 12;
const WORKFLOW_EVENT_LIMIT: usize = 40;
const WORKFLOW_TRACE_OP_LIMIT: usize = 8;
const WORKFLOW_TRACE_EVENT_LIMIT: usize = 8;
const GOAL_OBJECTIVE_STATUS_LIMIT: usize = 120;
const GOAL_OBJECTIVE_LIST_LIMIT: usize = 72;
const GOAL_OBJECTIVE_TRACE_LIMIT: usize = 160;

pub fn handle_workflow(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let mut parts = args.split_whitespace();
    let command = parts.next().unwrap_or("status");
    let Some(sid) = session_id else {
        return match command {
            "" | "status" => Ok(display_only(
                "## Workflow Mode\n\nNo active session yet. Workflow Mode is **Off** (`off`) by default.\n\nUse `/workflow on` or the composer Workflow button to enable autonomous workflow orchestration for a new session.",
            )),
            "off" | "disable" | "disabled" => Ok(display_only(
                "Workflow Mode is already **Off** (`off`) because there is no active session yet.",
            )),
            "help" => Ok(display_only(workflow_usage())),
            _ => Err("No active session. Use `/workflow on` or create a chat session first.".into()),
        };
    };

    match command {
        "" | "status" => render_workflow_status(session_db, sid),
        "on" | "enable" | "enabled" => set_workflow_mode(session_db, sid, WorkflowMode::On),
        "off" | "disable" | "disabled" => set_workflow_mode(session_db, sid, WorkflowMode::Off),
        "ultracode" | "ultra" => set_workflow_mode(session_db, sid, WorkflowMode::Ultracode),
        "runs" | "list" => render_workflow_list(session_db, sid),
        "trace" | "show" => {
            let run_id = parts.next();
            render_workflow_trace(session_db, sid, run_id)
        }
        "approve" => {
            transition_workflow_run(session_db, sid, parts.next(), WorkflowCommand::Approve)
        }
        "pause" => transition_workflow_run(session_db, sid, parts.next(), WorkflowCommand::Pause),
        "resume" => transition_workflow_run(session_db, sid, parts.next(), WorkflowCommand::Resume),
        "cancel" => transition_workflow_run(session_db, sid, parts.next(), WorkflowCommand::Cancel),
        "help" => Ok(display_only(workflow_usage())),
        _ => Err("Usage: /workflow [on|off|ultracode|status|runs|trace|approve|pause|resume|cancel] [run_id]".into()),
    }
}

pub fn handle_mode(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let mode = args.split_whitespace().next().unwrap_or("status");
    let Some(sid) = session_id else {
        return match mode {
            "" | "status" => Ok(display_only(
                "Current execution mode: **Off** (`off`).\n\nNo active session exists yet, so no session policy has been persisted.",
            )),
            "off" => Ok(display_only(
                "Execution mode is already **Off** (`off`) because there is no active session yet.",
            )),
            _ => Err("No active session. Create a chat session before changing execution mode.".into()),
        };
    };
    match mode {
        "" | "status" => {
            let current = session_db
                .get_session_execution_mode(sid)
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            Ok(display_only(format!(
                "Current execution mode: **{}** (`{}`).\n\nModes: `off`, `guarded`, `deep`, `autonomous`.\nUse `/mode guarded` to persist a guarded execution policy for this session.",
                current.label(),
                current.as_str()
            )))
        }
        "off" | "guarded" | "deep" | "autonomous" => {
            let parsed = ExecutionMode::from_str(mode)
                .ok_or_else(|| "Usage: /mode [off|guarded|deep|autonomous|status]".to_string())?;
            session_db
                .update_session_execution_mode(sid, parsed)
                .map_err(|e| e.to_string())?;
            Ok(display_only(format!(
                "Execution mode for this session is now **{}** (`{}`).\n\nThe policy is persisted and will be injected into subsequent system prompts.",
                parsed.label(),
                parsed.as_str()
            )))
        }
        _ => Err("Usage: /mode [off|guarded|deep|autonomous|status]".into()),
    }
}

#[derive(Clone, Copy)]
enum WorkflowCommand {
    Approve,
    Pause,
    Resume,
    Cancel,
}

impl WorkflowCommand {
    fn as_str(&self) -> &'static str {
        match self {
            WorkflowCommand::Approve => "approve",
            WorkflowCommand::Pause => "pause",
            WorkflowCommand::Resume => "resume",
            WorkflowCommand::Cancel => "cancel",
        }
    }

    fn target_state(&self) -> Option<WorkflowRunState> {
        match self {
            WorkflowCommand::Approve => Some(WorkflowRunState::AwaitingApproval),
            WorkflowCommand::Pause => Some(WorkflowRunState::Running),
            WorkflowCommand::Resume => Some(WorkflowRunState::Paused),
            WorkflowCommand::Cancel => None,
        }
    }

    fn starts_runtime(&self) -> bool {
        matches!(self, WorkflowCommand::Approve | WorkflowCommand::Resume)
    }
}

fn session_is_incognito(session_db: &Arc<SessionDB>, sid: &str) -> Result<bool, String> {
    session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .map(|meta| meta.incognito)
        .ok_or_else(|| "Session not found".to_string())
}

fn set_workflow_mode(
    session_db: &Arc<SessionDB>,
    sid: &str,
    mode: WorkflowMode,
) -> Result<CommandResult, String> {
    if mode.enabled() && session_is_incognito(session_db, sid)? {
        return Err(
            "Workflow Mode is unavailable for incognito sessions because workflow runs are durable."
                .into(),
        );
    }
    session_db
        .update_session_workflow_mode(sid, mode)
        .map_err(|e| e.to_string())?;
    let guidance = match mode {
        WorkflowMode::Off => {
            "The model will not receive the workflow_run tool in subsequent turns."
        }
        WorkflowMode::On => {
            "The model may now create observable workflow runs when dynamic orchestration helps."
        }
        WorkflowMode::Ultracode => {
            "The model is now biased toward exhaustive, review-heavy workflow orchestration for substantive tasks."
        }
    };
    Ok(CommandResult {
        content: format!(
            "Workflow Mode is now **{}** (`{}`).\n\n{}",
            mode.label(),
            mode.as_str(),
            guidance
        ),
        action: Some(CommandAction::SetWorkflowMode {
            mode: mode.as_str().to_string(),
        }),
    })
}

fn render_workflow_status(session_db: &Arc<SessionDB>, sid: &str) -> Result<CommandResult, String> {
    let incognito = session_is_incognito(session_db, sid)?;
    let mode = session_db
        .get_session_workflow_mode(sid)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let display_mode = if incognito { WorkflowMode::Off } else { mode };
    let runs = session_db
        .list_workflow_runs_for_session(sid, WORKFLOW_LIST_LIMIT)
        .map_err(|e| e.to_string())?;
    let active = runs
        .iter()
        .filter(|run| workflow_run_is_active(run.state))
        .count();
    let mut lines = vec![format!(
        "## Workflow Mode\n\nCurrent: **{}** (`{}`)\n\n{}",
        display_mode.label(),
        display_mode.as_str(),
        if incognito {
            "Workflow orchestration is unavailable in incognito sessions because workflow runs are durable."
        } else if mode.enabled() {
            "The model can autonomously create durable workflow runs when the task benefits from orchestration."
        } else {
            "Workflow orchestration is off. Use `/workflow on` or `/workflow ultracode` to enable it."
        }
    )];
    lines.push(format!(
        "\nRuns: **{}** active · {} recent",
        active,
        runs.len()
    ));
    lines.push(format_active_goal_line(session_db, sid)?);
    if let Some(run) = runs.first() {
        lines.push(format!(
            "Latest: `{}` · **{}** · `{}` · updated `{}`{}",
            short_id(&run.id),
            run.state.as_str(),
            run.kind,
            run.updated_at,
            workflow_goal_suffix(session_db, run)?
        ));
    }
    lines.push(
        "\nUse `/workflow runs` to list runs, `/workflow trace [run_id]` to inspect one, or `/workflow help` for all commands."
            .into(),
    );
    Ok(display_only(lines.join("\n")))
}

fn render_workflow_list(session_db: &Arc<SessionDB>, sid: &str) -> Result<CommandResult, String> {
    let incognito = session_is_incognito(session_db, sid)?;
    let mode = session_db
        .get_session_workflow_mode(sid)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let display_mode = if incognito { WorkflowMode::Off } else { mode };
    let runs = session_db
        .list_workflow_runs_for_session(sid, WORKFLOW_LIST_LIMIT)
        .map_err(|e| e.to_string())?;

    if runs.is_empty() {
        if incognito {
            return Ok(display_only(
                "No workflow runs for this session yet.\n\nWorkflow Mode is unavailable in incognito sessions because workflow runs are durable.",
            ));
        }
        return Ok(display_only(format!(
            "No workflow runs for this session yet.\n\nWorkflow Mode: **{}** (`{}`). Use `/workflow on` to let the model create runs autonomously.",
            display_mode.label(),
            display_mode.as_str()
        )));
    }

    let active = runs
        .iter()
        .filter(|run| workflow_run_is_active(run.state))
        .count();
    let mut lines = vec![format!(
        "## Workflow runs\n\nMode: **{}** (`{}`) · Active: **{}** · showing latest {}",
        display_mode.label(),
        display_mode.as_str(),
        active,
        runs.len()
    )];
    for run in runs {
        lines.push(format!(
            "- `{}` · **{}** · `{}` · {} · updated `{}`{}{}",
            short_id(&run.id),
            run.state.as_str(),
            run.execution_mode,
            run.kind,
            run.updated_at,
            workflow_goal_suffix(session_db, &run)?,
            run.blocked_reason
                .as_deref()
                .map(|reason| format!(" · blocked: {}", truncate(reason, 80)))
                .unwrap_or_default()
        ));
    }
    lines.push("\nUse `/workflow trace <run_id>` for ops/events. The short id prefix is accepted when unique.".into());
    Ok(display_only(lines.join("\n")))
}

fn render_workflow_trace(
    session_db: &Arc<SessionDB>,
    sid: &str,
    run_id: Option<&str>,
) -> Result<CommandResult, String> {
    let snapshot = resolve_workflow_snapshot(session_db, sid, run_id)?;
    let mut lines = vec![format!(
        "## Workflow trace `{}`\n\nState: **{}** · kind: `{}` · mode: `{}` · updated `{}`",
        short_id(&snapshot.run.id),
        snapshot.run.state.as_str(),
        snapshot.run.kind,
        snapshot.run.execution_mode,
        snapshot.run.updated_at
    )];

    if let Some(goal_line) = workflow_goal_trace_line(session_db, &snapshot.run)? {
        lines.push(goal_line);
    }

    if let Some(reason) = snapshot.run.blocked_reason.as_deref() {
        lines.push(format!("Blocked: {}", truncate(reason, 180)));
    }

    lines.push(format!(
        "\nOps: {} · Events: {}",
        workflow_op_summary(&snapshot.ops),
        snapshot.events.len()
    ));

    if !snapshot.ops.is_empty() {
        lines.push("\n### Recent ops".into());
        for op in snapshot
            .ops
            .iter()
            .rev()
            .take(WORKFLOW_TRACE_OP_LIMIT)
            .rev()
        {
            lines.push(format!(
                "- `{}` · **{}** · `{}` · {}",
                truncate(&op.op_key, 96),
                op.state.as_str(),
                op.op_type,
                op.error
                    .as_ref()
                    .map(|err| truncate(&json_compact(err), 100))
                    .unwrap_or_else(|| op.effect_class.as_str().to_string())
            ));
        }
    }

    if !snapshot.events.is_empty() {
        lines.push("\n### Recent events".into());
        for event in snapshot
            .events
            .iter()
            .rev()
            .take(WORKFLOW_TRACE_EVENT_LIMIT)
            .rev()
        {
            lines.push(format!(
                "- #{} `{}` · {}",
                event.seq,
                event.event_type,
                truncate(&json_compact(&event.payload), 120)
            ));
        }
    }

    Ok(display_only(lines.join("\n")))
}

fn transition_workflow_run(
    session_db: &Arc<SessionDB>,
    sid: &str,
    run_id: Option<&str>,
    command: WorkflowCommand,
) -> Result<CommandResult, String> {
    let run = resolve_workflow_run(session_db, sid, run_id, command.target_state())?;
    if command.starts_runtime() {
        crate::workflow::ensure_workflow_launcher_primary().map_err(|e| e.to_string())?;
    }
    let updated = match command {
        WorkflowCommand::Approve => session_db.approve_workflow_run(&run.id),
        WorkflowCommand::Pause => session_db.pause_workflow_run(&run.id),
        WorkflowCommand::Resume => session_db.resume_workflow_run(&run.id),
        WorkflowCommand::Cancel => session_db.cancel_workflow_run(&run.id),
    }
    .map_err(|e| e.to_string())?;

    let launch_accepted = if command.starts_runtime() {
        crate::workflow::spawn_workflow_run_if_primary(
            session_db.clone(),
            updated.id.clone(),
            format!("slash:{}:pid:{}", command.as_str(), std::process::id()),
        )
    } else {
        false
    };

    let launch_note = if command.starts_runtime() {
        if launch_accepted {
            " Runtime launch accepted."
        } else {
            " Runtime launch was not accepted by this process."
        }
    } else {
        ""
    };
    Ok(display_only(format!(
        "Workflow `{}` {} → **{}**.{}",
        short_id(&updated.id),
        command.as_str(),
        updated.state.as_str(),
        launch_note
    )))
}

fn resolve_workflow_snapshot(
    session_db: &Arc<SessionDB>,
    sid: &str,
    run_id: Option<&str>,
) -> Result<WorkflowRunSnapshot, String> {
    let run = resolve_workflow_run(session_db, sid, run_id, None)?;
    session_db
        .workflow_run_snapshot(&run.id, WORKFLOW_EVENT_LIMIT)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Workflow run `{}` not found", run.id))
}

fn resolve_workflow_run(
    session_db: &Arc<SessionDB>,
    sid: &str,
    run_id: Option<&str>,
    preferred_state: Option<WorkflowRunState>,
) -> Result<WorkflowRun, String> {
    let runs = session_db
        .list_workflow_runs_for_session(sid, 200)
        .map_err(|e| e.to_string())?;
    if let Some(raw_id) = run_id {
        let matches: Vec<WorkflowRun> = runs
            .into_iter()
            .filter(|run| run.id == raw_id || run.id.starts_with(raw_id))
            .collect();
        return match matches.len() {
            0 => Err(format!(
                "Workflow run `{}` not found in this session",
                raw_id
            )),
            1 => Ok(matches.into_iter().next().expect("len checked")),
            _ => Err(format!(
                "Workflow run prefix `{}` matches multiple runs; use a longer id",
                raw_id
            )),
        };
    }

    if let Some(state) = preferred_state {
        let matches: Vec<_> = runs.iter().filter(|run| run.state == state).collect();
        if matches.len() == 1 {
            return Ok(matches[0].clone());
        }
        if matches.len() > 1 {
            return Err(format!(
                "Multiple {} workflow runs exist; pass a run id",
                state.as_str()
            ));
        }
    }

    runs.into_iter()
        .find(|run| workflow_run_is_active(run.state))
        .or_else(|| {
            session_db
                .list_workflow_runs_for_session(sid, 1)
                .ok()
                .and_then(|mut runs| runs.pop())
        })
        .ok_or_else(|| "No workflow runs for this session".to_string())
}

fn workflow_usage() -> String {
    [
        "## Workflow commands",
        "",
        "Workflow Mode is a session switch, not a separate coding mode. Turn it on, then send your normal request; the model decides whether the task is substantial enough to create a durable JavaScript workflow run.",
        "",
        "### Normal use",
        "",
        "1. `/workflow on` enables autonomous workflow orchestration for this session.",
        "2. Send the actual task normally, for example: `research these options and produce a recommendation`.",
        "3. If orchestration helps, the model calls `workflow_run` and the run appears in the Workspace / Workflow control center.",
        "4. Inspect progress with `/workflow status`, `/workflow runs`, `/workflow trace`, or the GUI. Approve, pause, resume, or cancel when needed.",
        "5. Use `/workflow off` when you want ordinary chat/tool behavior again.",
        "",
        "Use `/workflow ultracode` for high-rigor work where broader exploration, parallel review, validation, or long-running recovery is worth the extra cost.",
        "",
        "- `/workflow` or `/workflow status`: show the current Workflow Mode and run summary",
        "- `/workflow on`: allow the model to autonomously create durable workflow runs when useful",
        "- `/workflow ultracode`: bias the model toward exhaustive workflow orchestration for substantive tasks",
        "- `/workflow off`: hide the workflow_run tool and disable autonomous workflow creation",
        "- `/workflow runs`: list recent workflow runs",
        "- `/workflow trace [run_id]`: show ops and recent events",
        "- `/workflow approve [run_id]`: approve a permission-preview-gated run",
        "- `/workflow pause [run_id]`: pause a running run",
        "- `/workflow resume [run_id]`: resume a paused run",
        "- `/workflow cancel [run_id]`: cancel a draft/live run",
        "",
        "When `run_id` is omitted, the command targets the only matching live run when that is unambiguous.",
    ]
    .join("\n")
}

fn workflow_run_is_active(state: WorkflowRunState) -> bool {
    matches!(
        state,
        WorkflowRunState::AwaitingApproval
            | WorkflowRunState::Running
            | WorkflowRunState::AwaitingUser
            | WorkflowRunState::Paused
            | WorkflowRunState::Recovering
    )
}

fn workflow_op_summary(ops: &[WorkflowOp]) -> String {
    let completed = ops
        .iter()
        .filter(|op| op.state == crate::workflow::WorkflowOpState::Completed)
        .count();
    let failed = ops
        .iter()
        .filter(|op| op.state == crate::workflow::WorkflowOpState::Failed)
        .count();
    if failed > 0 {
        format!("{}/{} completed, {} failed", completed, ops.len(), failed)
    } else {
        format!("{}/{} completed", completed, ops.len())
    }
}

fn format_active_goal_line(session_db: &Arc<SessionDB>, sid: &str) -> Result<String, String> {
    let goal = session_db
        .active_goal_for_session(sid)
        .map_err(|e| e.to_string())?
        .map(|snapshot| snapshot.goal);
    Ok(match goal {
        Some(goal) => format!(
            "Active Goal: {}",
            format_goal_reference(&goal, GOAL_OBJECTIVE_STATUS_LIMIT)
        ),
        None => "Active Goal: none".to_string(),
    })
}

fn workflow_goal_suffix(session_db: &Arc<SessionDB>, run: &WorkflowRun) -> Result<String, String> {
    let Some(goal) = workflow_goal(session_db, run)? else {
        return Ok(String::new());
    };
    Ok(format!(
        " · Goal: {}",
        format_goal_reference(&goal, GOAL_OBJECTIVE_LIST_LIMIT)
    ))
}

fn workflow_goal_trace_line(
    session_db: &Arc<SessionDB>,
    run: &WorkflowRun,
) -> Result<Option<String>, String> {
    let Some(goal_id) = run.goal_id.as_deref() else {
        return Ok(Some("Linked Goal: none".to_string()));
    };
    match session_db.get_goal(goal_id).map_err(|e| e.to_string())? {
        Some(goal) => Ok(Some(format!(
            "Linked Goal: {}",
            format_goal_reference(&goal, GOAL_OBJECTIVE_TRACE_LIMIT)
        ))),
        None => Ok(Some(format!(
            "Linked Goal: `{}` · missing",
            short_id(goal_id)
        ))),
    }
}

fn workflow_goal(session_db: &Arc<SessionDB>, run: &WorkflowRun) -> Result<Option<Goal>, String> {
    let Some(goal_id) = run.goal_id.as_deref() else {
        return Ok(None);
    };
    session_db.get_goal(goal_id).map_err(|e| e.to_string())
}

fn format_goal_reference(goal: &Goal, objective_limit: usize) -> String {
    format!(
        "`{}` · **{}** · {}",
        short_id(&goal.id),
        goal.state.as_str(),
        truncate(goal.objective.trim(), objective_limit)
    )
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

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let head = keep * 2 / 3;
    let tail = keep.saturating_sub(head);
    let start: String = value.chars().take(head).collect();
    let end: String = value
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{start}…{end}")
}

fn json_compact(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_db_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hope-agent-workflow-slash-{name}-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn open_workflow_test_db(name: &str) -> (PathBuf, Arc<SessionDB>) {
        let db_path = temp_db_path(name);
        let db = Arc::new(SessionDB::open(&db_path).expect("open session db"));
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("migrate channel table");
        (db_path, db)
    }

    #[test]
    fn workflow_mode_command_persists_and_returns_sync_action() {
        let (db_path, db) = open_workflow_test_db("mode-on");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        let result = handle_workflow(&db, Some(&session.id), "on").expect("workflow on");

        match result.action {
            Some(CommandAction::SetWorkflowMode { mode }) => assert_eq!(mode, "on"),
            other => panic!("unexpected action: {:?}", other),
        }
        assert_eq!(
            db.get_session_workflow_mode(&session.id)
                .expect("read workflow mode"),
            Some(WorkflowMode::On)
        );
        assert!(result.content.contains("Workflow Mode is now **On**"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn workflow_no_session_reports_default_state_without_materializing() {
        let (db_path, db) = open_workflow_test_db("no-session");

        let status = handle_workflow(&db, None, "").expect("workflow status without session");
        assert!(matches!(status.action, Some(CommandAction::DisplayOnly)));
        assert!(status.content.contains("No active session yet"));
        assert!(status.content.contains("**Off**"));

        let off = handle_workflow(&db, None, "off").expect("workflow off without session");
        assert!(off.content.contains("already **Off**"));

        let err = handle_workflow(&db, None, "ultracode")
            .expect_err("enabling workflow mode still requires a session");
        assert!(err.contains("No active session"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn execution_mode_no_session_reports_default_state_without_materializing() {
        let (db_path, db) = open_workflow_test_db("mode-no-session");

        let status = handle_mode(&db, None, "").expect("mode status without session");
        assert!(matches!(status.action, Some(CommandAction::DisplayOnly)));
        assert!(status.content.contains("**Off**"));

        let off = handle_mode(&db, None, "off").expect("mode off without session");
        assert!(off.content.contains("already **Off**"));

        let err = handle_mode(&db, None, "guarded")
            .expect_err("persisting execution mode still requires a session");
        assert!(err.contains("No active session"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn workflow_status_reports_mode_and_run_summary() {
        let (db_path, db) = open_workflow_test_db("status");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        db.update_session_workflow_mode(&session.id, WorkflowMode::Ultracode)
            .expect("set workflow mode");

        let result = handle_workflow(&db, Some(&session.id), "").expect("workflow status");

        assert!(matches!(result.action, Some(CommandAction::DisplayOnly)));
        assert!(result.content.contains("Workflow Mode"));
        assert!(result.content.contains("Ultracode"));
        assert!(result.content.contains("Runs: **0** active"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn workflow_commands_show_linked_goal_context() {
        let (db_path, db) = open_workflow_test_db("goal-context");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Ship slash workflow goal visibility".to_string(),
                completion_criteria: "workflow commands show linked Goal context".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let run = db
            .create_workflow_run(crate::workflow::CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "general.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source:
                    "export default async function main(workflow) { await workflow.finish({ summary: 'done' }); }"
                        .to_string(),
                budget: serde_json::json!({}),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                worktree_id: None,
            })
            .expect("create workflow run");
        assert_eq!(run.goal_id.as_deref(), Some(goal.goal.id.as_str()));

        let status = handle_workflow(&db, Some(&session.id), "").expect("workflow status");
        assert!(status.content.contains("Active Goal:"));
        assert!(status
            .content
            .contains("Ship slash workflow goal visibility"));
        assert!(status.content.contains("Goal:"));

        let runs = handle_workflow(&db, Some(&session.id), "runs").expect("workflow runs");
        assert!(runs.content.contains("Goal:"));
        assert!(runs.content.contains("Ship slash workflow goal visibility"));

        let trace = handle_workflow(&db, Some(&session.id), "trace").expect("workflow trace");
        assert!(trace.content.contains("Linked Goal:"));
        assert!(trace
            .content
            .contains("Ship slash workflow goal visibility"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn workflow_mode_is_unavailable_for_incognito_sessions() {
        let (db_path, db) = open_workflow_test_db("incognito");
        let session = db
            .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, Some(true))
            .expect("create incognito session");

        let err = handle_workflow(&db, Some(&session.id), "on")
            .expect_err("incognito workflow mode should be rejected");
        assert!(err.contains("incognito sessions"));
        assert_eq!(
            db.get_session_workflow_mode(&session.id)
                .expect("read workflow mode"),
            Some(WorkflowMode::Off)
        );

        let status = handle_workflow(&db, Some(&session.id), "").expect("workflow status");
        assert!(status.content.contains("unavailable in incognito sessions"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn workflow_resume_slash_does_not_mark_running_when_runtime_cannot_start() {
        if crate::runtime_lock::is_primary() {
            return;
        }
        let (db_path, db) = open_workflow_test_db("resume-non-primary");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let run = db
            .create_workflow_run(crate::workflow::CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "general.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source:
                    "export default async function main(workflow) { await workflow.finish({ summary: 'done' }); }"
                        .to_string(),
                budget: serde_json::json!({}),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                worktree_id: None,
            })
            .expect("create workflow run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
            .expect("mark running");
        db.pause_workflow_run(&run.id).expect("pause workflow");

        let err = handle_workflow(&db, Some(&session.id), &format!("resume {}", run.id))
            .expect_err("non-primary slash resume should fail before state change");
        assert!(err.contains("primary runtime process"));
        let current = db
            .get_workflow_run(&run.id)
            .expect("get run")
            .expect("run exists");
        assert_eq!(current.state, WorkflowRunState::Paused);

        let _ = std::fs::remove_file(&db_path);
    }
}
