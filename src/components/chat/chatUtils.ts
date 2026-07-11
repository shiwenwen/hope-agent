import type * as React from "react"
import type {
  Message,
  ContentBlock,
  MessageAttachment,
  MediaItem,
  ToolCall,
  SessionMessage,
  SessionMeta,
  SessionSearchResult,
  MessageUsage,
} from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import i18n from "@/i18n/i18n"
import {
  contextCompactionData,
  isContextCompactionPayload,
  parseEventPayload,
  shouldReplaceContextCompactionNotice,
} from "./contextCompactionEvents"
import { hasToolError } from "./message/executionStatus"
import { MAX_MESSAGES, KEEP_AFTER_CAP } from "./hooks/constants"

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

/** Parse tool media persisted in `messages.attachments_meta`. Realtime stream
 *  events carry `media_items` directly; this path restores the same FileCard /
 *  image preview after history reload. */
function parseToolMediaItemsMeta(metaJson: string | null | undefined): MediaItem[] | undefined {
  if (!metaJson) return undefined
  try {
    const meta = JSON.parse(metaJson)
    const items = meta?.tool_media_items
    if (Array.isArray(items) && items.length > 0) {
      return items as MediaItem[]
    }
  } catch {
    /* malformed — ignore */
  }
  return undefined
}

/** True when a message should render as a centered system chip (event,
 *  sub-agent result, cron trigger, plan-mode approve/resume) rather than as
 *  a user/assistant bubble. */
export function isCenteredSystemMessage(msg: Message): boolean {
  if (msg.slashEvent?.displayAs === "user") return false
  return (
    msg.role === "event" ||
    !!msg.isSubagentResult ||
    !!msg.isCronTrigger ||
    !!msg.isWakeupTrigger ||
    !!msg.isLoopTrigger ||
    !!msg.isProcessNotification ||
    !!msg.isWorkflowResult ||
    !!msg.isPlanTrigger
  )
}

/** True when a message should align and style like a human user bubble. */
export function isUserAlignedMessage(msg: Message): boolean {
  if (isCenteredSystemMessage(msg)) return false
  return msg.role === "user" || msg.slashEvent?.displayAs === "user"
}

function isInterruptedStreamStatus(status: SessionMessage["streamStatus"]): boolean {
  return status === "orphaned" || status === "recovered" || status === "streaming"
}

function isStartupRecoveryNotice(content: string): boolean {
  return (
    content === "上次会话异常中断,已保留中断前的内容" ||
    content === "应用已关闭,中断前的内容已保留"
  )
}

function upsertContextCompactionEventMessage(
  displayMessages: Message[],
  nextMessage: Message,
): boolean {
  const nextPayload = parseEventPayload(nextMessage.content)
  if (!isContextCompactionPayload(nextPayload)) return false

  const previous = displayMessages[displayMessages.length - 1]
  const previousPayload =
    previous?.role === "event" ? parseEventPayload(previous.content) : null
  if (!isContextCompactionPayload(previousPayload)) {
    displayMessages.push(nextMessage)
    return true
  }
  if (shouldReplaceContextCompactionNotice(previousPayload, nextPayload)) {
    displayMessages[displayMessages.length - 1] = nextMessage
  }
  return true
}

