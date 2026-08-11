use crate::async_jobs::JobManager;
use crate::context_compact::{JobLedgerItem, RuntimeLedgerSnapshot, SubagentLedgerItem};
use crate::subagent::SubagentStatus;

pub(crate) fn build_runtime_ledger_snapshot(session_id: &str) -> RuntimeLedgerSnapshot {
    let mut snapshot = RuntimeLedgerSnapshot::default();

    match JobManager::list_session_snapshots(session_id) {
        Ok(jobs) => {
            snapshot.background_jobs = jobs
                .into_iter()
                .filter(|job| !job.status.is_terminal())
                .map(|job| {
                    let group_progress = job.child_count.map(|total| {
                        let terminal = job.children_terminal.unwrap_or(0);
                        format!("{}/{}", terminal, total)
                    });
                    JobLedgerItem {
                        job_id: job.job_id,
                        kind: job.kind.as_str().to_string(),
                        status: job.status.as_str().to_string(),
                        label: (!job.label.is_empty()).then_some(job.label),
                        tool: Some(job.tool),
                        group_progress,
                    }
                })
                .collect();
        }
        Err(e) => snapshot
            .warnings
            .push(format!("background_jobs_snapshot_failed: {}", e)),
    }

    if let Some(db) = crate::globals::get_session_db() {
        match db.list_subagent_runs(session_id) {
            Ok(runs) => {
                snapshot.subagents = runs
                    .into_iter()
                    .filter(|run| {
                        matches!(
                            run.status,
                            SubagentStatus::Queued
                                | SubagentStatus::Spawning
                                | SubagentStatus::Running
                        )
                    })
                    .map(|run| SubagentLedgerItem {
                        run_id: run.run_id,
                        status: run.status.as_str().to_string(),
                        child_agent_id: run.child_agent_id,
                        child_session_id: run.child_session_id,
                        task_preview: crate::truncate_utf8(&run.task, 160).to_string(),
                    })
                    .collect();
            }
            Err(e) => snapshot
                .warnings
                .push(format!("subagent_snapshot_failed: {}", e)),
        }
    } else {
        snapshot
            .warnings
            .push("session_db_unavailable_for_subagent_snapshot".to_string());
    }

    snapshot
}

/// Bounded dynamic suffix for terminal child threads that can be continued.
/// It contains only control-plane identities and statuses; provider error text
/// stays out of the system channel and can be inspected through the subagent
/// status tool if needed.
pub(crate) fn subagent_recovery_reminder(session_id: &str) -> Option<String> {
    let db = crate::globals::get_session_db()?;
    let runs = db
        .list_current_recoverable_subagent_runs(session_id, 8)
        .ok()?;
    let recoveries = db
        .list_current_subagent_provider_recoveries(session_id, 8)
        .unwrap_or_default();
    if runs.is_empty() && recoveries.is_empty() {
        return None;
    }
    let mut lines = vec![
        "<runtime-recovery>".to_string(),
        "The following current sub-agent attempts need a recovery decision, or their automatic parent-result delivery exhausted its retries. Inspect each thread first. Continue only resumable unfinished work; for a completed thread, fetch its result and respond without rerunning it. Never repeat completed side effects blindly. Workflow-owned threads must be resumed through their Workflow controller.".to_string(),
    ];
    for recovery in recoveries {
        lines.push(format!(
            "- run_id={} thread_id={} owner={}:{} status=retrying_provider attempt={}/{} next_attempt_at={}",
            recovery.run_id,
            recovery.thread_id,
            recovery.owner_kind.as_str(),
            recovery.owner_id,
            recovery.attempt,
            recovery.max_attempts,
            recovery.next_attempt_at,
        ));
    }
    for run in runs {
        if run.status == crate::subagent::SubagentStatus::Completed {
            lines.push(format!(
                "- run_id={} thread_id={} owner={}:{} status=result_delivery_exhausted action=inspect_completed_result",
                run.run_id,
                run.thread_id,
                run.owner_kind.as_str(),
                run.owner_id,
            ));
        } else {
            lines.push(format!(
                "- run_id={} thread_id={} owner={}:{} status={} reason={}",
                run.run_id,
                run.thread_id,
                run.owner_kind.as_str(),
                run.owner_id,
                run.status.as_str(),
                run.terminal_reason
                    .map(|reason| reason.as_str())
                    .unwrap_or("unknown")
            ));
        }
    }
    lines.push("</runtime-recovery>".to_string());
    Some(lines.join("\n"))
}

