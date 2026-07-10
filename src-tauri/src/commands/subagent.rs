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
        .run(move |db| db.list_subagent_runs(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_subagent_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Option<subagent::SubagentRun>, CmdError> {
    state
        .session_db
        .run(move |db| db.get_subagent_run(&run_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_subagent_runs_batch(
    run_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, subagent::SubagentRun>, CmdError> {
    state
        .session_db
        .run(move |db| db.get_subagent_runs_batch(&run_ids))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kill_subagent(run_id: String, state: State<'_, AppState>) -> Result<String, CmdError> {
    // Verify run exists
    let run = {
        let run_id = run_id.clone();
        state
            .session_db
            .run(move |db| db.get_subagent_run(&run_id))
            .await?
    }
    .ok_or_else(|| CmdError::msg(format!("Sub-agent run '{}' not found", run_id)))?;

    if run.status.is_terminal() {
        return Ok(format!(
            "Sub-agent already in terminal state: {}",
            run.status.as_str()
        ));
    }

    let cancelled = state.subagent_cancels.cancel(&run_id);
    if !cancelled {
        let run_id = run_id.clone();
        let _ = state
            .session_db
            .run(move |db| {
                db.update_subagent_status(
                    &run_id,
                    subagent::SubagentStatus::Killed,
                    None,
                    Some("Killed from UI"),
                    None,
                    None,
                )
            })
            .await;
    }
    Ok(format!("Sub-agent '{}' killed", run_id))
}
