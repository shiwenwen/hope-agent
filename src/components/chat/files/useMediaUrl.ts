import { useEffect, useState } from "react"

import { useTransport } from "@/lib/transport-provider"
import type { MediaItem } from "@/types/chat"

/**
 * Resolve an inline media URL without exposing a remote Owner Token. Cookie-
 * authenticated web and local Tauri sources resolve synchronously; an
 * explicit remote Bearer transport falls back to a revocable Blob URL.
 */
export function useMediaUrl(item: MediaItem, preferredUrl?: string | null): string | null {
  const transport = useTransport()
  const direct = preferredUrl || transport.resolveMediaUrl(item)
  const { url, localPath, name, mimeType, sizeBytes, kind } = item
  const itemKey = `${url}\u0000${localPath ?? ""}\u0000${sizeBytes}`
  const [loaded, setLoaded] = useState<{ key: string; url: string } | null>(null)

  useEffect(() => {
    if (direct) return

    let cancelled = false
    let release: (() => void) | undefined
    void transport
      .loadMediaUrl({ url, localPath, name, mimeType, sizeBytes, kind })
      .then((lease) => {
        if (cancelled) {
          lease.release()
          return
        }
        release = lease.release
        setLoaded({ key: itemKey, url: lease.url })
      })
      .catch(() => undefined)

    return () => {
      cancelled = true
      release?.()
    }
  }, [direct, itemKey, kind, localPath, mimeType, name, sizeBytes, transport, url])

  return direct || (loaded?.key === itemKey ? loaded.url : null)
}
