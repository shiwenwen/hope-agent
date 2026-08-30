//! Durable metadata, encrypted-payload and authorization foundation for
//! admitted tool results.
//!
//! Payload schema and bounded readers live here, but the capability gate is
//! currently fixed closed because this process cannot yet prove that both its
//! private root and key remain inaccessible to every later model subprocess.
//! New effective bodies therefore record `availability=lost`; no raw body or
//! filesystem path is used as a fallback. `session_result_refs` remains the
//! sole durable authorization/liveness source, and a result id in message text
//! never grants access.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use rand::RngCore;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use rusqlite::{params, types::Type, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

use super::SessionDB;

const MAX_OPAQUE_ID_BYTES: usize = 512;
const MAX_TOOL_NAME_BYTES: usize = 256;
const MAX_EXECUTION_STATUS_BYTES: usize = 256;
const MAX_DIGEST_BYTES: usize = 256;
const MAX_ZERO_REF_QUERY_LIMIT: u32 = 1_000;
/// Initial Phase-B/C writer ceiling. Larger effective values still get an
/// occurrence/ref and a bounded `availability=lost` projection; raw text is
/// never used as a persistence fallback.
pub const MAX_INLINE_RESULT_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Maximum inline model-visible projection while durable readback remains
/// unavailable. Pageable tools must emit pages no larger than this value;
/// otherwise Tier 1 would omit a middle range while the tool cursor advances
/// past it, making those bytes unreachable.
pub const MAX_EFFECTIVE_RESULT_INLINE_PREVIEW_BYTES: usize = 16 * 1024;
/// Maximum complete page returned by a cursor-based tool while durable
/// ResultStore readback is unavailable. Tier 1 must treat the whole page,
/// including its continuation cursor, as an exact C0; otherwise a head/tail
/// preview could advance past bytes the model never received.
pub const MAX_RESUMABLE_TOOL_PAGE_BYTES: usize = 2 * 1024;
pub const DEFAULT_RESULT_READ_BYTES: usize = 16 * 1024;
pub const MAX_RESULT_READ_BYTES: usize = 50 * 1024;
const RESULT_PAYLOAD_CIPHER_VERSION: u32 = 1;
const RESULT_STORE_KEY_BYTES: usize = 32;
const RESULT_STORE_NONCE_BYTES: usize = 12;
const RESULT_STORE_KEY_FILE: &str = "result-store-v1.key";
const RESULT_STORE_KEY_LOCK_FILE: &str = "result-store-v1.lock";
const RESULT_STORE_KEY_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);
static RESULT_STORE_KEY: OnceLock<[u8; RESULT_STORE_KEY_BYTES]> = OnceLock::new();

/// Phase-B capability gate. This process currently has neither an OS-backed
/// key service nor a separate kernel identity that remains unreachable from a
/// later host/YOLO shell. `Isolated` protects one execution turn but is a
/// reversible session setting, so it is not a durable private-storage proof.
/// Keep body persistence and body reads fail-closed until both the storage root
/// and encryption key are inaccessible to every model tool and subprocess.
pub(crate) const fn kernel_private_storage_available() -> bool {
    false
}

