use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use ha_core::cron::{
    compute_next_run, resolve_agent_id_for_execution, validate_schedule, validate_workspace_policy,
    CronDB, CronDeliveryTarget, CronJob, CronPayload, CronSchedule, CronWorkspaceMode,
    CronWorkspacePolicy, NewCronJob,
};
use ha_core::permission::{SandboxMode, SessionMode};
use ha_core::provider::ActiveModel;
use ha_core::session::SessionDB;
use ha_core::worktree::{preflight_managed_worktree_source, ManagedWorktreeState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CronPreflightRequest {
    Create {
        job: NewCronJob,
    },
    Update {
        job: CronJob,
        expected_revision: u64,
    },
    RunNow {
        job_id: String,
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CronPreflightOperation {
    Create,
    Update,
    RunNow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronManagedWorkspaceWritePolicy {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronPreflightSeverity {
    Blocker,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronPreflightIssueCode {
    InvalidSchedule,
    NoFutureRun,
    TaskNotFound,
    TaskDeleted,
    RevisionConflict,
    AlreadyRunning,
    NotPrimary,
    WorkspacePolicyInvalid,
    WorkspaceProjectRequired,
    ProjectMissing,
    ProjectUnavailable,
    ProjectDetached,
    ProjectArchived,
    AgentUnavailable,
    ModelUnconfigured,
    WorkspacePolicyLocked,
    WorkspaceSourceInvalid,
    WorkspaceConflicted,
    WorkspaceBusy,
    WorkspaceHandedOff,
    WorkspaceUnavailable,
    SandboxModeUnsupported,
    SandboxUnavailable,
    DeliveryUnavailable,
    DeliveryTargetInvalid,
    DeliveryAccountMissing,
    DeliveryAccountDisabled,
    DeliveryChannelMismatch,
    DeliveryTargetStale,
    RemoteWorkspaceWritesDisabled,
    ClaimConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronPreflightIssue {
    pub code: CronPreflightIssueCode,
    pub severity: CronPreflightSeverity,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronExecutionPreview {
    pub resolved_agent_id: Option<String>,
    pub project_name: Option<String>,
    pub workspace_mode: CronWorkspaceMode,
    pub base_ref: Option<String>,
    pub workspace_dirty_files: Option<u32>,
    pub effective_permission_mode: Option<SessionMode>,
    pub effective_sandbox_mode: Option<SandboxMode>,
    pub primary_model: Option<ActiveModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronPreflightReport {
    pub operation: CronPreflightOperation,
    pub checked_revision: Option<u64>,
    pub can_proceed: bool,
    pub next_runs: Vec<String>,
    pub issues: Vec<CronPreflightIssue>,
    pub execution: CronExecutionPreview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CronRunNowResult {
    Started {
        job_id: String,
        revision: u64,
        claimed_at: String,
    },
    Rejected {
        report: CronPreflightReport,
    },
}

struct Candidate {
    id: Option<String>,
    project_id: Option<String>,
    workspace: CronWorkspacePolicy,
    schedule: CronSchedule,
    payload: CronPayload,
    targets: Vec<CronDeliveryTarget>,
    permission: Option<SessionMode>,
    sandbox: Option<SandboxMode>,
}

impl Candidate {
    fn create(job: NewCronJob) -> Self {
        Self {
            id: None,
            project_id: job.project_id,
            workspace: job.workspace_policy,
            schedule: job.schedule,
            payload: job.payload,
            targets: job.delivery_targets.unwrap_or_default(),
            permission: job.permission_mode_override,
            sandbox: job.sandbox_mode_override,
        }
    }

    fn persisted(job: CronJob) -> Self {
        Self {
            id: Some(job.id),
            project_id: job.project_id,
            workspace: job.workspace_policy,
            schedule: job.schedule,
            payload: job.payload,
            targets: job.delivery_targets,
            permission: job.permission_mode_override,
            sandbox: job.sandbox_mode_override,
        }
    }
}

impl CronPreflightRequest {
    fn operation(&self) -> CronPreflightOperation {
        match self {
            Self::Create { .. } => CronPreflightOperation::Create,
            Self::Update { .. } => CronPreflightOperation::Update,
            Self::RunNow { .. } => CronPreflightOperation::RunNow,
        }
    }
}

impl CronPreflightReport {
    fn new(operation: CronPreflightOperation) -> Self {
        Self {
            operation,
            checked_revision: None,
            can_proceed: true,
            next_runs: Vec::new(),
            issues: Vec::new(),
            execution: CronExecutionPreview::default(),
        }
    }

    fn issue(&mut self, code: CronPreflightIssueCode, severity: CronPreflightSeverity) {
        self.issues.push(CronPreflightIssue { code, severity });
        if severity == CronPreflightSeverity::Blocker {
            self.can_proceed = false;
        }
    }

    fn block(&mut self, code: CronPreflightIssueCode) {
        self.issue(code, CronPreflightSeverity::Blocker);
    }

    pub fn add_blocker(&mut self, code: CronPreflightIssueCode) {
        if !self.issues.iter().any(|issue| issue.code == code) {
            self.block(code);
        }
    }

    fn warn(&mut self, code: CronPreflightIssueCode) {
        self.issue(code, CronPreflightSeverity::Warning);
    }

    fn context_problem(&mut self, code: CronPreflightIssueCode, managed: bool) {
        let severity = if managed {
            CronPreflightSeverity::Blocker
        } else {
            CronPreflightSeverity::Warning
        };
        self.issue(code, severity);
    }
}

/// Inspect persisted/configured state without creating a task, Session, run log,
/// running lease, or Worktree. Blocking reads and Git probes stay off async workers.
pub async fn evaluate_cron_preflight(
    cron_db: Arc<CronDB>,
    session_db: Arc<SessionDB>,
    request: CronPreflightRequest,
    workspace_writes: CronManagedWorkspaceWritePolicy,
) -> Result<CronPreflightReport> {
    let mut report =
        ha_core::blocking::run_blocking(move || evaluate_sync(&cron_db, &session_db, request))
            .await?;
    if workspace_writes == CronManagedWorkspaceWritePolicy::Denied
        && report.execution.workspace_mode != CronWorkspaceMode::Project
    {
        report.add_blocker(CronPreflightIssueCode::RemoteWorkspaceWritesDisabled);
    }
    let sandbox = report.execution.effective_sandbox_mode;
    let mismatch = report
        .issues
        .iter()
        .any(|issue| issue.code == CronPreflightIssueCode::SandboxModeUnsupported);
    if sandbox.is_some_and(SandboxMode::enabled) && !mismatch {
        if !ha_core::sandbox::check_sandbox_available().await.running {
            if report.operation == CronPreflightOperation::RunNow {
                report.block(CronPreflightIssueCode::SandboxUnavailable);
            } else {
                report.warn(CronPreflightIssueCode::SandboxUnavailable);
            }
        }
    }
    Ok(report)
}

fn evaluate_sync(
    cron_db: &CronDB,
    session_db: &SessionDB,
    request: CronPreflightRequest,
) -> Result<CronPreflightReport> {
    let operation = request.operation();
    let mut report = CronPreflightReport::new(operation);
    if operation == CronPreflightOperation::RunNow && !ha_core::runtime_lock::is_primary() {
        report.block(CronPreflightIssueCode::NotPrimary);
    }
    let candidate = match request {
        CronPreflightRequest::Create { job } => Some(Candidate::create(job)),
        CronPreflightRequest::Update {
            job,
            expected_revision,
        } => {
            if let Some(live) = check_live(cron_db, &job.id, expected_revision, true, &mut report)?
            {
                inspect_update_workspace_lock(cron_db, session_db, &live, &job, &mut report);
            }
            Some(Candidate::persisted(job))
        }
        CronPreflightRequest::RunNow {
            job_id,
            expected_revision,
        } => check_live(cron_db, &job_id, expected_revision, false, &mut report)?
            .map(Candidate::persisted),
    };
    let Some(candidate) = candidate else {
        return Ok(report);
    };

    report.execution.workspace_mode = candidate.workspace.mode;
    report.execution.base_ref =
        (candidate.workspace.mode != CronWorkspaceMode::Project).then(|| {
            candidate
                .workspace
                .base_ref
                .clone()
                .unwrap_or_else(|| "HEAD".into())
        });
    inspect_schedule(&candidate, operation, &mut report);
    inspect_policy(&candidate, &mut report);
    let project = resolve_project(&candidate, &mut report);
    inspect_agent(&candidate, project.as_ref(), &mut report);
    if candidate.workspace.mode == CronWorkspaceMode::Project {
        if let Some(source) = project
            .as_ref()
            .and_then(|project| project.working_dir.as_deref())
        {
            if let Ok(count) = preflight_managed_worktree_source(source, None) {
                report.execution.workspace_dirty_files = Some(count);
            }
        }
    } else {
        inspect_workspace(&candidate, project.as_ref(), session_db, &mut report);
    }
    inspect_delivery(&candidate.targets, &mut report);
    Ok(report)
}

fn check_live(
    cron_db: &CronDB,
    id: &str,
    expected_revision: u64,
    update: bool,
    report: &mut CronPreflightReport,
) -> Result<Option<CronJob>> {
    let Some((job, deleted)) = cron_db.get_job_including_deleted(id)? else {
        report.block(CronPreflightIssueCode::TaskNotFound);
        return Ok(None);
    };
    report.checked_revision = Some(job.revision);
    if deleted {
        report.block(CronPreflightIssueCode::TaskDeleted);
        return Ok(None);
    }
    if job.revision != expected_revision {
        report.block(CronPreflightIssueCode::RevisionConflict);
    }
    if job.running_at.is_some() {
        let severity = if update {
            CronPreflightSeverity::Warning
        } else {
            CronPreflightSeverity::Blocker
        };
        report.issue(CronPreflightIssueCode::AlreadyRunning, severity);
    }
    Ok(Some(job))
}

fn inspect_update_workspace_lock(
    cron_db: &CronDB,
    session_db: &SessionDB,
    live: &CronJob,
    draft: &CronJob,
    report: &mut CronPreflightReport,
) {
    if live.workspace_policy.clone().normalized() == draft.workspace_policy.clone().normalized()
        && normalized_id(live.project_id.as_deref()) == normalized_id(draft.project_id.as_deref())
    {
        return;
    }
    let locked = cron_db.workspace_policy_locked(&live.id);
    let worktree = (live.workspace_policy.mode == CronWorkspaceMode::Persistent)
        .then(|| session_db.get_scheduled_task_worktree(&live.id))
        .transpose()
        .map(Option::flatten);
    match (locked, worktree) {
        (Ok(true), _) | (Ok(false), Ok(Some(_))) => {
            report.block(CronPreflightIssueCode::WorkspacePolicyLocked)
        }
        (Ok(false), Ok(None)) => {}
        _ => report.block(CronPreflightIssueCode::WorkspaceUnavailable),
    }
}

fn normalized_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn inspect_schedule(
    candidate: &Candidate,
    operation: CronPreflightOperation,
    report: &mut CronPreflightReport,
) {
    if validate_schedule(&candidate.schedule).is_err() {
        report.block(CronPreflightIssueCode::InvalidSchedule);
        return;
    }
    let mut cursor = Utc::now();
    for _ in 0..3 {
        let Some(next) = compute_next_run(&candidate.schedule, &cursor) else {
            break;
        };
        report.next_runs.push(next.to_rfc3339());
        cursor = next;
    }
    if operation == CronPreflightOperation::Create
        && matches!(candidate.schedule, CronSchedule::At { .. })
        && report.next_runs.is_empty()
    {
        report.block(CronPreflightIssueCode::NoFutureRun);
    }
}

fn inspect_policy(candidate: &Candidate, report: &mut CronPreflightReport) {
    if let Err(error) = validate_workspace_policy(
        candidate.workspace.clone(),
        &candidate.payload,
        candidate.project_id.as_deref(),
    ) {
        let project_required = error.to_string().contains("workspace_project_required");
        let code = if project_required {
            CronPreflightIssueCode::WorkspaceProjectRequired
        } else {
            CronPreflightIssueCode::WorkspacePolicyInvalid
        };
        report.block(code);
    }
}

fn resolve_project(
    candidate: &Candidate,
    report: &mut CronPreflightReport,
) -> Option<ha_core::project::Project> {
    let id = candidate.project_id.as_deref()?;
    let managed = candidate.workspace.mode != CronWorkspaceMode::Project;
    let result = ha_core::get_project_db()
        .ok_or_else(|| anyhow::anyhow!("Project database is unavailable"))
        .and_then(|db| db.get(id));
    match result {
        Ok(Some(project)) => {
            report.execution.project_name = Some(project.name.clone());
            if project.archived {
                report.context_problem(CronPreflightIssueCode::ProjectArchived, managed);
            }
            Some(project)
        }
        Ok(None) => {
            let code = if managed {
                CronPreflightIssueCode::ProjectMissing
            } else {
                CronPreflightIssueCode::ProjectDetached
            };
            report.context_problem(code, managed);
            None
        }
        Err(_) => {
            report.context_problem(CronPreflightIssueCode::ProjectUnavailable, managed);
            None
        }
    }
}

fn inspect_agent(
    candidate: &Candidate,
    project: Option<&ha_core::project::Project>,
    report: &mut CronPreflightReport,
) {
    let CronPayload::AgentTurn { agent_id, .. } = &candidate.payload else {
        return;
    };
    let id = resolve_agent_id_for_execution(agent_id.as_deref(), project);
    report.execution.resolved_agent_id = Some(id.clone());
    let runnable = ha_core::agent_lifecycle::ensure_agent_runnable(&id);
    if runnable.is_err() {
        report.block(CronPreflightIssueCode::AgentUnavailable);
    }
    let definition = runnable
        .ok()
        .and_then(|_| ha_core::agent_loader::load_agent(&id).ok());
    let model = definition
        .as_ref()
        .map(|definition| definition.config.model.clone())
        .unwrap_or_default();
    let config = ha_core::config::cached_config();
    report.execution.primary_model = ha_core::provider::resolve_model_chain(&model, &config).0;
    if report.execution.primary_model.is_none() {
        report.block(CronPreflightIssueCode::ModelUnconfigured);
    }
    let capabilities = definition
        .as_ref()
        .map(|definition| &definition.config.capabilities);
    let permission = candidate
        .permission
        .or_else(|| capabilities?.default_session_permission_mode)
        .unwrap_or_default();
    let sandbox = candidate
        .sandbox
        .or_else(|| Some(capabilities?.effective_default_sandbox_mode()))
        .unwrap_or_default();
    report.execution.effective_permission_mode = Some(permission);
    report.execution.effective_sandbox_mode = Some(sandbox);
    if ha_core::sandbox::deployment_is_docker()
        && !ha_core::sandbox::container_sandbox_mode_supported(sandbox)
    {
        report.block(CronPreflightIssueCode::SandboxModeUnsupported);
    }
}

fn inspect_workspace(
    candidate: &Candidate,
    project: Option<&ha_core::project::Project>,
    session_db: &SessionDB,
    report: &mut CronPreflightReport,
) {
    if candidate.workspace.mode == CronWorkspaceMode::Persistent {
        let existing = candidate
            .id
            .as_deref()
            .map_or(Ok(None), |id| session_db.get_scheduled_task_worktree(id));
        match existing {
            Err(_) => {
                report.block(CronPreflightIssueCode::WorkspaceUnavailable);
                return;
            }
            Ok(Some(row)) => {
                if row.handoff_session_id.is_some() {
                    report.block(CronPreflightIssueCode::WorkspaceHandedOff);
                }
                if row.runtime_run_id.is_some() || row.runtime_session_id.is_some() {
                    report.block(CronPreflightIssueCode::WorkspaceBusy);
                }
                if row.state != ManagedWorktreeState::Active {
                    report.block(CronPreflightIssueCode::WorkspaceUnavailable);
                }
                match session_db.snapshot_managed_worktree(&row.id) {
                    Ok(snapshot) => {
                        report.execution.workspace_dirty_files = Some(snapshot.dirty.changed_files);
                        if snapshot.dirty.conflicted_files > 0 {
                            report.block(CronPreflightIssueCode::WorkspaceConflicted);
                        }
                    }
                    Err(_) => report.block(CronPreflightIssueCode::WorkspaceSourceInvalid),
                }
                return;
            }
            Ok(None) => {}
        }
    }
    let Some(source) = project.and_then(|project| project.working_dir.as_deref()) else {
        return;
    };
    match preflight_managed_worktree_source(source, candidate.workspace.base_ref.as_deref()) {
        Ok(count) => report.execution.workspace_dirty_files = Some(count),
        Err(_) => report.block(CronPreflightIssueCode::WorkspaceSourceInvalid),
    }
}

fn inspect_delivery(targets: &[CronDeliveryTarget], report: &mut CronPreflightReport) {
    if targets.is_empty() {
        return;
    }
    let Some(db) = ha_core::get_channel_db() else {
        report.block(CronPreflightIssueCode::DeliveryUnavailable);
        return;
    };
    let Some(registry) = ha_core::get_channel_registry() else {
        report.block(CronPreflightIssueCode::DeliveryUnavailable);
        return;
    };
    let config = ha_core::config::cached_config();
    for target in targets {
        if target.stale {
            report.block(CronPreflightIssueCode::DeliveryTargetStale);
        }
        match db.conversation_exists(
            &target.channel_id,
            &target.account_id,
            &target.chat_id,
            target.thread_id.as_deref(),
        ) {
            Err(_) => {
                report.block(CronPreflightIssueCode::DeliveryUnavailable);
                continue;
            }
            Ok(false) => {
                report.block(CronPreflightIssueCode::DeliveryTargetInvalid);
                continue;
            }
            Ok(true) => {}
        }
        let Some(account) = config.channels.find_account(&target.account_id) else {
            report.block(CronPreflightIssueCode::DeliveryAccountMissing);
            continue;
        };
        if !account.enabled {
            report.block(CronPreflightIssueCode::DeliveryAccountDisabled);
        }
        if account.channel_id.to_string() != target.channel_id {
            report.block(CronPreflightIssueCode::DeliveryChannelMismatch);
        }
        if registry.get_plugin(&account.channel_id).is_none() {
            report.block(CronPreflightIssueCode::DeliveryUnavailable);
        }
    }
}

/// Re-check live state and claim the exact owner revision before returning
/// `Started`; the existing ordinary ChatTurn executor consumes the lease.
pub async fn start_cron_run_now(
    cron_db: Arc<CronDB>,
    session_db: Arc<SessionDB>,
    job_id: String,
    expected_revision: u64,
    workspace_writes: CronManagedWorkspaceWritePolicy,
) -> Result<CronRunNowResult> {
    let request = CronPreflightRequest::RunNow {
        job_id: job_id.clone(),
        expected_revision,
    };
    let report = evaluate_cron_preflight(
        cron_db.clone(),
        session_db.clone(),
        request,
        workspace_writes,
    )
    .await?;
    if !report.can_proceed {
        return Ok(CronRunNowResult::Rejected { report });
    }
    let claim_db = cron_db.clone();
    let claim_id = job_id.clone();
    let claimed = ha_core::blocking::run_blocking(move || -> Result<_> {
        if !ha_core::runtime_lock::is_primary() {
            return Ok(None);
        }
        let Some((job, false)) = claim_db.get_job_including_deleted(&claim_id)? else {
            return Ok(None);
        };
        if job.revision != expected_revision {
            return Ok(None);
        }
        ha_core::agent_lifecycle::with_lifecycle_gate(|| {
            claim_db.claim_immediate_job_for_execution(&job)
        })
    })
    .await?;
    if let Some(claimed) = claimed {
        let result = CronRunNowResult::Started {
            job_id: claimed.job.id.clone(),
            revision: claimed.job.revision,
            claimed_at: claimed.claimed_at.clone(),
        };
        super::executor::spawn_claimed_job_execution(cron_db, session_db, claimed);
        return Ok(result);
    }
    let mut report = evaluate_cron_preflight(
        cron_db,
        session_db,
        CronPreflightRequest::RunNow {
            job_id,
            expected_revision,
        },
        workspace_writes,
    )
    .await?;
    if report.can_proceed {
        report.block(CronPreflightIssueCode::ClaimConflict);
    }
    Ok(CronRunNowResult::Rejected { report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contract_and_blocker_semantics_are_stable() {
        let request = CronPreflightRequest::RunNow {
            job_id: "job-1".into(),
            expected_revision: 7,
        };
        let wire = serde_json::to_value(request).unwrap();
        assert_eq!(wire["operation"], "runNow");
        assert_eq!(wire["jobId"], "job-1");
        assert_eq!(wire["expectedRevision"], 7);
        assert!(wire.get("expected_revision").is_none());

        let mut report = CronPreflightReport::new(CronPreflightOperation::RunNow);
        report.warn(CronPreflightIssueCode::ProjectDetached);
        assert!(report.can_proceed);
        report.block(CronPreflightIssueCode::RevisionConflict);
        assert!(!report.can_proceed);
        assert_eq!(
            serde_json::to_value(&report.issues[1]).unwrap()["code"],
            "revision_conflict"
        );
    }
}
