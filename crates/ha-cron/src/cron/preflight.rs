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
    SessionUnavailable,
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

/// Where the occurrence will actually land. Since a task may target any
/// existing chat, "which conversation receives this" is the single most
/// consequential thing to show before saving — a wrong pick is otherwise only
/// discovered once the task has posted into a stranger's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CronConversationPreview {
    /// Each run opens its own ordinary chat titled after the task.
    NewSession,
    /// Every run appends to this existing ordinary chat.
    ExistingSession {
        session_id: String,
        /// `None` when the chat is missing or unreadable (see the issue list).
        title: Option<String>,
    },
}

/// One resolved delivery target, with the blocking problem (if any) already
/// attributed to it so the dialog can point at the offending row instead of
/// showing one anonymous "delivery invalid" line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronDeliveryPreview {
    pub channel_id: String,
    pub account_id: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    /// Owner-facing label captured when the target was picked, when present.
    pub label: Option<String>,
    pub problem: Option<CronPreflightIssueCode>,
}

/// Runtime posture of the scheduler in this process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronSchedulerPreview {
    /// Only the Primary process executes tasks; a Secondary can configure them
    /// but cannot run one now.
    pub primary: bool,
    /// Tasks currently holding a running marker, across all tasks.
    pub running_tasks: u32,
    /// `0` = unlimited (mirrors `CronConfig.max_concurrent`).
    pub max_concurrent: u32,
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
    /// Absent only for a report that never resolved a candidate at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<CronConversationPreview>,
    /// Empty = no fan-out; the run only lands in its own conversation.
    #[serde(default)]
    pub delivery_targets: Vec<CronDeliveryPreview>,
    pub scheduler: CronSchedulerPreview,
    /// This task is executing right now (owner-visible before an edit/run-now).
    pub task_running: bool,
    /// When the in-flight occurrence claimed the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_running_since: Option<String>,
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
    let primary = ha_core::runtime_lock::is_primary();
    report.execution.scheduler = CronSchedulerPreview {
        primary,
        running_tasks: cron_db.count_running().unwrap_or_default() as u32,
        max_concurrent: ha_core::config::cached_config().cron.max_concurrent,
    };
    if operation == CronPreflightOperation::RunNow && !primary {
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

    report.execution.conversation = preview_conversation(&candidate.payload, session_db);
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
    inspect_agent(&candidate, project.as_ref(), session_db, &mut report);
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

/// Resolve the conversation the occurrence lands in. The title is read live so
/// a renamed chat shows its current name, and an unreadable one degrades to the
/// bare id rather than silently claiming a target that no longer exists.
fn preview_conversation(
    payload: &CronPayload,
    session_db: &SessionDB,
) -> Option<CronConversationPreview> {
    match payload {
        CronPayload::AgentTurn { .. } => Some(CronConversationPreview::NewSession),
        CronPayload::SessionTurn { session_id, .. }
        | CronPayload::SessionLoop { session_id, .. } => {
            let title = session_db
                .get_session(session_id)
                .ok()
                .flatten()
                .and_then(|session| session.title);
            Some(CronConversationPreview::ExistingSession {
                session_id: session_id.clone(),
                title,
            })
        }
    }
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
    if let Some(claimed_at) = job.running_at.as_deref() {
        report.execution.task_running = true;
        report.execution.task_running_since = Some(claimed_at.to_string());
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
    if matches!(candidate.payload, CronPayload::SessionTurn { .. })
        && (candidate.project_id.is_some()
            || candidate.permission.is_some()
            || candidate.sandbox.is_some())
    {
        report.block(CronPreflightIssueCode::WorkspacePolicyInvalid);
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
    session_db: &SessionDB,
    report: &mut CronPreflightReport,
) {
    if let CronPayload::SessionTurn { session_id, .. } = &candidate.payload {
        let Ok(Some(session)) = session_db.get_session(session_id) else {
            report.block(CronPreflightIssueCode::SessionUnavailable);
            return;
        };
        if !session.is_regular_chat() {
            report.block(CronPreflightIssueCode::SessionUnavailable);
            return;
        }
        report.execution.resolved_agent_id = Some(session.agent_id.clone());
        report.execution.effective_permission_mode = Some(session.permission_mode);
        report.execution.effective_sandbox_mode = Some(session.sandbox_mode);
        let defaults =
            ha_core::session::resolve_chat_runtime_defaults(Some(&session), &session.agent_id);
        report.execution.primary_model = defaults.model;
        if report.execution.primary_model.is_none() {
            report.block(CronPreflightIssueCode::ModelUnconfigured);
        }
        if ha_core::agent_lifecycle::ensure_agent_runnable(&session.agent_id).is_err() {
            report.block(CronPreflightIssueCode::AgentUnavailable);
        }
        return;
    }
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
    report.execution.delivery_targets = targets
        .iter()
        .map(|target| CronDeliveryPreview {
            channel_id: target.channel_id.clone(),
            account_id: target.account_id.clone(),
            chat_id: target.chat_id.clone(),
            thread_id: target.thread_id.clone(),
            label: target.label.clone(),
            problem: None,
        })
        .collect();
    if targets.is_empty() {
        return;
    }
    // Attribute each problem to its row as well as raising it as an issue, so
    // the dialog can show *which* target is broken instead of one anonymous line.
    let mut attribute = |index: usize, code: CronPreflightIssueCode| {
        if let Some(preview) = report.execution.delivery_targets.get_mut(index) {
            if preview.problem.is_none() {
                preview.problem = Some(code);
            }
        }
        report.block(code);
    };
    let Some(db) = ha_core::get_channel_db() else {
        for index in 0..targets.len() {
            attribute(index, CronPreflightIssueCode::DeliveryUnavailable);
        }
        return;
    };
    let Some(registry) = ha_core::get_channel_registry() else {
        for index in 0..targets.len() {
            attribute(index, CronPreflightIssueCode::DeliveryUnavailable);
        }
        return;
    };
    let config = ha_core::config::cached_config();
    for (index, target) in targets.iter().enumerate() {
        if target.stale {
            attribute(index, CronPreflightIssueCode::DeliveryTargetStale);
        }
        match db.conversation_exists(
            &target.channel_id,
            &target.account_id,
            &target.chat_id,
            target.thread_id.as_deref(),
        ) {
            Err(_) => {
                attribute(index, CronPreflightIssueCode::DeliveryUnavailable);
                continue;
            }
            Ok(false) => {
                attribute(index, CronPreflightIssueCode::DeliveryTargetInvalid);
                continue;
            }
            Ok(true) => {}
        }
        let Some(account) = config.channels.find_account(&target.account_id) else {
            attribute(index, CronPreflightIssueCode::DeliveryAccountMissing);
            continue;
        };
        if !account.enabled {
            attribute(index, CronPreflightIssueCode::DeliveryAccountDisabled);
        }
        if account.channel_id.to_string() != target.channel_id {
            attribute(index, CronPreflightIssueCode::DeliveryChannelMismatch);
        }
        if registry.get_plugin(&account.channel_id).is_none() {
            attribute(index, CronPreflightIssueCode::DeliveryUnavailable);
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
    let candidate_db = cron_db.clone();
    let candidate_id = job_id.clone();
    let candidate = ha_core::blocking::run_blocking(move || {
        candidate_db.get_job_including_deleted(&candidate_id)
    })
    .await?;
    let is_session_turn = candidate.as_ref().is_some_and(|(job, deleted)| {
        !deleted
            && job.revision == expected_revision
            && matches!(job.payload, CronPayload::SessionTurn { .. })
    });
    if is_session_turn {
        if let Some((job, false)) = candidate {
            let stage_db = cron_db.clone();
            let stage_sessions = session_db.clone();
            let staged_job = job.clone();
            let staged = ha_core::blocking::run_blocking(move || {
                super::executor::stage_session_turn(&stage_db, &stage_sessions, &staged_job, true)
            })
            .await?;
            if let Some(run) = staged {
                super::executor::spawn_session_turn_pump_once(cron_db.clone(), session_db.clone());
                return Ok(CronRunNowResult::Started {
                    job_id: job.id,
                    revision: job.revision,
                    claimed_at: run.started_at,
                });
            }
        }
    }
    if !is_session_turn {
        let claim_db = cron_db.clone();
        let claim_sessions = session_db.clone();
        let claim_id = job_id.clone();
        let claimed = ha_core::blocking::run_blocking(move || -> Result<_> {
            if !ha_core::runtime_lock::is_primary() {
                return Ok(None);
            }
            let foreground_stop_admission = claim_sessions.foreground_stop_admission(None)?;
            let Some((job, false)) = claim_db.get_job_including_deleted(&claim_id)? else {
                return Ok(None);
            };
            if job.revision != expected_revision {
                return Ok(None);
            }
            let claimed = ha_core::agent_lifecycle::with_lifecycle_gate(|| {
                claim_db.claim_immediate_job_for_execution(&job)
            })?;
            Ok(claimed.map(|claimed| (claimed, foreground_stop_admission)))
        })
        .await?;
        if let Some((claimed, foreground_stop_admission)) = claimed {
            let result = CronRunNowResult::Started {
                job_id: claimed.job.id.clone(),
                revision: claimed.job.revision,
                claimed_at: claimed.claimed_at.clone(),
            };
            super::executor::spawn_claimed_job_execution(
                cron_db,
                session_db,
                claimed,
                foreground_stop_admission,
            );
            return Ok(result);
        }
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

    #[test]
    fn execution_preview_names_its_conversation_target() {
        let session_wire = serde_json::to_value(CronConversationPreview::ExistingSession {
            session_id: "chat-1".into(),
            title: Some("Roadmap review".into()),
        })
        .unwrap();
        assert_eq!(session_wire["kind"], "existingSession");
        assert_eq!(session_wire["sessionId"], "chat-1");
        assert_eq!(session_wire["title"], "Roadmap review");
        assert_eq!(
            serde_json::to_value(CronConversationPreview::NewSession).unwrap()["kind"],
            "newSession"
        );
    }

    #[test]
    fn delivery_preview_attributes_the_problem_to_its_own_target() {
        let mut report = CronPreflightReport::new(CronPreflightOperation::Create);
        // No channel DB is wired in unit tests, so every target is unresolvable —
        // the point here is that the row itself carries the reason.
        inspect_delivery(
            &[CronDeliveryTarget {
                channel_id: "telegram".into(),
                account_id: "acct-1".into(),
                chat_id: "chat-9".into(),
                thread_id: None,
                label: Some("telegram / ops".into()),
                stale: false,
            }],
            &mut report,
        );
        assert_eq!(report.execution.delivery_targets.len(), 1);
        let preview = &report.execution.delivery_targets[0];
        assert_eq!(preview.label.as_deref(), Some("telegram / ops"));
        assert_eq!(
            preview.problem,
            Some(CronPreflightIssueCode::DeliveryUnavailable)
        );
        assert!(!report.can_proceed);

        let mut empty = CronPreflightReport::new(CronPreflightOperation::Create);
        inspect_delivery(&[], &mut empty);
        assert!(empty.execution.delivery_targets.is_empty());
        assert!(empty.can_proceed, "no targets is not a delivery problem");
    }
}
