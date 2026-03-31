use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::LazyLock;

use super::{
    ToolProvider, TOOL_ACP_SPAWN, TOOL_AGENTS_LIST, TOOL_AMEND_PLAN, TOOL_APPLY_PATCH,
    TOOL_BROWSER, TOOL_CANVAS, TOOL_DELETE_MEMORY, TOOL_EDIT, TOOL_EXEC, TOOL_FIND,
    TOOL_GET_WEATHER, TOOL_GREP, TOOL_IMAGE, TOOL_IMAGE_GENERATE, TOOL_LS, TOOL_MANAGE_CRON,
    TOOL_MEMORY_GET, TOOL_PDF, TOOL_PLAN_QUESTION, TOOL_PROCESS, TOOL_READ, TOOL_RECALL_MEMORY,
    TOOL_SAVE_MEMORY, TOOL_SEND_NOTIFICATION, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST,
    TOOL_SESSIONS_SEND, TOOL_SESSION_STATUS, TOOL_SUBAGENT, TOOL_SUBMIT_PLAN,
    TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY, TOOL_UPDATE_PLAN_STEP, TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH, TOOL_WRITE,
};

// ── Tool Definition (provider-agnostic) ───────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool parameters
    pub parameters: Value,
    /// Internal capability tools never require user approval.
    /// These are autonomous agent abilities (memory, cron, notification, read-only analysis)
    /// rather than system-interacting tools (exec, write, edit, etc.)
    #[serde(default)]
    pub internal: bool,
}

