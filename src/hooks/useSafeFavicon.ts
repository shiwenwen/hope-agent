import { useEffect, useState } from "react"
import { faviconPageUrlForHref, type SafeFaviconData } from "@/lib/favicon"
import { getTransport } from "@/lib/transport-provider"

const MAX_CONCURRENT_REQUESTS = 4
const cache = new Map<string, string | null>()
const inFlight = new Map<string, Promise<string | null>>()
const queue: Array<() => void> = []
let activeRequests = 0

export interface SafeFaviconBudget {
  seen: Set<string>
}

interface UseSafeFaviconOptions {
  enabled?: boolean
  budget?: SafeFaviconBudget | null
  maxRequests?: number
}

function pumpQueue() {
  while (activeRequests < MAX_CONCURRENT_REQUESTS) {
    const next = queue.shift()
    if (!next) return
    activeRequests += 1
    next()
  }
}

function enqueueSafeFaviconRequest(pageUrl: string): Promise<string | null> {
  return new Promise((resolve) => {
    queue.push(() => {
      getTransport()
        .call<SafeFaviconData | null>("fetch_url_favicon", { url: pageUrl })
        .then((data) => data?.dataUrl ?? null)
        .catch(() => null)
        .then((dataUrl) => {
          cache.set(pageUrl, dataUrl)
          resolve(dataUrl)
        })
        .finally(() => {
          activeRequests = Math.max(0, activeRequests - 1)
          inFlight.delete(pageUrl)
          pumpQueue()
        })
    })
    pumpQueue()
  })
}

function reserveBudget(
  pageUrl: string,
  budget: SafeFaviconBudget | null | undefined,
  maxRequests?: number,
): boolean {
  if (!budget || maxRequests == null) return true
  if (budget.seen.has(pageUrl)) return true
  if (budget.seen.size >= maxRequests) return false
  budget.seen.add(pageUrl)
  return true
}

function loadSafeFavicon(
  pageUrl: string,
  budget?: SafeFaviconBudget | null,
  maxRequests?: number,
): Promise<string | null> {
  const cached = cache.get(pageUrl)
  if (cached !== undefined) return Promise.resolve(cached)

  const pending = inFlight.get(pageUrl)
  if (pending) return pending

  if (!reserveBudget(pageUrl, budget, maxRequests)) return Promise.resolve(null)

  const request = enqueueSafeFaviconRequest(pageUrl)
  inFlight.set(pageUrl, request)
  return request
}

export function useSafeFavicon(
  href: string | undefined,
  options: UseSafeFaviconOptions = {},
): string | null {
  const { enabled = true, budget, maxRequests } = options
  const pageUrl = enabled ? faviconPageUrlForHref(href) : null
  const [loaded, setLoaded] = useState<{ pageUrl: string; dataUrl: string | null } | null>(null)

  useEffect(() => {
    let cancelled = false
    if (!pageUrl || cache.has(pageUrl)) return

    void loadSafeFavicon(pageUrl, budget, maxRequests).then((next) => {
      if (!cancelled) setLoaded({ pageUrl, dataUrl: next })
    })

    return () => {
      cancelled = true
    }
  }, [pageUrl, budget, maxRequests])

  if (!pageUrl) return null
  if (cache.has(pageUrl)) return cache.get(pageUrl) ?? null
  return loaded?.pageUrl === pageUrl ? loaded.dataUrl : null
}
