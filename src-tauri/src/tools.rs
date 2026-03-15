use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

use crate::process_registry::{
    ProcessSession, ProcessStatus, create_session_id, derive_session_name,
    format_duration_compact, get_registry, now_ms,
};

// ── Command Approval System ───────────────────────────────────────

/// Approval request sent to frontend
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub command: String,
    pub cwd: String,
}

/// Approval response from frontend
#[derive(Debug, Clone, Deserialize)]
pub enum ApprovalResponse {
    AllowOnce,
    AllowAlways,  // adds command pattern to allowlist
    Deny,
}

/// Global approval request registry
static PENDING_APPROVALS: OnceLock<TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<ApprovalResponse>>>> = OnceLock::new();

fn get_pending_approvals() -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<ApprovalResponse>>> {
    PENDING_APPROVALS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Submit an approval response (called by Tauri command from frontend)
pub async fn submit_approval_response(request_id: &str, response: ApprovalResponse) -> Result<()> {
    let mut pending = get_pending_approvals().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(response);
        Ok(())
    } else {
        Err(anyhow::anyhow!("No pending approval request: {}", request_id))
    }
}

/// Allowlist: command prefixes that are auto-approved
static COMMAND_ALLOWLIST: OnceLock<TokioMutex<Vec<String>>> = OnceLock::new();

fn get_allowlist() -> &'static TokioMutex<Vec<String>> {
    COMMAND_ALLOWLIST.get_or_init(|| {
        let list = load_allowlist().unwrap_or_default();
        TokioMutex::new(list)
    })
}

fn allowlist_path() -> std::path::PathBuf {
    crate::paths::root_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")).join("exec-approvals.json")
}

fn load_allowlist() -> Result<Vec<String>> {
    let path = allowlist_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(Vec::new())
    }
}

async fn save_allowlist(list: &[String]) -> Result<()> {
    let data = serde_json::to_string_pretty(list)?;
    tokio::fs::write(allowlist_path(), data).await?;
    Ok(())
}

/// Check if command is in the allowlist
async fn is_command_allowed(command: &str) -> bool {
    let list = get_allowlist().lock().await;
    let cmd_trimmed = command.trim();
    list.iter().any(|pattern| {
        cmd_trimmed.starts_with(pattern) || cmd_trimmed == *pattern
    })
}

/// Add command prefix to allowlist
async fn add_to_allowlist(command: &str) {
    let mut list = get_allowlist().lock().await;
    // Extract command prefix (first word or up to first pipe/semicolon)
    let prefix = extract_command_prefix(command);
    if !list.contains(&prefix) {
        list.push(prefix);
        let _ = save_allowlist(&list).await;
    }
}

/// Extract a meaningful command prefix for the allowlist
fn extract_command_prefix(command: &str) -> String {
    let trimmed = command.trim();
    // Take first word as the prefix
    trimmed.split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

/// Request approval from the user for a command.
/// Emits a Tauri event and waits for the response via oneshot channel.
async fn check_and_request_approval(command: &str, cwd: &str) -> Result<ApprovalResponse> {
    use tauri::Emitter;

    let request_id = create_session_id();
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Register the pending approval
    {
        let mut pending = get_pending_approvals().lock().await;
        pending.insert(request_id.clone(), tx);
    }

    // Emit event to frontend
    let request = ApprovalRequest {
        request_id: request_id.clone(),
        command: command.to_string(),
        cwd: cwd.to_string(),
    };

    if let Some(handle) = crate::get_app_handle() {
        let event_data = serde_json::to_string(&request)?;
        handle.emit("approval_required", event_data)
            .map_err(|e| anyhow::anyhow!("Failed to emit approval event: {}", e))?;
        log::info!("Approval requested for command: {} (id: {})", command, request_id);
    } else {
        // No AppHandle available, clean up and return error
        let mut pending = get_pending_approvals().lock().await;
        pending.remove(&request_id);
        return Err(anyhow::anyhow!("AppHandle not available for approval events"));
    }

    // Wait for response with timeout (5 minutes)
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => {
            // Channel dropped (sender was removed)
            Err(anyhow::anyhow!("Approval request cancelled"))
        }
        Err(_) => {
            // Timeout — clean up
            let mut pending = get_pending_approvals().lock().await;
            pending.remove(&request_id);
            Err(anyhow::anyhow!("Approval request timed out (5 min)"))
        }
    }
}