impl ToolDefinition {
    #[allow(dead_code)]
    fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            internal: false,
        }
    }

    #[allow(dead_code)]
    fn new_internal(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            internal: true,
        }
    }

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
            name: TOOL_EXEC.into(),
            description: "Execute a shell command. Returns stdout/stderr. Supports background execution with yield_ms/background params.".into(),
            internal: false,
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
            name: TOOL_PROCESS.into(),
            description: "Manage running exec sessions: list, poll, log, write, kill, clear, remove.".into(),
            internal: false,
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
            name: TOOL_READ.into(),
            description: "Read the contents of a file at the specified path. Supports text files with line-based pagination (offset/limit) and image files (auto-detected, returned as base64). For large files, use offset and limit to read specific sections.".into(),
            internal: false,
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
            name: TOOL_WRITE.into(),
            description: "Write content to a file at the specified path. Creates parent directories if needed. Accepts 'file_path' as alias for 'path'.".into(),
            internal: false,
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
            name: TOOL_EDIT.into(),
            description: "Edit a file by replacing specific text. More precise than write for making targeted changes. The old_text must match exactly once (including whitespace and indentation). Accepts aliases: 'file_path' for 'path', 'oldText'/'old_string' for 'old_text', 'newText'/'new_string' for 'new_text'.".into(),
            internal: false,
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
            name: TOOL_LS.into(),
            description: "List files and directories in the specified path. Returns sorted names with type indicators (/ for directories, @ for symlinks). Supports ~ expansion and entry limit.".into(),
            internal: false,
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
            name: TOOL_GREP.into(),
            description: "Search file contents using regex or literal patterns. Respects .gitignore. Returns matching lines with file paths and line numbers.".into(),
            internal: false,
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
            name: TOOL_FIND.into(),
            description: "Find files by glob pattern. Respects .gitignore. Returns matching file paths relative to the search directory.".into(),
            internal: false,
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
            name: TOOL_APPLY_PATCH.into(),
            description: "Apply a patch to create, modify, move, or delete files. Use the *** Begin Patch / *** End Patch format with Add File, Update File, Delete File, and Move to markers.".into(),
            internal: false,
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
            name: TOOL_WEB_FETCH.into(),
            description: "Fetch and extract readable content from a URL using Mozilla Readability. Supports markdown and plain text output modes. Returns structured JSON with page content, metadata, and extraction info. Use this to read web pages, documentation, articles, or API responses.".into(),
            internal: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to fetch"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum content characters to return (default from config, capped by server limit)"
                    },
                    "extract_mode": {
                        "type": "string",
                        "enum": ["markdown", "text"],
                        "description": "Content extraction mode: 'markdown' (default) preserves formatting with links/headings/lists, 'text' returns plain text"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_SAVE_MEMORY.into(),
            description: "Save information to persistent memory for future conversations. Use this when the user shares personal info, preferences, corrections to your behavior, project context, or reference materials. Memories persist across conversations and help you provide better, personalized assistance.".into(),
            internal: true,
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
                    },
                    "pinned": {
                        "type": "boolean",
                        "description": "If true, this memory is pinned and always prioritized in the system prompt regardless of age. Default: false"
                    }
                },
                "required": ["content", "type"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_RECALL_MEMORY.into(),
            description: "Search persistent memories by keyword or semantic query. Use this to recall previously stored information about the user, their preferences, project context, or reference materials. Set include_history=true to also search past conversation messages.".into(),
            internal: true,
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
                    },
                    "include_history": {
                        "type": "boolean",
                        "description": "Also search past conversation messages (default: false). Use when the user references previous conversations."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_UPDATE_MEMORY.into(),
            description: "Update an existing memory's content and tags by its ID. Use recall_memory first to find the memory ID. Use when a memory needs correction or its information has changed.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The memory ID to update (obtained from recall_memory results)"
                    },
                    "content": {
                        "type": "string",
                        "description": "The new content to replace the existing memory"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New tags (replaces existing tags). Omit to clear tags."
                    }
                },
                "required": ["id", "content"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_DELETE_MEMORY.into(),
            description: "Delete a memory by its ID. Use recall_memory first to find the memory ID, then use this tool to remove it. Use when the user asks to forget something or when a memory is outdated/incorrect.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The memory ID to delete (obtained from recall_memory results)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        // ── Cron / Scheduled Tasks ──────────────────────────────
        ToolDefinition {
            name: TOOL_MANAGE_CRON.into(),
            description: "Create, list, update, delete, and trigger scheduled tasks (cron jobs). Jobs run an agent turn with the given prompt on a schedule (isolated session, no prior history). Supports one-time (at), recurring (every), and cron expression schedules.".into(),
            internal: true,
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
                    "prompt": {
                        "type": "string",
                        "description": "The text prompt that the agent will execute when the job triggers. This runs as an isolated agent turn with no prior conversation history."
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
            name: TOOL_BROWSER.into(),
            description: "Control a Chrome browser via DevTools Protocol. Supports navigation, element interaction (click/fill/hover/drag), screenshots, accessibility snapshots, JavaScript execution, tab management, profile isolation, and PDF export. Chrome must be running with --remote-debugging-port=9222, or use action='launch' to start a managed instance. Use 'take_snapshot' to get element refs, then use those refs for click/fill/hover actions. Use 'list_profiles' to see available profiles and 'save_pdf' to export pages as PDF.".into(),
            internal: false,
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
                            "handle_dialog", "resize", "scroll",
                            "list_profiles", "save_pdf"
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
                    "profile": {
                        "type": "string",
                        "description": "Browser profile name for launch action. Each profile has isolated cookies, storage, and login state. Use 'list_profiles' to see existing profiles."
                    },
                    "output_path": {
                        "type": "string",
                        "description": "File path for save_pdf output. Defaults to ~/.opencomputer/share/page_<timestamp>.pdf"
                    },
                    "paper_format": {
                        "type": "string",
                        "enum": ["a3", "a4", "a5", "letter", "legal", "tabloid"],
                        "description": "Paper format for save_pdf (default: letter)"
                    },
                    "landscape": {
                        "type": "boolean",
                        "description": "Use landscape orientation for save_pdf (default: false)"
                    },
                    "print_background": {
                        "type": "boolean",
                        "description": "Include background graphics in save_pdf (default: false)"
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
        // ── Memory Get ──────────────────────────────────────────
        ToolDefinition {
            name: TOOL_MEMORY_GET.into(),
            description: "Retrieve a specific memory entry by its ID with full content and metadata. Use after recall_memory to get complete details of a specific memory.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Memory entry ID to retrieve (obtained from recall_memory results)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        // ── Update Core Memory ─────────────────────────────────
        ToolDefinition {
            name: TOOL_UPDATE_CORE_MEMORY.into(),
            description: "Update the core memory file (memory.md) that is always visible in the system prompt. Use for persistent rules, preferences, and standing instructions that the user wants you to always follow.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["append", "replace"],
                        "description": "append: add content to the end of core memory; replace: overwrite the entire core memory file"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["global", "agent"],
                        "description": "global: shared across all agents; agent: specific to current agent. Default: agent"
                    },
                    "content": {
                        "type": "string",
                        "description": "The rule, preference, or instruction to write"
                    }
                },
                "required": ["action", "content"],
                "additionalProperties": false
            }),
        },
        // ── Agents List ─────────────────────────────────────────
        ToolDefinition {
            name: TOOL_AGENTS_LIST.into(),
            description: "List all available agents with their descriptions and capabilities. Useful for choosing which agent to delegate tasks to via subagent.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        },
        // ── Sessions List ───────────────────────────────────────
        ToolDefinition {
            name: TOOL_SESSIONS_LIST.into(),
            description: "List all chat sessions with metadata (title, agent, model, message count). Use to discover existing sessions for cross-session communication.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Filter by agent ID (optional)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max sessions to return (default 20, max 100)"
                    },
                    "include_cron": {
                        "type": "boolean",
                        "description": "Include cron-triggered sessions (default false)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
        // ── Session Status ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_SESSION_STATUS.into(),
            description: "Query detailed status of a specific session including agent, model, message count, and timestamps.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID to query"
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }),
        },
        // ── Sessions History ────────────────────────────────────
        ToolDefinition {
            name: TOOL_SESSIONS_HISTORY.into(),
            description: "Get paginated chat history from a specific session. Use to read conversation context from other sessions. Tool call details are excluded by default to reduce noise.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Target session ID"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 50, max 200)"
                    },
                    "before_id": {
                        "type": "integer",
                        "description": "Pagination cursor: load messages before this message ID"
                    },
                    "include_tools": {
                        "type": "boolean",
                        "description": "Include tool call/result details (default false)"
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }),
        },
        // ── Sessions Send ───────────────────────────────────────
        ToolDefinition {
            name: TOOL_SESSIONS_SEND.into(),
            description: "Send a message to another session for cross-session communication. The message is delivered as a user message. With wait=true, blocks until the target agent responds (up to timeout_secs).".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Target session ID"
                    },
                    "message": {
                        "type": "string",
                        "description": "Message content to send"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "Wait for agent reply (default false)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Max seconds to wait for reply (default 60, max 300). Only applies when wait=true."
                    }
                },
                "required": ["session_id", "message"],
                "additionalProperties": false
            }),
        },
        // ── Image Analysis ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_IMAGE.into(),
            description: "Analyze an image file. Reads the image at the given path and returns it as base64-encoded data for visual analysis. Supports PNG, JPEG, GIF, WebP, BMP, TIFF. Oversized images are auto-resized. Use 'prompt' to specify what to analyze.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Image file path (supports ~ expansion)"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "What to analyze or describe about the image (optional)"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        // ── PDF Extraction ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_PDF.into(),
            description: "Extract text content from a PDF document. Supports page-range filtering and character limit. Use for reading documents, reports, and papers. Returns extracted text organized by page.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "PDF file path (supports ~ expansion)"
                    },
                    "pages": {
                        "type": "string",
                        "description": "Page range: '1-5', '3', '1-3,7,10-12'. Default: all pages."
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Max output characters (default 50000)"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        // ── Weather ─────────────────────────────────────────────
        ToolDefinition {
            name: TOOL_GET_WEATHER.into(),
            description: "Get current weather and forecast for a location. Uses Open-Meteo API (free, no API key required). Defaults to the user's configured location if no location parameter is provided.".into(),
            internal: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name (e.g. 'Shanghai', 'New York') or 'latitude,longitude' (e.g. '31.23,121.47'). If omitted, uses the user's configured location."
                    },
                    "forecast_days": {
                        "type": "integer",
                        "description": "Number of forecast days (1-16, default 1). Use 1 for current weather only."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
    ]
}

