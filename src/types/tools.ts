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
export const TOOL_TASK_CREATE = "task_create" as const
export const TOOL_TASK_UPDATE = "task_update" as const
export const TOOL_TASK_LIST = "task_list" as const
export const TOOL_MCP_RESOURCE = "mcp_resource" as const
export const TOOL_MCP_PROMPT = "mcp_prompt" as const
export const TOOL_IMAGE_GENERATE = "image_generate" as const
export const TOOL_CANVAS = "canvas" as const
export const TOOL_ACP_SPAWN = "acp_spawn" as const

/**
 * Hardcoded ID of the "main" agent. Mirrors `agent_loader::DEFAULT_AGENT_ID`
 * on the Rust side. The user can change which agent picks up new chats via
 * `AppConfig.default_agent_id`, but the literal "default" agent is always
 * the main one (it gets richer Tier 2/3 toggle defaults).
 */
export const DEFAULT_AGENT_ID = "default" as const

export const isMainAgent = (id: string) => id === DEFAULT_AGENT_ID

/**
 * Maps a Tier 3 tool name to the property key under
 * `AgentConfig.capabilities.capabilityToggles`. Mirrors
 * `CapabilityToggles::override_for` on the Rust side.
 */
export type CapabilityToggleKey =
  | "webSearch"
  | "imageGenerate"
  | "canvas"
  | "sendNotification"
  | "subagent"
  | "acpSpawn"

export const TOOL_NAME_TO_TOGGLE_KEY: Record<string, CapabilityToggleKey> = {
  [TOOL_WEB_SEARCH]: "webSearch",
  [TOOL_IMAGE_GENERATE]: "imageGenerate",
  [TOOL_CANVAS]: "canvas",
  [TOOL_SEND_NOTIFICATION]: "sendNotification",
  [TOOL_SUBAGENT]: "subagent",
  [TOOL_ACP_SPAWN]: "acpSpawn",
}

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
  [TOOL_TASK_CREATE]: "TaskCreate",
  [TOOL_TASK_UPDATE]: "TaskUpdate",
  [TOOL_TASK_LIST]: "TaskList",
  [TOOL_MCP_RESOURCE]: "McpResource",
  [TOOL_MCP_PROMPT]: "McpPrompt",
}