const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 1800; // 30 minutes, aligned with OpenClaw
const MAX_EXEC_TIMEOUT_SECS: u64 = 7200; // 2 hours max
// TODO: Read user-configured default timeout from ~/.opencomputer/config.json

/// Default output truncation (200K chars, aligned with OpenClaw's DEFAULT_MAX_OUTPUT)
const DEFAULT_MAX_OUTPUT_CHARS: usize = 200_000;
/// Minimum output truncation for small-context models
const MIN_MAX_OUTPUT_CHARS: usize = 8_000;
/// Default yield window for background commands (10 seconds)
const DEFAULT_YIELD_MS: u64 = 10_000;
const MAX_YIELD_MS: u64 = 120_000;

// ── Shell PATH Resolution ─────────────────────────────────────────

static LOGIN_SHELL_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Resolve the full PATH from the user's login shell.
/// This ensures tools like npm, python, etc. are available even when
/// launched from a desktop environment that doesn't source .bashrc/.zshrc.
fn get_login_shell_path() -> Option<&'static str> {
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
                    log::info!("Resolved login shell PATH: {}", &path[..path.len().min(120)]);
                    Some(path)
                } else {
                    None
                }
            } else {
                log::warn!("Failed to resolve login shell PATH");
                None
            }
        })
        .as_deref()
}

/// Compute dynamic max output chars based on model context window.
/// Uses ~20% of context window (at ~4 chars/token estimate).
fn compute_max_output_chars(context_window_tokens: Option<u32>) -> usize {
    match context_window_tokens {
        Some(tokens) if tokens > 0 => {
            let chars_from_context = (tokens as usize) * 4 / 5; // 20% of context * 4 chars/token
            chars_from_context.clamp(MIN_MAX_OUTPUT_CHARS, DEFAULT_MAX_OUTPUT_CHARS)
        }
        _ => DEFAULT_MAX_OUTPUT_CHARS,
    }
}

// ── Provider Enum ─────────────────────────────────────────────────

/// Supported LLM provider types for tool schema adaptation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAI,
}

// ── Tool Definition (provider-agnostic) ───────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool parameters
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn to_anthropic_schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters,
        })
    }

    pub fn to_openai_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.parameters,
        })
    }

    pub fn to_provider_schema(&self, provider: ToolProvider) -> Value {
        match provider {
            ToolProvider::Anthropic => self.to_anthropic_schema(),
            ToolProvider::OpenAI => self.to_openai_schema(),
        }
    }
}

// ── Tool Catalog ──────────────────────────────────────────────────

pub fn get_available_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "exec".into(),
            description: "Execute a shell command. Returns stdout/stderr. Supports background execution with yield_ms/background params.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory for the command. Defaults to user home directory."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (max 7200). Defaults to 1800 (30 min)."
                    },
                    "env": {
                        "type": "object",
                        "description": "Environment variables to set (key-value pairs)",
                        "additionalProperties": { "type": "string" }
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Run in background immediately, return session ID"
                    },
                    "yield_ms": {
                        "type": "integer",
                        "description": "Milliseconds to wait before backgrounding (default 10000). If command finishes before this, returns result directly."
                    },
                    "pty": {
                        "type": "boolean",
                        "description": "Run in a pseudo-terminal (PTY) for TTY-required commands (interactive CLIs, coding agents). Falls back to normal mode if PTY unavailable."
                    },
                    "sandbox": {
                        "type": "boolean",
                        "description": "Run command in a Docker sandbox container for isolation. Requires Docker to be installed and running. The working directory is mounted into the container."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "process".into(),
            description: "Manage running exec sessions: list, poll, log, write, kill, clear, remove.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Action: list, poll, log, write, kill, clear, remove",
                        "enum": ["list", "poll", "log", "write", "kill", "clear", "remove"]
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Session ID (required for all actions except list)"
                    },
                    "data": {
                        "type": "string",
                        "description": "Data to write to stdin (for write action)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "For poll: wait up to this many milliseconds before returning"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "For log: line offset"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "For log: max lines to return"
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the contents of a file at the specified path.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to read"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file at the specified path. Creates parent directories if needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "patch_file".into(),
            description: "Edit a file by replacing specific text. More precise than write_file for making targeted changes. The old_text must match exactly (including whitespace and indentation).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Exact text to find and replace (must match exactly)"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text"
                    }
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List files and directories in the specified path. Returns names with type indicators.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list. Defaults to current directory if not specified."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "web_search".into(),
            description: "Search the web for information. Returns relevant results with titles, URLs, and snippets. Use this when the user asks about current events, recent information, or anything that requires up-to-date knowledge.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query string"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results to return (1-10, default 5)"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch and extract readable content from a URL. Returns the page content as cleaned text. Use this to read web pages, documentation, articles, or API responses.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to fetch"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum characters to return (default 50000)"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Returns all tool schemas formatted for the given provider
