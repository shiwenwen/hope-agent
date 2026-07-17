import { useEffect, useMemo, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"

/** Mirrors `ha_core::tool_actions::ToolActionRecord` (event flattened). */
export interface PanelActionEntry {
  actionId: string
  source: "browser" | "mac_control"
  sessionId?: string | null
  action: string
  op?: string | null
  target?: string | null
  detail?: string | null
  url?: string | null
  app?: string | null
  ok: boolean
  error?: string | null
  durationMs: number
  startedAt: number
  toolCallId?: string | null
  hasFrame: boolean
  thumbJpegBase64?: string | null
}

export type PanelActionKind = "browser" | "mac-control"

const MAX_ENTRIES = 200

interface FrameEventWithAction {
  actionId?: string | null
  jpegBase64?: string
  sessionId?: string | null
}

/** Match the backend ring's thumbnail bound (≤240px, JPEG q60). */
const THUMB_MAX_WIDTH = 240
const THUMB_JPEG_QUALITY = 0.6
/** Mirror the backend's per-session thumbnail cap. */
const MAX_THUMBNAILS = 50

/** Backend ring policy applied to live state: only the newest
 *  [`MAX_THUMBNAILS`] entries keep their thumbnail — without this, a panel
 *  left open past 50 framed actions grows one retained base64 per step. */
function trimThumbnails(entries: PanelActionEntry[]): PanelActionEntry[] {
  const withThumb = entries.reduce((n, e) => n + (e.thumbJpegBase64 ? 1 : 0), 0)
  let toClear = withThumb - MAX_THUMBNAILS
  if (toClear <= 0) return entries
  return entries.map((e) => {
    if (toClear > 0 && e.thumbJpegBase64) {
      toClear -= 1
      return { ...e, thumbJpegBase64: null }
    }
    return e
  })
}

/** Downscale a full frame to a timeline thumbnail in the renderer — storing
 *  raw 50-200KB frames for up to 200 entries would balloon React state far
 *  past the backend's bounded ring. Best-effort: null on decode failure. */
async function downscaleJpegBase64(jpegBase64: string): Promise<string | null> {
  try {
    const img = new Image()
    img.src = `data:image/jpeg;base64,${jpegBase64}`
    await img.decode()
    const scale = Math.min(1, THUMB_MAX_WIDTH / Math.max(1, img.naturalWidth))
    const width = Math.max(1, Math.round(img.naturalWidth * scale))
    const height = Math.max(1, Math.round(img.naturalHeight * scale))
    const canvas = document.createElement("canvas")
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext("2d")
    if (!ctx) return null
    ctx.drawImage(img, 0, 0, width, height)
    const dataUrl = canvas.toDataURL("image/jpeg", THUMB_JPEG_QUALITY)
    return dataUrl.split(",")[1] ?? null
  } catch {
    return null
  }
}

/** Merge the authoritative backend seed with entries appended live while the
 *  fetch was in flight — a plain replace would silently drop those steps
 *  (their events are never re-delivered). */
function mergeSeed(
  records: PanelActionEntry[],
  liveEntries: PanelActionEntry[],
): PanelActionEntry[] {
  const seen = new Set(records.map((e) => e.actionId))
  const merged = [...records, ...liveEntries.filter((e) => !seen.has(e.actionId))]
  merged.sort((a, b) => a.startedAt - b.startedAt || a.actionId.localeCompare(b.actionId))
  return trimThumbnails(
    merged.length > MAX_ENTRIES ? merged.slice(merged.length - MAX_ENTRIES) : merged,
  )
}

/**
 * Execution timeline for a control panel: seeds from the backend ring buffer
 * (`tool_recent_actions`), then appends live `browser:action` /
 * `mac_control:action` events and backfills thumbnails from the matching
 * frame push. Docked-panel only — remount refetches the authoritative ring.
 */
export function usePanelActionHistory(kind: PanelActionKind, sessionId?: string | null) {
  const source = kind === "browser" ? "browser" : "mac_control"
  const actionEvent = kind === "browser" ? "browser:action" : "mac_control:action"
  const frameEvent = kind === "browser" ? "browser:frame" : "mac_control:frame"
  // Entries are keyed to their (source, session) seed so a session switch
  // renders empty immediately without a synchronous setState in the effect.
  const seedKey = `${source}:${sessionId ?? ""}`
  const [state, setState] = useState<{ key: string; entries: PanelActionEntry[] }>({
    key: seedKey,
    entries: [],
  })
  const entries = useMemo(
    () => (state.key === seedKey ? state.entries : []),
    [seedKey, state],
  )

  // Seed from the backend ring buffer.
  useEffect(() => {
    let alive = true
    getTransport()
      .call<PanelActionEntry[]>("tool_recent_actions", {
        source,
        sessionId: sessionId ?? undefined,
        limit: MAX_ENTRIES,
      })
      .then((records) => {
        if (!alive || !Array.isArray(records)) return
        setState((prev) => ({
          key: seedKey,
          entries: mergeSeed(records, prev.key === seedKey ? prev.entries : []),
        }))
      })
      .catch((e) => {
        logger.warn("ui", "PanelActionHistory::seed", "tool_recent_actions failed", e)
      })
    return () => {
      alive = false
    }
  }, [seedKey, source, sessionId])

  // Live append + thumbnail backfill (functional updates keyed to the seed).
  useEffect(() => {
    const append = (payload: PanelActionEntry) => {
      setState((prev) => {
        const base = prev.key === seedKey ? prev.entries : []
        if (base.some((e) => e.actionId === payload.actionId)) return prev
        const next = [...base, payload]
        return {
          key: seedKey,
          entries: next.length > MAX_ENTRIES ? next.slice(next.length - MAX_ENTRIES) : next,
        }
      })
    }
    const unlistenAction = getTransport().listen(actionEvent, (raw) => {
      const payload = parsePayload<PanelActionEntry>(raw)
      if (!payload?.actionId) return
      if (payload.sessionId && sessionId && payload.sessionId !== sessionId) return
      append(payload)
    })
    const unlistenFrame = getTransport().listen(frameEvent, (raw) => {
      const payload = parsePayload<FrameEventWithAction>(raw)
      if (!payload?.actionId || !payload.jpegBase64) return
      const actionId = payload.actionId
      void downscaleJpegBase64(payload.jpegBase64).then((thumb) => {
        if (!thumb) return
        setState((prev) => {
          if (prev.key !== seedKey) return prev
          const idx = prev.entries.findIndex((e) => e.actionId === actionId)
          if (idx < 0 || prev.entries[idx].thumbJpegBase64) return prev
          const next = [...prev.entries]
          next[idx] = { ...next[idx], hasFrame: true, thumbJpegBase64: thumb }
          return { key: prev.key, entries: trimThumbnails(next) }
        })
      })
    })
    return () => {
      try {
        unlistenAction?.()
        unlistenFrame?.()
      } catch {
        // ignore
      }
    }
  }, [actionEvent, frameEvent, seedKey, sessionId])

  const stats = useMemo(() => {
    const steps = entries.length
    const failed = entries.filter((e) => !e.ok).length
    const totalMs = entries.reduce((acc, e) => acc + (e.durationMs || 0), 0)
    const last = entries[entries.length - 1]
    const currentTarget =
      kind === "browser"
        ? (() => {
            const url = [...entries].reverse().find((e) => e.url)?.url
            try {
              return url ? new URL(url).host : null
            } catch {
              return url ?? null
            }
          })()
        : ([...entries].reverse().find((e) => e.app)?.app ?? last?.target ?? null)
    return { steps, failed, totalMs, currentTarget }
  }, [entries, kind])

  return { entries, stats }
}
