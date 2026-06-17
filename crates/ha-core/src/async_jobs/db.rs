use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::sync::Mutex;

use super::types::{AsyncJob, AsyncJobStatus};

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

/// SQLite-backed persistence for async tool jobs.
///
/// Independent of `session.db` to keep the hot chat path lock-free; mirrors
/// the layout used by `cron::CronDB` and `recap` (see `paths::async_jobs_db_path`).
pub struct AsyncJobsDB {
    pub(crate) conn: Mutex<Connection>,
}

impl AsyncJobsDB {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open async_jobs DB at {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Schema evolution for this rebuildable cache. The approval/governance
        // columns (approval_origin / incognito / pid / cancel_requested) are
        // referenced by every INSERT/SELECT below; an `async_tool_jobs` table
        // from a prior version lacks them, and `CREATE TABLE IF NOT EXISTS`
        // would NOT add them — every async spawn would then fail with "no such
        // column". Project policy is "no migration — drop and rebuild": this DB
        // is a pure cache (terminal rows are advisory, in-flight rows are
        // marked interrupted on restart regardless), so on a stale schema we
        // drop the table and let the CREATE below rebuild the current shape.
        // A failing probe means the table is either absent (DROP is a no-op) or
        // stale (DROP clears it); a current table passes and is untouched.
        if conn
            .prepare("SELECT approval_origin FROM async_tool_jobs LIMIT 0")
            .is_err()
        {
            conn.execute_batch("DROP TABLE IF EXISTS async_tool_jobs;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS async_tool_jobs (
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
                cancel_requested INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_async_jobs_session_status
                ON async_tool_jobs(session_id, status);
            CREATE INDEX IF NOT EXISTS idx_async_jobs_status_injected
                ON async_tool_jobs(status, injected);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, job: &AsyncJob) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO async_tool_jobs (
                job_id, session_id, agent_id, tool_name, tool_call_id,
                args_json, status, result_preview, result_path, error,
                created_at, completed_at, injected, origin,
                approval_origin, incognito, pid, cancel_requested
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
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
            ],
        )?;
        Ok(())
    }