pub fn get_tools_for_provider(provider: ToolProvider) -> Vec<Value> {
    get_available_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}

// ── Tool Execution Context ────────────────────────────────────────

/// Context passed to tool execution for dynamic behavior
#[derive(Debug, Clone)]
pub struct ToolExecContext {
    /// Model context window in tokens (for dynamic output truncation)
    pub context_window_tokens: Option<u32>,
}

impl Default for ToolExecContext {
    fn default() -> Self {
        Self {
            context_window_tokens: None,
        }
    }
}

// ── Tool Execution (provider-agnostic) ────────────────────────────

/// Execute a tool by name with the given JSON arguments.
pub async fn execute_tool(name: &str, args: &Value) -> Result<String> {
    execute_tool_with_context(name, args, &ToolExecContext::default()).await
}

/// Execute a tool with additional context (model info, etc.)
pub async fn execute_tool_with_context(name: &str, args: &Value, ctx: &ToolExecContext) -> Result<String> {
    match name {
        "exec" => tool_exec(args, ctx).await,
        "process" => tool_process(args).await,
        "read_file" => tool_read_file(args).await,
        "write_file" => tool_write_file(args).await,
        "patch_file" => tool_patch_file(args).await,
        "list_dir" => tool_list_dir(args).await,
        "web_search" => tool_web_search(args).await,
        "web_fetch" => tool_web_fetch(args).await,
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

// ── exec Tool ─────────────────────────────────────────────────────

async fn tool_exec(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

    let cwd = args.get("cwd").and_then(|v| v.as_str());

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_EXEC_TIMEOUT_SECS)
        .min(MAX_EXEC_TIMEOUT_SECS);

    let background = args.get("background").and_then(|v| v.as_bool()).unwrap_or(false);
    let use_pty = args.get("pty").and_then(|v| v.as_bool()).unwrap_or(false);
    let sandbox = args.get("sandbox").and_then(|v| v.as_bool()).unwrap_or(false);

    let yield_ms = args
        .get("yield_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_YIELD_MS)
        .min(MAX_YIELD_MS);

    let max_output = compute_max_output_chars(ctx.context_window_tokens);

    log::info!(
        "Executing command: {} (cwd: {:?}, timeout: {}s, bg: {}, pty: {}, max_out: {})",
        command, cwd, timeout_secs, background, use_pty, max_output
    );

    // Build the command
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);

    // Set working directory
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
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
        .map(|s| s.to_string())
        .unwrap_or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| ".".to_string()));

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
    // Check if command needs approval before execution
    if !is_command_allowed(command).await {
        match check_and_request_approval(command, &session_cwd).await {
            Ok(ApprovalResponse::AllowOnce) => {
                log::info!("Command approved (once): {}", command);
            }
            Ok(ApprovalResponse::AllowAlways) => {
                log::info!("Command approved (always): {}", command);
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
            Err(e) => {
                log::warn!("Approval check failed ({}), proceeding with execution", e);
                // If approval system is unavailable, allow by default for now
            }
        }
    }

    // ── Docker sandbox execution path ─────────────────────────
    if sandbox {
        log::info!("Using Docker sandbox for command: {}", command);
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
                ).await;

                let mut registry = get_registry().lock().await;
                match result {
                    Ok(sr) => {
                        let combined = if sr.stderr.is_empty() {
                            sr.stdout.clone()
                        } else {
                            format!("{}\n[stderr] {}", sr.stdout, sr.stderr)
                        };
                        registry.append_output(&sid, "stdout", &combined);
                        let status = if sr.exit_code == 0 { ProcessStatus::Completed } else { ProcessStatus::Failed };
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
        ).await {
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
                    result_text.push_str(&format!("\n[sandbox: command timed out after {}s]", timeout_secs));
                } else if result_text.is_empty() {
                    result_text = format!("[sandbox] Command completed with exit code {}", sr.exit_code);
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
                    let status = if sr.exit_code == 0 { ProcessStatus::Completed } else { ProcessStatus::Failed };
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
        log::info!("Using PTY mode for command: {}", command);
        match exec_via_pty(command, cwd, args, timeout_secs, max_output, &session_id).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                log::warn!("PTY execution failed ({}), falling back to normal mode", e);
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
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                cmd.output(),
            ).await;
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
                    let status = if exit_code == 0 { ProcessStatus::Completed } else { ProcessStatus::Failed };
                    registry.mark_exited(&sid, Some(exit_code), None, status);
                }
                Ok(Err(e)) => {
                    registry.append_output(&sid, "stderr", &format!("Failed to execute: {}", e));
                    registry.mark_exited(&sid, None, None, ProcessStatus::Failed);
                }
                Err(_) => {
                    registry.append_output(&sid, "stderr", &format!("Command timed out after {}s", timeout));
                    registry.mark_exited(&sid, None, Some("SIGKILL".to_string()), ProcessStatus::Failed);
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
    let cmd_future = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        cmd.output(),
    );

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
    result: std::result::Result<std::result::Result<std::process::Output, std::io::Error>, tokio::time::error::Elapsed>,
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
                let status = if exit_code == 0 { ProcessStatus::Completed } else { ProcessStatus::Failed };
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
            registry.mark_exited(session_id, None, Some("timeout".to_string()), ProcessStatus::Failed);
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
) -> Result<String> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::Read;

    let command_owned = command.to_string();
    let cwd_owned = cwd.map(|s| s.to_string());
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
            .map_err(|e| anyhow::anyhow!("Failed to open PTY: {}", e))?;

        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg(&command_owned);

        // Set working directory
        if let Some(ref dir) = cwd_owned {
            cmd.cwd(dir);
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
                    let exit_code = if status.success() { Some(0) } else { Some(status.exit_code() as i32) };
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
                            let exit_code = if status.success() { Some(0) } else { Some(status.exit_code() as i32) };
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
        let exit_code = status.and_then(|s| if s.success() { Some(0) } else { Some(s.exit_code() as i32) });
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
        let status = if exit_code_val == 0 { ProcessStatus::Completed } else { ProcessStatus::Failed };
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
                        if ch == '\x07' { break; }
                        if ch == '\x1b' {
                            if let Some(&'\\') = chars.peek() { chars.next(); break; }
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

// ── process Tool ──────────────────────────────────────────────────

async fn tool_process(args: &Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "list" => tool_process_list().await,
        "poll" => {
            let session_id = require_session_id(args)?;
            let timeout_ms = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(0).min(120_000);
            tool_process_poll(&session_id, timeout_ms).await
        }
        "log" => {
            let session_id = require_session_id(args)?;
            let offset = args.get("offset").and_then(|v| v.as_u64()).map(|v| v as usize);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            tool_process_log(&session_id, offset, limit).await
        }
        "write" => {
            let session_id = require_session_id(args)?;
            let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
            tool_process_write(&session_id, data).await
        }
        "kill" => {
            let session_id = require_session_id(args)?;
            tool_process_kill(&session_id).await
        }
        "clear" | "remove" => {
            let session_id = require_session_id(args)?;
            tool_process_remove(&session_id).await
        }
        _ => Err(anyhow::anyhow!("Unknown process action: {}", action)),
    }
}

fn require_session_id(args: &Value) -> Result<String> {
    args.get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("session_id is required for this action"))
}

