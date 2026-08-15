use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
};

use super::db::SessionDB;

/// Durable follow-up state left by a successful Tier 4 emergency recovery or
/// by an exact request that depended on deterministic Tier 0/2 projection.
///
/// `Required` gets one automatic attempt at the next safe main-request
/// boundary. A failed attempt becomes `RetryExhausted` so later turns do not
/// repeatedly spend tokens; manual compaction (or a normal pressure-triggered
/// Tier 3) can still clear the requirement by publishing a successful summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier3RecoveryState {
    Required,
    /// The one automatic summary has been claimed, but no durable summary or
    /// known terminal failure exists yet. A concurrent/restarted main request
    /// must fail closed instead of treating this as an ordinary exhausted
    /// marker and racing the paid summary call.
    InProgress,
    RetryExhausted,
}

/// Why the next safe main request owes one semantic summary. The legacy
/// `reason` column remains fixed for reader-first compatibility; this typed
/// discriminator is the authoritative operational reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier3RecoveryRequirementKind {
    EmergencyOverflow,
    CapacityProjection,
}

impl Tier3RecoveryRequirementKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EmergencyOverflow => "emergency_overflow",
            Self::CapacityProjection => "capacity_projection",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "emergency_overflow" => Ok(Self::EmergencyOverflow),
            "capacity_projection" => Ok(Self::CapacityProjection),
            other => anyhow::bail!("unknown Tier 3 recovery requirement kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier3RecoveryRequirement {
    pub state: Tier3RecoveryState,
    pub kind: Tier3RecoveryRequirementKind,
}

/// Incognito sessions deliberately have no durable recovery row. They still
/// need the recovery -> Tier 3 hand-off while the process/session is alive, so
/// keep the same bounded state machine in process memory. A crash loses
/// this map by design; incognito never promises crash recovery.
const MAX_INCOGNITO_BURN_TOMBSTONES: usize = 4_096;

#[derive(Default)]
struct IncognitoTier3RecoveryState {
    recovery: HashMap<String, IncognitoTier3RecoveryEntry>,
    burned: HashSet<String>,
    tombstones_saturated: bool,
}

#[derive(Debug, Clone)]
struct IncognitoTier3RecoveryEntry {
    requirement: Tier3RecoveryRequirement,
    source_request_plan_id: Option<String>,
}

static INCOGNITO_TIER3_RECOVERY: OnceLock<Mutex<IncognitoTier3RecoveryState>> = OnceLock::new();

fn incognito_tier3_recovery() -> &'static Mutex<IncognitoTier3RecoveryState> {
    INCOGNITO_TIER3_RECOVERY.get_or_init(|| Mutex::new(IncognitoTier3RecoveryState::default()))
}

pub(crate) fn require_incognito_tier3_recovery(session_id: &str) {
    require_incognito_tier3_recovery_with_kind(
        session_id,
        Tier3RecoveryRequirementKind::EmergencyOverflow,
    );
}

pub(crate) fn require_incognito_tier3_after_capacity_projection(
    session_id: &str,
    request_plan_id: &str,
) {
    if session_id.is_empty() || request_plan_id.is_empty() {
        return;
    }
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.tombstones_saturated || state.burned.contains(session_id) {
        return;
    }
    // An already-sent capacity projection owns the debt until a successful
    // or manually retried summary clears it. Reprepare first supersedes (and
    // clears) a provably-unsent owner before installing the replacement plan.
    if state.recovery.contains_key(session_id) {
        return;
    }
    state.recovery.insert(
        session_id.to_string(),
        IncognitoTier3RecoveryEntry {
            requirement: Tier3RecoveryRequirement {
                state: Tier3RecoveryState::Required,
                kind: Tier3RecoveryRequirementKind::CapacityProjection,
            },
            source_request_plan_id: Some(request_plan_id.to_string()),
        },
    );
}

pub(crate) fn clear_incognito_capacity_projection_recovery(
    session_id: &str,
    request_plan_id: &str,
) {
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let should_clear = state.recovery.get(session_id).is_some_and(|entry| {
        entry.requirement.kind == Tier3RecoveryRequirementKind::CapacityProjection
            && entry.source_request_plan_id.as_deref() == Some(request_plan_id)
            && entry.requirement.state == Tier3RecoveryState::Required
    });
    if should_clear {
        state.recovery.remove(session_id);
    }
}

