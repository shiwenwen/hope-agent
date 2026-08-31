use std::fs;
use std::io::{self, Write};
use std::os::windows::fs::MetadataExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

use super::PathOwnershipSnapshot;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

pub(super) fn process_user_group() -> Option<(u32, u32)> {
    None
}

pub(super) fn path_owner_no_follow(_path: &Path) -> io::Result<(u32, u32)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "numeric file ownership is unavailable on Windows",
    ))
}

pub(super) fn path_ownership_snapshot_no_follow(_path: &Path) -> io::Result<PathOwnershipSnapshot> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "numeric file ownership is unavailable on Windows",
    ))
}

pub(super) fn path_has_security_capability_no_follow(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

pub(super) fn set_path_owner_from_snapshot_beneath(
    _root: &Path,
    _expected_root: PathOwnershipSnapshot,
    _relative: &Path,
    _expected: PathOwnershipSnapshot,
    _uid: u32,
    _gid: u32,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "numeric file ownership is unavailable on Windows",
    ))
}

fn final_path_for_handle(file: &fs::File) -> io::Result<String> {
    // FILE_NAME_NORMALIZED | VOLUME_NAME_DOS are both zero. Querying the
    // required size first avoids truncation; the returned name is derived from
    // the open handle, so later pathname swaps cannot change the decision.
    let required = unsafe {
        GetFinalPathNameByHandleW(file.as_raw_handle() as isize, std::ptr::null_mut(), 0, 0)
    };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; required as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as isize,
            buffer.as_mut_ptr(),
            buffer.len().min(u32::MAX as usize) as u32,
            0,
        )
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(if written == 0 {
            io::Error::last_os_error()
        } else {
            io::Error::new(io::ErrorKind::InvalidData, "final handle path changed size")
        });
    }
    String::from_utf16(&buffer[..written as usize]).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "final handle path is not UTF-16",
        )
    })
}

fn normalized_windows_path(path: &str) -> String {
    let path = path.replace('/', "\\");
    let path = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path
    };
    path.to_lowercase()
}

fn handle_path_matches(expected: &Path, actual: &str) -> bool {
    let expected = normalized_windows_path(&expected.as_os_str().to_string_lossy());
    let actual = normalized_windows_path(actual);
    expected.trim_end_matches('\\') == actual.trim_end_matches('\\')
}

fn open_direct_directory(path: &Path) -> io::Result<fs::File> {
    let handle = fs::OpenOptions::new()
        .read(true)
        // Holding every namespace component without FILE_SHARE_DELETE prevents
        // a direct-directory rename substitution just as O_DIRECTORY fds do
        // for the Unix openat walk. Read/write sharing remains compatible with
        // ordinary editors operating inside the directory.
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let attributes = handle.metadata()?.file_attributes();
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "beneath-open component is not a direct directory handle",
        ));
    }
    let final_path = final_path_for_handle(&handle)?;
    if !handle_path_matches(path, &final_path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "beneath-open directory changed after authorization",
        ));
    }
    Ok(handle)
}

fn locked_directory_chain(root: &Path) -> io::Result<Vec<fs::File>> {
    // `ancestors()` stops at the drive/UNC-share root. Open from that stable
    // namespace root downward and keep every handle alive until the candidate
    // is open; no component can be renamed out and replaced between checks.
    let mut paths = root
        .ancestors()
        .filter(|path| path.is_absolute())
        .collect::<Vec<_>>();
    paths.reverse();
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "beneath-open root has no absolute namespace anchor",
        ));
    }
    paths
        .into_iter()
        .map(open_direct_directory)
        .collect::<io::Result<Vec<_>>>()
}

fn relative_components(relative: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "beneath-open relative path has an invalid component",
            )),
        })
        .collect()
}

fn open_file_beneath_inner<F>(
    root: &Path,
    relative: &Path,
    after_root_locked: F,
) -> io::Result<fs::File>
where
    F: FnOnce(),
{
    let mut directory_handles = locked_directory_chain(root)?;
    after_root_locked();

    let components = relative_components(relative)?;
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "beneath-open relative path is empty",
        ));
    };
    let mut candidate_path = root.to_path_buf();
    for component in parent_components {
        candidate_path.push(component);
        directory_handles.push(open_direct_directory(&candidate_path)?);
    }
    candidate_path.push(file_name);

    let candidate = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&candidate_path)?;
    let attributes = candidate.metadata()?.file_attributes();
    if attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "beneath-open candidate is a directory or reparse point",
        ));
    }
    let candidate_final_path = final_path_for_handle(&candidate)?;
    if !handle_path_matches(&candidate_path, &candidate_final_path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "beneath-open candidate changed after authorization",
        ));
    }
    Ok(candidate)
}

