//! Durable user-message queue for busy chat sessions.
//!
//! SQLite is the single source of truth for both "send after reply" and
//! "insert at the next tool boundary". Frontends keep projections only.

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::agent::Attachment;

use super::SessionDB;

pub const EVENT_TURN_QUEUE_CHANGED: &str = "chat:turn_queue_changed";
pub const MAX_QUEUED_TURN_MESSAGES_PER_SESSION: i64 = 100;
const MAX_QUEUED_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_QUEUED_ATTACHMENTS: usize = 64;
const MAX_QUEUED_ATTACHMENTS_JSON_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedTurnMessageMode {
    Queue,
    ForceInsert,
}

impl QueuedTurnMessageMode {
    fn parse(value: &str) -> Self {
        match value {
            "force_insert" => Self::ForceInsert,
            _ => Self::Queue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedTurnMessageStatus {
    Queued,
    WaitingToolBoundary,
    Inserting,
    Dispatching,
    FallbackAfterReply,
}

impl QueuedTurnMessageStatus {
    fn parse(value: &str) -> Self {
        match value {
            "waiting_tool_boundary" => Self::WaitingToolBoundary,
            "inserting" => Self::Inserting,
            "dispatching" => Self::Dispatching,
            "fallback_after_reply" => Self::FallbackAfterReply,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueuedTurnMessageRecord {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub message: String,
    pub display_text: Option<String>,
    pub attachments: Vec<Attachment>,
    pub is_plan_trigger: bool,
    pub goal_trigger: bool,
    pub plan_comment: Option<serde_json::Value>,
    pub plan_mode: Option<String>,
    pub workflow_mode: Option<String>,
    pub mode: QueuedTurnMessageMode,
    pub status: QueuedTurnMessageStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTurnMessageView {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub message: String,
    pub display_text: Option<String>,
    pub attachment_count: usize,
    pub quote_count: usize,
    pub is_plan_trigger: bool,
    pub goal_trigger: bool,
    pub plan_comment: Option<serde_json::Value>,
    pub plan_mode: Option<String>,
    pub workflow_mode: Option<String>,
    pub mode: QueuedTurnMessageMode,
    pub status: QueuedTurnMessageStatus,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&QueuedTurnMessageRecord> for QueuedTurnMessageView {
    fn from(value: &QueuedTurnMessageRecord) -> Self {
        Self {
            request_id: value.request_id.clone(),
            session_id: value.session_id.clone(),
            turn_id: value.turn_id.clone(),
            message: value.message.clone(),
            display_text: value.display_text.clone(),
            attachment_count: value
                .attachments
                .iter()
                .filter(|attachment| attachment.source.as_deref() != Some("quote"))
                .count(),
            quote_count: value
                .attachments
                .iter()
                .filter(|attachment| attachment.source.as_deref() == Some("quote"))
                .count(),
            is_plan_trigger: value.is_plan_trigger,
            goal_trigger: value.goal_trigger,
            plan_comment: value.plan_comment.clone(),
            plan_mode: value.plan_mode.clone(),
            workflow_mode: value.workflow_mode.clone(),
            mode: value.mode,
            status: value.status,
            created_at: value.created_at.clone(),
            updated_at: value.updated_at.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewQueuedTurnMessage {
    pub request_id: String,
    pub session_id: String,
    pub message: String,
    pub display_text: Option<String>,
    pub attachments: Vec<Attachment>,
    pub is_plan_trigger: bool,
    pub goal_trigger: bool,
    pub plan_comment: Option<serde_json::Value>,
    pub plan_mode: Option<String>,
    pub workflow_mode: Option<String>,
}

#[derive(Debug)]
pub struct EnqueueQueuedTurnMessageOutcome {
    pub item: QueuedTurnMessageView,
    pub inserted: bool,
}

fn emit_changed(session_id: &str, request_id: Option<&str>, operation: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EVENT_TURN_QUEUE_CHANGED,
            serde_json::json!({
                "sessionId": session_id,
                "requestId": request_id,
                "operation": operation,
            }),
        );
    }
}

fn parse_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedTurnMessageRecord> {
    let attachments_json: String = row.get(5)?;
    let plan_comment_json: Option<String> = row.get(8)?;
    let options_json: Option<String> = row.get(9)?;
    let options = options_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .unwrap_or_default();
    let mode: String = row.get(10)?;
    let status: String = row.get(11)?;
    Ok(QueuedTurnMessageRecord {
        request_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        message: row.get(3)?,
        display_text: row.get(4)?,
        attachments: serde_json::from_str(&attachments_json).unwrap_or_default(),
        is_plan_trigger: row.get::<_, i64>(6)? != 0,
        goal_trigger: row.get::<_, i64>(7)? != 0,
        plan_comment: plan_comment_json.and_then(|raw| serde_json::from_str(&raw).ok()),
        plan_mode: options
            .get("planMode")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        workflow_mode: options
            .get("workflowMode")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        mode: QueuedTurnMessageMode::parse(&mode),
        status: QueuedTurnMessageStatus::parse(&status),
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

const RECORD_SELECT: &str = "SELECT request_id, session_id, turn_id, message, display_text,
    attachments_json, is_plan_trigger, goal_trigger, plan_comment_json, options_json, mode, status,
    created_at, updated_at FROM queued_turn_user_messages";

impl SessionDB {
    pub(crate) fn ensure_turn_message_queue_table(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS queued_turn_user_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id TEXT NOT NULL UNIQUE,
                session_id TEXT NOT NULL,
                turn_id TEXT,
                message TEXT NOT NULL,
                display_text TEXT,
                attachments_json TEXT NOT NULL DEFAULT '[]',
                is_plan_trigger INTEGER NOT NULL DEFAULT 0,
                goal_trigger INTEGER NOT NULL DEFAULT 0,
                plan_comment_json TEXT,
                options_json TEXT,
                mode TEXT NOT NULL DEFAULT 'queue',
                status TEXT NOT NULL DEFAULT 'queued',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_queued_turn_messages_session_fifo
                ON queued_turn_user_messages(session_id, id);
            CREATE INDEX IF NOT EXISTS idx_queued_turn_messages_turn_status
                ON queued_turn_user_messages(session_id, turn_id, status);",
        )?;
        if conn
            .prepare("SELECT options_json FROM queued_turn_user_messages LIMIT 1")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE queued_turn_user_messages ADD COLUMN options_json TEXT;",
            )?;
        }
        Ok(())
    }

    pub(crate) fn recover_turn_message_queue(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute(
            "DELETE FROM queued_turn_user_messages
             WHERE request_id IN (
                SELECT queue_request_id FROM messages WHERE queue_request_id IS NOT NULL
             )",
            [],
        )?;
        conn.execute(
            "UPDATE queued_turn_user_messages
             SET mode = 'queue', status = CASE
                    WHEN status IN ('waiting_tool_boundary', 'inserting')
                        THEN 'fallback_after_reply'
                    ELSE 'queued'
                 END,
                 turn_id = NULL, updated_at = ?1
             WHERE status IN ('waiting_tool_boundary', 'inserting', 'dispatching')",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn enqueue_turn_user_message(
        &self,
        input: NewQueuedTurnMessage,
    ) -> Result<EnqueueQueuedTurnMessageOutcome> {
        if input.message.len() > MAX_QUEUED_MESSAGE_BYTES
            || input
                .display_text
                .as_ref()
                .is_some_and(|text| text.len() > MAX_QUEUED_MESSAGE_BYTES)
        {
            return Err(anyhow!("queued message is too large"));
        }
        if input.attachments.len() > MAX_QUEUED_ATTACHMENTS {
            return Err(anyhow!(
                "too many queued attachments (maximum {MAX_QUEUED_ATTACHMENTS})"
            ));
        }
        let attachments_json = serde_json::to_string(&input.attachments)?;
        if attachments_json.len() > MAX_QUEUED_ATTACHMENTS_JSON_BYTES {
            return Err(anyhow!("queued attachment metadata is too large"));
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let session_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![input.session_id],
            |row| row.get(0),
        )?;
        if !session_exists {
            return Err(anyhow!("session does not exist"));
        }
        let existing_session: Option<String> = tx
            .query_row(
                "SELECT session_id FROM queued_turn_user_messages WHERE request_id = ?1",
                params![input.request_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_session) = existing_session {
            if existing_session != input.session_id {
                return Err(anyhow!("request id already belongs to another session"));
            }
            tx.commit()?;
            drop(conn);
            let record = self
                .get_queued_turn_user_message(&input.session_id, &input.request_id)?
                .ok_or_else(|| anyhow!("queued message disappeared during idempotent enqueue"))?;
            return Ok(EnqueueQueuedTurnMessageOutcome {
                item: QueuedTurnMessageView::from(&record),
                inserted: false,
            });
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM queued_turn_user_messages WHERE session_id = ?1",
            params![input.session_id],
            |row| row.get(0),
        )?;
        if count >= MAX_QUEUED_TURN_MESSAGES_PER_SESSION {
            return Err(anyhow!(
                "message queue is full (maximum {} items per session)",
                MAX_QUEUED_TURN_MESSAGES_PER_SESSION
            ));
        }
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO queued_turn_user_messages (
                request_id, session_id, turn_id, message, display_text, attachments_json,
                is_plan_trigger, goal_trigger, plan_comment_json, options_json, mode, status,
                created_at, updated_at
             ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queue', 'queued', ?10, ?10)
             ON CONFLICT(request_id) DO NOTHING",
            params![
                input.request_id,
                input.session_id,
                input.message,
                input.display_text,
                attachments_json,
                input.is_plan_trigger as i64,
                input.goal_trigger as i64,
                input
                    .plan_comment
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                serde_json::to_string(&serde_json::json!({
                    "planMode": input.plan_mode,
                    "workflowMode": input.workflow_mode,
                }))?,
                now,
            ],
        )?;
        tx.commit()?;
        drop(conn);
        let record = self
            .get_queued_turn_user_message(&input.session_id, &input.request_id)?
            .ok_or_else(|| anyhow!("failed to read queued message after insert"))?;
        emit_changed(&input.session_id, Some(&input.request_id), "enqueued");
        Ok(EnqueueQueuedTurnMessageOutcome {
            item: QueuedTurnMessageView::from(&record),
            inserted: true,
        })
    }

    pub fn list_queued_turn_user_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<QueuedTurnMessageView>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(&format!(
            "{RECORD_SELECT} WHERE session_id = ?1 ORDER BY id ASC"
        ))?;
        let records = stmt
            .query_map(params![session_id], parse_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records.iter().map(QueuedTurnMessageView::from).collect())
    }

    pub fn get_queued_turn_user_message(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<Option<QueuedTurnMessageRecord>> {
        let conn = self.read_conn()?;
        conn.query_row(
            &format!("{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2"),
            params![session_id, request_id],
            parse_record,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn update_queued_turn_user_message(
        &self,
        session_id: &str,
        request_id: &str,
        message: &str,
        display_text: Option<&str>,
    ) -> Result<bool> {
        if message.trim().is_empty()
            || message.len() > MAX_QUEUED_MESSAGE_BYTES
            || display_text.is_some_and(|text| text.len() > MAX_QUEUED_MESSAGE_BYTES)
        {
            return Err(anyhow!("queued message is empty or too large"));
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages SET message = ?1, display_text = ?2, updated_at = ?3
             WHERE session_id = ?4 AND request_id = ?5
               AND status IN ('queued', 'waiting_tool_boundary', 'fallback_after_reply')",
            params![
                message,
                display_text,
                chrono::Utc::now().to_rfc3339(),
                session_id,
                request_id
            ],
        )? > 0;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "updated");
        }
        Ok(changed)
    }

    pub fn delete_queued_turn_user_message(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let record = tx
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2
                     AND status NOT IN ('inserting', 'dispatching')"
                ),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?;
        let changed = tx.execute(
            "DELETE FROM queued_turn_user_messages WHERE session_id = ?1 AND request_id = ?2
               AND status NOT IN ('inserting', 'dispatching')",
            params![session_id, request_id],
        )? > 0;
        tx.commit()?;
        drop(conn);
        if changed {
            if let Some(record) = record {
                crate::attachments::remove_discarded_queued_attachments(
                    session_id,
                    request_id,
                    &record.attachments,
                );
            }
            emit_changed(session_id, Some(request_id), "deleted");
        }
        Ok(changed)
    }

    pub fn request_turn_message_insertion(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages SET mode = 'force_insert', status = 'waiting_tool_boundary',
                 turn_id = ?1, updated_at = ?2 WHERE session_id = ?3 AND request_id = ?4
               AND status IN ('queued', 'fallback_after_reply')",
            params![turn_id, chrono::Utc::now().to_rfc3339(), session_id, request_id],
        )? > 0;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "waiting_tool_boundary");
        }
        Ok(changed)
    }

    pub fn cancel_turn_message_insertion(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages SET mode = 'queue', status = 'queued', turn_id = NULL,
                 updated_at = ?1 WHERE session_id = ?2 AND request_id = ?3 AND turn_id = ?4
               AND status = 'waiting_tool_boundary'",
            params![chrono::Utc::now().to_rfc3339(), session_id, request_id, turn_id],
        )? > 0;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "insertion_cancelled");
        }
        Ok(changed)
    }

    pub fn claim_turn_messages_for_insertion(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Vec<QueuedTurnMessageRecord>> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let records = {
            let mut stmt = tx.prepare(&format!(
                "{RECORD_SELECT} WHERE session_id = ?1 AND turn_id = ?2
                 AND status = 'waiting_tool_boundary' ORDER BY id ASC"
            ))?;
            let rows = stmt
                .query_map(params![session_id, turn_id], parse_record)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if !records.is_empty() {
            tx.execute(
                "UPDATE queued_turn_user_messages SET status = 'inserting', updated_at = ?1
                 WHERE session_id = ?2 AND turn_id = ?3 AND status = 'waiting_tool_boundary'",
                params![chrono::Utc::now().to_rfc3339(), session_id, turn_id],
            )?;
        }
        tx.commit()?;
        drop(conn);
        if !records.is_empty() {
            emit_changed(session_id, None, "inserting");
        }
        Ok(records)
    }

    pub fn complete_inserted_turn_message(
        &self,
        record: &QueuedTurnMessageRecord,
        message: &super::NewMessage,
    ) -> Result<i64> {
        let mut message = message.clone();
        message.queue_request_id = Some(record.request_id.clone());
        let message_id = self.append_message(&record.session_id, &message)?;
        if let Err(remove_error) =
            self.remove_consumed_turn_message(&record.session_id, &record.request_id)
        {
            // The user message is already durable. Never report this as an
            // insertion failure (the caller would correctly discard files for
            // a pre-persist failure). Reconcile by the message commit marker;
            // startup recovery provides the same idempotent fallback.
            let reconcile_error = self
                .reconcile_failed_turn_message_dispatch(
                    &record.session_id,
                    &record.request_id,
                    record.turn_id.as_deref().unwrap_or_default(),
                )
                .err();
            crate::app_warn!(
                "session",
                "turn_queue_consume_after_insert",
                "queued user message persisted but queue cleanup failed: {}; reconcile={:?}",
                remove_error,
                reconcile_error
            );
        }
        Ok(message_id)
    }

    /// Remove a queue row after a failed or rejected dispatch. Queue-owned
    /// attachment files are discarded because no durable message references them.
    pub fn remove_claimed_turn_message(&self, session_id: &str, request_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let record = tx
            .query_row(
                &format!("{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2"),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?;
        tx.execute(
            "DELETE FROM queued_turn_user_messages WHERE session_id = ?1 AND request_id = ?2",
            params![session_id, request_id],
        )?;
        tx.commit()?;
        drop(conn);
        if let Some(record) = record {
            crate::attachments::remove_discarded_queued_attachments(
                session_id,
                request_id,
                &record.attachments,
            );
        }
        emit_changed(session_id, Some(request_id), "removed");
        Ok(())
    }

    /// Remove a queue row only after its user message has been durably saved.
    /// Attachment files now belong to that message and must remain on disk.
    fn remove_consumed_turn_message(&self, session_id: &str, request_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "DELETE FROM queued_turn_user_messages WHERE session_id = ?1 AND request_id = ?2",
            params![session_id, request_id],
        )?;
        drop(conn);
        emit_changed(session_id, Some(request_id), "consumed");
        Ok(())
    }

    pub fn fallback_turn_message_insertions(&self, session_id: &str, turn_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages SET mode = 'queue', status = 'fallback_after_reply',
                 turn_id = NULL, updated_at = ?1 WHERE session_id = ?2 AND turn_id = ?3
               AND status IN ('waiting_tool_boundary', 'inserting')",
            params![chrono::Utc::now().to_rfc3339(), session_id, turn_id],
        )?;
        drop(conn);
        if changed > 0 {
            emit_changed(session_id, None, "fallback_after_reply");
        }
        Ok(())
    }

    pub fn claim_queued_turn_message_for_dispatch(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<Option<QueuedTurnMessageRecord>> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE queued_turn_user_messages SET mode = 'queue', status = 'dispatching', turn_id = ?1,
                 updated_at = ?2 WHERE session_id = ?3 AND request_id = ?4
               AND status IN ('queued', 'fallback_after_reply')
               AND id = (
                   SELECT MIN(id) FROM queued_turn_user_messages
                   WHERE session_id = ?3 AND status IN ('queued', 'fallback_after_reply')
               )",
            params![turn_id, chrono::Utc::now().to_rfc3339(), session_id, request_id],
        )? > 0;
        let record = if changed {
            tx.query_row(
                &format!("{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2"),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?
        } else {
            None
        };
        tx.commit()?;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "dispatching");
        }
        Ok(record)
    }

    pub fn release_queued_turn_message_dispatch(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages SET status = 'queued', turn_id = NULL, updated_at = ?1
             WHERE session_id = ?2 AND request_id = ?3 AND turn_id = ?4 AND status = 'dispatching'",
            params![chrono::Utc::now().to_rfc3339(), session_id, request_id, turn_id],
        )? > 0;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "dispatch_released");
        }
        Ok(changed)
    }

    pub fn consume_dispatched_turn_message(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "DELETE FROM queued_turn_user_messages WHERE session_id = ?1 AND request_id = ?2
             AND turn_id = ?3 AND status = 'dispatching'",
            params![session_id, request_id, turn_id],
        )? > 0;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "dispatched");
        }
        Ok(changed)
    }

    /// Reconcile the narrow failure window between persisting the user message
    /// and finishing chat-turn creation. The unique queue request id on
    /// `messages` is the commit marker: persisted means consume without
    /// deleting attachments; otherwise release the row for a safe retry.
    pub fn reconcile_failed_turn_message_dispatch(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let persisted = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = ?1 AND queue_request_id = ?2)",
            params![session_id, request_id],
            |row| row.get::<_, bool>(0),
        )?;
        let operation = if persisted {
            tx.execute(
                "DELETE FROM queued_turn_user_messages WHERE session_id = ?1 AND request_id = ?2",
                params![session_id, request_id],
            )?;
            "dispatch_reconciled_consumed"
        } else {
            tx.execute(
                "UPDATE queued_turn_user_messages SET status = 'queued', turn_id = NULL, updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND turn_id = ?4 AND status = 'dispatching'",
                params![
                    chrono::Utc::now().to_rfc3339(),
                    session_id,
                    request_id,
                    turn_id
                ],
            )?;
            "dispatch_reconciled_released"
        };
        tx.commit()?;
        drop(conn);
        emit_changed(session_id, Some(request_id), operation);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(session_id: &str, request_id: &str) -> NewQueuedTurnMessage {
        NewQueuedTurnMessage {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            message: format!("message-{request_id}"),
            display_text: None,
            attachments: Vec::new(),
            is_plan_trigger: false,
            goal_trigger: false,
            plan_comment: None,
            plan_mode: None,
            workflow_mode: None,
        }
    }

    #[test]
    fn queue_survives_reopen_and_is_session_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let (first, second) = {
            let db = SessionDB::open(&path).unwrap();
            let first = db.create_session("ha-main").unwrap().id;
            let second = db.create_session("ha-main").unwrap().id;
            db.enqueue_turn_user_message(queued(&first, "first"))
                .unwrap();
            db.enqueue_turn_user_message(queued(&second, "second"))
                .unwrap();
            (first, second)
        };
        let reopened = SessionDB::open(&path).unwrap();
        let first_items = reopened.list_queued_turn_user_messages(&first).unwrap();
        let second_items = reopened.list_queued_turn_user_messages(&second).unwrap();
        assert_eq!(first_items.len(), 1);
        assert_eq!(first_items[0].request_id, "first");
        assert_eq!(second_items.len(), 1);
        assert_eq!(second_items[0].request_id, "second");
    }

    #[test]
    fn insertion_claim_wins_over_late_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(queued(&session_id, "item"))
            .unwrap();
        assert!(db
            .request_turn_message_insertion(&session_id, "item", "turn")
            .unwrap());
        let claimed = db
            .claim_turn_messages_for_insertion(&session_id, "turn")
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(!db
            .cancel_turn_message_insertion(&session_id, "item", "turn")
            .unwrap());
    }

    #[test]
    fn queue_capacity_is_bounded_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        for index in 0..MAX_QUEUED_TURN_MESSAGES_PER_SESSION {
            db.enqueue_turn_user_message(queued(&session_id, &format!("item-{index}")))
                .unwrap();
        }
        let error = db
            .enqueue_turn_user_message(queued(&session_id, "overflow"))
            .unwrap_err();
        assert!(error.to_string().contains("message queue is full"));
    }

    #[test]
    fn startup_consumes_persisted_queue_request_and_recovers_uncommitted_claim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let session_id = {
            let db = SessionDB::open(&path).unwrap();
            let session_id = db.create_session("ha-main").unwrap().id;
            db.enqueue_turn_user_message(queued(&session_id, "persisted"))
                .unwrap();
            db.enqueue_turn_user_message(queued(&session_id, "retry"))
                .unwrap();
            let persisted = db
                .claim_queued_turn_message_for_dispatch(&session_id, "persisted", "turn-a")
                .unwrap()
                .unwrap();
            assert_eq!(persisted.request_id, "persisted");
            let mut message = super::super::NewMessage::user("persisted");
            message.queue_request_id = Some("persisted".to_string());
            db.append_message(&session_id, &message).unwrap();
            db.claim_queued_turn_message_for_dispatch(&session_id, "retry", "turn-b")
                .unwrap()
                .unwrap();
            session_id
        };
        let reopened = SessionDB::open(&path).unwrap();
        let items = reopened
            .list_queued_turn_user_messages(&session_id)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request_id, "retry");
        assert_eq!(items[0].status, QueuedTurnMessageStatus::Queued);
    }

    #[test]
    fn failed_dispatch_reconcile_consumes_committed_and_releases_uncommitted_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(queued(&session_id, "committed"))
            .unwrap();
        db.enqueue_turn_user_message(queued(&session_id, "retry"))
            .unwrap();
        db.claim_queued_turn_message_for_dispatch(&session_id, "committed", "turn-a")
            .unwrap()
            .unwrap();
        db.claim_queued_turn_message_for_dispatch(&session_id, "retry", "turn-b")
            .unwrap()
            .unwrap();

        let mut message = super::super::NewMessage::user("committed");
        message.queue_request_id = Some("committed".to_string());
        db.append_message(&session_id, &message).unwrap();

        db.reconcile_failed_turn_message_dispatch(&session_id, "committed", "turn-a")
            .unwrap();
        db.reconcile_failed_turn_message_dispatch(&session_id, "retry", "turn-b")
            .unwrap();

        let items = db.list_queued_turn_user_messages(&session_id).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request_id, "retry");
        assert_eq!(items[0].status, QueuedTurnMessageStatus::Queued);
    }

    #[test]
    fn dispatch_claim_cannot_skip_an_earlier_fifo_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(queued(&session_id, "first"))
            .unwrap();
        db.enqueue_turn_user_message(queued(&session_id, "second"))
            .unwrap();

        assert!(db
            .claim_queued_turn_message_for_dispatch(&session_id, "second", "turn-b")
            .unwrap()
            .is_none());
        assert!(db
            .claim_queued_turn_message_for_dispatch(&session_id, "first", "turn-a")
            .unwrap()
            .is_some());
    }
}
