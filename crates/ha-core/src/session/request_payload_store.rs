//! Encrypted ownership store for the exact, final Provider JSON request body.
//!
//! This module is deliberately not wired to Provider dispatch yet.  Its
//! production capability gate is fixed closed for the same reason as the
//! ResultStore body gate: the current process cannot prove that either the
//! data root or the key is inaccessible to every later model-controlled host
//! subprocess.  A closed gate returns a typed `Unavailable` result before a
//! key, database payload row, reservation, or managed file is created.  There
//! is never a plaintext fallback.

#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use rand::RngCore;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use super::SessionDB;

pub(crate) const EXACT_REQUEST_INLINE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_EXACT_REQUEST_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const DEFAULT_EXACT_REQUEST_STORE_QUOTA_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const DEFAULT_INCOGNITO_EXACT_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

const MAX_ID_BYTES: usize = 512;
const MAX_REASON_BYTES: usize = 2 * 1024;
const PAYLOAD_KEY_BYTES: usize = 32;
const PAYLOAD_NONCE_BYTES: usize = 12;
const PAYLOAD_CIPHER_VERSION: i64 = 1;
const PAYLOAD_KEY_FILE: &str = "request-payload-v1.key";
const PAYLOAD_KEY_LOCK_FILE: &str = "request-payload-v1.lock";
const PAYLOAD_KEY_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);
const STALE_RESERVATION_AGE_SECS: i64 = 5 * 60;
const MANAGED_BLOB_DIRECTORY: &str = "request-payloads-v1";
static REQUEST_PAYLOAD_KEY: OnceLock<[u8; PAYLOAD_KEY_BYTES]> = OnceLock::new();