pub(super) fn open_file_beneath(root: &Path, relative: &Path) -> io::Result<fs::File> {
    open_file_beneath_inner(root, relative, || {})
}

#[cfg(test)]
pub(super) fn open_file_beneath_with_root_hook<F>(
    root: &Path,
    relative: &Path,
    after_root_locked: F,
) -> io::Result<fs::File>
where
    F: FnOnce(),
{
    open_file_beneath_inner(root, relative, after_root_locked)
}

pub(super) fn remove_file_beneath(root: &Path, relative: &Path) -> io::Result<()> {
    let mut directory_handles = locked_directory_chain(root)?;
    let components = relative_components(relative)?;
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "beneath-remove relative path is empty",
        ));
    };
    let mut candidate_path = root.to_path_buf();
    for component in parent_components {
        candidate_path.push(component);
        directory_handles.push(open_direct_directory(&candidate_path)?);
    }
    candidate_path.push(file_name);
    // Every ancestor is still locked against delete/rename. DeleteFile removes
    // a final symlink/reparse directory entry itself rather than its target.
    fs::remove_file(candidate_path)
}

/// Convert a filesystem path to the absolute Win32 verbatim form accepted by
/// raw `*W` APIs. Rust's `std::fs` adds long-path handling internally, but a
/// direct `MoveFileExW` call must supply the `\\?\` / `\\?\UNC\` prefix itself.
fn to_win32_verbatim_wide(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    const BACKSLASH: u16 = b'\\' as u16;
    const FORWARD_SLASH: u16 = b'/' as u16;
    const QUESTION: u16 = b'?' as u16;
    const DOT: u16 = b'.' as u16;
    let absolute = std::path::absolute(path)?;
    let mut path_wide = absolute
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == FORWARD_SLASH {
                BACKSLASH
            } else {
                unit
            }
        })
        .collect::<Vec<_>>();
    let already_verbatim_or_device = path_wide
        .starts_with(&[BACKSLASH, BACKSLASH, QUESTION, BACKSLASH])
        || path_wide.starts_with(&[BACKSLASH, BACKSLASH, DOT, BACKSLASH]);
    if !already_verbatim_or_device {
        if path_wide.starts_with(&[BACKSLASH, BACKSLASH]) {
            let mut verbatim = r"\\?\UNC\".encode_utf16().collect::<Vec<_>>();
            verbatim.extend_from_slice(&path_wide[2..]);
            path_wide = verbatim;
        } else {
            let mut verbatim = r"\\?\".encode_utf16().collect::<Vec<_>>();
            verbatim.extend_from_slice(&path_wide);
            path_wide = verbatim;
        }
    }
    path_wide.push(0);
    Ok(path_wide)
}

pub(super) fn terminate_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub(super) fn send_graceful_stop(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub(super) fn detect_system_proxy() -> Option<String> {
    // Cache once per process: winreg access is cheap but callers
    // (provider/proxy, docker/proxy, …) would otherwise each re-read
    // on every client build.
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED.get_or_init(probe_system_proxy).clone()
}

pub(super) async fn current_location() -> Option<(f64, f64)> {
    crate::app_info!(
        "platform",
        "current_location",
        "OS precise location unavailable on Windows"
    );
    None
}

pub(super) fn pdfium_library_candidates() -> &'static [&'static str] {
    &["pdfium.dll"]
}

fn probe_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;

    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }

    let server: String = settings.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }

    // ProxyServer can be either a single "host:port" or a protocol-specific
    // list like "http=127.0.0.1:1082;https=127.0.0.1:1082;ftp=...".
    // Prefer https, fall back to http, fall back to the bare form.
    if server.contains('=') {
        let mut http: Option<&str> = None;
        let mut https: Option<&str> = None;
        for part in server.split(';') {
            let part = part.trim();
            if let Some(rest) = part.strip_prefix("https=") {
                https = Some(rest);
            } else if let Some(rest) = part.strip_prefix("http=") {
                http = Some(rest);
            }
        }
        let pick = https.or(http)?;
        return Some(format!("http://{}", pick));
    }

    Some(format!("http://{}", server))
}

