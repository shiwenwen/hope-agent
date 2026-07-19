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
  flushPendingStreamDeltas,
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
  acceptedSeq: number
  durableSeq: number
  committedSeq: number
  persistenceRunId?: string | null
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
  finalSeq?: number
  durableSeq?: number
  assistantMessageId?: number | null
  persistenceStatus?: "committed" | "recovered" | "pending"
}

interface SessionStreamSnapshot {
  sessionId: string
  streamId?: string | null
  turnId?: string | null
  persistenceRunId: string
  throughSeq: number
  durableSeq: number
  committedSeq: number
  status: string
  events: Array<{ seq: number; event: string }>
}

interface SnapshotHandshake {
  deltas: StreamDeltaPayload[]
  ended: boolean
  stagedMessages: Message[] | null
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
  const snapshotHandshakeRef = useRef(new Map<string, SnapshotHandshake>())

  const applyStreamPayload = (payload: StreamDeltaPayload) => {
    const sid = payload.sessionId
    const seq = payload.seq
    if (payload.streamId && endedStreamIdsRef.current.get(sid) === payload.streamId) return
    const cursorKey = streamCursorKey(sid, payload.streamId)
    const prev = lastSeqRef.current.get(cursorKey) ?? 0
    if (seq <= prev) return
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
  }

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
      const handshake = snapshotHandshakeRef.current.get(sid)
      if (handshake) {
        handshake.deltas.push(payload)
        return
      }
      applyStreamPayload(payload)
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(deltaBuffersRef)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Durable snapshot handshake: listeners are registered first, incoming
  // frames are buffered while DB + journal snapshots load, then the durable
  // prefix is replayed before frames newer than `throughSeq`.
  useEffect(() => {
    if (!currentSessionId) return
    const sid = currentSessionId
    let cancelled = false
    const handshakeRegistry = snapshotHandshakeRef.current
    const handshake: SnapshotHandshake = {
      deltas: [],
      ended: false,
      stagedMessages: null,
    }
    handshakeRegistry.set(sid, handshake)
    const stagedSessionCacheRef = {
      current: new Map(sessionCacheRef.current),
    }
    const stageMessages: React.Dispatch<React.SetStateAction<Message[]>> = (value) => {
      const previous = handshake.stagedMessages ?? stagedSessionCacheRef.current.get(sid) ?? []
      handshake.stagedMessages = typeof value === "function" ? value(previous) : value
    }
    Promise.all([
      getTransport().call<SessionStreamState>("get_session_stream_state", { sessionId: sid }),
      getTransport().call<SessionStreamSnapshot | null>("get_session_stream_snapshot", {
        sessionId: sid,
      }),
      reloadAndMergeSessionMessages({
        sessionId: sid,
        pageSize: PAGE_SIZE,
        sessionCacheRef: stagedSessionCacheRef,
        setMessages: stageMessages,
      }),
    ])
      .then(([state, snapshot]) => {
        if (cancelled || handshake.ended) return
        if (!state) return
        if (handshake.stagedMessages) {
          sessionCacheRef.current.set(sid, handshake.stagedMessages)
          setMessages(handshake.stagedMessages)
        }
        if (state.turnId && state.active) {
          onTurnStarted?.(sid, state.turnId)
        } else {
          onTurnEnded?.(
            sid,
            state.status ?? state.lastTerminalStatus ?? null,
            state.interruptReason ?? null,
          )
        }
        const streamId = snapshot?.streamId || state.streamId || undefined
        if (snapshot) {
          if (streamId) endedStreamIdsRef.current.delete(sid)
          const cursorKey = streamCursorKey(sid, streamId)
          // DB snapshot can lag accepted memory state. Rebuild from the
          // journal prefix instead of jumping to `lastSeq`.
          lastSeqRef.current.set(cursorKey, 0)
          const snapshotIsLive = snapshot.status === "running"
          if (snapshotIsLive) {
            // Replace, rather than append to, any pre-reload transient tail.
            // A cold reload has no optimistic placeholder, so create one
            // before replaying the full durable prefix.
            updateSessionMessages(sid, (prev) => {
              // Round checkpoints materialize query-friendly rows before the
              // run is terminal. The journal snapshot is authoritative for
              // that same run, so remove its DB projection before replaying;
              // otherwise text/tool blocks render twice after reload.
              const canonical = prev.filter(
                (message) => message.persistenceRunId !== snapshot.persistenceRunId,
              )
              const last = canonical[canonical.length - 1]
              const placeholder: Message = {
                role: "assistant",
                content: "",
                timestamp: new Date().toISOString(),
                _clientId: `durable-stream:${snapshot.persistenceRunId}`,
              }
              if (!last || last.role !== "assistant" || typeof last.dbId === "number") {
                return [...canonical, placeholder]
              }
              const updated = [...canonical]
              updated[updated.length - 1] = { ...last, ...placeholder }
              return updated
            })
            for (const durableEvent of snapshot.events) {
              applyStreamPayload({
                sessionId: sid,
                streamId,
                seq: durableEvent.seq,
                event: durableEvent.event,
              })
            }
          }
          // A committed/recovered run is already represented by canonical DB
          // rows loaded above. Replaying its journal would duplicate content.
          lastSeqRef.current.set(cursorKey, Number(snapshot.throughSeq) || 0)
          if (handshakeRegistry.get(sid) === handshake) {
            handshakeRegistry.delete(sid)
          }
          handshake.deltas
            // Sequence numbers are scoped to a stream. A newer turn may start
            // while the old stream's snapshot request is still in flight and
            // restart at seq=1; only apply the old snapshot watermark to
            // buffered frames from that same stream.
            .filter(
              (event) => event.streamId !== streamId || event.seq > snapshot.throughSeq,
            )
            .sort((a, b) => a.seq - b.seq)
            .forEach(applyStreamPayload)
        } else {
          // Legacy/no-run fallback: only durableSeq is safe to skip. Never
          // jump to acceptedSeq because the DB may not contain that tail.
          const cursorKey = streamCursorKey(sid, streamId)
          lastSeqRef.current.set(cursorKey, Number(state.durableSeq) || 0)
          if (handshakeRegistry.get(sid) === handshake) {
            handshakeRegistry.delete(sid)
          }
          handshake.deltas.sort((a, b) => a.seq - b.seq).forEach(applyStreamPayload)
        }
        if (!state.active) return
        if (!loadingSessionsRef.current.has(sid)) {
          loadingSessionsRef.current.add(sid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        }
        if (currentSessionIdRef.current === sid) setLoading(true)
      })
      .catch(() => {
        // Older backend without this command — gracefully degrade.
        if (handshake.ended) return
        if (handshakeRegistry.get(sid) === handshake) {
          handshakeRegistry.delete(sid)
        }
        handshake.deltas.sort((a, b) => a.seq - b.seq).forEach(applyStreamPayload)
      })
    return () => {
      cancelled = true
      if (handshakeRegistry.get(sid) === handshake) {
        handshakeRegistry.delete(sid)
      }
      if (!handshake.ended) {
        handshake.deltas.sort((a, b) => a.seq - b.seq).forEach(applyStreamPayload)
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentSessionId])

  useEffect(() => {
    const unlisten = getTransport().listen(EVENT_CHAT_STREAM_END, (raw) => {
      const payload = raw as StreamEndPayload
      if (!payload?.sessionId) return
      const sid = payload.sessionId
      const streamId = payload.streamId || streamIdFromPayload(raw)
      const handshake = snapshotHandshakeRef.current.get(sid)
      if (handshake) {
        handshake.ended = true
        snapshotHandshakeRef.current.delete(sid)
        // A committed/recovered end is reconciled from its atomic DB rows
        // below. A pending end has no such fallback, so preserve the staged
        // DB baseline (when ready) and drain every pre-end durable frame now.
        if (payload.persistenceStatus === "pending") {
          if (handshake.stagedMessages) {
            sessionCacheRef.current.set(sid, handshake.stagedMessages)
            setMessages(handshake.stagedMessages)
          }
          updateSessionMessages(sid, (prev) => {
            const last = prev[prev.length - 1]
            if (last?.role === "assistant" && typeof last.dbId !== "number") return prev
            return [
              ...prev,
              {
                role: "assistant",
                content: "",
                timestamp: new Date().toISOString(),
                _clientId: `durable-end:${streamId || sid}`,
              },
            ]
          })
          handshake.deltas.sort((a, b) => a.seq - b.seq).forEach(applyStreamPayload)
        }
      }
      if (streamId) endedStreamIdsRef.current.set(sid, streamId)
      onTurnEnded?.(sid, payload.status, payload.interruptReason)

      // The backend deliberately delivers every durable delta before end, but
      // the most recent frame may still be waiting in our 30fps RAF merge
      // buffer. Flush it before cleanup; a `pending` persistence end does not
      // reload DB rows and therefore has no later reconciliation fallback.
      flushPendingStreamDeltas(
        sid,
        { updateSessionMessages, deltaBuffersRef, setShowCodexAuthExpired },
        true,
      )
      discardPendingStreamDeltas(sid, deltaBuffersRef)
      loadingSessionsRef.current.delete(sid)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))

      if (currentSessionIdRef.current === sid) {
        setLoading(false)
        // A pending/degraded end has no atomic DB materialization yet. Keep
        // the durable in-memory snapshot visible instead of replacing it with
        // an older DB window.
        if (payload.persistenceStatus !== "pending") {
          reloadAndMergeSessionMessages({
            sessionId: sid,
            pageSize: PAGE_SIZE,
            sessionCacheRef,
            setMessages,
          })
        }
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
