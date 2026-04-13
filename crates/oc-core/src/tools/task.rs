use std::sync::Arc;

use serde_json::{json, Value};

use crate::session::{SessionDB, Task, TaskStatus};

fn resolve_ctx(
    session_id: Option<&str>,
) -> Result<(String, Arc<SessionDB>), String> {
    let sid = session_id
        .ok_or_else(|| "Error: no session context available".to_string())?
        .to_string();
    let db = crate::get_session_db()
        .ok_or_else(|| "Error: session database unavailable".to_string())?
        .clone();
    Ok((sid, db))
}

fn emit_snapshot(session_id: &str, tasks: &[Task]) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "task_updated",
            json!({ "sessionId": session_id, "tasks": tasks }),
        );
    }
}

fn render_snapshot(tasks: &[Task]) -> String {
    serde_json::to_string(tasks).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) async fn tool_task_create(args: &Value, session_id: Option<&str>) -> String {
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return "Error: content parameter is required (non-empty string)".to_string(),
    };
    let (sid, db) = match resolve_ctx(session_id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = db.create_task(&sid, &content) {
        return format!("Error: failed to create task: {}", e);
    }
    let tasks = db.list_tasks(&sid).unwrap_or_default();
    emit_snapshot(&sid, &tasks);
    render_snapshot(&tasks)
}

pub(crate) async fn tool_task_update(args: &Value, session_id: Option<&str>) -> String {
    let id = match args.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return "Error: id parameter is required (integer)".to_string(),
    };
    let status = match args.get("status").and_then(|v| v.as_str()) {
        Some(s) => match TaskStatus::from_str(s) {
            Some(st) => Some(st),
            None => {
                return format!(
                    "Error: invalid status '{}'. Must be one of: pending, in_progress, completed",
                    s
                )
            }
        },
        None => None,
    };
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if status.is_none() && content.is_none() {
        return "Error: at least one of 'status' or 'content' must be provided".to_string();
    }
    let (sid, db) = match resolve_ctx(session_id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = db.update_task(id, status, content.as_deref()) {
        return format!("Error: failed to update task #{}: {}", id, e);
    }
    let tasks = db.list_tasks(&sid).unwrap_or_default();
    emit_snapshot(&sid, &tasks);
    render_snapshot(&tasks)
}

pub(crate) async fn tool_task_list(_args: &Value, session_id: Option<&str>) -> String {
    let (sid, db) = match resolve_ctx(session_id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match db.list_tasks(&sid) {
        Ok(tasks) => render_snapshot(&tasks),
        Err(e) => format!("Error: failed to list tasks: {}", e),
    }
}
