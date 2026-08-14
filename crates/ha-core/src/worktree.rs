//! Managed git worktrees for durable sessions, workflows, and subagents.
//!
//! This is the product-facing control plane above `git worktree`: every
//! worktree has a durable DB row, lifecycle state, and owner API. The git
//! command remains an implementation detail so desktop, server, workflow, and
//! subagent paths share the same semantics.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::session::{effective_working_dir_for_meta, SessionDB};
use crate::util::now_rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWorktreeState {
    Active,
    Archived,
    Handoff,
    BootstrapFailed,
}

impl ManagedWorktreeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Handoff => "handoff",
            Self::BootstrapFailed => "bootstrap_failed",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "archived" => Self::Archived,
            "handoff" => Self::Handoff,
            "bootstrap_failed" => Self::BootstrapFailed,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWorktreePurpose {
    Manual,
    Workflow,
    Subagent,
    ScheduledRun,
    ScheduledTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWorktreePathSource {
    Builtin,
    Hook,
}

impl ManagedWorktreePathSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Hook => "hook",
        }
    }

    fn from_str(value: &str) -> Self {
        if value == "hook" {
            Self::Hook
        } else {
            Self::Builtin
        }
    }
}

impl ManagedWorktreePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Workflow => "workflow",
            Self::Subagent => "subagent",
            Self::ScheduledRun => "scheduled_run",
            Self::ScheduledTask => "scheduled_task",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "workflow" => Self::Workflow,
            "subagent" => Self::Subagent,
            "scheduled_run" => Self::ScheduledRun,
            "scheduled_task" => Self::ScheduledTask,
            _ => Self::Manual,
        }
    }

    /// Scheduled purposes are minted only by the scheduler's typed kernel API.
    /// Generic owner transports must not let a client forge their provenance.
    pub fn is_owner_transport_safe(self) -> bool {
        matches!(self, Self::Manual | Self::Workflow | Self::Subagent)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktreeDirtySnapshot {
    pub clean: bool,
    pub staged_files: u32,
    pub unstaged_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
    pub changed_files: u32,
}

/// Validate the same source/ref boundary as managed-worktree creation without
/// creating a directory or writing a database row.
pub fn preflight_managed_worktree_source(
    source_working_dir: &str,
    base_ref: Option<&str>,
) -> Result<u32> {
    let source = canonical_dir(source_working_dir)?;
    if !is_inside_git_work_tree(&source) {
        bail!("{} is not inside a git worktree", source.display());
    }
    canonical_dir(git_output(&source, &["rev-parse", "--show-toplevel"])?.trim())?;
    let base_ref = base_ref
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("HEAD");
    resolve_commit(&source, base_ref)?;
    Ok(worktree_dirty_snapshot(&source)
        .ok_or_else(|| anyhow!("{} is not inside a git worktree", source.display()))?
        .changed_files)
}

#[derive(Debug, Clone)]
pub struct ManagedWorktreeRuntimeSnapshot {
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub dirty: ManagedWorktreeDirtySnapshot,
    pub head_diverged: bool,
}

const MANAGED_WORKTREE_SELECT: &str =
    "SELECT id, session_id, child_session_id, workflow_run_id, purpose, state, label,
    repo_root, source_working_dir, path, base_ref, base_branch, base_sha, git_branch,
    dirty_snapshot_json, created_at, updated_at, archived_at, restored_at, handed_off_at,
    path_source, owner_session_id, owner_scheduled_task_id, scheduled_task_id,
    runtime_session_id, runtime_run_id, handoff_session_id FROM managed_worktrees";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktree {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    /// Durable owner. `session_id` remains the immutable creator/audit value;
    /// scheduled-task ownership deliberately survives deletion of that session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_scheduled_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_session_id: Option<String>,
    pub purpose: ManagedWorktreePurpose,
    pub state: ManagedWorktreeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub repo_root: String,
    pub source_working_dir: String,
    pub path: String,
    pub path_source: ManagedWorktreePathSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty_snapshot: Option<ManagedWorktreeDirtySnapshot>,
    pub path_exists: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handed_off_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateManagedWorktreeInput {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_purpose")]
    pub purpose: ManagedWorktreePurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Carry the current local branch's tracked and non-ignored untracked
    /// changes into the new detached worktree.
    #[serde(default)]
    pub include_local_changes: bool,
    /// Required when `include_local_changes` is enabled. Used only to isolate
    /// ephemeral patch/manifest files under the Hope data directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_request_id: Option<String>,
    /// Initial project-chat worktrees become the session cwd immediately,
    /// without being labelled as a later handoff.
    #[serde(default)]
    pub bind_session_working_dir: bool,
}

fn default_purpose() -> ManagedWorktreePurpose {
    ManagedWorktreePurpose::Manual
}

fn ensure_owner_key(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 {
        bail!("{label} must be non-empty and at most 256 bytes");
    }
    Ok(())
}

fn scheduled_session_idle_on(
    conn: &Connection,
    session_id: &str,
    ignored_turn_id: Option<&str>,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND incognito=0)
          AND NOT EXISTS(SELECT 1 FROM chat_turns WHERE session_id=?1
                           AND status IN ('running','cancelling')
                           AND (?2 IS NULL OR id<>?2))
          AND NOT EXISTS(SELECT 1 FROM chat_stream_runs WHERE session_id=?1 AND status='running')
          AND NOT EXISTS(SELECT 1 FROM subagent_runs WHERE parent_session_id=?1
                           AND status IN ('queued','spawning','running'))
          AND NOT EXISTS(SELECT 1 FROM workflow_runs WHERE session_id=?1
                           AND state IN ('running','awaiting_user','paused','recovering'))
          AND NOT EXISTS(SELECT 1 FROM queued_turn_user_messages WHERE session_id=?1
                           AND status IN ('inserting','dispatching'))",
        params![session_id, ignored_turn_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn require_scheduled_session_idle_on(
    conn: &Connection,
    session_id: &str,
    ignored_turn_id: Option<&str>,
) -> Result<()> {
    if !scheduled_session_idle_on(conn, session_id, ignored_turn_id)? {
        bail!("workspace_session_busy");
    }
    Ok(())
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS managed_worktrees (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            child_session_id TEXT,
            workflow_run_id TEXT,
            purpose TEXT NOT NULL,
            state TEXT NOT NULL,
            label TEXT,
            repo_root TEXT NOT NULL,
            source_working_dir TEXT NOT NULL,
            path TEXT NOT NULL,
            base_ref TEXT,
            base_branch TEXT,
            base_sha TEXT,
            git_branch TEXT,
            dirty_snapshot_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            archived_at TEXT,
            restored_at TEXT,
            handed_off_at TEXT,
            path_source TEXT NOT NULL DEFAULT 'builtin',
            owner_session_id TEXT,
            owner_scheduled_task_id TEXT,
            scheduled_task_id TEXT,
            runtime_session_id TEXT,
            runtime_run_id TEXT,
            handoff_session_id TEXT,
            FOREIGN KEY (child_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
            FOREIGN KEY (owner_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (runtime_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
            FOREIGN KEY (handoff_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
            CHECK (
                (owner_session_id IS NOT NULL AND owner_scheduled_task_id IS NULL) OR
                (owner_session_id IS NULL AND owner_scheduled_task_id IS NOT NULL)
            )
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_worktrees_path
            ON managed_worktrees(path);
        CREATE INDEX IF NOT EXISTS idx_managed_worktrees_session_updated
            ON managed_worktrees(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_managed_worktrees_child
            ON managed_worktrees(child_session_id);
        CREATE INDEX IF NOT EXISTS idx_managed_worktrees_workflow
            ON managed_worktrees(workflow_run_id);
        CREATE INDEX IF NOT EXISTS idx_managed_worktrees_state
            ON managed_worktrees(state);
        CREATE TABLE IF NOT EXISTS scheduled_worktree_pending (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS scheduled_worktree_session_custody_bd
        BEFORE DELETE ON sessions WHEN EXISTS (
            SELECT 1 FROM managed_worktrees
             WHERE purpose IN ('scheduled_run','scheduled_task')
               AND (owner_session_id=OLD.id OR runtime_session_id=OLD.id
                    OR handoff_session_id=OLD.id)
        ) BEGIN
            SELECT RAISE(ABORT, 'workspace_session_custody');
        END;",
    )?;
    for (column, ddl) in [
        (
            "child_session_id",
            "ALTER TABLE managed_worktrees ADD COLUMN child_session_id TEXT;",
        ),
        (
            "workflow_run_id",
            "ALTER TABLE managed_worktrees ADD COLUMN workflow_run_id TEXT;",
        ),
        (
            "dirty_snapshot_json",
            "ALTER TABLE managed_worktrees ADD COLUMN dirty_snapshot_json TEXT;",
        ),
        (
            "handed_off_at",
            "ALTER TABLE managed_worktrees ADD COLUMN handed_off_at TEXT;",
        ),
        (
            "path_source",
            "ALTER TABLE managed_worktrees ADD COLUMN path_source TEXT NOT NULL DEFAULT 'builtin';",
        ),
    ] {
        let sql = format!("SELECT {column} FROM managed_worktrees LIMIT 1");
        if conn.prepare(&sql).is_err() {
            conn.execute_batch(ddl)?;
        }
    }
    // Ownership v2: keep `session_id` as creator provenance, but move cascade
    // semantics to `owner_session_id`. This is the one schema rebuild required
    // for a task-owned worktree to survive deletion of its creating run chat.
    if conn
        .prepare("SELECT owner_session_id FROM managed_worktrees LIMIT 1")
        .is_err()
    {
        conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
        let rebuild = conn.execute_batch(
            "BEGIN IMMEDIATE;
             ALTER TABLE managed_worktrees RENAME TO managed_worktrees_owner_v1;
             CREATE TABLE managed_worktrees (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL, child_session_id TEXT,
                workflow_run_id TEXT, purpose TEXT NOT NULL, state TEXT NOT NULL, label TEXT,
                repo_root TEXT NOT NULL, source_working_dir TEXT NOT NULL, path TEXT NOT NULL,
                base_ref TEXT, base_branch TEXT, base_sha TEXT, git_branch TEXT,
                dirty_snapshot_json TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                archived_at TEXT, restored_at TEXT, handed_off_at TEXT,
                path_source TEXT NOT NULL DEFAULT 'builtin', owner_session_id TEXT,
                owner_scheduled_task_id TEXT, scheduled_task_id TEXT,
                runtime_session_id TEXT, runtime_run_id TEXT, handoff_session_id TEXT,
                FOREIGN KEY (child_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY (owner_session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (runtime_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY (handoff_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                CHECK ((owner_session_id IS NOT NULL AND owner_scheduled_task_id IS NULL) OR
                       (owner_session_id IS NULL AND owner_scheduled_task_id IS NOT NULL))
             );
             INSERT INTO managed_worktrees (
                id, session_id, child_session_id, workflow_run_id, purpose, state, label,
                repo_root, source_working_dir, path, base_ref, base_branch, base_sha,
                git_branch, dirty_snapshot_json, created_at, updated_at, archived_at,
                restored_at, handed_off_at, path_source, owner_session_id
             ) SELECT id, session_id, child_session_id, workflow_run_id, purpose, state, label,
                      repo_root, source_working_dir, path, base_ref, base_branch, base_sha,
                      git_branch, dirty_snapshot_json, created_at, updated_at, archived_at,
                      restored_at, handed_off_at, path_source, session_id
                 FROM managed_worktrees_owner_v1;
             DROP TABLE managed_worktrees_owner_v1;
             COMMIT;",
        );
        if let Err(error) = rebuild {
            let _ = conn.execute_batch("ROLLBACK;");
            conn.execute_batch("PRAGMA foreign_keys=ON;")?;
            return Err(error.into());
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let foreign_keys_enabled: i64 =
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        if foreign_keys_enabled != 1 {
            bail!("managed worktree owner migration did not restore foreign key enforcement");
        }
    }
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_managed_worktrees_session_updated;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_worktrees_path
             ON managed_worktrees(path);
         CREATE INDEX IF NOT EXISTS idx_managed_worktrees_session_updated
             ON managed_worktrees(owner_session_id, updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_managed_worktrees_child
             ON managed_worktrees(child_session_id);
         CREATE INDEX IF NOT EXISTS idx_managed_worktrees_workflow
             ON managed_worktrees(workflow_run_id);
         CREATE INDEX IF NOT EXISTS idx_managed_worktrees_state
             ON managed_worktrees(state);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_worktrees_scheduled_task_owner
             ON managed_worktrees(owner_scheduled_task_id)
             WHERE purpose='scheduled_task' AND owner_scheduled_task_id IS NOT NULL;",
    )?;
    Ok(())
}

/// Values computed by the (synchronous, blocking) pre-hook inspection phase of
/// [`SessionDB::create_managed_worktree`], carried across the async hook await
/// into the post-hook creation phase.
struct WorktreePrep {
    source_dir: std::path::PathBuf,
    repo_root: std::path::PathBuf,
    base_ref: String,
    base_sha: String,
    base_branch: Option<String>,
    id: String,
    default_path: std::path::PathBuf,
    local_changes: Option<LocalChangesSnapshot>,
    bootstrap_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct ScheduledCreate {
    task_id: String,
    run_log_id: i64,
}

struct LocalChangesSnapshot {
    patch: Vec<u8>,
    untracked: Vec<PathBuf>,
}

struct BootstrapDirGuard(Option<PathBuf>);

impl Drop for BootstrapDirGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

struct WorktreeCreationGuard {
    source_dir: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl Drop for WorktreeCreationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = git_status(
            &self.source_dir,
            &["worktree", "remove", "--force", path_arg(&self.path)],
        );
        let _ = git_status(&self.source_dir, &["worktree", "prune"]);
    }
}

impl SessionDB {
    pub async fn create_managed_worktree(
        self: &std::sync::Arc<Self>,
        input: CreateManagedWorktreeInput,
    ) -> Result<ManagedWorktree> {
        if !input.purpose.is_owner_transport_safe() {
            bail!("scheduled managed worktree purposes are internal to the scheduler");
        }
        self.create_managed_worktree_inner(input, None).await
    }

    pub async fn create_scheduled_run_worktree(
        self: &std::sync::Arc<Self>,
        input: CreateManagedWorktreeInput,
        task_id: &str,
        run_log_id: i64,
    ) -> Result<ManagedWorktree> {
        self.create_scheduled_worktree(
            input,
            task_id,
            run_log_id,
            ManagedWorktreePurpose::ScheduledRun,
        )
        .await
    }

    pub async fn create_scheduled_task_worktree(
        self: &std::sync::Arc<Self>,
        input: CreateManagedWorktreeInput,
        task_id: &str,
        run_log_id: i64,
    ) -> Result<ManagedWorktree> {
        self.create_scheduled_worktree(
            input,
            task_id,
            run_log_id,
            ManagedWorktreePurpose::ScheduledTask,
        )
        .await
    }

    async fn create_scheduled_worktree(
        self: &std::sync::Arc<Self>,
        mut input: CreateManagedWorktreeInput,
        task_id: &str,
        run_log_id: i64,
        purpose: ManagedWorktreePurpose,
    ) -> Result<ManagedWorktree> {
        ensure_owner_key(task_id, "scheduled task id")?;
        if run_log_id <= 0 {
            bail!("scheduled run log id must be positive");
        }
        input.purpose = purpose;
        input.bind_session_working_dir = true;
        input.include_local_changes = false;
        input.bootstrap_request_id = None;
        self.create_managed_worktree_inner(
            input,
            Some(ScheduledCreate {
                task_id: task_id.to_string(),
                run_log_id,
            }),
        )
        .await
    }

    async fn create_managed_worktree_inner(
        self: &std::sync::Arc<Self>,
        input: CreateManagedWorktreeInput,
        scheduled: Option<ScheduledCreate>,
    ) -> Result<ManagedWorktree> {
        // Phase 1 — validate the session and inspect the source repo. This runs
        // synchronous DB reads plus several `git rev-parse` subprocesses; on a
        // slow disk or large repo a subprocess wait can stall for seconds, so it
        // must never sit on the async worker (see `crate::blocking`).
        let prep = {
            let db = self.clone();
            let session_id = input.session_id.clone();
            let child_session_id = input.child_session_id.clone();
            let source_working_dir = input.source_working_dir.clone();
            let base_ref_in = input.base_ref.clone();
            let include_local_changes = input.include_local_changes;
            let bootstrap_request_id = input.bootstrap_request_id.clone();
            crate::blocking::run_blocking(move || -> Result<WorktreePrep> {
                let meta = db
                    .get_session(&session_id)?
                    .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
                if meta.incognito {
                    bail!("Cannot create managed worktree for incognito session {session_id}");
                }
                if let Some(child_session_id) = child_session_id.as_deref() {
                    if db.get_session(child_session_id)?.is_none() {
                        bail!("child session not found: {child_session_id}");
                    }
                }
                let source = source_working_dir
                    .as_deref()
                    .map(|p| p.to_string())
                    .or_else(|| effective_working_dir_for_meta(&meta))
                    .ok_or_else(|| anyhow!("session {session_id} has no working directory"))?;
                let source_dir = canonical_dir(&source)?;
                if !is_inside_git_work_tree(&source_dir) {
                    bail!("{} is not inside a git worktree", source_dir.display());
                }
                let repo_root = git_output(&source_dir, &["rev-parse", "--show-toplevel"])?;
                let repo_root = canonical_dir(repo_root.trim())?;
                let base_ref = base_ref_in
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("HEAD")
                    .to_string();
                let base_sha = resolve_commit(&source_dir, &base_ref)?;
                let base_branch = if base_ref == "HEAD" {
                    current_branch(&source_dir)
                } else {
                    branch_name_for_ref(&base_ref)
                };
                let (local_changes, bootstrap_dir) = if include_local_changes {
                    let request_id = bootstrap_request_id.as_deref().ok_or_else(|| {
                        anyhow!("bootstrapRequestId is required when including local changes")
                    })?;
                    ensure_bootstrap_not_cancelled(Some(request_id))?;
                    let current_ref =
                        git_output(&source_dir, &["symbolic-ref", "--quiet", "HEAD"])?;
                    let current_ref = current_ref.trim();
                    if current_ref != base_ref {
                        bail!("Local changes can only be included from the current local branch");
                    }
                    let current_head = git_output(&source_dir, &["rev-parse", "HEAD"])?;
                    if current_head.trim() != base_sha {
                        bail!("The source branch changed while preparing the worktree; retry");
                    }
                    let run_dir = crate::paths::bootstrap_run_dir(request_id)?;
                    let snapshot = snapshot_local_changes(
                        &repo_root, &run_dir, request_id, &base_ref, &base_sha,
                    )?;
                    ensure_bootstrap_not_cancelled(Some(request_id))?;
                    let head_after = git_output(&source_dir, &["rev-parse", "HEAD"])?;
                    if head_after.trim() != base_sha {
                        let _ = fs::remove_dir_all(&run_dir);
                        bail!("The source branch changed while preparing the worktree; retry");
                    }
                    (Some(snapshot), Some(run_dir))
                } else {
                    (None, None)
                };
                let id = format!("wt_{}", uuid::Uuid::new_v4().simple());
                let default_path = crate::paths::worktrees_dir()?
                    .join(repo_slug(&repo_root))
                    .join(&id);
                Ok(WorktreePrep {
                    source_dir,
                    repo_root,
                    base_ref,
                    base_sha,
                    base_branch,
                    id,
                    default_path,
                    local_changes,
                    bootstrap_dir,
                })
            })
            .await?
        };

        if scheduled.is_some() {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            conn.execute(
                "INSERT INTO scheduled_worktree_pending (id,created_at) VALUES (?1,?2)",
                params![prep.id, now_rfc3339()],
            )?;
        }

        // Arm cleanup before the async Hook boundary so task cancellation or a
        // Hook failure cannot strand snapshot material under bootstrap/.
        let _bootstrap_guard = BootstrapDirGuard(prep.bootstrap_dir.clone());
        let hook_outcome =
            crate::hooks::dispatch_worktree_create(&input.session_id, &prep.id, &prep.source_dir)
                .await;

        // Phase 2 — materialize the worktree on disk (`git worktree add`, file
        // copy) and persist the row (write lock + INSERT + goal-evidence refresh).
        // All synchronous; routed to the blocking pool for the same reason.
        let db = self.clone();
        let row = crate::blocking::run_blocking(move || -> Result<ManagedWorktree> {
            if let Some(request_id) = input.bootstrap_request_id.as_deref() {
                db.report_project_bootstrap_stage(
                    request_id,
                    "creating_worktree",
                    Some(&input.session_id),
                    Some(&prep.id),
                )?;
            }
            let path_source = if hook_outcome.is_some() {
                ManagedWorktreePathSource::Hook
            } else {
                ManagedWorktreePathSource::Builtin
            };
            let path = if let Some(outcome) = hook_outcome {
                match outcome.decision {
                    crate::hooks::HookDecision::Deny { reason }
                    | crate::hooks::HookDecision::Block { reason } => {
                        bail!("WorktreeCreate hook blocked creation: {reason}")
                    }
                    _ => {}
                }
                let hook_path = outcome.worktree_path.ok_or_else(|| {
                    anyhow!("WorktreeCreate hook must return hookSpecificOutput.worktreePath")
                })?;
                canonical_dir(&hook_path)?
            } else {
                ensure_parent(&prep.default_path)?;
                git_status(
                    &prep.source_dir,
                    &[
                        "worktree",
                        "add",
                        "--detach",
                        path_arg(&prep.default_path),
                        &prep.base_sha,
                    ],
                )?;
                canonical_dir(prep.default_path.to_string_lossy())?
            };
            let mut creation_guard = WorktreeCreationGuard {
                source_dir: prep.source_dir.clone(),
                path: path.clone(),
                // Drop only invokes `git worktree remove`; it never recursively
                // deletes the path, so Hook-owned locations are safe to arm.
                armed: true,
            };
            ensure_bootstrap_not_cancelled(input.bootstrap_request_id.as_deref())?;

            if let Some(request_id) = input.bootstrap_request_id.as_deref() {
                db.report_project_bootstrap_stage(
                    request_id,
                    "copying_changes",
                    Some(&input.session_id),
                    Some(&prep.id),
                )?;
            }
            if path == prep.default_path {
                copy_worktreeinclude(
                    &prep.repo_root,
                    &path,
                    input.bootstrap_request_id.as_deref(),
                )?;
            }
            if let Some(snapshot) = prep.local_changes.as_ref() {
                apply_local_changes(
                    &prep.repo_root,
                    &path,
                    snapshot,
                    input.bootstrap_request_id.as_deref(),
                )?;
            }
            ensure_bootstrap_not_cancelled(input.bootstrap_request_id.as_deref())?;

            if let Some(request_id) = input.bootstrap_request_id.as_deref() {
                db.report_project_bootstrap_stage(
                    request_id,
                    "binding_session",
                    Some(&input.session_id),
                    Some(&prep.id),
                )?;
            }

            let git_branch = current_branch(&path);
            let now = now_rfc3339();
            let task_owned = input.purpose == ManagedWorktreePurpose::ScheduledTask;
            let owner_session_id = (!task_owned).then(|| input.session_id.clone());
            let owner_scheduled_task_id = task_owned
                .then(|| scheduled.as_ref().map(|value| value.task_id.clone()))
                .flatten();
            let row = ManagedWorktree {
                id: prep.id,
                session_id: input.session_id.clone(),
                child_session_id: input.child_session_id,
                workflow_run_id: input.workflow_run_id,
                owner_session_id,
                owner_scheduled_task_id,
                scheduled_task_id: scheduled.as_ref().map(|value| value.task_id.clone()),
                runtime_session_id: scheduled.as_ref().map(|_| input.session_id.clone()),
                runtime_run_id: scheduled.as_ref().map(|value| value.run_log_id.to_string()),
                handoff_session_id: None,
                purpose: input.purpose,
                state: ManagedWorktreeState::Active,
                label: input.label.filter(|s| !s.trim().is_empty()),
                repo_root: prep.repo_root.to_string_lossy().to_string(),
                source_working_dir: prep.source_dir.to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                path_source,
                base_ref: Some(prep.base_ref),
                base_branch: prep.base_branch,
                base_sha: Some(prep.base_sha),
                git_branch,
                dirty_snapshot: None,
                path_exists: path.exists(),
                created_at: now.clone(),
                updated_at: now,
                archived_at: None,
                restored_at: None,
                handed_off_at: None,
            };
            {
                let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO managed_worktrees (
                        id, session_id, child_session_id, workflow_run_id, purpose, state, label,
                        repo_root, source_working_dir, path, base_ref, base_branch, base_sha,
                        git_branch, dirty_snapshot_json, created_at, updated_at,
                        archived_at, restored_at, handed_off_at, path_source,
                        owner_session_id, owner_scheduled_task_id, scheduled_task_id,
                        runtime_session_id, runtime_run_id, handoff_session_id
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                        NULL, ?15, ?15, NULL, NULL, NULL, ?16, ?17, ?18, ?19, ?20, ?21, NULL
                    )",
                    params![
                        &row.id,
                        &row.session_id,
                        row.child_session_id.as_deref(),
                        row.workflow_run_id.as_deref(),
                        row.purpose.as_str(),
                        row.state.as_str(),
                        row.label.as_deref(),
                        &row.repo_root,
                        &row.source_working_dir,
                        &row.path,
                        row.base_ref.as_deref(),
                        row.base_branch.as_deref(),
                        row.base_sha.as_deref(),
                        row.git_branch.as_deref(),
                        &row.created_at,
                        row.path_source.as_str(),
                        row.owner_session_id.as_deref(),
                        row.owner_scheduled_task_id.as_deref(),
                        row.scheduled_task_id.as_deref(),
                        row.runtime_session_id.as_deref(),
                        row.runtime_run_id.as_deref()
                    ],
                )?;
                if input.bind_session_working_dir
                    && tx.execute(
                        "UPDATE sessions SET working_dir=?1 WHERE id=?2 AND incognito=0",
                        params![row.path, row.session_id],
                    )? != 1
                {
                    bail!("scheduled run session is missing or incognito");
                }
                tx.commit()?;
            }
            if input.bind_session_working_dir {
                crate::hooks::fire_cwd_changed(&row.session_id, None, Some(&row.path));
            }
            creation_guard.armed = false;
            emit_worktree_changed("worktree:created", &row);
            if let Err(err) = db.refresh_goal_worktree_evidence(&row) {
                crate::app_warn!(
                    "goal",
                    "worktree_evidence",
                    "failed to refresh goal worktree evidence after create {}: {err:#}",
                    row.id
                );
            }
            Ok(row)
        })
        .await?;
        if row.scheduled_task_id.is_some() {
            if let Ok(conn) = self.conn.lock() {
                let _ = conn.execute(
                    "DELETE FROM scheduled_worktree_pending WHERE id=?1",
                    params![row.id],
                );
            }
        }
        Ok(row)
    }

    /// Roll back a prepared worktree through Git. Hook-owned paths are allowed
    /// here, but are never recursively deleted by Hope; cleanup stays
    /// Git-aware so an arbitrary hook path cannot become an `rm -rf` target.
    pub fn discard_managed_worktree(&self, id: &str) -> Result<()> {
        let worktree = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        generic_owner_session_id(&worktree, "discard")?;
        self.discard_managed_worktree_unchecked(worktree)
    }

    /// The caller must establish custody before entering this helper. Scheduled
    /// owners use their typed CAS claim, while generic callers are checked by
    /// `discard_managed_worktree` above.
    fn discard_managed_worktree_unchecked(&self, worktree: ManagedWorktree) -> Result<()> {
        let path = PathBuf::from(&worktree.path);
        if path.exists() {
            git_status(
                Path::new(&worktree.source_working_dir),
                &["worktree", "remove", "--force", path_arg(&path)],
            )?;
            let _ = git_status(
                Path::new(&worktree.source_working_dir),
                &["worktree", "prune"],
            );
        }
        {
            let conn = self
                .conn
                .lock()
                .map_err(|error| anyhow!("Lock error: {error}"))?;
            conn.execute(
                "DELETE FROM managed_worktrees WHERE id = ?1",
                params![worktree.id],
            )?;
        }
        if let Some(session_id) = worktree.owner_session_id.as_deref() {
            self.clear_worktree_cwd_if_matches(session_id, &worktree.path)?;
        }
        if let Some(session_id) = worktree.handoff_session_id.as_deref() {
            self.clear_worktree_cwd_if_matches(session_id, &worktree.path)?;
        }
        Ok(())
    }

    fn clear_worktree_cwd_if_matches(&self, session_id: &str, path: &str) -> Result<()> {
        let changed = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            conn.execute(
                "UPDATE sessions SET working_dir=NULL WHERE id=?1 AND working_dir=?2",
                params![session_id, path],
            )?
        };
        if changed > 0 {
            crate::hooks::fire_cwd_changed(session_id, Some(path), None);
        }
        Ok(())
    }

    pub(crate) fn mark_managed_worktree_bootstrap_failed(&self, id: &str) -> Result<()> {
        let now = now_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow!("Lock error: {error}"))?;
        conn.execute(
            "UPDATE managed_worktrees
             SET state = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, ManagedWorktreeState::BootstrapFailed.as_str(), now],
        )?;
        Ok(())
    }

    pub fn list_managed_worktrees_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<ManagedWorktree>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "{MANAGED_WORKTREE_SELECT}
             WHERE owner_session_id = ?1 OR child_session_id = ?1
                OR runtime_session_id = ?1 OR handoff_session_id = ?1
             ORDER BY updated_at DESC, created_at DESC"
        ))?;
        let rows = stmt.query_map(params![session_id], row_to_worktree)?;
        collect_rows(rows)
    }

    pub fn get_managed_worktree(&self, id: &str) -> Result<Option<ManagedWorktree>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            &format!("{MANAGED_WORKTREE_SELECT} WHERE id = ?1"),
            params![id],
            row_to_worktree,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_scheduled_worktrees(
        &self,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ManagedWorktree>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let mut stmt = conn.prepare(&format!(
            "{MANAGED_WORKTREE_SELECT}
              WHERE purpose IN ('scheduled_run','scheduled_task')
                AND (?1 IS NULL OR scheduled_task_id=?1)
              ORDER BY updated_at DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![task_id, limit.min(256) as i64], row_to_worktree)?;
        collect_rows(rows)
    }

    pub fn scheduled_session_durably_idle(
        &self,
        session_id: &str,
        ignored_turn_id: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        scheduled_session_idle_on(&conn, session_id, ignored_turn_id)
    }

    pub(crate) fn ensure_session_has_no_scheduled_worktree_custody(
        &self,
        session_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let held: Option<String> = conn
            .query_row(
                "SELECT id FROM managed_worktrees
                  WHERE purpose IN ('scheduled_run','scheduled_task')
                    AND (owner_session_id=?1 OR runtime_session_id=?1
                         OR handoff_session_id=?1) LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = held {
            bail!("workspace_session_custody:{id}");
        }
        Ok(())
    }

    pub fn list_scheduled_runtime_worktrees(&self, limit: usize) -> Result<Vec<ManagedWorktree>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let mut stmt = conn.prepare(&format!(
            "{MANAGED_WORKTREE_SELECT} WHERE runtime_run_id IS NOT NULL
                AND purpose IN ('scheduled_run','scheduled_task')
              ORDER BY updated_at LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit.min(256) as i64], row_to_worktree)?;
        collect_rows(rows)
    }

    pub fn recover_pending_scheduled_worktrees(&self, limit: usize) -> Result<usize> {
        let ids: Vec<String> = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT id FROM scheduled_worktree_pending
                  WHERE julianday(created_at)<=julianday('now','-1 minute')
                  ORDER BY created_at LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit.min(64) as i64], |row| row.get(0))?;
            collect_rows(rows)?
        };
        let mut recovered = 0;
        for id in ids {
            let result = (|| -> Result<usize> {
                if self.get_managed_worktree(&id)?.is_none() {
                    cleanup_orphan_builtin_worktree(&id)?;
                }
                let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
                Ok(conn.execute(
                    "DELETE FROM scheduled_worktree_pending WHERE id=?1",
                    params![id],
                )?)
            })();
            match result {
                Ok(count) => recovered += count,
                Err(error) => app_warn!(
                    "worktree",
                    "scheduled_recovery",
                    "Pending worktree {id} recovery failed: {error:#}"
                ),
            }
        }
        Ok(recovered)
    }

    pub fn snapshot_managed_worktree(&self, id: &str) -> Result<ManagedWorktreeRuntimeSnapshot> {
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("workspace_missing"))?;
        let path = canonical_dir(&row.path).map_err(|_| anyhow!("workspace_path_missing"))?;
        let listed = git_output(
            Path::new(&row.repo_root),
            &["worktree", "list", "--porcelain"],
        )?;
        if !listed
            .lines()
            .any(|line| line.strip_prefix("worktree ") == Some(path_arg(&path)))
        {
            bail!("workspace_repo_mismatch");
        }
        let dirty = worktree_dirty_snapshot(&path).ok_or_else(|| anyhow!("workspace_not_git"))?;
        let head_sha = Some(
            git_output(&path, &["rev-parse", "HEAD"])?
                .trim()
                .to_string(),
        );
        Ok(ManagedWorktreeRuntimeSnapshot {
            branch: current_branch(&path),
            head_diverged: row.base_sha.as_deref() != head_sha.as_deref(),
            head_sha,
            dirty,
        })
    }

    pub fn get_scheduled_task_worktree(
        &self,
        scheduled_task_id: &str,
    ) -> Result<Option<ManagedWorktree>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            &format!("{MANAGED_WORKTREE_SELECT} WHERE owner_scheduled_task_id=?1 AND purpose='scheduled_task'"),
            params![scheduled_task_id],
            row_to_worktree,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Acquire the task-owned checkout for one exact scheduled occurrence and
    /// atomically bind the run session cwd. The scheduler remains responsible
    /// for its task-level running lease; this CAS is only filesystem custody.
    pub fn acquire_scheduled_worktree_runtime(
        &self,
        id: &str,
        scheduled_task_id: &str,
        run_log_id: i64,
        session_id: &str,
    ) -> Result<ManagedWorktree> {
        if run_log_id <= 0 {
            bail!("scheduled run log id must be positive");
        }
        let run_id = run_log_id.to_string();
        let worktree = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        if self.snapshot_managed_worktree(id)?.dirty.conflicted_files > 0 {
            bail!("workspace_conflicted");
        }
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let old_cwd: Option<String> = tx
            .query_row(
                "SELECT working_dir FROM sessions WHERE id=?1 AND incognito=0",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let changed = tx.execute(
            "UPDATE managed_worktrees
                SET runtime_session_id=?1, runtime_run_id=?2, updated_at=?3
              WHERE id=?4 AND purpose='scheduled_task' AND state='active'
                AND owner_scheduled_task_id=?5 AND handoff_session_id IS NULL
                AND runtime_session_id IS NULL AND runtime_run_id IS NULL",
            params![session_id, run_id, now, id, scheduled_task_id],
        )?;
        if changed != 1 {
            bail!("scheduled worktree is handed off, busy, or no longer owned by this task");
        }
        let session_changed = tx.execute(
            "UPDATE sessions SET working_dir=?1 WHERE id=?2 AND incognito=0",
            params![worktree.path, session_id],
        )?;
        if session_changed != 1 {
            bail!("scheduled run session is missing or incognito");
        }
        tx.commit()?;
        drop(conn);
        if old_cwd.as_deref() != Some(worktree.path.as_str()) {
            crate::hooks::fire_cwd_changed(session_id, old_cwd.as_deref(), Some(&worktree.path));
        }
        self.scheduled_owner_update_result(id, 1, "acquire scheduled runtime")
    }

    pub fn release_scheduled_runtime(
        &self,
        row: &ManagedWorktree,
        run_log_id: i64,
        session_id: &str,
        ignored_turn_id: Option<&str>,
    ) -> Result<ManagedWorktree> {
        if run_log_id <= 0
            || !matches!(
                row.purpose,
                ManagedWorktreePurpose::ScheduledRun | ManagedWorktreePurpose::ScheduledTask
            )
        {
            bail!("workspace_wrong_mode");
        }
        let task = row
            .scheduled_task_id
            .as_deref()
            .ok_or_else(|| anyhow!("workspace_not_owner"))?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_scheduled_session_idle_on(&tx, session_id, ignored_turn_id)?;
        let changed = tx.execute(
            "UPDATE managed_worktrees SET runtime_session_id=NULL, runtime_run_id=NULL, updated_at=?1
              WHERE id=?2 AND scheduled_task_id=?3 AND runtime_run_id=?5 AND (
                (purpose='scheduled_run' AND owner_session_id=?4 AND runtime_session_id=?4) OR
                (purpose='scheduled_task' AND owner_scheduled_task_id=?3 AND
                  (runtime_session_id=?4 OR (runtime_session_id IS NULL AND
                    NOT EXISTS (SELECT 1 FROM sessions WHERE id=?4)))))",
            params![now_rfc3339(), row.id, task, session_id, run_log_id.to_string()],
        )?;
        if changed != 1 {
            bail!("workspace_busy");
        }
        let cwd_cleared = if row.purpose == ManagedWorktreePurpose::ScheduledTask {
            tx.execute(
                "UPDATE sessions SET working_dir=NULL WHERE id=?1 AND working_dir=?2",
                params![session_id, row.path],
            )?
        } else {
            0
        };
        tx.commit()?;
        drop(conn);
        if cwd_cleared > 0 {
            crate::hooks::fire_cwd_changed(session_id, Some(&row.path), None);
        }
        self.scheduled_owner_update_result(&row.id, changed, "release scheduled runtime")
    }

    /// Transfer task custody to an ordinary chat. Callers pause the schedule
    /// first; a live occurrence may only hand off to its own runtime session.
    pub fn take_over_scheduled_worktree(
        &self,
        id: &str,
        scheduled_task_id: &str,
        session_id: &str,
    ) -> Result<ManagedWorktree> {
        let worktree = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_scheduled_session_idle_on(&tx, session_id, None)?;
        let old_cwd: Option<String> = tx
            .query_row(
                "SELECT working_dir FROM sessions WHERE id=?1 AND incognito=0",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let changed = tx.execute(
            "UPDATE managed_worktrees
                SET handoff_session_id=?1, handed_off_at=?2, updated_at=?2
              WHERE id=?3 AND purpose='scheduled_task' AND state='active'
                AND owner_scheduled_task_id=?4
                AND (handoff_session_id IS NULL OR handoff_session_id=?1)
                AND runtime_session_id IS NULL AND runtime_run_id IS NULL",
            params![session_id, now, id, scheduled_task_id],
        )?;
        if changed != 1 {
            bail!("workspace_busy");
        }
        if tx.execute(
            "UPDATE sessions SET working_dir=?1 WHERE id=?2 AND incognito=0",
            params![worktree.path, session_id],
        )? != 1
        {
            bail!("handoff session is missing or incognito");
        }
        tx.commit()?;
        drop(conn);
        if old_cwd.as_deref() != Some(worktree.path.as_str()) {
            crate::hooks::fire_cwd_changed(session_id, old_cwd.as_deref(), Some(&worktree.path));
        }
        self.scheduled_owner_update_result(id, 1, "take over scheduled worktree")
    }

    pub fn return_scheduled_worktree(
        &self,
        id: &str,
        scheduled_task_id: &str,
        session_id: &str,
    ) -> Result<ManagedWorktree> {
        let worktree = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_scheduled_session_idle_on(&tx, session_id, None)?;
        let changed = tx.execute(
            "UPDATE managed_worktrees SET handoff_session_id=NULL, updated_at=?1
              WHERE id=?2 AND purpose='scheduled_task' AND owner_scheduled_task_id=?3
                AND handoff_session_id=?4 AND runtime_session_id IS NULL
                AND runtime_run_id IS NULL",
            params![now, id, scheduled_task_id, session_id],
        )?;
        if changed != 1 {
            bail!("scheduled worktree is running or not handed off to this session");
        }
        let cwd_cleared = tx.execute(
            "UPDATE sessions SET working_dir=NULL WHERE id=?1 AND working_dir=?2",
            params![session_id, worktree.path],
        )?;
        tx.commit()?;
        drop(conn);
        if cwd_cleared > 0 {
            crate::hooks::fire_cwd_changed(session_id, Some(&worktree.path), None);
        }
        self.scheduled_owner_update_result(id, 1, "return scheduled worktree")
    }

    pub fn discard_scheduled_run_worktree(
        &self,
        id: &str,
        scheduled_task_id: &str,
        owner_session_id: &str,
    ) -> Result<()> {
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("workspace_missing"))?;
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_scheduled_session_idle_on(&tx, owner_session_id, None)?;
        let changed = tx.execute(
            "UPDATE managed_worktrees SET state='archived', updated_at=?1
              WHERE id=?2 AND purpose='scheduled_run' AND scheduled_task_id=?3
                AND owner_session_id=?4 AND owner_scheduled_task_id IS NULL
                AND runtime_session_id IS NULL AND runtime_run_id IS NULL
                AND handoff_session_id IS NULL AND state='active'",
            params![now, id, scheduled_task_id, owner_session_id],
        )?;
        if changed != 1 {
            bail!("workspace_not_owner");
        }
        let cwd_cleared = tx.execute(
            "UPDATE sessions SET working_dir=NULL WHERE id=?1 AND working_dir=?2",
            params![owner_session_id, row.path],
        )?;
        tx.commit()?;
        drop(conn);
        if cwd_cleared > 0 {
            crate::hooks::fire_cwd_changed(owner_session_id, Some(&row.path), None);
        }
        self.finish_scheduled_discard(id)
    }

    pub fn discard_scheduled_task_worktree(&self, id: &str, scheduled_task_id: &str) -> Result<()> {
        let changed = self.claim_scheduled_task_discard(id, scheduled_task_id)?;
        if changed != 1 {
            bail!("scheduled task worktree is running, handed off, or no longer owned by the task");
        }
        self.finish_scheduled_discard(id)
    }

    fn claim_scheduled_task_discard(&self, id: &str, scheduled_task_id: &str) -> Result<usize> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        Ok(conn.execute(
            "UPDATE managed_worktrees SET state='archived', updated_at=?1
              WHERE id=?2 AND purpose='scheduled_task' AND scheduled_task_id=?3
                AND owner_scheduled_task_id=?3 AND owner_session_id IS NULL
                AND runtime_session_id IS NULL AND runtime_run_id IS NULL
                AND handoff_session_id IS NULL AND state IN ('active','archived')",
            params![now, id, scheduled_task_id],
        )?)
    }

    fn finish_scheduled_discard(&self, id: &str) -> Result<()> {
        let worktree = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        self.discard_managed_worktree_unchecked(worktree)
    }

    fn scheduled_owner_update_result(
        &self,
        id: &str,
        changed: usize,
        action: &str,
    ) -> Result<ManagedWorktree> {
        if changed != 1 {
            bail!("could not {action}: ownership changed or preconditions were not met");
        }
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree disappeared after {action}: {id}"))?;
        emit_worktree_changed("worktree:updated", &row);
        Ok(row)
    }

    pub(crate) fn link_managed_worktree_to_workflow_run(
        &self,
        id: &str,
        workflow_run_id: &str,
    ) -> Result<Option<ManagedWorktree>> {
        let Some(current) = self.get_managed_worktree(id)? else {
            return Ok(None);
        };
        if current.workflow_run_id.as_deref() == Some(workflow_run_id)
            || current.workflow_run_id.is_some()
        {
            return Ok(Some(current));
        }

        let now = now_rfc3339();
        let changed = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE managed_worktrees
                    SET workflow_run_id = ?1, updated_at = ?2
                 WHERE id = ?3 AND workflow_run_id IS NULL",
                params![workflow_run_id, now, id],
            )?
        };
        let updated = self.get_managed_worktree(id)?;
        if changed > 0 {
            if let Some(row) = updated.as_ref() {
                emit_worktree_changed("worktree:updated", row);
                if let Err(err) = self.refresh_goal_worktree_evidence(row) {
                    crate::app_warn!(
                        "goal",
                        "worktree_evidence",
                        "failed to refresh goal worktree evidence after workflow link {}: {err:#}",
                        row.id
                    );
                }
            }
        }
        Ok(updated)
    }

    pub fn archive_managed_worktree(&self, id: &str) -> Result<ManagedWorktree> {
        let current = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        let owner_session_id = archive_restore_owner_session_id(&current, "archive")?;
        let path = PathBuf::from(&current.path);
        let snapshot = worktree_dirty_snapshot(&path);
        let scheduled_safe_remove = if current.purpose == ManagedWorktreePurpose::ScheduledRun {
            self.snapshot_managed_worktree(id).ok().is_some_and(|live| {
                live.branch.is_none() && !live.head_diverged && live.dirty.clean
            })
        } else {
            true
        };
        let should_remove = current.state != ManagedWorktreeState::Handoff
            && snapshot.as_ref().map(|s| s.clean).unwrap_or(false)
            && path.exists()
            && scheduled_safe_remove;
        if should_remove && current.purpose != ManagedWorktreePurpose::ScheduledRun {
            match git_status(
                Path::new(&current.repo_root),
                &["worktree", "remove", path_arg(&path)],
            ) {
                Ok(()) => crate::hooks::fire_worktree_remove(owner_session_id, &current.path),
                Err(e) => {
                    app_warn!(
                        "worktree",
                        "archive",
                        "failed to remove clean worktree {}: {}",
                        current.path,
                        e
                    );
                }
            }
        }
        let now = now_rfc3339();
        let snapshot_json = snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serialize dirty snapshot")?;
        let mut removed = false;
        {
            let changed = if current.purpose == ManagedWorktreePurpose::ScheduledRun {
                let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                require_scheduled_session_idle_on(&tx, owner_session_id, None)?;
                let changed = tx.execute(
                    "UPDATE managed_worktrees
                        SET state=?1, dirty_snapshot_json=?2,
                            archived_at=COALESCE(archived_at,?3), updated_at=?3
                      WHERE id=?4 AND purpose='scheduled_run' AND owner_session_id=?5
                        AND runtime_session_id IS NULL AND runtime_run_id IS NULL
                        AND state='active'",
                    params![
                        ManagedWorktreeState::Archived.as_str(),
                        snapshot_json,
                        now,
                        id,
                        owner_session_id
                    ],
                )?;
                if changed == 1
                    && should_remove
                    && tx.query_row(
                        "SELECT NOT EXISTS(SELECT 1 FROM sessions WHERE working_dir=?1)",
                        params![current.path],
                        |row| row.get(0),
                    )?
                {
                    match git_status(
                        Path::new(&current.repo_root),
                        &["worktree", "remove", path_arg(&path)],
                    ) {
                        Ok(()) => removed = true,
                        Err(error) => app_warn!(
                            "worktree",
                            "archive",
                            "failed to remove clean worktree {}: {}",
                            current.path,
                            error
                        ),
                    }
                }
                tx.commit()?;
                changed
            } else {
                let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
                conn.execute(
                    "UPDATE managed_worktrees
                    SET state = ?1,
                        dirty_snapshot_json = ?2,
                        archived_at = COALESCE(archived_at, ?3),
                        updated_at = ?3
                 WHERE id = ?4",
                    params![
                        ManagedWorktreeState::Archived.as_str(),
                        snapshot_json,
                        now,
                        id
                    ],
                )?
            };
            if changed != 1 {
                bail!("workspace_busy");
            }
        }
        if removed {
            crate::hooks::fire_worktree_remove(owner_session_id, &current.path);
        }
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found after archive: {id}"))?;
        emit_worktree_changed("worktree:archived", &row);
        if let Err(err) = self.refresh_goal_worktree_evidence(&row) {
            crate::app_warn!(
                "goal",
                "worktree_evidence",
                "failed to refresh goal worktree evidence after archive {}: {err:#}",
                row.id
            );
        }
        Ok(row)
    }

    pub fn restore_managed_worktree(&self, id: &str) -> Result<ManagedWorktree> {
        let current = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        let owner_session_id = archive_restore_owner_session_id(&current, "restore")?;
        let path = PathBuf::from(&current.path);
        let restore_path = || -> Result<()> {
            if path.exists() && !path.is_dir() {
                bail!(
                    "managed worktree path exists but is not a directory: {}",
                    path.display()
                );
            }
            if !path.exists() {
                ensure_parent(&path)?;
                let base = current.base_sha.as_deref().unwrap_or("HEAD");
                git_status(
                    Path::new(&current.repo_root),
                    &["worktree", "add", "--detach", path_arg(&path), base],
                )?;
                if let Err(error) = copy_worktreeinclude(Path::new(&current.repo_root), &path, None)
                {
                    let _ = git_status(
                        Path::new(&current.repo_root),
                        &["worktree", "remove", "--force", path_arg(&path)],
                    );
                    let _ = git_status(Path::new(&current.repo_root), &["worktree", "prune"]);
                    return Err(error);
                }
            }
            Ok(())
        };
        let now = now_rfc3339();
        if current.purpose == ManagedWorktreePurpose::ScheduledRun {
            let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scheduled_session_idle_on(&tx, owner_session_id, None)?;
            if tx.execute(
                "UPDATE managed_worktrees SET state=?1, restored_at=?2, updated_at=?2
                  WHERE id=?3 AND purpose='scheduled_run' AND owner_session_id=?4
                    AND runtime_session_id IS NULL AND runtime_run_id IS NULL
                    AND state='archived'",
                params![
                    ManagedWorktreeState::Active.as_str(),
                    now,
                    id,
                    owner_session_id
                ],
            )? != 1
            {
                bail!("workspace_busy");
            }
            restore_path()?;
            if tx.execute(
                "UPDATE sessions SET working_dir=?1 WHERE id=?2 AND incognito=0",
                params![current.path, owner_session_id],
            )? != 1
            {
                bail!("workspace_busy");
            }
            tx.commit()?;
            crate::hooks::fire_cwd_changed(owner_session_id, None, Some(&current.path));
        } else {
            restore_path()?;
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            conn.execute(
                "UPDATE managed_worktrees
                    SET state = ?1,
                        restored_at = ?2,
                        updated_at = ?2
                 WHERE id = ?3",
                params![ManagedWorktreeState::Active.as_str(), now, id],
            )?;
        }
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found after restore: {id}"))?;
        emit_worktree_changed("worktree:restored", &row);
        if let Err(err) = self.refresh_goal_worktree_evidence(&row) {
            crate::app_warn!(
                "goal",
                "worktree_evidence",
                "failed to refresh goal worktree evidence after restore {}: {err:#}",
                row.id
            );
        }
        Ok(row)
    }

    pub fn handoff_managed_worktree(&self, id: &str) -> Result<ManagedWorktree> {
        let current = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found: {id}"))?;
        let owner_session_id = generic_owner_session_id(&current, "handoff")?;
        let path = canonical_dir(&current.path)?;
        self.update_session_working_dir(
            owner_session_id,
            Some(path.to_string_lossy().to_string()),
        )?;
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE managed_worktrees
                    SET state = ?1,
                        handed_off_at = ?2,
                        updated_at = ?2
                 WHERE id = ?3",
                params![ManagedWorktreeState::Handoff.as_str(), now, id],
            )?;
        }
        let row = self
            .get_managed_worktree(id)?
            .ok_or_else(|| anyhow!("managed worktree not found after handoff: {id}"))?;
        emit_worktree_changed("worktree:handoff", &row);
        if let Err(err) = self.refresh_goal_worktree_evidence(&row) {
            crate::app_warn!(
                "goal",
                "worktree_evidence",
                "failed to refresh goal worktree evidence after handoff {}: {err:#}",
                row.id
            );
        }
        Ok(row)
    }
}

