use serde_json::json;

use super::super::{
    TOOL_AGENTS_LIST, TOOL_APPLY_PATCH, TOOL_BROWSER, TOOL_DELETE_MEMORY, TOOL_EDIT, TOOL_EXEC,
    TOOL_FIND, TOOL_GET_SETTINGS, TOOL_GET_WEATHER, TOOL_GREP, TOOL_IMAGE,
    TOOL_LIST_SETTINGS_BACKUPS, TOOL_LS, TOOL_MANAGE_CRON, TOOL_MEMORY_GET, TOOL_PDF, TOOL_PROCESS,
    TOOL_PROJECT_READ_FILE, TOOL_READ, TOOL_RECALL_MEMORY, TOOL_RESTORE_SETTINGS_BACKUP,
    TOOL_SAVE_MEMORY, TOOL_SEND_ATTACHMENT, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST,
    TOOL_SESSIONS_SEND, TOOL_SESSION_STATUS, TOOL_SKILL, TOOL_UPDATE_CORE_MEMORY,
    TOOL_UPDATE_MEMORY, TOOL_UPDATE_SETTINGS, TOOL_WEB_FETCH, TOOL_WRITE,
};
use super::types::{is_core_tool, ToolDefinition};

pub fn get_available_tools() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: TOOL_EXEC.into(),
            description: "Execute a shell command. Returns stdout/stderr. Supports background execution with yield_ms/background params. Also supports `run_in_background: true` to detach the entire tool call as an async job whose result is auto-injected when ready.".into(),
            internal: false,
            deferred: false,
            always_load: false,
            async_capable: true,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            name: TOOL_PROJECT_READ_FILE.into(),
            description: "Read a file that has been uploaded to the CURRENT session's project. Only works when the session is attached to a project; use the `file_id` from the \"Project Files\" section of the system prompt (or `name` as a fallback). Returns extracted text with line-based pagination. Use the regular `read` tool for files outside a project.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_id": {
                        "type": "string",
                        "description": "Project file UUID from the Project Files section"
                    },
                    "name": {
                        "type": "string",
                        "description": "Project file display name, used when file_id is unknown"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (1-based). Defaults to 1"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read. Defaults to 2000, capped at 10000"
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WRITE.into(),
            description: "Write content to a file at the specified path. Creates parent directories if needed. Accepts 'file_path' as alias for 'path'.".into(),
            internal: false,
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            description: "Create, list, get, update, delete, and trigger scheduled tasks (cron jobs). Jobs run an agent turn with the given prompt on a schedule (isolated session, no prior history). Supports one-time (at), recurring (every), and cron expression schedules.\n\nResult delivery: a cron job's final output can be fanned out to one or more IM channel conversations (Telegram / WeChat / Slack / Feishu / Discord / etc.) via `delivery_targets`. Two workflows:\n\n1. When the user is chatting via an IM channel and creates a job without specifying `delivery_targets`, the job's output is delivered back to the same chat by default. Pass `delivery_targets=[]` to explicitly opt out.\n2. To fan out to other channels (or to discover target ids from a desktop chat), first call `action='list_channel_targets'` to enumerate available accounts and conversations, then pass the exact channel_id/account_id/chat_id triples.\n\nFailures are also delivered (as `⚠️ [Cron] {name} failed: {error}`) to the same targets.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "create", "update", "list", "get",
                            "delete", "pause", "resume", "run_now",
                            "list_channel_targets"
                        ],
                        "description": "Action to perform. 'list_channel_targets' enumerates IM channel conversations you can pass into 'delivery_targets'."
                    },
                    "id": {
                        "type": "string",
                        "description": "Job ID (required for get/update/delete/pause/resume/run_now)"
                    },
                    "name": {
                        "type": "string",
                        "description": "Job name (required on create; optional on update)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Job description (optional on create/update)"
                    },
                    "schedule_type": {
                        "type": "string",
                        "enum": ["at", "every", "cron"],
                        "description": "Schedule type (required on create; passing any schedule field on update replaces the schedule)"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO8601 timestamp for 'at' schedule"
                    },
                    "interval_ms": {
                        "type": "integer",
                        "description": "Interval in milliseconds for 'every' schedule (min 60000)"
                    },
                    "start_at": {
                        "type": "string",
                        "description": "Optional ISO8601 first-fire timestamp for 'every' schedules. When omitted, the backend anchors the first run at create/update time + interval."
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
                    },
                    "max_failures": {
                        "type": "integer",
                        "description": "Auto-disable the job after this many consecutive failures (default 5)"
                    },
                    "notify_on_complete": {
                        "type": "boolean",
                        "description": "Show a desktop notification when this job completes (default true)"
                    },
                    "delivery_targets": {
                        "type": "array",
                        "description": "IM channel conversations to fan the job's final output out to. If this field is omitted on `create` and the user is currently chatting via an IM channel, the job's output will be delivered back to that same chat by default. Pass `[]` to explicitly opt out. To deliver to other channels, first call `action='list_channel_targets'` to discover the exact channel_id/account_id/chat_id triples.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "channel_id": { "type": "string", "description": "e.g. 'telegram', 'feishu', 'slack'" },
                                "account_id": { "type": "string", "description": "from list_channel_targets" },
                                "chat_id":    { "type": "string", "description": "from list_channel_targets" },
                                "thread_id":  { "type": "string", "description": "optional — threaded chats (feishu topic / slack thread)" },
                                "label":      { "type": "string", "description": "optional human-readable label cached for UI display" }
                            },
                            "required": ["channel_id", "account_id", "chat_id"]
                        }
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        // ── Browser Control ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_BROWSER.into(),
            description: "Control a Chrome browser via DevTools Protocol. Supports navigation, element interaction (click/fill/hover/drag), screenshots, accessibility snapshots, JavaScript execution, tab management, profile isolation, and PDF export. `new_page` is the usual entry point: it first tries an existing Chrome at --remote-debugging-port=9222, then auto-launches a managed instance if needed. Managed launches use a large responsive window instead of chromiumoxide's default 800x600 viewport emulation. Use 'take_snapshot' to get element refs, then use those refs for click/fill/hover actions. Use 'list_profiles' to see available profiles and 'save_pdf' to export pages as PDF.".into(),
            internal: false,
            deferred: false,
            always_load: false,
            async_capable: false,
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
                        "description": "File path for save_pdf output. Defaults to ~/.hope-agent/share/page_<timestamp>.pdf"
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            deferred: false,
            always_load: false,
            async_capable: false,
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
            description: "Analyze one or more images for visual understanding. Supports multiple sources: local files, HTTP/HTTPS URLs, data URIs, system clipboard, and desktop screenshots. Up to 10 images per call — each image is sent directly to the model as raw vision data for maximum quality. Supports PNG, JPEG, GIF, WebP, BMP, TIFF. Oversized images are auto-resized.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Single image file path (shorthand for images: [{type:'file', path:'...'}]). Supports ~ expansion."
                    },
                    "url": {
                        "type": "string",
                        "description": "Single image URL (shorthand for images: [{type:'url', url:'...'}]). Supports HTTP/HTTPS and data: URIs."
                    },
                    "images": {
                        "type": "array",
                        "description": "Array of image sources (max 10). Use this for multi-image analysis.",
                        "maxItems": 10,
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["file", "url", "clipboard", "screenshot"],
                                    "description": "Source type: 'file' (local path), 'url' (HTTP/HTTPS/data URI), 'clipboard' (system clipboard image), 'screenshot' (capture desktop)"
                                },
                                "path": {
                                    "type": "string",
                                    "description": "File path (for type='file')"
                                },
                                "url": {
                                    "type": "string",
                                    "description": "URL (for type='url')"
                                },
                                "monitor": {
                                    "type": "integer",
                                    "description": "Monitor index for screenshot (default: 0 = primary)"
                                }
                            },
                            "required": ["type"]
                        }
                    },
                    "prompt": {
                        "type": "string",
                        "description": "What to analyze or describe about the image(s)"
                    }
                },
                "additionalProperties": false
            }),
        },
        // ── PDF Extraction / Vision ─────────────────────────────
        ToolDefinition {
            name: TOOL_PDF.into(),
            description: "Analyze PDF documents with text extraction or visual page rendering. Modes: 'auto' (default) extracts text, falls back to vision for scanned/image PDFs; 'text' for pure text extraction; 'vision' renders pages as images for the model to see directly. Supports local files, URLs, and multiple PDFs.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "PDF file path (supports ~ expansion). Shorthand for a single local PDF."
                    },
                    "url": {
                        "type": "string",
                        "description": "PDF URL (http/https). Shorthand for a single remote PDF."
                    },
                    "pdfs": {
                        "type": "array",
                        "description": "Multiple PDF sources (default max 5, configurable up to 10). Each item: {type:'file',path:'...'} or {type:'url',url:'...'}, or a bare string (auto-detect).",
                        "items": {},
                        "maxItems": 10
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "text", "vision"],
                        "description": "Processing mode. 'auto' (default): text extraction, auto-fallback to vision for scanned PDFs. 'text': pure text extraction. 'vision': render pages as images for visual analysis."
                    },
                    "pages": {
                        "type": "string",
                        "description": "Page range: '1-5', '3', '1-3,7,10-12'. Default: all pages."
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Max output characters for text mode (default 50000)"
                    }
                },
                "additionalProperties": false
            }),
        },
        // ── Weather ─────────────────────────────────────────────
        ToolDefinition {
            name: TOOL_GET_WEATHER.into(),
            description: "Get current weather and forecast for a location. Uses Open-Meteo API (free, no API key required). Defaults to the user's configured location if no location parameter is provided.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
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
        // ── Settings ────────────────────────────────────────────
        ToolDefinition {
            name: TOOL_GET_SETTINGS.into(),
            description: "Read application settings for a given category. Returns the current configuration as JSON. Use category 'all' for an overview of all settings.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Settings category to read. Use 'all' for an overview (includes risk-level groupings).",
                        "enum": [
                            "all", "user", "theme", "language", "ui_effects", "proxy",
                            "web_search", "web_fetch", "compact", "notification",
                            "temperature", "tool_timeout", "approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget", "embedding",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr",
                            "recap", "awareness", "shortcuts",
                            "active_model", "fallback_models", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "security", "security.ssrf", "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams"
                        ]
                    }
                },
                "required": ["category"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_UPDATE_SETTINGS.into(),
            description: "Update application settings for a given category. Accepts partial JSON — only the fields you pass are changed, others are preserved. Response includes `riskLevel` (low/medium/high); HIGH-risk categories MUST have explicit user confirmation before being called. Cannot modify providers or API keys for security. 'channels' config is HIGH-risk.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Settings category to update. HIGH-risk: proxy, embedding, shortcuts, skills, server, acp_control, skill_env, security, security.ssrf — require explicit user confirmation first. `security` toggles the global dangerous-mode switch that skips ALL tool approvals; treat with extreme caution.",
                        "enum": [
                            "user", "theme", "language", "ui_effects", "proxy",
                            "web_search", "web_fetch", "compact", "notification",
                            "temperature", "tool_timeout", "approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget", "embedding",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr",
                            "recap", "awareness", "shortcuts", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "security", "security.ssrf", "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams"
                        ]
                    },
                    "values": {
                        "type": "object",
                        "description": "JSON object with the fields to update. Only include fields you want to change."
                    }
                },
                "required": ["category", "values"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_LIST_SETTINGS_BACKUPS.into(),
            description: "List recent automatic settings backups (newest first). Every call to update_settings (or any other code path that writes config.json / user.json) creates a snapshot beforehand. Use this to show the user a rollback history; pass the returned `id` to restore_settings_backup.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Max number of entries to return (default 20, max 200).",
                        "minimum": 1,
                        "maximum": 200
                    },
                    "kind": {
                        "type": "string",
                        "description": "Optional filter by snapshot kind.",
                        "enum": ["config", "user"]
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_RESTORE_SETTINGS_BACKUP.into(),
            description: "Roll back to a previously-captured automatic settings snapshot. Creates a fresh snapshot of the current state first so the rollback itself is reversible. HIGH risk: ALWAYS confirm with the user (show the entry's timestamp, kind, and category) before calling.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Snapshot ID returned by list_settings_backups (the filename stem, e.g. '2026-04-17T10-30-45-123__config__theme__skill')."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        // ── Send Attachment (universal file delivery) ────────────
        ToolDefinition {
            name: TOOL_SEND_ATTACHMENT.into(),
            description: "Deliver a file attachment to the user (PDF, archive, doc, image, any binary). \
                          Works across all transports: desktop (FileCard + open/reveal), Web (authenticated download URL, \
                          inline preview for images/video/PDF), and IM channels (native media via Telegram / WeChat / \
                          Discord / Slack / Feishu / etc. — automatically falls back to a download link when the channel \
                          doesn't support the MIME type). Copies the file into the session's attachments directory. \
                          The `path` argument is always a server-local absolute path.".into(),
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path (supports ~) to an existing file inside the user's home directory. Max 20 MB."
                    },
                    "display_name": {
                        "type": "string",
                        "description": "Optional filename shown in the UI card. Defaults to the basename of `path`."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional short caption (<=200 chars) displayed under the file card."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        // ── Skill (activate a skill by name — preferred over read SKILL.md) ──
        ToolDefinition {
            name: TOOL_SKILL.into(),
            description: "Activate a skill from the skill catalog by name. Preferred over \
                          `read`-ing the SKILL.md file directly — this tool handles loading, \
                          optional sub-agent isolation (`context: fork` skills), and argument \
                          substitution. For inline skills it returns the SKILL.md content so \
                          you can follow its instructions; for fork skills it runs the skill \
                          in a sub-agent and returns only the final summary.".into(),
            internal: true,
            deferred: false,
            always_load: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Skill name as shown in the skill catalog (e.g. 'simplify', 'stlc-delivery')."
                    },
                    "args": {
                        "type": "string",
                        "description": "Optional arguments forwarded to the skill. Replaces `$ARGUMENTS` in the SKILL.md body for inline skills; for fork skills it becomes the task description sent to the sub-agent."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        },
    ];
    // ── Ask User Question (interactive Q&A, always available) ──
    tools.push(super::plan_tools::get_ask_user_question_tool());

    // ── Task Management (session-scoped TODO tracking, always available) ──
    tools.push(super::task_tools::get_task_create_tool());
    tools.push(super::task_tools::get_task_update_tool());
    tools.push(super::task_tools::get_task_list_tool());

    // ── Agent Team (deferred — discovered via tool_search) ──
    tools.push(super::special_tools::get_team_tool());

    // ── Cross-Session Peek (deferred, read-only) ──
    tools.push(crate::awareness::peek_sessions_schema());

    // Mark non-core tools as deferred, core tools as always_load
    for tool in &mut tools {
        if is_core_tool(&tool.name) {
            tool.always_load = true;
        } else {
            tool.deferred = true;
        }
    }
    tools
}
