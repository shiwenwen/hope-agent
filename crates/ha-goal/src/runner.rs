//! Post-turn Goal progression and wakeup scheduling.

use anyhow::{anyhow, Result};
use serde_json::json;

use ha_core::chat_engine::ChatSource;
use ha_core::goal::{
    goal_has_current_satisfied_semantic_grade, goal_requires_semantic_grade,
    goal_runner_should_continue, goal_runner_should_evaluate,
};
use ha_core::session::SessionDB;

const GOAL_AUTO_CONTINUE_DELAY_SECS: i64 = 10;
const GOAL_AUTO_CONTINUE_MAX_PER_REVISION: usize = 20;

pub(crate) fn maybe_schedule_goal_continuation(
    db: &SessionDB,
    session_id: &str,
    agent_id: &str,
    source: ChatSource,
    turn_id: Option<&str>,
    assistant_message_id: Option<i64>,
) -> Result<Option<ha_core::wakeup::ScheduleOutcome>> {
    if matches!(source, ChatSource::Subagent) {
        return Ok(None);
    }
    // Stop owns a durable session-level admission fence. Even if a parent turn
    // is concurrently settling, it must not schedule the next autonomous Goal
    // turn until explicit Continue clears that receipt.
    if db.is_session_or_ancestor_autonomy_paused(session_id)? {
        return Ok(None);
    }
    let Some(mut snapshot) = db.active_goal_for_session(session_id)? else {
        return Ok(None);
    };
    let scheduled_this_turn = snapshot.events.iter().any(|event| {
        event.kind == "goal_auto_continue_scheduled"
            && event
                .payload
                .get("turnId")
                .and_then(serde_json::Value::as_str)
                == turn_id
    });
    if scheduled_this_turn {
        return Ok(None);
    }
    let evaluated_this_turn = snapshot.events.iter().any(|event| {
        event.kind == "goal_runner_evaluated"
            && event
                .payload
                .get("turnId")
                .and_then(serde_json::Value::as_str)
                == turn_id
    });
    if !evaluated_this_turn && goal_runner_should_evaluate(&snapshot) {
        snapshot = db.record_goal_runner_evaluation(
            &snapshot.goal.id,
            source.as_str(),
            turn_id,
            assistant_message_id,
        )?;
    }
    if !goal_runner_should_continue(&snapshot) {
        return Ok(None);
    }
    if goal_runner_should_wait_for_background_jobs(db, session_id, &snapshot.goal.id)? {
        return Ok(None);
    }
    let scheduled_for_revision = snapshot
        .events
        .iter()
        .filter(|event| {
            event.kind == "goal_auto_continue_scheduled"
                && event
                    .payload
                    .get("goalRevision")
                    .and_then(serde_json::Value::as_i64)
                    == Some(snapshot.goal.revision)
        })
        .count();
    if scheduled_for_revision >= GOAL_AUTO_CONTINUE_MAX_PER_REVISION {
        let _ = db.append_goal_event(
            &snapshot.goal.id,
            "goal_auto_continue_halted",
            json!({
                "reason": "max_auto_continues_per_revision",
                "limit": GOAL_AUTO_CONTINUE_MAX_PER_REVISION,
                "goalRevision": snapshot.goal.revision,
                "turnId": turn_id,
            }),
        );
        return Ok(None);
    }

    let semantic_instruction = if goal_requires_semantic_grade(&snapshot)
        && !goal_has_current_satisfied_semantic_grade(&snapshot).unwrap_or(false)
    {
        "- The deterministic audit may have passed, but independent semantic evaluation is still required. Call `goal_evaluate` before requesting closure.\n"
    } else {
        ""
    };
    let note = format!(
        "<goal-continuation>\n\
         Continue the active Goal autonomously.\n\
         - Goal id: {}\n\
         - Revision: {}\n\
         - First call `goal_status` to verify the latest objective, revision, budget, and evidence.\n\
         {}\
         - If required criteria are satisfied, call `goal_finish_request` before the final user summary.\n\
         - If real progress is impossible, call `goal_block_request` with concrete attempts.\n\
         - Otherwise complete one meaningful step, update tasks/checkpoints/evidence, and continue until the Goal is done.\n\
         </goal-continuation>",
        snapshot.goal.id, snapshot.goal.revision, semantic_instruction
    );
    let admitted_global_stop_epoch = db.global_stop_epoch()?;
    let outcome = ha_core::wakeup::schedule(
        session_id,
        agent_id,
        GOAL_AUTO_CONTINUE_DELAY_SECS,
        Some(note),
        false,
        admitted_global_stop_epoch,
    )
    .map_err(|error| anyhow!("failed to schedule goal continuation: {error:?}"))?;
    let _ = db.append_goal_event(
        &snapshot.goal.id,
        "goal_auto_continue_scheduled",
        json!({
            "wakeupId": outcome.id,
            "fireAt": outcome.fire_at,
            "delaySecs": outcome.delay_secs,
            "source": source.as_str(),
            "turnId": turn_id,
            "assistantMessageId": assistant_message_id,
            "goalRevision": snapshot.goal.revision,
            "scheduledForRevision": scheduled_for_revision + 1,
        }),
    );
    Ok(Some(outcome))
}

fn goal_runner_should_wait_for_background_jobs(
    db: &SessionDB,
    session_id: &str,
    goal_id: &str,
) -> Result<bool> {
    let active_jobs = match ha_core::async_jobs::JobManager::list_active_work_by_session(session_id)
    {
        Ok(jobs) => jobs,
        Err(error) => {
            let _ = db.append_goal_event(
                goal_id,
                "goal_auto_continue_waiting_background_jobs",
                json!({
                    "reason": "background_jobs_read_failed",
                    "error": error.to_string(),
                }),
            );
            return Ok(true);
        }
    };
    if active_jobs.is_empty() {
        return Ok(false);
    }
    let _ = db.append_goal_event(
        goal_id,
        "goal_auto_continue_waiting_background_jobs",
        json!({
            "reason": "active_background_jobs",
            "activeJobs": active_jobs.iter().take(12).map(|job| {
                json!({
                    "jobId": job.job_id,
                    "kind": job.kind.as_str(),
                    "status": job.status.as_str(),
                    "toolName": job.tool_name,
                })
            }).collect::<Vec<_>>(),
            "activeCount": active_jobs.len(),
        }),
    );
    Ok(true)
}
