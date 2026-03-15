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
            name: "read".into(),
            description: "Read the contents of a file at the specified path. Supports text files with line-based pagination (offset/limit) and image files (auto-detected, returned as base64). For large files, use offset and limit to read specific sections.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to read (also accepts 'file_path')"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (1-based). Defaults to 1"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read. If omitted, reads up to the internal max size"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "write".into(),
            description: "Write content to a file at the specified path. Creates parent directories if needed. Accepts 'file_path' as alias for 'path'.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to write (also accepts 'file_path')"
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
            name: "edit".into(),
            description: "Edit a file by replacing specific text. More precise than write for making targeted changes. The old_text must match exactly once (including whitespace and indentation). Accepts aliases: 'file_path' for 'path', 'oldText'/'old_string' for 'old_text', 'newText'/'new_string' for 'new_text'.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit (also accepts 'file_path')"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Exact text to find and replace (also accepts 'oldText' or 'old_string')"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text (also accepts 'newText' or 'new_string'). Can be empty to delete text."
                    }
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "ls".into(),
            description: "List files and directories in the specified path. Returns sorted names with type indicators (/ for directories, @ for symlinks). Supports ~ expansion and entry limit.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (also accepts 'file_path'). Defaults to current directory. Supports ~ for home directory."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entries to return. Defaults to 500."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "grep".into(),
            description: "Search file contents using regex or literal patterns. Respects .gitignore. Returns matching lines with file paths and line numbers.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (regex by default, or literal if literal=true)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search in (default: current directory). Supports ~ expansion."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.rs'"
                    },
                    "ignore_case": {
                        "type": "boolean",
                        "description": "Case-insensitive search (default: false)"
                    },
                    "literal": {
                        "type": "boolean",
                        "description": "Treat pattern as literal string instead of regex (default: false)"
                    },
                    "context": {
                        "type": "integer",
                        "description": "Number of lines to show before and after each match (default: 0)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of matches to return (default: 100)"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "find".into(),
            description: "Find files by glob pattern. Respects .gitignore. Returns matching file paths relative to the search directory.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', 'src/**/*.spec.ts'"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: current directory). Supports ~ expansion."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 1000)"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "apply_patch".into(),
            description: "Apply a patch to create, modify, move, or delete files. Use the *** Begin Patch / *** End Patch format with Add File, Update File, Delete File, and Move to markers.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Patch content using *** Begin Patch / *** End Patch format. Supported hunks: '*** Add File: <path>' (lines prefixed with +), '*** Update File: <path>' (@@ context marker, - for old lines, + for new lines), '*** Delete File: <path>', '*** Move to: <path>' (within Update hunk)."
                    }
                },
                "required": ["input"],
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
        "read" | "read_file" => tool_read_file(args, ctx).await,
        "write" | "write_file" => tool_write_file(args).await,
        "edit" | "patch_file" => tool_edit(args).await,
        "ls" | "list_dir" => tool_ls(args).await,
        "grep" => tool_grep(args).await,
        "find" => tool_find(args).await,
        "apply_patch" => tool_apply_patch(args).await,
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

/// Known image MIME types detected by magic bytes.
fn detect_image_mime(header: &[u8]) -> Option<&'static str> {
    if header.len() < 4 {
        return None;
    }
    // PNG: 89 50 4E 47
    if header.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some("image/png");
    }
    // JPEG: FF D8 FF
    if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    // GIF: GIF87a or GIF89a
    if header.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    // WebP: RIFF....WEBP
    if header.len() >= 12 && header.starts_with(b"RIFF") && &header[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // BMP: BM
    if header.starts_with(b"BM") {
        return Some("image/bmp");
    }
    // ICO: 00 00 01 00
    if header.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        return Some("image/x-icon");
    }
    // TIFF: II (little-endian) or MM (big-endian)
    if header.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || header.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        return Some("image/tiff");
    }
    None
}

