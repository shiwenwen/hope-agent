export interface ToolCall {
  callId: string
  name: string
  arguments: string
  result?: string
}

export interface Message {
  role: "user" | "assistant" | "event"
  content: string
  toolCalls?: ToolCall[]
  thinking?: string
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
  providerName?: string | null
  modelId?: string | null
  createdAt: string
  updatedAt: string
  messageCount: number
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
}

export function getEffortOptionsForType(apiType: string | undefined, t: (key: string) => string) {
  const off = t("effort.off")
  const on = t("effort.on")
  const low = t("effort.low")
  const medium = t("effort.medium")
  const high = t("effort.high")
  const xhigh = t("effort.xhigh")
  switch (apiType) {
    case "openai-responses":
    case "codex":
      return [
        { value: "none", label: off },
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
