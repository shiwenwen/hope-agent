//! Startup-cause sentinel file.
//!
//! Signal handlers write `~/.hope-agent/.shutdown-clean` before calling
//! `exit(0)` so the next launch can tell "graceful shutdown" apart from
//! "crash / SIGKILL / power loss". Panic hooks intentionally do NOT write
//! it: a panic counts as a crash for finalize semantics.
//!
//! The file is a marker — its existence is the signal; contents are
//! reserved for future debugging hints (currently a short UTF-8 stamp).
//! `read_and_clear()` consumes the file so the next non-graceful exit is
//! identifiable.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupCause {
    /// Sentinel was present — previous run exited cleanly (SIGTERM/SIGINT
    /// signal handler ran to completion).
    Clean,
    /// Sentinel was missing — previous run crashed (panic, SIGKILL,
    /// power loss, OOM kill) or this is a first-ever launch.
    Crash,
}

impl StartupCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Crash => "crash",
        }
    }
}

fn sentinel_path() -> anyhow::Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join(".shutdown-clean"))
}

/// Write the clean-shutdown marker. Called from signal handlers right
/// before `exit(0)`. Synchronous std::fs; safe to call from a signal
/// handler context (no allocator-after-fork hazards because we're not
/// forked, just exiting).
///
/// Failure is best-effort logged to stderr — the process is exiting
/// anyway, and the next launch will simply see "Crash" cause and finalize
/// any orphan turns accordingly, which is the safe fallback.
pub fn write_clean_marker() {
    if let Err(e) = try_write_clean_marker() {
        eprintln!("[hope-agent] failed to write shutdown sentinel: {}", e);
    }
}

fn try_write_clean_marker() -> anyhow::Result<()> {
    let path = sentinel_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stamp = chrono::Utc::now().to_rfc3339();
    std::fs::write(&path, stamp.as_bytes())?;
    Ok(())
}

/// Read sentinel state and remove the file atomically (next process exit
/// must re-write it to claim "clean"). Called once during startup before
/// the orphan-turn finalize sweep so the sweep knows which
/// `TerminationReason` to attach to each stale turn.
///
/// Any I/O error → `Crash` (the safe-by-default classification).
pub fn read_and_clear() -> StartupCause {
    let path = match sentinel_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[hope-agent] sentinel path resolution failed: {}", e);
            return StartupCause::Crash;
        }
    };
    if !path.exists() {
        return StartupCause::Crash;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => StartupCause::Clean,
        Err(e) => {
            eprintln!(
                "[hope-agent] sentinel exists but could not be removed: {} (treating as crash)",
                e
            );
            StartupCause::Crash
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", tmp.path())], || {
            // 1. Missing sentinel reads as Crash.
            assert_eq!(read_and_clear(), StartupCause::Crash);

            // 2. write_clean_marker creates the file.
            write_clean_marker();
            assert!(sentinel_path().unwrap().exists());

            // 3. read_and_clear reports Clean and removes the file.
            assert_eq!(read_and_clear(), StartupCause::Clean);
            assert!(!sentinel_path().unwrap().exists(), "must be removed");

            // 4. Re-read after consume -> Crash again.
            assert_eq!(read_and_clear(), StartupCause::Crash);
        });
    }
}