/// Max dimension (width or height) for images sent to LLM.
const IMAGE_MAX_DIMENSION: u32 = 1200;
/// Max bytes for base64-encoded image payload.
const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Resize an image buffer if it exceeds dimension or byte limits.
/// Returns (base64_data, mime_type).
fn resize_image_if_needed(data: &[u8], original_mime: &str) -> Result<(String, &'static str)> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("Cannot detect image format: {}", e))?;
    let img = reader
        .decode()
        .map_err(|e| anyhow::anyhow!("Cannot decode image: {}", e))?;

    let (w, h) = (img.width(), img.height());
    let needs_resize = w > IMAGE_MAX_DIMENSION || h > IMAGE_MAX_DIMENSION || data.len() > IMAGE_MAX_BYTES;

    if !needs_resize {
        // Return original data as base64
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
        // Keep original mime, but map to static str
        let mime: &'static str = match original_mime {
            "image/png" => "image/png",
            "image/gif" => "image/gif",
            "image/webp" => "image/webp",
            "image/bmp" => "image/bmp",
            "image/tiff" => "image/tiff",
            "image/x-icon" => "image/x-icon",
            _ => "image/jpeg",
        };
        return Ok((b64, mime));
    }

    // Resize to fit within IMAGE_MAX_DIMENSION, preserving aspect ratio
    let resized = img.resize(
        IMAGE_MAX_DIMENSION,
        IMAGE_MAX_DIMENSION,
        image::imageops::FilterType::Lanczos3,
    );

    // Encode as JPEG with quality steps
    for quality in [85u8, 70, 50] {
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        resized
            .write_with_encoder(encoder)
            .map_err(|e| anyhow::anyhow!("Failed to encode resized image: {}", e))?;
        let jpeg_bytes = buf.into_inner();
        if jpeg_bytes.len() <= IMAGE_MAX_BYTES {
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes);
            return Ok((b64, "image/jpeg"));
        }
    }

    Err(anyhow::anyhow!(
        "Image too large: could not reduce below {}MB (original {}x{}, {} bytes)",
        IMAGE_MAX_BYTES / 1024 / 1024,
        w,
        h,
        data.len()
    ))
}

/// Default max bytes for a single read page (50KB).
const DEFAULT_READ_PAGE_MAX_BYTES: usize = 50 * 1024;
/// Max bytes for adaptive read (512KB).
const MAX_ADAPTIVE_READ_MAX_BYTES: usize = 512 * 1024;
/// Share of model context window to use for read output (20%).
const ADAPTIVE_READ_CONTEXT_SHARE: f64 = 0.2;
/// Estimated chars per token.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
/// Max pages for adaptive paging.
const MAX_ADAPTIVE_READ_PAGES: usize = 8;
/// Default max lines per page when no limit is specified.
const READ_DEFAULT_MAX_LINES: usize = 2000;

/// Compute max bytes for a single adaptive read page based on model context window.
fn compute_adaptive_read_max_bytes(context_window_tokens: Option<u32>) -> usize {
    match context_window_tokens {
        Some(tokens) if tokens > 0 => {
            let from_context =
                (tokens as usize) * CHARS_PER_TOKEN_ESTIMATE * (ADAPTIVE_READ_CONTEXT_SHARE * 100.0) as usize / 100;
            from_context.clamp(DEFAULT_READ_PAGE_MAX_BYTES, MAX_ADAPTIVE_READ_MAX_BYTES)
        }
        _ => DEFAULT_READ_PAGE_MAX_BYTES,
    }
}

/// Extract a string value from a Value that might be a plain string or `{type:"text", text:"..."}`.
fn extract_string_param(val: &Value) -> Option<&str> {
    // Plain string
    if let Some(s) = val.as_str() {
        return Some(s);
    }
    // Structured content: {type: "text", text: "..."}
    if let Some(obj) = val.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
            return obj.get("text").and_then(|v| v.as_str());
        }
    }
    None
}

/// Verify base64 image data's actual MIME type by decoding first 192 bytes and re-sniffing magic bytes.
fn verify_base64_mime(b64: &str, declared_mime: &str) -> &'static str {
    // Decode first 256 base64 chars (aligned to 4)
    let take = b64.len().min(256);
    let slice_len = take - (take % 4);
    if slice_len < 8 {
        return match declared_mime {
            "image/png" => "image/png",
            "image/gif" => "image/gif",
            "image/webp" => "image/webp",
            "image/bmp" => "image/bmp",
            "image/tiff" => "image/tiff",
            "image/x-icon" => "image/x-icon",
            _ => "image/jpeg",
        };
    }

    if let Ok(head) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64[..slice_len]) {
        if let Some(sniffed) = detect_image_mime(&head) {
            return sniffed;
        }
    }

    // Fallback to declared
    match declared_mime {
        "image/png" => "image/png",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/bmp" => "image/bmp",
        "image/tiff" => "image/tiff",
        "image/x-icon" => "image/x-icon",
        _ => "image/jpeg",
    }
}

