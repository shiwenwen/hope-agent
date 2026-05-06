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
 * Per-channel-account IM reply split mode. Stored as a string in
 * `ChannelAccountConfig.settings.imReplyMode`. Mirrors the Rust
 * `ImReplyMode` enum.
 *
 * Only applies to non-streaming IM channels (channels whose plugin does NOT
 * advertise `supportsCardStream` or `supportsEdit`). Streaming channels
 * (telegram / discord / feishu) ignore this — every round already shows up
 * in the live preview, so a post-hoc split would just duplicate text.
 */
export type ImReplyMode = "final" | "split"

export const IM_REPLY_MODE_DEFAULT: ImReplyMode = "final"

export function readImReplyMode(account: { settings: unknown }): ImReplyMode {
  const v = (account.settings as Record<string, unknown> | null | undefined)?.imReplyMode
  return v === "split" ? "split" : IM_REPLY_MODE_DEFAULT
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
