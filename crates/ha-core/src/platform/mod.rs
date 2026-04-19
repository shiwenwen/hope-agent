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
