use anyhow::{bail, Context, Result};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SERVICE_LABEL: &str = "com.opencomputer.server";

/// Minimal XML-text escape for plist `<string>` bodies. launchd parses
/// the plist as XML, so any user-controlled value (home path, api key)
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

/// Install OpenComputer as a user-level background service.
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
pub fn install_service(bind_addr: &str, api_key: Option<&str>) -> Result<String> {
    let exe_path = std::env::current_exe()
        .context("Cannot resolve own executable path")?
        .to_string_lossy()
        .to_string();

    let log_dir = crate::paths::logs_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    return install_launchd(&exe_path, bind_addr, api_key, &log_path);

    #[cfg(target_os = "linux")]
    return install_systemd(&exe_path, bind_addr, api_key, &log_path);

    #[cfg(windows)]
    return windows_task::install_scheduled_task(&exe_path, bind_addr, api_key, &log_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    bail!("Service installation is not supported on this platform")
}

/// Uninstall the OpenComputer system service.
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

/// Query the current status of the OpenComputer system service.
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

/// Stop the running OpenComputer server.
///
/// Unix: sends SIGTERM to the PID in `~/.opencomputer/server.pid`.
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
    let home = dirs::home_dir().context("Cannot find home directory")?;
    let launch_agents = home.join("Library").join("LaunchAgents");
    std::fs::create_dir_all(&launch_agents)?;
    Ok(launch_agents.join(format!("{}.plist", SERVICE_LABEL)))
}

#[cfg(target_os = "macos")]
fn install_launchd(
    exe_path: &str,
    bind_addr: &str,
    api_key: Option<&str>,
    log_path: &str,
) -> Result<String> {
    let plist = plist_path()?;

    // Build ProgramArguments entries. Every user-controlled value
    // (exe path, bind addr, api key, log path) is XML-escaped so that
    // characters like `<`, `>`, `"` or `&` cannot break out of the
    // surrounding `<string>` element and inject additional argv entries.
    let mut args_xml = format!(
        "        <string>{}</string>\n\
         \x20       <string>server</string>\n\
         \x20       <string>--bind</string>\n\
         \x20       <string>{}</string>",
        xml_escape(exe_path),
        xml_escape(bind_addr)
    );
    if let Some(key) = api_key {
        args_xml.push_str(&format!(
            "\n        <string>--api-key</string>\n\
             \x20       <string>{}</string>",
            xml_escape(key)
        ));
    }

    let content = format!(
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
        label = SERVICE_LABEL,
        args = args_xml,
        log = xml_escape(log_path),
    );

    // Unload the existing service if present (ignore errors)
    if plist.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist.to_string_lossy()])
            .output();
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
    if !plist.exists() {
        bail!(
            "Service plist not found at {:?} — is the service installed?",
            plist
        );
    }

    let status = std::process::Command::new("launchctl")
        .args(["unload", &plist.to_string_lossy()])
        .status()
        .context("Failed to run launchctl unload")?;

    if !status.success() {
        eprintln!(
            "[service] Warning: launchctl unload exited with status {}",
            status
        );
    }

    std::fs::remove_file(&plist).with_context(|| format!("Failed to remove plist {:?}", plist))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn status_launchd() -> Result<String> {
    let plist = plist_path()?;
    if !plist.exists() {
        return Ok("not installed".to_string());
    }

    let output = std::process::Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()
        .context("Failed to run launchctl list")?;

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
            "installed (plist: {})\n  PID: {}\n  Last exit status: {}",
            plist.display(),
            pid,
            exit_status,
        ))
    } else {
        Ok(format!(
            "installed but not loaded (plist: {})",
            plist.display()
        ))
    }
}

// ── Linux systemd ─────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot find home directory")?;
    let systemd_user = home.join(".config").join("systemd").join("user");
    std::fs::create_dir_all(&systemd_user)?;
    Ok(systemd_user.join("opencomputer.service"))
}

#[cfg(target_os = "linux")]
fn install_systemd(
    exe_path: &str,
    bind_addr: &str,
    api_key: Option<&str>,
    log_path: &str,
) -> Result<String> {
    let unit = unit_path()?;

    // Quote every argv token individually so whitespace / quotes in any
    // user-controlled value (exe path, bind addr, api key) cannot split
    // the line into extra tokens or inject shell metacharacters into
    // `ExecStart`.
    let mut exec_start = format!(
        "{} server --bind {}",
        systemd_escape_arg(exe_path),
        systemd_escape_arg(bind_addr)
    );
    if let Some(key) = api_key {
        exec_start.push_str(&format!(" --api-key {}", systemd_escape_arg(key)));
    }

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

    let content = format!(
        "[Unit]\n\
         Description=OpenComputer Server\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         StandardOutput=append:{stdout}\n\
         StandardError=append:{stderr}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec = exec_start,
        stdout = stdout_log,
        stderr = stderr_log,
    );

    std::fs::write(&unit, &content)
        .with_context(|| format!("Failed to write unit file to {:?}", unit))?;

    // Reload systemd user daemon
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "opencomputer.service"])
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
        .args(["--user", "disable", "--now", "opencomputer.service"])
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
        .args(["--user", "status", "opencomputer.service"])
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
    const TASK_NAME: &str = "OpenComputer";

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

    pub(super) fn install_scheduled_task(
        exe_path: &str,
        bind_addr: &str,
        api_key: Option<&str>,
        log_path: &str,
    ) -> Result<String> {
        // Build the argv the task will run. We use a wrapper `cmd /C` so
        // stdout/stderr can be redirected into our logs directory — the
        // native schtasks TaskRun doesn't capture output on its own.
        let mut inner = format!(
            "{} server --bind {}",
            quote_arg(exe_path),
            quote_arg(bind_addr)
        );
        if let Some(key) = api_key {
            inner.push_str(&format!(" --api-key {}", quote_arg(key)));
        }

        let stdout_log = format!("{}\\server.stdout.log", log_path);
        let stderr_log = format!("{}\\server.stderr.log", log_path);

        let tr = format!(
            "cmd /C {} >> {} 2>> {}",
            inner,
            quote_arg(&stdout_log),
            quote_arg(&stderr_log),
        );

        // `/F` on /Create force-overwrites any existing task with the
        // same name — no separate /Delete needed.
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
