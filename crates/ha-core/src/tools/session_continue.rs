use anyhow::{anyhow, Result};
use serde_json::Value;

use super::ToolExecContext;

/// Model-facing adapter for the canonical Stop/Continue service.
///
/// The target is always derived from the bound tool context; the model never
/// supplies a session id. The execution context must prove that the invocation
/// belongs to a new foreground user turn; prompt instructions and Stop-watcher
/// timing are not authorization boundaries.
pub(crate) async fn tool_session_continue(_args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let pause_id = _args
        .get("pause_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("session_continue requires the current system pause_id"))?;
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("session_continue requires a current session"))?;
    ensure_current_turn_can_continue(ctx, session_id)?;
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| anyhow!("session_continue: session database is unavailable"))?;

    let lookup_session_id = session_id.to_string();
    let pause = db
        .clone()
        .run(move |db| db.active_session_or_ancestor_autonomy_pause(&lookup_session_id))
        .await?;
    let Some(pause) = pause else {
        return Ok(serde_json::to_string(&serde_json::json!({
            "resumed": false,
            "reason": "not_paused",
            "message": "The current session has no active Stop receipt. Continue the user's request normally."
        }))?);
    };
    if pause.id != pause_id {
        return Ok(serde_json::to_string(&serde_json::json!({
            "resumed": false,
            "reason": "stale_pause",
            "message": "The supplied Stop receipt is no longer current. Do not retry with a different id unless a fresh <session-paused> system reminder explicitly provides it."
        }))?);
    }
    ensure_current_turn_can_continue(ctx, &pause.session_id)?;

    let outcome =
        crate::chat_engine::stop::continue_session_from_pause(db, &pause.session_id, pause_id)
            .await?;
    Ok(serde_json::to_string(&serde_json::json!({
        "resumed": outcome.resumed,
        "pauseId": outcome.pause_id,
        "goalId": outcome.goal_id,
        "workflowRunIds": outcome.workflow_run_ids,
        "subagentRunIds": outcome.subagent_run_ids,
        "message": "The Stop fence is cleared. Inspect captured results before resuming unfinished threads; do not repeat completed side effects. A captured sub-agent may still be converging to session_paused, so check or wait for terminal state before creating its continuation."
    }))?)
}

fn ensure_current_turn_can_continue(ctx: &ToolExecContext, session_id: &str) -> Result<()> {
    if ctx.turn_provenance != crate::tool_defs::ToolTurnProvenance::ForegroundUser {
        anyhow::bail!(
            "session_continue requires a foreground user turn; autonomous work cannot clear Stop"
        );
    }
    if ctx
        .cancellation_token
        .as_ref()
        .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        || crate::chat_engine::active_turn::stop_cleanup_active(session_id)
    {
        anyhow::bail!(
            "session_continue was superseded by a newer Stop; the pause fence remains active"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::tool_defs::{SessionDbHandle, ToolTurnProvenance};

    fn foreground_context(
        db: Arc<crate::session::SessionDB>,
        session_id: String,
    ) -> ToolExecContext {
        ToolExecContext {
            session_id: Some(session_id),
            session_db: Some(SessionDbHandle(db)),
            turn_provenance: ToolTurnProvenance::ForegroundUser,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn current_child_context_resumes_its_inherited_root_stop_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &dir.path().join("session-continue.db"),
            )
            .expect("session db"),
        );
        let root = db.create_session("ha-main").expect("root session");
        let child = db
            .create_session_with_parent("helper", Some(&root.id))
            .expect("child session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: root.id.clone(),
                objective: "Resume only after explicit user intent".to_string(),
                completion_criteria: "The durable fence is consumed".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("goal");
        let pause = db
            .prepare_session_autonomy_pause(&root.id)
            .expect("pause receipt");
        db.pause_goal(&goal.goal.id).expect("pause goal");

        let output = tool_session_continue(
            &serde_json::json!({ "pause_id": pause.id }),
            &foreground_context(db.clone(), child.id.clone()),
        )
        .await
        .expect("continue result");
        let value: Value = serde_json::from_str(&output).expect("json result");

        assert_eq!(value["resumed"], true);
        assert_eq!(value["pauseId"], pause.id);
        assert_eq!(value["goalId"], goal.goal.id);
        assert!(!db.is_session_autonomy_paused(&root.id).unwrap());
        assert_eq!(
            db.get_goal(&goal.goal.id).unwrap().unwrap().state,
            crate::goal::GoalState::Active
        );
    }

    #[tokio::test]
    async fn no_active_stop_receipt_is_a_structured_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &dir.path().join("session-continue-noop.db"),
            )
            .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");

        let output = tool_session_continue(
            &serde_json::json!({ "pause_id": "pause_missing" }),
            &foreground_context(db, session.id),
        )
        .await
        .expect("continue noop");
        let value: Value = serde_json::from_str(&output).expect("json result");

        assert_eq!(value["resumed"], false);
        assert_eq!(value["reason"], "not_paused");
    }

    #[tokio::test]
    async fn stale_pause_id_cannot_consume_the_current_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &dir.path().join("session-continue-stale.db"),
            )
            .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("pause receipt");

        let output = tool_session_continue(
            &serde_json::json!({ "pause_id": "pause_stale" }),
            &foreground_context(db.clone(), session.id.clone()),
        )
        .await
        .expect("stale continue result");
        let value: Value = serde_json::from_str(&output).expect("json result");

        assert_eq!(value["resumed"], false);
        assert_eq!(value["reason"], "stale_pause");
        assert_eq!(
            db.active_session_autonomy_pause(&session.id)
                .unwrap()
                .expect("receipt remains")
                .id,
            pause.id
        );
    }

    #[tokio::test]
    async fn a_new_stop_signal_wins_over_the_model_continue_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &dir.path().join("session-continue-cancelled.db"),
            )
            .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("pause receipt");
        let cancellation_token = tokio_util::sync::CancellationToken::new();
        cancellation_token.cancel();

        let error = tool_session_continue(
            &serde_json::json!({ "pause_id": pause.id }),
            &ToolExecContext {
                session_id: Some(session.id.clone()),
                session_db: Some(SessionDbHandle(db.clone())),
                turn_provenance: ToolTurnProvenance::ForegroundUser,
                cancellation_token: Some(cancellation_token),
                ..Default::default()
            },
        )
        .await
        .expect_err("newer Stop must win");

        assert!(error.to_string().contains("superseded by a newer Stop"));
        assert!(db.is_session_autonomy_paused(&session.id).unwrap());
    }

    #[tokio::test]
    async fn autonomous_turn_cannot_consume_a_stop_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &dir.path().join("session-continue-autonomous.db"),
            )
            .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("pause receipt");

        let error = tool_session_continue(
            &serde_json::json!({ "pause_id": pause.id }),
            &ToolExecContext {
                session_id: Some(session.id.clone()),
                session_db: Some(SessionDbHandle(db.clone())),
                turn_provenance: ToolTurnProvenance::Autonomous,
                ..Default::default()
            },
        )
        .await
        .expect_err("autonomous work must not clear a user Stop");

        assert!(error.to_string().contains("foreground user turn"));
        assert_eq!(
            db.active_session_autonomy_pause(&session.id)
                .unwrap()
                .expect("receipt remains")
                .id,
            pause.id
        );
    }
}