macro_rules! db_string_enum {
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

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultProvenance {
        EffectiveVerified => "effective_verified",
        LegacyHookUnknown => "legacy_hook_unknown",
        RawOwnerOnly => "raw_owner_only",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultObjectLifecycle {
        Staging => "staging",
        Ready => "ready",
        Deleting => "deleting",
        Corrupt => "corrupt",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultStorageKind {
        InlineDb => "inline_db",
        ManagedBlob => "managed_blob",
        JobSpoolRef => "job_spool_ref",
        MediaRef => "media_ref",
    }
}

// Persisted availability deliberately excludes `ephemeral`: incognito results
// belong to the future bounded in-memory store and must never enter these
// tables.
db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PersistentResultAvailability {
        Stored => "stored",
        Lost => "lost",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ToolResultExecutionPhase {
        Started => "started",
        OutcomeKnown => "outcome_known",
        OutcomeUnknown => "outcome_unknown",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ToolResultHookState {
        NotConfigured => "not_configured",
        Pending => "pending",
        Started => "started",
        Completed => "completed",
        OutcomeUnknown => "outcome_unknown",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultCaptureStatus {
        Pending => "pending",
        EffectiveReady => "effective_ready",
        PayloadStored => "payload_stored",
        PayloadLost => "payload_lost",
        CaptureInterrupted => "capture_interrupted",
        SanitizerFailed => "sanitizer_failed",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultDeliveryRole {
        ProviderToolResult => "provider_tool_result",
        BackgroundUserNotification => "background_user_notification",
        ReadView => "read_view",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultReadbackPolicy {
        None => "none",
        SourceOnly => "source_only",
        SelfReadable => "self",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultRefCreatedFrom {
        Direct => "direct",
        Fork => "fork",
        Import => "import",
        BackgroundAttach => "background_attach",
    }
}

db_string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ResultViewDirection {
        Forward => "forward",
        Backward => "backward",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultViewDescriptor {
    pub start: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    pub direction: ResultViewDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultObjectMetadata {
    pub object_id: String,
    pub effective_bytes: u64,
    pub content_manifest_version: u32,
    pub provenance: ResultProvenance,
    pub lifecycle: ResultObjectLifecycle,
    pub storage_kind: ResultStorageKind,
    pub availability: PersistentResultAvailability,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultOccurrence {
    pub result_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_result_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_descriptor: Option<ResultViewDescriptor>,
    pub run_id: String,
    pub turn_id: String,
    pub attempt: u32,
    pub retry_no: u32,
    pub group_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub effective_bytes: u64,
    pub tool_dispatch_attempt_id: String,
    pub execution_key: String,
    pub execution_phase: ToolResultExecutionPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_hook_attempt_id: Option<String>,
    pub tool_hook_state: ToolResultHookState,
    pub capture_status: ResultCaptureStatus,
    pub delivery_role: ResultDeliveryRole,
    pub model_readable: bool,
    pub readback_policy: ResultReadbackPolicy,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResultRef {
    pub ref_id: String,
    pub session_id: String,
    pub result_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_block_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_item_key: Option<String>,
    pub created_from: ResultRefCreatedFrom,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResultReadDenial {
    SessionNotFound,
    IncognitoSession,
    MissingReference,
    ModelUnreadable,
    ReadbackDisabled,
    SourceOnly,
    MissingObject,
    UnverifiedProvenance,
    ObjectNotReady,
    ObjectUnavailable,
    ObjectCorrupt,
    InvalidCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedResultRead {
    pub ref_id: String,
    pub result_id: String,
    pub object: ResultObjectMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_result_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_descriptor: Option<ResultViewDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum ModelResultReadAuthorization {
    Authorized(AuthorizedResultRead),
    Denied(ModelResultReadDenial),
}

/// This is only a zero-*session*-reference candidate. A future collector must
/// also verify owner references and acquire a read/deletion lease before it
/// transitions or deletes an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZeroSessionRefResultObject {
    pub object: ResultObjectMetadata,
    pub occurrence_count: u64,
}

#[derive(Debug, Clone)]
pub struct NewResultObjectMetadata {
    pub object_id: String,
    pub digest: String,
    pub effective_bytes: u64,
    pub content_manifest_version: u32,
    pub provenance: ResultProvenance,
    pub lifecycle: ResultObjectLifecycle,
    pub storage_kind: ResultStorageKind,
    pub availability: PersistentResultAvailability,
}

#[derive(Debug, Clone)]
pub struct NewToolResultOccurrence {
    pub result_id: String,
    pub object_id: Option<String>,
    pub source_result_id: Option<String>,
    pub view_descriptor: Option<ResultViewDescriptor>,
    pub run_id: String,
    pub turn_id: String,
    pub attempt: u32,
    pub retry_no: u32,
    pub group_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub effective_bytes: u64,
    pub tool_dispatch_attempt_id: String,
    pub execution_key: String,
    pub execution_phase: ToolResultExecutionPhase,
    pub execution_status: Option<String>,
    pub tool_hook_attempt_id: Option<String>,
    pub tool_hook_state: ToolResultHookState,
    pub capture_status: ResultCaptureStatus,
    pub delivery_role: ResultDeliveryRole,
    pub model_readable: bool,
    pub readback_policy: ResultReadbackPolicy,
}

#[derive(Debug, Clone)]
pub struct NewSessionResultRef {
    pub ref_id: String,
    pub result_id: String,
    pub message_id: Option<i64>,
    pub provider_block_key: Option<String>,
    pub source_message_id: Option<i64>,
    pub source_plan_id: Option<String>,
    pub projection_item_key: Option<String>,
    pub created_from: ResultRefCreatedFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResultMetadata {
    pub result_id: String,
    pub tool_name: String,
    pub execution_status: Option<String>,
    pub effective_bytes: u64,
    pub capture_status: ResultCaptureStatus,
    pub availability: PersistentResultAvailability,
    pub readback_policy: ResultReadbackPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum ModelResultMetadataAccess {
    Authorized(ModelResultMetadata),
    Denied(ModelResultReadDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveTextPayloadRecord {
    pub availability: PersistentResultAvailability,
    pub capture_status: ResultCaptureStatus,
    pub readback_policy: ResultReadbackPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedResultTextPage {
    pub result_id: String,
    pub text: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub total_bytes: u64,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub availability: PersistentResultAvailability,
    pub integrity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultTextReadDirection {
    Forward,
    Backward,
}

impl ResultTextReadDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ResultReadCursor {
    version: u8,
    result_id: String,
    offset: u64,
    direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum ModelResultTextRead {
    Authorized(AuthorizedResultTextPage),
    Denied(ModelResultReadDenial),
}

impl SessionDB {
    pub(crate) fn ensure_result_store_tables(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS result_objects (
                object_id TEXT PRIMARY KEY,
                digest TEXT NOT NULL,
                effective_bytes INTEGER NOT NULL CHECK (effective_bytes >= 0),
                content_manifest_version INTEGER NOT NULL CHECK (content_manifest_version > 0),
                provenance TEXT NOT NULL CHECK (
                    provenance IN ('effective_verified', 'legacy_hook_unknown', 'raw_owner_only')
                ),
                lifecycle TEXT NOT NULL CHECK (
                    lifecycle IN ('staging', 'ready', 'deleting', 'corrupt')
                ),
                storage_kind TEXT NOT NULL CHECK (
                    storage_kind IN ('inline_db', 'managed_blob', 'job_spool_ref', 'media_ref')
                ),
                availability TEXT NOT NULL CHECK (availability IN ('stored', 'lost')),
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tool_result_occurrences (
                result_id TEXT PRIMARY KEY,
                object_id TEXT,
                source_result_id TEXT,
                view_start INTEGER,
                view_end INTEGER,
                view_direction TEXT,
                run_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                attempt INTEGER NOT NULL CHECK (attempt >= 0),
                retry_no INTEGER NOT NULL CHECK (retry_no >= 0),
                group_id TEXT NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                effective_bytes INTEGER NOT NULL CHECK (effective_bytes >= 0),
                tool_dispatch_attempt_id TEXT NOT NULL,
                execution_key TEXT NOT NULL,
                execution_phase TEXT NOT NULL CHECK (
                    execution_phase IN ('started', 'outcome_known', 'outcome_unknown')
                ),
                execution_status TEXT,
                tool_hook_attempt_id TEXT,
                tool_hook_state TEXT NOT NULL CHECK (
                    tool_hook_state IN (
                        'not_configured', 'pending', 'started', 'completed', 'outcome_unknown'
                    )
                ),
                capture_status TEXT NOT NULL CHECK (
                    capture_status IN (
                        'pending', 'effective_ready', 'payload_stored', 'payload_lost',
                        'capture_interrupted', 'sanitizer_failed'
                    )
                ),
                delivery_role TEXT NOT NULL CHECK (
                    delivery_role IN (
                        'provider_tool_result', 'background_user_notification', 'read_view'
                    )
                ),
                model_readable INTEGER NOT NULL CHECK (model_readable IN (0, 1)),
                readback_policy TEXT NOT NULL CHECK (
                    readback_policy IN ('none', 'source_only', 'self')
                ),
                created_at TEXT NOT NULL,
                FOREIGN KEY (object_id) REFERENCES result_objects(object_id) ON DELETE SET NULL,
                FOREIGN KEY (source_result_id) REFERENCES tool_result_occurrences(result_id),
                UNIQUE (run_id, attempt, group_id, call_id, retry_no),
                CHECK (source_result_id IS NULL OR source_result_id != result_id),
                CHECK (
                    (execution_phase = 'outcome_known' AND execution_status IS NOT NULL)
                    OR (execution_phase != 'outcome_known' AND execution_status IS NULL)
                ),
                CHECK (
                    tool_hook_state NOT IN ('started', 'completed', 'outcome_unknown')
                    OR tool_hook_attempt_id IS NOT NULL
                ),
                CHECK (
                    readback_policy != 'source_only' OR source_result_id IS NOT NULL
                ),
                CHECK (
                    delivery_role != 'read_view'
                    OR (source_result_id IS NOT NULL AND readback_policy = 'source_only')
                ),
                CHECK (
                    (view_start IS NULL AND view_end IS NULL AND view_direction IS NULL)
                    OR (
                        view_start IS NOT NULL
                        AND view_start >= 0
                        AND (view_end IS NULL OR view_end >= view_start)
                        AND view_direction IN ('forward', 'backward')
                    )
                ),
                CHECK (delivery_role != 'read_view' OR view_start IS NOT NULL)
            );

            -- The body is encrypted and physically separated from the
            -- metadata/authorization relation. No caller-provided locator or
            -- filesystem path is stored here.
            CREATE TABLE IF NOT EXISTS result_object_payloads (
                object_id TEXT PRIMARY KEY,
                cipher_version INTEGER NOT NULL CHECK (cipher_version = 1),
                nonce BLOB NOT NULL CHECK (length(nonce) = 12),
                ciphertext BLOB NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (object_id) REFERENCES result_objects(object_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS session_result_refs (
                ref_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                result_id TEXT NOT NULL,
                message_id INTEGER,
                provider_block_key TEXT,
                source_message_id INTEGER,
                source_plan_id TEXT,
                projection_item_key TEXT,
                created_from TEXT NOT NULL CHECK (
                    created_from IN ('direct', 'fork', 'import', 'background_attach')
                ),
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (result_id) REFERENCES tool_result_occurrences(result_id) ON DELETE CASCADE,
                FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE,
                FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE CASCADE,
                CHECK (provider_block_key IS NULL OR message_id IS NOT NULL),
                CHECK (projection_item_key IS NULL OR source_plan_id IS NOT NULL)
            );

            CREATE INDEX IF NOT EXISTS idx_result_occurrences_object
                ON tool_result_occurrences(object_id);
            CREATE INDEX IF NOT EXISTS idx_result_occurrences_source
                ON tool_result_occurrences(source_result_id);
            CREATE INDEX IF NOT EXISTS idx_session_result_refs_session_result
                ON session_result_refs(session_id, result_id);
            CREATE INDEX IF NOT EXISTS idx_session_result_refs_result
                ON session_result_refs(result_id);
            CREATE INDEX IF NOT EXISTS idx_session_result_refs_message
                ON session_result_refs(session_id, message_id)
                WHERE message_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_session_result_refs_plan_item
                ON session_result_refs(session_id, source_plan_id, projection_item_key)
                WHERE source_plan_id IS NOT NULL;

            CREATE TRIGGER IF NOT EXISTS result_objects_immutable_metadata
            BEFORE UPDATE OF
                object_id, digest, effective_bytes, content_manifest_version,
                provenance, storage_kind, created_at
            ON result_objects
            BEGIN
                SELECT RAISE(ABORT, 'result object metadata is immutable');
            END;

            CREATE TRIGGER IF NOT EXISTS result_object_payloads_immutable
            BEFORE UPDATE ON result_object_payloads
            BEGIN
                SELECT RAISE(ABORT, 'result payloads are immutable');
            END;

            CREATE TRIGGER IF NOT EXISTS session_result_refs_immutable
            BEFORE UPDATE ON session_result_refs
            BEGIN
                SELECT RAISE(ABORT, 'session result refs are immutable; revoke and recreate');
            END;

            CREATE TRIGGER IF NOT EXISTS session_result_refs_ready_object_insert
            BEFORE INSERT ON session_result_refs
            WHEN EXISTS (
                SELECT 1
                  FROM tool_result_occurrences occurrence
                  JOIN result_objects object ON object.object_id = occurrence.object_id
                 WHERE occurrence.result_id = NEW.result_id
                   AND object.lifecycle != 'ready'
            )
            BEGIN
                SELECT RAISE(ABORT, 'session refs require a ready result object');
            END;

            CREATE TRIGGER IF NOT EXISTS session_result_refs_message_scope_insert
            BEFORE INSERT ON session_result_refs
            WHEN (
                NEW.message_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM messages
                     WHERE id = NEW.message_id AND session_id = NEW.session_id
                )
            ) OR (
                NEW.source_message_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM messages
                     WHERE id = NEW.source_message_id AND session_id = NEW.session_id
                )
            )
            BEGIN
                SELECT RAISE(ABORT, 'result ref message must belong to the authorized session');
            END;",
        )?;
        // Reader-first development builds created the occurrence table before
        // effective byte accounting was added. Keep the migration probe-based
        // so those local databases do not need a destructive rebuild.
        if conn
            .prepare("SELECT effective_bytes FROM tool_result_occurrences LIMIT 1")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE tool_result_occurrences
                 ADD COLUMN effective_bytes INTEGER NOT NULL DEFAULT 0 CHECK (effective_bytes >= 0);",
            )?;
        }
        Ok(())
    }

    /// Atomically records an optional immutable object, one result occurrence,
    /// and the session authorization reference. It writes metadata only; no
    /// result payload or storage locator is accepted by this API.
    pub fn record_result_foundation(
        &self,
        session_id: &str,
        object: Option<&NewResultObjectMetadata>,
        occurrence: &NewToolResultOccurrence,
        reference: &NewSessionResultRef,
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        record_result_foundation_in_tx(&tx, session_id, object, occurrence, reference, None)?;
        tx.commit()?;
        Ok(())
    }

    /// Admits a PostToolUse-effective UTF-8 payload. The caller decides only
    /// whether the current execution surface is eligible for persistent body
    /// storage; this method owns encryption, digesting and the atomic
    /// object/occurrence/ref write. A body-encryption/write failure degrades to
    /// a durable `payload_lost` occurrence without ever accepting raw text as
    /// a fallback. Failure to record that metadata is returned to the caller.
    pub fn record_effective_text_payload(
        &self,
        session_id: &str,
        object_id: &str,
        occurrence: &NewToolResultOccurrence,
        reference: &NewSessionResultRef,
        effective_text: &str,
        permit_persistent_payload: bool,
    ) -> Result<EffectiveTextPayloadRecord> {
        // This read happens before key publication so an incognito call cannot
        // create even installation-level ResultStore state.
        {
            let conn = self.read_conn()?;
            require_persistent_result_session(&conn, session_id)?;
        }

        let effective_bytes =
            u64::try_from(effective_text.len()).context("effective result length exceeds u64")?;
        let may_store = permit_persistent_payload
            && kernel_private_storage_available()
            && effective_text.len() <= MAX_INLINE_RESULT_PAYLOAD_BYTES
            && occurrence.readback_policy == ResultReadbackPolicy::SelfReadable;

        if may_store {
            let digest = result_payload_digest(effective_text.as_bytes());
            let encrypted = load_or_create_result_store_key()
                .and_then(|key| encrypt_result_payload(object_id, &digest, effective_text, &key));

            if let Ok((nonce, ciphertext)) = encrypted {
                let object = NewResultObjectMetadata {
                    object_id: object_id.to_string(),
                    digest,
                    effective_bytes,
                    content_manifest_version: 1,
                    provenance: ResultProvenance::EffectiveVerified,
                    lifecycle: ResultObjectLifecycle::Ready,
                    storage_kind: ResultStorageKind::InlineDb,
                    availability: PersistentResultAvailability::Stored,
                };
                let mut stored_occurrence = occurrence.clone();
                stored_occurrence.object_id = Some(object_id.to_string());
                stored_occurrence.effective_bytes = effective_bytes;
                stored_occurrence.capture_status = ResultCaptureStatus::PayloadStored;

                let write_result = (|| -> Result<()> {
                    let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
                    let tx = conn.transaction()?;
                    record_result_foundation_in_tx(
                        &tx,
                        session_id,
                        Some(&object),
                        &stored_occurrence,
                        reference,
                        Some((&nonce, &ciphertext)),
                    )?;
                    tx.commit()?;
                    Ok(())
                })();
                if write_result.is_ok() {
                    return Ok(EffectiveTextPayloadRecord {
                        availability: PersistentResultAvailability::Stored,
                        capture_status: ResultCaptureStatus::PayloadStored,
                        readback_policy: ResultReadbackPolicy::SelfReadable,
                    });
                }
                crate::app_warn!(
                    "context",
                    "result_store_payload_write",
                    "Encrypted result payload write failed; recording availability=lost"
                );
            } else {
                crate::app_warn!(
                    "context",
                    "result_store_payload_encrypt",
                    "Result payload encryption failed; recording availability=lost"
                );
            }
        }

        let mut lost_occurrence = occurrence.clone();
        lost_occurrence.object_id = None;
        lost_occurrence.effective_bytes = effective_bytes;
        lost_occurrence.capture_status = ResultCaptureStatus::PayloadLost;
        if lost_occurrence.readback_policy != ResultReadbackPolicy::SourceOnly {
            lost_occurrence.readback_policy = ResultReadbackPolicy::None;
        }
        self.record_result_foundation(session_id, None, &lost_occurrence, reference)?;
        Ok(EffectiveTextPayloadRecord {
            availability: PersistentResultAvailability::Lost,
            capture_status: ResultCaptureStatus::PayloadLost,
            readback_policy: lost_occurrence.readback_policy,
        })
    }

    /// Adds another explicit session authorization/liveness reference to an
    /// existing occurrence. Fork/import/background callers must use this API
    /// instead of inferring access from message text or session ancestry.
    pub fn attach_existing_result_ref(
        &self,
        session_id: &str,
        reference: &NewSessionResultRef,
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        require_persistent_result_session(&tx, session_id)?;
        validate_reference(reference)?;

        let occurrence_object_id = tx
            .query_row(
                "SELECT object_id FROM tool_result_occurrences WHERE result_id = ?1",
                params![reference.result_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("result occurrence not found: {}", reference.result_id))?;
        if let Some(object_id) = occurrence_object_id {
            let (lifecycle, _) = load_object_write_guard(&tx, &object_id)?;
            if lifecycle != ResultObjectLifecycle::Ready {
                bail!("session refs require a ready result object: {object_id}");
            }
        }

        let now = chrono::Utc::now().to_rfc3339();
        insert_session_result_ref(&tx, session_id, reference, &now)?;
        tx.commit()?;
        Ok(())
    }

    /// Revokes exactly one session reference. Objects and occurrences are kept
    /// for the future owner-ref-aware collector; this method never performs
    /// payload deletion.
    pub fn remove_session_result_ref(&self, session_id: &str, ref_id: &str) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        require_persistent_result_session(&tx, session_id)?;
        validate_bounded_text("ref_id", ref_id, MAX_OPAQUE_ID_BYTES)?;
        let changed = tx.execute(
            "DELETE FROM session_result_refs WHERE session_id = ?1 AND ref_id = ?2",
            params![session_id, ref_id],
        )?;
        tx.commit()?;
        Ok(changed != 0)
    }

    /// Metadata-only object lookup. The internal digest and any future storage
    /// locator are intentionally absent from the returned type.
    pub fn get_result_object_metadata(
        &self,
        object_id: &str,
    ) -> Result<Option<ResultObjectMetadata>> {
        let conn = self.read_conn()?;
        conn.query_row(
            RESULT_OBJECT_METADATA_SELECT,
            params![object_id],
            row_to_result_object_metadata,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_tool_result_occurrence(
        &self,
        result_id: &str,
    ) -> Result<Option<ToolResultOccurrence>> {
        let conn = self.read_conn()?;
        conn.query_row(OCCURRENCE_SELECT, params![result_id], row_to_occurrence)
            .optional()
            .map_err(Into::into)
    }

    /// Returns all explicit references from one session to one occurrence.
    /// The result is metadata only and is not itself a payload-open lease.
    pub fn list_session_result_refs(
        &self,
        session_id: &str,
        result_id: &str,
    ) -> Result<Vec<SessionResultRef>> {
        let conn = self.read_conn()?;
        let mut statement = conn.prepare(&format!(
            "{SESSION_RESULT_REF_SELECT}
             WHERE session_id = ?1 AND result_id = ?2
             ORDER BY created_at, ref_id"
        ))?;
        let rows =
            statement.query_map(params![session_id, result_id], row_to_session_result_ref)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Performs the metadata half of model read authorization. No payload or
    /// storage locator is returned, so a future body reader must still acquire
    /// its read lease and repeat these predicates atomically in `open_authorized`.
    pub fn authorize_model_result_read(
        &self,
        session_id: &str,
        result_id: &str,
    ) -> Result<ModelResultReadAuthorization> {
        let conn = self.read_conn()?;
        let incognito = conn
            .query_row(
                "SELECT incognito FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(incognito) = incognito else {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::SessionNotFound,
            ));
        };
        if incognito != 0 {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::IncognitoSession,
            ));
        }
        let candidate = conn
            .query_row(
                AUTHORIZATION_SELECT,
                params![session_id, result_id],
                row_to_authorization_candidate,
            )
            .optional()?;
        let Some(candidate) = candidate else {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::MissingReference,
            ));
        };

        if !candidate.model_readable {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::ModelUnreadable,
            ));
        }
        match candidate.readback_policy {
            ResultReadbackPolicy::None => {
                return Ok(ModelResultReadAuthorization::Denied(
                    ModelResultReadDenial::ReadbackDisabled,
                ));
            }
            ResultReadbackPolicy::SourceOnly => {
                return Ok(ModelResultReadAuthorization::Denied(
                    ModelResultReadDenial::SourceOnly,
                ));
            }
            ResultReadbackPolicy::SelfReadable => {}
        }

        let Some(object) = candidate.object else {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::MissingObject,
            ));
        };
        if object.provenance != ResultProvenance::EffectiveVerified {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::UnverifiedProvenance,
            ));
        }
        if object.lifecycle != ResultObjectLifecycle::Ready {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::ObjectNotReady,
            ));
        }
        if object.availability != PersistentResultAvailability::Stored {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::ObjectUnavailable,
            ));
        }
        if !kernel_private_storage_available() {
            return Ok(ModelResultReadAuthorization::Denied(
                ModelResultReadDenial::ObjectUnavailable,
            ));
        }

        Ok(ModelResultReadAuthorization::Authorized(
            AuthorizedResultRead {
                ref_id: candidate.ref_id,
                result_id: candidate.result_id,
                object,
                source_result_id: candidate.source_result_id,
                view_descriptor: candidate.view_descriptor,
            },
        ))
    }

    /// Returns only bounded, non-sensitive metadata for an explicitly
    /// referenced result. Digests, ciphertext, keys and locators are never
    /// included in this view.
    pub fn get_model_result_metadata(
        &self,
        session_id: &str,
        result_id: &str,
    ) -> Result<ModelResultMetadataAccess> {
        let conn = self.read_conn()?;
        if let Some(denial) = persistent_result_session_denial(&conn, session_id)? {
            return Ok(ModelResultMetadataAccess::Denied(denial));
        }
        let row = conn
            .query_row(
                "SELECT occurrence.result_id, occurrence.tool_name,
                        occurrence.execution_status, occurrence.effective_bytes,
                        occurrence.capture_status,
                        COALESCE(object.availability, 'lost'),
                        occurrence.readback_policy, occurrence.model_readable
                   FROM session_result_refs refs
                   JOIN tool_result_occurrences occurrence
                     ON occurrence.result_id = refs.result_id
                   LEFT JOIN result_objects object
                     ON object.object_id = occurrence.object_id
                  WHERE refs.session_id = ?1 AND refs.result_id = ?2
                  ORDER BY refs.created_at, refs.ref_id
                  LIMIT 1",
                params![session_id, result_id],
                |row| {
                    Ok((
                        ModelResultMetadata {
                            result_id: row.get(0)?,
                            tool_name: row.get(1)?,
                            execution_status: row.get(2)?,
                            effective_bytes: nonnegative_i64_to_u64(row.get(3)?, 3)?,
                            capture_status: ResultCaptureStatus::from_db(
                                &row.get::<_, String>(4)?,
                                4,
                            )?,
                            availability: PersistentResultAvailability::from_db(
                                &row.get::<_, String>(5)?,
                                5,
                            )?,
                            readback_policy: ResultReadbackPolicy::from_db(
                                &row.get::<_, String>(6)?,
                                6,
                            )?,
                        },
                        strict_bool(row.get(7)?, 7)?,
                    ))
                },
            )
            .optional()?;
        let Some((metadata, model_readable)) = row else {
            return Ok(ModelResultMetadataAccess::Denied(
                ModelResultReadDenial::MissingReference,
            ));
        };
        if !model_readable {
            return Ok(ModelResultMetadataAccess::Denied(
                ModelResultReadDenial::ModelUnreadable,
            ));
        }
        let metadata = if kernel_private_storage_available() {
            metadata
        } else {
            ModelResultMetadata {
                availability: PersistentResultAvailability::Lost,
                readback_policy: match metadata.readback_policy {
                    ResultReadbackPolicy::SelfReadable => ResultReadbackPolicy::None,
                    other => other,
                },
                ..metadata
            }
        };
        Ok(ModelResultMetadataAccess::Authorized(metadata))
    }

    /// Reads one UTF-8-safe page from an explicitly referenced, verified
    /// effective payload. Cursors are opaque and bound to both result id and
    /// direction. The hard byte ceiling is enforced even if a caller supplies
    /// a larger value.
    pub fn read_authorized_result_text_page(
        &self,
        session_id: &str,
        result_id: &str,
        cursor: Option<&str>,
        max_bytes: Option<usize>,
        direction: ResultTextReadDirection,
    ) -> Result<ModelResultTextRead> {
        let conn = self.read_conn()?;
        if let Some(denial) = persistent_result_session_denial(&conn, session_id)? {
            return Ok(ModelResultTextRead::Denied(denial));
        }
        let candidate = conn
            .query_row(
                AUTHORIZATION_SELECT,
                params![session_id, result_id],
                row_to_authorization_candidate,
            )
            .optional()?;
        let Some(candidate) = candidate else {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::MissingReference,
            ));
        };
        let object = match authorize_candidate_for_self_read(&candidate) {
            Ok(object) => object,
            Err(denial) => return Ok(ModelResultTextRead::Denied(denial)),
        };
        if !kernel_private_storage_available() {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::ObjectUnavailable,
            ));
        }

        let encrypted = conn
            .query_row(
                "SELECT object.digest, payload.cipher_version, payload.nonce,
                        payload.ciphertext
                   FROM result_objects object
                   JOIN result_object_payloads payload
                     ON payload.object_id = object.object_id
                  WHERE object.object_id = ?1",
                params![object.object_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((digest, cipher_version, nonce, ciphertext)) = encrypted else {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::ObjectUnavailable,
            ));
        };
        if cipher_version != i64::from(RESULT_PAYLOAD_CIPHER_VERSION) {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::ObjectCorrupt,
            ));
        }
        let key = match read_result_store_key() {
            Ok(Some(key)) => key,
            Ok(None) | Err(_) => {
                return Ok(ModelResultTextRead::Denied(
                    ModelResultReadDenial::ObjectCorrupt,
                ));
            }
        };
        let text =
            match decrypt_result_payload(&object.object_id, &digest, &nonce, &ciphertext, &key) {
                Ok(text) => text,
                Err(_) => {
                    return Ok(ModelResultTextRead::Denied(
                        ModelResultReadDenial::ObjectCorrupt,
                    ));
                }
            };
        if text.len() as u64 != object.effective_bytes {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::ObjectCorrupt,
            ));
        }

        let total = text.len();
        let requested_offset = match cursor {
            Some(cursor) => match decode_result_cursor(cursor, result_id, direction) {
                Ok(offset) => usize::try_from(offset).unwrap_or(usize::MAX),
                Err(_) => {
                    return Ok(ModelResultTextRead::Denied(
                        ModelResultReadDenial::InvalidCursor,
                    ));
                }
            },
            None => match direction {
                ResultTextReadDirection::Forward => 0,
                ResultTextReadDirection::Backward => total,
            },
        };
        if requested_offset > total {
            return Ok(ModelResultTextRead::Denied(
                ModelResultReadDenial::InvalidCursor,
            ));
        }
        let limit = max_bytes
            .unwrap_or(DEFAULT_RESULT_READ_BYTES)
            // Four bytes are required to guarantee progress over one UTF-8
            // scalar while remaining far below the hard per-call ceiling.
            .clamp(4, MAX_RESULT_READ_BYTES);
        let (start, end) = match direction {
            ResultTextReadDirection::Forward => {
                let start = next_char_boundary(&text, requested_offset);
                let end = previous_char_boundary(&text, start.saturating_add(limit).min(total));
                (start, end.max(start))
            }
            ResultTextReadDirection::Backward => {
                let end = previous_char_boundary(&text, requested_offset);
                let start = next_char_boundary(&text, end.saturating_sub(limit));
                (start.min(end), end)
            }
        };
        let next_cursor = match direction {
            ResultTextReadDirection::Forward if end < total => {
                Some(encode_result_cursor(result_id, end as u64, direction)?)
            }
            ResultTextReadDirection::Backward if start > 0 => {
                Some(encode_result_cursor(result_id, start as u64, direction)?)
            }
            _ => None,
        };
        Ok(ModelResultTextRead::Authorized(AuthorizedResultTextPage {
            result_id: result_id.to_string(),
            text: text[start..end].to_string(),
            start_byte: start as u64,
            end_byte: end as u64,
            total_bytes: total as u64,
            truncated: start > 0 || end < total,
            next_cursor,
            availability: PersistentResultAvailability::Stored,
            integrity: "verified".to_string(),
        }))
    }

    /// Lists objects with no live session reference through any occurrence.
    /// This is a discovery query only, not permission to delete.
    pub fn list_zero_session_ref_result_objects(
        &self,
        limit: u32,
    ) -> Result<Vec<ZeroSessionRefResultObject>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::from(limit.min(MAX_ZERO_REF_QUERY_LIMIT));
        let conn = self.read_conn()?;
        let mut statement = conn.prepare(ZERO_SESSION_REF_OBJECTS_SELECT)?;
        let rows = statement.query_map(params![limit], |row| {
            let object = row_to_result_object_metadata(row)?;
            let occurrence_count = nonnegative_i64_to_u64(row.get(8)?, 8)?;
            Ok(ZeroSessionRefResultObject {
                object,
                occurrence_count,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

const RESULT_OBJECT_METADATA_SELECT: &str = "SELECT object_id, effective_bytes,
    content_manifest_version, provenance, lifecycle, storage_kind, availability, created_at
    FROM result_objects WHERE object_id = ?1";
const OCCURRENCE_SELECT: &str = "SELECT result_id, object_id, source_result_id,
    view_start, view_end, view_direction, run_id, turn_id, attempt, retry_no, group_id,
    call_id, tool_name, effective_bytes, tool_dispatch_attempt_id, execution_key, execution_phase,
    execution_status, tool_hook_attempt_id, tool_hook_state, capture_status, delivery_role,
    model_readable, readback_policy, created_at
    FROM tool_result_occurrences WHERE result_id = ?1";
const SESSION_RESULT_REF_SELECT: &str = "SELECT ref_id, session_id, result_id, message_id,
    provider_block_key, source_message_id, source_plan_id, projection_item_key,
    created_from, created_at FROM session_result_refs";
const AUTHORIZATION_SELECT: &str = "SELECT refs.ref_id, occurrence.result_id,
    occurrence.object_id, occurrence.source_result_id, occurrence.view_start,
    occurrence.view_end, occurrence.view_direction, occurrence.model_readable,
    occurrence.readback_policy, object.effective_bytes, object.content_manifest_version,
    object.provenance, object.lifecycle, object.storage_kind, object.availability,
    object.created_at
    FROM session_result_refs refs
    JOIN tool_result_occurrences occurrence ON occurrence.result_id = refs.result_id
    LEFT JOIN result_objects object ON object.object_id = occurrence.object_id
    WHERE refs.session_id = ?1 AND refs.result_id = ?2
    ORDER BY refs.created_at, refs.ref_id
    LIMIT 1";
const ZERO_SESSION_REF_OBJECTS_SELECT: &str = "SELECT object.object_id,
    object.effective_bytes, object.content_manifest_version, object.provenance,
    object.lifecycle, object.storage_kind, object.availability, object.created_at,
    COUNT(occurrence.result_id)
    FROM result_objects object
    LEFT JOIN tool_result_occurrences occurrence ON occurrence.object_id = object.object_id
    WHERE NOT EXISTS (
        SELECT 1
          FROM tool_result_occurrences linked_occurrence
          JOIN session_result_refs refs ON refs.result_id = linked_occurrence.result_id
         WHERE linked_occurrence.object_id = object.object_id
    )
    GROUP BY object.object_id
    ORDER BY object.created_at, object.object_id
    LIMIT ?1";

#[derive(Debug)]
struct AuthorizationCandidate {
    ref_id: String,
    result_id: String,
    source_result_id: Option<String>,
    view_descriptor: Option<ResultViewDescriptor>,
    model_readable: bool,
    readback_policy: ResultReadbackPolicy,
    object: Option<ResultObjectMetadata>,
}

fn authorize_candidate_for_self_read(
    candidate: &AuthorizationCandidate,
) -> std::result::Result<&ResultObjectMetadata, ModelResultReadDenial> {
    if !candidate.model_readable {
        return Err(ModelResultReadDenial::ModelUnreadable);
    }
    match candidate.readback_policy {
        ResultReadbackPolicy::None => return Err(ModelResultReadDenial::ReadbackDisabled),
        ResultReadbackPolicy::SourceOnly => return Err(ModelResultReadDenial::SourceOnly),
        ResultReadbackPolicy::SelfReadable => {}
    }
    let object = candidate
        .object
        .as_ref()
        .ok_or(ModelResultReadDenial::MissingObject)?;
    if object.provenance != ResultProvenance::EffectiveVerified {
        return Err(ModelResultReadDenial::UnverifiedProvenance);
    }
    if object.lifecycle != ResultObjectLifecycle::Ready {
        return Err(ModelResultReadDenial::ObjectNotReady);
    }
    if object.availability != PersistentResultAvailability::Stored {
        return Err(ModelResultReadDenial::ObjectUnavailable);
    }
    Ok(object)
}

fn insert_result_object(
    tx: &rusqlite::Transaction<'_>,
    object: &NewResultObjectMetadata,
) -> Result<()> {
    tx.execute(
        "INSERT INTO result_objects (
            object_id, digest, effective_bytes, content_manifest_version, provenance,
            lifecycle, storage_kind, availability, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            object.object_id,
            object.digest,
            checked_i64("effective_bytes", object.effective_bytes)?,
            i64::from(object.content_manifest_version),
            object.provenance.as_str(),
            object.lifecycle.as_str(),
            object.storage_kind.as_str(),
            object.availability.as_str(),
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn insert_result_occurrence(
    tx: &rusqlite::Transaction<'_>,
    occurrence: &NewToolResultOccurrence,
    created_at: &str,
) -> Result<()> {
    let (view_start, view_end, view_direction) = match occurrence.view_descriptor.as_ref() {
        Some(view) => (
            Some(checked_i64("view start", view.start)?),
            view.end
                .map(|end| checked_i64("view end", end))
                .transpose()?,
            Some(view.direction.as_str()),
        ),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO tool_result_occurrences (
            result_id, object_id, source_result_id, view_start, view_end, view_direction,
            run_id, turn_id, attempt, retry_no, group_id, call_id, tool_name, effective_bytes,
            tool_dispatch_attempt_id, execution_key, execution_phase, execution_status,
            tool_hook_attempt_id, tool_hook_state, capture_status, delivery_role,
            model_readable, readback_policy, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
         )",
        params![
            occurrence.result_id,
            occurrence.object_id,
            occurrence.source_result_id,
            view_start,
            view_end,
            view_direction,
            occurrence.run_id,
            occurrence.turn_id,
            i64::from(occurrence.attempt),
            i64::from(occurrence.retry_no),
            occurrence.group_id,
            occurrence.call_id,
            occurrence.tool_name,
            checked_i64("effective_bytes", occurrence.effective_bytes)?,
            occurrence.tool_dispatch_attempt_id,
            occurrence.execution_key,
            occurrence.execution_phase.as_str(),
            occurrence.execution_status,
            occurrence.tool_hook_attempt_id,
            occurrence.tool_hook_state.as_str(),
            occurrence.capture_status.as_str(),
            occurrence.delivery_role.as_str(),
            i64::from(occurrence.model_readable),
            occurrence.readback_policy.as_str(),
            created_at,
        ],
    )?;
    Ok(())
}

/// Copy only references proven to belong to the inherited stable history.
/// Result objects/occurrences remain immutable; the side gets its own refs.
pub(super) fn copy_side_chat_result_refs(
    tx: &rusqlite::Transaction<'_>,
    source_session_id: &str,
    target_session_id: &str,
    message_ids: &std::collections::BTreeMap<i64, i64>,
    inherited_context: Option<&str>,
    created_at: &str,
) -> Result<()> {
    let mut stmt = tx.prepare(&format!(
        "{SESSION_RESULT_REF_SELECT} WHERE session_id = ?1
         AND NOT EXISTS (
             SELECT 1 FROM tool_result_occurrences occurrence
             JOIN result_objects object ON object.object_id = occurrence.object_id
             WHERE occurrence.result_id = session_result_refs.result_id
               AND object.lifecycle <> 'ready'
         )"
    ))?;
    let references = stmt
        .query_map(params![source_session_id], row_to_session_result_ref)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for reference in references {
        if reference
            .message_id
            .is_some_and(|id| !message_ids.contains_key(&id))
            || reference
                .source_message_id
                .is_some_and(|id| !message_ids.contains_key(&id))
        {
            continue;
        }
        if reference.message_id.is_none()
            && reference.source_message_id.is_none()
            && !inherited_context.is_some_and(|context| {
                context
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
                    .any(|token| token == reference.result_id)
            })
        {
            continue;
        }
        insert_session_result_ref(
            tx,
            target_session_id,
            &NewSessionResultRef {
                ref_id: format!("rr_{}", uuid::Uuid::new_v4().simple()),
                result_id: reference.result_id,
                message_id: reference
                    .message_id
                    .and_then(|id| message_ids.get(&id).copied()),
                provider_block_key: reference.provider_block_key,
                source_message_id: reference
                    .source_message_id
                    .and_then(|id| message_ids.get(&id).copied()),
                // A parent request plan is not owned by the side session.
                source_plan_id: None,
                projection_item_key: None,
                created_from: ResultRefCreatedFrom::Fork,
            },
            created_at,
        )?;
    }
    Ok(())
}

fn insert_session_result_ref(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    reference: &NewSessionResultRef,
    created_at: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO session_result_refs (
            ref_id, session_id, result_id, message_id, provider_block_key,
            source_message_id, source_plan_id, projection_item_key, created_from, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            reference.ref_id,
            session_id,
            reference.result_id,
            reference.message_id,
            reference.provider_block_key,
            reference.source_message_id,
            reference.source_plan_id,
            reference.projection_item_key,
            reference.created_from.as_str(),
            created_at,
        ],
    )?;
    Ok(())
}

fn record_result_foundation_in_tx(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    object: Option<&NewResultObjectMetadata>,
    occurrence: &NewToolResultOccurrence,
    reference: &NewSessionResultRef,
    encrypted_payload: Option<(&[u8; RESULT_STORE_NONCE_BYTES], &[u8])>,
) -> Result<()> {
    require_persistent_result_session(tx, session_id)?;
    validate_occurrence(occurrence)?;
    validate_reference(reference)?;

    if reference.result_id != occurrence.result_id {
        bail!("result ref id must match the recorded occurrence");
    }
    match (object, occurrence.object_id.as_deref()) {
        (Some(object), Some(occurrence_object_id)) if object.object_id == occurrence_object_id => {
            validate_object(object)?;
            insert_result_object(tx, object)?;
        }
        (Some(_), Some(_)) => bail!("result object id must match the recorded occurrence"),
        (Some(_), None) => {
            bail!("cannot record an unreferenced result object in a session transaction")
        }
        (None, _) => {}
    }

    if let Some((nonce, ciphertext)) = encrypted_payload {
        let object = object.context("encrypted payload requires new result object metadata")?;
        if occurrence.object_id.as_deref() != Some(object.object_id.as_str()) {
            bail!("encrypted payload object id must match occurrence object id");
        }
        tx.execute(
            "INSERT INTO result_object_payloads (
                object_id, cipher_version, nonce, ciphertext, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                object.object_id,
                i64::from(RESULT_PAYLOAD_CIPHER_VERSION),
                nonce.as_slice(),
                ciphertext,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
    } else if object.is_some()
        && object.map(|value| value.availability) == Some(PersistentResultAvailability::Stored)
        && occurrence.capture_status == ResultCaptureStatus::PayloadStored
    {
        // Metadata-only tests and legacy import callers may still create
        // object metadata, but the production effective writer must never
        // claim a stored body without an encrypted payload row.
        crate::app_warn!(
            "context",
            "result_store_metadata_only_object",
            "Stored result metadata recorded without an encrypted payload body"
        );
    }

    if let Some(object_id) = occurrence.object_id.as_deref() {
        let (lifecycle, provenance) = load_object_write_guard(tx, object_id)?;
        if lifecycle != ResultObjectLifecycle::Ready {
            bail!("session refs require a ready result object: {object_id}");
        }
        if occurrence.model_readable && provenance != ResultProvenance::EffectiveVerified {
            bail!("model-readable results require effective_verified provenance");
        }
    } else if occurrence.readback_policy == ResultReadbackPolicy::SelfReadable {
        bail!("result without a stored object cannot be self-readable");
    }

    let now = chrono::Utc::now().to_rfc3339();
    insert_result_occurrence(tx, occurrence, &now)?;
    insert_session_result_ref(tx, session_id, reference, &now)?;
    Ok(())
}

fn require_persistent_result_session(conn: &rusqlite::Connection, session_id: &str) -> Result<()> {
    validate_bounded_text("session_id", session_id, MAX_OPAQUE_ID_BYTES)?;
    let incognito = conn
        .query_row(
            "SELECT incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(incognito) = incognito else {
        bail!("session not found: {session_id}");
    };
    if incognito != 0 {
        bail!("incognito sessions cannot write persistent result metadata");
    }
    Ok(())
}

fn persistent_result_session_denial(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<Option<ModelResultReadDenial>> {
    validate_bounded_text("session_id", session_id, MAX_OPAQUE_ID_BYTES)?;
    let incognito = conn
        .query_row(
            "SELECT incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(match incognito {
        None => Some(ModelResultReadDenial::SessionNotFound),
        Some(value) if value != 0 => Some(ModelResultReadDenial::IncognitoSession),
        Some(_) => None,
    })
}

fn result_payload_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn result_payload_aad(object_id: &str, digest: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(object_id.len() + digest.len() + 64);
    aad.extend_from_slice(b"hope-agent:result-store:v1\0");
    aad.extend_from_slice(&(object_id.len() as u64).to_le_bytes());
    aad.extend_from_slice(object_id.as_bytes());
    aad.extend_from_slice(&(digest.len() as u64).to_le_bytes());
    aad.extend_from_slice(digest.as_bytes());
    aad
}

fn encrypt_result_payload(
    object_id: &str,
    digest: &str,
    text: &str,
    key: &[u8; RESULT_STORE_KEY_BYTES],
) -> Result<([u8; RESULT_STORE_NONCE_BYTES], Vec<u8>)> {
    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| anyhow!("initialize result payload cipher"))?;
    let key = LessSafeKey::new(unbound);
    let mut nonce = [0_u8; RESULT_STORE_NONCE_BYTES];
    rand::rng().fill_bytes(&mut nonce);
    let mut ciphertext = text.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(result_payload_aad(object_id, digest)),
        &mut ciphertext,
    )
    .map_err(|_| anyhow!("encrypt result payload"))?;
    Ok((nonce, ciphertext))
}

fn decrypt_result_payload(
    object_id: &str,
    digest: &str,
    nonce: &[u8],
    ciphertext: &[u8],
    key: &[u8; RESULT_STORE_KEY_BYTES],
) -> Result<String> {
    let nonce: [u8; RESULT_STORE_NONCE_BYTES] = nonce
        .try_into()
        .map_err(|_| anyhow!("invalid result payload nonce"))?;
    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| anyhow!("initialize result payload cipher"))?;
    let key = LessSafeKey::new(unbound);
    let mut plaintext = ciphertext.to_vec();
    let opened = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(result_payload_aad(object_id, digest)),
            &mut plaintext,
        )
        .map_err(|_| anyhow!("authenticate result payload"))?;
    if result_payload_digest(opened) != digest {
        bail!("result payload digest mismatch");
    }
    std::str::from_utf8(opened)
        .context("stored result payload is not UTF-8")
        .map(str::to_owned)
}

fn load_or_create_result_store_key() -> Result<[u8; RESULT_STORE_KEY_BYTES]> {
    if let Some(key) = RESULT_STORE_KEY.get() {
        return Ok(*key);
    }
    let directory = crate::paths::credentials_dir()?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("create credentials directory {}", directory.display()))?;
    let path = directory.join(RESULT_STORE_KEY_FILE);
    let lock_path = directory.join(RESULT_STORE_KEY_LOCK_FILE);
    let deadline = Instant::now() + RESULT_STORE_KEY_PUBLICATION_TIMEOUT;
    let mut delay = Duration::from_millis(5);
    loop {
        if let Some(key) = read_result_store_key_at(&path)? {
            let _ = RESULT_STORE_KEY.set(key);
            return Ok(key);
        }
        match crate::platform::try_acquire_exclusive_lock(&lock_path)
            .with_context(|| format!("lock result store key {}", lock_path.display()))?
        {
            Some(_guard) => {
                if let Some(key) = read_result_store_key_at(&path)? {
                    let _ = RESULT_STORE_KEY.set(key);
                    return Ok(key);
                }
                let key: [u8; RESULT_STORE_KEY_BYTES] = rand::random();
                crate::platform::write_secure_file(&path, &key)
                    .with_context(|| format!("write result store key {}", path.display()))?;
                let published = read_result_store_key_at(&path)?
                    .context("result store key write was not readable")?;
                let _ = RESULT_STORE_KEY.set(published);
                return Ok(published);
            }
            None => {
                let now = Instant::now();
                if now >= deadline {
                    if let Some(key) = read_result_store_key_at(&path)? {
                        let _ = RESULT_STORE_KEY.set(key);
                        return Ok(key);
                    }
                    bail!("timed out waiting for result store key publication");
                }
                std::thread::sleep(delay.min(deadline.saturating_duration_since(now)));
                delay = delay.saturating_mul(2).min(Duration::from_millis(50));
            }
        }
    }
}

fn read_result_store_key() -> Result<Option<[u8; RESULT_STORE_KEY_BYTES]>> {
    if let Some(key) = RESULT_STORE_KEY.get() {
        return Ok(Some(*key));
    }
    let path = crate::paths::credentials_dir()?.join(RESULT_STORE_KEY_FILE);
    let key = read_result_store_key_at(&path)?;
    if let Some(key) = key {
        let _ = RESULT_STORE_KEY.set(key);
    }
    Ok(key)
}

fn read_result_store_key_at(path: &Path) -> Result<Option<[u8; RESULT_STORE_KEY_BYTES]>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("result store key path is not a regular file");
    }
    let bytes = std::fs::read(path)?;
    let key = bytes
        .try_into()
        .map_err(|_| anyhow!("result store key has an invalid length"))?;
    Ok(Some(key))
}

