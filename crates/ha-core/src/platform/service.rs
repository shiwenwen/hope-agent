use anyhow::{bail, Context, Result};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
const SERVICE_LABEL: &str = "ai.hopeagent.server";
#[cfg(target_os = "macos")]
const LEGACY_SERVICE_LABEL: &str = "com.hopeagent.server";

fn definition_uses_cli_api_key(content: &str) -> bool {
    content.contains("<string>--api-key</string>")
        || content.split_whitespace().any(|part| part == "--api-key")
}

/// Minimal XML-text escape for plist `<string>` bodies. launchd parses
/// the plist as XML, so any user-controlled value (executable path, bind address)
/// MUST be escaped or `<`/`>`/`&`/quotes in the input will be interpreted
/// as XML markup — in the worst case injecting extra `<string>` elements
/// that become additional argv entries to the launched process.
#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape a value for a systemd unit `ExecStart=` line so that whitespace,
/// quotes and backslashes can't split the command into multiple args or
/// inject extra tokens. systemd supports double-quoted strings with
/// backslash escapes — see systemd.exec(5) "Command lines". `$` is doubled
/// to `$$` so systemd's `$VAR` / `${VAR}` expansion can't substitute an
/// environment value into the command.
#[cfg(target_os = "linux")]
fn systemd_escape_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("$$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── Public API ────────────────────────────────────────────────────

/// Install Hope Agent as a user-level background service.
///
/// - macOS: `launchctl` LaunchAgent at `~/Library/LaunchAgents/…plist`
/// - Linux: `systemctl --user` unit at `~/.config/systemd/user/…service`
/// - Windows: a Task Scheduler entry (`schtasks /create /sc onlogon`) that
///   auto-launches the binary at user login. The task runs under the
///   current user with Interactive token and no admin escalation. This
///   is *not* a real Windows Service (which would require implementing
///   the SCM protocol via `StartServiceCtrlDispatcher`); for the
///   "background agent per user" use case Task Scheduler matches the
///   behavior of launchd `LaunchAgent` and `systemctl --user` more
///   closely than a system-scoped service would.
///
/// Returns a human-readable status message on success.
pub fn install_service(bind_addr: &str) -> Result<String> {
    let exe_path = std::env::current_exe()
        .context("Cannot resolve own executable path")?
        .to_string_lossy()
        .to_string();

    let log_dir = crate::paths::logs_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    return install_launchd(&exe_path, bind_addr, &log_path);

    #[cfg(target_os = "linux")]
    return install_systemd(&exe_path, bind_addr, &log_path);

    #[cfg(windows)]
    return windows_task::install_scheduled_task(&exe_path, bind_addr, &log_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    bail!("Service installation is not supported on this platform")
}

/// Return whether the installed user-service definition still embeds the
/// legacy `--api-key <token>` argv pair. The definition is inspected without
/// ever returning or logging its contents.
pub fn legacy_service_uses_cli_api_key() -> Result<bool> {
    #[cfg(target_os = "macos")]
    return launchd_uses_cli_api_key();

    #[cfg(target_os = "linux")]
    return systemd_uses_cli_api_key();

    #[cfg(windows)]
    return windows_task::scheduled_task_uses_cli_api_key();

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Ok(false)
}

/// Rewrite an installed legacy user-service definition without stopping or
/// starting the current process. This removes the command-line secret for the
/// next launch while avoiding a self-unload outage during an upgrade.
pub fn rewrite_service_without_cli_api_key(bind_addr: &str) -> Result<()> {
    let exe_path = std::env::current_exe()
        .context("Cannot resolve own executable path")?
        .to_string_lossy()
        .to_string();
    let log_dir = crate::paths::logs_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    return rewrite_launchd_without_cli_api_key(&exe_path, bind_addr, &log_path);

    #[cfg(target_os = "linux")]
    return rewrite_systemd_without_cli_api_key(&exe_path, bind_addr, &log_path);

    #[cfg(windows)]
    return windows_task::rewrite_scheduled_task_without_cli_api_key(
        &exe_path, bind_addr, &log_path,
    );

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    bail!("Service migration is not supported on this platform")
}

/// Uninstall the Hope Agent system service.
pub fn uninstall_service() -> Result<()> {
    #[cfg(target_os = "macos")]
    return uninstall_launchd();

    #[cfg(target_os = "linux")]
    return uninstall_systemd();

    #[cfg(windows)]
    return windows_task::uninstall_scheduled_task();

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    bail!("Service uninstallation is not supported on this platform")
}

/// Query the current status of the Hope Agent system service.
///
/// Returns a human-readable status string.
pub fn service_status() -> Result<String> {
    #[cfg(target_os = "macos")]
    return status_launchd();

    #[cfg(target_os = "linux")]
    return status_systemd();

    #[cfg(windows)]
    return windows_task::status_scheduled_task();

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    bail!("Service status is not supported on this platform")
}

/// Stop the running Hope Agent server.
///
/// Unix: sends SIGTERM to the PID in `~/.hope-agent/server.pid`.
/// Windows: calls `taskkill` on the same PID (a polite shutdown request;
/// `run_guardian` catches `ctrl_break` delivered by `taskkill` and exits
/// cleanly).
pub fn stop_server() -> Result<()> {
    let pid_path = crate::paths::root_dir()?.join("server.pid");
    if !pid_path.exists() {
        bail!(
            "PID file not found at {:?} — is the server running?",
            pid_path
        );
    }

    let pid_str = std::fs::read_to_string(&pid_path).context("Failed to read PID file")?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .context("Invalid PID in server.pid")?;

    #[cfg(unix)]
    {
        use std::process::Command;
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .context("Failed to send SIGTERM")?;
        if !status.success() {
            bail!("kill -TERM {} exited with status {}", pid, status);
        }
    }

    #[cfg(windows)]
    {
        crate::platform::send_graceful_stop(pid);
    }

    // Clean up PID file
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

// ── macOS launchd ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn plist_path() -> Result<PathBuf> {
    plist_path_for_label(SERVICE_LABEL)
}

#[cfg(target_os = "macos")]
fn legacy_plist_path() -> Result<PathBuf> {
    plist_path_for_label(LEGACY_SERVICE_LABEL)
}

#[cfg(target_os = "macos")]
fn plist_path_for_label(label: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot find home directory")?;
    let launch_agents = home.join("Library").join("LaunchAgents");
    std::fs::create_dir_all(&launch_agents)?;
    Ok(launch_agents.join(format!("{}.plist", label)))
}

#[cfg(target_os = "macos")]
fn unload_launchd_plist(plist: &std::path::Path) {
    let _ = std::process::Command::new("launchctl")
        .args(["unload", &plist.to_string_lossy()])
        .output();
}

#[cfg(target_os = "macos")]
fn launchd_content(label: &str, exe_path: &str, bind_addr: &str, log_path: &str) -> String {
    let args_xml = format!(
        "        <string>{}</string>\n\
         \x20       <string>server</string>\n\
         \x20       <string>--bind</string>\n\
         \x20       <string>{}</string>",
        xml_escape(exe_path),
        xml_escape(bind_addr)
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args}
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log}/server.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{log}/server.stderr.log</string>
</dict>
</plist>
"#,
        label = label,
        args = args_xml,
        log = xml_escape(log_path),
    )
}

