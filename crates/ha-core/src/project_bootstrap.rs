//! First-turn preparation for project chats that start in a managed worktree.
//! The desktop and HTTP shells both call this module so validation, durable
//! progress, Git semantics, and session binding stay identical.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::session::SessionDB;
use crate::worktree::{CreateManagedWorktreeInput, ManagedWorktree, ManagedWorktreePurpose};

type ActiveBootstrapMap = std::collections::HashMap<String, Arc<AtomicBool>>;
static ACTIVE_BOOTSTRAPS: OnceLock<Mutex<ActiveBootstrapMap>> = OnceLock::new();

fn active_bootstraps() -> &'static Mutex<ActiveBootstrapMap> {
    ACTIVE_BOOTSTRAPS.get_or_init(|| Mutex::new(ActiveBootstrapMap::new()))
}

struct ActiveBootstrapGuard {
    id: String,
    flag: Arc<AtomicBool>,
}

impl Drop for ActiveBootstrapGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = active_bootstraps().lock() {
            if active
                .get(&self.id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.flag))
            {
                active.remove(&self.id);
            }
        }
    }
}

pub fn cancel_project_bootstrap(request_id: &str) -> bool {
    let Ok(active) = active_bootstraps().lock() else {
        return false;
    };
    let Some(flag) = active.get(request_id) else {
        return false;
    };
    flag.store(true, Ordering::SeqCst);
    true
}

