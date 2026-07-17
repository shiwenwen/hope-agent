import { useEffect, useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { createFrameStore } from "@/lib/frame-store"

// ── Types (mirror ha_core::mac_control::MacControlFramePayload) ─────────

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

const store = createFrameStore<MacControlFramePayload>({
  name: "mac-control",
  eventName: MAC_CONTROL_FRAME_EVENT,
  capture: async ({ displayId }) => {
    const response = await getTransport().call<MacControlFrameResponse>(
      "mac_control_capture_frame",
      displayId != null ? { displayId } : undefined,
    )
    return { frame: response.frame ?? null, error: response.error ?? null }
  },
})

export const macControlFrameStore = store

export function useMacControlFrame(opts: { pollKey: string; pollActive: boolean }) {
  const { pollKey, pollActive } = opts
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
    displayId: store.getParams().displayId,
  }
}