pub(super) fn default_shell_command(cmdline: &str) -> Command {
    // `cmd /C` consumes the *rest* of the command line verbatim, so we use
    // `raw_arg` to avoid std's automatic quoting rewriting the user payload.
    let mut cmd = Command::new("cmd");
    cmd.raw_arg("/C").raw_arg(cmdline);
    // Never flash a `cmd` console window for shell-exec / tool commands.
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub(super) fn default_shell_command_tokio(cmdline: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.raw_arg("/C").raw_arg(cmdline);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub(super) fn hide_console(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

pub(super) fn hide_console_tokio(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

pub(super) fn wsl_command() -> Option<tokio::process::Command> {
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.creation_flags(CREATE_NO_WINDOW).kill_on_drop(true);
    Some(cmd)
}

async fn wsl_command_succeeds(args: &[&str]) -> bool {
    let Some(mut cmd) = wsl_command() else {
        return false;
    };
    cmd.args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    matches!(
        tokio::time::timeout(Duration::from_secs(5), cmd.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

pub(super) async fn wsl_status() -> super::WslStatus {
    let installed = wsl_command_succeeds(&["--status"]).await;
    if !installed {
        return super::WslStatus::default();
    }

    super::WslStatus {
        installed: true,
        distribution_installed: wsl_command_succeeds(&["--exec", "true"]).await,
    }
}

pub(super) async fn path_to_wsl(path: &Path) -> io::Result<Option<String>> {
    let Some(mut cmd) = wsl_command() else {
        return Ok(None);
    };
    let path = path.to_string_lossy();
    // `canonicalize()` commonly returns the extended-length prefix on
    // Windows, which `wslpath` does not accept. Convert it back to the normal
    // drive/UNC spelling before crossing the WSL boundary.
    let normalized = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.into_owned()
    };
    cmd.args(["--exec", "wslpath", "-a", "-u", &normalized]);
    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "wslpath timed out"))??;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "wslpath failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let converted = String::from_utf8(output.stdout)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let converted = converted.trim();
    if converted.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "wslpath returned an empty path",
        ));
    }

    // Resolve Linux-side symlinks before the caller applies its mount
    // blocklist. Windows canonicalization alone cannot safely classify WSL UNC
    // paths such as \\wsl.localhost\<distro>\var\run.
    let Some(mut canonicalizer) = wsl_command() else {
        return Ok(None);
    };
    canonicalizer.args(["--exec", "readlink", "-f", "--", converted]);
    let output = tokio::time::timeout(Duration::from_secs(5), canonicalizer.output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WSL readlink timed out"))??;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "WSL readlink failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let canonical = String::from_utf8(output.stdout)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let canonical = canonical.trim();
    if canonical.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WSL readlink returned an empty path",
        ));
    }
    Ok(Some(canonical.to_string()))
}

pub(super) fn find_chrome_executable() -> Option<PathBuf> {
    // Use env vars rather than hard-coding `C:\Program Files` so we
    // handle localized / ARM / alternate-drive installs. %LOCALAPPDATA%
    // covers per-user installs.
    let relatives: &[&str] = &[
        r"Google\Chrome\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"Chromium\Application\chrome.exe",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ];

    for env_var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Ok(root) = std::env::var(env_var) else {
            continue;
        };
        for rel in relatives {
            let full = PathBuf::from(&root).join(rel);
            if full.is_file() {
                return Some(full);
            }
        }
    }

    None
}

pub(super) async fn chrome_already_running() -> bool {
    // tasklist's CSV output is one line per matching process. `/FI` (filter)
    // accepts simple wildcards. We check the three common bin names.
    for name in ["chrome.exe", "msedge.exe", "chromium.exe"] {
        let filter = format!("IMAGENAME eq {name}");
        let output = match tokio::process::Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true)
            .output()
            .await
        {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // tasklist prints "INFO: No tasks ..." when nothing matches; CSV
        // lines start with a quote when matches exist.
        if stdout.trim_start().starts_with('"') {
            return true;
        }
    }
    false
}