/// Read a single page of a text file. Returns (output_text, lines_read, truncated, total_lines).
fn read_text_page(
    lines: &[&str],
    start_idx: usize,
    max_lines: usize,
) -> (String, usize, bool, usize) {
    let total_lines = lines.len();
    let start = start_idx.min(total_lines);
    let end = (start + max_lines).min(total_lines);
    let selected = &lines[start..end];

    let mut output = String::new();
    for (i, line) in selected.iter().enumerate() {
        let line_num = start + i + 1;
        output.push_str(&format!("{:6}\t{}\n", line_num, line));
    }

    let truncated = end < total_lines;
    (output, selected.len(), truncated, total_lines)
}

async fn tool_read_file(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1) as usize)
        .unwrap_or(1); // 1-based

    let explicit_limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    log::info!("Reading file: {} (offset={}, limit={:?})", path, offset, explicit_limit);

    // Read raw bytes first to detect file type
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    // Check if file is an image via magic bytes
    let mime = detect_image_mime(&data);
    if let Some(mime_type) = mime {
        log::info!("Detected image file: {} ({})", path, mime_type);
        match resize_image_if_needed(&data, mime_type) {
            Ok((b64, declared_mime)) => {
                // Secondary MIME verification: decode base64 header and re-sniff
                let verified_mime = verify_base64_mime(&b64, declared_mime);
                return Ok(format!(
                    "Read image file [{}] ({} bytes, {})\nbase64:{}\n",
                    verified_mime,
                    data.len(),
                    path,
                    b64
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Image file '{}' detected as {} but cannot be processed: {}",
                    path,
                    mime_type,
                    e
                ));
            }
        }
    }

    // Text file — convert to string
    let content = String::from_utf8(data)
        .map_err(|_| anyhow::anyhow!("File '{}' contains invalid UTF-8 (binary file?)", path))?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // If user specified an explicit limit, use single-page mode (no adaptive paging)
    if let Some(limit) = explicit_limit {
        let (output, lines_read, truncated, _) =
            read_text_page(&lines, offset - 1, limit);
        let mut result = output;
        if truncated {
            result.push_str(&format!(
                "\n[Read {} lines ({}–{} of {}). Use offset={} to continue reading.]\n",
                lines_read,
                offset,
                offset - 1 + lines_read,
                total_lines,
                offset + lines_read
            ));
        }
        return Ok(result);
    }

    // Adaptive paging: auto-aggregate multiple pages up to max_bytes budget
    let max_bytes = compute_adaptive_read_max_bytes(ctx.context_window_tokens);
    let page_max_lines = READ_DEFAULT_MAX_LINES;
    let mut aggregated = String::new();
    let mut aggregated_bytes: usize = 0;
    let mut next_offset = offset - 1; // convert to 0-based
    let mut capped = false;

    for _page in 0..MAX_ADAPTIVE_READ_PAGES {
        if next_offset >= total_lines {
            break;
        }

        let (page_text, lines_read, truncated, _) =
            read_text_page(&lines, next_offset, page_max_lines);

        if lines_read == 0 {
            break;
        }

        let page_bytes = page_text.len();

        // Check if adding this page would exceed budget (skip check for first page)
        if !aggregated.is_empty() && aggregated_bytes + page_bytes > max_bytes {
            capped = true;
            break;
        }

        aggregated.push_str(&page_text);
        aggregated_bytes += page_bytes;
        next_offset += lines_read;

        if !truncated {
            // Reached end of file
            break;
        }
    }

    // Add truncation/continuation notice
    if next_offset < total_lines {
        aggregated.push_str(&format!(
            "\n[Read lines {}–{} of {} ({} bytes). {}Use offset={} to continue reading.]\n",
            offset,
            next_offset,
            total_lines,
            aggregated_bytes,
            if capped {
                format!(
                    "Output capped at ~{}KB for this call. ",
                    max_bytes / 1024
                )
            } else {
                String::new()
            },
            next_offset + 1
        ));
    }

    Ok(aggregated)
}

