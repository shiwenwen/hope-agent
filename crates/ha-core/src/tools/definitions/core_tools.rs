use serde_json::json;

use super::super::{
    TOOL_AGENTS_LIST, TOOL_APPLY_PATCH, TOOL_BROWSER, TOOL_DELETE_MEMORY, TOOL_EDIT, TOOL_EXEC,
    TOOL_FIND, TOOL_GET_SETTINGS, TOOL_GET_WEATHER, TOOL_GREP, TOOL_IMAGE,
    TOOL_LIST_SETTINGS_BACKUPS, TOOL_LS, TOOL_MAC_CONTROL, TOOL_MANAGE_CRON, TOOL_MEMORY_GET,
    TOOL_PDF, TOOL_PROCESS, TOOL_PROJECT_READ_FILE, TOOL_READ, TOOL_RECALL_MEMORY,
    TOOL_RESTORE_SETTINGS_BACKUP, TOOL_RUNTIME_CANCEL, TOOL_SAVE_MEMORY, TOOL_SEND_ATTACHMENT,
    TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST, TOOL_SESSIONS_SEND, TOOL_SESSION_STATUS, TOOL_SKILL,
    TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY, TOOL_UPDATE_SETTINGS, TOOL_WEB_FETCH, TOOL_WRITE,
};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

