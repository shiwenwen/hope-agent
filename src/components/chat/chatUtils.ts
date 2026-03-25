import type {
  Message,
  ContentBlock,
  ToolCall,
  SessionMessage,
  MessageUsage,
} from "@/types/chat"

/** Format token count: ≥10000 → "12.3k tokens", else "1,234 tokens" */
export function formatTokens(n: number): string {
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k tokens`
  return `${n.toLocaleString()} tokens`
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
      // Detect sub-agent result messages via attachments_meta marker
      let isSubagentResult = false
      let subagentResultAgentId: string | undefined
      if (msg.attachmentsMeta) {
        try {
          const meta = JSON.parse(msg.attachmentsMeta)
          if (meta?.subagent_result) {
            isSubagentResult = true
            subagentResultAgentId = meta.subagent_result.agent_id
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
      })
    } else if (msg.role === "tool" && msg.toolCallId) {
      // Extract mediaUrls from image_generate tool results (for DB-loaded history)
      let mediaUrls: string[] | undefined
      if (msg.toolName === "image_generate" && msg.toolResult) {
        const paths = msg.toolResult
          .split("\n")
          .filter((l) => l.startsWith("Saved to: "))
          .map((l) => l.slice("Saved to: ".length).trim())
          .filter(Boolean)
        if (paths.length > 0) mediaUrls = paths
      }
      const tool: ToolCall = {
        callId: msg.toolCallId,
        name: msg.toolName || "",
        arguments: msg.toolArguments || "",
        result: msg.toolResult || undefined,
        mediaUrls,
      }
      // Check if already exists in pendingTools (merge result)
      const existing = pendingTools.find((c) => c.callId === msg.toolCallId)
      if (existing) {
        if (msg.toolResult) existing.result = msg.toolResult
        if (msg.toolName && !existing.name) existing.name = msg.toolName
        if (msg.toolArguments && !existing.arguments) existing.arguments = msg.toolArguments
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
      const hasUsage = msg.toolDurationMs || msg.tokensIn || msg.tokensOut
      const usage: MessageUsage | undefined = hasUsage
        ? {
            durationMs: msg.toolDurationMs || undefined,
            inputTokens: msg.tokensIn || undefined,
            outputTokens: msg.tokensOut || undefined,
          }
        : undefined
      displayMessages.push({
        role: "assistant",
        content: msg.content,
        contentBlocks: blocks.length > 0 ? blocks : undefined,
        toolCalls,
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
