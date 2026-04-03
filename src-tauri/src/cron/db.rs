use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use rusqlite::{params, Connection};
use std::str::FromStr;
use std::sync::Mutex;

use super::schedule::{backoff_delay_ms, compute_next_run, validate_cron_expression};
use super::types::*;

// ── CronDB (Persistence Layer) ──────────────────────────────────

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
        let now = Utc::now().to_rfc3339();
        let schedule_json = serde_json::to_string(&input.schedule)?;
        let payload_json = serde_json::to_string(&input.payload)?;
        let max_failures = input.max_failures.unwrap_or(5);

        // Compute initial next_run_at
        let next_run = compute_next_run(&input.schedule, &Utc::now()).map(|dt| dt.to_rfc3339());

        let notify = input.notify_on_complete.unwrap_or(true);

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO cron_jobs (id, name, description, schedule_json, payload_json, status, next_run_at, max_failures, notify_on_complete, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, ?9)",
            params![id, input.name, input.description, schedule_json, payload_json, next_run, max_failures, notify as i32, now],
        )?;

        Ok(CronJob {
            id,
            name: input.name.clone(),
            description: input.description.clone(),
            schedule: input.schedule.clone(),
            payload: input.payload.clone(),
            status: CronJobStatus::Active,
            next_run_at: next_run,
            last_run_at: None,
            running_at: None,
            consecutive_failures: 0,
            max_failures,
            created_at: now.clone(),
            updated_at: now,
            notify_on_complete: notify,
        })
    }

    /// Update an existing job.
    pub fn update_job(&self, job: &CronJob) -> Result<()> {
        // Validate cron expression if applicable
        if let CronSchedule::Cron { ref expression, .. } = job.schedule {
            validate_cron_expression(expression)?;
        }

        let now = Utc::now().to_rfc3339();
        let schedule_json = serde_json::to_string(&job.schedule)?;
        let payload_json = serde_json::to_string(&job.payload)?;

        // Recompute next_run_at if schedule changed
        let next_run = if job.status == CronJobStatus::Active {
            compute_next_run(&job.schedule, &Utc::now()).map(|dt| dt.to_rfc3339())
        } else {
            job.next_run_at.clone()
        };

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET name=?1, description=?2, schedule_json=?3, payload_json=?4, status=?5, next_run_at=?6, max_failures=?7, notify_on_complete=?8, updated_at=?9
             WHERE id=?10",
            params![
                job.name, job.description, schedule_json, payload_json,
                job.status.as_str(), next_run, job.max_failures, job.notify_on_complete as i32, now, job.id
            ],
        )?;
        Ok(())
    }

    /// Delete a job by ID.
    pub fn delete_job(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute("DELETE FROM cron_jobs WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Get a single job by ID.
    pub fn get_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete
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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete
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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete
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

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

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

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
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
            let occurrences = self.compute_occurrences(&job.schedule, start, end, &job.status);

            for occ in occurrences {
                let occ_str = occ.to_rfc3339();
                // Check if there's a matching run log
                let run_log = self.find_run_log_near(&job.id, &occ)?;

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
        schedule: &CronSchedule,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        status: &CronJobStatus,
    ) -> Vec<DateTime<Utc>> {
        match schedule {
            CronSchedule::At { timestamp } => {
                if let Some(ts) = super::schedule::parse_flexible_timestamp(timestamp) {
                    if ts >= *start && ts < *end {
                        return vec![ts];
                    }
                }
                vec![]
            }
            CronSchedule::Every { interval_ms } => {
                if *interval_ms == 0 || *status != CronJobStatus::Active {
                    return vec![];
                }
                let dur = Duration::milliseconds(*interval_ms as i64);
                let mut results = Vec::new();
                // Start from the first occurrence at or after `start`
                let mut t = *start;
                // Limit to prevent infinite loops (max 1000 events per month)
                let max_events = 1000;
                while t < *end && results.len() < max_events {
                    results.push(t);
                    t = t + dur;
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
                        if results.len() >= 1000 {
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

    /// Find a run log entry near a specific time for a job (within +/-2 minutes).
    fn find_run_log_near(&self, job_id: &str, time: &DateTime<Utc>) -> Result<Option<CronRunLog>> {
        let window_start = (*time - Duration::minutes(2)).to_rfc3339();
        let window_end = (*time + Duration::minutes(2)).to_rfc3339();

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error
             FROM cron_run_logs WHERE job_id=?1 AND started_at >= ?2 AND started_at <= ?3
             ORDER BY started_at DESC LIMIT 1"
        )?;
        let mut rows = stmt.query(params![job_id, window_start, window_end])?;
        if let Some(row) = rows.next()? {
            Ok(Some(CronRunLog {
                id: row.get(0)?,
                job_id: row.get(1)?,
                session_id: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                duration_ms: crate::sql_opt_u64(row, 6)?,
                result_preview: row.get(7)?,
                error: row.get(8)?,
            }))
        } else {
            Ok(None)
        }
    }

    // ── Startup Recovery ────────────────────────────────────────

    /// Mark orphaned runs (started but never finished) as error.
    pub fn recover_orphaned_runs(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;

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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE cron_jobs SET running_at=?1 WHERE id=?2 AND running_at IS NULL",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    /// Clear running_at after job execution completes (called by execute_job).
    pub fn clear_running(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }

    /// Clear all stale running_at markers (for startup recovery after crash).
    pub fn clear_all_running(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
        let count = conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE running_at IS NOT NULL",
            [],
        )?;
        Ok(count)
    }

    /// Mark missed one-shot At jobs as 'missed'.
    pub fn mark_missed_at_jobs(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("CronDB lock poisoned: {e}"))?;
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

// ── Helper: Row -> CronJob ───────────────────────────────────────

pub(crate) fn row_to_cron_job(row: &rusqlite::Row) -> Result<CronJob> {
    let schedule_json: String = row.get(3)?;
    let payload_json: String = row.get(4)?;
    let status_str: String = row.get(5)?;

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
    })
}
