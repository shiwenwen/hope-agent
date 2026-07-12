import { useCallback, useEffect, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import type { LspDiagnosticsSnapshot, LspStatusSnapshot } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

export interface LspDiagnosticsState {
  status: LspStatusSnapshot | null
  snapshot: LspDiagnosticsSnapshot | null
  loading: boolean
  error: string | null
  refresh: () => void
}

const LSP_EVENT_REFRESH_DEBOUNCE_MS = 250

export function useLspDiagnostics(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean } = {},
): LspDiagnosticsState {
  const { incognito = false, turnActive = false } = opts
  const [status, setStatus] = useState<LspStatusSnapshot | null>(null)
  const [snapshot, setSnapshot] = useState<LspDiagnosticsSnapshot | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchLsp = useCallback(() => {
    if (!sessionId || incognito) {
      reqRef.current += 1
      setStatus(null)
      setSnapshot(null)
      setLoading(false)
      setError(null)
      return
    }
    const req = ++reqRef.current
    setLoading(true)
    setError(null)
    const transport = getTransport()
    Promise.all([
      transport.call<LspStatusSnapshot>("get_lsp_status", { sessionId }),
      transport.call<LspDiagnosticsSnapshot>("get_lsp_diagnostics", { sessionId }),
    ])
      .then(([nextStatus, nextSnapshot]) => {
        if (reqRef.current !== req) return
        setStatus(nextStatus)
        setSnapshot(nextSnapshot)
        setLoading(false)
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useLspDiagnostics", "Failed to load LSP diagnostics", e)
        setError(message)
        setLoading(false)
      })
  }, [incognito, sessionId])

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchLsp()
    })
    return () => {
      cancelled = true
    }
  }, [fetchLsp])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) fetchLsp()
      })
    }
    return () => {
      cancelled = true
    }
  }, [fetchLsp, turnActive])

  useEffect(() => {
    if (!sessionId || incognito) return
    const scheduleRefresh = () => {
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchLsp()
      }, LSP_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const transport = getTransport()
    const offDiagnostics = transport.listen("lsp:diagnostics", scheduleRefresh)
    const offLagged = transport.listen("_lagged", scheduleRefresh)
    return () => {
      offDiagnostics()
      offLagged()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [fetchLsp, incognito, sessionId])

  return { status, snapshot, loading, error, refresh: fetchLsp }
}
