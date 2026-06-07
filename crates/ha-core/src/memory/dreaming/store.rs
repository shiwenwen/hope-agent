//! Durable store for the Dreaming pipeline (next-gen Phase 0).
//!
//! Persists every cycle as a `dreaming_runs` row (replacing reliance on the
//! process-local `LAST_REPORT` as the source of truth), records a
//! machine-readable decision log, coordinates cross-process runs with a
//! SQLite lease, and keeps a durable pending-source queue with retention GC.
//!
//! All five tables live in `memory.db` (created in
//! [`crate::memory::sqlite::SqliteMemoryBackend::open`]) so future
//! claim/evidence tables can sync transactionally against `memories.id`.
//! This store reuses the backend's single write connection — never opening a
//! second connection to the same file — and treats every operation as
//! best-effort: a durable-layer failure logs a warning but never aborts a
//! dreaming cycle (the diary on disk remains the human-readable record).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};

use crate::memory::SqliteMemoryBackend;

use super::types::{
    DreamReport, DreamRunStatus, DreamingDecisionRecord, DreamingRunDetail, DreamingRunRecord,
    PromotionRecord,
};

/// Floor for the cross-process lease lifetime. A healthy Light cycle with the
/// default narrative timeout finishes well within this.
const LEASE_MIN_TTL_SECS: i64 = 600;

/// Margin added on top of the configured narrative timeout when sizing a
/// lease, to cover the rest of a cycle (agent build, scan, promotion, diary).
const LEASE_BUFFER_SECS: i64 = 300;

/// A `claimed` pending row whose claim is older than this is considered
/// abandoned (the claiming run died) and is returned to `pending`.
const PENDING_CLAIM_STALE_SECS: i64 = 600;

/// `processed` / `skipped` pending rows older than this are garbage-collected.
const PENDING_RETENTION_DAYS: i64 = 30;

/// Lease TTL for a run, sized from the configurable narrative timeout so a
/// healthy cycle can never lose its lease mid-run (which would let another
/// process start a concurrent cycle). The narrative side_query is the only
/// step that scales with config (`narrative_timeout_secs`, which has no upper
/// bound); the buffer covers agent build + scan + promotion + diary. No
/// heartbeat in Phase 0 (Light is bounded by the timeout); heartbeat renewal
/// for unbounded Deep runs lands with the Deep phase.
pub(super) fn lease_ttl_secs(narrative_timeout_secs: u64) -> i64 {
    (narrative_timeout_secs as i64 + LEASE_BUFFER_SECS).max(LEASE_MIN_TTL_SECS)
}

/// Process-wide store handle. Initialised once at startup from the concrete
/// `SqliteMemoryBackend` (see [`init_store`]); `None` in contexts that never
/// opened the memory backend (some tests, minimal ACP), in which case the
/// pipeline falls back to the in-process `DREAMING_RUNNING` guard only.
static DREAMING_STORE: OnceLock<DreamingStore> = OnceLock::new();

/// Stable per-process id used as the lease owner. Lazily minted; survives for
/// the life of the process.
static INSTANCE_ID: OnceLock<String> = OnceLock::new();

fn instance_id() -> &'static str {
    INSTANCE_ID
        .get_or_init(|| uuid::Uuid::new_v4().to_string())
        .as_str()
}

/// Fixed-width UTC RFC3339 (`...T..:..:..SSSZ`). Always 3 fractional digits
/// and a `Z` suffix so the lease / pending / retention comparisons — which
/// are SQL string comparisons (`lease_expires_at < now`) — are lexically
/// monotonic. chrono's default `to_rfc3339()` uses AutoSi (variable 0/3/6/9
/// fractional digits) + `+00:00`, where same-instant values rendered at
/// different precisions don't compare equal lexically; this avoids that.
fn ts(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_rfc3339() -> String {
    ts(chrono::Utc::now())
}

fn rfc3339_in(secs: i64) -> String {
    ts(chrono::Utc::now() + chrono::Duration::seconds(secs))
}

fn rfc3339_ago(secs: i64) -> String {
    ts(chrono::Utc::now() - chrono::Duration::seconds(secs))
}

/// Initialise the global store. Called once during app init with the same
/// concrete backend that backs `MEMORY_BACKEND`. Idempotent.
pub fn init_store(backend: Arc<SqliteMemoryBackend>) {
    let _ = DREAMING_STORE.set(DreamingStore::new(backend));
}

/// Borrow the global store, if initialised.
pub(crate) fn store() -> Option<&'static DreamingStore> {
    DREAMING_STORE.get()
}

