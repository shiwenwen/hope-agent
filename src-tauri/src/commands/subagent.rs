use crate::subagent;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_subagent_runs(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<subagent::SubagentRun>, String> {
    state
        .session_db
        .list_subagent_runs(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_subagent_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Option<subagent::SubagentRun>, String> {
    state
        .session_db
        .get_subagent_run(&run_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_subagent(run_id: String, state: State<'_, AppState>) -> Result<String, String> {
    // Verify run exists
    let run = state
        .session_db
        .get_subagent_run(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Sub-agent run '{}' not found", run_id))?;

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