/// Returns the subagent tool definition (conditionally injected when enabled).
pub fn get_subagent_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBAGENT.into(),
        description: "Spawn and manage sub-agents to delegate tasks. Sub-agents run asynchronously — their results are automatically pushed to you when complete. Use steer to redirect a running sub-agent. Use check(wait=true) as fallback if you need to actively wait for a result.".into(),
        internal: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "check", "list", "result", "kill", "kill_all", "steer", "batch_spawn", "wait_all"],
                    "description": "Action: spawn (delegate task), check (poll/wait), list (all runs), result (full output), kill (terminate one), kill_all (terminate all), steer (redirect running sub-agent), batch_spawn (spawn multiple), wait_all (wait for multiple)"
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the sub-agent (required for spawn)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent to delegate to (default: 'default')"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID (for check/result/kill/steer)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds for spawn (default 300, max 1800)"
                },
                "wait": {
                    "type": "boolean",
                    "description": "For check: block until sub-agent completes (default false). Use as fallback if push notification was missed."
                },
                "wait_timeout": {
                    "type": "integer",
                    "description": "For check with wait=true: max seconds to wait (default 60, max 300)"
                },
                "model": {
                    "type": "string",
                    "description": "Model override: 'provider_id/model_id'"
                },
                "message": {
                    "type": "string",
                    "description": "For steer: message to inject into the running sub-agent to redirect its behavior"
                },
                "label": {
                    "type": "string",
                    "description": "For spawn: display label for tracking this run (also usable in kill to target by label)"
                },
                "tasks": {
                    "type": "array",
                    "description": "For batch_spawn: array of task objects [{task, agent_id?, label?, timeout_secs?, model?}]",
                    "items": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string" },
                            "agent_id": { "type": "string" },
                            "label": { "type": "string" },
                            "timeout_secs": { "type": "integer" },
                            "model": { "type": "string" }
                        },
                        "required": ["task"]
                    }
                },
                "run_ids": {
                    "type": "array",
                    "description": "For wait_all: array of run IDs to wait for",
                    "items": { "type": "string" }
                },
                "files": {
                    "type": "array",
                    "description": "For spawn: file attachments to pass to the sub-agent",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "File name" },
                            "content": { "type": "string", "description": "File content (UTF-8 text or base64 encoded)" },
                            "mime_type": { "type": "string", "description": "MIME type (default: text/plain)" },
                            "encoding": { "type": "string", "enum": ["utf8", "base64"], "description": "Content encoding (default: utf8)" }
                        },
                        "required": ["name", "content"]
                    }
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Get the ACP spawn tool definition (conditionally injected).
pub fn get_acp_spawn_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ACP_SPAWN.into(),
        description: "Spawn and manage external ACP agents (Claude Code, Codex CLI, Gemini CLI, etc.). External agents run as separate processes with their own tools, context, and capabilities. Use for tasks that benefit from a specialized external coding agent.".into(),
        internal: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "check", "list", "result", "kill", "kill_all", "steer", "backends"],
                    "description": "Action: spawn (start external agent), check (poll/wait), list (all runs), result (full output), kill (terminate), kill_all (terminate all), steer (send follow-up), backends (list available)"
                },
                "backend": {
                    "type": "string",
                    "description": "ACP backend ID (e.g. 'claude-code', 'codex-cli', 'gemini-cli'). Required for spawn."
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the external agent (required for spawn)"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID (for check/result/kill/steer)"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the external agent"
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the external agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 600, max 3600)"
                },
                "message": {
                    "type": "string",
                    "description": "Follow-up message to send (for steer action)"
                },
                "wait": {
                    "type": "boolean",
                    "description": "For check: block until completion (default false)"
                },
                "label": {
                    "type": "string",
                    "description": "Optional label for tracking"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Cached set of internal tool names — derived from ToolDefinition.internal flag.
/// This is the single source of truth; no separate hardcoded list needed.
static INTERNAL_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set: HashSet<String> = get_available_tools()
        .into_iter()
        .filter(|t| t.internal)
        .map(|t| t.name)
        .collect();
    // Include conditionally-injected tools
    for t in [
        get_notification_tool(),
        get_subagent_tool(),
        get_image_generate_tool(),
        get_canvas_tool(),
        get_acp_spawn_tool(),
    ] {
        if t.internal {
            set.insert(t.name);
        }
    }
    set
});