fn encode_result_cursor(
    result_id: &str,
    offset: u64,
    direction: ResultTextReadDirection,
) -> Result<String> {
    let cursor = ResultReadCursor {
        version: 1,
        result_id: result_id.to_string(),
        offset,
        direction: direction.as_str().to_string(),
    };
    let json = serde_json::to_vec(&cursor)?;
    Ok(format!(
        "rsc1.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    ))
}

fn decode_result_cursor(
    cursor: &str,
    result_id: &str,
    direction: ResultTextReadDirection,
) -> Result<u64> {
    if cursor.len() > 2_048 {
        bail!("result cursor is too large");
    }
    let encoded = cursor
        .strip_prefix("rsc1.")
        .context("unsupported result cursor version")?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .context("invalid result cursor encoding")?;
    let decoded: ResultReadCursor =
        serde_json::from_slice(&raw).context("invalid result cursor payload")?;
    if decoded.version != 1
        || decoded.result_id != result_id
        || decoded.direction != direction.as_str()
    {
        bail!("result cursor does not match this read");
    }
    Ok(decoded.offset)
}

fn previous_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn next_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn load_object_write_guard(
    tx: &rusqlite::Transaction<'_>,
    object_id: &str,
) -> Result<(ResultObjectLifecycle, ResultProvenance)> {
    let raw = tx
        .query_row(
            "SELECT lifecycle, provenance FROM result_objects WHERE object_id = ?1",
            params![object_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow!("result object not found: {object_id}"))?;
    Ok((
        parse_text_value::<ResultObjectLifecycle>(&raw.0, "result lifecycle")?,
        parse_text_value::<ResultProvenance>(&raw.1, "result provenance")?,
    ))
}

trait ParseDbEnum: Sized {
    fn parse_db(value: &str, column: usize) -> rusqlite::Result<Self>;
}

macro_rules! impl_parse_db_enum {
    ($($type:ty),+ $(,)?) => {
        $(
            impl ParseDbEnum for $type {
                fn parse_db(value: &str, column: usize) -> rusqlite::Result<Self> {
                    Self::from_db(value, column)
                }
            }
        )+
    };
}

impl_parse_db_enum!(
    ResultProvenance,
    ResultObjectLifecycle,
    ResultStorageKind,
    PersistentResultAvailability,
    ToolResultExecutionPhase,
    ToolResultHookState,
    ResultCaptureStatus,
    ResultDeliveryRole,
    ResultReadbackPolicy,
    ResultRefCreatedFrom,
    ResultViewDirection,
);

fn parse_text_value<T: ParseDbEnum>(value: &str, label: &str) -> Result<T> {
    T::parse_db(value, 0)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("invalid {label} value in persistent result metadata: {value}"))
}

fn validate_object(object: &NewResultObjectMetadata) -> Result<()> {
    validate_bounded_text("object_id", &object.object_id, MAX_OPAQUE_ID_BYTES)?;
    validate_bounded_text("digest", &object.digest, MAX_DIGEST_BYTES)?;
    if object.content_manifest_version == 0 {
        bail!("content_manifest_version must be greater than zero");
    }
    checked_i64("effective_bytes", object.effective_bytes)?;
    Ok(())
}

fn validate_occurrence(occurrence: &NewToolResultOccurrence) -> Result<()> {
    for (label, value) in [
        ("result_id", occurrence.result_id.as_str()),
        ("run_id", occurrence.run_id.as_str()),
        ("turn_id", occurrence.turn_id.as_str()),
        ("group_id", occurrence.group_id.as_str()),
        ("call_id", occurrence.call_id.as_str()),
        (
            "tool_dispatch_attempt_id",
            occurrence.tool_dispatch_attempt_id.as_str(),
        ),
        ("execution_key", occurrence.execution_key.as_str()),
    ] {
        validate_bounded_text(label, value, MAX_OPAQUE_ID_BYTES)?;
    }
    validate_bounded_text("tool_name", &occurrence.tool_name, MAX_TOOL_NAME_BYTES)?;
    checked_i64("effective_bytes", occurrence.effective_bytes)?;
    if let Some(value) = occurrence.object_id.as_deref() {
        validate_bounded_text("object_id", value, MAX_OPAQUE_ID_BYTES)?;
    }
    if let Some(value) = occurrence.source_result_id.as_deref() {
        validate_bounded_text("source_result_id", value, MAX_OPAQUE_ID_BYTES)?;
        if value == occurrence.result_id {
            bail!("source_result_id cannot refer to the same occurrence");
        }
    }
    if let Some(status) = occurrence.execution_status.as_deref() {
        validate_bounded_text("execution_status", status, MAX_EXECUTION_STATUS_BYTES)?;
    }
    match occurrence.execution_phase {
        ToolResultExecutionPhase::OutcomeKnown if occurrence.execution_status.is_none() => {
            bail!("outcome_known result requires execution_status");
        }
        ToolResultExecutionPhase::Started | ToolResultExecutionPhase::OutcomeUnknown
            if occurrence.execution_status.is_some() =>
        {
            bail!("execution_status must be empty before the outcome is known");
        }
        _ => {}
    }
    if matches!(
        occurrence.tool_hook_state,
        ToolResultHookState::Started
            | ToolResultHookState::Completed
            | ToolResultHookState::OutcomeUnknown
    ) && occurrence.tool_hook_attempt_id.is_none()
    {
        bail!("started or terminal hook state requires tool_hook_attempt_id");
    }
    if let Some(value) = occurrence.tool_hook_attempt_id.as_deref() {
        validate_bounded_text("tool_hook_attempt_id", value, MAX_OPAQUE_ID_BYTES)?;
    }
    if occurrence.readback_policy == ResultReadbackPolicy::SourceOnly
        && occurrence.source_result_id.is_none()
    {
        bail!("source_only readback requires source_result_id");
    }
    if occurrence.delivery_role == ResultDeliveryRole::ReadView
        && (occurrence.source_result_id.is_none()
            || occurrence.readback_policy != ResultReadbackPolicy::SourceOnly
            || occurrence.view_descriptor.is_none())
    {
        bail!("read_view results require source_only policy and a view descriptor");
    }
    if let Some(view) = occurrence.view_descriptor.as_ref() {
        checked_i64("view start", view.start)?;
        if let Some(end) = view.end {
            checked_i64("view end", end)?;
            if end < view.start {
                bail!("view end must not precede view start");
            }
        }
    }
    Ok(())
}

fn validate_reference(reference: &NewSessionResultRef) -> Result<()> {
    validate_bounded_text("ref_id", &reference.ref_id, MAX_OPAQUE_ID_BYTES)?;
    validate_bounded_text("result_id", &reference.result_id, MAX_OPAQUE_ID_BYTES)?;
    for (label, value) in [
        (
            "provider_block_key",
            reference.provider_block_key.as_deref(),
        ),
        ("source_plan_id", reference.source_plan_id.as_deref()),
        (
            "projection_item_key",
            reference.projection_item_key.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_bounded_text(label, value, MAX_OPAQUE_ID_BYTES)?;
        }
    }
    if reference.provider_block_key.is_some() && reference.message_id.is_none() {
        bail!("provider_block_key requires message_id");
    }
    if reference.projection_item_key.is_some() && reference.source_plan_id.is_none() {
        bail!("projection_item_key requires source_plan_id");
    }
    Ok(())
}

fn validate_bounded_text(label: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} cannot be empty");
    }
    if value.len() > max_bytes {
        bail!("{label} exceeds {max_bytes} bytes");
    }
    Ok(())
}

