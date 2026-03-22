use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule as CronExpression;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::paths;

// ── Data Structures ─────────────────────────────────────────────

/// Schedule types: one-shot, fixed interval, or cron expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CronSchedule {
    /// Fire once at a specific timestamp
    At { timestamp: String },
    /// Fire every N milliseconds
    Every {
        interval_ms: u64,
    },
    /// Cron expression with optional timezone (default UTC)
    Cron {
        expression: String,
        timezone: Option<String>,
    },
}

/// What the job does when triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CronPayload {
    /// Run an agent turn with the given prompt
    AgentTurn {
        prompt: String,
        agent_id: Option<String>,
    },
}

/// Job status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CronJobStatus {
    Active,
    Paused,
    Disabled,
    Completed,
    Missed,
}

impl CronJobStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
            Self::Completed => "completed",
            Self::Missed => "missed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "disabled" => Self::Disabled,
            "completed" => Self::Completed,
            "missed" => Self::Missed,
            _ => Self::Active,
        }
    }
}

/// A scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub status: CronJobStatus,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    /// Set when the job is currently executing; cleared on completion.
    pub running_at: Option<String>,
    pub consecutive_failures: u32,
    pub max_failures: u32,
    pub created_at: String,
    pub updated_at: String,
    /// Whether to send a desktop notification when this job completes.
    #[serde(default = "default_true")]
    pub notify_on_complete: bool,
}

fn default_true() -> bool {
    true
}

/// A single run log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunLog {
    pub id: i64,
    pub job_id: String,
    pub session_id: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
}

/// Input for creating a new job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCronJob {
    pub name: String,
    pub description: Option<String>,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub max_failures: Option<u32>,
    pub notify_on_complete: Option<bool>,
}

/// Calendar event for the calendar view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub job_id: String,
    pub job_name: String,
    pub scheduled_at: String,
    pub status: CronJobStatus,
    pub run_log: Option<CronRunLog>,
}

// ── Schedule Computation ────────────────────────────────────────

/// Compute the next run time for a schedule, from a given reference time.
pub fn compute_next_run(schedule: &CronSchedule, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        CronSchedule::At { timestamp } => {
            let ts = DateTime::parse_from_rfc3339(timestamp)
                .ok()?
                .with_timezone(&Utc);
            if ts > *after { Some(ts) } else { None }
        }
        CronSchedule::Every { interval_ms } => {
            let dur = Duration::milliseconds(*interval_ms as i64);
            Some(*after + dur)
        }
        CronSchedule::Cron { expression, timezone } => {
            compute_next_cron(expression, timezone.as_deref(), after)
        }
    }
}

/// Parse cron expression and find the next occurrence after `after`.
fn compute_next_cron(
    expression: &str,
    _timezone: Option<&str>,
    after: &DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let schedule = CronExpression::from_str(expression).ok()?;
    // Find next occurrence after `after`
    schedule.after(after).next()
}

/// Validate a cron expression. Returns Ok if valid, Err with message if not.
pub fn validate_cron_expression(expression: &str) -> Result<()> {
    CronExpression::from_str(expression)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("Invalid cron expression: {}", e))
}

/// Compute exponential backoff delay for failed jobs.
/// Returns milliseconds to add to next_run_at.
pub fn backoff_delay_ms(consecutive_failures: u32) -> u64 {
    let base_ms: u64 = 30_000; // 30 seconds
    let max_ms: u64 = 3_600_000; // 1 hour
    let delay = base_ms.saturating_mul(2u64.saturating_pow(consecutive_failures.min(20)));
    delay.min(max_ms)
}

// ── CronDB (Persistence Layer) ──────────────────────────────────

/// SQLite-based persistence for cron jobs and run logs.
pub struct CronDB {
    conn: Mutex<Connection>,
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
            conn.execute_batch(
                "ALTER TABLE cron_jobs ADD COLUMN running_at TEXT;",
            )?;
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
        let next_run = compute_next_run(&input.schedule, &Utc::now())
            .map(|dt| dt.to_rfc3339());

        let notify = input.notify_on_complete.unwrap_or(true);

        let conn = self.conn.lock().unwrap();
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

        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM cron_jobs WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Get a single job by ID.
    pub fn get_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, schedule_json, payload_json, status, next_run_at, last_run_at, running_at, consecutive_failures, max_failures, created_at, updated_at, notify_on_complete
             FROM cron_jobs ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            row_to_cron_job(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    }