/// Check if a tool is an internal capability tool (never requires approval).
pub fn is_internal_tool(name: &str) -> bool {
    INTERNAL_TOOL_NAMES.contains(name)
}

/// Returns all tool schemas formatted for the given provider
pub fn get_tools_for_provider(provider: ToolProvider) -> Vec<serde_json::Value> {
    get_available_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}

/// Returns the image_generate tool definition (static fallback for is_internal_tool etc.).
pub fn get_image_generate_tool() -> ToolDefinition {
    get_image_generate_tool_dynamic(&crate::tools::image_generate::ImageGenConfig::default())
}

/// Returns the image_generate tool definition with dynamic description based on enabled providers.
pub fn get_image_generate_tool_dynamic(
    config: &crate::tools::image_generate::ImageGenConfig,
) -> ToolDefinition {
    use crate::tools::image_generate;

    // Build available models list from enabled providers
    let enabled: Vec<_> = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
        .collect();

    let models_desc = if enabled.is_empty() {
        "No models configured".to_string()
    } else {
        enabled
            .iter()
            .map(|p| {
                let model = image_generate::effective_model(p);
                let display = image_generate::provider_display_name(p);
                format!("{} ({})", model, display)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Build dynamic capability summaries from enabled providers
    let mut edit_providers: Vec<String> = Vec::new();
    let mut multi_image_providers: Vec<String> = Vec::new();
    let mut ar_providers: Vec<String> = Vec::new();
    let mut res_providers: Vec<String> = Vec::new();
    let mut max_n: u32 = 4;

    for p in &enabled {
        if let Some(impl_) = image_generate::resolve_provider(&p.id) {
            let caps = impl_.capabilities();
            let name = impl_.display_name().to_string();
            if caps.edit.enabled {
                let detail = if caps.edit.max_input_images > 1 {
                    format!("{} (up to {})", name, caps.edit.max_input_images)
                } else {
                    name.clone()
                };
                edit_providers.push(detail);
                if caps.edit.max_input_images > 1 {
                    multi_image_providers.push(name.clone());
                }
            }
            if caps.generate.supports_aspect_ratio {
                ar_providers.push(name.clone());
            }
            if caps.generate.supports_resolution {
                res_providers.push(name.clone());
            }
            max_n = max_n.max(caps.generate.max_count);
            if caps.edit.enabled {
                max_n = max_n.max(caps.edit.max_count);
            }
        }
    }

    let edit_desc = if edit_providers.is_empty() {
        String::new()
    } else {
        format!(
            " Supports image editing with reference images ({}).",
            edit_providers.join(", ")
        )
    };

    let description = format!(
        "Generate or edit images from text descriptions. \
         Available models (priority order): {}.{} \
         Use action='list' to see all providers with detailed capabilities. \
         Images are saved to disk and returned for visual inspection. \
         Default: auto — tries models in order with automatic failover on failure.",
        models_desc, edit_desc
    );

    let model_param_desc = if enabled.is_empty() {
        "Specify a model. Default: auto.".to_string()
    } else {
        let model_list = enabled
            .iter()
            .map(|p| format!("'{}'", image_generate::effective_model(p)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Specify a model. Available: {}. Default: auto (uses priority order with failover).",
            model_list
        )
    };

    // Dynamic descriptions for parameters
    let image_desc = if edit_providers.is_empty() {
        "Path or URL of a reference/input image for editing.".to_string()
    } else {
        format!(
            "Path or URL of a reference/input image for editing. Supported by: {}.",
            edit_providers.join(", ")
        )
    };

    let images_desc = if multi_image_providers.is_empty() {
        "Array of paths/URLs for multiple reference images (max 5 total).".to_string()
    } else {
        format!(
            "Array of paths/URLs for multiple reference images (max 5 total). Supported by: {}.",
            multi_image_providers.join(", ")
        )
    };

    let ar_desc = if ar_providers.is_empty() {
        "Aspect ratio hint: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, or 21:9.".to_string()
    } else {
        format!(
            "Aspect ratio hint: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, or 21:9. Supported by: {}.",
            ar_providers.join(", ")
        )
    };

    let res_desc = if res_providers.is_empty() {
        "Output resolution: 1K=1024px, 2K=2048px, 4K=4096px. Auto-inferred from input images when editing.".to_string()
    } else {
        format!(
            "Output resolution: 1K=1024px, 2K=2048px, 4K=4096px. Supported by: {}. Auto-inferred from input images when editing.",
            res_providers.join(", ")
        )
    };

    ToolDefinition {
        name: TOOL_IMAGE_GENERATE.into(),
        description,
        internal: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["generate", "list"],
                    "description": "Action: 'generate' (default) creates images, 'list' shows available providers and capabilities."
                },
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate or edit"
                },
                "image": {
                    "type": "string",
                    "description": image_desc
                },
                "images": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": images_desc
                },
                "size": {
                    "type": "string",
                    "description": "Image dimensions (e.g. '1024x1024', '1024x1536', '1536x1024', '1024x1792', '1792x1024'). Default: 1024x1024"
                },
                "aspectRatio": {
                    "type": "string",
                    "description": ar_desc
                },
                "resolution": {
                    "type": "string",
                    "enum": ["1K", "2K", "4K"],
                    "description": res_desc
                },
                "n": {
                    "type": "integer",
                    "description": format!("Number of images to generate (1-{} depending on provider, default 1)", max_n),
                    "minimum": 1,
                    "maximum": max_n
                },
                "model": {
                    "type": "string",
                    "description": model_param_desc
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }),
    }
}