fn generic_owner_session_id<'a>(worktree: &'a ManagedWorktree, action: &str) -> Result<&'a str> {
    if !worktree.purpose.is_owner_transport_safe() {
        bail!(
            "cannot {action} scheduled managed worktree {} through the generic owner API",
            worktree.id
        );
    }
    worktree.owner_session_id.as_deref().ok_or_else(|| {
        anyhow!(
            "cannot {action} managed worktree {} without a session owner",
            worktree.id
        )
    })
}

fn archive_restore_owner_session_id<'a>(
    worktree: &'a ManagedWorktree,
    action: &str,
) -> Result<&'a str> {
    if worktree.purpose == ManagedWorktreePurpose::ScheduledRun {
        return worktree
            .owner_session_id
            .as_deref()
            .ok_or_else(|| anyhow!("workspace_not_owner"));
    }
    generic_owner_session_id(worktree, action)
}

/// Recover a built-in worktree that was registered by Git but whose DB row was
/// not committed before process interruption. The search is constrained to
/// Hope's managed two-level root and removal remains Git-aware.
pub(crate) fn cleanup_orphan_builtin_worktree(id: &str) -> Result<bool> {
    if !id.starts_with("wt_")
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        bail!("invalid managed worktree id");
    }
    let root = crate::paths::worktrees_dir()?;
    if !root.is_dir() {
        return Ok(true);
    }
    for repo_entry in fs::read_dir(&root)? {
        let repo_entry = repo_entry?;
        if !repo_entry.file_type()?.is_dir() {
            continue;
        }
        let candidate = repo_entry.path().join(id);
        if !candidate.exists() {
            continue;
        }
        git_status(
            &candidate,
            &["worktree", "remove", "--force", path_arg(&candidate)],
        )?;
        return Ok(true);
    }
    Ok(true)
}

