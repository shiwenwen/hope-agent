import { useCallback, useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parseSessionMessages, reloadAndMergeSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "./constants"
import type { Message, SessionMessage } from "@/types/chat"
import {
  createStreamDeltaBuffers,
  discardAllPendingStreamDeltas,
  discardPendingStreamDeltas,
  handleStreamEvent,
} from "./useStreamEventHandler"

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
  const deltaBuffersRef = useRef(createStreamDeltaBuffers())
  const preparingStreamRef = useRef<Set<string>>(new Set())
  const preparedStreamRef = useRef<Set<string>>(new Set())
  const queuedEventsRef = useRef<Map<string, Record<string, unknown>[]>>(new Map())
  const streamGenerationRef = useRef<Map<string, number>>(new Map())
  const nextStreamGenerationRef = useRef(0)

  const updateSessionMessages = useCallback(
    (sessionId: string, updater: (prev: Message[]) => Message[]) => {
      const isActive = currentSessionIdRef.current === sessionId
      const hasCached = sessionCacheRef.current.has(sessionId)
      if (!isActive && !hasCached) return
      const prev = sessionCacheRef.current.get(sessionId) || []
      const next = updater(prev)
      sessionCacheRef.current.set(sessionId, next)
      if (isActive) {
        setMessages(next)
      }
    },
    [currentSessionIdRef, sessionCacheRef, setMessages],
  )

  const appendStreamingPlaceholder = useCallback((messages: Message[]) => {
    return [
      ...messages,
      {
        role: "assistant" as const,
        content: "",
        isStreaming: true,
        timestamp: new Date().toISOString(),
      },
    ]
  }, [])

  const replayQueuedEvents = useCallback(
    (sessionId: string) => {
      const queued = queuedEventsRef.current.get(sessionId)
      if (!queued?.length) return
      queuedEventsRef.current.delete(sessionId)
      for (const event of queued) {
        handleStreamEvent(event, sessionId, {
          updateSessionMessages,
          deltaBuffersRef,
        })
      }
    },
    [updateSessionMessages],
  )

  // Listen for channel stream lifecycle — loading state + message placeholder
  useEffect(() => {
    const unlisteners: Array<() => void> = []
    const preparingStreams = preparingStreamRef.current
    const preparedStreams = preparedStreamRef.current
    const queuedEvents = queuedEventsRef.current
    const streamGenerations = streamGenerationRef.current

    unlisteners.push(getTransport().listen("channel:stream_start", (raw) => {
      const payload = raw as { sessionId: string }
      if (!payload.sessionId) return
      const sessionId = payload.sessionId
      const generation = nextStreamGenerationRef.current + 1
      nextStreamGenerationRef.current = generation
      streamGenerationRef.current.set(sessionId, generation)
      preparedStreamRef.current.delete(sessionId)
      preparingStreamRef.current.delete(sessionId)
      queuedEventsRef.current.delete(sessionId)
      discardPendingStreamDeltas(sessionId, deltaBuffersRef)

      // Mark session as loading
      loadingSessionsRef.current.add(sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      // Refresh sidebar to show new session / update title
      reloadSessions()
      const isActive = sessionId === currentSessionIdRef.current
      const hadCached = sessionCacheRef.current.has(sessionId)

      if (isActive || hadCached) {
        preparingStreamRef.current.add(sessionId)
        if (!isActive) {
          sessionCacheRef.current.delete(sessionId)
        }
        getTransport().call<[SessionMessage[], number, boolean]>("load_session_messages_latest_cmd", {
          sessionId,
          limit: PAGE_SIZE,
        }).then(([msgs]) => {
          if (streamGenerationRef.current.get(sessionId) !== generation) return
          const withPlaceholder = appendStreamingPlaceholder(parseSessionMessages(msgs))
          preparingStreamRef.current.delete(sessionId)
          preparedStreamRef.current.add(sessionId)
          sessionCacheRef.current.set(sessionId, withPlaceholder)
          if (sessionId === currentSessionIdRef.current) {
            setMessages(withPlaceholder)
          }
          replayQueuedEvents(sessionId)
        }).catch(() => {
          if (streamGenerationRef.current.get(sessionId) !== generation) return
          preparingStreamRef.current.delete(sessionId)
          preparedStreamRef.current.delete(sessionId)
          queuedEventsRef.current.delete(sessionId)
          discardPendingStreamDeltas(sessionId, deltaBuffersRef)
          if (sessionId === currentSessionIdRef.current) {
            const baseMessages = sessionCacheRef.current.get(sessionId)
            if (!baseMessages) {
              setMessages((prev) => {
                const fallback = appendStreamingPlaceholder(prev)
                sessionCacheRef.current.set(sessionId, fallback)
                return fallback
              })
              preparedStreamRef.current.add(sessionId)
              queueMicrotask(() => replayQueuedEvents(sessionId))
              return
            }
            const fallback = appendStreamingPlaceholder(baseMessages)
            preparedStreamRef.current.add(sessionId)
            sessionCacheRef.current.set(sessionId, fallback)
            setMessages(fallback)
            replayQueuedEvents(sessionId)
          } else {
            sessionCacheRef.current.delete(sessionId)
          }
        })
      }

      if (isActive) {
        setLoading(true)
      }
    }))

    unlisteners.push(getTransport().listen("channel:stream_end", (raw) => {
      const payload = raw as { sessionId: string }
      if (!payload.sessionId) return
      const sessionId = payload.sessionId
      streamGenerationRef.current.delete(sessionId)
      preparingStreamRef.current.delete(sessionId)
      preparedStreamRef.current.delete(sessionId)
      queuedEventsRef.current.delete(sessionId)

      loadingSessionsRef.current.delete(sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))

      discardPendingStreamDeltas(sessionId, deltaBuffersRef)

      if (sessionId === currentSessionIdRef.current) {
        setLoading(false)
        reloadAndMergeSessionMessages({
          sessionId,
          pageSize: PAGE_SIZE,
          sessionCacheRef,
          setMessages,
        })
      } else {
        sessionCacheRef.current.delete(sessionId)
      }
    }))

    return () => {
      unlisteners.forEach((fn) => fn())
      discardAllPendingStreamDeltas(deltaBuffersRef)
      preparingStreams.clear()
      preparedStreams.clear()
      queuedEvents.clear()
      streamGenerations.clear()
    }
  }, [
    appendStreamingPlaceholder,
    currentSessionIdRef,
    loadingSessionsRef,
    reloadSessions,
    replayQueuedEvents,
    sessionCacheRef,
    setLoading,
    setLoadingSessionIds,
    setMessages,
  ])

  // Listen for channel streaming events. The event handler is shared with the
  // main chat stream so text/tool/usage rendering and rAF buffering follow the
  // same lifecycle contract across GUI, HTTP, and IM-originated turns.
  useEffect(() => {
    const unlisten = getTransport().listen("channel:stream_delta", (raw) => {
      const payload = raw as { sessionId: string; event: string }
      if (!payload.sessionId) return

      let event: Record<string, unknown>
      try {
        event = JSON.parse(payload.event) as Record<string, unknown>
      } catch {
        return
      }
      if (!event?.type) return

      const sid = payload.sessionId
      if (preparingStreamRef.current.has(sid)) {
        const queued = queuedEventsRef.current.get(sid) || []
        queued.push(event)
        queuedEventsRef.current.set(sid, queued)
        return
      }

      const isActive = sid === currentSessionIdRef.current
      if (!isActive && !preparedStreamRef.current.has(sid)) {
        return
      }

      if (isActive && !preparedStreamRef.current.has(sid)) {
        const cached = sessionCacheRef.current.get(sid)
        const last = cached?.[cached.length - 1]
        if (cached && (last?.role !== "assistant" || !last.isStreaming)) {
          const withPlaceholder = appendStreamingPlaceholder(cached)
          preparedStreamRef.current.add(sid)
          sessionCacheRef.current.set(sid, withPlaceholder)
          setMessages(withPlaceholder)
        }
      }

      handleStreamEvent(event, sid, {
        updateSessionMessages,
        deltaBuffersRef,
      })
    })
    return unlisten
  }, [appendStreamingPlaceholder, currentSessionIdRef, sessionCacheRef, setMessages, updateSessionMessages])

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
