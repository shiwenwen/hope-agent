import type React from "react"
import type { ContentBlock, FallbackEvent, MediaItem, Message } from "@/types/chat"
import { mergeUsageFromEvent } from "../chatUtils"
import { hasToolError } from "../message/executionStatus"

export interface StreamEventHandlerDeps {
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  deltaBuffersRef: React.MutableRefObject<StreamDeltaBuffers>
  setShowCodexAuthExpired?: React.Dispatch<React.SetStateAction<boolean>>
}

export interface StreamDeltaBuffers {
  pending: Map<string, { text: string; thinking: string }>
  rafs: Map<string, number>
}

const LEGACY_STREAM_ID = "__legacy__"

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

function flushPendingStreamDeltas(
  sid: string,
  deps: StreamEventHandlerDeps,
  cancelScheduled: boolean,
): void {
  const pending = takePendingStreamDeltas(sid, deps.deltaBuffersRef, cancelScheduled)
  if (!pending) return

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
    deps.deltaBuffersRef.current.rafs.delete(sid)
    flushPendingStreamDeltas(sid, deps, false)
  })
  state.rafs.set(sid, raf)
}

function stringField(event: Record<string, unknown>, key: string): string {
  const value = event[key]
  return typeof value === "string" ? value : ""
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

  // Flush pending thinking/text buffer before tool_call to preserve display order
  if (event.type === "tool_call") {
    flushPendingStreamDeltas(sid, deps, true)
  }

  // Handle tool_call, tool_result, model_fallback, codex_auth_expired via updateSessionMessages
  if (event.type === "thinking_auto_disabled") {
    updateSessionMessages(sid, (prev) => {
      const notice: Message = {
        role: "event",
        content: JSON.stringify(event),
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
      case "tool_result": {
        const mediaItems: MediaItem[] | undefined =
          Array.isArray(event.media_items) && (event.media_items as MediaItem[]).length
            ? (event.media_items as MediaItem[])
            : undefined
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
          }
        }
        const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
        const blockIdx = blocks.findIndex(
          (b) => b.type === "tool_call" && b.tool.callId === event.call_id,
        )
        if (blockIdx >= 0) {
          const block = blocks[blockIdx] as {
            type: "tool_call"
            tool: { callId: string; name: string; arguments: string; result?: string; mediaItems?: MediaItem[] }
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