#[cfg(target_os = "macos")]
fn launchd_uses_cli_api_key() -> Result<bool> {
    for path in [plist_path()?, legacy_plist_path()?] {
        match std::fs::read_to_string(&path) {
            Ok(content) if definition_uses_cli_api_key(&content) => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to inspect plist {:?}", path));
            }
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn rewrite_launchd_without_cli_api_key(
    exe_path: &str,
    bind_addr: &str,
    log_path: &str,
) -> Result<()> {
    let mut rewritten = false;
    for (label, path) in [
        (SERVICE_LABEL, plist_path()?),
        (LEGACY_SERVICE_LABEL, legacy_plist_path()?),
    ] {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to inspect plist {:?}", path));
            }
        };
        if !definition_uses_cli_api_key(&content) {
            continue;
        }
        let replacement = launchd_content(label, exe_path, bind_addr, log_path);
        crate::platform::write_atomic(&path, replacement.as_bytes())
            .with_context(|| format!("Failed to rewrite plist {:?}", path))?;
        rewritten = true;
    }
    if !rewritten {
        bail!("No legacy launchd service definition was found")
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_launchd(exe_path: &str, bind_addr: &str, log_path: &str) -> Result<String> {
    let plist = plist_path()?;
    let legacy_plist = legacy_plist_path()?;

    let content = launchd_content(SERVICE_LABEL, exe_path, bind_addr, log_path);

    // Remove the pre-hopeagent.ai service label so users don't end up
    // with two LaunchAgents racing to run the same server.
    if legacy_plist.exists() {
        unload_launchd_plist(&legacy_plist);
        std::fs::remove_file(&legacy_plist)
            .with_context(|| format!("Failed to remove legacy plist {:?}", legacy_plist))?;
    }

    // Unload the existing service if present (ignore errors)
    if plist.exists() {
        unload_launchd_plist(&plist);
    }

    std::fs::write(&plist, &content)
        .with_context(|| format!("Failed to write plist to {:?}", plist))?;

    let status = std::process::Command::new("launchctl")
        .args(["load", &plist.to_string_lossy()])
        .status()
        .context("Failed to run launchctl load")?;

    if !status.success() {
        bail!("launchctl load failed with status {}", status);
    }

    Ok(format!(
        "Service installed and started.\n  Plist: {}\n  Bind:  {}\n  Logs:  {}/server.{{stdout,stderr}}.log",
        plist.display(),
        bind_addr,
        log_path,
    ))
}

#[cfg(target_os = "macos")]
fn uninstall_launchd() -> Result<()> {
    let plist = plist_path()?;
    let legacy_plist = legacy_plist_path()?;
    if !plist.exists() && !legacy_plist.exists() {
        bail!(
            "Service plist not found at {:?} or {:?} — is the service installed?",
            plist,
            legacy_plist
        );
    }

    for service_plist in [&plist, &legacy_plist] {
        if !service_plist.exists() {
            continue;
        }

        let status = std::process::Command::new("launchctl")
            .args(["unload", &service_plist.to_string_lossy()])
            .status()
            .context("Failed to run launchctl unload")?;

        if !status.success() {
            eprintln!(
                "[service] Warning: launchctl unload exited with status {}",
                status
            );
        }

        std::fs::remove_file(service_plist)
            .with_context(|| format!("Failed to remove plist {:?}", service_plist))?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn status_launchd() -> Result<String> {
    let plist = plist_path()?;
    if plist.exists() {
        return status_launchd_for(SERVICE_LABEL, &plist);
    }

    let legacy_plist = legacy_plist_path()?;
    if legacy_plist.exists() {
        return status_launchd_for(LEGACY_SERVICE_LABEL, &legacy_plist);
    }

    Ok("not installed".to_string())
}

#[cfg(target_os = "macos")]
fn status_launchd_for(label: &str, plist: &std::path::Path) -> Result<String> {
    if !plist.exists() {
        return Ok("not installed".to_string());
    }

    let output = std::process::Command::new("launchctl")
        .args(["list", label])
        .output()
        .context("Failed to run launchctl list")?;

    let install_state = if label == SERVICE_LABEL {
        "installed"
    } else {
        "installed with legacy label"
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse the launchctl list output for PID and status
        let mut pid = "–";
        let mut exit_status = "–";
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                pid = if parts[0] == "-" {
                    "not running"
                } else {
                    parts[0]
                };
                exit_status = parts[1];
            }
        }
        Ok(format!(
            "{} (plist: {})\n  Label: {}\n  PID: {}\n  Last exit status: {}",
            install_state,
            plist.display(),
            label,
            pid,
            exit_status,
        ))
    } else {
        Ok(format!(
            "{} but not loaded (plist: {})\n  Label: {}",
            install_state,
            plist.display(),
            label,
        ))
    }
}

// ── Linux systemd ─────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot find home directory")?;
    let systemd_user = home.join(".config").join("systemd").join("user");
    std::fs::create_dir_all(&systemd_user)?;
    Ok(systemd_user.join("hope-agent.service"))
}

#[cfg(target_os = "linux")]
fn systemd_content(exe_path: &str, bind_addr: &str, log_path: &str) -> String {
    let exec_start = format!(
        "{} server --bind {}",
        systemd_escape_arg(exe_path),
        systemd_escape_arg(bind_addr)
    );
    format!(
        "[Unit]\n\
         Description=Hope Agent Server\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         StandardOutput=append:{stdout}/server.stdout.log\n\
         StandardError=append:{stderr}/server.stderr.log\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec = exec_start,
        stdout = log_path,
        stderr = log_path,
    )
}

#[cfg(target_os = "linux")]
fn systemd_uses_cli_api_key() -> Result<bool> {
    let unit = unit_path()?;
    match std::fs::read_to_string(&unit) {
        Ok(content) => Ok(definition_uses_cli_api_key(&content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Failed to inspect unit {:?}", unit)),
    }
}

#[cfg(target_os = "linux")]
fn rewrite_systemd_without_cli_api_key(
    exe_path: &str,
    bind_addr: &str,
    log_path: &str,
) -> Result<()> {
    let unit = unit_path()?;
    let existing = std::fs::read_to_string(&unit)
        .with_context(|| format!("Failed to inspect unit {:?}", unit))?;
    if !definition_uses_cli_api_key(&existing) {
        bail!("No legacy systemd service definition was found");
    }
    let replacement = systemd_content(exe_path, bind_addr, log_path);
    crate::platform::write_atomic(&unit, replacement.as_bytes())
        .with_context(|| format!("Failed to rewrite unit {:?}", unit))?;
    let status = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .context("Failed to run systemctl daemon-reload")?;
    if !status.success() {
        bail!("systemctl daemon-reload failed with status {}", status);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_systemd(exe_path: &str, bind_addr: &str, log_path: &str) -> Result<String> {
    let unit = unit_path()?;
    let stdout_log = format!("{}/server.stdout.log", log_path);
    let stderr_log = format!("{}/server.stderr.log", log_path);

    // Pre-create log files so systemd's append: redirection always has a target.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_log);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_log);

    let content = systemd_content(exe_path, bind_addr, log_path);

    std::fs::write(&unit, &content)
        .with_context(|| format!("Failed to write unit file to {:?}", unit))?;

    // Reload systemd user daemon
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "hope-agent.service"])
        .status()
        .context("Failed to run systemctl enable")?;

    if !status.success() {
        bail!("systemctl enable failed with status {}", status);
    }

    // Enable linger so the user service keeps running after logout (and auto-starts at boot).
    // Requires polkit authorization; on some distros this needs sudo. Failure is non-fatal.
    let linger_note = enable_linger_for_current_user();

    Ok(format!(
        "Service installed and started.\n  Unit: {}\n  Bind: {}\n  Logs: {}/server.{{stdout,stderr}}.log\n  {}",
        unit.display(),
        bind_addr,
        log_path,
        linger_note,
    ))
}

#[cfg(target_os = "linux")]
fn enable_linger_for_current_user() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        return "Linger: skipped (USER env not set; run `loginctl enable-linger <user>` manually)"
            .to_string();
    }

    let output = std::process::Command::new("loginctl")
        .args(["enable-linger", &user])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            format!("Linger: enabled for {} (service survives logout)", user)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!(
                "Linger: not enabled ({}). Run `sudo loginctl enable-linger {}` manually so the service survives logout.",
                stderr.trim(),
                user
            )
        }
        Err(e) => format!(
            "Linger: loginctl unavailable ({}). Run `sudo loginctl enable-linger {}` manually.",
            e, user
        ),
    }
}

