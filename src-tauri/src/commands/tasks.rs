use ha_core::session::{Task, TaskStatus};
use serde_json::json;

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

fn emit_snapshot(session_id: &str, tasks: &[Task]) {
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "task_updated",
            json!({ "sessionId": session_id, "tasks": tasks }),
        );
    }
}

#[tauri::command]
pub async fn list_session_tasks(session_id: String) -> Result<Vec<Task>, CmdError> {
    let db = db()?;
    db.list_tasks(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn update_task_status(id: i64, status: String) -> Result<Vec<Task>, CmdError> {
    let db = db()?;
    let parsed = parse_status(&status)?;
    let updated = db.update_task(id, Some(parsed), None, None)?;
    let tasks = db.list_tasks(&updated.session_id).unwrap_or_default();
    emit_snapshot(&updated.session_id, &tasks);
    Ok(tasks)
}

#[tauri::command]
pub async fn delete_task(id: i64) -> Result<Vec<Task>, CmdError> {
    let db = db()?;
    let session_id = db
        .lookup_task_session(id)?
        .ok_or_else(|| CmdError::msg(format!("task {} not found", id)))?;
    db.delete_task(id)?;
    let tasks = db.list_tasks(&session_id).unwrap_or_default();
    emit_snapshot(&session_id, &tasks);
    Ok(tasks)
}