fn row_to_worktree(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedWorktree> {
    let purpose: String = row.get(4)?;
    let state: String = row.get(5)?;
    let snapshot_json: Option<String> = row.get(14)?;
    let path: String = row.get(9)?;
    Ok(ManagedWorktree {
        id: row.get(0)?,
        session_id: row.get(1)?,
        child_session_id: row.get(2)?,
        workflow_run_id: row.get(3)?,
        owner_session_id: row.get(21)?,
        owner_scheduled_task_id: row.get(22)?,
        scheduled_task_id: row.get(23)?,
        runtime_session_id: row.get(24)?,
        runtime_run_id: row.get(25)?,
        handoff_session_id: row.get(26)?,
        purpose: ManagedWorktreePurpose::from_str(&purpose),
        state: ManagedWorktreeState::from_str(&state),
        label: row.get(6)?,
        repo_root: row.get(7)?,
        source_working_dir: row.get(8)?,
        path_exists: Path::new(&path).exists(),
        path,
        base_ref: row.get(10)?,
        base_branch: row.get(11)?,
        base_sha: row.get(12)?,
        git_branch: row.get(13)?,
        dirty_snapshot: snapshot_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    14,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        archived_at: row.get(17)?,
        restored_at: row.get(18)?,
        handed_off_at: row.get(19)?,
        path_source: ManagedWorktreePathSource::from_str(row.get::<_, String>(20)?.as_str()),
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn canonical_dir(path: impl AsRef<str>) -> Result<PathBuf> {
    let raw = path.as_ref();
    let canon = Path::new(raw)
        .canonicalize()
        .map_err(|e| anyhow!("Cannot resolve directory '{}': {}", raw, e))?;
    if !canon.is_dir() {
        bail!("'{}' is not a directory", canon.display());
    }
    Ok(canon)
}

fn ensure_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| anyhow!("Cannot create directory '{}': {}", parent.display(), e))
}

fn path_arg(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_output_bytes(cwd, args)?).into_owned())
}