pub(super) fn try_acquire_exclusive_lock(path: &Path) -> io::Result<Option<fs::File>> {
    use std::io::ErrorKind;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // FILE_SHARE_READ keeps the open kernel-exclusive against other
    // *writers* — any second process trying to open the same path with
    // write access gets ERROR_SHARING_VIOLATION, which is the only
    // exclusion we need (the holder body is the only thing written). We
    // can't use FILE_SHARE_NONE: it would also block read-only opens from
    // the same process, breaking `current_holder()`'s diagnostic read.
    // The handle is released automatically when the process exits or
    // panics, matching flock(LOCK_EX) semantics. FILE_FLAG_NO_INHERIT_HANDLE
    // keeps spawned children from holding the handle alive past their
    // parent's death.
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_NO_INHERIT_HANDLE: u32 = 0x0000_0080;
    let result = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_NO_INHERIT_HANDLE)
        .open(path);

    match result {
        Ok(file) => Ok(Some(file)),
        Err(e) => {
            // ERROR_SHARING_VIOLATION (32) — another process owns it.
            // PermissionDenied is what `io::Error::kind` maps it to.
            if matches!(e.kind(), ErrorKind::PermissionDenied) || e.raw_os_error() == Some(32) {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

/// Shared atomic-replace core: write `bytes` to a sibling temp (same dir), fsync,
/// then replace the target with one `MoveFileExW` operation. The temp is cleaned
/// up on a publication failure.
fn write_replace(path: &Path, bytes: &[u8]) -> super::SecureWriteOutcome {
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    let mut temp_created = false;
    let prepared = (|| -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        temp_created = true;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(error) = prepared {
        // `create_new` may fail because another same-process writer owns the
        // candidate name. Never delete a temp we did not create.
        if temp_created {
            let _ = fs::remove_file(&tmp);
        }
        return super::SecureWriteOutcome::NotPublished(error);
    }
    // NTFS inherits a DACL from the parent directory — `~/.hope-agent/`
    // lives under the user profile so by default only the owning user
    // and SYSTEM/Administrators can read. Hardening to an explicit DACL
    // (strip inherited ACEs, grant only the owner) is a future pass.
    // Publish in one Win32 rename operation. Removing the destination first
    // leaves a crash window where config.json is missing; MoveFileExW with
    // REPLACE_EXISTING keeps the old-or-new atomic replacement contract.
    if let Err(e) = publish_atomic_file(&tmp, path, true) {
        let _ = fs::remove_file(&tmp);
        return super::SecureWriteOutcome::NotPublished(e);
    }
    super::SecureWriteOutcome::Durable
}

pub(super) fn write_secure_file_outcome(path: &Path, bytes: &[u8]) -> super::SecureWriteOutcome {
    write_replace(path, bytes)
}

pub(super) fn ensure_credential_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other(
            "credential directory must not be a reparse point or non-directory",
        ));
    }
    Ok(())
}

/// Atomic write for user documents (knowledge-base notes). On Windows there is no
/// Unix-style mode to preserve — NTFS DACL inheritance applies — so this shares
/// the same temp + atomic-replace path as `write_secure_file`.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    match write_replace(path, bytes) {
        super::SecureWriteOutcome::Durable => Ok(()),
        super::SecureWriteOutcome::PublishedButNotDurable(error)
        | super::SecureWriteOutcome::NotPublished(error) => Err(error),
    }
}

pub(super) fn publish_dir_atomic(source: &Path, target: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staging source is not a directory",
        ));
    }
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target directory already exists",
        ));
    }
    fs::rename(source, target)
}

/// Atomically create a user document without replacing an existing path.
/// `hard_link` is the Windows no-clobber publication primitive: it either adds
/// the destination name for the fully fsynced temp file or fails because that
/// name already exists. `std::fs::rename` cannot be used here because Rust's
/// Windows implementation replaces an existing destination.
pub(super) fn write_atomic_create_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let published = fs::hard_link(&tmp, path).map_err(|error| {
        // Windows can surface either ERROR_FILE_EXISTS (80) or
        // ERROR_ALREADY_EXISTS (183), depending on the filesystem. Normalize
        // both so the cross-platform create-only contract stays stable.
        if matches!(error.raw_os_error(), Some(80) | Some(183)) {
            io::Error::new(io::ErrorKind::AlreadyExists, error)
        } else {
            error
        }
    });
    let _ = fs::remove_file(&tmp);
    published
}

