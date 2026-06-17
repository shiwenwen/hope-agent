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

pub(crate) mod keep_awake;
pub(crate) mod service;
pub(crate) mod system_permissions;
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

/// Best-effort: is a process with this pid still running on this host?
///
/// Used by [`crate::browser::singleton_lock`] to detect stale SingletonLock
/// files (lock present, but owner crashed without cleanup). False negatives
/// (live process, reported dead) leave a real Chrome's lock alone — the
/// worst outcome is a misleading "already in use" error. False positives
/// (dead process, reported alive) keep stale locks around — the worst
/// outcome is the user has to hand-clean. sysinfo polls `/proc` on Linux,
/// `proc_pidinfo` on macOS, and `Process32First` on Windows; ~1ms cost is
/// acceptable for the once-per-launch caller.
pub fn pid_alive(pid: u32) -> bool {
    let target = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), false);
    sys.process(target).is_some()
}

/// Try to discover the user-configured HTTP proxy from the OS.
///
/// - macOS: reads `scutil --proxy`.
/// - Linux / BSD: env vars first, then GNOME `gsettings`, then KDE
///   `kreadconfig6` / `kreadconfig5`.
/// - Windows: reads
///   `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
///   and returns e.g. `"http://127.0.0.1:1082"` when enabled.
pub fn detect_system_proxy() -> Option<String> {
    imp::detect_system_proxy()
}

/// Try to obtain a precise OS-backed location for weather.
///
/// macOS: uses CoreLocation. Other platforms currently return `None`, so
/// callers can fall back to IP geolocation without carrying `#[cfg]` branches.
pub async fn current_location() -> Option<(f64, f64)> {
    imp::current_location().await
}

/// Candidate dynamic-library names/paths for pdfium-render fallback binding.
///
/// Callers should try `Pdfium::bind_to_system_library()` first, then these
/// platform-specific well-known locations.
pub fn pdfium_library_candidates() -> &'static [&'static str] {
    imp::pdfium_library_candidates()
}

/// Platform-specific implementation backing the v2 system permission catalog.
pub(crate) fn system_permissions_platform_name() -> &'static str {
    system_permissions::platform_name()
}

pub(crate) fn system_permissions_supported() -> bool {
    system_permissions::supported()
}

pub(crate) fn check_system_permission_item(id: &str) -> crate::permissions::SystemPermissionStatus {
    system_permissions::check_item(id)
}

