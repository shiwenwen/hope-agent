//! Thin scheduled Worktree orchestration over the typed SessionDB custody API.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use ha_core::cron::{
    CronDB, CronJob, CronJobStatus, CronRunLog, CronWorkspaceCleanup, CronWorkspaceMode,
    CronWorkspaceSnapshot,
};
use ha_core::session::SessionDB;
use ha_core::worktree::{
    CreateManagedWorktreeInput, ManagedWorktree, ManagedWorktreePurpose, ManagedWorktreeState,
};
use serde::{Deserialize, Serialize};

const RESOURCE_LIMIT: usize = 256;
const IDLE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronWorkspaceActionAvailability {
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronWorkspaceActions {
    pub take_over: CronWorkspaceActionAvailability,
    pub return_to_task: CronWorkspaceActionAvailability,
    pub return_and_resume: CronWorkspaceActionAvailability,
    pub discard: CronWorkspaceActionAvailability,
    pub archive: CronWorkspaceActionAvailability,
    pub restore: CronWorkspaceActionAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronWorkspaceResource {
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_log_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub mode: CronWorkspaceMode,
    pub workspace_status: String,
    pub worktree: ManagedWorktree,
    pub actions: CronWorkspaceActions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronWorkspaceActionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<CronWorkspaceResource>,
    pub resumed: bool,
}

fn no(code: &str) -> CronWorkspaceActionAvailability {
    available(false, code)
}

fn available(allowed: bool, code: &str) -> CronWorkspaceActionAvailability {
    CronWorkspaceActionAvailability {
        allowed,
        reason_code: (!allowed).then(|| code.to_string()),
    }
}

fn terminal(run: Option<&CronRunLog>) -> bool {
    run.is_none_or(|run| run.finished_at.is_some() && run.status != "running")
}

fn resource_for(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    worktree: ManagedWorktree,
    exact_run: Option<&CronRunLog>,
) -> Result<CronWorkspaceResource> {
    let task = worktree
        .scheduled_task_id
        .clone()
        .ok_or_else(|| anyhow!("workspace_not_owner"))?;
    let runs = if exact_run.is_none() {
        cron_db.get_run_logs(&task, RESOURCE_LIMIT, 0)?
    } else {
        Vec::new()
    };
    let run = exact_run.or_else(|| {
        runs.iter()
            .find(|run| run.worktree_id.as_deref() == Some(&worktree.id))
    });
    let job = cron_db.get_job_including_deleted(&task)?;
    let deleted = job.as_ref().map(|(_, deleted)| *deleted).unwrap_or(true);
    let paused = deleted || job.is_some_and(|(job, _)| job.status == CronJobStatus::Paused);
    let idle = worktree.runtime_run_id.is_none();
    let handed = worktree.handoff_session_id.is_some();
    let active = worktree.state == ManagedWorktreeState::Active;
    let run_terminal = terminal(run);
    let fixed_session = worktree
        .handoff_session_id
        .as_ref()
        .or_else(|| exact_run.map(|run| &run.session_id))
        .or_else(|| {
            (worktree.purpose == ManagedWorktreePurpose::ScheduledRun)
                .then_some(worktree.owner_session_id.as_ref())
                .flatten()
        });
    let mut session = fixed_session
        .map(|id| session_db.get_session(id))
        .transpose()?
        .flatten();
    if fixed_session.is_none() {
        for candidate in run
            .into_iter()
            .chain(&runs)
            .filter(|run| run.worktree_id.as_deref() == Some(&worktree.id))
        {
            if let Some(row) = session_db
                .get_session(&candidate.session_id)?
                .filter(|row| !row.incognito)
            {
                session = Some(row);
                break;
            }
        }
    }
    session = session.filter(|row| !row.incognito);
    let session_idle = session
        .as_ref()
        .is_some_and(|row| session_workspace_idle(session_db, &row.id, None).unwrap_or(false));
    let unavailable = if session.is_some() {
        "workspace_session_busy"
    } else {
        "workspace_session_missing"
    };
    let (mode, actions) = match worktree.purpose {
        ManagedWorktreePurpose::ScheduledRun => (
            CronWorkspaceMode::Fresh,
            CronWorkspaceActions {
                take_over: no("workspace_wrong_mode"),
                return_to_task: no("workspace_wrong_mode"),
                return_and_resume: no("workspace_wrong_mode"),
                discard: available(idle && active && run_terminal && session_idle, unavailable),
                archive: available(idle && active && run_terminal && session_idle, unavailable),
                restore: available(
                    idle && worktree.state == ManagedWorktreeState::Archived && session_idle,
                    unavailable,
                ),
            },
        ),
        ManagedWorktreePurpose::ScheduledTask => (
            CronWorkspaceMode::Persistent,
            CronWorkspaceActions {
                take_over: available(idle && active && !handed && session_idle, unavailable),
                return_to_task: available(idle && handed && session_idle, unavailable),
                return_and_resume: if deleted {
                    no("workspace_task_deleted")
                } else {
                    available(idle && handed && session_idle, unavailable)
                },
                discard: available(idle && !handed && paused, "workspace_task_not_paused"),
                archive: no("workspace_wrong_mode"),
                restore: no("workspace_wrong_mode"),
            },
        ),
        _ => bail!("workspace_wrong_mode"),
    };
    Ok(CronWorkspaceResource {
        job_id: task,
        run_log_id: run.map(|run| run.id),
        session_id: session.map(|row| row.id),
        mode,
        workspace_status: run
            .and_then(|run| run.workspace_status.clone())
            .unwrap_or_else(|| if handed { "handed_off" } else { "ready" }.to_string()),
        worktree,
        actions,
    })
}

pub fn workspace_resources(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job_id: Option<&str>,
) -> Result<Vec<CronWorkspaceResource>> {
    session_db
        .list_scheduled_worktrees(job_id, RESOURCE_LIMIT)?
        .into_iter()
        .map(|row| resource_for(cron_db, session_db, row, None))
        .collect()
}

pub fn workspace_resource_for_run(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    run_log_id: i64,
) -> Result<Option<CronWorkspaceResource>> {
    let Some(run) = cron_db.get_run_log(run_log_id)? else {
        return Ok(None);
    };
    let Some(id) = run.worktree_id.clone() else {
        return Ok(None);
    };
    session_db
        .get_managed_worktree(&id)?
        .map(|row| resource_for(cron_db, session_db, row, Some(&run)))
        .transpose()
}

fn session_workspace_idle(
    session_db: &SessionDB,
    session_id: &str,
    ignored_turn_id: Option<&str>,
) -> Result<bool> {
    if ha_core::chat_engine::active_turn::current(session_id)
        .is_some_and(|turn| Some(turn.turn_id.as_str()) != ignored_turn_id)
        || !ha_core::async_jobs::JobManager::list_active_work_by_session(session_id)?.is_empty()
        || !session_db.scheduled_session_durably_idle(session_id, ignored_turn_id)?
    {
        return Ok(false);
    }
    let registry = match ha_core::process_registry::get_registry().try_lock() {
        Ok(registry) => registry,
        Err(_) => return Ok(false),
    };
    Ok(registry
        .list_running_ids_for_parent_session(Some(session_id))
        .is_empty())
}

fn require_idle(session_db: &SessionDB, session_id: &str) -> Result<()> {
    if !session_workspace_idle(session_db, session_id, None)? {
        bail!("workspace_session_busy");
    }
    Ok(())
}

fn done(resource: Option<CronWorkspaceResource>, resumed: bool) -> CronWorkspaceActionResult {
    CronWorkspaceActionResult { resource, resumed }
}

pub fn take_over_persistent_worktree(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job_id: &str,
    session_id: &str,
) -> Result<CronWorkspaceActionResult> {
    require_idle(session_db, session_id)?;
    if cron_db
        .get_job_including_deleted(job_id)?
        .is_some_and(|(_, deleted)| !deleted)
    {
        cron_db.toggle_job(job_id, false)?;
    }
    let row = session_db
        .get_scheduled_task_worktree(job_id)?
        .ok_or_else(|| anyhow!("workspace_missing"))?;
    let row = session_db.take_over_scheduled_worktree(&row.id, job_id, session_id)?;
    let resource = resource_for(cron_db, session_db, row, None)?;
    Ok(done(Some(resource), false))
}

pub fn return_persistent_worktree(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job_id: &str,
    session_id: &str,
    resume: bool,
) -> Result<CronWorkspaceActionResult> {
    require_idle(session_db, session_id)?;
    let deleted = cron_db
        .get_job_including_deleted(job_id)?
        .map(|(_, deleted)| deleted)
        .unwrap_or(true);
    if resume && deleted {
        bail!("workspace_task_deleted");
    }
    let row = session_db
        .get_scheduled_task_worktree(job_id)?
        .ok_or_else(|| anyhow!("workspace_missing"))?;
    let row = session_db.return_scheduled_worktree(&row.id, job_id, session_id)?;
    if resume {
        cron_db.toggle_job(job_id, true)?;
    }
    let resource = resource_for(cron_db, session_db, row, None)?;
    Ok(done(Some(resource), resume))
}

pub fn discard_run_worktree(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    run_log_id: i64,
    session_id: &str,
) -> Result<CronWorkspaceActionResult> {
    let run = cron_db
        .get_run_log(run_log_id)?
        .ok_or_else(|| anyhow!("workspace_run_not_found"))?;
    if run.session_id != session_id || !terminal(Some(&run)) {
        bail!("workspace_busy");
    }
    require_idle(session_db, session_id)?;
    let id = run
        .worktree_id
        .ok_or_else(|| anyhow!("workspace_missing"))?;
    session_db.discard_scheduled_run_worktree(&id, &run.job_id, session_id)?;
    cron_db.mark_worktree_discarded(&id)?;
    Ok(done(None, false))
}

pub fn discard_persistent_worktree(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job_id: &str,
) -> Result<CronWorkspaceActionResult> {
    let row = session_db
        .get_scheduled_task_worktree(job_id)?
        .ok_or_else(|| anyhow!("workspace_missing"))?;
    if row.state == ManagedWorktreeState::Active
        && cron_db
            .get_job_including_deleted(job_id)?
            .is_some_and(|(job, deleted)| !deleted && job.status != CronJobStatus::Paused)
    {
        bail!("workspace_task_not_paused");
    }
    session_db.discard_scheduled_task_worktree(&row.id, job_id)?;
    cron_db.finish_persistent_discard(job_id, Some(&row.id))?;
    Ok(done(None, false))
}

pub fn workspace_error_code(error: &anyhow::Error) -> &'static str {
    let message = format!("{error:#}");
    "workspace_task_deleted workspace_task_not_paused workspace_policy_locked workspace_project_required workspace_base_ref_invalid workspace_conflicted workspace_path_missing workspace_repo_mismatch workspace_not_git workspace_session_busy workspace_session_missing workspace_run_not_found workspace_not_handed_off workspace_not_owner workspace_busy workspace_missing workspace_wrong_mode"
    .split_ascii_whitespace()
    .find(|code| message.contains(code))
    .unwrap_or("workspace_conflict")
}

pub struct PreparedCronWorkspace {
    mode: CronWorkspaceMode,
    cleanup: CronWorkspaceCleanup,
    row: Option<ManagedWorktree>,
    run_log_id: i64,
    session_id: String,
    turn_id: String,
    cron_db: Arc<CronDB>,
    session_db: Arc<SessionDB>,
}

/// Does this settled Fresh Worktree get removed? `DiscardIfClean` reads the
/// audit that was just taken: any file the run left behind — staged, unstaged,
/// untracked, conflicted — or any commit ahead of the base keeps it.
fn cleanup_discards(cleanup: CronWorkspaceCleanup, audit: &CronWorkspaceSnapshot) -> bool {
    match cleanup {
        CronWorkspaceCleanup::Retain => false,
        CronWorkspaceCleanup::Always => true,
        CronWorkspaceCleanup::DiscardIfClean => {
            audit.staged == 0
                && audit.unstaged == 0
                && audit.untracked == 0
                && audit.conflicted == 0
                && !audit.head_diverged
        }
    }
}

fn snapshot_identity(mode: CronWorkspaceMode, row: &ManagedWorktree) -> CronWorkspaceSnapshot {
    CronWorkspaceSnapshot {
        mode,
        worktree_id: Some(row.id.clone()),
        base_ref: row.base_ref.clone(),
        base_sha: row.base_sha.clone(),
        retained: true,
        ..Default::default()
    }
}

fn snapshot(
    mode: CronWorkspaceMode,
    row: &ManagedWorktree,
    db: &SessionDB,
) -> Result<CronWorkspaceSnapshot> {
    let live = db.snapshot_managed_worktree(&row.id)?;
    Ok(CronWorkspaceSnapshot {
        head_sha: live.head_sha,
        branch: live.branch,
        staged: live.dirty.staged_files,
        unstaged: live.dirty.unstaged_files,
        untracked: live.dirty.untracked_files,
        conflicted: live.dirty.conflicted_files,
        head_diverged: live.head_diverged,
        ..snapshot_identity(mode, row)
    })
}

pub async fn prepare_workspace(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    job: &CronJob,
    run_log_id: i64,
    session_id: &str,
    turn_id: &str,
) -> Result<PreparedCronWorkspace> {
    let mode = job.workspace_policy.mode;
    let cleanup = job.workspace_policy.cleanup;
    let prepared = |row| PreparedCronWorkspace {
        mode,
        cleanup,
        row,
        run_log_id,
        session_id: session_id.into(),
        turn_id: turn_id.into(),
        cron_db: cron_db.clone(),
        session_db: session_db.clone(),
    };
    if mode == CronWorkspaceMode::Project {
        let db = cron_db.clone();
        ha_core::blocking::run_blocking(move || {
            db.set_run_workspace(
                run_log_id,
                None,
                "project",
                Some(&CronWorkspaceSnapshot::default()),
            )
        })
        .await?;
        return Ok(prepared(None));
    }
    let input = CreateManagedWorktreeInput {
        session_id: session_id.to_string(),
        source_working_dir: None,
        label: Some(job.name.clone()),
        purpose: ManagedWorktreePurpose::Manual,
        workflow_run_id: None,
        child_session_id: None,
        base_ref: job.workspace_policy.base_ref.clone(),
        include_local_changes: false,
        bootstrap_request_id: None,
        bind_session_working_dir: true,
    };
    let row = match mode {
        CronWorkspaceMode::Fresh => {
            session_db
                .create_scheduled_run_worktree(input, &job.id, run_log_id)
                .await?
        }
        CronWorkspaceMode::Persistent => {
            let (db, task) = (session_db.clone(), job.id.clone());
            if let Some(row) =
                ha_core::blocking::run_blocking(move || db.get_scheduled_task_worktree(&task))
                    .await?
            {
                let (db, task, session, id) = (
                    session_db.clone(),
                    job.id.clone(),
                    session_id.to_string(),
                    row.id,
                );
                ha_core::blocking::run_blocking(move || {
                    db.acquire_scheduled_worktree_runtime(&id, &task, run_log_id, &session)
                })
                .await?
            } else {
                let (db, task) = (cron_db.clone(), job.id.clone());
                ha_core::blocking::run_blocking(move || {
                    db.set_persistent_workspace_lock(&task, true)
                })
                .await?;
                match session_db
                    .create_scheduled_task_worktree(input, &job.id, run_log_id)
                    .await
                {
                    Ok(row) => row,
                    Err(error) => {
                        let (sdb, cdb, task) =
                            (session_db.clone(), cron_db.clone(), job.id.clone());
                        let _ = ha_core::blocking::run_blocking(move || -> Result<()> {
                            if sdb.get_scheduled_task_worktree(&task)?.is_none() {
                                cdb.set_persistent_workspace_lock(&task, false)?;
                            }
                            Ok(())
                        })
                        .await;
                        return Err(error);
                    }
                }
            }
        }
        CronWorkspaceMode::Project => unreachable!(),
    };
    let (cdb, sdb, audit_row) = (cron_db.clone(), session_db.clone(), row.clone());
    let audit = ha_core::blocking::run_blocking(move || {
        let audit = snapshot(mode, &audit_row, &sdb)?;
        if audit.conflicted > 0 {
            bail!("workspace_conflicted");
        }
        cdb.set_run_workspace(run_log_id, Some(&audit_row.id), "running", Some(&audit))?;
        Ok(audit)
    })
    .await;
    if let Err(error) = audit {
        let (cdb, sdb, failed_row, session, turn) = (
            cron_db.clone(),
            session_db.clone(),
            row.clone(),
            session_id.to_string(),
            turn_id.to_string(),
        );
        let recovered = ha_core::blocking::run_blocking(move || {
            let identity = snapshot_identity(mode, &failed_row);
            cdb.set_run_workspace(
                run_log_id,
                Some(&failed_row.id),
                "attention",
                Some(&identity),
            )?;
            sdb.release_scheduled_runtime(&failed_row, run_log_id, &session, Some(&turn))
                .map(|_| ())
        })
        .await;
        return Err(if recovered.is_ok() {
            error
        } else {
            anyhow!("workspace_recovery_required: {error:#}")
        });
    }
    Ok(prepared(Some(row)))
}

impl PreparedCronWorkspace {
    pub async fn defer(self) -> Result<()> {
        let Some(row) = self.row else {
            return Ok(());
        };
        ha_core::blocking::run_blocking(move || {
            self.cron_db
                .set_run_workspace(self.run_log_id, Some(&row.id), "attention", None)
        })
        .await
    }

    pub async fn finalize(self) -> Result<()> {
        let Some(row) = self.row.clone() else {
            return Ok(());
        };
        let started = Instant::now();
        loop {
            let (db, session, turn) = (
                self.session_db.clone(),
                self.session_id.clone(),
                self.turn_id.clone(),
            );
            if ha_core::blocking::run_blocking(move || {
                session_workspace_idle(&db, &session, Some(&turn))
            })
            .await?
            {
                break;
            }
            if started.elapsed() >= IDLE_WAIT {
                self.defer().await?;
                bail!("workspace_session_busy");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        ha_core::blocking::run_blocking(move || {
            let mut audit = snapshot(self.mode, &row, &self.session_db)?;
            self.session_db.release_scheduled_runtime(
                &row,
                self.run_log_id,
                &self.session_id,
                Some(&self.turn_id),
            )?;
            // Per-task cleanup runs only after custody is released, and only for
            // Fresh: `discard_scheduled_run_worktree` re-checks owner, custody,
            // handoff, and session idleness inside its own transaction, so any
            // doubt fails closed to a retained Worktree plus the pending-resource
            // panel — never a half-removed one.
            let discarded = self.mode == CronWorkspaceMode::Fresh
                && cleanup_discards(self.cleanup, &audit)
                && match row.scheduled_task_id.as_deref() {
                    Some(task) => {
                        match self.session_db.discard_scheduled_run_worktree(
                            &row.id,
                            task,
                            &self.session_id,
                        ) {
                            Ok(()) => true,
                            Err(error) => {
                                app_warn!(
                                    "cron",
                                    "workspace_cleanup",
                                    "Run {} kept its Worktree; cleanup failed: {error:#}",
                                    self.run_log_id
                                );
                                false
                            }
                        }
                    }
                    None => false,
                };
            audit.retained = !discarded;
            let status = if discarded {
                "discarded"
            } else if self.mode == CronWorkspaceMode::Fresh {
                "retained"
            } else {
                "ready"
            };
            if let Err(error) =
                self.cron_db
                    .set_run_workspace(self.run_log_id, Some(&row.id), status, Some(&audit))
            {
                app_warn!(
                    "cron",
                    "workspace_audit",
                    "Run {} audit failed after custody release: {error:#}",
                    self.run_log_id
                );
            }
            Ok(())
        })
        .await
    }
}

pub async fn reconcile_workspaces(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
) -> Result<usize> {
    let (cron_db, session_db) = (cron_db.clone(), session_db.clone());
    ha_core::blocking::run_blocking(move || reconcile_workspaces_sync(&cron_db, &session_db)).await
}

fn reconcile_workspaces_sync(cron_db: &Arc<CronDB>, session_db: &Arc<SessionDB>) -> Result<usize> {
    let _ = session_db.recover_pending_scheduled_worktrees(64)?;
    for row in session_db
        .list_scheduled_worktrees(None, RESOURCE_LIMIT)?
        .into_iter()
        .filter(|row| {
            row.purpose == ManagedWorktreePurpose::ScheduledTask
                && row.state == ManagedWorktreeState::Archived
        })
    {
        let Some(task) = row.scheduled_task_id.as_deref() else {
            continue;
        };
        if let Err(error) = session_db
            .discard_scheduled_task_worktree(&row.id, task)
            .and_then(|()| cron_db.finish_persistent_discard(task, Some(&row.id)))
        {
            app_warn!(
                "cron",
                "workspace_reconcile",
                "Persistent discard {} remains pending: {error:#}",
                row.id
            );
        }
    }
    for task in cron_db.list_locked_persistent_workspaces(RESOURCE_LIMIT)? {
        if session_db.get_scheduled_task_worktree(&task)?.is_none() {
            cron_db.finish_persistent_discard(&task, None)?;
        }
    }
    let mut released = 0;
    for row in session_db.list_scheduled_runtime_worktrees(RESOURCE_LIMIT)? {
        let Some(run_id) = row
            .runtime_run_id
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok())
        else {
            continue;
        };
        let outcome = (|| -> Result<bool> {
            let run = cron_db.get_run_log(run_id)?;
            let (session, ignored) = match run.as_ref() {
                Some(run) if terminal(Some(run)) => match run.turn_id.as_deref() {
                    Some(id)
                        if session_db
                            .get_chat_turn(id)?
                            .is_some_and(|turn| turn.status.is_terminal()) =>
                    {
                        (run.session_id.clone(), Some(id.to_string()))
                    }
                    Some(_) => return Ok(false),
                    None => (run.session_id.clone(), None),
                },
                Some(_) => return Ok(false),
                None => {
                    let task = row
                        .scheduled_task_id
                        .as_deref()
                        .ok_or_else(|| anyhow!("workspace_not_owner"))?;
                    if cron_db
                        .get_job_including_deleted(task)?
                        .is_some_and(|(job, _)| job.running_at.is_some())
                    {
                        return Ok(false);
                    }
                    (
                        row.runtime_session_id
                            .clone()
                            .ok_or_else(|| anyhow!("workspace_session_missing"))?,
                        None,
                    )
                }
            };
            if !session_workspace_idle(session_db, &session, ignored.as_deref())? {
                return Ok(false);
            }
            let mode = if row.purpose == ManagedWorktreePurpose::ScheduledRun {
                CronWorkspaceMode::Fresh
            } else {
                CronWorkspaceMode::Persistent
            };
            let audit = snapshot(mode, &row, session_db);
            let status = if audit.is_err() {
                "attention"
            } else if mode == CronWorkspaceMode::Fresh {
                "retained"
            } else {
                "ready"
            };
            if run.is_some() {
                let identity = snapshot_identity(mode, &row);
                cron_db.set_run_workspace(
                    run_id,
                    Some(&row.id),
                    status,
                    Some(audit.as_ref().unwrap_or(&identity)),
                )?;
            }
            session_db.release_scheduled_runtime(&row, run_id, &session, ignored.as_deref())?;
            if let Some(turn) = ignored.as_deref() {
                ha_core::chat_engine::active_turn::force_release(&session, turn);
            }
            if run.is_none() {
                app_warn!("cron", "workspace_reconcile", "Released orphan runtime {run_id} for worktree {} after proving its task and session idle", row.id);
            }
            Ok(true)
        })();
        match outcome {
            Ok(true) => released += 1,
            Ok(false) => {}
            Err(error) => {
                let _ = cron_db.set_run_workspace(run_id, Some(&row.id), "attention", None);
                app_warn!(
                    "cron",
                    "workspace_reconcile",
                    "Worktree {} remains attention: {error:#}",
                    row.id
                );
            }
        }
    }
    Ok(released)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit(
        staged: u32,
        unstaged: u32,
        untracked: u32,
        conflicted: u32,
        diverged: bool,
    ) -> CronWorkspaceSnapshot {
        CronWorkspaceSnapshot {
            mode: CronWorkspaceMode::Fresh,
            staged,
            unstaged,
            untracked,
            conflicted,
            head_diverged: diverged,
            ..Default::default()
        }
    }

    #[test]
    fn discard_if_clean_never_removes_a_worktree_that_holds_work() {
        assert!(cleanup_discards(
            CronWorkspaceCleanup::DiscardIfClean,
            &audit(0, 0, 0, 0, false)
        ));

        // Any surviving artifact of the run keeps the Worktree: a file the user
        // may still want is worth more than the disk it occupies.
        for dirty in [
            audit(1, 0, 0, 0, false),
            audit(0, 1, 0, 0, false),
            audit(0, 0, 1, 0, false),
            audit(0, 0, 0, 1, false),
            audit(0, 0, 0, 0, true),
        ] {
            assert!(
                !cleanup_discards(CronWorkspaceCleanup::DiscardIfClean, &dirty),
                "dirty audit must be retained: {dirty:?}"
            );
        }
    }

    #[test]
    fn retain_keeps_everything_and_always_discards_everything() {
        let dirty = audit(3, 2, 1, 0, true);
        assert!(!cleanup_discards(CronWorkspaceCleanup::Retain, &dirty));
        assert!(!cleanup_discards(
            CronWorkspaceCleanup::Retain,
            &audit(0, 0, 0, 0, false)
        ));
        assert!(cleanup_discards(CronWorkspaceCleanup::Always, &dirty));
    }
}