pub(super) fn publish_atomic_file(source: &Path, target: &Path, overwrite: bool) -> io::Result<()> {
    if !overwrite {
        fs::hard_link(source, target)?;
        return fs::remove_file(source);
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    extern "system" {
        fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source = to_win32_verbatim_wide(source)?;
    let target = to_win32_verbatim_wide(target)?;
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn move_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    extern "system" {
        fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source = to_win32_verbatim_wide(source)?;
    let target = to_win32_verbatim_wide(target)?;
    // Omitting MOVEFILE_REPLACE_EXISTING preserves the no-clobber contract.
    let result = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn run_hidden(cmd: &str, args: &[&str]) -> Option<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

pub(super) fn detect_dedicated_gpu_fallback() -> Option<super::DetectedGpu> {
    // `Win32_VideoController.AdapterRAM` is a 32-bit field that wraps at
    // 4 GiB. We surface 4096 MiB as a conservative floor so the recommender
    // doesn't think a high-end GPU has tiny memory; the GUI surfaces the
    // raw name so users can sanity-check.
    let script = "Get-CimInstance Win32_VideoController | \
                  Where-Object { $_.AdapterRAM -gt 0 } | \
                  Sort-Object AdapterRAM -Descending | \
                  Select-Object -First 1 | \
                  ForEach-Object { \"$($_.Name)|$($_.AdapterRAM)\" }";
    let output = run_hidden(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(2, '|');
    let name = parts.next()?.trim().to_string();
    let bytes = parts.next()?.trim().parse::<u64>().ok()?;
    let mut vram_mb = bytes / (1024 * 1024);
    if (4090..=4100).contains(&vram_mb) {
        vram_mb = 4096;
    }
    Some(super::DetectedGpu {
        name,
        vram_mb: Some(vram_mb),
    })
}

pub(super) fn os_version_string() -> String {
    let long = sysinfo::System::long_os_version();
    let kernel = sysinfo::System::kernel_version();
    match (long, kernel) {
        (Some(name), Some(build)) => format!("{} ({})", name, build),
        (Some(name), None) => name,
        (None, Some(build)) => format!("Windows ({})", build),
        (None, None) => "Windows (unknown build)".to_string(),
    }
}

pub(super) fn is_cross_device_rename_raw(err: &std::io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE
    err.raw_os_error() == Some(17)
}

/// Atomically swap the file at `target` with `source`.
///
/// Windows holds an exclusive handle on a currently-executing image so you
/// cannot `DeleteFile` or overwrite it in place — but since Vista you _can_
/// rename it. We use that to do the swap without taking the service down
/// first: move the in-use image aside (`target` → `target.old`), move the
/// new image into the live path, and schedule the old image for deletion
/// at the next reboot. The caller then restarts the service so future
/// process launches pick up the new image; the still-running old process
/// keeps reading from its handle on `target.old`.
///
/// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`
/// gives us a single-syscall atomic publish for the new image (the rename
/// is observable to other processes either as "old" or "new", never as
/// "missing"), and `WRITE_THROUGH` forces the directory entry to disk
/// before returning so a crash mid-swap doesn't leave a phantom.
pub(super) fn atomic_replace_binary(target: &Path, source: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    fn to_wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x4;

    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
    }

    let target_w = to_wide(target);
    let source_w = to_wide(source);

    // Fast path: target isn't in use → straight overwrite.
    let direct = unsafe {
        MoveFileExW(
            source_w.as_ptr(),
            target_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if direct != 0 {
        return Ok(());
    }
    let direct_err = io::Error::last_os_error();
    // ERROR_SHARING_VIOLATION (32) / ERROR_ACCESS_DENIED (5) — likely
    // self-update with the binary still running. Fall through to the
    // rename-aside path.
    let raw = direct_err.raw_os_error().unwrap_or(0);
    if raw != 5 && raw != 32 {
        return Err(direct_err);
    }

    let aside = target.with_extension("old");
    let aside_w = to_wide(&aside);
    let renamed = unsafe {
        MoveFileExW(
            target_w.as_ptr(),
            aside_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if renamed == 0 {
        return Err(io::Error::last_os_error());
    }

    let published = unsafe {
        MoveFileExW(
            source_w.as_ptr(),
            target_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if published == 0 {
        let publish_err = io::Error::last_os_error();
        // Roll the aside back so we don't leave the user with a dangling
        // `.old` and no live binary at all.
        let _ = unsafe {
            MoveFileExW(
                aside_w.as_ptr(),
                target_w.as_ptr(),
                MOVEFILE_REPLACE_EXISTING,
            )
        };
        return Err(publish_err);
    }

    // Best-effort: tell the OS to delete the aside on next boot so the
    // disk doesn't fill with stale `.old` copies across many upgrades.
    // A NULL `lpNewFileName` is the documented "delete on reboot" signal.
    unsafe {
        MoveFileExW(
            aside_w.as_ptr(),
            std::ptr::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    };

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    fn from_nul_terminated_wide(mut wide: Vec<u16>) -> OsString {
        assert_eq!(wide.pop(), Some(0));
        OsString::from_wide(&wide)
    }

    #[test]
    fn move_file_paths_use_absolute_verbatim_form() {
        let disk = from_nul_terminated_wide(
            to_win32_verbatim_wide(Path::new(r"C:\hope-agent\config.json")).unwrap(),
        );
        assert_eq!(disk, OsString::from(r"\\?\C:\hope-agent\config.json"));

        let unc = from_nul_terminated_wide(
            to_win32_verbatim_wide(Path::new(r"\\server\share\config.json")).unwrap(),
        );
        assert_eq!(unc, OsString::from(r"\\?\UNC\server\share\config.json"));
    }
}
