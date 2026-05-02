use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::db::SessionDB;

/// Emit a `task_updated` EventBus snapshot. Single source of truth for the
/// event shape — Tauri/HTTP shells, the `task_*` tools, and any future
/// task-mutating path all go through here so the frontend never sees stale
/// or shape-divergent updates.
pub fn emit_task_snapshot(session_id: &str, tasks: &[Task]) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "task_updated",
            json!({ "sessionId": session_id, "tasks": tasks }),
        );
    }
}

/// Set status on a task, emit the snapshot, return the post-update list.
/// The Tauri/HTTP shells delegate here so they stay as 1-line bodies.
pub fn set_task_status_and_snapshot(
    db: &SessionDB,
    id: i64,
    status: TaskStatus,
) -> Result<Vec<Task>> {
    let updated = db.update_task(id, Some(status), None, None)?;
    let tasks = db.list_tasks(&updated.session_id).unwrap_or_default();
    emit_task_snapshot(&updated.session_id, &tasks);
    Ok(tasks)
}

/// Delete a task, emit the post-delete snapshot, return the post-delete list.
pub fn delete_task_and_snapshot(db: &SessionDB, id: i64) -> Result<Vec<Task>> {
    let session_id = db.delete_task(id)?;
    let tasks = db.list_tasks(&session_id).unwrap_or_default();
    emit_task_snapshot(&session_id, &tasks);
    Ok(tasks)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "in_progress" => Some(TaskStatus::InProgress),
            "completed" => Some(TaskStatus::Completed),
            _ => None,
        }
    }
}

/// A session-scoped task tracked by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub session_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl SessionDB {
    pub fn create_task(
        &self,
        session_id: &str,
        content: &str,
        active_form: Option<&str>,
    ) -> Result<Task> {
        self.create_task_with_batch(session_id, content, active_form, None)
    }

    pub fn create_task_with_batch(
        &self,
        session_id: &str,
        content: &str,
        active_form: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Task> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO tasks (session_id, content, active_form, batch_id, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5)",
            params![session_id, content, active_form, batch_id, now],
        )?;
        Ok(Task {
            id: conn.last_insert_rowid(),
            session_id: session_id.to_string(),
            content: content.to_string(),
            active_form: active_form.map(|s| s.to_string()),
            batch_id: batch_id.map(|s| s.to_string()),
            status: TaskStatus::Pending.as_str().to_string(),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        content: Option<&str>,
        active_form: Option<&str>,
    ) -> Result<Task> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE tasks
                SET status = COALESCE(?1, status),
                    content = COALESCE(?2, content),
                    active_form = COALESCE(?3, active_form),
                    updated_at = ?4
                WHERE id = ?5",
            params![status.map(|s| s.as_str()), content, active_form, now, id],
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, content, active_form, batch_id, status, created_at, updated_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::row_to_task)?;
        match rows.next() {
            Some(Ok(task)) => Ok(task),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Err(anyhow::anyhow!("task {} not found", id)),
        }
    }

    /// Delete the row and return the session_id it belonged to in one round
    /// trip — callers need the session_id to refresh task list snapshots, so
    /// `DELETE … RETURNING` saves a separate `lookup_task_session` query.
    pub fn delete_task(&self, id: i64) -> Result<String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let session_id: Option<String> = conn
            .query_row(
                "DELETE FROM tasks WHERE id = ?1 RETURNING session_id",
                params![id],
                |row| row.get(0),
            )
            .ok();
        session_id.ok_or_else(|| anyhow::anyhow!("task {} not found", id))
    }

    /// Cheap existence probe for the streaming-loop hot path. Avoids the full
    /// row-deserialize + Vec allocation of `list_tasks` when the only question
    /// is "should I bother formatting the task reminder this round?". Lock +
    /// one prepared SELECT, returns on first non-completed row.
    pub fn has_active_tasks(&self, session_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT 1 FROM tasks WHERE session_id = ?1 AND status != 'completed' LIMIT 1",
        )?;
        let mut rows = stmt.query(params![session_id])?;
        Ok(rows.next()?.is_some())
    }

    pub fn list_tasks(&self, session_id: &str) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, content, active_form, batch_id, status, created_at, updated_at
             FROM tasks WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], Self::row_to_task)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub(crate) fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: row.get(0)?,
            session_id: row.get(1)?,
            content: row.get(2)?,
            active_form: row.get(3)?,
            batch_id: row.get(4)?,
            status: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}