/// Returns the web_search tool definition (conditionally injected when enabled).
pub fn get_web_search_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_WEB_SEARCH.into(),
        description: "Search the web for information. Returns relevant results with titles, URLs, and snippets. Use this when the user asks about current events, recent information, or anything that requires up-to-date knowledge.".into(),
        internal: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1-10, default from settings)"
                },
                "country": {
                    "type": "string",
                    "description": "ISO 3166-1 alpha-2 country code (e.g. 'US', 'CN'). Limits results to this country. Supported by: Brave, Google, Tavily."
                },
                "language": {
                    "type": "string",
                    "description": "ISO 639-1 language code (e.g. 'en', 'zh'). Prefer results in this language. Supported by: Brave, SearXNG, Google."
                },
                "freshness": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"],
                    "description": "Time filter: only return results from the specified period. Supported by: Brave, SearXNG, Perplexity, Google, Tavily."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// Returns the notification tool definition (conditionally injected).
pub fn get_notification_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SEND_NOTIFICATION.into(),
        description: "Send a native desktop notification to the user. Use this to proactively alert the user about important events, task completions, or findings that need their attention.".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Notification title (short, descriptive)"
                },
                "body": {
                    "type": "string",
                    "description": "Notification body text with details"
                }
            },
            "required": ["body"],
            "additionalProperties": false
        }),
    }
}

