import { useEffect, useState } from "react"
import type { TFunction } from "i18next"
import { getTransport } from "@/lib/transport-provider"

export interface ActiveChatCounts {
  desktop: number
  http: number
  channel: number
  total: number
}

export interface ServerRuntimeStatus {
  boundAddr: string | null
  startedAt: number | null
  uptimeSecs: number | null
  startupError: string | null
  eventsWsCount: number
  chatWsCount: number
  /**
   * True when this status was fetched through the Tauri shell — whose
   * webview talks to the backend via IPC, not WebSocket. UIs count the
   * desktop app itself as one "active connection" even though it doesn't
   * show up in the WS counters.
   */
  localDesktopClient: boolean
  /**
   * Back-compat alias for `activeChatCounts.total`. Meaning changed:
   * now counts in-flight chat engines (desktop + HTTP + channel), not
   * WebSocket subscribers like the original field did.
   */
  activeChatStreams: number
  activeChatCounts: ActiveChatCounts
}

/** Total "active connections" including the desktop shell when applicable. */
export function totalActiveConnections(status: ServerRuntimeStatus): number {
  return (
    status.eventsWsCount +
    status.chatWsCount +
    (status.localDesktopClient ? 1 : 0)
  )
}

interface UseServerStatusResult {
  status: ServerRuntimeStatus | null
  loading: boolean
  error: string | null
}

/**
 * Render the per-source breakdown as `X desktop · Y http[ · Z channel]`,
 * translated. Channel segment is omitted at 0 to keep the common (no-IM)
 * case tight. Returns `null` when `total === 0` so callers can hide the
 * parenthetical.
 */
export function formatActiveChatCounts(
  counts: ActiveChatCounts,
  t: TFunction,
): string | null {
  if (counts.total === 0) return null
  const parts = [
    t("settings.serverChatSourceDesktop", { count: counts.desktop }),
    t("settings.serverChatSourceHttp", { count: counts.http }),
  ]
  if (counts.channel > 0) {
    parts.push(t("settings.serverChatSourceChannel", { count: counts.channel }))
  }
  return parts.join(" · ")
}

/**
 * Render the WS breakdown `X events · Y chat[ · 1 local]`, translated.
 * `local` segment appears only when the desktop shell is the caller.
 */
export function formatActiveConnectionsSub(
  status: ServerRuntimeStatus,
  t: TFunction,
): string {
  const base = t("settings.serverWsSub", {
    events: status.eventsWsCount,
    chat: status.chatWsCount,
  })
  return status.localDesktopClient
    ? `${base} · ${t("settings.serverConnSubLocal", { count: 1 })}`
    : base
}

/**
 * Short-form uptime (`2h 15m` / `5m 30s` / `12s`) — finer granularity than
 * the dashboard's `formatUptime`, which rounds to minutes and would render
 * a just-started server as `0m`.
 */
export function formatServerUptime(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return "-"
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const s = secs % 60
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

function statusUnchanged(a: ServerRuntimeStatus, b: ServerRuntimeStatus): boolean {
  return (
    a.boundAddr === b.boundAddr &&
    a.startedAt === b.startedAt &&
    a.uptimeSecs === b.uptimeSecs &&
    a.startupError === b.startupError &&
    a.eventsWsCount === b.eventsWsCount &&
    a.chatWsCount === b.chatWsCount &&
    a.localDesktopClient === b.localDesktopClient &&
    a.activeChatCounts.desktop === b.activeChatCounts.desktop &&
    a.activeChatCounts.http === b.activeChatCounts.http &&
    a.activeChatCounts.channel === b.activeChatCounts.channel &&
    a.activeChatCounts.total === b.activeChatCounts.total
  )
}

/**
 * Poll the embedded server for its runtime status on a fixed interval.
 * Pauses when the tab is hidden; returns same `status` reference between
 * polls when nothing changed so consumers don't re-render on no-ops.
 */
export function useServerStatus(intervalMs: number = 5000): UseServerStatusResult {
  const [status, setStatus] = useState<ServerRuntimeStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setInterval> | null = null

    async function fetchOnce() {
      try {
        const s = await getTransport().call<ServerRuntimeStatus>(
          "get_server_runtime_status",
        )
        if (cancelled) return
        setStatus((prev) => (prev && statusUnchanged(prev, s) ? prev : s))
        setError(null)
      } catch (e) {
        if (cancelled) return
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    function start() {
      fetchOnce()
      timer = setInterval(fetchOnce, intervalMs)
    }
    function stop() {
      if (timer !== null) {
        clearInterval(timer)
        timer = null
      }
    }

    function handleVisibility() {
      if (document.hidden) {
        stop()
      } else if (timer === null) {
        start()
      }
    }

    start()
    document.addEventListener("visibilitychange", handleVisibility)

    return () => {
      cancelled = true
      stop()
      document.removeEventListener("visibilitychange", handleVisibility)
    }
  }, [intervalMs])

  return { status, loading, error }
}
