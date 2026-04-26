use std::fs;
use std::io;
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
    None
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
    None
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
