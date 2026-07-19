import type React from "react"
import type { ContentBlock, FallbackEvent, MediaItem, Message, ToolMetadata } from "@/types/chat"
import {
  contextCompactionData,
  isContextCompactionPayload,
  isContextCompactionStartPayload,
  parseEventPayload,
  shouldReplaceContextCompactionNotice,
} from "../contextCompactionEvents"
import { mergeUsageFromEvent, parseUserAttachmentsMeta } from "../chatUtils"
import { hasToolError } from "../message/executionStatus"

/** Extract a structured tool_metadata payload from a stream event when present. */
function extractToolMetadata(event: Record<string, unknown>): ToolMetadata | undefined {
  const raw = event.tool_metadata
  if (!raw || typeof raw !== "object") return undefined
  return raw as ToolMetadata
}

export interface StreamEventHandlerDeps {
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>
  setShowCodexAuthExpired?: React.Dispatch<React.SetStateAction<boolean>>
}

export interface StreamDeltaBuffers {
  pending: Map<string, { text: string; thinking: string }>
  rafs: Map<string, number>
  lastFlushAt: Map<string, number>
}

const LEGACY_STREAM_ID = "__legacy__"

// 流式 flush 节流：把每帧（≈60fps）flush 降到 ~30fps。每次 flush 都触发末块
// markdown re-lex + Shiki 重高亮 + React reconcile，长内容下这是主成本之一。
// char fadeIn 动画在视觉上补帧，掉到 30fps 几乎无感，CPU 直接减半。尾部因
// stream_end 前由 reattach handler 强制 flush RAF 缓冲；committed 终态仍会再用
// `mergeMessagesByDbId` 对账，pending 终态也不会丢掉已经耐久投递的尾帧。
const FLUSH_MIN_INTERVAL_MS = 33

export function streamCursorKey(sessionId: string, streamId?: string | null): string {
  return `${sessionId}\u0000${streamId || LEGACY_STREAM_ID}`
}

export function streamIdFromEvent(event: Record<string, unknown>): string | undefined {
  const value = event._oc_stream_id
  return typeof value === "string" && value ? value : undefined
}

export function streamIdFromPayload(raw: unknown): string | undefined {
  const value = (raw as { streamId?: unknown } | null)?.streamId
  return typeof value === "string" && value ? value : undefined
}

export function createStreamDeltaBuffers(): StreamDeltaBuffers {
  return {
    pending: new Map(),
    rafs: new Map(),
    lastFlushAt: new Map(),
  }
}

export function discardPendingStreamDeltas(
  sid: string,
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>,
): void {
  const state = deltaBuffersRef.current
  const raf = state.rafs.get(sid)
  if (raf !== undefined) {
    cancelAnimationFrame(raf)
  }
  state.rafs.delete(sid)
  state.pending.delete(sid)
  state.lastFlushAt.delete(sid)
}

export function discardAllPendingStreamDeltas(
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>,
): void {
  const state = deltaBuffersRef.current
  for (const raf of state.rafs.values()) {
    cancelAnimationFrame(raf)
  }
  state.rafs.clear()
  state.pending.clear()
  state.lastFlushAt.clear()
}

function pendingDeltasFor(
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>,
  sid: string,
): { text: string; thinking: string } {
  const state = deltaBuffersRef.current
  let pending = state.pending.get(sid)
  if (!pending) {
    pending = { text: "", thinking: "" }
    state.pending.set(sid, pending)
  }
  return pending
}

function takePendingStreamDeltas(
  sid: string,
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>,
  cancelScheduled: boolean,
): { text: string; thinking: string } | null {
  const state = deltaBuffersRef.current
  if (cancelScheduled) {
    const raf = state.rafs.get(sid)
    if (raf !== undefined) {
      cancelAnimationFrame(raf)
    }
  }
  state.rafs.delete(sid)
  const pending = state.pending.get(sid)
  state.pending.delete(sid)
  if (!pending || (!pending.text && !pending.thinking)) return null
  return pending
}