async fn tool_process_list() -> Result<String> {
    let registry = get_registry().lock().await;
    let mut sessions: Vec<_> = registry.list_all().into_iter().cloned().collect();
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if sessions.is_empty() {
        return Ok("No running or recent sessions.".to_string());
    }

    let now = now_ms();
    let lines: Vec<String> = sessions
        .iter()
        .map(|s| {
            let runtime = now.saturating_sub(s.started_at);
            let name = derive_session_name(&s.command);
            format!(
                "{} {:>9} {:>8} :: {}",
                s.id,
                s.status.to_string(),
                format_duration_compact(runtime),
                name
            )
        })
        .collect();

    Ok(lines.join("\n"))
}

async fn tool_process_poll(session_id: &str, timeout_ms: u64) -> Result<String> {
    // Wait for new output or timeout
    if timeout_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let registry = get_registry().lock().await;
                if let Some(session) = registry.get_session(session_id) {
                    if session.exited || !session.pending_stdout.is_empty() || !session.pending_stderr.is_empty() {
                        break;
                    }
                } else {
                    return Err(anyhow::anyhow!("No session found for {}", session_id));
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    let mut registry = get_registry().lock().await;
    let (stdout, stderr) = registry.drain_output(session_id);

    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    let mut output = String::new();
    if !stdout.is_empty() {
        output.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !output.is_empty() { output.push('\n'); }
        output.push_str(&stderr);
    }

    if session.exited {
        let exit_info = if let Some(signal) = &session.exit_signal {
            format!("signal {}", signal)
        } else {
            format!("code {}", session.exit_code.unwrap_or(0))
        };

        if output.is_empty() {
            output = format!("(no new output)\n\nProcess exited with {}.", exit_info);
        } else {
            output.push_str(&format!("\n\nProcess exited with {}.", exit_info));
        }
    } else if output.is_empty() {
        output = "(no new output)\n\nProcess still running.".to_string();
    } else {
        output.push_str("\n\nProcess still running.");
    }

    Ok(output)
}

async fn tool_process_log(session_id: &str, offset: Option<usize>, limit: Option<usize>) -> Result<String> {
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    let log_text = &session.aggregated_output;
    if log_text.is_empty() {
        return Ok("(no output recorded)".to_string());
    }

    let lines: Vec<&str> = log_text.lines().collect();
    let total = lines.len();
    let default_tail = 200;

    let start = offset.unwrap_or_else(|| total.saturating_sub(limit.unwrap_or(default_tail)));
    let end = limit.map(|l| (start + l).min(total)).unwrap_or(total);

    let slice: String = lines[start..end].join("\n");

    let mut result = if slice.is_empty() {
        "(no output in range)".to_string()
    } else {
        slice
    };

    if offset.is_none() && limit.is_none() && total > default_tail {
        result.push_str(&format!(
            "\n\n[showing last {} of {} lines; pass offset/limit to page]",
            default_tail, total
        ));
    }

    Ok(result)
}

async fn tool_process_write(session_id: &str, _data: &str) -> Result<String> {
    // TODO: Phase 3 will implement stdin writing via PTY/process supervisor
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    if session.exited {
        return Err(anyhow::anyhow!("Session {} has already exited", session_id));
    }

    Ok(format!(
        "Write to stdin is not yet supported in this version. Session {} is still running. Use kill to terminate.",
        session_id
    ))
}

async fn tool_process_kill(session_id: &str) -> Result<String> {
    let mut registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    if session.exited {
        return Ok(format!("Session {} has already exited.", session_id));
    }

    if let Some(pid) = session.pid {
        // Kill the process and its children
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = pid; // suppress unused warning on non-unix
        }
    }

    registry.mark_exited(session_id, None, Some("SIGKILL".to_string()), ProcessStatus::Failed);
    Ok(format!("Terminated session {}.", session_id))
}

