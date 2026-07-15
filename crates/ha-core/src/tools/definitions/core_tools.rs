use serde_json::json;

use super::super::{
    TOOL_AGENTS_LIST, TOOL_APPLY_PATCH, TOOL_BROWSER, TOOL_CORE_MEMORY, TOOL_DELETE_MEMORY,
    TOOL_EDIT, TOOL_EXEC, TOOL_FIND, TOOL_GET_SETTINGS, TOOL_GET_WEATHER, TOOL_GREP, TOOL_IMAGE,
    TOOL_ISSUE_REPORT, TOOL_KNOWLEDGE_RECALL, TOOL_LIST_SETTINGS_BACKUPS, TOOL_LS, TOOL_LSP,
    TOOL_MAC_CONTROL, TOOL_MANAGE_CRON, TOOL_MEMORY_GET, TOOL_NOTE_APPEND, TOOL_NOTE_ASSIGN_BLOCK,
    TOOL_NOTE_BACKLINKS, TOOL_NOTE_BROKEN_LINKS, TOOL_NOTE_BY_TAG, TOOL_NOTE_CREATE,
    TOOL_NOTE_DELETE, TOOL_NOTE_DISTILL, TOOL_NOTE_GRAPH, TOOL_NOTE_LINK, TOOL_NOTE_MOC,
    TOOL_NOTE_MOVE, TOOL_NOTE_ORPHANS, TOOL_NOTE_PATCH, TOOL_NOTE_READ, TOOL_NOTE_RELATED,
    TOOL_NOTE_RENAME, TOOL_NOTE_SEARCH, TOOL_NOTE_SET_FRONTMATTER, TOOL_NOTE_SIMILAR,
    TOOL_NOTE_SUGGEST_LINKS, TOOL_NOTE_TAGS, TOOL_NOTE_UPDATE, TOOL_PDF, TOOL_PROCESS,
    TOOL_PROJECT_MEMORY, TOOL_READ, TOOL_RECALL_MEMORY, TOOL_RESTORE_SETTINGS_BACKUP,
    TOOL_RUNTIME_CANCEL, TOOL_SAVE_MEMORY, TOOL_SEND_ATTACHMENT, TOOL_SESSIONS_HISTORY,
    TOOL_SESSIONS_LIST, TOOL_SESSIONS_SEARCH, TOOL_SESSIONS_SEND, TOOL_SESSION_STATUS,
    TOOL_SESSION_TO_NOTE, TOOL_SKILL, TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY,
    TOOL_UPDATE_SETTINGS, TOOL_WEB_FETCH, TOOL_WRITE,
};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

