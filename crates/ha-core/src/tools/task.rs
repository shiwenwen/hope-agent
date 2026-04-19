use std::sync::Arc;

use serde_json::{json, Value};

use crate::session::{SessionDB, Task, TaskStatus};

fn resolve_ctx(session_id: Option<&str>) -> Result<(String, Arc<SessionDB>), String> {
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

fn collect_task_items(tasks_arr: &[Value]) -> Result<Vec<(String, Option<String>)>, String> {
    let mut items: Vec<(String, Option<String>)> = Vec::with_capacity(tasks_arr.len());
    for (idx, entry) in tasks_arr.iter().enumerate() {
        let obj = entry
            .as_object()
            .ok_or_else(|| format!("Error: tasks[{}] must be an object with 'content'", idx))?;
        let content = obj
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(content) = content else {
            continue;
        };
        let active_form = obj
            .get("activeForm")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        items.push((content, active_form));
    }
    Ok(items)
}

const ERR_TASKS_REQUIRED: &str = "Error: 'tasks' must be a non-empty array of \
{content, activeForm?}. Single-task calls still use the array form, e.g. \
tasks: [{content: \"Fix bug #42\"}].";

pub(crate) async fn tool_task_create(args: &Value, session_id: Option<&str>) -> String {
    let tasks_arr = match args.get("tasks").and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return ERR_TASKS_REQUIRED.to_string(),
    };

    let items = match collect_task_items(tasks_arr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if items.is_empty() {
        return "Error: no valid tasks — every entry had empty content after trimming".to_string();
    }

    let (sid, db) = match resolve_ctx(session_id) {
        Ok(v) => v,
        Err(e) => return e,
    };

    for (idx, (content, active_form)) in items.iter().enumerate() {
        if let Err(e) = db.create_task(&sid, content, active_form.as_deref()) {
            return format!(
                "Error: failed to create task #{} of {}: {}",
                idx + 1,
                items.len(),
                e
            );
        }
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
    let active_form = args
        .get("activeForm")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if status.is_none() && content.is_none() && active_form.is_none() {
        return "Error: at least one of 'status', 'content', or 'activeForm' must be provided"
            .to_string();
    }
    let (sid, db) = match resolve_ctx(session_id) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = db.update_task(id, status, content.as_deref(), active_form.as_deref()) {
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
