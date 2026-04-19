import { useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { reloadAndMergeSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "../useChatSession"
import type { Message } from "@/types/chat"
import { handleStreamEvent } from "./useStreamEventHandler"

// Backend constants: see `crates/ha-core/src/chat_engine/stream_broadcast.rs`.
const EVENT_CHAT_STREAM_DELTA = "chat:stream_delta"
const EVENT_CHAT_STREAM_END = "chat:stream_end"

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
 * EventBus reattach path for the main chat stream. Runs alongside the primary
 * `Channel` / WebSocket path in useChatStream; when the primary sink dies
 * (frontend reload), this path keeps the UI updating. Dedup by `_oc_seq`
 * against `lastSeqRef` — whichever path sees an event first bumps the cursor.
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

  // Buffers are per-hook, not shared with useChatStream's primary path;
  // lastSeqRef dedup ensures each event hits at most one path.
  const deltaBufferRef = useRef({ text: "", thinking: "", sid: "" })
  const deltaFlushRafRef = useRef<number | null>(null)

  useEffect(() => {
    const unlisten = getTransport().listen(EVENT_CHAT_STREAM_DELTA, (raw) => {
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

  // Seeds lastSeqRef from the backend's cursor on session switch so events
  // already reflected in the DB snapshot we loaded are skipped.
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
        // Older backend without this command — gracefully degrade.
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentSessionId])

  useEffect(() => {
    const unlisten = getTransport().listen(EVENT_CHAT_STREAM_END, (raw) => {
      const payload = raw as StreamEndPayload
      if (!payload?.sessionId) return
      const sid = payload.sessionId

      lastSeqRef.current.delete(sid)
      loadingSessionsRef.current.delete(sid)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))

      if (currentSessionIdRef.current === sid) {
        setLoading(false)
        reloadAndMergeSessionMessages({
          sessionId: sid,
          pageSize: PAGE_SIZE,
          sessionCacheRef,
          setMessages,
        })
      } else {
        sessionCacheRef.current.delete(sid)
      }
      reloadSessions()
    })
    return unlisten
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
}
