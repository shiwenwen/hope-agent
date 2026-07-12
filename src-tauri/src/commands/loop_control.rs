use crate::commands::CmdError;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopSnapshot, LoopTriggerKind,
    LoopWatchdogFinding, UpdateLoopSchedulePolicyInput,
};
use serde_json::Value;

#[tauri::command]
pub async fn list_loop_schedules(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<LoopSchedule>, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.list_loop_schedules_for_session_with_cron(&cron, &session_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_loop_watchdog_findings(
    session_id: String,
    grace_secs: Option<i64>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<LoopWatchdogFinding>, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.list_loop_watchdog_findings(&cron, &session_id, grace_secs.unwrap_or(120)))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<LoopSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.loop_snapshot_with_cron(&cron, &loop_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_loop_schedule(
    session_id: String,
    trigger_kind: String,
    trigger_spec: Value,
    execution_strategy: Option<String>,
    prompt: Option<String>,
    goal_id: Option<String>,
    goal_criterion_id: Option<String>,
    max_runs: Option<i64>,
    max_runtime_secs: Option<i64>,
    token_budget: Option<i64>,
    cost_budget_micros: Option<i64>,
    max_no_progress_runs: Option<i64>,
    max_failures: Option<i64>,
    backoff_secs: Option<i64>,
    agent_id: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    let kind = LoopTriggerKind::from_str(&trigger_kind)
        .ok_or_else(|| CmdError::msg(format!("Invalid loop trigger kind: {trigger_kind}")))?;
    let strategy = execution_strategy
        .as_deref()
        .map(|value| {
            LoopExecutionStrategy::from_str(value)
                .ok_or_else(|| CmdError::msg(format!("Invalid loop execution strategy: {value}")))
        })
        .transpose()?
        .unwrap_or_default();
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| {
        db.create_loop_schedule(
            &cron,
            CreateLoopScheduleInput {
                session_id,
                goal_id,
                goal_criterion_id,
                prompt: prompt.unwrap_or_default(),
                trigger_kind: kind,
                trigger_spec,
                execution_strategy: strategy,
                max_runs,
                max_runtime_secs,
                token_budget,
                cost_budget_micros,
                max_no_progress_runs,
                max_failures,
                backoff_secs,
                agent_id,
            },
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_loop_schedule_policy(
    loop_id: String,
    max_runs: Option<i64>,
    max_runtime_secs: Option<i64>,
    token_budget: Option<i64>,
    max_no_progress_runs: Option<i64>,
    max_failures: Option<i64>,
    backoff_secs: Option<i64>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| {
        db.update_loop_schedule_policy(
            &cron,
            UpdateLoopSchedulePolicyInput {
                loop_id,
                max_runs,
                max_runtime_secs,
                token_budget,
                max_no_progress_runs,
                max_failures,
                backoff_secs,
            },
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn run_loop_schedule_now(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    let cron = app_state.cron_db.clone();
    let session_db = app_state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_core::loop_control::spawn_loop_schedule_run_now(&cron, &session_db, &loop_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.pause_loop_schedule(&cron, &loop_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.resume_loop_schedule(&cron, &loop_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn stop_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    let db = app_state.session_db.clone();
    let cron = app_state.cron_db.clone();
    db.run(move |db| db.stop_loop_schedule(&cron, &loop_id))
        .await
        .map_err(Into::into)
}
