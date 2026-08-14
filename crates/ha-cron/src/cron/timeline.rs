//! Cross-database assembly of the cron-run timeline.
//!
//! `cron.db` (run logs + jobs) and `sessions.db` (titles + unread) are two
//! independent SQLite files, so the timeline cannot be produced by a single SQL
//! join — the run rows come from `CronDB` and are hydrated with `title` /
//! `unread_count` from `SessionDB` here in Rust.

use std::sync::Arc;

use ha_core::cron::{CronDB, CronRunLog, CronTimelineRow};
use ha_core::session::SessionDB;

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

/// Logically delete a cron job while retaining its run logs and every linked
/// conversation, including legacy hidden Cron sessions. Historical readers can
/// therefore keep navigating completed and cancelled runs after task removal.
pub fn delete_job_and_legacy_sessions(
    cron_db: &Arc<CronDB>,
    _session_db: &Arc<SessionDB>,
    id: &str,
) -> anyhow::Result<()> {
    cron_db.delete_job(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::cron::{CronPayload, CronSchedule, NewCronJob};

    #[test]
    fn ordinary_run_sessions_keep_title_and_survive_task_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cron_db = Arc::new(CronDB::open(&dir.path().join("cron.db")).expect("cron db"));
        let session_db =
            Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("session db"));
        let job = cron_db
            .add_job(&NewCronJob {
                name: "Scheduled chat".into(),
                description: None,
                project_id: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "run".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: Some(false),
                delivery_targets: None,
                prefix_delivery_with_name: None,
                job_timeout_secs: None,
                permission_mode_override: None,
                sandbox_mode_override: None,
            })
            .expect("add job");

        let ordinary = session_db
            .create_session_with_project_and_origin(
                ha_core::agent_loader::DEFAULT_AGENT_ID,
                None,
                None,
                &ha_core::session::SessionOrigin {
                    kind: "cron".into(),
                    id: job.id.clone(),
                    label: job.name.clone(),
                },
            )
            .expect("ordinary session");
        session_db
            .update_session_title(&ordinary.id, "Ordinary run title")
            .expect("title ordinary");
        session_db
            .append_message(
                &ordinary.id,
                &ha_core::session::NewMessage::assistant("done"),
            )
            .expect("ordinary assistant");

        let legacy = session_db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .expect("legacy session");
        session_db
            .update_session_title(&legacy.id, "Legacy run title")
            .expect("title legacy");
        session_db
            .mark_session_cron(&legacy.id)
            .expect("mark legacy cron");
        session_db
            .append_message(
                &legacy.id,
                &ha_core::session::NewMessage::assistant("legacy done"),
            )
            .expect("legacy assistant");

        cron_db
            .add_running_run_log(&job.id, &ordinary.id, "2026-01-01T00:00:00Z")
            .expect("ordinary run log");
        cron_db
            .add_running_run_log(&job.id, &legacy.id, "2026-01-01T00:01:00Z")
            .expect("legacy run log");

        let state = session_db
            .cron_session_read_state(&[ordinary.id.clone(), legacy.id.clone()])
            .expect("hydrate timeline state");
        assert_eq!(
            state.get(&ordinary.id),
            Some(&(Some("Ordinary run title".to_string()), 1, false)),
            "ordinary Scheduled unread projects the regular Session watermark"
        );
        assert_eq!(
            state.get(&legacy.id),
            Some(&(Some("Legacy run title".to_string()), 1, false))
        );

        delete_job_and_legacy_sessions(&cron_db, &session_db, &job.id).expect("delete job");
        assert!(cron_db.get_job(&job.id).expect("load job").is_none());
        assert!(session_db
            .get_session(&ordinary.id)
            .expect("load ordinary")
            .is_some());
        assert!(session_db
            .get_session(&legacy.id)
            .expect("load legacy")
            .is_some());
        assert_eq!(
            cron_db
                .get_run_logs(&job.id, 10, 0)
                .expect("retained run logs")
                .len(),
            2
        );
        let timeline = cron_db.list_run_timeline(10, 0).expect("timeline");
        assert_eq!(timeline.len(), 2);
        assert!(timeline.iter().all(|row| row.job_deleted));
    }
}
