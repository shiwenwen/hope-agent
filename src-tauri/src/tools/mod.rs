use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod approval;
mod apply_patch;
pub(crate) mod browser;
mod cron;
mod edit;
mod exec;
mod find;
mod grep;
mod ls;
mod memory;
mod process;
mod read;
mod web;
mod write;

// ── Public Re-exports ─────────────────────────────────────────────

pub use approval::{ApprovalResponse, submit_approval_response};

// ── Shared Helpers ────────────────────────────────────────────────

/// Extract a string value from a Value that might be a plain string or `{type:"text", text:"..."}`.
pub(crate) fn extract_string_param(val: &Value) -> Option<&str> {
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

/// Expand ~ and ~/ to home directory.
pub(crate) fn expand_tilde(path: &str) -> String {
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
        ToolDefinition {
            name: "save_memory".into(),
            description: "Save information to persistent memory for future conversations. Use this when the user shares personal info, preferences, corrections to your behavior, project context, or reference materials. Memories persist across conversations and help you provide better, personalized assistance.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The information to remember. Be concise but complete."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference"],
                        "description": "Memory type: user (about the user), feedback (behavior preferences), project (project context), reference (external resources)"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags for categorization"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["global", "agent"],
                        "description": "Scope: global (shared across agents) or agent (private to current agent). Default: global"
                    }
                },
                "required": ["content", "type"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "recall_memory".into(),
            description: "Search persistent memories by keyword or semantic query. Use this to recall previously stored information about the user, their preferences, project context, or reference materials.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (keyword or natural language)"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference"],
                        "description": "Filter by memory type (optional)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 10)"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        // ── Cron / Scheduled Tasks ──────────────────────────────
        ToolDefinition {
            name: "manage_cron".into(),
            description: "Create, list, update, delete, and trigger scheduled tasks (cron jobs). Jobs automatically send messages to the AI agent on a schedule. Supports one-time (at), recurring (every), and cron expression schedules.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "list", "get", "delete", "pause", "resume", "run_now"],
                        "description": "Action to perform"
                    },
                    "id": {
                        "type": "string",
                        "description": "Job ID (required for get/delete/pause/resume/run_now)"
                    },
                    "name": {
                        "type": "string",
                        "description": "Job name (for create)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Job description (for create)"
                    },
                    "schedule_type": {
                        "type": "string",
                        "enum": ["at", "every", "cron"],
                        "description": "Schedule type (for create)"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO8601 timestamp for 'at' schedule"
                    },
                    "interval_ms": {
                        "type": "integer",
                        "description": "Interval in milliseconds for 'every' schedule (min 60000)"
                    },
                    "cron_expression": {
                        "type": "string",
                        "description": "Cron expression for 'cron' schedule (e.g. '0 0 9 * * 1-5 *' = weekdays 9am)"
                    },
                    "timezone": {
                        "type": "string",
                        "description": "Timezone for cron schedule (default UTC)"
                    },
                    "message": {
                        "type": "string",
                        "description": "Message to send to the agent when triggered (for create)"
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Target agent ID (default: current agent)"
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        // ── Browser Control ──────────────────────────────────────
        ToolDefinition {
            name: "browser".into(),
            description: "Control a Chrome browser via DevTools Protocol. Supports navigation, element interaction (click/fill/hover/drag), screenshots, accessibility snapshots, JavaScript execution, and tab management. Chrome must be running with --remote-debugging-port=9222, or use action='launch' to start a managed instance. Use 'take_snapshot' to get element refs, then use those refs for click/fill/hover actions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "connect", "launch", "disconnect",
                            "list_pages", "new_page", "select_page", "close_page",
                            "navigate", "go_back", "go_forward",
                            "take_snapshot", "take_screenshot",
                            "click", "fill", "fill_form", "hover", "drag",
                            "press_key", "upload_file",
                            "evaluate", "wait_for",
                            "handle_dialog", "resize", "scroll"
                        ],
                        "description": "Browser action to perform"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL for navigate/new_page/connect"
                    },
                    "ref": {
                        "type": "integer",
                        "description": "Element ref ID from take_snapshot for click/fill/hover/drag"
                    },
                    "value": {
                        "type": "string",
                        "description": "Value for fill action"
                    },
                    "expression": {
                        "type": "string",
                        "description": "JavaScript expression for evaluate action"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to wait for (wait_for action)"
                    },
                    "key": {
                        "type": "string",
                        "description": "Key name for press_key (e.g. 'Enter', 'Tab', 'Escape', 'ArrowDown')"
                    },
                    "page_id": {
                        "type": "string",
                        "description": "Page/tab target ID for select_page/close_page"
                    },
                    "fields": {
                        "type": "object",
                        "description": "For fill_form: map of ref IDs to values (e.g. {\"3\": \"hello\", \"5\": \"world\"})",
                        "additionalProperties": { "type": "string" }
                    },
                    "format": {
                        "type": "string",
                        "enum": ["png", "jpeg"],
                        "description": "Screenshot format (default: png)"
                    },
                    "full_page": {
                        "type": "boolean",
                        "description": "Capture full page screenshot (default: false)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in ms for navigate/wait_for (default: 30000)"
                    },
                    "width": {
                        "type": "integer",
                        "description": "Viewport width for resize action"
                    },
                    "height": {
                        "type": "integer",
                        "description": "Viewport height for resize action"
                    },
                    "double_click": {
                        "type": "boolean",
                        "description": "Double-click for click action"
                    },
                    "accept": {
                        "type": "boolean",
                        "description": "Accept (true) or dismiss (false) dialog"
                    },
                    "dialog_text": {
                        "type": "string",
                        "description": "Text to enter in prompt dialog"
                    },
                    "target_ref": {
                        "type": "integer",
                        "description": "Target element ref for drag action"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "File path for upload_file action"
                    },
                    "executable_path": {
                        "type": "string",
                        "description": "Chrome executable path for launch action"
                    },
                    "headless": {
                        "type": "boolean",
                        "description": "Launch in headless mode (default: false)"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "left", "right"],
                        "description": "Scroll direction (default: down)"
                    },
                    "amount": {
                        "type": "integer",
                        "description": "Scroll amount in pixels (default: 500)"
                    }
                },
                "required": ["action"]
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
    /// Agent home directory — used as default cwd/path for tools.
    /// Falls back to user ~ if None.
    pub home_dir: Option<String>,
}

