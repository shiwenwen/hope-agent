use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn process_user_group() -> Option<(u32, u32)> {
    // SAFETY: These process identity queries take no pointers and have no
    // caller-side preconditions.
    Some(unsafe { (libc::geteuid(), libc::getegid()) })
}

pub(super) fn path_owner_no_follow(path: &Path) -> io::Result<(u32, u32)> {
    let metadata = fs::symlink_metadata(path)?;
    Ok((metadata.uid(), metadata.gid()))
}

pub(super) fn path_hard_link_count_no_follow(path: &Path) -> io::Result<u64> {
    Ok(fs::symlink_metadata(path)?.nlink())
}

pub(super) fn set_path_owner_no_follow(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: `path` is NUL-terminated and valid for the syscall. `lchown`
    // changes a raced symlink itself rather than following it outside the
    // already-authorized workspace tree.
    if unsafe { libc::lchown(path.as_ptr(), uid, gid) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn openat_component(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    directory: bool,
) -> io::Result<fs::File> {
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path component contains NUL"))?;
    let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    if directory {
        flags |= libc::O_DIRECTORY;
    } else {
        // A raced FIFO/device must not block before the caller can reject its
        // handle metadata as non-regular. O_NONBLOCK is inert for files.
        flags |= libc::O_NONBLOCK;
    }
    // SAFETY: `parent` is live for the syscall, `name` is NUL-terminated, and
    // no creation flag is present so the variadic mode argument is unnecessary.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful openat returns a new owned descriptor.
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

fn open_canonical_directory(root: &Path) -> io::Result<fs::File> {
    // Anchor traversal at the immutable filesystem root, then acquire every
    // canonical-root component through its parent descriptor. Renames/repoints
    // after a component is opened cannot redirect descendants.
    let mut directory = fs::File::open(Path::new("/"))?;
    for component in root.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = openat_component(&directory, name, true)?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "canonical beneath-open root has an invalid component",
                ))
            }
        }
    }
    Ok(directory)
}

fn normal_relative_components(relative: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    let components = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "beneath-open relative path has an invalid component",
            )),
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(components)
}

pub(super) fn open_file_beneath(root: &Path, relative: &Path) -> io::Result<fs::File> {
    let mut directory = open_canonical_directory(root)?;
    let components = normal_relative_components(relative)?;
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "beneath-open relative path is empty",
        ));
    };
    for component in parent_components {
        directory = openat_component(&directory, component, true)?;
    }
    openat_component(&directory, file_name, false)
}

