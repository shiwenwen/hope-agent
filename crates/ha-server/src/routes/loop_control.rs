use axum::extract::Path;
use axum::Json;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopSnapshot, LoopTriggerKind,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::routes::helpers::{cron_db, session_db};

pub async fn list_loop_schedules(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<LoopSchedule>>, AppError> {
    Ok(Json(
        session_db()?.list_loop_schedules_for_session(&session_id, 100)?,
    ))
}

pub async fn get_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<Option<LoopSnapshot>>, AppError> {
    Ok(Json(session_db()?.loop_snapshot(&loop_id, 100)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopScheduleBody {
    pub trigger_kind: String,
    pub trigger_spec: Value,
    #[serde(default)]
    pub execution_strategy: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub max_runs: Option<i64>,
    #[serde(default)]
    pub max_runtime_secs: Option<i64>,
    #[serde(default)]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub cost_budget_micros: Option<i64>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

pub async fn create_loop_schedule(
    Path(session_id): Path<String>,
    Json(body): Json<CreateLoopScheduleBody>,
) -> Result<Json<LoopSchedule>, AppError> {
    let kind = LoopTriggerKind::from_str(&body.trigger_kind)
        .ok_or_else(|| AppError::bad_request("Invalid loop trigger kind"))?;
    let strategy = body
        .execution_strategy
        .as_deref()
        .map(|value| {
            LoopExecutionStrategy::from_str(value)
                .ok_or_else(|| AppError::bad_request("Invalid loop execution strategy"))
        })
        .transpose()?
        .unwrap_or_default();
    session_db()?
        .create_loop_schedule(
            cron_db()?,
            CreateLoopScheduleInput {
                session_id,
                goal_id: body.goal_id,
                prompt: body.prompt.unwrap_or_default(),
                trigger_kind: kind,
                trigger_spec: body.trigger_spec,
                execution_strategy: strategy,
                max_runs: body.max_runs,
                max_runtime_secs: body.max_runtime_secs,
                token_budget: body.token_budget,
                cost_budget_micros: body.cost_budget_micros,
                agent_id: body.agent_id,
            },
        )
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn pause_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    session_db()?
        .pause_loop_schedule(cron_db()?, &loop_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn resume_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    session_db()?
        .resume_loop_schedule(cron_db()?, &loop_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn stop_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    session_db()?
        .stop_loop_schedule(cron_db()?, &loop_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
