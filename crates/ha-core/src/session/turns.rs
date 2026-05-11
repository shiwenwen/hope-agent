use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::SessionDB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnStatus {
    Running,
    Cancelling,
    Completed,
    Interrupted,
    Failed,
}

impl ChatTurnStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "cancelling" => Some(Self::Cancelling),
            "completed" => Some(Self::Completed),
            "interrupted" => Some(Self::Interrupted),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Interrupted | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnInterruptReason {
    UserStop,
    Shutdown,
    CrashRecovery,
    ToolCancel,
    RuntimeCancel,
    Unknown,
}

impl ChatTurnInterruptReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserStop => "user_stop",
            Self::Shutdown => "shutdown",
            Self::CrashRecovery => "crash_recovery",
            Self::ToolCancel => "tool_cancel",
            Self::RuntimeCancel => "runtime_cancel",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "user_stop" => Some(Self::UserStop),
            "shutdown" => Some(Self::Shutdown),
            "crash_recovery" => Some(Self::CrashRecovery),
            "tool_cancel" => Some(Self::ToolCancel),
            "runtime_cancel" => Some(Self::RuntimeCancel),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    pub id: String,
    pub session_id: String,
    pub source: String,
    pub status: ChatTurnStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_reason: Option<ChatTurnInterruptReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub updated_at: String,
}

