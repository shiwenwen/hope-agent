use crate::commands::CmdError;
use ha_core::coding_eval::{
    self, CodingEvalFixture, FixtureReport, GoldTaskPackReport, GoldTaskPackRunInput,
    GoldTaskPackSummary, StrategyEffectEvalInput, StrategyEffectReport,
};

#[tauri::command]
pub async fn run_coding_task_eval_fixture(
    fixture: CodingEvalFixture,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<FixtureReport, CmdError> {
    coding_eval::evaluate(app_state.session_db.clone(), &fixture)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_coding_eval_gold_tasks() -> Result<GoldTaskPackSummary, CmdError> {
    Ok(coding_eval::gold_task_pack_summary())
}

#[tauri::command]
pub async fn run_coding_eval_gold_task_pack(
    input: GoldTaskPackRunInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoldTaskPackReport, CmdError> {
    coding_eval::run_gold_task_pack(app_state.session_db.clone(), input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_coding_eval_strategy_effect(
    input: StrategyEffectEvalInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<StrategyEffectReport, CmdError> {
    coding_eval::evaluate_strategy_effect_with_recording(&app_state.session_db, input)
        .map_err(Into::into)
}
