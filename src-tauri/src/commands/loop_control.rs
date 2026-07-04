use crate::commands::CmdError;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopSnapshot, LoopTriggerKind,
};
use serde_json::Value;

#[tauri::command]
pub async fn list_loop_schedules(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<LoopSchedule>, CmdError> {
    app_state
        .session_db
        .list_loop_schedules_for_session(&session_id, 100)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<LoopSnapshot>, CmdError> {
    app_state
        .session_db
        .loop_snapshot(&loop_id, 100)
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
    max_runs: Option<i64>,
    max_runtime_secs: Option<i64>,
    token_budget: Option<i64>,
    cost_budget_micros: Option<i64>,
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
    app_state
        .session_db
        .create_loop_schedule(
            &app_state.cron_db,
            CreateLoopScheduleInput {
                session_id,
                goal_id,
                prompt: prompt.unwrap_or_default(),
                trigger_kind: kind,
                trigger_spec,
                execution_strategy: strategy,
                max_runs,
                max_runtime_secs,
                token_budget,
                cost_budget_micros,
                agent_id,
            },
        )
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    app_state
        .session_db
        .pause_loop_schedule(&app_state.cron_db, &loop_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    app_state
        .session_db
        .resume_loop_schedule(&app_state.cron_db, &loop_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn stop_loop_schedule(
    loop_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<LoopSchedule, CmdError> {
    app_state
        .session_db
        .stop_loop_schedule(&app_state.cron_db, &loop_id)
        .map_err(Into::into)
}