pub fn get_available_tools() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: TOOL_EXEC.into(),
            description: "Execute a shell command. Returns stdout/stderr. For ordinary long-running commands, use `run_in_background: true` so the async job layer owns status, cancellation, output tail, and `<task-notification>` completion. The legacy exec-native `background`/`yield_ms` process session is reserved for cases that truly need the `process` tool's session surface; legacy flags are migrated to async jobs when async tools are enabled.".into(),
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
                        "description": "Optional command-kill timeout in seconds. Omit by default so user/system timeout policy applies. 0 = no exec command timeout. Set a positive value only when the user requested a deadline or this should be a short bounded probe; positive values are capped at 7200."
                    },
                    "env": {
                        "type": "object",
                        "description": "Environment variables to set (key-value pairs)",
                        "additionalProperties": { "type": "string" }
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Legacy process-session mode for exec-owned sessions. Prefer `run_in_background` for ordinary long-running commands; legacy flags are migrated to async jobs when async tools are enabled."
                    },
                    "yield_ms": {
                        "type": "integer",
                        "description": "Legacy process-session yield. Prefer `run_in_background` for ordinary long-running commands; legacy flags are migrated to async jobs when async tools are enabled."
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
            name: TOOL_LSP.into(),
            description: "Query Language Server Protocol code intelligence for the current workspace: diagnostics, definition, references, hover, symbols, implementation, call hierarchy, and file sync after edits. Use this when grep/find is not enough and the task depends on semantic code structure.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::FileSystem },
            internal: false,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "LSP action to perform.",
                        "enum": [
                            "status",
                            "sync_file",
                            "diagnostics",
                            "definition",
                            "references",
                            "hover",
                            "implementation",
                            "document_symbols",
                            "workspace_symbols",
                            "call_hierarchy"
                        ]
                    },
                    "path": {
                        "type": "string",
                        "description": "File path for file-scoped actions. Relative paths resolve from the session working directory."
                    },
                    "line": {
                        "type": "integer",
                        "description": "1-based line number for position-scoped actions."
                    },
                    "column": {
                        "type": "integer",
                        "description": "1-based UTF-16-ish character column. Defaults to 1 when omitted."
                    },
                    "query": {
                        "type": "string",
                        "description": "Workspace symbol query."
                    },
                    "direction": {
                        "type": "string",
                        "description": "Call hierarchy direction.",
                        "enum": ["incoming", "outgoing", "both"]
                    }
                },
                "required": ["action"],
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
                        "enum": ["global", "agent", "project"],
                        "description": "Scope: global (shared across agents), agent (private to current agent), or project (shared only inside the current project). Default: project when the current session belongs to a project; otherwise global."
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
            description: "Search persistent memories by keyword or semantic query. Use this when the current request plausibly depends on previously stored information about the user, their preferences, project context, or reference materials; do not call it routinely. Set include_history=true to also search past conversation messages.".into(),
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
            description: "Create, list, get, update, delete, and trigger scheduled tasks (cron jobs). Jobs run an agent turn with the given prompt on a schedule (isolated session, no prior history). Supports one-time (at), recurring (every), and cron expression schedules.\n\nUse this for reminders, follow-ups, and repeated nudges over time. If the user asks for something like \"remind me in 10 minutes\" or \"every 10 minutes for an hour\", create a scheduled task instead of simulating time with `exec`/`date`.\n\nProject context: pass `project_id` to bind each run's isolated session to a Project so Project instructions, Project memories, and the Project working directory are injected exactly like a normal Project chat. On create, omitting `project_id` inherits the current session's Project when there is one; pass `project_id=null` or an empty string to explicitly create a non-Project cron job. Use `action='list_projects'` to discover Project ids.\n\nResult delivery: a cron job's final output can be fanned out to one or more IM channel conversations (Telegram / WeChat / Slack / Feishu / Discord / etc.) via `delivery_targets`. Two workflows:\n\n1. When the user is chatting via an IM channel and creates a job without specifying `delivery_targets`, the job's output is delivered back to that same chat by default. Pass `delivery_targets=[]` to explicitly opt out.\n2. To fan out to other channels (or to discover target ids from a desktop chat), first call `action='list_channel_targets'` to enumerate available accounts and conversations, then pass the exact channel_id/account_id/chat_id triples.\n\nFailures are also delivered (as `⚠️ [Cron] {name} failed: {error}`) to the same targets.".into(),
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
                            "list_channel_targets", "list_projects"
                        ],
                        "description": "Action to perform. 'list_channel_targets' enumerates IM channel conversations you can pass into 'delivery_targets'. 'list_projects' enumerates Projects you can pass into 'project_id'."
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
                        "description": "Schedule type. Required on create. On update: omit to keep the existing schedule unchanged; pass it to REPLACE the schedule — you must then also supply all of that type's required fields (at→timestamp, every→interval_ms, cron→cron_expression), or the update is rejected. Other schedule fields passed WITHOUT schedule_type are ignored (the schedule stays as-is)."
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
                        "description": "IANA timezone name for a 'cron' schedule, e.g. 'Asia/Shanghai' / 'America/New_York'. The cron expression's hour/minute fields are interpreted as local wall-clock in this zone (DST-aware). Omit for UTC. Invalid names are rejected. Prefer the user's own timezone unless they ask otherwise."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The text prompt that the agent will execute when the job triggers. This runs as an isolated agent turn with no prior conversation history."
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Explicit target agent ID. When omitted and project_id is set, the Project default agent is used before falling back to the global default."
                    },
                    "project_id": {
                        "type": ["string", "null"],
                        "description": "Project ID for this scheduled task. On create, omit to inherit the current session's Project when present; pass null or an empty string to force no Project. On update, omit to leave unchanged; pass null or an empty string to clear."
                    },
                    "max_failures": {
                        "type": "integer",
                        "description": "Auto-disable the job after this many consecutive failures (default 5; 0 = never auto-disable)"
                    },
                    "job_timeout_secs": {
                        "type": "integer",
                        "description": "Optional per-run wall-clock timeout in seconds for THIS scheduled task. Omit or pass null to use the global cron default; on update a number sets it and null clears it. 0 = no cron-level timeout for this job; positive values are clamped to [30, 7200]. Do not set by default—use only when the user explicitly wants a per-task budget or this scheduled task truly needs a different budget."
                    },
                    "notify_on_complete": {
                        "type": "boolean",
                        "description": "Show a desktop notification when this job completes (default true)"
                    },
                    "prefix_delivery_with_name": {
                        "type": "boolean",
                        "description": "Prefix successful IM deliveries with `[Cron] {name}` so multiple jobs fanning out to the same chat are distinguishable (default false; failure deliveries always carry the name)."
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
                    },
                    "include_archived": {
                        "type": "boolean",
                        "description": "For action='list_projects', include archived Projects."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        // ── Browser Control ──────────────────────────────────────
        ToolDefinition {
            name: TOOL_BROWSER.into(),
            description: "Drive Chrome with Hope Agent's browser backend. Product policy is Chrome Extension + Native Messaging first, with CDP fallback only for actions that do not require the user's real Chrome tabs or logged-in session state. Eight high-level actions cover the full surface; see the `ha-browser` skill for the standard `status → tabs → snapshot → act` loop and stale-ref recovery rules. For explicit CDP lifecycle, use `profile.op=launch` with `profile=managed` (default, ephemeral isolated profile) or `profile=user_attach` (persistent, port 9222). Users can configure additional profiles in settings → Browser → Profiles.".into(),
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
                        "description": "Top-level action. `status` is read-only; `profile` manages the Chrome session (launch/connect/disconnect/list); `tabs` lists/opens/selects/closes tabs; `navigate` drives back/forward/reload/go; `snapshot` returns a role-tree, screenshot, or PDF; `act` performs the interaction (click/dblclick/fill/hover/drag/select/press/upload); `observe` reads the console/network/page_errors/downloads ring buffer; `control` covers resize/scroll/wait_for/handle_dialog/evaluate/raw_cdp/download_cancel."
                    },
                    "op": {
                        "type": "string",
                        "description": "Sub-operation for `profile` (list/launch/connect/disconnect/install_runtime — `install_runtime` downloads a Chromium snapshot when the system has no Chrome installed), `tabs` (list/new/select/close/open_user_tabs/claim/release/finalize; open_user_tabs/claim/release/finalize require the Chrome Extension and never silently fall back to CDP), `navigate` (go/back/forward/reload), or `control` (resize/scroll/wait_for/handle_dialog/evaluate/raw_cdp/download_cancel). `tabs.open_user_tabs`, `tabs.claim`, extension numeric-id `tabs.select`, `observe.downloads`, `control.raw_cdp`, and `control.download_cancel` use the normal Hope Agent tool approval flow."
                    },
                    "kind": {
                        "type": "string",
                        "description": "For `act`: click | dblclick | fill | hover | drag | select | press | upload. For `observe`: console | network | page_errors | downloads. Extension console/network/page_errors are filtered to the active controlled tab; downloads reads Chrome download activity and uses normal tool approval."
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
                        "description": "Tab target id (returned by `tabs.list`, `tabs.new`, or extension-backed `tabs.open_user_tabs`) for `tabs.select`, `tabs.close`, `tabs.claim`, `tabs.release`, and `tabs.finalize`."
                    },
                    "steal": {
                        "type": "boolean",
                        "description": "For extension-backed `tabs.claim` only: if true, explicitly steal a tab lease held by another Hope session. Defaults to false; without it a busy tab fails closed."
                    },
                    "keep": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For extension-backed `tabs.finalize`: target ids of Hope-created tabs to keep open. Claimed user Chrome tabs are released and kept open regardless."
                    },
                    "ref": {
                        "type": "integer",
                        "description": "Element ref id from the most recent `snapshot.role`. Used by every `act.kind`; also crops `snapshot.format=screenshot` to that element when provided. Stale refs are auto-recovered once for actions (re-snapshot + role+text fuzzy match) before bubbling up an error — successful recovery is flagged in the result with `(ref auto-recovered)`."
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
                    "method": {
                        "type": "string",
                        "description": "CDP method for `control.op=raw_cdp`, for example `Accessibility.getFullAXTree`, `Network.getCookies`, `Target.getTargets`, or `Runtime.evaluate`. Hope Agent validates the method name shape and then lets Chrome decide whether the method is supported."
                    },
                    "params": {
                        "type": "object",
                        "description": "JSON object of CDP parameters for `control.op=raw_cdp`. Raw CDP parameters are not payload-scanned; use this advanced path only when the normal higher-level browser action is insufficient."
                    },
                    "download_id": {
                        "type": "integer",
                        "description": "Chrome download id for `control.op=download_cancel`, from approved `observe.kind=downloads` entries."
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
                    "annotate": {
                        "type": "boolean",
                        "description": "For `snapshot.format=screenshot`: overlay visible element ref ids and bounding boxes from a fresh role snapshot. Ignored for element crop (`ref`) because crop coordinates are already scoped to one element."
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
            description: "Inspect and control the local macOS desktop through Hope Agent's native bridge. Supports `status`, `permissions`, `diagnostics` summary/export for failure analysis, `snapshot` with display/window screenshots, `visual.observe` screenshot-to-model context with optional annotated AX UI map, `visual.point` image-pixel/screen-point mapping with AX hit candidates and suggestedActions, `visual.ocr/find_text` OCR text positioning with AX-first click suggestions, `elements.find` scored AX element search, `wait` present/gone, `apps` list/frontmost/installed/search/activate/launch/quit, `dock` list/launch/hide/show/menu/select_menu, `spaces` list/switch/move_window, `windows` list/focus/move/resize/minimize/close including all-app window discovery, `act` dry_run/perform_action/click/click_point/move_cursor/double_click/right_click/type/paste/set_value/hotkey/press/scroll/drag/swipe plus dryRunOp/explain previews, `menu` list/click for app menus or system menu bar extras plus `menu.popover` for menu bar status popover detection, `clipboard` get/set/clear text, and `dialog` list/inspect/click/input/file/accept/dismiss. Prefer visual.observe annotate=true, visual.find_text, visual.point, elements.find, snapshot, or wait before mutation. Destructive quit/close/dangerous menu/dialog actions use strict approval; clipboard actions require approval because clipboard content may be sensitive.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: true },
            internal: false,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "permissions", "diagnostics", "snapshot", "visual", "elements", "wait", "apps", "dock", "spaces", "windows", "act", "menu", "clipboard", "dialog"],
                        "description": "`status` returns bridge/platform/readiness summary. `permissions` includes macOS system permissions. `diagnostics` is read-only and returns or exports readiness, snapshot-cache summaries, recent errors, and the current focus anchor. `snapshot` returns a read-only frontmost-app/window/AX element summary and optional display/window screenshot. `visual` observes a screenshot for model vision, optionally returns an annotated AX UI map, runs OCR text positioning, or maps a visual point to macOS screen points and AX hit candidates. `elements` finds and ranks AX element candidates without mutating UI. `wait` polls snapshots until a target query is present or gone. `apps`, `dock`, `spaces`, `windows`, `act`, `menu`, `clipboard`, and `dialog` perform desktop operations."
                    },
                    "op": {
                        "type": "string",
                        "enum": ["summary", "export", "observe", "point", "ocr", "find_text", "find", "present", "gone", "list", "frontmost", "installed", "search", "activate", "launch", "quit", "hide", "show", "menu", "select_menu", "switch", "move_window", "focus", "move", "resize", "minimize", "close", "dry_run", "perform_action", "click", "click_point", "move_cursor", "double_click", "right_click", "type", "paste", "set_value", "hotkey", "press", "scroll", "drag", "swipe", "popover", "input", "file", "get", "set", "clear", "inspect", "accept", "dismiss"],
                        "description": "Sub-operation. For `diagnostics`: summary|export. For `visual`: observe|point|ocr|find_text. For `elements`: find. For `wait`: present|gone. For `apps`: list|frontmost|installed|search|activate|launch|quit. For `dock`: list|launch|hide|show|menu|select_menu. For `spaces`: list|switch|move_window. For `windows`: list|focus|move|resize|minimize|close. For `act`: dry_run resolves a target without mutation and can preview `dryRunOp`, perform_action runs a named AX action after basic format validation, click for AX target clicks, click_point for raw screen coordinates, move_cursor to x/y or target center, double_click|right_click target clicks, type|paste|set_value|hotkey|press|scroll, drag/swipe between coordinate or AX element endpoints. For `menu`: list|click|popover. For `clipboard`: get|set|clear text. For `dialog`: list|inspect|click|input|file|accept|dismiss."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["app", "system"],
                        "description": "For `menu`: menu surface to inspect/click. Defaults to `app` for the frontmost app menu bar. Use `system` for macOS menu bar extras/status items."
                    },
                    "windowScope": {
                        "type": "string",
                        "enum": ["frontmost", "all"],
                        "description": "For `windows.list` and window resolution. Defaults to `frontmost`. Use `all` to list windows from all running apps; all-scope window ids have the form win_<pid>_<index> and can be reused for window mutations."
                    },
                    "appName": {
                        "type": "string",
                        "description": "For `apps` or `dock.launch`: app name query. By default this is an exact match against localized name, bundle id suffix, .app name, executable name, or Dock label. If launch/activate by name is ambiguous or fails, call apps.search/installed or dock.list and retry with bundleId/dockItemId."
                    },
                    "appNameMatch": {
                        "type": "string",
                        "enum": ["exact", "contains"],
                        "description": "For `apps` and `dock` appName matching. Defaults to exact. Use contains only for read-only discovery such as apps.search/installed or dock.list; prefer bundleId/dockItemId for mutations."
                    },
                    "appHint": {
                        "type": "string",
                        "description": "For `menu.popover`: optional app/status item hint used to rank menu bar popover candidates by app name, bundle id, window title, or OCR text."
                    },
                    "menuIndex": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "For `menu.click scope=system`: 0-based status item index from `menu.list scope=system`; ignored when a non-empty `path` is also provided. For `dock.select_menu`: 0-based Dock context menu item index from `dock.menu`; use only when `menuItem` is unavailable, because index-only Dock menu selections require strict approval."
                    },
                    "menuItem": {
                        "type": "string",
                        "description": "For `dock.select_menu`: Dock context menu item title to click, such as `Options` or `Remove from Dock`. Prefer this over `menuIndex`; when both are provided, `menuItem` is treated as the intended target."
                    },
                    "bundleId": {
                        "type": "string",
                        "description": "For `apps` or `dock.launch`: case-insensitive substring match against the app bundle id."
                    },
                    "pid": {
                        "type": "integer",
                        "description": "For `apps`: exact process id match."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "For `apps.list`: maximum running apps returned, default 50. For `diagnostics`: maximum cached snapshot summaries returned, default 10 and hard-capped at 20. For `elements.find`: maximum ranked candidates returned, default 20. For `visual.point`: maximum hit/nearest AX candidates returned, default 5. For `visual.find_text`: maximum OCR matches returned, default 5. For `menu.popover`: maximum ranked popover candidates returned, default 5 and hard-capped at 20. Other hard caps are 100 unless documented."
                    },
                    "windowId": {
                        "type": "string",
                        "description": "For `windows`: window id from the latest snapshot/list, e.g. win_1 or all-scope win_<pid>_<index>. Prefer all-scope ids when operating background app windows. For `snapshot` or `visual.observe` window screenshots: capture this AX window id; omit to capture the focused/frontmost window."
                    },
                    "dockItemId": {
                        "type": "string",
                        "description": "For `dock.launch`, `dock.menu`, or `dock.select_menu`: exact Dock item id from `dock.list`, e.g. dock_123456789. Prefer this over appName when mutating a Dock item."
                    },
                    "itemPath": {
                        "type": "string",
                        "description": "For `dock.launch`, `dock.menu`, or `dock.select_menu`: exact filesystem path or file:// URL from a Dock item. Prefer dockItemId or bundleId for app launches."
                    },
                    "spaceId": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 0,
                        "description": "For `spaces.switch`: managed Space id from `spaces.list`. Prefer this for exact Space targeting; Hope Agent resolves it to a target ManagedSpaceID."
                    },
                    "spaceIndex": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 9,
                        "default": 0,
                        "description": "For `spaces.switch`: 1-based Space index from `spaces.list`; use 0 or omit when not targeting by index. Pass exactly one of spaceId, spaceIndex, or direction."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["left", "right"],
                        "description": "For `spaces.switch`: switch to the adjacent Space. Pass exactly one of direction, spaceId, or spaceIndex."
                    },
                    "snapshotId": {
                        "type": "string",
                        "description": "For `visual.point`, `visual.ocr`, or `visual.find_text`: snapshot id returned by `visual.observe` or `snapshot includeScreenshot=true`. Omit for visual.ocr/find_text to capture a fresh screenshot first."
                    },
                    "coordinateSpace": {
                        "type": "string",
                        "enum": ["image_pixels", "screen_points"],
                        "description": "For `visual.point`: `image_pixels` means x/y are pixels from the screenshot's top-left origin; `screen_points` means x/y are macOS global screen points. Defaults to image_pixels."
                    },
                    "x": {
                        "type": "number",
                        "description": "For `visual.point`: x in coordinateSpace (`image_pixels` by default, 0 is valid). For `windows.move`, `act.click_point`, or `act.move_cursor`: x position in macOS screen points. For `act.drag`: destination x point. For `act.swipe`: start x point when using x/y source."
                    },
                    "y": {
                        "type": "number",
                        "description": "For `visual.point`: y in coordinateSpace (`image_pixels` by default, 0 is valid). For `windows.move`, `act.click_point`, or `act.move_cursor`: y position in macOS screen points. For `act.drag`: destination y point. For `act.swipe`: start y point when using x/y source."
                    },
                    "fromX": {
                        "type": "number",
                        "description": "For `act.drag` or `act.swipe`: raw source x point when not using target. For backwards compatibility, `act.swipe` also accepts x/y as the source."
                    },
                    "fromY": {
                        "type": "number",
                        "description": "For `act.drag` or `act.swipe`: raw source y point when not using target. For backwards compatibility, `act.swipe` also accepts x/y as the source."
                    },
                    "toX": {
                        "type": "number",
                        "description": "For `act.drag` or `act.swipe`: raw destination x point when not using x/y, deltaX/deltaY, or toTarget."
                    },
                    "toY": {
                        "type": "number",
                        "description": "For `act.drag` or `act.swipe`: raw destination y point when not using x/y, deltaX/deltaY, or toTarget."
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
                        "description": "For `visual.find_text`: OCR text query. For `act.type`: text to set through Accessibility, or to type character-by-character when typingProfile/typingDelayMs is provided. For `act.paste`: text to paste via the pasteboard without echoing it in the result. For `dialog.input`: text to enter into a dialog field. For `clipboard.set`: text to place on the clipboard; the result does not echo it back. For target matching, use target.text."
                    },
                    "typingProfile": {
                        "type": "string",
                        "enum": ["instant", "steady", "human"],
                        "description": "For `act.type`: when provided, type text via CGEvent Unicode key events instead of AXSetValue. `steady` uses a short fixed delay, `human` adds small deterministic jitter, and `instant` posts characters without delay."
                    },
                    "dryRunOp": {
                        "type": "string",
                        "enum": ["perform_action", "click", "click_point", "move_cursor", "double_click", "right_click", "type", "paste", "set_value", "hotkey", "press", "scroll", "drag", "swipe"],
                        "description": "For `act.dry_run`: the real act op to preview after resolving the target. Defaults to click. The result includes `preview` with executionPlan/fallbackPlan/verificationPlan/warnings without mutating UI."
                    },
                    "explain": {
                        "type": "boolean",
                        "description": "For `act`: when true, include the same structured `preview` explanation with the result. For pre-mutation review, prefer `op=dry_run` plus dryRunOp."
                    },
                    "typingDelayMs": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 1000,
                        "description": "For `act.type`: explicit per-character delay in milliseconds when using CGEvent Unicode typing. Overrides typingProfile delay and is hard-capped at 1000."
                    },
                    "textMatch": {
                        "type": "string",
                        "enum": ["exact", "contains"],
                        "description": "For `visual.find_text`: OCR text matching strategy. Defaults to exact; use contains for partial visible text."
                    },
                    "languages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For `visual.ocr`, `visual.find_text`, and `menu.popover includeOcr=true`: optional Vision recognition languages such as [\"zh-Hans\", \"en-US\"]. Omit to let Vision auto-detect."
                    },
                    "minConfidence": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 1,
                        "description": "For `visual.ocr`, `visual.find_text`, and `menu.popover includeOcr=true`: discard OCR blocks below this confidence. Defaults to 0."
                    },
                    "recognitionLevel": {
                        "type": "string",
                        "enum": ["accurate", "fast"],
                        "description": "For `visual.ocr`, `visual.find_text`, and `menu.popover includeOcr=true`: Vision recognition level. Defaults to accurate."
                    },
                    "maxChars": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20000,
                        "description": "For `clipboard.get`: maximum returned UTF-8 characters. Defaults to 4000 and is hard-capped at 20000."
                    },
                    "value": {
                        "type": "string",
                        "description": "For `act.set_value`: value to set via Accessibility."
                    },
                    "axAction": {
                        "type": "string",
                        "description": "For `act.perform_action`: Accessibility action name to perform on the resolved target. Common aliases normalize to AX names; other names are accepted if non-empty, <=128 chars, and only ASCII letters/digits/_/-. The target does not have to advertise the action in `actions[]`; unsupported actions fail at execution."
                    },
                    "key": {
                        "type": "string",
                        "description": "For `act.hotkey` or `act.press`: single key name, e.g. n, Enter, Escape, Tab."
                    },
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For `act.hotkey`: ordered keys/modifiers, e.g. [\"cmd\",\"n\"] or [\"cmd\",\"shift\",\"g\"]. For `act.press`: ordered key names to press sequentially."
                    },
                    "modifiers": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For `act.press`, `act.drag`, and `act.swipe`: modifier keys to hold during the action, e.g. [\"shift\"] or [\"cmd\",\"option\"]."
                    },
                    "repeat": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "For `act.press`: repeat count for the key sequence. Defaults to 1 and is hard-capped at 100."
                    },
                    "holdMs": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 10000,
                        "description": "For `act.press`: how long to hold each key down in milliseconds. Defaults to a short key press and is hard-capped at 10000."
                    },
                    "intervalMs": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 5000,
                        "description": "For `act.press`: delay between repeated or sequential key presses. Defaults to 0 and is hard-capped at 5000."
                    },
                    "deltaX": {
                        "type": "number",
                        "description": "For `act.scroll`: horizontal scroll delta. For `act.swipe`: horizontal movement from the start point."
                    },
                    "deltaY": {
                        "type": "number",
                        "description": "For `act.scroll`: vertical scroll delta. For `act.swipe`: vertical movement from the start point."
                    },
                    "durationMs": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 10000,
                        "description": "For `act.move_cursor`, `act.drag`, and `act.swipe`: optional motion duration in milliseconds. Defaults to a short smooth movement and is hard-capped at 10000."
                    },
                    "steps": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 240,
                        "description": "For `act.move_cursor`, `act.drag`, and `act.swipe`: optional number of interpolation points. Defaults to a short smooth movement and is hard-capped at 240."
                    },
                    "motionProfile": {
                        "type": "string",
                        "enum": ["linear", "human"],
                        "description": "For `act.move_cursor`, `act.drag`, and `act.swipe`: optional cursor path profile. `linear` preserves deterministic straight interpolation; `human` uses eased movement with small deterministic wobble and long-distance correction."
                    },
                    "path": {
                        "oneOf": [
                            { "type": "array", "items": { "type": "string" } },
                            { "type": "string" }
                        ],
                        "description": "For `menu.click`: menu path array. For `dialog.file`, string alias for filePath to match Peekaboo-style args."
                    },
                    "buttonText": {
                        "type": "string",
                        "description": "For `dialog.click`, `dialog.accept`, `dialog.dismiss`, or `dialog.file`: preferred button label. `dialog.click` requires this; accept/dismiss/file can use conservative built-in/default labels when omitted."
                    },
                    "field": {
                        "type": "string",
                        "description": "For `dialog.input`: field label, value, or element id to target. Omit to use the focused/first text field in the dialog."
                    },
                    "fieldIndex": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "For `dialog.input`: 0-based text field index within the inspected dialog."
                    },
                    "clear": {
                        "type": "boolean",
                        "description": "For `dialog.input`: replace the field value through Accessibility instead of appending/pasting at the current insertion point."
                    },
                    "filePath": {
                        "type": "string",
                        "description": "For `dialog.file`: directory or full file path to enter in a native Open/Save panel using Go to Folder."
                    },
                    "fileName": {
                        "type": "string",
                        "description": "For `dialog.file`: filename to enter in the file dialog's text field, typically for Save panels."
                    },
                    "selectButton": {
                        "type": "string",
                        "description": "For `dialog.file`: button to click after entering path/name, such as Open, Save, Choose, Cancel, or default. Omit to click the default accept-style button."
                    },
                    "ensureExpanded": {
                        "type": "boolean",
                        "description": "For `dialog.file`: best-effort expand/click Show Details before entering path/name."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "For `dialog.dismiss`: when true, send Escape if no dismiss button can be resolved."
                    },
                    "includeScreenshot": {
                        "type": "boolean",
                        "description": "For `snapshot`: capture a JPEG, store it under ~/.hope-agent/mac-control/snapshots/, and emit a Mac Control mirror frame. Requires Screen Recording permission."
                    },
                    "includeSnapshot": {
                        "type": "boolean",
                        "description": "For `act`, `wait`, or `dialog`: include the full AX snapshot used for the operation in the result. Defaults to false to keep results compact. `act.dry_run` stays compact but can return a structured preview; use `snapshot` or `elements.find` for full tree context."
                    },
                    "includeOcr": {
                        "type": "boolean",
                        "description": "For `menu.popover`: run a best-effort display OCR pass and attach visible text inside candidate popover windows. Defaults to true; set false for AX-window-only detection."
                    },
                    "verify": {
                        "type": "boolean",
                        "description": "For `menu.click scope=system`: after clicking a status item, attempt to verify/opened popover by returning `menu.popover` candidates and screenshot metadata."
                    },
                    "annotate": {
                        "type": "boolean",
                        "description": "For `visual.observe`: when true, return an annotated screenshot with AX element ids overlaid plus a compact uiMap. Defaults to false."
                    },
                    "uiMapLimit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "For `visual.observe annotate=true`: maximum annotated/uiMap AX elements. Defaults to 80 and is hard-capped at 200."
                    },
                    "screenshotTarget": {
                        "type": "string",
                        "enum": ["display", "window"],
                        "description": "For `snapshot.includeScreenshot=true` or `visual.observe`: capture a display (default) or the frontmost/specified window."
                    },
                    "displayId": {
                        "type": "integer",
                        "description": "For `snapshot` or `visual.observe` display screenshots: display id from snapshot.displays. Omit to capture the primary display."
                    },
                    "maxElements": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "description": "Maximum AX elements to traverse for snapshot, elements.find, wait, windows, dialog, or act. Defaults to 120 and is hard-capped at 500."
                    },
                    "maxDepth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 16,
                        "description": "Maximum AX tree traversal depth for snapshot, elements.find, wait, windows, dialog, act, or menu.list/click. Defaults to 8 for AX trees; menu defaults to 3 and is hard-capped at 8."
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
                        "description": "Target query for `wait`, `windows`, `act`, and `dialog`. App/window filters combine with element filters when provided.",
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
                                "description": "Exact AX element id from snapshot/elements.find/visual.observe. Prefer passing snapshotId with it so the runtime can verify and re-resolve the original element fingerprint."
                            },
                            "snapshotId": {
                                "type": "string",
                                "description": "Snapshot id that produced elementId. When provided with elementId, mutations anchor to the original element fingerprint and reject/re-resolve stale ids instead of blindly trusting a fresh el_N."
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
                    },
                    "toTarget": {
                        "type": "object",
                        "description": "Destination target query for `act.drag` and `act.swipe`, using the same fields as target: appName, bundleId, windowTitle, windowTitleMatch, elementId, snapshotId, text, role, enabled, focused. Runtime parsing uses the same strict target query type."
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
            name: TOOL_CORE_MEMORY.into(),
            description: "Read and maintain bounded Core Memory across Global, current Agent, and current Project scopes. MEMORY.md is the concise always-loaded index; topic bodies under topics/ are loaded only through this tool. Prefer topics for detail. Project scope is limited to the project bound to this session.".into(),
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_index", "append_index", "replace_index", "list", "read", "search", "write", "delete", "rebuild_index", "promote", "reload"]
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["global", "agent", "project"],
                        "description": "Defaults to agent. Project means the current session's bound project."
                    },
                    "fileName": { "type": "string" },
                    "expectedFileHash": { "type": "string" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "memoryType": {
                        "type": "string",
                        "enum": ["feedback", "project", "reference", "user"]
                    },
                    "content": { "type": "string" },
                    "query": { "type": "string" },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
                    "maxChars": { "type": "integer", "minimum": 100, "maximum": 20000 }
                    ,"sourceKind": { "type": "string", "enum": ["memory", "claim"] }
                    ,"sourceId": { "type": "string" }
                    ,"topicName": { "type": "string" }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_UPDATE_CORE_MEMORY.into(),
            description: "Compatibility alias: append or replace the Global/current-Agent Core MEMORY.md index. Prefer core_memory for scoped indexes and progressively loaded topics.".into(),
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
        // ── Project Auto Memory ────────────────────────────────
        ToolDefinition {
            name: TOOL_PROJECT_MEMORY.into(),
            description: "Manage machine-local project auto memory with progressive disclosure. MEMORY.md is a concise index loaded every project turn; topic files are not loaded until this tool reads them. Use search/read before relying on a topic, and write only durable learnings useful in future sessions.".into(),
            tier: ToolTier::Memory,
            internal: true,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "read", "search", "write", "delete", "rebuild_index"]
                    },
                    "fileName": {
                        "type": "string",
                        "description": "Safe topic filename such as project_architecture.md. Required for read/delete; optional for write."
                    },
                    "expectedFileHash": {
                        "type": "string",
                        "description": "BLAKE3 returned by read. Required when updating or deleting an existing topic; the operation fails if the disk file changed."
                    },
                    "query": { "type": "string", "description": "Case-insensitive search query." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Character offset for chunked read. Default 0."
                    },
                    "maxChars": {
                        "type": "integer",
                        "minimum": 1000,
                        "maximum": 20000,
                        "description": "Maximum characters returned by read. Default 12000."
                    },
                    "name": { "type": "string", "description": "Short topic title for write." },
                    "description": { "type": "string", "description": "One-line summary shown in MEMORY.md." },
                    "memoryType": {
                        "type": "string",
                        "enum": ["feedback", "project", "reference", "user"],
                        "description": "Topic category. Default project."
                    },
                    "content": { "type": "string", "description": "Detailed Markdown body for write." }
                },
                "required": ["action"],
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
        // ── Sessions Search ─────────────────────────────────────
        ToolDefinition {
            name: TOOL_SESSIONS_SEARCH.into(),
            description: "Search persisted chat messages and return matched messages with surrounding context windows. Defaults to the current session; use scope='all' to search visible regular non-incognito sessions. This is the preferred way to recall specific details from compressed or older conversation history.".into(),
            tier: ToolTier::Core { subclass: CoreSubclass::SessionAware },
            internal: true,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Full-text search query. Use concrete keywords, identifiers, filenames, error text, or quoted phrases."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional target session ID. Omit to search the current session. Ignored when scope='all'."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["session", "all"],
                        "description": "session: search one session (default). all: search globally visible regular non-incognito sessions."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max matches to return (default 8, max 20)."
                    },
                    "before": {
                        "type": "integer",
                        "description": "Messages to include before each hit in the context window (default 4, max 20)."
                    },
                    "after": {
                        "type": "integer",
                        "description": "Messages to include after each hit in the context window (default 4, max 20)."
                    },
                    "include_tools": {
                        "type": "boolean",
                        "description": "Include tool/text/thinking block rows in context windows (default false)."
                    }
                },
                "required": ["query"],
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
        // ── Vision Input ────────────────────────────────────────
        ToolDefinition {
            name: TOOL_IMAGE.into(),
            description: "Attach one or more images as visual input for the next model round. The tool itself does not analyze images; it packages local files, URLs/data URIs, clipboard images, or desktop screenshots so the vision-capable model can inspect them together with the supplied task/question. Use this when visual layout, screenshots, charts, rendered pages, or image content must be seen rather than read as text. Supports PNG, JPEG, GIF, WebP, BMP, TIFF; oversized images are auto-resized.".into(),
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
                        "description": format!("Array of image sources (max {}). Use this for multiple visual inputs. Add label when helpful so the next model round can refer to each image precisely.", crate::tools::image::effective_max_images()),
                        "maxItems": crate::tools::image::effective_max_images(),
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
                                },
                                "label": {
                                    "type": "string",
                                    "description": "Optional human-readable label, e.g. 'report page 1' or 'before screenshot'."
                                }
                            },
                            "required": ["type"]
                        }
                    },
                    "task": {
                        "type": "string",
                        "description": "Preferred. The visual task/question for the next model round, e.g. 'check whether text is clipped or charts overflow'."
                    },
                    "question": {
                        "type": "string",
                        "description": "Alias for task. Use when phrasing the visual request as a question."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Deprecated alias for task. Kept for backward compatibility."
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
                        "description": "Processing mode. 'auto' (default): text extraction, auto-fallback to vision for scanned PDFs. 'text': pure text extraction. 'vision': render pages as images for model vision input."
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
        // ── Issue Reporting ───────────────────────────────────
        ToolDefinition {
            name: TOOL_ISSUE_REPORT.into(),
            description: "Search, draft, or create GitHub issues for Hope Agent bugs, feature requests, and improvements. `draft` needs no token. `create` uses the configured Issue Reporting token and always asks the user to confirm before submitting.".into(),
            tier: ToolTier::Standard { default_for_main: true, default_for_others: false, default_deferred: true },
            internal: false,
            concurrent_safe: false,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["search", "draft", "create"],
                        "description": "search existing open issues, draft an issue payload, or create it after user confirmation"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["bug", "feature", "improvement"],
                        "description": "Issue type. Required for draft/create; defaults to bug if omitted."
                    },
                    "query": {
                        "type": "string",
                        "description": "Search text for action=search. If omitted, title is used."
                    },
                    "title": {
                        "type": "string",
                        "description": "Issue title for draft/create, or fallback search query."
                    },
                    "body": {
                        "type": "string",
                        "description": "Markdown issue body. Include summary, motivation, expected behavior, acceptance criteria, and relevant modules."
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Optional diagnostic evidence such as redacted logs, version/platform, session/tool failures, or reproduction notes. The tool redacts and truncates before submission."
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional label override. If omitted, labelsByKind from Issue Reporting settings are used."
                    },
                    "duplicateIssueUrls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional URLs of possible duplicates that were checked."
                    }
                },
                "required": ["action"],
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
                            "all", "user", "theme", "language", "focus_indicator", "ui_effects", "prevent_sleep", "sidebar_ui", "proxy",
                            "web_search", "web_fetch", "browser", "compact", "session_title", "notification", "startup_notification",
                            "temperature", "tool_timeout", "timeout_policy", "approval", "unattended_approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "cron", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget", "embedding",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr", "multimodal", "dreaming", "knowledge_maintenance",
                            "knowledge_media_retention", "knowledge_passive_recall", "knowledge_search", "sprite",
                            "knowledge_vision", "note_tools",
                            "recap", "awareness", "shortcuts",
                            "active_model", "fallback_models", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "issue_reporting",
                            "security", "security.ssrf", "smart_mode", "filesystem",
                            "function_models",
                            "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams",
                            "default_agent",
                            "channels", "mcp_global", "mcp_servers",
                            "hooks",
                            "local_llm_auto_maintenance",
                            "auto_update"
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
                        "description": "Update application settings for a category. HIGH-risk: proxy, shortcuts, skills, server, acp_control, skill_env, security, security.ssrf, smart_mode, mcp_global, knowledge_maintenance, knowledge_media_retention, unattended_approval, auto_update, browser — require explicit user confirmation first. `browser` gates whether the agent drives the user's real logged-in Chrome (extension backend) and toggles the raw-CDP escape hatch (extension.allowRawCdp). `knowledge_media_retention` can persist private original audio/video/image files on disk. `security` toggles the global dangerous-mode switch that skips ALL tool approvals; `smart_mode` reshapes which tool calls auto-approve; `mcp_global` is the MCP subsystem kill switch; `unattended_approval` decides whether approvals with no human surface (cron / headless / ACP / subagent) auto-deny or auto-proceed.",
                        "enum": [
                            "user", "theme", "language", "focus_indicator", "ui_effects", "prevent_sleep", "sidebar_ui", "proxy",
                            "web_search", "web_fetch", "browser", "compact", "session_title", "notification", "startup_notification",
                            "temperature", "tool_timeout", "timeout_policy", "approval", "unattended_approval",
                            "image_generate", "canvas", "image", "pdf",
                            "async_tools", "cron", "deferred_tools",
                            "memory_extract", "memory_selection", "memory_budget",
                            "embedding_cache", "dedup", "hybrid_search",
                            "temporal_decay", "mmr", "multimodal", "dreaming", "knowledge_maintenance",
                            "knowledge_media_retention", "knowledge_passive_recall", "knowledge_search", "sprite",
                            "knowledge_vision", "note_tools",
                            "recap", "awareness", "shortcuts", "skills",
                            "server", "acp_control", "skill_env",
                            "tool_result_disk_threshold",
                            "ask_user_question_timeout", "plan",
                            "issue_reporting",
                            "security", "security.ssrf", "smart_mode", "filesystem",
                            "function_models",
                            "skills_auto_review",
                            "recall_summary", "tool_call_narration", "teams",
                            "default_agent",
                            "mcp_global",
                            "local_llm_auto_maintenance",
                            "auto_update"
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

    // ── Goal Runtime (session-scoped durable objective control, always available) ──
    tools.push(super::goal_tools::get_goal_status_tool());
    tools.push(super::goal_tools::get_goal_prepare_contract_tool());
    tools.push(super::goal_tools::get_goal_checkpoint_tool());
    tools.push(super::goal_tools::get_goal_record_evidence_tool());
    tools.push(super::goal_tools::get_goal_evaluate_tool());
    tools.push(super::goal_tools::get_goal_finish_request_tool());
    tools.push(super::goal_tools::get_goal_block_request_tool());

    // ── Loop Runtime (session-scoped durable recurrence control, always available) ──
    tools.push(super::loop_tools::get_loop_status_tool());
    tools.push(super::loop_tools::get_loop_reschedule_tool());
    tools.push(super::loop_tools::get_loop_stop_tool());
    tools.push(super::loop_tools::get_loop_record_progress_tool());
    tools.push(super::loop_tools::get_loop_watch_tool());
    tools.push(super::loop_tools::get_loop_unwatch_tool());

    // ── Self-Update (Meta tier — always eager so model can suggest upgrades) ──
    tools.push(super::update_tools::get_app_update_tool());

    // ── Agent Team (deferred — discovered via tool_search) ──
    tools.push(super::special_tools::get_team_tool());

    // ── Cross-Session Peek (deferred, read-only) ──
    tools.push(crate::awareness::peek_sessions_schema());

    // ── Knowledge base (note_*) tools ──
    tools.extend(note_tools());
    tools
}

