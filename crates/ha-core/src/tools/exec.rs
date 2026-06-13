use anyhow::Result;
use serde_json::Value;
use std::process::Stdio;
#[cfg(unix)]
use std::sync::OnceLock;
use tokio_util::sync::CancellationToken;

use crate::process_registry::{
    create_session_id, get_registry, now_ms, ProcessSession, ProcessStatus,
};

use super::approval::{
    approval_timeout_action, check_and_request_approval, is_command_allowed, ApprovalCheckError,
    ApprovalResponse,
};
use super::TOOL_EXEC;

pub(crate) const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 0; // unlimited by default
pub(crate) const MAX_EXEC_TIMEOUT_SECS: u64 = 7200; // 2 hours max

/// Default output truncation (200K chars)
pub(crate) const DEFAULT_MAX_OUTPUT_CHARS: usize = 200_000;
/// Minimum output truncation for small-context models
pub(crate) const MIN_MAX_OUTPUT_CHARS: usize = 8_000;
/// Default yield window for background commands (10 seconds)
pub(crate) const DEFAULT_YIELD_MS: u64 = 10_000;
pub(crate) const MAX_YIELD_MS: u64 = 120_000;

// ── Shell Environment Resolution ──────────────────────────────────

#[cfg(unix)]
static LOGIN_SHELL_ENV: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Capture the full environment of the user's login + interactive shell.
///
/// GUI apps launched from Finder/Dock inherit a minimal environment that never
/// sources `.zprofile` / `.zshrc`, so tools installed via nvm / pyenv / brew and
/// anything the user exports (API keys, language config, …) are invisible to
/// spawned commands. We run the login shell once in *interactive* mode
/// (`-l -i`) — `.zshrc`, where nvm/pyenv/brew shellenv almost always live, is
/// only sourced for interactive shells — then snapshot every variable with
/// `env -0` (NUL-separated so values containing newlines survive intact).
///
/// Guarded by a 5s timeout with stdin closed, so a misbehaving `.zshrc` cannot
/// hang the process; returns `None` to trigger the PATH-only fallback.
#[cfg(unix)]
fn capture_login_shell_env() -> Option<Vec<(String, String)>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut child = Command::new(&shell)
        .args([
            "-l",
            "-i",
            "-c",
            "printf __HA_ENV_SNAPSHOT__; command env -0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let timeout = Duration::from_secs(5);
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                app_warn!(
                    "tool",
                    "exec",
                    "Login shell env resolution timed out after 5s"
                );
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    };
    if !status.success() {
        return None;
    }

    let mut buf = Vec::new();
    child.stdout.take()?.read_to_end(&mut buf).ok()?;
    let raw = String::from_utf8_lossy(&buf);
    // A chatty .zshrc may print to stdout before our command runs; everything
    // up to and including the marker is noise, so parse only what follows it.
    let payload = raw
        .split_once("__HA_ENV_SNAPSHOT__")
        .map(|(_, after)| after)
        .unwrap_or(&raw);
    let vars: Vec<(String, String)> = payload
        .split('\0')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            (!k.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect();
    if vars.is_empty() {
        return None;
    }
    app_info!(
        "tool",
        "exec",
        "Resolved {} env vars from login shell",
        vars.len()
    );
    Some(vars)
}

/// Full login-shell environment snapshot (Unix), cached for the process
/// lifetime. Empty on Windows or when resolution fails — callers then fall
/// back to the inherited environment plus a PATH-only probe.
#[cfg(unix)]
pub(crate) fn login_shell_env() -> &'static [(String, String)] {
    LOGIN_SHELL_ENV
        .get_or_init(|| capture_login_shell_env().unwrap_or_default())
        .as_slice()
}

#[cfg(windows)]
pub(crate) fn login_shell_env() -> &'static [(String, String)] {
    &[]
}

/// Resolve the user's login-shell PATH. Prefers the value from the full
/// environment snapshot; if that failed, probes PATH on its own (login,
/// non-interactive to avoid re-triggering a hang) as a last resort.
///
/// On Windows this returns `None` — the inherited process PATH already reflects
/// the user's HKCU + HKLM PATH; spawning a "login shell" is a Unix-only concept.
#[cfg(windows)]
pub(crate) fn get_login_shell_path() -> Option<&'static str> {
    None
}

