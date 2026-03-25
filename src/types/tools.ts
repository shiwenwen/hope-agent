/**
 * Tool name constants — must stay in sync with `src-tauri/src/tools/mod.rs`.
 */
export const TOOL_EXEC = "exec" as const
export const TOOL_PROCESS = "process" as const
export const TOOL_READ = "read" as const
export const TOOL_WRITE = "write" as const
export const TOOL_EDIT = "edit" as const
export const TOOL_LS = "ls" as const
export const TOOL_GREP = "grep" as const
export const TOOL_FIND = "find" as const
export const TOOL_APPLY_PATCH = "apply_patch" as const
export const TOOL_WEB_SEARCH = "web_search" as const
export const TOOL_WEB_FETCH = "web_fetch" as const
export const TOOL_SAVE_MEMORY = "save_memory" as const
export const TOOL_RECALL_MEMORY = "recall_memory" as const
export const TOOL_UPDATE_MEMORY = "update_memory" as const
export const TOOL_DELETE_MEMORY = "delete_memory" as const
export const TOOL_MANAGE_CRON = "manage_cron" as const
export const TOOL_BROWSER = "browser" as const
export const TOOL_SEND_NOTIFICATION = "send_notification" as const
export const TOOL_SUBAGENT = "subagent" as const

/**
 * @deprecated Use the `internal` flag from `list_builtin_tools` API response instead.
 * Kept only as a fallback — the backend ToolDefinition.internal field is the source of truth.
 */
export const INTERNAL_TOOLS = new Set([
  TOOL_SAVE_MEMORY,
  TOOL_RECALL_MEMORY,
  TOOL_UPDATE_MEMORY,
  TOOL_DELETE_MEMORY,
  TOOL_MANAGE_CRON,
  TOOL_SEND_NOTIFICATION,
])

/** Map from tool name to i18n key suffix. */
export const TOOL_I18N_KEY: Record<string, string> = {
  [TOOL_EXEC]: "Exec",
  [TOOL_PROCESS]: "Process",
  [TOOL_READ]: "Read",
  [TOOL_WRITE]: "Write",
  [TOOL_EDIT]: "Edit",
  [TOOL_LS]: "Ls",
  [TOOL_GREP]: "Grep",
  [TOOL_FIND]: "Find",
  [TOOL_APPLY_PATCH]: "ApplyPatch",
  [TOOL_WEB_SEARCH]: "WebSearch",
  [TOOL_WEB_FETCH]: "WebFetch",
  [TOOL_SAVE_MEMORY]: "SaveMemory",
  [TOOL_RECALL_MEMORY]: "RecallMemory",
  [TOOL_UPDATE_MEMORY]: "UpdateMemory",
  [TOOL_DELETE_MEMORY]: "DeleteMemory",
  [TOOL_MANAGE_CRON]: "ManageCron",
  [TOOL_BROWSER]: "Browser",
  [TOOL_SEND_NOTIFICATION]: "SendNotification",
  [TOOL_SUBAGENT]: "Subagent",
}
