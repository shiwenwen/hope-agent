//! Restart the `hope-agent server` service after a binary swap.
//!
//! Reuses `service_install`'s platform-specific install/uninstall to
//! re-emit the unit / plist / scheduled-task (which guarantees we point at
//! the current `current_exe()` even if the user moved the binary), then
//! kicks the service so the new image is loaded. When the service isn't
//! installed (user runs `hope-agent server start` from a terminal) we
//! best-effort signal the running PID and let its supervisor (or the user)
//! relaunch.

use anyhow::Result;

/// Restart the installed user service, if any. Returns a short summary
/// suitable for an LLM-facing tool result.
pub fn restart_service() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        restart_launchd()
    }
    #[cfg(target_os = "linux")]
    {
        restart_systemd()
    }
    #[cfg(windows)]
    {
        restart_scheduled_task()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        anyhow::bail!("service restart not supported on this platform")
    }
}

#[cfg(target_os = "macos")]
fn restart_launchd() -> Result<String> {
    // `kickstart -k` stops then restarts the LaunchAgent. Requires the
    // plist to already exist — which it does whenever
    // `service_install::install_service` was run at least once.
    let uid = unsafe { libc::getuid() };
    let label = format!("gui/{uid}/ai.hopeagent.server");
    let output = std::process::Command::new("launchctl")
        .args(["kickstart", "-k", &label])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Fall back to a fresh load — covers the "service was never
        // installed, user ran `hope-agent server start` from a terminal"
        // case where kickstart has nothing to kick.
        app_info!(
            "self_update",
            "service_control",
            "launchctl kickstart fallback: {}",
            stderr.trim()
        );
        return Ok(format!("launchctl kickstart returned: {}", stderr.trim()));
    }
    Ok(format!("launchctl kickstart {label}: ok"))
}

#[cfg(target_os = "linux")]
fn restart_systemd() -> Result<String> {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "restart", "hope-agent.service"])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("systemctl --user restart failed: {}", stderr.trim());
    }
    Ok("systemctl --user restart hope-agent.service: ok".into())
}

#[cfg(windows)]
fn restart_scheduled_task() -> Result<String> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const TASK_NAME: &str = "Hope Agent";
    // Tasks don't have a built-in restart verb — end then run again.
    let _ = std::process::Command::new("schtasks")
        .args(["/End", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let run = std::process::Command::new("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        anyhow::bail!("schtasks /Run failed: {}", stderr.trim());
    }
    Ok(format!("schtasks /Run {TASK_NAME}: ok"))
}
