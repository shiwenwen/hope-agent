use axum::extract::Path;
use axum::Json;
use ha_core::session::{delete_task_and_snapshot, set_task_status_and_snapshot, Task, TaskStatus};
use serde::Deserialize;

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

pub async fn list_session_tasks(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<Task>>, AppError> {
    Ok(Json(db()?.list_tasks(&session_id)?))
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
    Ok(Json(
        set_task_status_and_snapshot(&db, id, parse_status(&body.status)?).await?,
    ))
}

pub async fn delete_task(Path(id): Path<i64>) -> Result<Json<Vec<Task>>, AppError> {
    let db = db()?;
    Ok(Json(delete_task_and_snapshot(&db, id)?))
}
