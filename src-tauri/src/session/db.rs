use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::Mutex;

use super::types::{SessionMeta, SessionMessage, MessageRole, NewMessage};

// ── Database Manager ─────────────────────────────────────────────

pub struct SessionDB {
    pub(crate) conn: Mutex<Connection>,
}

impl SessionDB {
    /// Open (or create) the database at the given path, enable WAL mode,
    /// and ensure tables exist.
    pub fn open(db_path: &PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for crash safety and better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        // Create tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                agent_id TEXT NOT NULL DEFAULT 'default',
                provider_id TEXT,
                provider_name TEXT,
                model_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                context_json TEXT,
                last_read_message_id INTEGER DEFAULT 0,
                is_cron INTEGER NOT NULL DEFAULT 0,
                parent_session_id TEXT
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                timestamp TEXT NOT NULL,
                attachments_meta TEXT,
                model TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                reasoning_effort TEXT,
                tool_call_id TEXT,
                tool_name TEXT,
                tool_arguments TEXT,
                tool_result TEXT,
                tool_duration_ms INTEGER,
                is_error INTEGER DEFAULT 0,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_agent_id ON sessions(agent_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated_at ON sessions(updated_at DESC);

            -- Sub-agent runs
            CREATE TABLE IF NOT EXISTS subagent_runs (
                run_id TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL,
                parent_agent_id TEXT NOT NULL,
                child_agent_id TEXT NOT NULL,
                child_session_id TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'spawning',
                result TEXT,
                error TEXT,
                depth INTEGER NOT NULL DEFAULT 1,
                model_used TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                duration_ms INTEGER,
                label TEXT,
                attachment_count INTEGER DEFAULT 0,
                input_tokens INTEGER,
                output_tokens INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent_runs(parent_session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_subagent_status ON subagent_runs(status);
            CREATE INDEX IF NOT EXISTS idx_subagent_label ON subagent_runs(label);"
        )?;

        // Migration: add is_cron column if missing
        let has_is_cron = conn
            .prepare("SELECT is_cron FROM sessions LIMIT 1")
            .is_ok();
        if !has_is_cron {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN is_cron INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: add thinking column to messages if missing
        let has_thinking = conn
            .prepare("SELECT thinking FROM messages LIMIT 1")
            .is_ok();
        if !has_thinking {
            conn.execute_batch(
                "ALTER TABLE messages ADD COLUMN thinking TEXT;",
            )?;
        }

        Ok(Self { conn: Mutex::new(conn) })
    }

    // ── Session CRUD ─────────────────────────────────────────────

    /// Create a new session, return its metadata.
    pub fn create_session(&self, agent_id: &str) -> Result<SessionMeta> {
        self.create_session_with_parent(agent_id, None)
    }

    /// Create a new session with an optional parent session ID (for sub-agent sessions).
    pub fn create_session_with_parent(&self, agent_id: &str, parent_session_id: Option<&str>) -> Result<SessionMeta> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO sessions (id, agent_id, created_at, updated_at, parent_session_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, agent_id, now, now, parent_session_id],
        )?;

        Ok(SessionMeta {
            id,
            title: None,
            agent_id: agent_id.to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            unread_count: 0,
            is_cron: false,
            parent_session_id: parent_session_id.map(|s| s.to_string()),
        })
    }

    /// List all sessions, ordered by most recently updated.
    /// Optionally filter by agent_id.
    pub fn list_sessions(&self, agent_id: Option<&str>) -> Result<Vec<SessionMeta>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut sessions = Vec::new();

        if let Some(agent_id) = agent_id {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
                        s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count,
                        s.is_cron,
                        s.parent_session_id
                 FROM sessions s
                 WHERE s.agent_id = ?1
                 ORDER BY s.updated_at DESC"
            )?;
            let rows = stmt.query_map(params![agent_id], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    agent_id: row.get(2)?,
                    provider_id: row.get(3)?,
                    provider_name: row.get(4)?,
                    model_id: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    message_count: row.get(8)?,
                    unread_count: row.get(9)?,
                    is_cron: row.get::<_, i64>(10).unwrap_or(0) != 0,
                    parent_session_id: row.get(11)?,
                })
            })?;
            for row in rows {
                sessions.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
                        s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count,
                        s.is_cron,
                        s.parent_session_id
                 FROM sessions s
                 ORDER BY s.updated_at DESC"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    agent_id: row.get(2)?,
                    provider_id: row.get(3)?,
                    provider_name: row.get(4)?,
                    model_id: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    message_count: row.get(8)?,
                    unread_count: row.get(9)?,
                    is_cron: row.get::<_, i64>(10).unwrap_or(0) != 0,
                    parent_session_id: row.get(11)?,
                })
            })?;
            for row in rows {
                sessions.push(row?);
            }
        }

        Ok(sessions)
    }

    /// Load all messages for a session.
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking
             FROM messages
             WHERE session_id = ?1
             ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    /// Load the latest N messages for a session (for initial page load).
    /// Returns (messages_in_asc_order, total_count).
    pub fn load_session_messages_latest(&self, session_id: &str, limit: u32) -> Result<(Vec<SessionMessage>, u32)> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u32 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking
             FROM messages
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2"
        )?;

        let rows = stmt.query_map(params![session_id, limit], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        // Reverse to get ASC order
        messages.reverse();
        Ok((messages, total))
    }

    /// Load messages before a given message id (for "load more" / scroll up).
    /// Returns messages in ASC order.
    pub fn load_session_messages_before(&self, session_id: &str, before_id: i64, limit: u32) -> Result<Vec<SessionMessage>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking
             FROM messages
             WHERE session_id = ?1 AND id < ?2
             ORDER BY id DESC
             LIMIT ?3"
        )?;

        let rows = stmt.query_map(params![session_id, before_id, limit], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse();
        Ok(messages)
    }

    pub(crate) fn row_to_session_message(row: &rusqlite::Row) -> rusqlite::Result<SessionMessage> {
        let is_error_val: Option<i64> = row.get(15)?;
        Ok(SessionMessage {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role: MessageRole::from_str(&row.get::<_, String>(2)?),
            content: row.get(3)?,
            timestamp: row.get(4)?,
            attachments_meta: row.get(5)?,
            model: row.get(6)?,
            tokens_in: row.get(7)?,
            tokens_out: row.get(8)?,
            reasoning_effort: row.get(9)?,
            tool_call_id: row.get(10)?,
            tool_name: row.get(11)?,
            tool_arguments: row.get(12)?,
            tool_result: row.get(13)?,
            tool_duration_ms: row.get(14)?,
            is_error: is_error_val.map(|v| v != 0),
            thinking: row.get(16)?,
        })
    }

    /// Append a message to a session and update the session's updated_at.
    pub fn append_message(&self, session_id: &str, msg: &NewMessage) -> Result<i64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let timestamp = if msg.timestamp.is_empty() { &now } else { &msg.timestamp };

        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp,
                attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                tool_call_id, tool_name, tool_arguments, tool_result,
                tool_duration_ms, is_error, thinking)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                session_id,
                msg.role.as_str(),
                msg.content,
                timestamp,
                msg.attachments_meta,
                msg.model,
                msg.tokens_in,
                msg.tokens_out,
                msg.reasoning_effort,
                msg.tool_call_id,
                msg.tool_name,
                msg.tool_arguments,
                msg.tool_result,
                msg.tool_duration_ms,
                msg.is_error.map(|b| if b { 1i64 } else { 0i64 }),
                msg.thinking,
            ],
        )?;

        let msg_id = conn.last_insert_rowid();

        // Update session's updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(msg_id)
    }

    /// Update an existing tool_call message with result, duration, and is_error.
    /// Matches by session_id + tool_call_id to find the original tool_call record.
    pub fn update_tool_result(&self, session_id: &str, call_id: &str, result: &str, duration_ms: Option<i64>, is_error: bool) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3
             WHERE session_id = ?4 AND tool_call_id = ?5",
            params![result, duration_ms, if is_error { 1i64 } else { 0i64 }, session_id, call_id],
        )?;
        Ok(())
    }

    /// Update session title.
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, session_id],
        )?;
        Ok(())
    }

    /// Mark a session as a cron-triggered session.
    pub fn mark_session_cron(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET is_cron = 1 WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Update session's provider/model info.
    pub fn update_session_model(&self, session_id: &str, provider_id: Option<&str>, provider_name: Option<&str>, model_id: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET provider_id = ?1, provider_name = ?2, model_id = ?3 WHERE id = ?4",
            params![provider_id, provider_name, model_id, session_id],
        )?;
        Ok(())
    }

    /// Delete a session and all its messages (CASCADE) and attachments.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;

        // Clean up attachments directory
        if let Ok(att_dir) = crate::paths::attachments_dir(session_id) {
            let _ = std::fs::remove_dir_all(att_dir);
        }

        Ok(())
    }

    /// Save the agent's conversation_history JSON for a session.
    pub fn save_context(&self, session_id: &str, context_json: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET context_json = ?1 WHERE id = ?2",
            params![context_json, session_id],
        )?;
        Ok(())
    }

    /// Load the agent's conversation_history JSON for a session.
    /// Returns None if the session has no saved context.
    pub fn load_context(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT context_json FROM sessions WHERE id = ?1"
        )?;
        let result = stmt.query_row(params![session_id], |row| {
            row.get::<_, Option<String>>(0)
        }).ok().flatten();
        Ok(result)
    }

    /// Get a single session's metadata.
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
                    s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count,
                    s.is_cron,
                    s.parent_session_id
             FROM sessions s WHERE s.id = ?1"
        )?;

        let mut rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                agent_id: row.get(2)?,
                provider_id: row.get(3)?,
                provider_name: row.get(4)?,
                model_id: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                message_count: row.get(8)?,
                unread_count: row.get(9)?,
                is_cron: row.get::<_, i64>(10).unwrap_or(0) != 0,
                parent_session_id: row.get(11)?,
            })
        })?;

        match rows.next() {
            Some(Ok(meta)) => Ok(Some(meta)),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Ok(None),
        }
    }

    /// Mark all messages in a session as read by updating last_read_message_id
    /// to the current maximum message id.
    pub fn mark_session_read(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?1) WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Mark all messages in multiple sessions as read.
    pub fn mark_session_read_batch(&self, session_ids: &[String]) -> Result<()> {
        if session_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?1) WHERE id = ?1"
        )?;
        for id in session_ids {
            stmt.execute(params![id])?;
        }
        Ok(())
    }

    /// Mark all sessions as read.
    pub fn mark_all_sessions_read(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute_batch(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE messages.session_id = sessions.id)"
        )?;
        Ok(())
    }
}
