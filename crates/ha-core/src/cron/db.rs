use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Mutex;

use super::schedule::{backoff_delay_ms, compute_next_run, validate_schedule};
use super::types::*;

// ── CronDB (Persistence Layer) ──────────────────────────────────

const CALENDAR_EVENT_WINDOW_MINUTES: i64 = 2;
const MAX_CALENDAR_EVENTS_PER_JOB: usize = 10_000;

/// SQLite-based persistence for cron jobs and run logs.
pub struct CronDB {
    pub(crate) conn: Mutex<Connection>,
}

impl CronDB {
    /// Open (or create) the cron database.
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open cron DB at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                schedule_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                next_run_at TEXT,
                last_run_at TEXT,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                max_failures INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                project_id TEXT,
                job_timeout_secs INTEGER,
                permission_mode_override TEXT,
                sandbox_mode_override TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_cron_jobs_status_next
                ON cron_jobs(status, next_run_at);

            CREATE TABLE IF NOT EXISTS cron_run_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                duration_ms INTEGER,
                result_preview TEXT,
                error TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_cron_runs_job
                ON cron_run_logs(job_id, started_at DESC);

            CREATE INDEX IF NOT EXISTS idx_cron_runs_started
                ON cron_run_logs(started_at DESC);",
        )?;

        // Migration: add running_at column if missing (for existing DBs)
        let has_running_at: bool = conn
            .prepare("SELECT running_at FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_running_at {
            conn.execute_batch("ALTER TABLE cron_jobs ADD COLUMN running_at TEXT;")?;
        }

        // Migration: add notify_on_complete column if missing (for existing DBs)
        let has_notify: bool = conn
            .prepare("SELECT notify_on_complete FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_notify {
            conn.execute_batch(
                "ALTER TABLE cron_jobs ADD COLUMN notify_on_complete INTEGER NOT NULL DEFAULT 1;",
            )?;
        }

        // Migration: add delivery_targets_json column if missing (for existing DBs)
        let has_delivery_targets: bool = conn
            .prepare("SELECT delivery_targets_json FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_delivery_targets {
            conn.execute_batch(
                "ALTER TABLE cron_jobs ADD COLUMN delivery_targets_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }

        // Migration: add project_id column if missing (for existing DBs)
        let has_project_id: bool = conn
            .prepare("SELECT project_id FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_project_id {
            conn.execute_batch("ALTER TABLE cron_jobs ADD COLUMN project_id TEXT;")?;
        }

        // Migration (§8): add prefix_delivery_with_name column if missing
        let has_prefix: bool = conn
            .prepare("SELECT prefix_delivery_with_name FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_prefix {
            conn.execute_batch(
                "ALTER TABLE cron_jobs ADD COLUMN prefix_delivery_with_name INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration (C19): add the per-job job_timeout_secs override column if
        // missing. Nullable — NULL means "use the global CronConfig default".
        let has_job_timeout: bool = conn
            .prepare("SELECT job_timeout_secs FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_job_timeout {
            conn.execute_batch("ALTER TABLE cron_jobs ADD COLUMN job_timeout_secs INTEGER;")?;
        }

        // Migration: add the per-job permission/sandbox override columns if
        // missing. Both nullable — NULL means "follow the agent default".
        let has_permission_override: bool = conn
            .prepare("SELECT permission_mode_override FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_permission_override {
            conn.execute_batch("ALTER TABLE cron_jobs ADD COLUMN permission_mode_override TEXT;")?;
        }
        let has_sandbox_override: bool = conn
            .prepare("SELECT sandbox_mode_override FROM cron_jobs LIMIT 0")
            .is_ok();
        if !has_sandbox_override {
            conn.execute_batch("ALTER TABLE cron_jobs ADD COLUMN sandbox_mode_override TEXT;")?;
        }

        // Migration (§8): add delivery_status column to run logs if missing
        let has_delivery_status: bool = conn
            .prepare("SELECT delivery_status FROM cron_run_logs LIMIT 0")
            .is_ok();
        if !has_delivery_status {
            conn.execute_batch("ALTER TABLE cron_run_logs ADD COLUMN delivery_status TEXT;")?;
        }

        // §9 (C6): tiny key/value table for the scheduler liveness heartbeat, so
        // a startup can tell how long the (Primary-only) scheduler was offline.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_cron_jobs_project
                ON cron_jobs(project_id);",
        )?;

        backfill_every_schedule_start_at(&conn)?;
        backfill_cron_schedule_timezone(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ── Job CRUD ────────────────────────────────────────────────

    /// Create a new job from NewCronJob input. Returns the full CronJob.
    pub fn add_job(&self, input: &NewCronJob) -> Result<CronJob> {
        // Persistence chokepoint: validate the whole schedule via the single
        // source of truth (`schedule::validate_schedule`) so the owner-plane
        // HTTP / Tauri create path enforces the SAME rules as the agent
        // `manage_cron` tool path — no `At` with a bad timestamp, no `Every` that
        // never fires, no unknown cron expression / timezone (which would silently
        // fall back to UTC at fire time).
        validate_schedule(&input.schedule)?;

        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let mut schedule = input.schedule.clone();
        normalize_every_schedule_start_at(&mut schedule, &now);
        let schedule_json = serde_json::to_string(&schedule)?;
        let payload_json = serde_json::to_string(&input.payload)?;
        let max_failures = input.max_failures.unwrap_or(5);

        // Compute initial next_run_at
        let next_run = compute_next_run(&schedule, &now).map(|dt| dt.to_rfc3339());
        // A one-shot `At` whose time is already past has no next run — terminalize
        // it as `missed` at create time instead of persisting a perpetual
        // active/never-fires zombie that only mark_missed_at_jobs would reap on the
        // next restart (§7). Recurring schedules always produce a future next_run.
        let initial_status = if matches!(schedule, CronSchedule::At { .. }) && next_run.is_none() {
            CronJobStatus::Missed
        } else {
            CronJobStatus::Active
        };

        let notify = input.notify_on_complete.unwrap_or(true);
        let prefix_delivery_with_name = input.prefix_delivery_with_name.unwrap_or(false);
        let delivery_targets = input.delivery_targets.clone().unwrap_or_default();
        let delivery_targets_json = serde_json::to_string(&delivery_targets)?;

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_jobs (id, name, description, project_id, schedule_json, payload_json, status, next_run_at, max_failures, notify_on_complete, delivery_targets_json, prefix_delivery_with_name, created_at, updated_at, job_timeout_secs, permission_mode_override, sandbox_mode_override)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?12, ?7, ?8, ?9, ?10, ?13, ?11, ?11, ?14, ?15, ?16)",
            params![
                id,
                input.name,
                input.description,
                normalize_optional_string(input.project_id.as_deref()),
                schedule_json,
                payload_json,
                next_run,
                max_failures,
                notify as i32,
                delivery_targets_json,
                now_str,
                initial_status.as_str(),
                prefix_delivery_with_name as i32,
                input.job_timeout_secs.map(|v| v as i64),
                input.permission_mode_override.map(|m| m.as_str()),
                input.sandbox_mode_override.map(|m| m.as_str())
            ],
        )?;

        Ok(CronJob {
            id,
            name: input.name.clone(),
            description: input.description.clone(),
            project_id: normalize_optional_string(input.project_id.as_deref()),
            schedule,
            payload: input.payload.clone(),
            status: initial_status,
            next_run_at: next_run,
            last_run_at: None,
            running_at: None,
            consecutive_failures: 0,
            max_failures,
            created_at: now_str.clone(),
            updated_at: now_str,
            notify_on_complete: notify,
            delivery_targets,
            prefix_delivery_with_name,
            job_timeout_secs: input.job_timeout_secs,
            permission_mode_override: input.permission_mode_override,
            sandbox_mode_override: input.sandbox_mode_override,
        })
    }

    /// Update an existing job. `status`, `next_run_at`, and `consecutive_failures`
    /// are **system-managed** (toggle / auto-disable / claim / backoff) and read
    /// from the LIVE row rather than taken from the caller's `CronJob` snapshot
    /// (C04): editing a field must not clobber an in-flight backoff offset (the
    /// snapshot's `next_run_at` is stale) nor revive a status the system changed
    /// after the snapshot was taken (e.g. a job auto-disabled while the edit form
    /// was open). Only an edit that actually changes the schedule recomputes
    /// `next_run_at`, and only an *Active* job edited to a past `At` terminalizes
    /// (→ `missed`); a terminal/paused status is never resurrected to active.
    pub fn update_job(&self, job: &CronJob) -> Result<()> {
        // Persistence chokepoint — validate the whole schedule (see `add_job`).
        validate_schedule(&job.schedule)?;

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let mut schedule = job.schedule.clone();
        normalize_every_schedule_start_at(&mut schedule, &now);
        let schedule_json = serde_json::to_string(&schedule)?;
        let payload_json = serde_json::to_string(&job.payload)?;
        let delivery_targets_json = serde_json::to_string(&job.delivery_targets)?;

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

        // C04: read the live system-managed fields — never trust the caller's
        // snapshot of status / next_run_at (it may be stale). Missing row = the job
        // was deleted; preserve the previous silent no-op.
        let (current_status, current_next_run, current_schedule_json): (
            String,
            Option<String>,
            String,
        ) = match conn.query_row(
            "SELECT status, next_run_at, schedule_json FROM cron_jobs WHERE id=?1",
            params![job.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ) {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
            Err(e) => return Err(anyhow::anyhow!("CronDB query error: {e}")),
        };
        let db_status = CronJobStatus::from_str(&current_status);

        // Preserve the current next_run_at unless the schedule actually changed —
        // a non-schedule edit (rename / prompt / targets) keeps its mid-cycle
        // position and any in-flight backoff offset. A schedule change recomputes
        // only for an Active job.
        let next_run =
            if schedule_json != current_schedule_json && db_status == CronJobStatus::Active {
                compute_next_run(&schedule, &now).map(|dt| dt.to_rfc3339())
            } else {
                current_next_run
            };
        // Editing an Active `At` to a past time leaves no next run — terminalize as
        // `missed` (§7, mirrors add_job). Otherwise the LIVE status is preserved, so
        // a disabled / missed / completed / paused job is never resurrected.
        let status = if db_status == CronJobStatus::Active
            && matches!(schedule, CronSchedule::At { .. })
            && next_run.is_none()
        {
            CronJobStatus::Missed
        } else {
            db_status
        };

        conn.execute(
            "UPDATE cron_jobs SET name=?1, description=?2, project_id=?3, schedule_json=?4, payload_json=?5, status=?6, next_run_at=?7, max_failures=?8, notify_on_complete=?9, delivery_targets_json=?10, prefix_delivery_with_name=?13, job_timeout_secs=?14, permission_mode_override=?15, sandbox_mode_override=?16, updated_at=?11
             WHERE id=?12",
            params![
                job.name,
                job.description,
                normalize_optional_string(job.project_id.as_deref()),
                schedule_json,
                payload_json,
                status.as_str(),
                next_run,
                job.max_failures,
                job.notify_on_complete as i32,
                delivery_targets_json,
                now_str,
                job.id,
                job.prefix_delivery_with_name as i32,
                job.job_timeout_secs.map(|v| v as i64),
                job.permission_mode_override.map(|m| m.as_str()),
                job.sandbox_mode_override.map(|m| m.as_str())
            ],
        )?;
        Ok(())
    }

    /// §8: atomically flip `stale` flags on a job's **current** delivery targets,
    /// keyed by `account_id`, under a single lock (read-modify-write). Targets
    /// whose account is in `mark_stale` become stale; those in `clear_stale` are
    /// un-staled. Returns whether anything changed.
    ///
    /// Re-reading the live row (rather than overwriting from a caller-held
    /// snapshot) is the whole point: a cron run can last up to 2h, and the
    /// delivery-time writeback must not clobber a `delivery_targets` edit the
    /// user made via `update_job` mid-run. Only the `stale` field of
    /// account-matched targets is touched; the schedule is never re-validated
    /// (so a since-§6-rejected schedule can't block the flag), and the rest of
    /// the live target list (additions / removals) is preserved verbatim.
    pub fn apply_delivery_target_stale_flags(
        &self,
        job_id: &str,
        mark_stale: &HashSet<String>,
        clear_stale: &HashSet<String>,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let json: String = match conn.query_row(
            "SELECT delivery_targets_json FROM cron_jobs WHERE id=?1",
            params![job_id],
            |r| r.get::<_, String>(0),
        ) {
            Ok(j) => j,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
            Err(e) => return Err(anyhow::anyhow!("CronDB query error: {e}")),
        };
        let mut targets: Vec<CronDeliveryTarget> = serde_json::from_str(&json).unwrap_or_default();
        let mut changed = false;
        for target in &mut targets {
            if mark_stale.contains(&target.account_id) && !target.stale {
                target.stale = true;
                changed = true;
            } else if clear_stale.contains(&target.account_id) && target.stale {
                target.stale = false;
                changed = true;
            }
        }
        if changed {
            let targets_json = serde_json::to_string(&targets)?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE cron_jobs SET delivery_targets_json=?1, updated_at=?2 WHERE id=?3",
                params![targets_json, now, job_id],
            )?;
        }
        Ok(changed)
    }

    /// §8: jobs whose `delivery_targets` reference `account_id`, with the count
    /// of matching targets. Feeds the channel-account delete confirmation
    /// ("N cron jobs reference this account").
    pub fn jobs_referencing_account(&self, account_id: &str) -> Result<Vec<CronAccountRef>> {
        let jobs = self.list_jobs()?;
        let mut refs = Vec::new();
        for job in jobs {
            let count = job
                .delivery_targets
                .iter()
                .filter(|t| t.account_id == account_id)
                .count();
            if count > 0 {
                refs.push(CronAccountRef {
                    job_id: job.id,
                    job_name: job.name,
                    target_count: count,
                });
            }
        }
        Ok(refs)
    }

    /// §8: eagerly flag every delivery target pointing at `account_id` as stale
    /// (the account was just removed). Returns the number of jobs touched.
    /// Idempotent — already-stale targets are not rewritten. The per-job write
    /// goes through the atomic [`Self::apply_delivery_target_stale_flags`] so a
    /// concurrent edit is not clobbered (the candidate scan is only used to find
    /// which jobs to flip).
    pub fn mark_account_delivery_targets_stale(&self, account_id: &str) -> Result<usize> {
        let candidate_ids: Vec<String> = self
            .list_jobs()?
            .into_iter()
            .filter(|job| {
                job.delivery_targets
                    .iter()
                    .any(|t| t.account_id == account_id)
            })
            .map(|job| job.id)
            .collect();
        let mark: HashSet<String> = std::iter::once(account_id.to_string()).collect();
        let empty: HashSet<String> = HashSet::new();
        let mut touched = 0usize;
        for job_id in candidate_ids {
            if self.apply_delivery_target_stale_flags(&job_id, &mark, &empty)? {
                touched += 1;
            }
        }
        Ok(touched)
    }

    /// Clear a job's Project association without changing its schedule.
    pub fn clear_job_project(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET project_id=NULL, updated_at=?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// Delete a job by ID.
    pub fn delete_job(&self, id: &str) -> Result<()> {
        // C15: if the job is mid-run, request cancellation first so the in-flight
        // turn stops promptly (cron's `abort_on_cancel=false` → it returns `Ok("")`
        // → classified Cancelled → no wasted completion, no IM delivery to a
        // now-deleted job) instead of running to the end against a row that's about
        // to vanish. Read-then-cancel is run-keyed by `running_at` (the run's
        // claimed_at), so a stale read can't cancel a later run of a recurring job.
        // The cancelled run's own terminal writes then no-op against the deleted row
        // (its in-progress run_log is CASCADE-removed — an accepted loss for a
        // user-initiated delete). Done before taking the lock since `get_job` locks.
        if let Ok(Some(job)) = self.get_job(id) {
            if let Some(running_at) = job.running_at.as_deref() {
                super::cancel::cancel(id, running_at);
            }
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute("DELETE FROM cron_jobs WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Distinct run-session ids produced by a job. Must be read BEFORE
    /// `delete_job` (which CASCADE-removes `cron_run_logs`), so the caller can
    /// clean up the now-orphaned cron sessions in `sessions.db`.
    pub fn session_ids_for_job(&self, job_id: &str) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt =
            conn.prepare("SELECT DISTINCT session_id FROM cron_run_logs WHERE job_id=?1")?;
        let rows = stmt.query_map(params![job_id], |row| row.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    /// Get a single job by ID.
    pub fn get_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json, project_id, prefix_delivery_with_name, job_timeout_secs, permission_mode_override, sandbox_mode_override
             FROM cron_jobs WHERE id=?1"
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_cron_job(row)?))
        } else {
            Ok(None)
        }
    }

    /// Find the cron job that owns a run session (the most recent run log for
    /// that session). Used by background-job injection delivery (G2): a
    /// background job spawned during a cron run completes later, and its injected
    /// result turn should fan out to the job's `delivery_targets` just like the
    /// inline run did — otherwise it bills a turn delivered to nobody.
    pub fn find_job_by_session(&self, session_id: &str) -> Result<Option<CronJob>> {
        let job_id: Option<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
            match conn.query_row(
                // §10: order by the autoincrement id (not started_at) so the
                // "most recent run log" is deterministic even when two logs share
                // the same started_at second — avoids G2 misrouting the injection.
                "SELECT job_id FROM cron_run_logs WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
                params![session_id],
                |row| row.get::<_, String>(0),
            ) {
                Ok(jid) => Some(jid),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(anyhow::anyhow!("CronDB query error: {e}")),
            }
        };
        match job_id {
            Some(jid) => self.get_job(&jid),
            None => Ok(None),
        }
    }

    /// List all jobs.
    pub fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json, project_id, prefix_delivery_with_name, job_timeout_secs, permission_mode_override, sandbox_mode_override
             FROM cron_jobs ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            row_to_cron_job(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            match row {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    app_warn!("cron", "db", "Skipping corrupted job row: {}", e);
                }
            }
        }
        Ok(jobs)
    }

    /// Get all jobs that are due for execution (status=active, not running, next_run_at <= now).
    pub fn get_due_jobs(&self, now: &DateTime<Utc>) -> Result<Vec<CronJob>> {
        let now_str = now.to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            // ORDER BY next_run_at ASC: §4 claims at most `available_slots` due
            // jobs per pass and defers the rest, so dispatch order is load-bearing
            // — without it SQLite returns rows in arbitrary rowid order and, under
            // sustained cap pressure, the most-overdue job could be skipped every
            // tick (starvation). Most-overdue-first makes the cap fair.
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json, project_id, prefix_delivery_with_name, job_timeout_secs, permission_mode_override, sandbox_mode_override
             FROM cron_jobs WHERE status='active' AND running_at IS NULL AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC"
        )?;
        let rows = stmt.query_map(params![now_str], |row| {
            row_to_cron_job(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    }

    /// Toggle job status between active/paused.
    pub fn toggle_job(&self, id: &str, enabled: bool) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let new_status = if enabled { "active" } else { "paused" };

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

        // If re-enabling, recompute next_run_at
        if enabled {
            // Read current schedule
            let schedule_json: String = conn.query_row(
                "SELECT schedule_json FROM cron_jobs WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            let schedule: CronSchedule = serde_json::from_str(&schedule_json)?;
            let next_run = compute_next_run(&schedule, &Utc::now()).map(|dt| dt.to_rfc3339());
            // C24: re-enabling a one-shot `At` whose time is already past yields no
            // next run — terminalize it as `missed` rather than persisting an
            // active/never-fires zombie (mirrors the §7 handling in add_job /
            // update_job; toggle_job was the one resume path missing it).
            let resumed_status =
                if matches!(schedule, CronSchedule::At { .. }) && next_run.is_none() {
                    "missed"
                } else {
                    new_status
                };
            conn.execute(
                "UPDATE cron_jobs SET status=?1, next_run_at=?2, consecutive_failures=0, updated_at=?3 WHERE id=?4",
                params![resumed_status, next_run, now, id],
            )?;
        } else {
            conn.execute(
                "UPDATE cron_jobs SET status=?1, updated_at=?2 WHERE id=?3",
                params![new_status, now, id],
            )?;
        }
        Ok(())
    }

    /// Update job state after a run (success or failure).
    /// Update job state after a run. Returns `true` iff this failure pushed the
    /// job to the auto-disable threshold (`consecutive_failures >= max_failures`)
    /// and the status was flipped to `disabled` — the caller uses this to fire a
    /// one-shot "job disabled" notification (§5). A success or a non-final
    /// failure returns `false`.
    pub fn update_after_run(
        &self,
        id: &str,
        success: bool,
        schedule: &CronSchedule,
    ) -> Result<bool> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

        if success {
            // Success: reset failures, compute next run
            let (next_status, next_run) = match schedule {
                CronSchedule::At { .. } => ("completed".to_string(), None),
                _ => {
                    let next = compute_next_run(schedule, &now).map(|dt| dt.to_rfc3339());
                    ("active".to_string(), next)
                }
            };
            // Guard on `status='active'`: a scheduled run is only ever claimed
            // while active, but the user can pause (or delete) the job mid-run
            // (toggle_job doesn't cancel the in-flight run). When that run then
            // completes successfully, advancing it back to `active` with a fresh
            // next_run_at would silently undo the explicit pause. The guard makes
            // the success update a no-op for a job no longer active, preserving the
            // user's pause. (Mirrors the failure/auto-disable branch's
            // `status != 'disabled'` guard below; run-now skips this path entirely
            // via `immediate`.)
            conn.execute(
                "UPDATE cron_jobs SET status=?1, next_run_at=?2, last_run_at=?3, consecutive_failures=0, updated_at=?3 WHERE id=?4 AND status='active'",
                params![next_status, next_run, now_str, id],
            )?;
            Ok(false)
        } else {
            // Failure: increment failures, apply backoff
            let (failures,): (u32,) = conn.query_row(
                "SELECT consecutive_failures FROM cron_jobs WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?,)),
            )?;
            let (max_failures,): (u32,) = conn.query_row(
                "SELECT max_failures FROM cron_jobs WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?,)),
            )?;

            let new_failures = failures + 1;

            // §7 / Sweep#1: a one-shot `At` that fails or times out must NOT be
            // retried — its agent turn may have already produced side effects (sent
            // an email, placed an order). Terminalize it as `missed` (the same
            // side-effect-safe terminal a claimed-then-crashed At gets), recording
            // the failure count but never re-firing. Without this, a failed/timed-out
            // At backoff-retries and repeats its side effect up to 1+max_failures
            // times — directly contradicting §7's one-shot side-effect safety.
            // Recurring jobs fall through to the backoff / auto-disable logic below.
            if matches!(schedule, CronSchedule::At { .. }) {
                conn.execute(
                    "UPDATE cron_jobs SET status='missed', next_run_at=NULL, consecutive_failures=?1, last_run_at=?2, updated_at=?2 WHERE id=?3",
                    params![new_failures, now_str, id],
                )?;
                return Ok(false);
            }

            // C26: `max_failures == 0` means "never auto-disable" (unlimited
            // failures), aligning with the `0 = unlimited` convention `max_concurrent`
            // uses. Without this guard `new_failures >= 0` is always true, so a job
            // created via the model tool / HTTP with maxFailures=0 (the GUI's `|| 5`
            // hides this path) would auto-disable on its very first failure.
            if max_failures > 0 && new_failures >= max_failures {
                // Auto-disable. Gate on `status != 'disabled'` so ONLY the
                // active→disabled transition returns true (fires the one-shot
                // notification): the manual run-now path bypasses the status
                // filter (`claim_immediate_job_for_execution` checks only
                // `running_at`), so re-running an already-disabled job that fails
                // again must NOT re-notify or re-bump the count. (A re-run that
                // *succeeds* still re-activates via the success branch above.)
                let rows = conn.execute(
                    "UPDATE cron_jobs SET status='disabled', consecutive_failures=?1, last_run_at=?2, updated_at=?2 WHERE id=?3 AND status != 'disabled'",
                    params![new_failures, now_str, id],
                )?;
                Ok(rows > 0)
            } else {
                // Apply backoff to next run
                let backoff = backoff_delay_ms(new_failures);
                let next_run_base = match schedule {
                    CronSchedule::At { .. } => {
                        // One-shot with failure: retry with backoff
                        now + Duration::milliseconds(backoff as i64)
                    }
                    _ => {
                        let base = compute_next_run(schedule, &now)
                            .unwrap_or(now + Duration::milliseconds(backoff as i64));
                        // Add backoff on top
                        base + Duration::milliseconds(backoff as i64)
                    }
                };
                conn.execute(
                    "UPDATE cron_jobs SET consecutive_failures=?1, next_run_at=?2, last_run_at=?3, updated_at=?3 WHERE id=?4",
                    params![new_failures, next_run_base.to_rfc3339(), now_str, id],
                )?;
                Ok(false)
            }
        }
    }

    /// Review fix (#4): reschedule after an *infrastructure* failure (e.g. the
    /// run's session couldn't be created) WITHOUT bumping `consecutive_failures`.
    /// The agent turn never ran, so a transient infra hiccup must not push a
    /// healthy job toward auto-disable. Recurring jobs advance to their next
    /// occurrence; a one-shot `At` retries shortly (its slot was already cleared
    /// to NULL at claim). Status stays `active`; the failure counter is untouched.
    pub fn reschedule_without_failure(&self, id: &str, schedule: &CronSchedule) -> Result<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let next_run = match schedule {
            // One-shot: retry shortly rather than terminalize (the infra failure
            // is likely transient). Bounded by the fact that a permanently-broken
            // session DB is a whole-app outage, not a per-job problem.
            CronSchedule::At { .. } => Some((now + Duration::seconds(60)).to_rfc3339()),
            _ => compute_next_run(schedule, &now).map(|dt| dt.to_rfc3339()),
        };
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET next_run_at=?1, last_run_at=?2, updated_at=?2 WHERE id=?3",
            params![next_run, now_str, id],
        )?;
        Ok(())
    }

    /// Push the next active run out by a Loop-owned progress backoff. This is a
    /// narrow scheduling override: it never changes the job's schedule, never
    /// revives paused/terminal jobs, and is intended to run after `update_after_run`
    /// has already advanced the recurring schedule to its normal next slot.
    pub fn delay_next_run(&self, id: &str, delay_secs: i64) -> Result<Option<String>> {
        if delay_secs <= 0 {
            return Ok(None);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let next_run = (now + Duration::seconds(delay_secs)).to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let rows = conn.execute(
            "UPDATE cron_jobs
             SET next_run_at = ?1, updated_at = ?2
             WHERE id = ?3 AND status = 'active'",
            params![next_run, now_str, id],
        )?;
        if rows > 0 {
            Ok(Some(next_run))
        } else {
            Ok(None)
        }
    }

    /// §11 review fix: terminalize a cancelled one-shot `At` job as `completed`.
    /// Its `next_run_at` was advanced to NULL at claim, so leaving it `active`
    /// strands an un-fireable zombie until the next restart's `mark_missed_at_jobs`.
    /// It ran (then was cancelled) and won't fire again, so `completed` is the
    /// right terminal. Recurring jobs are never passed here (they keep firing).
    pub fn terminalize_one_shot_completed(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET status='completed', next_run_at=NULL, updated_at=?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    // ── Run Logs ────────────────────────────────────────────────

    /// Add a run log entry. Returns the log ID.
    pub fn add_run_log(&self, log: &CronRunLog) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_run_logs (job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error, delivery_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                log.job_id, log.session_id, log.status, log.started_at,
                log.finished_at, log.duration_ms.map(|v| v as i64), log.result_preview, log.error,
                log.delivery_status
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// §9 (D2): insert an **in-progress** run log at run start — status
    /// `"running"`, `finished_at` NULL. This gives a crashed mid-run a durable
    /// trace: [`recover_orphaned_runs`](Self::recover_orphaned_runs) marks any
    /// such row `error` on the next startup, and the live row drives a real-time
    /// "running" indicator in the UI. Returns the row id to [`finalize_run_log`].
    pub fn add_running_run_log(
        &self,
        job_id: &str,
        session_id: &str,
        started_at: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_run_logs (job_id, session_id, status, started_at, finished_at)
             VALUES (?1, ?2, 'running', ?3, NULL)",
            params![job_id, session_id, started_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// §9 (D2): finalize an in-progress run log to its terminal state (status +
    /// timing + result/error + delivery outcome) in one UPDATE. No-op-safe — a
    /// missing / already-finalized id simply matches no rows.
    #[allow(clippy::too_many_arguments)]
    pub fn finalize_run_log(
        &self,
        run_log_id: i64,
        status: &str,
        finished_at: &str,
        duration_ms: Option<u64>,
        result_preview: Option<&str>,
        error: Option<&str>,
        delivery_status: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_run_logs
             SET status=?1, finished_at=?2, duration_ms=?3, result_preview=?4, error=?5, delivery_status=?6
             WHERE id=?7",
            params![
                status,
                finished_at,
                duration_ms.map(|v| v as i64),
                result_preview,
                error,
                delivery_status,
                run_log_id
            ],
        )?;
        Ok(())
    }

    /// Review fix: finalize the in-progress run log if it was opened
    /// (`Some(id)`), otherwise INSERT a complete terminal row. `add_running_run_log`
    /// can fail on a transient DB error; without this fallback every terminal path
    /// would UPDATE a non-existent `id=0` and the whole run would leave no run-log
    /// at all. One lock acquisition either way (delegates to `finalize_run_log` /
    /// `add_run_log`, each of which locks internally).
    #[allow(clippy::too_many_arguments)]
    pub fn finalize_or_insert_run_log(
        &self,
        run_log_id: Option<i64>,
        job_id: &str,
        session_id: &str,
        started_at: &str,
        status: &str,
        finished_at: &str,
        duration_ms: Option<u64>,
        result_preview: Option<&str>,
        error: Option<&str>,
        delivery_status: Option<&str>,
    ) -> Result<()> {
        match run_log_id {
            Some(id) => self.finalize_run_log(
                id,
                status,
                finished_at,
                duration_ms,
                result_preview,
                error,
                delivery_status,
            ),
            None => {
                let log = CronRunLog {
                    id: 0,
                    job_id: job_id.to_string(),
                    session_id: session_id.to_string(),
                    status: status.to_string(),
                    started_at: started_at.to_string(),
                    finished_at: Some(finished_at.to_string()),
                    duration_ms,
                    result_preview: result_preview.map(|s| s.to_string()),
                    error: error.map(|s| s.to_string()),
                    delivery_status: delivery_status.map(|s| s.to_string()),
                };
                self.add_run_log(&log).map(|_| ())
            }
        }
    }

    /// Get run logs for a job, ordered by most recent first.
    pub fn get_run_logs(
        &self,
        job_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<CronRunLog>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error, delivery_status
             FROM cron_run_logs WHERE job_id=?1 ORDER BY started_at DESC LIMIT ?2 OFFSET ?3"
        )?;
        let rows = stmt.query_map(params![job_id, limit as i64, offset as i64], |row| {
            Ok(CronRunLog {
                id: row.get(0)?,
                job_id: row.get(1)?,
                session_id: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                duration_ms: crate::sql_opt_u64(row, 6)?,
                result_preview: row.get(7)?,
                error: row.get(8)?,
                delivery_status: row.get(9)?,
            })
        })?;
        let mut logs = Vec::new();
        for row in rows {
            logs.push(row?);
        }
        Ok(logs)
    }

    /// Cross-job timeline: every cron run across all jobs, newest-first,
    /// paginated. `title` / `unread_count` are left at their defaults here — the
    /// caller (`cron_run_timeline`) hydrates them from `SessionDB`, a separate
    /// database. LEFT JOIN so a run whose job row was deleted still surfaces,
    /// with `job_name` falling back to `(deleted job)`.
    pub fn list_run_timeline(&self, limit: usize, offset: usize) -> Result<Vec<CronTimelineRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT l.session_id, l.job_id,
                    COALESCE(j.name, '(deleted job)') AS job_name,
                    l.status, l.started_at, l.finished_at, l.result_preview
             FROM cron_run_logs l
             LEFT JOIN cron_jobs j ON j.id = l.job_id
             ORDER BY l.started_at DESC, l.id DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            Ok(CronTimelineRow {
                session_id: row.get(0)?,
                job_id: row.get(1)?,
                job_name: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                result_preview: row.get(6)?,
                title: None,
                unread_count: 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    // ── Calendar Range Query ────────────────────────────────────

    /// Get calendar events for a time range.
    /// Expands recurring schedules into individual events within the range.
    pub fn get_calendar_events(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>> {
        let jobs = self.list_jobs()?;
        let mut events = Vec::new();

        for job in &jobs {
            // Skip completed/missed one-shot jobs outside our interest
            let occurrences = self.compute_occurrences(job, start, end);
            if occurrences.is_empty() {
                continue;
            }

            let run_logs = self.get_run_logs_in_range(
                &job.id,
                &(*start - Duration::minutes(CALENDAR_EVENT_WINDOW_MINUTES)),
                &(*end + Duration::minutes(CALENDAR_EVENT_WINDOW_MINUTES)),
            )?;
            let matched_logs = match_run_logs_to_occurrences(&occurrences, &run_logs)?;

            for (occ, run_log) in occurrences.into_iter().zip(matched_logs) {
                let occ_str = occ.to_rfc3339();

                events.push(CalendarEvent {
                    job_id: job.id.clone(),
                    job_name: job.name.clone(),
                    project_id: job.project_id.clone(),
                    scheduled_at: occ_str,
                    status: job.status.clone(),
                    run_log,
                });
            }
        }

        // Sort by scheduled_at
        events.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at));
        Ok(events)
    }

    /// Compute all occurrence times of a schedule within a range.
    fn compute_occurrences(
        &self,
        job: &CronJob,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Vec<DateTime<Utc>> {
        match &job.schedule {
            CronSchedule::At { timestamp } => {
                if let Some(ts) = super::schedule::parse_flexible_timestamp(timestamp) {
                    if ts >= *start && ts < *end {
                        return vec![ts];
                    }
                }
                vec![]
            }
            CronSchedule::Every {
                interval_ms,
                start_at,
            } => {
                if *interval_ms == 0 || job.status != CronJobStatus::Active {
                    return vec![];
                }

                let interval_ms_u64 = *interval_ms;
                let interval_ms = match i64::try_from(interval_ms_u64) {
                    Ok(v) if v > 0 => v,
                    _ => return vec![],
                };

                let first_run = resolve_every_start_at(job, interval_ms, start_at.as_deref())
                    .unwrap_or_else(|| *start + Duration::milliseconds(interval_ms));
                if first_run >= *end {
                    return vec![];
                }

                let mut results = Vec::new();
                let first_ms = first_run.timestamp_millis();
                let start_ms = start.timestamp_millis();
                let end_ms = end.timestamp_millis();

                let steps_from_start = if start_ms <= first_ms {
                    0
                } else {
                    (start_ms - first_ms + interval_ms - 1).div_euclid(interval_ms)
                };

                let mut current_ms =
                    match first_ms.checked_add(steps_from_start.saturating_mul(interval_ms)) {
                        Some(v) => v,
                        None => return vec![],
                    };

                while current_ms < end_ms && results.len() < MAX_CALENDAR_EVENTS_PER_JOB {
                    if let Some(ts) = DateTime::<Utc>::from_timestamp_millis(current_ms) {
                        results.push(ts);
                    } else {
                        break;
                    }
                    current_ms = match current_ms.checked_add(interval_ms) {
                        Some(v) => v,
                        None => break,
                    };
                }
                results
            }
            CronSchedule::Cron {
                expression,
                timezone,
            } => {
                if let Ok(cron_schedule) = CronExpression::from_str(expression) {
                    let mut results = Vec::new();
                    // Use a time slightly before start to catch events at exactly start
                    let query_start = *start - Duration::seconds(1);
                    // Window + safety-cap filter on a UTC occurrence. Returns false
                    // when iteration should stop. Shared by both branches so the
                    // timezone-aware path and the UTC fallback stay in lock-step
                    // with `compute_next_cron`'s identical interpretation.
                    let take = |next_utc: DateTime<Utc>, out: &mut Vec<DateTime<Utc>>| -> bool {
                        if next_utc >= *end {
                            return false;
                        }
                        if next_utc >= *start {
                            out.push(next_utc);
                        }
                        out.len() < MAX_CALENDAR_EVENTS_PER_JOB
                    };
                    match timezone
                        .as_deref()
                        .and_then(super::schedule::parse_timezone)
                    {
                        // Interpret cron fields as wall-clock in `tz` (DST-aware),
                        // convert each occurrence back to UTC.
                        Some(tz) => {
                            for next in cron_schedule.after(&query_start.with_timezone(&tz)) {
                                if !take(next.with_timezone(&Utc), &mut results) {
                                    break;
                                }
                            }
                        }
                        // No / unknown zone → UTC interpretation (historical).
                        None => {
                            for next in cron_schedule.after(&query_start) {
                                if !take(next, &mut results) {
                                    break;
                                }
                            }
                        }
                    }
                    results
                } else {
                    vec![]
                }
            }
        }
    }

    fn get_run_logs_in_range(
        &self,
        job_id: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<CronRunLog>> {
        let window_start = start.to_rfc3339();
        let window_end = end.to_rfc3339();

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error, delivery_status
             FROM cron_run_logs WHERE job_id=?1 AND started_at >= ?2 AND started_at <= ?3
             ORDER BY started_at ASC"
        )?;
        let rows = stmt.query_map(params![job_id, window_start, window_end], |row| {
            Ok(CronRunLog {
                id: row.get(0)?,
                job_id: row.get(1)?,
                session_id: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                duration_ms: crate::sql_opt_u64(row, 6)?,
                result_preview: row.get(7)?,
                error: row.get(8)?,
                delivery_status: row.get(9)?,
            })
        })?;
        let mut logs = Vec::new();
        for row in rows {
            logs.push(row?);
        }
        Ok(logs)
    }

    // ── Startup Recovery ────────────────────────────────────────

    /// Mark orphaned runs (started but never finished) as error. §9 (D2): now
    /// load-bearing — `add_running_run_log` leaves a `finished_at IS NULL` row
    /// for the duration of every run, so a process that died mid-run is detected
    /// and its run log closed out as `error` on the next startup (runs only at
    /// startup, when nothing is legitimately in flight).
    pub fn recover_orphaned_runs(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let count = conn.execute(
            "UPDATE cron_run_logs SET status='error', error='Interrupted by app shutdown', finished_at=datetime('now')
             WHERE finished_at IS NULL",
            [],
        )?;
        Ok(count)
    }

    /// §9 (C6): record the (Primary-only) scheduler's liveness heartbeat. Called
    /// each scheduler tick; persisted so a later startup can tell how long the
    /// scheduler was offline (see [`last_scheduler_heartbeat`](Self::last_scheduler_heartbeat)).
    pub fn record_scheduler_heartbeat(&self) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_meta (key, value) VALUES ('scheduler_heartbeat', ?1)
             ON CONFLICT(key) DO UPDATE SET value=?1",
            params![now],
        )?;
        Ok(())
    }

    /// §9 (C6): the last recorded scheduler heartbeat, or `None` if never set
    /// (fresh DB / first run).
    pub fn last_scheduler_heartbeat(&self) -> Result<Option<DateTime<Utc>>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let raw: Option<String> = match conn.query_row(
            "SELECT value FROM cron_meta WHERE key='scheduler_heartbeat'",
            [],
            |r| r.get::<_, String>(0),
        ) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(anyhow::anyhow!("CronDB query error: {e}")),
        };
        Ok(raw
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)))
    }

    /// Atomically claim a scheduled due job: set running_at and advance next_run_at.
    /// Returns an execution lease if no one else grabbed it first.
    pub fn claim_scheduled_job_for_execution(
        &self,
        job: &CronJob,
    ) -> Result<Option<ClaimedCronJob>> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

        // Compute next scheduled time
        let next_run = match &job.schedule {
            CronSchedule::At { .. } => None, // one-shot: clear next_run_at
            other => compute_next_run(other, &now).map(|dt| dt.to_rfc3339()),
        };
        let payload_json = serde_json::to_string(&job.payload)?;

        // Atomically claim only the exact payload snapshot that was listed.
        // Agent deletion may rebind payload_json between list and claim; in
        // that case defer to the next scheduler tick instead of executing the
        // stale Agent id while consuming this occurrence.
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=?1, next_run_at=?2, updated_at=?1
             WHERE id=?3 AND next_run_at=?4 AND next_run_at <= ?5
               AND payload_json=?6 AND status='active' AND running_at IS NULL",
            params![
                now_str,
                next_run,
                job.id,
                job.next_run_at,
                now_str,
                payload_json
            ],
        )?;
        Ok((rows > 0).then(|| ClaimedCronJob {
            job: job.clone(),
            claimed_at: now_str,
            immediate: false,
        }))
    }

    /// Atomically claim a job for immediate execution without changing its schedule.
    /// Used by manual run-now entrypoints.
    pub fn claim_immediate_job_for_execution(
        &self,
        job: &CronJob,
    ) -> Result<Option<ClaimedCronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let payload_json = serde_json::to_string(&job.payload)?;
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=?1
             WHERE id=?2 AND payload_json=?3 AND running_at IS NULL",
            params![now, job.id, payload_json],
        )?;
        Ok((rows > 0).then(|| ClaimedCronJob {
            job: job.clone(),
            claimed_at: now,
            immediate: true,
        }))
    }

    /// Clear running_at after job execution completes (called by execute_job).
    pub fn clear_running(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }

    /// Count jobs currently executing (running marker set). The single source of
    /// truth for cron concurrency accounting — covers scheduled, catch-up, and
    /// manual `run now` paths, since all three set `running_at`. Used by the
    /// scheduler's slot-before-claim gate (§4).
    pub fn count_running(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cron_jobs WHERE running_at IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Clear the running marker **only if it still belongs to this run**
    /// (`running_at` equals the value stamped when the job was claimed). Used by
    /// the panic-safe RAII backstop in `execute_claimed_job` so a cron run that
    /// unwinds (rather than reaching one of its normal terminal paths) still
    /// releases its concurrency slot — without disturbing a marker that a *later*
    /// re-claim has since replaced (the timestamp won't match, so it no-ops).
    /// Returns whether a row was actually cleared.
    pub fn clear_running_if_owner(&self, id: &str, expected_running_at: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE id=?1 AND running_at=?2",
            params![id, expected_running_at],
        )?;
        Ok(rows > 0)
    }

    /// Clear all stale running_at markers (for startup recovery after crash).
    pub fn clear_all_running(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let count = conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE running_at IS NOT NULL",
            [],
        )?;
        Ok(count)
    }

    /// Drive un-fireable one-shot `At` jobs to the `missed` terminal state at
    /// startup (§7). Marks an active `At` job `missed` when EITHER:
    /// - it is past its scheduled time by **more than** `grace_secs` (too late to
    ///   late-fire); or
    /// - its `next_run_at IS NULL` — created with a past timestamp
    ///   (`compute_next_run` returned `None`) or **claimed-then-crashed** (the run
    ///   may have partially executed, so it is missed, never re-fired — the
    ///   one-shot side-effect-safety choice).
    ///
    /// Only `running_at IS NULL` jobs are eligible: an `At` job mid-execution also
    /// has `next_run_at IS NULL` (cleared at claim) but is running, not a zombie,
    /// and must not be reaped. Claimed-then-crashed jobs become eligible after
    /// startup's `clear_all_running` resets their stale `running_at`.
    ///
    /// `At` jobs past-due by no more than `grace_secs` are LEFT active so the
    /// startup catch-up can late-fire them (slot-aware via §4's `dispatch_due_jobs`).
    /// `grace_secs = 0` ⇒ strict: any past-due `At` is missed (pre-§7 behavior).
    /// Returns the number of rows marked.
    pub fn mark_missed_at_jobs(&self, grace_secs: u64) -> Result<usize> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cutoff = (now - Duration::seconds(grace_secs as i64)).to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        // `running_at IS NULL` guard (red line): a one-shot `At` that is CURRENTLY
        // EXECUTING has its `next_run_at` cleared to NULL at claim (see
        // `claim_scheduled_job_for_execution`) while `status` stays `active`.
        // Without this guard, the per-tick reap fires while the run is in flight
        // (tick interval 15s, so any run ≳15s spans a tick) and the running job —
        // `status='active'` + `next_run_at IS NULL` — matches the NULL branch and
        // is wrongly marked `missed`; the subsequent successful `update_after_run`
        // is then a no-op (its `status='active'` guard fails) and the job is stuck
        // `missed` despite a `success` run log. Claimed-then-crashed zombies are
        // NOT lost: startup recovery runs `clear_all_running` (resetting their
        // `running_at` to NULL) BEFORE `mark_missed_at_jobs`, so they still match.
        let count = conn.execute(
            "UPDATE cron_jobs SET status='missed', updated_at=?1
             WHERE status='active' AND running_at IS NULL
               AND schedule_json LIKE '%\"type\":\"at\"%'
               AND (next_run_at IS NULL OR next_run_at < ?2)",
            params![now_str, cutoff],
        )?;
        Ok(count)
    }
}