async fn tool_write_file(args: &Value) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    // Accept structured content: plain string or {type:"text", text:"..."}
    let content = args
        .get("content")
        .and_then(|v| extract_string_param(v))
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

async fn tool_edit(args: &Value) -> Result<String> {
    // Accept path aliases: path, file_path
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    // Accept old_text aliases: old_text, oldText, old_string
    let old_text = args
        .get("old_text")
        .or_else(|| args.get("oldText"))
        .or_else(|| args.get("old_string"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' parameter"))?;

    // Accept new_text aliases: new_text, newText, new_string (empty string allowed for deletion)
    let new_text = args
        .get("new_text")
        .or_else(|| args.get("newText"))
        .or_else(|| args.get("new_string"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(""); // empty = deletion

    log::info!("Editing file: {}", path);

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    let count = content.matches(old_text).count();
    if count == 0 {
        // Post-write recovery: the file may already contain new_text from a previous
        // edit that threw after writing (e.g. interrupted tool call). If new_text is
        // present and old_text is absent, treat as success rather than false failure.
        if !new_text.is_empty() && content.contains(new_text) {
            log::info!("Post-write recovery: old_text absent but new_text already present in '{}'", path);
            return Ok(format!("Successfully edited {} (recovered — replacement already applied)", path));
        }
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

    let write_result = tokio::fs::write(path, &new_content).await;

    if let Err(ref e) = write_result {
        // Post-write recovery: if write returned an error but the file on disk actually
        // contains the correct content, treat as success. This handles edge cases where
        // data was flushed but the OS reported an error (e.g. network mounts, interrupted fsync).
        if let Ok(on_disk) = tokio::fs::read_to_string(path).await {
            let has_new = new_text.is_empty() || on_disk.contains(new_text);
            let still_has_old = !old_text.is_empty() && on_disk.contains(old_text);
            if has_new && !still_has_old {
                log::warn!("Post-write recovery: write error but file correct in '{}': {}", path, e);
                return Ok(format!("Successfully edited {} (recovered after write error)", path));
            }
        }
        return Err(anyhow::anyhow!("Failed to write file '{}': {}", path, e));
    }

    Ok(format!("Successfully edited {} (replaced 1 occurrence)", path))
}

/// Default max entries for ls.
const LS_DEFAULT_LIMIT: usize = 500;
/// Max output bytes for ls (50KB).
const LS_MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Expand ~ and ~/ to home directory.
fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return if path == "~" {
                home.to_string_lossy().to_string()
            } else {
                home.join(&path[2..]).to_string_lossy().to_string()
            };
        }
    }
    path.to_string()
}

async fn tool_ls(args: &Value) -> Result<String> {
    // Accept path aliases: path, file_path; with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(".");

    let path = expand_tilde(raw_path);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(LS_DEFAULT_LIMIT);

    log::info!("Listing directory: {} (limit={})", path, limit);

    // Validate path exists and is a directory
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| anyhow::anyhow!("Path not found: {}", path))?;

    if !meta.is_dir() {
        return Err(anyhow::anyhow!("Not a directory: {}", path));
    }

    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read directory '{}': {}", path, e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip entries that cannot be stat'd
        let indicator = match entry.file_type().await {
            Ok(ft) => {
                if ft.is_dir() {
                    "/"
                } else if ft.is_symlink() {
                    "@"
                } else {
                    ""
                }
            }
            Err(_) => "", // skip type indicator if stat fails
        };
        items.push(format!("{}{}", name, indicator));
    }

    // Case-insensitive sort
    items.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    if items.is_empty() {
        return Ok("(empty directory)".to_string());
    }

    // Apply entry limit and byte limit
    let mut output = String::new();
    let mut count = 0;
    let mut byte_limited = false;
    let mut entry_limited = false;

    for item in &items {
        if count >= limit {
            entry_limited = true;
            break;
        }
        let line = format!("{}\n", item);
        if output.len() + line.len() > LS_MAX_OUTPUT_BYTES {
            byte_limited = true;
            break;
        }
        output.push_str(&line);
        count += 1;
    }

    // Append truncation notice
    if entry_limited || byte_limited {
        let mut notices = Vec::new();
        if entry_limited {
            notices.push(format!("{} entries limit reached. Use limit={} for more.", limit, limit * 2));
        }
        if byte_limited {
            notices.push(format!("{}KB output limit reached.", LS_MAX_OUTPUT_BYTES / 1024));
        }
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}

// ── Web Tools ─────────────────────────────────────────────────────

// ── Grep & Find Tools ────────────────────────────────────────────

/// Max matches for grep (default).
const GREP_DEFAULT_LIMIT: usize = 100;
/// Max chars per grep output line.
const GREP_MAX_LINE_LENGTH: usize = 500;
/// Max output bytes for grep/find (50KB).
const GREP_FIND_MAX_OUTPUT_BYTES: usize = 50 * 1024;
/// Default max results for find.
const FIND_DEFAULT_LIMIT: usize = 1000;

async fn tool_grep(args: &Value) -> Result<String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(".");
    let search_path = expand_tilde(raw_path);

    let glob_pattern = args.get("glob").and_then(|v| extract_string_param(v));
    let ignore_case = args.get("ignore_case").and_then(|v| v.as_bool()).unwrap_or(false)
        || args.get("ignoreCase").and_then(|v| v.as_bool()).unwrap_or(false);
    let literal = args.get("literal").and_then(|v| v.as_bool()).unwrap_or(false);
    let context_lines = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(GREP_DEFAULT_LIMIT as u64) as usize;

    log::info!("Grep: pattern='{}', path='{}', glob={:?}, limit={}", pattern_str, search_path, glob_pattern, limit);

    // Build regex
    let regex_pattern = if literal {
        regex::escape(pattern_str)
    } else {
        pattern_str.to_string()
    };
    let re = regex::RegexBuilder::new(&regex_pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| anyhow::anyhow!("Invalid regex pattern '{}': {}", pattern_str, e))?;

    // Build glob matcher if provided
    let glob_matcher = if let Some(g) = glob_pattern {
        Some(
            glob::Pattern::new(g)
                .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", g, e))?,
        )
    } else {
        None
    };

    // Walk directory respecting .gitignore
    let search_path_clone = search_path.clone();
    let walker = ignore::WalkBuilder::new(&search_path)
        .hidden(false) // include hidden files
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let mut output = String::new();
    let mut match_count: usize = 0;
    let mut byte_limited = false;
    let mut lines_truncated = false;

    let search_base = std::path::Path::new(&search_path_clone);

    for entry_result in walker {
        if match_count >= limit || byte_limited {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip directories
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if ft.is_dir() {
            continue;
        }

        let entry_path = entry.path();

        // Apply glob filter
        if let Some(ref gm) = glob_matcher {
            let rel = entry_path
                .strip_prefix(search_base)
                .unwrap_or(entry_path);
            let rel_str = rel.to_string_lossy();
            let file_name = entry_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            // Match against filename or relative path
            if !gm.matches(&file_name) && !gm.matches(&rel_str) {
                continue;
            }
        }

        // Read file as text (skip binary)
        let content = match std::fs::read_to_string(entry_path) {
            Ok(c) => c,
            Err(_) => continue, // skip binary/unreadable files
        };

        let rel_path = entry_path
            .strip_prefix(search_base)
            .unwrap_or(entry_path)
            .to_string_lossy();

        let file_lines: Vec<&str> = content.lines().collect();

        for (line_idx, line) in file_lines.iter().enumerate() {
            if match_count >= limit {
                break;
            }
            if !re.is_match(line) {
                continue;
            }

            match_count += 1;

            // Add context lines before
            if context_lines > 0 {
                let ctx_start = line_idx.saturating_sub(context_lines);
                for ci in ctx_start..line_idx {
                    let ctx_line = truncate_line(file_lines[ci], GREP_MAX_LINE_LENGTH, &mut lines_truncated);
                    let formatted = format!("{}-{}- {}\n", rel_path, ci + 1, ctx_line);
                    if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                        byte_limited = true;
                        break;
                    }
                    output.push_str(&formatted);
                }
            }

            if byte_limited {
                break;
            }

            // Match line
            let match_line = truncate_line(line, GREP_MAX_LINE_LENGTH, &mut lines_truncated);
            let formatted = format!("{}:{}: {}\n", rel_path, line_idx + 1, match_line);
            if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                byte_limited = true;
                break;
            }
            output.push_str(&formatted);

            // Add context lines after
            if context_lines > 0 {
                let ctx_end = (line_idx + 1 + context_lines).min(file_lines.len());
                for ci in (line_idx + 1)..ctx_end {
                    let ctx_line = truncate_line(file_lines[ci], GREP_MAX_LINE_LENGTH, &mut lines_truncated);
                    let formatted = format!("{}-{}- {}\n", rel_path, ci + 1, ctx_line);
                    if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                        byte_limited = true;
                        break;
                    }
                    output.push_str(&formatted);
                }
                if !byte_limited {
                    output.push('\n'); // separator between match groups
                }
            }
        }
    }

    if match_count == 0 {
        return Ok("No matches found.".to_string());
    }

    // Append notices
    let mut notices = Vec::new();
    if match_count >= limit {
        notices.push(format!("{} matches limit reached. Use limit={} for more, or refine pattern.", limit, limit * 2));
    }
    if byte_limited {
        notices.push(format!("{}KB output limit reached.", GREP_FIND_MAX_OUTPUT_BYTES / 1024));
    }
    if lines_truncated {
        notices.push("Some lines truncated to 500 chars. Use read tool to see full lines.".to_string());
    }
    if !notices.is_empty() {
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}

