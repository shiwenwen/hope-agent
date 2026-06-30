use std::sync::Arc;

use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use crate::workflow::{WorkflowOp, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState};

const WORKFLOW_LIST_LIMIT: usize = 12;
const WORKFLOW_EVENT_LIMIT: usize = 40;
const WORKFLOW_TRACE_OP_LIMIT: usize = 8;
const WORKFLOW_TRACE_EVENT_LIMIT: usize = 8;

pub fn handle_workflow(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let mut parts = args.split_whitespace();
    let command = parts.next().unwrap_or("status");

    match command {
        "" | "status" | "list" => render_workflow_list(session_db, sid),
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
        _ => Err("Usage: /workflow [status|trace|approve|pause|resume|cancel] [run_id]".into()),
    }
}

pub fn handle_loop(args: &str) -> Result<CommandResult, String> {
    let mode = args.split_whitespace().next().unwrap_or("status");
    match mode {
        "" | "status" => Ok(display_only(
            "Loop modes: `off`, `guarded`, `deep`, `autonomous`.\n\
             Use `/loop guarded` to ask the agent to use a guarded coding loop for subsequent work.",
        )),
        "off" | "guarded" | "deep" | "autonomous" => {
            let message = format!(
                "Set coding loop mode to `{}` for this session. Apply it to subsequent coding work: choose workflow/script execution only when it improves stability, keep approvals and validation visible, and report any mode-specific limits before acting.",
                mode
            );
            Ok(CommandResult {
                content: format!(
                    "Loop mode request sent to the agent: `{}`.\n\nThis is a session instruction for the next turn; persistent loop-policy storage is not enabled yet.",
                    mode
                ),
                action: Some(CommandAction::PassThrough { message }),
            })
        }
        _ => Err("Usage: /loop [off|guarded|deep|autonomous|status]".into()),
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
}

fn render_workflow_list(session_db: &Arc<SessionDB>, sid: &str) -> Result<CommandResult, String> {
    let runs = session_db
        .list_workflow_runs_for_session(sid, WORKFLOW_LIST_LIMIT)
        .map_err(|e| e.to_string())?;

    if runs.is_empty() {
        return Ok(display_only(
            "No workflow runs for this session yet.\n\nUse `/workflow help` for commands.",
        ));
    }

    let active = runs
        .iter()
        .filter(|run| workflow_run_is_active(run.state))
        .count();
    let mut lines = vec![format!(
        "## Workflow runs\n\nActive: **{}** · showing latest {}",
        active,
        runs.len()
    )];
    for run in runs {
        lines.push(format!(
            "- `{}` · **{}** · `{}` · {} · updated `{}`{}",
            short_id(&run.id),
            run.state.as_str(),
            run.loop_mode,
            run.kind,
            run.updated_at,
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
        "## Workflow trace `{}`\n\nState: **{}** · kind: `{}` · loop: `{}` · updated `{}`",
        short_id(&snapshot.run.id),
        snapshot.run.state.as_str(),
        snapshot.run.kind,
        snapshot.run.loop_mode,
        snapshot.run.updated_at
    )];

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
    let updated = match command {
        WorkflowCommand::Approve => session_db.approve_workflow_run(&run.id),
        WorkflowCommand::Pause => session_db.pause_workflow_run(&run.id),
        WorkflowCommand::Resume => session_db.resume_workflow_run(&run.id),
        WorkflowCommand::Cancel => session_db.cancel_workflow_run(&run.id),
    }
    .map_err(|e| e.to_string())?;

    Ok(display_only(format!(
        "Workflow `{}` {} → **{}**.",
        short_id(&updated.id),
        command.as_str(),
        updated.state.as_str()
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
        "- `/workflow` or `/workflow status`: list recent runs",
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
