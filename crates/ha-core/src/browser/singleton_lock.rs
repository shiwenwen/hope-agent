//! Chrome user-data-dir lock detection.
//!
//! Chrome's "only one instance per profile" guarantee is enforced via a
//! lock file in the user-data-dir. We never bypass it — instead we check
//! it before `target=system` launch so we can either ask the user to
//! quit gracefully or escalate to a forced quit ourselves.

use anyhow::{bail, Result};
use std::path::Path;
use std::time::{Duration, Instant};

/// Returns true if Chrome currently holds the user-data-dir.
///
/// Unix: `SingletonLock` is a symlink (target encodes hostname + pid);
/// we use `symlink_metadata` so a dangling symlink (Chrome crashed
/// without cleanup) still reports locked — that file is what blocks
/// the next launch anyway.
/// Windows: `lockfile` is a plain file, exists() is fine.
pub fn user_data_dir_is_locked(user_data_dir: &Path) -> bool {
    #[cfg(unix)]
    {
        user_data_dir
            .join("SingletonLock")
            .symlink_metadata()
            .is_ok()
    }
    #[cfg(windows)]
    {
        user_data_dir.join("lockfile").exists()
    }
}

/// Poll the lock file until it disappears or the timeout elapses.
/// Used after issuing a graceful_quit / force_kill so the subsequent
/// launch doesn't race the cleanup.
pub async fn wait_for_release(user_data_dir: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while user_data_dir_is_locked(user_data_dir) {
        if Instant::now() >= deadline {
            bail!(
                "Chrome did not release the user-data-dir lock within {:?}",
                timeout
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_no_lock_in_fresh_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(!user_data_dir_is_locked(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn detects_existing_singleton_lock_symlink() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Chrome's lock is a symlink whose target encodes hostname-pid;
        // we don't care what the target is, just that the entry exists.
        std::os::unix::fs::symlink("dangling-pid", tmp.path().join("SingletonLock"))
            .expect("create lock symlink");
        assert!(user_data_dir_is_locked(tmp.path()));
    }

    #[cfg(windows)]
    #[test]
    fn detects_existing_lockfile() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("lockfile"), b"").expect("create lockfile");
        assert!(user_data_dir_is_locked(tmp.path()));
    }

    #[tokio::test]
    async fn wait_for_release_returns_immediately_when_unlocked() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let start = Instant::now();
        wait_for_release(tmp.path(), Duration::from_secs(5))
            .await
            .expect("should return ok");
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