/// Returns the canvas tool definition (conditionally injected when enabled).
pub fn get_canvas_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CANVAS.into(),
        description: "Create and manage interactive canvas projects — HTML/CSS/JS live preview, documents (Markdown/code), data visualizations (Chart.js), diagrams (Mermaid), presentations (slides), and SVG graphics. Canvas content is rendered in a sandboxed preview panel visible to the user. Use snapshot to capture the current visual state for analysis.".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update", "show", "hide", "snapshot", "eval_js", "list", "delete", "versions", "restore", "export"],
                    "description": "Canvas operation to perform"
                },
                "project_id": {
                    "type": "string",
                    "description": "Canvas project ID (returned by create, required for most actions)"
                },
                "title": {
                    "type": "string",
                    "description": "Project title (for create/update)"
                },
                "content_type": {
                    "type": "string",
                    "enum": ["html", "markdown", "code", "svg", "mermaid", "chart", "slides"],
                    "description": "Content type (default: html). Determines rendering mode."
                },
                "html": {
                    "type": "string",
                    "description": "HTML content (for html/slides content_type)"
                },
                "css": {
                    "type": "string",
                    "description": "CSS styles"
                },
                "js": {
                    "type": "string",
                    "description": "JavaScript code (for html content_type or eval_js action)"
                },
                "content": {
                    "type": "string",
                    "description": "Text content (for markdown/code/svg/mermaid/chart content_type)"
                },
                "language": {
                    "type": "string",
                    "description": "Programming language (for code content_type, e.g. 'python', 'rust')"
                },
                "version_id": {
                    "type": "integer",
                    "description": "Version number (for restore action)"
                },
                "version_message": {
                    "type": "string",
                    "description": "Optional commit message for this version (for update)"
                },
                "format": {
                    "type": "string",
                    "enum": ["html", "markdown", "png"],
                    "description": "Export format (for export action)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Tool for updating plan step status (conditionally injected during Executing state).
pub fn get_plan_step_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_UPDATE_PLAN_STEP.into(),
        description: "Update the status of a plan step during plan execution. Call this after starting or completing each step to track progress in the Plan panel.".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "step_index": {
                    "type": "integer",
                    "description": "Zero-based index of the plan step to update"
                },
                "status": {
                    "type": "string",
                    "enum": ["in_progress", "completed", "skipped", "failed"],
                    "description": "New status for the step"
                }
            },
            "required": ["step_index", "status"],
            "additionalProperties": false
        }),
    }
}

