import type {
  Message,
  ContentBlock,
  MediaItem,
  ToolCall,
  SessionMessage,
  MessageUsage,
} from "@/types/chat"

/** Parse `__MEDIA_ITEMS__<json>\n<text>` header from a tool result, if present.
 *  Returns the structured items; falls back to undefined on malformed JSON. */
function parseMediaItemsHeader(result: string): MediaItem[] | undefined {
  const prefix = "__MEDIA_ITEMS__"
  if (!result.startsWith(prefix)) return undefined
  const rest = result.slice(prefix.length)
  const nlIdx = rest.indexOf("\n")
  const jsonLine = nlIdx >= 0 ? rest.slice(0, nlIdx) : rest
  try {
    const parsed = JSON.parse(jsonLine)
    if (Array.isArray(parsed) && parsed.length > 0) {
      return parsed as MediaItem[]
    }
  } catch {
    /* malformed — ignore */
  }
  return undefined
}

/** Format token count: ≥10000 → "12.3k tokens", else "1,234 tokens" */
export function formatTokens(n: number): string {
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k tokens`
  return `${n.toLocaleString()} tokens`
}

/** Fold a streaming `usage` event into an existing `MessageUsage`. Shared
 *  by the main chat stream and the IM channel stream so both paths pick up
 *  new usage fields without each handler growing in lockstep. */
export function mergeUsageFromEvent(
  prev: MessageUsage | undefined,
  event: Record<string, unknown>,
): MessageUsage {
  const copyNumber = (src: string, dst: keyof MessageUsage) => {
    const v = event[src]
    return typeof v === "number" ? ({ [dst]: v } as Partial<MessageUsage>) : {}
  }
  return {
    ...(prev || {}),
    ...copyNumber("duration_ms", "durationMs"),
    ...copyNumber("input_tokens", "inputTokens"),
    ...copyNumber("output_tokens", "outputTokens"),
    ...copyNumber("cache_creation_input_tokens", "cacheCreationInputTokens"),
    ...copyNumber("cache_read_input_tokens", "cacheReadInputTokens"),
    ...copyNumber("last_input_tokens", "lastInputTokens"),
  }
}

/** Preferred token count for "how full is the context window right now":
 *  the last round's input tokens (what the model actually saw on its most
 *  recent invocation). Falls back to cumulative `inputTokens` for turns
 *  written before `lastInputTokens` existed. `??` — not `||` — so a real
 *  zero doesn't silently fall through to cumulative. */
export function getContextUsageTokens(usage?: MessageUsage): number | undefined {
  return usage?.lastInputTokens ?? usage?.inputTokens
}

/** Format message timestamp to HH:mm */
export function formatMessageTime(timestamp?: string): string {
  if (!timestamp) return ""
  try {
    const date = new Date(timestamp)
    if (isNaN(date.getTime())) return ""
    const now = new Date()
    const isToday = date.toDateString() === now.toDateString()
    const yesterday = new Date(now)
    yesterday.setDate(yesterday.getDate() - 1)
    const isYesterday = date.toDateString() === yesterday.toDateString()
    const hours = date.getHours().toString().padStart(2, "0")
    const minutes = date.getMinutes().toString().padStart(2, "0")
    const time = `${hours}:${minutes}`
    if (isToday) return time
    if (isYesterday) return `昨天 ${time}`
    const month = date.getMonth() + 1
    const day = date.getDate()
    if (date.getFullYear() === now.getFullYear()) return `${month}/${day} ${time}`
    return `${date.getFullYear()}/${month}/${day} ${time}`
  } catch {
    return ""
  }
}

/** Format duration in ms to human-readable string */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}s`
  const minutes = Math.floor(seconds / 60)
  const remainingSeconds = Math.round(seconds % 60)
  return `${minutes}m ${remainingSeconds}s`
}

/** Extract file paths modified by tool calls (write/edit/apply_patch) */
export function extractModifiedFiles(blocks: ContentBlock[]): string[] {
  const files = new Set<string>()
  for (const block of blocks) {
    if (block.type !== "tool_call") continue
    const { name, arguments: args, result } = block.tool
    if (!result) continue

    if (
      (name === "write" || name === "write_file") &&
      result.startsWith("Successfully wrote")
    ) {
      try {
        const parsed = JSON.parse(args)
        const p = parsed.path || parsed.file_path
        if (p) files.add(p)
      } catch {
        /* ignore */
      }
    } else if (
      (name === "edit" || name === "patch_file") &&
      result.startsWith("Successfully edited")
    ) {
      try {
        const parsed = JSON.parse(args)
        const p = parsed.path || parsed.file_path
        if (p) files.add(p)
      } catch {
        /* ignore */
      }
    } else if (name === "apply_patch" && result.startsWith("Patch applied")) {
      for (const line of result.split("\n")) {
        const trimmed = line.trim()
        if (trimmed.startsWith("Deleted:")) continue
        const match = trimmed.match(/^(?:Added|Modified|Renamed):\s*(.+)$/)
        if (!match) continue
        for (const entry of match[1].split(", ")) {
          const arrow = entry.indexOf(" -> ")
          const filePath = arrow >= 0 ? entry.slice(arrow + 4).trim() : entry.trim()
          if (filePath) files.add(filePath)
        }
      }
    }
  }
  return Array.from(files)
}