export function flushPendingStreamDeltas(
  sid: string,
  deps: StreamEventHandlerDeps,
  cancelScheduled: boolean,
): void {
  const pending = takePendingStreamDeltas(sid, deps.deltaBuffersRef, cancelScheduled)
  if (!pending) return
  deps.deltaBuffersRef.current.lastFlushAt.set(sid, Date.now())

  const textChunk = pending.text
  const thinkingChunk = pending.thinking
  deps.updateSessionMessages(sid, (prev) => {
    const last = prev[prev.length - 1]
    if (!last || last.role !== "assistant") return prev
    const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
    if (thinkingChunk) {
      const lastBlock = blocks[blocks.length - 1]
      if (lastBlock && lastBlock.type === "thinking") {
        blocks[blocks.length - 1] = {
          type: "thinking",
          content: lastBlock.content + thinkingChunk,
        }
      } else {
        blocks.push({ type: "thinking", content: thinkingChunk })
      }
    }
    if (textChunk) {
      const lastBlock = blocks[blocks.length - 1]
      if (lastBlock && lastBlock.type === "text") {
        blocks[blocks.length - 1] = {
          type: "text",
          content: lastBlock.content + textChunk,
        }
      } else {
        blocks.push({ type: "text", content: textChunk })
      }
    }
    const updated = prev.slice()
    updated[updated.length - 1] = {
      ...last,
      contentBlocks: blocks,
      ...(textChunk ? { content: last.content + textChunk } : {}),
      ...(thinkingChunk ? { thinking: (last.thinking || "") + thinkingChunk } : {}),
    }
    return updated
  })
}

function pickString(event: Record<string, unknown>, key: string): string | undefined {
  const v = event[key]
  return typeof v === "string" ? v : undefined
}

function pickNumber(event: Record<string, unknown>, key: string): number | undefined {
  const v = event[key]
  if (typeof v === "number" && Number.isFinite(v)) return v
  if (typeof v === "string") {
    const parsed = Number(v)
    if (Number.isFinite(parsed)) return parsed
  }
  return undefined
}

function fallbackEventFromStreamEvent(event: Record<string, unknown>): FallbackEvent {
  const model =
    pickString(event, "model") ??
    pickString(event, "model_id") ??
    pickString(event, "to_model") ??
    ""
  return {
    type: pickString(event, "type"),
    model,
    from_model: pickString(event, "from_model"),
    reason: pickString(event, "reason"),
    error: pickString(event, "error"),
    attempt: pickNumber(event, "attempt"),
    total: pickNumber(event, "total"),
    provider_id: pickString(event, "provider_id"),
    model_id: pickString(event, "model_id"),
  }
}

function schedulePendingStreamFlush(sid: string, deps: StreamEventHandlerDeps): void {
  const state = deps.deltaBuffersRef.current
  if (state.rafs.has(sid)) return
  const raf = requestAnimationFrame(() => {
    const s = deps.deltaBuffersRef.current
    s.rafs.delete(sid)
    // Throttle to ~30fps: if the previous flush was < FLUSH_MIN_INTERVAL_MS ago,
    // wait one more frame instead of flushing now. Pending deltas keep
    // accumulating in the buffer, so nothing is lost — just coalesced.
    const last = s.lastFlushAt.get(sid) ?? 0
    if (Date.now() - last < FLUSH_MIN_INTERVAL_MS) {
      schedulePendingStreamFlush(sid, deps)
      return
    }
    flushPendingStreamDeltas(sid, deps, false)
  })
  state.rafs.set(sid, raf)
}

function stringField(event: Record<string, unknown>, key: string): string {
  const value = event[key]
  return typeof value === "string" ? value : ""
}

function eventPayloadFromMessage(message: Message): Record<string, unknown> | null {
  if (message.role !== "event") return null
  return parseEventPayload(message.content)
}

function isContextCompactionStartDescription(description: unknown): boolean {
  return description === "summarizing" || description === "emergency_compacting"
}

function isContextCompactionProgress(event: Record<string, unknown>): boolean {
  return event.type === "context_compaction_progress"
}

function isContextCompactionDoneProgress(event: Record<string, unknown>): boolean {
  if (!isContextCompactionProgress(event)) return false
  return contextCompactionData(event).phase === "done"
}