/// Keep this synchronized with ResultStore's capability decision.  A regular
/// directory below the application root plus an in-process key is not a
/// kernel-private boundary against a later host/YOLO subprocess.
pub(crate) const fn kernel_private_request_payload_storage_available() -> bool {
    super::result_store::kernel_private_storage_available()
}

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

            fn from_db(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => bail!(concat!("invalid ", stringify!($name), ": {}"), value),
                }
            }
        }
    };
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

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadStorageKind {
        InlineDb => "inline_db",
        ManagedBlob => "managed_blob",
        IncognitoMemory => "incognito_memory",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadObjectState {
        Live => "live",
        ScrubPending => "scrub_pending",
        Scrubbed => "scrubbed",
        Lost => "lost",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadQuotaState {
        Reserved => "reserved",
        Committed => "committed",
        Released => "released",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadRetentionState {
        Retained => "retained",
        ReleasePending => "release_pending",
        Released => "released",
    }
}

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ExactPayloadOwnerState {
        Active => "active",
        SendUnknown => "send_unknown",
        CleanupTombstone => "cleanup_tombstone",
        Released => "released",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExactPayloadUnavailableReason {
    KernelPrivateStorageUnavailable,
    IncognitoRequiresMemoryStore,
    QuotaExceeded,
    PayloadTooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExactPayloadUnavailable {
    pub availability: ExactPayloadAvailability,
    pub reason: ExactPayloadUnavailableReason,
    pub requested_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExactPayloadLost {
    pub availability: ExactPayloadAvailability,
    pub payload_id: String,
    pub reservation_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExactPayloadHold {
    pub availability: ExactPayloadAvailability,
    pub payload_id: String,
    pub owner_id: String,
    pub reservation_id: String,
    pub storage_kind: ExactPayloadStorageKind,
    pub plaintext_bytes: u64,
    /// Keyed, domain-separated digest.  It is an identity/integrity value,
    /// never a public content hash and never contains request bytes.
    pub keyed_digest: String,
    pub object_state: ExactPayloadObjectState,
    pub quota_state: ExactPayloadQuotaState,
    pub retention_state: ExactPayloadRetentionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub(crate) enum StageExactPayloadOutcome {
    Stored(ExactPayloadHold),
    Unavailable(ExactPayloadUnavailable),
    Lost(ExactPayloadLost),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExactPayloadReadDenial {
    KernelPrivateStorageUnavailable,
    SessionNotFound,
    IncognitoSession,
    OwnerMismatch,
    OwnerNotReadable,
    PayloadNotFound,
    PayloadNotLive,
    PayloadLost,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExactPayloadRead {
    Authorized(Vec<u8>),
    Denied(ExactPayloadReadDenial),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExactPayloadScrubReason {
    RequestTerminal,
    RequestSuperseded,
    SendUnknownResolved,
    SessionDeleted,
    RetentionExpired,
    ReconcileCorrupt,
}

impl ExactPayloadScrubReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RequestTerminal => "request_terminal",
            Self::RequestSuperseded => "request_superseded",
            Self::SendUnknownResolved => "send_unknown_resolved",
            Self::SessionDeleted => "session_deleted",
            Self::RetentionExpired => "retention_expired",
            Self::ReconcileCorrupt => "reconcile_corrupt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestExactPayloadScrubOutcome {
    Pending,
    AlreadyScrubbed,
    HeldBySendUnknown,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ExactPayloadReconcileReport {
    pub capability_available: bool,
    pub tombstones_claimed: u64,
    pub terminal_payloads_claimed: u64,
    pub orphan_payloads_claimed: u64,
    pub expired_claimed: u64,
    pub payloads_scrubbed: u64,
    pub payloads_marked_lost: u64,
    pub stale_reservations_released: u64,
    pub orphan_blobs_removed: u64,
    pub orphan_metadata_removed: u64,
}

#[derive(Clone)]
struct RequestPayloadCapability {
    root: PathBuf,
    key: [u8; PAYLOAD_KEY_BYTES],
    quota_bytes: u64,
}

#[derive(Debug)]
struct StoredPayloadRow {
    owner_id: String,
    storage_kind: ExactPayloadStorageKind,
    availability: ExactPayloadAvailability,
    state: ExactPayloadObjectState,
    retention_state: ExactPayloadRetentionState,
    plaintext_bytes: u64,
    ciphertext_bytes: u64,
    keyed_digest: String,
    nonce: Option<Vec<u8>>,
    inline_ciphertext: Option<Vec<u8>>,
    managed_blob_name: Option<String>,
}

impl SessionDB {
    pub(crate) fn ensure_request_payload_store_tables(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    /// Stage the exact body that would be sent to a Provider.  Authentication
    /// belongs in headers/transport state and must not be included in `json`.
    /// With the current capability gate this returns `Unavailable` with no
    /// payload-store database or filesystem side effect.
    pub(crate) fn stage_exact_request_payload(
        &self,
        session_id: &str,
        owner_id: &str,
        json: &[u8],
        expires_at: Option<&str>,
    ) -> Result<StageExactPayloadOutcome> {
        validate_id("session_id", session_id)?;
        validate_id("owner_id", owner_id)?;
        let requested_bytes = u64::try_from(json.len()).context("request body exceeds u64")?;
        if json.len() > MAX_EXACT_REQUEST_PAYLOAD_BYTES {
            return Ok(StageExactPayloadOutcome::Unavailable(
                ExactPayloadUnavailable {
                    availability: ExactPayloadAvailability::Unavailable,
                    reason: ExactPayloadUnavailableReason::PayloadTooLarge,
                    requested_bytes,
                },
            ));
        }
        validate_final_provider_json(json)?;

        // Incognito is decided before consulting the capability or key, so it
        // cannot create installation-level state.  The caller must use a
        // turn-owned `IncognitoExactPayloadStore` instead.
        let incognito = {
            let conn = self.read_conn()?;
            conn.query_row(
                "SELECT incognito FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?
                != 0
        };
        if incognito {
            return Ok(StageExactPayloadOutcome::Unavailable(
                ExactPayloadUnavailable {
                    availability: ExactPayloadAvailability::Unavailable,
                    reason: ExactPayloadUnavailableReason::IncognitoRequiresMemoryStore,
                    requested_bytes,
                },
            ));
        }
        let Some(capability) = production_capability()? else {
            return Ok(StageExactPayloadOutcome::Unavailable(
                ExactPayloadUnavailable {
                    availability: ExactPayloadAvailability::Unavailable,
                    reason: ExactPayloadUnavailableReason::KernelPrivateStorageUnavailable,
                    requested_bytes,
                },
            ));
        };
        self.stage_exact_request_payload_with_capability(
            session_id,
            owner_id,
            json,
            expires_at,
            &capability,
        )
    }

    fn stage_exact_request_payload_with_capability(
        &self,
        session_id: &str,
        owner_id: &str,
        json: &[u8],
        expires_at: Option<&str>,
        capability: &RequestPayloadCapability,
    ) -> Result<StageExactPayloadOutcome> {
        validate_id("session_id", session_id)?;
        validate_id("owner_id", owner_id)?;
        if json.len() > MAX_EXACT_REQUEST_PAYLOAD_BYTES {
            return Ok(unavailable(
                ExactPayloadUnavailableReason::PayloadTooLarge,
                json.len(),
            ));
        }
        validate_final_provider_json(json)?;
        let normalized_expires_at = normalize_expires_at(expires_at)?;
        let expires_at = normalized_expires_at.as_deref();

        let payload_id = uuid::Uuid::new_v4().to_string();
        let reservation_id = uuid::Uuid::new_v4().to_string();
        let plaintext_bytes = u64::try_from(json.len()).context("request body exceeds u64")?;
        let storage_kind = if json.len() <= EXACT_REQUEST_INLINE_BYTES {
            ExactPayloadStorageKind::InlineDb
        } else {
            ExactPayloadStorageKind::ManagedBlob
        };
        let keyed_digest = keyed_payload_digest(json, &capability.key);
        let (nonce, ciphertext) =
            encrypt_payload(&payload_id, owner_id, &keyed_digest, json, &capability.key)?;
        let ciphertext_bytes =
            u64::try_from(ciphertext.len()).context("ciphertext length exceeds u64")?;
        let now = chrono::Utc::now().to_rfc3339();

        // Reservation and owner are durable before a managed file is
        // published.  A crash at that boundary is reconciled by reservation
        // id and deterministic blob name; no partial plaintext ever exists.
        {
            let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_persistent_session(&tx, session_id)?;
            insert_or_validate_owner(&tx, owner_id, session_id, &now)?;
            let in_use: i64 = tx.query_row(
                "SELECT COALESCE(SUM(reserved_bytes), 0)
                   FROM request_payload_reservations
                  WHERE quota_state IN ('reserved', 'committed')",
                [],
                |row| row.get(0),
            )?;
            let in_use = u64::try_from(in_use).context("negative request payload quota")?;
            if in_use.saturating_add(plaintext_bytes) > capability.quota_bytes {
                tx.rollback()?;
                return Ok(unavailable(
                    ExactPayloadUnavailableReason::QuotaExceeded,
                    json.len(),
                ));
            }
            tx.execute(
                "INSERT INTO request_payload_reservations (
                    reservation_id, owner_id, payload_id, reserved_bytes,
                    quota_state, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 'reserved', ?5, ?5)",
                params![
                    reservation_id,
                    owner_id,
                    payload_id,
                    u64_to_i64(plaintext_bytes)?,
                    now
                ],
            )?;

            if storage_kind == ExactPayloadStorageKind::InlineDb {
                insert_live_payload(
                    &tx,
                    &payload_id,
                    owner_id,
                    &reservation_id,
                    storage_kind,
                    plaintext_bytes,
                    ciphertext_bytes,
                    &keyed_digest,
                    &nonce,
                    Some(&ciphertext),
                    None,
                    expires_at,
                    &now,
                )?;
                commit_reservation(&tx, &reservation_id, &now)?;
            }
            tx.commit()?;
        }

        if storage_kind == ExactPayloadStorageKind::ManagedBlob {
            let blob_name = managed_blob_name(&payload_id);
            if let Err(error) = write_managed_ciphertext(capability, &blob_name, &ciphertext) {
                self.record_lost_payload(
                    owner_id,
                    &payload_id,
                    &reservation_id,
                    storage_kind,
                    plaintext_bytes,
                    &keyed_digest,
                    expires_at,
                    &format!("managed blob publication failed: {error:#}"),
                )?;
                return Ok(StageExactPayloadOutcome::Lost(ExactPayloadLost {
                    availability: ExactPayloadAvailability::Lost,
                    payload_id,
                    reservation_id: Some(reservation_id),
                    reason: "managed_blob_publication_failed".to_string(),
                }));
            }

            let finalization = (|| -> Result<()> {
                let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let owner_state: String = tx
                    .query_row(
                        "SELECT owner_state FROM request_payload_owners WHERE owner_id = ?1",
                        params![owner_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .context("request payload owner disappeared during stage")?;
                if owner_state != ExactPayloadOwnerState::Active.as_str() {
                    bail!("request payload owner stopped being active during stage")
                }
                insert_live_payload(
                    &tx,
                    &payload_id,
                    owner_id,
                    &reservation_id,
                    storage_kind,
                    plaintext_bytes,
                    ciphertext_bytes,
                    &keyed_digest,
                    &nonce,
                    None,
                    Some(&blob_name),
                    expires_at,
                    &now,
                )?;
                commit_reservation(&tx, &reservation_id, &now)?;
                tx.commit()?;
                Ok(())
            })();
            if let Err(error) = finalization {
                let _ = remove_managed_ciphertext(capability, &blob_name);
                self.record_lost_payload(
                    owner_id,
                    &payload_id,
                    &reservation_id,
                    storage_kind,
                    plaintext_bytes,
                    &keyed_digest,
                    expires_at,
                    &format!("managed blob database finalization failed: {error:#}"),
                )?;
                return Ok(StageExactPayloadOutcome::Lost(ExactPayloadLost {
                    availability: ExactPayloadAvailability::Lost,
                    payload_id,
                    reservation_id: Some(reservation_id),
                    reason: "managed_blob_finalization_failed".to_string(),
                }));
            }
        }

        Ok(StageExactPayloadOutcome::Stored(ExactPayloadHold {
            availability: ExactPayloadAvailability::Stored,
            payload_id,
            owner_id: owner_id.to_string(),
            reservation_id,
            storage_kind,
            plaintext_bytes,
            keyed_digest,
            object_state: ExactPayloadObjectState::Live,
            quota_state: ExactPayloadQuotaState::Committed,
            retention_state: ExactPayloadRetentionState::Retained,
        }))
    }

    fn record_lost_payload(
        &self,
        owner_id: &str,
        payload_id: &str,
        reservation_id: &str,
        storage_kind: ExactPayloadStorageKind,
        plaintext_bytes: u64,
        keyed_digest: &str,
        expires_at: Option<&str>,
        error: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let bounded_error = crate::truncate_utf8(error, MAX_REASON_BYTES);
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO request_payload_objects (
                payload_id, owner_id, reservation_id, storage_kind, availability,
                object_state, quota_state, retention_state, plaintext_bytes,
                ciphertext_bytes, cipher_version, keyed_digest, nonce,
                inline_ciphertext, managed_blob_name, expires_at, last_error,
                created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, 'lost', 'lost', 'released', 'released',
                ?5, 0, 1, ?6, NULL, NULL, NULL, ?7, ?8, ?9, ?9
             )",
            params![
                payload_id,
                owner_id,
                reservation_id,
                storage_kind.as_str(),
                u64_to_i64(plaintext_bytes)?,
                keyed_digest,
                expires_at,
                bounded_error,
                now
            ],
        )?;
        // `COMMIT` can have an ambiguous error at the storage boundary.  If
        // the live row nevertheless became visible, converge it to the same
        // typed-lost state after the managed file has been removed.
        tx.execute(
            "UPDATE request_payload_objects
                SET availability = 'lost', object_state = 'lost',
                    quota_state = 'released', retention_state = 'released',
                    nonce = NULL, inline_ciphertext = NULL, managed_blob_name = NULL,
                    ciphertext_bytes = 0, last_error = ?2, updated_at = ?3
              WHERE payload_id = ?1 AND object_state = 'live'",
            params![payload_id, bounded_error, now],
        )?;
        tx.execute(
            "UPDATE request_payload_reservations
                SET quota_state = 'released', updated_at = ?2
              WHERE reservation_id = ?1",
            params![reservation_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn read_authorized_exact_request_payload(
        &self,
        session_id: &str,
        owner_id: &str,
        payload_id: &str,
    ) -> Result<ExactPayloadRead> {
        if !kernel_private_request_payload_storage_available() {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::KernelPrivateStorageUnavailable,
            ));
        }
        let Some(capability) = production_capability()? else {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::KernelPrivateStorageUnavailable,
            ));
        };
        self.read_authorized_exact_request_payload_with_capability(
            session_id,
            owner_id,
            payload_id,
            &capability,
        )
    }

    fn read_authorized_exact_request_payload_with_capability(
        &self,
        session_id: &str,
        owner_id: &str,
        payload_id: &str,
        capability: &RequestPayloadCapability,
    ) -> Result<ExactPayloadRead> {
        validate_id("session_id", session_id)?;
        validate_id("owner_id", owner_id)?;
        validate_uuid_id("payload_id", payload_id)?;
        let conn = self.read_conn()?;
        let session_incognito = conn
            .query_row(
                "SELECT incognito FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(session_incognito) = session_incognito else {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::SessionNotFound,
            ));
        };
        if session_incognito != 0 {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::IncognitoSession,
            ));
        }
        let owner = conn
            .query_row(
                "SELECT session_id, owner_state FROM request_payload_owners
                  WHERE owner_id = ?1",
                params![owner_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((owner_session, owner_state)) = owner else {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::OwnerMismatch,
            ));
        };
        if owner_session != session_id {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::OwnerMismatch,
            ));
        }
        let owner_state = ExactPayloadOwnerState::from_db(&owner_state)?;
        if !matches!(
            owner_state,
            ExactPayloadOwnerState::Active | ExactPayloadOwnerState::SendUnknown
        ) {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::OwnerNotReadable,
            ));
        }
        let row = load_payload_row(&conn, payload_id)?;
        drop(conn);
        let Some(row) = row else {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::PayloadNotFound,
            ));
        };
        if row.owner_id != owner_id {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::OwnerMismatch,
            ));
        }
        if row.availability == ExactPayloadAvailability::Lost
            || row.state == ExactPayloadObjectState::Lost
        {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::PayloadLost,
            ));
        }
        if row.state != ExactPayloadObjectState::Live
            || row.retention_state != ExactPayloadRetentionState::Retained
        {
            return Ok(ExactPayloadRead::Denied(
                ExactPayloadReadDenial::PayloadNotLive,
            ));
        }
        let ciphertext = match row.storage_kind {
            ExactPayloadStorageKind::InlineDb => row.inline_ciphertext,
            ExactPayloadStorageKind::ManagedBlob => {
                let Some(blob_name) = row.managed_blob_name else {
                    return Ok(ExactPayloadRead::Denied(ExactPayloadReadDenial::Corrupt));
                };
                read_managed_ciphertext(capability, &blob_name)
                    .ok()
                    .filter(|bytes| bytes.len() as u64 == row.ciphertext_bytes)
            }
            ExactPayloadStorageKind::IncognitoMemory => None,
        };
        let (Some(nonce), Some(ciphertext)) = (row.nonce, ciphertext) else {
            return Ok(ExactPayloadRead::Denied(ExactPayloadReadDenial::Corrupt));
        };
        let plaintext = match decrypt_payload(
            payload_id,
            owner_id,
            &row.keyed_digest,
            &nonce,
            &ciphertext,
            &capability.key,
        ) {
            Ok(plaintext) => plaintext,
            Err(_) => return Ok(ExactPayloadRead::Denied(ExactPayloadReadDenial::Corrupt)),
        };
        if plaintext.len() as u64 != row.plaintext_bytes
            || keyed_payload_digest(&plaintext, &capability.key) != row.keyed_digest
        {
            return Ok(ExactPayloadRead::Denied(ExactPayloadReadDenial::Corrupt));
        }
        Ok(ExactPayloadRead::Authorized(plaintext))
    }

    /// SendUnknown is a durable retention hold.  TTL reconciliation and
    /// ordinary terminal cleanup are forbidden until it is explicitly
    /// resolved (session deletion still creates a cleanup tombstone).
    pub(crate) fn hold_exact_payload_for_send_unknown(
        &self,
        owner_id: &str,
        payload_id: &str,
    ) -> Result<bool> {
        validate_id("owner_id", owner_id)?;
        validate_uuid_id("payload_id", payload_id)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM request_payload_objects
                 WHERE payload_id = ?1 AND owner_id = ?2
                   AND object_state = 'live' AND retention_state = 'retained'
             )",
            params![payload_id, owner_id],
            |row| row.get(0),
        )?;
        if exists {
            tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'send_unknown', updated_at = ?2
                  WHERE owner_id = ?1 AND owner_state IN ('active', 'send_unknown')",
                params![owner_id, now],
            )?;
        }
        tx.commit()?;
        Ok(exists)
    }

    pub(crate) fn request_exact_payload_scrub(
        &self,
        owner_id: &str,
        payload_id: &str,
        reason: ExactPayloadScrubReason,
    ) -> Result<RequestExactPayloadScrubOutcome> {
        validate_id("owner_id", owner_id)?;
        validate_uuid_id("payload_id", payload_id)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = tx
            .query_row(
                "SELECT object.object_state, owner.owner_state
                   FROM request_payload_objects object
                   JOIN request_payload_owners owner ON owner.owner_id = object.owner_id
                  WHERE object.payload_id = ?1 AND object.owner_id = ?2",
                params![payload_id, owner_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((object_state, owner_state)) = state else {
            return Ok(RequestExactPayloadScrubOutcome::NotFound);
        };
        let object_state = ExactPayloadObjectState::from_db(&object_state)?;
        if object_state == ExactPayloadObjectState::Scrubbed {
            return Ok(RequestExactPayloadScrubOutcome::AlreadyScrubbed);
        }
        if object_state == ExactPayloadObjectState::Lost {
            // Lost has no ciphertext/nonce/blob left to scrub. Converge it
            // directly to the scrubbed audit state; routing it through
            // scrub_pending would violate both the transition trigger and the
            // storage-material CHECK constraints.
            tx.execute(
                "UPDATE request_payload_objects
                    SET availability = 'unavailable', object_state = 'scrubbed',
                        quota_state = 'released', retention_state = 'released',
                        nonce = NULL, inline_ciphertext = NULL,
                        managed_blob_name = NULL, ciphertext_bytes = 0,
                        scrub_reason = ?3, updated_at = ?4
                  WHERE payload_id = ?1 AND owner_id = ?2 AND object_state = 'lost'",
                params![payload_id, owner_id, reason.as_str(), now],
            )?;
            tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'released', updated_at = ?2
                  WHERE owner_id = ?1
                    AND NOT EXISTS (
                        SELECT 1 FROM request_payload_objects object
                         WHERE object.owner_id = ?1
                           AND object.object_state NOT IN ('scrubbed', 'lost')
                    )",
                params![owner_id, now],
            )?;
            tx.commit()?;
            return Ok(RequestExactPayloadScrubOutcome::AlreadyScrubbed);
        }
        let owner_state = ExactPayloadOwnerState::from_db(&owner_state)?;
        if owner_state == ExactPayloadOwnerState::SendUnknown
            && reason != ExactPayloadScrubReason::SendUnknownResolved
            && reason != ExactPayloadScrubReason::SessionDeleted
        {
            return Ok(RequestExactPayloadScrubOutcome::HeldBySendUnknown);
        }
        tx.execute(
            "UPDATE request_payload_objects
                SET object_state = 'scrub_pending',
                    retention_state = 'release_pending',
                    scrub_reason = ?3, updated_at = ?4
              WHERE payload_id = ?1 AND owner_id = ?2
                AND object_state = 'live'",
            params![payload_id, owner_id, reason.as_str(), now],
        )?;
        if reason == ExactPayloadScrubReason::SendUnknownResolved {
            tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'active', updated_at = ?2
                  WHERE owner_id = ?1 AND owner_state = 'send_unknown'",
                params![owner_id, now],
            )?;
        }
        tx.commit()?;
        Ok(RequestExactPayloadScrubOutcome::Pending)
    }

    pub(crate) fn scrub_exact_request_payload(
        &self,
        owner_id: &str,
        payload_id: &str,
    ) -> Result<RequestExactPayloadScrubOutcome> {
        if !kernel_private_request_payload_storage_available() {
            return Ok(RequestExactPayloadScrubOutcome::NotFound);
        }
        let Some(capability) = production_capability()? else {
            return Ok(RequestExactPayloadScrubOutcome::NotFound);
        };
        self.scrub_exact_request_payload_with_capability(owner_id, payload_id, &capability)
    }

    fn scrub_exact_request_payload_with_capability(
        &self,
        owner_id: &str,
        payload_id: &str,
        capability: &RequestPayloadCapability,
    ) -> Result<RequestExactPayloadScrubOutcome> {
        validate_id("owner_id", owner_id)?;
        validate_uuid_id("payload_id", payload_id)?;
        let row = {
            let conn = self.read_conn()?;
            load_payload_row(&conn, payload_id)?
        };
        let Some(row) = row else {
            return Ok(RequestExactPayloadScrubOutcome::NotFound);
        };
        if row.owner_id != owner_id {
            return Ok(RequestExactPayloadScrubOutcome::NotFound);
        }
        if row.state == ExactPayloadObjectState::Scrubbed {
            return Ok(RequestExactPayloadScrubOutcome::AlreadyScrubbed);
        }
        if row.state != ExactPayloadObjectState::ScrubPending {
            bail!("request payload must be scrub_pending before physical scrub")
        }
        if let Some(blob_name) = row.managed_blob_name.as_deref() {
            remove_managed_ciphertext(capability, blob_name)?;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reservation_id: String = tx.query_row(
            "SELECT reservation_id FROM request_payload_objects
              WHERE payload_id = ?1 AND owner_id = ?2 AND object_state = 'scrub_pending'",
            params![payload_id, owner_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE request_payload_objects
                SET availability = 'unavailable', object_state = 'scrubbed',
                    quota_state = 'released', retention_state = 'released',
                    nonce = NULL, inline_ciphertext = NULL, managed_blob_name = NULL,
                    ciphertext_bytes = 0, updated_at = ?2
              WHERE payload_id = ?1 AND object_state = 'scrub_pending'",
            params![payload_id, now],
        )?;
        tx.execute(
            "UPDATE request_payload_reservations
                SET quota_state = 'released', updated_at = ?2
              WHERE reservation_id = ?1",
            params![reservation_id, now],
        )?;
        let outstanding: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM request_payload_objects
                 WHERE owner_id = ?1 AND object_state NOT IN ('scrubbed', 'lost')
             )",
            params![owner_id],
            |row| row.get(0),
        )?;
        if !outstanding {
            tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'released', updated_at = ?2
                  WHERE owner_id = ?1",
                params![owner_id, now],
            )?;
        }
        tx.commit()?;
        Ok(RequestExactPayloadScrubOutcome::Pending)
    }

    pub(crate) fn reconcile_exact_request_payloads(&self) -> Result<ExactPayloadReconcileReport> {
        let Some(capability) = production_capability()? else {
            return Ok(ExactPayloadReconcileReport::default());
        };
        self.reconcile_exact_request_payloads_with_capability(
            &chrono::Utc::now().to_rfc3339(),
            &capability,
        )
    }

    fn reconcile_exact_request_payloads_with_capability(
        &self,
        now: &str,
        capability: &RequestPayloadCapability,
    ) -> Result<ExactPayloadReconcileReport> {
        let reconcile_time =
            chrono::DateTime::parse_from_rfc3339(now).context("reconcile time must be RFC3339")?;
        let stale_reservation_before = (reconcile_time.with_timezone(&chrono::Utc)
            - chrono::Duration::seconds(STALE_RESERVATION_AGE_SECS))
        .to_rfc3339();
        let mut report = ExactPayloadReconcileReport {
            capability_available: true,
            ..Default::default()
        };
        {
            let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            report.tombstones_claimed = tx.execute(
                "UPDATE request_payload_objects
                    SET object_state = 'scrub_pending',
                        retention_state = 'release_pending',
                        scrub_reason = 'session_deleted', updated_at = ?1
                  WHERE object_state = 'live'
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners
                         WHERE owner_state = 'cleanup_tombstone'
                    )",
                params![now],
            )? as u64;
            // Lost objects have neither ciphertext nor quota left, so they
            // can move directly to the scrubbed audit state.  Moving them via
            // scrub_pending would violate the invariant that a pending
            // physical scrub still owns nonce/storage material.
            report.payloads_scrubbed += tx.execute(
                "UPDATE request_payload_objects
                    SET availability = 'unavailable', object_state = 'scrubbed',
                        scrub_reason = 'session_deleted', nonce = NULL,
                        inline_ciphertext = NULL, managed_blob_name = NULL,
                        ciphertext_bytes = 0, quota_state = 'released',
                        retention_state = 'released', updated_at = ?1
                  WHERE object_state = 'lost'
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners
                         WHERE owner_state = 'cleanup_tombstone'
                    )",
                params![now],
            )? as u64;
            // The assistant/context transaction owns the request-plan
            // terminal edge and cannot also perform filesystem deletion.
            // Convert its durable outcome into scrub work here; a crash after
            // the terminal transaction therefore leaks neither quota nor
            // ciphertext indefinitely.
            report.terminal_payloads_claimed = tx.execute(
                "UPDATE request_payload_objects
                    SET object_state = 'scrub_pending',
                        retention_state = 'release_pending',
                        scrub_reason = CASE
                            WHEN EXISTS (
                                SELECT 1 FROM request_projection_plans plan
                                 WHERE plan.request_plan_id = request_payload_objects.owner_id
                                   AND plan.state = 'superseded'
                            ) THEN 'request_superseded'
                            ELSE 'request_terminal'
                        END,
                        updated_at = ?1
                  WHERE object_state = 'live'
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners
                         WHERE owner_state = 'active'
                    )
                    AND EXISTS (
                        SELECT 1 FROM request_projection_plans plan
                         WHERE plan.request_plan_id = request_payload_objects.owner_id
                           AND plan.state IN ('terminal', 'superseded')
                    )",
                params![now],
            )? as u64;
            report.payloads_scrubbed += tx.execute(
                "UPDATE request_payload_objects
                    SET availability = 'unavailable', object_state = 'scrubbed',
                        quota_state = 'released', retention_state = 'released',
                        nonce = NULL, inline_ciphertext = NULL,
                        managed_blob_name = NULL, ciphertext_bytes = 0,
                        scrub_reason = 'request_terminal', updated_at = ?1
                  WHERE object_state = 'lost'
                    AND EXISTS (
                        SELECT 1 FROM request_projection_plans plan
                         WHERE plan.request_plan_id = request_payload_objects.owner_id
                           AND plan.state IN ('terminal', 'superseded')
                    )",
                params![now],
            )? as u64;
            // SendUnknown intentionally does not participate in TTL cleanup.
            report.expired_claimed = tx.execute(
                "UPDATE request_payload_objects
                    SET object_state = 'scrub_pending',
                        retention_state = 'release_pending',
                        scrub_reason = 'retention_expired', updated_at = ?1
                  WHERE object_state = 'live'
                    AND expires_at IS NOT NULL AND expires_at <= ?1
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners
                         WHERE owner_state = 'active'
                    )",
                params![now],
            )? as u64;
            // Payload staging necessarily precedes request-plan publication
            // because the plan records the immutable payload hold. A crash or
            // a lost plan-version CAS can therefore leave an active owner
            // without a plan. Do not race a live publisher: only claim rows
            // older than the same bounded reservation grace used below.
            report.orphan_payloads_claimed = tx.execute(
                "UPDATE request_payload_objects
                    SET object_state = 'scrub_pending',
                        retention_state = 'release_pending',
                        scrub_reason = 'reconcile_corrupt', updated_at = ?1
                  WHERE object_state = 'live' AND created_at <= ?2
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners
                         WHERE owner_state = 'active'
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM request_projection_plans plan
                         WHERE plan.request_plan_id = request_payload_objects.owner_id
                    )",
                params![now, stale_reservation_before],
            )? as u64;
            report.stale_reservations_released = tx.execute(
                "UPDATE request_payload_reservations
                    SET quota_state = 'released', updated_at = ?1
                  WHERE quota_state = 'reserved'
                    AND created_at <= ?2
                    AND NOT EXISTS (
                        SELECT 1 FROM request_payload_objects object
                         WHERE object.reservation_id = request_payload_reservations.reservation_id
                    )",
                params![now, stale_reservation_before],
            )? as u64;
            tx.commit()?;
        }

        let pending: Vec<(String, String)> = {
            let conn = self.read_conn()?;
            let mut statement = conn.prepare(
                "SELECT owner_id, payload_id FROM request_payload_objects
                  WHERE object_state = 'scrub_pending' ORDER BY created_at, payload_id",
            )?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (owner_id, payload_id) in pending {
            if self
                .scrub_exact_request_payload_with_capability(&owner_id, &payload_id, capability)
                .is_ok()
            {
                report.payloads_scrubbed += 1;
            }
        }

        {
            let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE request_payload_owners
                    SET owner_state = 'released', updated_at = ?1
                  WHERE owner_state IN ('active', 'cleanup_tombstone')
                    AND NOT EXISTS (
                        SELECT 1 FROM request_payload_objects object
                         WHERE object.owner_id = request_payload_owners.owner_id
                           AND object.object_state NOT IN ('scrubbed', 'lost')
                    )",
                params![now],
            )?;
            // Session deletion and failed plan publication have no surviving
            // stream/run row through which journal GC could discover these
            // owners. Once their material is gone and no request plan refers
            // to them, remove the bounded coordination metadata here.
            report.orphan_metadata_removed += tx.execute(
                "DELETE FROM request_payload_objects
                  WHERE object_state IN ('scrubbed', 'lost')
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners owner
                         WHERE owner.owner_state = 'released'
                           AND NOT EXISTS (
                               SELECT 1 FROM request_projection_plans plan
                                WHERE plan.request_plan_id = owner.owner_id
                           )
                    )",
                [],
            )? as u64;
            report.orphan_metadata_removed += tx.execute(
                "DELETE FROM request_payload_reservations
                  WHERE quota_state = 'released'
                    AND owner_id IN (
                        SELECT owner_id FROM request_payload_owners owner
                         WHERE owner.owner_state = 'released'
                           AND NOT EXISTS (
                               SELECT 1 FROM request_projection_plans plan
                                WHERE plan.request_plan_id = owner.owner_id
                           )
                    )",
                [],
            )? as u64;
            report.orphan_metadata_removed += tx.execute(
                "DELETE FROM request_payload_owners
                  WHERE owner_state = 'released'
                    AND NOT EXISTS (
                        SELECT 1 FROM request_projection_plans plan
                         WHERE plan.request_plan_id = request_payload_owners.owner_id
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM request_payload_objects object
                         WHERE object.owner_id = request_payload_owners.owner_id
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM request_payload_reservations reservation
                         WHERE reservation.owner_id = request_payload_owners.owner_id
                    )",
                [],
            )? as u64;
            tx.commit()?;
        }

        let live_managed: Vec<(String, String, String)> = {
            let conn = self.read_conn()?;
            let mut statement = conn.prepare(
                "SELECT owner_id, payload_id, managed_blob_name
                   FROM request_payload_objects
                  WHERE storage_kind = 'managed_blob' AND object_state = 'live'",
            )?;
            let rows =
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (owner_id, payload_id, blob_name) in live_managed {
            if !managed_blob_path(capability, &blob_name).is_file() {
                self.mark_live_payload_lost(&owner_id, &payload_id, now)?;
                report.payloads_marked_lost += 1;
            }
        }

        let known_blob_names: std::collections::HashSet<String> = {
            let conn = self.read_conn()?;
            let mut statement = conn.prepare(
                "SELECT managed_blob_name FROM request_payload_objects
                  WHERE managed_blob_name IS NOT NULL
                 UNION
                 SELECT payload_id || '.bin' FROM request_payload_reservations
                  WHERE quota_state = 'reserved'",
            )?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let directory = capability.root.join(MANAGED_BLOB_DIRECTORY);
        if let Ok(entries) = std::fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if is_managed_blob_name(&name) && !known_blob_names.contains(&name) {
                    // Close the snapshot race with a concurrent stage: its
                    // reservation is committed before the blob is published.
                    // If the file exists now, this live recheck must observe
                    // either that reservation or its finalized object.
                    let payload_id = name.trim_end_matches(".bin");
                    let claimed: bool = {
                        let conn = self.read_conn()?;
                        conn.query_row(
                            "SELECT EXISTS(
                                SELECT 1 FROM request_payload_objects
                                 WHERE managed_blob_name = ?1
                                UNION ALL
                                SELECT 1 FROM request_payload_reservations
                                 WHERE payload_id = ?2 AND quota_state = 'reserved'
                             )",
                            params![name, payload_id],
                            |row| row.get(0),
                        )?
                    };
                    if claimed {
                        continue;
                    }
                    if remove_managed_ciphertext(capability, &name).is_ok() {
                        report.orphan_blobs_removed += 1;
                    }
                }
            }
        }
        Ok(report)
    }

    fn mark_live_payload_lost(&self, owner_id: &str, payload_id: &str, now: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reservation_id: Option<String> = tx
            .query_row(
                "SELECT reservation_id FROM request_payload_objects
                  WHERE payload_id = ?1 AND owner_id = ?2 AND object_state = 'live'",
                params![payload_id, owner_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(reservation_id) = reservation_id else {
            return Ok(());
        };
        tx.execute(
            "UPDATE request_payload_objects
                SET availability = 'lost', object_state = 'lost',
                    quota_state = 'released', retention_state = 'released',
                    nonce = NULL, inline_ciphertext = NULL, managed_blob_name = NULL,
                    ciphertext_bytes = 0, last_error = 'managed blob missing',
                    updated_at = ?2
              WHERE payload_id = ?1",
            params![payload_id, now],
        )?;
        tx.execute(
            "UPDATE request_payload_reservations
                SET quota_state = 'released', updated_at = ?2
              WHERE reservation_id = ?1",
            params![reservation_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Per-turn/session exact-body holder for incognito requests.  It owns all
/// bytes, has no global registry, never opens SQLite or creates a file, and
/// zeros every retained buffer when scrubbed or dropped.
pub(crate) struct IncognitoExactPayloadStore {
    session_id: String,
    max_bytes: usize,
    used_bytes: usize,
    entries: HashMap<String, IncognitoPayloadEntry>,
}

struct IncognitoPayloadEntry {
    owner_id: String,
    bytes: Vec<u8>,
    send_unknown: bool,
}

impl Drop for IncognitoPayloadEntry {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

impl IncognitoExactPayloadStore {
    pub(crate) fn new(session_id: impl Into<String>, max_bytes: Option<usize>) -> Result<Self> {
        let session_id = session_id.into();
        validate_id("session_id", &session_id)?;
        let max_bytes = max_bytes
            .unwrap_or(DEFAULT_INCOGNITO_EXACT_PAYLOAD_BYTES)
            .min(MAX_EXACT_REQUEST_PAYLOAD_BYTES);
        Ok(Self {
            session_id,
            max_bytes,
            used_bytes: 0,
            entries: HashMap::new(),
        })
    }

    pub(crate) fn stage(
        &mut self,
        session_id: &str,
        owner_id: &str,
        json: &[u8],
    ) -> Result<StageExactPayloadOutcome> {
        validate_id("owner_id", owner_id)?;
        validate_final_provider_json(json)?;
        if session_id != self.session_id {
            bail!("incognito payload store belongs to another session")
        }
        if json.len() > self.max_bytes.saturating_sub(self.used_bytes) {
            return Ok(unavailable(
                ExactPayloadUnavailableReason::QuotaExceeded,
                json.len(),
            ));
        }
        let payload_id = uuid::Uuid::new_v4().to_string();
        let reservation_id = uuid::Uuid::new_v4().to_string();
        self.used_bytes += json.len();
        self.entries.insert(
            payload_id.clone(),
            IncognitoPayloadEntry {
                owner_id: owner_id.to_string(),
                bytes: json.to_vec(),
                send_unknown: false,
            },
        );
        Ok(StageExactPayloadOutcome::Stored(ExactPayloadHold {
            availability: ExactPayloadAvailability::Stored,
            payload_id,
            owner_id: owner_id.to_string(),
            reservation_id,
            storage_kind: ExactPayloadStorageKind::IncognitoMemory,
            plaintext_bytes: json.len() as u64,
            // Incognito values never get a durable/cross-run identity.
            keyed_digest: "incognito:ephemeral".to_string(),
            object_state: ExactPayloadObjectState::Live,
            quota_state: ExactPayloadQuotaState::Committed,
            retention_state: ExactPayloadRetentionState::Retained,
        }))
    }

    pub(crate) fn read(
        &self,
        session_id: &str,
        owner_id: &str,
        payload_id: &str,
    ) -> ExactPayloadRead {
        if session_id != self.session_id {
            return ExactPayloadRead::Denied(ExactPayloadReadDenial::OwnerMismatch);
        }
        match self.entries.get(payload_id) {
            Some(entry) if entry.owner_id == owner_id => {
                ExactPayloadRead::Authorized(entry.bytes.clone())
            }
            Some(_) => ExactPayloadRead::Denied(ExactPayloadReadDenial::OwnerMismatch),
            None => ExactPayloadRead::Denied(ExactPayloadReadDenial::PayloadNotFound),
        }
    }

    pub(crate) fn hold_send_unknown(&mut self, owner_id: &str, payload_id: &str) -> bool {
        let Some(entry) = self.entries.get_mut(payload_id) else {
            return false;
        };
        if entry.owner_id != owner_id {
            return false;
        }
        entry.send_unknown = true;
        true
    }

    pub(crate) fn scrub(
        &mut self,
        owner_id: &str,
        payload_id: &str,
        send_unknown_resolved: bool,
    ) -> RequestExactPayloadScrubOutcome {
        let Some(entry) = self.entries.get(payload_id) else {
            return RequestExactPayloadScrubOutcome::NotFound;
        };
        if entry.owner_id != owner_id {
            return RequestExactPayloadScrubOutcome::NotFound;
        }
        if entry.send_unknown && !send_unknown_resolved {
            return RequestExactPayloadScrubOutcome::HeldBySendUnknown;
        }
        let mut entry = self
            .entries
            .remove(payload_id)
            .expect("entry checked above");
        self.used_bytes = self.used_bytes.saturating_sub(entry.bytes.len());
        entry.bytes.fill(0);
        RequestExactPayloadScrubOutcome::Pending
    }
}

impl Drop for IncognitoExactPayloadStore {
    fn drop(&mut self) {
        self.entries.clear();
        self.used_bytes = 0;
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS request_payload_owners (
    owner_id TEXT PRIMARY KEY,
    -- Deliberately no FK: the row becomes a cleanup tombstone when the
    -- session is deleted and must survive long enough to scrub its body.
    session_id TEXT NOT NULL,
    owner_state TEXT NOT NULL CHECK (
        owner_state IN ('active', 'send_unknown', 'cleanup_tombstone', 'released')
    ),
    tombstoned_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (owner_state != 'cleanup_tombstone' OR tombstoned_at IS NOT NULL)
);

CREATE TABLE IF NOT EXISTS request_payload_reservations (
    reservation_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL UNIQUE,
    payload_id TEXT NOT NULL UNIQUE,
    reserved_bytes INTEGER NOT NULL CHECK (reserved_bytes >= 0),
    quota_state TEXT NOT NULL CHECK (quota_state IN ('reserved', 'committed', 'released')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_id) REFERENCES request_payload_owners(owner_id)
);

CREATE TABLE IF NOT EXISTS request_payload_objects (
    payload_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    reservation_id TEXT NOT NULL UNIQUE,
    storage_kind TEXT NOT NULL CHECK (storage_kind IN ('inline_db', 'managed_blob')),
    availability TEXT NOT NULL CHECK (availability IN ('stored', 'unavailable', 'lost')),
    object_state TEXT NOT NULL CHECK (
        object_state IN ('live', 'scrub_pending', 'scrubbed', 'lost')
    ),
    quota_state TEXT NOT NULL CHECK (quota_state IN ('reserved', 'committed', 'released')),
    retention_state TEXT NOT NULL CHECK (
        retention_state IN ('retained', 'release_pending', 'released')
    ),
    plaintext_bytes INTEGER NOT NULL CHECK (plaintext_bytes >= 0),
    ciphertext_bytes INTEGER NOT NULL CHECK (ciphertext_bytes >= 0),
    cipher_version INTEGER NOT NULL CHECK (cipher_version = 1),
    keyed_digest TEXT NOT NULL,
    nonce BLOB,
    inline_ciphertext BLOB,
    -- A generated UUID filename only, never a caller path or absolute path.
    managed_blob_name TEXT,
    expires_at TEXT,
    scrub_reason TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_id) REFERENCES request_payload_owners(owner_id),
    FOREIGN KEY (reservation_id) REFERENCES request_payload_reservations(reservation_id),
    CHECK (
        object_state IN ('scrubbed', 'lost')
        OR (nonce IS NOT NULL AND length(nonce) = 12)
    ),
    CHECK (
        object_state IN ('scrubbed', 'lost')
        OR (
            (storage_kind = 'inline_db' AND inline_ciphertext IS NOT NULL AND managed_blob_name IS NULL)
            OR
            (storage_kind = 'managed_blob' AND inline_ciphertext IS NULL AND managed_blob_name IS NOT NULL)
        )
    ),
    CHECK (object_state != 'live' OR availability = 'stored'),
    CHECK (object_state != 'scrubbed' OR (
        availability = 'unavailable' AND nonce IS NULL AND inline_ciphertext IS NULL
        AND managed_blob_name IS NULL AND ciphertext_bytes = 0
        AND quota_state = 'released' AND retention_state = 'released'
    )),
    CHECK (object_state != 'lost' OR (
        availability = 'lost' AND quota_state = 'released' AND retention_state = 'released'
    ))
);

CREATE INDEX IF NOT EXISTS idx_request_payload_owner_state
    ON request_payload_owners(owner_state, updated_at);
CREATE INDEX IF NOT EXISTS idx_request_payload_object_cleanup
    ON request_payload_objects(object_state, retention_state, expires_at);
CREATE INDEX IF NOT EXISTS idx_request_payload_reservation_quota
    ON request_payload_reservations(quota_state, updated_at);

CREATE TRIGGER IF NOT EXISTS request_payload_session_delete_tombstone
AFTER DELETE ON sessions
BEGIN
    UPDATE request_payload_owners
       SET owner_state = 'cleanup_tombstone',
           tombstoned_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE session_id = OLD.id AND owner_state != 'released';
END;

CREATE TRIGGER IF NOT EXISTS request_payload_owner_identity_immutable
BEFORE UPDATE OF owner_id, session_id, created_at ON request_payload_owners
BEGIN
    SELECT RAISE(ABORT, 'request payload owner identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_reservation_identity_immutable
BEFORE UPDATE OF reservation_id, owner_id, payload_id, reserved_bytes, created_at
ON request_payload_reservations
BEGIN
    SELECT RAISE(ABORT, 'request payload reservation identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_object_identity_immutable
BEFORE UPDATE OF payload_id, owner_id, reservation_id, storage_kind,
                 plaintext_bytes, cipher_version, keyed_digest, created_at
ON request_payload_objects
BEGIN
    SELECT RAISE(ABORT, 'request payload object identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_owner_state_guard
BEFORE UPDATE OF owner_state ON request_payload_owners
WHEN NOT (
    OLD.owner_state = NEW.owner_state
    OR (OLD.owner_state = 'active' AND NEW.owner_state IN ('send_unknown', 'cleanup_tombstone', 'released'))
    OR (OLD.owner_state = 'send_unknown' AND NEW.owner_state IN ('active', 'cleanup_tombstone'))
    OR (OLD.owner_state = 'cleanup_tombstone' AND NEW.owner_state = 'released')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request payload owner state transition');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_reservation_state_guard
BEFORE UPDATE OF quota_state ON request_payload_reservations
WHEN NOT (
    OLD.quota_state = NEW.quota_state
    OR (OLD.quota_state = 'reserved' AND NEW.quota_state IN ('committed', 'released'))
    OR (OLD.quota_state = 'committed' AND NEW.quota_state = 'released')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request payload reservation state transition');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_object_state_guard
BEFORE UPDATE OF object_state ON request_payload_objects
WHEN NOT (
    OLD.object_state = NEW.object_state
    OR (OLD.object_state = 'live' AND NEW.object_state IN ('scrub_pending', 'lost'))
    OR (OLD.object_state = 'scrub_pending' AND NEW.object_state IN ('scrubbed', 'lost'))
    OR (OLD.object_state = 'lost' AND NEW.object_state = 'scrubbed')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request payload object state transition');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_object_quota_state_guard
BEFORE UPDATE OF quota_state ON request_payload_objects
WHEN NOT (
    OLD.quota_state = NEW.quota_state
    OR (OLD.quota_state = 'reserved' AND NEW.quota_state IN ('committed', 'released'))
    OR (OLD.quota_state = 'committed' AND NEW.quota_state = 'released')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request payload object quota transition');
END;

CREATE TRIGGER IF NOT EXISTS request_payload_retention_state_guard
BEFORE UPDATE OF retention_state ON request_payload_objects
WHEN NOT (
    OLD.retention_state = NEW.retention_state
    OR (OLD.retention_state = 'retained' AND NEW.retention_state IN ('release_pending', 'released'))
    OR (OLD.retention_state = 'release_pending' AND NEW.retention_state = 'released')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid request payload retention transition');
END;
"#;

fn unavailable(reason: ExactPayloadUnavailableReason, bytes: usize) -> StageExactPayloadOutcome {
    StageExactPayloadOutcome::Unavailable(ExactPayloadUnavailable {
        availability: ExactPayloadAvailability::Unavailable,
        reason,
        requested_bytes: bytes as u64,
    })
}

fn validate_final_provider_json(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        bail!("exact Provider request JSON cannot be empty")
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("exact Provider request is not valid JSON")?;
    let object = value
        .as_object()
        .context("exact Provider request JSON must be an object")?;
    // Transport authorization must remain outside the durable body.  Check
    // only root keys so user text mentioning these words is not rejected.
    for key in object.keys() {
        let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
        if matches!(
            normalized.as_str(),
            "authorization" | "apikey" | "accesstoken" | "oauthtoken" | "bearertoken"
        ) {
            bail!("exact Provider request JSON contains a transport credential field")
        }
    }
    Ok(())
}

fn normalize_expires_at(value: Option<&str>) -> Result<Option<String>> {
    value
        .map(|value| {
            validate_text("expires_at", value, 128)?;
            let parsed = chrono::DateTime::parse_from_rfc3339(value)
                .context("expires_at must be RFC3339")?;
            Ok(parsed.with_timezone(&chrono::Utc).to_rfc3339())
        })
        .transpose()
}

fn validate_id(name: &str, value: &str) -> Result<()> {
    validate_text(name, value, MAX_ID_BYTES)
}

fn validate_uuid_id(name: &str, value: &str) -> Result<()> {
    validate_id(name, value)?;
    uuid::Uuid::parse_str(value).with_context(|| format!("{name} must be a UUID"))?;
    Ok(())
}

fn validate_text(name: &str, value: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.len() > max || value.contains('\0') {
        bail!("{name} must contain 1..={max} non-NUL UTF-8 bytes")
    }
    Ok(())
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("request payload byte count exceeds SQLite INTEGER")
}

fn require_persistent_session(conn: &rusqlite::Connection, session_id: &str) -> Result<()> {
    let incognito = conn
        .query_row(
            "SELECT incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    if incognito != 0 {
        bail!("incognito sessions cannot persist exact request payloads")
    }
    Ok(())
}

fn insert_or_validate_owner(
    tx: &rusqlite::Transaction<'_>,
    owner_id: &str,
    session_id: &str,
    now: &str,
) -> Result<()> {
    let existing = tx
        .query_row(
            "SELECT session_id, owner_state FROM request_payload_owners WHERE owner_id = ?1",
            params![owner_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            tx.execute(
                "INSERT INTO request_payload_owners (
                    owner_id, session_id, owner_state, tombstoned_at, created_at, updated_at
                 ) VALUES (?1, ?2, 'active', NULL, ?3, ?3)",
                params![owner_id, session_id, now],
            )?;
        }
        Some((existing_session, state))
            if existing_session == session_id
                && state == ExactPayloadOwnerState::Active.as_str() => {}
        Some(_) => bail!("request payload owner is not active for this session"),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_live_payload(
    tx: &rusqlite::Transaction<'_>,
    payload_id: &str,
    owner_id: &str,
    reservation_id: &str,
    storage_kind: ExactPayloadStorageKind,
    plaintext_bytes: u64,
    ciphertext_bytes: u64,
    keyed_digest: &str,
    nonce: &[u8; PAYLOAD_NONCE_BYTES],
    inline_ciphertext: Option<&[u8]>,
    managed_blob_name: Option<&str>,
    expires_at: Option<&str>,
    now: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO request_payload_objects (
            payload_id, owner_id, reservation_id, storage_kind, availability,
            object_state, quota_state, retention_state, plaintext_bytes,
            ciphertext_bytes, cipher_version, keyed_digest, nonce,
            inline_ciphertext, managed_blob_name, expires_at, scrub_reason,
            last_error, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, 'stored', 'live', 'committed', 'retained',
            ?5, ?6, 1, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12, ?12
         )",
        params![
            payload_id,
            owner_id,
            reservation_id,
            storage_kind.as_str(),
            u64_to_i64(plaintext_bytes)?,
            u64_to_i64(ciphertext_bytes)?,
            keyed_digest,
            nonce.as_slice(),
            inline_ciphertext,
            managed_blob_name,
            expires_at,
            now
        ],
    )?;
    Ok(())
}

fn commit_reservation(
    tx: &rusqlite::Transaction<'_>,
    reservation_id: &str,
    now: &str,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE request_payload_reservations
            SET quota_state = 'committed', updated_at = ?2
          WHERE reservation_id = ?1 AND quota_state = 'reserved'",
        params![reservation_id, now],
    )?;
    if changed != 1 {
        bail!("request payload reservation was not reserved")
    }
    Ok(())
}

fn load_payload_row(
    conn: &rusqlite::Connection,
    payload_id: &str,
) -> Result<Option<StoredPayloadRow>> {
    conn.query_row(
        "SELECT owner_id, storage_kind, availability, object_state,
                retention_state, plaintext_bytes, ciphertext_bytes,
                keyed_digest, nonce, inline_ciphertext, managed_blob_name
           FROM request_payload_objects WHERE payload_id = ?1",
        params![payload_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<Vec<u8>>>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        },
    )
    .optional()?
    .map(
        |(
            owner_id,
            storage_kind,
            availability,
            state,
            retention_state,
            plaintext_bytes,
            ciphertext_bytes,
            keyed_digest,
            nonce,
            inline_ciphertext,
            managed_blob_name,
        )| {
            Ok(StoredPayloadRow {
                owner_id,
                storage_kind: ExactPayloadStorageKind::from_db(&storage_kind)?,
                availability: ExactPayloadAvailability::from_db(&availability)?,
                state: ExactPayloadObjectState::from_db(&state)?,
                retention_state: ExactPayloadRetentionState::from_db(&retention_state)?,
                plaintext_bytes: u64::try_from(plaintext_bytes)
                    .context("negative request payload plaintext bytes")?,
                ciphertext_bytes: u64::try_from(ciphertext_bytes)
                    .context("negative request payload ciphertext bytes")?,
                keyed_digest,
                nonce,
                inline_ciphertext,
                managed_blob_name,
            })
        },
    )
    .transpose()
}

fn keyed_payload_digest(bytes: &[u8], key: &[u8; PAYLOAD_KEY_BYTES]) -> String {
    let digest_key = blake3::derive_key("hope-agent request-payload keyed digest v1", key);
    // The request-plan schema stores fingerprints as canonical hexadecimal
    // bytes. The algorithm/version is already fixed by the column contract;
    // adding a textual prefix here would make an otherwise valid stored hold
    // fail the plan's trigger-backed fingerprint validation.
    blake3::keyed_hash(&digest_key, bytes).to_hex().to_string()
}

fn payload_aad(payload_id: &str, owner_id: &str, keyed_digest: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(payload_id.len() + owner_id.len() + keyed_digest.len() + 96);
    aad.extend_from_slice(b"hope-agent:request-payload:v1\0");
    for value in [payload_id, owner_id, keyed_digest] {
        aad.extend_from_slice(&(value.len() as u64).to_le_bytes());
        aad.extend_from_slice(value.as_bytes());
    }
    aad
}

fn encrypt_payload(
    payload_id: &str,
    owner_id: &str,
    keyed_digest: &str,
    plaintext: &[u8],
    key: &[u8; PAYLOAD_KEY_BYTES],
) -> Result<([u8; PAYLOAD_NONCE_BYTES], Vec<u8>)> {
    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| anyhow!("initialize request payload cipher"))?;
    let key = LessSafeKey::new(unbound);
    let mut nonce = [0_u8; PAYLOAD_NONCE_BYTES];
    rand::rng().fill_bytes(&mut nonce);
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(payload_aad(payload_id, owner_id, keyed_digest)),
        &mut ciphertext,
    )
    .map_err(|_| anyhow!("encrypt exact request payload"))?;
    Ok((nonce, ciphertext))
}

fn decrypt_payload(
    payload_id: &str,
    owner_id: &str,
    keyed_digest: &str,
    nonce: &[u8],
    ciphertext: &[u8],
    key: &[u8; PAYLOAD_KEY_BYTES],
) -> Result<Vec<u8>> {
    let nonce: [u8; PAYLOAD_NONCE_BYTES] = nonce
        .try_into()
        .map_err(|_| anyhow!("invalid request payload nonce"))?;
    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| anyhow!("initialize request payload cipher"))?;
    let key = LessSafeKey::new(unbound);
    let mut plaintext = ciphertext.to_vec();
    let opened = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(payload_aad(payload_id, owner_id, keyed_digest)),
            &mut plaintext,
        )
        .map_err(|_| anyhow!("authenticate exact request payload"))?;
    Ok(opened.to_vec())
}

fn managed_blob_name(payload_id: &str) -> String {
    format!("{payload_id}.bin")
}

fn is_managed_blob_name(name: &str) -> bool {
    name.strip_suffix(".bin")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .is_some()
}

fn managed_blob_path(capability: &RequestPayloadCapability, blob_name: &str) -> PathBuf {
    debug_assert!(is_managed_blob_name(blob_name));
    capability.root.join(MANAGED_BLOB_DIRECTORY).join(blob_name)
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("request payload root is not a regular directory")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_managed_ciphertext(
    capability: &RequestPayloadCapability,
    blob_name: &str,
    ciphertext: &[u8],
) -> Result<()> {
    if !is_managed_blob_name(blob_name) {
        bail!("invalid managed request payload blob name")
    }
    let directory = capability.root.join(MANAGED_BLOB_DIRECTORY);
    ensure_private_directory(&directory)?;
    let path = managed_blob_path(capability, blob_name);
    crate::platform::write_secure_file(&path, ciphertext)
        .with_context(|| format!("publish encrypted request payload {}", path.display()))?;
    Ok(())
}

fn read_managed_ciphertext(
    capability: &RequestPayloadCapability,
    blob_name: &str,
) -> Result<Vec<u8>> {
    if !is_managed_blob_name(blob_name) {
        bail!("invalid managed request payload blob name")
    }
    let path = managed_blob_path(capability, blob_name);
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("managed request payload is not a regular file")
    }
    std::fs::read(&path)
        .with_context(|| format!("read encrypted request payload {}", path.display()))
}

fn remove_managed_ciphertext(capability: &RequestPayloadCapability, blob_name: &str) -> Result<()> {
    if !is_managed_blob_name(blob_name) {
        bail!("invalid managed request payload blob name")
    }
    let path = managed_blob_path(capability, blob_name);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn production_capability() -> Result<Option<RequestPayloadCapability>> {
    if !kernel_private_request_payload_storage_available() {
        return Ok(None);
    }
    let key = load_or_create_request_payload_key()?;
    let root = crate::paths::root_dir()?.join("kernel-private");
    ensure_private_directory(&root)?;
    Ok(Some(RequestPayloadCapability {
        root,
        key,
        quota_bytes: DEFAULT_EXACT_REQUEST_STORE_QUOTA_BYTES,
    }))
}

fn load_or_create_request_payload_key() -> Result<[u8; PAYLOAD_KEY_BYTES]> {
    if let Some(key) = REQUEST_PAYLOAD_KEY.get() {
        return Ok(*key);
    }
    let directory = crate::paths::credentials_dir()?;
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(PAYLOAD_KEY_FILE);
    let lock_path = directory.join(PAYLOAD_KEY_LOCK_FILE);
    let deadline = Instant::now() + PAYLOAD_KEY_PUBLICATION_TIMEOUT;
    let mut delay = Duration::from_millis(5);
    loop {
        if let Some(key) = read_request_payload_key_at(&path)? {
            let _ = REQUEST_PAYLOAD_KEY.set(key);
            return Ok(key);
        }
        match crate::platform::try_acquire_exclusive_lock(&lock_path)? {
            Some(_guard) => {
                if let Some(key) = read_request_payload_key_at(&path)? {
                    let _ = REQUEST_PAYLOAD_KEY.set(key);
                    return Ok(key);
                }
                let key: [u8; PAYLOAD_KEY_BYTES] = rand::random();
                crate::platform::write_secure_file(&path, &key)?;
                let published = read_request_payload_key_at(&path)?
                    .context("request payload key publication was not readable")?;
                let _ = REQUEST_PAYLOAD_KEY.set(published);
                return Ok(published);
            }
            None => {
                let now = Instant::now();
                if now >= deadline {
                    bail!("timed out waiting for request payload key publication")
                }
                std::thread::sleep(delay.min(deadline.saturating_duration_since(now)));
                delay = delay.saturating_mul(2).min(Duration::from_millis(50));
            }
        }
    }
}

fn read_request_payload_key_at(path: &Path) -> Result<Option<[u8; PAYLOAD_KEY_BYTES]>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("request payload key path is not a regular file")
    }
    let bytes = std::fs::read(path)?;
    let key = bytes
        .try_into()
        .map_err(|_| anyhow!("request payload key has an invalid length"))?;
    Ok(Some(key))
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
        let directory = tempfile::tempdir().expect("tempdir");
        let db =
            SessionDB::open_ephemeral_for_test(&directory.path().join(name)).expect("session db");
        (directory, db)
    }

    fn capability(directory: &tempfile::TempDir, quota_bytes: u64) -> RequestPayloadCapability {
        RequestPayloadCapability {
            root: directory.path().join("private"),
            key: [0x5a; PAYLOAD_KEY_BYTES],
            quota_bytes,
        }
    }

    fn request_json(padding: usize) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "x".repeat(padding)}]
        }))
        .expect("json")
    }

    #[test]
    fn closed_capability_and_incognito_have_zero_store_side_effects() {
        let (directory, db) = test_db("closed.db");
        let regular = db.create_session("ha-main").expect("regular session");
        let incognito = db
            .create_session_with_project("ha-main", None, Some(true))
            .expect("incognito session");
        let json = request_json(16);
        assert!(matches!(
            db.stage_exact_request_payload(&regular.id, "plan-regular", &json, None)
                .expect("closed stage"),
            StageExactPayloadOutcome::Unavailable(ExactPayloadUnavailable {
                reason: ExactPayloadUnavailableReason::KernelPrivateStorageUnavailable,
                ..
            })
        ));
        assert!(matches!(
            db.stage_exact_request_payload(&incognito.id, "plan-incognito", &json, None)
                .expect("incognito stage"),
            StageExactPayloadOutcome::Unavailable(ExactPayloadUnavailable {
                reason: ExactPayloadUnavailableReason::IncognitoRequiresMemoryStore,
                ..
            })
        ));
        db.with_conn_for_test(|conn| {
            for table in [
                "request_payload_owners",
                "request_payload_reservations",
                "request_payload_objects",
            ] {
                let count: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                assert_eq!(count, 0, "unexpected row in {table}");
            }
            Ok(())
        })
        .expect("row counts");
        assert!(!directory.path().join("private").exists());
    }

    #[test]
    fn inline_crypto_authorization_send_unknown_and_scrub_contract() {
        let (directory, db) = test_db("inline.db");
        let session = db.create_session("ha-main").expect("session");
        let stranger = db.create_session("ha-main").expect("stranger");
        let capability = capability(&directory, 16 * 1024 * 1024);
        let json = request_json(128);
        let hold = match db
            .stage_exact_request_payload_with_capability(
                &session.id,
                "plan-inline",
                &json,
                Some("2030-01-01T00:00:00Z"),
                &capability,
            )
            .expect("stage")
        {
            StageExactPayloadOutcome::Stored(hold) => hold,
            other => panic!("unexpected stage: {other:?}"),
        };
        assert_eq!(hold.storage_kind, ExactPayloadStorageKind::InlineDb);
        assert_eq!(
            db.read_authorized_exact_request_payload_with_capability(
                &session.id,
                &hold.owner_id,
                &hold.payload_id,
                &capability,
            )
            .expect("read"),
            ExactPayloadRead::Authorized(json)
        );
        assert_eq!(
            db.read_authorized_exact_request_payload_with_capability(
                &stranger.id,
                &hold.owner_id,
                &hold.payload_id,
                &capability,
            )
            .expect("stranger read"),
            ExactPayloadRead::Denied(ExactPayloadReadDenial::OwnerMismatch)
        );
        assert!(db
            .hold_exact_payload_for_send_unknown(&hold.owner_id, &hold.payload_id)
            .expect("hold"));
        assert_eq!(
            db.request_exact_payload_scrub(
                &hold.owner_id,
                &hold.payload_id,
                ExactPayloadScrubReason::RequestTerminal,
            )
            .expect("blocked scrub"),
            RequestExactPayloadScrubOutcome::HeldBySendUnknown
        );
        assert_eq!(
            db.request_exact_payload_scrub(
                &hold.owner_id,
                &hold.payload_id,
                ExactPayloadScrubReason::SendUnknownResolved,
            )
            .expect("request scrub"),
            RequestExactPayloadScrubOutcome::Pending
        );
        db.scrub_exact_request_payload_with_capability(
            &hold.owner_id,
            &hold.payload_id,
            &capability,
        )
        .expect("physical scrub");
        db.with_conn_for_test(|conn| {
            let row: (String, String, String, Option<Vec<u8>>) = conn.query_row(
                "SELECT object_state, quota_state, retention_state, inline_ciphertext
                   FROM request_payload_objects WHERE payload_id = ?1",
                params![hold.payload_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert_eq!(row.0, "scrubbed");
            assert_eq!(row.1, "released");
            assert_eq!(row.2, "released");
            assert!(row.3.is_none());
            Ok(())
        })
        .expect("scrubbed row");
    }

    #[test]
    fn managed_blob_is_encrypted_atomic_and_session_delete_keeps_tombstone() {
        let (directory, db) = test_db("blob.db");
        ensure_channel_conversations_table(&db);
        let session = db.create_session("ha-main").expect("session");
        let capability = capability(&directory, 32 * 1024 * 1024);
        let json = request_json(EXACT_REQUEST_INLINE_BYTES + 1);
        let hold = match db
            .stage_exact_request_payload_with_capability(
                &session.id,
                "plan-blob",
                &json,
                Some("2030-01-01T00:00:00Z"),
                &capability,
            )
            .expect("stage")
        {
            StageExactPayloadOutcome::Stored(hold) => hold,
            other => panic!("unexpected stage: {other:?}"),
        };
        assert_eq!(hold.storage_kind, ExactPayloadStorageKind::ManagedBlob);
        let blob = managed_blob_path(&capability, &managed_blob_name(&hold.payload_id));
        let stored = std::fs::read(&blob).expect("encrypted blob");
        assert_ne!(stored, json, "managed body must never be plaintext");
        assert_eq!(
            db.read_authorized_exact_request_payload_with_capability(
                &session.id,
                &hold.owner_id,
                &hold.payload_id,
                &capability,
            )
            .expect("read"),
            ExactPayloadRead::Authorized(json)
        );

        db.delete_session(&session.id).expect("delete session");
        db.with_conn_for_test(|conn| {
            let owner_state: String = conn.query_row(
                "SELECT owner_state FROM request_payload_owners WHERE owner_id = ?1",
                params![hold.owner_id],
                |row| row.get(0),
            )?;
            assert_eq!(owner_state, "cleanup_tombstone");
            let object_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM request_payload_objects WHERE payload_id = ?1",
                params![hold.payload_id],
                |row| row.get(0),
            )?;
            assert_eq!(
                object_count, 1,
                "cleanup owner/object must survive session delete"
            );
            Ok(())
        })
        .expect("tombstone");
        let report = db
            .reconcile_exact_request_payloads_with_capability("2030-01-01T00:00:00Z", &capability)
            .expect("reconcile");
        assert_eq!(report.tombstones_claimed, 1);
        assert_eq!(report.payloads_scrubbed, 1);
        assert!(!blob.exists());
    }

    #[test]
    fn quota_and_incognito_memory_are_bounded_without_persistence() {
        let (directory, db) = test_db("quota.db");
        let session = db.create_session("ha-main").expect("session");
        let json = request_json(256);
        let capability = capability(&directory, (json.len() - 1) as u64);
        assert!(matches!(
            db.stage_exact_request_payload_with_capability(
                &session.id,
                "plan-quota",
                &json,
                None,
                &capability,
            )
            .expect("quota stage"),
            StageExactPayloadOutcome::Unavailable(ExactPayloadUnavailable {
                reason: ExactPayloadUnavailableReason::QuotaExceeded,
                ..
            })
        ));

        let mut memory =
            IncognitoExactPayloadStore::new(&session.id, Some(json.len())).expect("memory store");
        let hold = match memory
            .stage(&session.id, "plan-memory", &json)
            .expect("memory stage")
        {
            StageExactPayloadOutcome::Stored(hold) => hold,
            other => panic!("unexpected stage: {other:?}"),
        };
        assert_eq!(hold.storage_kind, ExactPayloadStorageKind::IncognitoMemory);
        assert!(matches!(
            memory
                .stage(&session.id, "plan-memory-2", b"{}")
                .expect("bounded stage"),
            StageExactPayloadOutcome::Unavailable(_)
        ));
        assert!(memory.hold_send_unknown(&hold.owner_id, &hold.payload_id));
        assert_eq!(
            memory.scrub(&hold.owner_id, &hold.payload_id, false),
            RequestExactPayloadScrubOutcome::HeldBySendUnknown
        );
        assert_eq!(
            memory.scrub(&hold.owner_id, &hold.payload_id, true),
            RequestExactPayloadScrubOutcome::Pending
        );
        db.with_conn_for_test(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM request_payload_objects", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(count, 0);
            Ok(())
        })
        .expect("no incognito rows");
    }

    #[test]
    fn crypto_binds_payload_owner_digest_and_rejects_auth_fields() {
        let key = [7_u8; PAYLOAD_KEY_BYTES];
        let body = request_json(32);
        let digest = keyed_payload_digest(&body, &key);
        let payload_id = uuid::Uuid::new_v4().to_string();
        let (nonce, ciphertext) =
            encrypt_payload(&payload_id, "plan-a", &digest, &body, &key).expect("encrypt");
        assert_eq!(
            decrypt_payload(&payload_id, "plan-a", &digest, &nonce, &ciphertext, &key)
                .expect("decrypt"),
            body
        );
        assert!(
            decrypt_payload(&payload_id, "plan-b", &digest, &nonce, &ciphertext, &key).is_err()
        );
        assert!(validate_final_provider_json(br#"{"authorization":"Bearer secret"}"#).is_err());
    }
}
