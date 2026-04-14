use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

use super::types::{AsyncJob, AsyncJobStatus};

/// SQLite-backed persistence for async tool jobs.
///
/// Independent of `session.db` to keep the hot chat path lock-free; mirrors
/// the layout used by `cron::CronDB` and `recap` (see `paths::async_jobs_db_path`).
pub struct AsyncJobsDB {
    pub(crate) conn: Mutex<Connection>,
}

impl AsyncJobsDB {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path).with_context(|| {
            format!("Failed to open async_jobs DB at {}", db_path.display())
        })?;
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
        stmt.query_row(params![job_id], row_to_job).optional().map_err(Into::into)
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

    /// All terminal jobs that have not yet been injected — used by startup
    /// replay to push pending notifications back into their parent sessions.
    pub fn list_pending_injection(&self) -> Result<Vec<AsyncJob>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT job_id, session_id, agent_id, tool_name, tool_call_id,
                    args_json, status, result_preview, result_path, error,
                    created_at, completed_at, injected, origin
             FROM async_tool_jobs
             WHERE status IN (?1, ?2, ?3, ?4)
               AND injected=0",
        )?;
        let rows = stmt.query_map(
            params![
                AsyncJobStatus::Completed.as_str(),
                AsyncJobStatus::Failed.as_str(),
                AsyncJobStatus::Interrupted.as_str(),
                AsyncJobStatus::TimedOut.as_str(),
            ],
            row_to_job,
        )?;
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
    Ok(AsyncJob {
        job_id: row.get(0)?,
        session_id: row.get(1)?,
        agent_id: row.get(2)?,
        tool_name: row.get(3)?,
        tool_call_id: row.get(4)?,
        args_json: row.get(5)?,
        status: AsyncJobStatus::parse(&status_str),
        result_preview: row.get(7)?,
        result_path: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        completed_at: row.get(11)?,
        injected: injected != 0,
        origin: row.get(13)?,
    })
}