pub(crate) fn request_system_permission_item(
    def: crate::permissions::PermissionDef,
) -> crate::permissions::SystemPermissionStatus {
    system_permissions::request_item(def)
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

/// Suppress the transient console window that Windows would otherwise flash
/// when spawning a console subprocess. No-op on Unix.
///
/// Apply this to every `std::process::Command` whose program exists on
/// Windows and that runs during normal operation — git probes, docker, ACP
/// backends, etc. — so the user never sees a `cmd`/`conhost` window blink.
/// On Windows it sets the `CREATE_NO_WINDOW` (0x0800_0000) creation flag;
/// output pipes still work, only the visible console is suppressed.
pub fn hide_console(cmd: &mut Command) {
    imp::hide_console(cmd);
}

/// `tokio::process::Command` variant of [`hide_console`], for async spawn sites.
pub fn hide_console_tokio(cmd: &mut tokio::process::Command) {
    imp::hide_console_tokio(cmd);
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

/// Atomically replace `path` with `bytes` (temp in the same dir → fsync → rename),
/// so a crash / power loss leaves either the old file intact or the new one
/// complete — never a truncated file. Creates parent dirs if missing.
///
/// Unlike [`write_secure_file`] (which forces 0600 for secrets), this is for user
/// documents — knowledge-base notes: it preserves the destination's existing
/// permissions when present, else a regular-file default (0644 on Unix). On
/// Windows it relies on NTFS DACL inheritance.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    imp::write_atomic(path, bytes)
}

/// Best-effort search for a Chrome / Chromium / Edge executable when the
/// user has not configured an explicit path. Mostly used as a safety net
/// in front of `chromiumoxide`'s own lookup, which is good but can miss
/// non-default install locations on Windows.
///
/// Unix: probes the `.app` bundle on macOS plus `which google-chrome` /
/// `chromium` on Linux. Windows: probes the standard install dirs.
pub fn find_chrome_executable() -> Option<PathBuf> {
    imp::find_chrome_executable()
}

/// Best-effort detection of whether the user has a Chrome / Chromium
/// process already running. Used by the "Take over user Chrome" path
/// in settings to surface a "we'll start a separate Chrome with its
/// own user-data-dir" confirmation prompt.
///
/// Always returns `false` when the underlying probe (`pgrep` /
/// `tasklist`) is unavailable or errors — callers treat this as a hint,
/// not a gate.
pub async fn chrome_already_running() -> bool {
    imp::chrome_already_running().await
}

/// Synchronous, best-effort detection of a discrete GPU. Used by the local
/// LLM recommender to pick a model size that fits in VRAM rather than RAM.
///
/// macOS: returns `None` — Apple Silicon and recent Intel Macs use unified
///   memory, so the recommender uses system RAM instead.
/// Linux: tries `nvidia-smi`; on failure parses `lspci` for any VGA/3D
///   adapter so the GUI can still render a name (VRAM falls back to `None`).
/// Windows: tries `nvidia-smi`, then PowerShell `Win32_VideoController`.
///   Note: `AdapterRAM` is a 32-bit field that wraps at 4 GiB on cards with
///   more memory; in that case we report 4096 MiB as a conservative floor.
pub fn detect_dedicated_gpu() -> Option<DetectedGpu> {
    if let Some(gpu) = nvidia_smi_query() {
        return Some(gpu);
    }
    imp::detect_dedicated_gpu_fallback()
}

fn nvidia_smi_query() -> Option<DetectedGpu> {
    let output = imp::run_hidden(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().next()?;
    let mut parts = line.splitn(2, ',');
    let name = parts.next()?.trim().to_string();
    let vram_mb = parts.next()?.trim().parse::<u64>().ok();
    Some(DetectedGpu { name, vram_mb })
}

/// Bare GPU descriptor returned by [`detect_dedicated_gpu`]. The `local_llm`
/// module wraps this into its own `GpuInfo` for the wire format.
#[derive(Debug, Clone)]
pub struct DetectedGpu {
    pub name: String,
    /// VRAM in MiB. `None` when the OS reports the adapter but not its memory.
    pub vram_mb: Option<u64>,
}

/// Whether an `io::Error` from `std::fs::rename` indicates that the source
/// and destination live on different filesystems (so the caller should fall
/// back to copy + remove).
///
/// Modern stable Rust (≥ 1.85) returns [`std::io::ErrorKind::CrossesDevices`];
/// older toolchains surface raw OS errors. We accept both for portability.
///
/// Unix: `EXDEV` (errno 18 on Linux + macOS + BSDs).
/// Windows: `ERROR_NOT_SAME_DEVICE` (raw_os_error 17).
pub fn is_cross_device_rename(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::CrossesDevices {
        return true;
    }
    imp::is_cross_device_rename_raw(err)
}

/// Atomically replace the executable at `target` with the one at `source`.
///
/// Used by [`crate::updater`] to swap in a freshly-downloaded `hope-agent`
/// binary without taking a stop-the-world window. The Unix path relies on
/// `rename(2)` mutating the directory entry (the running process keeps its
/// open inode); the Windows path renames the in-use binary aside then
/// moves the new one into place.
///
/// On success the caller is responsible for restarting the service so a
/// new process picks up the swapped-in binary. On failure `target` is
/// guaranteed to still point at a valid executable (either the original
/// or, on Windows, restored from the aside).
pub fn atomic_replace_binary(
    target: &std::path::Path,
    source: &std::path::Path,
) -> std::io::Result<()> {
    imp::atomic_replace_binary(target, source)
}

/// Outcome of [`redirect_updater_tmpdir_if_cross_volume`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdaterTmpdir {
    /// No action: not macOS, not a `.app` bundle, temp already on the bundle's
    /// volume, or already evaluated once this process.
    Unchanged,
    /// `$TMPDIR` was redirected onto the bundle's volume (path returned).
    Redirected(PathBuf),
    /// A cross-volume install was detected but a same-volume temp dir could not
    /// be staged (read-only mount such as a DMG, or an unwritable parent). The
    /// desktop self-update will likely still fail with `EXDEV` — there is
    /// nothing we can do from here; the caller should log a breadcrumb.
    CrossVolumeUnfixable,
}

/// macOS desktop-updater cross-device (`EXDEV`) workaround.
///
/// `tauri-plugin-updater` stages the new `.app` under `std::env::temp_dir()`
/// (`$TMPDIR`) and then `rename(2)`s both the current bundle out to a backup
/// and the new bundle into place (`updater.rs::install_inner`). When the app
/// runs from a different volume than `$TMPDIR` (external / secondary volume)
/// the very first rename returns `EXDEV` ("Cross-device link (os error 18)")
/// and the update aborts — the plugin treats any non-`PermissionDenied` rename
/// error as fatal (no AppleScript / copy fallback on `EXDEV`), and unlike its
/// Linux AppImage path it has no same-volume retry on macOS.
///
/// We pre-empt it: when the bundle's volume differs from the temp volume, point
/// `$TMPDIR` at a directory on the bundle's own volume so both of the plugin's
/// renames stay intra-volume.
///
/// Scope/why-global: both desktop update entry points drive the plugin's Rust
/// install command — the GUI "Check for Updates" menu path calls it straight
/// from JS ([`src/lib/desktopUpdater.ts`]) and the `app_update` tool calls it via
/// `update_bridge`. A Rust-side wrapper around one call site wouldn't cover the
/// other, so we set the process `$TMPDIR` once at startup. The redirect is a
/// no-op for the common case (app on the boot volume → same volume as `$TMPDIR`),
/// so the broader-temp-locality cost is paid only by the rare cross-volume user.
///
/// MUST run before any threads spawn — POSIX `setenv` is not thread-safe; a
/// process-once guard makes re-entry (the `run()` panic-restart loop) a no-op.
#[cfg(target_os = "macos")]
pub fn redirect_updater_tmpdir_if_cross_volume() -> UpdaterTmpdir {
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Evaluate at most once per process: `run()` can re-enter via the
    // panic-restart loop, and a second `set_var` would race threads spawned by
    // the prior attempt.
    static RAN: AtomicBool = AtomicBool::new(false);
    if RAN.swap(true, Ordering::SeqCst) {
        return UpdaterTmpdir::Unchanged;
    }

    let resolve = || -> Option<UpdaterTmpdir> {
        let exe = std::env::current_exe().ok()?;
        // Innermost `.app` ancestor is the bundle root.
        let app_root = exe
            .ancestors()
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))?;
        let install_parent = app_root.parent()?;
        // The plugin renames temp ⇄ the bundle itself, so the device that must
        // match is the bundle's own — not merely its parent's.
        let bundle_dev = std::fs::metadata(app_root).ok()?.dev();
        let tmp_dev = std::fs::metadata(std::env::temp_dir()).ok()?.dev();
        if tmp_dev == bundle_dev {
            // Temp already on the bundle's volume — the plugin's rename works.
            return Some(UpdaterTmpdir::Unchanged);
        }
        // Cross-volume: stage the updater's temp on the bundle's own volume.
        let updater_tmp = install_parent.join(".hope-agent-updater-tmp");
        if std::fs::create_dir_all(&updater_tmp).is_err() {
            // Read-only mount (e.g. a DMG) or unwritable parent — can't help.
            return Some(UpdaterTmpdir::CrossVolumeUnfixable);
        }
        // Verify the staged dir actually landed on the bundle's volume (guard
        // against firmlink / synthetic-mount edges where parent and the new dir
        // report different devices) — otherwise redirecting wouldn't fix the
        // rename and would relocate unrelated temp for nothing.
        match std::fs::metadata(&updater_tmp).ok().map(|m| m.dev()) {
            Some(dev) if dev == bundle_dev => {
                std::env::set_var("TMPDIR", &updater_tmp);
                Some(UpdaterTmpdir::Redirected(updater_tmp))
            }
            _ => Some(UpdaterTmpdir::CrossVolumeUnfixable),
        }
    };
    // Failure to even resolve the bundle/devices → safe no-op.
    resolve().unwrap_or(UpdaterTmpdir::Unchanged)
}

