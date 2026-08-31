use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::SessionDB;
use super::types::NewMessage;

/// Shared terminal read boundary for Pet activity and embedded-chat badges.
/// The callers bind the current turn as `t` and its owning session as `s`.
/// Failure/Stop finalization appends a visible notice after any partial reply;
/// reading that partial reply must not acknowledge the later terminal notice.
pub(super) const TERMINAL_MESSAGE_ID_SQL: &str =
    "COALESCE(t.terminal_message_id, t.assistant_message_id, t.user_message_id)";

/// Generate the opaque durable identity shared by chat entry points.
pub fn new_chat_turn_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(super) fn ensure_no_competing_durable_chat_work(
    conn: &rusqlite::Connection,
    session_id: &str,
    admitted_turn_id: Option<&str>,
) -> Result<()> {
    let active_turn: Option<(String, String)> = conn
        .query_row(
            "SELECT id, source FROM chat_turns
              WHERE session_id = ?1 AND status IN ('running', 'cancelling')
                AND (?2 IS NULL OR id <> ?2)
              ORDER BY started_at ASC, id ASC
              LIMIT 1",
            params![session_id, admitted_turn_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((active_turn_id, active_source)) = active_turn {
        anyhow::bail!(
            "Session '{}' already has an active {} turn ({})",
            session_id,
            active_source,
            active_turn_id
        );
    }
    let active_stream: Option<(String, String)> = conn
        .query_row(
            "SELECT run_id, source FROM chat_stream_runs
              WHERE session_id = ?1 AND status = 'running'
              ORDER BY started_at ASC, run_id ASC
              LIMIT 1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((active_run_id, active_source)) = active_stream {
        anyhow::bail!(
            "Session '{}' already has an active {} stream ({})",
            session_id,
            active_source,
            active_run_id
        );
    }
    Ok(())
}

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
    /// Configuration error — no usable auth profile (all disabled, in
    /// cooldown, or unconfigured). Zero LLM API calls were attempted.
    NoProfile,
    /// All `model_chain` attempts failed at the provider layer.
    /// `chat_turns.error` carries the raw last-attempt message.
    ProviderFailed,
    /// The current tool-result group's cheapest protocol-legal envelope still
    /// exceeded capacity after the bounded recovery ladder. This terminal
    /// application verdict must survive independently of display/error text.
    CurrentToolGroupOverflow,
    /// The exact Provider request crossed the dispatch claim, but no response
    /// proof was observed. This verdict must remain typed across reconnect and
    /// crash recovery so no caller silently turns it into an automatic retry.
    DispatchUnknown,
    /// Emergency context compaction ran but the history still exceeds
    /// the hard threshold; the turn cannot continue.
    CompactionFailed,
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
            Self::NoProfile => "no_profile",
            Self::ProviderFailed => "provider_failed",
            Self::CurrentToolGroupOverflow => "current_tool_group_overflow",
            Self::DispatchUnknown => "dispatch_unknown",
            Self::CompactionFailed => "compaction_failed",
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
            "no_profile" => Some(Self::NoProfile),
            "provider_failed" => Some(Self::ProviderFailed),
            "current_tool_group_overflow" => Some(Self::CurrentToolGroupOverflow),
            "dispatch_unknown" => Some(Self::DispatchUnknown),
            "compaction_failed" => Some(Self::CompactionFailed),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_surface: Option<crate::pet::ChatUiSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiChatDispatchRecord {
    pub request_fingerprint: String,
    pub session_id: String,
    pub turn_id: String,
    pub queue_request_id: Option<String>,
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
                terminal_message_id INTEGER,
                error TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                updated_at TEXT NOT NULL,
                ui_surface TEXT,
                client_request_id TEXT,
                request_fingerprint TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chat_turns_session_started
                ON chat_turns(session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_session_status
                ON chat_turns(session_id, status);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_stream_id
                ON chat_turns(stream_id);",
        )?;
        if conn
            .prepare("SELECT terminal_message_id FROM chat_turns LIMIT 1")
            .is_err()
        {
            // Freeze legacy boundaries once. The end timestamp and next user
            // exclude later control messages; never recompute this on reads.
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(
                "ALTER TABLE chat_turns ADD COLUMN terminal_message_id INTEGER;
                 UPDATE chat_turns AS t SET terminal_message_id = COALESCE(
                     CASE WHEN t.status IN ('failed', 'interrupted') THEN (
                         SELECT MAX(m.id) FROM messages m
                          WHERE m.session_id = t.session_id
                            AND m.id > COALESCE(t.user_message_id, 0)
                            AND m.timestamp <= t.ended_at
                            AND m.id < COALESCE((
                                SELECT MIN(t2.user_message_id) FROM chat_turns t2
                                 WHERE t2.session_id = t.session_id
                                   AND t2.user_message_id > t.user_message_id
                            ), 9223372036854775807)
                     ) END, t.assistant_message_id, t.user_message_id
                 ) WHERE t.status IN ('completed', 'failed', 'interrupted');",
            )?;
            tx.commit()?;
        }
        if conn
            .prepare("SELECT ui_surface FROM chat_turns LIMIT 1")
            .is_err()
        {
            conn.execute_batch("ALTER TABLE chat_turns ADD COLUMN ui_surface TEXT;")?;
        }
        if conn
            .prepare("SELECT client_request_id FROM chat_turns LIMIT 1")
            .is_err()
        {
            conn.execute_batch("ALTER TABLE chat_turns ADD COLUMN client_request_id TEXT;")?;
        }
        if conn
            .prepare("SELECT request_fingerprint FROM chat_turns LIMIT 1")
            .is_err()
        {
            conn.execute_batch("ALTER TABLE chat_turns ADD COLUMN request_fingerprint TEXT;")?;
        }
        // Drop the pre-release draft name once, then keep the final index
        // stable. Rebuilding a potentially large turn index on every startup
        // would make Pet's additive migration unnecessarily expensive.
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_chat_turns_ui_surface_session_started;
             CREATE INDEX IF NOT EXISTS idx_chat_turns_session_surface_started
                 ON chat_turns(session_id, ui_surface, started_at DESC);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_turns_client_request_id
                 ON chat_turns(client_request_id) WHERE client_request_id IS NOT NULL;",
        )?;
        Ok(())
    }

    /// Project the final, backend-verified typed mention receipt onto the
    /// durable user message owned by this turn. Joining through `chat_turns`
    /// prevents a caller from attaching provenance to an unrelated message;
    /// incognito sessions intentionally receive no history sidecar.
    #[doc(hidden)]
    pub fn merge_chat_turn_typed_mention_receipt(
        &self,
        session_id: &str,
        turn_id: &str,
        projection: &crate::prompt_context::TypedMentionReceiptProjection,
    ) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let target = tx
            .query_row(
                "SELECT m.id, m.content, m.attachments_meta
                   FROM chat_turns ct
                   JOIN sessions s ON s.id = ct.session_id
                   JOIN messages m
                     ON m.id = ct.user_message_id AND m.session_id = ct.session_id
                  WHERE ct.id = ?1 AND ct.session_id = ?2
                    AND s.incognito = 0 AND m.role = 'user'",
                params![turn_id, session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((message_id, persisted_content, existing)) = target else {
            tx.commit()?;
            return Ok(false);
        };
        if !crate::prompt_context::typed_mention_receipt_projection_matches_message(
            &persisted_content,
            projection,
        ) {
            tx.commit()?;
            return Ok(false);
        }
        let merged =
            super::types::merge_typed_mention_receipt_attachments_meta(projection, existing);
        let updated = tx.execute(
            "UPDATE messages SET attachments_meta = ?1
              WHERE id = ?2 AND session_id = ?3 AND role = 'user'",
            params![merged, message_id, session_id],
        )?;
        if updated != 1 {
            anyhow::bail!("typed mention receipt user message changed during projection");
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn create_chat_turn(
        &self,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        user_message_id: Option<i64>,
    ) -> Result<ChatTurn> {
        let id = new_chat_turn_id();
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
        self.create_chat_turn_with_id_surface(
            id,
            session_id,
            source,
            stream_id,
            user_message_id,
            None,
        )
    }

    pub fn create_chat_turn_with_id_surface(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        user_message_id: Option<i64>,
        ui_surface: Option<crate::pet::ChatUiSurface>,
    ) -> Result<ChatTurn> {
        self.create_chat_turn_with_id_surface_dispatch(
            id,
            session_id,
            source,
            stream_id,
            user_message_id,
            ui_surface,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_chat_turn_with_id_surface_dispatch(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        user_message_id: Option<i64>,
        ui_surface: Option<crate::pet::ChatUiSurface>,
        client_request_id: Option<&str>,
        request_fingerprint: Option<&str>,
    ) -> Result<ChatTurn> {
        if client_request_id.is_some() != request_fingerprint.is_some() {
            anyhow::bail!(
                "UI chat dispatch identity requires both client_request_id and request_fingerprint"
            );
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_no_competing_durable_chat_work(&tx, session_id, None)?;
        let now = chrono::Utc::now().to_rfc3339();
        // A non-UI turn can replace a currently projected UI turn in the same
        // session. Invalidate only for that transition (or for a new UI turn)
        // so background-only sessions do not create a pet refresh storm.
        let pet_relevant = ui_surface.is_some()
            || tx
                .query_row(
                    "SELECT ui_surface IS NOT NULL
                       FROM chat_turns
                      WHERE session_id = ?1
                      ORDER BY started_at DESC, id DESC
                      LIMIT 1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false);
        tx.execute(
            "INSERT INTO chat_turns (
                id, session_id, source, status, interrupt_reason, stream_id,
                user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                ui_surface, client_request_id, request_fingerprint
             ) VALUES (?1, ?2, ?3, 'running', NULL, ?4, ?5, NULL, NULL, ?6, NULL, ?6, ?7, ?8, ?9)",
            params![
                id,
                session_id,
                source,
                stream_id,
                user_message_id,
                now,
                ui_surface.map(crate::pet::ChatUiSurface::as_str),
                client_request_id,
                request_fingerprint,
            ],
        )?;
        let turn = ChatTurn {
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
            ui_surface,
        };
        tx.commit()?;
        drop(conn);
        if pet_relevant {
            crate::pet::emit_activity_changed();
        }
        Ok(turn)
    }

    /// Atomically persist an inbound message and its running chat turn. The UI
    /// dispatch id lives on the turn in the same transaction, so a lost ACK or
    /// process restart can never leave a replayable message without its
    /// idempotency record.
    #[allow(clippy::too_many_arguments)]
    pub fn append_message_and_create_chat_turn_with_id_surface_dispatch(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        message: &NewMessage,
        ui_surface: Option<crate::pet::ChatUiSurface>,
        client_request_id: Option<&str>,
        request_fingerprint: Option<&str>,
        stop_admission: Option<super::ForegroundStopAdmission>,
    ) -> Result<(i64, ChatTurn)> {
        self.append_message_and_create_chat_turn_with_id_surface_dispatch_inner(
            id,
            session_id,
            source,
            stream_id,
            message,
            ui_surface,
            client_request_id,
            request_fingerprint,
            stop_admission,
            None,
            None,
        )
        .map(|(message_id, turn, _)| (message_id, turn))
    }

    /// Atomically persist an interactive user message, its visible chat turn,
    /// and the durability stream run that owns Stop epochs and recovery.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_interactive_chat_turn(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: &str,
        message: &NewMessage,
        ui_surface: Option<crate::pet::ChatUiSurface>,
        client_request_id: Option<&str>,
        request_fingerprint: Option<&str>,
        stop_admission: Option<super::ForegroundStopAdmission>,
        stream_run: &super::CreateStreamRun,
    ) -> Result<(i64, ChatTurn, super::StreamRunRegistration)> {
        let (message_id, turn, registration) = self
            .append_message_and_create_chat_turn_with_id_surface_dispatch_inner(
                id,
                session_id,
                source,
                Some(stream_id),
                message,
                ui_surface,
                client_request_id,
                request_fingerprint,
                stop_admission,
                Some(stream_run),
                None,
            )?;
        Ok((
            message_id,
            turn,
            registration.expect("interactive admission always creates a stream registration"),
        ))
    }

    /// Session-tool turns carry no fresh user intent, so their persisted Stop
    /// admission must be checked in the same write transaction as the message
    /// and turn. If this transaction wins, a later Stop enumerates the durable
    /// running turn; if Stop wins, the generation/pause checks fail closed.
    pub(crate) fn append_message_and_create_session_tool_turn_with_id(
        &self,
        id: &str,
        session_id: &str,
        source_session_id: Option<&str>,
        message: &NewMessage,
        expected_global_stop_epoch: u64,
    ) -> Result<(i64, ChatTurn)> {
        self.append_message_and_create_chat_turn_with_id_surface_dispatch_inner(
            id,
            session_id,
            crate::chat_engine::ChatSource::SessionTool.as_str(),
            None,
            message,
            None,
            None,
            None,
            None,
            None,
            Some((expected_global_stop_epoch, source_session_id)),
        )
        .map(|(message_id, turn, _)| (message_id, turn))
    }

    #[allow(clippy::too_many_arguments)]
    fn append_message_and_create_chat_turn_with_id_surface_dispatch_inner(
        &self,
        id: &str,
        session_id: &str,
        source: &str,
        stream_id: Option<&str>,
        message: &NewMessage,
        ui_surface: Option<crate::pet::ChatUiSurface>,
        client_request_id: Option<&str>,
        request_fingerprint: Option<&str>,
        stop_admission: Option<super::ForegroundStopAdmission>,
        stream_run: Option<&super::CreateStreamRun>,
        session_tool_admission: Option<(u64, Option<&str>)>,
    ) -> Result<(i64, ChatTurn, Option<super::StreamRunRegistration>)> {
        if client_request_id.is_some() != request_fingerprint.is_some() {
            anyhow::bail!(
                "UI chat dispatch identity requires both client_request_id and request_fingerprint"
            );
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        // Every foreground admission takes the SQLite write lock before it
        // checks and inserts the durable running turn. This serializes regular
        // Desktop/HTTP turns with SessionTool turns across processes instead
        // of relying only on the process-local active-turn registry.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut session_tool_message = None;
        if let Some((expected_global_stop_epoch, source_session_id)) = session_tool_admission {
            let current_global_stop_epoch =
                super::autonomy_pause::global_stop_epoch_with_conn(&tx)?;
            if current_global_stop_epoch != expected_global_stop_epoch {
                anyhow::bail!("Global Stop began before the cross-session turn was persisted");
            }
            if let Some(source_session_id) = source_session_id {
                let (source_incognito, source_title, source_side_parent) = tx
                    .query_row(
                        "SELECT incognito, title,
                                CASE WHEN kind = 'side' THEN forked_from_session_id END
                           FROM sessions WHERE id = ?1",
                        params![source_session_id],
                        |row| {
                            Ok((
                                row.get::<_, bool>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )
                    .optional()?
                    .ok_or_else(|| {
                        anyhow::anyhow!("Source session '{}' no longer exists", source_session_id)
                    })?;
                if source_incognito {
                    anyhow::bail!("Refusing cross-session messaging from an incognito session");
                }
                // Persist provenance from the live source in the same admission
                // transaction. Neither the prompt nor caller-supplied metadata
                // may impersonate another conversation.
                let mut enriched = message.clone();
                enriched.attachments_meta = Some(super::types::merge_user_message_meta(
                    serde_json::json!({
                        "session_message": {
                            "sessionId": source_session_id,
                            "title": source_title,
                            "sideParentSessionId": source_side_parent,
                        }
                    }),
                    enriched.attachments_meta,
                ));
                session_tool_message = Some(enriched);
            }
            let target_incognito = tx
                .query_row(
                    "SELECT incognito FROM sessions WHERE id = ?1",
                    params![session_id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .ok_or_else(|| {
                    anyhow::anyhow!("Target session '{}' no longer exists", session_id)
                })?;
            if target_incognito {
                anyhow::bail!("Refusing to send to incognito session '{}'", session_id);
            }
            let paused = tx.query_row(
                super::autonomy_pause::SESSION_LINEAGE_PAUSE_EXISTS_SQL,
                params![session_id],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if paused {
                anyhow::bail!(
                    "Target session '{}' is paused; use Continue before delegating another turn",
                    session_id
                );
            }
        }
        let message = session_tool_message.as_ref().unwrap_or(message);
        ensure_no_competing_durable_chat_work(&tx, session_id, None)?;
        if let Some(admission) = stop_admission {
            if !super::autonomy_pause::foreground_stop_admission_is_current_with_conn(
                &tx, session_id, admission,
            )? {
                anyhow::bail!("{}", super::FOREGROUND_STOP_FENCE_ERROR);
            }
        }
        if message.queue_request_id.is_none() {
            let consumed =
                super::turn_queue::consume_direct_turn_admission(&tx, session_id, id, source)?;
            if matches!(source, "desktop" | "http") && !consumed {
                anyhow::bail!("direct turn lost its durable admission");
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let timestamp = if message.timestamp.is_empty() {
            now.as_str()
        } else {
            message.timestamp.as_str()
        };
        let pet_relevant = ui_surface.is_some()
            || tx
                .query_row(
                    "SELECT ui_surface IS NOT NULL
                       FROM chat_turns
                      WHERE session_id = ?1
                      ORDER BY started_at DESC, id DESC
                      LIMIT 1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false);

        tx.execute(
            "INSERT INTO messages (session_id, role, content, timestamp,
                attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                tool_call_id, tool_name, tool_arguments, tool_result,
                tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status, source,
                queue_request_id, persistence_run_id, logical_block_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            params![
                session_id,
                message.role.as_str(),
                message.content,
                timestamp,
                message.attachments_meta,
                message.model,
                message.tokens_in,
                message.tokens_out,
                message.reasoning_effort,
                message.tool_call_id,
                message.tool_name,
                message.tool_arguments,
                message.tool_result,
                message.tool_duration_ms,
                message.is_error.map(|value| if value { 1i64 } else { 0i64 }),
                message.thinking,
                message.ttft_ms,
                message.tokens_in_last,
                message.tokens_cache_creation,
                message.tokens_cache_read,
                message.tool_metadata,
                message.stream_status,
                message.source,
                message.queue_request_id,
                message.persistence_run_id,
                message.logical_block_seq,
            ],
        )?;
        let message_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.execute(
            "INSERT INTO chat_turns (
                id, session_id, source, status, interrupt_reason, stream_id,
                user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                ui_surface, client_request_id, request_fingerprint
             ) VALUES (?1, ?2, ?3, 'running', NULL, ?4, ?5, NULL, NULL, ?6, NULL, ?6, ?7, ?8, ?9)",
            params![
                id,
                session_id,
                source,
                stream_id,
                message_id,
                now,
                ui_surface.map(crate::pet::ChatUiSurface::as_str),
                client_request_id,
                request_fingerprint,
            ],
        )?;
        if let Some(request_id) = message.queue_request_id.as_deref() {
            let consumed = tx.execute(
                "DELETE FROM queued_turn_user_messages
                  WHERE session_id = ?1 AND request_id = ?2
                    AND turn_id = ?3 AND status = 'dispatching'",
                params![session_id, request_id, id],
            )?;
            if consumed != 1 {
                anyhow::bail!("queued message dispatch was not owned by this turn");
            }
        }
        let stream_registration = stream_run
            .map(|stream_run| {
                debug_assert_eq!(stream_run.session_id, session_id);
                debug_assert_eq!(stream_run.turn_id.as_deref(), Some(id));
                debug_assert_eq!(stream_run.stream_id.as_deref(), stream_id);
                SessionDB::create_stream_run_in_transaction(&tx, stream_run, stop_admission)
            })
            .transpose()?;
        tx.commit()?;
        drop(conn);

        self.mirror_persisted_message_for_hooks(session_id, message_id, message, timestamp);
        if let Some(request_id) = message.queue_request_id.as_deref() {
            super::turn_queue::emit_changed(session_id, Some(request_id), "dispatched");
        }
        if pet_relevant {
            crate::pet::emit_activity_changed();
        }
        Ok((
            message_id,
            ChatTurn {
                id: id.to_string(),
                session_id: session_id.to_string(),
                source: source.to_string(),
                status: ChatTurnStatus::Running,
                interrupt_reason: None,
                stream_id: stream_id.map(ToOwned::to_owned),
                user_message_id: Some(message_id),
                assistant_message_id: None,
                error: None,
                started_at: now.clone(),
                ended_at: None,
                updated_at: now,
                ui_surface,
            },
            stream_registration,
        ))
    }

    pub fn get_ui_chat_dispatch(
        &self,
        client_request_id: &str,
    ) -> Result<Option<UiChatDispatchRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT ct.request_fingerprint, ct.session_id, ct.id, m.queue_request_id
               FROM chat_turns ct
               LEFT JOIN messages m ON m.id = ct.user_message_id
              WHERE ct.client_request_id = ?1",
            params![client_request_id],
            |row| {
                Ok(UiChatDispatchRecord {
                    request_fingerprint: row.get(0)?,
                    session_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    queue_request_id: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_chat_turn(&self, turn_id: &str) -> Result<Option<ChatTurn>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                    ui_surface
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
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                    ui_surface
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

    /// Whether this exact terminal turn's result crossed the durable read
    /// watermark. Active/missing turns return None, never an inferred receipt.
    pub fn chat_turn_terminal_read(&self, session_id: &str, turn_id: &str) -> Result<Option<bool>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let sql = format!(
            "SELECT CASE WHEN t.status IN ('completed', 'interrupted', 'failed')
                THEN COALESCE(({TERMINAL_MESSAGE_ID_SQL}) <= COALESCE(s.last_read_message_id, 0), 0)
                ELSE NULL END
             FROM chat_turns t JOIN sessions s ON s.id = t.session_id
             WHERE s.id = ?1 AND t.id = ?2"
        );
        Ok(conn
            .query_row(&sql, params![session_id, turn_id], |row| {
                row.get::<_, Option<bool>>(0)
            })
            .optional()?
            .flatten())
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
        let changed = n > 0;
        let pet_relevant = if changed {
            conn.query_row(
                "SELECT ui_surface IS NOT NULL FROM chat_turns WHERE id = ?1",
                params![turn_id],
                |row| row.get(0),
            )?
        } else {
            false
        };
        drop(conn);
        if pet_relevant {
            crate::pet::emit_activity_changed();
        }
        Ok(changed)
    }

    pub fn finish_chat_turn_once(
        &self,
        turn_id: &str,
        status: ChatTurnStatus,
        interrupt_reason: Option<ChatTurnInterruptReason>,
        error: Option<&str>,
        assistant_message_id: Option<i64>,
    ) -> Result<bool> {
        self.finish_chat_turn_once_with_notice(
            turn_id,
            status,
            interrupt_reason,
            error,
            assistant_message_id,
            None,
        )
    }

    /// Seal the exact visible finalization notice together with terminal state.
    /// Legacy callers without a notice capture the current boundary once.
    pub(crate) fn finish_chat_turn_once_with_notice(
        &self,
        turn_id: &str,
        status: ChatTurnStatus,
        interrupt_reason: Option<ChatTurnInterruptReason>,
        error: Option<&str>,
        assistant_message_id: Option<i64>,
        terminal_notice_id: Option<i64>,
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
                 terminal_message_id = COALESCE(?7,
                     CASE WHEN ?1 IN ('failed', 'interrupted') THEN (
                         SELECT MAX(m.id) FROM messages m
                          WHERE m.session_id = chat_turns.session_id
                            AND m.id >= COALESCE(chat_turns.user_message_id, 0)
                     ) END, ?4, assistant_message_id, user_message_id),
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
                terminal_notice_id,
            ],
        )?;
        let changed = n > 0;
        let pet_relevant = if changed {
            conn.query_row(
                "SELECT ui_surface IS NOT NULL FROM chat_turns WHERE id = ?1",
                params![turn_id],
                |row| row.get(0),
            )?
        } else {
            false
        };
        drop(conn);
        if pet_relevant {
            crate::pet::emit_activity_changed();
        }
        Ok(changed)
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
                        user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                        ui_surface
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
        let final_reason = interrupted.then_some(
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
                 terminal_message_id = COALESCE(
                     CASE WHEN ?1 IN ('failed', 'interrupted') THEN (
                         SELECT MAX(m.id) FROM messages m
                          WHERE m.session_id = chat_turns.session_id
                            AND m.id >= COALESCE(chat_turns.user_message_id, 0)
                     ) END, ?4, assistant_message_id, user_message_id),
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

        let result = conn
            .query_row(
                "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error, started_at, ended_at, updated_at,
                    ui_surface
             FROM chat_turns WHERE id = ?1",
                params![turn_id],
                Self::row_to_chat_turn,
            )
            .optional()?;
        let pet_relevant = current.ui_surface.is_some();
        drop(conn);
        if pet_relevant {
            crate::pet::emit_activity_changed();
        }
        Ok(result)
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
                 terminal_message_id = COALESCE(terminal_message_id,
                     (SELECT MAX(m.id) FROM messages m WHERE m.session_id = chat_turns.session_id),
                     assistant_message_id, user_message_id),
                 ended_at = COALESCE(ended_at, ?1),
                 updated_at = ?1
             WHERE status IN ('running', 'cancelling')",
            params![now],
        )?;
        Ok(n)
    }

    /// Read-only counterpart of [`recover_stale_chat_turns`] for the
    /// finalize sweep path. Returns every turn left in `running` or
    /// `cancelling` state without mutating the row — the unified
    /// finalize entry point will write the final status and the right
    /// `interrupt_reason` based on whether the previous exit was
    /// clean (Shutdown) or not (Crash).
    pub fn find_stale_chat_turns_for_finalize(&self) -> Result<Vec<ChatTurn>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, source, status, interrupt_reason, stream_id,
                    user_message_id, assistant_message_id, error,
                    started_at, ended_at, updated_at, ui_surface
             FROM chat_turns
             WHERE status IN ('running', 'cancelling')
             ORDER BY started_at ASC",
        )?;
        let rows = stmt.query_map([], Self::row_to_chat_turn)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
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
            ui_surface: row
                .get::<_, Option<String>>(12)?
                .as_deref()
                .and_then(crate::pet::ChatUiSurface::from_str),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_read_migration_freezes_legacy_notice_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        let session = db.create_session("ha-main").unwrap();
        let user = db
            .append_message(&session.id, &NewMessage::user("question"))
            .unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", None, Some(user))
            .unwrap();
        let notice = db
            .append_message(&session.id, &NewMessage::error_event("failure"))
            .unwrap();
        db.finish_chat_turn_once_with_notice(
            &turn.id,
            ChatTurnStatus::Failed,
            None,
            Some("failure"),
            None,
            Some(notice),
        )
        .unwrap();
        db.mark_session_read_through(&session.id, Some(notice))
            .unwrap();
        let command = db
            .append_message(&session.id, &NewMessage::event("/usage"))
            .unwrap();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE messages SET timestamp = '2026-01-01T00:00:00Z' WHERE id = ?1",
                [notice],
            )?;
            conn.execute(
                "UPDATE chat_turns SET ended_at = '2026-01-01T00:00:01Z' WHERE id = ?1",
                [&turn.id],
            )?;
            conn.execute(
                "UPDATE messages SET timestamp = '2026-01-01T00:00:02Z' WHERE id = ?1",
                [command],
            )?;
            conn.execute_batch("ALTER TABLE chat_turns DROP COLUMN terminal_message_id;")?;
            Ok(())
        })
        .unwrap();
        drop(db);
        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &turn.id).unwrap(),
            Some(true)
        );
        // A clock change or later command cannot affect the migrated value.
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE messages SET timestamp = '2025-01-01T00:00:00Z' WHERE id = ?1",
                [command],
            )?;
            Ok(())
        })
        .unwrap();
        drop(db);
        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &turn.id).unwrap(),
            Some(true)
        );
    }

    #[test]
    fn terminal_read_boundary_survives_reopen_without_acknowledging_later_turns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        let session = db.create_session("ha-main").unwrap();
        let user_id = db
            .append_message(&session.id, &NewMessage::user("question"))
            .unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", None, Some(user_id))
            .unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &turn.id).unwrap(),
            None
        );
        let answer_id = db
            .append_message(&session.id, &NewMessage::assistant("answer"))
            .unwrap();
        db.finish_chat_turn_once(
            &turn.id,
            ChatTurnStatus::Completed,
            None,
            None,
            Some(answer_id),
        )
        .unwrap();
        db.mark_session_read_through(&session.id, Some(user_id))
            .unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &turn.id).unwrap(),
            Some(false)
        );
        db.mark_session_read_through(&session.id, Some(answer_id))
            .unwrap();
        drop(db);

        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &turn.id).unwrap(),
            Some(true)
        );
        let next_user = db
            .append_message(&session.id, &NewMessage::user("next"))
            .unwrap();
        let failed = db
            .create_chat_turn(&session.id, "desktop", None, Some(next_user))
            .unwrap();
        db.mark_session_read_through(&session.id, Some(next_user))
            .unwrap();
        let error_id = db
            .append_message(&session.id, &NewMessage::assistant("provider error"))
            .unwrap();
        db.finish_chat_turn_once(
            &failed.id,
            ChatTurnStatus::Failed,
            None,
            Some("error"),
            None,
        )
        .unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &failed.id).unwrap(),
            Some(false)
        );
        db.mark_session_read_through(&session.id, Some(error_id))
            .unwrap();
        assert_eq!(
            db.chat_turn_terminal_read(&session.id, &failed.id).unwrap(),
            Some(true)
        );
        assert_eq!(
            db.chat_turn_terminal_read("other-session", &turn.id)
                .unwrap(),
            None
        );
    }

    fn typed_mention_projection(
        canonical_message: &str,
    ) -> crate::prompt_context::TypedMentionReceiptProjection {
        crate::prompt_context::TypedMentionReceiptProjection {
            receipt_version: crate::prompt_context::TYPED_MENTION_RECEIPT_VERSION,
            source_journal_seq: 1,
            prompt_contract_version: crate::prompt_context::PROMPT_CONTRACT_VERSION,
            mention_wire_version: crate::prompt_context::MENTION_WIRE_VERSION,
            canonical_text_fingerprint: crate::prompt_context::canonical_text_fingerprint(
                canonical_message,
            ),
            context_fingerprint: "context".into(),
            mentions: vec![crate::prompt_context::TypedMentionSpanReceipt {
                mention_id: "file-1".into(),
                kind: crate::prompt_context::MentionKind::File,
                target_id: "README.md".into(),
                display_label: "README".into(),
                origin: crate::prompt_context::StructuredMentionOrigin::FirstPartyComposerGesture,
                status: crate::prompt_context::MentionResolutionStatus::Resolved,
                raw: "@README.md".into(),
                start_utf8: 0,
                end_utf8: 10,
            }],
        }
    }

    fn temp_db() -> SessionDB {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        // Leak tempdir for test lifetime so SQLite can keep the file open.
        std::mem::forget(dir);
        SessionDB::open_ephemeral_for_test(&path).unwrap()
    }

    /// A Desktop/HTTP turn only persists after it owns a durable admission —
    /// both shells reserve one before calling the append helper, and the append
    /// consumes it in the same transaction. Mirror that here so these tests run
    /// the same FIFO path production does instead of a shape that cannot occur.
    fn admit_direct_turn(
        db: &SessionDB,
        session_id: &str,
        turn_id: &str,
        source: super::super::QueuedTurnMessageSource,
    ) -> super::super::DirectTurnAdmission {
        db.reserve_direct_turn_admission(session_id, turn_id, source, None)
            .expect("reserve direct admission")
            .expect("direct admission granted")
    }

    #[test]
    fn terminal_status_is_written_once() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
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
    fn session_tool_turn_persists_live_message_source_and_preserves_attachments() {
        let db = temp_db();
        let source = db.create_session("ha-main").unwrap();
        db.update_session_title(&source.id, "来源对话").unwrap();
        let attachment = serde_json::json!({
            "name": "note.txt", "mimeType": "text/plain", "path": "/tmp/note.txt"
        });
        for attachments_meta in [
            serde_json::json!([attachment.clone()]),
            serde_json::json!({
                "session_message": { "sessionId": "forged", "title": "forged" },
                "user_attachments": [attachment.clone()],
                "queued_message": false,
            }),
        ] {
            let target = db.create_session("ha-main").unwrap();
            let mut message = NewMessage::user("来自另一个对话的正文");
            message.attachments_meta = Some(attachments_meta.to_string());
            let (message_id, turn) = db
                .append_message_and_create_session_tool_turn_with_id(
                    &format!("message-source-{}", target.id),
                    &target.id,
                    Some(&source.id),
                    &message,
                    db.global_stop_epoch().unwrap(),
                )
                .unwrap();
            let rows = db.load_session_messages(&target.id).unwrap();
            let persisted = &rows[0];
            let meta: serde_json::Value =
                serde_json::from_str(persisted.attachments_meta.as_deref().unwrap()).unwrap();
            assert_eq!(persisted.id, message_id);
            assert_eq!(turn.user_message_id, Some(message_id));
            assert_eq!(persisted.content, message.content);
            assert_eq!(meta["session_message"]["sessionId"], source.id);
            assert_eq!(meta["session_message"]["title"], "来源对话");
            assert_eq!(meta["user_attachments"][0], attachment);
        }
    }

    #[test]
    fn session_tool_turn_preserves_side_chat_source_navigation() {
        let db = temp_db();
        let parent = db.create_session("ha-main").unwrap();
        let side = db.create_session("ha-main").unwrap();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE sessions SET kind = 'side', forked_from_session_id = ?1 WHERE id = ?2",
                params![parent.id, side.id],
            )?;
            Ok(())
        })
        .unwrap();
        let target = db.create_session("ha-main").unwrap();
        db.append_message_and_create_session_tool_turn_with_id(
            "side-message-source",
            &target.id,
            Some(&side.id),
            &NewMessage::user("hello"),
            db.global_stop_epoch().unwrap(),
        )
        .unwrap();
        let rows = db.load_session_messages(&target.id).unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(rows[0].attachments_meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["session_message"]["sessionId"], side.id);
        assert_eq!(meta["session_message"]["sideParentSessionId"], parent.id);
    }

    #[test]
    fn session_tool_turn_rejects_a_global_stop_that_won_admission() {
        let db = temp_db();
        let session = db.create_session("ha-main").unwrap();
        let expected_global_stop_epoch = db.global_stop_epoch().unwrap();
        db.begin_global_stop_enumeration().unwrap();

        let error = db
            .append_message_and_create_session_tool_turn_with_id(
                "delegated-after-stop",
                &session.id,
                None,
                &NewMessage::user("hello"),
                expected_global_stop_epoch,
            )
            .expect_err("stale admission must fail closed");

        assert!(error.to_string().contains("Global Stop"));
        assert!(db.load_session_messages(&session.id).unwrap().is_empty());
        assert!(db.get_chat_turn("delegated-after-stop").unwrap().is_none());
    }

    #[test]
    fn session_tool_turn_rejects_an_active_pause_in_its_write_transaction() {
        let db = temp_db();
        let session = db.create_session("ha-main").unwrap();
        let expected_global_stop_epoch = db.global_stop_epoch().unwrap();
        db.prepare_session_autonomy_pause(&session.id).unwrap();

        let error = db
            .append_message_and_create_session_tool_turn_with_id(
                "delegated-while-paused",
                &session.id,
                None,
                &NewMessage::user("hello"),
                expected_global_stop_epoch,
            )
            .expect_err("active pause must fail closed");

        assert!(error.to_string().contains("use Continue"));
        assert!(db.load_session_messages(&session.id).unwrap().is_empty());
        assert!(db
            .get_chat_turn("delegated-while-paused")
            .unwrap()
            .is_none());
    }

    #[test]
    fn session_tool_turn_serializes_admission_across_database_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let first = std::sync::Arc::new(SessionDB::open(&path).unwrap());
        let session = first.create_session("ha-main").unwrap();
        let second = std::sync::Arc::new(SessionDB::open(&path).unwrap());
        let expected_global_stop_epoch = first.global_stop_epoch().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));

        let run = |db: std::sync::Arc<SessionDB>, turn_id: &'static str| {
            let session_id = session.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                db.append_message_and_create_session_tool_turn_with_id(
                    turn_id,
                    &session_id,
                    None,
                    &NewMessage::user(turn_id),
                    expected_global_stop_epoch,
                )
            })
        };
        let first_attempt = run(first.clone(), "delegated-a");
        let second_attempt = run(second, "delegated-b");
        barrier.wait();

        let results = [
            first_attempt.join().unwrap(),
            second_attempt.join().unwrap(),
        ];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let error = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one cross-process admission must lose");
        assert!(error.to_string().contains("already has an active"));
        assert_eq!(first.load_session_messages(&session.id).unwrap().len(), 1);
        assert_eq!(
            first
                .find_stale_chat_turns_for_finalize()
                .unwrap()
                .into_iter()
                .filter(|turn| turn.session_id == session.id)
                .count(),
            1
        );
    }

    #[test]
    fn current_tool_group_terminal_reason_survives_chat_turn_storage() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-c0"), Some(1))
            .unwrap();

        assert!(db
            .finish_chat_turn_once(
                &turn.id,
                ChatTurnStatus::Failed,
                Some(ChatTurnInterruptReason::CurrentToolGroupOverflow),
                Some("display text intentionally has no classifier token"),
                None,
            )
            .unwrap());

        let persisted = db.get_chat_turn(&turn.id).unwrap().unwrap();
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::CurrentToolGroupOverflow)
        );
        assert_eq!(
            persisted.error.as_deref(),
            Some("display text intentionally has no classifier token")
        );
    }

    #[test]
    fn regular_turn_rejects_an_active_session_tool_turn_across_database_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let delegated_db = SessionDB::open(&path).unwrap();
        let session = delegated_db.create_session("ha-main").unwrap();
        let regular_db = SessionDB::open(&path).unwrap();
        let expected_global_stop_epoch = delegated_db.global_stop_epoch().unwrap();

        delegated_db
            .append_message_and_create_session_tool_turn_with_id(
                "delegated-active",
                &session.id,
                None,
                &NewMessage::user("delegated"),
                expected_global_stop_epoch,
            )
            .unwrap();
        let error = regular_db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                "regular-rejected",
                &session.id,
                crate::chat_engine::ChatSource::Http.as_str(),
                None,
                &NewMessage::user("regular"),
                None,
                None,
                None,
                None,
            )
            .expect_err("a regular turn must not overlap a delegated turn");

        assert!(error.to_string().contains("already has an active"));
        let messages = delegated_db.load_session_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "delegated");
        assert!(delegated_db
            .get_chat_turn("regular-rejected")
            .unwrap()
            .is_none());
    }

    #[test]
    fn session_tool_turn_rejects_an_active_acp_stream_across_database_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let acp_db = SessionDB::open(&path).unwrap();
        let session = acp_db.create_session("ha-main").unwrap();
        let delegated_db = SessionDB::open(&path).unwrap();
        acp_db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: "active-acp-run".to_string(),
                session_id: session.id.clone(),
                source: crate::chat_engine::ChatSource::Acp.as_str().to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .unwrap();

        let error = delegated_db
            .append_message_and_create_session_tool_turn_with_id(
                "delegated-rejected-by-acp",
                &session.id,
                None,
                &NewMessage::user("delegated"),
                delegated_db.global_stop_epoch().unwrap(),
            )
            .expect_err("a delegated turn must not overlap an ACP stream");

        assert!(error.to_string().contains("active acp stream"));
        assert!(delegated_db
            .load_session_messages(&session.id)
            .unwrap()
            .is_empty());
        assert!(delegated_db
            .get_chat_turn("delegated-rejected-by-acp")
            .unwrap()
            .is_none());
    }

    #[test]
    fn acp_stream_rejects_an_active_session_tool_turn_across_database_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let delegated_db = SessionDB::open(&path).unwrap();
        let session = delegated_db.create_session("ha-main").unwrap();
        delegated_db
            .append_message_and_create_session_tool_turn_with_id(
                "active-delegated-turn",
                &session.id,
                None,
                &NewMessage::user("delegated"),
                delegated_db.global_stop_epoch().unwrap(),
            )
            .unwrap();
        let acp_db = SessionDB::open(&path).unwrap();

        let error = acp_db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: "rejected-acp-run".to_string(),
                session_id: session.id.clone(),
                source: crate::chat_engine::ChatSource::Acp.as_str().to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect_err("an ACP stream must not overlap a delegated turn");

        assert!(error.to_string().contains("active session_tool turn"));
        assert_eq!(acp_db.stream_run_status("rejected-acp-run").unwrap(), None);
    }

    #[test]
    fn session_tool_turn_rechecks_source_incognito_in_its_write_transaction() {
        let db = temp_db();
        let source = db.create_session("ha-main").unwrap();
        let target = db.create_session("ha-main").unwrap();
        let expected_global_stop_epoch = db.global_stop_epoch().unwrap();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE sessions SET incognito = 1 WHERE id = ?1",
                params![source.id],
            )?;
            Ok(())
        })
        .unwrap();

        let error = db
            .append_message_and_create_session_tool_turn_with_id(
                "delegated-after-incognito",
                &target.id,
                Some(&source.id),
                &NewMessage::user("hello"),
                expected_global_stop_epoch,
            )
            .expect_err("live source privacy state must fail closed");

        assert!(error.to_string().contains("incognito"));
        assert!(db.load_session_messages(&target.id).unwrap().is_empty());
        assert!(db
            .get_chat_turn("delegated-after-incognito")
            .unwrap()
            .is_none());
    }

    #[test]
    fn dispatch_unknown_terminal_reason_survives_chat_turn_storage() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-send-unknown"), Some(1))
            .unwrap();

        assert!(db
            .finish_chat_turn_once(
                &turn.id,
                ChatTurnStatus::Failed,
                Some(ChatTurnInterruptReason::DispatchUnknown),
                Some("display text intentionally has no classifier token"),
                None,
            )
            .unwrap());

        let persisted = db.get_chat_turn(&turn.id).unwrap().unwrap();
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::DispatchUnknown)
        );
        assert_eq!(
            persisted.error.as_deref(),
            Some("display text intentionally has no classifier token")
        );
    }

    #[test]
    fn typed_mention_receipt_merges_only_onto_turn_user_message() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let mut message = NewMessage::user("@README.md");
        message.attachments_meta = Some(r#"{"plan_trigger":true}"#.into());
        let _admission = admit_direct_turn(
            &db,
            &session.id,
            "typed-turn",
            super::super::QueuedTurnMessageSource::Desktop,
        );
        let (message_id, turn) = db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                "typed-turn",
                &session.id,
                "desktop",
                None,
                &message,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let projection = typed_mention_projection("@README.md");

        assert!(db
            .merge_chat_turn_typed_mention_receipt(&session.id, &turn.id, &projection)
            .unwrap());
        // Failover retries re-merge the same receipt without duplicating it.
        assert!(db
            .merge_chat_turn_typed_mention_receipt(&session.id, &turn.id, &projection)
            .unwrap());

        let persisted = db.get_message(message_id).unwrap().unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(persisted.attachments_meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["plan_trigger"], true);
        assert_eq!(
            meta[super::super::types::ATTACHMENT_META_KEY_TYPED_MENTION_RECEIPT]["mentions"][0]
                ["raw"],
            "@README.md"
        );
    }

    #[test]
    fn typed_mention_receipt_rejects_a_different_persisted_message_snapshot() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let persisted_message = NewMessage::user("@README.md rewritten by hook");
        let _admission = admit_direct_turn(
            &db,
            &session.id,
            "typed-turn-rewritten",
            super::super::QueuedTurnMessageSource::Desktop,
        );
        let (message_id, turn) = db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                "typed-turn-rewritten",
                &session.id,
                "desktop",
                None,
                &persisted_message,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // The mention still occupies the same byte range, so raw/span-only
        // validation would incorrectly attach provenance from the old prompt.
        assert!(!db
            .merge_chat_turn_typed_mention_receipt(
                &session.id,
                &turn.id,
                &typed_mention_projection("@README.md"),
            )
            .unwrap());
        assert!(db
            .get_message(message_id)
            .unwrap()
            .unwrap()
            .attachments_meta
            .is_none());
    }

    #[test]
    fn typed_mention_receipt_rejects_a_span_that_does_not_match_persisted_content() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let _admission = admit_direct_turn(
            &db,
            &session.id,
            "typed-turn-invalid-span",
            super::super::QueuedTurnMessageSource::Desktop,
        );
        let (message_id, turn) = db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                "typed-turn-invalid-span",
                &session.id,
                "desktop",
                None,
                &NewMessage::user("@README.md"),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let mut projection = typed_mention_projection("@README.md");
        projection.mentions[0].raw = "@README.mismatch".into();

        assert!(!db
            .merge_chat_turn_typed_mention_receipt(&session.id, &turn.id, &projection)
            .unwrap());
        assert!(db
            .get_message(message_id)
            .unwrap()
            .unwrap()
            .attachments_meta
            .is_none());
    }

    #[test]
    fn typed_mention_receipt_skips_incognito_history() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, Some(true))
            .unwrap();
        let _admission = admit_direct_turn(
            &db,
            &session.id,
            "incognito-typed-turn",
            super::super::QueuedTurnMessageSource::Desktop,
        );
        let (message_id, turn) = db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                "incognito-typed-turn",
                &session.id,
                "desktop",
                None,
                &NewMessage::user("@README.md"),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert!(!db
            .merge_chat_turn_typed_mention_receipt(
                &session.id,
                &turn.id,
                &typed_mention_projection("@README.md"),
            )
            .unwrap());
        assert!(db
            .get_message(message_id)
            .unwrap()
            .unwrap()
            .attachments_meta
            .is_none());
    }

    #[test]
    fn recover_stale_running_turns_marks_interrupted() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
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
    fn ui_chat_dispatch_identity_survives_reopen_and_is_unique() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let session_id;
        {
            let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
            let session = db
                .create_session_with_project("ha-main", None, None)
                .unwrap();
            session_id = session.id.clone();
            let _admission = admit_direct_turn(
                &db,
                &session.id,
                "turn-1",
                super::super::QueuedTurnMessageSource::Http,
            );
            let (message_id, turn) = db
                .append_message_and_create_chat_turn_with_id_surface_dispatch(
                    "turn-1",
                    &session.id,
                    "http",
                    None,
                    &NewMessage::user("first request"),
                    Some(crate::pet::ChatUiSurface::MainChat),
                    Some("request-1"),
                    Some("fingerprint-1"),
                    None,
                )
                .unwrap();
            assert_eq!(turn.user_message_id, Some(message_id));
            // A second admission is only grantable once the first turn settles;
            // the duplicate below must fail on the request id, not on the fence.
            assert!(db
                .finish_chat_turn_once(&turn.id, ChatTurnStatus::Completed, None, None, None)
                .unwrap());
        }

        let db = SessionDB::open_ephemeral_for_test(&path).unwrap();
        assert_eq!(
            db.get_ui_chat_dispatch("request-1").unwrap(),
            Some(UiChatDispatchRecord {
                request_fingerprint: "fingerprint-1".to_string(),
                session_id: session_id.clone(),
                turn_id: "turn-1".to_string(),
                queue_request_id: None,
            })
        );
        let _duplicate_admission = admit_direct_turn(
            &db,
            &session_id,
            "turn-2",
            super::super::QueuedTurnMessageSource::Http,
        );
        let duplicate = db.append_message_and_create_chat_turn_with_id_surface_dispatch(
            "turn-2",
            &session_id,
            "http",
            None,
            &NewMessage::user("must roll back"),
            Some(crate::pet::ChatUiSurface::MainChat),
            Some("request-1"),
            Some("fingerprint-1"),
            None,
        );
        assert!(duplicate.is_err(), "request id must be globally unique");
        let messages = db.load_session_messages(&session_id).unwrap();
        assert_eq!(messages.len(), 1, "duplicate message insert must roll back");
        assert_eq!(messages[0].content, "first request");
    }

    #[test]
    fn execution_success_after_cancelling_finishes_interrupted() {
        let db = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
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
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
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
