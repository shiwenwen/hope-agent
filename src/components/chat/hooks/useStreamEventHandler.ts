import type React from "react"
import type { ContentBlock, Message, MessageUsage } from "@/types/chat"

export interface StreamEventHandlerDeps {
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  deltaBufferRef: React.MutableRefObject<{ text: string; thinking: string; sid: string }>
  deltaFlushRafRef: React.MutableRefObject<number | null>
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
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
  const { updateSessionMessages, deltaBufferRef, deltaFlushRafRef, setShowCodexAuthExpired } = deps

  // text_delta and thinking_delta: buffer and flush via rAF
  if (event.type === "text_delta" || event.type === "thinking_delta") {
    if (event.type === "text_delta") {
      deltaBufferRef.current.text += (event.content as string) || ""
    } else {
      deltaBufferRef.current.thinking += (event.content as string) || ""
    }
    deltaBufferRef.current.sid = sid
    if (deltaFlushRafRef.current === null) {
      deltaFlushRafRef.current = requestAnimationFrame(() => {
        deltaFlushRafRef.current = null
        const buf = deltaBufferRef.current
        const textChunk = buf.text
        const thinkingChunk = buf.thinking
        const flushSid = buf.sid
        buf.text = ""
        buf.thinking = ""
        if (!textChunk && !thinkingChunk) return
        updateSessionMessages(flushSid, (prev) => {
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
          // Only replace the last element to minimize GC pressure
          const updated = prev.slice()
          updated[updated.length - 1] = {
            ...last,
            contentBlocks: blocks,
            ...(textChunk ? { content: last.content + textChunk } : {}),
            ...(thinkingChunk ? { thinking: (last.thinking || "") + thinkingChunk } : {}),
          }
          return updated
        })
      })
    }
    return true
  }

  // Handle usage event
  if (event.type === "usage") {
    updateSessionMessages(sid, (prev) => {
      const updated = [...prev]
      const last = updated[updated.length - 1]
      if (!last || last.role !== "assistant") return updated
      const prevUsage = last.usage || {}
      const usage: MessageUsage = {
        ...prevUsage,
        ...(event.duration_ms != null ? { durationMs: event.duration_ms as number } : {}),
        ...(event.input_tokens != null ? { inputTokens: event.input_tokens as number } : {}),
        ...(event.output_tokens != null ? { outputTokens: event.output_tokens as number } : {}),
        ...(event.cache_creation_input_tokens != null
          ? { cacheCreationInputTokens: event.cache_creation_input_tokens as number }
          : {}),
        ...(event.cache_read_input_tokens != null
          ? { cacheReadInputTokens: event.cache_read_input_tokens as number }
          : {}),
      }
      const model = event.model ? String(event.model) : last.model
      updated[updated.length - 1] = { ...last, usage, model }
      return updated
    })
    return true
  }

  // Flush pending thinking/text buffer before tool_call to preserve display order
  if (event.type === "tool_call") {
    if (deltaFlushRafRef.current !== null) {
      cancelAnimationFrame(deltaFlushRafRef.current)
      deltaFlushRafRef.current = null
    }
    const buf = deltaBufferRef.current
    const textChunk = buf.text
    const thinkingChunk = buf.thinking
    buf.text = ""
    buf.thinking = ""
    if (textChunk || thinkingChunk) {
      updateSessionMessages(sid, (prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (!last || last.role !== "assistant") return updated
        const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
        if (thinkingChunk) {
          const lastBlock = blocks[blocks.length - 1]
          if (lastBlock && lastBlock.type === "thinking") {
            blocks[blocks.length - 1] = { type: "thinking", content: lastBlock.content + thinkingChunk }
          } else {
            blocks.push({ type: "thinking", content: thinkingChunk })
          }
        }
        if (textChunk) {
          const lastBlock = blocks[blocks.length - 1]
          if (lastBlock && lastBlock.type === "text") {
            blocks[blocks.length - 1] = { type: "text", content: lastBlock.content + textChunk }
          } else {
            blocks.push({ type: "text", content: textChunk })
          }
        }
        updated[updated.length - 1] = {
          ...last,
          contentBlocks: blocks,
          ...(textChunk ? { content: last.content + textChunk } : {}),
          ...(thinkingChunk ? { thinking: (last.thinking || "") + thinkingChunk } : {}),
        }
        return updated
      })
    }
  }

  // Handle tool_call, tool_result, model_fallback, codex_auth_expired via updateSessionMessages
  updateSessionMessages(sid, (prev) => {
    const updated = [...prev]
    const last = updated[updated.length - 1]
    if (!last || last.role !== "assistant") return updated

    switch (event.type) {
      case "tool_call": {
        const calls = [...(last.toolCalls || [])]
        const newTool = {
          callId: event.call_id as string,
          name: event.name as string,
          arguments: event.arguments as string,
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
        const mediaUrls: string[] | undefined = (event.media_urls as string[])?.length ? (event.media_urls as string[]) : undefined
        const calls = [...(last.toolCalls || [])]
        const idx = calls.findIndex((c) => c.callId === event.call_id)
        const resolvedDurationMs = (event.duration_ms as number | undefined) ?? (
          idx >= 0 && calls[idx].startedAtMs ? Date.now() - calls[idx].startedAtMs! : undefined
        )
        if (idx >= 0) {
          calls[idx] = {
            ...calls[idx],
            result: event.result as string,
            ...(mediaUrls && { mediaUrls }),
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
            tool: { callId: string; name: string; arguments: string; result?: string; mediaUrls?: string[] }
          }
          blocks[blockIdx] = {
            type: "tool_call",
            tool: {
              ...block.tool,
              result: event.result as string,
              ...(mediaUrls && { mediaUrls }),
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
          fallbackEvent: event,
        }
        break
      }
      case "codex_auth_expired": {
        setShowCodexAuthExpired(true)
        break
      }
    }
    return updated
  })

  return false
}