#[cfg(unix)]
pub(crate) fn get_login_shell_path() -> Option<&'static str> {
    if let Some((_, path)) = login_shell_env().iter().find(|(k, _)| k == "PATH") {
        return Some(path.as_str());
    }
    static PATH_ONLY: OnceLock<Option<String>> = OnceLock::new();
    PATH_ONLY
        .get_or_init(|| {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let output = std::process::Command::new(&shell)
                .args(["-l", "-c", "echo $PATH"])
                .output()
                .ok()?;
            if !output.status.success() {
                app_warn!("tool", "exec", "Failed to resolve login shell PATH");
                return None;
            }
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!path.is_empty()).then_some(path)
        })
        .as_deref()
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

fn parse_exec_timeout_secs(args: &Value) -> u64 {
    match args.get("timeout").and_then(|v| v.as_u64()) {
        Some(0) => 0,
        Some(secs) => secs.min(MAX_EXEC_TIMEOUT_SECS),
        None => DEFAULT_EXEC_TIMEOUT_SECS,
    }
}

/// Shared handling for `ApprovalCheckError::TimedOut` in the exec path.
/// Returns `Ok(())` when the configured timeout action is Proceed and
/// `Err(..)` (after marking the process session Failed) when Deny.
/// Decide what to do when the exec approval dialog times out, per
/// `approval_timeout_action`. Registry-free (see
/// [`resolve_exec_command_approval`]); callers own any `ProcessSession`
/// cleanup on the `Deny` branch.
fn exec_approval_timeout_outcome(command: &str, timeout_secs: u64) -> Result<()> {
    match approval_timeout_action() {
        crate::config::ApprovalTimeoutAction::Deny => {
            app_warn!(
                "tool",
                "exec",
                "Approval timed out after {}s; blocking command execution: {}",
                timeout_secs,
                command
            );
            Err(super::rejection::ToolRejection::approval_timeout(
                TOOL_EXEC,
                timeout_secs,
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

/// Single source of truth for `exec`'s command-level approval gate: the
/// unified permission engine + the legacy AllowAlways command-prefix shortcut
/// + the interactive approval dialog + the timeout action. Returns `Ok(())`
/// when the command may run, `Err` (carrying a [`super::rejection::ToolRejection`])
/// when it must be blocked.
///
/// Deliberately does NOT touch the process-session registry: the async
/// approval-reorder path ([`super::execution::execute_tool_with_context`])
/// calls this *before* any `ProcessSession` exists, so registry cleanup on
/// denial is the caller's responsibility. `tool_exec` (the synchronous path)
/// owns a live session and marks it `Failed` on the `Err` branch.
pub(crate) async fn resolve_exec_command_approval(
    command: &str,
    args: &Value,
    ctx: &super::ToolExecContext,
    session_cwd: &str,
) -> Result<()> {
    let decision = super::execution::resolve_tool_permission(TOOL_EXEC, args, ctx, false).await;
    match decision {
        crate::permission::Decision::Allow => Ok(()),
        crate::permission::Decision::Deny { reason } => {
            app_warn!(
                "tool",
                "exec",
                "Command execution denied by policy ({}): {}",
                reason,
                command
            );
            Err(super::rejection::ToolRejection::denied_by_policy(
                TOOL_EXEC, reason,
            ))
        }
        crate::permission::Decision::Ask { reason } => {
            let allow_always_ok = !reason.forbids_allow_always();
            // Legacy command-prefix shortcut still applies for non-strict ask
            // reasons — once the user has AllowAlways'd `git status`, future
            // `git status *` skips the dialog.
            if allow_always_ok && is_command_allowed(command).await {
                app_info!(
                    "tool",
                    "exec",
                    "Command auto-approved by allowlist prefix: {}",
                    command
                );
                return Ok(());
            }
            let reason_payload = Some(super::approval::ApprovalReasonPayload::from(&reason));
            match check_and_request_approval(
                command,
                session_cwd,
                ctx.session_id.as_deref(),
                reason_payload,
            )
            .await
            {
                Ok(ApprovalResponse::AllowOnce) => {
                    app_info!("tool", "exec", "Command approved (once): {}", command);
                    Ok(())
                }
                Ok(ApprovalResponse::AllowAlways) => {
                    if allow_always_ok {
                        app_info!("tool", "exec", "Command approved (always): {}", command);
                        if let Err(e) = crate::permission::allowlist::add_allow_always_for_call(
                            TOOL_EXEC,
                            args,
                            ctx.allowlist_grant_context(),
                        ) {
                            app_warn!(
                                "tool",
                                "exec",
                                "Command AllowAlways persistence failed: {}",
                                e
                            );
                        }
                    } else {
                        app_info!(
                            "tool",
                            "exec",
                            "Command approved once (AllowAlways unavailable: {:?}): {}",
                            reason,
                            command
                        );
                    }
                    Ok(())
                }
                Ok(ApprovalResponse::Deny) => {
                    app_warn!("tool", "exec", "Command execution denied by user: {}", command);
                    Err(super::rejection::ToolRejection::denied_by_user(TOOL_EXEC))
                }
                Err(ApprovalCheckError::TimedOut { timeout_secs }) => {
                    exec_approval_timeout_outcome(command, timeout_secs)
                }
                Err(e) => {
                    app_warn!(
                        "tool",
                        "exec",
                        "Approval check failed ({}); blocking command execution: {}",
                        e,
                        command
                    );
                    Err(super::rejection::ToolRejection::approval_failed(
                        TOOL_EXEC,
                        e.to_string(),
                    ))
                }
            }
        }
    }
}

#[derive(Debug)]
enum ExecWaitError {
    Wait(std::io::Error),
    Timeout { timeout_secs: u64 },
}

/// RAII guard that SIGKILLs the entire process group on drop unless
/// `disarm()` was called. Replaces tokio's `kill_on_drop(true)` for
/// the exec path: `kill_on_drop` only signals the immediate child,
/// which leaks grandchildren spawned by shells like
/// `sh -c 'long_task & long_task & wait'`. We instead signal the
/// whole process group with `kill(-pid, SIGKILL)` so all descendants
/// die together.
///
/// Triggered by:
/// - tokio task panic (the spawned waiter future)
/// - tokio runtime shutdown dropping live tasks
/// - early-return after `cmd.spawn()` failed paths
/// `disarm()` is called once the waiter has explicitly `wait_with_output`
/// or `terminate_process_tree`d the child — at that point the process
/// group is already gone and signaling it again is a no-op anyway.
struct ProcessGroupGuard(Option<u32>);

impl ProcessGroupGuard {
    fn new(pid: Option<u32>) -> Self {
        Self(pid)
    }

    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            crate::platform::terminate_process_tree(pid);
        }
    }
}

async fn spawn_exec_waiter(
    session_id: String,
    mut cmd: tokio::process::Command,
    timeout_secs: u64,
    max_output: usize,
) -> Result<tokio::task::JoinHandle<Result<String>>> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);
    // Note: we don't use `cmd.kill_on_drop(true)` here — that only
    // SIGKILLs the immediate child. `ProcessGroupGuard` below handles
    // the full process tree.
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let mut registry = get_registry().lock().await;
            registry.mark_exited(&session_id, None, None, ProcessStatus::Failed);
            return Err(anyhow::anyhow!("Failed to execute command: {}", e));
        }
    };
    let pid = child.id();
    // Arm the process-group guard *immediately* after `spawn()` so the
    // registry-lock `await` below can't leave a stray process group
    // behind if this future gets cancelled before we hand the child
    // off to the waiter task.
    let guard = ProcessGroupGuard::new(pid);
    let cancelled_before_pid_registered = {
        let mut registry = get_registry().lock().await;
        let already_exited = registry
            .get_session(&session_id)
            .map(|session| session.exited)
            .unwrap_or(true);
        registry.set_pid(&session_id, pid);
        already_exited
    };
    if cancelled_before_pid_registered {
        if let Some(pid) = pid {
            crate::platform::terminate_process_tree(pid);
        }
        // The waiter never runs — the explicit terminate above already
        // tore down the process group; disarm so Drop doesn't signal a
        // dead group.
        guard.disarm();
        return Err(anyhow::anyhow!(
            "Exec session cancelled before pid registration"
        ));
    }

    Ok(tokio::spawn(async move {
        // Guard moved in; it will SIGKILL the process group if the
        // waiter task is dropped (panic / runtime shutdown).
        let guard = guard;
        let result = if timeout_secs == 0 {
            child.wait_with_output().await.map_err(ExecWaitError::Wait)
        } else {
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                child.wait_with_output(),
            )
            .await
            {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(e)) => Err(ExecWaitError::Wait(e)),
                Err(_) => {
                    if let Some(pid) = pid {
                        crate::platform::terminate_process_tree(pid);
                    }
                    Err(ExecWaitError::Timeout { timeout_secs })
                }
            }
        };
        // Reaching here means we either got an exit (process tree gone)
        // or already SIGKILLed it via the timeout branch above. Either
        // way, dropping the guard would be a no-op or redundant — disarm.
        guard.disarm();
        finish_exec_sync(&session_id, result, max_output).await
    }))
}

