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

use std::path::{Path, PathBuf};
use std::process::Command;

/// Stable identity captured before a numeric ownership handoff.
///
/// Callers must pass this snapshot back to
/// [`set_path_owner_from_snapshot_beneath`] so the ownership mutation is
/// applied to the same inode through a descriptor-relative, no-follow walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathOwnershipSnapshot {
    pub uid: u32,
    pub gid: u32,
    pub device: u64,
    pub inode: u64,
    pub hard_link_count: u64,
    pub is_directory: bool,
}

// `pub`（原为 `pub(crate)`）：ha-core 的 `app_init::spawn_keep_awake_listener`
// 跨 crate 调用 `keep_awake::apply`，搬进 ha-base 后 crate 级可见性不再够用。
pub mod keep_awake;
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
/// Used by `ha_browser::browser::singleton_lock` to detect stale SingletonLock
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

/// Numeric identity of the current process on Unix. Windows returns `None`;
/// callers use that to keep platform-specific ownership policy out of business
/// crates.
pub fn process_user_group() -> Option<(u32, u32)> {
    imp::process_user_group()
}

/// Read an entry's numeric owner without following a final symlink.
pub fn path_owner_no_follow(path: &Path) -> std::io::Result<(u32, u32)> {
    imp::path_owner_no_follow(path)
}

/// Capture the owner, inode identity, type, and hard-link count without
/// following a final symlink.
pub fn path_ownership_snapshot_no_follow(path: &Path) -> std::io::Result<PathOwnershipSnapshot> {
    imp::path_ownership_snapshot_no_follow(path)
}

/// Change an entry's numeric owner through a descriptor-relative walk rooted
/// at `root`, after proving that the opened inode still matches `expected`.
/// Regular files must remain singly linked at the mutation boundary.
pub fn set_path_owner_from_snapshot_beneath(
    root: &Path,
    expected_root: PathOwnershipSnapshot,
    relative: &Path,
    expected: PathOwnershipSnapshot,
    uid: u32,
    gid: u32,
) -> std::io::Result<()> {
    imp::set_path_owner_from_snapshot_beneath(root, expected_root, relative, expected, uid, gid)
}

/// Prevent same-UID processes from attaching to or dumping this process on
/// Linux. Real-model evaluation servers keep Provider credentials in memory
/// while deliberately executing model-selected tools, so this hardening is a
/// required boundary rather than a best-effort diagnostic setting.
pub fn prevent_process_dumping() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn validate_beneath_path(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> std::io::Result<()> {
    if !root.is_absolute()
        || relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "beneath-open requires an absolute root and a non-empty normal relative path",
        ));
    }
    Ok(())
}

/// Open a regular-file candidate through an already-authorized directory
/// namespace without allowing `..`, absolute paths, or symlink/reparse escapes.
/// The returned handle is the exact object the caller must read; callers must
/// not reopen `root.join(relative)` after this succeeds.
pub fn open_file_beneath(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> std::io::Result<std::fs::File> {
    validate_beneath_path(root, relative)?;
    let file = imp::open_file_beneath(root, relative)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "beneath-open candidate is not a regular file",
        ));
    }
    Ok(file)
}