/// Truncate a line to max_len chars, setting flag if truncated.
fn truncate_line(line: &str, max_len: usize, truncated_flag: &mut bool) -> String {
    if line.len() <= max_len {
        line.to_string()
    } else {
        *truncated_flag = true;
        // Truncate at char boundary
        let end = line
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        format!("{}... [truncated]", &line[..end])
    }
}

async fn tool_find(args: &Value) -> Result<String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(".");
    let search_path = expand_tilde(raw_path);

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(FIND_DEFAULT_LIMIT as u64) as usize;

    log::info!("Find: pattern='{}', path='{}', limit={}", pattern_str, search_path, limit);

    // Build glob matcher
    let glob_matcher = glob::Pattern::new(pattern_str)
        .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", pattern_str, e))?;

    // Validate path
    let meta = tokio::fs::metadata(&search_path)
        .await
        .map_err(|_| anyhow::anyhow!("Path not found: {}", search_path))?;
    if !meta.is_dir() {
        return Err(anyhow::anyhow!("Not a directory: {}", search_path));
    }

    // Walk directory respecting .gitignore
    let walker = ignore::WalkBuilder::new(&search_path)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let search_base = std::path::Path::new(&search_path);
    let mut output = String::new();
    let mut count: usize = 0;
    let mut byte_limited = false;

    for entry_result in walker {
        if count >= limit || byte_limited {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip directories themselves (but walk into them)
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if ft.is_dir() {
            continue;
        }

        let entry_path = entry.path();
        let rel_path = entry_path
            .strip_prefix(search_base)
            .unwrap_or(entry_path);
        let rel_str = rel_path.to_string_lossy();
        let file_name = entry_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        // Match against filename or full relative path
        if !glob_matcher.matches(&file_name) && !glob_matcher.matches(&rel_str) {
            continue;
        }

        count += 1;
        let line = format!("{}\n", rel_str);
        if output.len() + line.len() > GREP_FIND_MAX_OUTPUT_BYTES {
            byte_limited = true;
            break;
        }
        output.push_str(&line);
    }

    if count == 0 {
        return Ok("No files found.".to_string());
    }

    // Append notices
    let mut notices = Vec::new();
    if count >= limit {
        notices.push(format!("{} results limit reached. Use limit={} for more, or refine pattern.", limit, limit * 2));
    }
    if byte_limited {
        notices.push(format!("{}KB output limit reached.", GREP_FIND_MAX_OUTPUT_BYTES / 1024));
    }
    if !notices.is_empty() {
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}

// ── Apply Patch Tool ─────────────────────────────────────────────

/// Parsed hunk kinds.
#[derive(Debug)]
enum PatchHunkKind {
    Add { path: String, contents: String },
    Delete { path: String },
    Update { path: String, chunks: Vec<UpdateChunk>, move_to: Option<String> },
}

/// A chunk within an Update hunk: context lines + old/new replacements.
#[derive(Debug)]
struct UpdateChunk {
    context: Vec<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

/// Parse a patch text into hunks.
fn parse_patch(input: &str) -> Result<Vec<PatchHunkKind>> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return Err(anyhow::anyhow!("Invalid patch: input is empty."));
    }

    // Find *** Begin Patch / *** End Patch boundaries (lenient: skip heredoc wrappers)
    let start = lines
        .iter()
        .position(|l| l.trim() == "*** Begin Patch")
        .ok_or_else(|| anyhow::anyhow!("The first line of the patch must be '*** Begin Patch'"))?;
    let end = lines
        .iter()
        .rposition(|l| l.trim() == "*** End Patch")
        .ok_or_else(|| anyhow::anyhow!("The last line of the patch must be '*** End Patch'"))?;

    if start >= end {
        return Err(anyhow::anyhow!("Invalid patch: Begin Patch must come before End Patch"));
    }

    let body = &lines[start + 1..end];
    let mut hunks = Vec::new();
    let mut i = 0;

    while i < body.len() {
        let line = body[i].trim();

        if line.is_empty() {
            i += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = path.trim().to_string();
            let mut contents = String::new();
            i += 1;
            while i < body.len() && !body[i].trim().starts_with("*** ") {
                let l = body[i];
                if let Some(stripped) = l.strip_prefix('+') {
                    contents.push_str(stripped);
                } else {
                    contents.push_str(l);
                }
                contents.push('\n');
                i += 1;
            }
            hunks.push(PatchHunkKind::Add { path, contents });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            hunks.push(PatchHunkKind::Delete { path: path.trim().to_string() });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            let mut chunks = Vec::new();
            let mut move_to: Option<String> = None;
            i += 1;

            let mut current_context: Vec<String> = Vec::new();
            let mut current_old: Vec<String> = Vec::new();
            let mut current_new: Vec<String> = Vec::new();
            let mut in_change = false;

            while i < body.len() {
                let l = body[i];
                let trimmed = l.trim();

                // Check for next hunk boundary (but not End of File / Move to)
                if trimmed.starts_with("*** ") && trimmed != "*** End of File"
                    && !trimmed.starts_with("*** Move to: ")
                {
                    break;
                }

                if trimmed == "*** End of File" {
                    if in_change || !current_context.is_empty() {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    i += 1;
                    continue;
                }

                if let Some(mp) = trimmed.strip_prefix("*** Move to: ") {
                    move_to = Some(mp.trim().to_string());
                    i += 1;
                    continue;
                }

                if trimmed.starts_with("@@") {
                    if in_change || !current_context.is_empty() {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    let ctx = trimmed.strip_prefix("@@").unwrap().trim();
                    if !ctx.is_empty() {
                        current_context.push(ctx.to_string());
                    }
                    i += 1;
                    continue;
                }

                if let Some(old) = l.strip_prefix('-') {
                    in_change = true;
                    current_old.push(old.to_string());
                    i += 1;
                } else if let Some(new_line) = l.strip_prefix('+') {
                    in_change = true;
                    current_new.push(new_line.to_string());
                    i += 1;
                } else {
                    if in_change {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    let ctx_line = l.strip_prefix(' ').unwrap_or(l);
                    current_context.push(ctx_line.to_string());
                    i += 1;
                }
            }

            // Flush remaining chunk
            if in_change || !current_old.is_empty() || !current_new.is_empty() {
                chunks.push(UpdateChunk {
                    context: std::mem::take(&mut current_context),
                    old_lines: std::mem::take(&mut current_old),
                    new_lines: std::mem::take(&mut current_new),
                });
            }

            hunks.push(PatchHunkKind::Update { path, chunks, move_to });
        } else {
            i += 1;
        }
    }

    Ok(hunks)
}

/// Find a sequence of lines in file_lines using fuzzy matching (3-pass).
fn seek_sequence(file_lines: &[&str], needle: &[String], start_from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start_from);
    }
    let len = needle.len();
    if len > file_lines.len() {
        return None;
    }

    let max_i = file_lines.len() - len;

    // Helper: search range with a comparator
    let search = |cmp: &dyn Fn(&str, &str) -> bool| -> Option<usize> {
        // Search forward from start_from first
        for i in start_from..=max_i {
            if (0..len).all(|j| cmp(file_lines[i + j], &needle[j])) {
                return Some(i);
            }
        }
        // Then search before start_from
        for i in 0..start_from.min(max_i + 1) {
            if (0..len).all(|j| cmp(file_lines[i + j], &needle[j])) {
                return Some(i);
            }
        }
        None
    };

    // Pass 1: exact
    if let Some(pos) = search(&|a: &str, b: &str| a == b) {
        return Some(pos);
    }
    // Pass 2: trimmed end
    if let Some(pos) = search(&|a: &str, b: &str| a.trim_end() == b.trim_end()) {
        return Some(pos);
    }
    // Pass 3: fully trimmed
    search(&|a: &str, b: &str| a.trim() == b.trim())
}

/// Apply update chunks to file content.
fn apply_update_hunks(content: &str, path: &str, chunks: &[UpdateChunk]) -> Result<String> {
    let mut file_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut cursor: usize = 0;

    for chunk in chunks {
        let file_refs: Vec<&str> = file_lines.iter().map(|s| s.as_str()).collect();

        // Find position using context lines
        if !chunk.context.is_empty() {
            match seek_sequence(&file_refs, &chunk.context, cursor) {
                Some(pos) => cursor = pos + chunk.context.len(),
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to find context in {}: '{}'",
                        path,
                        chunk.context.first().unwrap_or(&String::new())
                    ));
                }
            }
        }

        // Apply old→new replacement
        if !chunk.old_lines.is_empty() {
            let file_refs: Vec<&str> = file_lines.iter().map(|s| s.as_str()).collect();
            match seek_sequence(&file_refs, &chunk.old_lines, cursor) {
                Some(pos) => {
                    file_lines.splice(
                        pos..pos + chunk.old_lines.len(),
                        chunk.new_lines.iter().cloned(),
                    );
                    cursor = pos + chunk.new_lines.len();
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to find expected lines in {}: '{}'",
                        path,
                        chunk.old_lines.first().unwrap_or(&String::new())
                    ));
                }
            }
        } else if !chunk.new_lines.is_empty() {
            // Insert-only (no old lines)
            for (j, new_line) in chunk.new_lines.iter().enumerate() {
                file_lines.insert(cursor + j, new_line.clone());
            }
            cursor += chunk.new_lines.len();
        }
    }

    let mut result = file_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

