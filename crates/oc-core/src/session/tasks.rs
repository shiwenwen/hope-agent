use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::db::SessionDB;

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
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl SessionDB {
    pub fn create_task(&self, session_id: &str, content: &str) -> Result<Task> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO tasks (session_id, content, status, created_at, updated_at)
             VALUES (?1, ?2, 'pending', ?3, ?3)",
            params![session_id, content, now],
        )?;
        Ok(Task {
            id: conn.last_insert_rowid(),
            session_id: session_id.to_string(),
            content: content.to_string(),
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
                    updated_at = ?3
                WHERE id = ?4",
            params![status.map(|s| s.as_str()), content, now, id],
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, content, status, created_at, updated_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::row_to_task)?;
        match rows.next() {
            Some(Ok(task)) => Ok(task),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Err(anyhow::anyhow!("task {} not found", id)),
        }
    }

    pub fn list_tasks(&self, session_id: &str) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, content, status, created_at, updated_at
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
            status: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    }
}
