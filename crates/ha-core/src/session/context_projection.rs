//! Durable metadata foundation for request-only context projections.
//!
//! Projection epochs contain no result bodies. Exact request plans may contain
//! an inline request payload, but their quota/reservation/retention columns are
//! coordination metadata only: this module does not claim ACID with a future
//! blob store or media/result-object lease manager.

#![allow(dead_code)]

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::db::SessionDB;

const MAX_ID_BYTES: usize = 512;
const MAX_FINGERPRINT_BYTES: usize = 1_024;
const MAX_PROFILE_BYTES: usize = 512;
const MAX_OUTCOME_BYTES: usize = 4 * 1_024;

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            fn from_db(value: &str, column: usize) -> rusqlite::Result<Self> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(invalid_text_value(column, stringify!($name), value)),
                }
            }
        }
    };
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionEpochScope {
        SessionHead => "session_head",
        RequestLocal => "request_local",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionEpochState {
        Active => "active",
        Superseded => "superseded",
        Revoked => "revoked",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionTrigger {
        TurnStart => "turn_start",
        ToolLoop => "tool_loop",
        Manual => "manual",
        OverflowRecovery => "overflow_recovery",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionAction {
        Tier0Omit => "tier0_omit",
        Tier2Soft => "tier2_soft",
        Tier2Minimal => "tier2_minimal",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RequestProjectionRole {
        MainContinuation => "main_continuation",
        Tier3SummaryInput => "tier3_summary_input",
        SideQuery => "side_query",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RequestProjectionPlanState {
        Prepared => "prepared",
        ContextCommitted => "context_committed",
        Dispatching => "dispatching",
        ResponseStarted => "response_started",
        SendUnknown => "send_unknown",
        Terminal => "terminal",
        Superseded => "superseded",
    }
}

impl RequestProjectionPlanState {
    fn can_transition_to(self, next: Self) -> bool {
        use RequestProjectionPlanState as S;
        matches!(
            (self, next),
            (S::Prepared, S::ContextCommitted | S::Superseded)
                | (S::ContextCommitted, S::Dispatching | S::Superseded)
                | (S::Dispatching, S::ResponseStarted | S::SendUnknown)
                | (S::ResponseStarted, S::Terminal)
                | (S::SendUnknown, S::Terminal)
        )
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionReplayability {
        ManagedResult => "managed_result",
        ExactPlanOnly => "exact_plan_only",
        Lost => "lost",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadAvailability {
        Stored => "stored",
        Unavailable => "unavailable",
        Lost => "lost",
    }
}

#[derive(Debug, Clone)]
pub struct NewProjectionEpoch {
    pub epoch_id: String,
    pub session_id: String,
    pub branch_id: String,
    pub scope: ProjectionEpochScope,
    pub owner_request_plan_id: Option<String>,
    pub cache_identity_hash: String,
    pub parent_epoch_id: Option<String>,
    pub canonical_generation: i64,
    pub created_at_revision: i64,
    pub provider_request_shape: String,
    pub policy_fingerprint: String,
    pub renderer_version: u32,
    pub counter_profile: String,
    pub trigger: ProjectionTrigger,
    pub max_tier: u8,
    pub earliest_changed_item_key: Option<String>,
    pub action_digest: String,
}

#[derive(Debug, Clone)]
pub struct NewProjectionItem {
    pub projection_item_key: String,
    pub result_id: Option<String>,
    pub stable_ordinal: u64,
    pub action: ProjectionAction,
    /// A keyed digest (or equally strong source-version guard) over the exact
    /// source text/shape from which the replacement was rendered.
    pub source_guard: String,
    /// Fingerprint of the exact replacement bytes and renderer profile.
    pub replacement_fingerprint: String,
    pub replayability: ProjectionReplayability,
    pub renderer_profile: String,
    pub target_variant: String,
    pub source_plan_id: Option<String>,
    pub source_projection_item_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionEpochRecord {
    pub epoch_id: String,
    pub session_id: String,
    pub branch_id: String,
    pub scope: ProjectionEpochScope,
    pub owner_request_plan_id: Option<String>,
    pub cache_identity_hash: String,
    pub parent_epoch_id: Option<String>,
    pub canonical_generation: i64,
    pub created_at_revision: i64,
    pub provider_request_shape: String,
    pub policy_fingerprint: String,
    pub renderer_version: u32,
    pub counter_profile: String,
    pub trigger: ProjectionTrigger,
    pub max_tier: u8,
    pub earliest_changed_item_key: Option<String>,
    pub action_digest: String,
    pub state: ProjectionEpochState,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionItemRecord {
    pub epoch_id: String,
    pub projection_item_key: String,
    pub result_id: Option<String>,
    pub stable_ordinal: u64,
    pub action: ProjectionAction,
    pub source_guard: String,
    pub replacement_fingerprint: String,
    pub replayability: ProjectionReplayability,
    pub renderer_profile: String,
    pub target_variant: String,
    pub source_plan_id: Option<String>,
    pub source_projection_item_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionHeadRecord {
    pub session_id: String,
    pub branch_id: String,
    pub cache_identity_hash: String,
    pub epoch_id: String,
    pub projection_revision: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewRequestProjectionPlan {
    pub request_plan_id: String,
    pub session_id: String,
    pub branch_id: String,
    pub run_id: String,
    pub attempt_no: u32,
    pub request_role: RequestProjectionRole,
    /// Version of the canonical history frozen into this request. The value is
    /// checked against the SessionDB-owned monotonic version source.
    pub expected_canonical_generation: i64,
    /// Session context revision frozen into this request.
    pub expected_context_revision: i64,
    pub projection_epoch_id: Option<String>,
    pub cache_identity_hash: String,
    pub provider_id: String,
    pub provider_profile_id: Option<String>,
    pub model_id: String,
    pub request_shape: String,
    pub writer_version: u32,
    pub renderer_version: u32,
    pub policy_fingerprint: String,
    pub counter_profile: String,
    /// Soft reference to the independently staged encrypted payload. Absence
    /// is valid when kernel-private storage is unavailable; the plan remains a
    /// send-state WAL but is explicitly non-recoverable after a crash.
    pub exact_payload_id: Option<String>,
    pub exact_payload_reservation_id: Option<String>,
    pub exact_payload_keyed_digest: Option<String>,
    pub exact_payload_storage_kind: Option<String>,
    pub exact_payload_stored_bytes: Option<u64>,
    pub payload_availability: ExactPayloadAvailability,
    pub projection_bytes: u64,
    pub expires_at: Option<String>,
    pub final_capacity_count_json: String,
    /// Identity of the exact provider-ready body frozen in memory, independent
    /// of whether an encrypted payload hold can be persisted.
    pub prepared_body_fingerprint: String,
    pub prepared_body_bytes: u64,
    pub endpoint_kind: String,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestProjectionPlanRecord {
    pub request_plan_id: String,
    pub session_id: String,
    pub branch_id: String,
    pub run_id: String,
    pub attempt_no: u32,
    pub request_role: RequestProjectionRole,
    pub request_sequence: u64,
    pub expected_canonical_generation: i64,
    pub expected_context_revision: i64,
    pub projection_epoch_id: Option<String>,
    pub cache_identity_hash: String,
    pub provider_id: String,
    pub provider_profile_id: Option<String>,
    pub model_id: String,
    pub request_shape: String,
    pub writer_version: u32,
    pub renderer_version: u32,
    pub policy_fingerprint: String,
    pub counter_profile: String,
    pub exact_payload_id: Option<String>,
    pub exact_payload_reservation_id: Option<String>,
    pub exact_payload_keyed_digest: Option<String>,
    pub exact_payload_storage_kind: Option<String>,
    pub exact_payload_stored_bytes: Option<u64>,
    pub payload_availability: ExactPayloadAvailability,
    pub projection_bytes: u64,
    pub expires_at: Option<String>,
    pub final_capacity_count_json: String,
    pub prepared_body_fingerprint: String,
    pub prepared_body_bytes: u64,
    pub endpoint_kind: String,
    pub content_type: String,
    pub state: RequestProjectionPlanState,
    pub request_attempt_id: Option<String>,
    pub provider_idempotency_key: Option<String>,
    pub provider_request_id: Option<String>,
    pub dispatch_started_at: Option<String>,
    pub response_started_at: Option<String>,
    pub response_provider_attempt: Option<u32>,
    pub response_status: Option<u16>,
    pub terminal_outcome: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestProjectionVersion {
    pub context_revision: i64,
    pub canonical_generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequestBodyIdentity {
    pub fingerprint: String,
    pub bytes: u64,
    pub endpoint_kind: String,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainProjectionHeadCommit {
    Insert,
    Replace {
        expected_epoch_id: String,
        expected_projection_revision: i64,
    },
}

/// Reader-first development builds briefly created a v1 draft containing
/// inline plaintext and permissive transitions. The feature was never wired
/// to production dispatch, but local databases may still contain that draft.
/// Migrate it fail-closed: strip payload bytes/references, terminalize every
/// unsent plan as superseded, and classify any possibly-sent state as
/// send-unknown. No cross-blob atomicity or recoverability is inferred.
fn migrate_legacy_projection_tables(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS context_projection_migration_meta (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )?;
    let retry_secure_checkpoint = conn
        .query_row(
            "SELECT value FROM context_projection_migration_meta
              WHERE key = 'legacy_payload_secure_checkpoint'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        == Some("pending");
    if retry_secure_checkpoint {
        let (busy, _log_frames, _checkpointed_frames): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            bail!("legacy exact request payload WAL could not be securely truncated");
        }
        conn.execute(
            "DELETE FROM context_projection_migration_meta
              WHERE key = 'legacy_payload_secure_checkpoint'",
            [],
        )?;
    }
    let request_table_exists = sqlite_table_exists(conn, "request_projection_plans")?;
    let legacy_table_exists = sqlite_table_exists(conn, "request_projection_plans_legacy_v1")?;
    let current_is_v1 = request_table_exists
        && !sqlite_column_exists(conn, "request_projection_plans", "attempt_no")?;
    if current_is_v1 && legacy_table_exists {
        bail!("both active and renamed legacy request projection tables exist");
    }

    if current_is_v1 || legacy_table_exists {
        let plaintext_table = if legacy_table_exists {
            "request_projection_plans_legacy_v1"
        } else {
            "request_projection_plans"
        };
        // The draft blob-ref field was never wired to a writer; it is not an
        // authorized filesystem locator. Never follow or delete it. Scrub both
        // inline fields in SQLite, checkpoint those zeroes, and only then copy
        // fail-closed metadata into v2.
        conn.execute_batch("PRAGMA secure_delete = ON;")?;
        conn.execute(
            "INSERT INTO context_projection_migration_meta (key, value)
                  VALUES ('legacy_payload_secure_checkpoint', 'pending')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        conn.execute_batch(&format!(
            "UPDATE {plaintext_table}
                SET exact_projection_payload = CASE
                        WHEN exact_projection_payload IS NULL THEN NULL
                        ELSE zeroblob(length(exact_projection_payload))
                    END,
                    exact_payload_blob_ref = CASE
                        WHEN exact_payload_blob_ref IS NULL THEN NULL
                        ELSE zeroblob(length(exact_payload_blob_ref))
                    END;"
        ))?;
        let (busy, _log_frames, _checkpointed_frames): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            bail!("legacy exact request payload WAL could not be securely truncated");
        }
        conn.execute(
            "DELETE FROM context_projection_migration_meta
              WHERE key = 'legacy_payload_secure_checkpoint'",
            [],
        )?;

        let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        tx.execute_batch(DROP_CONTEXT_PROJECTION_TRIGGERS)?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_request_projection_active_key;
             DROP INDEX IF EXISTS idx_request_projection_sequence;
             DROP INDEX IF EXISTS idx_request_projection_session_state;
             DROP INDEX IF EXISTS idx_request_projection_run_state;",
        )?;
        if current_is_v1 {
            tx.execute_batch(
                "ALTER TABLE request_projection_plans
                    RENAME TO request_projection_plans_legacy_v1;",
            )?;
        }
        tx.execute_batch(SCHEMA)?;
        // Live insert guards deliberately reject legacy versions and missing
        // payload holds. Disable them only inside this all-or-nothing metadata
        // conversion and recreate them before commit.
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS request_projection_plan_version_insert;
             DROP TRIGGER IF EXISTS request_projection_plan_epoch_scope_insert;
             DROP TRIGGER IF EXISTS request_projection_payload_hold_insert;",
        )?;
        tx.execute(
            "INSERT INTO session_projection_versions (
                 session_id, canonical_generation, last_request_sequence
             )
             SELECT session.id,
                    COALESCE(MAX(legacy.canonical_generation), 0),
                    COALESCE((
                        SELECT MAX(current.request_sequence)
                          FROM request_projection_plans current
                         WHERE current.session_id = session.id
                    ), 0)
               FROM sessions session
               LEFT JOIN request_projection_plans_legacy_v1 legacy
                 ON legacy.session_id = session.id
              WHERE session.incognito = 0
              GROUP BY session.id
             ON CONFLICT(session_id) DO UPDATE SET
                 canonical_generation = MAX(
                     session_projection_versions.canonical_generation,
                     excluded.canonical_generation
                 ),
                 last_request_sequence = MAX(
                     session_projection_versions.last_request_sequence,
                     excluded.last_request_sequence
                 )",
            [],
        )?;
        tx.execute_batch(
            "INSERT INTO request_projection_plans (
             request_plan_id, session_id, branch_id, run_id, attempt_no,
             request_role, request_sequence, expected_canonical_generation,
             expected_context_revision, projection_epoch_id, cache_identity_hash,
             provider_id, provider_profile_id, model_id, request_shape,
             writer_version, renderer_version, policy_fingerprint, counter_profile,
             exact_payload_id, exact_payload_reservation_id,
             exact_payload_keyed_digest, exact_payload_storage_kind,
             exact_payload_stored_bytes, payload_availability, projection_bytes,
             expires_at, final_capacity_count_json, prepared_body_fingerprint,
             prepared_body_bytes, endpoint_kind, content_type, state, request_attempt_id,
             provider_idempotency_key, provider_request_id, dispatch_started_at,
             response_started_at, response_provider_attempt, response_status,
             terminal_outcome, created_at, updated_at
         )
         SELECT legacy.request_plan_id, legacy.session_id, legacy.branch_id,
                legacy.run_id, legacy.attempt, legacy.request_role,
                COALESCE((
                    SELECT MAX(current.request_sequence)
                      FROM request_projection_plans current
                     WHERE current.session_id = legacy.session_id
                ), 0) + ROW_NUMBER() OVER (
                    PARTITION BY legacy.session_id
                    ORDER BY legacy.created_at, legacy.request_plan_id
                ),
                legacy.canonical_generation, legacy.base_context_revision,
                legacy.projection_epoch_id,
                cache_identity_hash, 'legacy-unknown', NULL, 'legacy-unknown',
                provider_request_shape, writer_version, renderer_version,
                policy_fingerprint, counter_profile, NULL, NULL, NULL, NULL, NULL,
                CASE
                    WHEN exact_projection_payload IS NOT NULL
                      OR exact_payload_blob_ref IS NOT NULL THEN 'lost'
                    ELSE 'unavailable'
                END,
                projection_bytes, expires_at, final_capacity_count_json,
                '0000000000000000000000000000000000000000000000000000000000000000',
                projection_bytes, 'legacy-unknown',
                'application/octet-stream',
                CASE
                    WHEN state IN ('dispatching', 'dispatched', 'send_unknown')
                        THEN 'send_unknown'
                    WHEN state = 'terminal' THEN 'terminal'
                    ELSE 'superseded'
                END,
                request_attempt_id, provider_idempotency_key, provider_request_id,
                dispatch_started_at,
                CASE WHEN state = 'dispatched' THEN updated_at ELSE NULL END,
                NULL, NULL,
                CASE
                    WHEN state IN ('dispatching', 'dispatched', 'send_unknown')
                        THEN COALESCE(terminal_outcome, 'legacy_migration_send_unknown')
                    WHEN state = 'terminal'
                        THEN COALESCE(terminal_outcome, 'legacy_migration_terminal')
                    ELSE COALESCE(terminal_outcome, 'legacy_migration_unrecoverable')
                END,
                created_at, updated_at
               FROM request_projection_plans_legacy_v1 legacy
              WHERE 1
             ON CONFLICT(request_plan_id) DO NOTHING;",
        )?;
        let missing_legacy_plans: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM request_projection_plans_legacy_v1 legacy
               LEFT JOIN request_projection_plans current
                 ON current.request_plan_id = legacy.request_plan_id
                AND current.session_id = legacy.session_id
              WHERE current.request_plan_id IS NULL",
            [],
            |row| row.get(0),
        )?;
        if missing_legacy_plans != 0 {
            bail!(
                "legacy request projection migration left {missing_legacy_plans} plan(s) unresolved"
            );
        }
        tx.execute(
            "UPDATE session_projection_versions AS version
                SET last_request_sequence = MAX(
                    last_request_sequence,
                    COALESCE((
                        SELECT MAX(plan.request_sequence)
                          FROM request_projection_plans plan
                         WHERE plan.session_id = version.session_id
                    ), 0)
                )",
            [],
        )?;
        tx.execute_batch("DROP TABLE request_projection_plans_legacy_v1;")?;
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        return Ok(());
    }

    // Incremental v2 upgrades are checked column-by-column and performed in
    // one IMMEDIATE transaction. A crash therefore rolls back the whole set;
    // no first-column sentinel can hide a partially applied batch.
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute_batch(DROP_CONTEXT_PROJECTION_TRIGGERS)?;
    if !request_table_exists {
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        return Ok(());
    }
    if sqlite_table_exists(&tx, "context_projection_items")? {
        if !sqlite_column_exists(&tx, "context_projection_items", "source_guard")? {
            tx.execute_batch(
                "DELETE FROM session_projection_heads
                  WHERE epoch_id IN (
                      SELECT epoch_id FROM context_projection_epochs WHERE scope = 'session_head'
                  );
                 UPDATE context_projection_epochs SET state = 'revoked' WHERE state = 'active';
                 ALTER TABLE context_projection_items ADD COLUMN
                     source_guard TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';",
            )?;
        }
        if !sqlite_column_exists(&tx, "context_projection_items", "replacement_fingerprint")? {
            tx.execute_batch(
                "ALTER TABLE context_projection_items ADD COLUMN
                     replacement_fingerprint TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';",
            )?;
        }
        if !sqlite_column_exists(&tx, "context_projection_items", "replayability")? {
            tx.execute_batch(
                "ALTER TABLE context_projection_items ADD COLUMN
                     replayability TEXT NOT NULL DEFAULT 'lost';",
            )?;
        }
    }
    for (column, ddl) in [
        ("exact_payload_storage_kind", "ALTER TABLE request_projection_plans ADD COLUMN exact_payload_storage_kind TEXT"),
        ("exact_payload_stored_bytes", "ALTER TABLE request_projection_plans ADD COLUMN exact_payload_stored_bytes INTEGER"),
        ("response_provider_attempt", "ALTER TABLE request_projection_plans ADD COLUMN response_provider_attempt INTEGER"),
        ("response_status", "ALTER TABLE request_projection_plans ADD COLUMN response_status INTEGER"),
        ("prepared_body_fingerprint", "ALTER TABLE request_projection_plans ADD COLUMN prepared_body_fingerprint TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'"),
        ("prepared_body_bytes", "ALTER TABLE request_projection_plans ADD COLUMN prepared_body_bytes INTEGER NOT NULL DEFAULT 0"),
        ("endpoint_kind", "ALTER TABLE request_projection_plans ADD COLUMN endpoint_kind TEXT NOT NULL DEFAULT 'legacy-unknown'"),
        ("content_type", "ALTER TABLE request_projection_plans ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/octet-stream'"),
    ] {
        if !sqlite_column_exists(&tx, "request_projection_plans", column)? {
            tx.execute_batch(ddl)?;
        }
    }
    tx.execute_batch(
        "UPDATE request_projection_plans
            SET payload_availability = 'lost', exact_payload_id = NULL,
                exact_payload_reservation_id = NULL, exact_payload_keyed_digest = NULL,
                exact_payload_storage_kind = NULL, exact_payload_stored_bytes = NULL
          WHERE payload_availability = 'stored'
            AND (exact_payload_storage_kind IS NULL OR exact_payload_stored_bytes IS NULL);
         UPDATE request_projection_plans
            SET prepared_body_bytes = projection_bytes
          WHERE prepared_body_bytes = 0;",
    )?;
    tx.execute_batch(SCHEMA)?;
    tx.commit()?;
    Ok(())
}

fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get(0),
    )?)
}

fn sqlite_column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("SELECT {column} FROM {table} LIMIT 0");
    Ok(conn.prepare(&sql).is_ok())
}

impl SessionDB {
    pub(crate) fn ensure_context_projection_tables(conn: &rusqlite::Connection) -> Result<()> {
        migrate_legacy_projection_tables(conn)?;
        // `CREATE TRIGGER IF NOT EXISTS` cannot harden a trigger left by an
        // earlier reader-first draft. Recreate the small trigger set under one
        // writer transaction so another process never observes an unguarded
        // schema between DROP and CREATE.
        let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        tx.execute_batch(DROP_CONTEXT_PROJECTION_TRIGGERS)?;
        tx.execute_batch(SCHEMA)?;
        tx.execute(
            "INSERT OR IGNORE INTO session_projection_versions (
                 session_id, canonical_generation, last_request_sequence
             ) SELECT id, 0, 0 FROM sessions WHERE incognito = 0",
            [],
        )?;
        tx.execute(
            "UPDATE session_projection_versions AS version
                SET last_request_sequence = MAX(
                    last_request_sequence,
                    COALESCE((
                        SELECT MAX(plan.request_sequence)
                          FROM request_projection_plans plan
                         WHERE plan.session_id = version.session_id
                    ), 0)
                ),
                    canonical_generation = MAX(
                        canonical_generation,
                        COALESCE((
                            SELECT MAX(epoch.canonical_generation)
                              FROM context_projection_epochs epoch
                             WHERE epoch.session_id = version.session_id
                        ), 0)
                    )",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Insert one immutable epoch and its actions in stable ordinal order.
    pub fn insert_projection_epoch(
        &self,
        epoch: &NewProjectionEpoch,
        items: &[NewProjectionItem],
    ) -> Result<()> {
        validate_epoch(epoch, items)?;
        if epoch.scope == ProjectionEpochScope::RequestLocal {
            bail!("request-local epochs must be created atomically with their exact request plan");
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_projection_epoch_in_tx(&tx, epoch, items)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_projection_epoch(
        &self,
        session_id: &str,
        epoch_id: &str,
    ) -> Result<Option<(ProjectionEpochRecord, Vec<ProjectionItemRecord>)>> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        validate_text("epoch_id", epoch_id, MAX_ID_BYTES)?;
        let conn = self.read_conn()?;
        let epoch = conn
            .query_row(
                &format!("{EPOCH_SELECT} WHERE session_id = ?1 AND epoch_id = ?2"),
                params![session_id, epoch_id],
                row_to_epoch,
            )
            .optional()?;
        let Some(epoch) = epoch else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(&format!(
            "{ITEM_SELECT} WHERE epoch_id = ?1 ORDER BY stable_ordinal, projection_item_key"
        ))?;
        let items = stmt
            .query_map(params![epoch_id], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some((epoch, items)))
    }

    pub fn get_projection_head(
        &self,
        session_id: &str,
        branch_id: &str,
        cache_identity_hash: &str,
    ) -> Result<Option<ProjectionHeadRecord>> {
        let conn = self.read_conn()?;
        query_head(&conn, session_id, branch_id, cache_identity_hash)
    }

    /// Read the SessionDB-owned versions that an exact request must freeze.
    pub fn get_request_projection_version(
        &self,
        session_id: &str,
    ) -> Result<RequestProjectionVersion> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        let conn = self.read_conn()?;
        require_persistent_session(&conn, session_id)?;
        query_projection_version(&conn, session_id)
    }

    /// Advance only the main conversation's canonical generation by one.
    /// Auxiliary request roles can bind this version, but have no API capable
    /// of changing it.
    pub fn advance_main_canonical_generation(
        &self,
        session_id: &str,
        expected: RequestProjectionVersion,
    ) -> Result<Option<RequestProjectionVersion>> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_persistent_session(&tx, session_id)?;
        if query_projection_version(&tx, session_id)? != expected {
            tx.commit()?;
            return Ok(None);
        }
        let next_generation = expected
            .canonical_generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("canonical generation overflow"))?;
        let changed = tx.execute(
            "UPDATE session_projection_versions
                SET canonical_generation = ?1
              WHERE session_id = ?2 AND canonical_generation = ?3
                AND EXISTS (
                    SELECT 1 FROM sessions
                     WHERE id = ?2 AND context_revision = ?4 AND incognito = 0
                )",
            params![
                next_generation,
                session_id,
                expected.canonical_generation,
                expected.context_revision,
            ],
        )?;
        tx.commit()?;
        Ok((changed == 1).then_some(RequestProjectionVersion {
            context_revision: expected.context_revision,
            canonical_generation: next_generation,
        }))
    }

    /// Create one immutable request identity and allocate its monotonic
    /// per-session sequence inside the same IMMEDIATE transaction. This API
    /// never accepts request plaintext. `Unavailable` is a valid, explicitly
    /// non-recoverable payload state when kernel-private storage is disabled.
    pub fn create_request_projection_plan(
        &self,
        plan: &NewRequestProjectionPlan,
    ) -> Result<RequestProjectionPlanRecord> {
        validate_plan(plan)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(epoch_id) = plan.projection_epoch_id.as_deref() {
            let scope = tx
                .query_row(
                    "SELECT scope FROM context_projection_epochs WHERE epoch_id = ?1",
                    params![epoch_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if scope.as_deref() == Some(ProjectionEpochScope::RequestLocal.as_str()) {
                bail!("request-local projection plans must be created with their epoch atomically");
            }
            require_plan_epoch(&tx, plan, epoch_id)?;
        }
        let record = insert_request_projection_plan_in_tx(&tx, plan)?;
        tx.commit()?;
        Ok(record)
    }

    /// Publish a request without a new projection epoch directly at the
    /// context-committed boundary.  This avoids a durable Prepared row that
    /// the live coordinator does not yet know about if the context fence is
    /// stale or a second transaction fails.
    pub fn create_context_committed_request_projection_plan(
        &self,
        plan: &NewRequestProjectionPlan,
    ) -> Result<RequestProjectionPlanRecord> {
        self.create_context_committed_request_projection_plan_with_followup(plan, false)
    }

    pub(crate) fn create_context_committed_request_projection_plan_with_followup(
        &self,
        plan: &NewRequestProjectionPlan,
        require_tier3_after_capacity_projection: bool,
    ) -> Result<RequestProjectionPlanRecord> {
        validate_plan(plan)?;
        if plan.projection_epoch_id.is_some() {
            bail!("projection epochs require their typed atomic constructor");
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_request_projection_plan_in_tx(&tx, plan)?;
        commit_new_plan_context_in_tx(&tx, &plan.session_id, &plan.request_plan_id)?;
        if require_tier3_after_capacity_projection {
            Self::require_tier3_after_capacity_projection_in_tx(
                &tx,
                &plan.session_id,
                &plan.request_plan_id,
                plan.expected_canonical_generation,
            )?;
        }
        let record = tx.query_row(
            &format!("{PLAN_SELECT} WHERE request_plan_id = ?1"),
            params![plan.request_plan_id],
            row_to_plan,
        )?;
        tx.commit()?;
        Ok(record)
    }

    /// Create a request-local epoch, its guarded action manifest, and the exact
    /// request-plan WAL row in one IMMEDIATE transaction. This is the only
    /// constructor for request-local epochs, so a crash can leave neither an
    /// orphan epoch nor a plan referencing a missing manifest.
    pub fn create_request_local_projection_plan(
        &self,
        epoch: &NewProjectionEpoch,
        items: &[NewProjectionItem],
        plan: &NewRequestProjectionPlan,
    ) -> Result<RequestProjectionPlanRecord> {
        validate_epoch(epoch, items)?;
        validate_plan(plan)?;
        if epoch.scope != ProjectionEpochScope::RequestLocal
            || epoch.owner_request_plan_id.as_deref() != Some(plan.request_plan_id.as_str())
            || plan.projection_epoch_id.as_deref() != Some(epoch.epoch_id.as_str())
            || epoch.session_id != plan.session_id
            || epoch.branch_id != plan.branch_id
            || epoch.cache_identity_hash != plan.cache_identity_hash
            || epoch.canonical_generation != plan.expected_canonical_generation
            || epoch.created_at_revision != plan.expected_context_revision
            || epoch.provider_request_shape != plan.request_shape
        {
            bail!("request-local epoch and exact request plan identity/version do not match");
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_expected_projection_version(&tx, plan)?;
        insert_projection_epoch_in_tx(&tx, epoch, items)?;
        require_plan_epoch(&tx, plan, &epoch.epoch_id)?;
        let record = insert_request_projection_plan_in_tx(&tx, plan)?;
        tx.commit()?;
        Ok(record)
    }

    /// Atomically publish a request-local projection and its request plan at
    /// the context-committed boundary. No DB-only Prepared row can survive a
    /// failed live publication.
    pub fn create_context_committed_request_local_projection_plan(
        &self,
        epoch: &NewProjectionEpoch,
        items: &[NewProjectionItem],
        plan: &NewRequestProjectionPlan,
    ) -> Result<RequestProjectionPlanRecord> {
        self.create_context_committed_request_local_projection_plan_with_followup(
            epoch, items, plan, false,
        )
    }

    pub(crate) fn create_context_committed_request_local_projection_plan_with_followup(
        &self,
        epoch: &NewProjectionEpoch,
        items: &[NewProjectionItem],
        plan: &NewRequestProjectionPlan,
        require_tier3_after_capacity_projection: bool,
    ) -> Result<RequestProjectionPlanRecord> {
        validate_epoch(epoch, items)?;
        validate_plan(plan)?;
        if epoch.scope != ProjectionEpochScope::RequestLocal
            || epoch.owner_request_plan_id.as_deref() != Some(plan.request_plan_id.as_str())
            || plan.projection_epoch_id.as_deref() != Some(epoch.epoch_id.as_str())
            || epoch.session_id != plan.session_id
            || epoch.branch_id != plan.branch_id
            || epoch.cache_identity_hash != plan.cache_identity_hash
            || epoch.canonical_generation != plan.expected_canonical_generation
            || epoch.created_at_revision != plan.expected_context_revision
            || epoch.provider_request_shape != plan.request_shape
        {
            bail!("request-local epoch and exact request plan identity/version do not match");
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_expected_projection_version(&tx, plan)?;
        insert_projection_epoch_in_tx(&tx, epoch, items)?;
        require_plan_epoch(&tx, plan, &epoch.epoch_id)?;
        insert_request_projection_plan_in_tx(&tx, plan)?;
        commit_new_plan_context_in_tx(&tx, &plan.session_id, &plan.request_plan_id)?;
        if require_tier3_after_capacity_projection {
            Self::require_tier3_after_capacity_projection_in_tx(
                &tx,
                &plan.session_id,
                &plan.request_plan_id,
                plan.expected_canonical_generation,
            )?;
        }
        let record = tx.query_row(
            &format!("{PLAN_SELECT} WHERE request_plan_id = ?1"),
            params![plan.request_plan_id],
            row_to_plan,
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn get_request_projection_plan(
        &self,
        session_id: &str,
        request_plan_id: &str,
    ) -> Result<Option<RequestProjectionPlanRecord>> {
        let conn = self.read_conn()?;
        conn.query_row(
            &format!("{PLAN_SELECT} WHERE session_id = ?1 AND request_plan_id = ?2"),
            params![session_id, request_plan_id],
            row_to_plan,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_nonterminal_request_plans_for_run_attempt(
        &self,
        session_id: &str,
        run_id: &str,
        attempt_no: u32,
    ) -> Result<Vec<RequestProjectionPlanRecord>> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        validate_text("run_id", run_id, MAX_ID_BYTES)?;
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(&format!(
            "{PLAN_SELECT} WHERE session_id = ?1 AND run_id = ?2 AND attempt_no = ?3
                AND state NOT IN ('terminal', 'superseded')
              ORDER BY request_sequence"
        ))?;
        let records = stmt
            .query_map(
                params![session_id, run_id, i64::from(attempt_no)],
                row_to_plan,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    /// Read-only fence used before failover/retry. A new provider attempt may
    /// supersede only when every nonterminal plan for this run attempt is still
    /// provably unsent. `Dispatching`, `ResponseStarted`, and `SendUnknown`
    /// return false and must be resolved rather than retried.
    pub fn assert_run_attempt_supersedable(
        &self,
        session_id: &str,
        run_id: &str,
        attempt_no: u32,
    ) -> Result<bool> {
        let plans =
            self.list_nonterminal_request_plans_for_run_attempt(session_id, run_id, attempt_no)?;
        Ok(plans.iter().all(|plan| {
            matches!(
                plan.state,
                RequestProjectionPlanState::Prepared | RequestProjectionPlanState::ContextCommitted
            )
        }))
    }

    /// Atomically supersede every provably-unsent plan belonging to one failed
    /// provider attempt. If any possibly-sent plan exists, the entire
    /// transaction returns false and changes nothing.
    pub fn supersede_unsent_run_attempt(
        &self,
        session_id: &str,
        run_id: &str,
        attempt_no: u32,
        reason: &str,
    ) -> Result<bool> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        validate_text("run_id", run_id, MAX_ID_BYTES)?;
        validate_text("supersede_reason", reason, MAX_OUTCOME_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed =
            supersede_unsent_run_attempt_in_tx(&tx, session_id, run_id, attempt_no, reason)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Commit a main request's context fence. A session-head change, when
    /// supplied, is CASed in this same transaction. Request-local main plans
    /// remain request-only and therefore do not alter the head.
    pub fn commit_main_request_context(
        &self,
        session_id: &str,
        request_plan_id: &str,
        head_commit: Option<&MainProjectionHeadCommit>,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(guard) = load_plan_commit_guard(&tx, session_id, request_plan_id)? else {
            tx.commit()?;
            return Ok(false);
        };
        if guard.role != RequestProjectionRole::MainContinuation {
            bail!("auxiliary request plans cannot commit main context/head state");
        }
        require_live_plan_version(&tx, session_id, &guard)?;
        if !commit_plan_projection_head(&tx, session_id, request_plan_id, &guard, head_commit)? {
            tx.commit()?;
            return Ok(false);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'context_committed', updated_at = ?1
              WHERE session_id = ?2 AND request_plan_id = ?3 AND state = 'prepared'",
            params![now, session_id, request_plan_id],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Commit a Tier-3 summary or side-query context fence. This API can never
    /// update the main projection head or canonical version source.
    pub fn commit_auxiliary_request_context(
        &self,
        session_id: &str,
        request_plan_id: &str,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(guard) = load_plan_commit_guard(&tx, session_id, request_plan_id)? else {
            tx.commit()?;
            return Ok(false);
        };
        if guard.role == RequestProjectionRole::MainContinuation {
            bail!("main request plans require commit_main_request_context");
        }
        require_live_plan_version(&tx, session_id, &guard)?;
        require_auxiliary_plan_epoch(&tx, session_id, request_plan_id, &guard)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'context_committed', updated_at = ?1
              WHERE session_id = ?2 AND request_plan_id = ?3 AND state = 'prepared'",
            params![now, session_id, request_plan_id],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn claim_request_dispatch(
        &self,
        session_id: &str,
        request_plan_id: &str,
        request_attempt_id: &str,
        provider_idempotency_key: Option<&str>,
        body: &PreparedRequestBodyIdentity,
    ) -> Result<bool> {
        validate_text("request_attempt_id", request_attempt_id, MAX_ID_BYTES)?;
        validate_optional_text(
            "provider_idempotency_key",
            provider_idempotency_key,
            MAX_ID_BYTES,
        )?;
        validate_prepared_body_identity(body)?;
        let body_bytes = unsigned_to_i64("prepared_body_bytes", body.bytes)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'dispatching', request_attempt_id = ?1,
                    provider_idempotency_key = ?2, dispatch_started_at = ?3,
                    updated_at = ?3
              WHERE session_id = ?4 AND request_plan_id = ?5
                AND state = 'context_committed'
                AND prepared_body_fingerprint = ?6 AND prepared_body_bytes = ?7
                AND endpoint_kind = ?8 AND content_type = ?9
                AND EXISTS (
                    SELECT 1
                      FROM sessions session
                      JOIN session_projection_versions version
                        ON version.session_id = session.id
                     WHERE session.id = request_projection_plans.session_id
                       AND session.context_revision =
                           request_projection_plans.expected_context_revision
                       AND version.canonical_generation =
                           request_projection_plans.expected_canonical_generation
                )
                AND (
                    projection_epoch_id IS NULL
                    OR EXISTS (
                        SELECT 1 FROM context_projection_epochs epoch
                         WHERE epoch.epoch_id = request_projection_plans.projection_epoch_id
                           AND epoch.scope = 'request_local'
                           AND epoch.owner_request_plan_id =
                               request_projection_plans.request_plan_id
                           AND epoch.state = 'active'
                    )
                    OR EXISTS (
                        SELECT 1 FROM session_projection_heads head
                         WHERE head.session_id = request_projection_plans.session_id
                           AND head.branch_id = request_projection_plans.branch_id
                           AND head.cache_identity_hash =
                               request_projection_plans.cache_identity_hash
                           AND head.epoch_id = request_projection_plans.projection_epoch_id
                    )
                )",
            params![
                request_attempt_id,
                provider_idempotency_key,
                now,
                session_id,
                request_plan_id,
                body.fingerprint,
                body_bytes,
                body.endpoint_kind,
                body.content_type,
            ],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn mark_request_response_started(
        &self,
        session_id: &str,
        request_plan_id: &str,
        provider_attempt: u32,
        status: u16,
        provider_request_id: Option<&str>,
    ) -> Result<bool> {
        if !(100..=599).contains(&status) {
            bail!("response status must be an HTTP status code");
        }
        validate_optional_text("provider_request_id", provider_request_id, MAX_ID_BYTES)?;
        transition_from_dispatching(
            self,
            session_id,
            request_plan_id,
            RequestProjectionPlanState::ResponseStarted,
            provider_request_id,
            Some(provider_attempt),
            Some(status),
            None,
        )
    }

    pub fn mark_request_send_unknown(
        &self,
        session_id: &str,
        request_plan_id: &str,
        reason: &str,
    ) -> Result<bool> {
        validate_text("send_unknown_reason", reason, MAX_OUTCOME_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let payload_availability = tx
            .query_row(
                "SELECT payload_availability FROM request_projection_plans
                  WHERE session_id = ?1 AND request_plan_id = ?2
                    AND state = 'dispatching'",
                params![session_id, request_plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(payload_availability) = payload_availability else {
            tx.commit()?;
            return Ok(false);
        };
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'send_unknown', terminal_outcome = ?1, updated_at = ?2
              WHERE session_id = ?3 AND request_plan_id = ?4
                AND state = 'dispatching'",
            params![reason, now, session_id, request_plan_id],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(false);
        }
        if payload_availability == ExactPayloadAvailability::Stored.as_str() {
            let held = tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'send_unknown', updated_at = ?2
                  WHERE owner_id = ?1 AND session_id = ?3
                    AND owner_state IN ('active', 'send_unknown')",
                params![request_plan_id, now, session_id],
            )?;
            if held != 1 {
                bail!("stored send-unknown request lost its exact-payload owner hold");
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// Restart reconciliation for the only ambiguous pre-response state. It
    /// deliberately shares the strict Dispatching -> SendUnknown edge; it
    /// never rewinds, retries, or supersedes a possibly-sent request.
    pub fn reconcile_interrupted_dispatch(
        &self,
        session_id: &str,
        request_plan_id: &str,
        reason: &str,
    ) -> Result<bool> {
        self.mark_request_send_unknown(session_id, request_plan_id, reason)
    }

    pub fn complete_response_started_request(
        &self,
        session_id: &str,
        request_plan_id: &str,
        outcome: &str,
    ) -> Result<bool> {
        complete_request_from(
            self,
            session_id,
            request_plan_id,
            RequestProjectionPlanState::ResponseStarted,
            outcome,
        )
    }

    pub fn resolve_send_unknown_request(
        &self,
        session_id: &str,
        request_plan_id: &str,
        outcome: &str,
    ) -> Result<bool> {
        complete_request_from(
            self,
            session_id,
            request_plan_id,
            RequestProjectionPlanState::SendUnknown,
            outcome,
        )
    }

    /// Treat a newly admitted foreground user run as an explicit decision to
    /// retry with a brand-new request. Every older ambiguous request for the
    /// session is terminalized atomically; no old body is replayed. The run
    /// row is the durable provenance gate, so background/side-query callers
    /// cannot manufacture this transition merely by calling the API.
    pub(crate) fn resolve_send_unknown_for_manual_foreground_run(
        &self,
        session_id: &str,
        new_run_id: &str,
    ) -> Result<usize> {
        validate_text("session_id", session_id, MAX_ID_BYTES)?;
        validate_text("new_run_id", new_run_id, MAX_ID_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed =
            resolve_send_unknown_for_manual_foreground_run_in_tx(&tx, session_id, new_run_id)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Supersede only a request that is provably unsent. Once dispatch begins,
    /// neither this API nor the DB trigger permits supersession.
    pub fn supersede_unsent_request(
        &self,
        session_id: &str,
        request_plan_id: &str,
        reason: &str,
    ) -> Result<bool> {
        validate_text("supersede_reason", reason, MAX_OUTCOME_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'superseded', terminal_outcome = ?1, updated_at = ?2
              WHERE session_id = ?3 AND request_plan_id = ?4
                AND state IN ('prepared', 'context_committed')",
            params![reason, now, session_id, request_plan_id],
        )?;
        if changed == 1 {
            Self::clear_capacity_projection_requirement_for_unsent_plan_in_tx(
                &tx,
                session_id,
                request_plan_id,
            )?;
            revoke_request_local_epoch(&tx, session_id, request_plan_id)?;
            tx.execute(
                "UPDATE request_payload_objects
                    SET object_state = 'scrub_pending',
                        retention_state = 'release_pending',
                        scrub_reason = 'request_superseded', updated_at = ?2
                  WHERE owner_id = ?1 AND object_state = 'live'",
                params![request_plan_id, now],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Safely terminate a prepared/context-committed plan whose exact payload
    /// was unavailable or lost across restart. Stored plans are intentionally
    /// excluded: their recovery policy belongs to the payload owner.
    pub fn reconcile_unrecoverable_unsent_request(
        &self,
        session_id: &str,
        request_plan_id: &str,
        reason: &str,
    ) -> Result<bool> {
        validate_text("reconcile_reason", reason, MAX_OUTCOME_BYTES)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE request_projection_plans
                SET state = 'superseded', terminal_outcome = ?1, updated_at = ?2
              WHERE session_id = ?3 AND request_plan_id = ?4
                AND state IN ('prepared', 'context_committed')
                AND payload_availability IN ('unavailable', 'lost')",
            params![reason, now, session_id, request_plan_id],
        )?;
        if changed == 1 {
            Self::clear_capacity_projection_requirement_for_unsent_plan_in_tx(
                &tx,
                session_id,
                request_plan_id,
            )?;
            revoke_request_local_epoch(&tx, session_id, request_plan_id)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }
}

/// Transaction-scoped half of failover supersession. Stream-attempt owners use
/// this inside their existing IMMEDIATE transaction so journal attempt state
/// and all request plans change together. A possibly-sent plan makes the whole
/// operation return false without modifying any plan.
pub(super) fn supersede_unsent_run_attempt_in_tx(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    run_id: &str,
    attempt_no: u32,
    reason: &str,
) -> Result<bool> {
    validate_text("session_id", session_id, MAX_ID_BYTES)?;
    validate_text("run_id", run_id, MAX_ID_BYTES)?;
    validate_text("supersede_reason", reason, MAX_OUTCOME_BYTES)?;
    let unsafe_count = tx.query_row(
        "SELECT COUNT(*) FROM request_projection_plans
          WHERE session_id = ?1 AND run_id = ?2 AND attempt_no = ?3
            AND state IN ('dispatching', 'response_started', 'send_unknown')",
        params![session_id, run_id, i64::from(attempt_no)],
        |row| row.get::<_, i64>(0),
    )?;
    if unsafe_count != 0 {
        return Ok(false);
    }
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "UPDATE request_projection_plans
            SET state = 'superseded', terminal_outcome = ?1, updated_at = ?2
          WHERE session_id = ?3 AND run_id = ?4 AND attempt_no = ?5
            AND state IN ('prepared', 'context_committed')",
        params![reason, now, session_id, run_id, i64::from(attempt_no)],
    )?;
    tx.execute(
        "UPDATE context_projection_epochs
            SET state = 'revoked'
          WHERE session_id = ?1 AND scope = 'request_local' AND state = 'active'
            AND owner_request_plan_id IN (
                SELECT request_plan_id FROM request_projection_plans
                 WHERE session_id = ?1 AND run_id = ?2 AND attempt_no = ?3
                   AND state = 'superseded'
            )",
        params![session_id, run_id, i64::from(attempt_no)],
    )?;
    tx.execute(
        "DELETE FROM session_context_compaction_recovery
          WHERE session_id = ?1
            AND requirement_kind = 'capacity_projection'
            AND automatic_attempt_in_progress = 0
            AND source_request_plan_id IN (
                SELECT request_plan_id FROM request_projection_plans
                 WHERE session_id = ?1 AND run_id = ?2 AND attempt_no = ?3
                   AND state = 'superseded'
            )",
        params![session_id, run_id, i64::from(attempt_no)],
    )?;
    Ok(true)
}

const DROP_CONTEXT_PROJECTION_TRIGGERS: &str = r#"
DROP TRIGGER IF EXISTS session_projection_version_after_session_insert;
DROP TRIGGER IF EXISTS context_projection_epoch_immutable;
DROP TRIGGER IF EXISTS context_projection_item_session_authorized;
DROP TRIGGER IF EXISTS context_projection_item_replay_scope;
DROP TRIGGER IF EXISTS context_projection_item_fingerprint_insert;
DROP TRIGGER IF EXISTS context_projection_item_update_immutable;
DROP TRIGGER IF EXISTS context_projection_item_active_delete_guard;
DROP TRIGGER IF EXISTS context_projection_result_ref_delete_guard;
DROP TRIGGER IF EXISTS request_projection_plan_immutable;
DROP TRIGGER IF EXISTS request_projection_plan_version_insert;
DROP TRIGGER IF EXISTS request_projection_plan_epoch_scope_insert;
DROP TRIGGER IF EXISTS request_projection_payload_hold_insert;
DROP TRIGGER IF EXISTS request_projection_body_identity_insert;
DROP TRIGGER IF EXISTS request_projection_main_active_insert;
DROP TRIGGER IF EXISTS request_projection_state_transition;
DROP TRIGGER IF EXISTS request_projection_dispatch_fields;
DROP TRIGGER IF EXISTS request_projection_payload_transition;
"#;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session_projection_versions (
    session_id TEXT PRIMARY KEY,
    canonical_generation INTEGER NOT NULL DEFAULT 0 CHECK (canonical_generation >= 0),
    last_request_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_request_sequence >= 0),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS context_projection_epochs (
    epoch_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('session_head', 'request_local')),
    owner_request_plan_id TEXT,
    cache_identity_hash TEXT NOT NULL,
    parent_epoch_id TEXT,
    canonical_generation INTEGER NOT NULL CHECK (canonical_generation >= 0),
    created_at_revision INTEGER NOT NULL CHECK (created_at_revision >= 0),
    provider_request_shape TEXT NOT NULL,
    policy_fingerprint TEXT NOT NULL,
    renderer_version INTEGER NOT NULL CHECK (renderer_version > 0),
    counter_profile TEXT NOT NULL,
    trigger TEXT NOT NULL CHECK (
        trigger IN ('turn_start', 'tool_loop', 'manual', 'overflow_recovery')
    ),
    max_tier INTEGER NOT NULL CHECK (max_tier IN (0, 2)),
    earliest_changed_item_key TEXT,
    action_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'superseded', 'revoked')),
    created_at TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_epoch_id) REFERENCES context_projection_epochs(epoch_id),
    CHECK (
        (scope = 'session_head' AND owner_request_plan_id IS NULL)
        OR (scope = 'request_local' AND owner_request_plan_id IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS context_projection_items (
    epoch_id TEXT NOT NULL,
    projection_item_key TEXT NOT NULL,
    result_id TEXT,
    stable_ordinal INTEGER NOT NULL CHECK (stable_ordinal >= 0),
    action TEXT NOT NULL CHECK (action IN ('tier0_omit', 'tier2_soft', 'tier2_minimal')),
    source_guard TEXT NOT NULL,
    replacement_fingerprint TEXT NOT NULL,
    replayability TEXT NOT NULL CHECK (
        replayability IN ('managed_result', 'exact_plan_only', 'lost')
    ),
    renderer_profile TEXT NOT NULL,
    target_variant TEXT NOT NULL,
    source_plan_id TEXT,
    source_projection_item_key TEXT,
    PRIMARY KEY (epoch_id, projection_item_key),
    UNIQUE (epoch_id, stable_ordinal),
    FOREIGN KEY (epoch_id) REFERENCES context_projection_epochs(epoch_id) ON DELETE CASCADE,
    FOREIGN KEY (result_id) REFERENCES tool_result_occurrences(result_id),
    CHECK (
        source_projection_item_key IS NULL OR source_plan_id IS NOT NULL
    ),
    CHECK (
        (replayability = 'managed_result' AND result_id IS NOT NULL)
        OR (replayability != 'managed_result' AND result_id IS NULL)
    )
);

CREATE TABLE IF NOT EXISTS session_projection_heads (
    session_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    cache_identity_hash TEXT NOT NULL,
    epoch_id TEXT NOT NULL,
    projection_revision INTEGER NOT NULL CHECK (projection_revision > 0),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (session_id, branch_id, cache_identity_hash),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (epoch_id) REFERENCES context_projection_epochs(epoch_id)
);

CREATE TABLE IF NOT EXISTS request_projection_plans (
    request_plan_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    attempt_no INTEGER NOT NULL CHECK (attempt_no >= 0),
    request_role TEXT NOT NULL CHECK (
        request_role IN ('main_continuation', 'tier3_summary_input', 'side_query')
    ),
    request_sequence INTEGER NOT NULL CHECK (request_sequence >= 0),
    expected_canonical_generation INTEGER NOT NULL CHECK (expected_canonical_generation >= 0),
    expected_context_revision INTEGER NOT NULL CHECK (expected_context_revision >= 0),
    projection_epoch_id TEXT,
    cache_identity_hash TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    provider_profile_id TEXT,
    model_id TEXT NOT NULL,
    request_shape TEXT NOT NULL,
    writer_version INTEGER NOT NULL CHECK (writer_version > 0),
    renderer_version INTEGER NOT NULL CHECK (renderer_version > 0),
    policy_fingerprint TEXT NOT NULL,
    counter_profile TEXT NOT NULL,
    exact_payload_id TEXT,
    exact_payload_reservation_id TEXT,
    exact_payload_keyed_digest TEXT,
    exact_payload_storage_kind TEXT,
    exact_payload_stored_bytes INTEGER CHECK (exact_payload_stored_bytes >= 0),
    payload_availability TEXT NOT NULL CHECK (
        payload_availability IN ('stored', 'unavailable', 'lost')
    ),
    projection_bytes INTEGER NOT NULL CHECK (projection_bytes >= 0),
    expires_at TEXT,
    final_capacity_count_json TEXT NOT NULL,
    prepared_body_fingerprint TEXT NOT NULL,
    prepared_body_bytes INTEGER NOT NULL CHECK (prepared_body_bytes >= 0),
    endpoint_kind TEXT NOT NULL,
    content_type TEXT NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN (
            'prepared', 'context_committed', 'dispatching', 'response_started',
            'send_unknown', 'terminal', 'superseded'
        )
    ),
    active_key INTEGER GENERATED ALWAYS AS (
        CASE WHEN state NOT IN ('terminal', 'superseded') THEN 1 ELSE NULL END
    ) VIRTUAL,
    request_attempt_id TEXT,
    provider_idempotency_key TEXT,
    provider_request_id TEXT,
    dispatch_started_at TEXT,
    response_started_at TEXT,
    response_provider_attempt INTEGER CHECK (response_provider_attempt >= 0),
    response_status INTEGER CHECK (response_status BETWEEN 100 AND 599),
    terminal_outcome TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (projection_epoch_id) REFERENCES context_projection_epochs(epoch_id),
    CHECK (
        (payload_availability = 'stored'
            AND exact_payload_id IS NOT NULL
            AND exact_payload_reservation_id IS NOT NULL
            AND exact_payload_keyed_digest IS NOT NULL
            AND exact_payload_storage_kind IS NOT NULL
            AND exact_payload_stored_bytes = prepared_body_bytes)
        OR (payload_availability != 'stored'
            AND exact_payload_id IS NULL
            AND exact_payload_reservation_id IS NULL
            AND exact_payload_keyed_digest IS NULL
            AND exact_payload_storage_kind IS NULL
            AND exact_payload_stored_bytes IS NULL)
    ),
    CHECK (
        (state IN ('terminal', 'superseded', 'send_unknown')
            AND terminal_outcome IS NOT NULL)
        OR (state NOT IN ('terminal', 'superseded', 'send_unknown'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_request_projection_sequence
    ON request_projection_plans (
        session_id, request_sequence
    );
CREATE INDEX IF NOT EXISTS idx_projection_epochs_session_generation
    ON context_projection_epochs (
        session_id, branch_id, canonical_generation, cache_identity_hash, created_at_revision
    );
CREATE INDEX IF NOT EXISTS idx_projection_items_ordinal
    ON context_projection_items (epoch_id, stable_ordinal);
CREATE INDEX IF NOT EXISTS idx_request_projection_session_state
    ON request_projection_plans (session_id, branch_id, state, created_at);
CREATE INDEX IF NOT EXISTS idx_request_projection_run_state
    ON request_projection_plans (run_id, state, request_sequence);
CREATE UNIQUE INDEX IF NOT EXISTS idx_request_projection_active_key
    ON request_projection_plans (
        session_id, branch_id, request_role, active_key
    )
    WHERE active_key IS NOT NULL
      AND request_role = 'main_continuation'
      AND state != 'send_unknown';

CREATE TRIGGER IF NOT EXISTS request_projection_main_active_insert
BEFORE INSERT ON request_projection_plans
WHEN NEW.request_role = 'main_continuation'
 AND NEW.state NOT IN ('terminal', 'superseded')
 AND EXISTS (
     SELECT 1 FROM request_projection_plans existing
      WHERE existing.session_id = NEW.session_id
        AND existing.branch_id = NEW.branch_id
        AND existing.request_role = 'main_continuation'
        AND existing.state NOT IN ('terminal', 'superseded')
 )
BEGIN
    SELECT RAISE(ABORT, 'another main request plan is still active');
END;

CREATE TRIGGER IF NOT EXISTS session_projection_version_after_session_insert
AFTER INSERT ON sessions
WHEN NEW.incognito = 0
BEGIN
    INSERT OR IGNORE INTO session_projection_versions (
        session_id, canonical_generation, last_request_sequence
    ) VALUES (NEW.id, 0, 0);
END;

CREATE TRIGGER IF NOT EXISTS context_projection_epoch_immutable
BEFORE UPDATE OF
    epoch_id, session_id, branch_id, scope, owner_request_plan_id,
    cache_identity_hash, parent_epoch_id, canonical_generation, created_at_revision,
    provider_request_shape, policy_fingerprint, renderer_version, counter_profile,
    trigger, max_tier, earliest_changed_item_key, action_digest, created_at
ON context_projection_epochs
BEGIN
    SELECT RAISE(ABORT, 'projection epoch metadata is immutable');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_item_session_authorized
BEFORE INSERT ON context_projection_items
WHEN NEW.result_id IS NOT NULL AND NOT EXISTS (
    SELECT 1
      FROM context_projection_epochs epoch
      JOIN session_result_refs ref
        ON ref.session_id = epoch.session_id AND ref.result_id = NEW.result_id
     WHERE epoch.epoch_id = NEW.epoch_id
)
BEGIN
    SELECT RAISE(ABORT, 'projection result is not authorized for epoch session');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_item_replay_scope
BEFORE INSERT ON context_projection_items
WHEN EXISTS (
    SELECT 1 FROM context_projection_epochs epoch
     WHERE epoch.epoch_id = NEW.epoch_id
       AND (
           (epoch.scope = 'session_head'
               AND (NEW.result_id IS NULL OR NEW.replayability != 'managed_result'))
           OR (NEW.result_id IS NULL
               AND (epoch.scope != 'request_local'
                    OR NEW.replayability = 'managed_result'
                    OR NEW.source_plan_id IS NULL
                    OR NEW.source_plan_id != epoch.owner_request_plan_id))
       )
)
BEGIN
    SELECT RAISE(ABORT, 'projection item is not replayable in epoch scope');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_item_fingerprint_insert
BEFORE INSERT ON context_projection_items
WHEN length(NEW.source_guard) NOT BETWEEN 32 AND 128
  OR length(NEW.source_guard) % 2 != 0
  OR NEW.source_guard GLOB '*[^0-9A-Fa-f]*'
  OR length(NEW.replacement_fingerprint) NOT BETWEEN 32 AND 128
  OR length(NEW.replacement_fingerprint) % 2 != 0
  OR NEW.replacement_fingerprint GLOB '*[^0-9A-Fa-f]*'
BEGIN
    SELECT RAISE(ABORT, 'projection fingerprints must be hexadecimal keyed identities');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_item_update_immutable
BEFORE UPDATE ON context_projection_items
BEGIN
    SELECT RAISE(ABORT, 'projection item manifest is immutable');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_item_active_delete_guard
BEFORE DELETE ON context_projection_items
WHEN EXISTS (
    SELECT 1 FROM context_projection_epochs epoch
     WHERE epoch.epoch_id = OLD.epoch_id AND epoch.state = 'active'
)
BEGIN
    SELECT RAISE(ABORT, 'active projection item cannot be deleted');
END;

CREATE TRIGGER IF NOT EXISTS context_projection_result_ref_delete_guard
BEFORE DELETE ON session_result_refs
WHEN EXISTS (SELECT 1 FROM sessions WHERE id = OLD.session_id)
 AND EXISTS (
    SELECT 1
      FROM context_projection_epochs epoch
      JOIN context_projection_items item ON item.epoch_id = epoch.epoch_id
     WHERE epoch.session_id = OLD.session_id
       AND item.result_id = OLD.result_id
       AND epoch.state = 'active'
)
BEGIN
    SELECT RAISE(ABORT, 'active projection must be revoked before result authorization');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_plan_immutable
BEFORE UPDATE OF
    request_plan_id, session_id, branch_id, run_id, attempt_no, request_role,
    request_sequence, expected_canonical_generation, expected_context_revision,
    projection_epoch_id, cache_identity_hash, provider_id, provider_profile_id,
    model_id, request_shape, writer_version, renderer_version, policy_fingerprint,
    counter_profile, exact_payload_id, exact_payload_reservation_id,
    exact_payload_keyed_digest, exact_payload_storage_kind,
    exact_payload_stored_bytes, payload_availability, projection_bytes,
    expires_at, final_capacity_count_json, prepared_body_fingerprint,
    prepared_body_bytes, endpoint_kind, content_type, created_at
ON request_projection_plans
BEGIN
    SELECT RAISE(ABORT, 'request projection identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_plan_version_insert
BEFORE INSERT ON request_projection_plans
WHEN NOT EXISTS (
    SELECT 1
      FROM sessions session
      JOIN session_projection_versions version ON version.session_id = session.id
     WHERE session.id = NEW.session_id AND session.incognito = 0
       AND session.context_revision = NEW.expected_context_revision
       AND version.canonical_generation = NEW.expected_canonical_generation
       AND version.last_request_sequence = NEW.request_sequence
)
BEGIN
    SELECT RAISE(ABORT, 'request projection version/sequence is stale');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_plan_epoch_scope_insert
BEFORE INSERT ON request_projection_plans
WHEN NEW.projection_epoch_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM context_projection_epochs epoch
     WHERE epoch.epoch_id = NEW.projection_epoch_id
       AND epoch.session_id = NEW.session_id
       AND epoch.branch_id = NEW.branch_id
       AND epoch.cache_identity_hash = NEW.cache_identity_hash
       AND epoch.canonical_generation = NEW.expected_canonical_generation
       AND epoch.created_at_revision = NEW.expected_context_revision
       AND epoch.provider_request_shape = NEW.request_shape
       AND epoch.state = 'active'
       AND (
           (NEW.request_role = 'main_continuation'
               AND (epoch.scope = 'session_head'
                    OR (epoch.scope = 'request_local'
                        AND epoch.owner_request_plan_id = NEW.request_plan_id)))
           OR (NEW.request_role != 'main_continuation'
               AND epoch.scope = 'request_local'
               AND epoch.owner_request_plan_id = NEW.request_plan_id)
       )
)
BEGIN
    SELECT RAISE(ABORT, 'request projection epoch does not match role/scope');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_payload_hold_insert
BEFORE INSERT ON request_projection_plans
WHEN NOT (
    (NEW.payload_availability = 'stored'
        AND NEW.exact_payload_id IS NOT NULL
        AND NEW.exact_payload_reservation_id IS NOT NULL
        AND NEW.exact_payload_keyed_digest IS NOT NULL
        AND NEW.exact_payload_storage_kind IN ('inline_db', 'managed_blob')
        AND NEW.exact_payload_stored_bytes = NEW.prepared_body_bytes
        AND length(NEW.exact_payload_keyed_digest) BETWEEN 32 AND 128
        AND length(NEW.exact_payload_keyed_digest) % 2 = 0
        AND NEW.exact_payload_keyed_digest NOT GLOB '*[^0-9A-Fa-f]*'
        AND EXISTS (
            SELECT 1
              FROM request_payload_owners owner
              JOIN request_payload_reservations reservation
                ON reservation.owner_id = owner.owner_id
              JOIN request_payload_objects object
                ON object.owner_id = owner.owner_id
               AND object.reservation_id = reservation.reservation_id
             WHERE owner.owner_id = NEW.request_plan_id
               AND owner.session_id = NEW.session_id
               AND owner.owner_state = 'active'
               AND reservation.reservation_id = NEW.exact_payload_reservation_id
               AND reservation.payload_id = NEW.exact_payload_id
               AND reservation.quota_state = 'committed'
               AND object.payload_id = NEW.exact_payload_id
               AND object.storage_kind = NEW.exact_payload_storage_kind
               AND object.availability = 'stored'
               AND object.object_state = 'live'
               AND object.quota_state = 'committed'
               AND object.retention_state = 'retained'
               AND object.plaintext_bytes = NEW.exact_payload_stored_bytes
               AND object.keyed_digest = NEW.exact_payload_keyed_digest
        ))
    OR (NEW.payload_availability IN ('unavailable', 'lost')
        AND NEW.exact_payload_id IS NULL
        AND NEW.exact_payload_reservation_id IS NULL
        AND NEW.exact_payload_keyed_digest IS NULL
        AND NEW.exact_payload_storage_kind IS NULL
        AND NEW.exact_payload_stored_bytes IS NULL)
)
BEGIN
    SELECT RAISE(ABORT, 'request projection payload hold is inconsistent');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_body_identity_insert
BEFORE INSERT ON request_projection_plans
WHEN NEW.prepared_body_fingerprint = ''
  OR length(NEW.prepared_body_fingerprint) NOT BETWEEN 32 AND 128
  OR length(NEW.prepared_body_fingerprint) % 2 != 0
  OR NEW.prepared_body_fingerprint GLOB '*[^0-9A-Fa-f]*'
  OR NEW.endpoint_kind = ''
  OR NEW.content_type = ''
BEGIN
    SELECT RAISE(ABORT, 'request projection prepared body identity is invalid');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_state_transition
BEFORE UPDATE OF state ON request_projection_plans
WHEN NOT (
    (OLD.state = 'prepared' AND NEW.state IN ('context_committed', 'superseded'))
    OR (OLD.state = 'context_committed' AND NEW.state IN ('dispatching', 'superseded'))
    OR (OLD.state = 'dispatching' AND NEW.state IN ('response_started', 'send_unknown'))
    OR (OLD.state = 'response_started' AND NEW.state = 'terminal')
    OR (OLD.state = 'send_unknown' AND NEW.state = 'terminal')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request projection state transition');
END;

CREATE TRIGGER IF NOT EXISTS request_projection_dispatch_fields
BEFORE UPDATE OF state ON request_projection_plans
WHEN (NEW.state = 'dispatching'
        AND (NEW.request_attempt_id IS NULL OR NEW.dispatch_started_at IS NULL))
   OR (NEW.state = 'response_started'
        AND (NEW.response_started_at IS NULL
             OR NEW.response_provider_attempt IS NULL
             OR NEW.response_status IS NULL))
   OR (NEW.state IN ('send_unknown', 'terminal', 'superseded')
        AND NEW.terminal_outcome IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'request projection transition metadata is incomplete');
END;
"#;

const EPOCH_SELECT: &str = "SELECT epoch_id, session_id, branch_id, scope,
    owner_request_plan_id, cache_identity_hash, parent_epoch_id, canonical_generation,
    created_at_revision, provider_request_shape, policy_fingerprint, renderer_version,
    counter_profile, trigger, max_tier, earliest_changed_item_key, action_digest,
    state, created_at FROM context_projection_epochs";
const ITEM_SELECT: &str = "SELECT epoch_id, projection_item_key, result_id,
    stable_ordinal, action, source_guard, replacement_fingerprint, replayability,
    renderer_profile, target_variant, source_plan_id,
    source_projection_item_key FROM context_projection_items";
const HEAD_SELECT: &str = "SELECT session_id, branch_id, cache_identity_hash,
    epoch_id, projection_revision, updated_at FROM session_projection_heads";
const PLAN_SELECT: &str = "SELECT request_plan_id, session_id, branch_id, run_id,
    attempt_no, request_role, request_sequence, expected_canonical_generation,
    expected_context_revision, projection_epoch_id, cache_identity_hash, provider_id,
    provider_profile_id, model_id, request_shape, writer_version, renderer_version,
    policy_fingerprint, counter_profile, exact_payload_id, exact_payload_reservation_id,
    exact_payload_keyed_digest, exact_payload_storage_kind, exact_payload_stored_bytes,
    payload_availability, projection_bytes, expires_at, final_capacity_count_json,
    prepared_body_fingerprint, prepared_body_bytes, endpoint_kind, content_type, state,
    request_attempt_id, provider_idempotency_key, provider_request_id, dispatch_started_at,
    response_started_at, response_provider_attempt, response_status, terminal_outcome,
    created_at, updated_at
    FROM request_projection_plans";

fn validate_epoch(epoch: &NewProjectionEpoch, items: &[NewProjectionItem]) -> Result<()> {
    validate_text("epoch_id", &epoch.epoch_id, MAX_ID_BYTES)?;
    validate_text("session_id", &epoch.session_id, MAX_ID_BYTES)?;
    validate_text("branch_id", &epoch.branch_id, MAX_ID_BYTES)?;
    validate_text(
        "cache_identity_hash",
        &epoch.cache_identity_hash,
        MAX_FINGERPRINT_BYTES,
    )?;
    validate_text(
        "provider_request_shape",
        &epoch.provider_request_shape,
        MAX_PROFILE_BYTES,
    )?;
    validate_text(
        "policy_fingerprint",
        &epoch.policy_fingerprint,
        MAX_FINGERPRINT_BYTES,
    )?;
    validate_text("counter_profile", &epoch.counter_profile, MAX_PROFILE_BYTES)?;
    validate_text("action_digest", &epoch.action_digest, MAX_FINGERPRINT_BYTES)?;
    if epoch.canonical_generation < 0 || epoch.created_at_revision < 0 {
        bail!("projection epoch generations/revisions must be nonnegative");
    }
    if epoch.renderer_version == 0 || !matches!(epoch.max_tier, 0 | 2) {
        bail!("invalid projection renderer version or max tier");
    }
    match epoch.scope {
        ProjectionEpochScope::SessionHead if epoch.owner_request_plan_id.is_some() => {
            bail!("session-head epochs cannot have an owner request plan")
        }
        ProjectionEpochScope::RequestLocal if epoch.owner_request_plan_id.is_none() => {
            bail!("request-local epochs require an owner request plan")
        }
        _ => {}
    }
    let mut ordinals = std::collections::HashSet::new();
    let mut keys = std::collections::HashSet::new();
    for item in items {
        validate_text(
            "projection_item_key",
            &item.projection_item_key,
            MAX_ID_BYTES,
        )?;
        validate_text(
            "renderer_profile",
            &item.renderer_profile,
            MAX_PROFILE_BYTES,
        )?;
        validate_text("source_guard", &item.source_guard, MAX_FINGERPRINT_BYTES)?;
        validate_hex_fingerprint("source_guard", &item.source_guard)?;
        validate_text(
            "replacement_fingerprint",
            &item.replacement_fingerprint,
            MAX_FINGERPRINT_BYTES,
        )?;
        validate_hex_fingerprint("replacement_fingerprint", &item.replacement_fingerprint)?;
        validate_text("target_variant", &item.target_variant, MAX_PROFILE_BYTES)?;
        match (epoch.scope, item.replayability, item.result_id.as_deref()) {
            (
                ProjectionEpochScope::SessionHead,
                ProjectionReplayability::ManagedResult,
                Some(_),
            ) => {}
            (ProjectionEpochScope::SessionHead, _, _) => {
                bail!("session-head projection items require managed result replay")
            }
            (
                ProjectionEpochScope::RequestLocal,
                ProjectionReplayability::ManagedResult,
                Some(_),
            ) => {}
            (ProjectionEpochScope::RequestLocal, ProjectionReplayability::ManagedResult, None) => {
                bail!("managed-result projection items require result_id")
            }
            (ProjectionEpochScope::RequestLocal, _, Some(_)) => {
                bail!("non-managed projection items cannot claim result_id")
            }
            (ProjectionEpochScope::RequestLocal, _, None) => {
                if item.source_plan_id.as_deref() != epoch.owner_request_plan_id.as_deref() {
                    bail!("request-only projection items must name their owning exact plan")
                }
            }
        }
        if !ordinals.insert(item.stable_ordinal) || !keys.insert(&item.projection_item_key) {
            bail!("projection items require unique keys and stable ordinals");
        }
    }
    Ok(())
}

fn validate_plan(plan: &NewRequestProjectionPlan) -> Result<()> {
    for (name, value, max) in [
        (
            "request_plan_id",
            plan.request_plan_id.as_str(),
            MAX_ID_BYTES,
        ),
        ("session_id", plan.session_id.as_str(), MAX_ID_BYTES),
        ("branch_id", plan.branch_id.as_str(), MAX_ID_BYTES),
        ("run_id", plan.run_id.as_str(), MAX_ID_BYTES),
        ("provider_id", plan.provider_id.as_str(), MAX_ID_BYTES),
        ("model_id", plan.model_id.as_str(), MAX_ID_BYTES),
        (
            "cache_identity_hash",
            plan.cache_identity_hash.as_str(),
            MAX_FINGERPRINT_BYTES,
        ),
        (
            "request_shape",
            plan.request_shape.as_str(),
            MAX_PROFILE_BYTES,
        ),
        (
            "policy_fingerprint",
            plan.policy_fingerprint.as_str(),
            MAX_FINGERPRINT_BYTES,
        ),
        (
            "counter_profile",
            plan.counter_profile.as_str(),
            MAX_PROFILE_BYTES,
        ),
        (
            "final_capacity_count_json",
            plan.final_capacity_count_json.as_str(),
            MAX_OUTCOME_BYTES,
        ),
        (
            "prepared_body_fingerprint",
            plan.prepared_body_fingerprint.as_str(),
            MAX_FINGERPRINT_BYTES,
        ),
        (
            "endpoint_kind",
            plan.endpoint_kind.as_str(),
            MAX_PROFILE_BYTES,
        ),
        (
            "content_type",
            plan.content_type.as_str(),
            MAX_PROFILE_BYTES,
        ),
    ] {
        validate_text(name, value, max)?;
    }
    validate_optional_text(
        "provider_profile_id",
        plan.provider_profile_id.as_deref(),
        MAX_ID_BYTES,
    )?;
    if let Some(digest) = plan.exact_payload_keyed_digest.as_deref() {
        validate_hex_fingerprint("exact_payload_keyed_digest", digest)?;
    }
    validate_optional_text(
        "exact_payload_id",
        plan.exact_payload_id.as_deref(),
        MAX_ID_BYTES,
    )?;
    validate_optional_text(
        "exact_payload_reservation_id",
        plan.exact_payload_reservation_id.as_deref(),
        MAX_ID_BYTES,
    )?;
    validate_optional_text(
        "exact_payload_keyed_digest",
        plan.exact_payload_keyed_digest.as_deref(),
        MAX_FINGERPRINT_BYTES,
    )?;
    validate_optional_text(
        "exact_payload_storage_kind",
        plan.exact_payload_storage_kind.as_deref(),
        MAX_PROFILE_BYTES,
    )?;
    if plan
        .exact_payload_storage_kind
        .as_deref()
        .is_some_and(|kind| !matches!(kind, "inline_db" | "managed_blob"))
    {
        bail!("persistent exact payload storage kind must be inline_db or managed_blob");
    }
    if plan.expected_canonical_generation < 0 || plan.expected_context_revision < 0 {
        bail!("request plan generations/revisions must be nonnegative");
    }
    if plan.writer_version == 0 || plan.renderer_version == 0 {
        bail!("request plan writer/renderer versions must be positive");
    }
    unsigned_to_i64("projection_bytes", plan.projection_bytes)?;
    unsigned_to_i64("prepared_body_bytes", plan.prepared_body_bytes)?;
    validate_hex_fingerprint("prepared_body_fingerprint", &plan.prepared_body_fingerprint)?;
    if let Some(bytes) = plan.exact_payload_stored_bytes {
        unsigned_to_i64("exact_payload_stored_bytes", bytes)?;
    }
    let has_complete_hold = plan.exact_payload_id.is_some()
        && plan.exact_payload_reservation_id.is_some()
        && plan.exact_payload_keyed_digest.is_some()
        && plan.exact_payload_storage_kind.is_some()
        && plan.exact_payload_stored_bytes == Some(plan.prepared_body_bytes);
    let has_any_hold = plan.exact_payload_id.is_some()
        || plan.exact_payload_reservation_id.is_some()
        || plan.exact_payload_keyed_digest.is_some()
        || plan.exact_payload_storage_kind.is_some()
        || plan.exact_payload_stored_bytes.is_some();
    match plan.payload_availability {
        ExactPayloadAvailability::Stored if !has_complete_hold => {
            bail!("stored exact payloads require id, reservation, and keyed digest")
        }
        ExactPayloadAvailability::Unavailable | ExactPayloadAvailability::Lost if has_any_hold => {
            bail!("unavailable/lost exact payloads cannot retain a fake storage hold")
        }
        _ => {}
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    if value.len() > max {
        bail!("{name} exceeds {max} bytes");
    }
    Ok(())
}

fn validate_optional_text(name: &str, value: Option<&str>, max: usize) -> Result<()> {
    if let Some(value) = value {
        validate_text(name, value, max)?;
    }
    Ok(())
}

fn validate_prepared_body_identity(body: &PreparedRequestBodyIdentity) -> Result<()> {
    validate_text(
        "prepared_body_fingerprint",
        &body.fingerprint,
        MAX_FINGERPRINT_BYTES,
    )?;
    validate_hex_fingerprint("prepared_body_fingerprint", &body.fingerprint)?;
    validate_text("endpoint_kind", &body.endpoint_kind, MAX_PROFILE_BYTES)?;
    validate_text("content_type", &body.content_type, MAX_PROFILE_BYTES)?;
    unsigned_to_i64("prepared_body_bytes", body.bytes)?;
    Ok(())
}

fn validate_hex_fingerprint(name: &str, value: &str) -> Result<()> {
    if !(32..=128).contains(&value.len())
        || value.len() % 2 != 0
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("{name} must be a 32-128 character hexadecimal keyed fingerprint");
    }
    Ok(())
}

fn unsigned_to_i64(name: &str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow!("{name} exceeds SQLite INTEGER range"))
}

fn require_persistent_session(conn: &rusqlite::Connection, session_id: &str) -> Result<()> {
    let incognito = conn
        .query_row(
            "SELECT incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?;
    match incognito {
        None => bail!("session not found: {session_id}"),
        Some(true) => bail!("incognito sessions cannot write durable projection state"),
        Some(false) => Ok(()),
    }
}

fn validate_parent_epoch(conn: &rusqlite::Connection, epoch: &NewProjectionEpoch) -> Result<()> {
    let Some(parent_id) = epoch.parent_epoch_id.as_deref() else {
        return Ok(());
    };
    let parent = conn
        .query_row(
            "SELECT session_id, branch_id, cache_identity_hash, canonical_generation
               FROM context_projection_epochs WHERE epoch_id = ?1",
            params![parent_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("parent projection epoch not found: {parent_id}"))?;
    if parent.0 != epoch.session_id
        || parent.1 != epoch.branch_id
        || parent.2 != epoch.cache_identity_hash
        || parent.3 != epoch.canonical_generation
    {
        bail!("parent projection epoch belongs to a different scope/generation");
    }
    Ok(())
}

fn validate_projection_item_authorization(
    conn: &rusqlite::Connection,
    epoch: &NewProjectionEpoch,
    items: &[NewProjectionItem],
) -> Result<()> {
    for item in items {
        let Some(result_id) = item.result_id.as_deref() else {
            continue;
        };
        let authorized = conn.query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM session_result_refs
                  WHERE session_id = ?1 AND result_id = ?2
             )",
            params![epoch.session_id, result_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !authorized {
            bail!("projection result is not authorized for epoch session: {result_id}");
        }
    }
    Ok(())
}

fn insert_projection_epoch_in_tx(
    conn: &rusqlite::Connection,
    epoch: &NewProjectionEpoch,
    items: &[NewProjectionItem],
) -> Result<()> {
    require_persistent_session(conn, &epoch.session_id)?;
    let current = query_projection_version(conn, &epoch.session_id)?;
    if current.context_revision != epoch.created_at_revision
        || current.canonical_generation != epoch.canonical_generation
    {
        bail!("projection epoch is stale relative to the SessionDB version source");
    }
    validate_parent_epoch(conn, epoch)?;
    validate_projection_item_authorization(conn, epoch, items)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO context_projection_epochs (
             epoch_id, session_id, branch_id, scope, owner_request_plan_id,
             cache_identity_hash, parent_epoch_id, canonical_generation,
             created_at_revision, provider_request_shape, policy_fingerprint,
             renderer_version, counter_profile, trigger, max_tier,
             earliest_changed_item_key, action_digest, state, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
             ?13, ?14, ?15, ?16, ?17, 'active', ?18
         )",
        params![
            epoch.epoch_id,
            epoch.session_id,
            epoch.branch_id,
            epoch.scope.as_str(),
            epoch.owner_request_plan_id,
            epoch.cache_identity_hash,
            epoch.parent_epoch_id,
            epoch.canonical_generation,
            epoch.created_at_revision,
            epoch.provider_request_shape,
            epoch.policy_fingerprint,
            i64::from(epoch.renderer_version),
            epoch.counter_profile,
            epoch.trigger.as_str(),
            i64::from(epoch.max_tier),
            epoch.earliest_changed_item_key,
            epoch.action_digest,
            now,
        ],
    )?;
    let mut ordered = items.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|item| item.stable_ordinal);
    for item in ordered {
        conn.execute(
            "INSERT INTO context_projection_items (
                 epoch_id, projection_item_key, result_id, stable_ordinal,
                 action, source_guard, replacement_fingerprint, replayability,
                 renderer_profile, target_variant, source_plan_id,
                 source_projection_item_key
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                epoch.epoch_id,
                item.projection_item_key,
                item.result_id,
                unsigned_to_i64("stable_ordinal", item.stable_ordinal)?,
                item.action.as_str(),
                item.source_guard,
                item.replacement_fingerprint,
                item.replayability.as_str(),
                item.renderer_profile,
                item.target_variant,
                item.source_plan_id,
                item.source_projection_item_key,
            ],
        )?;
    }
    Ok(())
}

fn insert_request_projection_plan_in_tx(
    conn: &rusqlite::Connection,
    plan: &NewRequestProjectionPlan,
) -> Result<RequestProjectionPlanRecord> {
    require_persistent_session(conn, &plan.session_id)?;
    require_expected_projection_version(conn, plan)?;
    let request_sequence = allocate_request_sequence(conn, &plan.session_id)?;
    let exact_payload_stored_bytes = plan
        .exact_payload_stored_bytes
        .map(|bytes| unsigned_to_i64("exact_payload_stored_bytes", bytes))
        .transpose()?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO request_projection_plans (
             request_plan_id, session_id, branch_id, run_id, attempt_no,
             request_role, request_sequence, expected_canonical_generation,
             expected_context_revision, projection_epoch_id, cache_identity_hash,
             provider_id, provider_profile_id, model_id, request_shape,
             writer_version, renderer_version, policy_fingerprint, counter_profile,
             exact_payload_id, exact_payload_reservation_id,
             exact_payload_keyed_digest, exact_payload_storage_kind,
             exact_payload_stored_bytes, payload_availability, projection_bytes,
             expires_at, final_capacity_count_json, prepared_body_fingerprint,
             prepared_body_bytes, endpoint_kind, content_type, state, created_at, updated_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
             ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
             ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, 'prepared', ?33, ?33
         )",
        params![
            plan.request_plan_id,
            plan.session_id,
            plan.branch_id,
            plan.run_id,
            i64::from(plan.attempt_no),
            plan.request_role.as_str(),
            request_sequence,
            plan.expected_canonical_generation,
            plan.expected_context_revision,
            plan.projection_epoch_id,
            plan.cache_identity_hash,
            plan.provider_id,
            plan.provider_profile_id,
            plan.model_id,
            plan.request_shape,
            i64::from(plan.writer_version),
            i64::from(plan.renderer_version),
            plan.policy_fingerprint,
            plan.counter_profile,
            plan.exact_payload_id,
            plan.exact_payload_reservation_id,
            plan.exact_payload_keyed_digest,
            plan.exact_payload_storage_kind,
            exact_payload_stored_bytes,
            plan.payload_availability.as_str(),
            unsigned_to_i64("projection_bytes", plan.projection_bytes)?,
            plan.expires_at,
            plan.final_capacity_count_json,
            plan.prepared_body_fingerprint,
            unsigned_to_i64("prepared_body_bytes", plan.prepared_body_bytes)?,
            plan.endpoint_kind,
            plan.content_type,
            now,
        ],
    )?;
    Ok(conn.query_row(
        &format!("{PLAN_SELECT} WHERE request_plan_id = ?1"),
        params![plan.request_plan_id],
        row_to_plan,
    )?)
}

fn require_session_head_epoch(
    conn: &rusqlite::Connection,
    session_id: &str,
    branch_id: &str,
    cache_identity_hash: &str,
    epoch_id: &str,
) -> Result<()> {
    let valid = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM context_projection_epochs
              WHERE epoch_id = ?1 AND session_id = ?2 AND branch_id = ?3
                AND cache_identity_hash = ?4 AND scope = 'session_head'
                AND state = 'active'
         )",
        params![epoch_id, session_id, branch_id, cache_identity_hash],
        |row| row.get::<_, bool>(0),
    )?;
    if !valid {
        bail!("projection head target is not an active session-head epoch");
    }
    Ok(())
}

fn require_plan_epoch(
    conn: &rusqlite::Connection,
    plan: &NewRequestProjectionPlan,
    epoch_id: &str,
) -> Result<()> {
    let valid = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM context_projection_epochs
              WHERE epoch_id = ?1 AND session_id = ?2 AND branch_id = ?3
                AND cache_identity_hash = ?4 AND canonical_generation = ?5
                AND created_at_revision = ?6 AND provider_request_shape = ?7
                AND state = 'active'
                AND (
                    (?8 = 'main_continuation' AND (
                        scope = 'session_head'
                        OR (scope = 'request_local' AND owner_request_plan_id = ?9)
                    ))
                    OR (?8 != 'main_continuation'
                        AND scope = 'request_local' AND owner_request_plan_id = ?9)
                )
         )",
        params![
            epoch_id,
            plan.session_id,
            plan.branch_id,
            plan.cache_identity_hash,
            plan.expected_canonical_generation,
            plan.expected_context_revision,
            plan.request_shape,
            plan.request_role.as_str(),
            plan.request_plan_id,
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !valid {
        bail!("request plan projection epoch does not match its request scope");
    }
    Ok(())
}

fn query_projection_version(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<RequestProjectionVersion> {
    conn.query_row(
        "SELECT session.context_revision, version.canonical_generation
           FROM sessions session
           JOIN session_projection_versions version ON version.session_id = session.id
          WHERE session.id = ?1 AND session.incognito = 0",
        params![session_id],
        |row| {
            Ok(RequestProjectionVersion {
                context_revision: row.get(0)?,
                canonical_generation: row.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("persistent session projection version not found: {session_id}"))
}

fn require_expected_projection_version(
    conn: &rusqlite::Connection,
    plan: &NewRequestProjectionPlan,
) -> Result<()> {
    let current = query_projection_version(conn, &plan.session_id)?;
    let expected = RequestProjectionVersion {
        context_revision: plan.expected_context_revision,
        canonical_generation: plan.expected_canonical_generation,
    };
    if current != expected {
        bail!("stale request projection version: expected {expected:?}, current {current:?}");
    }
    Ok(())
}

fn allocate_request_sequence(conn: &rusqlite::Connection, session_id: &str) -> Result<i64> {
    let current = conn.query_row(
        "SELECT last_request_sequence FROM session_projection_versions WHERE session_id = ?1",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )?;
    let next = current
        .checked_add(1)
        .ok_or_else(|| anyhow!("request sequence overflow"))?;
    let changed = conn.execute(
        "UPDATE session_projection_versions
            SET last_request_sequence = ?1
          WHERE session_id = ?2 AND last_request_sequence = ?3",
        params![next, session_id, current],
    )?;
    if changed != 1 {
        bail!("request sequence allocation conflict");
    }
    Ok(next)
}

#[derive(Debug)]
struct PlanCommitGuard {
    role: RequestProjectionRole,
    branch_id: String,
    cache_identity_hash: String,
    projection_epoch_id: Option<String>,
    expected: RequestProjectionVersion,
}

fn load_plan_commit_guard(
    conn: &rusqlite::Connection,
    session_id: &str,
    request_plan_id: &str,
) -> Result<Option<PlanCommitGuard>> {
    conn.query_row(
        "SELECT request_role, branch_id, cache_identity_hash, projection_epoch_id,
                expected_context_revision, expected_canonical_generation
           FROM request_projection_plans
          WHERE session_id = ?1 AND request_plan_id = ?2 AND state = 'prepared'",
        params![session_id, request_plan_id],
        |row| {
            Ok(PlanCommitGuard {
                role: RequestProjectionRole::from_db(&row.get::<_, String>(0)?, 0)?,
                branch_id: row.get(1)?,
                cache_identity_hash: row.get(2)?,
                projection_epoch_id: row.get(3)?,
                expected: RequestProjectionVersion {
                    context_revision: row.get(4)?,
                    canonical_generation: row.get(5)?,
                },
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn commit_new_plan_context_in_tx(
    conn: &rusqlite::Connection,
    session_id: &str,
    request_plan_id: &str,
) -> Result<()> {
    let guard = load_plan_commit_guard(conn, session_id, request_plan_id)?
        .ok_or_else(|| anyhow!("new request plan lost its prepared state"))?;
    require_live_plan_version(conn, session_id, &guard)?;
    match guard.role {
        RequestProjectionRole::MainContinuation => {
            if !commit_plan_projection_head(conn, session_id, request_plan_id, &guard, None)? {
                bail!("request context fence lost its projection head CAS");
            }
        }
        RequestProjectionRole::Tier3SummaryInput | RequestProjectionRole::SideQuery => {
            require_auxiliary_plan_epoch(conn, session_id, request_plan_id, &guard)?;
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn.execute(
        "UPDATE request_projection_plans
            SET state = 'context_committed', updated_at = ?1
          WHERE session_id = ?2 AND request_plan_id = ?3 AND state = 'prepared'",
        params![now, session_id, request_plan_id],
    )?;
    if changed != 1 {
        bail!("new request plan context transition lost its prepared CAS");
    }
    Ok(())
}

fn require_live_plan_version(
    conn: &rusqlite::Connection,
    session_id: &str,
    guard: &PlanCommitGuard,
) -> Result<()> {
    let current = query_projection_version(conn, session_id)?;
    if current != guard.expected {
        bail!(
            "request context fence is stale: expected {:?}, current {:?}",
            guard.expected,
            current
        );
    }
    Ok(())
}

fn require_auxiliary_plan_epoch(
    conn: &rusqlite::Connection,
    session_id: &str,
    request_plan_id: &str,
    guard: &PlanCommitGuard,
) -> Result<()> {
    let Some(epoch_id) = guard.projection_epoch_id.as_deref() else {
        return Ok(());
    };
    let valid = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM context_projection_epochs
              WHERE epoch_id = ?1 AND session_id = ?2 AND branch_id = ?3
                AND cache_identity_hash = ?4 AND scope = 'request_local'
                AND owner_request_plan_id = ?5 AND state = 'active'
         )",
        params![
            epoch_id,
            session_id,
            guard.branch_id,
            guard.cache_identity_hash,
            request_plan_id,
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !valid {
        bail!("auxiliary request projection is not request-local to its exact plan");
    }
    Ok(())
}

fn commit_plan_projection_head(
    conn: &rusqlite::Connection,
    session_id: &str,
    request_plan_id: &str,
    guard: &PlanCommitGuard,
    head_commit: Option<&MainProjectionHeadCommit>,
) -> Result<bool> {
    let Some(epoch_id) = guard.projection_epoch_id.as_deref() else {
        if head_commit.is_some() {
            bail!("cannot commit a projection head without a plan epoch");
        }
        return Ok(true);
    };
    let (scope, owner): (String, Option<String>) = conn.query_row(
        "SELECT scope, owner_request_plan_id FROM context_projection_epochs
          WHERE epoch_id = ?1 AND session_id = ?2 AND state = 'active'",
        params![epoch_id, session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if scope == ProjectionEpochScope::RequestLocal.as_str() {
        if owner.as_deref() != Some(request_plan_id) || head_commit.is_some() {
            bail!("request-local projections cannot be installed as session heads");
        }
        return Ok(true);
    }
    if scope != ProjectionEpochScope::SessionHead.as_str() || owner.is_some() {
        bail!("invalid main projection epoch scope");
    }
    let Some(head_commit) = head_commit else {
        bail!("session-head projection plans require an explicit head CAS");
    };
    require_session_head_epoch(
        conn,
        session_id,
        &guard.branch_id,
        &guard.cache_identity_hash,
        epoch_id,
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    let (changed, superseded_epoch): (usize, Option<&str>) = match head_commit {
        MainProjectionHeadCommit::Insert => (
            conn.execute(
                "INSERT OR IGNORE INTO session_projection_heads (
                     session_id, branch_id, cache_identity_hash, epoch_id,
                     projection_revision, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                params![
                    session_id,
                    guard.branch_id,
                    guard.cache_identity_hash,
                    epoch_id,
                    now,
                ],
            )?,
            None,
        ),
        MainProjectionHeadCommit::Replace {
            expected_epoch_id,
            expected_projection_revision,
        } => (
            conn.execute(
                "UPDATE session_projection_heads
                    SET epoch_id = ?1, projection_revision = projection_revision + 1,
                        updated_at = ?2
                  WHERE session_id = ?3 AND branch_id = ?4 AND cache_identity_hash = ?5
                    AND epoch_id = ?6 AND projection_revision = ?7",
                params![
                    epoch_id,
                    now,
                    session_id,
                    guard.branch_id,
                    guard.cache_identity_hash,
                    expected_epoch_id,
                    expected_projection_revision,
                ],
            )?,
            Some(expected_epoch_id.as_str()),
        ),
    };
    if changed != 1 {
        return Ok(false);
    }
    if let Some(old_epoch_id) = superseded_epoch {
        conn.execute(
            "UPDATE context_projection_epochs SET state = 'superseded'
              WHERE epoch_id = ?1 AND session_id = ?2 AND state = 'active'
                AND NOT EXISTS (
                    SELECT 1 FROM session_projection_heads WHERE epoch_id = ?1
                )",
            params![old_epoch_id, session_id],
        )?;
    }
    Ok(true)
}

fn transition_from_dispatching(
    db: &SessionDB,
    session_id: &str,
    request_plan_id: &str,
    next: RequestProjectionPlanState,
    provider_request_id: Option<&str>,
    response_provider_attempt: Option<u32>,
    response_status: Option<u16>,
    terminal_outcome: Option<&str>,
) -> Result<bool> {
    debug_assert!(matches!(
        next,
        RequestProjectionPlanState::ResponseStarted | RequestProjectionPlanState::SendUnknown
    ));
    let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let response_started_at =
        (next == RequestProjectionPlanState::ResponseStarted).then_some(now.as_str());
    let changed = conn.execute(
        "UPDATE request_projection_plans
            SET state = ?1, provider_request_id = COALESCE(?2, provider_request_id),
                response_started_at = COALESCE(?3, response_started_at),
                response_provider_attempt = COALESCE(?4, response_provider_attempt),
                response_status = COALESCE(?5, response_status),
                terminal_outcome = COALESCE(?6, terminal_outcome), updated_at = ?7
          WHERE session_id = ?8 AND request_plan_id = ?9 AND state = 'dispatching'",
        params![
            next.as_str(),
            provider_request_id,
            response_started_at,
            response_provider_attempt.map(i64::from),
            response_status.map(i64::from),
            terminal_outcome,
            now,
            session_id,
            request_plan_id,
        ],
    )?;
    Ok(changed == 1)
}

fn complete_request_from(
    db: &SessionDB,
    session_id: &str,
    request_plan_id: &str,
    expected: RequestProjectionPlanState,
    outcome: &str,
) -> Result<bool> {
    debug_assert!(matches!(
        expected,
        RequestProjectionPlanState::ResponseStarted | RequestProjectionPlanState::SendUnknown
    ));
    validate_text("terminal_outcome", outcome, MAX_OUTCOME_BYTES)?;
    let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().to_rfc3339();
    let changed = tx.execute(
        "UPDATE request_projection_plans
            SET state = 'terminal', terminal_outcome = ?1, updated_at = ?2
          WHERE session_id = ?3 AND request_plan_id = ?4 AND state = ?5",
        params![outcome, now, session_id, request_plan_id, expected.as_str()],
    )?;
    if changed == 1 {
        revoke_request_local_epoch(&tx, session_id, request_plan_id)?;
    }
    tx.commit()?;
    Ok(changed == 1)
}

/// Transaction-scoped manual-retry convergence. The caller supplies a newly
/// registered run; this helper independently proves that its persisted source
/// carries fresh foreground user intent before touching any ambiguous plan.
pub(super) fn resolve_send_unknown_for_manual_foreground_run_in_tx(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    new_run_id: &str,
) -> Result<usize> {
    let new_run_source = tx
        .query_row(
            "SELECT source FROM chat_stream_runs
              WHERE run_id = ?1 AND session_id = ?2 AND status = 'running'",
            params![new_run_id, session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("manual retry requires a live run for the same session"))?;
    if !matches!(
        new_run_source.as_str(),
        "desktop" | "http" | "channel" | "acp"
    ) {
        bail!("manual retry requires fresh foreground user intent");
    }

    let stored_count = tx.query_row(
        "SELECT COUNT(*) FROM request_projection_plans
          WHERE session_id = ?1 AND run_id != ?2 AND state = 'send_unknown'
            AND payload_availability = 'stored'",
        params![session_id, new_run_id],
        |row| row.get::<_, i64>(0),
    )?;
    let live_stored_count = tx.query_row(
        "SELECT COUNT(*)
           FROM request_projection_plans plan
           JOIN request_payload_owners owner
             ON owner.owner_id = plan.request_plan_id
            AND owner.session_id = plan.session_id
           JOIN request_payload_reservations reservation
             ON reservation.owner_id = plan.request_plan_id
            AND reservation.reservation_id = plan.exact_payload_reservation_id
            AND reservation.payload_id = plan.exact_payload_id
           JOIN request_payload_objects object
             ON object.owner_id = plan.request_plan_id
            AND object.payload_id = plan.exact_payload_id
            AND object.reservation_id = plan.exact_payload_reservation_id
          WHERE plan.session_id = ?1 AND plan.run_id != ?2
            AND plan.state = 'send_unknown'
            AND plan.payload_availability = 'stored'
            AND owner.owner_state = 'send_unknown'
            AND reservation.quota_state = 'committed'
            AND object.storage_kind = plan.exact_payload_storage_kind
            AND object.availability = 'stored'
            AND object.object_state = 'live'
            AND object.quota_state = 'committed'
            AND object.retention_state = 'retained'
            AND object.plaintext_bytes = plan.exact_payload_stored_bytes
            AND object.keyed_digest = plan.exact_payload_keyed_digest
            AND NOT EXISTS (
                SELECT 1 FROM request_payload_objects other
                 WHERE other.owner_id = plan.request_plan_id
                   AND other.payload_id != plan.exact_payload_id
            )",
        params![session_id, new_run_id],
        |row| row.get::<_, i64>(0),
    )?;
    // A typed lost/scrubbed object is already unrecoverable and therefore
    // cannot be replayed. Its hold may already have been released by storage
    // reconciliation; validate only immutable identity, then converge the
    // remaining audit/quota rows below. `scrub_pending` is intentionally not
    // accepted because it may still own ciphertext without a retained hold.
    let unrecoverable_stored_count = tx.query_row(
        "SELECT COUNT(*)
           FROM request_projection_plans plan
           JOIN request_payload_owners owner
             ON owner.owner_id = plan.request_plan_id
            AND owner.session_id = plan.session_id
           JOIN request_payload_reservations reservation
             ON reservation.owner_id = plan.request_plan_id
            AND reservation.reservation_id = plan.exact_payload_reservation_id
            AND reservation.payload_id = plan.exact_payload_id
           JOIN request_payload_objects object
             ON object.owner_id = plan.request_plan_id
            AND object.payload_id = plan.exact_payload_id
            AND object.reservation_id = plan.exact_payload_reservation_id
          WHERE plan.session_id = ?1 AND plan.run_id != ?2
            AND plan.state = 'send_unknown'
            AND plan.payload_availability = 'stored'
            AND owner.owner_state IN ('send_unknown', 'active', 'released')
            AND object.storage_kind = plan.exact_payload_storage_kind
            AND object.plaintext_bytes = plan.exact_payload_stored_bytes
            AND object.keyed_digest = plan.exact_payload_keyed_digest
            AND object.quota_state = 'released'
            AND object.retention_state = 'released'
            AND NOT EXISTS (
                SELECT 1 FROM request_payload_objects other
                 WHERE other.owner_id = plan.request_plan_id
                   AND other.payload_id != plan.exact_payload_id
            )
            AND (
                (object.object_state = 'lost' AND object.availability = 'lost')
                OR (object.object_state = 'scrubbed'
                    AND object.availability = 'unavailable')
            )",
        params![session_id, new_run_id],
        |row| row.get::<_, i64>(0),
    )?;
    if stored_count != live_stored_count + unrecoverable_stored_count {
        bail!("stored send-unknown payload ownership is inconsistent; refusing manual retry");
    }

    let now = chrono::Utc::now().to_rfc3339();
    let payloads_claimed = tx.execute(
        "UPDATE request_payload_objects
            SET object_state = 'scrub_pending', retention_state = 'release_pending',
                scrub_reason = 'send_unknown_resolved', updated_at = ?1
          WHERE object_state = 'live' AND retention_state = 'retained'
            AND EXISTS (
                SELECT 1 FROM request_projection_plans plan
                 WHERE plan.request_plan_id = request_payload_objects.owner_id
                   AND plan.exact_payload_id = request_payload_objects.payload_id
                   AND plan.session_id = ?2 AND plan.run_id != ?3
                   AND plan.state = 'send_unknown'
                   AND plan.payload_availability = 'stored'
            )",
        params![now, session_id, new_run_id],
    )?;
    if i64::try_from(payloads_claimed).unwrap_or(i64::MAX) != live_stored_count {
        bail!("manual retry did not claim every stored ambiguous payload");
    }
    // Lost is a typed proof that no exact body remains available. Convert it
    // directly to the scrubbed audit state and clear all storage material;
    // replay of the old body is never attempted.
    tx.execute(
        "UPDATE request_payload_objects
            SET availability = 'unavailable', object_state = 'scrubbed',
                quota_state = 'released', retention_state = 'released',
                nonce = NULL, inline_ciphertext = NULL,
                managed_blob_name = NULL, ciphertext_bytes = 0,
                scrub_reason = 'send_unknown_resolved', updated_at = ?1
          WHERE object_state IN ('lost', 'scrubbed')
            AND EXISTS (
                SELECT 1 FROM request_projection_plans plan
                 WHERE plan.request_plan_id = request_payload_objects.owner_id
                   AND plan.exact_payload_id = request_payload_objects.payload_id
                   AND plan.session_id = ?2 AND plan.run_id != ?3
                   AND plan.state = 'send_unknown'
                   AND plan.payload_availability = 'stored'
            )",
        params![now, session_id, new_run_id],
    )?;
    tx.execute(
        "UPDATE request_payload_reservations
            SET quota_state = 'released', updated_at = ?1
          WHERE reservation_id IN (
                SELECT plan.exact_payload_reservation_id
                  FROM request_projection_plans plan
                  JOIN request_payload_objects object
                    ON object.owner_id = plan.request_plan_id
                   AND object.payload_id = plan.exact_payload_id
                 WHERE plan.session_id = ?2 AND plan.run_id != ?3
                   AND plan.state = 'send_unknown'
                   AND plan.payload_availability = 'stored'
                   AND object.object_state = 'scrubbed'
            )",
        params![now, session_id, new_run_id],
    )?;
    tx.execute(
        "UPDATE request_payload_owners
            SET owner_state = 'active', updated_at = ?1
          WHERE owner_state = 'send_unknown'
            AND owner_id IN (
                SELECT request_plan_id FROM request_projection_plans
                 WHERE session_id = ?2 AND run_id != ?3
                   AND state = 'send_unknown' AND payload_availability = 'stored'
            )",
        params![now, session_id, new_run_id],
    )?;
    // `send_unknown -> released` is deliberately not a legal one-step owner
    // transition. The preceding update releases the hold first; objects with
    // typed unrecoverable state can then finish `active -> released` here.
    tx.execute(
        "UPDATE request_payload_owners
            SET owner_state = 'released', updated_at = ?1
          WHERE owner_state = 'active'
            AND NOT EXISTS (
                SELECT 1 FROM request_payload_objects other
                 WHERE other.owner_id = request_payload_owners.owner_id
                   AND other.object_state NOT IN ('scrubbed', 'lost')
            )
            AND owner_id IN (
                SELECT plan.request_plan_id
                  FROM request_projection_plans plan
                  JOIN request_payload_objects object
                    ON object.owner_id = plan.request_plan_id
                   AND object.payload_id = plan.exact_payload_id
                 WHERE plan.session_id = ?2 AND plan.run_id != ?3
                   AND plan.state = 'send_unknown'
                   AND plan.payload_availability = 'stored'
                   AND object.object_state = 'scrubbed'
            )",
        params![now, session_id, new_run_id],
    )?;
    let converged_stored_count = tx.query_row(
        "SELECT COUNT(*)
           FROM request_projection_plans plan
           JOIN request_payload_owners owner
             ON owner.owner_id = plan.request_plan_id
           JOIN request_payload_reservations reservation
             ON reservation.owner_id = plan.request_plan_id
            AND reservation.reservation_id = plan.exact_payload_reservation_id
           JOIN request_payload_objects object
             ON object.owner_id = plan.request_plan_id
            AND object.payload_id = plan.exact_payload_id
            AND object.reservation_id = plan.exact_payload_reservation_id
          WHERE plan.session_id = ?1 AND plan.run_id != ?2
            AND plan.state = 'send_unknown'
            AND plan.payload_availability = 'stored'
            AND (
                (object.object_state = 'scrub_pending'
                    AND object.retention_state = 'release_pending'
                    AND owner.owner_state = 'active'
                    AND reservation.quota_state = 'committed')
                OR (object.object_state = 'scrubbed'
                    AND object.availability = 'unavailable'
                    AND object.quota_state = 'released'
                    AND object.retention_state = 'released'
                    AND object.nonce IS NULL
                    AND object.inline_ciphertext IS NULL
                    AND object.managed_blob_name IS NULL
                    AND object.ciphertext_bytes = 0
                    AND owner.owner_state = 'released'
                    AND reservation.quota_state = 'released')
            )",
        params![session_id, new_run_id],
        |row| row.get::<_, i64>(0),
    )?;
    if converged_stored_count != stored_count {
        bail!("manual retry did not converge every ambiguous payload hold");
    }

    tx.execute(
        "UPDATE context_projection_epochs
            SET state = 'revoked'
          WHERE session_id = ?1 AND scope = 'request_local' AND state = 'active'
            AND owner_request_plan_id IN (
                SELECT request_plan_id FROM request_projection_plans
                 WHERE session_id = ?1 AND run_id != ?2
                   AND state = 'send_unknown'
            )",
        params![session_id, new_run_id],
    )?;
    let changed = tx.execute(
        "UPDATE request_projection_plans
            SET state = 'terminal', terminal_outcome = 'manual_retry_as_new', updated_at = ?1
          WHERE session_id = ?2 AND run_id != ?3 AND state = 'send_unknown'",
        params![now, session_id, new_run_id],
    )?;
    Ok(changed)
}

fn revoke_request_local_epoch(
    conn: &rusqlite::Connection,
    session_id: &str,
    request_plan_id: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE context_projection_epochs SET state = 'revoked'
          WHERE session_id = ?1 AND scope = 'request_local'
            AND owner_request_plan_id = ?2 AND state = 'active'",
        params![session_id, request_plan_id],
    )?;
    Ok(())
}

fn query_head(
    conn: &rusqlite::Connection,
    session_id: &str,
    branch_id: &str,
    cache_identity_hash: &str,
) -> Result<Option<ProjectionHeadRecord>> {
    conn.query_row(
        &format!(
            "{HEAD_SELECT} WHERE session_id = ?1 AND branch_id = ?2 AND cache_identity_hash = ?3"
        ),
        params![session_id, branch_id, cache_identity_hash],
        row_to_head,
    )
    .optional()
    .map_err(Into::into)
}

fn row_to_epoch(row: &Row<'_>) -> rusqlite::Result<ProjectionEpochRecord> {
    Ok(ProjectionEpochRecord {
        epoch_id: row.get(0)?,
        session_id: row.get(1)?,
        branch_id: row.get(2)?,
        scope: ProjectionEpochScope::from_db(&row.get::<_, String>(3)?, 3)?,
        owner_request_plan_id: row.get(4)?,
        cache_identity_hash: row.get(5)?,
        parent_epoch_id: row.get(6)?,
        canonical_generation: row.get(7)?,
        created_at_revision: row.get(8)?,
        provider_request_shape: row.get(9)?,
        policy_fingerprint: row.get(10)?,
        renderer_version: nonnegative_i64_to_u32(row.get(11)?, 11)?,
        counter_profile: row.get(12)?,
        trigger: ProjectionTrigger::from_db(&row.get::<_, String>(13)?, 13)?,
        max_tier: nonnegative_i64_to_u8(row.get(14)?, 14)?,
        earliest_changed_item_key: row.get(15)?,
        action_digest: row.get(16)?,
        state: ProjectionEpochState::from_db(&row.get::<_, String>(17)?, 17)?,
        created_at: row.get(18)?,
    })
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<ProjectionItemRecord> {
    Ok(ProjectionItemRecord {
        epoch_id: row.get(0)?,
        projection_item_key: row.get(1)?,
        result_id: row.get(2)?,
        stable_ordinal: nonnegative_i64_to_u64(row.get(3)?, 3)?,
        action: ProjectionAction::from_db(&row.get::<_, String>(4)?, 4)?,
        source_guard: row.get(5)?,
        replacement_fingerprint: row.get(6)?,
        replayability: ProjectionReplayability::from_db(&row.get::<_, String>(7)?, 7)?,
        renderer_profile: row.get(8)?,
        target_variant: row.get(9)?,
        source_plan_id: row.get(10)?,
        source_projection_item_key: row.get(11)?,
    })
}

fn row_to_head(row: &Row<'_>) -> rusqlite::Result<ProjectionHeadRecord> {
    Ok(ProjectionHeadRecord {
        session_id: row.get(0)?,
        branch_id: row.get(1)?,
        cache_identity_hash: row.get(2)?,
        epoch_id: row.get(3)?,
        projection_revision: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn row_to_plan(row: &Row<'_>) -> rusqlite::Result<RequestProjectionPlanRecord> {
    Ok(RequestProjectionPlanRecord {
        request_plan_id: row.get(0)?,
        session_id: row.get(1)?,
        branch_id: row.get(2)?,
        run_id: row.get(3)?,
        attempt_no: nonnegative_i64_to_u32(row.get(4)?, 4)?,
        request_role: RequestProjectionRole::from_db(&row.get::<_, String>(5)?, 5)?,
        request_sequence: nonnegative_i64_to_u64(row.get(6)?, 6)?,
        expected_canonical_generation: row.get(7)?,
        expected_context_revision: row.get(8)?,
        projection_epoch_id: row.get(9)?,
        cache_identity_hash: row.get(10)?,
        provider_id: row.get(11)?,
        provider_profile_id: row.get(12)?,
        model_id: row.get(13)?,
        request_shape: row.get(14)?,
        writer_version: nonnegative_i64_to_u32(row.get(15)?, 15)?,
        renderer_version: nonnegative_i64_to_u32(row.get(16)?, 16)?,
        policy_fingerprint: row.get(17)?,
        counter_profile: row.get(18)?,
        exact_payload_id: row.get(19)?,
        exact_payload_reservation_id: row.get(20)?,
        exact_payload_keyed_digest: row.get(21)?,
        exact_payload_storage_kind: row.get(22)?,
        exact_payload_stored_bytes: row
            .get::<_, Option<i64>>(23)?
            .map(|value| nonnegative_i64_to_u64(value, 23))
            .transpose()?,
        payload_availability: ExactPayloadAvailability::from_db(&row.get::<_, String>(24)?, 24)?,
        projection_bytes: nonnegative_i64_to_u64(row.get(25)?, 25)?,
        expires_at: row.get(26)?,
        final_capacity_count_json: row.get(27)?,
        prepared_body_fingerprint: row.get(28)?,
        prepared_body_bytes: nonnegative_i64_to_u64(row.get(29)?, 29)?,
        endpoint_kind: row.get(30)?,
        content_type: row.get(31)?,
        state: RequestProjectionPlanState::from_db(&row.get::<_, String>(32)?, 32)?,
        request_attempt_id: row.get(33)?,
        provider_idempotency_key: row.get(34)?,
        provider_request_id: row.get(35)?,
        dispatch_started_at: row.get(36)?,
        response_started_at: row.get(37)?,
        response_provider_attempt: row
            .get::<_, Option<i64>>(38)?
            .map(|value| nonnegative_i64_to_u32(value, 38))
            .transpose()?,
        response_status: row
            .get::<_, Option<i64>>(39)?
            .map(|value| nonnegative_i64_to_u16(value, 39))
            .transpose()?,
        terminal_outcome: row.get(40)?,
        created_at: row.get(41)?,
        updated_at: row.get(42)?,
    })
}

fn nonnegative_i64_to_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative integer in unsigned projection column",
            )),
        )
    })
}

fn nonnegative_i64_to_u32(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "integer outside u32 range in projection column",
            )),
        )
    })
}

fn nonnegative_i64_to_u8(value: i64, column: usize) -> rusqlite::Result<u8> {
    u8::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "integer outside u8 range in projection column",
            )),
        )
    })
}

fn nonnegative_i64_to_u16(value: i64, column: usize) -> rusqlite::Result<u16> {
    u16::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "integer outside u16 range in projection column",
            )),
        )
    })
}

fn invalid_text_value(column: usize, expected: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {expected}: {value}"),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open session db");
        (dir, db)
    }

    fn plan(
        session_id: &str,
        plan_id: &str,
        role: RequestProjectionRole,
        version: RequestProjectionVersion,
        projection_epoch_id: Option<String>,
    ) -> NewRequestProjectionPlan {
        NewRequestProjectionPlan {
            request_plan_id: plan_id.into(),
            session_id: session_id.into(),
            branch_id: "main".into(),
            run_id: "run-1".into(),
            attempt_no: 0,
            request_role: role,
            expected_canonical_generation: version.canonical_generation,
            expected_context_revision: version.context_revision,
            projection_epoch_id,
            cache_identity_hash: "cache-v1".into(),
            provider_id: "provider-1".into(),
            provider_profile_id: Some("profile-1".into()),
            model_id: "model-1".into(),
            request_shape: "messages-v1".into(),
            writer_version: 1,
            renderer_version: 1,
            policy_fingerprint: "policy-v1".into(),
            counter_profile: "counter-v1".into(),
            exact_payload_id: None,
            exact_payload_reservation_id: None,
            exact_payload_keyed_digest: None,
            exact_payload_storage_kind: None,
            exact_payload_stored_bytes: None,
            payload_availability: ExactPayloadAvailability::Unavailable,
            projection_bytes: 2,
            expires_at: None,
            final_capacity_count_json: "{}".into(),
            prepared_body_fingerprint: "a".repeat(64),
            prepared_body_bytes: 2,
            endpoint_kind: "chat".into(),
            content_type: "application/json".into(),
        }
    }

    fn request_local_epoch(
        session_id: &str,
        epoch_id: &str,
        plan_id: &str,
        version: RequestProjectionVersion,
    ) -> NewProjectionEpoch {
        NewProjectionEpoch {
            epoch_id: epoch_id.into(),
            session_id: session_id.into(),
            branch_id: "main".into(),
            scope: ProjectionEpochScope::RequestLocal,
            owner_request_plan_id: Some(plan_id.into()),
            cache_identity_hash: "cache-v1".into(),
            parent_epoch_id: None,
            canonical_generation: version.canonical_generation,
            created_at_revision: version.context_revision,
            provider_request_shape: "messages-v1".into(),
            policy_fingerprint: "policy-v1".into(),
            renderer_version: 1,
            counter_profile: "counter-v1".into(),
            trigger: ProjectionTrigger::OverflowRecovery,
            max_tier: 2,
            earliest_changed_item_key: Some("item-1".into()),
            action_digest: "actions-v1".into(),
        }
    }

    fn prepared_body() -> PreparedRequestBodyIdentity {
        PreparedRequestBodyIdentity {
            fingerprint: "a".repeat(64),
            bytes: 2,
            endpoint_kind: "chat".into(),
            content_type: "application/json".into(),
        }
    }

    fn create_send_unknown_plan(db: &SessionDB, session_id: &str, plan_id: &str) {
        let version = db
            .get_request_projection_version(session_id)
            .expect("projection version");
        db.create_request_projection_plan(&plan(
            session_id,
            plan_id,
            RequestProjectionRole::MainContinuation,
            version,
            None,
        ))
        .expect("create request plan");
        assert!(db
            .commit_main_request_context(session_id, plan_id, None)
            .expect("commit request context"));
        assert!(db
            .claim_request_dispatch(
                session_id,
                plan_id,
                "provider-attempt-1",
                None,
                &prepared_body(),
            )
            .expect("claim dispatch"));
        assert!(db
            .mark_request_send_unknown(session_id, plan_id, "test transport ambiguity")
            .expect("mark send unknown"));
    }

    fn create_stored_send_unknown_plan(
        db: &SessionDB,
        session_id: &str,
        plan_id: &str,
    ) -> (String, String) {
        let payload_id = uuid::Uuid::new_v4().to_string();
        let reservation_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "INSERT INTO request_payload_owners (
                    owner_id, session_id, owner_state, tombstoned_at, created_at, updated_at
                 ) VALUES (?1, ?2, 'active', NULL, ?3, ?3)",
                params![plan_id, session_id, now],
            )?;
            conn.execute(
                "INSERT INTO request_payload_reservations (
                    reservation_id, owner_id, payload_id, reserved_bytes,
                    quota_state, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 2, 'committed', ?4, ?4)",
                params![reservation_id, plan_id, payload_id, now],
            )?;
            conn.execute(
                "INSERT INTO request_payload_objects (
                    payload_id, owner_id, reservation_id, storage_kind, availability,
                    object_state, quota_state, retention_state, plaintext_bytes,
                    ciphertext_bytes, cipher_version, keyed_digest, nonce,
                    inline_ciphertext, managed_blob_name, expires_at, scrub_reason,
                    last_error, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, 'inline_db', 'stored', 'live', 'committed',
                    'retained', 2, 18, 1, ?4, ?5, ?6, NULL, NULL, NULL, NULL, ?7, ?7
                 )",
                params![
                    payload_id,
                    plan_id,
                    reservation_id,
                    "b".repeat(64),
                    vec![0_u8; 12],
                    vec![1_u8; 18],
                    now
                ],
            )?;
            Ok(())
        })
        .expect("insert stored payload fixture");

        let version = db
            .get_request_projection_version(session_id)
            .expect("projection version");
        let mut stored_plan = plan(
            session_id,
            plan_id,
            RequestProjectionRole::MainContinuation,
            version,
            None,
        );
        stored_plan.exact_payload_id = Some(payload_id.clone());
        stored_plan.exact_payload_reservation_id = Some(reservation_id.clone());
        stored_plan.exact_payload_keyed_digest = Some("b".repeat(64));
        stored_plan.exact_payload_storage_kind = Some("inline_db".into());
        stored_plan.exact_payload_stored_bytes = Some(2);
        stored_plan.payload_availability = ExactPayloadAvailability::Stored;
        db.create_request_projection_plan(&stored_plan)
            .expect("create stored request plan");
        assert!(db
            .commit_main_request_context(session_id, plan_id, None)
            .expect("commit stored request context"));
        assert!(db
            .claim_request_dispatch(
                session_id,
                plan_id,
                "provider-attempt-stored",
                None,
                &prepared_body(),
            )
            .expect("claim stored dispatch"));
        assert!(db
            .mark_request_send_unknown(session_id, plan_id, "test stored ambiguity")
            .expect("mark stored send unknown"));
        (payload_id, reservation_id)
    }

    fn make_stored_payload_unrecoverable(
        db: &SessionDB,
        plan_id: &str,
        payload_id: &str,
        reservation_id: &str,
        already_scrubbed_and_released: bool,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE request_payload_objects
                    SET availability = 'lost', object_state = 'lost',
                        quota_state = 'released', retention_state = 'released',
                        nonce = NULL, inline_ciphertext = NULL,
                        managed_blob_name = NULL, ciphertext_bytes = 0,
                        last_error = 'test payload loss', updated_at = ?2
                  WHERE payload_id = ?1",
                params![payload_id, now],
            )?;
            conn.execute(
                "UPDATE request_payload_reservations
                    SET quota_state = 'released', updated_at = ?2
                  WHERE reservation_id = ?1",
                params![reservation_id, now],
            )?;
            if already_scrubbed_and_released {
                conn.execute(
                    "UPDATE request_payload_objects
                        SET availability = 'unavailable', object_state = 'scrubbed',
                            scrub_reason = 'test prior reconciliation', updated_at = ?2
                      WHERE payload_id = ?1",
                    params![payload_id, now],
                )?;
                conn.execute(
                    "UPDATE request_payload_owners
                        SET owner_state = 'active', updated_at = ?2
                      WHERE owner_id = ?1 AND owner_state = 'send_unknown'",
                    params![plan_id, now],
                )?;
                conn.execute(
                    "UPDATE request_payload_owners
                        SET owner_state = 'released', updated_at = ?2
                      WHERE owner_id = ?1 AND owner_state = 'active'",
                    params![plan_id, now],
                )?;
            }
            Ok(())
        })
        .expect("make payload unrecoverable");
    }

    #[test]
    fn lifecycle_transition_contracts_are_forward_only() {
        assert!(RequestProjectionPlanState::Prepared
            .can_transition_to(RequestProjectionPlanState::ContextCommitted));
        assert!(RequestProjectionPlanState::Dispatching
            .can_transition_to(RequestProjectionPlanState::SendUnknown));
        assert!(RequestProjectionPlanState::Dispatching
            .can_transition_to(RequestProjectionPlanState::ResponseStarted));
        assert!(!RequestProjectionPlanState::Dispatching
            .can_transition_to(RequestProjectionPlanState::Superseded));
        assert!(!RequestProjectionPlanState::ResponseStarted
            .can_transition_to(RequestProjectionPlanState::Superseded));
        assert!(!RequestProjectionPlanState::SendUnknown
            .can_transition_to(RequestProjectionPlanState::Superseded));
        assert!(!RequestProjectionPlanState::Terminal
            .can_transition_to(RequestProjectionPlanState::Dispatching));
    }

    #[test]
    fn fresh_foreground_run_resolves_send_unknown_as_a_new_manual_request() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        create_send_unknown_plan(&db, &session.id, "ambiguous-plan");
        db.create_stream_run(&crate::session::CreateStreamRun {
            run_id: "foreground-run".into(),
            session_id: session.id.clone(),
            source: "desktop".into(),
            stream_id: Some("foreground-stream".into()),
            turn_id: None,
            provider_shape: None,
        })
        .expect("register foreground run");

        assert_eq!(
            db.resolve_send_unknown_for_manual_foreground_run(&session.id, "foreground-run")
                .expect("manual retry convergence"),
            1
        );
        let resolved = db
            .get_request_projection_plan(&session.id, "ambiguous-plan")
            .expect("read plan")
            .expect("plan");
        assert_eq!(resolved.state, RequestProjectionPlanState::Terminal);
        assert_eq!(
            resolved.terminal_outcome.as_deref(),
            Some("manual_retry_as_new")
        );
    }

    #[test]
    fn manual_retry_claims_live_stored_payload_for_scrub_without_replay() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let (payload_id, _) =
            create_stored_send_unknown_plan(&db, &session.id, "stored-ambiguous-plan");
        db.create_stream_run(&crate::session::CreateStreamRun {
            run_id: "stored-foreground-run".into(),
            session_id: session.id.clone(),
            source: "http".into(),
            stream_id: Some("stored-foreground-stream".into()),
            turn_id: None,
            provider_shape: None,
        })
        .expect("register foreground run");

        assert_eq!(
            db.resolve_send_unknown_for_manual_foreground_run(
                &session.id,
                "stored-foreground-run",
            )
            .expect("manual retry convergence"),
            1
        );
        db.with_conn_for_test(|conn| {
            let row: (String, String, String) = conn.query_row(
                "SELECT object.object_state, object.retention_state, owner.owner_state
                   FROM request_payload_objects object
                   JOIN request_payload_owners owner ON owner.owner_id = object.owner_id
                  WHERE object.payload_id = ?1",
                params![payload_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(
                row,
                (
                    "scrub_pending".into(),
                    "release_pending".into(),
                    "active".into()
                )
            );
            Ok(())
        })
        .expect("read claimed payload");
    }

    #[test]
    fn manual_retry_converges_lost_and_already_scrubbed_stored_payloads() {
        for already_scrubbed in [false, true] {
            let (_dir, db) = test_db();
            let session = db
                .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
                .expect("session");
            let plan_id = if already_scrubbed {
                "scrubbed-ambiguous-plan"
            } else {
                "lost-ambiguous-plan"
            };
            let (payload_id, reservation_id) =
                create_stored_send_unknown_plan(&db, &session.id, plan_id);
            make_stored_payload_unrecoverable(
                &db,
                plan_id,
                &payload_id,
                &reservation_id,
                already_scrubbed,
            );
            db.create_stream_run(&crate::session::CreateStreamRun {
                run_id: "replacement-foreground-run".into(),
                session_id: session.id.clone(),
                source: "acp".into(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect("register foreground run");

            assert_eq!(
                db.resolve_send_unknown_for_manual_foreground_run(
                    &session.id,
                    "replacement-foreground-run",
                )
                .expect("manual retry convergence"),
                1
            );
            let resolved = db
                .get_request_projection_plan(&session.id, plan_id)
                .expect("read plan")
                .expect("plan");
            assert_eq!(resolved.state, RequestProjectionPlanState::Terminal);
            assert_eq!(
                resolved.terminal_outcome.as_deref(),
                Some("manual_retry_as_new")
            );
            db.with_conn_for_test(|conn| {
                let row: (String, String, String, i64, String) = conn.query_row(
                    "SELECT object.object_state, object.availability, owner.owner_state,
                            object.ciphertext_bytes, reservation.quota_state
                       FROM request_payload_objects object
                       JOIN request_payload_owners owner ON owner.owner_id = object.owner_id
                       JOIN request_payload_reservations reservation
                         ON reservation.reservation_id = object.reservation_id
                      WHERE object.payload_id = ?1",
                    params![payload_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?;
                assert_eq!(
                    row,
                    (
                        "scrubbed".into(),
                        "unavailable".into(),
                        "released".into(),
                        0,
                        "released".into(),
                    )
                );
                Ok(())
            })
            .expect("read converged payload");
        }
    }

    #[test]
    fn background_or_missing_run_cannot_resolve_send_unknown() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        create_send_unknown_plan(&db, &session.id, "ambiguous-plan");
        db.create_stream_run(&crate::session::CreateStreamRun {
            run_id: "background-run".into(),
            session_id: session.id.clone(),
            source: "subagent".into(),
            stream_id: None,
            turn_id: None,
            provider_shape: None,
        })
        .expect("register background run");

        db.resolve_send_unknown_for_manual_foreground_run(&session.id, "background-run")
            .expect_err("background run must not authorize manual retry");
        db.resolve_send_unknown_for_manual_foreground_run(&session.id, "missing-run")
            .expect_err("missing run must not authorize manual retry");
        let retained = db
            .get_request_projection_plan(&session.id, "ambiguous-plan")
            .expect("read plan")
            .expect("plan");
        assert_eq!(retained.state, RequestProjectionPlanState::SendUnknown);
        assert_eq!(
            retained.terminal_outcome.as_deref(),
            Some("test transport ambiguity")
        );
    }

    #[test]
    fn epoch_validation_requires_stable_unique_ordinals() {
        let epoch = NewProjectionEpoch {
            epoch_id: "epoch-1".into(),
            session_id: "session-1".into(),
            branch_id: "main".into(),
            scope: ProjectionEpochScope::SessionHead,
            owner_request_plan_id: None,
            cache_identity_hash: "cache".into(),
            parent_epoch_id: None,
            canonical_generation: 0,
            created_at_revision: 1,
            provider_request_shape: "anthropic_messages".into(),
            policy_fingerprint: "policy".into(),
            renderer_version: 1,
            counter_profile: "counter".into(),
            trigger: ProjectionTrigger::TurnStart,
            max_tier: 2,
            earliest_changed_item_key: Some("item-a".into()),
            action_digest: "digest".into(),
        };
        let item = |key: &str| NewProjectionItem {
            projection_item_key: key.into(),
            result_id: Some("result-1".into()),
            stable_ordinal: 1,
            action: ProjectionAction::Tier2Soft,
            source_guard: "b".repeat(64),
            replacement_fingerprint: "c".repeat(64),
            replayability: ProjectionReplayability::ManagedResult,
            renderer_profile: "text-v1".into(),
            target_variant: "preview".into(),
            source_plan_id: None,
            source_projection_item_key: None,
        };

        assert!(validate_epoch(&epoch, &[item("item-a")]).is_ok());
        assert!(validate_epoch(&epoch, &[item("item-a"), item("item-b")]).is_err());
    }

    #[test]
    fn request_sequence_and_state_triggers_are_monotonic() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let version = db
            .get_request_projection_version(&session.id)
            .expect("version");
        let first = db
            .create_request_projection_plan(&plan(
                &session.id,
                "plan-1",
                RequestProjectionRole::MainContinuation,
                version,
                None,
            ))
            .expect("create first plan");
        assert_eq!(first.request_sequence, 1);
        assert!(db
            .commit_main_request_context(&session.id, "plan-1", None)
            .expect("commit context"));
        assert!(db
            .claim_request_dispatch(&session.id, "plan-1", "attempt-1", None, &prepared_body(),)
            .expect("claim"));
        let illegal = db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE request_projection_plans SET state = 'superseded',
                    terminal_outcome = 'illegal' WHERE request_plan_id = 'plan-1'",
                [],
            )?;
            Ok(())
        });
        assert!(illegal.is_err());
        assert!(db
            .mark_request_response_started(
                &session.id,
                "plan-1",
                0,
                200,
                Some("provider-request-1"),
            )
            .expect("response started"));
        assert!(!db
            .supersede_unsent_request(&session.id, "plan-1", "illegal")
            .expect("supersede CAS"));
        assert!(db
            .complete_response_started_request(&session.id, "plan-1", "completed")
            .expect("complete"));

        let second = db
            .create_request_projection_plan(&plan(
                &session.id,
                "plan-2",
                RequestProjectionRole::MainContinuation,
                version,
                None,
            ))
            .expect("create second plan");
        assert_eq!(second.request_sequence, 2);
        assert!(db
            .commit_main_request_context(&session.id, "plan-2", None)
            .expect("commit context"));
        assert!(db
            .claim_request_dispatch(&session.id, "plan-2", "attempt-2", None, &prepared_body(),)
            .expect("claim"));
        assert!(db
            .mark_request_send_unknown(&session.id, "plan-2", "transport_interrupted")
            .expect("send unknown"));
        assert!(!db
            .supersede_unsent_request(&session.id, "plan-2", "illegal")
            .expect("supersede CAS"));
        assert!(db
            .resolve_send_unknown_request(&session.id, "plan-2", "operator_resolved")
            .expect("resolve unknown"));
    }

    #[test]
    fn request_local_epoch_and_plan_are_created_atomically() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let version = db
            .get_request_projection_version(&session.id)
            .expect("version");
        let epoch = request_local_epoch(&session.id, "epoch-local", "plan-local", version);
        let item = NewProjectionItem {
            projection_item_key: "item-1".into(),
            result_id: None,
            stable_ordinal: 0,
            action: ProjectionAction::Tier2Soft,
            source_guard: "b".repeat(64),
            replacement_fingerprint: "c".repeat(64),
            replayability: ProjectionReplayability::ExactPlanOnly,
            renderer_profile: "text-v1".into(),
            target_variant: "preview".into(),
            source_plan_id: Some("plan-local".into()),
            source_projection_item_key: None,
        };
        assert!(db.insert_projection_epoch(&epoch, &[item.clone()]).is_err());
        let record = db
            .create_request_local_projection_plan(
                &epoch,
                &[item],
                &plan(
                    &session.id,
                    "plan-local",
                    RequestProjectionRole::SideQuery,
                    version,
                    Some("epoch-local".into()),
                ),
            )
            .expect("atomic local plan");
        assert_eq!(record.request_sequence, 1);
        assert!(db
            .commit_main_request_context(&session.id, "plan-local", None)
            .is_err());
        assert!(db
            .commit_auxiliary_request_context(&session.id, "plan-local")
            .expect("aux context"));
        assert!(db
            .get_projection_head(&session.id, "main", "cache-v1")
            .expect("head query")
            .is_none());
    }

    #[test]
    fn projection_result_requires_same_session_authorization() {
        let (_dir, db) = test_db();
        let source = db.create_session("ha-main").expect("source session");
        let other = db.create_session("ha-main").expect("other session");
        db.with_conn_for_test(|conn| {
            conn.execute(
                "INSERT INTO tool_result_occurrences (
                     result_id, run_id, turn_id, attempt, retry_no, group_id, call_id,
                     tool_name, effective_bytes, tool_dispatch_attempt_id, execution_key,
                     execution_phase, execution_status, tool_hook_state, capture_status,
                     delivery_role, model_readable, readback_policy, created_at
                 ) VALUES (
                     'result-1', 'run', 'turn', 0, 0, 'group', 'call', 'read', 10,
                     'dispatch', 'execution', 'outcome_known', 'ok', 'not_configured',
                     'payload_lost', 'provider_tool_result', 1, 'none', 'now'
                 )",
                [],
            )?;
            conn.execute(
                "INSERT INTO session_result_refs (
                     ref_id, session_id, result_id, created_from, created_at
                 ) VALUES ('ref-1', ?1, 'result-1', 'direct', 'now')",
                params![source.id],
            )?;
            Ok(())
        })
        .expect("seed authorized result");
        let version = db
            .get_request_projection_version(&other.id)
            .expect("version");
        let mut epoch = request_local_epoch(&other.id, "epoch-other", "plan-other", version);
        epoch.scope = ProjectionEpochScope::SessionHead;
        epoch.owner_request_plan_id = None;
        let item = NewProjectionItem {
            projection_item_key: "item-1".into(),
            result_id: Some("result-1".into()),
            stable_ordinal: 0,
            action: ProjectionAction::Tier0Omit,
            source_guard: "b".repeat(64),
            replacement_fingerprint: "c".repeat(64),
            replayability: ProjectionReplayability::ManagedResult,
            renderer_profile: "text-v1".into(),
            target_variant: "omitted".into(),
            source_plan_id: None,
            source_projection_item_key: None,
        };
        assert!(db.insert_projection_epoch(&epoch, &[item]).is_err());
    }
}