/// Remove one exact directory entry beneath an authorized root without
/// following a symlink/reparse-point ancestor. The final entry itself is
/// unlinked, never dereferenced. This is the destructive counterpart to
/// [`open_file_beneath`] and is intended for backend-owned cleanup ledgers.
pub fn remove_file_beneath(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> std::io::Result<()> {
    validate_beneath_path(root, relative)?;
    imp::remove_file_beneath(root, relative)
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

/// Raw single-permission preflight for the `--tcc-probe` process mode.
/// Synchronous and spawn-free — see `system_permissions::raw_probe`.
pub(crate) fn system_permission_raw_probe(id: &str) -> Option<bool> {
    system_permissions::raw_probe(id)
}

/// Whether this platform/build can reset the OS permission record for `id`
/// (macOS `tccutil`, packaged app only).
pub(crate) fn system_permission_supports_reset(id: &str) -> bool {
    system_permissions::supports_reset(id)
}

/// Reset the OS permission record for `id` so the OS prompts again.
pub(crate) fn reset_system_permission_item(id: &str) -> Result<(), String> {
    system_permissions::reset_item(id)
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

/// Windows Subsystem for Linux availability relevant to command execution.
///
/// `installed` means the WSL runtime itself answered its status probe, while
/// `distribution_installed` additionally means the default Linux distribution
/// can execute commands. Non-Windows platforms always return `false` for both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WslStatus {
    pub installed: bool,
    pub distribution_installed: bool,
}

/// Probe WSL without opening a console window.
pub async fn wsl_status() -> WslStatus {
    imp::wsl_status().await
}

/// Build a hidden async `wsl.exe` command on Windows.
///
/// Returns `None` on non-Windows platforms so shared callers can keep their
/// platform branching in this module.
pub fn wsl_command() -> Option<tokio::process::Command> {
    imp::wsl_command()
}

/// Convert a host path into the default WSL distribution's Linux path.
/// Returns `None` where WSL is not available.
pub async fn path_to_wsl(path: &Path) -> std::io::Result<Option<String>> {
    imp::path_to_wsl(path).await
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
/// same directory, `fsync`s, sets 0600 (Unix) / uses the inherited NTFS
/// DACL (Windows), then renames over the target path. Callers should
/// use this for anything that must not be readable by other local users.
///
/// Unix: `chmod 0600` after write so the file inherits the stricter
/// permission even if the parent dir is group-writable.
/// Windows: writes the file and relies on NTFS DACL inheritance — a
/// stronger ACL pass can be layered on later without API change.
pub fn write_secure_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    write_secure_file_outcome(path, bytes).into_legacy_result()
}

/// Publication result for a credential-grade atomic file write.
///
/// A successful atomic rename can become visible before syncing its parent
/// directory fails. Callers that mirror the file into process state must treat
/// both published variants as a logical commit while retaining the durability
/// warning for diagnostics.
#[derive(Debug)]
#[must_use = "the secure write publication outcome must be handled"]
pub enum SecureWriteOutcome {
    /// The replacement was published and its durability barrier completed.
    Durable,
    /// The replacement was published, but the final durability barrier failed.
    PublishedButNotDurable(std::io::Error),
    /// The replacement was not published; an existing target remains unchanged.
    NotPublished(std::io::Error),
}

impl SecureWriteOutcome {
    fn into_legacy_result(self) -> std::io::Result<()> {
        match self {
            Self::Durable => Ok(()),
            Self::PublishedButNotDurable(error) | Self::NotPublished(error) => Err(error),
        }
    }
}

/// Atomically write a credential-bearing file and report whether publication
/// happened independently from whether its final durability barrier succeeded.
///
/// Prefer this over [`write_secure_file`] when the caller also maintains an
/// in-memory snapshot or must run post-commit side effects. On Unix, a parent
/// directory open/`fsync` failure after `rename(2)` is reported as
/// [`SecureWriteOutcome::PublishedButNotDurable`]. Windows uses a write-through
/// atomic replacement, so it reports either [`SecureWriteOutcome::Durable`] or
/// [`SecureWriteOutcome::NotPublished`].
pub fn write_secure_file_outcome(path: &std::path::Path, bytes: &[u8]) -> SecureWriteOutcome {
    imp::write_secure_file_outcome(path, bytes)
}

/// Stream `source` into a credential-grade sibling temp file, `fsync` it, then
/// atomically replace `target`. This keeps memory usage constant for unbounded
/// inputs while preserving the old-or-new publication contract.
pub fn copy_secure_file_atomic(
    source: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<u64> {
    if source == target {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source and target must differ",
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = target.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    let mut temp_created = false;
    let result = (|| {
        let mut input = std::fs::File::open(source)?;
        #[cfg(unix)]
        let mut output = {
            use std::os::unix::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temp)?;
            temp_created = true;
            file
        };
        #[cfg(not(unix))]
        let mut output = {
            let file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            temp_created = true;
            file
        };
        let copied = std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
        }
        imp::publish_atomic_file(&temp, target, true)?;
        Ok(copied)
    })();
    if result.is_err() && temp_created {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Move a fully-written file to a new path without replacing an existing
/// destination. The directory entry change is atomic; on Unix both source and
/// destination parent directories are fsynced before success is reported.
///
/// This is intended for cross-directory security transitions such as moving a
/// malformed credential-bearing backup into quarantine. The paths must reside
/// on the same filesystem.
pub fn move_file_atomic(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    if source == target {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source and target must differ",
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    imp::move_file_atomic(source, target)
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

/// Atomically create `path` with `bytes`, failing with `AlreadyExists` if a
/// competing writer published the destination first.
pub fn write_atomic_create_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    imp::write_atomic_create_new(path, bytes)
}

/// Publish a fully-written sibling staging file at `target` without buffering
/// it again. `overwrite=false` fails with `AlreadyExists`; `overwrite=true`
/// atomically replaces the existing directory entry when the OS supports it.
pub fn publish_atomic_file(
    source: &std::path::Path,
    target: &std::path::Path,
    overwrite: bool,
) -> std::io::Result<()> {
    imp::publish_atomic_file(source, target, overwrite)
}

/// Publish a fully prepared sibling directory without exposing a partial
/// package.  Both paths must share the same parent and `target` must not exist.
/// The caller is responsible for fsyncing files inside `source` first.
pub fn publish_dir_atomic(
    source: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    if source.parent() != target.parent() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory publish requires sibling paths",
        ));
    }
    imp::publish_dir_atomic(source, target)
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
/// Used by `ha_updater` to swap in a freshly-downloaded `hope-agent`
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
    /// No action: not macOS, not a `.app` bundle, or temp already on the
    /// bundle's volume.
    Unchanged,
    /// The `tempfile` default temp dir was overridden onto the bundle's volume
    /// (path returned).
    Redirected(PathBuf),
    /// A cross-volume install was detected but a same-volume temp dir could not
    /// be staged (read-only mount such as a DMG, or an unwritable parent). The
    /// desktop self-update will likely still fail with `EXDEV` — there is
    /// nothing we can do from here; the caller should log a breadcrumb.
    CrossVolumeUnfixable,
}

/// macOS desktop-updater cross-device (`EXDEV`) workaround.
///
/// `tauri-plugin-updater` stages the new `.app` under the default temp dir via
/// `tempfile::Builder` and then `rename(2)`s both the current bundle out to a
/// backup and the new bundle into place (`updater.rs::install_inner`). When the
/// app runs from a different volume than the temp dir (external / secondary
/// volume) the very first rename returns `EXDEV` ("Cross-device link (os error
/// 18)") and the update aborts — the plugin treats any non-`PermissionDenied`
/// rename error as fatal (no AppleScript / copy fallback on `EXDEV`), and unlike
/// its Linux AppImage path it has no same-volume retry on macOS.
///
/// We pre-empt it: when the bundle's volume differs from the temp volume, point
/// the `tempfile` crate's default temp dir at a directory on the bundle's own
/// volume (via [`tempfile::env::override_temp_dir`]) so both of the plugin's
/// renames stay intra-volume.
///
/// Scope: this overrides only the `tempfile` crate's in-process default — it
/// does NOT mutate `$TMPDIR`, so spawned child processes (exec / hooks / MCP,
/// which inherit and even whitelist `$TMPDIR`) keep the per-user system temp.
/// It's set at startup (rather than wrapped around a single update call)
/// because both desktop update entry points reach the plugin independently —
/// the GUI "Check for Updates" menu path from JS ([`src/lib/desktopUpdater.ts`])
/// and the `app_update` tool via `update_bridge`. The override is a no-op for
/// the common case (app on the boot volume → same volume as the temp dir), so
/// the (now in-process-only) temp-locality cost is paid solely by the rare
/// cross-volume user. `override_temp_dir` is set-once and thread-safe, so
/// `run()` panic-restart re-entry is harmless.
#[cfg(target_os = "macos")]
pub fn redirect_updater_tmpdir_if_cross_volume() -> UpdaterTmpdir {
    use std::os::unix::fs::MetadataExt;

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
        // Compare against the OS default temp (`std::env::temp_dir`), which is
        // what `tempfile` falls back to when no override is set.
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
                // Process-local override for the `tempfile` crate only (the
                // plugin stages via `tempfile::Builder`, which honors it). Does
                // NOT touch `$TMPDIR`, so child processes are unaffected.
                // Set-once: a later call (panic-restart re-entry) returns Err
                // and is ignored.
                let _ = tempfile::env::override_temp_dir(&updater_tmp);
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
    fn secure_write_outcome_reports_durable_publication() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");

        let outcome = write_secure_file_outcome(&target, b"published");

        assert!(matches!(outcome, SecureWriteOutcome::Durable));
        assert_eq!(std::fs::read(&target).unwrap(), b"published");
    }

    #[test]
    fn secure_write_outcome_reports_prepublication_failure() {
        let dir = tempfile::tempdir().unwrap();
        let blocked_parent = dir.path().join("not-a-directory");
        std::fs::write(&blocked_parent, b"keep").unwrap();
        let target = blocked_parent.join("config.json");

        let outcome = write_secure_file_outcome(&target, b"unpublished");

        assert!(matches!(outcome, SecureWriteOutcome::NotPublished(_)));
        assert_eq!(std::fs::read(&blocked_parent).unwrap(), b"keep");
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn secure_write_outcome_distinguishes_postpublication_sync_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");

        let outcome =
            imp::write_secure_file_outcome_with_parent_sync(&target, b"published", |_| {
                Err(std::io::Error::other("injected parent sync failure"))
            });

        match outcome {
            SecureWriteOutcome::PublishedButNotDurable(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::Other);
            }
            other => panic!("unexpected secure write outcome: {other:?}"),
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"published");
    }

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

    #[test]
    fn write_atomic_create_new_reports_existing_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("note.md");

        write_atomic_create_new(&target, b"first").unwrap();
        let error = write_atomic_create_new(&target, b"second").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
    }

    #[test]
    fn open_file_beneath_returns_the_authorized_handle_and_rejects_parent_paths() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/note.txt"), b"authorized").unwrap();

        let mut file = open_file_beneath(&root, Path::new("nested/note.txt")).unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        assert_eq!(content, "authorized");
        assert_eq!(
            open_file_beneath(&root, Path::new("../outside.txt"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_file_beneath_rejects_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
        symlink(&outside, root.join("redirect")).unwrap();
        let root = root.canonicalize().unwrap();

        assert!(open_file_beneath(&root, Path::new("redirect/secret.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn open_file_beneath_rejects_authorized_root_replacement() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let displaced = dir.path().join("displaced-root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
        let authorized_root = root.canonicalize().unwrap();
        std::fs::rename(&root, &displaced).unwrap();
        symlink(&outside, &root).unwrap();

        assert!(open_file_beneath(&authorized_root, Path::new("secret.txt")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn open_file_beneath_rejects_authorized_root_reparse_replacement() {
        use std::os::windows::fs::symlink_dir;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let displaced = dir.path().join("displaced-root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
        let authorized_root = root.canonicalize().unwrap();
        std::fs::rename(&root, &displaced).unwrap();
        if let Err(error) = symlink_dir(&outside, &root) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return; // Windows developer-mode policy unavailable on this host.
            }
            panic!("create directory reparse point: {error}");
        }

        assert!(open_file_beneath(&authorized_root, Path::new("secret.txt")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn open_file_beneath_locks_out_direct_directory_rename_substitution() {
        use std::io::Read;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        let root = parent.join("root");
        let displaced = parent.join("displaced-root");
        let replacement = parent.join("replacement-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(root.join("selected.txt"), b"authorized").unwrap();
        std::fs::write(replacement.join("selected.txt"), b"outside").unwrap();
        let root = root.canonicalize().unwrap();
        let rename_result = Arc::new(Mutex::new(None));
        let observed = rename_result.clone();

        let mut file =
            imp::open_file_beneath_with_root_hook(&root, Path::new("selected.txt"), || {
                let result = std::fs::rename(&root, &displaced);
                *observed.lock().unwrap() = Some(result);
                if displaced.exists() {
                    std::fs::rename(&replacement, &root).unwrap();
                }
            })
            .unwrap();
        assert!(rename_result
            .lock()
            .unwrap()
            .as_ref()
            .expect("rename attempted")
            .is_err());
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        assert_eq!(content, "authorized");
    }

    #[cfg(unix)]
    #[test]
    fn open_file_beneath_rejects_fifo_without_blocking() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let fifo = root.join("pipe");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `fifo_c` is a live NUL-terminated path and mode is valid.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);

        assert!(open_file_beneath(&root, Path::new("pipe")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn remove_file_beneath_never_follows_a_symlinked_parent() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("owned.txt");
        std::fs::write(&outside_file, b"keep").unwrap();
        symlink(&outside, root.join("redirect")).unwrap();
        let root = root.canonicalize().unwrap();

        assert!(remove_file_beneath(&root, Path::new("redirect/owned.txt")).is_err());
        assert_eq!(std::fs::read(outside_file).unwrap(), b"keep");
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
        let replacement = r#"{"label":"服务器（测试）"}"#;
        write_secure_file(&target, replacement.as_bytes()).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        assert_eq!(bytes, replacement.as_bytes());
        assert!(!bytes.starts_with(b"\xef\xbb\xbf"));

        // The fully-written temp must be consumed by the atomic replacement.
        let entries = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec![target.file_name().unwrap().to_os_string()]);
    }

    #[test]
    fn copy_secure_file_atomic_streams_exact_bytes_without_temp_leaks() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let target = dir.path().join("target.json");
        let bytes = vec![0x5a; 2 * 1024 * 1024 + 3];
        std::fs::write(&source, &bytes).unwrap();

        let copied = copy_secure_file_atomic(&source, &target).unwrap();

        assert_eq!(copied, bytes.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        let entries = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn move_file_atomic_moves_without_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::create_dir_all(&target_dir).unwrap();
        let source = source_dir.join("config.json");
        let target = target_dir.join("config.json");
        std::fs::write(&source, b"secret").unwrap();

        move_file_atomic(&source, &target).unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"secret");

        let replacement = source_dir.join("replacement.json");
        std::fs::write(&replacement, b"replacement").unwrap();
        let error = move_file_atomic(&replacement, &target).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&replacement).unwrap(), b"replacement");
        assert_eq!(std::fs::read(&target).unwrap(), b"secret");
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
