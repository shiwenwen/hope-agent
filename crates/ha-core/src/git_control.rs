//! Session-scoped Git control plane used by both desktop and HTTP adapters.
//!
//! The frontend never supplies an arbitrary working directory or patch. Every
//! request starts from a session id, resolves the effective workspace through
//! `WorkspaceScope`, and re-generates any selected hunk under a repository
//! lock before mutating Git state.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::filesystem::{git_info, GitBranchInfo, GitDirtySummary, WorkspaceScope, WorktreeInfo};
use crate::session::{
    effective_working_dir_for_meta, SessionDB, WorkspaceGitCommit, WorkspaceGitStatus,
    WorkspaceGitSync, WorkspaceGitSyncState,
};
use crate::workflow::WorkflowRunState;

pub const EVENT_GIT_PROGRESS: &str = "session:git_progress";
pub const EVENT_GIT_CHANGED: &str = "session:git_changed";
pub const EVENT_GIT_COMPLETED: &str = "session:git_completed";

const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const GH_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_DIFF_TEXT_BYTES: u64 = 256 * 1024;

pub fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS git_operation_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            status TEXT NOT NULL,
            stage TEXT NOT NULL,
            before_head TEXT,
            after_head TEXT,
            result_json TEXT,
            error_code TEXT,
            error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_git_operation_session
            ON git_operation_runs(session_id, updated_at DESC);",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitDiffScope {
    Unstaged,
    Staged,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHunkInfo {
    pub id: String,
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    pub kind: &'static str,
    pub path: String,
    pub old_path: Option<String>,
    pub action: String,
    pub status: String,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub before: Option<String>,
    pub after: Option<String>,
    pub language: &'static str,
    pub truncated: bool,
    pub binary: bool,
    pub submodule: bool,
    pub conflicted: bool,
    pub untracked: bool,
    pub hunks: Vec<GitHunkInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionGitDiffSnapshot {
    pub revision: String,
    pub scope: GitDiffScope,
    pub changes: Vec<GitFileChange>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteInfo {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
    pub host: Option<String>,
    pub is_default: bool,
    pub is_github: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitCapabilities {
    pub can_switch_branch: bool,
    pub can_create_branch: bool,
    pub can_commit: bool,
    pub can_push: bool,
    pub can_create_pull_request: bool,
    pub can_handoff: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionGitControlSnapshot {
    pub root: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub revision: String,
    pub branches: Vec<GitBranchInfo>,
    pub remotes: Vec<GitRemoteInfo>,
    pub worktrees: Vec<WorktreeInfo>,
    pub dirty: GitDirtySummary,
    pub status: WorkspaceGitStatus,
    pub sync: WorkspaceGitSync,
    pub last_commit: Option<WorkspaceGitCommit>,
    pub active_location: String,
    pub managed_worktree_id: Option<String>,
    pub capabilities: GitCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitMutationAction {
    Stage,
    Unstage,
    Discard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitMutationTargetKind {
    All,
    File,
    Hunk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitMutationTarget {
    pub kind: GitMutationTargetKind,
    pub path: Option<String>,
    pub hunk_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitIndexMutationInput {
    pub expected_revision: String,
    pub action: GitMutationAction,
    pub target: GitMutationTarget,
    #[serde(default)]
    pub confirm_discard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSwitchBranchInput {
    pub request_id: String,
    pub expected_revision: String,
    pub full_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCreateBranchInput {
    pub request_id: String,
    pub expected_revision: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitInput {
    pub request_id: String,
    pub expected_revision: String,
    pub subject: String,
    pub body: Option<String>,
    #[serde(default)]
    pub stage_all: bool,
    #[serde(default)]
    pub push_after: bool,
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushInput {
    pub request_id: String,
    pub expected_revision: String,
    pub remote: Option<String>,
    #[serde(default)]
    pub set_upstream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCreatePullRequestInput {
    pub request_id: String,
    pub expected_revision: String,
    pub title: String,
    pub body: Option<String>,
    pub base_branch: Option<String>,
    #[serde(default = "default_true")]
    pub draft: bool,
    #[serde(default)]
    pub push_first: bool,
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitPullRequestMergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl GitPullRequestMergeMethod {
    fn gh_flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitEnablePullRequestAutoMergeInput {
    pub request_id: String,
    pub expected_revision: String,
    pub method: GitPullRequestMergeMethod,
    #[serde(default)]
    pub confirm_auto_merge: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub base_branch: String,
    pub head_branch: String,
    pub body: String,
    pub author: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub mergeable: String,
    pub merge_state_status: String,
    pub review_decision: Option<String>,
    pub auto_merge_enabled: bool,
    pub auto_merge_method: Option<String>,
    pub reviewers: Vec<GitPullRequestReviewer>,
    pub reviews: Vec<GitPullRequestReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestReviewer {
    pub login: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestReview {
    pub id: String,
    pub author: String,
    pub state: String,
    pub body: String,
    pub submitted_at: Option<String>,
    pub commit_oid: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestPreflight {
    pub available: bool,
    pub gh_available: bool,
    pub authenticated: bool,
    pub host: Option<String>,
    pub repository: Option<String>,
    pub default_branch: Option<String>,
    pub current: Option<GitPullRequestInfo>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestCheck {
    pub name: String,
    pub workflow: Option<String>,
    pub state: String,
    pub bucket: String,
    pub description: Option<String>,
    pub link: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestReviewComment {
    pub thread_id: String,
    pub comment_id: String,
    pub author: String,
    pub body: String,
    pub path: String,
    pub line: Option<u64>,
    pub start_line: Option<u64>,
    pub side: Option<String>,
    pub url: Option<String>,
    pub created_at: Option<String>,
    pub reply_count: usize,
    pub is_resolved: bool,
    pub is_outdated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestFeedback {
    pub preflight: GitPullRequestPreflight,
    pub checks: Vec<GitPullRequestCheck>,
    pub review_comments: Vec<GitPullRequestReviewComment>,
    pub failed_checks: usize,
    pub pending_checks: usize,
    pub passed_checks: usize,
    pub unresolved_comments: usize,
    pub checks_truncated: bool,
    pub comments_truncated: bool,
    pub checks_error: Option<String>,
    pub comments_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHandoffTarget {
    Local,
    Worktree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHandoffInput {
    pub request_id: String,
    pub expected_revision: String,
    pub target: GitHandoffTarget,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitMutationResult {
    pub revision: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub message: String,
    pub url: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationRun {
    pub id: String,
    pub session_id: String,
    pub operation: String,
    pub status: String,
    pub stage: String,
    pub before_head: Option<String>,
    pub after_head: Option<String>,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitProgressEvent<'a> {
    request_id: &'a str,
    session_id: &'a str,
    operation: &'a str,
    status: &'a str,
    stage: &'a str,
    message: Option<&'a str>,
    error_code: Option<&'a str>,
}

struct RepoContext {
    workspace_root: PathBuf,
    checkout_root: PathBuf,
    common_dir: PathBuf,
}

struct RepoLock {
    _file: fs::File,
}

#[derive(Debug, Clone)]
struct ParsedHunk {
    info: GitHunkInfo,
    patch: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandoffManifest {
    request_id: String,
    session_id: String,
    source_path: String,
    target_path: String,
    source_head: String,
    target_head: String,
    source_branch: Option<String>,
    target_branch: Option<String>,
    untracked: Vec<String>,
    stage: String,
}

pub fn load_control_snapshot(
    db: &SessionDB,
    session_id: &str,
) -> Result<SessionGitControlSnapshot> {
    let ctx = repo_context(db, session_id)?;
    let info = git_info(&ctx.workspace_root)
        .ok_or_else(|| anyhow!("workspace is not a Git repository"))?;
    let rich = crate::session::build_git_snapshot(&ctx.checkout_root)
        .ok_or_else(|| anyhow!("workspace Git status is unavailable"))?;
    let head = git_optional(&ctx.workspace_root, &["rev-parse", "HEAD"]);
    let revision = repository_revision(&ctx.checkout_root)?;
    let remotes = load_remotes(&ctx.checkout_root);
    let managed = db
        .list_managed_worktrees_for_session(session_id)?
        .into_iter()
        .find(|wt| wt.path_exists && Path::new(&wt.path) == ctx.workspace_root);
    let active_location = if managed.is_some() {
        "worktree"
    } else {
        "local"
    }
    .to_string();
    let busy = is_session_busy(session_id);
    let has_github = remotes.iter().any(|remote| remote.is_github);
    let detached = info.branch.is_none();
    let push_blocked = matches!(
        rich.sync.state,
        WorkspaceGitSyncState::Behind | WorkspaceGitSyncState::Diverged
    );
    Ok(SessionGitControlSnapshot {
        root: ctx.checkout_root.to_string_lossy().into_owned(),
        head,
        branch: info.branch,
        detached,
        revision,
        branches: info.branches,
        remotes,
        worktrees: info.worktrees,
        dirty: info.dirty,
        status: rich.status,
        sync: rich.sync,
        last_commit: rich.last_commit,
        active_location,
        managed_worktree_id: managed.map(|wt| wt.id),
        capabilities: GitCapabilities {
            can_switch_branch: !busy,
            can_create_branch: !busy,
            can_commit: !busy && !detached,
            can_push: !busy && !detached && !push_blocked,
            can_create_pull_request: !busy && !detached && has_github,
            can_handoff: !busy,
            reason: busy.then(|| "session is currently running".to_string()),
        },
    })
}

pub fn load_session_git_diff_snapshot(
    db: &SessionDB,
    session_id: &str,
    scope: GitDiffScope,
) -> Result<SessionGitDiffSnapshot> {
    let ctx = repo_context(db, session_id)?;
    load_diff_for_context(&ctx, scope)
}

pub fn mutate_index(
    db: &SessionDB,
    session_id: &str,
    input: &GitIndexMutationInput,
) -> Result<SessionGitDiffSnapshot> {
    ensure_session_idle(db, session_id)?;
    let ctx = repo_context(db, session_id)?;
    let _lock = acquire_repo_lock(&ctx.common_dir)?;
    let scope = match input.action {
        GitMutationAction::Stage | GitMutationAction::Discard => GitDiffScope::Unstaged,
        GitMutationAction::Unstage => GitDiffScope::Staged,
    };
    let snapshot = load_diff_for_context(&ctx, scope)?;
    if snapshot.revision != input.expected_revision {
        bail!("stale_snapshot: repository changed; refresh and retry");
    }
    if matches!(input.action, GitMutationAction::Discard) && !input.confirm_discard {
        bail!("discard_confirmation_required: discarding changes requires confirmation");
    }
    apply_index_mutation(&ctx, &snapshot, input)?;
    emit_git_changed(session_id, input.action_name(), None);
    load_diff_for_context(&ctx, scope)
}

impl GitIndexMutationInput {
    fn action_name(&self) -> &'static str {
        match self.action {
            GitMutationAction::Stage => "stage",
            GitMutationAction::Unstage => "unstage",
            GitMutationAction::Discard => "discard",
        }
    }
}

pub fn switch_branch(
    db: &SessionDB,
    session_id: &str,
    input: &GitSwitchBranchInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    with_idempotent_operation(db, session_id, &input.request_id, "switch_branch", |ctx| {
        require_revision(&ctx, &input.expected_revision)?;
        require_clean(&ctx.checkout_root, "switching branches")?;
        let info =
            git_info(&ctx.workspace_root).ok_or_else(|| anyhow!("Git information unavailable"))?;
        let selected = info
            .branches
            .iter()
            .find(|branch| branch.full_ref == input.full_ref)
            .ok_or_else(|| anyhow!("invalid_branch: selected branch no longer exists"))?;
        if selected.is_checked_out && !selected.is_current {
            bail!("branch_checked_out: branch is checked out by another worktree");
        }
        match selected.kind {
            crate::filesystem::GitBranchKind::Local => {
                run_git_ok(
                    &ctx.workspace_root,
                    &["switch", "--no-guess", &selected.name],
                )?;
            }
            crate::filesystem::GitBranchKind::Remote => {
                let short = selected
                    .name
                    .split_once('/')
                    .map(|(_, name)| name)
                    .ok_or_else(|| anyhow!("invalid remote branch"))?;
                if git_optional(
                    &ctx.checkout_root,
                    &["show-ref", "--verify", &format!("refs/heads/{short}")],
                )
                .is_some()
                {
                    bail!("branch_exists: local tracking branch already exists");
                }
                run_git_ok(&ctx.workspace_root, &["switch", "--track", &selected.name])?;
            }
        }
        Ok(result_from_context(&ctx, "Branch switched", None)?)
    })
}

pub fn create_branch(
    db: &SessionDB,
    session_id: &str,
    input: &GitCreateBranchInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    let name = input.name.trim();
    if name.is_empty() || name.len() > 240 {
        bail!("invalid_branch: branch name is empty or too long");
    }
    with_idempotent_operation(db, session_id, &input.request_id, "create_branch", |ctx| {
        require_revision(&ctx, &input.expected_revision)?;
        run_git_ok(&ctx.checkout_root, &["check-ref-format", "--branch", name])?;
        if git_optional(
            &ctx.checkout_root,
            &["show-ref", "--verify", &format!("refs/heads/{name}")],
        )
        .is_some()
        {
            bail!("branch_exists: branch already exists");
        }
        run_git_ok(&ctx.workspace_root, &["switch", "-c", name])?;
        db.update_managed_worktree_git_branch_for_path(&ctx.workspace_root, Some(name))?;
        Ok(result_from_context(&ctx, "Branch created", None)?)
    })
}

pub fn commit(
    db: &SessionDB,
    session_id: &str,
    input: &GitCommitInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    let subject = input.subject.trim();
    if subject.is_empty() || subject.len() > 512 || subject.contains(['\n', '\r']) {
        bail!("invalid_commit_message: commit subject must be one non-empty line");
    }
    with_idempotent_operation(db, session_id, &input.request_id, "commit", |ctx| {
        require_revision(&ctx, &input.expected_revision)?;
        commit_changes(&ctx, input, subject)
    })
}

fn commit_changes(
    ctx: &RepoContext,
    input: &GitCommitInput,
    subject: &str,
) -> Result<GitMutationResult> {
    if current_branch(&ctx.checkout_root).is_none() {
        bail!("detached_head: create a branch before committing");
    }
    if input.stage_all {
        run_git_ok(&ctx.checkout_root, &["add", "-A", "--", "."])?;
    }
    if git_output(&ctx.checkout_root, &["diff", "--cached", "--quiet"])?
        .status
        .success()
    {
        bail!("nothing_to_commit: no staged changes");
    }
    let mut args = vec!["commit", "-m", subject];
    let body_owned;
    if let Some(body) = input
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
    {
        body_owned = body.to_string();
        args.extend(["-m", body_owned.as_str()]);
    }
    run_git_ok_timeout(&ctx.checkout_root, &args, GIT_TIMEOUT)?;
    let push_warning = if input.push_after {
        push_current_branch(ctx, input.remote.as_deref(), true)
            .err()
            .map(|error| format!("{error:#}"))
    } else {
        None
    };
    let mut result = result_from_context(
        ctx,
        if push_warning.is_some() {
            "Commit created, but push failed"
        } else {
            "Commit created"
        },
        None,
    )?;
    result.warning = push_warning;
    Ok(result)
}

pub fn push(db: &SessionDB, session_id: &str, input: &GitPushInput) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    with_idempotent_operation(db, session_id, &input.request_id, "push", |ctx| {
        require_revision(&ctx, &input.expected_revision)?;
        push_current_branch(&ctx, input.remote.as_deref(), input.set_upstream)?;
        Ok(result_from_context(&ctx, "Branch pushed", None)?)
    })
}

pub fn pull_request_preflight(db: &SessionDB, session_id: &str) -> Result<GitPullRequestPreflight> {
    let ctx = repo_context(db, session_id)?;
    let Some(branch) = current_branch(&ctx.workspace_root) else {
        return Ok(pr_unavailable(
            "detached_head",
            "Create a branch before opening a pull request",
        ));
    };
    let remotes = load_remotes(&ctx.checkout_root);
    let Some(remote) = remotes
        .iter()
        .find(|remote| remote.is_github && remote.is_default)
        .or_else(|| remotes.iter().find(|remote| remote.is_github))
    else {
        return Ok(pr_unavailable(
            "not_github_remote",
            "The repository has no GitHub remote",
        ));
    };
    let Some(host) = remote.host.clone() else {
        return Ok(pr_unavailable(
            "not_github_remote",
            "Cannot determine the GitHub host",
        ));
    };
    let Ok(gh) = which::which("gh") else {
        let mut result = pr_unavailable("gh_unavailable", "GitHub CLI is not installed");
        result.host = Some(host);
        return Ok(result);
    };
    let auth = run_command_timeout(
        gh_command(
            &gh,
            &ctx.workspace_root,
            &["auth", "status", "--hostname", &host],
        ),
        GH_TIMEOUT,
    )?;
    if !auth.status.success() {
        let mut result = pr_unavailable(
            "gh_unauthenticated",
            &format!(
                "GitHub CLI is not authenticated for {host}: {}",
                output_error(&auth)
            ),
        );
        result.gh_available = true;
        result.host = Some(host);
        return Ok(result);
    }
    let repo_output = run_command_timeout(
        gh_command(
            &gh,
            &ctx.workspace_root,
            &["repo", "view", "--json", "nameWithOwner,defaultBranchRef"],
        ),
        GH_TIMEOUT,
    )?;
    if !repo_output.status.success() {
        let mut result = pr_unavailable("gh_repo_unavailable", &output_error(&repo_output));
        result.gh_available = true;
        result.authenticated = true;
        result.host = Some(host);
        return Ok(result);
    }
    let repo_json: Value =
        serde_json::from_slice(&repo_output.stdout).context("decode gh repo view")?;
    let repository = repo_json
        .get("nameWithOwner")
        .and_then(Value::as_str)
        .map(str::to_string);
    let default_branch = repo_json
        .pointer("/defaultBranchRef/name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| default_base_branch(&ctx.checkout_root));
    let current = load_current_pr(&gh, &ctx.workspace_root, &branch)?;
    Ok(GitPullRequestPreflight {
        available: true,
        gh_available: true,
        authenticated: true,
        host: Some(host),
        repository,
        default_branch,
        current,
        error_code: None,
        error_message: None,
    })
}

pub fn pull_request_feedback(db: &SessionDB, session_id: &str) -> Result<GitPullRequestFeedback> {
    let preflight = pull_request_preflight(db, session_id)?;
    let Some(pull_request) = preflight.current.as_ref() else {
        return Ok(empty_pull_request_feedback(preflight));
    };
    if !preflight.available {
        return Ok(empty_pull_request_feedback(preflight));
    }

    let ctx = repo_context(db, session_id)?;
    let gh = which::which("gh").context("gh_unavailable: GitHub CLI is not installed")?;
    let (checks, checks_truncated, checks_error) =
        match load_pull_request_checks(&gh, &ctx.checkout_root, pull_request.number) {
            Ok((checks, truncated)) => (checks, truncated, None),
            Err(error) => (Vec::new(), false, Some(format!("{error:#}"))),
        };
    let repository = preflight
        .repository
        .as_deref()
        .ok_or_else(|| anyhow!("gh_repo_unavailable: GitHub repository is unavailable"))?;
    let host = preflight
        .host
        .as_deref()
        .ok_or_else(|| anyhow!("gh_repo_unavailable: GitHub host is unavailable"))?;
    let (review_comments, comments_truncated, comments_error) =
        match load_pull_request_review_comments(
            &gh,
            &ctx.checkout_root,
            host,
            repository,
            pull_request.number,
        ) {
            Ok((comments, truncated)) => (comments, truncated, None),
            Err(error) => (Vec::new(), false, Some(format!("{error:#}"))),
        };
    let failed_checks = checks
        .iter()
        .filter(|check| matches!(check.bucket.as_str(), "fail" | "cancel"))
        .count();
    let pending_checks = checks
        .iter()
        .filter(|check| check.bucket == "pending")
        .count();
    let passed_checks = checks.iter().filter(|check| check.bucket == "pass").count();
    let unresolved_comments = review_comments
        .iter()
        .filter(|comment| !comment.is_resolved && !comment.is_outdated)
        .count();

    Ok(GitPullRequestFeedback {
        preflight,
        checks,
        review_comments,
        failed_checks,
        pending_checks,
        passed_checks,
        unresolved_comments,
        checks_truncated,
        comments_truncated,
        checks_error,
        comments_error,
    })
}

pub fn create_pull_request(
    db: &SessionDB,
    session_id: &str,
    input: &GitCreatePullRequestInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    let title = input.title.trim();
    if title.is_empty() || title.len() > 512 || title.contains(['\n', '\r']) {
        bail!("invalid_pr_title: pull request title must be one non-empty line");
    }
    with_idempotent_operation(
        db,
        session_id,
        &input.request_id,
        "create_pull_request",
        |ctx| {
            require_revision(&ctx, &input.expected_revision)?;
            let branch = current_branch(&ctx.workspace_root).ok_or_else(|| {
                anyhow!("detached_head: create a branch before opening a pull request")
            })?;
            let preflight = pull_request_preflight(db, session_id)?;
            if let Some(existing) = preflight.current {
                return result_from_context(
                    &ctx,
                    "Pull request already exists",
                    Some(existing.url),
                );
            }
            if !preflight.available {
                bail!(
                    "{}: {}",
                    preflight
                        .error_code
                        .unwrap_or_else(|| "pr_unavailable".into()),
                    preflight
                        .error_message
                        .unwrap_or_else(|| "Pull request unavailable".into())
                );
            }
            if input.push_first {
                push_current_branch(&ctx, input.remote.as_deref(), true)?;
            }
            let gh = which::which("gh").context("gh_unavailable: GitHub CLI is not installed")?;
            let base = input
                .base_branch
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or(preflight.default_branch)
                .or_else(|| default_base_branch(&ctx.checkout_root))
                .ok_or_else(|| {
                    anyhow!("base_branch_unavailable: cannot determine pull request base branch")
                })?;
            let mut args = vec![
                "pr",
                "create",
                "--head",
                branch.as_str(),
                "--base",
                base.as_str(),
                "--title",
                title,
                "--body-file",
                "-",
            ];
            if input.draft {
                args.push("--draft");
            }
            let output = run_command_with_stdin_timeout(
                gh_command(&gh, &ctx.workspace_root, &args),
                input.body.as_deref().unwrap_or("").as_bytes(),
                GH_TIMEOUT,
            )?;
            if !output.status.success() {
                bail!("gh_pr_create_failed: {}", output_error(&output));
            }
            let url = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with("http://") || line.starts_with("https://"))
                .map(str::to_string)
                .ok_or_else(|| {
                    anyhow!("gh_pr_create_failed: gh did not return a pull request URL")
                })?;
            result_from_context(&ctx, "Pull request created", Some(url))
        },
    )
}

pub fn enable_pull_request_auto_merge(
    db: &SessionDB,
    session_id: &str,
    input: &GitEnablePullRequestAutoMergeInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    if !input.confirm_auto_merge {
        bail!(
            "auto_merge_confirmation_required: enabling auto-merge requires explicit confirmation"
        );
    }
    with_idempotent_operation(
        db,
        session_id,
        &input.request_id,
        "enable_pull_request_auto_merge",
        |ctx| {
            require_revision(&ctx, &input.expected_revision)?;
            let preflight = pull_request_preflight(db, session_id)?;
            if !preflight.available {
                bail!(
                    "{}: {}",
                    preflight
                        .error_code
                        .unwrap_or_else(|| "pr_unavailable".into()),
                    preflight
                        .error_message
                        .unwrap_or_else(|| "Pull request unavailable".into())
                );
            }
            let pull_request = preflight.current.ok_or_else(|| {
                anyhow!("pull_request_missing: current branch has no pull request")
            })?;
            if pull_request.state != "OPEN" {
                bail!("pull_request_not_open: auto-merge requires an open pull request");
            }
            if pull_request.mergeable == "CONFLICTING" || pull_request.merge_state_status == "DIRTY"
            {
                bail!("merge_conflicts: resolve pull request conflicts before enabling auto-merge");
            }
            if pull_request.auto_merge_enabled {
                return result_from_context(
                    &ctx,
                    "Pull request auto-merge is already enabled",
                    Some(pull_request.url),
                );
            }
            let gh = which::which("gh").context("gh_unavailable: GitHub CLI is not installed")?;
            let number = pull_request.number.to_string();
            let output = run_command_timeout(
                gh_command(
                    &gh,
                    &ctx.workspace_root,
                    &["pr", "merge", &number, "--auto", input.method.gh_flag()],
                ),
                GH_TIMEOUT,
            )?;
            if !output.status.success() {
                bail!("gh_auto_merge_failed: {}", output_error(&output));
            }
            result_from_context(
                &ctx,
                "Pull request auto-merge enabled",
                Some(pull_request.url),
            )
        },
    )
}

pub async fn handoff(
    db: Arc<SessionDB>,
    session_id: String,
    input: GitHandoffInput,
) -> Result<GitMutationResult> {
    validate_request_id(&input.request_id)?;
    ensure_session_idle(&db, &session_id)?;
    if let Some(existing) = db.get_git_operation_run(&input.request_id)? {
        if existing.session_id != session_id || existing.operation != "handoff" {
            bail!("request_id_conflict: requestId belongs to another Git operation");
        }
        if existing.status == "completed" {
            return serde_json::from_value(
                existing
                    .result
                    .ok_or_else(|| anyhow!("completed handoff has no result"))?,
            )
            .context("decode handoff result");
        }
        bail!("operation_running: this handoff has already started");
    }

    let current = repo_context(&db, &session_id)?;
    require_revision(&current, &input.expected_revision)?;
    let (target_path, created_worktree_id) = match input.target {
        GitHandoffTarget::Local => {
            let current_path = current.workspace_root.clone();
            let row = db
                .list_managed_worktrees_for_session(&session_id)?
                .into_iter()
                .find(|worktree| {
                    Path::new(&worktree.path).canonicalize().ok().as_ref() == Some(&current_path)
                })
                .ok_or_else(|| {
                    anyhow!(
                        "not_in_managed_worktree: current session is not using a managed worktree"
                    )
                })?;
            (PathBuf::from(row.source_working_dir), None)
        }
        GitHandoffTarget::Worktree => {
            let existing = if let Some(id) = input.worktree_id.as_deref() {
                let worktree = db
                    .get_managed_worktree(id)?
                    .ok_or_else(|| anyhow!("managed_worktree_not_found: {id}"))?;
                if !worktree_in_session_scope(
                    &worktree.session_id,
                    worktree.child_session_id.as_deref(),
                    &session_id,
                ) {
                    bail!("worktree_scope_mismatch: selected worktree does not belong to this session");
                }
                Some(worktree)
            } else {
                None
            };
            match existing {
                Some(worktree) => {
                    if !worktree.path_exists {
                        db.restore_managed_worktree(&worktree.id)?;
                    }
                    (PathBuf::from(worktree.path), None)
                }
                None => {
                    let worktree = db
                        .create_managed_worktree(crate::worktree::CreateManagedWorktreeInput {
                            session_id: session_id.clone(),
                            source_working_dir: Some(
                                current.workspace_root.to_string_lossy().into_owned(),
                            ),
                            label: Some("Session worktree".to_string()),
                            purpose: crate::worktree::ManagedWorktreePurpose::Manual,
                            workflow_run_id: None,
                            child_session_id: None,
                            base_ref: None,
                            include_local_changes: false,
                            bootstrap_request_id: None,
                            bind_session_working_dir: false,
                        })
                        .await?;
                    let id = worktree.id.clone();
                    (PathBuf::from(worktree.path), Some(id))
                }
            }
        }
    };

    let blocking_db = db.clone();
    let request_id = input.request_id.clone();
    let expected_revision = input.expected_revision.clone();
    let source_workspace = current.workspace_root.clone();
    let cleanup_session_id = session_id.clone();
    let result = crate::blocking::run_blocking(move || {
        with_idempotent_operation(
            &blocking_db,
            &session_id,
            &request_id,
            "handoff",
            |source| {
                require_revision(&source, &expected_revision)?;
                transfer_workspace(&blocking_db, &session_id, &request_id, source, &target_path)
            },
        )
    })
    .await;
    if result.is_err() {
        if let Some(worktree_id) = created_worktree_id {
            let cleanup_id = worktree_id.clone();
            let cleanup = db
                .run(move |db| -> Result<()> {
                    let Some(worktree) = db.get_managed_worktree(&cleanup_id)? else {
                        return Ok(());
                    };
                    let path = Path::new(&worktree.path);
                    let clean = crate::filesystem::git_info(path)
                        .is_some_and(|info| info.dirty.changed_files == 0);
                    if !clean {
                        bail!("handoff cleanup preserved dirty worktree {}", worktree.path);
                    }
                    db.discard_managed_worktree(&cleanup_id)
                })
                .await;
            let source = source_workspace.to_string_lossy().into_owned();
            let _ = db
                .run(move |db| db.update_session_working_dir(&cleanup_session_id, Some(source)))
                .await;
            if let Err(cleanup_error) = cleanup {
                return Err(anyhow!(
                    "{}; created worktree cleanup failed: {cleanup_error:#}",
                    result.as_ref().unwrap_err()
                ));
            }
        }
    }
    result
}

fn worktree_in_session_scope(owner: &str, child: Option<&str>, session_id: &str) -> bool {
    owner == session_id || child == Some(session_id)
}

fn transfer_workspace(
    db: &SessionDB,
    session_id: &str,
    request_id: &str,
    source: RepoContext,
    target_path: &Path,
) -> Result<GitMutationResult> {
    let target_workspace = target_path
        .canonicalize()
        .with_context(|| format!("handoff target is unavailable: {}", target_path.display()))?;
    let target_checkout = PathBuf::from(run_git(
        &target_workspace,
        &["rev-parse", "--show-toplevel"],
    )?)
    .canonicalize()?;
    let target_common_dir = git_common_dir(&target_checkout)?;
    let workspace_relative = source
        .workspace_root
        .strip_prefix(&source.checkout_root)
        .context("session workspace is outside its checkout")?;
    let target_workspace = target_checkout.join(workspace_relative);
    if target_checkout == source.checkout_root {
        bail!("handoff_same_location: session is already using the selected location");
    }
    if target_common_dir != source.common_dir {
        bail!("cross_repository_handoff: source and target are not in the same repository");
    }
    if !target_workspace.is_dir() {
        bail!("handoff_target_missing: matching project subdirectory is absent in target checkout");
    }
    require_clean(&target_checkout, "handoff")?;
    if git_optional(
        &source.checkout_root,
        &["diff", "--name-only", "--diff-filter=U"],
    )
    .is_some()
    {
        bail!("conflicts_present: resolve conflicts before handoff");
    }

    db.set_git_operation_stage(request_id, "snapshotting_source")?;
    emit_progress(
        request_id,
        session_id,
        "handoff",
        "running",
        "snapshotting_source",
        Some("Snapshotting source changes"),
        None,
    );
    let run_dir = crate::paths::git_operation_run_dir(request_id)?;
    if run_dir.exists() {
        fs::remove_dir_all(&run_dir)?;
    }
    fs::create_dir_all(run_dir.join("untracked"))?;
    let staged = run_git_bytes(
        &source.checkout_root,
        &["diff", "--binary", "--cached", "HEAD", "--"],
    )?;
    let unstaged = run_git_bytes(&source.checkout_root, &["diff", "--binary", "--"])?;
    let untracked = untracked_paths(&source.checkout_root)?;
    for path in &untracked {
        snapshot_untracked_file(&source.checkout_root, &run_dir, path)?;
    }
    let source_head = run_git(&source.checkout_root, &["rev-parse", "HEAD"])?;
    let target_head = run_git(&target_checkout, &["rev-parse", "HEAD"])?;
    let source_branch = current_branch(&source.checkout_root);
    let target_branch = current_branch(&target_checkout);
    if source_branch.is_none() && source_head != target_head {
        let _ = fs::remove_dir_all(&run_dir);
        bail!("target_head_mismatch: detached source and target do not point to the same commit");
    }
    crate::platform::write_atomic(&run_dir.join("staged.patch"), &staged)?;
    crate::platform::write_atomic(&run_dir.join("unstaged.patch"), &unstaged)?;
    let mut manifest = HandoffManifest {
        request_id: request_id.to_string(),
        session_id: session_id.to_string(),
        source_path: source.checkout_root.to_string_lossy().into_owned(),
        target_path: target_checkout.to_string_lossy().into_owned(),
        source_head: source_head.clone(),
        target_head: target_head.clone(),
        source_branch: source_branch.clone(),
        target_branch: target_branch.clone(),
        untracked: untracked.clone(),
        stage: "snapshotting_source".to_string(),
    };
    persist_handoff_manifest(&run_dir, &manifest)?;
    let expected_fingerprint = snapshot_fingerprint(&staged, &unstaged, &run_dir, &untracked)?;

    let result = (|| -> Result<GitMutationResult> {
        db.set_git_operation_stage(request_id, "moving_branch")?;
        emit_progress(
            request_id,
            session_id,
            "handoff",
            "running",
            "moving_branch",
            Some("Moving branch ownership"),
            None,
        );
        manifest.stage = "moving_branch".to_string();
        persist_handoff_manifest(&run_dir, &manifest)?;
        if run_git(&source.checkout_root, &["rev-parse", "HEAD"])? != source_head
            || current_branch(&source.checkout_root) != source_branch
            || checkout_fingerprint(&source.checkout_root, &run_dir, &untracked)?
                != expected_fingerprint
        {
            bail!("handoff_source_changed: source checkout changed while it was being snapshotted");
        }
        clean_checkout(&source.checkout_root, &untracked)?;
        if let Some(branch) = source_branch.as_deref() {
            run_git_ok(&source.checkout_root, &["switch", "--detach", &source_head])?;
            run_git_ok(&target_checkout, &["switch", "--no-guess", branch])?;
        }

        db.set_git_operation_stage(request_id, "transferring_changes")?;
        emit_progress(
            request_id,
            session_id,
            "handoff",
            "running",
            "transferring_changes",
            Some("Transferring staged and unstaged changes"),
            None,
        );
        manifest.stage = "transferring_changes".to_string();
        persist_handoff_manifest(&run_dir, &manifest)?;
        if !staged.is_empty() {
            apply_patch(&target_checkout, &staged, &["--index"])?;
        }
        if !unstaged.is_empty() {
            apply_patch(&target_checkout, &unstaged, &[])?;
        }
        restore_untracked_files(&target_checkout, &run_dir, &untracked)?;

        db.set_git_operation_stage(request_id, "verifying_target")?;
        emit_progress(
            request_id,
            session_id,
            "handoff",
            "running",
            "verifying_target",
            Some("Verifying target checkout"),
            None,
        );
        let actual = checkout_fingerprint(&target_checkout, &run_dir, &untracked)?;
        if actual != expected_fingerprint {
            bail!("handoff_verification_failed: target Git state differs from source snapshot");
        }

        db.set_git_operation_stage(request_id, "binding_session")?;
        emit_progress(
            request_id,
            session_id,
            "handoff",
            "running",
            "binding_session",
            Some("Binding session workspace"),
            None,
        );
        db.update_session_working_dir(
            session_id,
            Some(target_workspace.to_string_lossy().into_owned()),
        )?;
        db.mark_handoff_worktrees_active(session_id)?;
        result_from_context(
            &RepoContext {
                workspace_root: target_workspace.clone(),
                checkout_root: target_checkout.clone(),
                common_dir: source.common_dir.clone(),
            },
            "Workspace handed off",
            None,
        )
    })();

    if let Err(error) = result {
        let rollback = rollback_handoff(
            &source.checkout_root,
            &target_checkout,
            &run_dir,
            &manifest,
            &staged,
            &unstaged,
        );
        let _ = db.update_session_working_dir(
            session_id,
            Some(source.workspace_root.to_string_lossy().into_owned()),
        );
        let _ = fs::remove_dir_all(&run_dir);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(anyhow!(
                "handoff_rollback_failed: {error:#}; rollback failed: {rollback_error:#}"
            )),
        };
    }
    let result = result.unwrap();
    fs::remove_dir_all(&run_dir)?;
    Ok(result)
}

fn push_current_branch(
    ctx: &RepoContext,
    remote: Option<&str>,
    allow_set_upstream: bool,
) -> Result<()> {
    let branch = current_branch(&ctx.workspace_root)
        .ok_or_else(|| anyhow!("detached_head: create a branch before pushing"))?;
    if let Some((ahead, behind)) = ahead_behind(&ctx.workspace_root) {
        if behind > 0 {
            bail!("remote_behind: branch is behind or diverged; synchronize it manually first");
        }
        let _ = ahead;
    }
    let upstream = git_optional(
        &ctx.workspace_root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    if upstream.is_some() {
        run_git_ok_timeout(&ctx.workspace_root, &["push"], GIT_TIMEOUT)?;
        return Ok(());
    }
    if !allow_set_upstream {
        bail!("no_upstream: select a remote and allow setting the upstream");
    }
    let remote_owned = remote
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_string)
        .or_else(|| {
            git_optional(&ctx.checkout_root, &["remote"])
                .and_then(|out| choose_default_remote(&out))
        })
        .ok_or_else(|| anyhow!("no_remote: repository has no remote"))?;
    let remote = remote_owned.as_str();
    validate_remote_name(&ctx.checkout_root, remote)?;
    run_git_ok_timeout(
        &ctx.workspace_root,
        &["push", "-u", remote, &branch],
        GIT_TIMEOUT,
    )
}

fn apply_index_mutation(
    ctx: &RepoContext,
    snapshot: &SessionGitDiffSnapshot,
    input: &GitIndexMutationInput,
) -> Result<()> {
    match input.target.kind {
        GitMutationTargetKind::All => match input.action {
            GitMutationAction::Stage => run_git_ok(&ctx.checkout_root, &["add", "-A", "--", "."]),
            GitMutationAction::Unstage => {
                run_git_ok(&ctx.checkout_root, &["reset", "-q", "HEAD", "--", "."])
            }
            GitMutationAction::Discard => {
                run_git_ok(&ctx.checkout_root, &["restore", "--worktree", "--", "."])?;
                for path in untracked_paths(&ctx.checkout_root)? {
                    remove_untracked_path(&ctx.checkout_root, &path)?;
                }
                Ok(())
            }
        },
        GitMutationTargetKind::File => {
            let path = validated_target_path(snapshot, &input.target)?;
            let change = snapshot
                .changes
                .iter()
                .find(|change| change.path == path)
                .ok_or_else(|| anyhow!("stale_snapshot: selected file no longer exists"))?;
            match input.action {
                GitMutationAction::Stage => match change.old_path.as_deref() {
                    Some(old_path) => {
                        run_git_ok(&ctx.checkout_root, &["add", "-A", "--", old_path, path])
                    }
                    None => run_git_ok(&ctx.checkout_root, &["add", "--", path]),
                },
                GitMutationAction::Unstage => match change.old_path.as_deref() {
                    Some(old_path) => run_git_ok(
                        &ctx.checkout_root,
                        &["reset", "-q", "HEAD", "--", old_path, path],
                    ),
                    None => run_git_ok(&ctx.checkout_root, &["reset", "-q", "HEAD", "--", path]),
                },
                GitMutationAction::Discard => {
                    if change.untracked {
                        remove_untracked_path(&ctx.checkout_root, path)
                    } else if change.conflicted {
                        bail!("conflicted_file: resolve conflicts before discarding from the review panel")
                    } else {
                        run_git_ok(&ctx.checkout_root, &["restore", "--worktree", "--", path])
                    }
                }
            }
        }
        GitMutationTargetKind::Hunk => {
            let path = validated_target_path(snapshot, &input.target)?;
            let change = snapshot
                .changes
                .iter()
                .find(|change| change.path == path)
                .ok_or_else(|| anyhow!("stale_snapshot: selected file no longer exists"))?;
            if change.conflicted {
                bail!("conflicted_file: hunk operations are disabled for conflicts");
            }
            if change.binary || change.submodule || change.untracked || change.old_path.is_some() {
                bail!("file_level_only: this change only supports file-level Git operations");
            }
            let hunk_id = input
                .target
                .hunk_id
                .as_deref()
                .ok_or_else(|| anyhow!("hunkId is required"))?;
            let hunks = parsed_hunks(&ctx.checkout_root, snapshot.scope, path, &snapshot.revision)?;
            let hunk = hunks
                .iter()
                .find(|hunk| hunk.info.id == hunk_id)
                .ok_or_else(|| anyhow!("stale_snapshot: selected hunk no longer exists"))?;
            match input.action {
                GitMutationAction::Stage => {
                    apply_patch(&ctx.checkout_root, &hunk.patch, &["--cached"])
                }
                GitMutationAction::Unstage => {
                    apply_patch(&ctx.checkout_root, &hunk.patch, &["--cached", "--reverse"])
                }
                GitMutationAction::Discard => {
                    apply_patch(&ctx.checkout_root, &hunk.patch, &["--reverse"])
                }
            }
        }
    }
}

fn load_diff_for_context(ctx: &RepoContext, scope: GitDiffScope) -> Result<SessionGitDiffSnapshot> {
    let revision = repository_revision(&ctx.checkout_root)?;
    let mut specs = diff_specs(&ctx.checkout_root, scope)?;
    if matches!(scope, GitDiffScope::Unstaged | GitDiffScope::All) {
        let seen = specs
            .iter()
            .map(|spec| spec.path.clone())
            .collect::<HashSet<_>>();
        for path in untracked_paths(&ctx.checkout_root)? {
            if !seen.contains(path.as_str()) {
                specs.push(ChangeSpec::untracked(path));
            }
        }
    }
    let mut changes = Vec::new();
    for spec in specs {
        if let Some(change) = build_change(ctx, scope, &revision, &spec)? {
            changes.push(change);
        }
    }
    Ok(SessionGitDiffSnapshot {
        revision,
        scope,
        changes,
    })
}

#[derive(Debug, Clone)]
struct ChangeSpec {
    status: String,
    path: String,
    old_path: Option<String>,
    untracked: bool,
}

impl ChangeSpec {
    fn untracked(path: String) -> Self {
        Self {
            status: "??".into(),
            path,
            old_path: None,
            untracked: true,
        }
    }
}

fn diff_specs(root: &Path, scope: GitDiffScope) -> Result<Vec<ChangeSpec>> {
    let mut args = vec!["diff", "--name-status", "-z", "--find-renames"];
    match scope {
        GitDiffScope::Unstaged => {}
        GitDiffScope::Staged => args.push("--cached"),
        GitDiffScope::All => args.push("HEAD"),
    }
    args.extend(["--", "."]);
    parse_name_status(&run_git_bytes(root, &args)?)
}

fn parse_name_status(bytes: &[u8]) -> Result<Vec<ChangeSpec>> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = String::from_utf8_lossy(fields[index]).into_owned();
        index += 1;
        let code = status.chars().next().unwrap_or('M');
        if matches!(code, 'R' | 'C') {
            if index + 1 >= fields.len() {
                break;
            }
            let old_path = String::from_utf8_lossy(fields[index]).into_owned();
            let path = String::from_utf8_lossy(fields[index + 1]).into_owned();
            index += 2;
            out.push(ChangeSpec {
                status,
                path,
                old_path: Some(old_path),
                untracked: false,
            });
        } else {
            if index >= fields.len() {
                break;
            }
            let path = String::from_utf8_lossy(fields[index]).into_owned();
            index += 1;
            out.push(ChangeSpec {
                status,
                path,
                old_path: None,
                untracked: false,
            });
        }
    }
    Ok(out)
}

fn build_change(
    ctx: &RepoContext,
    scope: GitDiffScope,
    revision: &str,
    spec: &ChangeSpec,
) -> Result<Option<GitFileChange>> {
    validate_relative_path(&spec.path)?;
    if let Some(old) = spec.old_path.as_deref() {
        validate_relative_path(old)?;
    }
    let code = spec.status.chars().next().unwrap_or('M');
    let old_path = spec.old_path.as_deref().unwrap_or(&spec.path);
    let before = if spec.untracked || code == 'A' {
        None
    } else {
        match scope {
            GitDiffScope::Unstaged => read_index_text(&ctx.checkout_root, old_path),
            GitDiffScope::Staged | GitDiffScope::All => {
                read_head_text(&ctx.checkout_root, old_path)
            }
        }
    };
    let after = if code == 'D' {
        None
    } else {
        match scope {
            GitDiffScope::Staged => read_index_text(&ctx.checkout_root, &spec.path),
            GitDiffScope::Unstaged | GitDiffScope::All => {
                read_worktree_text(&ctx.checkout_root, &spec.path)
            }
        }
    };
    let binary = before.as_ref().is_some_and(|content| content.binary)
        || after.as_ref().is_some_and(|content| content.binary);
    let submodule = is_submodule(&ctx.checkout_root, &spec.path);
    let truncated = before.as_ref().is_some_and(|content| content.truncated)
        || after.as_ref().is_some_and(|content| content.truncated);
    if before.is_none() && after.is_none() && !binary {
        return Ok(None);
    }
    let (lines_added, lines_removed) =
        numstat_for_path(&ctx.checkout_root, scope, &spec.path).unwrap_or((0, 0));
    let hunks = if spec.untracked || binary || submodule || matches!(code, 'R' | 'C') {
        Vec::new()
    } else {
        parsed_hunks(&ctx.checkout_root, scope, &spec.path, revision)?
            .into_iter()
            .map(|hunk| hunk.info)
            .collect()
    };
    let status_porcelain = git_optional(
        &ctx.checkout_root,
        &["status", "--porcelain=v1", "--", &spec.path],
    )
    .unwrap_or_default();
    let conflicted = status_porcelain.lines().any(|line| {
        let pair = line.as_bytes();
        pair.len() >= 2
            && (pair[0] == b'U' || pair[1] == b'U' || &pair[..2] == b"AA" || &pair[..2] == b"DD")
    });
    Ok(Some(GitFileChange {
        kind: "file_change",
        path: spec.path.clone(),
        old_path: spec.old_path.clone(),
        action: match code {
            'A' => "create",
            'D' => "delete",
            _ => "edit",
        }
        .to_string(),
        status: spec.status.clone(),
        lines_added,
        lines_removed,
        before: before.and_then(|content| content.text),
        after: after.and_then(|content| content.text),
        language: crate::tools::diff_util::detect_language(&spec.path),
        truncated,
        binary,
        submodule,
        conflicted,
        untracked: spec.untracked,
        hunks,
    }))
}

fn is_submodule(root: &Path, path: &str) -> bool {
    git_optional(root, &["ls-files", "--stage", "--", path])
        .is_some_and(|value| value.lines().any(|line| line.starts_with("160000 ")))
        || git_optional(root, &["cat-file", "-t", &format!("HEAD:{path}")]).as_deref()
            == Some("commit")
}

struct TextContent {
    text: Option<String>,
    binary: bool,
    truncated: bool,
}

fn read_head_text(root: &Path, path: &str) -> Option<TextContent> {
    read_git_object(root, &format!("HEAD:{path}"))
}
fn read_index_text(root: &Path, path: &str) -> Option<TextContent> {
    read_git_object(root, &format!(":{path}"))
}

fn read_git_object(root: &Path, object: &str) -> Option<TextContent> {
    let size = git_optional(root, &["cat-file", "-s", object])?
        .parse::<u64>()
        .ok()?;
    if size > MAX_DIFF_TEXT_BYTES {
        return Some(TextContent {
            text: None,
            binary: false,
            truncated: true,
        });
    }
    let bytes = run_git_bytes(root, &["show", object]).ok()?;
    match String::from_utf8(bytes) {
        Ok(text) => Some(TextContent {
            text: Some(text),
            binary: false,
            truncated: false,
        }),
        Err(_) => Some(TextContent {
            text: None,
            binary: true,
            truncated: false,
        }),
    }
}

fn read_worktree_text(root: &Path, path: &str) -> Option<TextContent> {
    let target = root.join(path);
    let metadata = fs::symlink_metadata(&target).ok()?;
    if metadata.file_type().is_symlink() {
        return Some(TextContent {
            text: fs::read_link(target)
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            binary: false,
            truncated: false,
        });
    }
    let canonical = target.canonicalize().ok()?;
    if !canonical.starts_with(root) || !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_DIFF_TEXT_BYTES {
        return Some(TextContent {
            text: None,
            binary: false,
            truncated: true,
        });
    }
    match fs::read(&canonical)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
    {
        Some(text) => Some(TextContent {
            text: Some(text),
            binary: false,
            truncated: false,
        }),
        None => Some(TextContent {
            text: None,
            binary: true,
            truncated: false,
        }),
    }
}

fn parsed_hunks(
    root: &Path,
    scope: GitDiffScope,
    path: &str,
    revision: &str,
) -> Result<Vec<ParsedHunk>> {
    let mut args = vec![
        "diff",
        "--binary",
        "--no-ext-diff",
        "--no-color",
        "--unified=3",
    ];
    match scope {
        GitDiffScope::Unstaged => {}
        GitDiffScope::Staged => args.push("--cached"),
        GitDiffScope::All => args.push("HEAD"),
    }
    args.extend(["--", path]);
    let patch = run_git_bytes(root, &args)?;
    split_patch_hunks(&patch, revision, path)
}

fn split_patch_hunks(patch: &[u8], revision: &str, path: &str) -> Result<Vec<ParsedHunk>> {
    let text = String::from_utf8_lossy(patch);
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let first_hunk = lines.iter().position(|line| line.starts_with("@@ "));
    let Some(first_hunk) = first_hunk else {
        return Ok(Vec::new());
    };
    let header = lines[..first_hunk].concat();
    let mut starts = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.starts_with("@@ ").then_some(i))
        .collect::<Vec<_>>();
    starts.push(lines.len());
    let mut out = Vec::new();
    for pair in starts.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let hunk_text = lines[start..end].concat();
        let header_line = lines[start].trim_end().to_string();
        let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(&header_line)?;
        let full_patch = format!("{header}{hunk_text}").into_bytes();
        let mut id_hasher = blake3::Hasher::new();
        id_hasher.update(revision.as_bytes());
        id_hasher.update(&[0]);
        id_hasher.update(path.as_bytes());
        id_hasher.update(&[0]);
        id_hasher.update(header_line.as_bytes());
        id_hasher.update(&[0]);
        id_hasher.update(&full_patch);
        let id = id_hasher.finalize().to_hex().to_string();
        out.push(ParsedHunk {
            info: GitHunkInfo {
                id,
                header: header_line,
                old_start,
                old_lines,
                new_start,
                new_lines,
            },
            patch: full_patch,
        });
    }
    Ok(out)
}

fn parse_hunk_header(header: &str) -> Result<(u32, u32, u32, u32)> {
    let body = header
        .strip_prefix("@@ -")
        .and_then(|v| v.split_once(" @@").map(|p| p.0))
        .ok_or_else(|| anyhow!("invalid hunk header"))?;
    let (old, new) = body
        .split_once(" +")
        .ok_or_else(|| anyhow!("invalid hunk header"))?;
    let parse = |part: &str| -> Result<(u32, u32)> {
        let (start, lines) = part.split_once(',').unwrap_or((part, "1"));
        Ok((start.parse()?, lines.parse()?))
    };
    let (old_start, old_lines) = parse(old)?;
    let (new_start, new_lines) = parse(new)?;
    Ok((old_start, old_lines, new_start, new_lines))
}

fn apply_patch(root: &Path, patch: &[u8], options: &[&str]) -> Result<()> {
    let mut check_args = vec!["apply", "--binary", "--check"];
    check_args.extend_from_slice(options);
    run_git_with_stdin(root, &check_args, patch)?;
    let mut args = vec!["apply", "--binary"];
    args.extend_from_slice(options);
    run_git_with_stdin(root, &args, patch)
}

fn validated_target_path<'a>(
    snapshot: &'a SessionGitDiffSnapshot,
    target: &'a GitMutationTarget,
) -> Result<&'a str> {
    let path = target
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("path is required"))?;
    validate_relative_path(path)?;
    if !snapshot.changes.iter().any(|change| change.path == path) {
        bail!("stale_snapshot: selected file no longer exists");
    }
    Ok(path)
}

fn repo_context(db: &SessionDB, session_id: &str) -> Result<RepoContext> {
    let meta = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let effective = effective_working_dir_for_meta(&meta)
        .ok_or_else(|| anyhow!("session has no working directory"))?;
    let scope = WorkspaceScope::for_session(session_id).context("workspace scope unavailable")?;
    let workspace_root = scope.root().canonicalize()?;
    if workspace_root != Path::new(&effective).canonicalize()? {
        bail!("session workspace changed while resolving Git state");
    }
    let checkout_root = PathBuf::from(run_git(&workspace_root, &["rev-parse", "--show-toplevel"])?)
        .canonicalize()?;
    if !workspace_root.starts_with(&checkout_root) {
        bail!("workspace is outside the resolved Git checkout");
    }
    let common_dir = git_common_dir(&checkout_root)?;
    Ok(RepoContext {
        workspace_root,
        checkout_root,
        common_dir,
    })
}

fn git_common_dir(checkout_root: &Path) -> Result<PathBuf> {
    let raw = PathBuf::from(run_git(checkout_root, &["rev-parse", "--git-common-dir"])?);
    let common = if raw.is_absolute() {
        raw
    } else {
        checkout_root.join(raw)
    };
    common
        .canonicalize()
        .context("canonicalize Git common directory")
}

fn acquire_repo_lock(repository_identity: &Path) -> Result<RepoLock> {
    let path = crate::paths::git_repo_lock_path(repository_identity)?;
    let file = crate::platform::try_acquire_exclusive_lock(&path)?
        .ok_or_else(|| anyhow!("repo_busy: another Git operation is already running"))?;
    Ok(RepoLock { _file: file })
}

fn ensure_session_idle(db: &SessionDB, session_id: &str) -> Result<()> {
    if is_session_busy(session_id) {
        bail!("workspace_busy: wait for the active task to finish");
    }
    if !crate::async_jobs::JobManager::list_active_work_by_session(session_id)?.is_empty() {
        bail!("background_jobs_active: stop active background jobs before changing Git state");
    }
    if db
        .list_workflow_runs_for_session(session_id, 200)?
        .iter()
        .any(|run| {
            matches!(
                run.state,
                WorkflowRunState::Running | WorkflowRunState::Recovering
            )
        })
    {
        bail!("workflow_active: stop the active workflow before changing Git state");
    }
    Ok(())
}

fn is_session_busy(session_id: &str) -> bool {
    crate::subagent::ACTIVE_CHAT_SESSIONS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(session_id)
        .copied()
        .unwrap_or(0)
        > 0
}

fn repository_revision(root: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for args in [
        vec!["rev-parse", "HEAD"],
        vec!["status", "--porcelain=v1", "-z"],
        vec!["diff", "--binary"],
        vec!["diff", "--binary", "--cached"],
    ] {
        match run_git_bytes(root, &args) {
            Ok(bytes) => hasher.update(&bytes),
            Err(err) => hasher.update(err.to_string().as_bytes()),
        };
        hasher.update(&[0]);
    }
    for path in untracked_paths(root)? {
        hasher.update(path.as_bytes());
        if let Ok(meta) = fs::symlink_metadata(root.join(&path)) {
            hasher.update(&meta.len().to_le_bytes());
            if let Ok(modified) = meta.modified().and_then(|v| {
                v.duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            }) {
                hasher.update(&modified.as_nanos().to_le_bytes());
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn require_revision(ctx: &RepoContext, expected: &str) -> Result<()> {
    if expected.is_empty() || repository_revision(&ctx.checkout_root)? != expected {
        bail!("stale_snapshot: repository changed; refresh and retry");
    }
    Ok(())
}

fn require_clean(root: &Path, operation: &str) -> Result<()> {
    let info = git_info(root).ok_or_else(|| anyhow!("Git information unavailable"))?;
    if info.dirty.changed_files > 0 {
        bail!("dirty_worktree: clean the workspace before {operation}");
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("invalid_path: Git path must stay inside the repository");
    }
    Ok(())
}

fn untracked_paths(root: &Path) -> Result<Vec<String>> {
    Ok(
        run_git_bytes(root, &["ls-files", "--others", "--exclude-standard", "-z"])?
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect(),
    )
}

fn remove_untracked_path(root: &Path, path: &str) -> Result<()> {
    validate_relative_path(path)?;
    let target = root.join(path);
    let meta = fs::symlink_metadata(&target)?;
    if meta.file_type().is_symlink() {
        bail!("symlink_not_supported: refusing to delete an untracked symlink");
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("invalid path"))?
        .canonicalize()?;
    if !parent.starts_with(root) {
        bail!("path_outside_repo: refusing to delete outside repository");
    }
    if meta.is_dir() {
        fs::remove_dir_all(&target)?;
    } else {
        fs::remove_file(&target)?;
    }
    Ok(())
}

fn snapshot_untracked_file(source: &Path, run_dir: &Path, path: &str) -> Result<()> {
    validate_relative_path(path)?;
    let source_path = source.join(path);
    let metadata = fs::symlink_metadata(&source_path)?;
    if metadata.file_type().is_symlink() {
        bail!("symlink_not_supported: untracked symlinks cannot be handed off");
    }
    if !metadata.is_file() {
        bail!("unsupported_untracked_entry: only regular untracked files can be handed off");
    }
    let canonical = source_path.canonicalize()?;
    if !canonical.starts_with(source) {
        bail!("path_outside_repo: untracked file resolves outside repository");
    }
    let destination = run_dir.join("untracked").join(path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(canonical, destination)?;
    Ok(())
}

fn restore_untracked_files(target: &Path, run_dir: &Path, paths: &[String]) -> Result<()> {
    for path in paths {
        validate_relative_path(path)?;
        let source = run_dir.join("untracked").join(path);
        let destination = target.join(path);
        if destination.exists() {
            bail!("untracked_collision: target already contains {path}");
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
            let canonical_parent = parent.canonicalize()?;
            if !canonical_parent.starts_with(target) {
                bail!("path_outside_repo: target path resolves outside repository");
            }
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn clean_checkout(root: &Path, untracked: &[String]) -> Result<()> {
    run_git_ok(root, &["reset", "--hard", "HEAD"])?;
    for path in untracked {
        let target = root.join(path);
        if target.exists() {
            remove_untracked_path(root, path)?;
        }
    }
    Ok(())
}

fn persist_handoff_manifest(run_dir: &Path, manifest: &HandoffManifest) -> Result<()> {
    crate::platform::write_atomic(
        &run_dir.join("metadata.json"),
        &serde_json::to_vec_pretty(manifest)?,
    )?;
    let mut bytes = Vec::new();
    for path in &manifest.untracked {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
    }
    crate::platform::write_atomic(&run_dir.join("untracked.manifest"), &bytes)?;
    Ok(())
}

fn snapshot_fingerprint(
    staged: &[u8],
    unstaged: &[u8],
    run_dir: &Path,
    untracked: &[String],
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(staged);
    hasher.update(&[0]);
    hasher.update(unstaged);
    hasher.update(&[0]);
    for path in untracked {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(run_dir.join("untracked").join(path))?);
        hasher.update(&[0]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn checkout_fingerprint(root: &Path, run_dir: &Path, untracked: &[String]) -> Result<String> {
    let staged = run_git_bytes(root, &["diff", "--binary", "--cached", "HEAD", "--"])?;
    let unstaged = run_git_bytes(root, &["diff", "--binary", "--"])?;
    let actual_untracked = untracked_paths(root)?;
    if actual_untracked != untracked {
        bail!("handoff_verification_failed: target untracked manifest differs");
    }
    let verify_dir = run_dir.join("verify");
    if verify_dir.exists() {
        fs::remove_dir_all(&verify_dir)?;
    }
    fs::create_dir_all(verify_dir.join("untracked"))?;
    for path in untracked {
        let destination = verify_dir.join("untracked").join(path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(root.join(path), destination)?;
    }
    let fingerprint = snapshot_fingerprint(&staged, &unstaged, &verify_dir, untracked)?;
    fs::remove_dir_all(verify_dir)?;
    Ok(fingerprint)
}

fn rollback_handoff(
    source: &Path,
    target: &Path,
    run_dir: &Path,
    manifest: &HandoffManifest,
    staged: &[u8],
    unstaged: &[u8],
) -> Result<()> {
    let target_branch_now = current_branch(target);
    let target_holds_source_branch = manifest
        .source_branch
        .as_deref()
        .is_some_and(|branch| target_branch_now.as_deref() == Some(branch));
    let target_was_prepared = manifest.source_branch.is_none() || target_holds_source_branch;

    if target_was_prepared {
        let target_head_now = run_git(target, &["rev-parse", "HEAD"])?;
        if target_head_now != manifest.source_head {
            bail!(
                "handoff_rollback_conflict: target HEAD changed after handoff started; preserving target"
            );
        }
        validate_snapshot_untracked_on_target(target, run_dir, &manifest.untracked)?;
        reverse_patch_if_present(target, unstaged, &[], "unstaged")?;
        reverse_patch_if_present(target, staged, &["--index"], "staged")?;
        remove_snapshot_untracked_from_target(target, &manifest.untracked)?;
    }

    if target_holds_source_branch {
        run_git_ok(target, &["switch", "--detach", &manifest.source_head])?;
    }
    if let Some(branch) = manifest.source_branch.as_deref() {
        if current_branch(source).as_deref() != Some(branch) {
            run_git_ok(source, &["switch", "--no-guess", branch])?;
        }
    } else if run_git(source, &["rev-parse", "HEAD"])? != manifest.source_head {
        run_git_ok(source, &["switch", "--detach", &manifest.source_head])?;
    }

    let source_head_now = run_git(source, &["rev-parse", "HEAD"])?;
    if source_head_now != manifest.source_head {
        bail!(
            "handoff_rollback_conflict: source HEAD changed after handoff started; preserving source"
        );
    }
    let expected = snapshot_fingerprint(staged, unstaged, run_dir, &manifest.untracked)?;
    let source_already_restored = checkout_fingerprint(source, run_dir, &manifest.untracked)
        .is_ok_and(|actual| actual == expected);
    if !source_already_restored {
        require_clean(source, "restoring the handoff source")?;
        if !staged.is_empty() {
            apply_patch(source, staged, &["--index"])?;
        }
        if !unstaged.is_empty() {
            apply_patch(source, unstaged, &[])?;
        }
        restore_untracked_files(source, run_dir, &manifest.untracked)?;
    }

    if let Some(branch) = manifest.target_branch.as_deref() {
        if current_branch(target).as_deref() != Some(branch) {
            run_git_ok(target, &["switch", "--no-guess", branch])?;
        }
    } else if run_git(target, &["rev-parse", "HEAD"])? != manifest.target_head {
        run_git_ok(target, &["switch", "--detach", &manifest.target_head])?;
    }
    Ok(())
}

fn validate_snapshot_untracked_on_target(
    target: &Path,
    run_dir: &Path,
    paths: &[String],
) -> Result<()> {
    for path in paths {
        validate_relative_path(path)?;
        let destination = target.join(path);
        if !destination.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&destination)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("handoff_rollback_conflict: target untracked entry changed externally: {path}");
        }
        let snapshot = run_dir.join("untracked").join(path);
        if fs::read(&destination)? != fs::read(&snapshot)? {
            bail!("handoff_rollback_conflict: target untracked file changed externally: {path}");
        }
    }
    Ok(())
}

fn remove_snapshot_untracked_from_target(target: &Path, paths: &[String]) -> Result<()> {
    for path in paths {
        let destination = target.join(path);
        if destination.exists() {
            remove_untracked_path(target, path)?;
        }
    }
    Ok(())
}

fn reverse_patch_if_present(
    root: &Path,
    patch: &[u8],
    options: &[&str],
    label: &str,
) -> Result<()> {
    if patch.is_empty() {
        return Ok(());
    }
    let current = if options.contains(&"--index") {
        run_git_bytes(root, &["diff", "--binary", "--cached", "HEAD", "--"])?
    } else {
        run_git_bytes(root, &["diff", "--binary", "--"])?
    };
    if current.is_empty() {
        return Ok(());
    }
    if current != patch {
        let status = run_git(root, &["status", "--short"]).unwrap_or_default();
        bail!(
            "handoff_rollback_conflict: target {label} changes no longer match the handoff snapshot ({})",
            status.trim().replace('\n', ", ")
        );
    }
    if options.contains(&"--index") {
        let unstaged = run_git_bytes(root, &["diff", "--binary", "--"])?;
        if !unstaged.is_empty() {
            bail!(
                "handoff_rollback_conflict: target worktree changed while restoring staged changes"
            );
        }
        run_git_ok(root, &["update-index", "--refresh"])?;
    }
    let mut reverse = options.to_vec();
    reverse.push("--reverse");
    apply_patch(root, patch, &reverse)
        .with_context(|| format!("failed to reverse {label} handoff patch"))
}

fn load_remotes(root: &Path) -> Vec<GitRemoteInfo> {
    let names = git_optional(root, &["remote"]).unwrap_or_default();
    names
        .lines()
        .filter_map(|name| {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            let fetch = git_optional(root, &["remote", "get-url", name])?;
            let push = git_optional(root, &["remote", "get-url", "--push", name])
                .unwrap_or_else(|| fetch.clone());
            let clean_fetch = sanitize_remote_url(&fetch);
            let clean_push = sanitize_remote_url(&push);
            let host = remote_host(&clean_fetch);
            Some(GitRemoteInfo {
                name: name.to_string(),
                fetch_url: clean_fetch,
                push_url: clean_push,
                is_default: name == "origin",
                is_github: host.as_deref().is_some_and(|host| {
                    host.eq_ignore_ascii_case("github.com")
                        || host.to_ascii_lowercase().contains("github")
                }),
                host,
            })
        })
        .collect()
}

fn pr_unavailable(code: &str, message: &str) -> GitPullRequestPreflight {
    GitPullRequestPreflight {
        available: false,
        gh_available: which::which("gh").is_ok(),
        authenticated: false,
        host: None,
        repository: None,
        default_branch: None,
        current: None,
        error_code: Some(code.to_string()),
        error_message: Some(message.to_string()),
    }
}

fn empty_pull_request_feedback(preflight: GitPullRequestPreflight) -> GitPullRequestFeedback {
    GitPullRequestFeedback {
        preflight,
        checks: Vec::new(),
        review_comments: Vec::new(),
        failed_checks: 0,
        pending_checks: 0,
        passed_checks: 0,
        unresolved_comments: 0,
        checks_truncated: false,
        comments_truncated: false,
        checks_error: None,
        comments_error: None,
    }
}

fn default_base_branch(root: &Path) -> Option<String> {
    git_optional(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .and_then(|name| name.split_once('/').map(|(_, branch)| branch.to_string()))
    .or_else(|| {
        git_optional(root, &["show-ref", "--verify", "refs/heads/main"]).map(|_| "main".to_string())
    })
    .or_else(|| {
        git_optional(root, &["show-ref", "--verify", "refs/heads/master"])
            .map(|_| "master".to_string())
    })
}

fn load_current_pr(gh: &Path, root: &Path, branch: &str) -> Result<Option<GitPullRequestInfo>> {
    let output = run_command_timeout(
        gh_command(
            gh,
            root,
            &[
                "pr",
                "view",
                branch,
                "--json",
                "number,title,body,url,state,isDraft,baseRefName,headRefName,author,additions,deletions,changedFiles,mergeable,mergeStateStatus,reviewDecision,autoMergeRequest,reviewRequests,latestReviews",
            ],
        ),
        GH_TIMEOUT,
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(parse_pull_request_info(&output.stdout, branch)?))
}

fn parse_pull_request_info(bytes: &[u8], branch: &str) -> Result<GitPullRequestInfo> {
    let value: Value = serde_json::from_slice(bytes).context("decode gh pr view")?;
    let reviewers = value
        .get("reviewRequests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|reviewer| {
            let login = value_string(reviewer, "login")
                .or_else(|| value_string(reviewer, "slug"))
                .or_else(|| value_string(reviewer, "name"))?;
            let kind = value_string(reviewer, "__typename").unwrap_or_else(|| "User".to_string());
            Some(GitPullRequestReviewer { login, kind })
        })
        .take(100)
        .collect();
    let reviews = value
        .get("latestReviews")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(100)
        .map(|review| GitPullRequestReview {
            id: value_string(review, "id").unwrap_or_default(),
            author: review
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            state: value_string(review, "state").unwrap_or_else(|| "COMMENTED".to_string()),
            body: bounded_text(
                review
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                16_000,
            ),
            submitted_at: value_string(review, "submittedAt"),
            commit_oid: review
                .pointer("/commit/oid")
                .and_then(Value::as_str)
                .map(str::to_string),
            url: value_string(review, "url"),
        })
        .collect();
    let auto_merge_request = value
        .get("autoMergeRequest")
        .filter(|request| !request.is_null());
    Ok(GitPullRequestInfo {
        number: value.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        url: value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state: value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("OPEN")
            .to_string(),
        is_draft: value
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        base_branch: value
            .get("baseRefName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        head_branch: value
            .get("headRefName")
            .and_then(Value::as_str)
            .unwrap_or(branch)
            .to_string(),
        body: bounded_text(
            value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            32_000,
        ),
        author: value
            .pointer("/author/login")
            .and_then(Value::as_str)
            .map(str::to_string),
        additions: value.get("additions").and_then(Value::as_u64).unwrap_or(0),
        deletions: value.get("deletions").and_then(Value::as_u64).unwrap_or(0),
        changed_files: value
            .get("changedFiles")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        mergeable: value_string(&value, "mergeable").unwrap_or_else(|| "UNKNOWN".to_string()),
        merge_state_status: value_string(&value, "mergeStateStatus")
            .unwrap_or_else(|| "UNKNOWN".to_string()),
        review_decision: value_string(&value, "reviewDecision"),
        auto_merge_enabled: auto_merge_request.is_some(),
        auto_merge_method: auto_merge_request
            .and_then(|request| value_string(request, "mergeMethod")),
        reviewers,
        reviews,
    })
}

fn load_pull_request_checks(
    gh: &Path,
    root: &Path,
    pull_request_number: u64,
) -> Result<(Vec<GitPullRequestCheck>, bool)> {
    let number = pull_request_number.to_string();
    let output = run_command_timeout(
        gh_command(
            gh,
            root,
            &[
                "pr",
                "checks",
                &number,
                "--json",
                "bucket,completedAt,description,link,name,startedAt,state,workflow",
            ],
        ),
        GH_TIMEOUT,
    )?;
    if output.stdout.is_empty() && !output.status.success() {
        let error = output_error(&output);
        if error.to_ascii_lowercase().contains("no checks reported") {
            return Ok((Vec::new(), false));
        }
        bail!("gh_pr_checks_failed: {error}");
    }
    parse_pull_request_checks(&output.stdout)
}

fn parse_pull_request_checks(bytes: &[u8]) -> Result<(Vec<GitPullRequestCheck>, bool)> {
    let value: Value = serde_json::from_slice(bytes).context("decode gh pr checks")?;
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("decode gh pr checks: expected an array"))?;
    let truncated = values.len() > 100;
    let checks = values
        .iter()
        .take(100)
        .map(|check| GitPullRequestCheck {
            name: value_string(check, "name").unwrap_or_else(|| "Unnamed check".to_string()),
            workflow: value_string(check, "workflow"),
            state: value_string(check, "state").unwrap_or_else(|| "UNKNOWN".to_string()),
            bucket: value_string(check, "bucket")
                .unwrap_or_else(|| "pending".to_string())
                .to_ascii_lowercase(),
            description: value_string(check, "description").map(|text| bounded_text(&text, 2_000)),
            link: value_string(check, "link"),
            started_at: value_string(check, "startedAt"),
            completed_at: value_string(check, "completedAt"),
        })
        .collect();
    Ok((checks, truncated))
}

const REVIEW_THREADS_QUERY: &str = r#"
query PullRequestReviewThreads($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        pageInfo { hasNextPage }
        nodes {
          id
          isResolved
          isOutdated
          comments(first: 1) {
            totalCount
            nodes {
              id
              author { login }
              body
              path
              line
              originalLine
              startLine
              originalStartLine
              diffSide
              url
              createdAt
            }
          }
        }
      }
    }
  }
}
"#;

fn load_pull_request_review_comments(
    gh: &Path,
    root: &Path,
    host: &str,
    repository: &str,
    pull_request_number: u64,
) -> Result<(Vec<GitPullRequestReviewComment>, bool)> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| anyhow!("gh_repo_unavailable: expected owner/name repository"))?;
    let number = pull_request_number.to_string();
    let query = format!("query={REVIEW_THREADS_QUERY}");
    let owner_field = format!("owner={owner}");
    let name_field = format!("name={name}");
    let number_field = format!("number={number}");
    let output = run_command_timeout(
        gh_command(
            gh,
            root,
            &[
                "api",
                "graphql",
                "--hostname",
                host,
                "-f",
                &query,
                "-F",
                &owner_field,
                "-F",
                &name_field,
                "-F",
                &number_field,
            ],
        ),
        GH_TIMEOUT,
    )?;
    if !output.status.success() {
        bail!("gh_pr_comments_failed: {}", output_error(&output));
    }
    parse_pull_request_review_comments(&output.stdout)
}

fn parse_pull_request_review_comments(
    bytes: &[u8],
) -> Result<(Vec<GitPullRequestReviewComment>, bool)> {
    let value: Value = serde_json::from_slice(bytes).context("decode GitHub review threads")?;
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        if !errors.is_empty() {
            let message = errors
                .iter()
                .filter_map(|error| value_string(error, "message"))
                .collect::<Vec<_>>()
                .join("; ");
            bail!("gh_pr_comments_failed: {}", bounded_text(&message, 2_000));
        }
    }
    let threads = value
        .pointer("/data/repository/pullRequest/reviewThreads")
        .ok_or_else(|| anyhow!("decode GitHub review threads: missing reviewThreads"))?;
    let truncated = threads
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let nodes = threads
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("decode GitHub review threads: expected nodes"))?;
    let mut comments = Vec::new();
    for thread in nodes {
        let is_resolved = thread
            .get("isResolved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_outdated = thread
            .get("isOutdated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let comment_connection = thread
            .get("comments")
            .ok_or_else(|| anyhow!("decode GitHub review threads: missing comments"))?;
        let comment_nodes = comment_connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("decode GitHub review threads: expected comment nodes"))?;
        let Some(comment) = comment_nodes.first() else {
            continue;
        };
        comments.push(GitPullRequestReviewComment {
            thread_id: value_string(thread, "id").unwrap_or_default(),
            comment_id: value_string(comment, "id").unwrap_or_default(),
            author: comment
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            body: bounded_text(
                comment
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                16_000,
            ),
            path: value_string(comment, "path").unwrap_or_default(),
            line: comment
                .get("line")
                .and_then(Value::as_u64)
                .or_else(|| comment.get("originalLine").and_then(Value::as_u64)),
            start_line: comment
                .get("startLine")
                .and_then(Value::as_u64)
                .or_else(|| comment.get("originalStartLine").and_then(Value::as_u64)),
            side: value_string(comment, "diffSide"),
            url: value_string(comment, "url"),
            created_at: value_string(comment, "createdAt"),
            reply_count: comment_connection
                .get("totalCount")
                .and_then(Value::as_u64)
                .unwrap_or(comment_nodes.len() as u64)
                .saturating_sub(1) as usize,
            is_resolved,
            is_outdated,
        });
    }
    comments.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok((comments, truncated))
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn remote_host(remote: &str) -> Option<String> {
    if let Ok(url) = url::Url::parse(remote) {
        return url.host_str().map(str::to_string);
    }
    remote
        .split_once(':')
        .map(|(host, _)| host.trim_start_matches("ssh://").to_string())
}

fn sanitize_remote_url(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw.trim()) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    let without_user = raw
        .trim()
        .split_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(raw.trim());
    without_user
        .split(['?', '#'])
        .next()
        .unwrap_or(without_user)
        .to_string()
}

fn ahead_behind(root: &Path) -> Option<(u32, u32)> {
    let out = git_optional(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )?;
    let mut parts = out.split_whitespace();
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn choose_default_remote(output: &str) -> Option<String> {
    let names = output
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    names
        .iter()
        .copied()
        .find(|name| *name == "origin")
        .or_else(|| (names.len() == 1).then_some(names[0]))
        .map(str::to_string)
}

fn validate_remote_name(root: &Path, remote: &str) -> Result<()> {
    if remote.is_empty() || remote.starts_with('-') || remote.contains(['\0', '\n', '\r']) {
        bail!("invalid remote name");
    }
    let names = run_git(root, &["remote"])?;
    if !names.lines().any(|name| name.trim() == remote) {
        bail!("remote_not_found: selected remote does not exist");
    }
    Ok(())
}

fn numstat_for_path(root: &Path, scope: GitDiffScope, path: &str) -> Option<(u32, u32)> {
    let mut args = vec!["diff", "--numstat"];
    match scope {
        GitDiffScope::Unstaged => {}
        GitDiffScope::Staged => args.push("--cached"),
        GitDiffScope::All => args.push("HEAD"),
    }
    args.extend(["--", path]);
    let out = git_optional(root, &args)?;
    let mut fields = out.split_whitespace();
    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

fn current_branch(root: &Path) -> Option<String> {
    git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

fn result_from_context(
    ctx: &RepoContext,
    message: &str,
    url: Option<String>,
) -> Result<GitMutationResult> {
    Ok(GitMutationResult {
        revision: repository_revision(&ctx.checkout_root)?,
        head: git_optional(&ctx.checkout_root, &["rev-parse", "HEAD"]),
        branch: current_branch(&ctx.workspace_root),
        message: message.to_string(),
        url,
        warning: None,
    })
}

fn with_idempotent_operation<F>(
    db: &SessionDB,
    session_id: &str,
    request_id: &str,
    operation: &str,
    run: F,
) -> Result<GitMutationResult>
where
    F: FnOnce(RepoContext) -> Result<GitMutationResult>,
{
    ensure_session_idle(db, session_id)?;
    if let Some(existing) = db.get_git_operation_run(request_id)? {
        if existing.session_id != session_id || existing.operation != operation {
            bail!("request_id_conflict: requestId belongs to another Git operation");
        }
        if existing.status == "completed" {
            return serde_json::from_value(
                existing
                    .result
                    .ok_or_else(|| anyhow!("completed Git operation has no result"))?,
            )
            .context("decode Git operation result");
        }
        if existing.status == "running" {
            bail!("operation_running: this Git operation is already running");
        }
        bail!("operation_already_finished: retry with a new requestId");
    }
    let ctx = repo_context(db, session_id)?;
    let _lock = acquire_repo_lock(&ctx.common_dir)?;
    if let Some(existing) = db.get_git_operation_run(request_id)? {
        if existing.session_id != session_id || existing.operation != operation {
            bail!("request_id_conflict: requestId belongs to another Git operation");
        }
        if existing.status == "completed" {
            return serde_json::from_value(
                existing
                    .result
                    .ok_or_else(|| anyhow!("completed Git operation has no result"))?,
            )
            .context("decode Git operation result");
        }
        if existing.status == "running" {
            bail!("operation_running: this Git operation is already running");
        }
        bail!("operation_already_finished: retry with a new requestId");
    }
    let before_head = git_optional(&ctx.checkout_root, &["rev-parse", "HEAD"]);
    db.insert_git_operation_run(request_id, session_id, operation, before_head.as_deref())?;
    emit_progress(
        request_id,
        session_id,
        operation,
        "running",
        "executing",
        None,
        None,
    );
    match run(ctx) {
        Ok(result) => {
            db.complete_git_operation_run(request_id, &result)?;
            emit_git_changed(session_id, operation, Some(request_id));
            emit_progress(
                request_id,
                session_id,
                operation,
                "completed",
                "completed",
                Some(&result.message),
                None,
            );
            Ok(result)
        }
        Err(error) => {
            let message = format!("{error:#}");
            let code = message
                .split_once(':')
                .map(|(code, _)| code)
                .filter(|code| code.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                .unwrap_or("git_operation_failed");
            db.fail_git_operation_run(request_id, code, &message)?;
            emit_progress(
                request_id,
                session_id,
                operation,
                "failed",
                "failed",
                Some(&message),
                Some(code),
            );
            Err(error)
        }
    }
}

impl SessionDB {
    pub fn get_git_operation_run(&self, id: &str) -> Result<Option<GitOperationRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row("SELECT id, session_id, operation, status, stage, before_head, after_head, result_json, error_code, error_message, created_at, updated_at, completed_at FROM git_operation_runs WHERE id=?1", params![id], row_to_operation).optional().map_err(Into::into)
    }

    fn insert_git_operation_run(
        &self,
        id: &str,
        session_id: &str,
        operation: &str,
        before_head: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("INSERT INTO git_operation_runs (id,session_id,operation,status,stage,before_head,created_at,updated_at) VALUES (?1,?2,?3,'running','executing',?4,?5,?5)", params![id,session_id,operation,before_head,now])?;
        Ok(())
    }

    fn complete_git_operation_run(&self, id: &str, result: &GitMutationResult) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let value = serde_json::to_string(result)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("UPDATE git_operation_runs SET status='completed',stage='completed',after_head=?2,result_json=?3,updated_at=?4,completed_at=?4 WHERE id=?1", params![id,result.head.as_deref(),value,now])?;
        Ok(())
    }

    fn fail_git_operation_run(&self, id: &str, code: &str, message: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("UPDATE git_operation_runs SET status='failed',stage='failed',error_code=?2,error_message=?3,updated_at=?4,completed_at=?4 WHERE id=?1", params![id,code,message,now])?;
        Ok(())
    }

    fn set_git_operation_stage(&self, id: &str, stage: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE git_operation_runs SET stage=?2,updated_at=?3 WHERE id=?1 AND status='running'",
            params![id, stage, now],
        )?;
        Ok(())
    }

    fn mark_handoff_worktrees_active(&self, session_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE managed_worktrees SET state='active',handed_off_at=?2,updated_at=?2
             WHERE session_id=?1 AND state!='archived'",
            params![session_id, now],
        )?;
        Ok(())
    }

    /// Primary-only startup reconciliation. Handoffs persist enough snapshot
    /// material to restore the source checkout; other operations are marked
    /// interrupted because commit/push/PR outcomes cannot be safely replayed.
    pub fn reconcile_interrupted_git_operations(&self) -> Result<usize> {
        let stale = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let mut statement = conn.prepare(
                "SELECT id,session_id,operation,stage FROM git_operation_runs WHERE status='running'",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (id, session_id, operation, stage) in &stale {
            let mut recovery_error = None;
            if operation == "handoff" {
                let run_dir = crate::paths::git_operation_run_dir(id)?;
                let manifest_path = run_dir.join("metadata.json");
                if manifest_path.is_file() && stage != "snapshotting_source" {
                    let recovery = (|| -> Result<()> {
                        let manifest: HandoffManifest =
                            serde_json::from_slice(&fs::read(&manifest_path)?)?;
                        let staged = fs::read(run_dir.join("staged.patch"))?;
                        let unstaged = fs::read(run_dir.join("unstaged.patch"))?;
                        rollback_handoff(
                            Path::new(&manifest.source_path),
                            Path::new(&manifest.target_path),
                            &run_dir,
                            &manifest,
                            &staged,
                            &unstaged,
                        )?;
                        self.update_session_working_dir(
                            &session_id,
                            Some(manifest.source_path.clone()),
                        )?;
                        Ok(())
                    })();
                    if let Err(error) = recovery {
                        recovery_error = Some(format!("handoff recovery failed: {error:#}"));
                    }
                }
                if recovery_error.is_none() {
                    let _ = fs::remove_dir_all(run_dir);
                }
            }
            let now = chrono::Utc::now().timestamp_millis();
            let message = recovery_error.unwrap_or_else(|| {
                "The application restarted while the Git operation was running".to_string()
            });
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            conn.execute(
                "UPDATE git_operation_runs SET status='interrupted',stage='interrupted',
                    error_code='process_restarted',error_message=?2,updated_at=?3,completed_at=?3
                 WHERE id=?1",
                params![id, message, now],
            )?;
        }
        Ok(stale.len())
    }

    fn update_managed_worktree_git_branch_for_path(
        &self,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let path = path.to_string_lossy();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE managed_worktrees SET git_branch=?2,updated_at=?3 WHERE path=?1",
            params![path.as_ref(), branch, now],
        )?;
        Ok(())
    }
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitOperationRun> {
    let result_json: Option<String> = row.get(7)?;
    Ok(GitOperationRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        operation: row.get(2)?,
        status: row.get(3)?,
        stage: row.get(4)?,
        before_head: row.get(5)?,
        after_head: row.get(6)?,
        result: result_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        error_code: row.get(8)?,
        error_message: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

fn emit_progress(
    request_id: &str,
    session_id: &str,
    operation: &str,
    status: &str,
    stage: &str,
    message: Option<&str>,
    error_code: Option<&str>,
) {
    if let Some(bus) = crate::get_event_bus() {
        let payload = GitProgressEvent {
            request_id,
            session_id,
            operation,
            status,
            stage,
            message,
            error_code,
        };
        bus.emit(
            if status == "completed" {
                EVENT_GIT_COMPLETED
            } else {
                EVENT_GIT_PROGRESS
            },
            json!(payload),
        );
    }
}

fn emit_git_changed(session_id: &str, operation: &str, request_id: Option<&str>) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EVENT_GIT_CHANGED,
            json!({"sessionId":session_id,"operation":operation,"requestId":request_id}),
        );
    }
}

fn validate_request_id(id: &str) -> Result<()> {
    let _ = crate::paths::git_operation_run_dir(id)?;
    Ok(())
}

fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(root, args).output()?;
    if !output.status.success() {
        bail!(
            "git_failed: git {} failed: {}",
            args.join(" "),
            output_error(&output)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn git_optional(root: &Path, args: &[&str]) -> Option<String> {
    run_git(root, args)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
fn git_output(root: &Path, args: &[&str]) -> Result<Output> {
    Ok(git_command(root, args).output()?)
}
fn run_git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_command(root, args).output()?;
    if !output.status.success() {
        bail!("git_failed: {}", output_error(&output));
    }
    Ok(output.stdout)
}
fn run_git_ok(root: &Path, args: &[&str]) -> Result<()> {
    let output = git_command(root, args).output()?;
    if !output.status.success() {
        bail!(
            "git_failed: git {} failed: {}",
            args.join(" "),
            output_error(&output)
        );
    }
    Ok(())
}
fn run_git_ok_timeout(root: &Path, args: &[&str], timeout: Duration) -> Result<()> {
    let output = command_output_timeout(git_command(root, args), timeout)?;
    if !output.status.success() {
        bail!(
            "git_failed: git {} failed: {}",
            args.join(" "),
            output_error(&output)
        );
    }
    Ok(())
}
fn run_git_with_stdin(root: &Path, args: &[&str], stdin: &[u8]) -> Result<()> {
    let output = run_command_with_stdin_timeout(git_command(root, args), stdin, GIT_TIMEOUT)?;
    if !output.status.success() {
        bail!("patch_conflict: {}", output_error(&output));
    }
    Ok(())
}
fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    crate::filesystem::isolate_repository_env(&mut cmd);
    cmd.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::hide_console(&mut cmd);
    cmd
}
fn gh_command(gh: &Path, root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(gh);
    cmd.current_dir(root)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::hide_console(&mut cmd);
    cmd
}
fn command_output_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
    let child = command.spawn()?;
    wait_child_with_output(child, timeout, "git_timeout")
}
fn run_command_timeout(command: Command, timeout: Duration) -> Result<Output> {
    command_output_timeout(command, timeout)
}
fn run_command_with_stdin_timeout(
    mut command: Command,
    stdin: &[u8],
    timeout: Duration,
) -> Result<Output> {
    command.stdin(Stdio::piped());
    let mut child = command.spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("command stdin unavailable"))?;
    let bytes = stdin.to_vec();
    let writer = std::thread::spawn(move || input.write_all(&bytes));
    let result = wait_child_with_output(child, timeout, "command_timeout");
    if result.is_ok() {
        writer
            .join()
            .map_err(|_| anyhow!("command stdin writer panicked"))??;
    }
    result
}

fn wait_child_with_output(
    mut child: std::process::Child,
    timeout: Duration,
    timeout_code: &str,
) -> Result<Output> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_pipe(stdout));
    let stderr_reader = std::thread::spawn(move || read_pipe(stderr));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{timeout_code}: command timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("stderr reader panicked"))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe<R: Read>(pipe: Option<R>) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}
fn output_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        stderr.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunks_and_generates_stable_ids() {
        let patch=b"diff --git a/a.txt b/a.txt\nindex 111..222 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n@@ -5 +5,2 @@\n x\n+y\n";
        let hunks = split_patch_hunks(patch, "revision-a", "a.txt").unwrap();
        let other_revision = split_patch_hunks(patch, "revision-b", "a.txt").unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].info.old_start, 1);
        assert_eq!(hunks[1].info.new_lines, 2);
        assert_ne!(hunks[0].info.id, hunks[1].info.id);
        assert_ne!(hunks[0].info.id, other_revision[0].info.id);
    }

    #[test]
    fn parses_pull_request_check_buckets() {
        let payload = br#"[
          {"name":"test (ubuntu)","workflow":"CI","state":"FAILURE","bucket":"fail","description":"failed","link":"https://example.test/1","startedAt":"2026-07-12T00:00:00Z","completedAt":"2026-07-12T00:01:00Z"},
          {"name":"test (macOS)","workflow":"CI","state":"IN_PROGRESS","bucket":"pending","description":"running","link":"https://example.test/2","startedAt":"2026-07-12T00:00:00Z","completedAt":null}
        ]"#;
        let (checks, truncated) = parse_pull_request_checks(payload).unwrap();
        assert!(!truncated);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].bucket, "fail");
        assert_eq!(checks[1].bucket, "pending");
        assert_eq!(checks[1].workflow.as_deref(), Some("CI"));
    }

    #[test]
    fn parses_pull_request_detail_and_review_state() {
        let payload = br#"{
          "number":456,"title":"Safe lifecycle","body":"Summary","url":"https://example.test/pr/456","state":"OPEN","isDraft":false,
          "baseRefName":"main","headRefName":"feature","author":{"login":"author"},"additions":25,"deletions":7,"changedFiles":4,
          "mergeable":"CONFLICTING","mergeStateStatus":"DIRTY","reviewDecision":"CHANGES_REQUESTED",
          "autoMergeRequest":{"mergeMethod":"SQUASH"},
          "reviewRequests":[{"__typename":"User","login":"reviewer"},{"__typename":"Team","name":"Platform","slug":"platform"}],
          "reviews":[{"id":"old-review","author":{"login":"reviewer"},"state":"CHANGES_REQUESTED","body":"Obsolete feedback."}],
          "latestReviews":[{"id":"review-1","author":{"login":"reviewer"},"state":"CHANGES_REQUESTED","body":"Please fix this.","submittedAt":"2026-07-12T00:00:00Z","commit":{"oid":"abcdef"},"url":"https://example.test/review"}]
        }"#;
        let detail = parse_pull_request_info(payload, "fallback").unwrap();
        assert_eq!(detail.number, 456);
        assert_eq!(detail.author.as_deref(), Some("author"));
        assert_eq!(detail.additions, 25);
        assert_eq!(detail.deletions, 7);
        assert_eq!(detail.mergeable, "CONFLICTING");
        assert_eq!(detail.merge_state_status, "DIRTY");
        assert_eq!(detail.review_decision.as_deref(), Some("CHANGES_REQUESTED"));
        assert!(detail.auto_merge_enabled);
        assert_eq!(detail.auto_merge_method.as_deref(), Some("SQUASH"));
        assert_eq!(detail.reviewers.len(), 2);
        assert_eq!(detail.reviewers[1].login, "platform");
        assert_eq!(detail.reviews.len(), 1);
        assert_eq!(detail.reviews[0].id, "review-1");
        assert_eq!(detail.reviews[0].commit_oid.as_deref(), Some("abcdef"));
    }

    #[test]
    fn maps_auto_merge_methods_to_non_force_flags() {
        assert_eq!(GitPullRequestMergeMethod::Merge.gh_flag(), "--merge");
        assert_eq!(GitPullRequestMergeMethod::Squash.gh_flag(), "--squash");
        assert_eq!(GitPullRequestMergeMethod::Rebase.gh_flag(), "--rebase");
    }

    #[test]
    fn parses_unresolved_pull_request_review_threads() {
        let payload = br#"{
          "data":{"repository":{"pullRequest":{"reviewThreads":{
            "pageInfo":{"hasNextPage":false},
            "nodes":[{
              "id":"thread-1","isResolved":false,"isOutdated":false,
              "comments":{"totalCount":2,"nodes":[
                {"id":"comment-1","author":{"login":"reviewer"},"body":"Keep this fail-closed.","path":"src/lib.rs","line":23,"originalLine":23,"startLine":null,"originalStartLine":null,"diffSide":"RIGHT","url":"https://example.test/comment","createdAt":"2026-07-12T00:00:00Z"}
              ]}
            }]}
          }}}}"#;
        let (comments, truncated) = parse_pull_request_review_comments(payload).unwrap();
        assert!(!truncated);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "reviewer");
        assert_eq!(comments[0].path, "src/lib.rs");
        assert_eq!(comments[0].line, Some(23));
        assert_eq!(comments[0].reply_count, 1);
        assert!(!comments[0].is_resolved);
    }

    #[test]
    fn rejects_unsafe_git_paths() {
        assert!(validate_relative_path("../secret").is_err());
        assert!(validate_relative_path("/tmp/x").is_err());
        assert!(validate_relative_path("src/main.rs").is_ok());
    }

    #[test]
    fn handoff_worktree_scope_accepts_only_owner_or_child_session() {
        assert!(worktree_in_session_scope("owner", None, "owner"));
        assert!(worktree_in_session_scope("owner", Some("child"), "child"));
        assert!(!worktree_in_session_scope("other", Some("child"), "owner"));
    }

    #[test]
    fn stages_and_unstages_a_single_hunk() {
        let repo = initialized_repo();
        fs::write(
            repo.path().join("notes.txt"),
            "changed-one\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\nchanged-twelve\n",
        )
        .unwrap();
        let ctx = test_repo_context(repo.path());
        let unstaged = load_diff_for_context(&ctx, GitDiffScope::Unstaged).unwrap();
        let change = unstaged
            .changes
            .iter()
            .find(|change| change.path == "notes.txt")
            .unwrap();
        assert_eq!(change.hunks.len(), 2);

        apply_index_mutation(
            &ctx,
            &unstaged,
            &GitIndexMutationInput {
                expected_revision: unstaged.revision.clone(),
                action: GitMutationAction::Stage,
                target: GitMutationTarget {
                    kind: GitMutationTargetKind::Hunk,
                    path: Some("notes.txt".into()),
                    hunk_id: Some(change.hunks[0].id.clone()),
                },
                confirm_discard: false,
            },
        )
        .unwrap();

        let staged = load_diff_for_context(&ctx, GitDiffScope::Staged).unwrap();
        let remaining = load_diff_for_context(&ctx, GitDiffScope::Unstaged).unwrap();
        assert_eq!(staged.changes[0].hunks.len(), 1);
        assert_eq!(remaining.changes[0].hunks.len(), 1);

        apply_index_mutation(
            &ctx,
            &staged,
            &GitIndexMutationInput {
                expected_revision: staged.revision.clone(),
                action: GitMutationAction::Unstage,
                target: GitMutationTarget {
                    kind: GitMutationTargetKind::All,
                    path: None,
                    hunk_id: None,
                },
                confirm_discard: false,
            },
        )
        .unwrap();
        assert!(load_diff_for_context(&ctx, GitDiffScope::Staged)
            .unwrap()
            .changes
            .is_empty());
    }

    #[test]
    fn discards_only_the_selected_untracked_file() {
        let repo = initialized_repo();
        fs::write(repo.path().join("keep.txt"), "keep").unwrap();
        fs::write(repo.path().join("remove.txt"), "remove").unwrap();
        let ctx = test_repo_context(repo.path());
        let snapshot = load_diff_for_context(&ctx, GitDiffScope::Unstaged).unwrap();
        apply_index_mutation(
            &ctx,
            &snapshot,
            &GitIndexMutationInput {
                expected_revision: snapshot.revision.clone(),
                action: GitMutationAction::Discard,
                target: GitMutationTarget {
                    kind: GitMutationTargetKind::File,
                    path: Some("remove.txt".into()),
                    hunk_id: None,
                },
                confirm_discard: true,
            },
        )
        .unwrap();
        assert!(repo.path().join("keep.txt").is_file());
        assert!(!repo.path().join("remove.txt").exists());
    }

    #[test]
    fn linked_worktrees_share_repository_identity() {
        let repo = initialized_repo();
        let parent = tempfile::tempdir().unwrap();
        let linked = parent.path().join("linked");
        test_git(
            repo.path(),
            &[
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );
        assert_eq!(
            git_common_dir(repo.path()).unwrap(),
            git_common_dir(&linked).unwrap()
        );
        test_git(
            repo.path(),
            &["worktree", "remove", "--force", linked.to_str().unwrap()],
        );
    }

    #[test]
    fn mutations_use_checkout_root_for_nested_session_workspaces() {
        let repo = initialized_repo();
        fs::create_dir_all(repo.path().join("project")).unwrap();
        fs::write(repo.path().join("root.txt"), "before\n").unwrap();
        fs::write(repo.path().join("project/inside.txt"), "inside\n").unwrap();
        test_git(repo.path(), &["add", "."]);
        test_git(repo.path(), &["commit", "-m", "add nested project"]);
        fs::write(repo.path().join("root.txt"), "after\n").unwrap();

        let checkout_root = repo.path().canonicalize().unwrap();
        let ctx = RepoContext {
            workspace_root: checkout_root.join("project"),
            checkout_root: checkout_root.clone(),
            common_dir: git_common_dir(&checkout_root).unwrap(),
        };
        let snapshot = load_diff_for_context(&ctx, GitDiffScope::Unstaged).unwrap();
        assert!(snapshot
            .changes
            .iter()
            .any(|change| change.path == "root.txt"));
        apply_index_mutation(
            &ctx,
            &snapshot,
            &GitIndexMutationInput {
                expected_revision: snapshot.revision.clone(),
                action: GitMutationAction::Stage,
                target: GitMutationTarget {
                    kind: GitMutationTargetKind::File,
                    path: Some("root.txt".into()),
                    hunk_id: None,
                },
                confirm_discard: false,
            },
        )
        .unwrap();
        assert_eq!(
            run_git(&checkout_root, &["diff", "--cached", "--name-only"]).unwrap(),
            "root.txt"
        );
    }

    #[test]
    fn commit_stays_successful_when_optional_push_fails() {
        let repo = initialized_repo();
        fs::write(repo.path().join("notes.txt"), "committed\n").unwrap();
        test_git(repo.path(), &["add", "notes.txt"]);
        let ctx = test_repo_context(repo.path());
        let before = run_git(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        let result = commit_changes(
            &ctx,
            &GitCommitInput {
                request_id: "commit-request".into(),
                expected_revision: repository_revision(repo.path()).unwrap(),
                subject: "commit before failed push".into(),
                body: None,
                stage_all: false,
                push_after: true,
                remote: Some("missing-remote".into()),
            },
            "commit before failed push",
        )
        .unwrap();
        let after = run_git(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_ne!(before, after);
        assert!(result.warning.is_some());
        assert_eq!(result.head.as_deref(), Some(after.as_str()));
    }

    #[test]
    fn handoff_rollback_preserves_unrelated_target_untracked_files() {
        let repo = initialized_repo();
        let source = repo.path().canonicalize().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let target_path = parent.path().join("target");
        test_git(
            &source,
            &[
                "worktree",
                "add",
                "--detach",
                target_path.to_str().unwrap(),
                "HEAD",
            ],
        );
        let target = target_path.canonicalize().unwrap();

        fs::write(source.join("notes.txt"), "staged\n").unwrap();
        test_git(&source, &["add", "notes.txt"]);
        fs::write(source.join("notes.txt"), "unstaged\n").unwrap();
        fs::write(source.join("handoff.txt"), "handoff\n").unwrap();

        let staged =
            run_git_bytes(&source, &["diff", "--binary", "--cached", "HEAD", "--"]).unwrap();
        let unstaged = run_git_bytes(&source, &["diff", "--binary", "--"]).unwrap();
        let untracked = untracked_paths(&source).unwrap();
        let run = tempfile::tempdir().unwrap();
        fs::create_dir_all(run.path().join("untracked")).unwrap();
        for path in &untracked {
            snapshot_untracked_file(&source, run.path(), path).unwrap();
        }
        let source_head = run_git(&source, &["rev-parse", "HEAD"]).unwrap();
        let target_head = run_git(&target, &["rev-parse", "HEAD"]).unwrap();
        let manifest = HandoffManifest {
            request_id: "request-rollback".into(),
            session_id: "session-rollback".into(),
            source_path: source.to_string_lossy().into_owned(),
            target_path: target.to_string_lossy().into_owned(),
            source_head: source_head.clone(),
            target_head,
            source_branch: Some("main".into()),
            target_branch: None,
            untracked: untracked.clone(),
            stage: "transferring_changes".into(),
        };
        let expected = snapshot_fingerprint(&staged, &unstaged, run.path(), &untracked).unwrap();

        clean_checkout(&source, &untracked).unwrap();
        test_git(&source, &["switch", "--detach", &source_head]);
        test_git(&target, &["switch", "--no-guess", "main"]);
        apply_patch(&target, &staged, &["--index"]).unwrap();
        apply_patch(&target, &unstaged, &[]).unwrap();
        restore_untracked_files(&target, run.path(), &untracked).unwrap();
        fs::write(target.join("external.txt"), "external\n").unwrap();

        rollback_handoff(&source, &target, run.path(), &manifest, &staged, &unstaged).unwrap();

        assert_eq!(
            checkout_fingerprint(&source, run.path(), &untracked).unwrap(),
            expected
        );
        assert_eq!(untracked_paths(&target).unwrap(), vec!["external.txt"]);
        assert_eq!(
            fs::read_to_string(target.join("external.txt")).unwrap(),
            "external\n"
        );
        test_git(
            &source,
            &["worktree", "remove", "--force", target.to_str().unwrap()],
        );
    }

    fn initialized_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        test_git(repo.path(), &["init", "-b", "main"]);
        test_git(repo.path(), &["config", "user.email", "tests@example.com"]);
        test_git(repo.path(), &["config", "user.name", "Hope Agent Tests"]);
        fs::write(
            repo.path().join("notes.txt"),
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n",
        )
        .unwrap();
        test_git(repo.path(), &["add", "notes.txt"]);
        test_git(repo.path(), &["commit", "-m", "initial"]);
        repo
    }

    fn test_repo_context(root: &Path) -> RepoContext {
        let root = root.canonicalize().unwrap();
        RepoContext {
            workspace_root: root.clone(),
            checkout_root: root.clone(),
            common_dir: git_common_dir(&root).unwrap(),
        }
    }

    fn test_git(root: &Path, args: &[&str]) {
        let output = git_command(root, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            output_error(&output)
        );
    }
}
