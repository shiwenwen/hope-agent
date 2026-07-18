import { useEffect, useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { createFrameStore } from "@/lib/frame-store"

// ── Types (mirror ha_core::browser::frame::BrowserFramePayload) ─────────

export interface BrowserFramePayload {
  sessionId?: string | null
  targetId?: string | null
  url?: string | null
  title?: string | null
  jpegBase64: string
  capturedAt: number
  backend: string
  /** Action-timeline foreign key when this frame followed a recorded step. */
  actionId?: string | null
}

/** Backend-emitted event name (see `crates/ha-core/src/browser/frame.rs`). */
export const BROWSER_FRAME_EVENT = "browser:frame"

const store = createFrameStore<BrowserFramePayload>({
  name: "browser",
  eventName: BROWSER_FRAME_EVENT,
  capture: async ({ sessionId }) => {
    // `null` is the backend's empty signal — no active backend. Clear the
    // frame so the mirror doesn't freeze on a stale screenshot.
    const frame = await getTransport().call<BrowserFramePayload | null>("browser_capture_frame", {
      sessionId,
    })
    return { frame, error: null }
  },
  acceptEvent: (payload, { sessionId }) =>
    !(payload.sessionId && sessionId && payload.sessionId !== sessionId),
})

export function useBrowserFrame(opts: {
  sessionId?: string | null
  /** Unique per container ("docked" / "floating"). */
  pollKey: string
  pollActive: boolean
}) {
  const { sessionId = null, pollKey, pollActive } = opts
  useEffect(() => {
    store.setSessionId(sessionId)
  }, [sessionId])
  useEffect(() => {
    store.setPollActive(pollKey, pollActive)
    return () => store.setPollActive(pollKey, false)
  }, [pollKey, pollActive])
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot)
  return { frame: snapshot.frame, error: snapshot.error, refresh: store.refresh }
}
