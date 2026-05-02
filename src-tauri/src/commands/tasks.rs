use ha_core::session::{delete_task_and_snapshot, set_task_status_and_snapshot, Task, TaskStatus};

use crate::commands::CmdError;

fn db() -> Result<std::sync::Arc<ha_core::session::SessionDB>, CmdError> {
    ha_core::get_session_db()
        .ok_or_else(|| CmdError::msg("Session DB not initialized"))
        .cloned()
}

fn parse_status(status: &str) -> Result<TaskStatus, CmdError> {
    TaskStatus::from_str(status).ok_or_else(|| {
        CmdError::msg(format!(
            "invalid status '{}': must be pending | in_progress | completed",
            status
        ))
    })
}

#[tauri::command]
pub async fn list_session_tasks(session_id: String) -> Result<Vec<Task>, CmdError> {
    Ok(db()?.list_tasks(&session_id)?)
}

#[tauri::command]
pub async fn update_task_status(id: i64, status: String) -> Result<Vec<Task>, CmdError> {
    let db = db()?;
    Ok(set_task_status_and_snapshot(&db, id, parse_status(&status)?).await?)
}

#[tauri::command]
pub async fn delete_task(id: i64) -> Result<Vec<Task>, CmdError> {
    let db = db()?;
    Ok(delete_task_and_snapshot(&db, id).await?)
}
