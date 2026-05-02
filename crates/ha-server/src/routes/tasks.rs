use axum::extract::Path;
use axum::Json;
use ha_core::session::{Task, TaskStatus};
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;

fn db() -> Result<std::sync::Arc<ha_core::session::SessionDB>, AppError> {
    ha_core::get_session_db()
        .ok_or_else(|| AppError::internal("Session DB not initialized"))
        .cloned()
}

fn parse_status(status: &str) -> Result<TaskStatus, AppError> {
    TaskStatus::from_str(status).ok_or_else(|| {
        AppError::bad_request(format!(
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

pub async fn list_session_tasks(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<Task>>, AppError> {
    let db = db()?;
    Ok(Json(db.list_tasks(&session_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskStatusBody {
    pub status: String,
}

pub async fn update_task_status(
    Path(id): Path<i64>,
    Json(body): Json<UpdateTaskStatusBody>,
) -> Result<Json<Vec<Task>>, AppError> {
    let db = db()?;
    let parsed = parse_status(&body.status)?;
    let updated = db.update_task(id, Some(parsed), None, None)?;
    let tasks = db.list_tasks(&updated.session_id).unwrap_or_default();
    emit_snapshot(&updated.session_id, &tasks);
    Ok(Json(tasks))
}

pub async fn delete_task(Path(id): Path<i64>) -> Result<Json<Vec<Task>>, AppError> {
    let db = db()?;
    let session_id = db
        .lookup_task_session(id)?
        .ok_or_else(|| AppError::not_found(format!("task {} not found", id)))?;
    db.delete_task(id)?;
    let tasks = db.list_tasks(&session_id).unwrap_or_default();
    emit_snapshot(&session_id, &tasks);
    Ok(Json(tasks))
}
