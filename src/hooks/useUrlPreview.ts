import { useEffect, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { extractUrls } from "@/lib/urlDetect"
import type { UrlPreviewData } from "@/components/chat/UrlPreviewCard"

const MAX_CONCURRENT = 3

interface UseUrlPreviewOptions {
  enabled?: boolean
  debounceMs?: number
}

/**
 * Hook for managing URL preview fetching with debounce and caching.
 * Used in ChatInput for real-time preview as user types.
 */
export function useUrlPreview(text: string, options: UseUrlPreviewOptions = {}) {
  const { enabled = true, debounceMs = 500 } = options
  const [previews, setPreviews] = useState<Map<string, UrlPreviewData | null>>(new Map())
  const [dismissedUrls, setDismissedUrls] = useState<Set<string>>(new Set())

  // Persistent cache across re-renders (not cleared on text change)
  const cacheRef = useRef<Map<string, UrlPreviewData>>(new Map())
  const pendingRef = useRef<Set<string>>(new Set())
  const activeRef = useRef(0)

  const dismiss = (url: string) => {
    setDismissedUrls((prev) => new Set(prev).add(url))
  }

  const reset = () => {
    setPreviews(new Map())
    setDismissedUrls(new Set())
  }

  useEffect(() => {
    const timer = setTimeout(() => {
      const urls = enabled ? extractUrls(text) : []

      if (!enabled || !text.trim() || urls.length === 0) {
        setPreviews(new Map())
        return
      }

      // Build initial preview map from cache
      const newPreviews = new Map<string, UrlPreviewData | null>()
      const toFetch: string[] = []

      for (const url of urls) {
        if (cacheRef.current.has(url)) {
          newPreviews.set(url, cacheRef.current.get(url)!)
        } else if (!pendingRef.current.has(url)) {
          newPreviews.set(url, null) // loading
          toFetch.push(url)
        } else {
          newPreviews.set(url, null) // still loading
        }
      }

      setPreviews(newPreviews)

      // Fetch new URLs with concurrency limit
      const queue = [...toFetch]
      const startNext = () => {
        if (queue.length === 0 || activeRef.current >= MAX_CONCURRENT) return
        const url = queue.shift()
        if (!url) return

        pendingRef.current.add(url)
        activeRef.current++

        getTransport()
          .call<UrlPreviewData>("fetch_url_preview", { url })
          .then((meta) => {
            cacheRef.current.set(url, meta)
            setPreviews((prev) => {
              const next = new Map(prev)
              if (next.has(url)) next.set(url, meta)
              return next
            })
          })
          .catch(() => {
            // Silently skip failed previews
            setPreviews((prev) => {
              const next = new Map(prev)
              next.delete(url)
              return next
            })
          })
          .finally(() => {
            pendingRef.current.delete(url)
            activeRef.current--
            startNext()
          })

        startNext()
      }

      startNext()
    }, debounceMs)

    return () => clearTimeout(timer)
  }, [text, enabled, debounceMs])

  return { previews, dismissedUrls, dismiss, reset }
}

/**
 * Hook for fetching URL previews for a static message (no debounce).
 * Used in MessageBubble for displaying previews on already-sent messages.
 */
export function useMessageUrlPreviews(content: string, enabled: boolean) {
  const [previews, setPreviews] = useState<UrlPreviewData[]>([])
  const fetchedRef = useRef(false)

  useEffect(() => {
    if (!enabled || fetchedRef.current || !content.trim()) return

    const urls = extractUrls(content)
    if (urls.length === 0) return

    fetchedRef.current = true

    // Limit to 5 URLs per message
    const urlsToFetch = urls.slice(0, 5)

    getTransport()
      .call<UrlPreviewData[]>("fetch_url_previews", { urls: urlsToFetch })
      .then((results) => {
        setPreviews(results)
      })
      .catch(() => {
        // Silently ignore
      })
  }, [content, enabled])

  return previews
}
