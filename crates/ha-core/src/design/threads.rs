//! Design-space per-project chat threads.
//!
//! A "thread" is a `kind='design'` session bound to the design project it
//! iterates on. Mirrors the knowledge-space chat threads (`knowledge/registry.rs`):
//! the anchor rows live in **sessions.db** (`design_chat_threads`, created in
//! `session/db.rs`) so the history picker can JOIN `sessions` / `messages`; the
//! `project_id` is a plain column because the design project row lives in the
//! separate `design.db` (no cross-db FK). Threads are hidden from the main
//! sidebar / `/sessions` / global FTS via `SessionKind::Design`.
//!
//! This is NOT a security boundary — like knowledge threads it only scopes the
//! conversation container.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// A design-space chat thread — one row per `kind='design'` session, joined with
/// session metadata for the history picker (title / recency / size).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignChatThread {
    pub session_id: String,
    pub project_id: String,
    /// Agent baked into this thread's session — restored when the history picker
    /// switches to it so follow-ups run with the thread's own agent + model.
    pub agent_id: String,
    /// Session title (LLM- or user-set), `None` until named.
    pub title: Option<String>,
    /// Thread creation time (epoch ms).
    pub created_at: i64,
    /// Session `updated_at` (rfc3339) — recency sort key for the picker.
    pub updated_at: String,
    /// Count of persisted messages (user + assistant + tool rows).
    pub message_count: i64,
    /// Last user/assistant message preview for the picker (trimmed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_snippet: Option<String>,
}

fn session_db() -> Result<&'static std::sync::Arc<crate::session::db::SessionDB>> {
    crate::globals::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))
}

/// Record a `kind='design'` session as a chat thread anchored to a project.
/// Idempotent on `session_id` (re-recording keeps the first row).
pub fn create_thread(session_id: &str, project_id: &str) -> Result<()> {
    let db = session_db()?;
    let conn = db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO design_chat_threads (session_id, project_id, created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO NOTHING",
        params![session_id, project_id, now],
    )?;
    Ok(())
}

/// The design project a chat-thread session is anchored to, if any. Used by the
/// `design` tool to resolve which project a `kind='design'` chat turn edits.
pub fn project_for_session(session_id: &str) -> Result<Option<String>> {
    let db = session_db()?;
    let conn = db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let pid: Option<String> = conn
        .query_row(
            "SELECT project_id FROM design_chat_threads WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(pid)
}

/// Most-recently-active chat thread session for a project (default-load target).
/// `None` when the project has no prior thread.
pub fn latest_thread_for_project(project_id: &str) -> Result<Option<String>> {
    let db = session_db()?;
    let conn = db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let sid: Option<String> = conn
        .query_row(
            "SELECT t.session_id
             FROM design_chat_threads t
             JOIN sessions s ON s.id = t.session_id
             WHERE t.project_id = ?1
             ORDER BY s.updated_at DESC
             LIMIT 1",
            params![project_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(sid)
}

/// A page of chat threads in a project, newest-active first, joined with session
/// metadata for the history picker. `query` (when non-empty) restricts to
/// threads whose messages match an FTS search. `limit` (default 50, clamped to
/// 1..=200) + `offset` paginate.
pub fn list_threads(
    project_id: &str,
    query: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DesignChatThread>> {
    fn map_row(r: &rusqlite::Row) -> rusqlite::Result<DesignChatThread> {
        Ok(DesignChatThread {
            session_id: r.get(0)?,
            project_id: r.get(1)?,
            created_at: r.get(2)?,
            title: r.get(3)?,
            updated_at: r.get(4)?,
            agent_id: r.get(5)?,
            message_count: r.get(6)?,
            last_snippet: r.get::<_, Option<String>>(7)?.map(|s| {
                let trimmed = s.trim();
                crate::truncate_utf8(trimmed, 160).to_string()
            }),
        })
    }
    const SELECT: &str = "t.session_id, t.project_id, t.created_at,
                s.title, s.updated_at, s.agent_id,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = t.session_id) AS msg_count,
                (SELECT m.content FROM messages m
                   WHERE m.session_id = t.session_id
                     AND m.role IN ('user','assistant') AND length(m.content) > 0
                   ORDER BY m.id DESC LIMIT 1) AS last_snippet";

    let db = session_db()?;
    let conn = db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let limit = limit.unwrap_or(50).clamp(1, 200);
    let offset = offset.unwrap_or(0).max(0);

    let sanitized = query.and_then(|q| {
        let s = crate::session::db::sanitize_fts_query(q);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    });

    let out = if let Some(q) = sanitized {
        let sql = format!(
            "SELECT {SELECT}
             FROM design_chat_threads t
             JOIN sessions s ON s.id = t.session_id
             WHERE t.project_id = ?1
               AND t.session_id IN (
                   SELECT DISTINCT m.session_id FROM messages_fts fts
                   JOIN messages m ON m.id = fts.rowid
                   JOIN design_chat_threads dt ON dt.session_id = m.session_id
                   WHERE dt.project_id = ?1 AND messages_fts MATCH ?2)
             ORDER BY s.updated_at DESC
             LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project_id, q, limit, offset], map_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let sql = format!(
            "SELECT {SELECT}
             FROM design_chat_threads t
             JOIN sessions s ON s.id = t.session_id
             WHERE t.project_id = ?1
             ORDER BY s.updated_at DESC
             LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project_id, limit, offset], map_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(out)
}

/// Session ids of every design chat thread bound to `project_id`. Used by the
/// design-project delete cascade to tear down the (otherwise hidden)
/// `kind=design` sessions before the project row is removed.
pub fn thread_session_ids(project_id: &str) -> Result<Vec<String>> {
    let db = session_db()?;
    let conn = db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let mut stmt =
        conn.prepare("SELECT session_id FROM design_chat_threads WHERE project_id = ?1")?;
    let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