fn git_output_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    git_output_bytes_mode(cwd, args, false)
}

fn git_output_read_only(cwd: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_output_bytes_mode(cwd, args, true)?).into_owned())
}

fn git_output_bytes_mode(cwd: &Path, args: &[&str], no_optional_locks: bool) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    crate::filesystem::isolate_repository_env(&mut cmd);
    if no_optional_locks {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }
    cmd.current_dir(cwd).args(args);
    crate::platform::hide_console(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| anyhow!("failed to run git {:?}: {}", args, e))?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

fn resolve_commit(cwd: &Path, base_ref: &str) -> Result<String> {
    let commit_ref = format!("{base_ref}^{{commit}}");
    Ok(git_output(cwd, &["rev-parse", "--verify", &commit_ref])?
        .trim()
        .to_string())
}

fn git_status(cwd: &Path, args: &[&str]) -> Result<()> {
    git_output(cwd, args).map(|_| ())
}

fn is_inside_git_work_tree(cwd: &Path) -> bool {
    git_output(cwd, &["rev-parse", "--is-inside-work-tree"])
        .map(|out| out.trim() == "true")
        .unwrap_or(false)
}

fn current_branch(cwd: &Path) -> Option<String> {
    git_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD")
}

fn branch_name_for_ref(base_ref: &str) -> Option<String> {
    base_ref
        .strip_prefix("refs/heads/")
        .or_else(|| base_ref.strip_prefix("refs/remotes/"))
        .map(str::to_string)
}