// ── Public command API (Tauri / HTTP layers call these) ─────────

/// List durable run records, newest first.
pub fn list_runs(limit: Option<usize>, offset: Option<usize>) -> Result<Vec<DreamingRunRecord>> {
    let store = store().ok_or_else(|| anyhow!("dreaming store not initialised"))?;
    store.list_runs(limit.unwrap_or(50).min(500), offset.unwrap_or(0))
}

/// Fetch a single run plus its decision log.
pub fn get_run(run_id: &str) -> Result<Option<DreamingRunDetail>> {
    let store = store().ok_or_else(|| anyhow!("dreaming store not initialised"))?;
    store.get_run(run_id)
}

// ── Startup recovery + retention ────────────────────────────────

/// Startup recovery (Primary-only). Marks crash-orphaned `running` rows as
/// failed, deletes expired locks, and returns abandoned `claimed` pending
/// rows to `pending`. Best-effort.
pub fn recover_on_startup() {
    let Some(store) = store() else { return };
    let runs = store.recover_stale_runs().unwrap_or(0);
    let locks = store.recover_stale_locks().unwrap_or(0);
    let claimed = store.recover_stale_claimed().unwrap_or(0);
    if runs + locks + claimed > 0 {
        app_info!(
            "memory",
            "dreaming::recover",
            "startup recovery: {} stale run(s) failed, {} expired lock(s) cleared, {} pending reclaimed",
            runs,
            locks,
            claimed
        );
    }
}

/// Spawn the daily retention loop: GC old pending rows + reap expired locks /
/// abandoned claims. Mirrors `async_jobs::spawn_retention_loop`.
pub fn spawn_retention_loop() {
    tokio::spawn(async move {
        // Initial sweep, detached so a slow first pass doesn't delay the loop.
        tokio::task::spawn_blocking(retention_run_once);
        let mut ticker = tokio::time::interval(Duration::from_secs(crate::SECS_PER_DAY));
        ticker.tick().await; // interval fires immediately on first tick; consume it
        loop {
            ticker.tick().await;
            tokio::task::spawn_blocking(retention_run_once);
        }
    });
}

fn retention_run_once() {
    let Some(store) = store() else { return };
    let reclaimed = store.recover_stale_claimed().unwrap_or(0);
    let gced = store.gc_pending().unwrap_or(0);
    let locks = store.recover_stale_locks().unwrap_or(0);
    if reclaimed + gced + locks > 0 {
        app_info!(
            "memory",
            "dreaming::retention",
            "retention sweep: {} pending reclaimed, {} pending purged, {} expired lock(s) cleared",
            reclaimed,
            gced,
            locks
        );
    }
}

// ── Lease guard ─────────────────────────────────────────────────

/// RAII handle releasing the cross-process lease on drop. An `inert` guard
/// (no durable store, or a lease-acquire DB error) does nothing on drop — the
/// in-process `DREAMING_RUNNING` flag still serialises this process.
pub(super) struct LeaseGuard {
    lock_key: String,
    run_id: String,
    active: bool,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(store) = store() {
            if let Err(e) = store.release_lease(&self.lock_key, &self.run_id) {
                app_warn!(
                    "memory",
                    "dreaming::locks",
                    "failed to release lease {}: {}",
                    self.lock_key,
                    e
                );
            }
        }
    }
}

/// Acquire the cross-process lease for `lock_key`.
///
/// - `Some(active guard)` — lease acquired (or stolen from an expired holder).
/// - `Some(inert guard)` — no durable store configured, or a DB error: the
///   cycle proceeds under the in-process flag only.
/// - `None` — another live run holds the lease; the caller must skip.
pub(super) fn acquire_lease(lock_key: &str, run_id: &str, ttl_secs: i64) -> Option<LeaseGuard> {
    let Some(store) = store() else {
        return Some(LeaseGuard {
            lock_key: lock_key.to_string(),
            run_id: run_id.to_string(),
            active: false,
        });
    };
    match store.acquire_lease(lock_key, run_id, ttl_secs) {
        Ok(true) => Some(LeaseGuard {
            lock_key: lock_key.to_string(),
            run_id: run_id.to_string(),
            active: true,
        }),
        Ok(false) => None,
        Err(e) => {
            app_warn!(
                "memory",
                "dreaming::locks",
                "lease acquire failed for {} (proceeding without durable lease): {}",
                lock_key,
                e
            );
            Some(LeaseGuard {
                lock_key: lock_key.to_string(),
                run_id: run_id.to_string(),
                active: false,
            })
        }
    }
}