impl SessionDB {
    pub(crate) fn ensure_chat_turns_table(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chat_turns (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                status TEXT NOT NULL,
                interrupt_reason TEXT,
                stream_id TEXT,
                user_message_id INTEGER,
                assistant_message_id INTEGER,
                error TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chat_turns_session_started
                ON chat_turns(session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_session_status
                ON chat_turns(session_id, status);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_stream_id
                ON chat_turns(stream_id);",
        )?;
        Ok(())
    }

    pub fn create_chat_turn(
        &self,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        user_message_id: Option<i64>,
    ) -> Result<ChatTurn> {
        let id = uuid::Uuid::new_v4().to_string();
        self.create_chat_turn_with_id(&id, session_id, source, stream_id, user_message_id)
    }

    pub fn create_chat_turn_with_id(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        user_message_id: Option<i64>,
    ) -> Result<ChatTurn> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chat_turns (
                id, session_id, source, status, interrupt_reason, stream_id,
                user_message_id, assistant_message_id, error, started_at, ended_at, updated_at
             ) VALUES (?1, ?2, ?3, 'running', NULL, ?4, ?5, NULL, NULL, ?6, NULL, ?6)",
            params![id, session_id, source, stream_id, user_message_id, now],
        )?;
        Ok(ChatTurn {
            id: id.to_string(),
            session_id: session_id.to_string(),
            source: source.to_string(),
            status: ChatTurnStatus::Running,
            interrupt_reason: None,
            stream_id: stream_id.map(ToOwned::to_owned),
            user_message_id,
            assistant_message_id: None,
            error: None,
            started_at: now.clone(),
            ended_at: None,
            updated_at: now,
        })
    }

    pub fn get_chat_turn(&self, turn_id: &str) -> Result<Option<ChatTurn>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at
             FROM chat_turns WHERE id = ?1",
            params![turn_id],
            Self::row_to_chat_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_latest_chat_turn(&self, session_id: &str) -> Result<Option<ChatTurn>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at
             FROM chat_turns
             WHERE session_id = ?1
             ORDER BY started_at DESC
             LIMIT 1",
            params![session_id],
            Self::row_to_chat_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn mark_chat_turn_cancelling(
        &self,
        turn_id: &str,
        reason: ChatTurnInterruptReason,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let n = conn.execute(
            "UPDATE chat_turns
             SET status = 'cancelling', interrupt_reason = ?1, updated_at = ?2
             WHERE id = ?3 AND status IN ('running', 'cancelling')",
            params![reason.as_str(), now, turn_id],
        )?;
        Ok(n > 0)
    }

    pub fn finish_chat_turn_once(
        &self,
        turn_id: &str,
        status: ChatTurnStatus,
        interrupt_reason: Option<ChatTurnInterruptReason>,
        error: Option<&str>,
        assistant_message_id: Option<i64>,
    ) -> Result<bool> {
        if !status.is_terminal() {
            return Err(anyhow::anyhow!(
                "finish_chat_turn_once requires terminal status, got {}",
                status.as_str()
            ));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let n = conn.execute(
            "UPDATE chat_turns
             SET status = ?1,
                 interrupt_reason = COALESCE(interrupt_reason, ?2),
                 error = ?3,
                 assistant_message_id = COALESCE(?4, assistant_message_id),
                 ended_at = COALESCE(ended_at, ?5),
                 updated_at = ?5
             WHERE id = ?6 AND status NOT IN ('completed', 'interrupted', 'failed')",
            params![
                status.as_str(),
                interrupt_reason.map(|r| r.as_str()),
                error,
                assistant_message_id,
                now,
                turn_id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn finish_chat_turn_after_execution(
        &self,
        turn_id: &str,
        cancel_requested: bool,
        error: Option<&str>,
        assistant_message_id: Option<i64>,
    ) -> Result<Option<ChatTurn>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let current = conn
            .query_row(
                "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                        user_message_id, assistant_message_id, error, started_at, ended_at, updated_at
                 FROM chat_turns WHERE id = ?1",
                params![turn_id],
                Self::row_to_chat_turn,
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(None);
        };
        if current.status.is_terminal() {
            return Ok(Some(current));
        }

        let interrupted = cancel_requested || current.status == ChatTurnStatus::Cancelling;
        let final_status = if interrupted {
            ChatTurnStatus::Interrupted
        } else if error.is_some() {
            ChatTurnStatus::Failed
        } else {
            ChatTurnStatus::Completed
        };
        let final_reason = interrupted
            .then_some(
                current
                    .interrupt_reason
                    .unwrap_or(ChatTurnInterruptReason::RuntimeCancel),
            );
        let final_error = (final_status == ChatTurnStatus::Failed)
            .then_some(error)
            .flatten();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE chat_turns
             SET status = ?1,
                 interrupt_reason = ?2,
                 error = ?3,
                 assistant_message_id = COALESCE(?4, assistant_message_id),
                 ended_at = COALESCE(ended_at, ?5),
                 updated_at = ?5
             WHERE id = ?6 AND status NOT IN ('completed', 'interrupted', 'failed')",
            params![
                final_status.as_str(),
                final_reason.map(|r| r.as_str()),
                final_error,
                assistant_message_id,
                now,
                turn_id,
            ],
        )?;

        conn.query_row(
            "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at
             FROM chat_turns WHERE id = ?1",
            params![turn_id],
            Self::row_to_chat_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn update_chat_turn_stream_id(&self, turn_id: &str, stream_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let n = conn.execute(
            "UPDATE chat_turns SET stream_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![stream_id, now, turn_id],
        )?;
        Ok(n > 0)
    }

    pub fn recover_stale_chat_turns(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let n = conn.execute(
            "UPDATE chat_turns
             SET status = 'interrupted',
                 interrupt_reason = 'crash_recovery',
                 ended_at = COALESCE(ended_at, ?1),
                 updated_at = ?1
             WHERE status IN ('running', 'cancelling')",
            params![now],
        )?;
        Ok(n)
    }

    fn row_to_chat_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatTurn> {
        let status_str: String = row.get(3)?;
        let reason_str: Option<String> = row.get(4)?;
        Ok(ChatTurn {
            id: row.get(0)?,
            session_id: row.get(1)?,
            source: row.get(2)?,
            status: ChatTurnStatus::from_str(&status_str).unwrap_or(ChatTurnStatus::Failed),
            interrupt_reason: reason_str
                .as_deref()
                .and_then(ChatTurnInterruptReason::from_str),
            stream_id: row.get(5)?,
            user_message_id: row.get(6)?,
            assistant_message_id: row.get(7)?,
            error: row.get(8)?,
            started_at: row.get(9)?,
            ended_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> SessionDB {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        // Leak tempdir for test lifetime so SQLite can keep the file open.
        std::mem::forget(dir);
        SessionDB::open(&path).unwrap()
    }

    #[test]
    fn terminal_status_is_written_once() {
        let db = temp_db();
        let session = db.create_session_with_project("ha-main", None, None).unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-1"), Some(1))
            .unwrap();

        assert!(db
            .finish_chat_turn_once(
                &turn.id,
                ChatTurnStatus::Interrupted,
                Some(ChatTurnInterruptReason::UserStop),
                None,
                None,
            )
            .unwrap());
        assert!(!db
            .finish_chat_turn_once(
                &turn.id,
                ChatTurnStatus::Completed,
                None,
                Some("late success"),
                None,
            )
            .unwrap());

        let persisted = db.get_chat_turn(&turn.id).unwrap().unwrap();
        assert_eq!(persisted.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::UserStop)
        );
        assert!(persisted.error.is_none());
    }

    #[test]
    fn recover_stale_running_turns_marks_interrupted() {
        let db = temp_db();
        let session = db.create_session_with_project("ha-main", None, None).unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-1"), None)
            .unwrap();

        assert_eq!(db.recover_stale_chat_turns().unwrap(), 1);
        let persisted = db.get_chat_turn(&turn.id).unwrap().unwrap();
        assert_eq!(persisted.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::CrashRecovery)
        );
        assert!(persisted.ended_at.is_some());
    }

    #[test]
    fn execution_success_after_cancelling_finishes_interrupted() {
        let db = temp_db();
        let session = db.create_session_with_project("ha-main", None, None).unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-1"), None)
            .unwrap();
        db.mark_chat_turn_cancelling(&turn.id, ChatTurnInterruptReason::UserStop)
            .unwrap();

        let persisted = db
            .finish_chat_turn_after_execution(&turn.id, false, None, Some(42))
            .unwrap()
            .unwrap();
        assert_eq!(persisted.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::UserStop)
        );
        assert_eq!(persisted.assistant_message_id, Some(42));
        assert!(persisted.error.is_none());
    }

    #[test]
    fn execution_failure_after_cancel_request_finishes_interrupted_without_error() {
        let db = temp_db();
        let session = db.create_session_with_project("ha-main", None, None).unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-1"), None)
            .unwrap();

        let persisted = db
            .finish_chat_turn_after_execution(&turn.id, true, Some("late provider error"), None)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::RuntimeCancel)
        );
        assert!(persisted.error.is_none());
    }
}