fn spawn_exec_cancel_watcher(session_id: String, token: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    let pid = {
                        let mut registry = get_registry().lock().await;
                        let Some(session) = registry.get_session(&session_id) else {
                            return;
                        };
                        if session.exited {
                            return;
                        }
                        let pid = session.pid;
                        registry.mark_exited(
                            &session_id,
                            None,
                            Some("cancelled".to_string()),
                            ProcessStatus::Failed,
                        );
                        pid
                    };
                    if let Some(pid) = pid {
                        crate::platform::terminate_process_tree(pid);
                    }
                    return;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    let registry = get_registry().lock().await;
                    match registry.get_session(&session_id) {
                        Some(session) if !session.exited => {}
                        _ => return,
                    }
                }
            }
        }
    });
}

pub(crate) async fn tool_exec(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|raw| ctx.resolve_path(raw));

    let timeout_secs = parse_exec_timeout_secs(args);

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
    let session_cwd = cwd.clone().unwrap_or_else(|| ctx.default_cwd());

    app_info!(
        "tool",
        "exec",
        "Executing command: {} (cwd: {}, timeout: {}s, bg: {}, pty: {}, max_out: {})",
        command,
        session_cwd,
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
                    "cwd": &session_cwd, "explicitCwd": &cwd, "timeout": timeout_secs,
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

    cmd.current_dir(&session_cwd);

    // Inject the user's full login-shell environment (PATH plus everything they
    // export in .zprofile/.zshrc) so commands resolve like they do in a real
    // terminal. Falls back to a PATH-only injection if the snapshot is empty.
    let shell_env = login_shell_env();
    if shell_env.is_empty() {
        if let Some(shell_path) = get_login_shell_path() {
            cmd.env("PATH", shell_path);
        }
    } else {
        cmd.envs(shell_env.iter().map(|(k, v)| (k, v)));
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
    let session = ProcessSession {
        id: session_id.clone(),
        parent_session_id: ctx.session_id.clone(),
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
    if let Some(token) = ctx.cancellation_token.clone() {
        spawn_exec_cancel_watcher(session_id.clone(), token);
    }

    // ── Command approval gate ───────────────────────────────────
    // Single source of truth: `resolve_exec_command_approval` (unified
    // permission engine + legacy AllowAlways command-prefix shortcut +
    // interactive approval + timeout action). The async approval-reorder path
    // runs the SAME gate *before* detaching (see
    // `execution::execute_tool_with_context`) and then sets `exec_pre_approved`
    // so this inner call is skipped — the user approves the command once,
    // before any synthetic "started" job id is returned (ASYNC-1 / HOOKS-2).
    //
    // Gate predicate is `ToolExecContext::should_run_exec_command_gate` — it
    // ignores `external_pre_approved` (async-job re-entry silences only the
    // *engine* gate, never this command-level audit) and honors
    // `exec_pre_approved` (set only after the reorder already ran this gate).
    if ctx.should_run_exec_command_gate() {
        if let Err(e) = resolve_exec_command_approval(command, args, ctx, &session_cwd).await {
            // The reorder path owns no `ProcessSession`; here in `tool_exec` one
            // already exists, so mark it `Failed` before surfacing the rejection.
            let mut registry = get_registry().lock().await;
            registry.mark_exited(&session_id, None, None, ProcessStatus::Failed);
            return Err(e);
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
            let cancellation_token = ctx.cancellation_token.clone();

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
                    cancellation_token,
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
            ctx.cancellation_token.clone(),
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
                if crate::truncate_string_utf8(&mut result_text, max_output) {
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

    let mut exec_handle =
        spawn_exec_waiter(session_id.clone(), cmd, timeout_secs, max_output).await?;

    // If background=true, return immediately after the process is spawned and
    // registered. The detached waiter updates the process registry on exit.
    if background {
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

    // If yield_ms is specified (and not default 10s for non-background), use it
    let wants_yield = args.get("yield_ms").is_some();

    if wants_yield {
        // Wait yield_ms, if not done, leave the already-spawned waiter
        // detached and mark the session as backgrounded.
        let yield_duration = std::time::Duration::from_millis(yield_ms);

        match tokio::time::timeout(yield_duration, &mut exec_handle).await {
            Ok(joined) => return joined.map_err(|e| anyhow::anyhow!("Exec task failed: {}", e))?,
            Err(_) => {
                // yield_ms elapsed, command still running — background it
                {
                    let mut registry = get_registry().lock().await;
                    if let Some(s) = registry.get_session_mut(&session_id) {
                        s.backgrounded = true;
                    }
                }

                return Ok(format!(
                    "Command still running after {}ms (session {}). Use process(action=\"poll\", session_id=\"{}\") to check status.",
                    yield_ms, session_id, session_id
                ));
            }
        }
    }

    // Standard synchronous execution
    exec_handle
        .await
        .map_err(|e| anyhow::anyhow!("Exec task failed: {}", e))?
}

/// Finish a synchronous exec and return result
async fn finish_exec_sync(
    session_id: &str,
    result: std::result::Result<std::process::Output, ExecWaitError>,
    max_output: usize,
) -> Result<String> {
    match result {
        Ok(output) => {
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
            if crate::truncate_string_utf8(&mut result_text, max_output) {
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
        Err(ExecWaitError::Wait(e)) => {
            let mut registry = get_registry().lock().await;
            registry.mark_exited(session_id, None, None, ProcessStatus::Failed);
            Err(anyhow::anyhow!("Failed to execute command: {}", e))
        }
        Err(ExecWaitError::Timeout { timeout_secs }) => {
            let mut registry = get_registry().lock().await;
            registry.mark_exited(
                session_id,
                None,
                Some("timeout".to_string()),
                ProcessStatus::Failed,
            );
            Err(anyhow::anyhow!(
                "Command timed out after {}s. If this command is expected to take longer, re-run with a higher timeout (e.g., exec timeout=3600), or timeout=0 to disable the exec command timeout.",
                timeout_secs
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
    let default_cwd_owned = ctx.default_cwd();
    let env_vars: Vec<(String, String)> = args
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let shell_env: Vec<(String, String)> = login_shell_env().to_vec();
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

        let effective_cwd = cwd_owned.as_deref().unwrap_or(&default_cwd_owned);
        cmd.cwd(effective_cwd);

        // Inject the user's full login-shell environment; fall back to PATH only.
        if shell_env.is_empty() {
            if let Some(ref path) = login_path {
                cmd.env("PATH", path);
            }
        } else {
            for (k, v) in &shell_env {
                cmd.env(k, v);
            }
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
        let deadline = (timeout_secs > 0)
            .then(|| std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs));

        loop {
            if deadline
                .map(|deadline| std::time::Instant::now() >= deadline)
                .unwrap_or(false)
            {
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
                                if crate::truncate_string_utf8(&mut output, max_output) {
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
                    if crate::truncate_string_utf8(&mut output, max_output) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exec_timeout_defaults_to_unlimited_and_clamps_positive_values() {
        assert_eq!(
            parse_exec_timeout_secs(&json!({})),
            DEFAULT_EXEC_TIMEOUT_SECS
        );
        assert_eq!(parse_exec_timeout_secs(&json!({ "timeout": 3600 })), 3600);
        assert_eq!(
            parse_exec_timeout_secs(&json!({ "timeout": 99_999 })),
            MAX_EXEC_TIMEOUT_SECS
        );
    }

    #[test]
    fn exec_timeout_zero_means_unlimited() {
        assert_eq!(parse_exec_timeout_secs(&json!({ "timeout": 0 })), 0);
    }
}