async fn tool_apply_patch(args: &Value) -> Result<String> {
    let input = args
        .get("input")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'input' parameter"))?;

    if input.trim().is_empty() {
        return Err(anyhow::anyhow!("Provide a patch input."));
    }

    log::info!("Applying patch ({} chars)", input.len());

    let hunks = parse_patch(input)?;
    if hunks.is_empty() {
        return Err(anyhow::anyhow!("No files were modified."));
    }

    let mut added: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for hunk in &hunks {
        match hunk {
            PatchHunkKind::Add { path, contents } => {
                let p = Path::new(path);
                if let Some(parent) = p.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to create directories for '{}': {}", path, e))?;
                }
                tokio::fs::write(path, contents)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to write new file '{}': {}", path, e))?;
                added.push(path.clone());
            }
            PatchHunkKind::Delete { path } => {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to delete file '{}': {}", path, e))?;
                deleted.push(path.clone());
            }
            PatchHunkKind::Update { path, chunks, move_to } => {
                let content = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

                let new_content = apply_update_hunks(&content, path, chunks)?;

                if let Some(new_path) = move_to {
                    let np = Path::new(new_path);
                    if let Some(parent) = np.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to create dirs for '{}': {}", new_path, e))?;
                    }
                    tokio::fs::write(new_path, &new_content)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", new_path, e))?;
                    tokio::fs::remove_file(path)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to remove old file '{}': {}", path, e))?;
                    modified.push(format!("{} -> {}", path, new_path));
                } else {
                    tokio::fs::write(path, &new_content)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", path, e))?;
                    modified.push(path.clone());
                }
            }
        }
    }

    let mut summary_parts = Vec::new();
    if !added.is_empty() {
        summary_parts.push(format!("Added: {}", added.join(", ")));
    }
    if !modified.is_empty() {
        summary_parts.push(format!("Modified: {}", modified.join(", ")));
    }
    if !deleted.is_empty() {
        summary_parts.push(format!("Deleted: {}", deleted.join(", ")));
    }

    Ok(format!("Patch applied successfully.\n{}", summary_parts.join("\n")))
}

// ── Web Tools ────────────────────────────────────────────────────

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