/** Format token count: ≥10000 → "12.3k", else "1,234". */
export function formatTokens(n: number): string {
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k`
  return n.toLocaleString()
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

export interface ContextUsageInfo {
  /** Tokens the model saw on its most recent call (the latest assistant turn). */
  usedTokens: number
  /** The active model's context window, in tokens. */
  contextWindow: number
  /** `usedTokens` rounded to thousands, for `{usedK}k/{ctxK}k` displays. */
  usedK: number
  ctxK: number
  /** Fullness 0–100 (rounded; may exceed 100 in rare over-window cases). */
  pct: number
}

/**
 * How full the context window is, derived from the most recent assistant turn
 * that carries usage. Iterates from the end (no array copy) — see
 * `getContextUsageTokens` for the cumulative-vs-last-round rule. Returns null
 * when the model's window is unknown, so call sites can skip the UI entirely.
 *
 * Single source of truth shared by the status popover, the workspace session
 * card, and the input-dock bottom bar so all three never drift.
 */
/**
 * Build a `ContextUsageInfo` from a raw used-token count and a context window.
 * Single source of truth for the usedK/ctxK/pct derivation, shared by
 * `computeContextUsage` (latest-assistant scan) and the manual-compaction usage
 * override in ChatScreen. Returns null when the window is unknown / non-positive.
 */
export function formatContextUsage(
  usedTokens: number,
  contextWindow: number | null | undefined,
): ContextUsageInfo | null {
  if (!contextWindow || contextWindow <= 0) return null
  const safeUsedTokens = Math.max(0, usedTokens)
  return {
    usedTokens: safeUsedTokens,
    contextWindow,
    usedK: Math.round(safeUsedTokens / 1000),
    ctxK: Math.round(contextWindow / 1000),
    pct: Math.round((safeUsedTokens / contextWindow) * 100),
  }
}

export function computeContextUsage(
  messages: Message[],
  contextWindow: number | null | undefined,
): ContextUsageInfo | null {
  if (!contextWindow || contextWindow <= 0) return null
  let usedTokens = 0
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role === "event") {
      const payload = parseEventPayload(m.content)
      if (payload?.type === "context_compacted" && isContextCompactionPayload(payload)) {
        const data = contextCompactionData(payload)
        const tokensAfter = data.tokens_after
        if (typeof tokensAfter === "number") {
          usedTokens = tokensAfter
          break
        }
      }
      continue
    }

    if (m.role === "assistant") {
      const tok = getContextUsageTokens(m.usage)
      if (tok != null) {
        usedTokens = tok
        break
      }
    }
  }
  return formatContextUsage(usedTokens, contextWindow)
}

// Pure color/level helpers live in a dependency-free leaf module so the input
// dock can import them without chatUtils' runtime chain; re-exported here for
// the popover / workspace card that already pull chatUtils at runtime.
export {
  type ContextUsageLevel,
  contextUsageLevel,
  contextUsageBarClass,
} from "./contextUsageColor"

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
    if (isYesterday) {
      return String(i18n.t("chat.messageTimeYesterday", { time, defaultValue: `Yesterday ${time}` }))
    }
    const month = date.getMonth() + 1
    const day = date.getDate()
    if (date.getFullYear() === now.getFullYear()) return `${month}/${day} ${time}`
    return `${date.getFullYear()}/${month}/${day} ${time}`
  } catch {
    return ""
  }
}

/**
 * Format a duration (ms) with second / minute / hour rollover — the single
 * source of truth for all elapsed-time displays (tool steps, thinking blocks,
 * processed-group totals, message total). Sub-minute shows one decimal second
 * (`0.1s`, `5.3s`); from a minute up, whole units (`1m 30s`, `2h 5m`).
 */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, ms) / 1000
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`
  const totalMinutes = Math.floor(totalSeconds / 60)
  if (totalMinutes < 60) {
    const seconds = Math.round(totalSeconds % 60)
    return `${totalMinutes}m ${seconds}s`
  }
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return `${hours}h ${minutes}m`
}

export type MessageFileAttachment =
  | { kind: "path"; path: string }
  | { kind: "media"; item: MediaItem }

function inferAttachmentKind(mimeType: string): MessageAttachment["kind"] {
  return mimeType.toLowerCase().startsWith("image/") ? "image" : "file"
}

function stringField(obj: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = obj[key]
    if (typeof value === "string" && value.trim()) return value
  }
  return undefined
}

function numberField(obj: Record<string, unknown>, ...keys: string[]): number {
  for (const key of keys) {
    const value = obj[key]
    if (typeof value === "number" && Number.isFinite(value)) return value
  }
  return 0
}