    pub fn update_terminal(
        &self,
        job_id: &str,
        status: AsyncJobStatus,
        result_preview: Option<&str>,
        result_path: Option<&str>,
        error: Option<&str>,
        completed_at: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "UPDATE async_tool_jobs
                SET status=?1, result_preview=?2, result_path=?3, error=?4, completed_at=?5
                WHERE job_id=?6
                  AND status IN ('running','cancelling')",
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
            "UPDATE async_tool_jobs
                SET status=?1, error=COALESCE(?2, error)
                WHERE job_id=?3
                  AND status IN ('running','cancelling')",
            params![AsyncJobStatus::Cancelling.as_str(), error, job_id],
        )?;
        Ok(rows > 0)
    }

    pub fn mark_injected(&self, job_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE async_tool_jobs SET injected=1 WHERE job_id=?1",
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
            "UPDATE async_tool_jobs SET pid=?2
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
            "UPDATE async_tool_jobs SET cancel_requested=1
                WHERE job_id=?1
                  AND status IN ('running','cancelling','awaiting_approval')",
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
                "SELECT cancel_requested FROM async_tool_jobs WHERE job_id=?1",
                params![job_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(flag.unwrap_or(0) != 0)
    }

    pub fn load(&self, job_id: &str) -> Result<Option<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested
             FROM async_tool_jobs WHERE job_id=?1",
        )?;
        stmt.query_row(params![job_id], row_to_job)
            .optional()
            .map_err(Into::into)
    }

    /// All jobs whose status is still active (`running` / `cancelling` /
    /// `awaiting_approval`) — used by startup replay. `awaiting_approval` is
    /// included because a restart kills the in-memory approval channel, so the
    /// job is unrecoverable and must be marked `interrupted` like the rest.
    pub fn list_running(&self) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested
             FROM async_tool_jobs WHERE status IN ('running','cancelling','awaiting_approval')",
        )?;
        let rows = stmt.query_map([], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// All active (`running`/`cancelling`/`awaiting_approval`) jobs owned by a
    /// session — used by session-delete cleanup to cancel them (DELETE-4).
    /// Hits the `idx_async_jobs_session_status` index.
    pub fn list_active_by_session(&self, session_id: &str) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested
             FROM async_tool_jobs
             WHERE session_id=?1 AND status IN ('running','cancelling','awaiting_approval')",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_job)?;
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
            conn.prepare("SELECT result_path FROM async_tool_jobs WHERE result_path IS NOT NULL")?;
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
                "DELETE FROM async_tool_jobs
                 WHERE status IN ({})
                   AND completed_at IS NOT NULL
                   AND completed_at < ?1
                 RETURNING job_id, result_path",
                AsyncJobStatus::TERMINAL_STATUS_SQL_LIST
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
                "DELETE FROM async_tool_jobs WHERE session_id = ?1
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
    pub fn list_pending_injection(&self) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let sql = format!(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin,
                    approval_origin, incognito, pid, cancel_requested
             FROM async_tool_jobs
             WHERE status IN ({})
               AND injected=0",
            AsyncJobStatus::TERMINAL_STATUS_SQL_LIST
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

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<AsyncJob> {
    let injected: i32 = row.get(12)?;
    let incognito: i32 = row.get(15)?;
    let cancel_requested: i32 = row.get(17)?;
    let status_str: String = row.get(6)?;
    let status = AsyncJobStatus::parse(&status_str).unwrap_or_else(|| {
        crate::app_warn!(
            "async_jobs",
            "row_to_job",
            "Unknown status '{}' in DB; defaulting to Interrupted",
            status_str
        );
        AsyncJobStatus::Interrupted
    });
    Ok(AsyncJob {
        job_id: row.get(0)?,
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

    fn sample_job(id: &str) -> AsyncJob {
        AsyncJob {
            job_id: id.to_string(),
            session_id: None,
            agent_id: None,
            tool_name: "exec".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: AsyncJobStatus::Running,
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

    /// A table from before the approval/governance columns must be rebuilt on
    /// open so the current-shape (18-column) INSERT succeeds — otherwise every
    /// async spawn fails with "no such column" on upgrade.
    #[test]
    fn open_rebuilds_table_missing_approval_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async_jobs.db");
        // Simulate the pre-A-7 (14-column) schema + a stale row.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE async_tool_jobs (
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
                "INSERT INTO async_tool_jobs (job_id, tool_name, args_json, status, created_at)
                 VALUES ('old', 'exec', '{}', 'completed', 0)",
                [],
            )
            .unwrap();
        }
        let db = AsyncJobsDB::open(&path).expect("open must rebuild stale schema");
        db.insert(&sample_job("new"))
            .expect("18-column insert must succeed after rebuild");
        // Policy is drop-and-rebuild, not migrate — the stale row is gone.
        assert!(db.load("old").unwrap().is_none());
        assert!(db.load("new").unwrap().is_some());
    }

    #[test]
    fn set_pid_and_cancel_requested_roundtrip_on_active_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = AsyncJobsDB::open(&dir.path().join("async_jobs.db")).unwrap();
        db.insert(&sample_job("j")).unwrap();
        // pid (I3) lands on the active row.
        assert!(db.set_pid("j", 4242).unwrap());
        assert_eq!(db.load("j").unwrap().unwrap().pid, Some(4242));
        // cross-process cancel flag (I4) roundtrips.
        assert!(!db.is_cancel_requested("j").unwrap());
        assert!(db.set_cancel_requested("j").unwrap());
        assert!(db.is_cancel_requested("j").unwrap());
        // Both setters no-op once the row is terminal (guard: active statuses only).
        db.update_terminal("j", AsyncJobStatus::Completed, None, None, None, 1)
            .unwrap();
        assert!(!db.set_pid("j", 9999).unwrap());
        assert!(!db.set_cancel_requested("j").unwrap());
    }

    /// A current-shape table must NOT be dropped on reopen (no spurious data loss).
    #[test]
    fn open_preserves_current_schema_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async_jobs.db");
        {
            let db = AsyncJobsDB::open(&path).unwrap();
            db.insert(&sample_job("keep")).unwrap();
        }
        let db = AsyncJobsDB::open(&path).expect("reopen current schema");
        assert!(
            db.load("keep").unwrap().is_some(),
            "current-schema table must survive reopen"
        );
    }
}
