import { useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { mergeUsageFromEvent, parseSessionMessages, reloadAndMergeSessionMessages } from "../chatUtils"
import { hasToolError } from "../message/executionStatus"
import { PAGE_SIZE } from "./constants"
import type { Message, ContentBlock, MediaItem, SessionMessage } from "@/types/chat"

interface UseChannelStreamingParams {
  currentSessionIdRef: React.MutableRefObject<string | null>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  reloadSessions: () => Promise<void>
}

export function useChannelStreaming({
  currentSessionIdRef,
  sessionCacheRef,
  loadingSessionsRef,
  setMessages,
  setLoading,
  setLoadingSessionIds,
  reloadSessions,
}: UseChannelStreamingParams): void {
  // Channel streaming delta buffer + rAF handle (mirrors useChatStream's batching)
  const channelDeltaBufferRef = useRef({ text: "", thinking: "", sid: "" })
  const channelDeltaFlushRafRef = useRef<number | null>(null)

  // Listen for channel stream lifecycle — loading state + message placeholder
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    unlisteners.push(getTransport().listen("channel:stream_start", (raw) => {
      const payload = raw as { sessionId: string }
      if (!payload.sessionId) return
      // Mark session as loading
      loadingSessionsRef.current.add(payload.sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      // Refresh sidebar to show new session / update title
      reloadSessions()
      if (payload.sessionId === currentSessionIdRef.current) {
        setLoading(true)
        // Load latest messages from DB (includes the just-saved user message),
        // then append an empty assistant placeholder for streaming into.
        getTransport().call<[SessionMessage[], number, boolean]>("load_session_messages_latest_cmd", {
          sessionId: payload.sessionId,
          limit: PAGE_SIZE,
        }).then(([msgs]) => {
          const parsed = parseSessionMessages(msgs)
          const withPlaceholder = [
            ...parsed,
            {
              role: "assistant" as const,
              content: "",
              isStreaming: true,
              timestamp: new Date().toISOString(),
            },
          ]
          sessionCacheRef.current.set(payload.sessionId, withPlaceholder)
          setMessages(withPlaceholder)
        }).catch(() => {
          // Fallback: just add placeholder to existing messages
          setMessages((prev) => {
            const next = [
              ...prev,
              {
                role: "assistant" as const,
                content: "",
                isStreaming: true,
                timestamp: new Date().toISOString(),
              },
            ]
            sessionCacheRef.current.set(payload.sessionId, next)
            return next
          })
        })
      }
    }))

    unlisteners.push(getTransport().listen("channel:stream_end", (raw) => {
      const payload = raw as { sessionId: string }
      if (!payload.sessionId) return
      loadingSessionsRef.current.delete(payload.sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))

      // Flush any remaining delta buffer
      if (channelDeltaFlushRafRef.current !== null) {
        cancelAnimationFrame(channelDeltaFlushRafRef.current)
        channelDeltaFlushRafRef.current = null
      }
      channelDeltaBufferRef.current = { text: "", thinking: "", sid: "" }

      if (payload.sessionId === currentSessionIdRef.current) {
        setLoading(false)
        reloadAndMergeSessionMessages({
          sessionId: payload.sessionId,
          pageSize: PAGE_SIZE,
          sessionCacheRef,
          setMessages,
        })
      }
    }))

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [setLoading, setLoadingSessionIds, reloadSessions, currentSessionIdRef, sessionCacheRef, loadingSessionsRef, setMessages])

  // Listen for channel streaming events — full event processing (mirrors useChatStream)
  useEffect(() => {
    const unlisten = getTransport().listen("channel:stream_delta", (raw) => {
      const payload = raw as { sessionId: string; event: string }
      if (!payload.sessionId || payload.sessionId !== currentSessionIdRef.current) return

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      let ev: any = null
      try { ev = JSON.parse(payload.event) } catch { return }
      if (!ev || !ev.type) return

      const sid = payload.sessionId

      // ── text_delta / thinking_delta: buffer and flush via rAF ──
      if (ev.type === "text_delta" || ev.type === "thinking_delta") {
        if (ev.type === "text_delta") {
          channelDeltaBufferRef.current.text += ev.text || ev.content || ""
        } else {
          channelDeltaBufferRef.current.thinking += ev.content || ""
        }
        channelDeltaBufferRef.current.sid = sid
        if (channelDeltaFlushRafRef.current === null) {
          channelDeltaFlushRafRef.current = requestAnimationFrame(() => {
            channelDeltaFlushRafRef.current = null
            const buf = channelDeltaBufferRef.current
            const textChunk = buf.text
            const thinkingChunk = buf.thinking
            buf.text = ""
            buf.thinking = ""
            if (!textChunk && !thinkingChunk) return
            setMessages((prev) => {
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
              sessionCacheRef.current.set(sid, updated)
              return updated
            })
          })
        }
        return
      }

      // ── Flush pending buffer before tool_call to preserve display order ──
      if (ev.type === "tool_call") {
        if (channelDeltaFlushRafRef.current !== null) {
          cancelAnimationFrame(channelDeltaFlushRafRef.current)
          channelDeltaFlushRafRef.current = null
        }
        const buf = channelDeltaBufferRef.current
        const textChunk = buf.text
        const thinkingChunk = buf.thinking
        buf.text = ""
        buf.thinking = ""
        if (textChunk || thinkingChunk) {
          setMessages((prev) => {
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
            sessionCacheRef.current.set(sid, updated)
            return updated
          })
        }
      }

      // ── Process structured events (tool_call, tool_result, usage, model_fallback) ──
      setMessages((prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (!last || last.role !== "assistant") return updated

        switch (ev.type) {
          case "tool_call": {
            const calls = [...(last.toolCalls || [])]
            const newTool = {
              callId: ev.call_id,
              name: ev.name,
              arguments: ev.arguments || "",
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
              Array.isArray(ev.media_items) && (ev.media_items as MediaItem[]).length
                ? (ev.media_items as MediaItem[])
                : undefined
            const calls = [...(last.toolCalls || [])]
            const idx = calls.findIndex((c) => c.callId === ev.call_id)
            const resolvedDurationMs = ev.duration_ms ?? (
              idx >= 0 && calls[idx].startedAtMs ? Date.now() - calls[idx].startedAtMs! : undefined
            )
            const isError = typeof ev.is_error === "boolean"
              ? ev.is_error
              : hasToolError({ result: ev.result })
            if (idx >= 0) {
              calls[idx] = {
                ...calls[idx],
                result: ev.result,
                isError,
                ...(mediaItems && { mediaItems }),
                ...(resolvedDurationMs != null ? { durationMs: resolvedDurationMs } : {}),
              }
            }
            const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
            const blockIdx = blocks.findIndex(
              (b) => b.type === "tool_call" && b.tool.callId === ev.call_id,
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
                  result: ev.result,
                  isError,
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
          case "usage": {
            const usage = mergeUsageFromEvent(last.usage, ev)
            const model = ev.model ? String(ev.model) : last.model
            updated[updated.length - 1] = { ...last, usage, model }
            break
          }
          case "model_fallback": {
            updated[updated.length - 1] = { ...last, fallbackEvent: ev }
            break
          }
          default:
            return updated
        }
        sessionCacheRef.current.set(sid, updated)
        return updated
      })
    })
    return unlisten
  }, [currentSessionIdRef, sessionCacheRef, setMessages])

  // Listen for channel message updates — refresh sessions + reload current session messages
  // (but SKIP DB reload if the session is currently streaming to avoid overwriting stream state)
  useEffect(() => {
    const unlisten = getTransport().listen("channel:message_update", (raw) => {
      const payload = raw as { sessionId: string }
      reloadSessions()
      // If the session is currently streaming, skip DB reload — stream_end will reload
      if (payload.sessionId && loadingSessionsRef.current.has(payload.sessionId)) {
        return
      }
      // If the updated session is currently active, reload its messages from DB
      if (payload.sessionId && payload.sessionId === currentSessionIdRef.current) {
        getTransport().call<[SessionMessage[], number, boolean]>("load_session_messages_latest_cmd", {
          sessionId: payload.sessionId,
          limit: PAGE_SIZE,
        }).then(([msgs]) => {
          const parsed = parseSessionMessages(msgs)
          setMessages(parsed)
          sessionCacheRef.current.set(payload.sessionId, parsed)
        }).catch(() => {})
      }
    })
    return unlisten
  }, [reloadSessions, currentSessionIdRef, loadingSessionsRef, sessionCacheRef, setMessages])
}
