export interface AgentInfo {
  id: string
  name: string
  emoji?: string | null
  avatar?: string | null
}

export interface ToolCall {
  callId: string
  name: string
  arguments: string
  result?: string
  mediaUrls?: string[]
  durationMs?: number
  startedAtMs?: number
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
  | { type: "thinking"; content: string; durationMs?: number }
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
  /** If true, this user message is a sub-agent result injected by the backend */
  isSubagentResult?: boolean
  /** The child agent ID that produced the sub-agent result */
  subagentResultAgentId?: string
  /** If true, this user message was triggered by a cron job */
  isCronTrigger?: boolean
  /** The cron job name that triggered this message */
  cronJobName?: string
  /** If set, this user message came from an IM channel */
  channelInbound?: {
    channelId: string
    senderName?: string
  }
  /** Model picker data for rendering interactive model selection cards */
  modelPickerData?: {
    models: { providerId: string; providerName: string; modelId: string; modelName: string }[]
    activeProviderId?: string
    activeModelId?: string
  }
  /** Database row ID, used for deduplication during streaming append */
  dbId?: number
  /** If true, this message is currently being streamed (channel streaming) */
  isStreaming?: boolean
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

export type ToolPermissionMode = "auto" | "ask_every_time" | "full_approve"

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
  channelInfo?: {
    channelId: string
    accountId: string
    chatId: string
    chatType: string
    senderName?: string | null
  } | null
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
  thinking?: string | null
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
  eventType: "spawned" | "running" | "completed" | "error" | "killed" | "timeout" | "steered"
  runId: string
  parentSessionId: string
  childAgentId: string
  childSessionId: string
  taskPreview: string
  status: "spawning" | "running" | "completed" | "error" | "timeout" | "killed"
  resultPreview?: string
  resultFull?: string
  error?: string
  durationMs?: number
  label?: string
  inputTokens?: number
  outputTokens?: number
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
  label?: string
  attachmentCount?: number
  inputTokens?: number
  outputTokens?: number
}

export interface ParentAgentStreamEvent {
  eventType: "started" | "delta" | "done" | "error"
  parentSessionId: string
  runId: string
  pushMessage?: string // only for "started"
  delta?: string // raw JSON delta string, only for "delta"
  error?: string // only for "error"
}

export interface SubagentConfig {
  enabled: boolean
  allowedAgents: string[]
  deniedAgents: string[]
  maxConcurrent: number
  defaultTimeoutSecs: number
  model?: string
  deniedTools: string[]
  maxSpawnDepth?: number
  archiveAfterMinutes?: number
  announceTimeoutSecs?: number
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
