use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasProject {
    pub id: String,
    pub title: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasVersion {
    pub id: i64,
    pub project_id: String,
    pub version_number: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub js: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub created_at: String,
}

// ── Database ───────────────────────────────────────────────────────

const PROJECT_COLUMNS: &str =
    "SELECT id, title, content_type, session_id, agent_id, created_at, updated_at, version_count, metadata
     FROM canvas_projects";

fn map_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasProject> {
    Ok(CanvasProject {
        id: row.get(0)?,
        title: row.get(1)?,
        content_type: row.get(2)?,
        session_id: row.get(3)?,
        agent_id: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        version_count: row.get(7)?,
        metadata: row.get(8)?,
    })
}

pub struct CanvasDB {
    conn: Mutex<Connection>,
}

impl CanvasDB {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS canvas_projects (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content_type TEXT NOT NULL DEFAULT 'html',
                session_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                version_count INTEGER DEFAULT 1,
                metadata TEXT
            );

            CREATE TABLE IF NOT EXISTS canvas_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL REFERENCES canvas_projects(id) ON DELETE CASCADE,
                version_number INTEGER NOT NULL,
                message TEXT,
                html TEXT,
                css TEXT,
                js TEXT,
                content TEXT,
                created_at TEXT NOT NULL,
                UNIQUE(project_id, version_number)
            );

            CREATE INDEX IF NOT EXISTS idx_canvas_versions_project
                ON canvas_versions(project_id, version_number DESC);

            CREATE INDEX IF NOT EXISTS idx_canvas_projects_session
                ON canvas_projects(session_id, updated_at DESC);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ── Projects ───────────────────────────────────────────────────

    pub fn create_project(&self, project: &CanvasProject) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO canvas_projects (id, title, content_type, session_id, agent_id, created_at, updated_at, version_count, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                project.id,
                project.title,
                project.content_type,
                project.session_id,
                project.agent_id,
                project.created_at,
                project.updated_at,
                project.version_count,
                project.metadata,
            ],
        )?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<CanvasProject>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS} WHERE id = ?1"))?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_project_row)?;
        match rows.next() {
            Some(Ok(p)) => Ok(Some(p)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<CanvasProject>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS} ORDER BY updated_at DESC"))?;
        let rows = stmt.query_map([], map_project_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_projects_by_session(&self, session_id: &str) -> Result<Vec<CanvasProject>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(&format!(
            "{PROJECT_COLUMNS} WHERE session_id = ?1 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map(rusqlite::params![session_id], map_project_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_project_meta(
        &self,
        id: &str,
        title: Option<&str>,
        updated_at: &str,
        version_count: i64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        if let Some(t) = title {
            conn.execute(
                "UPDATE canvas_projects SET title = ?1, updated_at = ?2, version_count = ?3 WHERE id = ?4",
                rusqlite::params![t, updated_at, version_count, id],
            )?;
        } else {
            conn.execute(
                "UPDATE canvas_projects SET updated_at = ?1, version_count = ?2 WHERE id = ?3",
                rusqlite::params![updated_at, version_count, id],
            )?;
        }
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        conn.execute(
            "DELETE FROM canvas_projects WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    // ── Versions ───────────────────────────────────────────────────

    pub fn create_version(&self, version: &CanvasVersion) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO canvas_versions (project_id, version_number, message, html, css, js, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                version.project_id,
                version.version_number,
                version.message,
                version.html,
                version.css,
                version.js,
                version.content,
                version.created_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_versions(&self, project_id: &str) -> Result<Vec<CanvasVersion>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, version_number, message, html, css, js, content, created_at
             FROM canvas_versions WHERE project_id = ?1 ORDER BY version_number DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| {
            Ok(CanvasVersion {
                id: row.get(0)?,
                project_id: row.get(1)?,
                version_number: row.get(2)?,
                message: row.get(3)?,
                html: row.get(4)?,
                css: row.get(5)?,
                js: row.get(6)?,
                content: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_version(
        &self,
        project_id: &str,
        version_number: i64,
    ) -> Result<Option<CanvasVersion>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, version_number, message, html, css, js, content, created_at
             FROM canvas_versions WHERE project_id = ?1 AND version_number = ?2",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![project_id, version_number], |row| {
            Ok(CanvasVersion {
                id: row.get(0)?,
                project_id: row.get(1)?,
                version_number: row.get(2)?,
                message: row.get(3)?,
                html: row.get(4)?,
                css: row.get(5)?,
                js: row.get(6)?,
                content: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Cleanup old versions, keeping the latest `keep` versions per project.
    pub fn cleanup_old_versions(&self, project_id: &str, keep: i64) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CanvasDB lock poisoned: {e}"))?;
        let deleted = conn.execute(
            "DELETE FROM canvas_versions WHERE project_id = ?1 AND version_number NOT IN (
                SELECT version_number FROM canvas_versions WHERE project_id = ?1
                ORDER BY version_number DESC LIMIT ?2
            )",
            rusqlite::params![project_id, keep],
        )?;
        Ok(deleted as u64)
    }
}