pub(super) fn remove_file_beneath(root: &Path, relative: &Path) -> io::Result<()> {
    let mut directory = open_canonical_directory(root)?;
    let components = normal_relative_components(relative)?;
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "beneath-remove relative path is empty",
        ));
    };
    for component in parent_components {
        directory = openat_component(&directory, component, true)?;
    }
    let file_name = std::ffi::CString::new(file_name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path component contains NUL"))?;
    // SAFETY: `directory` is a live descriptor for the verified parent and
    // `file_name` is NUL-terminated. With flags=0, unlinkat removes the final
    // directory entry itself (including a symlink) rather than following it.
    if unsafe { libc::unlinkat(directory.as_raw_fd(), file_name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(super) fn terminate_process_tree(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

pub(super) fn send_graceful_stop(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

pub(super) fn detect_system_proxy() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED.get_or_init(probe_system_proxy).clone()
}

#[cfg(target_os = "macos")]
pub(super) async fn current_location() -> Option<(f64, f64)> {
    crate::weather_location_macos::system_locate().await
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn current_location() -> Option<(f64, f64)> {
    crate::app_info!(
        "platform",
        "current_location",
        "OS precise location unavailable on this Unix platform"
    );
    None
}

#[cfg(target_os = "macos")]
pub(super) fn pdfium_library_candidates() -> &'static [&'static str] {
    &[
        "/usr/local/lib/libpdfium.dylib",
        "/opt/homebrew/lib/libpdfium.dylib",
    ]
}

#[cfg(not(target_os = "macos"))]
pub(super) fn pdfium_library_candidates() -> &'static [&'static str] {
    &["/usr/lib/libpdfium.so", "/usr/local/lib/libpdfium.so"]
}

fn probe_system_proxy() -> Option<String> {
    env_proxy_url()
        .or_else(detect_macos_system_proxy)
        .or_else(detect_gnome_system_proxy)
        .or_else(detect_kde_system_proxy)
}

fn env_proxy_url() -> Option<String> {
    [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ]
    .iter()
    .find_map(|key| {
        let value = std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())?;
        normalize_proxy_url(&value)
    })
}

#[cfg(target_os = "macos")]
fn detect_macos_system_proxy() -> Option<String> {
    let output = run_hidden("scutil", &["--proxy"])?;
    if !output.status.success() {
        return None;
    }
    parse_scutil_proxy(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_system_proxy() -> Option<String> {
    None
}

#[cfg(any(target_os = "macos", test))]
fn parse_scutil_proxy(text: &str) -> Option<String> {
    for prefix in ["HTTPS", "HTTP"] {
        let enabled = text
            .lines()
            .find(|line| line.trim().starts_with(&format!("{prefix}Enable")))
            .and_then(|line| line.split(':').nth(1))
            .map(|value| value.trim() == "1")
            .unwrap_or(false);
        if !enabled {
            continue;
        }

        let host = text
            .lines()
            .find(|line| {
                let trimmed = line.trim();
                trimmed.starts_with(&format!("{prefix}Proxy"))
                    && !trimmed.contains("Enable")
                    && !trimmed.contains("Port")
            })
            .and_then(|line| line.split(':').nth(1))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let port = text
            .lines()
            .find(|line| line.trim().starts_with(&format!("{prefix}Port")))
            .and_then(|line| line.split(':').nth(1))
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let (Some(host), Some(port)) = (host, port) {
            return Some(format!("http://{host}:{port}"));
        }
    }
    None
}

fn detect_gnome_system_proxy() -> Option<String> {
    let mode = gsettings_string("org.gnome.system.proxy", "mode")?;
    if mode != "manual" {
        return None;
    }

    for schema in [
        "org.gnome.system.proxy.https",
        "org.gnome.system.proxy.http",
    ] {
        let Some(host) = gsettings_string(schema, "host") else {
            continue;
        };
        if host.is_empty() {
            continue;
        }
        let Some(port) = command_stdout("gsettings", &["get", schema, "port"])
            .and_then(|port| port.trim().parse::<u16>().ok())
            .filter(|port| *port > 0)
        else {
            continue;
        };
        return Some(format!("http://{host}:{port}"));
    }

    None
}

fn gsettings_string(schema: &str, key: &str) -> Option<String> {
    let raw = command_stdout("gsettings", &["get", schema, key])?;
    Some(unquote_gsettings_string(&raw))
}

fn unquote_gsettings_string(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        trimmed[1..trimmed.len() - 1]
            .replace("\\'", "'")
            .trim()
            .to_string()
    } else {
        trimmed.to_string()
    }
}

fn detect_kde_system_proxy() -> Option<String> {
    for binary in ["kreadconfig6", "kreadconfig5"] {
        let proxy_type = command_stdout(
            binary,
            &[
                "--file",
                "kioslaverc",
                "--group",
                "Proxy Settings",
                "--key",
                "ProxyType",
            ],
        );
        if matches!(proxy_type.as_deref().map(str::trim), Some(value) if value != "1") {
            continue;
        }

        for key in ["httpsProxy", "httpProxy"] {
            let Some(value) = command_stdout(
                binary,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    key,
                ],
            ) else {
                continue;
            };
            if let Some(url) = normalize_proxy_url(&value) {
                return Some(url);
            }
        }
    }
    None
}

fn command_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    let output = run_hidden(cmd, args)?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_proxy_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    if let (Some(host), Some(port), None) = (parts.next(), parts.next(), parts.next()) {
        if port.parse::<u16>().ok().filter(|port| *port > 0).is_some() {
            let host = host.trim_end_matches('/');
            if host.contains("://") {
                return Some(format!("{host}:{port}"));
            }
            return Some(format!("http://{host}:{port}"));
        }
    }

    if trimmed.contains("://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("http://{trimmed}"))
    }
}

pub(super) fn default_shell_command(cmdline: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(cmdline);
    cmd
}

pub(super) fn default_shell_command_tokio(cmdline: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(cmdline);
    cmd
}

// No console windows exist on Unix, so hiding one is a no-op. The `&mut`
// signature mirrors the Windows impl (which mutates `creation_flags`) so
// callers stay platform-agnostic.
#[allow(clippy::needless_pass_by_ref_mut)]
pub(super) fn hide_console(_cmd: &mut Command) {}

#[allow(clippy::needless_pass_by_ref_mut)]
pub(super) fn hide_console_tokio(_cmd: &mut tokio::process::Command) {}

pub(super) fn wsl_command() -> Option<tokio::process::Command> {
    None
}

pub(super) async fn wsl_status() -> super::WslStatus {
    super::WslStatus::default()
}

pub(super) async fn path_to_wsl(_path: &Path) -> io::Result<Option<String>> {
    Ok(None)
}

pub(super) fn find_chrome_executable() -> Option<PathBuf> {
    // macOS-specific .app bundles first; if present, prefer Chrome over
    // Chromium (matches the user's likely daily browser).
    #[cfg(target_os = "macos")]
    {
        for candidate in [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
    }
    // Linux + BSD: `which` the well-known binary names. Defensive — these
    // distros often install Chromium under different bin names.
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "brave-browser",
        "brave-browser-stable",
        "brave",
    ] {
        if let Ok(p) = which::which(name) {
            return Some(p);
        }
    }
    None
}

pub(super) async fn chrome_already_running() -> bool {
    // `pgrep -f` matches against the full command line. The pattern needs
    // to be broad enough to catch macOS's `Google Chrome` (with space) and
    // Linux's `chrome` / `chromium-browser`, but narrow enough that random
    // tools with "chrome" in their name don't trip it.
    let output = match tokio::process::Command::new("pgrep")
        .args([
            "-f",
            "Google Chrome|chrome-stable|chromium|chromium-browser|/chrome\\b",
        ])
        .kill_on_drop(true)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    // `pgrep` exits 0 when at least one match, 1 when none, >1 on error.
    output.status.success() && !output.stdout.is_empty()
}

pub(super) fn try_acquire_exclusive_lock(path: &Path) -> io::Result<Option<fs::File>> {
    use std::io::ErrorKind;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // O_CLOEXEC keeps fork()ed children (Guardian → app child) from
    // inheriting the lock-holding fd, which would prevent the child
    // from acquiring as Primary.
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;

    // SAFETY: file is a valid open fd for the duration of this block.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(Some(file))
    } else {
        let err = io::Error::last_os_error();
        // EWOULDBLOCK / EAGAIN means another process holds the lock —
        // not an error condition for the caller, just "be Secondary".
        if matches!(err.kind(), ErrorKind::WouldBlock) || err.raw_os_error() == Some(libc::EAGAIN) {
            Ok(None)
        } else {
            Err(err)
        }
    }
}

