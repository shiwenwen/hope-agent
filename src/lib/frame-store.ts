import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"

/**
 * Reference-counted live-frame store shared by the docked panel and the
 * floating window. Exactly one transport listener + one 1Hz poll exists per
 * store no matter how many containers are mounted; the last unsubscribe is
 * delayed so a docked↔floating container swap never drops the event stream.
 */

/** Sentinel error set when the capture command itself throws — callers map it
 *  to a localized message. Backend-reported errors pass through verbatim. */
export const FRAME_CAPTURE_FAILED = "capture_failed"

export interface FrameSnapshot<F> {
  frame: F | null
  error: string | null
  /** Mac control only — currently selected capture display (reactive). */
  displayId: number | null
}

export interface FrameCaptureParams {
  sessionId: string | null
  /** Mac control only — panel capture target display. */
  displayId: number | null
}

interface FrameStoreOptions<F> {
  name: string
  eventName: string
  pollIntervalMs?: number
  capture: (params: FrameCaptureParams) => Promise<{ frame: F | null; error: string | null }>
  /** Reject pushed frames that belong to another session. */
  acceptEvent?: (payload: F, params: FrameCaptureParams) => boolean
}

/** Grace period covering the unmount/mount gap when content moves between
 *  the docked shell and the floating window. */
const DETACH_DELAY_MS = 300

export interface FrameStore<F> {
  subscribe: (cb: () => void) => () => void
  getSnapshot: () => FrameSnapshot<F>
  refresh: () => Promise<void>
  /** Multiple containers vote; polling runs while any key is active. */
  setPollActive: (key: string, active: boolean) => void
  setSessionId: (sessionId: string | null) => void
  setDisplayId: (displayId: number | null) => void
}

export function createFrameStore<F>(options: FrameStoreOptions<F>): FrameStore<F> {
  const pollIntervalMs = options.pollIntervalMs ?? 1000
  const params: FrameCaptureParams = { sessionId: null, displayId: null }
  let snapshot: FrameSnapshot<F> = { frame: null, error: null, displayId: null }
  const listeners = new Set<() => void>()
  const pollVotes = new Set<string>()
  let unlistenTransport: (() => void) | null = null
  let detachTimer: ReturnType<typeof setTimeout> | null = null
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let refreshSeq = 0
  let inFlight = false

  function notify() {
    listeners.forEach((fn) => fn())
  }

  /** Publish a new snapshot stamped with the current display selection. */
  function publish(frame: F | null, error: string | null) {
    if (
      frame === snapshot.frame &&
      error === snapshot.error &&
      params.displayId === snapshot.displayId
    ) {
      return
    }
    snapshot = { frame, error, displayId: params.displayId }
    notify()
  }

  async function refresh(): Promise<void> {
    const seq = ++refreshSeq
    inFlight = true
    try {
      const next = await options.capture({ ...params })
      // A newer refresh (or session/display switch) superseded this response.
      if (seq !== refreshSeq) return
      publish(next.frame, next.error)
    } catch (e) {
      logger.warn("ui", `FrameStore::${options.name}`, "frame capture failed", e)
      if (seq === refreshSeq) {
        publish(snapshot.frame, FRAME_CAPTURE_FAILED)
      }
    } finally {
      if (seq === refreshSeq) inFlight = false
    }
  }

  function attachTransport() {
    if (unlistenTransport) return
    unlistenTransport = getTransport().listen(options.eventName, (raw) => {
      const payload = parsePayload<F>(raw)
      if (!payload) return
      if (options.acceptEvent && !options.acceptEvent(payload, params)) return
      publish(payload, null)
    })
  }

  function detachTransport() {
    try {
      unlistenTransport?.()
    } catch {
      // ignore
    }
    unlistenTransport = null
  }

  function syncPollTimer() {
    const shouldPoll = listeners.size > 0 && pollVotes.size > 0
    if (shouldPoll && !pollTimer) {
      // Coalesce: a capture slower than the poll interval must not be
      // superseded by the next tick, or every request gets discarded and the
      // panel starves on slow/remote backends.
      pollTimer = setInterval(() => {
        if (!inFlight) void refresh()
      }, pollIntervalMs)
    } else if (!shouldPoll && pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  return {
    subscribe(cb) {
      if (detachTimer) {
        clearTimeout(detachTimer)
        detachTimer = null
      }
      attachTransport()
      listeners.add(cb)
      syncPollTimer()
      return () => {
        listeners.delete(cb)
        syncPollTimer()
        if (listeners.size === 0 && !detachTimer) {
          detachTimer = setTimeout(() => {
            detachTimer = null
            if (listeners.size === 0) detachTransport()
          }, DETACH_DELAY_MS)
        }
      }
    },
    getSnapshot: () => snapshot,
    refresh,
    setPollActive(key, active) {
      const changed = active ? !pollVotes.has(key) : pollVotes.has(key)
      if (active) pollVotes.add(key)
      else pollVotes.delete(key)
      syncPollTimer()
      // Becoming active with no frame yet → prime immediately instead of
      // waiting a full poll interval.
      if (changed && active && !snapshot.frame) void refresh()
    },
    setSessionId(sessionId) {
      if (params.sessionId === sessionId) return
      params.sessionId = sessionId
      // Invalidate any in-flight capture unconditionally — even with no poll
      // vote a manual refresh for the old session must not land in the new
      // one after we clear the frame below. The superseded request can no
      // longer clear `inFlight` (seq mismatch), so reset it here or polling
      // would stay blocked forever.
      refreshSeq += 1
      inFlight = false
      publish(null, null)
      if (pollVotes.size > 0) void refresh()
    },
    setDisplayId(displayId) {
      if (params.displayId === displayId) return
      params.displayId = displayId
      // Same invalidation as setSessionId: an old-display capture in flight
      // must not overwrite the new selection's mirror.
      refreshSeq += 1
      inFlight = false
      // Republish so the display selector re-renders even before a new frame.
      publish(snapshot.frame, snapshot.error)
      void refresh()
    },
  }
}