fn checked_i64(label: &str, value: u64) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds SQLite integer range"))
}

fn row_to_result_object_metadata(row: &Row<'_>) -> rusqlite::Result<ResultObjectMetadata> {
    Ok(ResultObjectMetadata {
        object_id: row.get(0)?,
        effective_bytes: nonnegative_i64_to_u64(row.get(1)?, 1)?,
        content_manifest_version: positive_i64_to_u32(row.get(2)?, 2)?,
        provenance: ResultProvenance::from_db(&row.get::<_, String>(3)?, 3)?,
        lifecycle: ResultObjectLifecycle::from_db(&row.get::<_, String>(4)?, 4)?,
        storage_kind: ResultStorageKind::from_db(&row.get::<_, String>(5)?, 5)?,
        availability: PersistentResultAvailability::from_db(&row.get::<_, String>(6)?, 6)?,
        created_at: row.get(7)?,
    })
}

fn row_to_occurrence(row: &Row<'_>) -> rusqlite::Result<ToolResultOccurrence> {
    let view_descriptor = view_descriptor_from_columns(row, 3, 4, 5)?;
    Ok(ToolResultOccurrence {
        result_id: row.get(0)?,
        object_id: row.get(1)?,
        source_result_id: row.get(2)?,
        view_descriptor,
        run_id: row.get(6)?,
        turn_id: row.get(7)?,
        attempt: nonnegative_i64_to_u32(row.get(8)?, 8)?,
        retry_no: nonnegative_i64_to_u32(row.get(9)?, 9)?,
        group_id: row.get(10)?,
        call_id: row.get(11)?,
        tool_name: row.get(12)?,
        effective_bytes: nonnegative_i64_to_u64(row.get(13)?, 13)?,
        tool_dispatch_attempt_id: row.get(14)?,
        execution_key: row.get(15)?,
        execution_phase: ToolResultExecutionPhase::from_db(&row.get::<_, String>(16)?, 16)?,
        execution_status: row.get(17)?,
        tool_hook_attempt_id: row.get(18)?,
        tool_hook_state: ToolResultHookState::from_db(&row.get::<_, String>(19)?, 19)?,
        capture_status: ResultCaptureStatus::from_db(&row.get::<_, String>(20)?, 20)?,
        delivery_role: ResultDeliveryRole::from_db(&row.get::<_, String>(21)?, 21)?,
        model_readable: strict_bool(row.get(22)?, 22)?,
        readback_policy: ResultReadbackPolicy::from_db(&row.get::<_, String>(23)?, 23)?,
        created_at: row.get(24)?,
    })
}

