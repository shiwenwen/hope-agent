//! Cross-process Primary / Secondary election via an OS advisory lock
//! on `~/.hope-agent/runtime.lock`.
//!
//! ## Why
//!
//! `init_runtime()` and `start_background_tasks()` run startup cleanup
//! sweeps and several "global only-one" loops (cron scheduler, channel
//! auto-start, dreaming, retention timers, MCP watchdog, ACP backend
//! discovery, async-job replay). Before this module they assumed a
//! single live hope-agent process per data dir — desktop on its own.
//! Once `hope-agent server` and `hope-agent acp` started calling the
//! same paths (PR 6) that assumption broke: a second process's startup
//! sweep happily mark-errors the first process's running subagents,
//! deletes its incognito sessions, clears its cron `running_at` tokens,
//! flips its async-tool jobs to `Interrupted`. Two cron schedulers also
//! double-claim the same job; two channel auto-starts fight for the
//! same Telegram bot webhook.
//!
//! ## Contract
//!
//! - First process to acquire the lock is **Primary**. Tier is decided
//!   by `acquire_or_secondary()` exactly once per process; further
//!   callers see the same tier via `is_primary()`.
//! - Mode (desktop / server / acp) does not influence tier — first-come
//!   first-served. ACP-only deployments naturally become Primary.
//! - The OS releases the lock when the holding fd is closed, including
//!   on `exit`, panic, `SIGKILL`, and power loss. There is no heartbeat
//!   to tune; no time-based "is the previous owner really dead?" check.
//! - The lock file persists across reboots (deletion + recreation is a
//!   race on Unix because `flock` is inode-based). Only the diagnostic
//!   contents change.
//!
//! ## What "Primary" gates
//!
//! See `app_init.rs` — every callsite that mutates *shared* SQLite
//! state at startup or runs a "single owner" loop is wrapped in
//! `if runtime_lock::is_primary() { ... }`. Per-process state and
//! manual user actions stay tier-agnostic.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::platform;

/// Result of `acquire_or_secondary()`. Once decided for a given process
/// it does not change — secondary processes do not get promoted by
/// observing the holder die. The next process to start gets Primary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Primary,
    Secondary,
}

/// Diagnostic snapshot of who claims to hold the lock right now. Read
/// from the lock file's body, not from the OS lock state, so it can be
/// stale (e.g. between Primary process death and the next Primary
/// truncating the body). Useful for log lines, never for correctness.
#[derive(Debug, Clone)]
pub struct HolderInfo {
    pub pid: u32,
    pub started_at_unix: i64,
    pub role: String,
}

// Module state. `LOCK_FILE` keeps the holder File alive for the
// process's lifetime — dropping it would release the OS lock. `TIER`
// caches the decision so `is_primary()` is constant-time after the
// first acquire.
static LOCK_FILE: OnceLock<Option<File>> = OnceLock::new();
static TIER: OnceLock<Tier> = OnceLock::new();

/// Path to the lock file: `<root>/runtime.lock`. Returns `None` when
/// the data dir cannot be resolved (no `$HOME` etc.) — callers should
/// treat that as a degraded environment and run as Secondary.
fn lock_path() -> Option<PathBuf> {
    crate::paths::root_dir()
        .ok()
        .map(|d| d.join("runtime.lock"))
}

/// Acquire the runtime lock or fall back to Secondary. Idempotent —
/// safe to call from each entry point (desktop / server / acp). The
/// first call decides the tier for the whole process.
pub fn acquire_or_secondary(role: &str) -> Tier {
    if let Some(t) = TIER.get() {
        return *t;
    }
    let tier = match try_acquire(role) {
        Ok(true) => Tier::Primary,
        Ok(false) => Tier::Secondary,
        Err(e) => {
            // FS errors that aren't contention: log and downgrade.
            // Running as Secondary is the safer fallback — losing one
            // primary's worth of cleanup beats running two.
            eprintln!(
                "[runtime_lock] failed to acquire {:?}: {} — running as Secondary",
                lock_path(),
                e
            );
            Tier::Secondary
        }
    };
    let _ = TIER.set(tier);
    tier
}

/// Inner helper. Returns `Ok(true)` when this process now holds the
/// lock (and the holder file is parked in `LOCK_FILE` for the process
/// lifetime), `Ok(false)` on contention, `Err` on FS / permission
/// errors that weren't contention.
fn try_acquire(role: &str) -> std::io::Result<bool> {
    let Some(path) = lock_path() else {
        return Ok(false);
    };
    match platform::try_acquire_exclusive_lock(&path)? {
        Some(file) => {
            write_holder_body(&file, role);
            // Park the File in the static so dropping doesn't release
            // the OS lock. If `LOCK_FILE.set` loses a race (someone
            // else got there first via a concurrent caller — unlikely
            // since we're idempotent on TIER but still) the parked
            // handle is the original; ours drops and releases. That's
            // a benign race — the winner becomes Primary.
            let _ = LOCK_FILE.set(Some(file));
            Ok(true)
        }
        None => Ok(false),
    }
}

fn write_holder_body(mut file: &File, role: &str) {
    use std::io::Seek;
    let _ = file.set_len(0);
    let _ = file.seek(std::io::SeekFrom::Start(0));
    let body = format!(
        "{}\n{}\n{}\n",
        std::process::id(),
        chrono::Utc::now().timestamp(),
        role,
    );
    let _ = file.write_all(body.as_bytes());
    let _ = file.sync_all();
}

/// `true` when this process holds the runtime lock and is responsible
/// for startup cleanup + Primary-only loops.
pub fn is_primary() -> bool {
    matches!(TIER.get(), Some(Tier::Primary))
}

/// Decoded tier, or `None` if `acquire_or_secondary` has not been
/// called yet on this process. Mostly useful in tests; production
/// callers should prefer `is_primary()`.
pub fn tier() -> Option<Tier> {
    TIER.get().copied()
}

/// Best-effort diagnostic read of who claims to hold the lock. Reads
/// the lock file body without taking the lock, so it does **not**
/// signal liveness — the process may have died moments ago. Useful for
/// log lines like "we're Secondary, Primary appears to be PID 12345".
pub fn current_holder() -> Option<HolderInfo> {
    let path = lock_path()?;
    let body = std::fs::read_to_string(&path).ok()?;
    let mut lines = body.lines();
    let pid: u32 = lines.next()?.parse().ok()?;
    let started_at_unix: i64 = lines.next()?.parse().ok()?;
    let role = lines.next()?.to_string();
    Some(HolderInfo {
        pid,
        started_at_unix,
        role,
    })
}
