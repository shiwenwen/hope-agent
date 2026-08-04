//! Design chat thread rows（表 `design_chat_threads` 在 sessions.db，
//! `session/db.rs` 建表——类型与查询随表住 kernel；ha-design 特征侧经
//! 原路径再导出并以类型化方法访问，**不再暴露原始连接**（crate 数据边界：
//! 核心表 schema 不做跨 crate 隐式 API）。
//!
//! 这不是安全边界——与 knowledge threads 相同，只作会话容器的范围划分。

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::{sanitize_fts_query, SessionDB};

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

impl SessionDB {
    /// Record a `kind='design'` session as a chat thread anchored to a project.
    /// Idempotent on `session_id` (re-recording keeps the first row).
    pub fn create_design_thread(&self, session_id: &str, project_id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn_internal(|conn| {
            conn.execute(
                "INSERT INTO design_chat_threads (session_id, project_id, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id) DO NOTHING",
                params![session_id, project_id, now],
            )?;
            Ok(())
        })
    }

    /// The design project a chat-thread session is anchored to, if any.
    pub fn design_thread_project(&self, session_id: &str) -> Result<Option<String>> {
        self.with_conn_internal(|conn| {
            let pid: Option<String> = conn
                .query_row(
                    "SELECT project_id FROM design_chat_threads WHERE session_id = ?1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(pid)
        })
    }

    /// Most-recently-active chat thread session for a project.
    pub fn latest_design_thread(&self, project_id: &str) -> Result<Option<String>> {
        self.with_conn_internal(|conn| {
            let sid: Option<String> = conn
                .query_row(
                    "SELECT t.session_id
                     FROM design_chat_threads t
                     JOIN sessions s ON s.id = t.session_id
                     WHERE t.project_id = ?1 AND s.archived_at IS NULL
                     ORDER BY s.updated_at DESC
                     LIMIT 1",
                    params![project_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(sid)
        })
    }

    /// A page of chat threads in a project, newest-active first. `query`
    /// (non-empty) restricts to threads whose messages match an FTS search.
    pub fn list_design_threads(
        &self,
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

        let limit = limit.unwrap_or(50).clamp(1, 200);
        let offset = offset.unwrap_or(0).max(0);
        let sanitized = query.and_then(|q| {
            let s = sanitize_fts_query(q);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        });

        self.with_conn_internal(|conn| {
            let out = if let Some(q) = sanitized {
                let sql = format!(
                    "SELECT {SELECT}
                     FROM design_chat_threads t
                     JOIN sessions s ON s.id = t.session_id
                     WHERE t.project_id = ?1
                       AND s.archived_at IS NULL
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
                     WHERE t.project_id = ?1 AND s.archived_at IS NULL
                     ORDER BY s.updated_at DESC
                     LIMIT ?2 OFFSET ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![project_id, limit, offset], map_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            Ok(out)
        })
    }

    /// Session ids of every design chat thread bound to `project_id`.
    pub fn design_thread_session_ids(&self, project_id: &str) -> Result<Vec<String>> {
        self.with_conn_internal(|conn| {
            let mut stmt =
                conn.prepare("SELECT session_id FROM design_chat_threads WHERE project_id = ?1")?;
            let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }
}
