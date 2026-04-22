use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use rusqlite::{params, Connection};
use std::str::FromStr;
use std::sync::Mutex;

use super::schedule::{backoff_delay_ms, compute_next_run, validate_cron_expression};
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
                updated_at TEXT NOT NULL
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
                ON cron_run_logs(job_id, started_at DESC);",
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

        backfill_every_schedule_start_at(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ── Job CRUD ────────────────────────────────────────────────

    /// Create a new job from NewCronJob input. Returns the full CronJob.
    pub fn add_job(&self, input: &NewCronJob) -> Result<CronJob> {
        // Validate cron expression if applicable
        if let CronSchedule::Cron { ref expression, .. } = input.schedule {
            validate_cron_expression(expression)?;
        }

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

        let notify = input.notify_on_complete.unwrap_or(true);
        let delivery_targets = input.delivery_targets.clone().unwrap_or_default();
        let delivery_targets_json = serde_json::to_string(&delivery_targets)?;

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_jobs (id, name, description, schedule_json, payload_json, status, next_run_at, max_failures, notify_on_complete, delivery_targets_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, ?10, ?10)",
            params![id, input.name, input.description, schedule_json, payload_json, next_run, max_failures, notify as i32, delivery_targets_json, now_str],
        )?;

        Ok(CronJob {
            id,
            name: input.name.clone(),
            description: input.description.clone(),
            schedule,
            payload: input.payload.clone(),
            status: CronJobStatus::Active,
            next_run_at: next_run,
            last_run_at: None,
            running_at: None,
            consecutive_failures: 0,
            max_failures,
            created_at: now_str.clone(),
            updated_at: now_str,
            notify_on_complete: notify,
            delivery_targets,
        })
    }

    /// Update an existing job.
    pub fn update_job(&self, job: &CronJob) -> Result<()> {
        // Validate cron expression if applicable
        if let CronSchedule::Cron { ref expression, .. } = job.schedule {
            validate_cron_expression(expression)?;
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let mut schedule = job.schedule.clone();
        normalize_every_schedule_start_at(&mut schedule, &now);
        let schedule_json = serde_json::to_string(&schedule)?;
        let payload_json = serde_json::to_string(&job.payload)?;
        let delivery_targets_json = serde_json::to_string(&job.delivery_targets)?;

        // Recompute next_run_at if schedule changed
        let next_run = if job.status == CronJobStatus::Active {
            compute_next_run(&schedule, &now).map(|dt| dt.to_rfc3339())
        } else {
            job.next_run_at.clone()
        };

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET name=?1, description=?2, schedule_json=?3, payload_json=?4, status=?5, next_run_at=?6, max_failures=?7, notify_on_complete=?8, delivery_targets_json=?9, updated_at=?10
             WHERE id=?11",
            params![
                job.name, job.description, schedule_json, payload_json,
                job.status.as_str(), next_run, job.max_failures, job.notify_on_complete as i32, delivery_targets_json, now_str, job.id
            ],
        )?;
        Ok(())
    }

    /// Delete a job by ID.
    pub fn delete_job(&self, id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute("DELETE FROM cron_jobs WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Get a single job by ID.
    pub fn get_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json
             FROM cron_jobs WHERE id=?1"
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_cron_job(row)?))
        } else {
            Ok(None)
        }
    }

    /// List all jobs.
    pub fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json
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
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete, delivery_targets_json
             FROM cron_jobs WHERE status='active' AND running_at IS NULL AND next_run_at IS NOT NULL AND next_run_at <= ?1"
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
            conn.execute(
                "UPDATE cron_jobs SET status=?1, next_run_at=?2, consecutive_failures=0, updated_at=?3 WHERE id=?4",
                params![new_status, next_run, now, id],
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
    pub fn update_after_run(&self, id: &str, success: bool, schedule: &CronSchedule) -> Result<()> {
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
            conn.execute(
                "UPDATE cron_jobs SET status=?1, next_run_at=?2, last_run_at=?3, consecutive_failures=0, updated_at=?3 WHERE id=?4",
                params![next_status, next_run, now_str, id],
            )?;
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

            if new_failures >= max_failures {
                // Auto-disable
                conn.execute(
                    "UPDATE cron_jobs SET status='disabled', consecutive_failures=?1, last_run_at=?2, updated_at=?2 WHERE id=?3",
                    params![new_failures, now_str, id],
                )?;
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
            }
        }
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
            "INSERT INTO cron_run_logs (job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                log.job_id, log.session_id, log.status, log.started_at,
                log.finished_at, log.duration_ms.map(|v| v as i64), log.result_preview, log.error
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get run logs for a job, ordered by most recent first.
    pub fn get_run_logs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunLog>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error
             FROM cron_run_logs WHERE job_id=?1 ORDER BY started_at DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![job_id, limit as i64], |row| {
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
            })
        })?;
        let mut logs = Vec::new();
        for row in rows {
            logs.push(row?);
        }
        Ok(logs)
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
            CronSchedule::Cron { expression, .. } => {
                if let Ok(cron_schedule) = CronExpression::from_str(expression) {
                    let mut results = Vec::new();
                    // Use a time slightly before start to catch events at exactly start
                    let query_start = *start - Duration::seconds(1);
                    for next in cron_schedule.after(&query_start) {
                        if next >= *end {
                            break;
                        }
                        if next >= *start {
                            results.push(next);
                        }
                        // Safety limit
                        if results.len() >= MAX_CALENDAR_EVENTS_PER_JOB {
                            break;
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
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error
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
            })
        })?;
        let mut logs = Vec::new();
        for row in rows {
            logs.push(row?);
        }
        Ok(logs)
    }

    // ── Startup Recovery ────────────────────────────────────────

    /// Mark orphaned runs (started but never finished) as error.
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

    /// Atomically claim a job for execution: set running_at and advance next_run_at.
    /// Returns true if the job was claimed (no one else grabbed it first).
    pub fn claim_job_for_execution(&self, job: &CronJob) -> Result<bool> {
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

        // Atomically claim: only succeed if still active, not running, and next_run_at matches
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=?1, next_run_at=?2, updated_at=?1
             WHERE id=?3 AND next_run_at=?4 AND status='active' AND running_at IS NULL",
            params![now_str, next_run, job.id, job.next_run_at],
        )?;
        Ok(rows > 0)
    }

    /// Atomically claim a job for execution. Returns `false` if already running.
    pub fn try_mark_running(&self, id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=?1 WHERE id=?2 AND running_at IS NULL",
            params![now, id],
        )?;
        Ok(rows > 0)
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

    /// Mark missed one-shot At jobs as 'missed'.
    pub fn mark_missed_at_jobs(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        // Find active At jobs whose next_run_at is in the past
        let count = conn.execute(
            "UPDATE cron_jobs SET status='missed', updated_at=?1
             WHERE status='active' AND next_run_at IS NOT NULL AND next_run_at < ?1
             AND schedule_json LIKE '%\"type\":\"at\"%'",
            params![now],
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
    let window_ms = Duration::minutes(CALENDAR_EVENT_WINDOW_MINUTES).num_milliseconds();
    let occurrence_ms: Vec<i64> = occurrences
        .iter()
        .map(|occ| occ.timestamp_millis())
        .collect();
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
        let insertion = match occurrence_ms.binary_search(&log_ms) {
            Ok(idx) => idx,
            Err(idx) => idx,
        };

        let candidate_indices = [
            insertion.checked_sub(1),
            (insertion < occurrence_ms.len()).then_some(insertion),
        ];

        let mut best: Option<(usize, i64)> = None;
        for candidate in candidate_indices.into_iter().flatten() {
            let diff = (occurrence_ms[candidate] - log_ms).abs();
            if diff > window_ms {
                continue;
            }
            match best {
                Some((_, best_diff)) if diff >= best_diff => {}
                _ => best = Some((candidate, diff)),
            }
        }

        if let Some((best_idx, diff)) = best {
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
    })
}

#[cfg(test)]
mod tests {
    use super::{match_run_logs_to_occurrences, row_to_cron_job, CronDB};
    use crate::cron::{CronPayload, CronSchedule, NewCronJob};
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
    fn add_job_assigns_every_start_at_and_next_run() {
        let path = temp_db_path("add-job");
        let db = CronDB::open(&path).expect("open db");

        let job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
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
}