pub fn get_available_tools() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: TOOL_EXEC.into(),
            description: "Execute a shell command. Returns stdout/stderr. Supports background execution with yield_ms/background params. Also supports `run_in_background: true` to detach the entire tool call as an async job whose result is auto-injected as a `<task-notification>` when ready.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: false,
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
                        "description": "Working directory for the command. Relative paths resolve from the session working directory when set, otherwise the agent home. Defaults to session working directory > agent home > user home."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds. Defaults to 1800 (30 min). Positive values are capped at 7200; set 0 to disable the exec command timeout."
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
            description: "Manage running exec sessions: list, poll, log, kill, clear, remove.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Action: list, poll, log, kill, clear, remove",
                        "enum": ["list", "poll", "log", "kill", "clear", "remove"]
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Session ID (required for all actions except list)"
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
            name: TOOL_RUNTIME_CANCEL.into(),
            description: "Cancel a running background task by id. Supports async tool jobs (`kind='async_job'` with job_id), sub-agent runs (`kind='subagent'` with run_id), exec process sessions (`kind='process'` with session_id), and running cron jobs (`kind='cron'` with job id). Cancellation is best-effort; completed tasks are not changed.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::Meta },
            internal: true,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["async_job", "subagent", "process", "cron"],
                        "description": "The kind of runtime task to cancel."
                    },
                    "id": {
                        "type": "string",
                        "description": "Task id: job_id, run_id, process session_id, or cron job id depending on kind."
                    }
                },
                "required": ["kind", "id"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_READ.into(),
            description: "Read the contents of a file at the specified path. Relative paths resolve from the session working directory when set, otherwise the agent home. Supports text files with line-based pagination (offset/limit) and image files (auto-detected, returned as base64). For large files, use offset and limit to read specific sections.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: true,
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
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: true,
            concurrent_safe: true,
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
            description: "Write content to a file at the specified path. Relative paths resolve from the session working directory when set, otherwise the agent home. Creates parent directories if needed. Accepts 'file_path' as alias for 'path'.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: false,
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
            description: "Edit a file by replacing specific text. Relative paths resolve from the session working directory when set, otherwise the agent home. More precise than write for making targeted changes. The old_text must match exactly once (including whitespace and indentation). Accepts aliases: 'file_path' for 'path', 'oldText'/'old_string' for 'old_text', 'newText'/'new_string' for 'new_text'.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: false,
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
            description: "List files and directories in the specified path. Relative paths resolve from the session working directory when set, otherwise the agent home. Returns sorted names with type indicators (/ for directories, @ for symlinks). Supports ~ expansion and entry limit.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (also accepts 'file_path'). Defaults to the session working directory when set, otherwise the agent home. Supports ~ for home directory."
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
            description: "Search file contents using regex or literal patterns. Relative paths resolve from the session working directory when set, otherwise the agent home. Respects .gitignore. Returns matching lines with file paths and line numbers.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: true,
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
                        "description": "Directory or file to search in. Defaults to the session working directory when set, otherwise the agent home. Supports ~ expansion."
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
            description: "Find files by glob pattern. Relative paths resolve from the session working directory when set, otherwise the agent home. Respects .gitignore. Returns matching file paths relative to the search directory.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: true,
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
                        "description": "Directory to search in. Defaults to the session working directory when set, otherwise the agent home. Supports ~ expansion."
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
            description: "Apply a patch to create, modify, move, or delete files. Relative paths resolve from the session working directory when set, otherwise the agent home. Use the *** Begin Patch / *** End Patch format with Add File, Update File, Delete File, and Move to markers.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: false,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: false },
            internal: false,
            concurrent_safe: true,
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
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
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
            description: "Create, list, get, update, delete, and trigger scheduled tasks (cron jobs). Jobs run an agent turn with the given prompt on a schedule (isolated session, no prior history). Supports one-time (at), recurring (every), and cron expression schedules.\n\nUse this for reminders, follow-ups, and repeated nudges over time. If the user asks for something like \"remind me in 10 minutes\" or \"every 10 minutes for an hour\", create a scheduled task instead of simulating time with `exec`/`date`.\n\nResult delivery: a cron job's final output can be fanned out to one or more IM channel conversations (Telegram / WeChat / Slack / Feishu / Discord / etc.) via `delivery_targets`. Two workflows:\n\n1. When the user is chatting via an IM channel and creates a job without specifying `delivery_targets`, the job's output is delivered back to the same chat by default. Pass `delivery_targets=[]` to explicitly opt out.\n2. To fan out to other channels (or to discover target ids from a desktop chat), first call `action='list_channel_targets'` to enumerate available accounts and conversations, then pass the exact channel_id/account_id/chat_id triples.\n\nFailures are also delivered (as `⚠️ [Cron] {name} failed: {error}`) to the same targets.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: false },
            internal: true,
            concurrent_safe: false,
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
            description: "Drive Chrome via DevTools Protocol. Eight high-level actions cover the full surface; see the `ha-browser` skill for the standard `status → tabs → snapshot → act` loop and stale-ref recovery rules. Backend is direct CDP via chromiumoxide. For `profile.op=launch`, pick `profile=managed` (default, ephemeral isolated profile) for automation or `profile=user_attach` (persistent, port 9222) for routine work where cookies / logins / extensions should survive disconnect. Users can configure additional profiles in settings → Browser → Profiles.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: true },
            internal: false,
            concurrent_safe: false,
            // async_capable enables `profile.op=install_runtime` to detach
            // into an async job; status / tabs / navigate etc. complete
            // synchronously regardless.
            async_capable: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "profile", "tabs", "navigate", "snapshot", "act", "observe", "control"],
                        "description": "Top-level action. `status` is read-only; `profile` manages the Chrome session (launch/connect/disconnect/list); `tabs` lists/opens/selects/closes tabs; `navigate` drives back/forward/reload/go; `snapshot` returns a role-tree, screenshot, or PDF; `act` performs the interaction (click/dblclick/fill/hover/drag/select/press/upload); `observe` reads the console/network/page_errors ring buffer; `control` covers resize/scroll/wait_for/handle_dialog/evaluate."
                    },
                    "op": {
                        "type": "string",
                        "description": "Sub-operation for `profile` (list/launch/connect/disconnect/install_runtime — `install_runtime` downloads a Chromium snapshot when the system has no Chrome installed), `tabs` (list/new/select/close), `navigate` (go/back/forward/reload), or `control` (resize/scroll/wait_for/handle_dialog/evaluate)."
                    },
                    "kind": {
                        "type": "string",
                        "description": "For `act`: click | dblclick | fill | hover | drag | select | press | upload. For `observe`: console | network | page_errors."
                    },
                    "format": {
                        "type": "string",
                        "description": "For `snapshot`: role | screenshot | pdf (default: role)."
                    },
                    "url": {
                        "type": "string",
                        "description": "URL for `navigate.go`, `tabs.new`, or `profile.connect` (CDP endpoint). All outbound URLs are validated against the SSRF policy before reaching Chrome."
                    },
                    "target_id": {
                        "type": "string",
                        "description": "Tab target id (returned by `tabs.list` / `tabs.new`) for `tabs.select` and `tabs.close`."
                    },
                    "ref": {
                        "type": "integer",
                        "description": "Element ref id from the most recent `snapshot.role`. Used by every `act.kind`. Stale refs are auto-recovered once (re-snapshot + role+text fuzzy match) before bubbling up an error — successful recovery is flagged in the result with `(ref auto-recovered)`."
                    },
                    "target_ref": {
                        "type": "integer",
                        "description": "Destination ref for `act.kind=drag`."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text payload for `act.kind=fill` or the substring to wait for in `control.op=wait_for`."
                    },
                    "key": {
                        "type": "string",
                        "description": "Key for `act.kind=press` (e.g. 'Enter', 'Tab', 'Escape', 'ArrowDown')."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "File path for `act.kind=upload`."
                    },
                    "values": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Option values for `act.kind=select`."
                    },
                    "expression": {
                        "type": "string",
                        "description": "JavaScript expression for `control.op=evaluate`. URL literals inside fetch/import/XHR/new URL are SSRF-checked; dynamic URL construction is NOT validated."
                    },
                    "full_page": {
                        "type": "boolean",
                        "description": "Capture full page for `snapshot.format=screenshot` (default: false)."
                    },
                    "image_format": {
                        "type": "string",
                        "enum": ["png", "jpeg"],
                        "description": "Image format for `snapshot.format=screenshot` (default: png)."
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Destination file for `snapshot.format=pdf` (default: ~/.hope-agent/share/page_<timestamp>.pdf)."
                    },
                    "paper_format": {
                        "type": "string",
                        "enum": ["a3", "a4", "a5", "letter", "legal", "tabloid"],
                        "description": "Paper size for `snapshot.format=pdf` (default: letter)."
                    },
                    "landscape": {
                        "type": "boolean",
                        "description": "Landscape orientation for `snapshot.format=pdf`."
                    },
                    "print_background": {
                        "type": "boolean",
                        "description": "Include background graphics for `snapshot.format=pdf`."
                    },
                    "width": {
                        "type": "integer",
                        "description": "Viewport width for `control.op=resize`."
                    },
                    "height": {
                        "type": "integer",
                        "description": "Viewport height for `control.op=resize`."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "left", "right"],
                        "description": "Scroll direction for `control.op=scroll` (default: down)."
                    },
                    "amount": {
                        "type": "integer",
                        "description": "Scroll amount in pixels for `control.op=scroll` (default: 500)."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in ms for `control.op=wait_for` (default: 30000)."
                    },
                    "accept": {
                        "type": "boolean",
                        "description": "Accept (true) or dismiss (false) for `control.op=handle_dialog`."
                    },
                    "dialog_text": {
                        "type": "string",
                        "description": "Prompt text reply for `control.op=handle_dialog`."
                    },
                    "since": {
                        "type": "integer",
                        "description": "Unix-millis cursor for `observe` — only entries newer than this are returned. Use the last `at` from the previous response."
                    },
                    "executable_path": {
                        "type": "string",
                        "description": "Chrome executable override for `profile.op=launch`."
                    },
                    "headless": {
                        "type": "boolean",
                        "description": "Launch headless override for `profile.op=launch`. Omit to inherit the profile/environment default (headed on desktop, headless for Docker / no-display Linux)."
                    },
                    "profile": {
                        "type": "string",
                        "description": "Profile name for `profile.op=launch`. Built-ins: `managed` (default, ephemeral, OS-picked port) for automation that should NOT inherit user logins; `user_attach` (persistent, port 9222) for routine work where cookies / logins should survive disconnect. Additional names can be configured in settings → Browser → Profiles."
                    }
                },
                "required": ["action"]
            }),
        },
        // ── macOS Control ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_MAC_CONTROL.into(),
            description: "Inspect and control the local macOS desktop through Hope Agent's native bridge. Supports `status`, `permissions`, `snapshot`, `wait` present/gone, `apps` list/frontmost/installed/search/activate/launch/quit, `windows` list/focus/move/resize/minimize/close, `act` click/click_point/double_click/right_click/type/set_value/hotkey/scroll/drag, `menu` list/click, and `dialog` inspect/accept/dismiss. Prefer snapshot/wait before mutation. Destructive quit/close/dangerous menu/dialog actions use strict approval.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: true },
            internal: false,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "permissions", "snapshot", "wait", "apps", "windows", "act", "menu", "dialog"],
                        "description": "`status` returns bridge/platform/readiness summary. `permissions` includes macOS system permissions. `snapshot` returns a read-only frontmost-app/window/AX element summary and optional screenshot. `wait` polls snapshots until a target query is present or gone. `apps`, `windows`, `act`, `menu`, and `dialog` perform desktop operations."
                    },
                    "op": {
                        "type": "string",
                        "enum": ["present", "gone", "list", "frontmost", "installed", "search", "activate", "launch", "quit", "focus", "move", "resize", "minimize", "close", "click", "click_point", "double_click", "right_click", "type", "set_value", "hotkey", "scroll", "drag", "inspect", "accept", "dismiss"],
                        "description": "Sub-operation. For `wait`: present|gone. For `apps`: list|frontmost|installed|search|activate|launch|quit. For `windows`: list|focus|move|resize|minimize|close. For `act`: click for AX target clicks, click_point for raw screen coordinates, double_click|right_click target clicks, type|set_value|hotkey|scroll, drag from target center to x/y. For `menu`: list|click. For `dialog`: inspect|accept|dismiss."
                    },
                    "appName": {
                        "type": "string",
                        "description": "For `apps`: app name query. By default this is an exact match against localized name, bundle id suffix, .app name, or executable name. If launch/activate by name is ambiguous or fails, call apps.search/installed and retry with bundleId."
                    },
                    "appNameMatch": {
                        "type": "string",
                        "enum": ["exact", "contains"],
                        "description": "For `apps` appName matching. Defaults to exact. Use contains only for read-only discovery such as apps.search/installed; prefer bundleId for mutations."
                    },
                    "bundleId": {
                        "type": "string",
                        "description": "For `apps`: case-insensitive substring match against the running app bundle id."
                    },
                    "pid": {
                        "type": "integer",
                        "description": "For `apps`: exact process id match."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "For `apps.list`: maximum running apps returned. Defaults to 50 and is hard-capped at 100."
                    },
                    "windowId": {
                        "type": "string",
                        "description": "For `windows`: window id from the latest snapshot/list, e.g. win_1. Prefer target.windowTitle when possible."
                    },
                    "x": {
                        "type": "number",
                        "description": "For `windows.move` or `act.click_point`: x position in macOS screen points. For `act.drag`: destination x point; the source is target center."
                    },
                    "y": {
                        "type": "number",
                        "description": "For `windows.move` or `act.click_point`: y position in macOS screen points. For `act.drag`: destination y point; the source is target center."
                    },
                    "width": {
                        "type": "number",
                        "description": "For `windows.resize`: target width in macOS screen points."
                    },
                    "height": {
                        "type": "number",
                        "description": "For `windows.resize`: target height in macOS screen points."
                    },
                    "text": {
                        "type": "string",
                        "description": "For `act.type`: text to set/type. For target matching, use target.text."
                    },
                    "value": {
                        "type": "string",
                        "description": "For `act.set_value`: value to set via Accessibility."
                    },
                    "key": {
                        "type": "string",
                        "description": "For `act.hotkey`: single key name, e.g. n, Enter, Escape, Tab."
                    },
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For `act.hotkey`: ordered keys/modifiers, e.g. [\"cmd\",\"n\"] or [\"cmd\",\"shift\",\"g\"]."
                    },
                    "deltaX": {
                        "type": "number",
                        "description": "For `act.scroll`: horizontal scroll delta."
                    },
                    "deltaY": {
                        "type": "number",
                        "description": "For `act.scroll`: vertical scroll delta."
                    },
                    "path": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For `menu.click`: menu title path, e.g. [\"File\", \"New Window\"]."
                    },
                    "buttonText": {
                        "type": "string",
                        "description": "For `dialog.accept` or `dialog.dismiss`: preferred button label. If omitted, accept/dismiss use conservative built-in label lists such as OK/Save/Open or Cancel/Close/Don't Save."
                    },
                    "includeScreenshot": {
                        "type": "boolean",
                        "description": "For `snapshot`: capture the primary display as a JPEG, store it under ~/.hope-agent/mac-control/snapshots/, and emit a Mac Control mirror frame. Requires Screen Recording permission."
                    },
                    "maxElements": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "description": "Maximum AX elements to return for snapshot. Defaults to 120 and is hard-capped at 500."
                    },
                    "maxDepth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 16,
                        "description": "Maximum AX tree traversal depth for `snapshot` or `wait`. Defaults to 8 and is hard-capped at 16."
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 60000,
                        "description": "For `wait`: total polling timeout in milliseconds. Defaults to 10000 and is hard-capped at 60000."
                    },
                    "pollMs": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 5000,
                        "description": "For `wait`: polling interval in milliseconds. Defaults to 500 and is clamped to 100..5000."
                    },
                    "target": {
                        "type": "object",
                        "description": "Target query for `wait`, `windows`, and `act`. App/window filters combine with element filters when provided.",
                        "properties": {
                            "appName": {
                                "type": "string",
                                "description": "Case-insensitive substring match against the frontmost app name."
                            },
                            "bundleId": {
                                "type": "string",
                                "description": "Case-insensitive substring match against the frontmost app bundle id when available."
                            },
                            "windowTitle": {
                                "type": "string",
                                "description": "Window title query. Defaults to exact matching; when element filters are present, restricts matching elements to that window."
                            },
                            "windowTitleMatch": {
                                "type": "string",
                                "enum": ["exact", "contains"],
                                "description": "Matching strategy for windowTitle. Defaults to exact. Use contains only after listing windows or when a partial title is intentional."
                            },
                            "elementId": {
                                "type": "string",
                                "description": "Exact element id match within the freshly captured snapshot."
                            },
                            "text": {
                                "type": "string",
                                "description": "Case-insensitive substring match against element label or value."
                            },
                            "role": {
                                "type": "string",
                                "description": "Case-insensitive substring match against AX role, for example AXButton or text."
                            },
                            "enabled": {
                                "type": "boolean",
                                "description": "Set true to require an enabled element. Omit for no filter; false is treated as omitted to tolerate provider-filled defaults."
                            },
                            "focused": {
                                "type": "boolean",
                                "description": "Set true to require a focused element. Omit for no filter; false is treated as omitted to tolerate provider-filled defaults."
                            }
                        },
                        "additionalProperties": false
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        // ── Memory Get ──────────────────────────────────────────
        ToolDefinition {
            name: TOOL_MEMORY_GET.into(),
            description: "Retrieve a specific memory entry by its ID with full content and metadata. Use after recall_memory to get complete details of a specific memory.".into(),
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: true },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: true },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: true, default_deferred: true },
            internal: true,
            concurrent_safe: true,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: false },
            internal: true,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Settings category to read. Use 'all' for an overview (includes risk-level groupings).",
                        "enum": [
                            "all", "user", "theme", "language", "ui_effects", "proxy",
                            "web_search", "web_fetch", "compact", "session_title", "notification", "startup_notification",
                            "temperature", "tool_timeout", "approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget", "embedding",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr", "multimodal", "dreaming",
                            "recap", "awareness", "shortcuts",
                            "active_model", "fallback_models", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "security", "security.ssrf", "smart_mode",
                            "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams",
                            "default_agent",
                            "channels", "mcp_global", "mcp_servers",
                            "local_llm_auto_maintenance"
                        ]
                    }
                },
                "required": ["category"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_UPDATE_SETTINGS.into(),
            description: "Update application settings for a given category. Accepts partial JSON — only the fields you pass are changed, others are preserved. Response includes `riskLevel` (low/medium/high); HIGH-risk categories MUST have explicit user confirmation before being called. `channels` (IM Channel bot tokens) and `mcp_servers` (MCP OAuth/env/headers) are read-only here and must be edited in the GUI; providers and API keys are also GUI-only.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: false },
            internal: true,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Settings category to update. HIGH-risk: proxy, embedding, shortcuts, skills, server, acp_control, skill_env, security, security.ssrf, smart_mode, mcp_global — require explicit user confirmation first. `security` toggles the global dangerous-mode switch that skips ALL tool approvals; `smart_mode` reshapes which tool calls auto-approve; `mcp_global` is the MCP subsystem kill switch.",
                        "enum": [
                            "user", "theme", "language", "ui_effects", "proxy",
                            "web_search", "web_fetch", "compact", "session_title", "notification", "startup_notification",
                            "temperature", "tool_timeout", "approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget", "embedding",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr", "multimodal", "dreaming",
                            "recap", "awareness", "shortcuts", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "security", "security.ssrf", "smart_mode",
                            "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams",
                            "default_agent",
                            "mcp_global",
                            "local_llm_auto_maintenance"
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: true },
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: true },
            internal: true,
            concurrent_safe: false,
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
            tier: ToolTier::Core { subclass: CoreSubclass::Interaction },
            internal: true,
            concurrent_safe: false,
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
        // ── MCP Resources (list + read for resources exposed by connected MCP servers) ──
        ToolDefinition {
            name: super::super::TOOL_MCP_RESOURCE.into(),
            description: "Read resources hosted by a connected MCP server (files, \
                          records, etc.). `action=list` to enumerate URIs, `action=read` \
                          with a specific `uri` to fetch content."
                .into(),
            tier: ToolTier::Mcp,
            internal: false,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "MCP server name (the `<name>` from `mcp__<name>__<tool>`) or its UUID."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["list", "read"],
                        "description": "`list` returns the cached resource catalog; `read` fetches the content for a specific URI."
                    },
                    "uri": {
                        "type": "string",
                        "description": "Resource URI (required when action=read). Must match one of the URIs returned by `list`."
                    }
                },
                "required": ["server", "action"],
                "additionalProperties": false
            }),
        },
        // ── MCP Prompts (list + get server-hosted prompt templates) ──
        ToolDefinition {
            name: super::super::TOOL_MCP_PROMPT.into(),
            description: "Fetch prompt templates hosted by a connected MCP server. \
                          `action=list` enumerates available prompts; `action=get` \
                          expands a prompt by `name`, optionally filling in string \
                          `arguments`."
                .into(),
            tier: ToolTier::Mcp,
            internal: false,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "MCP server name or UUID."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["list", "get"],
                        "description": "`list` returns the cached prompt catalog; `get` expands a specific prompt template."
                    },
                    "name": {
                        "type": "string",
                        "description": "Prompt name (required when action=get)."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Template arguments (string values). Required arguments are shown in the prompt's `arguments` list from `action=list`.",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["server", "action"],
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
            tier: ToolTier::Core { subclass: CoreSubclass::Meta },
            internal: true,
            concurrent_safe: false,
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

    // ── Self-Update (Meta tier — always eager so model can suggest upgrades) ──
    tools.push(super::update_tools::get_app_update_tool());

    // ── Agent Team (deferred — discovered via tool_search) ──
    tools.push(super::special_tools::get_team_tool());

    // ── Cross-Session Peek (deferred, read-only) ──
    tools.push(crate::awareness::peek_sessions_schema());
    tools
}