fn snapshot_local_changes(
    repo_root: &Path,
    run_dir: &Path,
    request_id: &str,
    base_ref: &str,
    base_sha: &str,
) -> Result<LocalChangesSnapshot> {
    if run_dir.exists() {
        fs::remove_dir_all(run_dir).with_context(|| {
            format!("failed to reset bootstrap directory {}", run_dir.display())
        })?;
    }
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create bootstrap directory {}", run_dir.display()))?;
    let result = (|| -> Result<LocalChangesSnapshot> {
        let patch = git_output_bytes(repo_root, &["diff", "--binary", "HEAD", "--"])?;
        let untracked_raw = git_output_bytes(
            repo_root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?;
        let untracked = untracked_raw
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                String::from_utf8(entry.to_vec())
                    .map(PathBuf::from)
                    .map_err(|_| anyhow!("untracked path is not valid UTF-8"))
            })
            .collect::<Result<Vec<_>>>()?;

        fs::write(run_dir.join("tracked.patch"), &patch)?;
        let mut manifest = Vec::new();
        for rel in &untracked {
            manifest.extend_from_slice(rel.to_string_lossy().as_bytes());
            manifest.push(0);
        }
        fs::write(run_dir.join("untracked.manifest"), manifest)?;
        fs::write(
            run_dir.join("metadata.json"),
            serde_json::to_vec_pretty(&json!({
                "requestId": request_id,
                "baseRef": base_ref,
                "baseSha": base_sha,
            }))?,
        )?;
        Ok(LocalChangesSnapshot { patch, untracked })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(run_dir);
    }
    result
}