impl Default for ToolExecContext {
    fn default() -> Self {
        Self {
            context_window_tokens: None,
            home_dir: None,
        }
    }
}

impl ToolExecContext {
    /// Returns the default path for tools: agent home if set, otherwise ".".
    pub fn default_path(&self) -> &str {
        self.home_dir.as_deref().unwrap_or(".")
    }
}

// ── Tool Execution (provider-agnostic) ────────────────────────────

/// Execute a tool by name with the given JSON arguments.
pub async fn execute_tool(name: &str, args: &Value) -> anyhow::Result<String> {
    execute_tool_with_context(name, args, &ToolExecContext::default()).await
}

/// Execute a tool with additional context (model info, etc.)
pub async fn execute_tool_with_context(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();

    // Log tool execution start
    if let Some(logger) = crate::get_logger() {
        let args_preview = {
            let s = args.to_string();
            if s.len() > 500 { format!("{}...", &s[..500]) } else { s }
        };
        logger.log("info", "tool", &format!("tools::{}", name),
            &format!("Tool '{}' started", name),
            Some(serde_json::json!({"args": args_preview}).to_string()),
            None, None);
    }

    let result = match name {
        "exec" => exec::tool_exec(args, ctx).await,
        "process" => process::tool_process(args).await,
        "read" | "read_file" => read::tool_read_file(args, ctx).await,
        "write" | "write_file" => write::tool_write_file(args).await,
        "edit" | "patch_file" => edit::tool_edit(args).await,
        "ls" | "list_dir" => ls::tool_ls(args, ctx).await,
        "grep" => grep::tool_grep(args, ctx).await,
        "find" => find::tool_find(args, ctx).await,
        "apply_patch" => apply_patch::tool_apply_patch(args).await,
        "web_search" => web::tool_web_search(args).await,
        "web_fetch" => web::tool_web_fetch(args).await,
        "save_memory" => memory::tool_save_memory(args).await,
        "recall_memory" => memory::tool_recall_memory(args).await,
        "manage_cron" => cron::tool_manage_cron(args).await,
        "browser" => browser::tool_browser(args).await,
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Log tool execution result
    if let Some(logger) = crate::get_logger() {
        match &result {
            Ok(output) => {
                let output_preview = if output.len() > 300 { format!("{}...", &output[..300]) } else { output.clone() };
                logger.log("info", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' completed in {}ms", name, duration_ms),
                    Some(serde_json::json!({"duration_ms": duration_ms, "output_preview": output_preview}).to_string()),
                    None, None);
            }
            Err(e) => {
                logger.log("error", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' failed in {}ms: {}", name, duration_ms, e),
                    Some(serde_json::json!({"duration_ms": duration_ms, "error": e.to_string()}).to_string()),
                    None, None);
            }
        }
    }

    result
}
