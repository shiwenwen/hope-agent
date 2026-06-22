import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"

// Minimal shapes — mirror the camelCase serde of the Rust browser status types.
export interface BrowserExtStatus {
  kind: string
  backendAvailable: boolean
  extensionConnected: boolean
  extensionVersion?: string | null
  nativeHostManifestPath?: string | null
  nativeHostName: string
}
export interface BrowserCdpStatus {
  connected: boolean
  mode: "launch" | "connect" | null
  tabs: { targetId: string }[]
}

export interface UseBrowserBackendStatusResult {
  ext: BrowserExtStatus | null
  cdp: BrowserCdpStatus | null
}

/**
 * Poll the browser backend status (extension broker snapshot + CDP connection)
 * on a fixed interval. Both reads are cheap in-memory lookups — never the
 * pgrep-based doctor probe. Pauses when the tab is hidden. Mirrors the shape /
 * lifecycle of [`useServerStatus`].
 */
export function useBrowserBackendStatus(intervalMs: number = 5000): UseBrowserBackendStatusResult {
  const [ext, setExt] = useState<BrowserExtStatus | null>(null)
  const [cdp, setCdp] = useState<BrowserCdpStatus | null>(null)

  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setInterval> | null = null

    async function fetchOnce() {
      const [e, c] = await Promise.allSettled([
        getTransport().call<BrowserExtStatus>("browser_extension_status"),
        getTransport().call<BrowserCdpStatus>("browser_get_status"),
      ])
      if (cancelled) return
      if (e.status === "fulfilled") setExt(e.value)
      if (c.status === "fulfilled") setCdp(c.value)
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

  return { ext, cdp }
}
