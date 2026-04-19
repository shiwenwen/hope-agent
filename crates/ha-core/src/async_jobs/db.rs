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
                origin TEXT NOT NULL DEFAULT 'explicit'
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
                created_at, completed_at, injected, origin
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
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
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE async_tool_jobs
                SET status=?1, result_preview=?2, result_path=?3, error=?4, completed_at=?5
                WHERE job_id=?6",
            params![
                status.as_str(),
                result_preview,
                result_path,
                error,
                completed_at,
                job_id
            ],
        )?;
        Ok(())
    }

    pub fn mark_injected(&self, job_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "UPDATE async_tool_jobs SET injected=1 WHERE job_id=?1",
            params![job_id],
        )?;
        Ok(())
    }

    pub fn load(&self, job_id: &str) -> Result<Option<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin
             FROM async_tool_jobs WHERE job_id=?1",
        )?;
        stmt.query_row(params![job_id], row_to_job)
            .optional()
            .map_err(Into::into)
    }

    /// All jobs whose status is still `running` — used by startup replay.
    pub fn list_running(&self) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin
             FROM async_tool_jobs WHERE status=?1",
        )?;
        let rows = stmt.query_map(params![AsyncJobStatus::Running.as_str()], row_to_job)?;
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

        for (job_id, spool_path) in &deleted_rows {
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

        Ok(stats)
    }

    /// All terminal jobs that have not yet been injected — used by startup
    /// replay to push pending notifications back into their parent sessions.
    pub fn list_pending_injection(&self) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let sql = format!(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin
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
    })
}