fn apply_local_changes(
    repo_root: &Path,
    worktree_path: &Path,
    snapshot: &LocalChangesSnapshot,
    bootstrap_request_id: Option<&str>,
) -> Result<()> {
    ensure_bootstrap_not_cancelled(bootstrap_request_id)?;
    if !snapshot.patch.is_empty() {
        let mut cmd = Command::new("git");
        crate::filesystem::isolate_repository_env(&mut cmd);
        cmd.current_dir(worktree_path)
            .args(["apply", "--binary", "--whitespace=nowarn", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::platform::hide_console(&mut cmd);
        let mut child = cmd.spawn().context("failed to start git apply")?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("git apply stdin unavailable"))?
            .write_all(&snapshot.patch)?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!(
                "failed to apply local changes: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    for rel in &snapshot.untracked {
        ensure_bootstrap_not_cancelled(bootstrap_request_id)?;
        copy_one_rel(repo_root, worktree_path, rel)?;
    }
    Ok(())
}

fn repo_slug(repo_root: &Path) -> String {
    let name = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo");
    let safe: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let hash = blake3::hash(repo_root.to_string_lossy().as_bytes()).to_hex();
    format!(
        "{}-{}",
        safe.trim_matches('-').chars().take(48).collect::<String>(),
        &hash[..8]
    )
}

fn copy_worktreeinclude(
    repo_root: &Path,
    worktree_path: &Path,
    bootstrap_request_id: Option<&str>,
) -> Result<()> {
    let include_path = repo_root.join(".worktreeinclude");
    if include_path.is_file() {
        let content = fs::read_to_string(&include_path).with_context(|| {
            format!(
                "failed to read worktree include file {}",
                include_path.display()
            )
        })?;
        for line in content.lines() {
            ensure_bootstrap_not_cancelled(bootstrap_request_id)?;
            let pattern = line.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                continue;
            }
            for rel in ignored_paths_for_pattern(repo_root, pattern)? {
                copy_one_rel(repo_root, worktree_path, &rel)?;
            }
        }
    }
    let override_file = repo_root.join("AGENTS.override.md");
    if override_file.is_file() {
        copy_one_rel(repo_root, worktree_path, Path::new("AGENTS.override.md"))?;
    }
    Ok(())
}

fn ensure_bootstrap_not_cancelled(request_id: Option<&str>) -> Result<()> {
    if request_id.is_some_and(crate::project_bootstrap::is_project_bootstrap_cancelled) {
        bail!("worktree preparation was cancelled");
    }
    Ok(())
}

fn ignored_paths_for_pattern(repo_root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let mut cmd = Command::new("git");
    crate::filesystem::isolate_repository_env(&mut cmd);
    cmd.current_dir(repo_root)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--",
        ])
        .arg(pattern);
    crate::platform::hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| {
        anyhow!(
            "failed to inspect .worktreeinclude pattern '{}': {}",
            pattern,
            e
        )
    })?;
    if !out.status.success() {
        bail!(
            "git ls-files for .worktreeinclude pattern '{}' failed: {}",
            pattern,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn copy_one_rel(repo_root: &Path, worktree_path: &Path, rel: &Path) -> Result<()> {
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!(
            "refusing to copy path outside repository: {}",
            rel.display()
        );
    }
    if contains_symlink_component(repo_root, rel)?
        || contains_symlink_component(worktree_path, rel)?
    {
        crate::app_warn!(
            "project_bootstrap",
            "copy_local_changes",
            "skipped symlink path while copying into worktree: {}",
            rel.display()
        );
        return Ok(());
    }
    let src = repo_root.join(rel);
    let dst = worktree_path.join(rel);
    let canonical_repo = repo_root.canonicalize()?;
    let canonical_source = src
        .canonicalize()
        .with_context(|| format!("source path disappeared while copying: {}", src.display()))?;
    if !canonical_source.starts_with(&canonical_repo) {
        bail!("source path escapes repository: {}", rel.display());
    }
    let meta = fs::symlink_metadata(&canonical_source)?;
    if meta.file_type().is_symlink() {
        crate::app_warn!(
            "project_bootstrap",
            "copy_local_changes",
            "skipped symlink while copying into worktree: {}",
            rel.display()
        );
        return Ok(());
    }
    if meta.is_dir() {
        copy_dir_recursive(&canonical_source, &dst)?;
    } else if meta.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&src, &dst)?;
    }
    Ok(())
}

