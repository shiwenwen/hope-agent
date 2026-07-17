import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"

/**
 * Reference-counted live-frame store shared by the docked panel and the
 * floating window. Exactly one transport listener + one 1Hz poll exists per
 * store no matter how many containers are mounted; the last unsubscribe is
 * delayed so a docked↔floating container swap never drops the event stream.
 */

export interface FrameSnapshot<F> {
  frame: F | null
  error: string | null
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
  capture: (params: FrameCaptureParams) => Promise<FrameSnapshot<F>>
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
  getParams: () => FrameCaptureParams
}

export function createFrameStore<F>(options: FrameStoreOptions<F>): FrameStore<F> {
  const pollIntervalMs = options.pollIntervalMs ?? 1000
  let snapshot: FrameSnapshot<F> = { frame: null, error: null }
  const params: FrameCaptureParams = { sessionId: null, displayId: null }
  const listeners = new Set<() => void>()
  const pollVotes = new Set<string>()
  let unlistenTransport: (() => void) | null = null
  let detachTimer: ReturnType<typeof setTimeout> | null = null
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let refreshSeq = 0

  function notify() {
    listeners.forEach((fn) => fn())
  }

  function setSnapshot(next: FrameSnapshot<F>) {
    if (next.frame === snapshot.frame && next.error === snapshot.error) return
    snapshot = next
    notify()
  }

  async function refresh(): Promise<void> {
    const seq = ++refreshSeq
    try {
      const next = await options.capture({ ...params })
      // A newer refresh (or session switch) superseded this response.
      if (seq !== refreshSeq) return
      setSnapshot(next)
    } catch (e) {
      logger.warn("ui", `FrameStore::${options.name}`, "frame capture failed", e)
      if (seq === refreshSeq) {
        setSnapshot({ frame: snapshot.frame, error: "capture_failed" })
      }
    }
  }

  function attachTransport() {
    if (unlistenTransport) return
    unlistenTransport = getTransport().listen(options.eventName, (raw) => {
      const payload = parsePayload<F>(raw)
      if (!payload) return
      if (options.acceptEvent && !options.acceptEvent(payload, params)) return
      setSnapshot({ frame: payload, error: null })
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
      pollTimer = setInterval(() => void refresh(), pollIntervalMs)
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
      // Session switch: the cached frame belongs to the old session.
      setSnapshot({ frame: null, error: null })
      if (pollVotes.size > 0) void refresh()
    },
    setDisplayId(displayId) {
      if (params.displayId === displayId) return
      params.displayId = displayId
      if (pollVotes.size > 0) void refresh()
    },
    getParams: () => ({ ...params }),
  }
}
