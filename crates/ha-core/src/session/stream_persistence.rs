//! Durable append-only journal for streamed chat turns.
//!
//! The journal is the crash-recovery truth source for new streams. `messages`
//! remains the query-optimized materialized view and legacy streaming rows are
//! kept readable during the compatibility window.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::model_usage::ModelUsageEvent;

use super::{ChatTurnStatus, NewMessage, SessionDB};

#[derive(Debug, Clone)]
pub struct CreateStreamRun {
    pub run_id: String,
    pub session_id: String,
    pub source: String,
    pub stream_id: Option<String>,
    pub turn_id: Option<String>,
    pub provider_shape: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRunRegistration {
    pub run_id: String,
    pub context_revision: i64,
    pub initial_context_json: Option<String>,
    pub persistent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamRun {
    pub run_id: String,
    pub session_id: String,
    pub source: String,
    pub stream_id: Option<String>,
    pub turn_id: Option<String>,
    pub status: String,
    pub accepted_seq: u64,
    pub durable_seq: u64,
    /// Highest journal sequence already represented by `sessions.context_json`.
    pub checkpoint_seq: u64,
    pub committed_seq: u64,
    pub provider_shape: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamAttempt {
    pub run_id: String,
    pub attempt_no: u32,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub provider_shape: Option<String>,
    pub status: String,
    pub accepted_seq: u64,
    pub durable_seq: u64,
    /// Highest journal sequence already represented by the provider-native
    /// context checkpoint for this attempt.
    pub checkpoint_seq: u64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEvent {
    /// Inclusive start of a coalesced text/thinking segment. Older rows and
    /// non-mergeable events omit it and therefore start at `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq_start: Option<u64>,
    /// Inclusive end/cursor sequence for this event or merged segment.
    pub seq: u64,
    pub event: String,
}

impl JournalEvent {
    pub fn single(seq: u64, event: String) -> Self {
        Self {
            seq_start: None,
            seq,
            event,
        }
    }

    pub fn range(seq_start: u64, seq_end: u64, event: String) -> Self {
        Self {
            seq_start: (seq_start != seq_end).then_some(seq_start),
            seq: seq_end,
            event,
        }
    }

    pub fn start_seq(&self) -> u64 {
        self.seq_start.unwrap_or(self.seq)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalBatch {
    pub run_id: String,
    pub attempt_no: u32,
    pub block_no: u64,
    pub seq_start: u64,
    pub seq_end: u64,
    pub events: Vec<JournalEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamJournalBlock {
    pub run_id: String,
    pub attempt_no: u32,
    pub block_no: u64,
    pub seq_start: u64,
    pub seq_end: u64,
    pub checksum: String,
    pub payload: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRunSnapshot {
    pub run: ChatStreamRun,
    pub attempts: Vec<ChatStreamAttempt>,
    pub journal: Vec<ChatStreamJournalBlock>,
    pub through_seq: u64,
}

#[derive(Debug, Clone)]
pub struct CommitAssistantTurn {
    pub run_id: Option<String>,
    pub attempt_no: u32,
    pub session_id: String,
    pub assistant: NewMessage,
    pub trailing_placeholder_id: Option<i64>,
    pub context_json: String,
    pub expected_context_revision: i64,
    pub turn_id: Option<String>,
    pub usage: Option<ModelUsageEvent>,
    pub final_seq: u64,
}

#[derive(Debug, Clone)]
pub struct CommitInterruptedTurn {
    pub run_id: Option<String>,
    pub attempt_no: u32,
    pub session_id: String,
    pub assistant: Option<NewMessage>,
    pub context_json: String,
    pub expected_context_revision: i64,
    pub turn_id: Option<String>,
    pub final_seq: u64,
    pub status: ChatTurnStatus,
    pub interrupt_reason: Option<String>,
    pub error: Option<String>,
    pub recovery_event: Option<NewMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedTurn {
    pub assistant_message_id: i64,
    pub context_revision: i64,
    pub committed_seq: u64,
    pub persistence_status: String,
}

impl SessionDB {
    pub(crate) fn ensure_stream_persistence_tables(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chat_stream_runs (
                run_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                stream_id TEXT,
                turn_id TEXT,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','interrupted','failed','committed','recovered')),
                accepted_seq INTEGER NOT NULL DEFAULT 0,
                durable_seq INTEGER NOT NULL DEFAULT 0,
                checkpoint_seq INTEGER NOT NULL DEFAULT 0,
                committed_seq INTEGER NOT NULL DEFAULT 0,
                provider_shape TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                error TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (turn_id) REFERENCES chat_turns(id) ON DELETE SET NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chat_stream_runs_session_started
                ON chat_stream_runs(session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_chat_stream_runs_status
                ON chat_stream_runs(status, started_at);

            CREATE TABLE IF NOT EXISTS chat_stream_attempts (
                run_id TEXT NOT NULL,
                attempt_no INTEGER NOT NULL,
                provider_id TEXT,
                model_id TEXT,
                provider_shape TEXT,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','superseded','failed','succeeded','interrupted','recovered')),
                accepted_seq INTEGER NOT NULL DEFAULT 0,
                durable_seq INTEGER NOT NULL DEFAULT 0,
                checkpoint_seq INTEGER NOT NULL DEFAULT 0,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                error TEXT,
                PRIMARY KEY (run_id, attempt_no),
                FOREIGN KEY (run_id) REFERENCES chat_stream_runs(run_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS chat_stream_journal (
                run_id TEXT NOT NULL,
                attempt_no INTEGER NOT NULL,
                block_no INTEGER NOT NULL,
                seq_start INTEGER NOT NULL,
                seq_end INTEGER NOT NULL,
                checksum TEXT NOT NULL,
                payload BLOB NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (run_id, attempt_no, block_no),
                FOREIGN KEY (run_id, attempt_no)
                    REFERENCES chat_stream_attempts(run_id, attempt_no) ON DELETE CASCADE
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_stream_journal_seq_start
                ON chat_stream_journal(run_id, attempt_no, seq_start);
            CREATE INDEX IF NOT EXISTS idx_chat_stream_journal_replay
                ON chat_stream_journal(run_id, attempt_no, block_no);

            CREATE TABLE IF NOT EXISTS chat_stream_context_checkpoints (
                run_id TEXT NOT NULL,
                attempt_no INTEGER NOT NULL,
                through_seq INTEGER NOT NULL,
                context_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (run_id, attempt_no, through_seq),
                FOREIGN KEY (run_id, attempt_no)
                    REFERENCES chat_stream_attempts(run_id, attempt_no) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chat_stream_context_checkpoint_recovery
                ON chat_stream_context_checkpoints(run_id, attempt_no, through_seq DESC);",
        )?;
        // These columns were added after the first additive journal migration.
        // Keep startup compatible with databases created by that prerelease.
        if conn
            .prepare("SELECT checkpoint_seq FROM chat_stream_runs LIMIT 0")
            .is_err()
        {
            conn.execute(
                "ALTER TABLE chat_stream_runs ADD COLUMN checkpoint_seq INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if conn
            .prepare("SELECT checkpoint_seq FROM chat_stream_attempts LIMIT 0")
            .is_err()
        {
            conn.execute(
                "ALTER TABLE chat_stream_attempts ADD COLUMN checkpoint_seq INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(())
    }

    /// Register one durability run. Incognito sessions return an in-memory
    /// registration and deliberately leave no row in any journal table.
    pub fn create_stream_run(&self, input: &CreateStreamRun) -> Result<StreamRunRegistration> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let session = conn
            .query_row(
                "SELECT incognito, context_revision, context_json
                 FROM sessions WHERE id = ?1",
                params![input.session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? != 0,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((incognito, context_revision, initial_context_json)) = session else {
            anyhow::bail!(
                "cannot create persistence run for missing session {}",
                input.session_id
            );
        };
        if incognito {
            return Ok(StreamRunRegistration {
                run_id: input.run_id.clone(),
                context_revision,
                initial_context_json,
                persistent: false,
            });
        }
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chat_stream_runs (
                run_id, session_id, source, stream_id, turn_id, status,
                accepted_seq, durable_seq, checkpoint_seq, committed_seq, provider_shape,
                started_at, ended_at, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'running', 0, 0, 0, 0, ?6, ?7, NULL, NULL)",
            params![
                input.run_id,
                input.session_id,
                input.source,
                input.stream_id,
                input.turn_id,
                input.provider_shape,
                now,
            ],
        )?;
        Ok(StreamRunRegistration {
            run_id: input.run_id.clone(),
            context_revision,
            initial_context_json,
            persistent: true,
        })
    }

    pub fn begin_stream_attempt(
        &self,
        run_id: &str,
        attempt_no: u32,
        provider_id: Option<&str>,
        model_id: Option<&str>,
        provider_shape: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let run_durable: i64 = conn.query_row(
            "SELECT durable_seq FROM chat_stream_runs WHERE run_id = ?1 AND status = 'running'",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO chat_stream_attempts (
                run_id, attempt_no, provider_id, model_id, provider_shape,
                status, accepted_seq, durable_seq, checkpoint_seq, started_at, ended_at, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?6, ?6, ?7, NULL, NULL)",
            params![
                run_id,
                attempt_no,
                provider_id,
                model_id,
                provider_shape,
                run_durable,
                now
            ],
        )?;
        Ok(())
    }

    /// Append one immutable batch and advance both attempt and run durability
    /// watermarks in the same short transaction.
    pub fn append_stream_journal_batch(&self, batch: &JournalBatch) -> Result<u64> {
        self.append_stream_journal_batches(std::slice::from_ref(batch))?;
        Ok(batch.seq_end)
    }

    /// Process-level writer entry: batches from many sessions share one short
    /// FULL-synchronous transaction and therefore one WAL durability barrier.
    pub fn append_stream_journal_batches(&self, batches: &[JournalBatch]) -> Result<Vec<u64>> {
        if batches.is_empty() {
            return Ok(Vec::new());
        }
        for batch in batches {
            validate_journal_batch(batch)?;
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut seqs = Vec::with_capacity(batches.len());
        for batch in batches {
            append_journal_batch_tx(&tx, batch, &now)?;
            seqs.push(batch.seq_end);
        }
        tx.commit()?;
        Ok(seqs)
    }

    pub fn supersede_stream_attempt(
        &self,
        run_id: &str,
        attempt_no: u32,
        expected_context_revision: i64,
        base_context_json: Option<&str>,
        error: Option<&str>,
    ) -> Result<i64> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        let session_id: String = tx.query_row(
            "SELECT session_id FROM chat_stream_runs
             WHERE run_id = ?1 AND status = 'running'",
            params![run_id],
            |row| row.get(0),
        )?;
        let changed = tx.execute(
            "UPDATE chat_stream_attempts
             SET status = 'superseded', ended_at = ?1, error = ?2
             WHERE run_id = ?3 AND attempt_no = ?4 AND status = 'running'",
            params![now, error, run_id, attempt_no],
        )?;
        if changed != 1 {
            anyhow::bail!("supersede attempt affected {changed} rows");
        }
        // Checkpoints are query materializations, not canonical facts until an
        // attempt wins. Remove every prior materialization for this run while
        // retaining the append-only journal as the side-effect/audit record.
        tx.execute(
            "DELETE FROM messages WHERE persistence_run_id = ?1",
            params![run_id],
        )?;
        let changed_context = tx.execute(
            "UPDATE sessions
             SET context_json = ?1, context_revision = context_revision + 1,
                 context_run_id = ?2, updated_at = ?3
             WHERE id = ?4 AND context_revision = ?5",
            params![
                base_context_json,
                run_id,
                now,
                session_id,
                expected_context_revision,
            ],
        )?;
        if changed_context != 1 {
            anyhow::bail!("context revision conflict while superseding run {run_id}");
        }
        let changed_run = tx.execute(
            "UPDATE chat_stream_runs
             SET checkpoint_seq = durable_seq
             WHERE run_id = ?1 AND status = 'running'",
            params![run_id],
        )?;
        if changed_run != 1 {
            anyhow::bail!("supersede run checkpoint update affected {changed_run} rows");
        }
        tx.commit()?;
        Ok(expected_context_revision.saturating_add(1))
    }

    /// CAS provider-native context at a semantic boundary. The journal
    /// watermark and context revision advance atomically.
    pub fn checkpoint_stream_context(
        &self,
        run_id: &str,
        attempt_no: u32,
        expected_revision: i64,
        context_json: &str,
        through_seq: u64,
    ) -> Result<i64> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let (session_id, durable_seq, source): (String, i64, String) = tx.query_row(
            "SELECT session_id, durable_seq, source FROM chat_stream_runs
             WHERE run_id = ?1 AND status = 'running'",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if through_seq > durable_seq.max(0) as u64 {
            anyhow::bail!("context checkpoint exceeds durable journal watermark");
        }
        materialize_journal_tx(
            &tx,
            run_id,
            attempt_no,
            &session_id,
            Some(&source),
            Some(through_seq),
        )?;
        let changed = tx.execute(
            "UPDATE sessions
             SET context_json = ?1,
                 context_revision = context_revision + 1,
                 context_run_id = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND context_revision = ?5",
            params![
                context_json,
                run_id,
                chrono::Utc::now().to_rfc3339(),
                session_id,
                expected_revision,
            ],
        )?;
        if changed != 1 {
            anyhow::bail!("context revision conflict for session {session_id}");
        }
        tx.execute(
            "INSERT INTO chat_stream_context_checkpoints (
                 run_id, attempt_no, through_seq, context_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id, attempt_no, through_seq) DO UPDATE SET
                 context_json = excluded.context_json,
                 created_at = excluded.created_at",
            params![
                run_id,
                attempt_no,
                through_seq as i64,
                context_json,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        let changed_attempt = tx.execute(
            "UPDATE chat_stream_attempts
             SET checkpoint_seq = ?1
             WHERE run_id = ?2 AND attempt_no = ?3 AND status = 'running'",
            params![through_seq as i64, run_id, attempt_no],
        )?;
        if changed_attempt != 1 {
            anyhow::bail!("attempt checkpoint update affected {changed_attempt} rows");
        }
        let changed_run = tx.execute(
            "UPDATE chat_stream_runs
             SET checkpoint_seq = ?1
             WHERE run_id = ?2 AND status = 'running'",
            params![through_seq as i64, run_id],
        )?;
        if changed_run != 1 {
            anyhow::bail!("run checkpoint update affected {changed_run} rows");
        }
        tx.commit()?;
        Ok(expected_revision.saturating_add(1))
    }

    /// Atomically materialize the successful assistant and every durable
    /// terminal fact. No success event may be emitted before this returns.
    pub fn commit_assistant_turn(&self, input: &CommitAssistantTurn) -> Result<CommittedTurn> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();

        let persistent = if let Some(run_id) = input.run_id.as_deref() {
            let (session_id, durable_seq, status): (String, i64, String) = tx.query_row(
                "SELECT session_id, durable_seq, status FROM chat_stream_runs WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            if session_id != input.session_id {
                anyhow::bail!("persistence run belongs to another session");
            }
            if status == "committed" {
                let assistant_message_id = tx.query_row(
                    "SELECT id FROM messages
                     WHERE persistence_run_id = ?1 AND role = 'assistant'
                     ORDER BY logical_block_seq DESC LIMIT 1",
                    params![run_id],
                    |row| row.get::<_, i64>(0),
                )?;
                let context_revision = tx.query_row(
                    "SELECT context_revision FROM sessions WHERE id = ?1",
                    params![input.session_id],
                    |row| row.get::<_, i64>(0),
                )?;
                tx.commit()?;
                return Ok(CommittedTurn {
                    assistant_message_id,
                    context_revision,
                    committed_seq: durable_seq.max(0) as u64,
                    persistence_status: "committed".to_string(),
                });
            }
            if status != "running" {
                anyhow::bail!("persistence run is not active for this commit");
            }
            if durable_seq.max(0) as u64 != input.final_seq {
                anyhow::bail!(
                    "final durability barrier incomplete: durable={}, final={}",
                    durable_seq,
                    input.final_seq
                );
            }
            true
        } else {
            false
        };

        if let Some(run_id) = input.run_id.as_deref() {
            materialize_journal_tx(
                &tx,
                run_id,
                input.attempt_no,
                &input.session_id,
                input.assistant.source.as_deref(),
                None,
            )?;
        }

        let logical_seq = i64::try_from(input.final_seq.saturating_add(1)).unwrap_or(i64::MAX);
        let assistant_id = insert_message_tx(
            &tx,
            &input.session_id,
            &input.assistant,
            input.run_id.as_deref(),
            persistent.then_some(logical_seq),
        )?;

        if let Some(placeholder_id) = input.trailing_placeholder_id {
            let changed = tx.execute(
                "DELETE FROM messages WHERE id = ?1 AND session_id = ?2",
                params![placeholder_id, input.session_id],
            )?;
            if changed != 1 {
                anyhow::bail!("trailing placeholder delete affected {changed} rows");
            }
        }

        let changed_context = tx.execute(
            "UPDATE sessions
             SET context_json = ?1,
                 context_revision = context_revision + 1,
                 context_run_id = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND context_revision = ?5",
            params![
                input.context_json,
                input.run_id,
                now,
                input.session_id,
                input.expected_context_revision,
            ],
        )?;
        if changed_context != 1 {
            anyhow::bail!("context revision conflict for session {}", input.session_id);
        }

        if let Some(turn_id) = input.turn_id.as_deref() {
            let changed_turn = tx.execute(
                "UPDATE chat_turns
                 SET status = 'completed', interrupt_reason = NULL, error = NULL,
                     assistant_message_id = ?1, ended_at = ?2, updated_at = ?2
                 WHERE id = ?3 AND session_id = ?4
                   AND status NOT IN ('completed','interrupted','failed')",
                params![assistant_id, now, turn_id, input.session_id],
            )?;
            if changed_turn != 1 {
                anyhow::bail!("chat turn completion affected {changed_turn} rows");
            }
        }

        if let Some(usage) = input.usage.as_ref().filter(|_| persistent) {
            insert_usage_tx(&tx, usage, assistant_id, &input.session_id, &now)?;
        }

        if let Some(run_id) = input.run_id.as_deref() {
            let changed_attempt = tx.execute(
                "UPDATE chat_stream_attempts
                 SET status = 'succeeded', accepted_seq = ?1, durable_seq = ?1,
                     checkpoint_seq = ?1,
                     ended_at = ?2, error = NULL
                 WHERE run_id = ?3 AND attempt_no = ?4 AND status = 'running'",
                params![input.final_seq as i64, now, run_id, input.attempt_no],
            )?;
            if changed_attempt != 1 {
                anyhow::bail!("successful attempt update affected {changed_attempt} rows");
            }
            let changed_run = tx.execute(
                "UPDATE chat_stream_runs
                 SET status = 'committed', accepted_seq = ?1, durable_seq = ?1,
                     checkpoint_seq = ?1, committed_seq = ?1, ended_at = ?2, error = NULL
                 WHERE run_id = ?3 AND status = 'running'",
                params![input.final_seq as i64, now, run_id],
            )?;
            if changed_run != 1 {
                anyhow::bail!("successful run update affected {changed_run} rows");
            }
        }

        tx.commit()?;
        drop(conn);
        self.notify_assistant_persisted(&input.session_id);
        Ok(CommittedTurn {
            assistant_message_id: assistant_id,
            context_revision: input.expected_context_revision.saturating_add(1),
            committed_seq: input.final_seq,
            persistence_status: "committed".to_string(),
        })
    }

    pub fn interrupt_stream_run(
        &self,
        run_id: &str,
        attempt_no: u32,
        status: ChatTurnStatus,
        reason: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        if !matches!(status, ChatTurnStatus::Interrupted | ChatTurnStatus::Failed) {
            anyhow::bail!("interrupt_stream_run requires interrupted or failed status");
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed_attempt = tx.execute(
            "UPDATE chat_stream_attempts
             SET status = ?1, ended_at = ?2, error = ?3
             WHERE run_id = ?4 AND attempt_no = ?5 AND status = 'running'",
            params![status.as_str(), now, error, run_id, attempt_no],
        )?;
        let expected_attempt_rows = usize::from(attempt_no > 0);
        if changed_attempt != expected_attempt_rows {
            anyhow::bail!(
                "interrupt attempt update affected {changed_attempt} rows; expected {expected_attempt_rows}"
            );
        }
        let changed = tx.execute(
            "UPDATE chat_stream_runs
             SET status = ?1, ended_at = ?2, error = ?3
             WHERE run_id = ?4 AND status = 'running'",
            params![status.as_str(), now, error, run_id],
        )?;
        if changed != 1 {
            anyhow::bail!("interrupt run update affected {changed} rows");
        }
        tx.execute(
            "UPDATE chat_turns
             SET status = ?1, interrupt_reason = COALESCE(interrupt_reason, ?2),
                 error = ?3, ended_at = COALESCE(ended_at, ?4), updated_at = ?4
             WHERE id = (SELECT turn_id FROM chat_stream_runs WHERE run_id = ?5)
               AND status NOT IN ('completed','interrupted','failed')",
            params![status.as_str(), reason, error, now, run_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Atomic convergence for Stop/provider/persistence failures which have a
    /// durable journal prefix. Unlike success, the turn can never become
    /// `completed` here.
    pub fn commit_interrupted_turn(&self, input: &CommitInterruptedTurn) -> Result<CommittedTurn> {
        if !matches!(
            input.status,
            ChatTurnStatus::Interrupted | ChatTurnStatus::Failed
        ) {
            anyhow::bail!("interrupted commit requires interrupted or failed status");
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(run_id) = input.run_id.as_deref() {
            let (durable_seq, run_status): (i64, String) = tx.query_row(
                "SELECT durable_seq, status FROM chat_stream_runs
                 WHERE run_id = ?1 AND session_id = ?2",
                params![run_id, input.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if run_status != "running" {
                if matches!(run_status.as_str(), "interrupted" | "failed" | "recovered") {
                    let assistant_message_id = tx
                        .query_row(
                            "SELECT id FROM messages
                             WHERE persistence_run_id = ?1 AND role = 'assistant'
                             ORDER BY logical_block_seq DESC LIMIT 1",
                            params![run_id],
                            |row| row.get::<_, i64>(0),
                        )
                        .optional()?
                        .unwrap_or(0);
                    let context_revision = tx.query_row(
                        "SELECT context_revision FROM sessions WHERE id = ?1",
                        params![input.session_id],
                        |row| row.get::<_, i64>(0),
                    )?;
                    tx.commit()?;
                    return Ok(CommittedTurn {
                        assistant_message_id,
                        context_revision,
                        committed_seq: durable_seq.max(0) as u64,
                        persistence_status: if run_status == "recovered" {
                            "recovered".to_string()
                        } else {
                            "committed".to_string()
                        },
                    });
                }
                anyhow::bail!("persistence run is not active for interrupted commit");
            }
            if input.final_seq > durable_seq.max(0) as u64 {
                anyhow::bail!("interrupted commit exceeds durable watermark");
            }
            // Checkpoints may already have projected blocks beyond the
            // checksum-valid prefix selected by recovery. Rebuild every row
            // owned by this run inside the terminal transaction so neither a
            // corrupt suffix nor a partially coalesced block can survive.
            tx.execute(
                "DELETE FROM messages WHERE persistence_run_id = ?1",
                params![run_id],
            )?;
            materialize_journal_tx(
                &tx,
                run_id,
                input.attempt_no,
                &input.session_id,
                input
                    .assistant
                    .as_ref()
                    .and_then(|message| message.source.as_deref()),
                Some(input.final_seq),
            )?;
        }
        let assistant_message_id = if let Some(assistant) = input.assistant.as_ref() {
            Some(insert_message_tx(
                &tx,
                &input.session_id,
                assistant,
                input.run_id.as_deref(),
                input
                    .run_id
                    .as_ref()
                    .map(|_| i64::try_from(input.final_seq.saturating_add(1)).unwrap_or(i64::MAX)),
            )?)
        } else {
            None
        };
        if let Some(event) = input.recovery_event.as_ref() {
            insert_message_tx(
                &tx,
                &input.session_id,
                event,
                input.run_id.as_deref(),
                input
                    .run_id
                    .as_ref()
                    .map(|_| i64::try_from(input.final_seq.saturating_add(2)).unwrap_or(i64::MAX)),
            )?;
        }
        let changed_context = tx.execute(
            "UPDATE sessions
             SET context_json = ?1, context_revision = context_revision + 1,
                 context_run_id = ?2, updated_at = ?3
             WHERE id = ?4 AND context_revision = ?5",
            params![
                input.context_json,
                input.run_id,
                now,
                input.session_id,
                input.expected_context_revision,
            ],
        )?;
        if changed_context != 1 {
            anyhow::bail!("context revision conflict during interrupted commit");
        }
        if let Some(turn_id) = input.turn_id.as_deref() {
            let changed = tx.execute(
                "UPDATE chat_turns
                 SET status = CASE
                         WHEN status IN ('interrupted','failed') THEN status
                         ELSE ?1
                     END,
                     interrupt_reason = COALESCE(interrupt_reason, ?2),
                     error = COALESCE(error, ?3),
                     assistant_message_id = COALESCE(?4, assistant_message_id),
                     ended_at = COALESCE(ended_at, ?5), updated_at = ?5
                 WHERE id = ?6 AND session_id = ?7
                   AND (
                       status NOT IN ('completed','interrupted','failed')
                       OR (status IN ('interrupted','failed') AND assistant_message_id IS NULL)
                   )",
                params![
                    input.status.as_str(),
                    input.interrupt_reason,
                    input.error,
                    assistant_message_id,
                    now,
                    turn_id,
                    input.session_id,
                ],
            )?;
            if changed != 1 {
                anyhow::bail!("interrupted chat turn update affected {changed} rows");
            }
        }
        if let Some(run_id) = input.run_id.as_deref() {
            let recovered = matches!(
                input.interrupt_reason.as_deref(),
                Some("crash_recovery" | "shutdown")
            );
            // `input.attempt_no` identifies the journal whose visible prefix
            // won recovery. It may already be `superseded` when a newer
            // attempt was created but crashed before making any event durable.
            // Terminalize the one live attempt, while preserving the selected
            // superseded attempt as immutable failover evidence.
            let changed_attempt = tx.execute(
                "UPDATE chat_stream_attempts
                 SET status = ?1, checkpoint_seq = ?2, ended_at = ?3, error = ?4
                 WHERE run_id = ?5 AND status = 'running'",
                params![
                    if recovered {
                        "recovered"
                    } else {
                        input.status.as_str()
                    },
                    input.final_seq as i64,
                    now,
                    input.error,
                    run_id,
                ],
            )?;
            if changed_attempt > 1 {
                anyhow::bail!("interrupted stream attempt update affected {changed_attempt} rows");
            }
            let changed = tx.execute(
                "UPDATE chat_stream_runs
                 SET status = ?1, accepted_seq = ?2, durable_seq = ?2,
                     checkpoint_seq = ?2, committed_seq = ?2, ended_at = ?3, error = ?4
                 WHERE run_id = ?5 AND status = 'running'",
                params![
                    if recovered {
                        "recovered"
                    } else {
                        input.status.as_str()
                    },
                    input.final_seq as i64,
                    now,
                    input.error,
                    run_id,
                ],
            )?;
            if changed != 1 {
                anyhow::bail!("interrupted stream run update affected {changed} rows");
            }
        }
        tx.commit()?;
        drop(conn);
        if assistant_message_id.is_some() {
            self.notify_assistant_persisted(&input.session_id);
        }
        Ok(CommittedTurn {
            assistant_message_id: assistant_message_id.unwrap_or(0),
            context_revision: input.expected_context_revision.saturating_add(1),
            committed_seq: input.final_seq,
            persistence_status: "committed".to_string(),
        })
    }

    pub fn latest_stream_run_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<StreamRunSnapshot>> {
        let conn = self.read_conn()?;
        let run = conn
            .query_row(
                "SELECT run_id, session_id, source, stream_id, turn_id, status,
                        accepted_seq, durable_seq, checkpoint_seq, committed_seq, provider_shape,
                        started_at, ended_at, error
                 FROM chat_stream_runs
                 WHERE session_id = ?1
                 ORDER BY started_at DESC LIMIT 1",
                params![session_id],
                row_to_run,
            )
            .optional()?;
        let Some(run) = run else {
            return Ok(None);
        };
        let attempts = load_attempts(&conn, &run.run_id)?;
        let journal = load_journal(&conn, &run.run_id)?;
        let mut snapshot = StreamRunSnapshot {
            run,
            attempts,
            journal,
            through_seq: 0,
        };
        snapshot.through_seq = select_recoverable_attempt_prefix(&snapshot).1;
        Ok(Some(snapshot))
    }

    /// Lightweight watermarks/status lookup for polling and stream-end
    /// envelopes. It intentionally does not read journal payload blobs.
    pub fn latest_stream_run(&self, session_id: &str) -> Result<Option<ChatStreamRun>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT run_id, session_id, source, stream_id, turn_id, status,
                    accepted_seq, durable_seq, checkpoint_seq, committed_seq, provider_shape,
                    started_at, ended_at, error
             FROM chat_stream_runs
             WHERE session_id = ?1
             ORDER BY started_at DESC LIMIT 1",
            params![session_id],
            row_to_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn stream_run_snapshot(&self, run_id: &str) -> Result<Option<StreamRunSnapshot>> {
        let conn = self.read_conn()?;
        let run = conn
            .query_row(
                "SELECT run_id, session_id, source, stream_id, turn_id, status,
                        accepted_seq, durable_seq, checkpoint_seq, committed_seq, provider_shape,
                        started_at, ended_at, error
                 FROM chat_stream_runs WHERE run_id = ?1",
                params![run_id],
                row_to_run,
            )
            .optional()?;
        let Some(run) = run else {
            return Ok(None);
        };
        let attempts = load_attempts(&conn, &run.run_id)?;
        let journal = load_journal(&conn, &run.run_id)?;
        let mut snapshot = StreamRunSnapshot {
            run,
            attempts,
            journal,
            through_seq: 0,
        };
        snapshot.through_seq = select_recoverable_attempt_prefix(&snapshot).1;
        Ok(Some(snapshot))
    }

    pub fn load_context_with_revision(&self, session_id: &str) -> Result<(Option<String>, i64)> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT context_json, context_revision FROM sessions WHERE id = ?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(Into::into)
    }

    /// Load the newest provider-native context snapshot which is wholly inside
    /// a checksum-valid journal prefix. Keeping these semantic checkpoints
    /// append-only prevents a later corrupt journal block from making the
    /// mutable `sessions.context_json` smuggle content across the detected gap.
    /// The returned revision is always the current session revision and remains
    /// the CAS guard for the recovery transaction.
    pub fn recovery_context_for_prefix(
        &self,
        run_id: &str,
        attempt_no: u32,
        through_seq: u64,
    ) -> Result<(Option<String>, u64, i64)> {
        let conn = self.read_conn()?;
        let (current_context, context_revision): (Option<String>, i64) = conn.query_row(
            "SELECT s.context_json, s.context_revision
                 FROM chat_stream_runs r
                 JOIN sessions s ON s.id = r.session_id
                 WHERE r.run_id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let checkpoint = conn
            .query_row(
                "SELECT context_json, through_seq
                 FROM chat_stream_context_checkpoints
                 WHERE run_id = ?1 AND attempt_no = ?2 AND through_seq <= ?3
                 ORDER BY through_seq DESC LIMIT 1",
                params![run_id, attempt_no, through_seq as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?.max(0) as u64,
                    ))
                },
            )
            .optional()?;
        if let Some((context, checkpoint_seq)) = checkpoint {
            return Ok((Some(context), checkpoint_seq, context_revision));
        }

        // A turn may be cancelled (or the process may start shutting down)
        // after the run row is registered but before provider construction has
        // opened attempt 1. There is no attempt checkpoint to query in that
        // state; the session context captured with the run is the complete,
        // trusted seq=0 prefix.
        if attempt_no == 0 {
            return Ok((current_context, 0, context_revision));
        }

        // Compatibility for a run created by the first journal prerelease,
        // before append-only context snapshots existed. It is safe to use the
        // mutable session context only when its recorded watermark is not past
        // the verified prefix. New runs always write a seq=0 checkpoint before
        // provider IO, so the unsafe branch is fail-closed rather than guessing.
        let checkpoint_seq: i64 = conn.query_row(
            "SELECT checkpoint_seq FROM chat_stream_attempts
             WHERE run_id = ?1 AND attempt_no = ?2",
            params![run_id, attempt_no],
            |row| row.get(0),
        )?;
        let checkpoint_seq = checkpoint_seq.max(0) as u64;
        if checkpoint_seq > through_seq {
            anyhow::bail!(
                "no trusted context checkpoint for run {run_id} attempt {attempt_no} through {through_seq}; stored checkpoint is {checkpoint_seq}"
            );
        }
        Ok((current_context, checkpoint_seq, context_revision))
    }

    /// Whether the selected journal prefix has a provider-native context
    /// checkpoint. A run can fail after its attempt row is opened but before
    /// `run_streaming_chat` writes the seq=0 user-message checkpoint; callers
    /// must then restore the prompt from their turn input instead of treating
    /// the pre-turn session context as complete.
    pub fn stream_context_checkpoint_exists(
        &self,
        run_id: &str,
        attempt_no: u32,
        through_seq: u64,
    ) -> Result<bool> {
        let conn = self.read_conn()?;
        let exists: i64 = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM chat_stream_context_checkpoints
                 WHERE run_id = ?1 AND attempt_no = ?2 AND through_seq <= ?3
             )",
            params![run_id, attempt_no, through_seq as i64],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    pub fn recoverable_stream_runs(&self) -> Result<Vec<ChatStreamRun>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT run_id, session_id, source, stream_id, turn_id, status,
                    accepted_seq, durable_seq, checkpoint_seq, committed_seq, provider_shape,
                    started_at, ended_at, error
             FROM chat_stream_runs WHERE status = 'running' ORDER BY started_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn stream_run_status(&self, run_id: &str) -> Result<Option<String>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT status FROM chat_stream_runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn mark_stream_run_recovered(
        &self,
        run_id: &str,
        through_seq: u64,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = conn.execute(
            "UPDATE chat_stream_runs
             SET status = 'recovered', accepted_seq = ?1, durable_seq = ?1,
                 checkpoint_seq = ?1, committed_seq = ?1, ended_at = ?2, error = ?3
             WHERE run_id = ?4 AND status = 'running'",
            params![through_seq as i64, now, error, run_id],
        )?;
        if changed != 1 {
            anyhow::bail!("recover run update affected {changed} rows");
        }
        Ok(())
    }

    pub fn gc_stream_journals(&self, older_than: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {e}"))?;
        conn.execute(
            "DELETE FROM chat_stream_runs
             WHERE status IN ('committed','recovered','interrupted','failed')
               AND ended_at IS NOT NULL AND ended_at < ?1",
            params![older_than],
        )
        .map_err(Into::into)
    }

    pub fn assistant_message_id_for_run(&self, run_id: &str) -> Result<Option<i64>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT id FROM messages
             WHERE persistence_run_id = ?1 AND role = 'assistant'
             ORDER BY id DESC LIMIT 1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }
}

fn validate_journal_batch(batch: &JournalBatch) -> Result<()> {
    if batch.events.is_empty() {
        anyhow::bail!("journal batch may not be empty");
    }
    if batch.seq_start == 0 || batch.seq_end < batch.seq_start {
        anyhow::bail!(
            "invalid journal seq range {}..{}",
            batch.seq_start,
            batch.seq_end
        );
    }
    if batch.events.first().map(JournalEvent::start_seq) != Some(batch.seq_start)
        || batch.events.last().map(|event| event.seq) != Some(batch.seq_end)
    {
        anyhow::bail!("journal event boundaries do not match declared seq range");
    }
    if batch
        .events
        .iter()
        .any(|event| event.start_seq() == 0 || event.start_seq() > event.seq)
        || batch
            .events
            .windows(2)
            .any(|pair| pair[1].start_seq() != pair[0].seq.saturating_add(1))
    {
        anyhow::bail!("journal events are not a continuous sequence");
    }
    Ok(())
}

fn append_journal_batch_tx(tx: &Transaction<'_>, batch: &JournalBatch, now: &str) -> Result<()> {
    let payload = serde_json::to_string(&batch.events)?;
    let checksum = blake3::hash(payload.as_bytes()).to_hex().to_string();
    let existing = tx
        .query_row(
            "SELECT checksum, seq_start, seq_end
             FROM chat_stream_journal
             WHERE run_id = ?1 AND attempt_no = ?2 AND block_no = ?3",
            params![batch.run_id, batch.attempt_no, batch.block_no as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((stored_checksum, seq_start, seq_end)) = existing {
        if stored_checksum != checksum
            || seq_start != batch.seq_start as i64
            || seq_end != batch.seq_end as i64
        {
            anyhow::bail!(
                "journal idempotency collision for run {} block {}",
                batch.run_id,
                batch.block_no
            );
        }
        return Ok(());
    }

    let prior_durable: i64 = tx.query_row(
        "SELECT durable_seq FROM chat_stream_attempts
         WHERE run_id = ?1 AND attempt_no = ?2 AND status = 'running'",
        params![batch.run_id, batch.attempt_no],
        |row| row.get(0),
    )?;
    if batch.seq_start != (prior_durable.max(0) as u64).saturating_add(1) {
        anyhow::bail!(
            "journal gap for run {} attempt {}: durable={}, incoming={}..{}",
            batch.run_id,
            batch.attempt_no,
            prior_durable,
            batch.seq_start,
            batch.seq_end
        );
    }
    tx.execute(
        "INSERT INTO chat_stream_journal (
            run_id, attempt_no, block_no, seq_start, seq_end, checksum, payload, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            batch.run_id,
            batch.attempt_no,
            batch.block_no as i64,
            batch.seq_start as i64,
            batch.seq_end as i64,
            checksum,
            payload.as_bytes(),
            now,
        ],
    )?;
    let changed_attempt = tx.execute(
        "UPDATE chat_stream_attempts
         SET accepted_seq = ?1, durable_seq = ?1
         WHERE run_id = ?2 AND attempt_no = ?3 AND status = 'running'",
        params![batch.seq_end as i64, batch.run_id, batch.attempt_no],
    )?;
    if changed_attempt != 1 {
        anyhow::bail!("attempt watermark update affected {changed_attempt} rows");
    }
    let changed_run = tx.execute(
        "UPDATE chat_stream_runs
         SET accepted_seq = ?1, durable_seq = ?1
         WHERE run_id = ?2 AND status = 'running'",
        params![batch.seq_end as i64, batch.run_id],
    )?;
    if changed_run != 1 {
        anyhow::bail!("run watermark update affected {changed_run} rows");
    }
    Ok(())
}

fn insert_message_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    msg: &NewMessage,
    persistence_run_id: Option<&str>,
    logical_block_seq: Option<i64>,
) -> Result<i64> {
    let timestamp = if msg.timestamp.is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        msg.timestamp.clone()
    };
    tx.execute(
        "INSERT OR IGNORE INTO messages (
            session_id, role, content, timestamp, attachments_meta, model,
            tokens_in, tokens_out, reasoning_effort, tool_call_id, tool_name,
            tool_arguments, tool_result, tool_duration_ms, is_error, thinking,
            ttft_ms, tokens_in_last, tokens_cache_creation, tokens_cache_read,
            tool_metadata, stream_status, source, queue_request_id,
            persistence_run_id, logical_block_seq
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                   ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
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
            msg.is_error.map(i64::from),
            msg.thinking,
            msg.ttft_ms,
            msg.tokens_in_last,
            msg.tokens_cache_creation,
            msg.tokens_cache_read,
            msg.tool_metadata,
            msg.stream_status,
            msg.source,
            msg.queue_request_id,
            persistence_run_id.or(msg.persistence_run_id.as_deref()),
            logical_block_seq.or(msg.logical_block_seq),
        ],
    )?;
    if tx.changes() == 1 {
        return Ok(tx.last_insert_rowid());
    }
    let run_id = persistence_run_id
        .or(msg.persistence_run_id.as_deref())
        .context("idempotent message insert requires persistence run id")?;
    let block_seq = logical_block_seq
        .or(msg.logical_block_seq)
        .context("idempotent message insert requires logical block seq")?;
    tx.query_row(
        "SELECT id FROM messages
         WHERE persistence_run_id = ?1 AND logical_block_seq = ?2",
        params![run_id, block_seq],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn materialize_journal_tx(
    tx: &Transaction<'_>,
    run_id: &str,
    attempt_no: u32,
    session_id: &str,
    source: Option<&str>,
    through_seq: Option<u64>,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT seq_start, seq_end, checksum, CAST(payload AS TEXT)
         FROM chat_stream_journal
         WHERE run_id = ?1 AND attempt_no = ?2
         ORDER BY block_no ASC",
    )?;
    let rows = stmt.query_map(params![run_id, attempt_no], |row| {
        Ok((
            row.get::<_, i64>(0)?.max(0) as u64,
            row.get::<_, i64>(1)?.max(0) as u64,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut events = Vec::new();
    let attempt_base: i64 = tx.query_row(
        "SELECT COALESCE(MAX(durable_seq), 0)
         FROM chat_stream_attempts
         WHERE run_id = ?1 AND attempt_no < ?2",
        params![run_id, attempt_no],
        |row| row.get(0),
    )?;
    let mut previous_seq = Some(attempt_base.max(0) as u64);
    for row in rows {
        let (seq_start, seq_end, checksum, payload) = row?;
        if through_seq.is_some_and(|through| seq_end > through) {
            break;
        }
        if blake3::hash(payload.as_bytes()).to_hex().as_str() != checksum {
            anyhow::bail!("journal checksum mismatch for run {run_id} at seq {seq_start}");
        }
        if let Some(previous) = previous_seq {
            if seq_start != previous + 1 {
                anyhow::bail!("journal gap for run {run_id}: {previous} -> {seq_start}");
            }
        }
        let batch_events: Vec<JournalEvent> = serde_json::from_str(&payload)?;
        if batch_events.first().map(JournalEvent::start_seq) != Some(seq_start)
            || batch_events.last().map(|event| event.seq) != Some(seq_end)
            || batch_events
                .iter()
                .any(|event| event.start_seq() == 0 || event.start_seq() > event.seq)
            || batch_events
                .windows(2)
                .any(|pair| pair[1].start_seq() != pair[0].seq.saturating_add(1))
        {
            anyhow::bail!("journal payload range mismatch for run {run_id}");
        }
        previous_seq = Some(seq_end);
        events.extend(batch_events);
    }
    drop(stmt);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PendingRole {
        Text,
        Thinking,
    }
    let mut pending_role: Option<PendingRole> = None;
    let mut pending_seq = 0u64;
    let mut pending_content = String::new();
    let mut tool_rows = std::collections::HashMap::<String, i64>::new();

    let flush_pending = |tx: &Transaction<'_>,
                         role: Option<PendingRole>,
                         seq: u64,
                         content: &mut String|
     -> Result<()> {
        let Some(role) = role else {
            return Ok(());
        };
        if content.is_empty() {
            return Ok(());
        }
        let mut msg = match role {
            PendingRole::Text => NewMessage::text_block(content),
            PendingRole::Thinking => NewMessage::thinking_block(content),
        };
        msg.stream_status = Some("completed".to_string());
        msg.source = source.map(ToOwned::to_owned);
        let id = insert_message_tx(
            tx,
            session_id,
            &msg,
            Some(run_id),
            Some(i64::try_from(seq).unwrap_or(i64::MAX)),
        )?;
        // A checkpoint can observe a still-growing trailing thinking block.
        // Its logical start seq remains stable, so refresh the query
        // projection in place; the append-only journal remains the truth.
        let changed = tx.execute(
            "UPDATE messages SET content = ?1, stream_status = 'completed'
             WHERE id = ?2 AND persistence_run_id = ?3",
            params![content.as_str(), id, run_id],
        )?;
        if changed != 1 {
            anyhow::bail!("stream block projection update affected {changed} rows");
        }
        content.clear();
        Ok(())
    };

    for journal_event in events {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&journal_event.event) else {
            anyhow::bail!("invalid journal event JSON at seq {}", journal_event.seq);
        };
        match event.get("type").and_then(|value| value.as_str()) {
            Some("text_delta") => {
                if pending_role == Some(PendingRole::Thinking) {
                    flush_pending(tx, pending_role, pending_seq, &mut pending_content)?;
                    pending_role = None;
                }
                if pending_role.is_none() {
                    pending_role = Some(PendingRole::Text);
                    pending_seq = journal_event.start_seq();
                }
                if let Some(content) = event.get("content").and_then(|value| value.as_str()) {
                    pending_content.push_str(content);
                }
            }
            Some("thinking_delta") => {
                if pending_role == Some(PendingRole::Text) {
                    flush_pending(tx, pending_role, pending_seq, &mut pending_content)?;
                    pending_role = None;
                }
                if pending_role.is_none() {
                    pending_role = Some(PendingRole::Thinking);
                    pending_seq = journal_event.start_seq();
                }
                if let Some(content) = event.get("content").and_then(|value| value.as_str()) {
                    pending_content.push_str(content);
                }
            }
            Some("tool_call") => {
                flush_pending(tx, pending_role, pending_seq, &mut pending_content)?;
                pending_role = None;
                let call_id = event
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let name = event
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let arguments = event
                    .get("arguments")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let mut msg = NewMessage::tool(call_id, name, arguments, "", None, false);
                msg.stream_status = Some("streaming".to_string());
                msg.source = source.map(ToOwned::to_owned);
                let id = insert_message_tx(
                    tx,
                    session_id,
                    &msg,
                    Some(run_id),
                    Some(i64::try_from(journal_event.start_seq()).unwrap_or(i64::MAX)),
                )?;
                tool_rows.insert(call_id.to_string(), id);
            }
            Some("tool_call_args_rewritten") => {
                let call_id = event
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let arguments = event
                    .get("arguments")
                    .or_else(|| event.get("effective_arguments"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if let Some(id) = tool_rows.get(call_id) {
                    tx.execute(
                        "UPDATE messages SET tool_arguments = ?1 WHERE id = ?2",
                        params![arguments, id],
                    )?;
                }
            }
            Some("tool_result") => {
                let call_id = event
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let result = event
                    .get("result")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let duration_ms = event.get("duration_ms").and_then(|value| value.as_i64());
                let is_error = event
                    .get("is_error")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let metadata = event
                    .get("tool_metadata")
                    .filter(|value| !value.is_null())
                    .map(serde_json::to_string)
                    .transpose()?;
                let attachments = event
                    .get("media_items")
                    .and_then(super::build_tool_media_items_attachments_meta);
                let id = tool_rows
                    .get(call_id)
                    .copied()
                    .context("tool_result has no durable tool_call")?;
                let changed = tx.execute(
                    "UPDATE messages
                     SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3,
                         tool_metadata = COALESCE(?4, tool_metadata),
                         attachments_meta = COALESCE(?5, attachments_meta),
                         stream_status = 'completed'
                     WHERE id = ?6",
                    params![
                        result,
                        duration_ms,
                        i64::from(is_error),
                        metadata,
                        attachments,
                        id,
                    ],
                )?;
                if changed != 1 {
                    anyhow::bail!("tool result materialization affected {changed} rows");
                }
            }
            Some(
                "round_limit_reached"
                | "context_compacted"
                | "model_fallback"
                | "profile_rotation"
                | "codex_auth_expired"
                | "thinking_auto_disabled"
                | "vision_auto_disabled"
                | "vision_bridge",
            ) => {
                let mut msg = NewMessage::event(&journal_event.event);
                msg.source = source.map(ToOwned::to_owned);
                insert_message_tx(
                    tx,
                    session_id,
                    &msg,
                    Some(run_id),
                    Some(i64::try_from(journal_event.start_seq()).unwrap_or(i64::MAX)),
                )?;
            }
            _ => {}
        }
    }

    // The final text segment is written into the canonical assistant row by
    // the caller. Thinking remains a separate ordered block.
    if pending_role == Some(PendingRole::Thinking) {
        flush_pending(tx, pending_role, pending_seq, &mut pending_content)?;
    }
    Ok(())
}

fn insert_usage_tx(
    tx: &Transaction<'_>,
    event: &ModelUsageEvent,
    assistant_id: i64,
    session_id: &str,
    fallback_timestamp: &str,
) -> Result<()> {
    if event.kind.trim().is_empty() {
        return Ok(());
    }
    let metadata = event
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let clamp = |value: Option<u64>| value.map(|n| n.min(i64::MAX as u64) as i64);
    tx.execute(
        "INSERT INTO model_usage_events (
            request_key, timestamp, kind, operation, source, provider_id,
            provider_name, model_id, session_id, agent_id, input_tokens,
            output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
            context_input_tokens, fresh_input_tokens, duration_ms, ttft_ms,
            success, error, metadata
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                   ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            event
                .request_key
                .clone()
                .unwrap_or_else(|| format!("message:{assistant_id}")),
            event.timestamp.as_deref().unwrap_or(fallback_timestamp),
            event.kind,
            event.operation,
            event.source,
            event.provider_id,
            event.provider_name,
            event.model_id,
            session_id,
            event.agent_id,
            clamp(event.input_tokens),
            clamp(event.output_tokens),
            clamp(event.cache_creation_input_tokens),
            clamp(event.cache_read_input_tokens),
            clamp(event.context_input_tokens),
            clamp(event.fresh_input_tokens),
            clamp(event.duration_ms),
            clamp(event.ttft_ms),
            i64::from(event.success),
            event.error,
            metadata,
        ],
    )?;
    Ok(())
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatStreamRun> {
    Ok(ChatStreamRun {
        run_id: row.get(0)?,
        session_id: row.get(1)?,
        source: row.get(2)?,
        stream_id: row.get(3)?,
        turn_id: row.get(4)?,
        status: row.get(5)?,
        accepted_seq: row.get::<_, i64>(6)?.max(0) as u64,
        durable_seq: row.get::<_, i64>(7)?.max(0) as u64,
        checkpoint_seq: row.get::<_, i64>(8)?.max(0) as u64,
        committed_seq: row.get::<_, i64>(9)?.max(0) as u64,
        provider_shape: row.get(10)?,
        started_at: row.get(11)?,
        ended_at: row.get(12)?,
        error: row.get(13)?,
    })
}

fn load_attempts(conn: &rusqlite::Connection, run_id: &str) -> Result<Vec<ChatStreamAttempt>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, attempt_no, provider_id, model_id, provider_shape,
                status, accepted_seq, durable_seq, checkpoint_seq, started_at, ended_at, error
         FROM chat_stream_attempts WHERE run_id = ?1 ORDER BY attempt_no ASC",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(ChatStreamAttempt {
            run_id: row.get(0)?,
            attempt_no: row.get::<_, i64>(1)?.max(0) as u32,
            provider_id: row.get(2)?,
            model_id: row.get(3)?,
            provider_shape: row.get(4)?,
            status: row.get(5)?,
            accepted_seq: row.get::<_, i64>(6)?.max(0) as u64,
            durable_seq: row.get::<_, i64>(7)?.max(0) as u64,
            checkpoint_seq: row.get::<_, i64>(8)?.max(0) as u64,
            started_at: row.get(9)?,
            ended_at: row.get(10)?,
            error: row.get(11)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn load_journal(conn: &rusqlite::Connection, run_id: &str) -> Result<Vec<ChatStreamJournalBlock>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, attempt_no, block_no, seq_start, seq_end, checksum,
                CAST(payload AS TEXT), created_at
         FROM chat_stream_journal WHERE run_id = ?1
         ORDER BY attempt_no ASC, block_no ASC",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(ChatStreamJournalBlock {
            run_id: row.get(0)?,
            attempt_no: row.get::<_, i64>(1)?.max(0) as u32,
            block_no: row.get::<_, i64>(2)?.max(0) as u64,
            seq_start: row.get::<_, i64>(3)?.max(0) as u64,
            seq_end: row.get::<_, i64>(4)?.max(0) as u64,
            checksum: row.get(5)?,
            payload: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn verify_block(block: &ChatStreamJournalBlock) -> bool {
    blake3::hash(block.payload.as_bytes()).to_hex().as_str() == block.checksum
}

/// Resolve which durable events are already represented by the currently
/// stored provider-native context for the selected recovery attempt.
///
/// A superseded attempt's own checkpoint was rolled back atomically before
/// the next profile started, so its replay base is the previous attempt's end
/// rather than its stale checkpoint watermark.
pub fn stream_attempt_context_checkpoint(snapshot: &StreamRunSnapshot, attempt_no: u32) -> u64 {
    let Some(attempt) = snapshot
        .attempts
        .iter()
        .find(|attempt| attempt.attempt_no == attempt_no)
    else {
        return 0;
    };
    if attempt.status == "superseded" {
        snapshot
            .attempts
            .iter()
            .filter(|prior| prior.attempt_no < attempt_no)
            .map(|prior| prior.durable_seq)
            .max()
            .unwrap_or(0)
    } else {
        attempt.checkpoint_seq
    }
}

/// Select the newest attempt which made any event durable and return only its
/// largest checksum-valid, sequence-continuous prefix. If a newer attempt was
/// created but died before its reset marker/delta became durable, the previous
/// visible attempt remains authoritative.
pub fn select_recoverable_attempt_prefix(
    snapshot: &StreamRunSnapshot,
) -> (u32, u64, Vec<JournalEvent>, Option<String>) {
    let mut attempt_nos = snapshot
        .journal
        .iter()
        .map(|block| block.attempt_no)
        .chain(snapshot.attempts.iter().map(|attempt| attempt.attempt_no))
        .collect::<Vec<_>>();
    attempt_nos.sort_unstable();
    attempt_nos.dedup();
    attempt_nos.reverse();

    let fallback_attempt = attempt_nos.first().copied().unwrap_or(0);
    let mut fallback = None;
    for attempt_no in attempt_nos {
        let candidate = recoverable_attempt_prefix(snapshot, attempt_no);
        if fallback.is_none() {
            fallback = Some(candidate.clone());
        }
        // A newly opened failover attempt always contains a reset/fallback
        // marker, even if the provider fails before producing anything the
        // user can keep. Such bookkeeping must not hide the newest prior
        // attempt that actually contained visible partial output.
        if journal_events_have_visible_output(&candidate.2) {
            return candidate;
        }
    }

    fallback.unwrap_or((fallback_attempt, 0, Vec::new(), None))
}

fn recoverable_attempt_prefix(
    snapshot: &StreamRunSnapshot,
    attempt_no: u32,
) -> (u32, u64, Vec<JournalEvent>, Option<String>) {
    let attempt_base = snapshot
        .attempts
        .iter()
        .filter(|attempt| attempt.attempt_no < attempt_no)
        .map(|attempt| attempt.durable_seq)
        .max()
        .unwrap_or(0);
    let mut previous = attempt_base;
    let mut through = attempt_base;
    let mut events = Vec::new();
    let mut integrity_error = None;

    for block in snapshot
        .journal
        .iter()
        .filter(|block| block.attempt_no == attempt_no)
    {
        if !verify_block(block) {
            integrity_error = Some(format!(
                "journal checksum mismatch run={} seq={}..{}",
                snapshot.run.run_id, block.seq_start, block.seq_end
            ));
            break;
        }
        if block.seq_start != previous.saturating_add(1) {
            integrity_error = Some(format!(
                "journal sequence gap run={} after_seq={} next_seq={}",
                snapshot.run.run_id, previous, block.seq_start
            ));
            break;
        }
        let batch: Vec<JournalEvent> = match serde_json::from_str(&block.payload) {
            Ok(batch) => batch,
            Err(error) => {
                integrity_error = Some(format!(
                    "journal payload invalid run={} seq={} error={}",
                    snapshot.run.run_id, block.seq_start, error
                ));
                break;
            }
        };
        if batch.first().map(JournalEvent::start_seq) != Some(block.seq_start)
            || batch.last().map(|event| event.seq) != Some(block.seq_end)
            || batch
                .iter()
                .any(|event| event.start_seq() == 0 || event.start_seq() > event.seq)
            || batch
                .windows(2)
                .any(|pair| pair[1].start_seq() != pair[0].seq.saturating_add(1))
        {
            integrity_error = Some(format!(
                "journal payload range mismatch run={} seq={}..{}",
                snapshot.run.run_id, block.seq_start, block.seq_end
            ));
            break;
        }
        previous = block.seq_end;
        through = block.seq_end;
        events.extend(batch);
    }

    (attempt_no, through, events, integrity_error)
}

fn journal_events_have_visible_output(events: &[JournalEvent]) -> bool {
    events.iter().any(|journal_event| {
        serde_json::from_str::<serde_json::Value>(&journal_event.event)
            .ok()
            .and_then(|event| {
                let event_type = event.get("type")?.as_str()?;
                Some(match event_type {
                    "text_delta" | "thinking_delta" => event
                        .get("content")
                        .and_then(|value| value.as_str())
                        .is_some_and(|content| !content.is_empty()),
                    "tool_call" | "tool_result" => true,
                    _ => false,
                })
            })
            .unwrap_or(false)
    })
}

pub fn trailing_text_from_journal_events(events: &[JournalEvent]) -> String {
    let mut text = String::new();
    for journal_event in events {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&journal_event.event) else {
            continue;
        };
        match event.get("type").and_then(|value| value.as_str()) {
            Some("tool_call" | "thinking_delta") => text.clear(),
            Some("text_delta") => {
                if let Some(content) = event.get("content").and_then(|value| value.as_str()) {
                    text.push_str(content);
                }
            }
            _ => {}
        }
    }
    text
}

pub fn journal_events_have_assistant_output(events: &[JournalEvent]) -> bool {
    events.iter().any(|journal_event| {
        serde_json::from_str::<serde_json::Value>(&journal_event.event)
            .ok()
            .is_some_and(|event| {
                matches!(
                    event.get("type").and_then(|value| value.as_str()),
                    Some("text_delta" | "thinking_delta")
                ) && event
                    .get("content")
                    .and_then(|value| value.as_str())
                    .is_some_and(|content| !content.is_empty())
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RunFixture {
        _dir: tempfile::TempDir,
        db: SessionDB,
        session_id: String,
        turn_id: String,
        run_id: String,
        context_revision: i64,
        final_seq: u64,
    }

    fn fixture(tag: &str) -> RunFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join(format!("{tag}.db"))).expect("open db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let user_id = db
            .append_message(&session.id, &NewMessage::user("hello"))
            .expect("insert user");
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("stream-1"), Some(user_id))
            .expect("create turn");
        let run_id = uuid::Uuid::new_v4().to_string();
        let registration = db
            .create_stream_run(&CreateStreamRun {
                run_id: run_id.clone(),
                session_id: session.id.clone(),
                source: "desktop".to_string(),
                stream_id: Some("stream-1".to_string()),
                turn_id: Some(turn.id.clone()),
                provider_shape: Some("anthropic".to_string()),
            })
            .expect("create run");
        db.begin_stream_attempt(&run_id, 1, Some("p"), Some("m"), Some("anthropic"))
            .expect("begin attempt");
        let events = vec![
            JournalEvent {
                seq_start: None,
                seq: 1,
                event: serde_json::json!({"type":"text_delta","content":"before "}).to_string(),
            },
            JournalEvent {
                seq_start: None,
                seq: 2,
                event: serde_json::json!({
                    "type":"tool_call","call_id":"call-1","name":"read_file","arguments":"{}"
                })
                .to_string(),
            },
            JournalEvent {
                seq_start: None,
                seq: 3,
                event: serde_json::json!({
                    "type":"tool_result","call_id":"call-1","result":"ok","duration_ms":7,
                    "is_error":false
                })
                .to_string(),
            },
            JournalEvent {
                seq_start: None,
                seq: 4,
                event: serde_json::json!({"type":"thinking_delta","content":"reason"}).to_string(),
            },
            JournalEvent {
                seq_start: None,
                seq: 5,
                event: serde_json::json!({"type":"text_delta","content":"after"}).to_string(),
            },
        ];
        db.append_stream_journal_batch(&JournalBatch {
            run_id: run_id.clone(),
            attempt_no: 1,
            block_no: 1,
            seq_start: 1,
            seq_end: 5,
            events,
        })
        .expect("append journal");
        RunFixture {
            _dir: dir,
            db,
            session_id: session.id,
            turn_id: turn.id,
            run_id,
            context_revision: registration.context_revision,
            final_seq: 5,
        }
    }

    fn success_commit(fixture: &RunFixture, placeholder_id: Option<i64>) -> CommitAssistantTurn {
        let mut usage = ModelUsageEvent::new(crate::model_usage::KIND_CHAT).with_usage(11, 7, 0, 0);
        usage.session_id = Some(fixture.session_id.clone());
        usage.model_id = Some("m".to_string());
        CommitAssistantTurn {
            run_id: Some(fixture.run_id.clone()),
            attempt_no: 1,
            session_id: fixture.session_id.clone(),
            assistant: NewMessage::assistant("after"),
            trailing_placeholder_id: placeholder_id,
            context_json: r#"[{"role":"assistant","content":"after"}]"#.to_string(),
            expected_context_revision: fixture.context_revision,
            turn_id: Some(fixture.turn_id.clone()),
            usage: Some(usage),
            final_seq: fixture.final_seq,
        }
    }

    fn scalar_i64(db: &SessionDB, sql: &str, arg: &str) -> i64 {
        let conn = db.conn.lock().expect("db lock");
        conn.query_row(sql, params![arg], |row| row.get(0))
            .expect("scalar")
    }

    fn assert_success_rollback(fixture: &RunFixture, expected_initial_messages: i64) {
        assert_eq!(
            scalar_i64(
                &fixture.db,
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                &fixture.session_id,
            ),
            expected_initial_messages
        );
        let conn = fixture.db.conn.lock().expect("db lock");
        let (context, revision): (Option<String>, i64) = conn
            .query_row(
                "SELECT context_json, context_revision FROM sessions WHERE id = ?1",
                params![fixture.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("session state");
        assert!(context.is_none());
        assert_eq!(revision, 0);
        let turn_status: String = conn
            .query_row(
                "SELECT status FROM chat_turns WHERE id = ?1",
                params![fixture.turn_id],
                |row| row.get(0),
            )
            .expect("turn status");
        assert_eq!(turn_status, "running");
        let run_status: String = conn
            .query_row(
                "SELECT status FROM chat_stream_runs WHERE run_id = ?1",
                params![fixture.run_id],
                |row| row.get(0),
            )
            .expect("run status");
        assert_eq!(run_status, "running");
        let usage_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM model_usage_events", [], |row| {
                row.get(0)
            })
            .expect("usage count");
        assert_eq!(usage_count, 0);
    }

    #[test]
    fn final_commit_materializes_once_and_replay_is_idempotent() {
        let fixture = fixture("idempotent");
        let input = success_commit(&fixture, None);
        let first = fixture
            .db
            .commit_assistant_turn(&input)
            .expect("first commit");
        let second = fixture
            .db
            .commit_assistant_turn(&input)
            .expect("idempotent replay");
        assert_eq!(first.assistant_message_id, second.assistant_message_id);
        assert_eq!(first.committed_seq, 5);
        assert_eq!(
            scalar_i64(
                &fixture.db,
                "SELECT COUNT(*) FROM messages WHERE persistence_run_id = ?1",
                &fixture.run_id,
            ),
            4,
            "text + tool + thinking + final assistant"
        );
        assert_eq!(
            scalar_i64(
                &fixture.db,
                "SELECT COUNT(*) FROM model_usage_events WHERE session_id = ?1",
                &fixture.session_id,
            ),
            1
        );
        let turn = fixture
            .db
            .get_chat_turn(&fixture.turn_id)
            .expect("turn")
            .expect("turn exists");
        assert_eq!(turn.status, ChatTurnStatus::Completed);
        assert_eq!(turn.assistant_message_id, Some(first.assistant_message_id));
    }

    #[test]
    fn every_final_sql_failpoint_rolls_back_the_whole_turn() {
        let failpoints = [
            (
                "assistant_insert",
                "CREATE TRIGGER failpoint BEFORE INSERT ON messages
                 WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'assistant'); END;",
            ),
            (
                "context_update",
                "CREATE TRIGGER failpoint BEFORE UPDATE OF context_json ON sessions
                 BEGIN SELECT RAISE(ABORT, 'context'); END;",
            ),
            (
                "turn_update",
                "CREATE TRIGGER failpoint BEFORE UPDATE OF status ON chat_turns
                 WHEN NEW.status = 'completed' BEGIN SELECT RAISE(ABORT, 'turn'); END;",
            ),
            (
                "usage_insert",
                "CREATE TRIGGER failpoint BEFORE INSERT ON model_usage_events
                 BEGIN SELECT RAISE(ABORT, 'usage'); END;",
            ),
            (
                "run_update",
                "CREATE TRIGGER failpoint BEFORE UPDATE OF status ON chat_stream_runs
                 WHEN NEW.status = 'committed' BEGIN SELECT RAISE(ABORT, 'run'); END;",
            ),
        ];
        for (name, trigger) in failpoints {
            let fixture = fixture(name);
            fixture
                .db
                .conn
                .lock()
                .expect("db lock")
                .execute_batch(trigger)
                .expect("install failpoint");
            fixture
                .db
                .commit_assistant_turn(&success_commit(&fixture, None))
                .expect_err("failpoint must abort commit");
            assert_success_rollback(&fixture, 1);
        }
    }

    #[test]
    fn placeholder_delete_and_later_failure_are_both_rolled_back() {
        for fail_after_delete in [false, true] {
            let fixture = fixture(if fail_after_delete {
                "after-placeholder-delete"
            } else {
                "placeholder-delete"
            });
            let placeholder_id = fixture
                .db
                .append_message(&fixture.session_id, &NewMessage::text_block("tail"))
                .expect("placeholder");
            let trigger = if fail_after_delete {
                "CREATE TRIGGER failpoint BEFORE UPDATE OF context_json ON sessions
                 BEGIN SELECT RAISE(ABORT, 'after delete'); END;"
            } else {
                "CREATE TRIGGER failpoint BEFORE DELETE ON messages
                 WHEN OLD.id > 0 BEGIN SELECT RAISE(ABORT, 'delete'); END;"
            };
            fixture
                .db
                .conn
                .lock()
                .expect("db lock")
                .execute_batch(trigger)
                .expect("install failpoint");
            fixture
                .db
                .commit_assistant_turn(&success_commit(&fixture, Some(placeholder_id)))
                .expect_err("commit must roll back");
            assert_success_rollback(&fixture, 2);
            assert_eq!(
                scalar_i64(
                    &fixture.db,
                    "SELECT COUNT(*) FROM messages WHERE id = ?1",
                    &placeholder_id.to_string(),
                ),
                1
            );
        }
    }

    #[test]
    fn stale_context_revision_rejects_final_commit() {
        let fixture = fixture("context-revision");
        fixture
            .db
            .save_context_at_revision(&fixture.session_id, 0, r#"["newer"]"#, None)
            .expect("newer context");
        fixture
            .db
            .commit_assistant_turn(&success_commit(&fixture, None))
            .expect_err("stale finalizer must fail closed");
        assert_eq!(
            scalar_i64(
                &fixture.db,
                "SELECT COUNT(*) FROM messages WHERE persistence_run_id = ?1",
                &fixture.run_id,
            ),
            0
        );
        assert_eq!(
            fixture
                .db
                .load_context(&fixture.session_id)
                .expect("context")
                .as_deref(),
            Some(r#"["newer"]"#)
        );
    }

    #[test]
    fn superseded_attempt_never_materializes_into_messages() {
        let fixture = fixture("failover");
        let checkpoint_revision = fixture
            .db
            .checkpoint_stream_context(
                &fixture.run_id,
                1,
                fixture.context_revision,
                r#"[{"role":"assistant","content":"attempt one"}]"#,
                5,
            )
            .expect("attempt one checkpoint");
        assert_eq!(
            scalar_i64(
                &fixture.db,
                "SELECT COUNT(*) FROM messages
                 WHERE persistence_run_id = ?1 AND content LIKE '%before%'",
                &fixture.run_id,
            ),
            1,
            "checkpoint materializes the active attempt"
        );
        let winning_revision = fixture
            .db
            .supersede_stream_attempt(&fixture.run_id, 1, checkpoint_revision, None, Some("retry"))
            .expect("supersede");
        let rolled_back = fixture
            .db
            .stream_run_snapshot(&fixture.run_id)
            .expect("snapshot")
            .expect("run");
        assert_eq!(
            stream_attempt_context_checkpoint(&rolled_back, 1),
            0,
            "superseded attempt must replay from its pre-attempt context"
        );
        fixture
            .db
            .begin_stream_attempt(
                &fixture.run_id,
                2,
                Some("p2"),
                Some("m2"),
                Some("openai_chat"),
            )
            .expect("attempt 2");
        fixture
            .db
            .append_stream_journal_batch(&JournalBatch {
                run_id: fixture.run_id.clone(),
                attempt_no: 2,
                block_no: 1,
                seq_start: 6,
                seq_end: 7,
                events: vec![
                    JournalEvent {
                        seq_start: None,
                        seq: 6,
                        event: serde_json::json!({
                            "type":"stream_attempt_started","attempt_no":2,
                            "reset_superseded":true
                        })
                        .to_string(),
                    },
                    JournalEvent {
                        seq_start: None,
                        seq: 7,
                        event: serde_json::json!({"type":"text_delta","content":"winner"})
                            .to_string(),
                    },
                ],
            })
            .expect("attempt 2 journal");
        let mut input = success_commit(&fixture, None);
        input.attempt_no = 2;
        input.final_seq = 7;
        input.assistant = NewMessage::assistant("winner");
        input.expected_context_revision = winning_revision;
        fixture
            .db
            .commit_assistant_turn(&input)
            .expect("commit winner");
        let conn = fixture.db.conn.lock().expect("db lock");
        let old_visible: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE persistence_run_id = ?1 AND content LIKE '%before%'",
                params![fixture.run_id],
                |row| row.get(0),
            )
            .expect("old visible count");
        assert_eq!(old_visible, 0);
        let journal_attempts: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT attempt_no) FROM chat_stream_journal WHERE run_id = ?1",
                params![fixture.run_id],
                |row| row.get(0),
            )
            .expect("journal attempts");
        assert_eq!(journal_attempts, 2, "superseded bytes remain for retention");
    }

    #[test]
    fn failed_attempt_without_a_durable_reset_keeps_previous_visible_prefix() {
        let fixture = fixture("failover-before-first-delta");
        let revision = fixture
            .db
            .supersede_stream_attempt(
                &fixture.run_id,
                1,
                fixture.context_revision,
                None,
                Some("retry"),
            )
            .expect("supersede attempt one");
        assert_eq!(revision, 1);
        fixture
            .db
            .begin_stream_attempt(
                &fixture.run_id,
                2,
                Some("p2"),
                Some("m2"),
                Some("openai_chat"),
            )
            .expect("begin attempt two");

        let snapshot = fixture
            .db
            .stream_run_snapshot(&fixture.run_id)
            .expect("snapshot")
            .expect("run");
        let (attempt_no, through_seq, events, integrity_error) =
            select_recoverable_attempt_prefix(&snapshot);
        assert_eq!(attempt_no, 1);
        assert_eq!(through_seq, fixture.final_seq);
        assert_eq!(events.len(), 5);
        assert!(integrity_error.is_none());

        let (_, _, current_revision) = fixture
            .db
            .recovery_context_for_prefix(&fixture.run_id, attempt_no, through_seq)
            .expect("recovery context");
        fixture
            .db
            .commit_interrupted_turn(&CommitInterruptedTurn {
                run_id: Some(fixture.run_id.clone()),
                attempt_no,
                session_id: fixture.session_id.clone(),
                assistant: Some(NewMessage::assistant("after")),
                context_json: "[]".to_string(),
                expected_context_revision: current_revision,
                turn_id: Some(fixture.turn_id.clone()),
                final_seq: through_seq,
                status: ChatTurnStatus::Failed,
                interrupt_reason: Some("provider_failed".to_string()),
                error: Some("attempt two failed before output".to_string()),
                recovery_event: None,
            })
            .expect("converge from prior visible attempt");
        let terminal = fixture
            .db
            .stream_run_snapshot(&fixture.run_id)
            .expect("terminal snapshot")
            .expect("run");
        assert_eq!(terminal.run.status, "failed");
        assert_eq!(terminal.attempts[0].status, "superseded");
        assert_eq!(terminal.attempts[1].status, "failed");
    }

    #[test]
    fn interrupted_commit_refuses_to_overwrite_a_completed_turn() {
        let fixture = fixture("premature-terminal-turn");
        fixture
            .db
            .finish_chat_turn_once(
                &fixture.turn_id,
                ChatTurnStatus::Completed,
                None,
                None,
                None,
            )
            .expect("premature terminal status");
        let commit = CommitInterruptedTurn {
            run_id: Some(fixture.run_id.clone()),
            attempt_no: 1,
            session_id: fixture.session_id.clone(),
            assistant: Some(NewMessage::assistant("after")),
            context_json: r#"[{"role":"assistant","content":"after"}]"#.to_string(),
            expected_context_revision: fixture.context_revision,
            turn_id: Some(fixture.turn_id.clone()),
            final_seq: fixture.final_seq,
            status: ChatTurnStatus::Interrupted,
            interrupt_reason: Some("user_stop".to_string()),
            error: None,
            recovery_event: None,
        };
        fixture
            .db
            .commit_interrupted_turn(&commit)
            .expect_err("completed turn is an immutable success fact");
        let turn = fixture
            .db
            .get_chat_turn(&fixture.turn_id)
            .expect("turn")
            .expect("turn exists");
        assert_eq!(turn.status, ChatTurnStatus::Completed);
        assert_eq!(turn.assistant_message_id, None);
        let run = fixture
            .db
            .stream_run_snapshot(&fixture.run_id)
            .expect("snapshot")
            .expect("run");
        assert_eq!(run.run.status, "running");
    }

    #[test]
    fn randomized_journal_replay_matches_reference_and_is_ten_times_idempotent() {
        for seed in 1..=12u64 {
            let dir = tempfile::tempdir().expect("tempdir");
            let db =
                SessionDB::open(&dir.path().join(format!("property-{seed}.db"))).expect("open db");
            let session = db
                .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
                .expect("session");
            let run_id = uuid::Uuid::new_v4().to_string();
            let registration = db
                .create_stream_run(&CreateStreamRun {
                    run_id: run_id.clone(),
                    session_id: session.id.clone(),
                    source: "desktop".to_string(),
                    stream_id: Some(format!("property-{seed}")),
                    turn_id: None,
                    provider_shape: Some("anthropic".to_string()),
                })
                .expect("run");
            db.begin_stream_attempt(&run_id, 1, Some("p"), Some("m"), Some("anthropic"))
                .expect("attempt");

            let mut random = seed;
            let mut seq = 0u64;
            let mut events = Vec::<JournalEvent>::new();
            let mut expected = Vec::<(String, String, String)>::new();
            let mut pending_role: Option<&'static str> = None;
            let mut pending_content = String::new();
            let flush_reference =
                |role: &mut Option<&'static str>,
                 content: &mut String,
                 output: &mut Vec<(String, String, String)>| {
                    if let Some(role) = role.take() {
                        if !content.is_empty() {
                            output.push((role.to_string(), std::mem::take(content), String::new()));
                        }
                    }
                };

            for step in 0..48u64 {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                match random % 3 {
                    0 => {
                        if pending_role == Some("thinking_block") {
                            flush_reference(&mut pending_role, &mut pending_content, &mut expected);
                        }
                        pending_role.get_or_insert("text_block");
                        let content = format!("t{seed}:{step}|");
                        pending_content.push_str(&content);
                        seq += 1;
                        events.push(JournalEvent {
                            seq_start: None,
                            seq,
                            event: serde_json::json!({
                                "type": "text_delta", "content": content
                            })
                            .to_string(),
                        });
                    }
                    1 => {
                        if pending_role == Some("text_block") {
                            flush_reference(&mut pending_role, &mut pending_content, &mut expected);
                        }
                        pending_role.get_or_insert("thinking_block");
                        let content = format!("h{seed}:{step}|");
                        pending_content.push_str(&content);
                        seq += 1;
                        events.push(JournalEvent {
                            seq_start: None,
                            seq,
                            event: serde_json::json!({
                                "type": "thinking_delta", "content": content
                            })
                            .to_string(),
                        });
                    }
                    _ => {
                        flush_reference(&mut pending_role, &mut pending_content, &mut expected);
                        let call_id = format!("call-{seed}-{step}");
                        let result = format!("result-{seed}-{step}");
                        expected.push(("tool".to_string(), String::new(), result.clone()));
                        seq += 1;
                        events.push(JournalEvent {
                            seq_start: None,
                            seq,
                            event: serde_json::json!({
                                "type": "tool_call", "call_id": call_id,
                                "name": "fixture", "arguments": "{}"
                            })
                            .to_string(),
                        });
                        seq += 1;
                        events.push(JournalEvent {
                            seq_start: None,
                            seq,
                            event: serde_json::json!({
                                "type": "tool_result", "call_id": call_id,
                                "result": result, "is_error": false
                            })
                            .to_string(),
                        });
                    }
                }
            }
            let final_assistant = if pending_role == Some("text_block") {
                std::mem::take(&mut pending_content)
            } else {
                flush_reference(&mut pending_role, &mut pending_content, &mut expected);
                String::new()
            };
            expected.push((
                "assistant".to_string(),
                final_assistant.clone(),
                String::new(),
            ));

            let mut offset = 0usize;
            let mut block_no = 1u64;
            while offset < events.len() {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let take = (random as usize % 7 + 1).min(events.len() - offset);
                let batch_events = events[offset..offset + take].to_vec();
                db.append_stream_journal_batch(&JournalBatch {
                    run_id: run_id.clone(),
                    attempt_no: 1,
                    block_no,
                    seq_start: batch_events.first().expect("batch first").seq,
                    seq_end: batch_events.last().expect("batch last").seq,
                    events: batch_events,
                })
                .expect("journal batch");
                offset += take;
                block_no += 1;
            }

            let commit = CommitAssistantTurn {
                run_id: Some(run_id.clone()),
                attempt_no: 1,
                session_id: session.id.clone(),
                assistant: NewMessage::assistant(&final_assistant),
                trailing_placeholder_id: None,
                context_json: "[]".to_string(),
                expected_context_revision: registration.context_revision,
                turn_id: None,
                usage: None,
                final_seq: seq,
            };
            for _ in 0..10 {
                db.commit_assistant_turn(&commit)
                    .expect("idempotent commit");
            }

            let actual = {
                let conn = db.conn.lock().expect("db lock");
                let mut stmt = conn
                    .prepare(
                        "SELECT role, content, COALESCE(tool_result, '')
                         FROM messages WHERE persistence_run_id = ?1
                         ORDER BY logical_block_seq, id",
                    )
                    .expect("prepare materialized rows");
                stmt.query_map(params![run_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .expect("query rows")
                .collect::<rusqlite::Result<Vec<(String, String, String)>>>()
                .expect("collect rows")
            };
            assert_eq!(actual, expected, "seed {seed}");
        }
    }

    #[test]
    fn repeated_checkpoint_refreshes_a_growing_trailing_thinking_projection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("growing-thinking.db")).expect("open db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let run_id = uuid::Uuid::new_v4().to_string();
        let registration = db
            .create_stream_run(&CreateStreamRun {
                run_id: run_id.clone(),
                session_id: session.id.clone(),
                source: "desktop".to_string(),
                stream_id: Some("thinking".to_string()),
                turn_id: None,
                provider_shape: Some("anthropic".to_string()),
            })
            .expect("run");
        db.begin_stream_attempt(&run_id, 1, Some("p"), Some("m"), Some("anthropic"))
            .expect("attempt");
        for (block_no, seq, content) in [(1, 1, "think "), (2, 2, "more")] {
            db.append_stream_journal_batch(&JournalBatch {
                run_id: run_id.clone(),
                attempt_no: 1,
                block_no,
                seq_start: seq,
                seq_end: seq,
                events: vec![JournalEvent {
                    seq_start: None,
                    seq,
                    event: serde_json::json!({
                        "type": "thinking_delta", "content": content
                    })
                    .to_string(),
                }],
            })
            .expect("journal");
            db.checkpoint_stream_context(
                &run_id,
                1,
                registration.context_revision + (seq as i64 - 1),
                "[]",
                seq,
            )
            .expect("checkpoint");
        }
        let conn = db.conn.lock().expect("db lock");
        let rows: (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(content) FROM messages
                 WHERE persistence_run_id = ?1 AND role = 'thinking_block'",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("projection");
        assert_eq!(rows, (1, "think more".to_string()));
        drop(conn);
        let snapshot = db
            .stream_run_snapshot(&run_id)
            .expect("snapshot")
            .expect("run");
        assert_eq!(snapshot.run.checkpoint_seq, 2);
        assert_eq!(snapshot.attempts[0].checkpoint_seq, 2);
    }

    #[test]
    fn recovery_context_and_projection_stop_at_checksum_valid_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("trusted-prefix.db")).expect("open db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let run_id = uuid::Uuid::new_v4().to_string();
        let registration = db
            .create_stream_run(&CreateStreamRun {
                run_id: run_id.clone(),
                session_id: session.id.clone(),
                source: "desktop".to_string(),
                stream_id: Some("trusted-prefix".to_string()),
                turn_id: None,
                provider_shape: Some("anthropic".to_string()),
            })
            .expect("run");
        db.begin_stream_attempt(&run_id, 1, Some("p"), Some("m"), Some("anthropic"))
            .expect("attempt");

        let context_a = r#"[{"role":"user","content":"hello"},{"role":"assistant","content":"A"}]"#;
        let context_ab =
            r#"[{"role":"user","content":"hello"},{"role":"assistant","content":"AB"}]"#;
        db.append_stream_journal_batch(&JournalBatch {
            run_id: run_id.clone(),
            attempt_no: 1,
            block_no: 1,
            seq_start: 1,
            seq_end: 1,
            events: vec![JournalEvent::single(
                1,
                serde_json::json!({"type":"text_delta","content":"A"}).to_string(),
            )],
        })
        .expect("first journal block");
        let revision_a = db
            .checkpoint_stream_context(&run_id, 1, registration.context_revision, context_a, 1)
            .expect("first checkpoint");
        db.append_stream_journal_batch(&JournalBatch {
            run_id: run_id.clone(),
            attempt_no: 1,
            block_no: 2,
            seq_start: 2,
            seq_end: 2,
            events: vec![JournalEvent::single(
                2,
                serde_json::json!({"type":"text_delta","content":"B"}).to_string(),
            )],
        })
        .expect("second journal block");
        let revision_ab = db
            .checkpoint_stream_context(&run_id, 1, revision_a, context_ab, 2)
            .expect("second checkpoint");

        db.conn
            .lock()
            .expect("db lock")
            .execute(
                "UPDATE chat_stream_journal SET checksum = 'corrupt'
                 WHERE run_id = ?1 AND attempt_no = 1 AND block_no = 2",
                params![run_id],
            )
            .expect("corrupt suffix checksum");
        let snapshot = db
            .stream_run_snapshot(&run_id)
            .expect("snapshot")
            .expect("run");
        let (attempt_no, through_seq, events, integrity_error) =
            select_recoverable_attempt_prefix(&snapshot);
        assert_eq!((attempt_no, through_seq), (1, 1));
        assert_eq!(events.len(), 1);
        assert!(integrity_error.is_some());

        let (trusted_context, checkpoint_seq, current_revision) = db
            .recovery_context_for_prefix(&run_id, attempt_no, through_seq)
            .expect("trusted context");
        assert_eq!(trusted_context.as_deref(), Some(context_a));
        assert_eq!(checkpoint_seq, 1);
        assert_eq!(current_revision, revision_ab);

        db.commit_interrupted_turn(&CommitInterruptedTurn {
            run_id: Some(run_id.clone()),
            attempt_no,
            session_id: session.id.clone(),
            assistant: Some(NewMessage::assistant("A")),
            context_json: trusted_context.expect("context"),
            expected_context_revision: current_revision,
            turn_id: None,
            final_seq: through_seq,
            status: ChatTurnStatus::Failed,
            interrupt_reason: Some("journal_corrupt".to_string()),
            error: integrity_error,
            recovery_event: None,
        })
        .expect("recover valid prefix");

        let conn = db.conn.lock().expect("db lock");
        let projected: Vec<String> = conn
            .prepare(
                "SELECT content FROM messages WHERE persistence_run_id = ?1
                 ORDER BY logical_block_seq, id",
            )
            .expect("prepare projection")
            .query_map(params![run_id], |row| row.get(0))
            .expect("query projection")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect projection");
        assert_eq!(projected, vec!["A".to_string()]);
        let stored_context: String = conn
            .query_row(
                "SELECT context_json FROM sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .expect("stored context");
        assert_eq!(stored_context, context_a);
    }

    #[test]
    fn incognito_registration_leaves_no_durability_or_usage_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("incognito.db")).expect("open db");
        let session = db
            .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, Some(true))
            .expect("incognito session");
        let run_id = uuid::Uuid::new_v4().to_string();
        let registration = db
            .create_stream_run(&CreateStreamRun {
                run_id: run_id.clone(),
                session_id: session.id.clone(),
                source: "desktop".to_string(),
                stream_id: Some("private-stream".to_string()),
                turn_id: None,
                provider_shape: None,
            })
            .expect("memory-only registration");
        assert!(!registration.persistent);
        let conn = db.conn.lock().expect("db lock");
        for table in [
            "chat_stream_runs",
            "chat_stream_attempts",
            "chat_stream_journal",
            "chat_stream_context_checkpoints",
            "model_usage_events",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count");
            assert_eq!(count, 0, "incognito leaked into {table}");
        }
        drop(conn);

        let mut usage = ModelUsageEvent::new(crate::model_usage::KIND_CHAT);
        usage.session_id = Some(session.id.clone());
        db.commit_assistant_turn(&CommitAssistantTurn {
            run_id: None,
            attempt_no: 0,
            session_id: session.id.clone(),
            assistant: NewMessage::assistant("private"),
            trailing_placeholder_id: None,
            context_json: "[]".to_string(),
            expected_context_revision: registration.context_revision,
            turn_id: None,
            usage: Some(usage),
            final_seq: 0,
        })
        .expect("incognito in-session commit");
        let usage_count: i64 = db
            .conn
            .lock()
            .expect("db lock")
            .query_row("SELECT COUNT(*) FROM model_usage_events", [], |row| {
                row.get(0)
            })
            .expect("usage count");
        assert_eq!(usage_count, 0, "incognito usage must not reach ledger");
    }

    #[test]
    fn journal_storage_grows_linearly_instead_of_rewriting_the_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("linear-write.db")).expect("open db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let run_id = uuid::Uuid::new_v4().to_string();
        db.create_stream_run(&CreateStreamRun {
            run_id: run_id.clone(),
            session_id: session.id,
            source: "desktop".to_string(),
            stream_id: Some("linear".to_string()),
            turn_id: None,
            provider_shape: Some("anthropic".to_string()),
        })
        .expect("run");
        db.begin_stream_attempt(&run_id, 1, Some("p"), Some("m"), Some("anthropic"))
            .expect("attempt");

        let chunk = "x".repeat(1024);
        let mut raw_event_bytes = 0usize;
        for block_no in 0..8u64 {
            let mut events = Vec::new();
            for offset in 0..8u64 {
                let seq = block_no * 8 + offset + 1;
                let event = serde_json::json!({
                    "type":"text_delta",
                    "content":chunk.as_str()
                })
                .to_string();
                raw_event_bytes = raw_event_bytes.saturating_add(event.len());
                events.push(JournalEvent::single(seq, event));
            }
            db.append_stream_journal_batch(&JournalBatch {
                run_id: run_id.clone(),
                attempt_no: 1,
                block_no: block_no + 1,
                seq_start: block_no * 8 + 1,
                seq_end: block_no * 8 + 8,
                events,
            })
            .expect("journal batch");
        }

        let conn = db.conn.lock().expect("db lock");
        let (blocks, stored_bytes): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(payload)), 0)
                 FROM chat_stream_journal WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("journal size");
        assert_eq!(blocks, 8);
        assert!(stored_bytes > raw_event_bytes as i64);
        assert!(
            stored_bytes < (raw_event_bytes * 2) as i64,
            "journal payload should be O(output), stored={stored_bytes}, raw={raw_event_bytes}"
        );
    }
}
