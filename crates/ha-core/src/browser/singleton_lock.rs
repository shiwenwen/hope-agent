//! Chrome user-data-dir lock detection.
//!
//! Chrome's "only one instance per profile" guarantee is enforced via a
//! lock file in the user-data-dir. We never bypass it — instead we check
//! it before launching any app-owned Chrome profile, and refuse to reuse a
//! live-locked profile.

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

/// Read the SingletonLock symlink target on Unix, or parse the lockfile on
/// Windows. Returns the owner pid encoded in the lock.
///
/// Unix format: SingletonLock is a symlink whose target is `{hostname}-{pid}`
/// (see Chromium source `chrome/browser/process_singleton_posix.cc`). We
/// only need the trailing pid — the hostname guard is irrelevant when the
/// host changes (hibernate / rename) because pid_alive will return false
/// regardless.
/// Windows format: lockfile is a plain text file whose first line is the pid.
pub fn read_lock_owner_pid(user_data_dir: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        let link = user_data_dir.join("SingletonLock");
        let target = std::fs::read_link(&link).ok()?;
        let s = target.to_string_lossy();
        s.rsplit_once('-').and_then(|(_, p)| p.parse::<u32>().ok())
    }
    #[cfg(windows)]
    {
        let body = std::fs::read_to_string(user_data_dir.join("lockfile")).ok()?;
        body.lines().next()?.trim().parse::<u32>().ok()
    }
}

/// A SingletonLock exists and its owner pid is no longer alive on this host.
///
/// "No lock" returns false (nothing to clean). "Lock exists with unparseable
/// owner" returns true: a hostname mismatch / malformed target would
/// otherwise leave the lock permanently un-cleanable, which is worse than
/// occasionally clearing one whose owner is actually alive (in that case
/// Chrome itself would still bail on the subsequent re-lock).
pub fn is_lock_stale(user_data_dir: &Path) -> bool {
    if !user_data_dir_is_locked(user_data_dir) {
        return false;
    }
    match read_lock_owner_pid(user_data_dir) {
        Some(pid) => !crate::platform::pid_alive(pid),
        None => {
            app_warn!(
                "browser",
                "singleton_lock",
                "Lock at {} has unparseable owner — assuming stale",
                user_data_dir.display()
            );
            true
        }
    }
}

/// Remove a stale SingletonLock (and its sibling Singleton* files on Unix).
///
/// Errors when the lock owner is still alive — callers must escalate via a
/// graceful_quit / user prompt rather than yank the lock out from under a
/// running Chrome. Best-effort `remove_file` for each file: an absent
/// SingletonCookie / SingletonSocket on Unix is fine.
pub fn cleanup_stale_lock(user_data_dir: &Path) -> Result<()> {
    if !user_data_dir_is_locked(user_data_dir) {
        return Ok(());
    }
    if !is_lock_stale(user_data_dir) {
        bail!(
            "Chrome at {} is still running (lock owner alive). \
             Quit it or use a different `profile` arg.",
            user_data_dir.display()
        );
    }
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(user_data_dir.join("SingletonLock"));
        let _ = std::fs::remove_file(user_data_dir.join("SingletonCookie"));
        let _ = std::fs::remove_file(user_data_dir.join("SingletonSocket"));
    }
    #[cfg(windows)]
    {
        let _ = std::fs::remove_file(user_data_dir.join("lockfile"));
    }
    app_info!(
        "browser",
        "singleton_lock",
        "Cleaned stale lock at {}",
        user_data_dir.display()
    );
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

    #[test]
    fn is_lock_stale_returns_false_for_unlocked_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(!is_lock_stale(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn read_lock_owner_pid_parses_unix_format() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Chromium writes `{hostname}-{pid}`; rsplit_once('-') takes the
        // trailing pid even when the hostname itself contains hyphens.
        std::os::unix::fs::symlink("some-host-name-12345", tmp.path().join("SingletonLock"))
            .expect("symlink");
        assert_eq!(read_lock_owner_pid(tmp.path()), Some(12345));
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_stale_lock_refuses_live_owner() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Our own pid is guaranteed alive for the duration of the test.
        let target = format!("host-{}", std::process::id());
        std::os::unix::fs::symlink(&target, tmp.path().join("SingletonLock")).expect("symlink");
        let err = cleanup_stale_lock(tmp.path()).expect_err("should refuse live owner");
        assert!(err.to_string().contains("still running"));
        assert!(tmp.path().join("SingletonLock").symlink_metadata().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_stale_lock_succeeds_for_dead_pid() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // pid 4,000,000,000 is well beyond Linux's `kernel.pid_max` default
        // (4_194_303) and macOS's much-lower 99_998 cap — guaranteed dead.
        std::os::unix::fs::symlink("host-4000000000", tmp.path().join("SingletonLock"))
            .expect("symlink");
        // Also seed sibling files we'd expect Chrome to drop.
        std::fs::write(tmp.path().join("SingletonCookie"), b"").expect("seed cookie");
        cleanup_stale_lock(tmp.path()).expect("should clean stale lock");
        assert!(tmp.path().join("SingletonLock").symlink_metadata().is_err());
        assert!(!tmp.path().join("SingletonCookie").exists());
    }
}