fn normalize_every_schedule_start_at(schedule: &mut CronSchedule, reference_now: &DateTime<Utc>) {
    if let CronSchedule::Every {
        interval_ms,
        start_at,
    } = schedule
    {
        if *interval_ms == 0 {
            return;
        }
        let has_valid_start = start_at
            .as_deref()
            .and_then(super::schedule::parse_flexible_timestamp)
            .is_some();
        if !has_valid_start {
            *start_at =
                Some((*reference_now + Duration::milliseconds(*interval_ms as i64)).to_rfc3339());
        }
    }
}

fn normalize_optional_string(input: Option<&str>) -> Option<String> {
    input.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_every_start_at(
    job: &CronJob,
    interval_ms: i64,
    schedule_start_at: Option<&str>,
) -> Option<DateTime<Utc>> {
    if let Some(ts) = schedule_start_at.and_then(super::schedule::parse_flexible_timestamp) {
        return Some(ts);
    }

    let created_at = super::schedule::parse_flexible_timestamp(&job.created_at)?;
    Some(created_at + Duration::milliseconds(interval_ms))
}

fn match_run_logs_to_occurrences(
    occurrences: &[DateTime<Utc>],
    run_logs: &[CronRunLog],
) -> Result<Vec<Option<CronRunLog>>> {
    let occurrence_ms: Vec<i64> = occurrences
        .iter()
        .map(|occ| occ.timestamp_millis())
        .collect();
    // §10 / review fix: a cron run starts at or AFTER its scheduled occurrence
    // (claim + execution latency, plus the 15s scheduler tick), so match each run
    // log FORWARD to the most recent occurrence at or before it — the slot it
    // fired for. This is correct for any latency up to the next occurrence,
    // unlike the old symmetric ±window which dropped (or mis-assigned) runs that
    // started more than half a gap late — a real problem for dense schedules
    // (second-/sub-minute cron expressions, where half the gap is below the tick
    // latency). A small backward tolerance absorbs clock skew when a log is
    // stamped just before its occurrence. Each log maps to exactly one occurrence,
    // so a log still can't double-count. (occurrences are ascending.)
    const BACKWARD_SKEW_MS: i64 = 60_000;
    let mut assignments: Vec<Option<(CronRunLog, i64)>> = vec![None; occurrences.len()];

    for log in run_logs {
        let Some(log_time) = super::schedule::parse_flexible_timestamp(&log.started_at) else {
            app_warn!(
                "cron",
                "db",
                "Skipping run log {} due to invalid started_at {}",
                log.id,
                log.started_at
            );
            continue;
        };

        let log_ms = log_time.timestamp_millis();

        // Floor occurrence (greatest <= log time). `diff` is the forward latency
        // (>= 0), used only to keep the closest log when several map to one slot.
        let candidate = match occurrence_ms.binary_search(&log_ms) {
            Ok(idx) => Some((idx, 0)),
            // Before the first occurrence: accept only within skew tolerance.
            Err(0) => occurrence_ms
                .first()
                .filter(|first| **first - log_ms <= BACKWARD_SKEW_MS)
                .map(|first| (0usize, (*first - log_ms).abs())),
            Err(idx) => Some((idx - 1, log_ms - occurrence_ms[idx - 1])),
        };

        if let Some((best_idx, diff)) = candidate {
            let replace = match &assignments[best_idx] {
                Some((_, existing_diff)) => diff < *existing_diff,
                None => true,
            };
            if replace {
                assignments[best_idx] = Some((log.clone(), diff));
            }
        }
    }

    Ok(assignments
        .into_iter()
        .map(|entry| entry.map(|(log, _)| log))
        .collect())
}

fn backfill_every_schedule_start_at(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, schedule_json, created_at
         FROM cron_jobs
         WHERE schedule_json LIKE '%\"type\":\"every\"%'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut updates = Vec::new();
    for row in rows {
        let (id, schedule_json, created_at) = row?;
        let mut schedule: CronSchedule = match serde_json::from_str(&schedule_json) {
            Ok(schedule) => schedule,
            Err(e) => {
                app_warn!(
                    "cron",
                    "db",
                    "Skipping every-schedule backfill for job {} due to invalid schedule JSON: {}",
                    id,
                    e
                );
                continue;
            }
        };

        let created_at = match super::schedule::parse_flexible_timestamp(&created_at) {
            Some(ts) => ts,
            None => {
                app_warn!(
                    "cron",
                    "db",
                    "Skipping every-schedule backfill for job {} due to invalid created_at {}",
                    id,
                    created_at
                );
                continue;
            }
        };

        if let CronSchedule::Every {
            interval_ms,
            start_at,
        } = &mut schedule
        {
            let has_valid_start = start_at
                .as_deref()
                .and_then(super::schedule::parse_flexible_timestamp)
                .is_some();
            if has_valid_start {
                continue;
            }
            *start_at =
                Some((created_at + Duration::milliseconds(*interval_ms as i64)).to_rfc3339());
            updates.push((id, serde_json::to_string(&schedule)?));
        }
    }
    drop(stmt);

    for (id, schedule_json) in &updates {
        conn.execute(
            "UPDATE cron_jobs SET schedule_json=?1 WHERE id=?2",
            params![schedule_json, id],
        )?;
    }

    if !updates.is_empty() {
        app_info!(
            "cron",
            "db",
            "Backfilled every schedule start_at for {} existing job(s)",
            updates.len()
        );
    }

    Ok(())
}

/// One-time correctness backfill for `Cron` jobs created before the timezone
/// field was actually honored. The field was persisted but silently dropped at
/// fire time, so every cron job has been firing in UTC. Populate a missing /
/// empty timezone with the host's detected IANA zone and recompute `next_run_at`
/// so the very next fire already lands at the corrected local time (rather than
/// one stale UTC fire later). Mirrors [`backfill_every_schedule_start_at`].
///
/// Idempotent and conservative: rows that already carry a valid zone are
/// skipped, and the whole pass is a no-op when the host zone can't be detected
/// or isn't a known IANA name — we leave such rows on the UTC fallback rather
/// than guess. Runs only at `CronDB::open`, before the scheduler starts, so the
/// `next_run_at` rewrite races nothing.
///
/// The `next_run_at` recompute is gated to clean `status='active' AND
/// consecutive_failures=0` rows, mirroring the status-gating in `update_job` /
/// `toggle_job`: a row mid-backoff has `next_run_at = next_slot + backoff`, and a
/// plain recompute would silently drop the backoff offset. Non-active /
/// in-backoff rows get the timezone fix only; their `next_run_at` is settled by
/// the run loop / re-enable path (which recompute with the corrected zone).
fn backfill_cron_schedule_timezone(conn: &Connection) -> Result<()> {
    // Review fix #8: run ONCE. Only rows present at the upgrade boundary are
    // "legacy" zone-less jobs to migrate. A deliberately zone-less (explicit-UTC)
    // cron job created later — the agent tool documents "Omit for UTC" — must NOT
    // be rewritten to the host zone on a subsequent boot. A sentinel makes this a
    // true one-time migration (and skips the per-boot full-table scan).
    const SENTINEL: &str = "tz_backfill_done";
    let already_done: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cron_meta WHERE key=?1",
        params![SENTINEL],
        |row| row.get(0),
    )?;
    if already_done > 0 {
        return Ok(());
    }

    let Some(host_tz) = iana_time_zone::get_timezone()
        .ok()
        .filter(|tz| super::schedule::parse_timezone(tz).is_some())
    else {
        // Host zone undetectable right now — there's nothing meaningful to
        // migrate, so DON'T set the sentinel: retry on a later boot when the zone
        // may be detectable (the narrow window before that is the only time a
        // legacy row stays UTC-interpreted, matching the pre-fix behavior).
        return Ok(());
    };

    let mut stmt = conn.prepare(
        "SELECT id, schedule_json, status, consecutive_failures
         FROM cron_jobs
         WHERE schedule_json LIKE '%\"type\":\"cron\"%'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let now = Utc::now();
    let mut updates: Vec<(String, String, Option<String>)> = Vec::new();
    for row in rows {
        let (id, schedule_json, status, consecutive_failures) = row?;
        let mut schedule: CronSchedule = match serde_json::from_str(&schedule_json) {
            Ok(schedule) => schedule,
            Err(e) => {
                app_warn!(
                    "cron",
                    "db",
                    "Skipping timezone backfill for job {} due to invalid schedule JSON: {}",
                    id,
                    e
                );
                continue;
            }
        };

        if let CronSchedule::Cron { timezone, .. } = &mut schedule {
            let already_set = timezone
                .as_deref()
                .and_then(super::schedule::parse_timezone)
                .is_some();
            // Only assign the host zone to rows that lack a valid one. A row that
            // already carries a valid zone keeps it.
            if !already_set {
                *timezone = Some(host_tz.clone());
            }
            // Recompute next_run_at for a clean active row REGARDLESS of whether
            // the timezone was just assigned or already valid (Codex review P1):
            // pre-fix, the scheduler computed next_run_at in UTC even when a valid
            // zone was stored (the field was silently dropped at fire time), so an
            // already-zoned job's next_run_at is just as stale as a zone-less one.
            // Skipping it would leave one wrong-time fire before the run loop
            // self-corrects. Only the timezone ASSIGNMENT is gated on !already_set;
            // the recompute is not. Never clobber a backoff offset or touch a
            // paused/disabled row's stamp.
            let next_run = if status == "active" && consecutive_failures == 0 {
                compute_next_run(&schedule, &now).map(|dt| dt.to_rfc3339())
            } else {
                None
            };
            // For an already-zoned row that isn't clean-active there's nothing to
            // write (zone unchanged + next_run None) — skip the no-op UPDATE.
            if !already_set || next_run.is_some() {
                updates.push((id, serde_json::to_string(&schedule)?, next_run));
            }
        }
    }
    drop(stmt);

    for (id, schedule_json, next_run) in &updates {
        match next_run {
            Some(next_run) => conn.execute(
                "UPDATE cron_jobs SET schedule_json=?1, next_run_at=?2 WHERE id=?3",
                params![schedule_json, next_run, id],
            )?,
            // Invalid expression → couldn't compute a next run; still record the
            // corrected timezone, leave next_run_at for the run loop to settle.
            None => conn.execute(
                "UPDATE cron_jobs SET schedule_json=?1 WHERE id=?2",
                params![schedule_json, id],
            )?,
        };
    }

    if !updates.is_empty() {
        app_info!(
            "cron",
            "db",
            "Backfilled timezone ({}) / recomputed next_run_at for {} existing cron job(s)",
            host_tz,
            updates.len()
        );
    }

    // Mark the one-time migration done (host zone was detectable, so all current
    // legacy rows have now been scanned). Later zone-less jobs are deliberate.
    conn.execute(
        "INSERT OR REPLACE INTO cron_meta (key, value) VALUES (?1, ?2)",
        params![SENTINEL, Utc::now().to_rfc3339()],
    )?;

    Ok(())
}

// ── Helper: Row -> CronJob ───────────────────────────────────────

pub(crate) fn row_to_cron_job(row: &rusqlite::Row) -> Result<CronJob> {
    let schedule_json: String = row.get(3)?;
    let payload_json: String = row.get(4)?;
    let status_str: String = row.get(5)?;
    let delivery_targets_json: Option<String> = row.get(14).ok();
    let delivery_targets = delivery_targets_json
        .as_deref()
        .map(
            |s| match serde_json::from_str::<Vec<crate::cron::CronDeliveryTarget>>(s) {
                Ok(v) => v,
                Err(e) => {
                    app_warn!(
                        "cron",
                        "db",
                        "failed to decode delivery_targets_json, treating as empty: {}",
                        e
                    );
                    Vec::new()
                }
            },
        )
        .unwrap_or_default();

    Ok(CronJob {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        project_id: row.get(15).ok().flatten(),
        schedule: serde_json::from_str(&schedule_json)?,
        payload: serde_json::from_str(&payload_json)?,
        status: CronJobStatus::from_str(&status_str),
        next_run_at: row.get(6)?,
        last_run_at: row.get(7)?,
        running_at: row.get(8)?,
        consecutive_failures: row.get(9)?,
        max_failures: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        notify_on_complete: row.get::<_, i32>(13).unwrap_or(1) != 0,
        delivery_targets,
        // Index 16, appended after project_id (15). `.ok()` keeps narrower
        // test SELECTs that stop before this column defaulting to false.
        prefix_delivery_with_name: row.get::<_, i32>(16).ok().map(|v| v != 0).unwrap_or(false),
        // Index 17 (C19), appended after prefix (16). `.ok().flatten()` keeps
        // narrower test SELECTs that stop before this column defaulting to None.
        job_timeout_secs: row
            .get::<_, Option<i64>>(17)
            .ok()
            .flatten()
            .map(|v| v as u64),
        // Index 18 / 19, appended after job_timeout_secs (17). `.ok().flatten()`
        // keeps narrower test SELECTs defaulting to None; unknown strings fall
        // back to the enum default inside `parse_or_default` (None stays None =
        // follow the agent default).
        permission_mode_override: row
            .get::<_, Option<String>>(18)
            .ok()
            .flatten()
            .map(|s| crate::permission::SessionMode::parse_or_default(&s)),
        sandbox_mode_override: row
            .get::<_, Option<String>>(19)
            .ok()
            .flatten()
            .map(|s| crate::permission::SandboxMode::parse_or_default(&s)),
    })
}

#[cfg(test)]
mod tests {
    use super::{match_run_logs_to_occurrences, row_to_cron_job, CronDB};
    use crate::cron::types::CronJobStatus;
    use crate::cron::{CronDeliveryTarget, CronPayload, CronSchedule, NewCronJob};
    use chrono::{DateTime, Utc};
    use rusqlite::params;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hope-agent-cron-{label}-{}.db", Uuid::new_v4()))
    }

    fn cleanup_db_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    fn parse(ts: &str) -> DateTime<Utc> {
        crate::cron::schedule::parse_flexible_timestamp(ts).expect("timestamp")
    }

    #[test]
    fn find_job_by_session_resolves_via_run_log() {
        // G2: a background job spawned during a cron run completes later; its
        // injected result turn must resolve the owning job (from the run log) to
        // fan out to delivery_targets.
        let path = temp_db_path("find-job-by-session");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "Report".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "x".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        db.add_run_log(&crate::cron::CronRunLog {
            id: 0,
            job_id: job.id.clone(),
            session_id: "cron-sess-1".into(),
            status: "success".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            duration_ms: None,
            result_preview: None,
            error: None,
            delivery_status: None,
        })
        .expect("add run log");

        let found = db
            .find_job_by_session("cron-sess-1")
            .expect("query ok")
            .expect("job resolved from session");
        assert_eq!(found.id, job.id);
        assert!(db
            .find_job_by_session("no-such-session")
            .expect("query ok")
            .is_none());

        cleanup_db_files(&path);
    }

    // §8 helpers ───────────────────────────────────────────────────

    fn target(account_id: &str) -> CronDeliveryTarget {
        CronDeliveryTarget {
            channel_id: "telegram".into(),
            account_id: account_id.into(),
            chat_id: "c1".into(),
            thread_id: None,
            label: None,
            stale: false,
        }
    }

    fn every_job(name: &str, targets: Vec<CronDeliveryTarget>, prefix: Option<bool>) -> NewCronJob {
        NewCronJob {
            name: name.into(),
            description: None,
            project_id: None,
            schedule: CronSchedule::Every {
                interval_ms: 300_000,
                start_at: None,
            },
            payload: CronPayload::AgentTurn {
                prompt: "x".into(),
                agent_id: None,
            },
            max_failures: None,
            notify_on_complete: None,
            delivery_targets: Some(targets),
            prefix_delivery_with_name: prefix,
            job_timeout_secs: None,
            permission_mode_override: None,
            sandbox_mode_override: None,
        }
    }

    #[test]
    fn job_timeout_secs_override_persists_and_clears() {
        // C19: the per-job timeout override round-trips through add_job / get_job /
        // update_job; `None` falls back to the global CronConfig default.
        let path = temp_db_path("job-timeout-override");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "long".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: Some(1800),
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        assert_eq!(job.job_timeout_secs, Some(1800));
        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(stored.job_timeout_secs, Some(1800), "override persists");

        // Clear it back to the global default.
        let mut cleared = stored.clone();
        cleared.job_timeout_secs = None;
        db.update_job(&cleared).expect("update");
        let after = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(after.job_timeout_secs, None, "override cleared on update");
        cleanup_db_files(&path);
    }

    #[test]
    fn permission_and_sandbox_overrides_persist_and_clear() {
        // The per-job permission/sandbox overrides round-trip through add_job /
        // get_job / update_job; `None` = follow the agent default.
        use crate::permission::{SandboxMode, SessionMode};
        let path = temp_db_path("perm-sandbox-override");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "guarded".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "clean temp".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: Some(SessionMode::Smart),
                sandbox_mode_override: Some(SandboxMode::Isolated),
            })
            .expect("add job");
        assert_eq!(job.permission_mode_override, Some(SessionMode::Smart));
        assert_eq!(job.sandbox_mode_override, Some(SandboxMode::Isolated));

        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(
            stored.permission_mode_override,
            Some(SessionMode::Smart),
            "permission override persists"
        );
        assert_eq!(
            stored.sandbox_mode_override,
            Some(SandboxMode::Isolated),
            "sandbox override persists"
        );

        // Clear both back to "follow agent default".
        let mut cleared = stored.clone();
        cleared.permission_mode_override = None;
        cleared.sandbox_mode_override = None;
        db.update_job(&cleared).expect("update");
        let after = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(after.permission_mode_override, None, "permission cleared");
        assert_eq!(after.sandbox_mode_override, None, "sandbox cleared");
        cleanup_db_files(&path);
    }

    #[test]
    fn update_job_preserves_backoff_and_does_not_revive_disabled() {
        // C04: update_job treats status / next_run_at / consecutive_failures as
        // system-managed (read from the live row), so editing a field neither
        // clobbers an in-flight backoff offset nor revives a status the system
        // changed after the caller took its (now stale) snapshot.
        let path = temp_db_path("update-job-c04");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "j".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        // The system backed the job off AND disabled it AFTER a client loaded its
        // (now stale: active + original next_run_at) snapshot.
        let backoff_next = "2099-01-01T00:00:00+00:00".to_string();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "UPDATE cron_jobs SET status='disabled', consecutive_failures=4, next_run_at=?1 WHERE id=?2",
                params![backoff_next, job.id],
            )
            .expect("seed disabled+backoff");
        }

        // The caller edits only the name; its snapshot still says active with the
        // original next_run_at.
        let mut edit = job.clone();
        edit.name = "renamed".into();
        db.update_job(&edit).expect("update");

        let after = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(after.name, "renamed", "the edited field is applied");
        assert_eq!(
            after.status,
            CronJobStatus::Disabled,
            "must NOT revive the disabled status from the stale snapshot"
        );
        assert_eq!(
            after.next_run_at.as_deref(),
            Some(backoff_next.as_str()),
            "in-flight backoff next_run_at preserved (schedule unchanged)"
        );
        assert_eq!(
            after.consecutive_failures, 4,
            "failure count preserved (system-managed)"
        );
        cleanup_db_files(&path);
    }

    #[test]
    fn prefix_delivery_with_name_round_trips() {
        let path = temp_db_path("prefix-roundtrip");
        let db = CronDB::open(&path).expect("open db");

        // Default (None) persists as false.
        let def = db
            .add_job(&every_job("default", vec![], None))
            .expect("add");
        assert!(!def.prefix_delivery_with_name);
        assert!(
            !db.get_job(&def.id)
                .expect("get")
                .expect("exists")
                .prefix_delivery_with_name
        );

        // Opt-in persists and survives update.
        let on = db
            .add_job(&every_job("on", vec![], Some(true)))
            .expect("add");
        assert!(on.prefix_delivery_with_name);
        let mut reloaded = db.get_job(&on.id).expect("get").expect("exists");
        assert!(reloaded.prefix_delivery_with_name);
        reloaded.prefix_delivery_with_name = false;
        db.update_job(&reloaded).expect("update");
        assert!(
            !db.get_job(&on.id)
                .expect("get")
                .expect("exists")
                .prefix_delivery_with_name
        );

        cleanup_db_files(&path);
    }

    #[test]
    fn in_progress_run_log_finalizes_with_delivery_status() {
        // §9 (D2) lifecycle: open an in-progress row at run start, finalize it on
        // terminal carrying status + timing + delivery outcome (§8) in one update.
        let path = temp_db_path("delivery-status");
        let db = CronDB::open(&path).expect("open db");
        let job = db.add_job(&every_job("job", vec![], None)).expect("add");

        let id = db
            .add_running_run_log(&job.id, "s1", "2026-01-01T00:00:00Z")
            .expect("open in-progress run log");
        // Mid-run the row reads as "running" with no finish.
        let running = db.get_run_logs(&job.id, 10, 0).expect("logs");
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, "running");
        assert!(running[0].finished_at.is_none());

        db.finalize_run_log(
            id,
            "success",
            "2026-01-01T00:00:01Z",
            Some(1000),
            Some("ok"),
            None,
            Some("partial"),
        )
        .expect("finalize");

        let logs = db.get_run_logs(&job.id, 10, 0).expect("logs");
        assert_eq!(logs.len(), 1, "finalize updates the same row, no duplicate");
        assert_eq!(logs[0].status, "success");
        assert_eq!(logs[0].delivery_status.as_deref(), Some("partial"));
        assert_eq!(logs[0].result_preview.as_deref(), Some("ok"));
        assert!(logs[0].finished_at.is_some());

        cleanup_db_files(&path);
    }

    #[test]
    fn recover_orphaned_closes_in_progress_run_log_as_error() {
        // §9 (D2): a run that crashed mid-flight leaves a finished_at-NULL row;
        // the next startup's recover_orphaned_runs closes it out as error.
        let path = temp_db_path("recover-orphaned");
        let db = CronDB::open(&path).expect("open db");
        let job = db.add_job(&every_job("job", vec![], None)).expect("add");
        db.add_running_run_log(&job.id, "s1", "2026-01-01T00:00:00Z")
            .expect("open in-progress");

        let recovered = db.recover_orphaned_runs().expect("recover");
        assert_eq!(recovered, 1);
        let logs = db.get_run_logs(&job.id, 10, 0).expect("logs");
        assert_eq!(logs[0].status, "error");
        assert!(logs[0].finished_at.is_some());

        cleanup_db_files(&path);
    }

    #[test]
    fn scheduler_heartbeat_round_trips() {
        // §9 (C6): heartbeat starts unset, then records + reads back.
        let path = temp_db_path("heartbeat");
        let db = CronDB::open(&path).expect("open db");
        assert!(db.last_scheduler_heartbeat().expect("read").is_none());
        db.record_scheduler_heartbeat().expect("record");
        assert!(
            db.last_scheduler_heartbeat().expect("read").is_some(),
            "heartbeat persists and parses back"
        );
        cleanup_db_files(&path);
    }

    #[test]
    fn at_schedule_serializes_with_type_at_tag() {
        // §10: mark_missed_at_jobs filters with `schedule_json LIKE '%"type":"at"%'`.
        // Lock the serde tag so a future rename can't silently break that query
        // (un-missing every overdue one-shot At job).
        let json = serde_json::to_string(&CronSchedule::At {
            timestamp: "2026-01-01T00:00:00Z".into(),
        })
        .expect("serialize");
        assert!(
            json.contains(r#""type":"at""#),
            "At must serialize with the `type:at` tag mark_missed_at_jobs greps for — got {json}"
        );
    }

    #[test]
    fn jobs_referencing_account_and_mark_stale() {
        let path = temp_db_path("account-refs");
        let db = CronDB::open(&path).expect("open db");

        // Job A: two targets on acc-1 + one on acc-2. Job B: one on acc-2 only.
        let a = db
            .add_job(&every_job(
                "A",
                vec![target("acc-1"), target("acc-1"), target("acc-2")],
                None,
            ))
            .expect("add A");
        let _b = db
            .add_job(&every_job("B", vec![target("acc-2")], None))
            .expect("add B");

        // Reverse scan: acc-1 referenced only by A (2 targets); acc-2 by both.
        let refs1 = db.jobs_referencing_account("acc-1").expect("scan acc-1");
        assert_eq!(refs1.len(), 1);
        assert_eq!(refs1[0].job_id, a.id);
        assert_eq!(refs1[0].target_count, 2);
        assert_eq!(db.jobs_referencing_account("acc-2").expect("scan").len(), 2);
        assert!(db
            .jobs_referencing_account("ghost")
            .expect("scan")
            .is_empty());

        // Mark acc-1 stale: only job A touched; acc-2 targets untouched.
        assert_eq!(
            db.mark_account_delivery_targets_stale("acc-1")
                .expect("mark"),
            1
        );
        let a_reloaded = db.get_job(&a.id).expect("get").expect("exists");
        let acc1_stale = a_reloaded
            .delivery_targets
            .iter()
            .filter(|t| t.account_id == "acc-1")
            .all(|t| t.stale);
        let acc2_fresh = a_reloaded
            .delivery_targets
            .iter()
            .filter(|t| t.account_id == "acc-2")
            .all(|t| !t.stale);
        assert!(acc1_stale, "acc-1 targets should be stale");
        assert!(acc2_fresh, "acc-2 target should stay fresh");

        // Idempotent: a second mark touches nothing.
        assert_eq!(
            db.mark_account_delivery_targets_stale("acc-1")
                .expect("mark"),
            0
        );

        cleanup_db_files(&path);
    }

    #[test]
    fn apply_stale_flags_reads_live_targets_not_caller_snapshot() {
        // Regression guard for the §8 review finding: the post-run stale
        // writeback must re-read the live target list, not overwrite from a
        // claim-time snapshot — otherwise a delivery-target edit the user made
        // mid-run is silently reverted.
        let path = temp_db_path("apply-stale-live");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&every_job(
                "J",
                vec![target("acc-1"), target("acc-2")],
                None,
            ))
            .expect("add");

        // Simulate a concurrent edit DURING a long run: drop acc-2, add acc-3,
        // change acc-1's chat. A claim-time snapshot still says [acc-1, acc-2].
        let mut edited = db.get_job(&job.id).expect("get").expect("exists");
        edited.delivery_targets = vec![
            CronDeliveryTarget {
                chat_id: "c-new".into(),
                ..target("acc-1")
            },
            target("acc-3"),
        ];
        db.update_job(&edited).expect("concurrent edit");

        // Post-run delivery flips stale on acc-1 (missing account).
        let acc1: std::collections::HashSet<String> =
            std::iter::once("acc-1".to_string()).collect();
        let empty = std::collections::HashSet::new();
        assert!(db
            .apply_delivery_target_stale_flags(&job.id, &acc1, &empty)
            .expect("apply"));

        let reloaded = db.get_job(&job.id).expect("get").expect("exists");
        // The user's edit survived: acc-3 kept, acc-2 NOT resurrected, acc-1 chat preserved.
        let accounts: Vec<&str> = reloaded
            .delivery_targets
            .iter()
            .map(|t| t.account_id.as_str())
            .collect();
        assert_eq!(accounts, vec!["acc-1", "acc-3"]);
        let t1 = reloaded
            .delivery_targets
            .iter()
            .find(|t| t.account_id == "acc-1")
            .expect("acc-1 present");
        assert_eq!(t1.chat_id, "c-new", "user's chat edit preserved");
        assert!(t1.stale, "acc-1 marked stale");
        assert!(
            !reloaded
                .delivery_targets
                .iter()
                .find(|t| t.account_id == "acc-3")
                .expect("acc-3 present")
                .stale,
            "unrelated target untouched"
        );

        // Clear path + idempotency + missing-row safety.
        assert!(db
            .apply_delivery_target_stale_flags(&job.id, &empty, &acc1)
            .expect("clear"));
        assert!(!db
            .apply_delivery_target_stale_flags(&job.id, &empty, &acc1)
            .expect("clear no-op"));
        assert!(!db
            .apply_delivery_target_stale_flags("ghost", &acc1, &empty)
            .expect("missing row → false"));

        cleanup_db_files(&path);
    }

    #[test]
    fn add_job_assigns_every_start_at_and_next_run() {
        let path = temp_db_path("add-job");
        let db = CronDB::open(&path).expect("open db");

        let job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "drink water".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        match &job.schedule {
            CronSchedule::Every {
                interval_ms,
                start_at: Some(start_at),
            } => {
                assert_eq!(*interval_ms, 300_000);
                let created_at = parse(&job.created_at);
                let scheduled_start = parse(start_at);
                assert_eq!(
                    scheduled_start,
                    created_at + chrono::Duration::milliseconds(*interval_ms as i64)
                );
                assert_eq!(job.next_run_at.as_deref(), Some(start_at.as_str()));
            }
            other => panic!("unexpected schedule: {other:?}"),
        }

        cleanup_db_files(&path);
    }

    #[test]
    fn job_project_id_persists_updates_and_clears() {
        let path = temp_db_path("project-id");
        let db = CronDB::open(&path).expect("open db");

        let mut job = db
            .add_job(&NewCronJob {
                name: "Project digest".into(),
                description: None,
                project_id: Some("project-a".into()),
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "summarize project".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        assert_eq!(job.project_id.as_deref(), Some("project-a"));
        let stored = db.get_job(&job.id).expect("load").expect("job exists");
        assert_eq!(stored.project_id.as_deref(), Some("project-a"));

        job.project_id = Some("project-b".into());
        db.update_job(&job).expect("update");
        let updated = db.get_job(&job.id).expect("load").expect("job exists");
        assert_eq!(updated.project_id.as_deref(), Some("project-b"));

        db.clear_job_project(&job.id).expect("clear project");
        let cleared = db.get_job(&job.id).expect("load").expect("job exists");
        assert_eq!(cleared.project_id, None);

        cleanup_db_files(&path);
    }

    #[test]
    fn open_migrates_legacy_schema_before_creating_project_index() {
        let path = temp_db_path("legacy-project-id");
        {
            let conn = rusqlite::Connection::open(&path).expect("open raw db");
            conn.execute_batch(
                "CREATE TABLE cron_jobs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    schedule_json TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'active',
                    next_run_at TEXT,
                    last_run_at TEXT,
                    consecutive_failures INTEGER NOT NULL DEFAULT 0,
                    max_failures INTEGER NOT NULL DEFAULT 5,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );",
            )
            .expect("create legacy cron_jobs");
        }

        let db = CronDB::open(&path).expect("open legacy db");
        let conn = db.conn.lock().expect("lock");
        conn.prepare("SELECT project_id FROM cron_jobs LIMIT 0")
            .expect("project_id column migrated");
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_cron_jobs_project'",
                [],
                |row| row.get(0),
            )
            .expect("query project index");
        assert_eq!(index_count, 1);

        drop(conn);
        cleanup_db_files(&path);
    }

    #[test]
    fn open_backfills_legacy_every_schedule_and_calendar_uses_it() {
        let path = temp_db_path("legacy-backfill");
        let db = CronDB::open(&path).expect("open db");

        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO cron_jobs (
                    id, name, description, schedule_json, payload_json, status,
                    next_run_at, last_run_at, running_at, consecutive_failures, max_failures,
                    notify_on_complete, delivery_targets_json, created_at, updated_at
                ) VALUES (
                    ?1, ?2, NULL, ?3, ?4, 'active',
                    ?5, NULL, NULL, 0, 5,
                    1, '[]', ?6, ?6
                )",
                params![
                    "legacy-water",
                    "喝水提醒",
                    r#"{"type":"every","interval_ms":300000}"#,
                    r#"{"type":"agentTurn","prompt":"drink water","agentId":null}"#,
                    "2026-04-22T12:20:11Z",
                    "2026-04-22T12:10:11Z",
                ],
            )
            .expect("insert legacy job");
        }
        drop(db);

        let reopened = CronDB::open(&path).expect("reopen db");
        let legacy_job = reopened
            .get_job("legacy-water")
            .expect("load job")
            .expect("job exists");
        let start_at = match legacy_job.schedule {
            CronSchedule::Every {
                start_at: Some(start_at),
                ..
            } => start_at,
            other => panic!("unexpected schedule: {other:?}"),
        };
        assert_eq!(parse(&start_at), parse("2026-04-22T12:15:11Z"));

        let events = reopened
            .get_calendar_events(
                &parse("2026-03-31T16:00:00Z"),
                &parse("2026-04-30T16:00:00Z"),
            )
            .expect("calendar events");
        let first = events.first().expect("first occurrence");
        assert_eq!(first.job_id, "legacy-water");
        assert_eq!(parse(&first.scheduled_at), parse("2026-04-22T12:15:11Z"));

        cleanup_db_files(&path);
    }

    #[test]
    fn tz_backfill_recomputes_next_run_for_already_zoned_clean_active_rows() {
        // Codex review P1: a cron job that already stored a valid (non-UTC) zone
        // BEFORE the timezone was honored still had its next_run_at computed in
        // UTC. The backfill must recompute next_run_at for such clean-active rows
        // (only the timezone ASSIGNMENT is skipped when a valid zone is present),
        // and must NOT touch an in-backoff / paused row's stamp.
        let path = temp_db_path("tz-backfill-recompute");
        let db = CronDB::open(&path).expect("open db");
        {
            let conn = db.conn.lock().expect("lock");
            // First open set the one-time sentinel; clear it so the backfill re-runs.
            conn.execute("DELETE FROM cron_meta WHERE key='tz_backfill_done'", [])
                .expect("clear sentinel");
            let insert = |id: &str, status: &str, failures: i64| {
                conn.execute(
                    "INSERT INTO cron_jobs (
                        id, name, description, schedule_json, payload_json, status,
                        next_run_at, last_run_at, running_at, consecutive_failures, max_failures,
                        notify_on_complete, delivery_targets_json, created_at, updated_at
                    ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, NULL, NULL, ?7, 5, 1, '[]', ?8, ?8)",
                    params![
                        id,
                        id,
                        r#"{"type":"cron","expression":"0 0 9 * * *","timezone":"Asia/Shanghai"}"#,
                        r#"{"type":"agentTurn","prompt":"p","agentId":null}"#,
                        status,
                        "2020-01-01T00:00:00Z", // deliberately stale UTC-era next_run_at
                        failures,
                        "2026-01-01T00:00:00Z",
                    ],
                )
                .expect("insert");
            };
            insert("zoned-clean", "active", 0); // clean active → must recompute
            insert("zoned-backoff", "active", 2); // mid-backoff → must NOT touch
            insert("zoned-paused", "paused", 0); // paused → must NOT touch

            super::backfill_cron_schedule_timezone(&conn).expect("backfill");

            // Only assert when the pass actually ran (host zone detectable → sentinel
            // re-written). On a host whose zone can't be detected the pass is a no-op
            // by design; don't false-fail there.
            let ran: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM cron_meta WHERE key='tz_backfill_done'",
                    [],
                    |r| r.get(0),
                )
                .expect("sentinel count");
            if ran > 0 {
                let next_of = |id: &str| -> Option<String> {
                    conn.query_row(
                        "SELECT next_run_at FROM cron_jobs WHERE id=?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .expect("read next_run_at")
                };
                assert_ne!(
                    next_of("zoned-clean").as_deref(),
                    Some("2020-01-01T00:00:00Z"),
                    "clean active already-zoned row must have next_run_at recomputed (P1)"
                );
                assert_eq!(
                    next_of("zoned-backoff").as_deref(),
                    Some("2020-01-01T00:00:00Z"),
                    "in-backoff row's next_run_at must be preserved"
                );
                assert_eq!(
                    next_of("zoned-paused").as_deref(),
                    Some("2020-01-01T00:00:00Z"),
                    "paused row's next_run_at must be preserved"
                );
            }
        }
        drop(db);
        cleanup_db_files(&path);
    }

    #[test]
    fn run_logs_match_nearest_occurrence_once() {
        let matches = match_run_logs_to_occurrences(
            &[
                parse("2026-04-22T12:00:00Z"),
                parse("2026-04-22T12:01:00Z"),
                parse("2026-04-22T12:02:00Z"),
            ],
            &[crate::cron::CronRunLog {
                id: 7,
                job_id: "job".into(),
                session_id: "session".into(),
                status: "success".into(),
                started_at: "2026-04-22T12:01:10Z".into(),
                finished_at: None,
                duration_ms: None,
                result_preview: None,
                error: None,
                delivery_status: None,
            }],
        )
        .expect("matches");

        assert_eq!(matches.iter().filter(|entry| entry.is_some()).count(), 1);
        assert!(matches[0].is_none());
        assert_eq!(matches[1].as_ref().map(|log| log.id), Some(7));
        assert!(matches[2].is_none());
    }

    #[test]
    fn row_to_cron_job_reads_every_start_at() {
        let path = temp_db_path("row-read");
        let db = CronDB::open(&path).expect("open db");
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO cron_jobs (
                id, name, description, schedule_json, payload_json, status,
                next_run_at, last_run_at, running_at, consecutive_failures, max_failures,
                notify_on_complete, delivery_targets_json, created_at, updated_at
            ) VALUES (
                ?1, ?2, NULL, ?3, ?4, 'active',
                ?5, NULL, NULL, 0, 5,
                1, '[]', ?6, ?6
            )",
            params![
                "row-read",
                "Hydrate",
                r#"{"type":"every","interval_ms":300000,"start_at":"2026-04-22T12:15:00Z"}"#,
                r#"{"type":"agentTurn","prompt":"drink water","agentId":null}"#,
                "2026-04-22T12:20:00Z",
                "2026-04-22T12:10:00Z",
            ],
        )
        .expect("insert");

        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json
                 FROM cron_jobs WHERE id='row-read'",
            )
            .expect("prepare");
        let job = stmt
            .query_row([], |row| {
                row_to_cron_job(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
            })
            .expect("row_to_cron_job");

        match job.schedule {
            CronSchedule::Every {
                start_at: Some(start_at),
                ..
            } => assert_eq!(start_at, "2026-04-22T12:15:00Z"),
            other => panic!("unexpected schedule: {other:?}"),
        }

        drop(stmt);
        drop(conn);
        cleanup_db_files(&path);
    }

    #[test]
    fn scheduled_claim_returns_execution_lease_and_clears_one_shot_next_run() {
        let path = temp_db_path("scheduled-claim");
        let db = CronDB::open(&path).expect("open db");
        let mut job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "drink water".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        let due_at = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "UPDATE cron_jobs SET next_run_at=?1 WHERE id=?2",
                params![due_at, job.id],
            )
            .expect("mark due");
        }
        job.next_run_at = Some(due_at);

        let claimed = db
            .claim_scheduled_job_for_execution(&job)
            .expect("claim")
            .expect("claimed job");
        let stored = db.get_job(&job.id).expect("load").expect("job exists");

        assert_eq!(claimed.job.id, job.id);
        assert_eq!(
            stored.running_at.as_deref(),
            Some(claimed.claimed_at.as_str())
        );
        assert_eq!(stored.next_run_at, None);
        assert!(db
            .claim_scheduled_job_for_execution(&job)
            .expect("second claim")
            .is_none());

        cleanup_db_files(&path);
    }

    #[test]
    fn scheduled_claim_does_not_claim_future_job() {
        let path = temp_db_path("scheduled-future-claim");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "drink water".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        assert!(db
            .claim_scheduled_job_for_execution(&job)
            .expect("claim")
            .is_none());

        cleanup_db_files(&path);
    }

    #[test]
    fn claims_reject_stale_payload_snapshot_after_agent_rebind() {
        let path = temp_db_path("stale-payload-claim");
        let db = CronDB::open(&path).expect("open db");
        let mut stale_job = db
            .add_job(&NewCronJob {
                name: "Rebind-safe".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "run once".into(),
                    agent_id: Some("old-agent".into()),
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        let due_at = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        stale_job.next_run_at = Some(due_at.clone());
        let rebound_payload = serde_json::to_string(&CronPayload::AgentTurn {
            prompt: "run once".into(),
            agent_id: Some("replacement-agent".into()),
        })
        .unwrap();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "UPDATE cron_jobs SET next_run_at=?1, payload_json=?2 WHERE id=?3",
                params![due_at, rebound_payload, stale_job.id],
            )
            .expect("simulate lifecycle rebind");
        }

        assert!(db
            .claim_scheduled_job_for_execution(&stale_job)
            .expect("scheduled stale claim")
            .is_none());
        assert!(db
            .claim_immediate_job_for_execution(&stale_job)
            .expect("immediate stale claim")
            .is_none());
        let stored = db.get_job(&stale_job.id).unwrap().unwrap();
        assert!(stored.running_at.is_none());
        assert_eq!(stored.next_run_at.as_deref(), Some(due_at.as_str()));

        cleanup_db_files(&path);
    }

    #[test]
    fn immediate_claim_returns_execution_lease_without_advancing_schedule() {
        let path = temp_db_path("immediate-claim");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "drink water".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        let original_next_run = job.next_run_at.clone();

        let claimed = db
            .claim_immediate_job_for_execution(&job)
            .expect("claim")
            .expect("claimed job");
        let stored = db.get_job(&job.id).expect("load").expect("job exists");

        assert_eq!(claimed.job.id, job.id);
        assert_eq!(
            stored.running_at.as_deref(),
            Some(claimed.claimed_at.as_str())
        );
        assert_eq!(stored.next_run_at, original_next_run);
        assert!(db
            .claim_immediate_job_for_execution(&job)
            .expect("second claim")
            .is_none());

        cleanup_db_files(&path);
    }

    #[test]
    fn count_running_tracks_running_markers() {
        // §4: count_running is the concurrency-accounting source of truth for the
        // scheduler's slot-before-claim gate. It must count every job whose
        // running marker is set (claimed, regardless of path) and drop back as
        // they clear.
        let path = temp_db_path("count-running");
        let db = CronDB::open(&path).expect("open db");
        let mk = |name: &str| NewCronJob {
            name: name.into(),
            description: None,
            project_id: None,
            schedule: CronSchedule::At {
                timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
            },
            payload: CronPayload::AgentTurn {
                prompt: "p".into(),
                agent_id: None,
            },
            max_failures: None,
            notify_on_complete: None,
            delivery_targets: None,
            prefix_delivery_with_name: None,
            job_timeout_secs: None,
            permission_mode_override: None,
            sandbox_mode_override: None,
        };
        let a = db.add_job(&mk("a")).expect("add a");
        let b = db.add_job(&mk("b")).expect("add b");

        assert_eq!(db.count_running().expect("count"), 0);
        db.claim_immediate_job_for_execution(&a)
            .expect("claim a")
            .expect("leased a");
        assert_eq!(db.count_running().expect("count"), 1);
        db.claim_immediate_job_for_execution(&b)
            .expect("claim b")
            .expect("leased b");
        assert_eq!(db.count_running().expect("count"), 2);
        db.clear_running(&a.id).expect("clear a");
        assert_eq!(db.count_running().expect("count"), 1);

        cleanup_db_files(&path);
    }

    #[test]
    fn clear_running_if_owner_only_clears_matching_marker() {
        // §4 panic-safe backstop: the owner-checked clear must release THIS run's
        // marker but never disturb a marker a later re-claim has replaced.
        let path = temp_db_path("clear-if-owner");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "j".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        let claimed = db
            .claim_immediate_job_for_execution(&job)
            .expect("claim")
            .expect("leased");

        // Wrong timestamp (e.g. a stale guard after a re-claim) → no-op.
        assert!(!db
            .clear_running_if_owner(&job.id, "1999-01-01T00:00:00+00:00")
            .expect("clear mismatch"));
        assert_eq!(db.count_running().expect("count"), 1);

        // Matching timestamp → releases the slot.
        assert!(db
            .clear_running_if_owner(&job.id, &claimed.claimed_at)
            .expect("clear match"));
        assert_eq!(db.count_running().expect("count"), 0);
        // Second call is now a no-op (already cleared).
        assert!(!db
            .clear_running_if_owner(&job.id, &claimed.claimed_at)
            .expect("clear again"));

        cleanup_db_files(&path);
    }

    #[test]
    fn update_after_run_reports_auto_disable_at_threshold() {
        // §5: update_after_run returns true exactly when a failure pushes the job
        // to max_failures and flips it to `disabled` — the signal the executor
        // uses to fire the one-shot disable notification. Uses a RECURRING (Every)
        // schedule: per §7 / Sweep#1 a one-shot `At` now terminalizes (missed) on
        // its first failure rather than accumulating toward auto-disable, so the
        // threshold behavior only applies to recurring jobs.
        let path = temp_db_path("auto-disable-signal");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "flaky".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(2),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        // First failure: below threshold → not disabled.
        assert!(!db
            .update_after_run(&job.id, false, &job.schedule)
            .expect("upd1"));
        let after1 = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(after1.consecutive_failures, 1);
        assert_eq!(after1.status, CronJobStatus::Active);

        // Second failure: hits max_failures (2) → auto-disabled, returns true.
        assert!(db
            .update_after_run(&job.id, false, &job.schedule)
            .expect("upd2"));
        let after2 = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(after2.consecutive_failures, 2);
        assert_eq!(after2.status, CronJobStatus::Disabled);

        // Third failure on the ALREADY-disabled job (e.g. a manual run-now that
        // failed again): must NOT re-notify (returns false) and must NOT re-bump
        // the count — the disable is one-shot.
        assert!(!db
            .update_after_run(&job.id, false, &job.schedule)
            .expect("upd3"));
        let after3 = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(
            after3.consecutive_failures, 2,
            "count must not grow once disabled"
        );
        assert_eq!(after3.status, CronJobStatus::Disabled);

        cleanup_db_files(&path);
    }

    #[test]
    fn update_after_run_success_does_not_revive_paused_job() {
        // Review fix: a recurring job the user pauses mid-run must NOT be revived to
        // `active` when its in-flight run completes successfully. The success branch
        // is guarded on `status='active'`, so a paused job stays paused (and keeps
        // its next_run_at), preserving the explicit pause instead of silently
        // re-arming the schedule.
        let path = temp_db_path("success-no-revive-paused");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "recurring".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        // User pauses the job (e.g. while its claimed run is still executing).
        db.toggle_job(&job.id, false).expect("pause");
        assert_eq!(
            db.get_job(&job.id).expect("get").expect("exists").status,
            CronJobStatus::Paused
        );

        // The in-flight run completes successfully → update_after_run(success).
        assert!(!db
            .update_after_run(&job.id, true, &job.schedule)
            .expect("upd"));

        // The job must still be Paused — NOT silently revived to Active.
        assert_eq!(
            db.get_job(&job.id).expect("get").expect("exists").status,
            CronJobStatus::Paused,
            "a successful in-flight run must not revive a job the user paused"
        );

        cleanup_db_files(&path);
    }

    #[test]
    fn one_shot_at_failure_terminalizes_missed_not_retries() {
        // §7 / Sweep#1: a one-shot `At` that fails (or times out) must NOT be
        // rescheduled with backoff — its agent turn may already have produced side
        // effects, so re-running it would repeat them. It terminalizes as `missed`
        // on the first failure (next_run_at cleared), never re-firing.
        let path = temp_db_path("at-fail-terminal");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "one-shot".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: "2999-01-01T00:00:00Z".into(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "do once".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        // First failure → terminalize (not auto-disable, not retry).
        assert!(!db
            .update_after_run(&job.id, false, &job.schedule)
            .expect("upd"));
        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(
            stored.status,
            CronJobStatus::Missed,
            "failed one-shot At terminalizes as missed, not retried"
        );
        assert!(
            stored.next_run_at.is_none(),
            "no future retry scheduled for a failed one-shot At"
        );

        cleanup_db_files(&path);
    }

    #[test]
    fn reschedule_without_failure_preserves_failure_count() {
        // C07 building block: a recurring Empty run advances its schedule via
        // reschedule_without_failure, which must NOT touch consecutive_failures — so
        // an intermittent empty output can't reset a failing job's counter and dodge
        // auto-disable.
        let path = temp_db_path("reschedule-keeps-failures");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "j".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "UPDATE cron_jobs SET consecutive_failures=3 WHERE id=?1",
                params![job.id],
            )
            .expect("seed failures");
        }
        db.reschedule_without_failure(&job.id, &job.schedule)
            .expect("reschedule");
        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(
            stored.consecutive_failures, 3,
            "empty/infra reschedule must not reset the failure counter"
        );
        assert_eq!(stored.status, CronJobStatus::Active);
        assert!(stored.next_run_at.is_some(), "schedule advanced");
        cleanup_db_files(&path);
    }

    #[test]
    fn toggle_resume_past_at_terminalizes_missed() {
        // C24: re-enabling a one-shot `At` whose time is now past must terminalize
        // as `missed`, not resurrect as active+next_run=NULL (an un-fireable zombie).
        let path = temp_db_path("toggle-past-at");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "elapsed".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: "2999-01-01T00:00:00Z".into(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        // Rewrite to a PAST timestamp + paused, simulating a one-shot that elapsed
        // while paused, then re-enable it.
        {
            let conn = db.conn.lock().expect("lock");
            let past = serde_json::to_string(&CronSchedule::At {
                timestamp: "2000-01-01T00:00:00Z".into(),
            })
            .unwrap();
            conn.execute(
                "UPDATE cron_jobs SET schedule_json=?1, status='paused', next_run_at=NULL WHERE id=?2",
                params![past, job.id],
            )
            .expect("rewrite to past");
        }
        db.toggle_job(&job.id, true).expect("enable");
        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(
            stored.status,
            CronJobStatus::Missed,
            "resuming a past one-shot At terminalizes missed, not active zombie"
        );
        assert!(stored.next_run_at.is_none());
        cleanup_db_files(&path);
    }

    #[test]
    fn max_failures_zero_never_auto_disables() {
        // C26: max_failures=0 = unlimited (never auto-disable). A recurring job that
        // keeps failing stays active with a growing failure count.
        let path = temp_db_path("max-failures-zero");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "unlimited".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(0),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        for _ in 0..10 {
            assert!(
                !db.update_after_run(&job.id, false, &job.schedule)
                    .expect("upd"),
                "max_failures=0 never reports auto-disable"
            );
        }
        let stored = db.get_job(&job.id).expect("get").expect("exists");
        assert_eq!(stored.status, CronJobStatus::Active);
        assert_eq!(stored.consecutive_failures, 10);
        cleanup_db_files(&path);
    }

    #[test]
    fn delete_running_job_requests_cancel() {
        // C15: deleting a mid-run job requests cancellation (run-keyed) so the
        // in-flight turn stops instead of running to completion + delivering against
        // a row that's about to be deleted.
        let path = temp_db_path("delete-cancel");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "running".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");
        let claimed = db
            .claim_immediate_job_for_execution(&job)
            .expect("claim")
            .expect("claimed");
        // Register the run's cancel flag exactly as execute_claimed_job would.
        let flag = super::super::cancel::register(&job.id, &claimed.claimed_at);
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        db.delete_job(&job.id).expect("delete");
        assert!(
            flag.load(std::sync::atomic::Ordering::SeqCst),
            "delete requested cancellation of the in-flight run"
        );
        super::super::cancel::remove(&job.id, &claimed.claimed_at);
        cleanup_db_files(&path);
    }

    #[test]
    fn mark_missed_respects_grace_and_recovers_zombies() {
        // §7: late-fire grace + terminal-state recovery for one-shot At jobs.
        let path = temp_db_path("at-grace");
        let db = CronDB::open(&path).expect("open db");
        let mk_at = |name: &str| {
            db.add_job(&NewCronJob {
                name: name.into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "p".into(),
                    agent_id: None,
                },
                max_failures: None,
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add")
        };
        let within = mk_at("within"); // past-due within grace → stays active (late-fires)
        let beyond = mk_at("beyond"); // past-due beyond grace → missed
        let zombie = mk_at("zombie"); // next_run_at NULL (claimed-then-crashed) → missed
        let future = mk_at("future"); // not yet due → stays active
        let running = mk_at("running"); // in-flight (running_at set, next_run_at NULL) → stays active

        {
            let conn = db.conn.lock().unwrap();
            let now = Utc::now();
            let set = |id: &str, val: Option<String>| {
                conn.execute(
                    "UPDATE cron_jobs SET next_run_at=?1 WHERE id=?2",
                    params![val, id],
                )
                .unwrap();
            };
            set(
                &within.id,
                Some((now - chrono::Duration::seconds(100)).to_rfc3339()),
            );
            set(
                &beyond.id,
                Some((now - chrono::Duration::seconds(1000)).to_rfc3339()),
            );
            set(&zombie.id, None);
            // In-flight one-shot: next_run_at cleared at claim, but running_at set.
            // Matches the NULL branch yet must NOT be reaped (it's running).
            conn.execute(
                "UPDATE cron_jobs SET running_at=?1 WHERE id=?2",
                params![now.to_rfc3339(), running.id],
            )
            .unwrap();
            set(&running.id, None);
            // `future` keeps its ~1h-ahead next_run_at from add_job.
        }

        let marked = db.mark_missed_at_jobs(300).expect("mark");
        assert_eq!(marked, 2, "only beyond-grace + zombie are missed");
        let status = |id: &str| db.get_job(id).unwrap().unwrap().status;
        assert_eq!(status(&within.id), CronJobStatus::Active);
        assert_eq!(status(&beyond.id), CronJobStatus::Missed);
        assert_eq!(status(&zombie.id), CronJobStatus::Missed);
        assert_eq!(status(&future.id), CronJobStatus::Active);
        // Regression: an in-flight one-shot (running_at set) is never reaped even
        // though its next_run_at is NULL — otherwise a run ≳15s gets marked
        // `missed` mid-execution and the later success can't recover it.
        assert_eq!(status(&running.id), CronJobStatus::Active);

        cleanup_db_files(&path);
    }

    #[test]
    fn inflight_at_survives_reap_then_success_completes() {
        // End-to-end pin for the "success run-log yet job stuck `missed`" race
        // across mark_missed_at_jobs (reap) and update_after_run (terminalize):
        // an in-flight one-shot must survive a mid-run reap tick AND then be
        // terminalized `completed` by its successful run. Both guards are
        // load-bearing — the reap's `running_at IS NULL` keeps the status
        // `active` so update_after_run's `status='active'` success branch can
        // fire. If either regresses, this test catches the stuck-`missed` bug.
        let path = temp_db_path("inflight-reap-success");
        let db = CronDB::open(&path).expect("open db");
        let job = db
            .add_job(&NewCronJob {
                name: "one-shot".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::At {
                    timestamp: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                },
                payload: CronPayload::AgentTurn {
                    prompt: "do once".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: None,
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        // Simulate claim_scheduled_job_for_execution: running_at set, next_run_at
        // cleared to NULL (the one-shot in-flight shape), status still active.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE cron_jobs SET running_at=?1, next_run_at=NULL WHERE id=?2",
                params![Utc::now().to_rfc3339(), job.id],
            )
            .unwrap();
        }

        // A reap tick fires mid-execution — must NOT touch the running job.
        let marked = db.mark_missed_at_jobs(300).expect("mark");
        assert_eq!(marked, 0, "in-flight one-shot must not be reaped");
        assert_eq!(
            db.get_job(&job.id).unwrap().unwrap().status,
            CronJobStatus::Active
        );

        // Run finishes successfully → update_after_run terminalizes it.
        assert!(!db
            .update_after_run(&job.id, true, &job.schedule)
            .expect("upd"));
        let final_job = db.get_job(&job.id).unwrap().unwrap();
        assert_eq!(
            final_job.status,
            CronJobStatus::Completed,
            "a successful in-flight one-shot must end `completed`, not `missed`"
        );
        assert!(
            final_job.next_run_at.is_none(),
            "completed one-shot has no next run"
        );

        cleanup_db_files(&path);
    }

    #[test]
    fn add_job_terminalizes_past_at_as_missed() {
        // §7: a one-shot At created with an already-past timestamp has no next run,
        // so it's persisted as `missed` immediately instead of an active zombie
        // that only the next restart's mark_missed would reap.
        let path = temp_db_path("past-at-missed");
        let db = CronDB::open(&path).expect("open db");
        let mk = |name: &str, ts: String| NewCronJob {
            name: name.into(),
            description: None,
            project_id: None,
            schedule: CronSchedule::At { timestamp: ts },
            payload: CronPayload::AgentTurn {
                prompt: "p".into(),
                agent_id: None,
            },
            max_failures: None,
            notify_on_complete: None,
            delivery_targets: None,
            prefix_delivery_with_name: None,
            job_timeout_secs: None,
            permission_mode_override: None,
            sandbox_mode_override: None,
        };

        let past = db
            .add_job(&mk(
                "past",
                (Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
            ))
            .expect("add past");
        assert_eq!(past.status, CronJobStatus::Missed);
        assert!(past.next_run_at.is_none());
        // Confirm persisted, not just the returned struct.
        assert_eq!(
            db.get_job(&past.id).unwrap().unwrap().status,
            CronJobStatus::Missed
        );

        // A future At stays active with a next_run.
        let future = db
            .add_job(&mk(
                "future",
                (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            ))
            .expect("add future");
        assert_eq!(future.status, CronJobStatus::Active);
        assert!(future.next_run_at.is_some());

        cleanup_db_files(&path);
    }
}