async fn tool_process_remove(session_id: &str) -> Result<String> {
    let mut registry = get_registry().lock().await;
    if registry.remove_session(session_id).is_some() {
        Ok(format!("Removed session {}.", session_id))
    } else {
        Err(anyhow::anyhow!("No session found for {}", session_id))
    }
}

// ── File Tools ────────────────────────────────────────────────────

async fn tool_read_file(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    log::info!("Reading file: {}", path);

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    const MAX_FILE_LEN: usize = 32000;
    if content.len() > MAX_FILE_LEN {
        let truncated = &content[..MAX_FILE_LEN];
        Ok(format!("{}\n... (file truncated, {} bytes total)", truncated, content.len()))
    } else {
        Ok(content)
    }
}

async fn tool_write_file(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    log::info!("Writing file: {}", path);

    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create directories: {}", e))?;
    }

    tokio::fs::write(path, content)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {}", path, e))?;

    Ok(format!("Successfully wrote {} bytes to {}", content.len(), path))
}

async fn tool_patch_file(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let old_text = args
        .get("old_text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' parameter"))?;
    let new_text = args
        .get("new_text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'new_text' parameter"))?;

    log::info!("Patching file: {}", path);

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    let count = content.matches(old_text).count();
    if count == 0 {
        return Err(anyhow::anyhow!(
            "old_text not found in '{}'. Make sure the text matches exactly (including whitespace and indentation).",
            path
        ));
    }
    if count > 1 {
        return Err(anyhow::anyhow!(
            "old_text found {} times in '{}'. Please provide more context to make the match unique.",
            count, path
        ));
    }

    let new_content = content.replacen(old_text, new_text, 1);

    tokio::fs::write(path, &new_content)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {}", path, e))?;

    Ok(format!("Successfully patched {} (replaced 1 occurrence)", path))
}

