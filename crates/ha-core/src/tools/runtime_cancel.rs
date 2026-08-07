use anyhow::{anyhow, Result};
use serde_json::Value;

use super::ToolExecContext;
use crate::runtime_tasks::{cancel_runtime_task, CancelRuntimeTaskResult, RuntimeTaskKind};

const NOT_CONTROLLED_MESSAGE: &str =
    "Runtime task was not found or is not controlled by the current session";

/// Model-facing, session-scoped wrapper around the canonical runtime cancel
/// entry. Stop/session cleanup intentionally continue to call
/// `cancel_runtime_task` directly with identities captured by their own trusted
/// snapshot; an opaque id supplied by a model never gets that unscoped power.
pub(crate) async fn tool_runtime_cancel(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kind_str = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("runtime_cancel: missing required `kind` parameter"))?;
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("runtime_cancel: missing required `id` parameter"))?;

    let kind = match kind_str {
        "async_job" => RuntimeTaskKind::AsyncJob,
        "subagent" => RuntimeTaskKind::Subagent,
        "process" => RuntimeTaskKind::Process,
        // Historical/manual calls may still contain cron, but CronJob has no
        // creator-session ownership field. Never turn a bare id into an
        // app-global cancellation capability; use manage_cron/Loop's native
        // control plane with its own authorization semantics instead.
        "cron" => {
            return serialize_result(CancelRuntimeTaskResult::refused(
                RuntimeTaskKind::Cron,
                id,
                "ownership_unavailable",
                "Cron cancellation is unavailable through runtime_cancel; use its native control plane with its own authorization semantics",
            ));
        }
        other => return Err(anyhow!("runtime_cancel: unknown kind `{}`", other)),
    };

    let Some(session_id) = ctx.session_id.as_deref() else {
        return serialize_result(refused_not_controlled(kind, id));
    };

    let controlled = match kind {
        RuntimeTaskKind::AsyncJob => async_job_is_controlled(id, session_id).await?,
        RuntimeTaskKind::Subagent => subagent_is_controlled(id, session_id, ctx).await?,
        RuntimeTaskKind::Process => process_is_controlled(id, session_id).await,
        RuntimeTaskKind::Cron => false,
    };
    if !controlled {
        return serialize_result(refused_not_controlled(kind, id));
    }

    // Ownership fields are immutable for async jobs/subagent attempts/process
    // sessions. Cancellation still goes through the established canonical
    // lifecycle entry so queued claims, durable projections, and tokens stay in
    // sync.
    serialize_result(cancel_runtime_task(kind, id).await?)
}

fn serialize_result(result: CancelRuntimeTaskResult) -> Result<String> {
    Ok(serde_json::to_string(&result)?)
}

fn refused_not_controlled(kind: RuntimeTaskKind, id: &str) -> CancelRuntimeTaskResult {
    CancelRuntimeTaskResult::refused(kind, id, "not_controlled", NOT_CONTROLLED_MESSAGE)
}

async fn async_job_is_controlled(id: &str, session_id: &str) -> Result<bool> {
    let Some(db) = crate::async_jobs::get_async_jobs_db().cloned() else {
        return Ok(false);
    };
    let id = id.to_string();
    let session_id = session_id.to_string();
    crate::blocking::run_blocking(move || {
        Ok(db.load(&id)?.is_some_and(|job| {
            runtime_task_session_owner_matches(job.session_id.as_deref(), &session_id)
        }))
    })
    .await
}

fn runtime_task_session_owner_matches(owner_session_id: Option<&str>, session_id: &str) -> bool {
    owner_session_id == Some(session_id)
}

async fn subagent_is_controlled(id: &str, session_id: &str, ctx: &ToolExecContext) -> Result<bool> {
    // `cancel_runtime_task(Subagent, ..)` uses the process-global session DB.
    // Authorize against that exact same store: accepting a colliding run id
    // from a different bound/test DB and then cancelling the global row would
    // turn the preflight into a cross-store confused-deputy bug.
    let Some(db) = crate::get_session_db().cloned() else {
        return Ok(false);
    };
    let id = id.to_string();
    let run = db.run(move |db| db.get_subagent_run(&id)).await?;
    Ok(run.is_some_and(|run| subagent_owner_matches(&run, session_id, ctx)))
}