pub(crate) fn is_project_bootstrap_cancelled(request_id: &str) -> bool {
    active_bootstraps()
        .lock()
        .ok()
        .and_then(|active| active.get(request_id).cloned())
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLaunchMode {
    Local,
    Worktree,
}

impl ProjectLaunchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSessionBootstrapInput {
    pub request_id: String,
    pub launch_mode: ProjectLaunchMode,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub include_local_changes: bool,
}

#[derive(Debug, Clone)]
pub struct PrepareProjectWorktreeInput {
    pub request: ProjectSessionBootstrapInput,
    pub session_id: String,
    pub project_id: String,
    pub source_working_dir: String,
}

/// Shared bootstrap entry used by both desktop IPC and HTTP chat adapters.
/// Progress and cancellation are keyed by `request.request_id` and routed
/// through the process EventBus / cancellation registry.
pub async fn bootstrap_project_session(
    db: &Arc<SessionDB>,
    input: PrepareProjectWorktreeInput,
) -> Result<Option<ManagedWorktree>> {
    match input.request.launch_mode {
        ProjectLaunchMode::Local => {
            db.prepare_project_local_branch(input).await?;
            Ok(None)
        }
        ProjectLaunchMode::Worktree => db.prepare_project_worktree(input).await.map(Some),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBootstrapRun {
    pub id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub launch_mode: ProjectLaunchMode,
    pub base_ref: Option<String>,
    pub include_local_changes: bool,
    pub status: String,
    pub stage: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_bootstrap_runs (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            session_id TEXT,
            worktree_id TEXT,
            launch_mode TEXT NOT NULL,
            base_ref TEXT,
            include_local_changes INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            stage TEXT NOT NULL,
            error_code TEXT,
            error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_project_bootstrap_project_updated
            ON project_bootstrap_runs(project_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_project_bootstrap_session
            ON project_bootstrap_runs(session_id);",
    )?;
    Ok(())
}

fn validate_request_id(request_id: &str) -> Result<()> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("bootstrap requestId must use 1-128 letters, digits, '-' or '_'");
    }
    Ok(())
}

fn normalized_branch_ref(base_ref: Option<&str>) -> Result<String> {
    let base_ref = base_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("baseRef is required for project launch"))?;
    if !(base_ref.starts_with("refs/heads/") || base_ref.starts_with("refs/remotes/")) {
        bail!("baseRef must be an existing local or remote-tracking branch");
    }
    Ok(base_ref.to_string())
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    crate::filesystem::isolate_repository_env(&mut command);
    command.current_dir(cwd).args(args);
    crate::platform::hide_console(&mut command);
    let output = command
        .output()
        .map_err(|error| anyhow!("failed to run git {args:?}: {error}"))?;
    if !output.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn prepare_local_branch_on_disk(
    root: &Path,
    base_ref: &str,
    include_local_changes: bool,
) -> Result<()> {
    let info = crate::filesystem::git_info(root)
        .ok_or_else(|| anyhow!("project working directory is not inside a Git repository"))?;
    let selected = info
        .branches
        .iter()
        .find(|branch| branch.full_ref == base_ref)
        .ok_or_else(|| anyhow!("selected branch no longer exists; refresh and retry"))?;

    if selected.is_current {
        if info.dirty.changed_files > 0 && !include_local_changes {
            bail!("current branch has local changes; refresh the branch selection and retry");
        }
        return Ok(());
    }
    if include_local_changes {
        bail!("local changes can only stay on the current local branch");
    }
    if info.dirty.changed_files > 0 {
        bail!("cannot switch the local workspace while it has uncommitted changes");
    }

    let expected_sha = git_output(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", selected.full_ref),
        ],
    )?;
    match selected.kind {
        crate::filesystem::GitBranchKind::Local => {
            git_output(root, &["switch", "--no-guess", &selected.name])?;
        }
        crate::filesystem::GitBranchKind::Remote => {
            git_output(root, &["switch", "--track", &selected.name])?;
        }
    }
    let actual_sha = git_output(root, &["rev-parse", "HEAD"])?;
    if actual_sha.trim() != expected_sha.trim() {
        bail!("the selected branch changed while switching; retry");
    }
    Ok(())
}

impl SessionDB {
    async fn prepare_project_local_branch(
        self: &Arc<Self>,
        input: PrepareProjectWorktreeInput,
    ) -> Result<()> {
        validate_request_id(&input.request.request_id)?;
        let base_ref = normalized_branch_ref(input.request.base_ref.as_deref())?;

        let existing = {
            let db = self.clone();
            let request_id = input.request.request_id.clone();
            db.run(move |db| db.get_project_bootstrap_run(&request_id))
                .await?
        };
        if let Some(existing) = existing {
            if existing.session_id.as_deref() == Some(input.session_id.as_str())
                && matches!(existing.status.as_str(), "ready" | "chatting" | "completed")
            {
                return Ok(());
            }
            bail!(
                "bootstrap request already exists with status {}",
                existing.status
            );
        }

        let session = {
            let db = self.clone();
            let session_id = input.session_id.clone();
            db.run(move |db| db.get_session(&session_id)).await?
        }
        .ok_or_else(|| anyhow!("session not found: {}", input.session_id))?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            bail!("bootstrap session is not bound to the requested project");
        }

        self.insert_project_bootstrap_run(&input, &base_ref)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = active_bootstraps()
                .lock()
                .map_err(|_| anyhow!("bootstrap cancellation registry is unavailable"))?;
            active.insert(input.request.request_id.clone(), cancel.clone());
        }
        let _active_guard = ActiveBootstrapGuard {
            id: input.request.request_id.clone(),
            flag: cancel.clone(),
        };
        self.report_project_bootstrap_stage(
            &input.request.request_id,
            "resolving_git",
            Some(&input.session_id),
            None,
        )?;

        let source = input.source_working_dir.clone();
        let include_local_changes = input.request.include_local_changes;
        let switch_result = crate::blocking::run_blocking(move || -> Result<()> {
            let root = Path::new(&source)
                .canonicalize()
                .map_err(|error| anyhow!("cannot resolve project working directory: {error}"))?;
            prepare_local_branch_on_disk(&root, &base_ref, include_local_changes)
        })
        .await;

        match switch_result {
            Ok(()) if !cancel.load(Ordering::SeqCst) => {
                self.update_project_bootstrap_stage(
                    &input.request.request_id,
                    "ready",
                    "ready",
                    None,
                    None,
                )?;
                emit_progress(
                    &input.request.request_id,
                    "ready",
                    "ready",
                    Some(&input.session_id),
                    None,
                    None,
                );
                Ok(())
            }
            Ok(()) => {
                self.update_project_bootstrap_stage(
                    &input.request.request_id,
                    "cancelled",
                    "cancelled",
                    None,
                    Some(("cancelled", "Local branch preparation was cancelled")),
                )?;
                emit_progress(
                    &input.request.request_id,
                    "cancelled",
                    "cancelled",
                    Some(&input.session_id),
                    None,
                    Some(("cancelled", "Local branch preparation was cancelled")),
                );
                bail!("local branch preparation was cancelled")
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.update_project_bootstrap_stage(
                    &input.request.request_id,
                    "failed",
                    "failed",
                    None,
                    Some(("local_branch_prepare_failed", message.as_str())),
                )?;
                emit_progress(
                    &input.request.request_id,
                    "failed",
                    "failed",
                    Some(&input.session_id),
                    None,
                    Some(("local_branch_prepare_failed", message.as_str())),
                );
                Err(error)
            }
        }
    }

    /// Primary-only startup recovery. A secondary process must never mark a
    /// bootstrap owned by the primary as interrupted merely by opening the DB.
    pub fn reconcile_interrupted_project_bootstraps(&self) -> Result<usize> {
        let stale = {
            let conn = self
                .conn
                .lock()
                .map_err(|error| anyhow!("Lock error: {error}"))?;
            let mut statement = conn.prepare(
                "SELECT id, session_id, worktree_id
                 FROM project_bootstrap_runs
                 WHERE status IN ('preparing', 'resolving_git', 'snapshotting',
                                  'creating_worktree', 'copying_changes', 'binding_session',
                                  'ready', 'chatting')",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let now = chrono::Utc::now().timestamp_millis();
        for (request_id, session_id, worktree_id) in &stale {
            {
                let conn = self
                    .conn
                    .lock()
                    .map_err(|error| anyhow!("Lock error: {error}"))?;
                conn.execute(
                    "UPDATE project_bootstrap_runs
                     SET status = 'interrupted', stage = 'interrupted',
                         error_code = 'process_restarted',
                         error_message = 'The application restarted while preparing the worktree',
                         updated_at = ?2, completed_at = ?2
                     WHERE id = ?1",
                    params![request_id, now],
                )?;
            }

            let has_chat_messages = if let Some(session_id) = session_id.as_deref() {
                let (user, assistant) = self.count_user_assistant_messages(session_id)?;
                user > 0 || assistant > 0
            } else {
                false
            };
            if has_chat_messages {
                // The bootstrap crossed into the chat path before the crash.
                // Preserve its session/worktree for normal chat recovery, but
                // never resume the first prompt automatically.
                continue;
            }

            let mut cleanup_ok = true;
            if let Some(worktree_id) = worktree_id.as_deref() {
                if let Err(error) = self.discard_managed_worktree(worktree_id) {
                    let orphan_cleanup =
                        crate::worktree::cleanup_orphan_builtin_worktree(worktree_id);
                    if let Err(orphan_error) = orphan_cleanup {
                        cleanup_ok = false;
                        let _ = self.mark_managed_worktree_bootstrap_failed(worktree_id);
                        let conn = self
                            .conn
                            .lock()
                            .map_err(|lock_error| anyhow!("Lock error: {lock_error}"))?;
                        conn.execute(
                            "UPDATE project_bootstrap_runs
                             SET error_message = ?2, updated_at = ?3
                             WHERE id = ?1",
                            params![
                                request_id,
                                format!(
                                    "Startup cleanup failed: {error:#}; orphan cleanup: {:#}",
                                    orphan_error
                                ),
                                now
                            ],
                        )?;
                    }
                }
            }

            if cleanup_ok {
                if let Some(session_id) = session_id.as_deref() {
                    self.delete_session(session_id)?;
                }
            }
            if let Ok(path) = crate::paths::bootstrap_run_dir(request_id) {
                let _ = std::fs::remove_dir_all(path);
            }
        }
        Ok(stale.len())
    }

    async fn prepare_project_worktree(
        self: &Arc<Self>,
        input: PrepareProjectWorktreeInput,
    ) -> Result<ManagedWorktree> {
        if input.request.launch_mode != ProjectLaunchMode::Worktree {
            bail!("project worktree preparation requires launchMode=worktree");
        }
        validate_request_id(&input.request.request_id)?;
        let base_ref = normalized_branch_ref(input.request.base_ref.as_deref())?;

        let existing = {
            let db = self.clone();
            let request_id = input.request.request_id.clone();
            db.run(move |db| db.get_project_bootstrap_run(&request_id))
                .await?
        };
        if let Some(existing) = existing {
            if existing.session_id.as_deref() != Some(input.session_id.as_str()) {
                bail!(
                    "bootstrap request already belongs to session {}",
                    existing.session_id.as_deref().unwrap_or("<unknown>")
                );
            }
            if existing.status == "ready"
                || existing.status == "chatting"
                || existing.status == "completed"
            {
                if let Some(worktree_id) = existing.worktree_id.as_deref() {
                    let worktree = {
                        let db = self.clone();
                        let worktree_id = worktree_id.to_string();
                        db.run(move |db| db.get_managed_worktree(&worktree_id))
                            .await?
                    };
                    if let Some(worktree) = worktree {
                        return Ok(worktree);
                    }
                }
            }
            bail!(
                "bootstrap request already exists with status {}",
                existing.status
            );
        }

        let session = {
            let db = self.clone();
            let session_id = input.session_id.clone();
            db.run(move |db| db.get_session(&session_id)).await?
        }
        .ok_or_else(|| anyhow!("session not found: {}", input.session_id))?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            bail!("bootstrap session is not bound to the requested project");
        }

        self.insert_project_bootstrap_run(&input, &base_ref)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = active_bootstraps()
                .lock()
                .map_err(|_| anyhow!("bootstrap cancellation registry is unavailable"))?;
            active.insert(input.request.request_id.clone(), cancel.clone());
        }
        let _active_guard = ActiveBootstrapGuard {
            id: input.request.request_id.clone(),
            flag: cancel.clone(),
        };
        self.report_project_bootstrap_stage(
            &input.request.request_id,
            "resolving_git",
            Some(&input.session_id),
            None,
        )?;

        let source = input.source_working_dir.clone();
        let base_ref_for_validation = base_ref.clone();
        let validation = crate::blocking::run_blocking(move || -> Result<()> {
            let root = Path::new(&source)
                .canonicalize()
                .map_err(|error| anyhow!("cannot resolve project working directory: {error}"))?;
            let info = crate::filesystem::git_info(&root).ok_or_else(|| {
                anyhow!("project working directory is not inside a Git repository")
            })?;
            if !info
                .branches
                .iter()
                .any(|branch| branch.full_ref == base_ref_for_validation)
            {
                bail!("selected branch no longer exists; refresh and retry");
            }
            Ok(())
        })
        .await;
        if let Err(error) = validation {
            let message = format!("{error:#}");
            self.update_project_bootstrap_stage(
                &input.request.request_id,
                "failed",
                "failed",
                None,
                Some(("git_validation_failed", message.as_str())),
            )?;
            emit_progress(
                &input.request.request_id,
                "failed",
                "failed",
                Some(&input.session_id),
                None,
                Some(("git_validation_failed", message.as_str())),
            );
            return Err(error);
        }
        if cancel.load(Ordering::SeqCst) {
            self.update_project_bootstrap_stage(
                &input.request.request_id,
                "cancelled",
                "cancelled",
                None,
                Some(("cancelled", "Worktree preparation was cancelled")),
            )?;
            emit_progress(
                &input.request.request_id,
                "cancelled",
                "cancelled",
                Some(&input.session_id),
                None,
                Some(("cancelled", "Worktree preparation was cancelled")),
            );
            bail!("worktree preparation was cancelled");
        }
        if input.request.include_local_changes {
            self.update_project_bootstrap_stage(
                &input.request.request_id,
                "preparing",
                "snapshotting",
                None,
                None,
            )?;
            emit_progress(
                &input.request.request_id,
                "preparing",
                "snapshotting",
                None,
                None,
                None,
            );
        }
        if cancel.load(Ordering::SeqCst) {
            self.update_project_bootstrap_stage(
                &input.request.request_id,
                "cancelled",
                "cancelled",
                None,
                Some(("cancelled", "Worktree preparation was cancelled")),
            )?;
            emit_progress(
                &input.request.request_id,
                "cancelled",
                "cancelled",
                Some(&input.session_id),
                None,
                Some(("cancelled", "Worktree preparation was cancelled")),
            );
            bail!("worktree preparation was cancelled");
        }

        let mut result = self
            .create_managed_worktree(CreateManagedWorktreeInput {
                session_id: input.session_id.clone(),
                source_working_dir: Some(input.source_working_dir),
                label: None,
                purpose: ManagedWorktreePurpose::Manual,
                workflow_run_id: None,
                child_session_id: None,
                base_ref: Some(base_ref),
                include_local_changes: input.request.include_local_changes,
                bootstrap_request_id: Some(input.request.request_id.clone()),
                bind_session_working_dir: true,
            })
            .await;

        if cancel.load(Ordering::SeqCst) {
            if let Ok(worktree) = result.as_ref() {
                let worktree_id = worktree.id.clone();
                let db = self.clone();
                let _ = db
                    .run(move |db| db.discard_managed_worktree(&worktree_id))
                    .await;
            }
            result = Err(anyhow!("worktree preparation was cancelled"));
        }

        match result {
            Ok(worktree) => {
                self.update_project_bootstrap_stage(
                    &input.request.request_id,
                    "ready",
                    "ready",
                    Some(&worktree.id),
                    None,
                )?;
                emit_progress(
                    &input.request.request_id,
                    "ready",
                    "ready",
                    Some(&input.session_id),
                    Some(&worktree.id),
                    None,
                );
                Ok(worktree)
            }
            Err(error) => {
                let cleanup_error = {
                    let run = {
                        let db = self.clone();
                        let request_id = input.request.request_id.clone();
                        db.run(move |db| db.get_project_bootstrap_run(&request_id))
                            .await
                            .ok()
                            .flatten()
                    };
                    if let Some(worktree_id) = run.and_then(|run| run.worktree_id) {
                        let db = self.clone();
                        let cleanup_id = worktree_id.clone();
                        db.run(move |db| {
                            if db.get_managed_worktree(&cleanup_id)?.is_some() {
                                db.discard_managed_worktree(&cleanup_id)
                            } else {
                                crate::worktree::cleanup_orphan_builtin_worktree(&cleanup_id)
                                    .map(|_| ())
                            }
                        })
                        .await
                        .err()
                        .map(|cleanup_error| {
                            let _ = self.mark_managed_worktree_bootstrap_failed(&worktree_id);
                            format!("; cleanup failed: {cleanup_error:#}")
                        })
                    } else {
                        None
                    }
                };
                let message = format!("{error:#}{}", cleanup_error.unwrap_or_default());
                let cancelled = cancel.load(Ordering::SeqCst);
                let status = if cancelled { "cancelled" } else { "failed" };
                let error_code = if cancelled {
                    "cancelled"
                } else {
                    "worktree_prepare_failed"
                };
                self.update_project_bootstrap_stage(
                    &input.request.request_id,
                    status,
                    status,
                    None,
                    Some((error_code, message.as_str())),
                )?;
                emit_progress(
                    &input.request.request_id,
                    status,
                    status,
                    Some(&input.session_id),
                    None,
                    Some((error_code, message.as_str())),
                );
                Err(error)
            }
        }
    }

    pub fn get_project_bootstrap_run(&self, id: &str) -> Result<Option<ProjectBootstrapRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow!("Lock error: {error}"))?;
        conn.query_row(
            "SELECT id, project_id, session_id, worktree_id, launch_mode, base_ref,
                    include_local_changes, status, stage, error_code, error_message,
                    created_at, updated_at, completed_at
             FROM project_bootstrap_runs WHERE id = ?1",
            params![id],
            |row| {
                let launch_mode: String = row.get(4)?;
                Ok(ProjectBootstrapRun {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    session_id: row.get(2)?,
                    worktree_id: row.get(3)?,
                    launch_mode: if launch_mode == "worktree" {
                        ProjectLaunchMode::Worktree
                    } else {
                        ProjectLaunchMode::Local
                    },
                    base_ref: row.get(5)?,
                    include_local_changes: row.get::<_, i64>(6)? != 0,
                    status: row.get(7)?,
                    stage: row.get(8)?,
                    error_code: row.get(9)?,
                    error_message: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    completed_at: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Atomically claim the first chat transition. Only one caller may move a
    /// prepared request from ready to chatting.
    pub fn claim_project_bootstrap_chatting(&self, id: &str) -> Result<bool> {
        let now = chrono::Utc::now().timestamp_millis();
        let changed = {
            let conn = self
                .conn
                .lock()
                .map_err(|error| anyhow!("Lock error: {error}"))?;
            conn.execute(
                "UPDATE project_bootstrap_runs
                 SET status = 'chatting', stage = 'chatting', updated_at = ?2
                 WHERE id = ?1 AND status = 'ready'",
                params![id, now],
            )?
        };
        if changed > 0 {
            emit_progress(id, "chatting", "chatting", None, None, None);
        }
        Ok(changed > 0)
    }

    pub fn mark_project_bootstrap_completed(&self, id: &str) -> Result<bool> {
        let now = chrono::Utc::now().timestamp_millis();
        let changed = {
            let conn = self
                .conn
                .lock()
                .map_err(|error| anyhow!("Lock error: {error}"))?;
            conn.execute(
                "UPDATE project_bootstrap_runs
                 SET status = 'completed', stage = 'completed', updated_at = ?2,
                     completed_at = ?2
                 WHERE id = ?1 AND status = 'chatting'",
                params![id, now],
            )?
        };
        if changed > 0 {
            emit_progress(id, "completed", "completed", None, None, None);
            emit_completed(id);
        }
        Ok(changed > 0)
    }

    pub(crate) fn report_project_bootstrap_stage(
        &self,
        id: &str,
        stage: &str,
        session_id: Option<&str>,
        worktree_id: Option<&str>,
    ) -> Result<()> {
        self.update_project_bootstrap_stage(id, "preparing", stage, worktree_id, None)?;
        emit_progress(id, "preparing", stage, session_id, worktree_id, None);
        Ok(())
    }

    fn insert_project_bootstrap_run(
        &self,
        input: &PrepareProjectWorktreeInput,
        base_ref: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow!("Lock error: {error}"))?;
        conn.execute(
            "INSERT INTO project_bootstrap_runs (
                id, project_id, session_id, launch_mode, base_ref, include_local_changes,
                status, stage, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'preparing', 'preparing', ?7, ?7)",
            params![
                input.request.request_id,
                input.project_id,
                input.session_id,
                input.request.launch_mode.as_str(),
                base_ref,
                input.request.include_local_changes as i64,
                now,
            ],
        )?;
        Ok(())
    }

    fn update_project_bootstrap_stage(
        &self,
        id: &str,
        status: &str,
        stage: &str,
        worktree_id: Option<&str>,
        error: Option<(&str, &str)>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let completed = matches!(status, "completed" | "failed" | "cancelled" | "interrupted");
        let (error_code, error_message) = error.unzip();
        let conn = self
            .conn
            .lock()
            .map_err(|lock_error| anyhow!("Lock error: {lock_error}"))?;
        conn.execute(
            "UPDATE project_bootstrap_runs
             SET status = ?2, stage = ?3, worktree_id = COALESCE(?4, worktree_id),
                 error_code = ?5, error_message = ?6, updated_at = ?7,
                 completed_at = CASE WHEN ?8 THEN ?7 ELSE completed_at END
             WHERE id = ?1",
            params![
                id,
                status,
                stage,
                worktree_id,
                error_code,
                error_message,
                now,
                completed
            ],
        )?;
        Ok(())
    }
}

fn emit_progress(
    request_id: &str,
    status: &str,
    stage: &str,
    session_id: Option<&str>,
    worktree_id: Option<&str>,
    error: Option<(&str, &str)>,
) {
    let (error_code, message) = error.unzip();
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "project:bootstrap_progress",
            json!({
                "requestId": request_id,
                "status": status,
                "stage": stage,
                "sessionId": session_id,
                "worktreeId": worktree_id,
                "errorCode": error_code,
                "message": message,
            }),
        );
    }
}