async fn tool_list_dir(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    log::info!("Listing directory: {}", path);

    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read directory '{}': {}", path, e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().await?;
        let indicator = if file_type.is_dir() {
            "/"
        } else if file_type.is_symlink() {
            "@"
        } else {
            ""
        };
        items.push(format!("{}{}", name, indicator));
    }

    items.sort();

    if items.is_empty() {
        Ok(format!("Directory '{}' is empty", path))
    } else {
        Ok(items.join("\n"))
    }
}

// ── Web Tools ─────────────────────────────────────────────────────

const WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const DEFAULT_WEB_FETCH_MAX_CHARS: usize = 50000;
const WEB_FETCH_TIMEOUT_SECS: u64 = 30;

async fn tool_web_search(args: &Value) -> Result<String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10) as usize;

    log::info!("Web search: {} (count: {})", query, count);

    let client = reqwest::Client::builder()
        .user_agent(WEB_FETCH_USER_AGENT)
        .timeout(std::time::Duration::from_secs(WEB_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let search_url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let resp = client
        .get(&search_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Search request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Search failed with status: {}", resp.status()));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read search response: {}", e))?;

    let results = parse_ddg_results(&html, count);

    if results.is_empty() {
        return Ok(format!("No results found for: {}", query));
    }

    let mut output = format!("Search results for: {}\n\n", query);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1, result.title, result.url, result.snippet
        ));
    }

    Ok(output)
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < max_results {
        let link_marker = "class=\"result__a\"";
        let link_start = match html[pos..].find(link_marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        let href_start = match html[..link_start].rfind("href=\"") {
            Some(idx) => idx + 6,
            None => { pos = link_start + link_marker.len(); continue; }
        };
        let href_end = match html[href_start..].find('"') {
            Some(idx) => href_start + idx,
            None => { pos = link_start + link_marker.len(); continue; }
        };
        let raw_url = &html[href_start..href_end];
        let url = extract_ddg_url(raw_url);

        let title_start = match html[link_start..].find('>') {
            Some(idx) => link_start + idx + 1,
            None => { pos = link_start + link_marker.len(); continue; }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(idx) => title_start + idx,
            None => { pos = link_start + link_marker.len(); continue; }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snippet_start) = html[title_end..].find(snippet_marker) {
            let abs_snippet_start = title_end + snippet_start;
            if let Some(tag_end) = html[abs_snippet_start..].find('>') {
                let content_start = abs_snippet_start + tag_end + 1;
                if let Some(end) = html[content_start..].find("</a>") {
                    strip_html_tags(&html[content_start..content_start + end])
                } else { String::new() }
            } else { String::new() }
        } else { String::new() };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet: html_decode(&snippet),
            });
        }

        pos = title_end;
    }

    results
}