fn row_to_session_result_ref(row: &Row<'_>) -> rusqlite::Result<SessionResultRef> {
    Ok(SessionResultRef {
        ref_id: row.get(0)?,
        session_id: row.get(1)?,
        result_id: row.get(2)?,
        message_id: row.get(3)?,
        provider_block_key: row.get(4)?,
        source_message_id: row.get(5)?,
        source_plan_id: row.get(6)?,
        projection_item_key: row.get(7)?,
        created_from: ResultRefCreatedFrom::from_db(&row.get::<_, String>(8)?, 8)?,
        created_at: row.get(9)?,
    })
}

fn row_to_authorization_candidate(row: &Row<'_>) -> rusqlite::Result<AuthorizationCandidate> {
    let object_id: Option<String> = row.get(2)?;
    let view_descriptor = view_descriptor_from_columns(row, 4, 5, 6)?;
    let object = match object_id {
        Some(object_id) => {
            let effective_bytes: Option<i64> = row.get(9)?;
            let manifest_version: Option<i64> = row.get(10)?;
            let provenance: Option<String> = row.get(11)?;
            let lifecycle: Option<String> = row.get(12)?;
            let storage_kind: Option<String> = row.get(13)?;
            let availability: Option<String> = row.get(14)?;
            let created_at: Option<String> = row.get(15)?;
            match (
                effective_bytes,
                manifest_version,
                provenance,
                lifecycle,
                storage_kind,
                availability,
                created_at,
            ) {
                (
                    Some(effective_bytes),
                    Some(manifest_version),
                    Some(provenance),
                    Some(lifecycle),
                    Some(storage_kind),
                    Some(availability),
                    Some(created_at),
                ) => Some(ResultObjectMetadata {
                    object_id,
                    effective_bytes: nonnegative_i64_to_u64(effective_bytes, 9)?,
                    content_manifest_version: positive_i64_to_u32(manifest_version, 10)?,
                    provenance: ResultProvenance::from_db(&provenance, 11)?,
                    lifecycle: ResultObjectLifecycle::from_db(&lifecycle, 12)?,
                    storage_kind: ResultStorageKind::from_db(&storage_kind, 13)?,
                    availability: PersistentResultAvailability::from_db(&availability, 14)?,
                    created_at,
                }),
                _ => None,
            }
        }
        None => None,
    };
    Ok(AuthorizationCandidate {
        ref_id: row.get(0)?,
        result_id: row.get(1)?,
        source_result_id: row.get(3)?,
        view_descriptor,
        model_readable: strict_bool(row.get(7)?, 7)?,
        readback_policy: ResultReadbackPolicy::from_db(&row.get::<_, String>(8)?, 8)?,
        object,
    })
}

