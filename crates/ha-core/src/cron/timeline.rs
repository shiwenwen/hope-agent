//! Cross-database assembly of the cron-run timeline.
//!
//! `cron.db` (run logs + jobs) and `sessions.db` (titles + unread) are two
//! independent SQLite files, so the timeline cannot be produced by a single SQL
//! join — the run rows come from `CronDB` and are hydrated with `title` /
//! `unread_count` from `SessionDB` here in Rust.

use std::sync::Arc;

use crate::cron::{CronDB, CronRunLog, CronTimelineRow};
use crate::session::SessionDB;

/// Assemble the global cron-run timeline: pull run rows from `CronDB`, filter
/// archived conversations using `SessionDB`, then paginate the visible rows.
/// `title` falls back to `job_name` and `unread_count` to `0` for runs whose
/// session row is missing (purged). Missing rows intentionally remain visible;
/// only an existing session with `archived_at` set is hidden.
pub fn cron_run_timeline(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<Vec<CronTimelineRow>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let target_len = offset.saturating_add(limit);
    let batch_size = limit.max(50);
    let mut raw_offset = 0usize;
    let mut visible = Vec::with_capacity(target_len);

    while visible.len() < target_len {
        let mut batch = cron_db.list_run_timeline(batch_size, raw_offset)?;
        if batch.is_empty() {
            break;
        }
        let raw_count = batch.len();
        raw_offset = raw_offset.saturating_add(raw_count);
        let ids: Vec<String> = batch.iter().map(|row| row.session_id.clone()).collect();
        let state = session_db.cron_session_read_state(&ids)?;

        batch.retain_mut(|row| match state.get(&row.session_id) {
            Some((_title, _unread, true)) => false,
            Some((title, unread, false)) => {
                row.title = title.clone().or_else(|| Some(row.job_name.clone()));
                row.unread_count = *unread;
                true
            }
            None => {
                row.title = Some(row.job_name.clone());
                row.unread_count = 0;
                true
            }
        });
        visible.extend(batch);
        if raw_count < batch_size {
            break;
        }
    }

    Ok(visible.into_iter().skip(offset).take(limit).collect())
}

/// Per-job run history with the same archive visibility and visible-row
/// pagination as the global timeline.
pub fn visible_cron_run_logs(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job_id: &str,
    limit: usize,
    offset: usize,
) -> anyhow::Result<Vec<CronRunLog>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let target_len = offset.saturating_add(limit);
    let batch_size = limit.max(50);
    let mut raw_offset = 0usize;
    let mut visible = Vec::with_capacity(target_len);

    while visible.len() < target_len {
        let mut batch = cron_db.get_run_logs(job_id, batch_size, raw_offset)?;
        if batch.is_empty() {
            break;
        }
        let raw_count = batch.len();
        raw_offset = raw_offset.saturating_add(raw_count);
        let ids: Vec<String> = batch.iter().map(|row| row.session_id.clone()).collect();
        let state = session_db.cron_session_read_state(&ids)?;
        batch.retain(|row| !matches!(state.get(&row.session_id), Some((_title, _unread, true))));
        visible.extend(batch);
        if raw_count < batch_size {
            break;
        }
    }

    Ok(visible.into_iter().skip(offset).take(limit).collect())
}

/// Permanently delete one conversation and any Cron run-log rows that point at
/// it. The run logs must go first: timeline hydration intentionally keeps
/// missing Session rows for legacy/purged history, so leaving them behind would
/// make a user-deleted conversation reappear as an unopenable shell.
pub fn delete_conversation_and_run_logs(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    session_id: &str,
) -> anyhow::Result<()> {
    cron_db.delete_run_logs_for_session(session_id)?;
    session_db.delete_session(session_id)
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
