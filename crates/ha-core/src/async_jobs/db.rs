use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::sync::Mutex;

use super::types::{BackgroundJob, JobKind, JobStatus};

/// Row-level result of a retention sweep.
#[derive(Debug, Clone, Default)]
pub struct PurgeStats {
    pub rows_deleted: u64,
    pub spool_files_deleted: u64,
    pub spool_bytes_freed: u64,
}

/// Delete the on-disk spool files for a batch of just-removed job rows,
/// tallying freed files/bytes into `stats`. Runs outside the DB mutex (the rows
/// are already gone); a missing file is a no-op and any other error is logged,
/// never propagated. Shared by `purge_terminal_older_than` (age sweep) and
/// `purge_jobs_for_session` (incognito burn).
fn remove_spool_files(deleted_rows: &[(String, Option<String>)], stats: &mut PurgeStats) {
    for (job_id, spool_path) in deleted_rows {
        let Some(path) = spool_path else { continue };
        match std::fs::metadata(path) {
            Ok(meta) => {
                let bytes = meta.len();
                match std::fs::remove_file(path) {
                    Ok(()) => {
                        stats.spool_files_deleted += 1;
                        stats.spool_bytes_freed += bytes;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => crate::app_warn!(
                        "async_jobs",
                        "purge",
                        "Failed to delete spool file {} for job {}: {}",
                        path,
                        job_id,
                        e
                    ),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => crate::app_warn!(
                "async_jobs",
                "purge",
                "Failed to stat spool file {} for job {}: {}",
                path,
                job_id,
                e
            ),
        }
    }
}

/// SQLite-backed persistence for background jobs (R1 unified `background_jobs`
/// table — was `async_tool_jobs`).
///
/// Independent of `session.db` to keep the hot chat path lock-free; mirrors
/// the layout used by `cron::CronDB` and `recap` (see `paths::background_jobs_db_path`).
pub struct JobsDB {
    pub(crate) conn: Mutex<Connection>,
}

impl JobsDB {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open async_jobs DB at {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Schema evolution for this rebuildable cache. Newer columns (R5's
        // `group_id`, R6's `subagent_run_id`, R1's `kind`, the A-7
        // approval/governance columns) are referenced by every INSERT/SELECT
        // below; a `background_jobs` table from a prior version lacks them, and
        // `CREATE TABLE IF NOT EXISTS` would NOT add them — every spawn would
        // then fail with "no such column". Project policy is "no migration —
        // drop and rebuild": this DB is a pure cache (terminal rows are
        // advisory, in-flight rows are marked interrupted on restart
        // regardless), so on a stale schema we drop the table and let the CREATE
        // below rebuild the current shape. The probe targets the newest column
        // (`group_id`, R5); a failing probe means the table is either absent
        // (DROP is a no-op) or stale (DROP clears it); a current table passes
        // and is untouched. Bump the probe column when adding new ones.
        if conn
            .prepare("SELECT group_id FROM background_jobs LIMIT 0")
            .is_err()
        {
            conn.execute_batch("DROP TABLE IF EXISTS background_jobs;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS background_jobs (
                job_id TEXT PRIMARY KEY,
                session_id TEXT,
                agent_id TEXT,
                tool_name TEXT NOT NULL,
                tool_call_id TEXT,
                args_json TEXT NOT NULL,
                status TEXT NOT NULL,
                result_preview TEXT,
                result_path TEXT,
                error TEXT,
                created_at INTEGER NOT NULL,
                completed_at INTEGER,
                injected INTEGER NOT NULL DEFAULT 0,
                origin TEXT NOT NULL DEFAULT 'explicit',
                approval_origin TEXT,
                incognito INTEGER NOT NULL DEFAULT 0,
                pid INTEGER,
                cancel_requested INTEGER NOT NULL DEFAULT 0,
                kind TEXT NOT NULL DEFAULT 'tool',
                subagent_run_id TEXT,
                group_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_background_jobs_session_status
                ON background_jobs(session_id, status);
            CREATE INDEX IF NOT EXISTS idx_background_jobs_status_injected
                ON background_jobs(status, injected);
            CREATE INDEX IF NOT EXISTS idx_background_jobs_subagent_run
                ON background_jobs(subagent_run_id);
            CREATE INDEX IF NOT EXISTS idx_background_jobs_group
                ON background_jobs(group_id);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, job: &BackgroundJob) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO background_jobs (
                job_id, session_id, agent_id, tool_name, tool_call_id,
                args_json, status, result_preview, result_path, error,
                created_at, completed_at, injected, origin,
                approval_origin, incognito, pid, cancel_requested, kind,
                subagent_run_id, group_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            params![
                job.job_id,
                job.session_id,
                job.agent_id,
                job.tool_name,
                job.tool_call_id,
                job.args_json,
                job.status.as_str(),
                job.result_preview,
                job.result_path,
                job.error,
                job.created_at,
                job.completed_at,
                job.injected as i32,
                job.origin,
                job.approval_origin,
                job.incognito as i32,
                job.pid,
                job.cancel_requested as i32,
                job.kind.as_str(),
                job.subagent_run_id,
                job.group_id,
            ],
        )?;
        Ok(())
    }

    pub fn update_terminal(
        &self,
        job_id: &str,
        status: JobStatus,
        result_preview: Option<&str>,
        result_path: Option<&str>,
        error: Option<&str>,
        completed_at: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs
                SET status=?1, result_preview=?2, result_path=?3, error=?4, completed_at=?5
                WHERE job_id=?6
                  AND status IN ('queued','running','cancelling')",
            params![
                status.as_str(),
                result_preview,
                result_path,
                error,
                completed_at,
                job_id
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn mark_cancelling(&self, job_id: &str, error: Option<&str>) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs
                SET status=?1, error=COALESCE(?2, error)
                WHERE job_id=?3
                  AND status IN ('running','cancelling')",
            params![JobStatus::Cancelling.as_str(), error, job_id],
        )?;
        Ok(rows > 0)
    }

    /// Promote a queued job to `running` when the scheduler grants it a slot.
    /// Guarded `WHERE status='queued'` so a concurrent cancel that already moved
    /// the row to a terminal status can't be clobbered back to running. Returns
    /// whether a row was updated (false ⇒ the job was cancelled/removed while
    /// queued, so the scheduler must drop it).
    pub fn mark_running(&self, job_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs SET status='running'
                WHERE job_id=?1 AND status='queued'",
            params![job_id],
        )?;
        Ok(rows > 0)
    }

    /// R6: one-way sync of a `kind='subagent'` projection's status from its
    /// `subagent_runs` source of truth, keyed by `subagent_run_id`. Updates ONLY
    /// status + completed_at — never run content (task/result/error live in
    /// `subagent_runs`). Guarded `status NOT IN (<terminal>)` so a terminal
    /// projection is frozen (a late/duplicate sync can't reopen it), and scoped
    /// `kind='subagent'` so it can never touch a tool job. Returns whether a row
    /// matched (false ⇒ no projection for this run — a foreground / internal /
    /// incognito run that was never projected, so the sync is a no-op).
    pub fn update_subagent_projection_status(
        &self,
        subagent_run_id: &str,
        status: JobStatus,
        completed_at: Option<i64>,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let sql = format!(
            "UPDATE background_jobs
                SET status = ?2, completed_at = COALESCE(?3, completed_at)
                WHERE subagent_run_id = ?1
                  AND kind = 'subagent'
                  AND status NOT IN ({})",
            JobStatus::TERMINAL_STATUS_SQL_LIST
        );
        let rows = conn.execute(
            &sql,
            params![subagent_run_id, status.as_str(), completed_at],
        )?;
        Ok(rows > 0)
    }

    /// R5: the `group_id` recorded on a `kind='subagent'` projection, keyed by
    /// its `subagent_run_id`. Lets the status-sync choke point find the owning
    /// `Group` when a grouped child settles, without threading the id through
    /// the subagent layer. `Ok(None)` when the run isn't projected or isn't
    /// grouped. Scoped `kind='subagent'` so a tool job can never masquerade.
    pub fn group_id_for_subagent_run(&self, subagent_run_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let group_id: Option<Option<String>> = conn
            .query_row(
                "SELECT group_id FROM background_jobs
                    WHERE subagent_run_id = ?1 AND kind = 'subagent'",
                params![subagent_run_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(group_id.flatten())
    }

    /// R5: all child jobs of a `Group` (rows whose `group_id` is this group's
    /// `job_id`). The join coordinator reads this to decide whether every child
    /// is terminal. Does NOT include the group row itself.
    pub fn group_children(&self, group_id: &str) -> Result<Vec<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs WHERE group_id=?1",
        )?;
        let rows = stmt.query_map(params![group_id], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// R5: mark a `Group` row as sealed — all of its children have been spawned,
    /// so the join coordinator may complete it once they all settle. Stored in
    /// `args_json` (`{"sealed":true}`) since the column set carries no dedicated
    /// flag. Scoped `kind='group'`; idempotent.
    pub fn mark_group_sealed(&self, group_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs SET args_json='{\"sealed\":true}'
                WHERE job_id=?1 AND kind='group'",
            params![group_id],
        )?;
        Ok(rows > 0)
    }

    /// R5: atomically claim the right to complete a `Group` (one winner). Flips
    /// `running`→`completed` only if the row is still non-terminal, so exactly
    /// one of N concurrently-settling children (or a racing cancel) fires the
    /// merged injection. Returns whether THIS caller won the claim. Scoped
    /// `kind='group'`.
    pub fn claim_group_completion(&self, group_id: &str, completed_at: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let sql = format!(
            "UPDATE background_jobs
                SET status='completed', completed_at=?2
                WHERE job_id=?1 AND kind='group'
                  AND status NOT IN ({})",
            JobStatus::TERMINAL_STATUS_SQL_LIST
        );
        let rows = conn.execute(&sql, params![group_id, completed_at])?;
        Ok(rows > 0)
    }

    /// Physically delete a single job row by id. Used to roll back a freshly
    /// inserted row whose enqueue was rejected (queue full) so it never lingers
    /// as a stale `queued` row.
    pub fn delete(&self, job_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "DELETE FROM background_jobs WHERE job_id=?1",
            params![job_id],
        )?;
        Ok(())
    }

    pub fn mark_injected(&self, job_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE background_jobs SET injected=1 WHERE job_id=?1",
            params![job_id],
        )?;
        Ok(())
    }

    /// Record the OS pid of a running job's spawned child process (I3), so a
    /// crash/restart can detect and terminate orphaned process trees. Only
    /// touches still-active rows. Returns whether a row was updated.
    pub fn set_pid(&self, job_id: &str, pid: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs SET pid=?2
                WHERE job_id=?1
                  AND status IN ('running','cancelling','awaiting_approval')",
            params![job_id, pid],
        )?;
        Ok(rows > 0)
    }

    /// Set the cross-process cancel flag (I4): a process that does not own the
    /// in-memory `CancellationToken` for `job_id` sets this so the owning
    /// process's runner observes it on its next poll and aborts the work. Only
    /// touches still-active rows. Returns whether a row was updated.
    pub fn set_cancel_requested(&self, job_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE background_jobs SET cancel_requested=1
                WHERE job_id=?1
                  AND status IN ('queued','running','cancelling','awaiting_approval')",
            params![job_id],
        )?;
        Ok(rows > 0)
    }

    /// Whether the cross-process cancel flag is set for `job_id` (I4). Single
    /// PK lookup; the runner polls this so another process can cancel it.
    pub fn is_cancel_requested(&self, job_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let flag: Option<i64> = conn
            .query_row(
                "SELECT cancel_requested FROM background_jobs WHERE job_id=?1",
                params![job_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(flag.unwrap_or(0) != 0)
    }

    pub fn load(&self, job_id: &str) -> Result<Option<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs WHERE job_id=?1",
        )?;
        stmt.query_row(params![job_id], row_to_job)
            .optional()
            .map_err(Into::into)
    }

    /// All jobs whose status is still active (`queued` / `running` /
    /// `cancelling` / `awaiting_approval`) — used by startup replay.
    /// `awaiting_approval` and `queued` are included because a restart kills the
    /// in-memory approval channel / scheduler queue (a queued job's live ctx is
    /// gone), so the job is unrecoverable and must be marked `interrupted` too.
    pub fn list_running(&self) -> Result<Vec<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs WHERE status IN ('queued','running','cancelling','awaiting_approval')",
        )?;
        let rows = stmt.query_map([], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// All active (`queued`/`running`/`cancelling`/`awaiting_approval`) jobs owned
    /// by a session — used by session-delete cleanup to cancel them (DELETE-4).
    /// `queued` is included so a session delete also drops jobs still waiting in
    /// the scheduler queue. Hits the `idx_background_jobs_session_status` index.
    pub fn list_active_by_session(&self, session_id: &str) -> Result<Vec<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs
             WHERE session_id=?1 AND status IN ('queued','running','cancelling','awaiting_approval')",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// R4 owner-plane panel: a session's jobs for the background-jobs panel —
    /// active jobs first (`completed_at IS NULL`), then recent terminal jobs
    /// newest-first — capped at `limit`. Unlike [`list_active_by_session`] (which
    /// is active-only for cleanup), this includes terminal rows so the panel can
    /// show "在跑/最近作业". Cap keeps a long-lived session's history bounded; the
    /// active set is small (bounded by the concurrency cap) so the cap never
    /// drops a running job.
    pub fn list_for_session(&self, session_id: &str, limit: usize) -> Result<Vec<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs
             WHERE session_id=?1
             ORDER BY (completed_at IS NOT NULL), created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Return the set of all `result_path` values currently referenced by the
    /// DB. Used by orphan spool-file cleanup to know which files on disk are
    /// still "owned" by a row.
    pub fn list_all_spool_paths(&self) -> Result<HashSet<String>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt =
            conn.prepare("SELECT result_path FROM background_jobs WHERE result_path IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = HashSet::new();
        for r in rows {
            out.insert(r?);
        }
        Ok(out)
    }

    /// Delete terminal job rows whose `completed_at < cutoff_ts` along with
    /// their on-disk spool files. Only touches `completed / failed /
    /// interrupted / timed_out` rows — `running` jobs are never purged even if
    /// they appear stale (they're handled by replay).
    ///
    /// Uses `DELETE ... RETURNING` so the row delete is atomic with the id/path
    /// capture — a single table scan instead of SELECT + DELETE. Spool-file
    /// cleanup runs outside the mutex after the DB row is gone; any leftover
    /// file on failure is caught by the orphan sweep in `retention.rs`.
    pub fn purge_terminal_older_than(&self, cutoff_ts: i64) -> Result<PurgeStats> {
        let mut stats = PurgeStats::default();

        let deleted_rows: Vec<(String, Option<String>)> = {
            let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            let sql = format!(
                "DELETE FROM background_jobs
                 WHERE status IN ({})
                   AND completed_at IS NOT NULL
                   AND completed_at < ?1
                 RETURNING job_id, result_path",
                JobStatus::TERMINAL_STATUS_SQL_LIST
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![cutoff_ts], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        stats.rows_deleted = deleted_rows.len() as u64;
        if deleted_rows.is_empty() {
            return Ok(stats);
        }

        remove_spool_files(&deleted_rows, &mut stats);
        Ok(stats)
    }

    /// Delete **every** job row owned by `session_id` (any status) and remove
    /// their on-disk spool files. Called by the session cleanup watcher on
    /// **purge** (incognito burn-on-close) so a burned session leaves no job row
    /// or spooled output behind. Distinct from `cancel_jobs_for_session` (stops
    /// only *active* jobs) and from the age-based retention sweep. Active jobs
    /// are cancelled first by the watcher; deleting a still-settling row here is
    /// safe — the runner's later `update_terminal` no-ops on the missing row, and
    /// any spool file written in that race is caught by the orphan sweep in
    /// `retention.rs`. Epic E (INCOG-2).
    pub fn purge_jobs_for_session(&self, session_id: &str) -> Result<PurgeStats> {
        let mut stats = PurgeStats::default();

        let deleted_rows: Vec<(String, Option<String>)> = {
            let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "DELETE FROM background_jobs WHERE session_id = ?1
                 RETURNING job_id, result_path",
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        stats.rows_deleted = deleted_rows.len() as u64;
        if deleted_rows.is_empty() {
            return Ok(stats);
        }

        remove_spool_files(&deleted_rows, &mut stats);
        Ok(stats)
    }

    /// All terminal jobs that have not yet been injected — used by startup
    /// replay to push pending notifications back into their parent sessions.
    pub fn list_pending_injection(&self) -> Result<Vec<BackgroundJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let sql = format!(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested, kind,
                    subagent_run_id, group_id
             FROM background_jobs
             WHERE status IN ({})
               AND injected=0",
            JobStatus::TERMINAL_STATUS_SQL_LIST
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<BackgroundJob> {
    let injected: i32 = row.get(12)?;
    let incognito: i32 = row.get(15)?;
    let cancel_requested: i32 = row.get(17)?;
    let status_str: String = row.get(6)?;
    let status = JobStatus::parse(&status_str).unwrap_or_else(|| {
        crate::app_warn!(
            "async_jobs",
            "row_to_job",
            "Unknown status '{}' in DB; defaulting to Interrupted",
            status_str
        );
        JobStatus::Interrupted
    });
    // `kind` (index 18); legacy/unknown values fall back to `Tool` (the only
    // kind written before R1) so a stale row never breaks load.
    let kind_str: String = row.get(18)?;
    let kind = JobKind::parse(&kind_str).unwrap_or(JobKind::Tool);
    Ok(BackgroundJob {
        job_id: row.get(0)?,
        kind,
        // `subagent_run_id` (index 19, R6) — FK for kind=subagent projections.
        subagent_run_id: row.get(19)?,
        // `group_id` (index 20, R5) — owning Group's job_id for fan-out children.
        group_id: row.get(20)?,
        session_id: row.get(1)?,
        agent_id: row.get(2)?,
        tool_name: row.get(3)?,
        tool_call_id: row.get(4)?,
        args_json: row.get(5)?,
        status,
        result_preview: row.get(7)?,
        result_path: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        completed_at: row.get(11)?,
        injected: injected != 0,
        origin: row.get(13)?,
        approval_origin: row.get(14)?,
        incognito: incognito != 0,
        pid: row.get(16)?,
        cancel_requested: cancel_requested != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job(id: &str) -> BackgroundJob {
        BackgroundJob {
            job_id: id.to_string(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: None,
            agent_id: None,
            tool_name: "exec".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: 0,
            completed_at: None,
            injected: false,
            origin: "explicit".into(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        }
    }

    /// A `background_jobs` table from a prior schema (here: pre-R1, missing the
    /// `kind` column and the A-7 approval columns) must be rebuilt on open so
    /// the current-shape INSERT/SELECT succeeds — otherwise every spawn fails
    /// with "no such column" on upgrade. The probe targets `group_id` (newest
    /// col, R5), so any pre-R5 table is dropped and rebuilt.
    #[test]
    fn open_rebuilds_table_missing_kind_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background_jobs.db");
        // Simulate a stale schema (no kind / approval columns) + a stale row.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE background_jobs (
                    job_id TEXT PRIMARY KEY, session_id TEXT, agent_id TEXT,
                    tool_name TEXT NOT NULL, tool_call_id TEXT, args_json TEXT NOT NULL,
                    status TEXT NOT NULL, result_preview TEXT, result_path TEXT, error TEXT,
                    created_at INTEGER NOT NULL, completed_at INTEGER,
                    injected INTEGER NOT NULL DEFAULT 0,
                    origin TEXT NOT NULL DEFAULT 'explicit'
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO background_jobs (job_id, tool_name, args_json, status, created_at)
                 VALUES ('old', 'exec', '{}', 'completed', 0)",
                [],
            )
            .unwrap();
        }
        let db = JobsDB::open(&path).expect("open must rebuild stale schema");
        db.insert(&sample_job("new"))
            .expect("insert must succeed after rebuild with kind column");
        // Policy is drop-and-rebuild, not migrate — the stale row is gone.
        assert!(db.load("old").unwrap().is_none());
        let loaded = db.load("new").unwrap().expect("new row present");
        assert_eq!(loaded.kind, JobKind::Tool, "default kind round-trips");
    }

    #[test]
    fn set_pid_and_cancel_requested_roundtrip_on_active_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        db.insert(&sample_job("j")).unwrap();
        // pid (I3) lands on the active row.
        assert!(db.set_pid("j", 4242).unwrap());
        assert_eq!(db.load("j").unwrap().unwrap().pid, Some(4242));
        // cross-process cancel flag (I4) roundtrips.
        assert!(!db.is_cancel_requested("j").unwrap());
        assert!(db.set_cancel_requested("j").unwrap());
        assert!(db.is_cancel_requested("j").unwrap());
        // Both setters no-op once the row is terminal (guard: active statuses only).
        db.update_terminal("j", JobStatus::Completed, None, None, None, 1)
            .unwrap();
        assert!(!db.set_pid("j", 9999).unwrap());
        assert!(!db.set_cancel_requested("j").unwrap());
    }

    #[test]
    fn list_for_session_orders_active_first_then_recent_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        let mk = |id: &str, sid: &str, created: i64, terminal: bool| {
            let mut j = sample_job(id);
            j.session_id = Some(sid.to_string());
            j.created_at = created;
            if terminal {
                j.status = JobStatus::Completed;
                j.completed_at = Some(created + 1);
            }
            j
        };
        // s1: two active (10, 30) + two terminal (20, 40); s2 must not leak in.
        db.insert(&mk("a_old", "s1", 10, false)).unwrap();
        db.insert(&mk("t_old", "s1", 20, true)).unwrap();
        db.insert(&mk("a_new", "s1", 30, false)).unwrap();
        db.insert(&mk("t_new", "s1", 40, true)).unwrap();
        db.insert(&mk("other", "s2", 50, false)).unwrap();

        let ids: Vec<String> = db
            .list_for_session("s1", 50)
            .unwrap()
            .into_iter()
            .map(|r| r.job_id)
            .collect();
        // Active first (newest-first), then terminal (newest-first).
        assert_eq!(ids, vec!["a_new", "a_old", "t_new", "t_old"]);

        // The cap bounds the result and keeps the highest-priority rows.
        let capped = db.list_for_session("s1", 2).unwrap();
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].job_id, "a_new");
        assert_eq!(capped[1].job_id, "a_old");
    }

    #[test]
    fn mark_running_promotes_only_queued_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        // A queued row promotes to running exactly once (scheduler grant).
        let mut q = sample_job("q");
        q.status = JobStatus::Queued;
        db.insert(&q).unwrap();
        assert!(db.mark_running("q").unwrap(), "queued -> running");
        assert_eq!(db.load("q").unwrap().unwrap().status, JobStatus::Running);
        // Already running → not re-promoted (guard WHERE status='queued').
        assert!(!db.mark_running("q").unwrap());
        // A never-queued (running) row cannot be promoted either.
        db.insert(&sample_job("r")).unwrap();
        assert!(!db.mark_running("r").unwrap());
    }

    #[test]
    fn queued_rows_are_active_and_can_settle_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        let mut q = sample_job("q");
        q.status = JobStatus::Queued;
        q.session_id = Some("s1".into());
        db.insert(&q).unwrap();
        // Startup replay (list_running) must include queued rows so they recover.
        assert!(db.list_running().unwrap().iter().any(|j| j.job_id == "q"));
        // Session-delete cleanup (list_active_by_session) must include them too.
        assert!(db
            .list_active_by_session("s1")
            .unwrap()
            .iter()
            .any(|j| j.job_id == "q"));
        // update_terminal must be able to settle a queued row (cancel / restart).
        assert!(db
            .update_terminal("q", JobStatus::Cancelled, None, None, Some("x"), 1)
            .unwrap());
        assert_eq!(db.load("q").unwrap().unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn delete_removes_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        db.insert(&sample_job("d")).unwrap();
        db.delete("d").unwrap();
        assert!(db.load("d").unwrap().is_none());
    }

    /// A current-shape table must NOT be dropped on reopen (no spurious data loss).
    #[test]
    fn open_preserves_current_schema_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async_jobs.db");
        {
            let db = JobsDB::open(&path).unwrap();
            db.insert(&sample_job("keep")).unwrap();
        }
        let db = JobsDB::open(&path).expect("reopen current schema");
        assert!(
            db.load("keep").unwrap().is_some(),
            "current-schema table must survive reopen"
        );
    }

    /// R6: the subagent projection status sync is one-way (status/completed_at
    /// only), round-trips the FK, and freezes once terminal (a late/duplicate
    /// sync can't reopen a settled projection).
    fn subagent_projection(job_id: &str, run_id: &str) -> BackgroundJob {
        let mut j = sample_job(job_id);
        j.kind = JobKind::Subagent;
        j.subagent_run_id = Some(run_id.to_string());
        j.tool_name = "subagent:helper".into();
        j.injected = true; // projections never inject via the tool path
        j
    }

    #[test]
    fn subagent_projection_status_sync_is_one_way_and_frozen_when_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        db.insert(&subagent_projection("proj1", "run_abc")).unwrap();
        // The FK + kind round-trip through the row mapper.
        let loaded = db.load("proj1").unwrap().unwrap();
        assert_eq!(loaded.kind, JobKind::Subagent);
        assert_eq!(loaded.subagent_run_id.as_deref(), Some("run_abc"));
        // running -> completed syncs status + completed_at.
        assert!(db
            .update_subagent_projection_status("run_abc", JobStatus::Completed, Some(123))
            .unwrap());
        let loaded = db.load("proj1").unwrap().unwrap();
        assert_eq!(loaded.status, JobStatus::Completed);
        assert_eq!(loaded.completed_at, Some(123));
        // Terminal is frozen: a stray later sync must NOT reopen it.
        assert!(!db
            .update_subagent_projection_status("run_abc", JobStatus::Cancelled, Some(456))
            .unwrap());
        assert_eq!(
            db.load("proj1").unwrap().unwrap().status,
            JobStatus::Completed
        );
        // Unknown run id is a no-op (foreground / unprojected run).
        assert!(!db
            .update_subagent_projection_status("nope", JobStatus::Running, None)
            .unwrap());
    }

    #[test]
    fn subagent_projection_cancel_never_stamps_error() {
        // R6 review fix: the cancel path marks a projection `cancelling` with a
        // None error (status-only), and the terminal sync to Cancelled must
        // leave `error` None — a projection must never hold run content.
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        db.insert(&subagent_projection("proj_c", "run_c")).unwrap();
        // Mirrors cancel_job's kind=Subagent branch: mark_cancelling(_, None).
        assert!(db.mark_cancelling("proj_c", None).unwrap());
        let row = db.load("proj_c").unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Cancelling);
        assert!(
            row.error.is_none(),
            "cancelling a subagent projection must not write the error column"
        );
        // Run settles Killed → sync maps to Cancelled; error stays None.
        assert!(db
            .update_subagent_projection_status("run_c", JobStatus::Cancelled, Some(9))
            .unwrap());
        let row = db.load("proj_c").unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Cancelled);
        assert!(
            row.error.is_none(),
            "terminal projection still holds no error"
        );
    }

    #[test]
    fn subagent_projection_sync_is_scoped_to_subagent_kind() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        // A tool job is never touched by the subagent sync even if its
        // subagent_run_id column somehow matched (scoped `kind='subagent'`).
        let mut tool = sample_job("tool1");
        tool.subagent_run_id = Some("shared".into());
        db.insert(&tool).unwrap();
        assert!(!db
            .update_subagent_projection_status("shared", JobStatus::Completed, Some(1))
            .unwrap());
        assert_eq!(
            db.load("tool1").unwrap().unwrap().status,
            JobStatus::Running,
            "tool job must be untouched by the subagent sync"
        );
    }

    // ── R5 Group fan-out ────────────────────────────────────────────────────

    fn group_row(job_id: &str, session: &str) -> BackgroundJob {
        let mut j = sample_job(job_id);
        j.kind = JobKind::Group;
        j.tool_name = "subagent:batch".into();
        j.args_json = "{\"sealed\":false}".into();
        j.session_id = Some(session.into());
        j.injected = true; // the group fires its own merged injection
        j
    }

    fn group_child(job_id: &str, run_id: &str, group_id: &str) -> BackgroundJob {
        let mut j = subagent_projection(job_id, run_id);
        j.group_id = Some(group_id.to_string());
        j
    }

    #[test]
    fn group_children_and_group_id_lookup_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        db.insert(&group_row("g1", "s1")).unwrap();
        db.insert(&group_child("c1", "run1", "g1")).unwrap();
        db.insert(&group_child("c2", "run2", "g1")).unwrap();
        // A subagent projection in a *different* group must not leak in.
        db.insert(&group_child("c3", "run3", "other")).unwrap();

        let mut kids: Vec<String> = db
            .group_children("g1")
            .unwrap()
            .into_iter()
            .map(|j| j.job_id)
            .collect();
        kids.sort();
        assert_eq!(kids, vec!["c1".to_string(), "c2".to_string()]);
        // group_id is resolvable from a child run id (status-sync choke point).
        assert_eq!(
            db.group_id_for_subagent_run("run1").unwrap().as_deref(),
            Some("g1")
        );
        assert_eq!(db.group_id_for_subagent_run("nope").unwrap(), None);
    }

    #[test]
    fn claim_group_completion_is_single_winner() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        db.insert(&group_row("g", "s")).unwrap();
        // Seal flips args_json; status stays running until claimed.
        assert!(db.mark_group_sealed("g").unwrap());
        assert_eq!(
            db.load("g").unwrap().unwrap().args_json,
            "{\"sealed\":true}"
        );
        // Exactly one claim wins; a second (or a post-cancel) loses.
        assert!(db.claim_group_completion("g", 100).unwrap(), "first wins");
        assert!(
            !db.claim_group_completion("g", 200).unwrap(),
            "second loses — group already terminal"
        );
        let g = db.load("g").unwrap().unwrap();
        assert_eq!(g.status, JobStatus::Completed);
        assert_eq!(g.completed_at, Some(100));
    }

    #[test]
    fn claim_group_completion_loses_after_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let db = JobsDB::open(&dir.path().join("background_jobs.db")).unwrap();
        db.insert(&group_row("g", "s")).unwrap();
        // A cancel marks the group terminal first → the join claim must lose so
        // a cancelled batch never fires a merged injection.
        db.update_terminal("g", JobStatus::Cancelled, None, None, Some("x"), 5)
            .unwrap();
        assert!(!db.claim_group_completion("g", 100).unwrap());
        assert_eq!(db.load("g").unwrap().unwrap().status, JobStatus::Cancelled);
    }
}
