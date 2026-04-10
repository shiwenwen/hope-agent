use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::subagent;

use crate::error::AppError;
use crate::routes::helpers::app_state as state;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub session_id: String,
}

/// `GET /api/subagent/runs?session_id=...`
pub async fn list_subagent_runs(
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<subagent::SubagentRun>>, AppError> {
    Ok(Json(state()?.session_db.list_subagent_runs(&q.session_id)?))
}

/// `GET /api/subagent/runs/{run_id}`
pub async fn get_subagent_run(Path(run_id): Path<String>) -> Result<Json<Value>, AppError> {
    Ok(Json(serde_json::to_value(
        state()?.session_db.get_subagent_run(&run_id)?,
    )?))
}

/// `POST /api/subagent/runs/{run_id}/kill`
pub async fn kill_subagent(Path(run_id): Path<String>) -> Result<Json<Value>, AppError> {
    let s = state()?;
    let run = s
        .session_db
        .get_subagent_run(&run_id)?
        .ok_or_else(|| AppError::not_found(format!("Sub-agent run '{}' not found", run_id)))?;
    if run.status.is_terminal() {
        return Ok(Json(
            json!({ "status": format!("Sub-agent already in terminal state: {}", run.status.as_str()) }),
        ));
    }
    let cancelled = s.subagent_cancels.cancel(&run_id);
    if !cancelled {
        let _ = s.session_db.update_subagent_status(
            &run_id,
            subagent::SubagentStatus::Killed,
            None,
            Some("Killed from UI"),
            None,
            None,
        );
    }
    Ok(Json(json!({ "killed": true })))
}