/// Dynamic system reminder for a foreground turn admitted while its durable
/// Stop receipt is still active. Natural-language intent stays with the model:
/// explicit resume requests call `session_continue`; status/replanning turns do
/// not silently clear the user's Stop.
pub(crate) async fn session_pause_reminder(session_id: &str) -> Option<String> {
    let db = crate::globals::get_session_db()?.clone();
    let lookup_session_id = session_id.to_string();
    let pause = db
        .run(move |db| db.active_session_or_ancestor_autonomy_pause(&lookup_session_id))
        .await
        .ok()??;
    render_session_pause_reminder(&pause)
}

fn render_session_pause_reminder(pause: &crate::session::SessionAutonomyPause) -> Option<String> {
    let workflow_ids = pause
        .workflow_run_ids
        .iter()
        .take(16)
        .cloned()
        .collect::<Vec<_>>();
    let subagent_ids = pause
        .subagent_run_ids
        .iter()
        .take(16)
        .cloned()
        .collect::<Vec<_>>();
    Some(format!(
        "<session-paused>\nThe user previously stopped this conversation and its durable autonomy fence is still active. If the latest foreground user message explicitly asks to continue, resume, proceed, or keep going (for example: '继续', '接着做', 'continue', or 'resume'), call `session_continue` before trying to restart Goal, Workflow, wakeup, or sub-agent work. If the user only asks for status, explanation, inspection, or a changed recovery plan, do not call it and keep the fence active. Never infer continuation from an autonomous/internal message.\n- pause_id={}\n- root_session_id={}\n- goal_id={}\n- workflow_run_ids={}\n- subagent_run_ids={}\n</session-paused>",
        pause.id,
        pause.session_id,
        pause.goal_id.as_deref().unwrap_or("none"),
        serde_json::to_string(&workflow_ids).ok()?,
        serde_json::to_string(&subagent_ids).ok()?,
    ))
}

/// Build the runtime ledger for emergency (Tier 4) compaction, honoring
/// incognito. An incognito session gets `None` so job / subagent ids are never
/// built or injected into the history that Tier 4 both sends to the model and
/// persists via `save_agent_context` — incognito parity with the Tier-3 path in
/// `agent/context.rs`. Callers resolve `is_incognito` via
/// `crate::session::is_session_incognito` (fail-closed) and pass it in, keeping
/// the gate unit-testable without the process-global session DB.
pub(crate) fn emergency_runtime_ledger(
    session_id: &str,
    is_incognito: bool,
) -> Option<RuntimeLedgerSnapshot> {
    if is_incognito {
        None
    } else {
        Some(build_runtime_ledger_snapshot(session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emergency_ledger_skipped_when_incognito() {
        // Regression guard: the Tier-4 emergency path must NOT build/inject a
        // runtime ledger for an incognito session (job/subagent ids would leak
        // into persisted history). is_incognito=true short-circuits before any
        // global query, so this is deterministic with no globals set.
        assert!(emergency_runtime_ledger("incognito-session", true).is_none());
    }

    #[test]
    fn emergency_ledger_built_when_not_incognito() {
        // Non-incognito → a snapshot is produced (empty/with warnings when the
        // job/session globals are unset in tests, but always `Some`).
        assert!(emergency_runtime_ledger("normal-session", false).is_some());
    }

    #[test]
    fn pause_reminder_keeps_resume_decision_with_the_model() {
        let pause = crate::session::SessionAutonomyPause {
            id: "pause_current".to_string(),
            session_id: "session_root".to_string(),
            goal_id: Some("goal_active".to_string()),
            workflow_run_ids: (0..18).map(|index| format!("workflow_{index}")).collect(),
            subagent_run_ids: vec!["subagent_current".to_string()],
            created_at: "2026-08-10T00:00:00Z".to_string(),
            resumed_at: None,
        };

        let reminder = render_session_pause_reminder(&pause).expect("pause reminder");
        for contract in [
            "call `session_continue`",
            "do not call it",
            "pause_id=pause_current",
            "root_session_id=session_root",
            "workflow_15",
            "subagent_current",
        ] {
            assert!(reminder.contains(contract), "missing contract: {contract}");
        }
        assert!(!reminder.contains("workflow_16"));
    }
}
