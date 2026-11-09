//! Owner-local assistant state. Execution remains in TurnKernel and its shared queue.
//!
//! All methods returning this module's projections are owner surfaces. Model
//! callers must use the scoped tools, never expose these as generic SQL access.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::session::{SessionDB, SessionMeta};

pub const INBOX_CHANGED_EVENT: &str = "personal_assistant:changed";
const MAX_OPEN_REQUESTS: i64 = 64;
const MAX_WATCHES: i64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantError {
    Unavailable,
    InvalidInput,
    Conflict,
    Busy,
    Stopped,
    LimitReached,
}

impl std::fmt::Display for AssistantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Personal assistant source or target is unavailable",
            Self::InvalidInput => "Invalid personal assistant request",
            Self::Conflict => "Personal assistant request identity was already used differently",
            Self::Busy => "Personal assistant target is awaiting an earlier turn",
            Self::Stopped => "Personal assistant request was superseded by Stop",
            Self::LimitReached => "Personal assistant active item limit reached",
        })
    }
}

impl std::error::Error for AssistantError {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDispatchInput {
    pub source_turn_id: String,
    pub item_key: String,
    pub target_session_id: String,
    pub instruction: String,
    pub global_stop_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantRequest {
    pub id: String,
    pub source_turn_id: String,
    pub source_message_id: i64,
    pub item_key: String,
    pub target_session_id: String,
    pub target_title: Option<String>,
    pub instruction: String,
    pub status: String,
    pub turn_id: Option<String>,
    pub result_message_id: Option<i64>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    /// Filled by the async owner projection from real approval/question state.
    /// None means the synchronous ledger has not inspected interactive waits.
    pub pending_interaction_count: Option<i64>,
    pub global_stop_epoch: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantCancelOutcome {
    pub request: AssistantRequest,
    pub cancel_turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantWatch {
    pub session_id: String,
    pub goal_id: String,
    pub objective: String,
    pub state: String,
    pub needs_attention: bool,
    pub reason: Option<String>,
    pub event_id: Option<i64>,
}

pub(crate) enum AssistantSessionCreation<'a> {
    Home,
    Target {
        source_turn_id: &'a str,
        item_key: &'a str,
        fingerprint: String,
        global_stop_epoch: u64,
    },
}

pub(crate) fn emit_inbox_changed() {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(INBOX_CHANGED_EVENT, serde_json::json!({}));
    }
}

pub async fn enrich_personal_assistant_requests(
    requests: &mut [AssistantRequest],
    db: &std::sync::Arc<SessionDB>,
) -> Result<()> {
    if requests.is_empty() {
        return Ok(());
    }
    let approvals = crate::tools::approval::pending_approvals_per_session().await;
    let questions = db
        .run(|db| db.count_pending_ask_user_groups_per_session())
        .await?;
    let running: Vec<_> = requests
        .iter()
        .filter(|request| {
            matches!(
                request.status.as_str(),
                "running" | "cancellation_requested"
            )
        })
        .filter_map(|request| {
            request.turn_id.as_ref().map(|turn| {
                (
                    request.id.clone(),
                    request.target_session_id.clone(),
                    turn.clone(),
                )
            })
        })
        .collect();
    let current = db.run(move |db| {
        let conn = db.read_conn()?;
        let mut current = std::collections::HashSet::new();
        for (request_id, target_id, turn_id) in running {
            let active: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_turns WHERE id=?1 AND session_id=?2 AND status IN ('running','cancelling'))",
                params![turn_id,target_id], |row| row.get(0),
            )?;
            if active { current.insert(request_id); }
        }
        Ok(current)
    }).await?;
    for request in requests {
        if !current.contains(&request.id) {
            request.pending_interaction_count = Some(0);
            continue;
        }
        request.pending_interaction_count = Some(
            approvals
                .get(&request.target_session_id)
                .map(|value| value.count)
                .unwrap_or(0)
                + questions
                    .get(&request.target_session_id)
                    .copied()
                    .unwrap_or(0),
        );
    }
    Ok(())
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS personal_assistant_single_home
            ON sessions(kind) WHERE kind = 'personal_assistant';
         CREATE TABLE IF NOT EXISTS personal_assistant_private_sessions (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS personal_assistant_created_sessions (
            source_turn_id TEXT NOT NULL REFERENCES chat_turns(id) ON DELETE CASCADE,
            item_key TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            target_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            PRIMARY KEY(source_turn_id, item_key)
         );
         CREATE TABLE IF NOT EXISTS personal_assistant_requests (
            id TEXT PRIMARY KEY,
            home_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            source_turn_id TEXT NOT NULL,
            source_message_id INTEGER NOT NULL,
            item_key TEXT NOT NULL,
            target_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            instruction TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'queued'
                CHECK(status IN ('queued','running','cancellation_requested','completed','interrupted','cancelled','failed')),
            turn_id TEXT UNIQUE,
            result_message_id INTEGER,
            error TEXT,
            global_stop_epoch INTEGER NOT NULL,
            source_stop_epoch INTEGER NOT NULL,
            target_stop_epoch INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(home_session_id, source_message_id, item_key)
         );
         CREATE INDEX IF NOT EXISTS personal_assistant_request_queue
            ON personal_assistant_requests(status, created_at, id);
         CREATE TABLE IF NOT EXISTS personal_assistant_watches (
            goal_id TEXT PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS personal_assistant_attention (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            goal_id TEXT NOT NULL REFERENCES personal_assistant_watches(goal_id) ON DELETE CASCADE,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            resolved_at TEXT
         );
         CREATE INDEX IF NOT EXISTS personal_assistant_attention_goal
            ON personal_assistant_attention(goal_id, id);
         CREATE TRIGGER IF NOT EXISTS personal_assistant_inherit_insert
            AFTER INSERT ON sessions
            WHEN EXISTS(SELECT 1 FROM sessions parent
                WHERE parent.id IN (NEW.parent_session_id,NEW.forked_from_session_id)
                  AND (parent.kind='personal_assistant' OR EXISTS(
                    SELECT 1 FROM personal_assistant_private_sessions p WHERE p.session_id=parent.id)))
            BEGIN INSERT OR IGNORE INTO personal_assistant_private_sessions(session_id) VALUES(NEW.id); END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_inherit_update
            AFTER UPDATE OF parent_session_id,forked_from_session_id ON sessions
            WHEN EXISTS(SELECT 1 FROM sessions parent
                WHERE parent.id IN (NEW.parent_session_id,NEW.forked_from_session_id)
                  AND (parent.kind='personal_assistant' OR EXISTS(
                    SELECT 1 FROM personal_assistant_private_sessions p WHERE p.session_id=parent.id)))
            BEGIN INSERT OR IGNORE INTO personal_assistant_private_sessions(session_id) VALUES(NEW.id); END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_inherit_descendants
            AFTER INSERT ON personal_assistant_private_sessions BEGIN
                INSERT OR IGNORE INTO personal_assistant_private_sessions(session_id)
                WITH RECURSIVE descendants(id) AS (
                    SELECT NEW.session_id
                    UNION
                    SELECT s.id FROM sessions s JOIN descendants d
                      ON s.parent_session_id=d.id OR s.forked_from_session_id=d.id
                ) SELECT id FROM descendants;
            END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_home_scope
            BEFORE UPDATE OF kind, incognito, archived_at, is_cron, parent_session_id, project_id ON sessions
            WHEN OLD.kind = 'personal_assistant' AND
                 (NEW.kind <> 'personal_assistant' OR NEW.incognito <> 0 OR NEW.archived_at IS NOT NULL
                  OR NEW.is_cron <> 0 OR NEW.parent_session_id IS NOT NULL OR NEW.project_id IS NOT NULL)
            BEGIN SELECT RAISE(ABORT, 'personal assistant home scope is fixed'); END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_request_deleted
            AFTER DELETE ON personal_assistant_requests
            BEGIN DELETE FROM queued_turn_user_messages
                WHERE request_id = OLD.id AND source = 'personal_assistant'; END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_session_scope_changed
            AFTER UPDATE OF kind, incognito, archived_at, is_cron, parent_session_id ON sessions
            WHEN NEW.kind <> 'regular' OR NEW.incognito <> 0 OR NEW.archived_at IS NOT NULL
                 OR NEW.is_cron <> 0 OR NEW.parent_session_id IS NOT NULL
            BEGIN
                DELETE FROM personal_assistant_watches WHERE session_id = NEW.id;
                DELETE FROM personal_assistant_requests WHERE target_session_id = NEW.id;
            END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_imported
            AFTER INSERT ON session_import_sources BEGIN
                DELETE FROM personal_assistant_watches WHERE session_id = NEW.session_id;
                DELETE FROM personal_assistant_requests WHERE target_session_id = NEW.session_id;
            END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_goal_changed
            AFTER UPDATE OF state, closure_decision ON goals
            WHEN NEW.state <> OLD.state OR NEW.closure_decision IS NOT OLD.closure_decision
            BEGIN
                UPDATE personal_assistant_attention SET resolved_at = NEW.updated_at
                    WHERE goal_id = NEW.id AND resolved_at IS NULL;
                INSERT INTO personal_assistant_attention(goal_id, reason, created_at)
                    SELECT NEW.id,
                        CASE WHEN NEW.state = 'blocked' THEN 'blocked' ELSE 'awaiting_acceptance' END,
                        NEW.updated_at
                    WHERE EXISTS (SELECT 1 FROM personal_assistant_watches WHERE goal_id = NEW.id)
                      AND (NEW.state = 'blocked' OR (NEW.state = 'completed' AND NEW.closure_decision IS NULL));
                DELETE FROM personal_assistant_watches WHERE goal_id = NEW.id
                    AND (NEW.state IN ('cancelled','failed') OR NEW.closure_decision IS NOT NULL);
            END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_goal_replaced
            AFTER INSERT ON goals BEGIN
                DELETE FROM personal_assistant_watches
                    WHERE session_id = NEW.session_id AND goal_id <> NEW.id;
            END;",
    )?;
    ensure_channel_guards(conn)
}

/// ChannelDB may be migrated after SessionDB. Install the guards when either
/// schema becomes available, without creating or modifying ordinary bindings.
pub(crate) fn ensure_channel_guards(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "channel_conversations")?
        || !table_exists(conn, "personal_assistant_private_sessions")?
    {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS personal_assistant_channel_insert
         BEFORE INSERT ON channel_conversations BEGIN
            SELECT CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE id = NEW.session_id AND kind = 'personal_assistant')
                OR EXISTS(SELECT 1 FROM personal_assistant_private_sessions WHERE session_id = NEW.session_id)
                THEN RAISE(ABORT, 'private assistant conversation cannot be attached to IM') END;
         END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_channel_update
         BEFORE UPDATE OF session_id ON channel_conversations BEGIN
            SELECT CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE id = NEW.session_id AND kind = 'personal_assistant')
                OR EXISTS(SELECT 1 FROM personal_assistant_private_sessions WHERE session_id = NEW.session_id)
                THEN RAISE(ABORT, 'private assistant conversation cannot be attached to IM') END;
         END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_channel_watch_insert
         AFTER INSERT ON channel_conversations BEGIN
            DELETE FROM personal_assistant_watches WHERE session_id = NEW.session_id;
            DELETE FROM personal_assistant_requests WHERE target_session_id = NEW.session_id;
         END;
         CREATE TRIGGER IF NOT EXISTS personal_assistant_channel_watch_update
         AFTER UPDATE OF session_id ON channel_conversations BEGIN
            DELETE FROM personal_assistant_watches WHERE session_id = NEW.session_id;
            DELETE FROM personal_assistant_requests WHERE target_session_id = NEW.session_id;
         END;",
    )?;
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn validate_target(conn: &Connection, session_id: &str) -> Result<()> {
    let eligible: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id = ?1
            AND s.kind = 'regular' AND s.incognito = 0 AND s.archived_at IS NULL
            AND s.is_cron = 0 AND s.parent_session_id IS NULL
            AND NOT EXISTS(SELECT 1 FROM session_import_sources i WHERE i.session_id = s.id))",
        [session_id],
        |row| row.get(0),
    )?;
    if !eligible {
        return Err(AssistantError::Unavailable.into());
    }
    if table_exists(conn, "channel_conversations")? {
        let attached: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM channel_conversations WHERE session_id = ?1)",
            [session_id],
            |row| row.get(0),
        )?;
        if attached {
            return Err(AssistantError::Unavailable.into());
        }
    }
    Ok(())
}