fn require_incognito_tier3_recovery_with_kind(
    session_id: &str,
    kind: Tier3RecoveryRequirementKind,
) {
    if session_id.is_empty() {
        return;
    }
    // Burn and late Tier 4 publication can race. A bounded, fail-closed
    // tombstone set prevents the late writer from recreating in-memory state
    // after cleanup already scrubbed it. Session IDs are never reused.
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.tombstones_saturated || state.burned.contains(session_id) {
        return;
    }
    state.recovery.insert(
        session_id.to_string(),
        IncognitoTier3RecoveryEntry {
            requirement: Tier3RecoveryRequirement {
                state: Tier3RecoveryState::Required,
                kind,
            },
            source_request_plan_id: None,
        },
    );
}

pub(crate) fn incognito_tier3_recovery_requirement(
    session_id: &str,
) -> Option<Tier3RecoveryRequirement> {
    incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .recovery
        .get(session_id)
        .map(|entry| entry.requirement)
}

/// Claim the single automatic incognito follow-up before any paid summary
/// request starts. `InProgress` is intentionally distinct from a known
/// failure so concurrent requests cannot cross the summary publication gap.
pub(crate) fn claim_incognito_tier3_recovery(session_id: &str) -> bool {
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = state.recovery.get_mut(session_id) else {
        return false;
    };
    if entry.requirement.state != Tier3RecoveryState::Required {
        return false;
    }
    entry.requirement.state = Tier3RecoveryState::InProgress;
    true
}

pub(crate) fn exhaust_incognito_tier3_recovery(session_id: &str) -> bool {
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = state.recovery.get_mut(session_id) else {
        return false;
    };
    if entry.requirement.state != Tier3RecoveryState::InProgress {
        return false;
    }
    entry.requirement.state = Tier3RecoveryState::RetryExhausted;
    true
}

pub(crate) fn clear_incognito_tier3_recovery(session_id: &str) {
    incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .recovery
        .remove(session_id);
}

pub(crate) fn purge_incognito_tier3_recovery(session_id: &str) {
    let mut state = incognito_tier3_recovery()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.recovery.remove(session_id);
    if state.burned.len() >= MAX_INCOGNITO_BURN_TOMBSTONES {
        // Never evict a tombstone while stale turn work might still exist.
        // Saturation disables future in-memory recovery markers for this
        // process, preserving privacy at the cost of a manual compact.
        state.tombstones_saturated = true;
        return;
    }
    state.burned.insert(session_id.to_string());
}

/// Context-recovery mutation committed with the provider-native context that
/// proves it. Keeping this typed avoids clearing a Tier 4 follow-up before the
/// winning Tier 3 history, or requiring it before the recovered turn, is
/// durable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tier3RecoveryCommit {
    #[default]
    Unchanged,
    ClearAfterSummary,
    RequireAfterEmergency,
}

