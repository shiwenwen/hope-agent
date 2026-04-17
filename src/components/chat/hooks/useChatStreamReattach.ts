import { useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { parseSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "../useChatSession"
import type { Message, SessionMessage } from "@/types/chat"
import { handleStreamEvent } from "./useStreamEventHandler"

export interface UseChatStreamReattachDeps {
  currentSessionId: string | null
  currentSessionIdRef: React.MutableRefObject<string | null>
  /** Per-session seq cursor shared with `useChatStream` for dedup. Owned by the
   *  parent (ChatScreen) so both hooks can see / update it. */
  lastSeqRef: React.MutableRefObject<Map<string, number>>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  reloadSessions: () => Promise<void>
}

interface SessionStreamState {
  active: boolean
  lastSeq: number
}

interface StreamDeltaPayload {
  sessionId: string
  seq: number
  event: string
}

interface StreamEndPayload {
  sessionId: string
}

/**
 * Global listener for the EventBus-backed chat stream ("chat:stream_delta" /
 * "chat:stream_end"). Survives frontend reloads — when the per-call Tauri
 * `Channel` / WebSocket dies, this hook continues delivering tool_call /
 * tool_result / text_delta / etc. events to the UI so ongoing chats resume
 * rendering after a window refresh.
 *
 * Deduplication is by `_oc_seq` against `lastSeqRef`, which `useChatStream`'s
 * primary-path handler also updates. When the Channel is alive it bumps the
 * cursor first; EventBus arrivals with `seq <= lastSeq` are dropped. When the
 * Channel is dead (post-reload), the cursor is seeded by
 * `handleSwitchSession`'s `get_session_stream_state` call so only truly new
 * events are applied.
 */
export function useChatStreamReattach(deps: UseChatStreamReattachDeps): void {
  const {
    currentSessionId,
    currentSessionIdRef,
    lastSeqRef,
    updateSessionMessages,
    setShowCodexAuthExpired,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
  } = deps

  // Independent delta buffers: the EventBus path only drives rendering when
  // the primary Channel is dead (post-reload). `handleStreamEvent` dedup via
  // `lastSeqRef` guarantees each event hits at most one path, so these
  // buffers never race the primary-path `useChatStream` buffers.
  const deltaBufferRef = useRef({ text: "", thinking: "", sid: "" })
  const deltaFlushRafRef = useRef<number | null>(null)

  useEffect(() => {
    const unlisten = getTransport().listen("chat:stream_delta", (raw) => {
      const payload = raw as StreamDeltaPayload
      if (!payload?.sessionId || typeof payload.seq !== "number") return

      const sid = payload.sessionId
      const seq = payload.seq
      const prev = lastSeqRef.current.get(sid) ?? 0
      if (seq <= prev) return // already handled via primary path
      lastSeqRef.current.set(sid, seq)

      let event: Record<string, unknown>
      try {
        event = JSON.parse(payload.event) as Record<string, unknown>
      } catch (e) {
        logger.warn("chat", "useChatStreamReattach::parse", "Failed to parse bus event", e)
        return
      }

      // session_created on the bus path can race with the primary-path
      // session_created that `useChatStream.onmessage` handles specially; we
      // only need to recognise it here for sessions we don't already track.
      handleStreamEvent(event, sid, {
        updateSessionMessages,
        deltaBufferRef,
        deltaFlushRafRef,
        setShowCodexAuthExpired,
      })
    })
    return unlisten
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // On session switch, ask the backend whether the target session currently
  // has an in-flight chat stream. If so, seed lastSeqRef to the backend's
  // cursor so events already reflected in the DB snapshot we just loaded are
  // skipped, and mark the session as loading so the spinner appears.
  useEffect(() => {
    if (!currentSessionId) return
    const sid = currentSessionId
    let cancelled = false
    getTransport()
      .call<SessionStreamState>("get_session_stream_state", { sessionId: sid })
      .then((state) => {
        if (cancelled) return
        if (!state?.active) return
        if (!lastSeqRef.current.has(sid)) {
          lastSeqRef.current.set(sid, Number(state.lastSeq) || 0)
        }
        if (!loadingSessionsRef.current.has(sid)) {
          loadingSessionsRef.current.add(sid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        }
        if (currentSessionIdRef.current === sid) setLoading(true)
      })
      .catch(() => {
        // Older backend without this command — ignore; primary path still works
        // for live sessions; reattach gracefully degrades.
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentSessionId])

  useEffect(() => {
    const unlisten = getTransport().listen("chat:stream_end", (raw) => {
      const payload = raw as StreamEndPayload
      if (!payload?.sessionId) return
      const sid = payload.sessionId

      lastSeqRef.current.delete(sid)
      loadingSessionsRef.current.delete(sid)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))

      if (currentSessionIdRef.current === sid) {
        setLoading(false)
        // Reload from DB so the final assistant row / synthetic streaming
        // assistant is replaced by the definitive message list.
        getTransport()
          .call<[SessionMessage[], number]>("load_session_messages_latest_cmd", {
            sessionId: sid,
            limit: PAGE_SIZE,
          })
          .then(([msgs]) => {
            const displayMessages = parseSessionMessages(msgs)
            sessionCacheRef.current.set(sid, displayMessages)
            setMessages(displayMessages)
          })
          .catch(() => {})
      } else {
        // Off-screen session — drop the cache so next visit reloads fresh.
        sessionCacheRef.current.delete(sid)
      }
      reloadSessions()
    })
    return unlisten
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
}