fn extract_ddg_url(raw: &str) -> String {
    if let Some(uddg_start) = raw.find("uddg=") {
        let url_start = uddg_start + 5;
        let url_end = raw[url_start..]
            .find('&')
            .map(|i| url_start + i)
            .unwrap_or(raw.len());
        let encoded = &raw[url_start..url_end];
        urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| encoded.to_string())
    } else if raw.starts_with("http") {
        raw.to_string()
    } else {
        raw.to_string()
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' { in_tag = true; }
        else if c == '>' { in_tag = false; }
        else if !in_tag { result.push(c); }
    }
    result.trim().to_string()
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

async fn tool_web_fetch(args: &Value) -> Result<String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_WEB_FETCH_MAX_CHARS as u64) as usize;

    log::info!("Fetching URL: {} (max_chars: {})", url, max_chars);

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow::anyhow!("Invalid URL: must start with http:// or https://"));
    }

    let client = reqwest::Client::builder()
        .user_agent(WEB_FETCH_USER_AGENT)
        .timeout(std::time::Duration::from_secs(WEB_FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
        .header("Accept", "text/html,application/json,text/plain,*/*")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Fetch request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Fetch failed with status: {}", status));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

    let text = if content_type.contains("text/html") {
        extract_readable_text(&body)
    } else if content_type.contains("application/json") {
        match serde_json::from_str::<Value>(&body) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(body),
            Err(_) => body,
        }
    } else {
        body
    };

    if text.len() > max_chars {
        let truncated = &text[..max_chars];
        Ok(format!(
            "URL: {}\nContent-Type: {}\n\n{}\n\n... (content truncated, {} chars total)",
            url, content_type, truncated, text.len()
        ))
    } else {
        Ok(format!("URL: {}\nContent-Type: {}\n\n{}", url, content_type, text))
    }
}

fn extract_readable_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut pos = 0;
    let lower = html.to_lowercase();

    let mut cleaned = String::with_capacity(html.len());
    while pos < html.len() {
        let remaining_lower = &lower[pos..];
        if remaining_lower.starts_with("<script") {
            if let Some(end) = lower[pos..].find("</script>") { pos += end + 9; continue; }
        }
        if remaining_lower.starts_with("<style") {
            if let Some(end) = lower[pos..].find("</style>") { pos += end + 8; continue; }
        }
        if remaining_lower.starts_with("<noscript") {
            if let Some(end) = lower[pos..].find("</noscript>") { pos += end + 11; continue; }
        }
        if remaining_lower.starts_with("<nav") {
            if let Some(end) = lower[pos..].find("</nav>") { pos += end + 6; continue; }
        }
        cleaned.push(html.as_bytes()[pos] as char);
        pos += 1;
    }

    let mut in_tag = false;
    let mut last_was_space = false;
    let mut newline_count = 0;

    for c in cleaned.chars() {
        if c == '<' { in_tag = true; continue; }
        if c == '>' { in_tag = false; if !last_was_space { result.push(' '); last_was_space = true; } continue; }
        if in_tag { continue; }
        if c == '\n' || c == '\r' {
            newline_count += 1;
            if newline_count <= 2 && !last_was_space { result.push('\n'); last_was_space = true; }
            continue;
        }
        if c.is_whitespace() {
            if !last_was_space { result.push(' '); last_was_space = true; }
            continue;
        }
        newline_count = 0;
        last_was_space = false;
        result.push(c);
    }

    html_decode(result.trim())
}
