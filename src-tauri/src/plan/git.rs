use anyhow::Result;

use super::store::store;

// ── Git Checkpoint ──────────────────────────────────────────────
// Creates a lightweight git checkpoint before plan execution starts,
// allowing rollback if execution fails.

/// Detect the git repository root directory by running `git rev-parse --show-toplevel`.
/// Returns None if not inside a git repository.
fn git_repo_root() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
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

/// Create a git checkpoint (branch) at the current HEAD for the working directory.
/// Returns the checkpoint branch name on success, or None if not in a git repo.
pub fn create_git_checkpoint(session_id: &str) -> Option<String> {
    let short_id = crate::truncate_utf8(session_id, 8);
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
    let branch_name = format!("opencomputer/checkpoint-{}-{}", short_id, ts);

    // Detect git repo root directory
    let git_root = git_repo_root()?;

    // Create a checkpoint branch at current HEAD (without switching to it)
    let result = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["branch", &branch_name, "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

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
    if let Some(ref_name) = create_git_checkpoint(session_id) {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            meta.checkpoint_ref = Some(ref_name);
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

    // Verify the checkpoint branch exists
    let check = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["rev-parse", "--verify", checkpoint_ref])
        .output();
    match check {
        Ok(o) if o.status.success() => {}
        _ => {
            return Err(anyhow::anyhow!(
                "Checkpoint branch '{}' does not exist",
                checkpoint_ref
            ))
        }
    }

    // Get current HEAD for logging
    let head_before = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Reset to checkpoint
    let result = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["reset", "--hard", checkpoint_ref])
        .output()?;

    if result.status.success() {
        let msg = format!(
            "Rolled back from {} to checkpoint '{}'",
            head_before, checkpoint_ref
        );
        app_info!("plan", "checkpoint", "{}", msg);

        // Clean up: delete the checkpoint branch
        let _ = std::process::Command::new("git")
            .current_dir(&git_root)
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        Ok(msg)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        Err(anyhow::anyhow!("Git reset failed: {}", stderr))
    }
}

/// Clean up a checkpoint branch (e.g., after successful execution).
pub fn cleanup_checkpoint(checkpoint_ref: &str) {
    let git_cmd = if let Some(git_root) = git_repo_root() {
        std::process::Command::new("git")
            .current_dir(git_root)
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    } else {
        std::process::Command::new("git")
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    };
    let _ = git_cmd;
    app_info!(
        "plan",
        "checkpoint",
        "Cleaned up checkpoint branch: {}",
        checkpoint_ref
    );
}
