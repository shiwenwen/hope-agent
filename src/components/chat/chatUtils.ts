import type * as React from "react"
import type {
  Message,
  ContentBlock,
  MediaItem,
  ToolCall,
  SessionMessage,
  SessionMeta,
  SessionSearchResult,
  MessageUsage,
} from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import { hasToolError } from "./message/executionStatus"

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

/** True when a message should render as a centered system chip (event,
 *  sub-agent result, cron trigger, plan-mode approve/resume) rather than as
 *  a user/assistant bubble. */
export function isCenteredSystemMessage(msg: Message): boolean {
  return (
    msg.role === "event" ||
    !!msg.isSubagentResult ||
    !!msg.isCronTrigger ||
    !!msg.isPlanTrigger
  )
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
    const metadata = block.tool.metadata

    if (metadata?.kind === "file_change") {
      if (metadata.action !== "delete") files.add(metadata.path)
    } else if (metadata?.kind === "file_changes") {
      for (const change of metadata.changes) {
        if (change.action !== "delete") files.add(change.path)
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
      // Detect sub-agent result / cron trigger / plan trigger messages via attachments_meta marker
      let isSubagentResult = false
      let subagentResultAgentId: string | undefined
      let isCronTrigger = false
      let cronJobName: string | undefined
      let isPlanTrigger = false
      let planComment: { selectedText: string; comment: string } | undefined
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
          if (meta?.plan_trigger) {
            isPlanTrigger = true
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
        isPlanTrigger,
        planComment,
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
        const interrupted = msg.streamStatus === "orphaned" || msg.streamStatus === "streaming"
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
        const interrupted = msg.streamStatus === "orphaned" || msg.streamStatus === "streaming"
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
      displayMessages.push({
        role: "event",
        content: msg.content,
        timestamp: msg.timestamp,
        dbId: msg.id,
      })
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
 * Sort FTS5 search results into the order users expect from arrow-key
 * navigation: oldest first by ISO timestamp, with `messageId` breaking
 * ties for messages that share a timestamp string. FTS5's native rank
 * order is opaque to humans skimming history.
 */
export function sortSessionSearchResults(
  results: SessionSearchResult[],
): SessionSearchResult[] {
  return results.slice().sort((a, b) => {
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
  const existing = sessionCacheRef.current.get(sessionId) ?? []
  const limit = Math.max(pageSize, existing.length)
  try {
    const [msgs] = await getTransport().call<[SessionMessage[], number, boolean]>(
      "load_session_messages_latest_cmd",
      { sessionId, limit },
    )
    const fresh = parseSessionMessages(msgs)
    const merged = mergeMessagesByDbId(existing, fresh)
    sessionCacheRef.current.set(sessionId, merged)
    setMessages(merged)
  } catch {
    // Stream has already ended and placeholders will eventually resolve via
    // the next session switch — swallowing here matches the pre-refactor
    // behavior on each of the three call sites.
  }
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
    a.thinking === b.thinking &&
    a.timestamp === b.timestamp &&
    a.model === b.model &&
    (a.contentBlocks?.length ?? 0) === (b.contentBlocks?.length ?? 0) &&
    (a.toolCalls?.length ?? 0) === (b.toolCalls?.length ?? 0)
  )
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
      newFresh[i] = { ...newFresh[i], _clientId: placeholder._clientId }
      claimed.add(i)
      break
    }
  }
  merged.push(...newFresh)

  return merged
}
