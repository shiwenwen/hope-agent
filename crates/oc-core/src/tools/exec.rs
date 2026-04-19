use anyhow::Result;
use serde_json::Value;
use std::sync::OnceLock;

use crate::process_registry::{
    create_session_id, get_registry, now_ms, ProcessSession, ProcessStatus,
};

use super::approval::{
    add_to_allowlist, approval_timeout_action, check_and_request_approval,
    get_tool_permission_mode, is_command_allowed, ApprovalCheckError, ApprovalResponse,
    ToolPermissionMode,
};

pub(crate) const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 1800; // 30 minutes
pub(crate) const MAX_EXEC_TIMEOUT_SECS: u64 = 7200; // 2 hours max

/// Default output truncation (200K chars)
pub(crate) const DEFAULT_MAX_OUTPUT_CHARS: usize = 200_000;
/// Minimum output truncation for small-context models
pub(crate) const MIN_MAX_OUTPUT_CHARS: usize = 8_000;
/// Default yield window for background commands (10 seconds)
pub(crate) const DEFAULT_YIELD_MS: u64 = 10_000;
pub(crate) const MAX_YIELD_MS: u64 = 120_000;

// ── Shell PATH Resolution ─────────────────────────────────────────

static LOGIN_SHELL_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Resolve the full PATH from the user's login shell.
/// This ensures tools like npm, python, etc. are available even when
/// launched from a desktop environment that doesn't source .bashrc/.zshrc.
///
/// On Windows this returns `None` — the inherited process PATH already
/// reflects the user's HKCU + HKLM PATH; spawning a "login shell" is a
/// Unix-only concept.
pub(crate) fn get_login_shell_path() -> Option<&'static str> {
    #[cfg(windows)]
    {
        return None;
    }

    #[cfg(unix)]
    {
        LOGIN_SHELL_PATH
            .get_or_init(|| {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let output = std::process::Command::new(&shell)
                    .args(["-l", "-c", "echo $PATH"])
                    .output()
                    .ok()?;
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        app_info!(
                            "tool",
                            "exec",
                            "Resolved login shell PATH: {}",
                            &path[..path.len().min(120)]
                        );
                        Some(path)
                    } else {
                        None
                    }
                } else {
                    app_warn!("tool", "exec", "Failed to resolve login shell PATH");
                    None
                }
            })
            .as_deref()
    }
}

/// Compute dynamic max output chars based on model context window.
/// Uses ~20% of context window (at ~4 chars/token estimate).
pub(crate) fn compute_max_output_chars(context_window_tokens: Option<u32>) -> usize {
    match context_window_tokens {
        Some(tokens) if tokens > 0 => {
            let chars_from_context = (tokens as usize) * 4 / 5; // 20% of context * 4 chars/token
            chars_from_context.clamp(MIN_MAX_OUTPUT_CHARS, DEFAULT_MAX_OUTPUT_CHARS)
        }
        _ => DEFAULT_MAX_OUTPUT_CHARS,
    }
}

/// Shared handling for `ApprovalCheckError::TimedOut` in the exec path.
/// Returns `Ok(())` when the configured timeout action is Proceed and
/// `Err(..)` (after marking the process session Failed) when Deny.
async fn handle_exec_approval_timeout(
    session_id: &str,
    command: &str,
    timeout_secs: u64,
) -> Result<()> {
    match approval_timeout_action() {
        crate::config::ApprovalTimeoutAction::Deny => {
            let mut registry = get_registry().lock().await;
            registry.mark_exited(session_id, None, None, ProcessStatus::Failed);
            app_warn!(
                "tool",
                "exec",
                "Approval timed out after {}s; blocking command execution: {}",
                timeout_secs,
                command
            );
            Err(anyhow::anyhow!(
                "Command execution denied: approval timed out after {}s: {}",
                timeout_secs,
                command
            ))
        }
        crate::config::ApprovalTimeoutAction::Proceed => {
            app_warn!(
                "tool",
                "exec",
                "Approval timed out after {}s; proceeding by config: {}",
                timeout_secs,
                command
            );
            Ok(())
        }
    }
}

