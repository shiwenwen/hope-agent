use anyhow::Result;
use std::sync::Arc;

use crate::session::SessionDB;
use super::events::emit_team_event;
use super::types::*;

/// Create a new team task.
pub fn create_task(
    db: &Arc<SessionDB>,
    team_id: &str,
    content: &str,
    owner_member_id: Option<&str>,
    priority: Option<u32>,
    blocked_by: Vec<i64>,
) -> Result<TeamTask> {
    let now = chrono::Utc::now().to_rfc3339();
    let task = TeamTask {
        id: 0, // will be set by DB
        team_id: team_id.to_string(),
        content: content.to_string(),
        status: "pending".to_string(),
        owner_member_id: owner_member_id.map(|s| s.to_string()),
        priority: priority.unwrap_or(100),
        blocked_by,
        blocks: Vec::new(),
        column_name: if owner_member_id.is_some() {
            "doing".to_string()
        } else {
            "todo".to_string()
        },
        created_at: now.clone(),
        updated_at: now,
    };

    let id = db.insert_team_task(&task)?;
    let mut task = task;
    task.id = id;

    // If the task has an owner, update the member's current_task_id
    if let Some(owner) = &task.owner_member_id {
        let _ = db.update_team_member_task(owner, Some(id));
    }

    emit_team_event("task_updated", &task);

    // Post system message
    let msg = if let Some(owner) = &task.owner_member_id {
        format!("Task #{} created and assigned to {}: {}", id, owner, content)
    } else {
        format!("Task #{} created: {}", id, content)
    };
    let _ = super::messaging::post_system_message(db, team_id, &msg);

    Ok(task)
}

/// Update a team task (status, owner, column, content).
pub fn update_task(
    db: &Arc<SessionDB>,
    team_id: &str,
    task_id: i64,
    status: Option<&str>,
    owner: Option<&str>,
    column: Option<&str>,
    content: Option<&str>,
) -> Result<TeamTask> {
    db.update_team_task(task_id, status, owner, column, content)?;

    // If owner changed, update member's current_task_id
    if let Some(new_owner) = owner {
        let _ = db.update_team_member_task(new_owner, Some(task_id));
    }

    let task = db
        .get_team_task(task_id)?
        .ok_or_else(|| anyhow::anyhow!("Task {} not found", task_id))?;

    emit_team_event("task_updated", &task);

    // Post system message for significant changes
    if let Some(s) = status {
        if s == "completed" {
            let owner_name = task.owner_member_id.as_deref().unwrap_or("unknown");
            let _ = super::messaging::post_system_message(
                db,
                team_id,
                &format!("Task #{} completed by {}: {}", task_id, owner_name, task.content),
            );
        }
    }

    Ok(task)
}

/// List all tasks for a team with optional status filter.
pub fn list_tasks(
    db: &Arc<SessionDB>,
    team_id: &str,
    status_filter: Option<&str>,
) -> Result<Vec<TeamTask>> {
    let all = db.list_team_tasks(team_id)?;
    if let Some(filter) = status_filter {
        Ok(all.into_iter().filter(|t| t.status == filter).collect())
    } else {
        Ok(all)
    }
}