fn subagent_owner_matches(
    run: &crate::subagent::SubagentRun,
    session_id: &str,
    ctx: &ToolExecContext,
) -> bool {
    if run.parent_session_id != session_id {
        return false;
    }
    match ctx.workflow_run_id.as_deref() {
        Some(workflow_run_id) => {
            run.owner_kind == crate::subagent::SubagentOwnerKind::Workflow
                && run.owner_id == workflow_run_id
        }
        None => {
            run.owner_kind == crate::subagent::SubagentOwnerKind::ParentSession
                && run.owner_id == session_id
        }
    }
}

async fn process_is_controlled(id: &str, session_id: &str) -> bool {
    let registry = crate::process_registry::get_registry().lock().await;
    registry.get_session(id).is_some_and(|process| {
        runtime_task_session_owner_matches(process.parent_session_id.as_deref(), session_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subagent_run(
        owner_kind: crate::subagent::SubagentOwnerKind,
        owner_id: &str,
    ) -> crate::subagent::SubagentRun {
        crate::subagent::SubagentRun {
            run_id: "run-1".into(),
            parent_session_id: "session-1".into(),
            owner_kind,
            owner_id: owner_id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn ordinary_session_cannot_control_team_workflow_or_internal_subagents() {
        let ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            ..Default::default()
        };
        assert!(subagent_owner_matches(
            &subagent_run(
                crate::subagent::SubagentOwnerKind::ParentSession,
                "session-1"
            ),
            "session-1",
            &ctx,
        ));
        for owner in [
            crate::subagent::SubagentOwnerKind::Workflow,
            crate::subagent::SubagentOwnerKind::Team,
            crate::subagent::SubagentOwnerKind::Internal,
        ] {
            assert!(!subagent_owner_matches(
                &subagent_run(owner, "session-1"),
                "session-1",
                &ctx,
            ));
        }
    }

    #[test]
    fn async_job_and_process_require_exact_explicit_session_owner() {
        assert!(runtime_task_session_owner_matches(
            Some("session-1"),
            "session-1"
        ));
        assert!(!runtime_task_session_owner_matches(
            Some("session-2"),
            "session-1"
        ));
        assert!(!runtime_task_session_owner_matches(None, "session-1"));
    }

    #[test]
    fn workflow_context_only_controls_its_exact_owner_lineage() {
        let ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            workflow_run_id: Some("workflow-1".into()),
            ..Default::default()
        };
        assert!(subagent_owner_matches(
            &subagent_run(crate::subagent::SubagentOwnerKind::Workflow, "workflow-1"),
            "session-1",
            &ctx,
        ));
        assert!(!subagent_owner_matches(
            &subagent_run(crate::subagent::SubagentOwnerKind::Workflow, "workflow-2"),
            "session-1",
            &ctx,
        ));
    }

    #[tokio::test]
    async fn cron_is_structurally_refused_without_calling_the_global_hook() {
        let output = tool_runtime_cancel(
            &serde_json::json!({ "kind": "cron", "id": "cron-1" }),
            &ToolExecContext {
                session_id: Some("session-1".into()),
                ..Default::default()
            },
        )
        .await
        .expect("structured refusal");
        let value: Value = serde_json::from_str(&output).expect("json");
        assert_eq!(value["accepted"], false);
        assert_eq!(value["disposition"], "refused");
        assert_eq!(value["status"], "refused");
        assert_eq!(value["reason"], "ownership_unavailable");
    }

    #[tokio::test]
    async fn supported_kinds_without_session_context_are_structurally_refused() {
        for kind in ["async_job", "subagent", "process"] {
            let output = tool_runtime_cancel(
                &serde_json::json!({ "kind": kind, "id": "opaque-id" }),
                &ToolExecContext::default(),
            )
            .await
            .expect("structured refusal");
            let value: Value = serde_json::from_str(&output).expect("json");
            assert_eq!(value["accepted"], false, "kind={kind}");
            assert_eq!(value["disposition"], "refused", "kind={kind}");
            assert_eq!(value["reason"], "not_controlled", "kind={kind}");
        }
    }
}
