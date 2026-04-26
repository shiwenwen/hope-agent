//! Cross-platform shims for OS-specific behavior.
//!
//! Entry points here are called from code that would otherwise carry
//! inline `#[cfg]` branches scattered across the codebase. Each entry
//! point has a single documented signature; platform-specific modules
//! (`unix.rs`, `windows.rs`) provide the concrete implementation for
//! their target.
//!
//! Guidelines:
//! - Prefer `#[cfg(unix)]` / `#[cfg(windows)]` over `target_os = "linux"`
//!   so macOS + Linux + BSDs share a path.
//! - Keep signatures the same across platforms so callers never need a
//!   `#[cfg]` branch themselves.

use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

/// Kill a process and its descendants forcefully.
///
/// Unix: sends `SIGKILL` to `-pid` (the whole process group) — requires
/// the child to have been spawned with `setpgid(0, 0)` in `pre_exec`.
/// Windows: `taskkill /F /T /PID {pid}` walks the job tree.
pub fn terminate_process_tree(pid: u32) {
    imp::terminate_process_tree(pid)
}

/// Ask a process to shut down cleanly. Best-effort; caller should
/// follow up with `wait()` + a timeout and then `terminate_process_tree`.
///
/// Unix: `SIGTERM` to `pid` (not the group — callers use this for
/// supervised children where the group-wide stop is handled separately).
/// Windows: `taskkill /PID {pid}` (no `/F` — sends WM_CLOSE to top-level
/// windows and CTRL_BREAK to console apps).
pub fn send_graceful_stop(pid: u32) {
    imp::send_graceful_stop(pid)
}

/// Try to discover the user-configured HTTP proxy from the OS.
///
/// - macOS: reads `scutil --proxy` (implemented per-caller in
///   `provider/proxy.rs` / `docker/proxy.rs` today — those paths
///   continue to own that logic and don't go through this shim).
/// - Linux: returns `None` (users set `HTTP_PROXY` / `HTTPS_PROXY` env).
/// - Windows: reads
///   `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
///   and returns e.g. `"http://127.0.0.1:1082"` when enabled.
pub fn detect_system_proxy() -> Option<String> {
    imp::detect_system_proxy()
}

/// Build a `std::process::Command` that runs `cmdline` through the
/// platform default shell.
///
/// Unix: `sh -c "<cmdline>"`.
/// Windows: `cmd /C <cmdline>` with `raw_arg` to preserve quoting
/// semantics. Callers still need to do their own argument escaping if
/// the command string contains untrusted input.
pub fn default_shell_command(cmdline: &str) -> Command {
    imp::default_shell_command(cmdline)
}

/// Same as [`default_shell_command`] but returns a
/// `tokio::process::Command` for async call sites.
pub fn default_shell_command_tokio(cmdline: &str) -> tokio::process::Command {
    imp::default_shell_command_tokio(cmdline)
}

/// Return a short, human-readable OS version string for diagnostic /
/// error reporting (e.g. `"macOS 14.2.1"`, `"Windows 11 (26100)"`,
/// `"Linux 6.8.0"`). Never fails — returns `"unknown"` as a last resort.
pub fn os_version_string() -> String {
    imp::os_version_string()
}

/// Try to take an exclusive, advisory, process-scoped lock on `path`.
///
/// - **Success** (`Ok(Some(file))`): caller holds the lock until `file`
///   is dropped or the process exits. The OS releases the lock on
///   process termination (normal exit, panic, SIGKILL, power loss).
/// - **Contention** (`Ok(None)`): another live process already holds it.
///   Caller should run as Secondary.
/// - **Error**: filesystem / permission failure unrelated to contention.
///
/// Used by [`crate::runtime_lock`] to elect a single Primary process
/// across desktop / `hope-agent server` / `hope-agent acp` so that
/// startup cleanup and "global only-one" loops don't run twice.
///
/// Unix: `flock(LOCK_EX | LOCK_NB)` on a file opened with `O_CLOEXEC`,
/// so `fork`ed children don't inherit the lock fd.
/// Windows: `OpenOptions::share_mode(0)` (`FILE_SHARE_NONE`) for a
/// kernel-enforced exclusive open, plus `FILE_FLAG_NO_INHERIT_HANDLE`.
pub fn try_acquire_exclusive_lock(
    path: &std::path::Path,
) -> std::io::Result<Option<std::fs::File>> {
    imp::try_acquire_exclusive_lock(path)
}

/// Atomically write a file containing a secret (OAuth tokens, API keys).
///
/// Creates parent directories if missing, writes to a temp file in the
/// same directory, `fsync`s, sets 0600 (Unix) / clears inherited ACL
/// entries (Windows), then renames over the target path. Callers should
/// use this for anything that must not be readable by other local users.
///
/// Unix: `chmod 0600` after write so the file inherits the stricter
/// permission even if the parent dir is group-writable.
/// Windows: writes the file and relies on NTFS DACL inheritance — a
/// stronger ACL pass can be layered on later without API change.
pub fn write_secure_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    imp::write_secure_file(path, bytes)
}

/// Best-effort search for a Chrome / Chromium / Edge executable when the
/// user has not configured an explicit path. Mostly used as a safety net
/// in front of `chromiumoxide`'s own lookup, which is good but can miss
/// non-default install locations on Windows.
///
/// Returns `None` on Unix — `chromiumoxide` already covers the macOS
/// `.app` bundle and common Linux paths via `which`.
pub fn find_chrome_executable() -> Option<PathBuf> {
    imp::find_chrome_executable()
}
