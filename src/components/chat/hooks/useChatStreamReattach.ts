import { useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { reloadAndMergeSessionMessages } from "../chatUtils"
import { PAGE_SIZE } from "../useChatSession"
import type { ChatTurnInterruptReason, ChatTurnStatus, Message } from "@/types/chat"
import {
  createStreamDeltaBuffers,
  discardAllPendingStreamDeltas,
  discardPendingStreamDeltas,
  handleStreamEvent,
  streamCursorKey,
  streamIdFromPayload,
} from "./useStreamEventHandler"

// Backend constants: see `crates/ha-core/src/chat_engine/stream_broadcast.rs`.
const EVENT_CHAT_STREAM_DELTA = "chat:stream_delta"
const EVENT_CHAT_STREAM_END = "chat:stream_end"
const EVENT_CHAT_TURN_STARTED = "chat:turn_started"

// `chat:stream_end` is the primary signal that clears a session's `loading`
// flag. If that event is ever missed (dropped frame, race, process boundary,
// abnormal turn termination) the session stays stuck "running" until a manual
// reload / session switch. While the current session is flagged loading we
// reconcile against the authoritative backend state (`get_session_stream_state`)
// on this interval as a self-healing safety net — a long-running turn keeps
// reporting `active: true`, so this never clears a genuinely-busy session.
const STREAM_STATE_RECONCILE_INTERVAL_MS = 15_000

// Before clearing a "stuck" loading flag we re-confirm the inactive state after
// this delay. `useChatStream` adds a session to `loadingSessionsRef` (optimistic
// "running") BEFORE awaiting `startChat`, so there is a brief window where a
// freshly-sent turn is flagged loading while the backend still reports the
// PREVIOUS turn as terminal. A genuinely-ended turn stays inactive across this
// delay; a just-starting turn flips to active and aborts the clear.
const STREAM_STATE_RECONCILE_CONFIRM_MS = 2_000

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

export interface UseChatStreamReattachDeps {
  currentSessionId: string | null
  currentSessionIdRef: React.MutableRefObject<string | null>
  /** Per-session seq cursor shared with `useChatStream` for dedup. Owned by the
   *  parent (ChatScreen) so both hooks can see / update it. */
  lastSeqRef: React.MutableRefObject<Map<string, number>>
  endedStreamIdsRef: React.MutableRefObject<Map<string, string>>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  reloadSessions: () => Promise<void>
  onTurnStarted?: (sessionId: string, turnId: string) => void
  onTurnEnded?: (
    sessionId: string,
    status?: ChatTurnStatus | null,
    interruptReason?: ChatTurnInterruptReason | null,
  ) => void
}

export interface SessionStreamState {
  active: boolean
  lastSeq: number
  streamId?: string | null
  turnId?: string | null
  status?: ChatTurnStatus | null
  lastTerminalStatus?: ChatTurnStatus | null
  interruptReason?: ChatTurnInterruptReason | null
}

interface StreamDeltaPayload {
  sessionId: string
  seq: number
  streamId?: string
  event: string
}

interface StreamEndPayload {
  sessionId: string
  streamId?: string
  turnId?: string | null
  status?: ChatTurnStatus | null
  interruptReason?: ChatTurnInterruptReason | null
}

/**
 * EventBus path for the chat stream. Role differs per transport:
 *  - Tauri mode: tertiary safety net for the in-flight `Channel` path inside
 *    `useChatStream` — when the primary sink dies (frontend reload) this path
 *    keeps the UI updating.
 *  - HTTP mode: this path *is* the primary delivery for stream deltas.
 *    `transport.startChat` over HTTP only synthesizes a `session_created`
 *    event for cache-rename bookkeeping; everything else flows here via
 *    `/ws/events` → `chat:stream_delta`.
 *
 * Dedup by `_oc_seq` against `lastSeqRef` — whichever path sees an event
 * first bumps the cursor.
 */
export function useChatStreamReattach(deps: UseChatStreamReattachDeps): void {
  const {
    currentSessionId,
    currentSessionIdRef,
    lastSeqRef,
    endedStreamIdsRef,
    updateSessionMessages,
    setShowCodexAuthExpired,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
    onTurnStarted,
    onTurnEnded,
  } = deps

  // Buffers are per-hook, not shared with useChatStream's primary path;
  // lastSeqRef dedup ensures each event hits at most one path. Within this
  // hook they are keyed by session so overlapping background streams cannot
  // mix pending text before the rAF flush runs.
  const deltaBuffersRef = useRef(createStreamDeltaBuffers())

  useEffect(() => {
    const unlisten = getTransport().listen(EVENT_CHAT_TURN_STARTED, (raw) => {
      const payload = raw as { sessionId?: string; turnId?: string } | null
      if (!payload?.sessionId || !payload.turnId) return
      onTurnStarted?.(payload.sessionId, payload.turnId)
    })
    return unlisten
  }, [onTurnStarted])

  useEffect(() => {
    const unlisten = getTransport().listen(EVENT_CHAT_STREAM_DELTA, (raw) => {
      const payload = raw as StreamDeltaPayload
      if (!payload?.sessionId || typeof payload.seq !== "number") return

      const sid = payload.sessionId
      const seq = payload.seq
      if (payload.streamId && endedStreamIdsRef.current.get(sid) === payload.streamId) return
      const cursorKey = streamCursorKey(sid, payload.streamId)
      const prev = lastSeqRef.current.get(cursorKey) ?? 0
      if (seq <= prev) return // already handled via primary path
      lastSeqRef.current.set(cursorKey, seq)

      let event: Record<string, unknown>
      try {
        event = JSON.parse(payload.event) as Record<string, unknown>
      } catch (e) {
        logger.warn("chat", "useChatStreamReattach::parse", "Failed to parse bus event", e)
        return
      }

      handleStreamEvent(event, sid, {
        updateSessionMessages,
        deltaBuffersRef,
        setShowCodexAuthExpired,
      })
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(deltaBuffersRef)
    }
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
        if (!state) return
        if (state.turnId && state.active) {
          onTurnStarted?.(sid, state.turnId)
        } else {
          onTurnEnded?.(
            sid,
            state.status ?? state.lastTerminalStatus ?? null,
            state.interruptReason ?? null,
          )
        }
        if (!state.active) return
        const streamId = state.streamId || undefined
        if (streamId) endedStreamIdsRef.current.delete(sid)
        const cursorKey = streamCursorKey(sid, streamId)
        if (!lastSeqRef.current.has(cursorKey)) {
          lastSeqRef.current.set(cursorKey, Number(state.lastSeq) || 0)
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
      const streamId = payload.streamId || streamIdFromPayload(raw)
      if (streamId) endedStreamIdsRef.current.set(sid, streamId)
      onTurnEnded?.(sid, payload.status, payload.interruptReason)

      discardPendingStreamDeltas(sid, deltaBuffersRef)
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

  // Self-healing reconcile for a stuck `loading` flag (see the constant's note).
  // Mirrors the `chat:stream_end` teardown above, but driven by polling the
  // backend's authoritative stream state instead of an event we might miss.
  useEffect(() => {
    const sid = currentSessionId
    if (!sid) return
    let cancelled = false
    let inFlight = false

    const reconcile = async () => {
      // Cheap pre-check: do nothing unless THIS session is believed loading.
      if (inFlight || !loadingSessionsRef.current.has(sid)) return
      inFlight = true
      try {
        const state = await getTransport().call<SessionStreamState>("get_session_stream_state", {
          sessionId: sid,
        })
        // Bail on anything that changed while the call was in flight, and never
        // clear a turn the backend still reports as active (e.g. a long
        // background-tool turn legitimately running for minutes).
        if (cancelled || currentSessionIdRef.current !== sid) return
        if (state.active || !loadingSessionsRef.current.has(sid)) return

        // Re-confirm after a short delay so we don't mistake a just-sent turn
        // (loading flagged before the backend registered it) for a finished one.
        await delay(STREAM_STATE_RECONCILE_CONFIRM_MS)
        if (cancelled || currentSessionIdRef.current !== sid) return
        if (!loadingSessionsRef.current.has(sid)) return // cleared by stream_end meanwhile
        const recheck = await getTransport().call<SessionStreamState>("get_session_stream_state", {
          sessionId: sid,
        })
        if (cancelled || currentSessionIdRef.current !== sid) return
        if (recheck.active || !loadingSessionsRef.current.has(sid)) return

        // Terminal but the stream_end never landed → run the same teardown.
        if (recheck.streamId) endedStreamIdsRef.current.set(sid, recheck.streamId)
        onTurnEnded?.(sid, recheck.status ?? recheck.lastTerminalStatus ?? null, recheck.interruptReason ?? null)
        discardPendingStreamDeltas(sid, deltaBuffersRef)
        loadingSessionsRef.current.delete(sid)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        setLoading(false)
        void reloadAndMergeSessionMessages({
          sessionId: sid,
          pageSize: PAGE_SIZE,
          sessionCacheRef,
          setMessages,
        })
        void reloadSessions()
      } catch {
        // Older backend without the command, or a transient failure — leave
        // loading as-is and try again on the next tick.
      } finally {
        inFlight = false
      }
    }

    const interval = setInterval(reconcile, STREAM_STATE_RECONCILE_INTERVAL_MS)
    // Coming back to a backgrounded window is the common moment to discover a
    // turn quietly ended while we were away — reconcile immediately then too.
    const onFocus = () => {
      void reconcile()
    }
    window.addEventListener("focus", onFocus)
    return () => {
      cancelled = true
      clearInterval(interval)
      window.removeEventListener("focus", onFocus)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentSessionId])
}
