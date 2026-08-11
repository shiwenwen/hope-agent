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
    HeldAfterStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedTurnMessageSource {
    Desktop,
    Http,
    Channel,
}

impl QueuedTurnMessageSource {
    fn parse(value: &str) -> Self {
        match value {
            "http" => Self::Http,
            "channel" => Self::Channel,
            _ => Self::Desktop,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Http => "http",
            Self::Channel => "channel",
        }
    }

    pub fn is_channel_managed(self) -> bool {
        matches!(self, Self::Channel)
    }
}

impl QueuedTurnMessageStatus {
    fn parse(value: &str) -> Self {
        match value {
            "waiting_tool_boundary" => Self::WaitingToolBoundary,
            "inserting" => Self::Inserting,
            "dispatching" => Self::Dispatching,
            "fallback_after_reply" => Self::FallbackAfterReply,
            "held_after_stop" => Self::HeldAfterStop,
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
    pub incoming_turn: Option<crate::prompt_context::IncomingTurnWire>,
    /// Frozen execution ceiling for server-expanded explicit Skill commands
    /// (notably IM). First-party typed turns re-resolve their Skill binding.
    pub skill_allowed_tools: Vec<String>,
    /// Fail-closed durable marker derived from the raw options JSON. A typed
    /// turn only requires full dispatch when it carries mention semantics; a
    /// valid empty typed envelope may be inserted as ordinary text. A future
    /// or malformed non-null sidecar must not become insertable merely because
    /// this binary cannot deserialize its payload yet.
    structured_sidecar_present: bool,
    /// A durable sidecar that this binary cannot consume must never be
    /// dispatched with an accidentally widened execution ceiling.
    sidecar_decode_error: Option<QueuedSidecarDecodeError>,
    pub source: QueuedTurnMessageSource,
    /// Minimal, credential-free routing envelope for a Channel-managed row.
    /// Provider tokens and raw webhook payloads must never be stored here.
    pub channel_origin: Option<serde_json::Value>,
    pub mode: QueuedTurnMessageMode,
    pub status: QueuedTurnMessageStatus,
    pub created_at: String,
    pub updated_at: String,
}

impl QueuedTurnMessageRecord {
    /// Typed mention bindings and a frozen Skill tool ceiling are resolved only
    /// while constructing a complete chat turn. Injecting either sidecar as a
    /// raw mid-turn user message would silently discard its semantics.
    fn requires_full_turn_dispatch(&self) -> bool {
        self.structured_sidecar_present
    }

    fn sidecar_dispatch_error(&self) -> Option<anyhow::Error> {
        self.sidecar_decode_error.map(|error| {
            anyhow!(
                "queued message sidecar cannot be safely decoded: {}",
                error.message()
            )
        })
    }

    /// Authoritative UI eligibility for requesting a tool-boundary insert.
    /// The execution path repeats these checks transactionally; this projection
    /// only prevents clients from offering an action the backend must reject.
    fn can_force_insert(&self) -> bool {
        !self.source.is_channel_managed()
            && self.sidecar_decode_error.is_none()
            && !self.requires_full_turn_dispatch()
            && matches!(
                self.status,
                QueuedTurnMessageStatus::Queued | QueuedTurnMessageStatus::FallbackAfterReply
            )
            && self.mode != QueuedTurnMessageMode::ForceInsert
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueuedSidecarDecodeError {
    OptionsJson,
    IncomingTurn,
    SkillAllowedTools,
}

impl QueuedSidecarDecodeError {
    fn message(self) -> &'static str {
        match self {
            Self::OptionsJson => "options_json is not a JSON object",
            Self::IncomingTurn => "incomingTurn is not a supported typed turn",
            Self::SkillAllowedTools => "skillAllowedTools must be an array containing only strings",
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_turn: Option<crate::prompt_context::IncomingTurnWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_allowed_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<&'static str>,
    /// Backend-authoritative projection of whether this row may be requested
    /// for tool-boundary insertion in its current state.
    pub can_force_insert: bool,
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
            incoming_turn: value.incoming_turn.clone(),
            skill_allowed_tools: (!value.skill_allowed_tools.is_empty())
                .then(|| value.skill_allowed_tools.clone()),
            managed_by: value.source.is_channel_managed().then_some("channel"),
            can_force_insert: value.can_force_insert(),
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
    pub incoming_turn: Option<crate::prompt_context::IncomingTurnWire>,
    pub skill_allowed_tools: Vec<String>,
    pub source: QueuedTurnMessageSource,
    pub channel_origin: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct EnqueueQueuedTurnMessageOutcome {
    pub item: QueuedTurnMessageView,
    pub inserted: bool,
}

pub(super) fn emit_changed(session_id: &str, request_id: Option<&str>, operation: &str) {
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
    let message: String = row.get(3)?;
    let attachments_json: String = row.get(5)?;
    let plan_comment_json: Option<String> = row.get(8)?;
    let options_json: Option<String> = row.get(9)?;
    let (options, options_parse_failed) = match options_json.as_deref() {
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(value) => (value, false),
            Err(_) => (serde_json::Value::Null, true),
        },
        None => (serde_json::Value::Null, false),
    };
    let options_shape_invalid = !options.is_null() && !options.is_object();
    let incoming_turn_present = options
        .get("incomingTurn")
        .is_some_and(|value| !value.is_null());
    let incoming_turn = options
        .get("incomingTurn")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let incoming_turn_decode_failed = incoming_turn_present
        && incoming_turn.as_ref().is_none_or(|wire| {
            crate::prompt_context::validate_incoming_turn(&message, Some(wire)).is_err()
        });
    let incoming_turn_requires_full_dispatch = incoming_turn
        .as_ref()
        .is_some_and(|wire| !wire.mentions.is_empty());
    let legacy_note_requires_full_dispatch = crate::knowledge::contains_legacy_wikilink(&message);
    let (skill_allowed_tools, skill_ceiling_present, skill_ceiling_decode_failed) =
        match options.get("skillAllowedTools") {
            None | Some(serde_json::Value::Null) => (Vec::new(), false, false),
            Some(serde_json::Value::Array(values)) => {
                let tools = values
                    .iter()
                    .map(|value| value.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>();
                match tools {
                    Some(tools) => {
                        let present = !tools.is_empty();
                        (tools, present, false)
                    }
                    None => (Vec::new(), true, true),
                }
            }
            Some(_) => (Vec::new(), true, true),
        };
    let sidecar_decode_error = if options_parse_failed || options_shape_invalid {
        Some(QueuedSidecarDecodeError::OptionsJson)
    } else if incoming_turn_decode_failed {
        Some(QueuedSidecarDecodeError::IncomingTurn)
    } else if skill_ceiling_decode_failed {
        Some(QueuedSidecarDecodeError::SkillAllowedTools)
    } else {
        None
    };
    let source: String = row.get(10)?;
    let channel_origin_json: Option<String> = row.get(11)?;
    let mode: String = row.get(12)?;
    let status: String = row.get(13)?;
    Ok(QueuedTurnMessageRecord {
        request_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        message,
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
        incoming_turn,
        skill_allowed_tools,
        structured_sidecar_present: options_parse_failed
            || options_shape_invalid
            || incoming_turn_decode_failed
            || incoming_turn_requires_full_dispatch
            || legacy_note_requires_full_dispatch
            || skill_ceiling_present,
        sidecar_decode_error,
        source: QueuedTurnMessageSource::parse(&source),
        channel_origin: channel_origin_json.and_then(|raw| serde_json::from_str(&raw).ok()),
        mode: QueuedTurnMessageMode::parse(&mode),
        status: QueuedTurnMessageStatus::parse(&status),
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

const RECORD_SELECT: &str = "SELECT request_id, session_id, turn_id, message, display_text,
    attachments_json, is_plan_trigger, goal_trigger, plan_comment_json, options_json, source,
    channel_origin_json, mode, status, created_at, updated_at FROM queued_turn_user_messages";

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
                source TEXT NOT NULL DEFAULT 'desktop',
                channel_origin_json TEXT,
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
        if conn
            .prepare("SELECT source FROM queued_turn_user_messages LIMIT 1")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE queued_turn_user_messages ADD COLUMN source TEXT NOT NULL DEFAULT 'desktop';",
            )?;
        }
        if conn
            .prepare("SELECT channel_origin_json FROM queued_turn_user_messages LIMIT 1")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE queued_turn_user_messages ADD COLUMN channel_origin_json TEXT;",
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
        crate::attachments::validate_typed_resource_attachment_bindings(
            &input.message,
            input.incoming_turn.as_ref(),
            &input.attachments,
        )?;
        match (input.source, input.channel_origin.is_some()) {
            (QueuedTurnMessageSource::Channel, false) => {
                return Err(anyhow!(
                    "Channel-managed queued message requires routing origin"
                ));
            }
            (QueuedTurnMessageSource::Desktop | QueuedTurnMessageSource::Http, true) => {
                return Err(anyhow!(
                    "non-Channel queued message cannot carry routing origin"
                ));
            }
            _ => {}
        }
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
        let channel_origin_json = input
            .channel_origin
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let inserted = tx.execute(
            "INSERT INTO queued_turn_user_messages (
                request_id, session_id, turn_id, message, display_text, attachments_json,
                is_plan_trigger, goal_trigger, plan_comment_json, options_json, source,
                channel_origin_json, mode, status, created_at, updated_at
             ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                       'queue', 'queued', ?12, ?12)
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
                    "incomingTurn": input.incoming_turn,
                    "skillAllowedTools": input.skill_allowed_tools,
                }))?,
                input.source.as_str(),
                channel_origin_json,
                now,
            ],
        )? > 0;
        tx.commit()?;
        drop(conn);
        let record = self
            .get_queued_turn_user_message(&input.session_id, &input.request_id)?
            .ok_or_else(|| anyhow!("failed to read queued message after insert"))?;
        if inserted {
            emit_changed(&input.session_id, Some(&input.request_id), "enqueued");
        }
        Ok(EnqueueQueuedTurnMessageOutcome {
            item: QueuedTurnMessageView::from(&record),
            inserted,
        })
    }

