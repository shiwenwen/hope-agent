export interface TelegramTopicConfig {
  requireMention?: boolean | null
  enabled?: boolean | null
  allowFrom: string[]
  agentId?: string | null
  systemPrompt?: string | null
}

export interface TelegramGroupConfig {
  requireMention?: boolean | null
  groupPolicy?: string | null
  enabled?: boolean | null
  allowFrom: string[]
  agentId?: string | null
  systemPrompt?: string | null
  topics: Record<string, TelegramTopicConfig>
}

export interface TelegramChannelConfig {
  requireMention?: boolean | null
  enabled?: boolean | null
  agentId?: string | null
  systemPrompt?: string | null
}

export interface ChannelAccountConfig {
  id: string
  channelId: string
  label: string
  enabled: boolean
  agentId?: string | null
  autoApproveTools?: boolean
  credentials: Record<string, unknown>
  settings: Record<string, unknown>
  security: {
    dmPolicy: string
    groupAllowlist: string[]
    userAllowlist: string[]
    adminIds: string[]
    groupPolicy: string
    groups: Record<string, TelegramGroupConfig>
    channels: Record<string, TelegramChannelConfig>
  }
}

export type { AgentInfo } from "@/types/chat"

export interface ChannelHealth {
  isRunning: boolean
  lastProbe: string | null
  probeOk: boolean | null
  error: string | null
  uptimeSecs: number | null
  botName: string | null
}

export interface ChannelPluginInfo {
  meta: {
    id: string
    displayName: string
    description: string
    version: string
  }
  capabilities: {
    chatTypes: string[]
    supportsPolls: boolean
    supportsReactions: boolean
    supportsEdit: boolean
    supportsMedia: string[]
    supportsTyping: boolean
    maxMessageLength: number | null
    supportsCardStream?: boolean
  }
}

/**
 * Per-channel-account IM reply mode. Stored as a string in
 * `ChannelAccountConfig.settings.imReplyMode`. Mirrors the Rust
 * `ImReplyMode` enum.
 *
 * - `split` (default): each round (narration + media) delivered in time
 *   order as independent messages. Streaming channels still get a typewriter
 *   effect *per round*, just not "one growing message".
 * - `final`: only the last-round narration + all media in one burst.
 * - `preview`: streaming channels render the full merged response in a
 *   single growing preview message; non-streaming channels degrade to
 *   `final` since they have no preview transport.
 */
export type ImReplyMode = "split" | "final" | "preview"

export const IM_REPLY_MODE_DEFAULT: ImReplyMode = "split"

export const IM_REPLY_MODE_VALUES: ImReplyMode[] = ["split", "final", "preview"]

export function readImReplyMode(account: { settings: unknown }): ImReplyMode {
  const v = (account.settings as Record<string, unknown> | null | undefined)?.imReplyMode
  if (v === "split" || v === "final" || v === "preview") return v
  return IM_REPLY_MODE_DEFAULT
}

export function channelSupportsStreamPreview(plugin: ChannelPluginInfo | undefined): boolean {
  const caps = plugin?.capabilities
  return Boolean(caps?.supportsCardStream || caps?.supportsEdit)
}

export interface WeChatConnection {
  botToken: string
  baseUrl: string
  remoteAccountId?: string | null
  userId?: string | null
}

export interface WeChatLoginStartResult {
  qrcodeUrl?: string | null
  sessionKey: string
  message: string
}

export interface WeChatLoginWaitResult {
  connected: boolean
  status?: string | null
  botToken?: string | null
  remoteAccountId?: string | null
  baseUrl?: string | null
  userId?: string | null
  message: string
}
