use std::path::PathBuf;
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