pub(crate) async fn tool_exec(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(super::expand_tilde);

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_EXEC_TIMEOUT_SECS)
        .min(MAX_EXEC_TIMEOUT_SECS);

    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let use_pty = args.get("pty").and_then(|v| v.as_bool()).unwrap_or(false);
    let sandbox = ctx.force_sandbox
        || args
            .get("sandbox")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let yield_ms = args
        .get("yield_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_YIELD_MS)
        .min(MAX_YIELD_MS);

    let max_output = compute_max_output_chars(ctx.context_window_tokens);

    app_info!(
        "tool",
        "exec",
        "Executing command: {} (cwd: {:?}, timeout: {}s, bg: {}, pty: {}, max_out: {})",
        command,
        cwd,
        timeout_secs,
        background,
        use_pty,
        max_output
    );

    // Structured logging
    if let Some(logger) = crate::get_logger() {
        let cmd_preview = if command.len() > 200 {
            format!("{}...", crate::truncate_utf8(command, 200))
        } else {
            command.to_string()
        };
        logger.log(
            "info",
            "tool",
            "exec::start",
            &format!("exec: {}", cmd_preview),
            Some(
                serde_json::json!({
                    "cwd": cwd, "timeout": timeout_secs,
                    "background": background, "pty": use_pty, "sandbox": sandbox,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    // Build the command via the platform shell (sh -c on Unix, cmd /C on Windows)
    let mut cmd = crate::platform::default_shell_command_tokio(command);

    // Set working directory: explicit cwd > agent home > user home
    if let Some(ref dir) = cwd {
        cmd.current_dir(dir);
    } else if let Some(ref agent_home) = ctx.home_dir {
        cmd.current_dir(agent_home);
    } else if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }

    // Apply login shell PATH
    if let Some(shell_path) = get_login_shell_path() {
        cmd.env("PATH", shell_path);
    }

    // Apply custom environment variables
    if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
        for (key, val) in env_obj {
            if let Some(v) = val.as_str() {
                cmd.env(key, v);
            }
        }
    }

    // Create a session for tracking
    let session_id = create_session_id();
    let session_cwd = cwd
        .clone()
        .or_else(|| ctx.home_dir.clone())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string())
        });

    let session = ProcessSession {
        id: session_id.clone(),
        command: command.to_string(),
        pid: None,
        cwd: session_cwd.clone(),
        started_at: now_ms(),
        exited: false,
        exit_code: None,
        exit_signal: None,
        status: ProcessStatus::Running,
        backgrounded: false,
        aggregated_output: String::new(),
        tail: String::new(),
        truncated: false,
        max_output_chars: max_output,
        pending_stdout: String::new(),
        pending_stderr: String::new(),
    };

    {
        let mut registry = get_registry().lock().await;
        registry.add_session(session);
    }

    // ── Command approval gate ───────────────────────────────────
    let dangerous_mode = crate::security::dangerous::is_dangerous_skip_active();
    if dangerous_mode && !ctx.auto_approve_tools {
        app_warn!(
            "tool",
            "exec",
            "Command bypassed approval (DANGEROUS MODE active via {}): {}",
            crate::security::dangerous::active_source(),
            command
        );
    }
    if !ctx.auto_approve_tools && !dangerous_mode {
        let perm_mode = get_tool_permission_mode().await;
        match perm_mode {
            ToolPermissionMode::FullApprove => {
                app_info!(
                    "tool",
                    "exec",
                    "Command auto-approved (full_approve mode): {}",
                    command
                );
            }
            ToolPermissionMode::AskEveryTime => {
                match check_and_request_approval(command, &session_cwd, ctx.session_id.as_deref())
                    .await
                {
                    Ok(ApprovalResponse::AllowOnce) => {
                        app_info!(
                            "tool",
                            "exec",
                            "Command approved (once, ask_every_time): {}",
                            command
                        );
                    }
                    Ok(ApprovalResponse::AllowAlways) => {
                        app_info!(
                            "tool",
                            "exec",
                            "Command approved (ask_every_time): {}",
                            command
                        );
                    }
                    Ok(ApprovalResponse::Deny) => {
                        let mut registry = get_registry().lock().await;
                        registry.mark_exited(&session_id, None, None, ProcessStatus::Failed);
                        return Err(anyhow::anyhow!(
                            "Command execution denied by user: {}",
                            command
                        ));
                    }
                    Err(ApprovalCheckError::TimedOut { timeout_secs }) => {
                        handle_exec_approval_timeout(&session_id, command, timeout_secs).await?;
                    }
                    Err(e) => {
                        app_warn!(
                            "tool",
                            "exec",
                            "Approval check failed ({}), proceeding with execution",
                            e
                        );
                    }
                }
            }
            ToolPermissionMode::Auto => {
                if !is_command_allowed(command).await {
                    match check_and_request_approval(
                        command,
                        &session_cwd,
                        ctx.session_id.as_deref(),
                    )
                    .await
                    {
                        Ok(ApprovalResponse::AllowOnce) => {
                            app_info!("tool", "exec", "Command approved (once): {}", command);
                        }
                        Ok(ApprovalResponse::AllowAlways) => {
                            app_info!("tool", "exec", "Command approved (always): {}", command);
                            add_to_allowlist(command).await;
                        }
                        Ok(ApprovalResponse::Deny) => {
                            let mut registry = get_registry().lock().await;
                            registry.mark_exited(&session_id, None, None, ProcessStatus::Failed);
                            return Err(anyhow::anyhow!(
                                "Command execution denied by user: {}",
                                command
                            ));
                        }
                        Err(ApprovalCheckError::TimedOut { timeout_secs }) => {
                            handle_exec_approval_timeout(&session_id, command, timeout_secs)
                                .await?;
                        }
                        Err(e) => {
                            app_warn!(
                                "tool",
                                "exec",
                                "Approval check failed ({}), proceeding with execution",
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    // ── Docker sandbox execution path ─────────────────────────
    if sandbox {
        app_info!(
            "tool",
            "exec",
            "Using Docker sandbox for command: {}",
            command
        );
        let sandbox_config = crate::sandbox::load_sandbox_config().unwrap_or_default();
        let env_map = args.get("env").and_then(|v| v.as_object());

        if background {
            // Background sandbox execution
            let cmd_owned = command.to_string();
            let cwd_owned = session_cwd.clone();
            let env_owned: Option<serde_json::Map<String, serde_json::Value>> = env_map.cloned();
            let config_owned = sandbox_config.clone();
            let sid = session_id.clone();

            {
                let mut registry = get_registry().lock().await;
                if let Some(s) = registry.get_session_mut(&sid) {
                    s.backgrounded = true;
                }
            }

            tokio::spawn(async move {
                let result = crate::sandbox::exec_in_sandbox(
                    &cmd_owned,
                    &cwd_owned,
                    env_owned.as_ref(),
                    &config_owned,
                    timeout_secs,
                )
                .await;

                let mut registry = get_registry().lock().await;
                match result {
                    Ok(sr) => {
                        let combined = if sr.stderr.is_empty() {
                            sr.stdout.clone()
                        } else {
                            format!("{}\n[stderr] {}", sr.stdout, sr.stderr)
                        };
                        registry.append_output(&sid, "stdout", &combined);
                        let status = if sr.exit_code == 0 {
                            ProcessStatus::Completed
                        } else {
                            ProcessStatus::Failed
                        };
                        registry.mark_exited(&sid, Some(sr.exit_code as i32), None, status);
                    }
                    Err(e) => {
                        registry.append_output(&sid, "stderr", &format!("Sandbox error: {}", e));
                        registry.mark_exited(&sid, Some(-1), None, ProcessStatus::Failed);
                    }
                }
            });

            return Ok(format!(
                "Command started in Docker sandbox (session {}). Use process(action=\"poll\", session_id=\"{}\") to check status.",
                session_id, session_id
            ));
        }

        // Synchronous sandbox execution
        match crate::sandbox::exec_in_sandbox(
            command,
            &session_cwd,
            env_map,
            &sandbox_config,
            timeout_secs,
        )
        .await
        {
            Ok(sr) => {
                let mut result_text = sr.stdout.clone();
                if !sr.stderr.is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str("[stderr] ");
                    result_text.push_str(&sr.stderr);
                }
                if sr.timed_out {
                    result_text.push_str(&format!(
                        "\n[sandbox: command timed out after {}s]",
                        timeout_secs
                    ));
                } else if result_text.is_empty() {
                    result_text = format!(
                        "[sandbox] Command completed with exit code {}",
                        sr.exit_code
                    );
                } else if sr.exit_code != 0 {
                    result_text.push_str(&format!("\n[exit code: {}]", sr.exit_code));
                }

                // Dynamic truncation
                if result_text.len() > max_output {
                    result_text.truncate(max_output);
                    result_text.push_str("\n... (output truncated)");
                }

                // Update registry
                {
                    let mut registry = get_registry().lock().await;
                    registry.append_output(&session_id, "stdout", &result_text);
                    let status = if sr.exit_code == 0 {
                        ProcessStatus::Completed
                    } else {
                        ProcessStatus::Failed
                    };
                    registry.mark_exited(&session_id, Some(sr.exit_code as i32), None, status);
                }

                return Ok(result_text);
            }
            Err(e) => {
                let mut registry = get_registry().lock().await;
                registry.mark_exited(&session_id, Some(-1), None, ProcessStatus::Failed);
                return Err(anyhow::anyhow!(
                    "Docker sandbox error: {}. Hint: ensure Docker is installed and running.",
                    e
                ));
            }
        }
    }

    // ── PTY execution path ──────────────────────────────────────
    if use_pty {
        app_info!("tool", "exec", "Using PTY mode for command: {}", command);
        match exec_via_pty(
            command,
            cwd.as_deref(),
            args,
            timeout_secs,
            max_output,
            &session_id,
            ctx,
        )
        .await
        {
            Ok(result) => return Ok(result),
            Err(e) => {
                app_warn!(
                    "tool",
                    "exec",
                    "PTY execution failed ({}), falling back to normal mode",
                    e
                );
                // Fall through to normal execution
            }
        }
    }

    // ── Normal execution path ──────────────────────────────────

    // If background=true, spawn and return immediately
    if background {
        let sid = session_id.clone();
        let timeout = timeout_secs;
        tokio::spawn(async move {
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(timeout), cmd.output()).await;
            let mut registry = get_registry().lock().await;
            match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let exit_code = output.status.code().unwrap_or(-1);
                    registry.append_output(&sid, "stdout", &stdout);
                    if !stderr.is_empty() {
                        registry.append_output(&sid, "stderr", &format!("[stderr] {}", stderr));
                    }
                    let status = if exit_code == 0 {
                        ProcessStatus::Completed
                    } else {
                        ProcessStatus::Failed
                    };
                    registry.mark_exited(&sid, Some(exit_code), None, status);
                }
                Ok(Err(e)) => {
                    registry.append_output(&sid, "stderr", &format!("Failed to execute: {}", e));
                    registry.mark_exited(&sid, None, None, ProcessStatus::Failed);
                }
                Err(_) => {
                    registry.append_output(
                        &sid,
                        "stderr",
                        &format!("Command timed out after {}s", timeout),
                    );
                    registry.mark_exited(
                        &sid,
                        None,
                        Some("SIGKILL".to_string()),
                        ProcessStatus::Failed,
                    );
                }
            }
        });

        {
            let mut registry = get_registry().lock().await;
            if let Some(s) = registry.get_session_mut(&session_id) {
                s.backgrounded = true;
            }
        }

        return Ok(format!(
            "Command started in background (session {}). Use process(action=\"poll\", session_id=\"{}\") to check status.",
            session_id, session_id
        ));
    }

    // Non-background: run with yield_ms support
    let cmd_future =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output());

    // If yield_ms is specified (and not default 10s for non-background), use it
    let wants_yield = args.get("yield_ms").is_some();

    if wants_yield {
        // Wait yield_ms, if not done, background it
        let yield_duration = std::time::Duration::from_millis(yield_ms);
        let sid = session_id.clone();

        match tokio::time::timeout(yield_duration, cmd_future).await {
            Ok(result) => {
                // Command finished within yield window
                return finish_exec_sync(&sid, result, max_output).await;
            }
            Err(_) => {
                // yield_ms elapsed, command still running — background it
                {
                    let mut registry = get_registry().lock().await;
                    if let Some(s) = registry.get_session_mut(&sid) {
                        s.backgrounded = true;
                    }
                }

                return Ok(format!(
                    "Command still running after {}ms (session {}). Use process(action=\"poll\", session_id=\"{}\") to check status.",
                    yield_ms, sid, sid
                ));
            }
        }
    }

    // Standard synchronous execution
    let result = cmd_future.await;
    finish_exec_sync(&session_id, result, max_output).await
}

/// Finish a synchronous exec and return result
async fn finish_exec_sync(
    session_id: &str,
    result: std::result::Result<
        std::result::Result<std::process::Output, std::io::Error>,
        tokio::time::error::Elapsed,
    >,
    max_output: usize,
) -> Result<String> {
    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut result_text = String::new();
            if !stdout.is_empty() {
                result_text.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result_text.is_empty() {
                    result_text.push('\n');
                }
                result_text.push_str("[stderr] ");
                result_text.push_str(&stderr);
            }
            if result_text.is_empty() {
                result_text = format!("Command completed with exit code {}", exit_code);
            } else if exit_code != 0 {
                result_text.push_str(&format!("\n[exit code: {}]", exit_code));
            }

            // Dynamic truncation
            if result_text.len() > max_output {
                result_text.truncate(max_output);
                result_text.push_str("\n... (output truncated)");
            }

            // Update registry
            {
                let mut registry = get_registry().lock().await;
                registry.append_output(session_id, "stdout", &result_text);
                let status = if exit_code == 0 {
                    ProcessStatus::Completed
                } else {
                    ProcessStatus::Failed
                };
                registry.mark_exited(session_id, Some(exit_code), None, status);
            }

            Ok(result_text)
        }
        Ok(Err(e)) => {
            let mut registry = get_registry().lock().await;
            registry.mark_exited(session_id, None, None, ProcessStatus::Failed);
            Err(anyhow::anyhow!("Failed to execute command: {}", e))
        }
        Err(_) => {
            let mut registry = get_registry().lock().await;
            let timeout = DEFAULT_EXEC_TIMEOUT_SECS;
            registry.mark_exited(
                session_id,
                None,
                Some("timeout".to_string()),
                ProcessStatus::Failed,
            );
            Err(anyhow::anyhow!(
                "Command timed out after {}s. If this command is expected to take longer, re-run with a higher timeout (e.g., exec timeout=3600).",
                timeout
            ))
        }
    }
}

