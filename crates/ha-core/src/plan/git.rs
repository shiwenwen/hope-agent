use anyhow::Result;

use super::store::store;

// ── Git Checkpoint ──────────────────────────────────────────────
// Creates a lightweight git checkpoint before plan execution starts,
// allowing rollback if execution fails.

/// A `git` command that never flashes a console window on Windows
/// (`CREATE_NO_WINDOW`); a no-op wrapper elsewhere. Every git invocation in
/// this module goes through it so a new call site can't silently regress the
/// console-flash fix.
fn git_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    crate::filesystem::isolate_repository_env(&mut cmd);
    crate::platform::hide_console(&mut cmd);
    cmd
}

/// Detect the git repository root directory by running `git rev-parse --show-toplevel`.
/// Returns None if not inside a git repository.
fn git_repo_root() -> Option<std::path::PathBuf> {
    let mut cmd = git_command();
    cmd.args(["rev-parse", "--show-toplevel"]);
    let output = cmd.output().ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path))
        }
    } else {
        None
    }
}

/// Return `true` when `rev` already resolves to a git object (branch, tag,
/// commit). Cheap probe — `rev-parse --verify --quiet` on a missing ref
/// returns in a few ms without touching the object database.
fn ref_exists(git_root: &std::path::Path, rev: &str) -> bool {
    let mut cmd = git_command();
    cmd.current_dir(git_root)
        .args(["rev-parse", "--verify", "--quiet", rev])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Create a git checkpoint (branch) at the current HEAD for the working directory.
/// Returns the checkpoint branch name on success, or None if not in a git repo.
///
/// Branch naming: `hope-agent/checkpoint-{session_short}-{UTC_YYYYMMDDTHHMMSSZ}-{uuid8}`.
/// UTC + UUID tail avoid DST / same-second collisions across devices.
pub fn create_git_checkpoint(session_id: &str) -> Option<String> {
    let git_root = git_repo_root()?;
    let short_id = crate::truncate_utf8(session_id, 8);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");

    let uuid_tail = uuid::Uuid::new_v4().simple().to_string();
    let branch_name = format!(
        "hope-agent/checkpoint-{}-{}-{}",
        short_id,
        ts,
        &uuid_tail[..8]
    );

    if ref_exists(&git_root, &branch_name) {
        app_warn!(
            "plan",
            "checkpoint",
            "Checkpoint branch '{}' already exists — aborting checkpoint",
            branch_name
        );
        return None;
    }

    let mut cmd = git_command();
    cmd.current_dir(&git_root)
        .args(["branch", &branch_name, "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let result = cmd.status();

    match result {
        Ok(s) if s.success() => {
            app_info!(
                "plan",
                "checkpoint",
                "Created git checkpoint branch: {}",
                branch_name
            );
            Some(branch_name)
        }
        _ => {
            app_warn!(
                "plan",
                "checkpoint",
                "Failed to create git checkpoint branch"
            );
            None
        }
    }
}

/// Create a checkpoint and store it in the plan's metadata.
pub async fn create_checkpoint_for_session(session_id: &str) {
    if get_checkpoint_ref(session_id).await.is_some() {
        return;
    }

    if let Some(ref_name) = create_git_checkpoint(session_id) {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            if meta.checkpoint_ref.is_none() {
                meta.checkpoint_ref = Some(ref_name);
            } else {
                drop(map);
                cleanup_checkpoint(&ref_name);
            }
        }
    }
}

/// Get the checkpoint reference for a session.
pub async fn get_checkpoint_ref(session_id: &str) -> Option<String> {
    let map = store().read().await;
    map.get(session_id).and_then(|m| m.checkpoint_ref.clone())
}

/// Rollback to a git checkpoint by resetting the current branch to the checkpoint.
/// This performs a `git reset --hard <checkpoint_branch>` to undo all changes
/// made during plan execution.
pub fn rollback_to_checkpoint(checkpoint_ref: &str) -> Result<String> {
    let git_root = git_repo_root().ok_or_else(|| anyhow::anyhow!("Not inside a git repository"))?;

    if !ref_exists(&git_root, checkpoint_ref) {
        return Err(anyhow::anyhow!(
            "Checkpoint branch '{}' does not exist",
            checkpoint_ref
        ));
    }

    // Get current HEAD for logging
    let mut head_cmd = git_command();
    head_cmd
        .current_dir(&git_root)
        .args(["rev-parse", "--short", "HEAD"]);
    let head_before = head_cmd
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Reset to checkpoint
    let mut reset_cmd = git_command();
    reset_cmd
        .current_dir(&git_root)
        .args(["reset", "--hard", checkpoint_ref]);
    let result = reset_cmd.output()?;

    if result.status.success() {
        let msg = format!(
            "Rolled back from {} to checkpoint '{}'",
            head_before, checkpoint_ref
        );
        app_info!("plan", "checkpoint", "{}", msg);

        // Clean up: delete the checkpoint branch
        let mut del_cmd = git_command();
        del_cmd
            .current_dir(&git_root)
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = del_cmd.status();

        Ok(msg)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        Err(anyhow::anyhow!("Git reset failed: {}", stderr))
    }
}

/// Clean up a checkpoint branch (e.g., after successful execution).
pub fn cleanup_checkpoint(checkpoint_ref: &str) {
    let mut cmd = git_command();
    if let Some(git_root) = git_repo_root() {
        cmd.current_dir(git_root);
    }
    cmd.args(["branch", "-D", checkpoint_ref])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = cmd.status();
    app_info!(
        "plan",
        "checkpoint",
        "Cleaned up checkpoint branch: {}",
        checkpoint_ref
    );
}
