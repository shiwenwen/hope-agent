export interface ToolCall {
  callId: string
  name: string
  arguments: string
  result?: string
}

export interface MessageUsage {
  durationMs?: number
  inputTokens?: number
  outputTokens?: number
  cacheCreationInputTokens?: number
  cacheReadInputTokens?: number
}

/** Ordered content block within an assistant message */
export type ContentBlock =
  | { type: "thinking"; content: string }
  | { type: "text"; content: string }
  | { type: "tool_call"; tool: ToolCall }

export interface Message {
  role: "user" | "assistant" | "event"
  content: string
  contentBlocks?: ContentBlock[]
  toolCalls?: ToolCall[]
  thinking?: string
  timestamp?: string
  usage?: MessageUsage
  model?: string
  fallbackEvent?: FallbackEvent
  /** If set, this user message was sent by a parent agent (not a human) */
  fromAgentId?: string
}

export interface FallbackEvent {
  type?: string
  model: string
  from_model?: string
  reason?: string
  error?: string
  attempt?: number
  total?: number
  provider_id?: string
  model_id?: string
}

export interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
}

export interface ActiveModel {
  providerId: string
  modelId: string
}

export interface SessionMeta {
  id: string
  title?: string | null
  agentId: string
  providerId?: string | null
  providerName?: string | null
  modelId?: string | null
  createdAt: string
  updatedAt: string
  messageCount: number
  unreadCount: number
  isCron: boolean
  parentSessionId?: string | null
}

export interface SessionMessage {
  id: number
  sessionId: string
  role: string
  content: string
  timestamp: string
  attachmentsMeta?: string | null
  model?: string | null
  tokensIn?: number | null
  tokensOut?: number | null
  toolCallId?: string | null
  toolName?: string | null
  toolArguments?: string | null
  toolResult?: string | null
  toolDurationMs?: number | null
  isError?: boolean | null
}

export interface AgentSummaryForSidebar {
  id: string
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  notifyOnComplete?: boolean | null
}

// ── Sub-Agent Types ─────────────────────────────────────────────

export interface SubagentEvent {
  eventType: "spawned" | "running" | "completed" | "error" | "killed" | "timeout"
  runId: string
  parentSessionId: string
  childAgentId: string
  taskPreview: string
  status: "spawning" | "running" | "completed" | "error" | "timeout" | "killed"
  resultPreview?: string
  resultFull?: string
  error?: string
  durationMs?: number
}

export interface SubagentRun {
  runId: string
  parentSessionId: string
  parentAgentId: string
  childAgentId: string
  childSessionId: string
  task: string
  status: "spawning" | "running" | "completed" | "error" | "timeout" | "killed"
  result?: string
  error?: string
  depth: number
  modelUsed?: string
  startedAt: string
  finishedAt?: string
  durationMs?: number
}

export interface SubagentConfig {
  enabled: boolean
  allowedAgents: string[]
  deniedAgents: string[]
  maxConcurrent: number
  defaultTimeoutSecs: number
  model?: string
}

export function getEffortOptionsForType(apiType: string | undefined, t: (key: string) => string) {
  const off = t("effort.off")
  const on = t("effort.on")
  const minimal = t("effort.minimal")
  const low = t("effort.low")
  const medium = t("effort.medium")
  const high = t("effort.high")
  const xhigh = t("effort.xhigh")
  switch (apiType) {
    case "openai-responses":
    case "codex":
      return [
        { value: "none", label: off },
        { value: "minimal", label: minimal },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
        { value: "xhigh", label: xhigh },
      ]
    case "anthropic":
    case "openai-chat":
      return [
        { value: "none", label: off },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
      ]
    default:
      return [
        { value: "none", label: off },
        { value: "medium", label: on },
      ]
  }
}