export function parseUserAttachmentsMeta(
  metaJson: string | null | undefined,
): MessageAttachment[] | undefined {
  if (!metaJson) return undefined
  try {
    const parsed = JSON.parse(metaJson)
    const rawItems = Array.isArray(parsed)
      ? parsed
      : parsed && typeof parsed === "object"
        ? (parsed.user_attachments ?? parsed.attachments)
        : null
    if (!Array.isArray(rawItems) || rawItems.length === 0) return undefined

    const attachments: MessageAttachment[] = []
    for (const item of rawItems) {
      if (!item || typeof item !== "object" || Array.isArray(item)) continue
      const obj = item as Record<string, unknown>
      // File-browser quote reference card.
      if (obj.kind === "quote") {
        const qname = stringField(obj, "name")
        if (!qname) continue
        attachments.push({
          name: qname,
          mimeType: "text/plain",
          sizeBytes: 0,
          kind: "quote",
          quotePath: stringField(obj, "path"),
          quoteLines: stringField(obj, "lines"),
          quoteContent: stringField(obj, "content"),
        })
        continue
      }
      const name = stringField(obj, "name")
      const mimeType = stringField(obj, "mime_type", "mimeType") ?? "application/octet-stream"
      const localPath = stringField(obj, "path", "localPath")
      const url = stringField(obj, "url")
      if (!name || (!localPath && !url)) continue
      attachments.push({
        name,
        mimeType,
        sizeBytes: numberField(obj, "size", "sizeBytes"),
        kind: inferAttachmentKind(mimeType),
        ...(localPath ? { localPath } : {}),
        ...(url ? { url } : {}),
      })
    }

    return attachments.length > 0 ? attachments : undefined
  } catch {
    return undefined
  }
}

/** Extract files produced by tool calls for the assistant message footer. */
export function extractMessageFileAttachments(blocks: ContentBlock[]): MessageFileAttachment[] {
  const pathItems = new Map<string, MessageFileAttachment>()
  const mediaItems = new Map<string, MessageFileAttachment>()
  const mediaLocalPaths = new Set<string>()

  const addPath = (path: string | null | undefined) => {
    const trimmed = path?.trim()
    if (!trimmed || mediaLocalPaths.has(trimmed)) return
    if (!pathItems.has(trimmed)) pathItems.set(trimmed, { kind: "path", path: trimmed })
  }

  const addMedia = (item: MediaItem) => {
    const key = item.localPath || item.url || item.name
    if (!key || mediaItems.has(key)) return
    if (item.localPath) {
      mediaLocalPaths.add(item.localPath)
      pathItems.delete(item.localPath)
    }
    mediaItems.set(key, { kind: "media", item })
  }

  for (const block of blocks) {
    if (block.type !== "tool_call") continue
    const { name, arguments: args, result } = block.tool
    const metadata = block.tool.metadata
    block.tool.mediaItems?.forEach(addMedia)
    block.tool.mediaUrls?.forEach(addPath)

    if (metadata?.kind === "file_change") {
      if (metadata.action !== "delete") addPath(metadata.path)
    } else if (metadata?.kind === "file_changes") {
      for (const change of metadata.changes) {
        if (change.action !== "delete") addPath(change.path)
      }
    }

    if (!result) continue

    if (
      (name === "write" || name === "write_file") &&
      result.startsWith("Successfully wrote")
    ) {
      try {
        const parsed = JSON.parse(args)
        const p = parsed.path || parsed.file_path
        addPath(p)
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
        addPath(p)
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
          addPath(filePath)
        }
      }
    }
  }
  return [...pathItems.values(), ...mediaItems.values()]
}