// ── PTY Execution ─────────────────────────────────────────────────

/// Execute a command via PTY (pseudo-terminal).
/// Runs in a blocking thread since portable-pty is synchronous.
/// Returns the combined output on completion.
async fn exec_via_pty(
    command: &str,
    cwd: Option<&str>,
    args: &Value,
    timeout_secs: u64,
    max_output: usize,
    session_id: &str,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;

    let command_owned = command.to_string();
    let cwd_owned = cwd.map(|s| s.to_string());
    let agent_home_owned = ctx.home_dir.clone();
    let env_vars: Vec<(String, String)> = args
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let login_path = get_login_shell_path().map(|s| s.to_string());
    let _sid = session_id.to_string();

    let result = tokio::task::spawn_blocking(move || -> Result<(String, Option<i32>)> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| {
                // ConPTY requires Windows 10 1809+. Older builds or sandboxed
                // environments (some CI runners) will surface a handle error
                // here; callers should fall back to the non-PTY path.
                #[cfg(windows)]
                {
                    app_warn!(
                        "tool",
                        "exec",
                        "ConPTY unavailable ({}): caller should retry with pty=false",
                        e
                    );
                }
                anyhow::anyhow!("Failed to open PTY: {}", e)
            })?;

        #[cfg(unix)]
        let mut cmd = {
            let mut c = CommandBuilder::new("sh");
            c.arg("-c");
            c.arg(&command_owned);
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = CommandBuilder::new("cmd");
            c.arg("/C");
            c.arg(&command_owned);
            c
        };

        // Set working directory: explicit cwd > agent home > user home
        if let Some(ref dir) = cwd_owned {
            cmd.cwd(dir);
        } else if let Some(ref agent_home) = agent_home_owned {
            cmd.cwd(agent_home);
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }

        // Apply login shell PATH
        if let Some(ref path) = login_path {
            cmd.env("PATH", path);
        }

        // Apply custom environment variables
        for (key, val) in &env_vars {
            cmd.env(key, val);
        }

        // Spawn the child process
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("Failed to spawn PTY command: {}", e))?;

        // Drop slave so reads on master will see EOF after child exits
        drop(pair.slave);

        // Read output from master PTY
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("Failed to clone PTY reader: {}", e))?;

        let mut output = String::new();
        let mut buf = [0u8; 4096];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        loop {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                output.push_str("\n[PTY: command timed out]");
                break;
            }

            // Check if child has exited
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Child exited, drain remaining output
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let chunk = String::from_utf8_lossy(&buf[..n]);
                                output.push_str(&chunk);
                                if output.len() > max_output {
                                    output.truncate(max_output);
                                    output.push_str("\n... (output truncated)");
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let exit_code = if status.success() {
                        Some(0)
                    } else {
                        Some(status.exit_code() as i32)
                    };
                    return Ok((output, exit_code));
                }
                Ok(None) => {
                    // Still running, try to read available data
                }
                Err(_) => break,
            }

            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — process likely exited
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            let exit_code = if status.success() {
                                Some(0)
                            } else {
                                Some(status.exit_code() as i32)
                            };
                            return Ok((output, exit_code));
                        }
                        _ => break,
                    }
                }
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]);
                    output.push_str(&chunk);
                    if output.len() > max_output {
                        output.truncate(max_output);
                        output.push_str("\n... (output truncated)");
                        let _ = child.kill();
                        return Ok((output, None));
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }

        // Final wait
        let status = child.wait().ok();
        let exit_code = status.and_then(|s| {
            if s.success() {
                Some(0)
            } else {
                Some(s.exit_code() as i32)
            }
        });
        Ok((output, exit_code))
    })
    .await
    .map_err(|e| anyhow::anyhow!("PTY task failed: {}", e))??;

    let (raw_output, exit_code) = result;
    let exit_code_val = exit_code.unwrap_or(-1);

    // Strip ANSI escape sequences for cleaner output
    let cleaned = strip_ansi_escapes(&raw_output);

    let mut result_text = cleaned;
    if result_text.is_empty() {
        result_text = format!("[PTY] Command completed with exit code {}", exit_code_val);
    } else if exit_code_val != 0 {
        result_text.push_str(&format!("\n[exit code: {}]", exit_code_val));
    }

    // Update registry
    {
        let mut registry = get_registry().lock().await;
        registry.append_output(session_id, "stdout", &result_text);
        let status = if exit_code_val == 0 {
            ProcessStatus::Completed
        } else {
            ProcessStatus::Failed
        };
        registry.mark_exited(session_id, Some(exit_code_val), None, status);
    }

    Ok(result_text)
}

/// Strip ANSI escape sequences from PTY output
fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC sequences
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next(); // consume '['
                                  // Read until we hit an alphabetic terminator
                    while let Some(&ch) = chars.peek() {
                        chars.next();
                        if ch.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else if next == ']' {
                    chars.next(); // consume ']'
                                  // Read until BEL or ST
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' {
                            break;
                        }
                        if ch == '\x1b' {
                            if let Some(&'\\') = chars.peek() {
                                chars.next();
                                break;
                            }
                        }
                    }
                } else {
                    chars.next(); // skip single char after ESC
                }
            }
        } else if c == '\r' {
            // Skip carriage returns (PTY uses \r\n)
            continue;
        } else {
            result.push(c);
        }
    }
    result
}
