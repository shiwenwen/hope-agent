use axum::extract::Path;
use axum::Json;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopSnapshot, LoopState,
    LoopTriggerKind, UpdateLoopSchedulePolicyInput,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::helpers::{cron_db, session_db};

pub async fn list_loop_schedules(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<LoopSchedule>>, AppError> {
    Ok(Json(
        session_db()?.list_loop_schedules_for_session_with_cron(cron_db()?, &session_id, 100)?,
    ))
}

pub async fn get_loop_schedule(
    Path(loop_id): Path<String>,
) -> Result<Json<Option<LoopSnapshot>>, AppError> {
    Ok(Json(session_db()?.loop_snapshot_with_cron(
        cron_db()?,
        &loop_id,
        100,
    )?))
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
    session_db()?
        .create_loop_schedule(
            cron_db()?,
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
    session_db()?
        .update_loop_schedule_policy(
            cron_db()?,
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
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn run_loop_schedule_now(Path(loop_id): Path<String>) -> Result<Json<Value>, AppError> {
    if !ha_core::runtime_lock::is_primary() {
        return Err(AppError::bad_request(
            "run-now is unavailable on this instance: scheduled jobs only run on the primary",
        ));
    }
    let schedule = session_db()?
        .get_loop_schedule(&loop_id)?
        .ok_or_else(|| AppError::not_found("Loop schedule not found"))?;
    if schedule.state.is_terminal() {
        return Err(AppError::bad_request(format!(
            "loop schedule {} is {}",
            schedule.id,
            schedule.state.as_str()
        )));
    }
    if schedule.state != LoopState::Active {
        return Err(AppError::bad_request(format!(
            "loop schedule {} must be active before run-now; current state is {}",
            schedule.id,
            schedule.state.as_str()
        )));
    }
    let job = cron_db()?
        .get_job(&schedule.cron_job_id)?
        .ok_or_else(|| AppError::not_found("Cron job not found"))?;
    let cdb = cron_db()?.clone();
    let sdb = session_db()?.clone();
    tokio::spawn(async move {
        ha_core::cron::execute_job_public(&cdb, &sdb, &job).await;
    });
    Ok(Json(json!({ "scheduled": true })))
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