fn stop_epoch(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?)
}

fn check_stop(conn: &Connection, session_id: &str, global_stop_epoch: u64) -> Result<()> {
    let current: i64 = conn.query_row(
        "SELECT epoch FROM runtime_control_epochs WHERE key = 'global_stop'",
        [],
        |row| row.get(0),
    )?;
    let paused: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_autonomy_pauses WHERE session_id = ?1 AND resumed_at IS NULL)",
        [session_id],
        |row| row.get(0),
    )?;
    if current != i64::try_from(global_stop_epoch).map_err(|_| AssistantError::InvalidInput)?
        || paused
    {
        return Err(AssistantError::Stopped.into());
    }
    Ok(())
}

fn source_turn(conn: &Connection, turn_id: &str, epoch: u64) -> Result<(String, i64)> {
    let source = conn.query_row(
        "SELECT s.id, t.user_message_id FROM chat_turns t JOIN sessions s ON s.id = t.session_id
         JOIN messages m ON m.id = t.user_message_id AND m.session_id = s.id
         WHERE t.id = ?1 AND t.status = 'running' AND t.source IN ('desktop','http')
             AND s.kind = 'personal_assistant' AND s.incognito = 0
             AND m.role = 'user' AND COALESCE(m.source,'desktop') IN ('desktop','http')",
        [turn_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    ).optional()?.ok_or(AssistantError::Unavailable)?;
    // The queue outlives the source turn. Until evaluation provenance is
    // durable, accepting it here could later execute an untagged paid run.
    if crate::eval_context::context_for_session(&source.0).is_some() {
        return Err(anyhow!(AssistantError::InvalidInput)
            .context("Personal assistant delegation requires durable evaluation provenance"));
    }
    check_stop(conn, &source.0, epoch)?;
    Ok(source)
}

fn valid_key(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256
}

pub(crate) fn existing_created_session(
    conn: &Connection,
    creation: &AssistantSessionCreation<'_>,
) -> Result<Option<String>> {
    match creation {
        AssistantSessionCreation::Home => Ok(conn
            .query_row(
                "SELECT id FROM sessions WHERE kind = 'personal_assistant'",
                [],
                |row| row.get(0),
            )
            .optional()?),
        AssistantSessionCreation::Target {
            source_turn_id,
            item_key,
            fingerprint,
            global_stop_epoch,
        } => {
            if !valid_key(item_key) {
                return Err(AssistantError::InvalidInput.into());
            }
            source_turn(conn, source_turn_id, *global_stop_epoch)?;
            let existing: Option<(String, String)> = conn
                .query_row(
                    "SELECT target_session_id, fingerprint FROM personal_assistant_created_sessions
                 WHERE source_turn_id = ?1 AND item_key = ?2",
                    params![source_turn_id, item_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((target, previous)) = existing {
                if &previous != fingerprint {
                    return Err(AssistantError::Conflict.into());
                }
                validate_target(conn, &target)?;
                return Ok(Some(target));
            }
            Ok(None)
        }
    }
}

pub(crate) fn record_created_session(
    conn: &Connection,
    creation: &AssistantSessionCreation<'_>,
    session_id: &str,
) -> Result<()> {
    match creation {
        AssistantSessionCreation::Home => {
            conn.execute(
                "INSERT INTO session_memory_policy(session_id,use_memories,contribute_to_memories,updated_at)
                 VALUES(?1,'deny','deny',?2)", params![session_id, chrono::Utc::now().to_rfc3339()],
            )?;
        }
        AssistantSessionCreation::Target {
            source_turn_id,
            item_key,
            fingerprint,
            ..
        } => {
            conn.execute(
                "INSERT INTO personal_assistant_created_sessions(source_turn_id,item_key,fingerprint,target_session_id)
                 VALUES(?1,?2,?3,?4)", params![source_turn_id,item_key,fingerprint,session_id],
            )?;
        }
    }
    Ok(())
}

impl SessionDB {
    pub fn ensure_personal_assistant_home(&self) -> Result<SessionMeta> {
        let existing = {
            let conn = self.read_conn()?;
            existing_created_session(&conn, &AssistantSessionCreation::Home)?
        };
        if let Some(id) = existing {
            return self
                .get_session(&id)?
                .ok_or_else(|| AssistantError::Unavailable.into());
        }
        self.create_session_for_personal_assistant(
            crate::agent_loader::DEFAULT_AGENT_ID,
            None,
            Some("Hope"),
            &AssistantSessionCreation::Home,
        )
    }

    pub fn create_personal_assistant_target(
        &self,
        source_turn_id: &str,
        item_key: &str,
        agent_id: &str,
        project_id: Option<&str>,
        title: Option<&str>,
        global_stop_epoch: u64,
    ) -> Result<SessionMeta> {
        let fingerprint = serde_json::to_string(&(agent_id, project_id, title))?;
        self.create_session_for_personal_assistant(
            agent_id,
            project_id,
            title,
            &AssistantSessionCreation::Target {
                source_turn_id,
                item_key,
                fingerprint,
                global_stop_epoch,
            },
        )
    }

    pub fn has_personal_assistant_private_data(&self) -> Result<bool> {
        let conn = self.read_conn()?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE kind = 'personal_assistant')
                OR EXISTS(SELECT 1 FROM personal_assistant_private_sessions)
                OR EXISTS(SELECT 1 FROM personal_assistant_watches)",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn is_personal_assistant_private_session(&self, session_id: &str) -> Result<bool> {
        let conn = self.read_conn()?;
        Ok(conn.query_row(
            "WITH RECURSIVE ancestors(id) AS (
                SELECT id FROM sessions WHERE id=?1
                UNION
                SELECT s.parent_session_id FROM sessions s JOIN ancestors a ON s.id=a.id
                    WHERE s.parent_session_id IS NOT NULL
                UNION
                SELECT s.forked_from_session_id FROM sessions s JOIN ancestors a ON s.id=a.id
                    WHERE s.forked_from_session_id IS NOT NULL
             ) SELECT EXISTS(SELECT 1 FROM sessions s JOIN ancestors a ON a.id=s.id
                WHERE s.kind='personal_assistant' OR EXISTS(
                    SELECT 1 FROM personal_assistant_private_sessions p WHERE p.session_id=s.id))",
            [session_id],
            |row| row.get(0),
        )?)
    }

    pub fn validate_personal_assistant_target(&self, session_id: &str) -> Result<()> {
        validate_target(&*self.read_conn()?, session_id)
    }

    pub fn enqueue_personal_assistant_request(
        &self,
        input: AssistantDispatchInput,
    ) -> Result<AssistantRequest> {
        if !valid_key(&input.item_key)
            || input.instruction.trim().is_empty()
            || input.instruction.len() > 64 * 1024
        {
            return Err(AssistantError::InvalidInput.into());
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (home_id, source_message_id) =
            source_turn(&tx, &input.source_turn_id, input.global_stop_epoch)?;
        validate_target(&tx, &input.target_session_id)?;
        check_stop(&tx, &input.target_session_id, input.global_stop_epoch)?;
        let existing: Option<(String, String, String)> = tx
            .query_row(
                "SELECT id,target_session_id,instruction FROM personal_assistant_requests
             WHERE home_session_id = ?1 AND source_message_id = ?2 AND item_key = ?3",
                params![home_id, source_message_id, input.item_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((id, target, instruction)) = existing {
            if target != input.target_session_id || instruction != input.instruction {
                return Err(AssistantError::Conflict.into());
            }
            return request_with_conn(&tx, &id)?.ok_or_else(|| AssistantError::Unavailable.into());
        }
        let count:i64 = tx.query_row(
            "SELECT COUNT(*) FROM personal_assistant_requests WHERE status IN ('queued','running','cancellation_requested')",
            [], |row| row.get(0),
        )?;
        if count >= MAX_OPEN_REQUESTS {
            return Err(AssistantError::LimitReached.into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO personal_assistant_requests(id,home_session_id,source_turn_id,source_message_id,item_key,
             target_session_id,instruction,global_stop_epoch,source_stop_epoch,target_stop_epoch,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
            params![id,home_id,input.source_turn_id,source_message_id,input.item_key,input.target_session_id,
                input.instruction,i64::try_from(input.global_stop_epoch).map_err(|_| AssistantError::InvalidInput)?,stop_epoch(&tx,&home_id)?,
                stop_epoch(&tx,&input.target_session_id)?,now],
        )?;
        crate::session::enqueue_personal_assistant_with_conn(
            &tx,
            &id,
            &input.target_session_id,
            &input.instruction,
        )?;
        let request = request_with_conn(&tx, &id)?.ok_or(AssistantError::Unavailable)?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(request)
    }

    pub fn personal_assistant_request(&self, request_id: &str) -> Result<Option<AssistantRequest>> {
        request_with_conn(&*self.read_conn()?, request_id)
    }

    pub fn list_personal_assistant_requests(&self) -> Result<Vec<AssistantRequest>> {
        self.personal_assistant_requests(false, 200)
    }

    pub fn queued_personal_assistant_requests(
        &self,
        limit: usize,
    ) -> Result<Vec<AssistantRequest>> {
        self.personal_assistant_requests(true, limit.min(64))
    }

    fn personal_assistant_requests(
        &self,
        queued_only: bool,
        limit: usize,
    ) -> Result<Vec<AssistantRequest>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM personal_assistant_requests WHERE (?1 = 0 OR status = 'queued')
             ORDER BY CASE WHEN status IN ('queued','running','cancellation_requested') THEN 0 ELSE 1 END,
                CASE WHEN status IN ('queued','running','cancellation_requested') THEN created_at END,
                created_at DESC, id LIMIT ?2",
        )?;
        let ids = stmt
            .query_map(params![queued_only, limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut requests = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(request) = request_with_conn(&conn, &id)? {
                requests.push(request);
            }
        }
        Ok(requests)
    }

    pub fn cancel_personal_assistant_request(
        &self,
        request_id: &str,
    ) -> Result<AssistantCancelOutcome> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let request = request_with_conn(&tx, request_id)?.ok_or(AssistantError::Unavailable)?;
        let cancel_turn_id = match request.status.as_str() {
            "queued" => {
                tx.execute("DELETE FROM queued_turn_user_messages WHERE request_id = ?1 AND source = 'personal_assistant'",[request_id])?;
                tx.execute("UPDATE personal_assistant_requests SET status = 'cancelled',updated_at = ?2 WHERE id = ?1",params![request_id,chrono::Utc::now().to_rfc3339()])?;
                None
            }
            "running" | "cancellation_requested" => {
                tx.execute("UPDATE personal_assistant_requests SET status = 'cancellation_requested',updated_at = ?2 WHERE id = ?1",params![request_id,chrono::Utc::now().to_rfc3339()])?;
                request.turn_id
            }
            _ => None,
        };
        let request = request_with_conn(&tx, request_id)?.ok_or(AssistantError::Unavailable)?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(AssistantCancelOutcome {
            request,
            cancel_turn_id,
        })
    }

    /// Reconcile from the exact bound turn, including startup crash recovery.
    pub fn finish_personal_assistant_request(&self, request_id: &str) -> Result<AssistantRequest> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let request = request_with_conn(&tx, request_id)?.ok_or(AssistantError::Unavailable)?;
        let changed = tx.execute(
            "UPDATE personal_assistant_requests SET status = ?2,result_message_id = ?3,error = ?4,updated_at = ?5
             WHERE id = ?1 AND (status <> ?2 OR result_message_id IS NOT ?3 OR error IS NOT ?4)",
            params![request_id,request.status,request.result_message_id,request.error,request.updated_at],
        )?;
        tx.commit()?;
        drop(conn);
        if changed > 0 {
            emit_inbox_changed();
        }
        Ok(request)
    }

    /// Primary reconciliation inspects the stored state, since read projections
    /// may already show the bound turn's terminal result before this commit.
    pub fn reconcile_personal_assistant_requests(&self) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| anyhow!("Lock error: {error}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids = {
            let mut stmt = tx.prepare(
                "SELECT id FROM personal_assistant_requests
                 WHERE status IN ('queued','running','cancellation_requested')",
            )?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut changed = false;
        for id in ids {
            let Some(request) = request_with_conn(&tx, &id)? else {
                continue;
            };
            if matches!(
                request.status.as_str(),
                "completed" | "interrupted" | "cancelled" | "failed"
            ) {
                changed |= tx.execute(
                    "UPDATE personal_assistant_requests SET status=?2,result_message_id=?3,error=?4,updated_at=?5 WHERE id=?1",
                    params![id,request.status,request.result_message_id,request.error,request.updated_at],
                )? > 0;
                continue;
            }
            if request.status != "queued" {
                continue;
            }
            let (home_id, source_epoch, target_epoch): (String, i64, i64) = tx.query_row(
                "SELECT home_session_id,source_stop_epoch,target_stop_epoch FROM personal_assistant_requests WHERE id=?1",
                [&id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            )?;
            let stopped =
                match check_stop(&tx, &home_id, request.global_stop_epoch).and_then(|_| {
                    check_stop(&tx, &request.target_session_id, request.global_stop_epoch)
                }) {
                    Ok(()) => false,
                    Err(error)
                        if error.downcast_ref::<AssistantError>()
                            == Some(&AssistantError::Stopped) =>
                    {
                        true
                    }
                    Err(error) => return Err(error),
                };
            let queued: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM queued_turn_user_messages WHERE request_id=?1 AND source='personal_assistant')",
                [&id], |row| row.get(0),
            )?;
            if stopped
                || !queued
                || stop_epoch(&tx, &home_id)? != source_epoch
                || stop_epoch(&tx, &request.target_session_id)? != target_epoch
            {
                tx.execute(
                    "UPDATE personal_assistant_requests SET status='cancelled',error='stopped_or_removed',updated_at=?2 WHERE id=?1",
                    params![id,chrono::Utc::now().to_rfc3339()],
                )?;
                tx.execute("DELETE FROM queued_turn_user_messages WHERE request_id=?1 AND source='personal_assistant'",[&id])?;
                changed = true;
            }
        }
        tx.commit()?;
        drop(conn);
        if changed {
            emit_inbox_changed();
        }
        Ok(())
    }

    /// This accepts a reason code, never a raw provider/tool error containing data.
    pub fn fail_personal_assistant_request(&self, request_id: &str, _error: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE personal_assistant_requests SET status = 'failed',error = 'dispatch_failed',updated_at = ?2
             WHERE id = ?1 AND status = 'queued'",
            params![request_id,chrono::Utc::now().to_rfc3339()],
        )?;
        if changed > 0 {
            tx.execute("DELETE FROM queued_turn_user_messages WHERE request_id = ?1 AND source = 'personal_assistant'",[request_id])?;
        }
        tx.commit()?;
        drop(conn);
        if changed > 0 {
            emit_inbox_changed();
        }
        Ok(())
    }

    pub fn watch_personal_goal(&self, session_id: &str, goal_id: &str) -> Result<AssistantWatch> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        validate_target(&tx, session_id)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT id FROM goals WHERE session_id = ?1
                AND (state IN ('active','paused','evaluating','blocked')
                     OR (state = 'completed' AND closure_decision IS NULL))
             ORDER BY updated_at DESC LIMIT 1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        if active.as_deref() != Some(goal_id) {
            return Err(AssistantError::Unavailable.into());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_assistant_watches WHERE goal_id=?1)",
            [goal_id],
            |row| row.get(0),
        )?;
        if !exists {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM personal_assistant_watches",
                [],
                |row| row.get(0),
            )?;
            if count >= MAX_WATCHES {
                return Err(AssistantError::LimitReached.into());
            }
            // The current state is the baseline. Only a later real transition
            // inserts attention, even if it is already blocked at opt-in.
            tx.execute("INSERT INTO personal_assistant_watches(goal_id,session_id,created_at) VALUES(?1,?2,?3)",
                params![goal_id,session_id,chrono::Utc::now().to_rfc3339()])?;
        }
        let watch = watch_with_conn(&tx, goal_id)?.ok_or(AssistantError::Unavailable)?;
        tx.commit()?;
        drop(conn);
        if !exists {
            emit_inbox_changed();
        }
        Ok(watch)
    }

    pub fn unwatch_personal_goal(&self, session_id: &str, goal_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let removed = conn.execute(
            "DELETE FROM personal_assistant_watches WHERE goal_id=?1 AND session_id=?2",
            params![goal_id, session_id],
        )?;
        drop(conn);
        if removed > 0 {
            emit_inbox_changed();
        }
        Ok(())
    }

    pub fn personal_assistant_inbox(&self) -> Result<Vec<AssistantWatch>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT goal_id FROM personal_assistant_watches ORDER BY created_at,goal_id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut watches = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(watch) = watch_with_conn(&conn, &id)? {
                watches.push(watch);
            }
        }
        Ok(watches)
    }

    /// Refresh at the next owner turn. Runtime must wrap this snapshot as
    /// untrusted data. No derived source text is copied into the home history.
    pub fn render_personal_assistant_receipts(&self) -> Result<String> {
        let requests = self.list_personal_assistant_requests()?;
        let watches = self.personal_assistant_inbox()?;
        let requests:Vec<_> = requests.iter().take(32).map(|request| serde_json::json!({
            "id":request.id,"sourceMessageId":request.source_message_id,"targetSessionId":request.target_session_id,"targetTitle":request.target_title,
            "status":request.status,"turnId":request.turn_id,"resultMessageId":request.result_message_id,
            "resultPreview":request.result_preview,
            "updatedAt":request.updated_at,
        })).collect();
        Ok(serde_json::to_string(
            &serde_json::json!({"requests":requests,"watchedGoals":watches}),
        )?)
    }
}