/// Tool for sending structured questions to the user during plan creation.
pub fn get_plan_question_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_PLAN_QUESTION.into(),
        description: "Send structured questions to the user during plan creation. Each question includes suggested options that render as an interactive UI. The user can select options or provide custom input. Use this to clarify requirements, confirm design decisions, and gather preferences before submitting the final plan.".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to ask the user",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question_id": {
                                "type": "string",
                                "description": "Unique identifier for this question (e.g. 'q_framework', 'q_scope')"
                            },
                            "text": {
                                "type": "string",
                                "description": "The question text to display to the user"
                            },
                            "options": {
                                "type": "array",
                                "description": "Suggested options for the user to choose from (2-5 recommended)",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "value": { "type": "string", "description": "Option identifier" },
                                        "label": { "type": "string", "description": "Display text" },
                                        "description": { "type": "string", "description": "Additional explanation" },
                                        "recommended": { "type": "boolean", "description": "Mark as recommended option (renders with ★ badge)", "default": false }
                                    },
                                    "required": ["value", "label"]
                                }
                            },
                            "allow_custom": {
                                "type": "boolean",
                                "description": "Whether to show a custom input field (default: true)",
                                "default": true
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Whether the user can select multiple options (default: false)",
                                "default": false
                            },
                            "template": {
                                "type": "string",
                                "description": "Question template category for specialized UI rendering: 'scope', 'tech_choice', 'priority'",
                                "enum": ["scope", "tech_choice", "priority"]
                            }
                        },
                        "required": ["question_id", "text", "options"]
                    }
                },
                "context": {
                    "type": "string",
                    "description": "Optional context text explaining why these questions are being asked"
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

/// Tool for submitting the final plan after interactive Q&A.
pub fn get_submit_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBMIT_PLAN.into(),
        description: "Submit the final implementation plan after gathering requirements through plan_question. The plan should be structured as markdown with phased checklists. This transitions the plan to Review mode where the user can approve and start execution.".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the plan (e.g. 'Refactor Auth Module')"
                },
                "content": {
                    "type": "string",
                    "description": "Full plan content in markdown format. Must include: ## Background section, then ### Phase N: <title> headers with - [ ] checklist items"
                }
            },
            "required": ["title", "content"],
            "additionalProperties": false
        }),
    }
}

/// Tool for amending the plan during execution (insert/delete/update steps).
pub fn get_amend_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_AMEND_PLAN.into(),
        description: "Modify the current plan during execution. Use this when you discover the plan needs changes (new steps needed, steps should be removed, or step descriptions need updating). Available actions: insert (add a new step), delete (remove a pending step), update (modify a pending step's title/description).".into(),
        internal: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The amendment action to perform",
                    "enum": ["insert", "delete", "update"]
                },
                "step_index": {
                    "type": "integer",
                    "description": "Target step index (required for delete and update actions)"
                },
                "after_index": {
                    "type": "integer",
                    "description": "Insert new step after this index (for insert action). Omit to append to end."
                },
                "title": {
                    "type": "string",
                    "description": "Step title (required for insert, optional for update)"
                },
                "description": {
                    "type": "string",
                    "description": "Step description (optional)"
                },
                "phase": {
                    "type": "string",
                    "description": "Phase name (optional, defaults to 'Amended' for insert)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}
