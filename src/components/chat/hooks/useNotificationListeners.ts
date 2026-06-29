import { useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import { reloadAndMergeSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "../useChatSession"
import {
  createStreamDeltaBuffers,
  discardAllPendingStreamDeltas,
  discardPendingStreamDeltas,
  flushPendingStreamDeltas,
  handleStreamEvent,
} from "./useStreamEventHandler"
import type {
  Message,
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
  /**
   * Main ChatScreen receives ParentInjection deltas through the resumable
   * `chat:stream_delta` bus. Lightweight surfaces that do not mount
   * `useChatStreamReattach` keep consuming the legacy parent stream.
   */
  consumeParentStreamDeltas?: boolean
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
    consumeParentStreamDeltas = true,
  } = deps
  const parentDeltaBuffersRef = useRef(createStreamDeltaBuffers())

  const parentStreamHandlerDeps = {
    deltaBuffersRef: parentDeltaBuffersRef,
    updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => {
      setMessages((prev) => {
        const next = updater(prev)
        sessionCacheRef.current.set(sessionId, next)
        return next
      })
    },
  }

  // Listen for agent-initiated notification events
  useEffect(() => {
    const unlisten = getTransport().listen("agent:send_notification", (raw) => {
      const { title, body } = raw as { title: string; body: string }
      notify(title || "Hope Agent", body)
    })
    return unlisten
  }, [])

  // Backend-generated session metadata updates (for example async LLM titles).
  useEffect(() => {
    const unlisten = getTransport().listen("session:title_updated", () => {
      reloadSessions()
    })
    return unlisten
    // reloadSessions is stable in practice; keep one listener per mounted chat screen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
      } else if (eventType === "delta" && delta && isCurrentSession && consumeParentStreamDeltas) {
        try {
          const event = JSON.parse(delta) as Record<string, unknown>
          const sid = parentSessionId
          if (!event?.type) return
          handleStreamEvent(event, sid, parentStreamHandlerDeps)
        } catch {
          /* ignore parse errors */
        }
      } else if (eventType === "done" || eventType === "error") {
        if (isCurrentSession) {
          flushPendingStreamDeltas(parentSessionId, parentStreamHandlerDeps, true)
        } else {
          discardPendingStreamDeltas(parentSessionId, parentDeltaBuffersRef)
        }
        if (eventType === "error") {
          logger.error("subagent", "inject", "Backend parent agent injection failed", payload.error)
        }
        setLoading(false)
        loadingSessionsRef.current.delete(parentSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        reloadSessions()
        // Reload messages from DB so subagent result message renders with correct type.
        if (currentSessionIdRef.current === parentSessionId) {
          reloadAndMergeSessionMessages({
            sessionId: parentSessionId,
            pageSize: PAGE_SIZE,
            sessionCacheRef,
            setMessages,
          })
        } else {
          // Not current session — clear cache so next visit loads fresh from DB
          sessionCacheRef.current.delete(parentSessionId)
        }
      }
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(parentDeltaBuffersRef)
    }
    // reloadSessions is useCallback([setSessions]) — setSessions is a stable
    // useState setter, so this effect subscribes once per mount in practice.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reloadSessions])
}
