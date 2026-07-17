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
        if (alive && Array.isArray(records)) setState({ key: seedKey, entries: records })
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
      setState((prev) => {
        if (prev.key !== seedKey) return prev
        const idx = prev.entries.findIndex((e) => e.actionId === payload.actionId)
        if (idx < 0 || prev.entries[idx].thumbJpegBase64) return prev
        const next = [...prev.entries]
        next[idx] = { ...next[idx], hasFrame: true, thumbJpegBase64: payload.jpegBase64 }
        return { key: prev.key, entries: next }
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