fn watch_with_conn(conn: &Connection, goal_id: &str) -> Result<Option<AssistantWatch>> {
    let watch = conn.query_row(
        "SELECT w.session_id,g.id,g.objective,g.state,a.resolved_at IS NULL AND a.id IS NOT NULL,
                CASE WHEN a.resolved_at IS NULL THEN a.reason END,a.id
         FROM personal_assistant_watches w JOIN goals g ON g.id=w.goal_id AND g.session_id=w.session_id
         LEFT JOIN personal_assistant_attention a ON a.id=(SELECT MAX(id) FROM personal_assistant_attention WHERE goal_id=g.id)
         WHERE w.goal_id=?1 AND g.id=(SELECT id FROM goals WHERE session_id=w.session_id
             AND (state IN ('active','paused','evaluating','blocked') OR (state='completed' AND closure_decision IS NULL))
             ORDER BY updated_at DESC LIMIT 1)", [goal_id], |row|Ok(AssistantWatch {
            session_id:row.get(0)?,goal_id:row.get(1)?,objective:row.get(2)?,state:row.get(3)?,
            needs_attention:row.get(4)?,reason:row.get(5)?,event_id:row.get(6)?,
         }),
    ).optional()?;
    if let Some(watch) = &watch {
        match validate_target(conn, &watch.session_id) {
            Ok(()) => {}
            Err(error)
                if error.downcast_ref::<AssistantError>() == Some(&AssistantError::Unavailable) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        }
    }
    Ok(watch)
}