#[cfg(target_os = "linux")]
fn uninstall_systemd() -> Result<()> {
    let unit = unit_path()?;
    if !unit.exists() {
        bail!(
            "Service unit not found at {:?} — is the service installed?",
            unit
        );
    }

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "hope-agent.service"])
        .status();

    std::fs::remove_file(&unit)
        .with_context(|| format!("Failed to remove unit file {:?}", unit))?;

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    Ok(())
}

#[cfg(target_os = "linux")]
fn status_systemd() -> Result<String> {
    let unit = unit_path()?;
    if !unit.exists() {
        return Ok("not installed".to_string());
    }

    let output = std::process::Command::new("systemctl")
        .args(["--user", "status", "hope-agent.service"])
        .output()
        .context("Failed to run systemctl status")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.to_string())
}

// ── Windows Task Scheduler ─────────────────────────────────────────

#[cfg(windows)]
mod windows_task {
    use super::*;
    use std::os::windows::process::CommandExt as _;
    use std::process::Command;

    /// CREATE_NO_WINDOW — prevent `schtasks.exe` from flashing a console
    /// window when invoked from the Tauri GUI process.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Task name as it appears in Task Scheduler. Mirrors the launchd
    /// label convention but uses the native forward-slash-free style
    /// that `schtasks.exe` expects.
    const TASK_NAME: &str = "Hope Agent";

