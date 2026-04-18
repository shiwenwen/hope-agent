use anyhow::{bail, Context, Result};
use std::path::PathBuf;

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

/// Install OpenComputer as a system service (launchd on macOS, systemd on Linux).
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

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("Service installation is not supported on this platform")
}

/// Uninstall the OpenComputer system service.
pub fn uninstall_service() -> Result<()> {
    #[cfg(target_os = "macos")]
    return uninstall_launchd();

    #[cfg(target_os = "linux")]
    return uninstall_systemd();

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("Service status is not supported on this platform")
}

/// Stop the running OpenComputer server by sending SIGTERM to the PID in the PID file.
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

    #[cfg(not(unix))]
    bail!("stop_server is only supported on Unix platforms");

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
        return "Linger: skipped (USER env not set; run `loginctl enable-linger <user>` manually)".to_string();
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
