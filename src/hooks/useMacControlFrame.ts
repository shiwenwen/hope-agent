import { useEffect, useLayoutEffect, useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { createFrameStore } from "@/lib/frame-store"

// ── Types (mirror ha_mac::MacControlFramePayload) ─────────

export interface MacControlAppSummary {
  pid: number
  bundleId?: string | null
  name?: string | null
}

export interface MacControlBounds {
  x: number
  y: number
  width: number
  height: number
}

export interface MacControlFramePayload {
  sessionId?: string | null
  snapshotId: string
  mediaId?: string | null
  path?: string | null
  jpegBase64: string
  widthPx: number
  heightPx: number
  target?: "display" | "window"
  displayId?: number | null
  windowId?: string | null
  windowTitle?: string | null
  boundsPoints?: MacControlBounds | null
  scale?: number | null
  capturedAt: number
  frontmostApp?: MacControlAppSummary | null
  /** Action-timeline foreign key when this frame followed a recorded step. */
  actionId?: string | null
}

export interface MacControlFrameResponse {
  frame?: MacControlFramePayload | null
  error?: string | null
}

export interface MacControlDisplaySummary {
  id: number
  framePoints: MacControlBounds
  scale: number
}

export interface MacControlDisplaysResponse {
  displays: MacControlDisplaySummary[]
  error?: string | null
}

export const MAC_CONTROL_FRAME_EVENT = "mac_control:frame"

type MacControlFrameOwner = Pick<MacControlFramePayload, "sessionId" | "mediaId" | "path" | "actionId">

export function isMacControlToolFrameForSession(
  frame: MacControlFrameOwner,
  sessionId: string | null,
): boolean {
  return !!sessionId && frame.sessionId === sessionId && !!(frame.mediaId || frame.path || frame.actionId)
}

const store = createFrameStore<MacControlFramePayload>({
  name: "mac-control",
  eventName: MAC_CONTROL_FRAME_EVENT,
  // Ownerless panel polls are global display captures, not tool activity.
  acceptEvent: (frame, { sessionId }) => frame.sessionId
    ? frame.sessionId === sessionId
    : !(frame.mediaId || frame.path || frame.actionId),
  capture: async ({ displayId }) => {
    const response = await getTransport().call<MacControlFrameResponse>(
      "mac_control_capture_frame",
      displayId != null ? { displayId } : undefined,
    )
    return { frame: response.frame ?? null, error: response.error ?? null }
  },
})

export function useMacControlFrame(opts: {
  sessionId?: string | null
  pollKey: string
  pollActive: boolean
}) {
  const { sessionId = null, pollKey, pollActive } = opts
  useLayoutEffect(() => {
    store.setSessionId(sessionId)
  }, [sessionId])
  useEffect(() => {
    store.setPollActive(pollKey, pollActive)
    return () => store.setPollActive(pollKey, false)
  }, [pollKey, pollActive])
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot)
  return {
    frame: snapshot.frame,
    error: snapshot.error,
    refresh: store.refresh,
    setDisplayId: store.setDisplayId,
    // Reactive: lives in the snapshot so the display Select re-renders on
    // selection change (setDisplayId republishes).
    displayId: snapshot.displayId,
  }
}
