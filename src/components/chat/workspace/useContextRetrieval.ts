import { useCallback, useEffect, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { ContextRetrievalSnapshot } from "@/lib/transport"

export interface ContextRetrievalState {
  snapshot: ContextRetrievalSnapshot | null
  loading: boolean
  error: string | null
  refresh: () => void
}

const CONTEXT_RETRIEVAL_DEBOUNCE_MS = 220
const CONTEXT_RETRIEVAL_EVENT_REFRESH_DEBOUNCE_MS = 300

function payloadBelongsToSession(payload: unknown, sessionId: string): boolean {
  if (typeof payload !== "object" || payload === null) return true
  const value = (payload as { sessionId?: unknown }).sessionId
  return typeof value !== "string" || value === sessionId
}

export function useContextRetrieval(
  sessionId: string | null | undefined,
  opts: {
    incognito?: boolean
    turnActive?: boolean
    disabled?: boolean
    query?: string
    limit?: number
    domain?: string | null
    templateId?: string | null
  } = {},
): ContextRetrievalState {
  const {
    incognito = false,
    turnActive = false,
    disabled = false,
    query = "",
    limit = 24,
    domain = null,
    templateId = null,
  } = opts
  const [snapshot, setSnapshot] = useState<ContextRetrievalSnapshot | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const debounceTimerRef = useRef<number | null>(null)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchSnapshot = useCallback(() => {
    if (disabled || !sessionId || incognito) {
      reqRef.current += 1
      setSnapshot(null)
      setLoading(false)
      setError(null)
      return
    }
    const req = ++reqRef.current
    setLoading(true)
    setError(null)
    getTransport()
      .call<ContextRetrievalSnapshot>("get_context_retrieval", {
        sessionId,
        query: query.trim() || null,
        limit,
        domain,
        templateId,
      })
      .then((next) => {
        if (reqRef.current !== req) return
        setSnapshot(next)
        setLoading(false)
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useContextRetrieval", "Failed to load context retrieval", e)
        setError(message)
        setLoading(false)
      })
  }, [disabled, domain, incognito, limit, query, sessionId, templateId])

  useEffect(() => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current)
      debounceTimerRef.current = null
    }
    debounceTimerRef.current = window.setTimeout(() => {
      debounceTimerRef.current = null
      fetchSnapshot()
    }, CONTEXT_RETRIEVAL_DEBOUNCE_MS)
    return () => {
      if (debounceTimerRef.current !== null) {
        window.clearTimeout(debounceTimerRef.current)
        debounceTimerRef.current = null
      }
    }
  }, [fetchSnapshot])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) fetchSnapshot()
      })
    }
    return () => {
      cancelled = true
    }
  }, [fetchSnapshot, turnActive])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const transport = getTransport()
    const scheduleRefresh = (payload?: unknown) => {
      if (payload !== undefined && !payloadBelongsToSession(payload, sessionId)) return
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchSnapshot()
      }, CONTEXT_RETRIEVAL_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const unsubs = [
      transport.listen("lsp:diagnostics", scheduleRefresh),
      transport.listen("review:created", scheduleRefresh),
      transport.listen("review:updated", scheduleRefresh),
      transport.listen("review:finding_updated", scheduleRefresh),
      transport.listen("verification:created", scheduleRefresh),
      transport.listen("verification:updated", scheduleRefresh),
      transport.listen("verification:step_updated", scheduleRefresh),
      transport.listen("workflow:updated", scheduleRefresh),
      transport.listen("_lagged", () => scheduleRefresh()),
    ]
    return () => {
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
      unsubs.forEach((unsub) => unsub())
    }
  }, [disabled, fetchSnapshot, incognito, sessionId])

  return { snapshot, loading, error, refresh: fetchSnapshot }
}
