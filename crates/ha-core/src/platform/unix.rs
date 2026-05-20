use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

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

pub(super) fn find_chrome_executable() -> Option<PathBuf> {
    // macOS-specific .app bundles first; if present, prefer Chrome over
    // Chromium (matches the user's likely daily browser).
    #[cfg(target_os = "macos")]
    {
        for candidate in [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
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

pub(super) fn write_secure_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // Defensive: in case the OS umask altered the initial mode.
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)?;
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