/// Knowledge base (`note_*`) tool definitions (design Layer 1). Core/Interaction
/// tier so they are always loaded; not internal so they pass through the
/// permission engine + plan-mode gating. `kb` is scoped by `effective_kb_access`
/// at execution time; writes are confined to `WorkspaceScope::for_knowledge`.
/// Exception: `knowledge_recall` (the memory+notes aggregator) is `Standard`
/// (default-deferred, discoverable via `tool_search`), not Core/always-loaded.
fn note_tools() -> Vec<ToolDefinition> {
    let interaction = || ToolTier::Core {
        subclass: CoreSubclass::Interaction,
    };
    let read_tool = |name: &str, description: &str, params: serde_json::Value| ToolDefinition {
        name: name.into(),
        description: description.into(),
        tier: interaction(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: params,
    };
    let write_tool = |name: &str, description: &str, params: serde_json::Value| ToolDefinition {
        name: name.into(),
        description: description.into(),
        tier: interaction(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: params,
    };

    vec![
        write_tool(
            TOOL_NOTE_CREATE,
            "Create a new note (markdown file) in a knowledge base. `kb` is required and must be attached with write access. `path` is relative to the KB root (`.md` appended if missing).",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id (write access required)." },
                    "path": { "type": "string", "description": "Note path relative to the KB root, e.g. 'Zettelkasten/idea'." },
                    "title": { "type": "string", "description": "Optional title (becomes an H1 if no frontmatter given)." },
                    "content": { "type": "string", "description": "Markdown body." },
                    "frontmatter": { "type": "object", "description": "Optional YAML frontmatter as key/value pairs (e.g. {title, tags})." }
                },
                "required": ["kb", "path"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_READ,
            "Read a note's raw content plus its outgoing links, backlinks, and tags. `kb` optional — when omitted, searches the accessible KB set (returns a disambiguation error on cross-KB ties). Identify the note by `path` or `title`.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Optional knowledge base id." },
                    "path": { "type": "string", "description": "Note path (folder/note) or basename." },
                    "title": { "type": "string", "description": "Note title (alternative to path)." }
                },
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_UPDATE,
            "Replace a note's full content. Pass `expected_file_hash` (from a prior note_read) to reject the write if the file changed on disk since (stale-write guard).",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "New full markdown content." },
                    "expected_file_hash": { "type": "string", "description": "Optional BLAKE3 of the file you read; write is rejected on mismatch." }
                },
                "required": ["kb", "path", "content"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_PATCH,
            "Edit a note by replacing a uniquely-matching `old` snippet with `new` (like the `edit` tool). `old` must match exactly once — 0 or 2+ matches are rejected. Optional `expected_file_hash` stale-write guard.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "old": { "type": "string", "description": "Exact text to replace (must be unique in the file)." },
                    "new": { "type": "string", "description": "Replacement text." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "path", "old", "new"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_APPEND,
            "Append content to a note, optionally under a specific `## section` heading (created if missing). Good for daily notes. Optional `expected_file_hash` stale-write guard.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "section": { "type": "string", "description": "Optional heading to append under." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "path", "content"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_DELETE,
            "Delete a note. Links pointing to it become broken (no other files are modified). Optional `expected_file_hash` stale-write guard.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "path"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_SEARCH,
            "Hybrid (full-text + vector) search over notes, returning the best-matching notes with a snippet + heading location. `kb` optional — searches the accessible KB set when omitted. Never searches inaccessible knowledge bases.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query." },
                    "kb": { "type": "string", "description": "Optional knowledge base id to restrict the search." },
                    "limit": { "type": "integer", "description": "Max notes to return (default 10, max 50)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        // Unified store-aware recall (D7): memory + knowledge notes in one call,
        // returned as two separately-ranked sections (never merged). Deferred —
        // recall_memory / note_search already cover the single-store cases eagerly;
        // the model discovers this via tool_search when it wants both at once.
        ToolDefinition {
            name: TOOL_KNOWLEDGE_RECALL.into(),
            description: "Search BOTH the memory store (one-line facts) and the knowledge notes (documents) in one call. Returns two separately-ranked sections (`memories` and `notes`) — they are NOT merged or score-normalized. Use when a question may be answered by either remembered facts or saved notes. `kb` optional (defaults to the accessible KB set); `type` filters memory type. Reads both stores only — does not write.".into(),
            tier: ToolTier::Standard {
                default_for_main: true,
                default_for_others: true,
                default_deferred: true,
            },
            internal: false,
            concurrent_safe: true,
            async_capable: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query." },
                    "kb": { "type": "string", "description": "Optional knowledge base id to restrict the notes side." },
                    "type": { "type": "string", "description": "Optional memory type filter (e.g. 'user', 'project')." },
                    "limit": { "type": "integer", "description": "Max hits per section (default 10, max 50)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        write_tool(
            TOOL_NOTE_LINK,
            "Insert a wikilink from one note to another. Phase 1: from.kb must equal to.kb. The link is appended under a section (default 'Related'). Optional `expected_file_hash` stale-write guard on the source note.",
            json!({
                "type": "object",
                "properties": {
                    "from": {
                        "type": "object",
                        "properties": { "kb": { "type": "string" }, "path": { "type": "string" } },
                        "required": ["kb", "path"]
                    },
                    "to": {
                        "type": "object",
                        "properties": { "kb": { "type": "string" }, "path": { "type": "string" } },
                        "required": ["kb", "path"]
                    },
                    "alias": { "type": "string", "description": "Optional display alias for the link." },
                    "section": { "type": "string", "description": "Heading to insert under (default 'Related')." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_BACKLINKS,
            "List the notes that link to a given note, with the exact link occurrence (line/column) for jump-to. Pass `block` (a `^block-id`) to list only references to that specific block. `kb` optional.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "note": { "type": "string", "description": "Target note path or title." },
                    "block": { "type": "string", "description": "Optional `^block-id` (with or without the leading caret) to return only block-level backlinks." }
                },
                "required": ["note"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_BY_TAG,
            "List notes carrying a tag (frontmatter or inline #tag). `kb` optional — searches the accessible KB set.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "tag": { "type": "string", "description": "Tag (with or without leading #)." }
                },
                "required": ["tag"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_TAGS,
            "Enumerate tags (with counts) across the accessible knowledge bases. `kb` optional.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_RENAME,
            "Rename or move a note within a knowledge base. Inbound `[[ ]]` links in other notes are rewritten automatically so they keep resolving. `to` is the new path relative to the KB root (a new folder is created if needed). Optional `expected_file_hash` stale-write guard on the source note.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id (write access required)." },
                    "from": { "type": "string", "description": "Current note path (folder/note)." },
                    "to": { "type": "string", "description": "New note path (folder/note); `.md` appended if missing." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "from", "to"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_MOVE,
            "Move a note to a different folder within a knowledge base (alias of note_rename — inbound `[[ ]]` links are rewritten automatically). `to` is the destination path relative to the KB root.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "from": { "type": "string", "description": "Current note path." },
                    "to": { "type": "string", "description": "Destination note path." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "from", "to"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_SET_FRONTMATTER,
            "Merge YAML frontmatter properties into a note (existing keys are preserved; a property set to null is removed). Optional `expected_file_hash` stale-write guard.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "props": { "type": "object", "description": "Frontmatter key/values to set (null value removes a key)." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "path", "props"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_ASSIGN_BLOCK,
            "Assign an Obsidian `^block-id` to a block so it can be referenced precisely with `[[Note#^id]]` / `![[Note#^id]]`. `block_text` must uniquely identify the target block (include surrounding context if needed, like note_patch's `old`). Provide `block_id` to choose the id, or omit it for a generated one. Idempotent: if the block already has an id, it's returned unchanged. Returns the ready-to-use reference. Optional `expected_file_hash` stale-write guard.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "path": { "type": "string" },
                    "block_text": { "type": "string", "description": "A unique snippet of the target block (the block whose end gets the `^id`)." },
                    "block_id": { "type": "string", "description": "Optional id (letters/digits/dashes); generated if omitted." },
                    "expected_file_hash": { "type": "string" }
                },
                "required": ["kb", "path", "block_text"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_BROKEN_LINKS,
            "List all broken (dangling) `[[ ]]` links in a knowledge base, with the source note + exact occurrence + unresolved target (a candidate note to create). `kb` is required.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" }
                },
                "required": ["kb"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_ORPHANS,
            "List orphan notes (no resolved inbound or outbound link) in a knowledge base — candidates to connect into the network. `kb` is required.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" }
                },
                "required": ["kb"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_GRAPH,
            "Return the note link graph (nodes = notes with in/out degree, edges = resolved `[[ ]]`/`![[ ]]` links). Pass `note` for that note's ego neighbourhood (`depth` 1–3, default 1); omit it for the whole-KB graph (capped — `truncated:true` flags a clipped result). `kb` optional when a `note` pins it down or only one KB is accessible.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id." },
                    "note": { "type": "string", "description": "Center note (path or title) for an ego neighbourhood; omit for the whole KB." },
                    "depth": { "type": "integer", "description": "Ego hops when `note` is given (1–3, default 1)." }
                },
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_SIMILAR,
            "Find notes semantically similar to a given note (vector nearest-neighbour). Requires a knowledge embedding model to be enabled; returns an empty result with a hint otherwise. `kb` optional.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "note": { "type": "string", "description": "Source note (path or title)." },
                    "k": { "type": "integer", "description": "Max similar notes (1–25, default 8)." }
                },
                "required": ["note"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_RELATED,
            "Fused 'related notes' for a note: backlinks ∪ resolved outgoing links ∪ vector neighbours ∪ shared tags, ranked by how many signals agree (each result lists its `reasons`). `kb` optional.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "note": { "type": "string", "description": "Source note (path or title)." }
                },
                "required": ["note"],
                "additionalProperties": false
            }),
        ),
        read_tool(
            TOOL_NOTE_SUGGEST_LINKS,
            "Suggest unlinked connections: other notes whose title/filename appears in this note's body but isn't yet a `[[ ]]` link (candidates to wire up). `kb` optional.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string" },
                    "note": { "type": "string", "description": "Note to scan (path or title)." }
                },
                "required": ["note"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_DISTILL,
            "Split a long note or pasted text into multiple atomic permanent notes (Zettelkasten). Provide `source` (an existing note path/title) OR `text` (raw content). Creates new `.md` files (2–8) under `folder` if given. Uses an LLM — may take a few seconds.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id (write access required)." },
                    "source": { "type": "string", "description": "Existing note (path or title) to distill." },
                    "text": { "type": "string", "description": "Raw text to distill (alternative to `source`)." },
                    "folder": { "type": "string", "description": "Optional destination folder for the new notes." }
                },
                "required": ["kb"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_NOTE_MOC,
            "Generate or refresh a Map-of-Content (MOC) hub note for a topic or tag, linking its related notes with [[wikilinks]]. Provide `topic` (free text, hybrid-searched) and/or `tag`. Written to `MOCs/<name>.md`. Uses an LLM.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id (write access required)." },
                    "topic": { "type": "string", "description": "Topic to gather related notes for (hybrid search)." },
                    "tag": { "type": "string", "description": "Tag to gather notes for (with or without leading #)." }
                },
                "required": ["kb"],
                "additionalProperties": false
            }),
        ),
        write_tool(
            TOOL_SESSION_TO_NOTE,
            "Distill a conversation into a single structured permanent note. `session` defaults to the current session. Refuses incognito sessions. Written to `path` or `Sessions/<title>.md`. Uses an LLM.",
            json!({
                "type": "object",
                "properties": {
                    "kb": { "type": "string", "description": "Knowledge base id (write access required)." },
                    "session": { "type": "string", "description": "Session id to distill (default: current session)." },
                    "path": { "type": "string", "description": "Optional destination note path." }
                },
                "required": ["kb"],
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::get_available_tools;

    #[test]
    fn manage_cron_schema_never_exposes_permission_or_sandbox_overrides() {
        // Regression guard (owner-plane-only red line): the per-job
        // permission_mode_override / sandbox_mode_override fields must NEVER be
        // settable by the model. They live only on the owner plane (GUI / Tauri /
        // HTTP). If a future change adds them to the manage_cron schema, an injected
        // model could schedule a `yolo` task to self-escalate unattended, or lower
        // its own sandbox. The tool-layer `NewCronJob` construction hardcodes `None`
        // and the `update` action refuses override-bearing jobs; this test pins the
        // schema so the lock can't silently regress.
        let tool = get_available_tools()
            .into_iter()
            .find(|tool| tool.name == crate::tools::TOOL_MANAGE_CRON)
            .expect("manage_cron schema");
        let schema = tool.parameters.to_string();
        for forbidden in [
            "permission_mode_override",
            "sandbox_mode_override",
            "permissionModeOverride",
            "sandboxModeOverride",
        ] {
            assert!(
                !schema.contains(forbidden),
                "manage_cron schema must not expose '{forbidden}' — these overrides are owner-plane only"
            );
        }
    }

    #[test]
    fn save_memory_schema_advertises_project_scope() {
        let tool = get_available_tools()
            .into_iter()
            .find(|tool| tool.name == crate::tools::TOOL_SAVE_MEMORY)
            .expect("save_memory schema");
        let scope = &tool.parameters["properties"]["scope"];
        let scope_enum = scope["enum"].as_array().expect("scope enum");

        assert!(scope_enum
            .iter()
            .any(|value| value.as_str() == Some("global")));
        assert!(scope_enum
            .iter()
            .any(|value| value.as_str() == Some("agent")));
        assert!(scope_enum
            .iter()
            .any(|value| value.as_str() == Some("project")));
        assert!(scope["description"]
            .as_str()
            .unwrap_or_default()
            .contains("current project"));
    }

    #[test]
    fn mac_control_schema_includes_visual_ops() {
        let tool = get_available_tools()
            .into_iter()
            .find(|tool| tool.name == crate::tools::TOOL_MAC_CONTROL)
            .expect("mac_control schema");
        let action_enum = tool.parameters["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum");
        assert!(action_enum
            .iter()
            .any(|value| value.as_str() == Some("visual")));
        assert!(action_enum
            .iter()
            .any(|value| value.as_str() == Some("diagnostics")));
        assert!(action_enum
            .iter()
            .any(|value| value.as_str() == Some("dock")));
        assert!(action_enum
            .iter()
            .any(|value| value.as_str() == Some("spaces")));

        let op_enum = tool.parameters["properties"]["op"]["enum"]
            .as_array()
            .expect("op enum");
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("summary")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("export")));
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("observe")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("point")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("ocr")));
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("find_text")));
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("perform_action")));
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("popover")));
        assert!(op_enum
            .iter()
            .any(|value| value.as_str() == Some("select_menu")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("switch")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("input")));
        assert!(op_enum.iter().any(|value| value.as_str() == Some("file")));
        assert!(tool.parameters["properties"].get("snapshotId").is_some());
        assert!(tool.parameters["properties"].get("dryRunOp").is_some());
        assert!(tool.parameters["properties"].get("explain").is_some());
        assert!(tool.parameters["properties"].get("dockItemId").is_some());
        assert!(tool.parameters["properties"].get("menuItem").is_some());
        assert!(tool.parameters["properties"].get("menuIndex").is_some());
        assert!(tool.parameters["properties"].get("appHint").is_some());
        assert!(tool.parameters["properties"].get("includeOcr").is_some());
        assert!(tool.parameters["properties"].get("spaceIndex").is_some());
        assert!(tool.parameters["properties"].get("direction").is_some());
        assert!(tool.parameters["properties"].get("field").is_some());
        assert!(tool.parameters["properties"].get("fieldIndex").is_some());
        assert!(tool.parameters["properties"].get("filePath").is_some());
        assert!(tool.parameters["properties"].get("fileName").is_some());
        assert!(tool.parameters["properties"].get("selectButton").is_some());
        assert!(tool.parameters["properties"]
            .get("coordinateSpace")
            .is_some());
        assert!(tool.parameters["properties"].get("textMatch").is_some());
        assert!(tool.parameters["properties"].get("languages").is_some());
        assert!(tool.parameters["properties"].get("minConfidence").is_some());
        assert!(tool.parameters["properties"].get("annotate").is_some());
        assert!(tool.parameters["properties"].get("uiMapLimit").is_some());
        assert!(tool.parameters["properties"].get("axAction").is_some());
    }

    #[test]
    fn image_schema_describes_vision_input_not_standalone_analysis() {
        let tool = get_available_tools()
            .into_iter()
            .find(|tool| tool.name == crate::tools::TOOL_IMAGE)
            .expect("image schema");

        assert!(tool.description.contains("Attach one or more images"));
        assert!(tool.description.contains("does not analyze images"));
        // The advertised `maxItems` must equal the effective runtime cap (the
        // configured value clamped to the hard cap), never the hard cap alone —
        // otherwise the model is told it may send more images than
        // `normalize_sources` will accept.
        let effective_cap = crate::tools::image::effective_max_images();
        assert!(effective_cap <= crate::tools::image::CAP_MAX_IMAGES);
        assert_eq!(
            tool.parameters["properties"]["images"]["maxItems"].as_u64(),
            Some(effective_cap as u64)
        );
        assert!(tool.parameters["properties"]["images"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains(&format!("max {effective_cap}")));
        assert!(tool.parameters["properties"].get("task").is_some());
        assert!(tool.parameters["properties"].get("question").is_some());
        assert!(tool.parameters["properties"].get("prompt").is_some());
        assert!(tool.parameters["properties"]["prompt"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("Deprecated alias"));
        assert!(
            tool.parameters["properties"]["images"]["items"]["properties"]
                .get("label")
                .is_some()
        );
    }
}