fn contains_symlink_component(root: &Path, rel: &Path) -> Result<bool> {
    let mut current = root.to_path_buf();
    for component in rel.components() {
        use std::path::Component;
        let Component::Normal(part) = component else {
            bail!("invalid relative path: {}", rel.display());
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());
        if meta.file_type().is_symlink() {
            crate::app_warn!(
                "project_bootstrap",
                "copy_local_changes",
                "skipped nested symlink while copying into worktree: {}",
                child_src.display()
            );
            continue;
        }
        if meta.is_dir() {
            copy_dir_recursive(&child_src, &child_dst)?;
        } else if meta.is_file() {
            if let Some(parent) = child_dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&child_src, &child_dst)?;
        }
    }
    Ok(())
}

fn worktree_dirty_snapshot(path: &Path) -> Option<ManagedWorktreeDirtySnapshot> {
    if !path.exists() || !is_inside_git_work_tree(path) {
        return None;
    }
    let out = git_output_read_only(path, &["status", "--porcelain=v1"]).ok()?;
    let mut staged = 0u32;
    let mut unstaged = 0u32;
    let mut untracked = 0u32;
    let mut conflicted = 0u32;
    let mut changed = 0u32;
    for line in out.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 2 {
            continue;
        }
        changed += 1;
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == '?' && y == '?' {
            untracked += 1;
            continue;
        }
        if matches!(x, 'U' | 'A' | 'D') && matches!(y, 'U' | 'A' | 'D') {
            conflicted += 1;
            continue;
        }
        if x != ' ' {
            staged += 1;
        }
        if y != ' ' {
            unstaged += 1;
        }
    }
    Some(ManagedWorktreeDirtySnapshot {
        clean: changed == 0,
        staged_files: staged,
        unstaged_files: unstaged,
        untracked_files: untracked,
        conflicted_files: conflicted,
        changed_files: changed,
    })
}