fn request_with_conn(conn: &Connection, request_id: &str) -> Result<Option<AssistantRequest>> {
    let request = conn.query_row(
        "SELECT r.id,r.source_turn_id,r.source_message_id,r.item_key,r.target_session_id,s.title,r.instruction,
         CASE WHEN t.status = 'completed' THEN 'completed'
              WHEN t.status = 'failed' THEN 'failed'
              WHEN t.status = 'interrupted' AND r.status = 'cancellation_requested' THEN 'cancelled'
              WHEN t.status = 'interrupted' THEN 'interrupted' ELSE r.status END,
         r.turn_id,COALESCE(t.terminal_message_id,t.assistant_message_id,r.result_message_id),
         CASE WHEN t.status = 'failed' THEN 'execution_failed' ELSE r.error END,
         r.global_stop_epoch,r.created_at,COALESCE(t.updated_at,r.updated_at),
         CASE WHEN t.status='completed' THEN (
            SELECT substr(m.content,1,2000) FROM messages m
            WHERE m.id=t.assistant_message_id AND m.session_id=t.session_id AND m.role='assistant'
         ) END
         FROM personal_assistant_requests r JOIN sessions s ON s.id = r.target_session_id
         JOIN sessions h ON h.id = r.home_session_id AND h.kind = 'personal_assistant' AND h.incognito = 0
         LEFT JOIN chat_turns t ON t.id = r.turn_id AND t.session_id = r.target_session_id
         WHERE r.id = ?1", [request_id], |row| Ok(AssistantRequest {
            id:row.get(0)?,source_turn_id:row.get(1)?,source_message_id:row.get(2)?,item_key:row.get(3)?,
            target_session_id:row.get(4)?,target_title:row.get(5)?,instruction:row.get(6)?,status:row.get(7)?,
            turn_id:row.get(8)?,result_message_id:row.get(9)?,error:row.get(10)?,pending_interaction_count:None,global_stop_epoch:row.get::<_,i64>(11)?.max(0) as u64,
            created_at:row.get(12)?,updated_at:row.get(13)?,result_preview:row.get(14)?,
         }),
    ).optional()?;
    if let Some(request) = &request {
        match validate_target(conn, &request.target_session_id) {
            Ok(()) => {}
            Err(error)
                if error.downcast_ref::<AssistantError>() == Some(&AssistantError::Unavailable) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        }
    }
    Ok(request)
}