/// Shared atomic-replace core: write `bytes` to a sibling temp (same dir, so the
/// rename stays on one filesystem), fsync, chmod to `mode`, then rename over the
/// target. Errors before the rename are distinguishable from a failure to fsync
/// the parent directory after the new target is already visible.
fn write_replace_with_parent_sync<F>(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    sync_parent: F,
) -> super::SecureWriteOutcome
where
    F: FnOnce(&Path) -> io::Result<()>,
{
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
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&tmp)?;
        temp_created = true;
        f.write_all(bytes)?;
        f.sync_all()?;
        // Defensive: in case the OS umask altered the initial mode.
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
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
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return super::SecureWriteOutcome::NotPublished(e);
    }
    // Persist the directory entry as well as the file contents. Without this,
    // power loss can discard a rename that was already reported as successful.
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if let Err(error) = sync_parent(parent) {
            return super::SecureWriteOutcome::PublishedButNotDurable(error);
        }
    }
    super::SecureWriteOutcome::Durable
}

fn write_replace(path: &Path, bytes: &[u8], mode: u32) -> super::SecureWriteOutcome {
    write_replace_with_parent_sync(path, bytes, mode, |parent| {
        fs::File::open(parent)?.sync_all()
    })
}

pub(super) fn write_secure_file_outcome(path: &Path, bytes: &[u8]) -> super::SecureWriteOutcome {
    write_replace(path, bytes, 0o600)
}