fn emit_completed(request_id: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "project:bootstrap_completed",
            json!({ "requestId": request_id }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) {
        git_output(cwd, args).unwrap_or_else(|error| panic!("git {args:?}: {error:#}"));
    }

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
            .expect("session db");
        (dir, db)
    }

    fn insert_run(db: &SessionDB, id: &str, status: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO project_bootstrap_runs (
                id, project_id, launch_mode, include_local_changes,
                status, stage, created_at, updated_at
             ) VALUES (?1, 'project-1', 'worktree', 0, ?2, ?2, 1, 1)",
            params![id, status],
        )
        .unwrap();
    }

    #[test]
    fn chatting_transition_is_claimed_once() {
        let (_dir, db) = test_db();
        insert_run(&db, "request-1", "ready");

        assert!(db.claim_project_bootstrap_chatting("request-1").unwrap());
        assert!(!db.claim_project_bootstrap_chatting("request-1").unwrap());
        assert!(db.mark_project_bootstrap_completed("request-1").unwrap());
        assert!(!db.mark_project_bootstrap_completed("request-1").unwrap());
        let run = db.get_project_bootstrap_run("request-1").unwrap().unwrap();
        assert_eq!(run.status, "completed");
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn table_migration_does_not_interrupt_primary_owned_run() {
        let (_dir, db) = test_db();
        insert_run(&db, "request-2", "creating_worktree");
        {
            let conn = db.conn.lock().unwrap();
            ensure_tables(&conn).unwrap();
        }
        assert_eq!(
            db.get_project_bootstrap_run("request-2")
                .unwrap()
                .unwrap()
                .status,
            "creating_worktree"
        );
        assert_eq!(db.reconcile_interrupted_project_bootstraps().unwrap(), 1);
        assert_eq!(
            db.get_project_bootstrap_run("request-2")
                .unwrap()
                .unwrap()
                .status,
            "interrupted"
        );
    }

    #[test]
    fn local_launch_switches_clean_workspace_and_rejects_dirty_switch() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        git(repo.path(), &["init", "-b", "main"]);
        std::fs::write(repo.path().join("file.txt"), "main\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(
            repo.path(),
            &[
                "-c",
                "user.name=Hope Test",
                "-c",
                "user.email=hope@example.invalid",
                "commit",
                "-m",
                "main",
            ],
        );
        git(repo.path(), &["switch", "-c", "feature"]);
        std::fs::write(repo.path().join("file.txt"), "feature\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(
            repo.path(),
            &[
                "-c",
                "user.name=Hope Test",
                "-c",
                "user.email=hope@example.invalid",
                "commit",
                "-m",
                "feature",
            ],
        );
        git(repo.path(), &["switch", "main"]);

        prepare_local_branch_on_disk(repo.path(), "refs/heads/feature", false).unwrap();
        assert_eq!(
            git_output(repo.path(), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "feature"
        );

        git(repo.path(), &["switch", "main"]);
        std::fs::write(repo.path().join("file.txt"), "dirty\n").unwrap();
        let error = prepare_local_branch_on_disk(repo.path(), "refs/heads/feature", false)
            .expect_err("dirty workspace must not switch branches");
        assert!(error.to_string().contains("uncommitted changes"));
        prepare_local_branch_on_disk(repo.path(), "refs/heads/main", true)
            .expect("current branch keeps local changes");
    }
}