/// Called inside the target message/turn INSERT transaction. The caller owns
/// the foreground admission lock and never invokes this on an untrusted ID.
pub(crate) fn admit_request_with_conn(
    conn: &Connection,
    request_id: &str,
    target_session_id: &str,
    turn_id: &str,
) -> Result<()> {
    let request = request_with_conn(conn, request_id)?.ok_or(AssistantError::Unavailable)?;
    if request.target_session_id != target_session_id {
        return Err(AssistantError::Unavailable.into());
    }
    if request.status != "queued" {
        return Err(AssistantError::Conflict.into());
    }
    let (home_id,source_epoch,target_epoch):(String,i64,i64) = conn.query_row(
        "SELECT home_session_id,source_stop_epoch,target_stop_epoch FROM personal_assistant_requests WHERE id = ?1",
        [request_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    check_stop(conn, &home_id, request.global_stop_epoch)?;
    check_stop(conn, target_session_id, request.global_stop_epoch)?;
    if stop_epoch(conn, &home_id)? != source_epoch
        || stop_epoch(conn, target_session_id)? != target_epoch
    {
        return Err(AssistantError::Stopped.into());
    }
    if !crate::session::personal_assistant_queue_is_head_with_conn(
        conn,
        request_id,
        target_session_id,
    )? {
        return Err(AssistantError::Busy.into());
    }
    conn.execute(
        "UPDATE personal_assistant_requests SET status = 'running',turn_id = ?2,updated_at = ?3 WHERE id = ?1 AND status = 'queued'",
        params![request_id,turn_id,chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO personal_assistant_private_sessions(session_id) VALUES(?1)",
        [target_session_id],
    )?;
    conn.execute("DELETE FROM queued_turn_user_messages WHERE request_id = ?1 AND source = 'personal_assistant'",[request_id])?;
    Ok(())
}

#[cfg(test)]
mod tests;
