//! Cross-database assembly of the cron-run timeline.
//!
//! `cron.db` (run logs + jobs) and `sessions.db` (titles + unread) are two
//! independent SQLite files, so the timeline cannot be produced by a single SQL
//! join — the run rows come from `CronDB` and are hydrated with `title` /
//! `unread_count` from `SessionDB` here in Rust.

use std::sync::Arc;

use crate::cron::{CronDB, CronTimelineRow};
use crate::session::SessionDB;

/// Assemble the global cron-run timeline: pull the run rows from `CronDB`
/// (newest-first, paginated), then hydrate `title` + `unread_count` from
/// `SessionDB`. `title` falls back to `job_name` and `unread_count` to `0` for
/// runs whose session row is missing (purged).
pub fn cron_run_timeline(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<Vec<CronTimelineRow>> {
    let mut rows = cron_db.list_run_timeline(limit, offset)?;
    if rows.is_empty() {
        return Ok(rows);
    }
    let ids: Vec<String> = rows.iter().map(|r| r.session_id.clone()).collect();
    let state = session_db.cron_session_read_state(&ids)?;
    for r in &mut rows {
        match state.get(&r.session_id) {
            Some((title, unread)) => {
                r.title = title.clone().or_else(|| Some(r.job_name.clone()));
                r.unread_count = *unread;
            }
            None => {
                r.title = Some(r.job_name.clone());
                r.unread_count = 0;
            }
        }
    }
    Ok(rows)
}

/// Delete a cron job AND its run conversations. Cron run sessions live in
/// `sessions.db` but are hidden from the main sidebar / search, so once the job
/// (and its CASCADE-deleted `cron_run_logs`) is gone they'd be both unreachable
/// AND a permanent orphan leak. Collect the session ids first (before the
/// cascade), delete the job, then purge those sessions. Session deletes are
/// best-effort so a single failure can't block removing the job.
pub fn delete_job_and_sessions(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    id: &str,
) -> anyhow::Result<()> {
    let session_ids = cron_db.session_ids_for_job(id).unwrap_or_default();
    cron_db.delete_job(id)?;
    for sid in session_ids {
        if let Err(e) = session_db.delete_session(&sid) {
            crate::app_warn!(
                "cron",
                "delete",
                "failed to delete cron run session {} of job {}: {:#}",
                sid,
                id,
                e
            );
        }
    }
    Ok(())
}