fn view_descriptor_from_columns(
    row: &Row<'_>,
    start_column: usize,
    end_column: usize,
    direction_column: usize,
) -> rusqlite::Result<Option<ResultViewDescriptor>> {
    let start: Option<i64> = row.get(start_column)?;
    let end: Option<i64> = row.get(end_column)?;
    let direction: Option<String> = row.get(direction_column)?;
    match (start, direction) {
        (None, None) if end.is_none() => Ok(None),
        (Some(start), Some(direction)) => Ok(Some(ResultViewDescriptor {
            start: nonnegative_i64_to_u64(start, start_column)?,
            end: end
                .map(|value| nonnegative_i64_to_u64(value, end_column))
                .transpose()?,
            direction: ResultViewDirection::from_db(&direction, direction_column)?,
        })),
        _ => Err(invalid_text_value(
            direction_column,
            "ResultViewDescriptor",
            "incomplete view descriptor",
        )),
    }
}

fn strict_bool(value: i64, column: usize) -> rusqlite::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_integer_value(column, "boolean", value)),
    }
}

fn nonnegative_i64_to_u32(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| invalid_integer_value(column, "nonnegative u32", value))
}

fn positive_i64_to_u32(value: i64, column: usize) -> rusqlite::Result<u32> {
    let value = nonnegative_i64_to_u32(value, column)?;
    if value == 0 {
        return Err(invalid_integer_value(column, "positive u32", 0));
    }
    Ok(value)
}