function isContextCompactionLiveNotice(message: Message): boolean {
  const payload = eventPayloadFromMessage(message)
  if (!payload) return false
  if (payload.type === "context_compaction_progress") return true
  if (payload.type !== "context_compacted") return false
  return isContextCompactionStartDescription(contextCompactionData(payload).description)
}

function isContextCompactionNotice(message: Message): boolean {
  return isContextCompactionPayload(eventPayloadFromMessage(message))
}

function contextCompactionStartNoticeIndex(messages: Message[]): number {
  let idx = messages.length - 1
  if (messages[idx]?.role === "assistant") idx -= 1
  if (idx >= 0 && isContextCompactionLiveNotice(messages[idx])) return idx
  return -1
}

function contextCompactionNoticeIndex(messages: Message[]): number {
  let idx = messages.length - 1
  if (messages[idx]?.role === "assistant") idx -= 1
  if (idx >= 0 && isContextCompactionNotice(messages[idx])) return idx
  return -1
}

function mergeWithContextCompactionStart(
  previous: Message,
  event: Record<string, unknown>,
): string {
  const previousPayload = eventPayloadFromMessage(previous)
  const previousData = previousPayload ? contextCompactionData(previousPayload) : {}
  const nextData = contextCompactionData(event)
  const shouldCarrySummarizedCount =
    nextData.messages_to_summarize == null && previousData.messages_to_summarize != null
  const mergedData = {
    ...nextData,
    ...(shouldCarrySummarizedCount
      ? { messages_to_summarize: previousData.messages_to_summarize }
      : {}),
  }
  return JSON.stringify({ ...event, data: mergedData })
}

/**
 * Processes a single parsed stream event (text_delta, thinking_delta, tool_call, tool_result, usage, etc.)
 * and updates the message list accordingly.
 *
 * Returns `true` if the event was fully handled (caller should skip further processing).
 */