// ── Store ───────────────────────────────────────────────────────

/// Thin SQL layer over the dreaming tables in `memory.db`. Shares the memory
/// backend's write/read connections (never opens its own).
pub(crate) struct DreamingStore {
    backend: Arc<SqliteMemoryBackend>,
}

impl DreamingStore {
    pub(crate) fn new(backend: Arc<SqliteMemoryBackend>) -> Self {
        Self { backend }
    }

    // ── Runs ────────────────────────────────────────────────────

    /// Insert a `running` run row at cycle start. `ttl_secs` sizes the run's
    /// lease window (same value passed to [`Self::acquire_lease`]).
    pub(crate) fn create_run(
        &self,
        id: &str,
        trigger: &str,
        phase: &str,
        scope_json: &str,
        ttl_secs: i64,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let expires = rfc3339_in(ttl_secs);
        conn.execute(
            "INSERT INTO dreaming_runs
                (id, trigger, phase, status, owner_instance_id, heartbeat_at,
                 lease_expires_at, started_at, scope_json)
             VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?5, ?7)",
            params![id, trigger, phase, instance_id(), now, expires, scope_json],
        )?;
        Ok(())
    }

    /// Finalise a run row, copying the terminal counts from the report.
    /// `decision_count` mirrors `promoted_count` in Phase 0 (only `promote`
    /// decisions are written); they diverge once Deep adds other decisions.
    pub(crate) fn finish_run(
        &self,
        id: &str,
        status: DreamRunStatus,
        report: &DreamReport,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let promoted = report.promoted.len() as i64;
        conn.execute(
            "UPDATE dreaming_runs SET
                status = ?1, finished_at = ?2, heartbeat_at = ?2,
                scanned_count = ?3, nominated_count = ?4,
                decision_count = ?5, promoted_count = ?6,
                duration_ms = ?7, diary_path = ?8, note = ?9
             WHERE id = ?10",
            params![
                status.as_str(),
                now,
                report.candidates_scanned as i64,
                report.candidates_nominated as i64,
                promoted,
                promoted,
                report.duration_ms as i64,
                report.diary_path,
                report.note,
                id,
            ],
        )?;
        Ok(())
    }

    /// Write one `promote` decision per promotion record.
    pub(crate) fn insert_decisions(
        &self,
        run_id: &str,
        promotions: &[PromotionRecord],
    ) -> Result<usize> {
        if promotions.is_empty() {
            return Ok(0);
        }
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let before = serde_json::json!({ "pinned": false }).to_string();
        let mut count = 0usize;
        for p in promotions {
            // Persist provenance in `after_json` so the audit trail + Dashboard
            // can trace each promotion to its source without a dedicated
            // evidence table (Evidence Layer Phase 1).
            let after = serde_json::json!({
                "pinned": true,
                "title": p.title,
                "evidence": p.evidence,
            })
            .to_string();
            conn.execute(
                "INSERT INTO dreaming_decisions
                    (id, run_id, decision_type, target_type, target_id, score,
                     rationale, before_json, after_json, created_at)
                 VALUES (?1, ?2, 'promote', 'memory', ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run_id,
                    p.memory_id.to_string(),
                    p.score as f64,
                    p.rationale,
                    before,
                    after,
                    now,
                ],
            )?;
            count += 1;
        }
        Ok(count)
    }

    /// Write one `dreaming_decisions` row for a Deep resolver mutation on a
    /// claim (`expire` / `merge` / `needs_review`). `merge_into` (the survivor)
    /// is stored in `after_json` for the audit trail.
    pub(crate) fn insert_claim_decision(
        &self,
        run_id: &str,
        decision_type: &str,
        claim_id: &str,
        rationale: &str,
        merge_into: Option<&str>,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let after = merge_into.map(|k| serde_json::json!({ "mergeInto": k }).to_string());
        conn.execute(
            "INSERT INTO dreaming_decisions
                (id, run_id, decision_type, target_type, target_id, score,
                 rationale, before_json, after_json, created_at)
             VALUES (?1, ?2, ?3, 'claim', ?4, NULL, ?5, NULL, ?6, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                decision_type,
                claim_id,
                rationale,
                after,
                now,
            ],
        )?;
        Ok(())
    }

    /// Finalise a Deep resolver run with explicit counts (the resolver has no
    /// promotions, so `finish_run`'s `promoted == decision_count` assumption
    /// doesn't hold).
    pub(crate) fn finish_resolver_run(
        &self,
        id: &str,
        status: DreamRunStatus,
        scanned: usize,
        decisions: usize,
        duration_ms: u64,
        note: Option<&str>,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        conn.execute(
            "UPDATE dreaming_runs SET
                status = ?1, finished_at = ?2, heartbeat_at = ?2,
                scanned_count = ?3, nominated_count = 0,
                decision_count = ?4, promoted_count = 0,
                duration_ms = ?5, note = ?6
             WHERE id = ?7",
            params![
                status.as_str(),
                now,
                scanned as i64,
                decisions as i64,
                duration_ms as i64,
                note,
                id,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn list_runs(&self, limit: usize, offset: usize) -> Result<Vec<DreamingRunRecord>> {
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, trigger, phase, status, started_at, finished_at, duration_ms,
                    scanned_count, nominated_count, promoted_count, decision_count,
                    diary_path, note
             FROM dreaming_runs
             ORDER BY started_at DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], row_to_run_record)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub(crate) fn get_run(&self, run_id: &str) -> Result<Option<DreamingRunDetail>> {
        let conn = self.backend.read_conn()?;
        let run = conn
            .query_row(
                "SELECT id, trigger, phase, status, started_at, finished_at, duration_ms,
                        scanned_count, nominated_count, promoted_count, decision_count,
                        diary_path, note
                 FROM dreaming_runs WHERE id = ?1",
                params![run_id],
                row_to_run_record,
            )
            .optional()?;
        let Some(run) = run else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(
            "SELECT id, decision_type, target_type, target_id, score, rationale,
                    before_json, after_json, created_at
             FROM dreaming_decisions WHERE run_id = ?1
             ORDER BY created_at ASC",
        )?;
        let decisions = stmt
            .query_map(params![run_id], row_to_decision)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(Some(DreamingRunDetail { run, decisions }))
    }

    /// Mark crash-orphaned `running` rows (expired or missing lease) as failed.
    pub(crate) fn recover_stale_runs(&self) -> Result<usize> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let n = conn.execute(
            "UPDATE dreaming_runs
             SET status = 'failed', finished_at = ?1,
                 note = COALESCE(note, 'interrupted before completion')
             WHERE status = 'running'
               AND (lease_expires_at IS NULL OR lease_expires_at < ?1)",
            params![now],
        )?;
        Ok(n)
    }

    // ── Locks (cross-process lease) ──────────────────────────────

    /// Atomically claim `lock_key` for `ttl_secs`. Succeeds when the key is
    /// free or its lease has expired (single-statement upsert; SQLite
    /// serialises the write across processes). Returns `true` when the lease
    /// is now ours.
    pub(crate) fn acquire_lease(
        &self,
        lock_key: &str,
        run_id: &str,
        ttl_secs: i64,
    ) -> Result<bool> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let expires = rfc3339_in(ttl_secs);
        let n = conn.execute(
            "INSERT INTO dreaming_locks
                (lock_key, run_id, owner_instance_id, heartbeat_at, lease_expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(lock_key) DO UPDATE SET
                run_id = excluded.run_id,
                owner_instance_id = excluded.owner_instance_id,
                heartbeat_at = excluded.heartbeat_at,
                lease_expires_at = excluded.lease_expires_at
             WHERE dreaming_locks.lease_expires_at < ?6",
            params![lock_key, run_id, instance_id(), now, expires, now],
        )?;
        Ok(n > 0)
    }

    /// Release a lease we own (no-op if another run already stole it).
    pub(crate) fn release_lease(&self, lock_key: &str, run_id: &str) -> Result<()> {
        let conn = self.backend.write_conn()?;
        conn.execute(
            "DELETE FROM dreaming_locks WHERE lock_key = ?1 AND run_id = ?2",
            params![lock_key, run_id],
        )?;
        Ok(())
    }

    /// Delete expired lock rows (hygiene; acquire already steals expired ones).
    pub(crate) fn recover_stale_locks(&self) -> Result<usize> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let n = conn.execute(
            "DELETE FROM dreaming_locks WHERE lease_expires_at < ?1",
            params![now],
        )?;
        Ok(n)
    }

    // ── Pending sources ──────────────────────────────────────────

    /// Enqueue a source whose capture was deferred (e.g. lease contention).
    pub(crate) fn enqueue_pending(
        &self,
        scope_key: &str,
        source_type: &str,
        source_id: &str,
        source_ts: Option<&str>,
        payload_json: &str,
    ) -> Result<String> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO dreaming_pending_sources
                (id, scope_key, source_type, source_id, source_ts, payload_json,
                 status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?7)",
            params![
                id,
                scope_key,
                source_type,
                source_id,
                source_ts,
                payload_json,
                now
            ],
        )?;
        Ok(id)
    }

    /// Atomically claim up to `limit` pending rows for `scope_key`
    /// (pending → claimed). Returns the claimed ids. `updated_at` doubles as
    /// the claim timestamp for stale-claim recovery.
    pub(crate) fn claim_pending(&self, scope_key: &str, limit: usize) -> Result<Vec<String>> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let mut stmt = conn.prepare(
            "UPDATE dreaming_pending_sources
             SET status = 'claimed', updated_at = ?1
             WHERE id IN (
                SELECT id FROM dreaming_pending_sources
                WHERE scope_key = ?2 AND status = 'pending'
                ORDER BY created_at ASC
                LIMIT ?3
             )
             RETURNING id",
            // (prepared so we can stream RETURNING rows)
        )?;
        let ids = stmt
            .query_map(params![now, scope_key, limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Mark claimed rows as processed in a single all-or-nothing statement —
    /// a mid-batch error can't leave some rows processed and others stuck
    /// claimed (which would otherwise linger until stale-claim recovery).
    pub(crate) fn mark_pending_processed(&self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        // `?1` = now; the id list binds to `?2..?N+1`.
        let placeholders = (2..=ids.len() + 1)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE dreaming_pending_sources
             SET status = 'processed', updated_at = ?1
             WHERE status = 'claimed' AND id IN ({placeholders})"
        );
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(ids.len() + 1);
        bind.push(Box::new(now));
        for id in ids {
            bind.push(Box::new(id.clone()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let n = conn.execute(&sql, refs.as_slice())?;
        Ok(n)
    }

    /// Return abandoned `claimed` rows (claim older than the stale window) to
    /// `pending` so a future run can re-drain them.
    pub(crate) fn recover_stale_claimed(&self) -> Result<usize> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let cutoff = rfc3339_ago(PENDING_CLAIM_STALE_SECS);
        let n = conn.execute(
            "UPDATE dreaming_pending_sources
             SET status = 'pending', updated_at = ?1
             WHERE status = 'claimed' AND updated_at < ?2",
            params![now, cutoff],
        )?;
        Ok(n)
    }

    /// Delete terminal (`processed`/`skipped`) pending rows past retention.
    pub(crate) fn gc_pending(&self) -> Result<usize> {
        let conn = self.backend.write_conn()?;
        let cutoff = rfc3339_ago(PENDING_RETENTION_DAYS * crate::SECS_PER_DAY as i64);
        let n = conn.execute(
            "DELETE FROM dreaming_pending_sources
             WHERE status IN ('processed', 'skipped') AND updated_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    // ── Watermarks ───────────────────────────────────────────────

    pub(crate) fn set_watermark(
        &self,
        scope_key: &str,
        source_type: &str,
        last_source_id: Option<&str>,
        last_source_ts: Option<&str>,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        conn.execute(
            "INSERT INTO dreaming_watermarks
                (scope_key, source_type, last_source_id, last_source_ts, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope_key, source_type) DO UPDATE SET
                last_source_id = excluded.last_source_id,
                last_source_ts = excluded.last_source_ts,
                updated_at = excluded.updated_at",
            params![scope_key, source_type, last_source_id, last_source_ts, now],
        )?;
        Ok(())
    }

    #[allow(dead_code)] // read path lands with the watermark-aware scanner (Phase 1+)
    pub(crate) fn get_watermark(
        &self,
        scope_key: &str,
        source_type: &str,
    ) -> Result<Option<(Option<String>, Option<String>)>> {
        let conn = self.backend.read_conn()?;
        let row = conn
            .query_row(
                "SELECT last_source_id, last_source_ts FROM dreaming_watermarks
                 WHERE scope_key = ?1 AND source_type = ?2",
                params![scope_key, source_type],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        Ok(row)
    }
}

fn row_to_run_record(row: &rusqlite::Row) -> rusqlite::Result<DreamingRunRecord> {
    Ok(DreamingRunRecord {
        id: row.get(0)?,
        trigger: row.get(1)?,
        phase: row.get(2)?,
        status: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        duration_ms: row.get::<_, i64>(6)? as u64,
        candidates_scanned: row.get::<_, i64>(7)? as usize,
        candidates_nominated: row.get::<_, i64>(8)? as usize,
        promoted_count: row.get::<_, i64>(9)? as usize,
        decision_count: row.get::<_, i64>(10)? as usize,
        diary_path: row.get(11)?,
        note: row.get(12)?,
    })
}

fn row_to_decision(row: &rusqlite::Row) -> rusqlite::Result<DreamingDecisionRecord> {
    Ok(DreamingDecisionRecord {
        id: row.get(0)?,
        decision_type: row.get(1)?,
        target_type: row.get(2)?,
        target_id: row.get(3)?,
        score: row.get::<_, Option<f64>>(4)?.map(|v| v as f32),
        rationale: row.get(5)?,
        before_json: row.get(6)?,
        after_json: row.get(7)?,
        created_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::types::EvidenceRef;
    use super::*;
    use crate::memory::dreaming::DreamTrigger;

    fn temp_store() -> DreamingStore {
        // A fresh on-disk DB per test; `open` creates the dreaming_* tables.
        let dir = std::env::temp_dir().join(format!("ha-dreaming-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let backend = SqliteMemoryBackend::open(&dir.join("memory.db")).unwrap();
        DreamingStore::new(Arc::new(backend))
    }

    fn sample_report(run_id: &str, promoted: usize) -> DreamReport {
        DreamReport {
            run_id: Some(run_id.to_string()),
            trigger: DreamTrigger::Manual,
            candidates_scanned: 10,
            candidates_nominated: promoted + 2,
            promoted: (0..promoted)
                .map(|i| PromotionRecord {
                    memory_id: i as i64 + 1,
                    score: 0.9,
                    title: format!("t{i}"),
                    rationale: format!("r{i}"),
                    evidence: vec![EvidenceRef::memory(i as i64 + 1)],
                })
                .collect(),
            diary_path: Some("/tmp/diary.md".to_string()),
            duration_ms: 1234,
            note: None,
        }
    }

    #[test]
    fn run_lifecycle_persists_and_reads_back() {
        let s = temp_store();
        let run_id = "run-1";
        s.create_run(run_id, "manual", "light", "{}", LEASE_MIN_TTL_SECS)
            .unwrap();
        // Mid-flight: visible as `running`.
        let mid = s.get_run(run_id).unwrap().unwrap();
        assert_eq!(mid.run.status, "running");

        let report = sample_report(run_id, 3);
        s.finish_run(run_id, DreamRunStatus::Completed, &report)
            .unwrap();
        s.insert_decisions(run_id, &report.promoted).unwrap();

        let detail = s.get_run(run_id).unwrap().unwrap();
        assert_eq!(detail.run.status, "completed");
        assert_eq!(detail.run.promoted_count, 3);
        assert_eq!(detail.run.decision_count, 3);
        assert_eq!(detail.run.candidates_scanned, 10);
        assert_eq!(detail.decisions.len(), 3);
        assert!(detail
            .decisions
            .iter()
            .all(|d| d.decision_type == "promote"));

        let list = s.list_runs(50, 0).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, run_id);
    }

    #[test]
    fn lease_grants_one_holder_and_recovers_after_expiry() {
        let s = temp_store();
        assert!(s
            .acquire_lease("light:global", "run-a", LEASE_MIN_TTL_SECS)
            .unwrap());
        // Second holder is refused while the lease is live.
        assert!(!s
            .acquire_lease("light:global", "run-b", LEASE_MIN_TTL_SECS)
            .unwrap());
        // Releasing frees it.
        s.release_lease("light:global", "run-a").unwrap();
        assert!(s
            .acquire_lease("light:global", "run-c", LEASE_MIN_TTL_SECS)
            .unwrap());
    }

    #[test]
    fn expired_lease_is_stolen_and_swept() {
        let s = temp_store();
        // Insert a lock that is already expired.
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO dreaming_locks (lock_key, run_id, owner_instance_id, heartbeat_at, lease_expires_at)
                 VALUES ('light:global', 'dead', 'old-instance', ?1, ?1)",
                params![rfc3339_ago(10)],
            )
            .unwrap();
        }
        // A new run steals the expired lease.
        assert!(s
            .acquire_lease("light:global", "fresh", LEASE_MIN_TTL_SECS)
            .unwrap());
        s.release_lease("light:global", "fresh").unwrap();

        // recover_stale_locks deletes expired rows.
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO dreaming_locks (lock_key, run_id, owner_instance_id, heartbeat_at, lease_expires_at)
                 VALUES ('deep:global', 'dead2', 'old', ?1, ?1)",
                params![rfc3339_ago(10)],
            )
            .unwrap();
        }
        assert_eq!(s.recover_stale_locks().unwrap(), 1);
    }

    #[test]
    fn pending_queue_claim_process_and_gc() {
        let s = temp_store();
        s.enqueue_pending("global", "light_rescan", "src-1", None, "{}")
            .unwrap();
        s.enqueue_pending("global", "light_rescan", "src-2", None, "{}")
            .unwrap();

        let claimed = s.claim_pending("global", 10).unwrap();
        assert_eq!(claimed.len(), 2);
        // Re-claim finds nothing (all claimed).
        assert!(s.claim_pending("global", 10).unwrap().is_empty());

        let processed = s.mark_pending_processed(&claimed).unwrap();
        assert_eq!(processed, 2);

        // Fresh `processed` rows are within retention → not GC'd.
        assert_eq!(s.gc_pending().unwrap(), 0);
    }

    #[test]
    fn stale_claimed_pending_is_recovered() {
        let s = temp_store();
        let id = s
            .enqueue_pending("global", "light_rescan", "src", None, "{}")
            .unwrap();
        // Simulate a stale claim (claimed long ago, claiming run died).
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "UPDATE dreaming_pending_sources SET status='claimed', updated_at=?1 WHERE id=?2",
                params![rfc3339_ago(PENDING_CLAIM_STALE_SECS + 60), id],
            )
            .unwrap();
        }
        assert_eq!(s.recover_stale_claimed().unwrap(), 1);
        // Now re-claimable.
        assert_eq!(s.claim_pending("global", 10).unwrap().len(), 1);
    }

    #[test]
    fn lease_ttl_scales_with_narrative_timeout() {
        // Default-ish timeout stays at the floor.
        assert_eq!(lease_ttl_secs(60), LEASE_MIN_TTL_SECS);
        // A timeout longer than the floor minus buffer pushes the TTL above
        // the floor so a healthy long run never loses its lease.
        assert_eq!(lease_ttl_secs(1200), 1200 + LEASE_BUFFER_SECS);
    }

    #[test]
    fn watermark_upsert_roundtrip() {
        let s = temp_store();
        assert!(s.get_watermark("global", "memories").unwrap().is_none());
        s.set_watermark(
            "global",
            "memories",
            Some("42"),
            Some("2026-06-06T00:00:00Z"),
        )
        .unwrap();
        s.set_watermark(
            "global",
            "memories",
            Some("99"),
            Some("2026-06-07T00:00:00Z"),
        )
        .unwrap();
        let wm = s.get_watermark("global", "memories").unwrap().unwrap();
        assert_eq!(wm.0.as_deref(), Some("99"));
        assert_eq!(wm.1.as_deref(), Some("2026-06-07T00:00:00Z"));
    }

    #[test]
    fn recover_stale_runs_fails_orphaned_running_rows() {
        let s = temp_store();
        s.create_run("run-x", "idle", "light", "{}", LEASE_MIN_TTL_SECS)
            .unwrap();
        // Force the lease window into the past so it counts as orphaned.
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "UPDATE dreaming_runs SET lease_expires_at=?1 WHERE id='run-x'",
                params![rfc3339_ago(10)],
            )
            .unwrap();
        }
        assert_eq!(s.recover_stale_runs().unwrap(), 1);
        let r = s.get_run("run-x").unwrap().unwrap();
        assert_eq!(r.run.status, "failed");
    }
}