fn nonnegative_i64_to_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| invalid_integer_value(column, "nonnegative u64", value))
}

fn invalid_text_value(column: usize, expected: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected {expected}, found {value}"),
        )),
    )
}

fn invalid_integer_value(column: usize, expected: &str, value: i64) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Integer,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected {expected}, found {value}"),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_channel_conversations_table(db: &SessionDB) {
        // Mirror the production 1:1 schema in `ChannelDB::migrate` so session
        // metadata reads and deletion exercise the same cross-feature shape.
        db.with_conn_for_test(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_tenant_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );
                CREATE UNIQUE INDEX IF NOT EXISTS uq_channel_conv_chat
                    ON channel_conversations(
                        channel_id, account_id, chat_id, COALESCE(thread_id, '')
                    );
                CREATE UNIQUE INDEX IF NOT EXISTS uq_channel_conv_session
                    ON channel_conversations(session_id);
                CREATE INDEX IF NOT EXISTS idx_channel_conv_lookup
                    ON channel_conversations(channel_id, account_id, chat_id);",
            )?;
            Ok(())
        })
        .expect("create channel conversations fixture");
    }

    fn test_db(name: &str) -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join(name)).expect("session db");
        (dir, db)
    }

    fn ready_object(suffix: &str) -> NewResultObjectMetadata {
        NewResultObjectMetadata {
            object_id: format!("object-{suffix}"),
            digest: format!("sha256:{suffix}"),
            effective_bytes: 128,
            content_manifest_version: 1,
            provenance: ResultProvenance::EffectiveVerified,
            lifecycle: ResultObjectLifecycle::Ready,
            storage_kind: ResultStorageKind::InlineDb,
            availability: PersistentResultAvailability::Stored,
        }
    }

    fn occurrence(suffix: &str) -> NewToolResultOccurrence {
        NewToolResultOccurrence {
            result_id: format!("result-{suffix}"),
            object_id: Some(format!("object-{suffix}")),
            source_result_id: None,
            view_descriptor: None,
            run_id: format!("run-{suffix}"),
            turn_id: format!("turn-{suffix}"),
            attempt: 0,
            retry_no: 0,
            group_id: format!("group-{suffix}"),
            call_id: format!("call-{suffix}"),
            tool_name: "read".to_string(),
            effective_bytes: 128,
            tool_dispatch_attempt_id: format!("dispatch-{suffix}"),
            execution_key: format!("execution-{suffix}"),
            execution_phase: ToolResultExecutionPhase::OutcomeKnown,
            execution_status: Some("succeeded".to_string()),
            tool_hook_attempt_id: None,
            tool_hook_state: ToolResultHookState::NotConfigured,
            capture_status: ResultCaptureStatus::PayloadStored,
            delivery_role: ResultDeliveryRole::ProviderToolResult,
            model_readable: true,
            readback_policy: ResultReadbackPolicy::SelfReadable,
        }
    }

    fn result_ref(suffix: &str) -> NewSessionResultRef {
        NewSessionResultRef {
            ref_id: format!("ref-{suffix}"),
            result_id: format!("result-{suffix}"),
            message_id: None,
            provider_block_key: None,
            source_message_id: None,
            source_plan_id: None,
            projection_item_key: None,
            created_from: ResultRefCreatedFrom::Direct,
        }
    }

    #[test]
    fn side_chat_result_refs_follow_only_the_stable_snapshot() {
        let (_dir, db) = test_db("side-result-refs.db");
        ensure_channel_conversations_table(&db);
        let source = db.create_session("ha-main").unwrap();
        let user = db
            .append_message(&source.id, &crate::session::NewMessage::user("question"))
            .unwrap();
        let tool = db
            .append_message(
                &source.id,
                &crate::session::NewMessage::assistant("result-stable"),
            )
            .unwrap();
        let mut stable_ref = result_ref("stable");
        stable_ref.message_id = Some(tool);
        stable_ref.source_message_id = Some(user);
        stable_ref.provider_block_key = Some("block-1".into());
        db.record_result_foundation(
            &source.id,
            Some(&ready_object("stable")),
            &occurrence("stable"),
            &stable_ref,
        )
        .unwrap();
        db.record_result_foundation(
            &source.id,
            Some(&ready_object("context")),
            &occurrence("context"),
            &result_ref("context"),
        )
        .unwrap();
        let stable_context =
            serde_json::json!([{"role":"assistant", "content":"result-stable result-context"}])
                .to_string();
        db.save_context(&source.id, &stable_context).unwrap();
        let active_user = db
            .append_message(&source.id, &crate::session::NewMessage::user("in flight"))
            .unwrap();
        let turn = db
            .create_chat_turn(&source.id, "desktop", None, Some(active_user))
            .unwrap();
        let live_message = db
            .append_message(
                &source.id,
                &crate::session::NewMessage::assistant("result-live"),
            )
            .unwrap();
        let mut live_ref = result_ref("live");
        live_ref.message_id = Some(live_message);
        db.record_result_foundation(
            &source.id,
            Some(&ready_object("live")),
            &occurrence("live"),
            &live_ref,
        )
        .unwrap();
        db.record_result_foundation(
            &source.id,
            Some(&ready_object("unrelated")),
            &occurrence("unrelated"),
            &result_ref("unrelated"),
        )
        .unwrap();
        db.record_result_foundation(
            &source.id,
            Some(&ready_object("con")),
            &occurrence("con"),
            &result_ref("con"),
        )
        .unwrap();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "INSERT INTO chat_stream_runs (run_id, session_id, source, turn_id, status, base_context_json, started_at)
                 VALUES ('side-ref-run', ?1, 'desktop', ?2, 'running', ?3, '2026-01-01T00:00:00Z')",
                params![source.id, turn.id, stable_context],
            )?;
            Ok(())
        }).unwrap();
        let side = db.create_side_chat(&source.id).unwrap();
        let copied = db.load_session_messages(&side.id).unwrap();
        assert_eq!(copied.len(), 2);
        let refs = db
            .list_session_result_refs(&side.id, "result-stable")
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].message_id, Some(copied[1].id));
        assert_eq!(refs[0].source_message_id, Some(copied[0].id));
        assert_eq!(refs[0].provider_block_key.as_deref(), Some("block-1"));
        assert_eq!(refs[0].created_from, ResultRefCreatedFrom::Fork);
        assert_ne!(refs[0].ref_id, stable_ref.ref_id);
        assert_eq!(
            db.list_session_result_refs(&side.id, "result-context")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.authorize_model_result_read(&side.id, "result-stable")
                .unwrap(),
            db.authorize_model_result_read(&source.id, "result-stable")
                .unwrap()
        );
        for excluded in ["result-live", "result-unrelated", "result-con"] {
            assert_eq!(
                db.authorize_model_result_read(&side.id, excluded).unwrap(),
                ModelResultReadAuthorization::Denied(ModelResultReadDenial::MissingReference)
            );
        }
        // Deleting a source message must not cascade the side's remapped refs.
        db.with_conn_for_test(|conn| {
            conn.execute("DELETE FROM messages WHERE id = ?1", [tool])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            db.list_session_result_refs(&side.id, "result-stable")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn schema_has_metadata_only_tables_and_immutable_object_identity() {
        let (_dir, db) = test_db("schema.db");
        let session = db
            .create_session_with_project("ha-main", None, None)
            .expect("session");
        let object = ready_object("schema");
        let recorded_occurrence = occurrence("schema");
        let reference = result_ref("schema");
        db.record_result_foundation(&session.id, Some(&object), &recorded_occurrence, &reference)
            .expect("record foundation");

        db.with_conn_for_test(|conn| {
            for table in [
                "result_objects",
                "tool_result_occurrences",
                "session_result_refs",
            ] {
                let exists: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )?;
                assert_eq!(exists, 1, "missing {table}");
            }

            for table in [
                "result_objects",
                "tool_result_occurrences",
                "session_result_refs",
            ] {
                let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
                let columns = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(!columns.iter().any(|column| {
                    column.contains("payload")
                        || column.contains("path")
                        || column.contains("managed_key")
                        || column.contains("body")
                }));
            }

            let metadata = db
                .get_result_object_metadata(&object.object_id)?
                .expect("object metadata");
            let serialized = serde_json::to_value(metadata)?;
            assert!(serialized.get("digest").is_none());

            let mutation = conn.execute(
                "UPDATE result_objects SET digest = 'replacement' WHERE object_id = ?1",
                params![object.object_id],
            );
            assert!(mutation.is_err(), "immutable metadata update must fail");
            Ok(())
        })
        .expect("schema assertions");
    }

    #[test]
    fn model_read_requires_explicit_ref_and_self_policy() {
        let (_dir, db) = test_db("auth.db");
        let owner = db
            .create_session_with_project("ha-main", None, None)
            .expect("owner session");
        let stranger = db
            .create_session_with_project("ha-main", None, None)
            .expect("stranger session");
        let object = ready_object("source");
        let source_occurrence = occurrence("source");
        let reference = result_ref("source");
        db.record_result_foundation(&owner.id, Some(&object), &source_occurrence, &reference)
            .expect("record source");

        let authorized = db
            .authorize_model_result_read(&owner.id, &source_occurrence.result_id)
            .expect("authorize owner");
        assert_eq!(
            authorized,
            ModelResultReadAuthorization::Denied(ModelResultReadDenial::ObjectUnavailable)
        );
        assert_eq!(
            db.authorize_model_result_read(&stranger.id, &source_occurrence.result_id)
                .expect("authorize stranger"),
            ModelResultReadAuthorization::Denied(ModelResultReadDenial::MissingReference)
        );

        let read_object = ready_object("read-view");
        let mut read_occurrence = occurrence("read-view");
        read_occurrence.source_result_id = Some(source_occurrence.result_id.clone());
        read_occurrence.view_descriptor = Some(ResultViewDescriptor {
            start: 0,
            end: Some(64),
            direction: ResultViewDirection::Forward,
        });
        read_occurrence.delivery_role = ResultDeliveryRole::ReadView;
        read_occurrence.readback_policy = ResultReadbackPolicy::SourceOnly;
        let read_ref = result_ref("read-view");
        db.record_result_foundation(&owner.id, Some(&read_object), &read_occurrence, &read_ref)
            .expect("record read view");
        assert_eq!(
            db.authorize_model_result_read(&owner.id, &read_occurrence.result_id)
                .expect("authorize read view"),
            ModelResultReadAuthorization::Denied(ModelResultReadDenial::SourceOnly)
        );
    }

    #[test]
    fn incognito_result_store_writes_fail_before_any_row_is_persisted() {
        let (_dir, db) = test_db("incognito.db");
        let incognito = db
            .create_session_with_project("ha-main", None, Some(true))
            .expect("incognito session");
        let object = ready_object("incognito");
        let incognito_occurrence = occurrence("incognito");
        let reference = result_ref("incognito");
        let error = db
            .record_result_foundation(
                &incognito.id,
                Some(&object),
                &incognito_occurrence,
                &reference,
            )
            .expect_err("incognito write must fail");
        assert!(error.to_string().contains("incognito"));

        db.with_conn_for_test(|conn| {
            for table in [
                "result_objects",
                "tool_result_occurrences",
                "session_result_refs",
            ] {
                let count: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                assert_eq!(count, 0, "unexpected persistent row in {table}");
            }
            Ok(())
        })
        .expect("row counts");

        let attach_error = db
            .attach_existing_result_ref(&incognito.id, &reference)
            .expect_err("incognito attach must fail before occurrence lookup");
        assert!(attach_error.to_string().contains("incognito"));
        let remove_error = db
            .remove_session_result_ref(&incognito.id, &reference.ref_id)
            .expect_err("incognito revoke must fail before deletion");
        assert!(remove_error.to_string().contains("incognito"));
    }

    #[test]
    fn deleting_session_cascades_refs_and_exposes_only_zero_ref_candidate() {
        let (_dir, db) = test_db("cascade.db");
        ensure_channel_conversations_table(&db);
        let session = db
            .create_session_with_project("ha-main", None, None)
            .expect("session");
        let object = ready_object("cascade");
        let cascade_occurrence = occurrence("cascade");
        let reference = result_ref("cascade");
        db.record_result_foundation(&session.id, Some(&object), &cascade_occurrence, &reference)
            .expect("record foundation");
        assert!(db
            .list_zero_session_ref_result_objects(10)
            .expect("candidates before delete")
            .is_empty());

        db.delete_session(&session.id).expect("delete session");
        assert!(db
            .list_session_result_refs(&session.id, &cascade_occurrence.result_id)
            .expect("refs after delete")
            .is_empty());
        assert!(db
            .get_tool_result_occurrence(&cascade_occurrence.result_id)
            .expect("occurrence after delete")
            .is_some());
        assert!(db
            .get_result_object_metadata(&object.object_id)
            .expect("object after delete")
            .is_some());
        let candidates = db
            .list_zero_session_ref_result_objects(10)
            .expect("candidates after delete");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].object.object_id, object.object_id);
        assert_eq!(candidates[0].occurrence_count, 1);
    }
}
