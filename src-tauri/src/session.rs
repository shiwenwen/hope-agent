use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

// ── Data Structures ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub title: Option<String>,
    pub agent_id: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    pub unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Event,
    Tool,
    /// Intermediate text block emitted before tool calls to preserve ordering.
    TextBlock,
}

impl MessageRole {
    pub fn as_str(&self) -> &str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Event => "event",
            MessageRole::Tool => "tool",
            MessageRole::TextBlock => "text_block",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "event" => MessageRole::Event,
            "tool" => MessageRole::Tool,
            "text_block" => MessageRole::TextBlock,
            _ => MessageRole::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    // User message fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments_meta: Option<String>, // JSON array of {name, mime_type, size}
    // Assistant message fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    // Tool call fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_arguments: Option<String>, // JSON string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

// ── Database Manager ─────────────────────────────────────────────

pub struct SessionDB {
    conn: Mutex<Connection>,
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
                last_read_message_id INTEGER DEFAULT 0
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
            CREATE INDEX IF NOT EXISTS idx_sessions_updated_at ON sessions(updated_at DESC);"
        )?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    // ── Session CRUD ─────────────────────────────────────────────

    /// Create a new session, return its metadata.
    pub fn create_session(&self, agent_id: &str) -> Result<SessionMeta> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO sessions (id, agent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, agent_id, now, now],
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
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count
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
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count
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
                    tool_duration_ms, is_error
             FROM messages
             WHERE session_id = ?1
             ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
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
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
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
                tool_duration_ms, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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

    /// Update session title.
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, session_id],
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
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0)) as unread_count
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
}

// ── NewMessage (for inserting) ───────────────────────────────────

/// A new message to be inserted (without auto-generated id).
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    pub attachments_meta: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub reasoning_effort: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub tool_result: Option<String>,
    pub tool_duration_ms: Option<i64>,
    pub is_error: Option<bool>,
}

impl NewMessage {
    /// Create a simple user message.
    pub fn user(content: &str) -> Self {
        Self {
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
        }
    }

    /// Create a simple assistant message.
    pub fn assistant(content: &str) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
        }
    }

    /// Create a tool call/result message.
    pub fn tool(call_id: &str, name: &str, arguments: &str, result: &str, duration_ms: Option<i64>, is_error: bool) -> Self {
        Self {
            role: MessageRole::Tool,
            content: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: Some(call_id.to_string()),
            tool_name: Some(name.to_string()),
            tool_arguments: Some(arguments.to_string()),
            tool_result: Some(result.to_string()),
            tool_duration_ms: duration_ms,
            is_error: Some(is_error),
        }
    }

    /// Create a text_block message (intermediate text before tool calls).
    pub fn text_block(content: &str) -> Self {
        Self {
            role: MessageRole::TextBlock,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
        }
    }

    /// Create an event message (e.g. errors, model fallback notifications).
    pub fn event(content: &str) -> Self {
        Self {
            role: MessageRole::Event,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
        }
    }
}

// ── Auto-title helper ────────────────────────────────────────────

/// Generate a short title from the first user message (truncated to 50 chars).
pub fn auto_title(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return "New Chat".to_string();
    }
    // Take first line only
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    // Use char count (not byte length) to handle CJK/emoji correctly
    if first_line.chars().count() <= 50 {
        first_line.to_string()
    } else {
        // Find the byte offset of the 47th character boundary
        let cut = first_line.char_indices().nth(47).map(|(i, _)| i).unwrap_or(first_line.len());
        format!("{}...", &first_line[..cut])
    }
}

// ── Database path helper ─────────────────────────────────────────

/// Get the database file path: ~/.opencomputer/sessions.db
pub fn db_path() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("sessions.db"))
}