impl SessionDB {
    pub(crate) fn ensure_context_compaction_recovery_table(
        conn: &rusqlite::Connection,
    ) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_context_compaction_recovery (
                session_id TEXT PRIMARY KEY,
                tier3_required INTEGER NOT NULL DEFAULT 1
                    CHECK (tier3_required = 1),
                automatic_attempt_exhausted INTEGER NOT NULL DEFAULT 0
                    CHECK (automatic_attempt_exhausted IN (0, 1)),
                automatic_attempt_in_progress INTEGER NOT NULL DEFAULT 0
                    CHECK (automatic_attempt_in_progress IN (0, 1)),
                reason TEXT NOT NULL
                    CHECK (reason = 'history_pressure_after_tier4'),
                requirement_kind TEXT NOT NULL DEFAULT 'emergency_overflow'
                    CHECK (requirement_kind IN ('emergency_overflow', 'capacity_projection')),
                source_request_plan_id TEXT,
                source_canonical_generation INTEGER,
                required_at TEXT NOT NULL,
                last_automatic_attempt_at TEXT,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )?;
        // Reader-first development builds created this pre-release table with
        // only the exhausted bit. Probe instead of requiring a destructive
        // migration for those local databases.
        if conn
            .prepare(
                "SELECT automatic_attempt_in_progress
                   FROM session_context_compaction_recovery LIMIT 0",
            )
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE session_context_compaction_recovery
                 ADD COLUMN automatic_attempt_in_progress INTEGER NOT NULL DEFAULT 0
                 CHECK (automatic_attempt_in_progress IN (0, 1));",
            )?;
        }
        for (column, ddl) in [
            (
                "requirement_kind",
                "ALTER TABLE session_context_compaction_recovery
                 ADD COLUMN requirement_kind TEXT NOT NULL DEFAULT 'emergency_overflow'
                 CHECK (requirement_kind IN ('emergency_overflow', 'capacity_projection'));",
            ),
            (
                "source_request_plan_id",
                "ALTER TABLE session_context_compaction_recovery
                 ADD COLUMN source_request_plan_id TEXT;",
            ),
            (
                "source_canonical_generation",
                "ALTER TABLE session_context_compaction_recovery
                 ADD COLUMN source_canonical_generation INTEGER;",
            ),
        ] {
            if conn
                .prepare(&format!(
                    "SELECT {column} FROM session_context_compaction_recovery LIMIT 0"
                ))
                .is_err()
            {
                conn.execute_batch(ddl)?;
            }
        }
        Ok(())
    }

    /// Read the Tier 3 follow-up state for one durable, non-incognito session.
    /// Missing and incognito sessions deliberately look like no pending state.
    pub fn tier3_recovery_state(&self, session_id: &str) -> Result<Option<Tier3RecoveryState>> {
        Ok(self
            .tier3_recovery_requirement(session_id)?
            .map(|requirement| requirement.state))
    }

    pub fn tier3_recovery_requirement(
        &self,
        session_id: &str,
    ) -> Result<Option<Tier3RecoveryRequirement>> {
        let conn = self.read_conn()?;
        let state = conn
            .query_row(
                "SELECT recovery.automatic_attempt_exhausted,
                        recovery.automatic_attempt_in_progress,
                        recovery.requirement_kind
                   FROM session_context_compaction_recovery recovery
                   JOIN sessions session ON session.id = recovery.session_id
                  WHERE recovery.session_id = ?1
                    AND recovery.tier3_required = 1
                    AND session.incognito = 0",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        state
            .map(|(exhausted, in_progress, kind)| {
                Ok(Tier3RecoveryRequirement {
                    state: if in_progress {
                        Tier3RecoveryState::InProgress
                    } else if exhausted {
                        Tier3RecoveryState::RetryExhausted
                    } else {
                        Tier3RecoveryState::Required
                    },
                    kind: Tier3RecoveryRequirementKind::parse(&kind)?,
                })
            })
            .transpose()
    }

    /// Atomically claim the one automatic follow-up *before* any paid summary
    /// request starts. Crash leaves `InProgress`, which is deliberately
    /// ambiguous and blocks automatic continuation until a manual summary
    /// publishes a winner. A known failure is separately finalized as
    /// `RetryExhausted`.
    pub fn claim_tier3_recovery_attempt(&self, session_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = conn.execute(
            "UPDATE session_context_compaction_recovery
                SET automatic_attempt_in_progress = 1,
                    automatic_attempt_exhausted = 0,
                    last_automatic_attempt_at = ?1,
                    updated_at = ?1
              WHERE session_id = ?2
                AND tier3_required = 1
                AND automatic_attempt_in_progress = 0
                AND automatic_attempt_exhausted = 0
                AND EXISTS (
                    SELECT 1 FROM sessions
                     WHERE id = ?2 AND incognito = 0
                )",
            params![now, session_id],
        )?;
        Ok(changed == 1)
    }

    /// Finalize a claimed automatic summary that returned a known non-success
    /// outcome. A missing row or a non-in-progress row is a state conflict,
    /// not an idempotent success: the caller must stop before Provider IO.
    pub fn exhaust_tier3_recovery_attempt(&self, session_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        Ok(conn.execute(
            "UPDATE session_context_compaction_recovery
                SET automatic_attempt_in_progress = 0,
                    automatic_attempt_exhausted = 1,
                    updated_at = ?1
              WHERE session_id = ?2
                AND tier3_required = 1
                AND automatic_attempt_in_progress = 1",
            params![now, session_id],
        )? == 1)
    }

    /// Clear a pending follow-up after a Tier 3 summary is already
    /// durable. This is mainly used by the manual compaction path; normal chat
    /// completion applies the same mutation inside its assistant transaction.
    pub fn clear_tier3_recovery_after_summary(&self, session_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        Ok(conn.execute(
            "DELETE FROM session_context_compaction_recovery WHERE session_id = ?1",
            params![session_id],
        )? == 1)
    }

    /// Manual-compaction CAS that publishes the summarized provider context
    /// and clears its pending Tier 3 follow-up atomically.
    pub fn save_context_if_unchanged_and_clear_tier3_recovery(
        &self,
        session_id: &str,
        expected_context_json: Option<&str>,
        context_json: &str,
    ) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        let tx = conn.transaction()?;
        let changed = if let Some(expected) = expected_context_json {
            tx.execute(
                "UPDATE sessions
                    SET context_json = ?1,
                        context_revision = context_revision + 1,
                        context_run_id = NULL
                  WHERE id = ?2 AND context_json = ?3",
                params![context_json, session_id, expected],
            )?
        } else {
            tx.execute(
                "UPDATE sessions
                    SET context_json = ?1,
                        context_revision = context_revision + 1,
                        context_run_id = NULL
                  WHERE id = ?2 AND context_json IS NULL",
                params![context_json, session_id],
            )?
        };
        if changed == 1 {
            Self::apply_tier3_recovery_commit(
                &tx,
                session_id,
                Tier3RecoveryCommit::ClearAfterSummary,
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Revision-CAS variant used by a live agent without a stream coordinator.
    /// The winning summary and marker removal must be one SQLite transaction;
    /// a separate DELETE would leave a crash window that charges for the same
    /// forced follow-up again.
    pub fn save_context_at_revision_and_clear_tier3_recovery(
        &self,
        session_id: &str,
        expected_revision: i64,
        context_json: &str,
    ) -> Result<i64> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE sessions
                SET context_json = ?1,
                    context_revision = context_revision + 1,
                    context_run_id = NULL,
                    updated_at = ?2
              WHERE id = ?3 AND context_revision = ?4",
            params![
                context_json,
                chrono::Utc::now().to_rfc3339(),
                session_id,
                expected_revision,
            ],
        )?;
        if changed != 1 {
            anyhow::bail!("context revision conflict for session {session_id}");
        }
        Self::apply_tier3_recovery_commit(&tx, session_id, Tier3RecoveryCommit::ClearAfterSummary)?;
        tx.commit()?;
        Ok(expected_revision.saturating_add(1))
    }

    pub(crate) fn apply_tier3_recovery_commit(
        conn: &rusqlite::Connection,
        session_id: &str,
        commit: Tier3RecoveryCommit,
    ) -> Result<()> {
        match commit {
            Tier3RecoveryCommit::Unchanged => {}
            Tier3RecoveryCommit::ClearAfterSummary => {
                conn.execute(
                    "DELETE FROM session_context_compaction_recovery WHERE session_id = ?1",
                    params![session_id],
                )?;
            }
            Tier3RecoveryCommit::RequireAfterEmergency => {
                let now = chrono::Utc::now().to_rfc3339();
                // INSERT ... SELECT is the incognito boundary: no durable row
                // is created when the owning session is incognito or missing.
                conn.execute(
                    "INSERT INTO session_context_compaction_recovery (
                         session_id, tier3_required, automatic_attempt_exhausted,
                         automatic_attempt_in_progress, reason, required_at,
                         requirement_kind, source_request_plan_id,
                         source_canonical_generation, last_automatic_attempt_at, updated_at
                     )
                     SELECT id, 1, 0, 0, 'history_pressure_after_tier4', ?1,
                            'emergency_overflow', NULL, NULL, NULL, ?1
                       FROM sessions
                      WHERE id = ?2 AND incognito = 0
                     ON CONFLICT(session_id) DO UPDATE SET
                         tier3_required = 1,
                         automatic_attempt_exhausted = 0,
                         automatic_attempt_in_progress = 0,
                         reason = excluded.reason,
                         requirement_kind = excluded.requirement_kind,
                         source_request_plan_id = NULL,
                         source_canonical_generation = NULL,
                         required_at = excluded.required_at,
                         last_automatic_attempt_at = NULL,
                         updated_at = excluded.updated_at",
                    params![now, session_id],
                )?;
            }
        }
        Ok(())
    }

    /// Bind a capacity-projection follow-up to the exact request plan that
    /// depended on it. Callers invoke this inside the same IMMEDIATE
    /// transaction that publishes the request at `ContextCommitted`.
    pub(crate) fn require_tier3_after_capacity_projection_in_tx(
        conn: &rusqlite::Connection,
        session_id: &str,
        request_plan_id: &str,
        canonical_generation: i64,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let changed = conn.execute(
            "INSERT INTO session_context_compaction_recovery (
                 session_id, tier3_required, automatic_attempt_exhausted,
                 automatic_attempt_in_progress, reason, required_at,
                 requirement_kind, source_request_plan_id,
                 source_canonical_generation, last_automatic_attempt_at, updated_at
             )
             SELECT session.id, 1, 0, 0, 'history_pressure_after_tier4', ?1,
                    ?2, ?3, ?4, NULL, ?1
               FROM sessions session
               JOIN request_projection_plans plan
                 ON plan.request_plan_id = ?3
                AND plan.session_id = session.id
                AND plan.state = 'context_committed'
              WHERE session.id = ?5 AND session.incognito = 0
             ON CONFLICT(session_id) DO UPDATE SET
                 tier3_required = 1,
                 automatic_attempt_exhausted = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                       OR (
                           session_context_compaction_recovery.requirement_kind = 'capacity_projection'
                           AND session_context_compaction_recovery.source_canonical_generation
                               IS excluded.source_canonical_generation
                       )
                     THEN session_context_compaction_recovery.automatic_attempt_exhausted
                     ELSE 0
                 END,
                 automatic_attempt_in_progress = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                       OR (
                           session_context_compaction_recovery.requirement_kind = 'capacity_projection'
                           AND session_context_compaction_recovery.source_canonical_generation
                               IS excluded.source_canonical_generation
                       )
                     THEN session_context_compaction_recovery.automatic_attempt_in_progress
                     ELSE 0
                 END,
                 reason = excluded.reason,
                 requirement_kind = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                     THEN session_context_compaction_recovery.requirement_kind
                     ELSE excluded.requirement_kind
                 END,
                 source_request_plan_id = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                       OR session_context_compaction_recovery.source_canonical_generation
                            IS excluded.source_canonical_generation
                     THEN session_context_compaction_recovery.source_request_plan_id
                     ELSE excluded.source_request_plan_id
                 END,
                 source_canonical_generation = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                     THEN session_context_compaction_recovery.source_canonical_generation
                     ELSE excluded.source_canonical_generation
                 END,
                 required_at = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                       OR session_context_compaction_recovery.source_canonical_generation
                            IS excluded.source_canonical_generation
                     THEN session_context_compaction_recovery.required_at
                     ELSE excluded.required_at
                 END,
                 last_automatic_attempt_at = CASE
                     WHEN session_context_compaction_recovery.requirement_kind = 'emergency_overflow'
                       OR session_context_compaction_recovery.source_canonical_generation
                            IS excluded.source_canonical_generation
                     THEN session_context_compaction_recovery.last_automatic_attempt_at
                     ELSE NULL
                 END,
                 updated_at = excluded.updated_at",
            params![
                now,
                Tier3RecoveryRequirementKind::CapacityProjection.as_str(),
                request_plan_id,
                canonical_generation,
                session_id,
            ],
        )?;
        if changed != 1 {
            anyhow::bail!(
                "capacity-projection Tier 3 requirement could not bind to committed request plan"
            );
        }
        Ok(())
    }

    pub(crate) fn clear_capacity_projection_requirement_for_unsent_plan_in_tx(
        conn: &rusqlite::Connection,
        session_id: &str,
        request_plan_id: &str,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM session_context_compaction_recovery
              WHERE session_id = ?1
                AND requirement_kind = 'capacity_projection'
                AND source_request_plan_id = ?2
                AND automatic_attempt_in_progress = 0",
            params![session_id, request_plan_id],
        )?;
        Ok(())
    }
}
