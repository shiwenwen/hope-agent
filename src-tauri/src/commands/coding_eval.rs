use crate::commands::CmdError;
use ha_core::coding_eval::{self, CodingEvalFixture, FixtureReport};

#[tauri::command]
pub async fn run_coding_task_eval_fixture(
    fixture: CodingEvalFixture,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<FixtureReport, CmdError> {
    coding_eval::evaluate(app_state.session_db.clone(), &fixture)
        .await
        .map_err(Into::into)
}
