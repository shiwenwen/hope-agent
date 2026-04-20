import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"

export interface ServerRuntimeStatus {
  boundAddr: string | null
  startedAt: number | null
  uptimeSecs: number | null
  startupError: string | null
  eventsWsCount: number
  chatWsCount: number
  activeChatStreams: number
}

interface UseServerStatusResult {
  status: ServerRuntimeStatus | null
  loading: boolean
  error: string | null
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
    a.activeChatStreams === b.activeChatStreams
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
