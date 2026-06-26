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
