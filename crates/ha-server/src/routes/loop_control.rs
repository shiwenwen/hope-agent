use axum::extract::{Path, Query};
use axum::Json;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopSnapshot, LoopTriggerKind,
    LoopWatchdogFinding, UpdateLoopSchedulePolicyInput,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::helpers::{cron_db, session_db};

pub async fn list_loop_schedules(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<LoopSchedule>>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    Ok(Json(
        db.run(move |db| db.list_loop_schedules_for_session_with_cron(&cron, &session_id, 100))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListLoopWatchdogQuery {
    #[serde(default)]
    pub grace_secs: Option<i64>,
}

pub async fn list_loop_watchdog_findings(
    Path(session_id): Path<String>,
    Query(query): Query<ListLoopWatchdogQuery>,
) -> Result<Json<Vec<LoopWatchdogFinding>>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    Ok(Json(
        db.run(move |db| {
            db.list_loop_watchdog_findings(&cron, &session_id, query.grace_secs.unwrap_or(120))
        })
        .await?,
    ))
}

pub async fn get_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<Option<LoopSnapshot>>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    Ok(Json(
        db.run(move |db| db.loop_snapshot_with_cron(&cron, &loop_id, 100))
            .await?,
    ))
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
    pub goal_criterion_id: Option<String>,
    #[serde(default)]
    pub max_runs: Option<i64>,
    #[serde(default)]
    pub max_runtime_secs: Option<i64>,
    #[serde(default)]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub cost_budget_micros: Option<i64>,
    #[serde(default)]
    pub max_no_progress_runs: Option<i64>,
    #[serde(default)]
    pub max_failures: Option<i64>,
    #[serde(default)]
    pub backoff_secs: Option<i64>,
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
    let db = session_db()?;
    let cron = cron_db()?.clone();
    db.run(move |db| {
        db.create_loop_schedule(
            &cron,
            CreateLoopScheduleInput {
                session_id,
                goal_id: body.goal_id,
                goal_criterion_id: body.goal_criterion_id,
                prompt: body.prompt.unwrap_or_default(),
                trigger_kind: kind,
                trigger_spec: body.trigger_spec,
                execution_strategy: strategy,
                max_runs: body.max_runs,
                max_runtime_secs: body.max_runtime_secs,
                token_budget: body.token_budget,
                cost_budget_micros: body.cost_budget_micros,
                max_no_progress_runs: body.max_no_progress_runs,
                max_failures: body.max_failures,
                backoff_secs: body.backoff_secs,
                agent_id: body.agent_id,
            },
        )
    })
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopPolicyBody {
    #[serde(default)]
    pub max_runs: Option<i64>,
    #[serde(default)]
    pub max_runtime_secs: Option<i64>,
    #[serde(default)]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub max_no_progress_runs: Option<i64>,
    #[serde(default)]
    pub max_failures: Option<i64>,
    #[serde(default)]
    pub backoff_secs: Option<i64>,
}

pub async fn update_loop_schedule_policy(
    Path(loop_id): Path<String>,
    Json(body): Json<UpdateLoopPolicyBody>,
) -> Result<Json<LoopSchedule>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    db.run(move |db| {
        db.update_loop_schedule_policy(
            &cron,
            UpdateLoopSchedulePolicyInput {
                loop_id,
                max_runs: body.max_runs,
                max_runtime_secs: body.max_runtime_secs,
                token_budget: body.token_budget,
                max_no_progress_runs: body.max_no_progress_runs,
                max_failures: body.max_failures,
                backoff_secs: body.backoff_secs,
            },
        )
    })
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn run_loop_schedule_now(Path(loop_id): Path<String>) -> Result<Json<Value>, AppError> {
    let cron = cron_db()?.clone();
    let db = session_db()?.clone();
    ha_core::blocking::run_blocking(move || {
        ha_core::loop_control::spawn_loop_schedule_run_now(&cron, &db, &loop_id)
    })
    .await
    .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "scheduled": true })))
}

pub async fn pause_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    db.run(move |db| db.pause_loop_schedule(&cron, &loop_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn resume_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    db.run(move |db| db.resume_loop_schedule(&cron, &loop_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn stop_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<LoopSchedule>, AppError> {
    let db = session_db()?;
    let cron = cron_db()?.clone();
    db.run(move |db| db.stop_loop_schedule(&cron, &loop_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