    pub fn queue_request_was_consumed(&self, request_id: &str) -> Result<bool> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE queue_request_id = ?1)",
            params![request_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub fn has_channel_turn_messages(&self, session_id: &str) -> Result<bool> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM queued_turn_user_messages
                WHERE session_id = ?1 AND source = 'channel'
             )",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub fn channel_dispatch_claim_is_active(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM queued_turn_user_messages
                WHERE session_id = ?1 AND request_id = ?2 AND source = 'channel'
                  AND turn_id = ?3 AND status = 'dispatching'
             )",
            params![session_id, request_id, turn_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
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
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let attachments_json = tx
            .query_row(
                "SELECT attachments_json FROM queued_turn_user_messages
                 WHERE session_id = ?1 AND request_id = ?2
                   AND source != 'channel'
                   AND status IN ('queued', 'waiting_tool_boundary', 'fallback_after_reply')",
                params![session_id, request_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(attachments_json) = attachments_json else {
            tx.commit()?;
            return Ok(false);
        };
        let attachments = serde_json::from_str::<Vec<Attachment>>(&attachments_json)?;
        let (removed_typed_attachments, retained_attachments): (Vec<_>, Vec<_>) =
            attachments.into_iter().partition(|attachment| {
                matches!(
                    attachment.source.as_deref(),
                    Some("mention" | "plan_mention")
                )
            });
        let retained_attachments_json = serde_json::to_string(&retained_attachments)?;
        let changed = tx.execute(
            "UPDATE queued_turn_user_messages SET message = ?1, display_text = ?2,
                    attachments_json = ?3,
                    options_json = CASE
                        WHEN json_valid(COALESCE(options_json, '{}'))
                        THEN CASE
                            WHEN json_type(COALESCE(options_json, '{}')) = 'object'
                            THEN json_remove(
                                COALESCE(options_json, '{}'),
                                '$.incomingTurn',
                                '$.skillAllowedTools'
                            )
                            ELSE '{}'
                        END
                        ELSE '{}'
                    END,
                    updated_at = ?4
             WHERE session_id = ?5 AND request_id = ?6
               AND source != 'channel'
               AND status IN ('queued', 'waiting_tool_boundary', 'fallback_after_reply')",
            params![
                message,
                display_text,
                retained_attachments_json,
                chrono::Utc::now().to_rfc3339(),
                session_id,
                request_id
            ],
        )? > 0;
        tx.commit()?;
        drop(conn);
        if changed {
            // Valid typed resources are path references and have no queued
            // copy, but route legacy rows through the same owner-scoped
            // request-prefix cleanup. The helper deliberately ignores an
            // arbitrary external mention path.
            crate::attachments::remove_discarded_queued_attachments(
                session_id,
                request_id,
                &removed_typed_attachments,
            );
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
               AND source != 'channel'
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
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let record = tx
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2
                     AND source != 'channel'
                     AND status IN ('queued', 'fallback_after_reply')"
                ),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?;
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(error) = record
            .as_ref()
            .and_then(QueuedTurnMessageRecord::sidecar_dispatch_error)
        {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'fallback_after_reply', turn_id = NULL,
                     updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND source != 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![now, session_id, request_id],
            )?;
            tx.commit()?;
            drop(conn);
            emit_changed(session_id, Some(request_id), "sidecar_decode_failed");
            return Err(error);
        }
        let deferred = record
            .as_ref()
            .is_some_and(QueuedTurnMessageRecord::requires_full_turn_dispatch);
        let changed = if deferred {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'fallback_after_reply', turn_id = NULL,
                     updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND source != 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![now, session_id, request_id],
            )?;
            false
        } else {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'force_insert', status = 'waiting_tool_boundary',
                     turn_id = ?1, updated_at = ?2
                 WHERE session_id = ?3 AND request_id = ?4 AND source != 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![turn_id, now, session_id, request_id],
            )? > 0
        };
        tx.commit()?;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "waiting_tool_boundary");
        } else if deferred {
            emit_changed(session_id, Some(request_id), "fallback_after_reply");
        }
        Ok(changed)
    }

    pub(crate) fn request_channel_turn_message_insertion(
        &self,
        session_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let record = tx
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2
                     AND source = 'channel'
                     AND status IN ('queued', 'fallback_after_reply')
                     AND id = (
                         SELECT MIN(id) FROM queued_turn_user_messages
                         WHERE session_id = ?1 AND source = 'channel'
                     )"
                ),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?;
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(error) = record
            .as_ref()
            .and_then(QueuedTurnMessageRecord::sidecar_dispatch_error)
        {
            // Channel rows have no owner edit surface. Reuse the existing held
            // state so the background FIFO pump cannot hot-loop on an
            // undecodable execution ceiling. A later inbound message may retry
            // it, but this binary will deterministically hold it again.
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'held_after_stop', turn_id = NULL,
                     updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![now, session_id, request_id],
            )?;
            tx.commit()?;
            drop(conn);
            emit_changed(session_id, Some(request_id), "sidecar_decode_failed");
            return Err(error);
        }
        let deferred = record
            .as_ref()
            .is_some_and(QueuedTurnMessageRecord::requires_full_turn_dispatch);
        let changed = if deferred {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'fallback_after_reply', turn_id = NULL,
                     updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![now, session_id, request_id],
            )?;
            false
        } else {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'force_insert', status = 'waiting_tool_boundary',
                     turn_id = ?1, updated_at = ?2
                 WHERE session_id = ?3 AND request_id = ?4 AND source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply')
                   AND id = (
                       SELECT MIN(id) FROM queued_turn_user_messages
                       WHERE session_id = ?3 AND source = 'channel'
                   )",
                params![turn_id, now, session_id, request_id],
            )? > 0
        };
        tx.commit()?;
        drop(conn);
        if changed {
            emit_changed(session_id, Some(request_id), "waiting_tool_boundary");
        } else if deferred {
            emit_changed(session_id, Some(request_id), "fallback_after_reply");
        }
        Ok(changed)
    }

    pub(crate) fn next_channel_turn_message_for_insertion(
        &self,
        session_id: &str,
    ) -> Result<Option<String>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT request_id FROM queued_turn_user_messages
             WHERE session_id = ?1 AND source = 'channel'
               AND status IN ('queued', 'fallback_after_reply')
             ORDER BY id ASC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
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
               AND source != 'channel'
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
        let candidates = {
            let mut stmt = tx.prepare(&format!(
                "{RECORD_SELECT} WHERE session_id = ?1 AND turn_id = ?2
                 AND status = 'waiting_tool_boundary' ORDER BY id ASC"
            ))?;
            let rows = stmt
                .query_map(params![session_id, turn_id], parse_record)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let now = chrono::Utc::now().to_rfc3339();
        let mut deferred_count = 0;
        let mut quarantined_count = 0;
        let mut decode_failed_count = 0;
        let mut records = Vec::with_capacity(candidates.len());
        for record in candidates {
            if record.sidecar_decode_error.is_some() {
                let channel_managed = record.source.is_channel_managed();
                let target_status = if channel_managed {
                    "held_after_stop"
                } else {
                    "fallback_after_reply"
                };
                let changed = tx.execute(
                    "UPDATE queued_turn_user_messages
                     SET mode = 'queue', status = ?1, turn_id = NULL, updated_at = ?2
                     WHERE session_id = ?3 AND request_id = ?4 AND turn_id = ?5
                       AND status = 'waiting_tool_boundary'",
                    params![target_status, now, session_id, record.request_id, turn_id],
                )?;
                decode_failed_count += changed;
                if channel_managed {
                    quarantined_count += changed;
                } else {
                    deferred_count += changed;
                }
            } else if record.requires_full_turn_dispatch() {
                deferred_count += tx.execute(
                    "UPDATE queued_turn_user_messages
                     SET mode = 'queue', status = 'fallback_after_reply', turn_id = NULL,
                         updated_at = ?1
                     WHERE session_id = ?2 AND request_id = ?3 AND turn_id = ?4
                       AND status = 'waiting_tool_boundary'",
                    params![now, session_id, record.request_id, turn_id],
                )?;
            } else if tx.execute(
                "UPDATE queued_turn_user_messages SET status = 'inserting', updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND turn_id = ?4
                   AND status = 'waiting_tool_boundary'",
                params![now, session_id, record.request_id, turn_id],
            )? > 0
            {
                records.push(record);
            }
        }
        tx.commit()?;
        drop(conn);
        if deferred_count > 0 {
            emit_changed(session_id, None, "fallback_after_reply");
        }
        if quarantined_count > 0 {
            emit_changed(session_id, None, "sidecar_decode_failed");
        }
        if decode_failed_count > 0 {
            crate::app_warn!(
                "session",
                "turn_queue_sidecar_decode",
                "held {} queued message(s) because a structured sidecar could not be decoded",
                decode_failed_count
            );
        }
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
        let candidate = tx
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE session_id = ?1 AND request_id = ?2
                     AND source != 'channel'
                     AND status IN ('queued', 'fallback_after_reply')
                     AND id = (
                         SELECT MIN(id) FROM queued_turn_user_messages
                         WHERE session_id = ?1 AND source != 'channel'
                           AND status IN ('queued', 'fallback_after_reply')
                     )"
                ),
                params![session_id, request_id],
                parse_record,
            )
            .optional()?;
        let Some(candidate) = candidate else {
            tx.commit()?;
            return Ok(None);
        };
        if let Some(error) = candidate.sidecar_dispatch_error() {
            return Err(error);
        }
        let changed = tx.execute(
            "UPDATE queued_turn_user_messages SET mode = 'queue', status = 'dispatching', turn_id = ?1,
                 updated_at = ?2 WHERE session_id = ?3 AND request_id = ?4
               AND source != 'channel'
               AND status IN ('queued', 'fallback_after_reply')
               AND id = (
                   SELECT MIN(id) FROM queued_turn_user_messages
                   WHERE session_id = ?3 AND source != 'channel'
                     AND status IN ('queued', 'fallback_after_reply')
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

    /// Atomically claim the oldest backend-managed IM row for a session.
    /// GUI / HTTP rows are deliberately excluded: their auto-send preference
    /// and user-editable draft semantics remain owned by their clients.
    pub fn claim_next_channel_turn_message_for_dispatch(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<QueuedTurnMessageRecord>> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let candidate = tx
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE session_id = ?1 AND source = 'channel'
                     AND status IN ('queued', 'fallback_after_reply')
                     AND id = (
                         SELECT MIN(id) FROM queued_turn_user_messages
                         WHERE session_id = ?1 AND source = 'channel'
                     )
                     AND NOT EXISTS (
                         SELECT 1 FROM queued_turn_user_messages active_claim
                         WHERE active_claim.session_id = ?1
                           AND active_claim.source = 'channel'
                           AND active_claim.status = 'dispatching'
                     )
                     ORDER BY id ASC LIMIT 1"
                ),
                params![session_id],
                parse_record,
            )
            .optional()?;
        let Some(candidate) = candidate else {
            tx.commit()?;
            return Ok(None);
        };
        let request_id = candidate.request_id.clone();
        if let Some(error) = candidate.sidecar_dispatch_error() {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'held_after_stop', turn_id = NULL,
                     updated_at = ?1
                 WHERE session_id = ?2 AND request_id = ?3 AND source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply')",
                params![chrono::Utc::now().to_rfc3339(), session_id, request_id],
            )?;
            tx.commit()?;
            drop(conn);
            emit_changed(session_id, Some(&request_id), "sidecar_decode_failed");
            return Err(error);
        }
        let changed = tx.execute(
            "UPDATE queued_turn_user_messages
             SET mode = 'queue', status = 'dispatching', turn_id = ?1, updated_at = ?2
             WHERE session_id = ?3 AND request_id = ?4 AND source = 'channel'
               AND status IN ('queued', 'fallback_after_reply')",
            params![
                turn_id,
                chrono::Utc::now().to_rfc3339(),
                session_id,
                request_id
            ],
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
        if let Some(record) = record.as_ref() {
            emit_changed(session_id, Some(&record.request_id), "dispatching");
        }
        Ok(record)
    }

    pub fn list_channel_queued_session_ids(&self) -> Result<Vec<String>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT q.session_id FROM queued_turn_user_messages q
             WHERE q.source = 'channel'
               AND q.status IN ('queued', 'fallback_after_reply')
               AND q.id = (
                   SELECT MIN(head.id) FROM queued_turn_user_messages head
                   WHERE head.session_id = q.session_id AND head.source = 'channel'
               )
             ORDER BY q.session_id",
        )?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Preserve backend-managed IM rows after an explicit user Stop. They are
    /// intentionally excluded from startup recovery and the Channel pump until
    /// the next ordinary inbound message explicitly resumes this session.
    pub fn hold_channel_turn_messages_after_stop(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages
             SET mode = 'queue', status = 'held_after_stop', turn_id = NULL, updated_at = ?1
             WHERE session_id = ?2 AND source = 'channel'
               AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                              'inserting', 'dispatching')",
            params![chrono::Utc::now().to_rfc3339(), session_id],
        )?;
        drop(conn);
        if changed > 0 {
            emit_changed(session_id, None, "held_after_stop");
        }
        Ok(changed)
    }

    /// Global Stop variant. Return the affected session ids so callers can
    /// include queue-only sessions in their stopped-session accounting.
    pub fn hold_all_channel_turn_messages_after_stop(&self) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let session_ids = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT session_id FROM queued_turn_user_messages
                 WHERE source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                                  'inserting', 'dispatching')
                 ORDER BY session_id",
            )?;
            let rows = stmt
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            rows
        };
        if !session_ids.is_empty() {
            tx.execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'queue', status = 'held_after_stop', turn_id = NULL, updated_at = ?1
                 WHERE source = 'channel'
                   AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                                  'inserting', 'dispatching')",
                params![chrono::Utc::now().to_rfc3339()],
            )?;
        }
        tx.commit()?;
        drop(conn);
        for session_id in &session_ids {
            emit_changed(session_id, None, "held_after_stop");
        }
        Ok(session_ids)
    }

    pub fn resume_channel_turn_messages_after_stop(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE queued_turn_user_messages
             SET status = 'queued', updated_at = ?1
             WHERE session_id = ?2 AND source = 'channel' AND status = 'held_after_stop'",
            params![chrono::Utc::now().to_rfc3339(), session_id],
        )?;
        drop(conn);
        if changed > 0 {
            emit_changed(session_id, None, "resumed_after_stop");
        }
        Ok(changed)
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
            incoming_turn: None,
            skill_allowed_tools: Vec::new(),
            source: QueuedTurnMessageSource::Desktop,
            channel_origin: None,
        }
    }

    fn channel_queued(session_id: &str, request_id: &str) -> NewQueuedTurnMessage {
        NewQueuedTurnMessage {
            source: QueuedTurnMessageSource::Channel,
            channel_origin: Some(serde_json::json!({
                "channelId": "wechat",
                "accountId": "account",
                "chatId": "chat",
                "messageId": request_id,
            })),
            ..queued(session_id, request_id)
        }
    }

    fn incoming_turn_for(message: &str) -> crate::prompt_context::IncomingTurnWire {
        crate::prompt_context::IncomingTurnWire {
            prompt_contract_version: crate::prompt_context::PROMPT_CONTRACT_VERSION,
            mention_wire_version: crate::prompt_context::MENTION_WIRE_VERSION,
            user_input: crate::prompt_context::CanonicalUserInput {
                input_item_id: "queued-input".into(),
                canonicalization_version: 1,
                text: message.into(),
                digest: crate::prompt_context::canonical_text_digest(message),
            },
            mentions: Vec::new(),
        }
    }

    fn incoming_turn_with_file_mention(
        message: &str,
        target_id: &str,
    ) -> crate::prompt_context::IncomingTurnWire {
        let mut wire = incoming_turn_for(message);
        wire.mentions
            .push(crate::prompt_context::MentionBindingWire {
                id: "queued-file".into(),
                kind: crate::prompt_context::MentionKind::File,
                target_id: target_id.into(),
                display_label: target_id.into(),
                origin: crate::prompt_context::StructuredMentionOrigin::ExplicitApiBinding,
                source_anchor: crate::prompt_context::SourceAnchor::Inline {
                    input_item_id: wire.user_input.input_item_id.clone(),
                    canonical_text_digest: wire.user_input.digest.clone(),
                    start_utf8: 0,
                    end_utf8: message.len() as u64,
                },
            });
        wire
    }

    fn queued_file_attachment(target_id: &str) -> Attachment {
        Attachment {
            name: target_id.to_string(),
            mime_type: "text/plain".into(),
            source: Some("mention".into()),
            data: None,
            file_path: Some(format!("/workspace/{target_id}")),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        }
    }

    fn replace_options_json(db: &SessionDB, session_id: &str, request_id: &str, raw_options: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE queued_turn_user_messages SET options_json = ?1
                 WHERE session_id = ?2 AND request_id = ?3",
                params![raw_options, session_id, request_id],
            )
            .unwrap();
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
    fn queue_round_trips_the_explicit_skill_tool_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let session_id = {
            let db = SessionDB::open(&path).unwrap();
            let session_id = db.create_session("ha-main").unwrap().id;
            let mut input = channel_queued(&session_id, "skill-command");
            input.skill_allowed_tools = vec!["read".into(), "glob".into()];
            db.enqueue_turn_user_message(input).unwrap();
            session_id
        };

        let reopened = SessionDB::open(&path).unwrap();
        let row = reopened
            .get_queued_turn_user_message(&session_id, "skill-command")
            .unwrap()
            .unwrap();
        assert_eq!(row.skill_allowed_tools, vec!["read", "glob"]);
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
    fn valid_empty_incoming_turn_can_force_insert_at_tool_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let mut input = queued(&session_id, "empty-typed-envelope");
        input.incoming_turn = Some(incoming_turn_for(&input.message));
        let enqueued = db.enqueue_turn_user_message(input).unwrap();
        assert!(enqueued.item.can_force_insert);

        assert!(db
            .request_turn_message_insertion(&session_id, "empty-typed-envelope", "active-turn",)
            .unwrap());
        let claimed = db
            .claim_turn_messages_for_insertion(&session_id, "active-turn")
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(claimed[0]
            .incoming_turn
            .as_ref()
            .is_some_and(|wire| wire.mentions.is_empty()));
        assert_eq!(
            db.get_queued_turn_user_message(&session_id, "empty-typed-envelope")
                .unwrap()
                .unwrap()
                .status,
            QueuedTurnMessageStatus::Inserting
        );
    }

    #[test]
    fn empty_incoming_turn_with_legacy_wikilink_requires_full_dispatch() {
        for (request_id, message) in [
            ("legacy-note", "Please use [[Roadmap]]"),
            ("legacy-note-inline-code", "Please use `[[Roadmap]]`"),
            (
                "legacy-note-fenced-code",
                "Example:\n```md\n[[Roadmap]]\n```",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
            let session_id = db.create_session("ha-main").unwrap().id;
            let mut input = queued(&session_id, request_id);
            input.message = message.into();
            input.incoming_turn = Some(incoming_turn_for(&input.message));
            let enqueued = db.enqueue_turn_user_message(input).unwrap();
            assert!(!enqueued.item.can_force_insert);

            assert!(!db
                .request_turn_message_insertion(&session_id, request_id, "active-turn")
                .unwrap());
            assert!(db
                .claim_turn_messages_for_insertion(&session_id, "active-turn")
                .unwrap()
                .is_empty());
            let held = db
                .get_queued_turn_user_message(&session_id, request_id)
                .unwrap()
                .unwrap();
            assert_eq!(held.status, QueuedTurnMessageStatus::FallbackAfterReply);
            let dispatched = db
                .claim_queued_turn_message_for_dispatch(&session_id, request_id, "next-turn")
                .unwrap()
                .unwrap();
            assert_eq!(dispatched.message, message);
            assert!(dispatched
                .incoming_turn
                .as_ref()
                .is_some_and(|wire| wire.mentions.is_empty()));
        }
    }

    #[test]
    fn empty_incoming_turn_with_markdown_link_can_force_insert() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let mut input = queued(&session_id, "markdown-link");
        input.message = "[Roadmap](notes/Roadmap.md)".into();
        input.incoming_turn = Some(incoming_turn_for(&input.message));
        db.enqueue_turn_user_message(input).unwrap();

        assert!(db
            .request_turn_message_insertion(&session_id, "markdown-link", "active-turn")
            .unwrap());
        assert_eq!(
            db.claim_turn_messages_for_insertion(&session_id, "active-turn")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn desktop_and_http_sidecars_fall_back_to_full_turn_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;

        let mut desktop = queued(&session_id, "desktop-typed");
        desktop.message = "@README.md".into();
        desktop.incoming_turn = Some(incoming_turn_with_file_mention(
            &desktop.message,
            "README.md",
        ));
        desktop.attachments = vec![queued_file_attachment("README.md")];
        let desktop_enqueued = db.enqueue_turn_user_message(desktop).unwrap();
        assert!(!desktop_enqueued.item.can_force_insert);

        let mut http = queued(&session_id, "http-skill");
        http.source = QueuedTurnMessageSource::Http;
        http.skill_allowed_tools = vec!["read".into()];
        let http_enqueued = db.enqueue_turn_user_message(http).unwrap();
        assert!(!http_enqueued.item.can_force_insert);

        assert!(!db
            .request_turn_message_insertion(&session_id, "desktop-typed", "active-turn")
            .unwrap());
        assert!(!db
            .request_turn_message_insertion(&session_id, "http-skill", "active-turn")
            .unwrap());
        assert!(db
            .claim_turn_messages_for_insertion(&session_id, "active-turn")
            .unwrap()
            .is_empty());

        for request_id in ["desktop-typed", "http-skill"] {
            let row = db
                .get_queued_turn_user_message(&session_id, request_id)
                .unwrap()
                .unwrap();
            assert_eq!(row.mode, QueuedTurnMessageMode::Queue);
            assert_eq!(row.status, QueuedTurnMessageStatus::FallbackAfterReply);
            assert!(row.turn_id.is_none());
        }

        let desktop = db
            .claim_queued_turn_message_for_dispatch(&session_id, "desktop-typed", "next-turn")
            .unwrap()
            .unwrap();
        assert!(desktop.incoming_turn.is_some());
        db.remove_claimed_turn_message(&session_id, "desktop-typed")
            .unwrap();
        let http = db
            .claim_queued_turn_message_for_dispatch(&session_id, "http-skill", "next-turn")
            .unwrap()
            .unwrap();
        assert_eq!(http.skill_allowed_tools, vec!["read"]);
    }

    #[test]
    fn editing_typed_queue_row_drops_typed_attachments_and_preserves_ordinary_ones() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let mut input = queued(&session_id, "edit-typed");
        input.message = "@README.md".into();
        input.incoming_turn = Some(incoming_turn_with_file_mention(&input.message, "README.md"));
        let ordinary = Attachment {
            name: "ordinary.txt".into(),
            mime_type: "text/plain".into(),
            source: Some("upload".into()),
            data: None,
            file_path: None,
            upload_id: Some("ordinary-upload-lease".into()),
            quote_lines: None,
            quote_role: None,
        };
        input.attachments = vec![queued_file_attachment("README.md"), ordinary.clone()];
        db.enqueue_turn_user_message(input).unwrap();

        assert!(db
            .update_queued_turn_user_message(&session_id, "edit-typed", "repaired message", None,)
            .unwrap());
        let edited = db
            .get_queued_turn_user_message(&session_id, "edit-typed")
            .unwrap()
            .unwrap();
        assert!(edited.incoming_turn.is_none());
        assert_eq!(edited.attachments.len(), 1);
        assert_eq!(edited.attachments[0].source.as_deref(), Some("upload"));
        assert_eq!(edited.attachments[0].upload_id, ordinary.upload_id);

        let dispatched = db
            .claim_queued_turn_message_for_dispatch(&session_id, "edit-typed", "repaired-turn")
            .unwrap()
            .unwrap();
        assert_eq!(dispatched.message, "repaired message");
        assert!(dispatched.incoming_turn.is_none());
        assert_eq!(dispatched.attachments.len(), 1);
        assert_eq!(dispatched.attachments[0].source.as_deref(), Some("upload"));
    }

    #[test]
    fn undecodable_non_channel_sidecars_fail_closed_and_remain_editable() {
        let cases = [
            (
                "future-skill-ceiling",
                r#"{"skillAllowedTools":{"version":2,"tools":["read"]}}"#,
            ),
            (
                "mixed-skill-ceiling",
                r#"{"skillAllowedTools":["read",{"future":"glob"}]}"#,
            ),
            (
                "future-incoming-turn",
                r#"{"incomingTurn":{"promptContractVersion":999,"mentionWireVersion":1,"userInput":{"inputItemId":"future-input","canonicalizationVersion":1,"text":"queued message","digest":"sha256:future"},"mentions":[]}}"#,
            ),
            ("malformed-options", "{not-json"),
        ];

        for (request_id, raw_options) in cases {
            let dir = tempfile::tempdir().unwrap();
            let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
            let session_id = db.create_session("ha-main").unwrap().id;
            let mut input = queued(&session_id, request_id);
            input.source = QueuedTurnMessageSource::Http;
            db.enqueue_turn_user_message(input).unwrap();
            replace_options_json(&db, &session_id, request_id, raw_options);

            let insertion_error = db
                .request_turn_message_insertion(&session_id, request_id, "active-turn")
                .unwrap_err();
            assert!(
                insertion_error
                    .to_string()
                    .contains("sidecar cannot be safely decoded"),
                "unexpected error for {request_id}: {insertion_error:#}"
            );
            let held = db
                .get_queued_turn_user_message(&session_id, request_id)
                .unwrap()
                .unwrap();
            assert_eq!(held.status, QueuedTurnMessageStatus::FallbackAfterReply);
            assert!(held.turn_id.is_none());

            let dispatch_error = db
                .claim_queued_turn_message_for_dispatch(&session_id, request_id, "next-turn")
                .unwrap_err();
            assert!(dispatch_error
                .to_string()
                .contains("sidecar cannot be safely decoded"));

            // Owner-managed rows remain repairable: editing the raw text
            // intentionally drops stale typed sidecars and restores ordinary
            // full-turn dispatch.
            assert!(db
                .update_queued_turn_user_message(&session_id, request_id, "repaired message", None,)
                .unwrap());
            let dispatched = db
                .claim_queued_turn_message_for_dispatch(&session_id, request_id, "repaired-turn")
                .unwrap()
                .unwrap();
            assert!(dispatched.incoming_turn.is_none());
            assert!(dispatched.skill_allowed_tools.is_empty());
        }
    }

    #[test]
    fn undecodable_channel_skill_ceiling_is_held_out_of_the_fifo_pump() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let request_id = "future-skill-command";
        db.enqueue_turn_user_message(channel_queued(&session_id, request_id))
            .unwrap();
        replace_options_json(
            &db,
            &session_id,
            request_id,
            r#"{"skillAllowedTools":{"version":2,"tools":["read"]}}"#,
        );

        let error = db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "next-turn")
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("skillAllowedTools must be an array containing only strings"));
        let held = db
            .get_queued_turn_user_message(&session_id, request_id)
            .unwrap()
            .unwrap();
        assert_eq!(held.status, QueuedTurnMessageStatus::HeldAfterStop);
        assert!(held.turn_id.is_none());
        assert!(!db
            .list_channel_queued_session_ids()
            .unwrap()
            .contains(&session_id));
        assert!(db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "retry-turn")
            .unwrap()
            .is_none());

        // A later inbound message may resume held Channel rows. The same
        // binary must detect and hold the unsupported ceiling again instead of
        // dispatching it as an unrestricted command.
        assert_eq!(
            db.resume_channel_turn_messages_after_stop(&session_id)
                .unwrap(),
            1
        );
        let retry_error = db
            .request_channel_turn_message_insertion(&session_id, request_id, "active-turn")
            .unwrap_err();
        assert!(retry_error
            .to_string()
            .contains("skillAllowedTools must be an array containing only strings"));
        assert_eq!(
            db.get_queued_turn_user_message(&session_id, request_id)
                .unwrap()
                .unwrap()
                .status,
            QueuedTurnMessageStatus::HeldAfterStop
        );
    }

    #[test]
    fn channel_sidecar_remains_fifo_head_for_after_reply_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let mut first = channel_queued(&session_id, "skill-command");
        first.skill_allowed_tools = vec!["read".into(), "glob".into()];
        db.enqueue_turn_user_message(first).unwrap();
        db.enqueue_turn_user_message(channel_queued(&session_id, "plain-second"))
            .unwrap();

        assert!(!db
            .request_channel_turn_message_insertion(&session_id, "skill-command", "active-turn")
            .unwrap());
        assert!(!db
            .request_channel_turn_message_insertion(&session_id, "plain-second", "active-turn")
            .unwrap());
        assert!(db
            .claim_turn_messages_for_insertion(&session_id, "active-turn")
            .unwrap()
            .is_empty());

        let row = db
            .get_queued_turn_user_message(&session_id, "skill-command")
            .unwrap()
            .unwrap();
        assert_eq!(row.status, QueuedTurnMessageStatus::FallbackAfterReply);
        assert_eq!(
            db.next_channel_turn_message_for_insertion(&session_id)
                .unwrap()
                .as_deref(),
            Some("skill-command")
        );
        let dispatched = db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "next-turn")
            .unwrap()
            .unwrap();
        assert_eq!(dispatched.request_id, "skill-command");
        assert_eq!(dispatched.skill_allowed_tools, vec!["read", "glob"]);
    }

    #[test]
    fn insertion_claim_defers_preexisting_waiting_sidecar_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        let mut input = queued(&session_id, "legacy-waiting");
        input.message = "@README.md".into();
        input.incoming_turn = Some(incoming_turn_with_file_mention(&input.message, "README.md"));
        input.attachments = vec![queued_file_attachment("README.md")];
        db.enqueue_turn_user_message(input).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE queued_turn_user_messages
                 SET mode = 'force_insert', status = 'waiting_tool_boundary', turn_id = ?1
                 WHERE session_id = ?2 AND request_id = ?3",
                params!["active-turn", session_id, "legacy-waiting"],
            )
            .unwrap();

        assert!(db
            .claim_turn_messages_for_insertion(&session_id, "active-turn")
            .unwrap()
            .is_empty());
        let row = db
            .get_queued_turn_user_message(&session_id, "legacy-waiting")
            .unwrap()
            .unwrap();
        assert_eq!(row.mode, QueuedTurnMessageMode::Queue);
        assert_eq!(row.status, QueuedTurnMessageStatus::FallbackAfterReply);
        assert!(row.turn_id.is_none());
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

    #[test]
    fn channel_claim_is_fifo_and_is_not_owned_by_gui_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(channel_queued(&session_id, "channel-first"))
            .unwrap();
        db.enqueue_turn_user_message(channel_queued(&session_id, "channel-second"))
            .unwrap();

        assert!(db.has_channel_turn_messages(&session_id).unwrap());

        assert!(db
            .claim_queued_turn_message_for_dispatch(&session_id, "channel-first", "gui-turn")
            .unwrap()
            .is_none());
        let first = db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "channel-turn-a")
            .unwrap()
            .unwrap();
        assert_eq!(first.request_id, "channel-first");
        assert!(db
            .channel_dispatch_claim_is_active(&session_id, "channel-first", "channel-turn-a")
            .unwrap());
        assert_eq!(
            QueuedTurnMessageView::from(&first).managed_by,
            Some("channel")
        );

        // The next row is not claimable until the first dispatch settles.
        assert!(db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "channel-turn-b")
            .unwrap()
            .is_none());
        db.remove_claimed_turn_message(&session_id, "channel-first")
            .unwrap();
        assert!(!db
            .channel_dispatch_claim_is_active(&session_id, "channel-first", "channel-turn-a")
            .unwrap());
        let second = db
            .claim_next_channel_turn_message_for_dispatch(&session_id, "channel-turn-b")
            .unwrap()
            .unwrap();
        assert_eq!(second.request_id, "channel-second");
        db.remove_claimed_turn_message(&session_id, "channel-second")
            .unwrap();
        assert!(!db.has_channel_turn_messages(&session_id).unwrap());
    }

    #[test]
    fn client_mutations_cannot_take_over_a_channel_managed_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(channel_queued(&session_id, "channel-row"))
            .unwrap();

        assert!(!db
            .update_queued_turn_user_message(
                &session_id,
                "channel-row",
                "tampered",
                Some("tampered")
            )
            .unwrap());
        assert!(!db
            .delete_queued_turn_user_message(&session_id, "channel-row")
            .unwrap());
        assert!(!db
            .request_turn_message_insertion(&session_id, "channel-row", "desktop-turn")
            .unwrap());
        assert!(db
            .request_channel_turn_message_insertion(&session_id, "channel-row", "channel-turn")
            .unwrap());
        assert!(!db
            .cancel_turn_message_insertion(&session_id, "channel-row", "channel-turn")
            .unwrap());
        let row = db
            .get_queued_turn_user_message(&session_id, "channel-row")
            .unwrap()
            .unwrap();
        assert_eq!(row.message, "message-channel-row");
        assert_eq!(row.status, QueuedTurnMessageStatus::WaitingToolBoundary);
    }

    #[test]
    fn channel_insertion_arms_only_the_fifo_head() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let session_id = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(channel_queued(&session_id, "first"))
            .unwrap();
        db.enqueue_turn_user_message(channel_queued(&session_id, "second"))
            .unwrap();

        assert!(db
            .request_channel_turn_message_insertion(&session_id, "first", "active-turn")
            .unwrap());
        assert!(!db
            .request_channel_turn_message_insertion(&session_id, "second", "active-turn")
            .unwrap());
        let claimed = db
            .claim_turn_messages_for_insertion(&session_id, "active-turn")
            .unwrap();
        assert_eq!(claimed.len(), 1);
        db.complete_inserted_turn_message(&claimed[0], &super::super::NewMessage::user("first"))
            .unwrap();

        let next = db
            .next_channel_turn_message_for_insertion(&session_id)
            .unwrap();
        assert_eq!(next.as_deref(), Some("second"));
        assert!(db
            .request_channel_turn_message_insertion(&session_id, "second", "active-turn")
            .unwrap());
    }

    #[test]
    fn stop_hold_survives_reopen_and_resumes_in_original_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let session_id = {
            let db = SessionDB::open(&path).unwrap();
            let session_id = db.create_session("ha-main").unwrap().id;
            db.enqueue_turn_user_message(channel_queued(&session_id, "first"))
                .unwrap();
            db.enqueue_turn_user_message(channel_queued(&session_id, "second"))
                .unwrap();
            assert!(db
                .request_channel_turn_message_insertion(&session_id, "first", "active-turn")
                .unwrap());
            assert_eq!(
                db.hold_channel_turn_messages_after_stop(&session_id)
                    .unwrap(),
                2
            );
            session_id
        };

        let reopened = SessionDB::open(&path).unwrap();
        let held = reopened
            .list_queued_turn_user_messages(&session_id)
            .unwrap();
        assert_eq!(held.len(), 2);
        assert!(held
            .iter()
            .all(|item| item.status == QueuedTurnMessageStatus::HeldAfterStop));
        assert!(reopened
            .claim_next_channel_turn_message_for_dispatch(&session_id, "too-early")
            .unwrap()
            .is_none());

        assert_eq!(
            reopened
                .resume_channel_turn_messages_after_stop(&session_id)
                .unwrap(),
            2
        );
        let first = reopened
            .claim_next_channel_turn_message_for_dispatch(&session_id, "resumed-turn")
            .unwrap()
            .unwrap();
        assert_eq!(first.request_id, "first");
    }

    #[test]
    fn global_stop_holds_queue_only_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&dir.path().join("sessions.db")).unwrap();
        let first_session = db.create_session("ha-main").unwrap().id;
        let second_session = db.create_session("ha-main").unwrap().id;
        db.enqueue_turn_user_message(channel_queued(&first_session, "first"))
            .unwrap();
        db.enqueue_turn_user_message(channel_queued(&second_session, "second"))
            .unwrap();

        let held_sessions = db.hold_all_channel_turn_messages_after_stop().unwrap();
        let mut expected_sessions = vec![first_session, second_session];
        expected_sessions.sort();
        assert_eq!(held_sessions, expected_sessions);
        assert!(db.list_channel_queued_session_ids().unwrap().is_empty());
    }
}
