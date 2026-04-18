import { useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import { parseSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "../useChatSession"
import type {
  Message,
  MediaItem,
  SessionMessage,
  ParentAgentStreamEvent,
} from "@/types/chat"

export interface UseNotificationListenersDeps {
  currentSessionIdRef: React.MutableRefObject<string | null>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  reloadSessions: () => Promise<void>
}

export function useNotificationListeners(deps: UseNotificationListenersDeps) {
  const {
    currentSessionIdRef,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
  } = deps

  // Listen for agent-initiated notification events
  useEffect(() => {
    return getTransport().listen("agent:send_notification", (raw) => {
      const { title, body } = raw as { title: string; body: string }
      notify(title || "OpenComputer", body)
    })
  }, [])

  // Listen for backend-driven parent agent streaming (sub-agent result injection)
  useEffect(() => {
    const unlisten = getTransport().listen("parent_agent_stream", (raw) => {
      const payload = raw as ParentAgentStreamEvent
      const { eventType, parentSessionId, delta } = payload
      const isCurrentSession = currentSessionIdRef.current === parentSessionId

      if (eventType === "started") {
        if (isCurrentSession) {
          setMessages((prev) => {
            const next = [
              ...prev,
              {
                role: "assistant" as const,
                content: "",
                timestamp: new Date().toISOString(),
              },
            ]
            sessionCacheRef.current.set(parentSessionId, next)
            return next
          })
        }
        setLoading(true)
        loadingSessionsRef.current.add(parentSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
      } else if (eventType === "delta" && delta && isCurrentSession) {
        try {
          const ev = JSON.parse(delta)
          const sid = parentSessionId
          if (ev.type === "text_delta" && ev.text) {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                updated[updated.length - 1] = {
                  ...last,
                  content: last.content + ev.text,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "tool_call") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                const toolCalls = [
                  ...(last.toolCalls || []),
                  {
                    callId: ev.call_id,
                    name: ev.name,
                    arguments: ev.arguments || "",
                    startedAtMs: Date.now(),
                  },
                ]
                const blocks = [...(last.contentBlocks || [])]
                if (last.content) blocks.push({ type: "text" as const, content: last.content })
                blocks.push({
                  type: "tool_call" as const,
                  tool: toolCalls[toolCalls.length - 1],
                })
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls,
                  contentBlocks: blocks,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "tool_result") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant" && last.toolCalls) {
                const mediaItems: MediaItem[] | undefined =
                  Array.isArray(ev.media_items) && (ev.media_items as MediaItem[]).length
                    ? (ev.media_items as MediaItem[])
                    : undefined
                const current = last.toolCalls.find((tc) => tc.callId === ev.call_id)
                const resolvedDurationMs = ev.duration_ms ?? (
                  current?.startedAtMs ? Date.now() - current.startedAtMs : undefined
                )
                const toolCalls = last.toolCalls.map((tc) =>
                  tc.callId === ev.call_id
                    ? {
                        ...tc,
                        result: ev.result,
                        ...(mediaItems && { mediaItems }),
                        ...(resolvedDurationMs != null ? { durationMs: resolvedDurationMs } : {}),
                      }
                    : tc,
                )
                const blocks = (last.contentBlocks || []).map((b) =>
                  b.type === "tool_call" && b.tool?.callId === ev.call_id
                    ? {
                        ...b,
                        tool: {
                          ...b.tool!,
                          result: ev.result,
                          ...(mediaItems && { mediaItems }),
                          ...(resolvedDurationMs != null ? { durationMs: resolvedDurationMs } : {}),
                        },
                      }
                    : b,
                )
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls,
                  contentBlocks: blocks,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "usage") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                updated[updated.length - 1] = {
                  ...last,
                  usage: ev,
                  model: ev.model,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          }
        } catch {
          /* ignore parse errors */
        }
      } else if (eventType === "done" || eventType === "error") {
        if (eventType === "error") {
          logger.error("subagent", "inject", "Backend parent agent injection failed", payload.error)
        }
        setLoading(false)
        loadingSessionsRef.current.delete(parentSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        reloadSessions()
        // Reload messages from DB so subagent result message renders with correct type
        if (currentSessionIdRef.current === parentSessionId) {
          getTransport().call<[SessionMessage[], number]>("load_session_messages_latest_cmd", {
            sessionId: parentSessionId,
            limit: PAGE_SIZE,
          })
            .then(([msgs]) => {
              const displayMessages = parseSessionMessages(msgs)
              sessionCacheRef.current.set(parentSessionId, displayMessages)
              setMessages(displayMessages)
            })
            .catch(() => {})
        } else {
          // Not current session — clear cache so next visit loads fresh from DB
          sessionCacheRef.current.delete(parentSessionId)
        }
      }
    })
    return unlisten
  }, [reloadSessions]) // eslint-disable-line react-hooks/exhaustive-deps
}