/// Non-macOS no-op: the desktop updater's `EXDEV` workaround is macOS-specific
/// (the Linux AppImage path already retries on the install volume; the Windows
/// installer is copied to temp and executed in place, never raw-renamed across
/// volumes).
#[cfg(not(target_os = "macos"))]
pub fn redirect_updater_tmpdir_if_cross_volume() -> UpdaterTmpdir {
    UpdaterTmpdir::Unchanged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("note.md");

        write_atomic(&target, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");

        // Overwrite — content fully replaced, not appended.
        write_atomic(&target, b"world!!").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "world!!");

        // The atomic temp must not survive a successful write.
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["note.md".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_new_file_gets_default_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("fresh.md");

        // A brand-new note (no existing file) must land at 0644, not the secret
        // 0600 — set_permissions makes this umask-independent.
        write_atomic(&target, b"x").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_preserves_existing_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("n.md");

        write_atomic(&target, b"a").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        // A subsequent atomic write keeps the destination's 0600, not the default.
        write_atomic(&target, b"bb").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn write_secure_file_still_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("secret.json");
        write_secure_file(&target, b"{}").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
        write_secure_file(&target, b"{\"k\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"k\":1}");
    }

    #[cfg(unix)]
    #[test]
    fn write_secure_file_forces_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cred.json");
        write_secure_file(&target, b"x").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