#[cfg(test)]
pub(super) fn write_secure_file_outcome_with_parent_sync<F>(
    path: &Path,
    bytes: &[u8],
    sync_parent: F,
) -> super::SecureWriteOutcome
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    write_replace_with_parent_sync(path, bytes, 0o600, sync_parent)
}

/// Atomic write for user documents (knowledge-base notes): preserves the
/// destination's existing permissions when present, else a regular-file default
/// (0644) — unlike `write_secure_file`, which forces 0600 for secrets.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mode = fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0o644);
    match write_replace(path, bytes, mode) {
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
    rename_noreplace(source, target)?;
    if let Some(parent) = target.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    // SAFETY: both pointers reference live NUL-terminated path buffers for the
    // duration of the call. RENAME_EXCL makes publication no-clobber atomically.
    let result = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    // SAFETY: both pointers reference live NUL-terminated path buffers for the
    // duration of the syscall and both directory fds are the process cwd.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE as _,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target path already exists",
        ));
    }
    fs::rename(source, target)
}

pub(super) fn move_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    rename_noreplace(source, target)?;
    let target_parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = target_parent {
        fs::File::open(parent)?.sync_all()?;
    }
    let source_parent = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    match source_parent {
        Some(parent) if Some(parent) != target_parent => fs::File::open(parent)?.sync_all()?,
        _ => {}
    }
    Ok(())
}

/// Atomically create a user document without replacing an existing path.
/// `hard_link` is the Unix no-clobber publication primitive: it either adds the
/// destination name for the fully fsynced temp inode or fails with AlreadyExists.
pub(super) fn write_atomic_create_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mode = 0o644;
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    let published = fs::hard_link(&tmp, path);
    let _ = fs::remove_file(&tmp);
    published
}

pub(super) fn publish_atomic_file(source: &Path, target: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        fs::rename(source, target)?;
    } else {
        fs::hard_link(source, target)?;
        fs::remove_file(source)?;
    }
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn run_hidden(cmd: &str, args: &[&str]) -> Option<std::process::Output> {
    Command::new(cmd).args(args).output().ok()
}

#[cfg(target_os = "macos")]
pub(super) fn detect_dedicated_gpu_fallback() -> Option<super::DetectedGpu> {
    // Unified memory architecture — let the caller fall back to system RAM.
    None
}

