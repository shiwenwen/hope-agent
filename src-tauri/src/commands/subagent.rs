use crate::commands::CmdError;
use crate::subagent;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_subagent_runs(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<subagent::SubagentRun>, CmdError> {
    state
        .session_db
        .list_subagent_runs(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_subagent_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Option<subagent::SubagentRun>, CmdError> {
    state
        .session_db
        .get_subagent_run(&run_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_subagent_runs_batch(
    run_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, subagent::SubagentRun>, CmdError> {
    state
        .session_db
        .get_subagent_runs_batch(&run_ids)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kill_subagent(run_id: String, state: State<'_, AppState>) -> Result<String, CmdError> {
    // Verify run exists
    let run = state
        .session_db
        .get_subagent_run(&run_id)?
        .ok_or_else(|| CmdError::msg(format!("Sub-agent run '{}' not found", run_id)))?;

    if run.status.is_terminal() {
        return Ok(format!(
            "Sub-agent already in terminal state: {}",
            run.status.as_str()
        ));
    }

    let cancelled = state.subagent_cancels.cancel(&run_id);
    if !cancelled {
        let _ = state.session_db.update_subagent_status(
            &run_id,
            subagent::SubagentStatus::Killed,
            None,
            Some("Killed from UI"),
            None,
            None,
        );
    }
    Ok(format!("Sub-agent '{}' killed", run_id))
}