fn emit_worktree_changed(event: &str, worktree: &ManagedWorktree) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            event,
            json!({
                "id": worktree.id,
                "sessionId": worktree.session_id,
                "childSessionId": worktree.child_session_id,
                "workflowRunId": worktree.workflow_run_id,
                "purpose": worktree.purpose,
                "state": worktree.state,
                "label": worktree.label,
                "path": worktree.path,
                "pathSource": worktree.path_source,
                "pathExists": worktree.path_exists,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) {
        git_status(cwd, args).unwrap_or_else(|error| panic!("git {args:?}: {error:#}"));
    }

    #[test]
    fn snapshots_and_applies_tracked_binary_and_untracked_changes() {
        let root = tempfile::tempdir().expect("repo tempdir");
        let repo = root.path();
        git(repo, &["init", "-b", "main"]);
        // Keep the fixture byte-stable on Windows runners instead of inheriting
        // a global core.autocrlf setting from the host.
        git(repo, &["config", "core.autocrlf", "false"]);
        fs::write(repo.join("staged.txt"), "base\n").unwrap();
        fs::write(repo.join("unstaged.txt"), "base\n").unwrap();
        fs::write(repo.join("binary.bin"), [0_u8, 1, 2, 3]).unwrap();
        git(repo, &["add", "."]);
        git(
            repo,
            &[
                "-c",
                "user.name=Hope Test",
                "-c",
                "user.email=hope@example.invalid",
                "commit",
                "-m",
                "base",
            ],
        );

        fs::write(repo.join("staged.txt"), "staged change\n").unwrap();
        git(repo, &["add", "staged.txt"]);
        fs::write(repo.join("unstaged.txt"), "unstaged change\n").unwrap();
        fs::write(repo.join("binary.bin"), [0_u8, 9, 8, 7, 0]).unwrap();
        fs::write(repo.join("untracked.txt"), "new\n").unwrap();

        let sha = git_output(repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let run_dir = root.path().join("bootstrap");
        let snapshot =
            snapshot_local_changes(repo, &run_dir, "request-1", "refs/heads/main", &sha).unwrap();
        let worktree = root.path().join("worktree");
        git(
            repo,
            &["worktree", "add", "--detach", path_arg(&worktree), &sha],
        );
        apply_local_changes(repo, &worktree, &snapshot, None).unwrap();

        assert_eq!(
            fs::read_to_string(worktree.join("staged.txt")).unwrap(),
            "staged change\n"
        );
        assert_eq!(
            fs::read_to_string(worktree.join("unstaged.txt")).unwrap(),
            "unstaged change\n"
        );
        assert_eq!(
            fs::read(worktree.join("binary.bin")).unwrap(),
            [0_u8, 9, 8, 7, 0]
        );
        assert_eq!(
            fs::read_to_string(worktree.join("untracked.txt")).unwrap(),
            "new\n"
        );
        let status = git_output(&worktree, &["status", "--porcelain=v1"]).unwrap();
        assert!(status.contains(" M staged.txt"));
        assert!(status.contains(" M unstaged.txt"));
        assert!(status.contains(" M binary.bin"));
        assert!(status.contains("?? untracked.txt"));
        assert!(!status.lines().any(|line| line.starts_with("M ")));

        git(
            repo,
            &["worktree", "remove", "--force", path_arg(&worktree)],
        );
    }

    #[test]
    fn nested_project_snapshot_keeps_untracked_paths_repo_relative() {
        let root = tempfile::tempdir().expect("repo tempdir");
        let repo = root.path();
        git(repo, &["init", "-b", "main"]);
        git(repo, &["config", "core.autocrlf", "false"]);
        let project = repo.join("packages").join("app");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("tracked.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(
            repo,
            &[
                "-c",
                "user.name=Hope Test",
                "-c",
                "user.email=hope@example.invalid",
                "commit",
                "-m",
                "base",
            ],
        );

        fs::write(repo.join("new.txt"), "root file\n").unwrap();
        fs::write(project.join("new.txt"), "nested file\n").unwrap();

        let sha = git_output(repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let run_dir = root.path().join("bootstrap");
        let snapshot =
            snapshot_local_changes(repo, &run_dir, "request-nested", "refs/heads/main", &sha)
                .unwrap();
        assert!(snapshot.untracked.contains(&PathBuf::from("new.txt")));
        assert!(snapshot
            .untracked
            .contains(&PathBuf::from("packages/app/new.txt")));

        let worktree = root.path().join("worktree");
        git(
            repo,
            &["worktree", "add", "--detach", path_arg(&worktree), &sha],
        );
        apply_local_changes(repo, &worktree, &snapshot, None).unwrap();
        assert_eq!(
            fs::read_to_string(worktree.join("new.txt")).unwrap(),
            "root file\n"
        );
        assert_eq!(
            fs::read_to_string(worktree.join("packages/app/new.txt")).unwrap(),
            "nested file\n"
        );

        git(
            repo,
            &["worktree", "remove", "--force", path_arg(&worktree)],
        );
    }

    #[test]
    fn refuses_parent_path_copy() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let error = copy_one_rel(source.path(), target.path(), Path::new("../outside"))
            .expect_err("parent traversal must fail");
        assert!(error.to_string().contains("outside repository"));
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_untracked_path() {
        use std::os::unix::fs::symlink;

        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), source.path().join("linked")).unwrap();
        copy_one_rel(source.path(), target.path(), Path::new("linked/secret.txt")).unwrap();
        assert!(!target.path().join("linked/secret.txt").exists());
    }
}