#[cfg(not(target_os = "macos"))]
pub(super) fn detect_dedicated_gpu_fallback() -> Option<super::DetectedGpu> {
    // lspci tells us the adapter name even when no NVIDIA driver is
    // installed. We can't read VRAM from this path.
    let output = run_hidden("lspci", &["-mm"])?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let lowered = line.to_lowercase();
        if lowered.contains("vga compatible controller") || lowered.contains("3d controller") {
            if let Some(name) = parse_lspci_name(line) {
                return Some(super::DetectedGpu {
                    name,
                    vram_mb: None,
                });
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn parse_lspci_name(line: &str) -> Option<String> {
    // `lspci -mm` quotes vendor/device fields, e.g.
    //   01:00.0 "VGA compatible controller" "NVIDIA Corporation" "GA106 [RTX 3060]"
    let mut chunks = line.split('"').filter(|c| !c.trim().is_empty());
    let _slot = chunks.next()?;
    let _class = chunks.next()?;
    let vendor = chunks.next()?.trim();
    let device = chunks.next().map(|s| s.trim()).unwrap_or("");
    if device.is_empty() {
        Some(vendor.to_string())
    } else {
        Some(format!("{vendor} {device}"))
    }
}

pub(super) fn os_version_string() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sw_vers").arg("-productVersion").output() {
            if output.status.success() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return format!("macOS {}", trimmed);
                    }
                }
            }
        }
    }

    sysinfo::System::long_os_version().unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn is_cross_device_rename_raw(err: &std::io::Error) -> bool {
    // EXDEV — same value on Linux, macOS, and the BSDs.
    const EXDEV: i32 = 18;
    err.raw_os_error() == Some(EXDEV)
}

/// Atomically swap the file at `target` with `source`.
///
/// Unix is forgiving: `rename(2)` mutates the directory entry, not the
/// underlying inode, so a process holding `target` open keeps executing the
/// old image until it exits — the new image becomes visible to future
/// `exec(2)` calls (which is what `systemctl --user restart` / `launchctl
/// kickstart -k` will do moments later).
///
/// Sets mode `0755` on the new file before the rename so the swapped-in
/// binary is immediately executable even when callers extracted it without
/// preserving permissions (`zip` on Windows, `flate2::GzDecoder` on a
/// shared filesystem mount, etc.).
///
/// Cross-device fallback: when `source` and `target` live on different
/// filesystems `rename` returns `EXDEV`. We copy to a sibling tempfile in
/// the target's directory, `fsync`, then rename — same atomicity guarantee
/// for the swap itself.
pub(super) fn atomic_replace_binary(target: &Path, source: &Path) -> io::Result<()> {
    fs::set_permissions(source, fs::Permissions::from_mode(0o755))?;
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(e) if super::is_cross_device_rename(&e) => {
            let parent = target.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "target binary path has no parent directory",
                )
            })?;
            let tmp = parent.join(format!(".hope-agent.swap.{}", std::process::id()));
            let _ = fs::remove_file(&tmp);
            fs::copy(source, &tmp)?;
            // fsync the new contents so the rename is durable across power
            // loss — without this we could rename a half-written file in.
            let f = fs::OpenOptions::new().read(true).open(&tmp)?;
            f.sync_all()?;
            drop(f);
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
            if let Err(e) = fs::rename(&tmp, target) {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            let _ = fs::remove_file(source);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scutil_proxy_prefer_https() {
        let text = r#"
<dictionary> {
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 1082
  HTTPSProxy : 10.0.0.2
}
"#;

        assert_eq!(
            parse_scutil_proxy(text).as_deref(),
            Some("http://10.0.0.2:1082")
        );
    }

    #[test]
    fn parses_scutil_proxy_fallback_to_http() {
        let text = r#"
HTTPEnable : 1
HTTPProxy : localhost
HTTPPort : 7890
HTTPSEnable : 0
"#;

        assert_eq!(
            parse_scutil_proxy(text).as_deref(),
            Some("http://localhost:7890")
        );
    }

    #[test]
    fn unquotes_gsettings_strings() {
        assert_eq!(unquote_gsettings_string("'manual'"), "manual");
        assert_eq!(unquote_gsettings_string("'127.0.0.1'"), "127.0.0.1");
        assert_eq!(unquote_gsettings_string("  ''  "), "");
    }

    #[test]
    fn normalizes_kde_proxy_values() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:8080").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            normalize_proxy_url("http://127.0.0.1 8080").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
    }
}