    /// Double every embedded double-quote so the argument survives the
    /// Windows command line quoting rules when passed through
    /// `schtasks /tr`, which takes a single quoted string that is parsed
    /// by the eventual target command.
    fn quote_arg(s: &str) -> String {
        if s.is_empty() {
            return "\"\"".to_string();
        }
        let needs_quotes = s.contains(' ') || s.contains('"') || s.contains('\t');
        if !needs_quotes {
            return s.to_string();
        }
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            if c == '"' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    fn task_command(exe_path: &str, bind_addr: &str, log_path: &str) -> String {
        let inner = format!(
            "{} server --bind {}",
            quote_arg(exe_path),
            quote_arg(bind_addr)
        );
        let stdout_log = format!("{}\\server.stdout.log", log_path);
        let stderr_log = format!("{}\\server.stderr.log", log_path);
        format!(
            "cmd /C {} >> {} 2>> {}",
            inner,
            quote_arg(&stdout_log),
            quote_arg(&stderr_log),
        )
    }

    fn create_scheduled_task(exe_path: &str, bind_addr: &str, log_path: &str) -> Result<()> {
        let tr = task_command(exe_path, bind_addr, log_path);
        let output = Command::new("schtasks")
            .args([
                "/Create", "/TN", TASK_NAME, "/TR", &tr, "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("Failed to run schtasks /Create")?;
        if !output.status.success() {
            bail!(
                "schtasks /Create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn scheduled_task_xml() -> Result<Option<Vec<u8>>> {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/XML"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("Failed to run schtasks /Query")?;
        if output.status.success() {
            return Ok(Some(output.stdout));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("cannot find")
            || stderr.contains("ERROR: The system")
            || stderr.contains("does not exist")
        {
            return Ok(None);
        }
        bail!("schtasks /Query failed: {}", stderr.trim())
    }

    pub(super) fn scheduled_task_uses_cli_api_key() -> Result<bool> {
        Ok(scheduled_task_xml()?
            .is_some_and(|xml| definition_uses_cli_api_key(&String::from_utf8_lossy(&xml))))
    }

    pub(super) fn rewrite_scheduled_task_without_cli_api_key(
        exe_path: &str,
        bind_addr: &str,
        log_path: &str,
    ) -> Result<()> {
        if !scheduled_task_uses_cli_api_key()? {
            bail!("No legacy scheduled-task definition was found");
        }
        // `/Create /F` updates the future task action without terminating or
        // duplicating the process that is performing this migration.
        create_scheduled_task(exe_path, bind_addr, log_path)
    }

    pub(super) fn install_scheduled_task(
        exe_path: &str,
        bind_addr: &str,
        log_path: &str,
    ) -> Result<String> {
        // `/F` on /Create force-overwrites any existing task with the same
        // name — no separate /Delete needed.
        create_scheduled_task(exe_path, bind_addr, log_path)?;

        // Start it now so the user doesn't have to log out / back in.
        let run_output = Command::new("schtasks")
            .args(["/Run", "/TN", TASK_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let run_note = match run_output {
            Ok(o) if o.status.success() => "running".to_string(),
            Ok(o) => format!(
                "created but failed to start: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => format!("created but failed to start: {}", e),
        };

        Ok(format!(
            "Scheduled task installed.\n  Task: {}\n  Bind: {}\n  Logs: {}\\server.{{stdout,stderr}}.log\n  Status: {}\n  Note: auto-starts at next user login; does not survive a reboot-before-login.",
            TASK_NAME, bind_addr, log_path, run_note,
        ))
    }

    pub(super) fn uninstall_scheduled_task() -> Result<()> {
        // Best-effort: stop any running instance first.
        let _ = Command::new("schtasks")
            .args(["/End", "/TN", TASK_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        let output = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("Failed to run schtasks /Delete")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("schtasks /Delete failed: {}", stderr.trim());
        }

        Ok(())
    }

    pub(super) fn status_scheduled_task() -> Result<String> {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/FO", "LIST"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("Failed to run schtasks /Query")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("cannot find")
                || stderr.contains("ERROR: The system")
                || stderr.contains("does not exist")
            {
                return Ok("not installed".to_string());
            }
            bail!("schtasks /Query failed: {}", stderr.trim());
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let mut summary = format!("installed (task: {})", TASK_NAME);
        for line in text.lines() {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("status:") || lower.starts_with("last run time:") {
                summary.push('\n');
                summary.push_str("  ");
                summary.push_str(line.trim());
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::definition_uses_cli_api_key;

    #[test]
    fn legacy_cli_key_detection_does_not_match_file_credentials() {
        assert!(definition_uses_cli_api_key(
            "ExecStart=hope-agent server --api-key secret"
        ));
        assert!(definition_uses_cli_api_key(
            "<string>--api-key</string>\n<string>secret</string>"
        ));
        assert!(!definition_uses_cli_api_key(
            "ExecStart=hope-agent server --api-key-file /run/secret"
        ));
    }
}