    /// Get all jobs that are due for execution (status=active, not running, next_run_at <= now).
    pub fn get_due_jobs(&self, now: &DateTime<Utc>) -> Result<Vec<CronJob>> {
        let now_str = now.to_rfc3339();
        let conn = self.conn.lock().unwrap();
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

        let conn = self.conn.lock().unwrap();

        // If re-enabling, recompute next_run_at
        if enabled {
            // Read current schedule
            let schedule_json: String = conn.query_row(
                "SELECT schedule_json FROM cron_jobs WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            let schedule: CronSchedule = serde_json::from_str(&schedule_json)?;
            let next_run = compute_next_run(&schedule, &Utc::now())
                .map(|dt| dt.to_rfc3339());
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
    pub fn update_after_run(
        &self,
        id: &str,
        success: bool,
        schedule: &CronSchedule,
    ) -> Result<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cron_run_logs (job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                log.job_id, log.session_id, log.status, log.started_at,
                log.finished_at, log.duration_ms, log.result_preview, log.error
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get run logs for a job, ordered by most recent first.
    pub fn get_run_logs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunLog>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_id, session_id, status, started_at, finished_at, duration_ms, result_preview, error
             FROM cron_run_logs WHERE job_id=?1 ORDER BY started_at DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![job_id, limit], |row| {
            Ok(CronRunLog {
                id: row.get(0)?,
                job_id: row.get(1)?,
                session_id: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                duration_ms: row.get(6)?,
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
                if let Ok(ts) = DateTime::parse_from_rfc3339(timestamp) {
                    let ts = ts.with_timezone(&Utc);
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
            CronSchedule::Cron { expression, timezone } => {
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

    /// Find a run log entry near a specific time for a job (within ±2 minutes).
    fn find_run_log_near(&self, job_id: &str, time: &DateTime<Utc>) -> Result<Option<CronRunLog>> {
        let window_start = (*time - Duration::minutes(2)).to_rfc3339();
        let window_end = (*time + Duration::minutes(2)).to_rfc3339();

        let conn = self.conn.lock().unwrap();
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
                duration_ms: row.get(6)?,
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();

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

    /// Clear running_at after job execution completes (called by execute_job).
    pub fn clear_running(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }

    /// Clear all stale running_at markers (for startup recovery after crash).
    pub fn clear_all_running(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE cron_jobs SET running_at=NULL WHERE running_at IS NOT NULL",
            [],
        )?;
        Ok(count)
    }

    /// Mark missed one-shot At jobs as 'missed'.
    pub fn mark_missed_at_jobs(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
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

// ── Helper: Row → CronJob ───────────────────────────────────────

fn row_to_cron_job(row: &rusqlite::Row) -> Result<CronJob> {
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

// ── Scheduler ───────────────────────────────────────────────────

/// Start the background cron scheduler on a dedicated OS thread with its own tokio runtime.
/// This avoids requiring an existing tokio runtime at call time (e.g. during Tauri .setup()).
pub fn start_scheduler(
    cron_db: Arc<CronDB>,
    session_db: Arc<crate::session::SessionDB>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cron-scheduler".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create cron tokio runtime");

            rt.block_on(async move {
                // Startup recovery
                if let Err(e) = cron_db.recover_orphaned_runs() {
                    app_error!("cron", "scheduler", "Failed to recover orphaned runs: {}", e);
                }
                match cron_db.clear_all_running() {
                    Ok(n) if n > 0 => app_warn!("cron", "scheduler", "Cleared {} stale running markers from previous session", n),
                    Err(e) => app_error!("cron", "scheduler", "Failed to clear stale running markers: {}", e),
                    _ => {}
                }
                if let Err(e) = cron_db.mark_missed_at_jobs() {
                    app_error!("cron", "scheduler", "Failed to mark missed at jobs: {}", e);
                }

                // Run catch-up for overdue recurring jobs
                if let Ok(due_jobs) = cron_db.get_due_jobs(&Utc::now()) {
                    if !due_jobs.is_empty() {
                        app_info!("cron", "scheduler", "Catch-up: {} overdue jobs found at startup", due_jobs.len());
                        for job in due_jobs {
                            match cron_db.claim_job_for_execution(&job) {
                                Ok(true) => {
                                    let db = cron_db.clone();
                                    let sdb = session_db.clone();
                                    tokio::spawn(async move {
                                        execute_job(&db, &sdb, &job).await;
                                    });
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    app_error!("cron", "scheduler", "Failed to claim catch-up job '{}': {}", job.name, e);
                                }
                            }
                        }
                    }
                }

                app_info!("cron", "scheduler", "Scheduler started");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
                let tick_running = Arc::new(AtomicBool::new(false));

                loop {
                    interval.tick().await;

                    // Scheduler-level guard: skip if previous tick is still processing
                    if tick_running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                        app_debug!("cron", "scheduler", "Previous tick still running, skipping");
                        continue;
                    }

                    let now = Utc::now();
                    match cron_db.get_due_jobs(&now) {
                        Ok(due_jobs) => {
                            for job in due_jobs {
                                // Claim job first to prevent duplicate execution
                                match cron_db.claim_job_for_execution(&job) {
                                    Ok(true) => {
                                        let db = cron_db.clone();
                                        let sdb = session_db.clone();
                                        tokio::spawn(async move {
                                            execute_job(&db, &sdb, &job).await;
                                        });
                                    }
                                    Ok(false) => {
                                        app_debug!("cron", "scheduler", "Job '{}' already claimed, skipping", job.name);
                                    }
                                    Err(e) => {
                                        app_error!("cron", "scheduler", "Failed to claim job '{}': {}", job.name, e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            app_error!("cron", "scheduler", "Failed to query due jobs: {}", e);
                        }
                    }

                    tick_running.store(false, Ordering::Release);
                }
            });
        })
        .expect("Failed to spawn cron scheduler thread")
}

/// Public wrapper for execute_job, callable from Tauri commands.
pub async fn execute_job_public(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    job: &CronJob,
) {
    execute_job(cron_db, session_db, job).await;
}

/// Execute a single cron job: build agent, run chat, record result.
async fn execute_job(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    job: &CronJob,
) {
    let start_time = std::time::Instant::now();
    let started_at = Utc::now().to_rfc3339();

    app_info!("cron", "executor", "Executing job '{}' ({})", job.name, job.id);

    // Extract prompt and agent_id from payload
    let (prompt, agent_id) = match &job.payload {
        CronPayload::AgentTurn { prompt, agent_id } => {
            (prompt.clone(), agent_id.clone().unwrap_or_else(|| "default".to_string()))
        }
    };

    // Create an isolated session for this cron run
    let session_id = match session_db.create_session(&agent_id) {
        Ok(meta) => {
            let _ = session_db.update_session_title(&meta.id, &job.name);
            let _ = session_db.mark_session_cron(&meta.id);
            meta.id
        }
        Err(e) => {
            app_error!("cron", "executor", "Failed to create session for job '{}': {}", job.name, e);
            record_failure(cron_db, job, &started_at, start_time, "no_session", &e.to_string(), "");
            return;
        }
    };

    // Build agent from provider store
    let result = build_and_run_agent(&agent_id, &prompt, &session_id, session_db).await;

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let finished_at = Utc::now().to_rfc3339();

    match result {
        Ok(response) => {
            app_info!("cron", "executor", "Job '{}' completed successfully ({}ms)", job.name, duration_ms);

            // Save user prompt and assistant response into the session
            let _ = session_db.append_message(&session_id, &crate::session::NewMessage::user(&prompt));
            let _ = session_db.append_message(&session_id, &crate::session::NewMessage::assistant(&response));

            // Record success run log
            let preview = if response.len() > 500 {
                Some(crate::truncate_utf8(&response, 500).to_string())
            } else {
                Some(response.clone())
            };
            let run_log = CronRunLog {
                id: 0,
                job_id: job.id.clone(),
                session_id: session_id.clone(),
                status: "success".to_string(),
                started_at,
                finished_at: Some(finished_at),
                duration_ms: Some(duration_ms),
                result_preview: preview,
                error: None,
            };
            let _ = cron_db.add_run_log(&run_log);
            let _ = cron_db.update_after_run(&job.id, true, &job.schedule);
            let _ = cron_db.clear_running(&job.id);

            // Emit Tauri event
            emit_cron_event(&job.id, &job.name, "success", job.notify_on_complete);
        }
        Err(e) => {
            app_error!("cron", "executor", "Job '{}' failed: {}", job.name, e);

            // Write the prompt + error message into the session so the user can see what happened
            let _ = session_db.append_message(&session_id, &crate::session::NewMessage::user(&prompt));
            let mut err_msg = crate::session::NewMessage::assistant(&e.to_string());
            err_msg.is_error = Some(true);
            let _ = session_db.append_message(&session_id, &err_msg);

            record_failure(cron_db, job, &started_at, start_time, "error", &e.to_string(), &session_id);
        }
    }
}

/// Build an AssistantAgent and run a chat message with full failover logic.
///
/// Uses the same error classification and retry strategy as the regular chat flow:
/// - Retryable errors (RateLimit/Overloaded/Timeout): retry same model up to MAX_RETRIES
///   with exponential backoff before falling back to the next model.
/// - Terminal errors (ContextOverflow): surface immediately, no fallback.
/// - Non-retryable errors (Auth/Billing/ModelNotFound/Unknown): skip to next model.
async fn build_and_run_agent(
    agent_id: &str,
    message: &str,
    session_id: &str,
    _session_db: &Arc<crate::session::SessionDB>,
) -> Result<String> {
    use crate::agent::AssistantAgent;
    use crate::failover;
    use crate::provider;

    const MAX_RETRIES: u32 = 2;
    const RETRY_BASE_MS: u64 = 1000;
    const RETRY_MAX_MS: u64 = 10_000;

    // Load provider store from disk
    let store = provider::load_store().unwrap_or_default();

    // Load agent config for model resolution
    let agent_model_config = crate::agent_loader::load_agent(agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();

    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

    // Build model chain
    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain.iter().any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        return Err(anyhow::anyhow!("No model configured for cron job execution"));
    }

    // Try each model in the chain with proper failover
    let mut last_error = String::new();
    for (idx, model_ref) in model_chain.iter().enumerate() {
        let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
            Some(p) => p,
            None => continue,
        };

        let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);

        // Per-model retry loop
        let mut retry_count: u32 = 0;
        loop {
            let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id);
            agent.set_agent_id(agent_id);
            agent.set_session_id(session_id);
            agent.set_extra_system_context(
                "## Execution Context\n\
                 You are running as a **scheduled task** (cron job), not an interactive chat.\n\
                 - No user is actively waiting — execute the prompt directly and concisely.\n\
                 - This is an isolated session with no prior conversation history.\n\
                 - Focus on completing the task described in the user message."
                .to_string()
            );

            let cancel = Arc::new(AtomicBool::new(false));
            match agent.chat(message, &[], None, cancel, |_delta| {}).await {
                Ok(response) => {
                    if idx > 0 {
                        app_info!("cron", "failover", "Fallback model {} succeeded", model_label);
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();
                    let reason = failover::classify_error(&last_error);

                    // Terminal error — surface immediately, no point trying other models
                    if reason.is_terminal() {
                        app_error!("cron", "failover", "Model {} hit terminal error ({:?}): {}", model_label, reason, last_error);
                        return Err(anyhow::anyhow!("{}", last_error));
                    }

                    // Retryable error — retry same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        app_warn!("cron", "failover", "Model {} retryable error ({:?}), attempt {}/{}, retrying in {}ms: {}", model_label, reason, retry_count, MAX_RETRIES, delay, last_error);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Non-retryable or retries exhausted — skip to next model
                    app_warn!("cron", "failover", "Model {} failed ({:?}), skipping to next model: {}", model_label, reason, last_error);
                    break;
                }
            }
        }
    }

    Err(anyhow::anyhow!("All models failed. Last error: {}", last_error))
}

/// Record a failure run log and update job state.
fn record_failure(
    cron_db: &Arc<CronDB>,
    job: &CronJob,
    started_at: &str,
    start_time: std::time::Instant,
    status: &str,
    error: &str,
    session_id: &str,
) {
    let duration_ms = start_time.elapsed().as_millis() as u64;
    let finished_at = Utc::now().to_rfc3339();

    let run_log = CronRunLog {
        id: 0,
        job_id: job.id.clone(),
        session_id: session_id.to_string(),
        status: status.to_string(),
        started_at: started_at.to_string(),
        finished_at: Some(finished_at),
        duration_ms: Some(duration_ms),
        result_preview: None,
        error: Some(error.to_string()),
    };
    let _ = cron_db.add_run_log(&run_log);
    let _ = cron_db.update_after_run(&job.id, false, &job.schedule);
    let _ = cron_db.clear_running(&job.id);

    // Emit Tauri event
    emit_cron_event(&job.id, &job.name, "error", job.notify_on_complete);
}

/// Emit a Tauri event to notify the frontend of a cron run result.
fn emit_cron_event(job_id: &str, job_name: &str, status: &str, notify: bool) {
    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let payload = serde_json::json!({
            "job_id": job_id,
            "job_name": job_name,
            "status": status,
            "notify": notify,
        });
        let _ = handle.emit("cron:run_completed", payload);
    }
}