export function handleStreamEvent(
  event: Record<string, unknown>,
  sid: string,
  deps: StreamEventHandlerDeps,
): boolean {
  const { updateSessionMessages, deltaBuffersRef, setShowCodexAuthExpired } = deps

  // A durable failover can happen after the failed attempt already rendered.
  // The backend journals this marker as the first event of the replacement
  // attempt, so clearing here is ordered and replay-safe. Never flush the old
  // rAF buffer: those bytes belong to the superseded attempt.
  if (event.type === "stream_attempt_started" && event.reset_superseded === true) {
    discardPendingStreamDeltas(sid, deltaBuffersRef)
    updateSessionMessages(sid, (prev) => {
      const last = prev[prev.length - 1]
      if (!last || last.role !== "assistant") return prev
      const updated = [...prev]
      updated[updated.length - 1] = {
        ...last,
        content: "",
        thinking: undefined,
        contentBlocks: undefined,
        toolCalls: undefined,
        usage: undefined,
        model: undefined,
        fallbackEvent: undefined,
      }
      return updated
    })
    return true
  }

  // text_delta and thinking_delta: buffer and flush via rAF
  if (event.type === "text_delta" || event.type === "thinking_delta") {
    const pending = pendingDeltasFor(deltaBuffersRef, sid)
    if (event.type === "text_delta") {
      pending.text += stringField(event, "content") || stringField(event, "text")
    } else {
      pending.thinking += stringField(event, "content")
    }
    schedulePendingStreamFlush(sid, deps)
    return true
  }

  // Handle usage event
  if (event.type === "usage") {
    updateSessionMessages(sid, (prev) => {
      const last = prev[prev.length - 1]
      if (!last || last.role !== "assistant") return prev
      const updated = [...prev]
      const usage = mergeUsageFromEvent(last.usage, event)
      const model = event.model ? String(event.model) : last.model
      updated[updated.length - 1] = { ...last, usage, model }
      return updated
    })
    return true
  }

  if (event.type === "queued_user_message_inserted") {
    flushPendingStreamDeltas(sid, deps, true)
    const requestId = stringField(event, "request_id")
    const content = stringField(event, "content")
    const messageIdRaw = event.message_id
    const dbId =
      typeof messageIdRaw === "number" && Number.isFinite(messageIdRaw)
        ? messageIdRaw
        : undefined
    const attachmentsMeta =
      typeof event.attachments_meta === "string" ? event.attachments_meta : null
    const attachments = parseUserAttachmentsMeta(attachmentsMeta)
    updateSessionMessages(sid, (prev) => {
      if (dbId != null && prev.some((msg) => msg.role === "user" && msg.dbId === dbId)) {
        return prev
      }
      const now = new Date().toISOString()
      const userMessage: Message = {
        role: "user",
        content,
        timestamp: now,
        ...(dbId != null ? { dbId } : {}),
        ...(attachments ? { attachments } : {}),
        ...(event.is_plan_trigger === true ? { isPlanTrigger: true } : {}),
        ...(event.plan_comment && typeof event.plan_comment === "object"
          ? {
              planComment: event.plan_comment as {
                selectedText: string
                comment: string
              },
            }
          : {}),
      }
      const assistantPlaceholder: Message = {
        role: "assistant",
        content: "",
        timestamp: now,
        _clientId: requestId ? `queued-assistant:${requestId}` : `queued-assistant:${now}`,
      }
      return [...prev, userMessage, assistantPlaceholder]
    })
    return true
  }

  // Flush pending thinking/text buffer before tool_call to preserve display order
  if (event.type === "tool_call") {
    flushPendingStreamDeltas(sid, deps, true)
  }

  // Handle tool_call, tool_result, model_fallback, codex_auth_expired via updateSessionMessages
  if (
    event.type === "thinking_auto_disabled" ||
    event.type === "vision_auto_disabled" ||
    event.type === "vision_bridge" ||
    event.type === "profile_rotation" ||
    event.type === "context_compacted" ||
    event.type === "context_compaction_progress" ||
    event.type === "queued_user_message_blocked" ||
    event.type === "round_limit_reached"
  ) {
    if (isContextCompactionDoneProgress(event)) {
      return true
    }
    // Mirror the backend persister for Tier 0/1 noise, but keep Tier 3/4
    // start markers live-only so a long compaction does not look like a
    // frozen turn. The final `context_compacted` event replaces them below.
    if (event.type === "context_compacted") {
      const data = contextCompactionData(event)
      const tier = typeof data.tier_applied === "number" ? data.tier_applied : 0
      if (tier < 2) {
        return true
      }
    }
    updateSessionMessages(sid, (prev) => {
      const notice: Message = {
        role: "event",
        content: JSON.stringify(event),
      }
      if (event.type === "context_compacted" || event.type === "context_compaction_progress") {
        const startIdx = contextCompactionStartNoticeIndex(prev)
        if (startIdx >= 0) {
          const previousNotice = prev[startIdx]
          if (!previousNotice) return prev
          const updated = [...prev]
          updated[startIdx] = {
            ...notice,
            content:
              event.type === "context_compaction_progress" ||
              isContextCompactionStartDescription(contextCompactionData(event).description)
                ? notice.content
                : mergeWithContextCompactionStart(previousNotice, event),
          }
          return updated
        }
        if (event.type === "context_compacted" && !isContextCompactionStartPayload(event)) {
          const noticeIdx = contextCompactionNoticeIndex(prev)
          if (noticeIdx >= 0) {
            const previousNotice = prev[noticeIdx]
            if (!previousNotice) return prev
            const previousPayload = eventPayloadFromMessage(previousNotice)
            if (!shouldReplaceContextCompactionNotice(previousPayload, event)) {
              return prev
            }
            const updated = [...prev]
            updated[noticeIdx] = notice
            return updated
          }
        }
      }
      const last = prev[prev.length - 1]
      if (last?.role === "assistant") {
        return [...prev.slice(0, -1), notice, last]
      }
      return [...prev, notice]
    })
    return true
  }

  updateSessionMessages(sid, (prev) => {
    const last = prev[prev.length - 1]
    if (!last || last.role !== "assistant") return prev
    const updated = [...prev]

    switch (event.type) {
      case "tool_call": {
        const calls = [...(last.toolCalls || [])]
        const newTool = {
          callId: stringField(event, "call_id"),
          name: stringField(event, "name"),
          arguments: stringField(event, "arguments"),
          startedAtMs: Date.now(),
        }
        calls.push(newTool)
        const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
        blocks.push({ type: "tool_call", tool: { ...newTool } })
        updated[updated.length - 1] = {
          ...last,
          toolCalls: calls,
          contentBlocks: blocks,
        }
        break
      }
      case "tool_call_args_rewritten": {
        // A `PreToolUse` hook rewrote the tool input via `updatedInput`.
        // Update the in-place tool block (same `call_id`) so the rendered
        // arguments match what actually ran, not the pre-rewrite arguments
        // the earlier `tool_call` event delivered. Drops silently if the
        // call_id isn't found (race with a turn reload, etc.).
        const callId = stringField(event, "call_id")
        const args = stringField(event, "arguments")
        const calls = [...(last.toolCalls || [])]
        const idx = calls.findIndex((c) => c.callId === callId)
        if (idx >= 0) {
          calls[idx] = { ...calls[idx], arguments: args }
        }
        const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
        const blockIdx = blocks.findIndex(
          (b) => b.type === "tool_call" && b.tool.callId === callId,
        )
        if (blockIdx >= 0) {
          const block = blocks[blockIdx] as { type: "tool_call"; tool: { callId: string; name: string; arguments: string } }
          blocks[blockIdx] = {
            type: "tool_call",
            tool: { ...block.tool, arguments: args },
          }
        }
        updated[updated.length - 1] = {
          ...last,
          toolCalls: calls,
          contentBlocks: blocks,
        }
        break
      }
      case "tool_result": {
        const mediaItems: MediaItem[] | undefined =
          Array.isArray(event.media_items) && (event.media_items as MediaItem[]).length
            ? (event.media_items as MediaItem[])
            : undefined
        const toolMetadata = extractToolMetadata(event)
        const calls = [...(last.toolCalls || [])]
        const idx = calls.findIndex((c) => c.callId === event.call_id)
        const resolvedDurationMs = (event.duration_ms as number | undefined) ?? (
          idx >= 0 && calls[idx].startedAtMs ? Date.now() - calls[idx].startedAtMs! : undefined
        )
        if (idx >= 0) {
          const isError = typeof event.is_error === "boolean"
            ? event.is_error as boolean
            : hasToolError({ result: event.result as string | undefined })
          calls[idx] = {
            ...calls[idx],
            result: event.result as string,
            isError,
            ...(mediaItems && { mediaItems }),
            ...(resolvedDurationMs != null ? { durationMs: resolvedDurationMs } : {}),
            ...(toolMetadata ? { metadata: toolMetadata } : {}),
          }
        }
        const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
        const blockIdx = blocks.findIndex(
          (b) => b.type === "tool_call" && b.tool.callId === event.call_id,
        )
        if (blockIdx >= 0) {
          const block = blocks[blockIdx] as {
            type: "tool_call"
            tool: {
              callId: string
              name: string
              arguments: string
              result?: string
              mediaItems?: MediaItem[]
              metadata?: ToolMetadata
            }
          }
          blocks[blockIdx] = {
            type: "tool_call",
            tool: {
              ...block.tool,
              result: event.result as string,
              isError: typeof event.is_error === "boolean"
                ? event.is_error as boolean
                : hasToolError({ result: event.result as string | undefined }),
              ...(mediaItems && { mediaItems }),
              ...(resolvedDurationMs != null ? { durationMs: resolvedDurationMs } : {}),
              ...(toolMetadata ? { metadata: toolMetadata } : {}),
            },
          }
        }
        updated[updated.length - 1] = {
          ...last,
          toolCalls: calls,
          contentBlocks: blocks,
        }
        break
      }
      case "model_fallback": {
        updated[updated.length - 1] = {
          ...last,
          fallbackEvent: fallbackEventFromStreamEvent(event),
        }
        break
      }
      case "codex_auth_expired": {
        setShowCodexAuthExpired?.(true)
        break
      }
    }
    return updated
  })

  return false
}