/** Extract file paths modified by tool calls (write/edit/apply_patch). */
export function extractModifiedFiles(blocks: ContentBlock[]): string[] {
  return extractMessageFileAttachments(blocks)
    .filter((item): item is { kind: "path"; path: string } => item.kind === "path")
    .map((item) => item.path)
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
  const seenPlainEventContentSinceLastUser = new Set<string>()
  for (const msg of msgs) {
    if (msg.role === "user") {
      seenPlainEventContentSinceLastUser.clear()
      // Detect sub-agent result / cron trigger / plan trigger messages via attachments_meta marker
      let isSubagentResult = false
      let subagentResultAgentId: string | undefined
      let isCronTrigger = false
      let cronJobName: string | undefined
      let isWakeupTrigger = false
      let isLoopTrigger = false
      let isProcessNotification = false
      let isWorkflowResult = false
      let isPlanTrigger = false
      let isGoalTrigger = false
      let planComment: { selectedText: string; comment: string } | undefined
      let channelInbound:
        | { channelId: string; accountId?: string; chatId?: string; senderName?: string }
        | undefined
      const attachments = parseUserAttachmentsMeta(msg.attachmentsMeta)
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
          if (meta?.wakeup_trigger) {
            isWakeupTrigger = true
          }
          if (meta?.loop_trigger) {
            isLoopTrigger = true
          }
          if (meta?.process_notification) {
            isProcessNotification = true
          }
          if (meta?.workflow_result) {
            isWorkflowResult = true
          }
          if (meta?.plan_trigger) {
            isPlanTrigger = true
          }
          if (meta?.goal_trigger) {
            isGoalTrigger = true
          }
          if (
            meta?.plan_comment &&
            typeof meta.plan_comment.selectedText === "string" &&
            typeof meta.plan_comment.comment === "string"
          ) {
            planComment = {
              selectedText: meta.plan_comment.selectedText,
              comment: meta.plan_comment.comment,
            }
          }
          if (meta?.channel_inbound && typeof meta.channel_inbound.channelId === "string") {
            channelInbound = {
              channelId: meta.channel_inbound.channelId,
              accountId:
                typeof meta.channel_inbound.accountId === "string"
                  ? meta.channel_inbound.accountId
                  : undefined,
              chatId:
                typeof meta.channel_inbound.chatId === "string"
                  ? meta.channel_inbound.chatId
                  : undefined,
              senderName:
                typeof meta.channel_inbound.senderName === "string"
                  ? meta.channel_inbound.senderName
                  : undefined,
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
        isWakeupTrigger,
        isLoopTrigger,
        isProcessNotification,
        isWorkflowResult,
        isPlanTrigger,
        isGoalTrigger,
        planComment,
        channelInbound,
        ...(attachments ? { attachments } : {}),
      })
    } else if (msg.role === "tool" && msg.toolCallId) {
      // Extract media info from tool results (for DB-loaded history):
      //   - image_generate still uses the old "Saved to:" text lines (mediaUrls)
      //   - send_attachment and future tools emit a `__MEDIA_ITEMS__<json>` header
      let mediaUrls: string[] | undefined
      let mediaItems: MediaItem[] | undefined = parseToolMediaItemsMeta(msg.attachmentsMeta)
      if (msg.toolResult) {
        if (!mediaItems) mediaItems = parseMediaItemsHeader(msg.toolResult)
        if (msg.toolName === "image_generate" && !mediaItems) {
          const paths = msg.toolResult
            .split("\n")
            .filter((l) => l.startsWith("Saved to: "))
            .map((l) => l.slice("Saved to: ".length).trim())
            .filter(Boolean)
          if (paths.length > 0) mediaUrls = paths
        }
      }
      let toolMetadata: ToolCall["metadata"]
      if (msg.toolMetadata) {
        try {
          toolMetadata = JSON.parse(msg.toolMetadata) as ToolCall["metadata"]
        } catch {
          toolMetadata = undefined
        }
      }
      const tool: ToolCall = {
        callId: msg.toolCallId,
        name: msg.toolName || "",
        arguments: msg.toolArguments || "",
        result: msg.toolResult || undefined,
        isError: msg.isError != null ? msg.isError : hasToolError({
          result: msg.toolResult || undefined,
        }),
        mediaUrls,
        mediaItems,
        durationMs: msg.toolDurationMs || undefined,
        ...(toolMetadata ? { metadata: toolMetadata } : {}),
      }
      // Check if already exists in pendingTools (merge result)
      const existing = pendingTools.find((c) => c.callId === msg.toolCallId)
      if (existing) {
        if (msg.toolResult) existing.result = msg.toolResult
        if (msg.isError != null || msg.toolResult != null) {
          existing.isError = msg.isError != null
            ? msg.isError
            : hasToolError({ result: msg.toolResult || undefined })
        }
        if (msg.toolName && !existing.name) existing.name = msg.toolName
        if (msg.toolArguments && !existing.arguments) existing.arguments = msg.toolArguments
        if (msg.toolDurationMs != null) existing.durationMs = msg.toolDurationMs
        if (toolMetadata) existing.metadata = toolMetadata
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
        const interrupted = isInterruptedStreamStatus(msg.streamStatus)
        pendingBlocks.push({
          type: "thinking",
          content: msg.content,
          durationMs: msg.toolDurationMs || undefined,
          interrupted: interrupted || undefined,
        })
      }
    } else if (msg.role === "text_block") {
      // Intermediate text emitted before tool calls — preserve ordering
      if (msg.content) {
        const interrupted = isInterruptedStreamStatus(msg.streamStatus)
        pendingBlocks.push({ type: "text", content: msg.content, interrupted: interrupted || undefined })
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
        msg.toolDurationMs ||
        msg.tokensIn ||
        msg.tokensOut ||
        msg.tokensInLast ||
        msg.tokensCacheCreation != null ||
        msg.tokensCacheRead != null
      const usage: MessageUsage | undefined = hasUsage
        ? {
            durationMs: msg.toolDurationMs || undefined,
            inputTokens: msg.tokensIn || undefined,
            outputTokens: msg.tokensOut || undefined,
            lastInputTokens: msg.tokensInLast || undefined,
            cacheCreationInputTokens: msg.tokensCacheCreation ?? undefined,
            cacheReadInputTokens: msg.tokensCacheRead ?? undefined,
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
      let slashEvent: Message["slashEvent"] | undefined
      if (msg.attachmentsMeta) {
        try {
          const meta = JSON.parse(msg.attachmentsMeta)
          const slash = meta?.slash_command
          if (slash?.kind === "command" || slash?.kind === "result") {
            slashEvent = {
              kind: slash.kind,
              command: typeof slash.command === "string" ? slash.command : undefined,
              displayAs: slash.displayAs === "user" ? "user" : undefined,
              mode: slash.mode === "goal" || slash.mode === "loop" ? slash.mode : undefined,
            }
          }
        } catch {
          /* ignore */
        }
      }
      if (!slashEvent && isStartupRecoveryNotice(msg.content)) {
        if (seenPlainEventContentSinceLastUser.has(msg.content)) {
          continue
        }
        seenPlainEventContentSinceLastUser.add(msg.content)
      }
      const eventMessage: Message = {
        role: "event",
        content: msg.content,
        timestamp: msg.timestamp,
        slashEvent,
        dbId: msg.id,
      }
      if (upsertContextCompactionEventMessage(displayMessages, eventMessage)) {
        continue
      }
      displayMessages.push(eventMessage)
    }
  }
  // Mid-stream load: if the loop ended with accumulated tool calls / interim
  // blocks that were never claimed by a final assistant row, surface them as
  // a synthetic in-progress assistant so the UI renders what has happened so
  // far and subsequent text/tool deltas have a message to attach to.
  if (pendingTools.length > 0 || pendingBlocks.length > 0) {
    displayMessages.push({
      role: "assistant",
      content: "",
      contentBlocks: pendingBlocks.length > 0 ? [...pendingBlocks] : undefined,
      toolCalls: pendingTools.length > 0 ? [...pendingTools] : undefined,
      timestamp: new Date().toISOString(),
    })
  }
  return displayMessages
}

/**
 * Reconcile a freshly-loaded DB window (`fresh`) with the current in-memory
 * window (`existing`) without truncating paged-in scrollback. Used after
 * `chat:stream_end` / `channel:stream_end` / subagent-done reloads.
 *
 * Trailing dbId-less items in `existing` are streaming placeholders whose
 * persisted counterparts are about to land in `fresh`; keeping them would
 * duplicate-render. dbId-less items mid-stream (rare) are left in place.
 */

/**
 * Resolve the parent session's agentId for a sub-agent session — needed by
 * `parseSessionMessages` so child rows can be tagged with the right "from"
 * agent. Tries the in-memory sessions cache first; falls back to a single-
 * row `get_session_cmd` lookup for sessions that aren't in the current
 * paginated window (typical when jumping in from search). Replaces the
 * legacy `list_sessions_cmd({})` full-table scan that used to run on every
 * load-more / switch / reset path.
 */
export async function resolveParentAgentId(
  sessionId: string,
  sessionsRef: React.MutableRefObject<SessionMeta[]>,
): Promise<string | undefined> {
  const lookup = (sid: string) => sessionsRef.current.find((s) => s.id === sid)
  let session = lookup(sessionId)
  if (!session) {
    session =
      (await getTransport()
        .call<SessionMeta | null>("get_session_cmd", { sessionId })
        .catch(() => null)) ?? undefined
  }
  const parentSid = session?.parentSessionId
  if (!parentSid) return undefined
  const parent =
    lookup(parentSid) ??
    (await getTransport()
      .call<SessionMeta | null>("get_session_cmd", { sessionId: parentSid })
      .catch(() => null)) ??
    undefined
  return parent?.agentId
}

/**
 * Sort search results into the order users expect from arrow-key navigation:
 * title matches first, then message matches oldest-first by ISO timestamp, with
 * `messageId` breaking ties. FTS5's native rank order is opaque to humans
 * skimming history.
 */
export function sortSessionSearchResults(
  results: SessionSearchResult[],
): SessionSearchResult[] {
  return results.slice().sort((a, b) => {
    const kindCmp = (a.matchKind === "title" ? 0 : 1) - (b.matchKind === "title" ? 0 : 1)
    if (kindCmp !== 0) return kindCmp
    const cmp = a.timestamp.localeCompare(b.timestamp)
    return cmp !== 0 ? cmp : a.messageId - b.messageId
  })
}

/** Convenience: resolve parent agentId then `parseSessionMessages`. */
export async function materializeMessages(
  sessionId: string,
  msgs: SessionMessage[],
  sessionsRef: React.MutableRefObject<SessionMeta[]>,
): Promise<Message[]> {
  const parentAgentId = await resolveParentAgentId(sessionId, sessionsRef)
  return parseSessionMessages(msgs, parentAgentId)
}

/**
 * Reload the latest DB window for a session and merge it with the current
 * in-memory window via `mergeMessagesByDbId`, then push the merged result to
 * both the per-session cache and the active-view state. Shared by every
 * "stream ended, reconcile with DB" call site.
 *
 * The reload `limit` floors at `PAGE_SIZE` but grows to whatever scrollback
 * the user has already paged in, so we don't silently truncate a long
 * window that outgrew the default page size.
 */
export async function reloadAndMergeSessionMessages(params: {
  sessionId: string
  pageSize: number
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  setMessages: (msgs: Message[]) => void
}): Promise<void> {
  const { sessionId, pageSize, sessionCacheRef, setMessages } = params
  const existingAtRequestStart = sessionCacheRef.current.get(sessionId) ?? []
  const limit = Math.max(pageSize, existingAtRequestStart.length)
  try {
    const [msgs] = await getTransport().call<[SessionMessage[], number, boolean]>(
      "load_session_messages_latest_cmd",
      { sessionId, limit },
    )
    const fresh = parseSessionMessages(msgs)
    const existing = sessionCacheRef.current.get(sessionId) ?? existingAtRequestStart
    const merged = preserveMessagesAppendedDuringReload(
      mergeMessagesByDbId(existing, fresh),
      existingAtRequestStart,
      existing,
    )
    sessionCacheRef.current.set(sessionId, merged)
    setMessages(merged)
  } catch {
    // Stream has already ended and placeholders will eventually resolve via
    // the next session switch — swallowing here matches the pre-refactor
    // behavior on each of the three call sites.
  }
}

function hasStableMessageIdentity(msg: Message): boolean {
  return typeof msg.dbId === "number" || !!msg._clientId
}

function stableMessageIdentityMatches(a: Message, b: Message): boolean {
  if (
    typeof a.dbId === "number" &&
    typeof b.dbId === "number" &&
    a.dbId === b.dbId
  ) {
    return true
  }
  return !!a._clientId && !!b._clientId && a._clientId === b._clientId
}

function sameTransientMessage(a: Message, b: Message): boolean {
  if (a === b) return true
  if (stableMessageIdentityMatches(a, b)) return true
  if (hasStableMessageIdentity(a) || hasStableMessageIdentity(b)) return false
  return a.role === b.role && a.timestamp === b.timestamp && a.content === b.content
}

function messagesAppendedAfterSnapshot(
  snapshot: Message[],
  latest: Message[],
): Message[] {
  if (latest.length <= snapshot.length) return []
  for (let i = 0; i < snapshot.length; i++) {
    if (!sameTransientMessage(snapshot[i], latest[i])) return []
  }
  return latest.slice(snapshot.length)
}

function preserveMessagesAppendedDuringReload(
  merged: Message[],
  snapshotAtRequestStart: Message[],
  latestExisting: Message[],
): Message[] {
  const appended = messagesAppendedAfterSnapshot(snapshotAtRequestStart, latestExisting)
  if (appended.length === 0) return merged

  const missing = appended.filter(
    (msg) =>
      !merged.some(
        (mergedMsg) =>
          mergedMsg === msg || stableMessageIdentityMatches(mergedMsg, msg),
      ),
  )
  return missing.length > 0 ? [...merged, ...missing] : merged
}

// Compare two Message snapshots for the purpose of preserving the existing
// reference across a DB reload. Returns true when the rendered output of the
// MessageBubble is identical for both, in which case `mergeMessagesByDbId`
// keeps the existing object so React.memo skips re-rendering the
// markdown/shiki/katex subtree.
//
// Why field-level instead of `JSON.stringify`: stringifying the whole message
// runs O(content_size) per pair × all pairs at stream_end. For long histories
// that's MBs of throwaway strings on the main thread once per turn. Two DB
// rows with the same `dbId` agree on the deep `contentBlocks` / `toolCalls`
// structure once their primitive fields and array lengths match, since the
// only field expected to mutate underneath us is the active streaming
// assistant turn — and *that* turn always changes either `content` length or
// `contentBlocks.length`. The cheap check below is exhaustive enough.
function messageContentEqual(a: Message, b: Message): boolean {
  if (a === b) return true
  return (
    a.dbId === b.dbId &&
    a.role === b.role &&
    a.content === b.content &&
    a.slashEvent?.kind === b.slashEvent?.kind &&
    a.slashEvent?.displayAs === b.slashEvent?.displayAs &&
    a.slashEvent?.command === b.slashEvent?.command &&
    a.slashEvent?.mode === b.slashEvent?.mode &&
    a.isGoalTrigger === b.isGoalTrigger &&
    a.isLoopTrigger === b.isLoopTrigger &&
    a.thinking === b.thinking &&
    a.timestamp === b.timestamp &&
    a.model === b.model &&
    messageAttachmentsEqual(a.attachments, b.attachments) &&
    (a.contentBlocks?.length ?? 0) === (b.contentBlocks?.length ?? 0) &&
    (a.toolCalls?.length ?? 0) === (b.toolCalls?.length ?? 0)
  )
}

function messageAttachmentsEqual(
  a: MessageAttachment[] | undefined,
  b: MessageAttachment[] | undefined,
): boolean {
  const aItems = a ?? []
  const bItems = b ?? []
  if (aItems.length !== bItems.length) return false
  return aItems.every((item, index) => {
    const other = bItems[index]
    return (
      item.name === other.name &&
      item.mimeType === other.mimeType &&
      item.sizeBytes === other.sizeBytes &&
      item.kind === other.kind &&
      item.localPath === other.localPath &&
      item.url === other.url &&
      item.previewUrl === other.previewUrl
    )
  })
}

function transferPlaceholderState(fresh: Message, placeholder: Message): Message {
  return {
    ...fresh,
    _clientId: placeholder._clientId,
    ...(!fresh.attachments?.length && placeholder.attachments?.length
      ? { attachments: placeholder.attachments }
      : {}),
  }
}

export function mergeMessagesByDbId(existing: Message[], fresh: Message[]): Message[] {
  if (existing.length === 0) return fresh

  let tailEnd = existing.length
  while (tailEnd > 0 && typeof existing[tailEnd - 1].dbId !== "number") {
    tailEnd--
  }
  // Each trailing dbId-less placeholder carries a `_clientId` we transfer to
  // its DB-finalized successor in `fresh` so React row keys stay stable
  // across the placeholder→DB transition. Both the user turn and the
  // assistant placeholder get one; transferring per-role keeps
  // `getMessageRowKey` and `getLatestUserTurnKey` from flipping at
  // stream_end (the latter would otherwise mis-fire forceFollow's
  // scroll-into-view, snapping the viewport back to the user bubble).
  const trailingPlaceholders = existing.slice(tailEnd)
  const trimmed = tailEnd < existing.length ? existing.slice(0, tailEnd) : existing

  if (fresh.length === 0) return trimmed

  const freshById = new Map<number, Message>()
  for (const m of fresh) {
    if (typeof m.dbId === "number") freshById.set(m.dbId, m)
  }

  const seenIds = new Set<number>()
  const merged: Message[] = []
  for (const m of trimmed) {
    if (typeof m.dbId !== "number") {
      merged.push(m)
      continue
    }
    const authoritative = freshById.get(m.dbId)
    if (authoritative === undefined) {
      merged.push(m)
    } else if (messageContentEqual(authoritative, m)) {
      // Content identical — keep existing reference so memoized children
      // (MessageBubble) skip re-renders. Only the genuinely-changed message
      // (typically the last assistant which got finalized contentBlocks /
      // usage from the server) takes the new reference and re-renders.
      merged.push(m)
    } else {
      merged.push(authoritative)
    }
    seenIds.add(m.dbId)
  }

  // Append fresh messages that didn't exist in `existing` and transfer each
  // trimmed placeholder's `_clientId` to the first newly-appended row of the
  // matching role. Order-preserving (placeholders are processed in original
  // order, each consumes the first unmatched fresh row of its role) so a
  // typical [user, assistant] pair maps to [user', assistant'] correctly.
  // If no fresh row of the placeholder's role lands, the id drops — safer
  // than attaching to a wrong-role row (would break the memo invariant
  // `_clientId` is meant to uphold).
  const newFresh: Message[] = []
  for (const m of fresh) {
    if (typeof m.dbId === "number" && !seenIds.has(m.dbId)) {
      newFresh.push(m)
    }
  }
  const claimed = new Set<number>()
  for (const placeholder of trailingPlaceholders) {
    if (!placeholder._clientId) continue
    for (let i = 0; i < newFresh.length; i++) {
      if (claimed.has(i)) continue
      if (newFresh[i].role !== placeholder.role) continue
      newFresh[i] = transferPlaceholderState(newFresh[i], placeholder)
      claimed.add(i)
      break
    }
  }
  // Fast-path: nothing actually changed. Returning `existing` lets
  // `setMessages(existing)` callers hit React's same-reference bail-out
  // (the cache-hit background reload-and-merge path takes this every
  // time the user toggles back to a session whose DB hasn't moved).
  if (
    newFresh.length === 0 &&
    merged.length === existing.length &&
    merged.every((m, i) => m === existing[i])
  ) {
    return existing
  }
  merged.push(...newFresh)

  return merged
}

/**
 * Bound a session's `messages` array to a runaway-protection ceiling.
 * Effective cap is dynamic: `MAX_MESSAGES + paginated`, where `paginated` is
 * the user's accumulated load-more depth — anything actively pulled in stays
 * headroom rather than being immediately reclaimed.
 *
 * On overflow, retains the tail (`KEEP_AFTER_CAP + paginated`) and syncs
 * `oldestDbIdRef` to the new head + flips `hasMoreRef` true so the dropped
 * prefix is recoverable via the existing load-more path.
 */
export function capMessagesAndSyncCursors(
  sessionId: string,
  msgs: Message[],
  paginated: number,
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>,
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>,
): Message[] {
  const effectiveCap = MAX_MESSAGES + paginated
  if (msgs.length <= effectiveCap) return msgs
  const keepLen = KEEP_AFTER_CAP + paginated
  const kept = msgs.slice(msgs.length - keepLen)
  const head = kept[0]
  if (head && typeof head.dbId === "number") {
    oldestDbIdRef.current.set(sessionId, head.dbId)
  }
  hasMoreRef.current.set(sessionId, true)
  return kept
}