/** Parse DB SessionMessage[] into display Message[] */
export function parseSessionMessages(
  msgs: SessionMessage[],
  parentAgentId?: string | null,
): Message[] {
  const displayMessages: Message[] = []
  const pendingTools: ToolCall[] = []
  const pendingBlocks: ContentBlock[] = []
  let firstUserSeen = false
  for (const msg of msgs) {
    if (msg.role === "user") {
      // Detect sub-agent result / cron trigger messages via attachments_meta marker
      let isSubagentResult = false
      let subagentResultAgentId: string | undefined
      let isCronTrigger = false
      let cronJobName: string | undefined
      let channelInbound: { channelId: string; senderName?: string } | undefined
      if (msg.attachmentsMeta) {
        try {
          const meta = JSON.parse(msg.attachmentsMeta)
          if (meta?.subagent_result) {
            isSubagentResult = true
            subagentResultAgentId = meta.subagent_result.agent_id
          }
          if (meta?.cron_trigger) {
            isCronTrigger = true
            cronJobName = meta.cron_trigger.job_name
          }
          if (meta?.channel_inbound) {
            channelInbound = {
              channelId: meta.channel_inbound.channelId,
              senderName: meta.channel_inbound.senderName,
            }
          }
        } catch {
          /* ignore */
        }
      }
      const isAgentMessage = parentAgentId && !firstUserSeen
      firstUserSeen = true
      displayMessages.push({
        role: "user",
        content: msg.content,
        timestamp: msg.timestamp,
        dbId: msg.id,
        fromAgentId: isAgentMessage ? parentAgentId : undefined,
        isSubagentResult,
        subagentResultAgentId,
        isCronTrigger,
        cronJobName,
        channelInbound,
      })
    } else if (msg.role === "tool" && msg.toolCallId) {
      // Extract media info from tool results (for DB-loaded history):
      //   - image_generate still uses the old "Saved to:" text lines (mediaUrls)
      //   - send_attachment and future tools emit a `__MEDIA_ITEMS__<json>` header
      let mediaUrls: string[] | undefined
      let mediaItems: MediaItem[] | undefined
      if (msg.toolResult) {
        mediaItems = parseMediaItemsHeader(msg.toolResult)
        if (msg.toolName === "image_generate" && !mediaItems) {
          const paths = msg.toolResult
            .split("\n")
            .filter((l) => l.startsWith("Saved to: "))
            .map((l) => l.slice("Saved to: ".length).trim())
            .filter(Boolean)
          if (paths.length > 0) mediaUrls = paths
        }
      }
      const tool: ToolCall = {
        callId: msg.toolCallId,
        name: msg.toolName || "",
        arguments: msg.toolArguments || "",
        result: msg.toolResult || undefined,
        mediaUrls,
        mediaItems,
        durationMs: msg.toolDurationMs || undefined,
      }
      // Check if already exists in pendingTools (merge result)
      const existing = pendingTools.find((c) => c.callId === msg.toolCallId)
      if (existing) {
        if (msg.toolResult) existing.result = msg.toolResult
        if (msg.toolName && !existing.name) existing.name = msg.toolName
        if (msg.toolArguments && !existing.arguments) existing.arguments = msg.toolArguments
        if (msg.toolDurationMs != null) existing.durationMs = msg.toolDurationMs
        // Update matching block too
        const blockIdx = pendingBlocks.findIndex(
          (b) => b.type === "tool_call" && b.tool.callId === msg.toolCallId,
        )
        if (blockIdx >= 0) {
          pendingBlocks[blockIdx] = {
            type: "tool_call",
            tool: { ...existing },
          }
        }
      } else {
        pendingTools.push(tool)
        pendingBlocks.push({ type: "tool_call", tool })
      }
    } else if (msg.role === "thinking_block") {
      // Intermediate thinking emitted before tool calls — preserve multi-round thinking ordering
      if (msg.content) {
        pendingBlocks.push({ type: "thinking", content: msg.content, durationMs: msg.toolDurationMs || undefined })
      }
    } else if (msg.role === "text_block") {
      // Intermediate text emitted before tool calls — preserve ordering
      if (msg.content) {
        pendingBlocks.push({ type: "text", content: msg.content })
      }
    } else if (msg.role === "assistant") {
      const toolCalls = pendingTools.length > 0 ? [...pendingTools] : undefined
      // Build contentBlocks: pending blocks (text_block + tool_call in order), then remaining text
      const blocks: ContentBlock[] = [...pendingBlocks]
      if (msg.content) {
        blocks.push({ type: "text", content: msg.content })
      }
      pendingTools.length = 0
      pendingBlocks.length = 0
      const hasUsage =
        msg.toolDurationMs || msg.tokensIn || msg.tokensOut || msg.tokensInLast
      const usage: MessageUsage | undefined = hasUsage
        ? {
            durationMs: msg.toolDurationMs || undefined,
            inputTokens: msg.tokensIn || undefined,
            outputTokens: msg.tokensOut || undefined,
            lastInputTokens: msg.tokensInLast || undefined,
          }
        : undefined
      // Prepend thinking block if present (from DB history),
      // but only if no thinking_blocks were already added from pendingBlocks
      const hasThinkingBlocks = blocks.some((b) => b.type === "thinking")
      if (msg.thinking && !hasThinkingBlocks) {
        blocks.unshift({ type: "thinking", content: msg.thinking })
      }
      displayMessages.push({
        role: "assistant",
        content: msg.content,
        contentBlocks: blocks.length > 0 ? blocks : undefined,
        toolCalls,
        thinking: msg.thinking || undefined,
        timestamp: msg.timestamp,
        usage,
        model: msg.model || undefined,
        dbId: msg.id,
      })
    } else if (msg.role === "event") {
      displayMessages.push({
        role: "event",
        content: msg.content,
        timestamp: msg.timestamp,
        dbId: msg.id,
      })
    }
  }
  return displayMessages
}
