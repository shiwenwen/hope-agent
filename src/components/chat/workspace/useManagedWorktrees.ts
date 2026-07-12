import { useCallback, useEffect, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import type { ManagedWorktree } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

export interface ManagedWorktreesState {
  worktrees: ManagedWorktree[]
  loading: boolean
  error: string | null
  refresh: () => void
}

const MANAGED_WORKTREE_EVENT_REFRESH_DEBOUNCE_MS = 250

function isManagedWorktreePayload(payload: unknown): payload is ManagedWorktree {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

export function useManagedWorktrees(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean } = {},
): ManagedWorktreesState {
  const { incognito = false, turnActive = false } = opts
  const [worktrees, setWorktrees] = useState<ManagedWorktree[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchWorktrees = useCallback(
    (fetchOpts: { clear?: boolean } = {}) => {
      if (!sessionId || incognito) {
        reqRef.current += 1
        setWorktrees([])
        setLoading(false)
        setError(null)
        return
      }
      const req = ++reqRef.current
      setLoading(true)
      setError(null)
      if (fetchOpts.clear) setWorktrees([])
      getTransport()
        .call<ManagedWorktree[]>("list_managed_worktrees", { sessionId })
        .then((next) => {
          if (reqRef.current !== req) return
          setWorktrees(Array.isArray(next) ? next : [])
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useManagedWorktrees", "Failed to load managed worktrees", e)
          setError(message)
          setLoading(false)
        })
    },
    [incognito, sessionId],
  )

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchWorktrees({ clear: true })
    })
    return () => {
      cancelled = true
    }
  }, [fetchWorktrees])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) fetchWorktrees()
      })
    }
    return () => {
      cancelled = true
    }
  }, [fetchWorktrees, turnActive])

  useEffect(() => {
    if (!sessionId || incognito) return
    const transport = getTransport()
    const scheduleRefresh = () => {
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchWorktrees()
      }, MANAGED_WORKTREE_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefresh = (payload: unknown) => {
      if (isManagedWorktreePayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const offCreated = transport.listen("worktree:created", maybeRefresh)
    const offUpdated = transport.listen("worktree:updated", maybeRefresh)
    const offArchived = transport.listen("worktree:archived", maybeRefresh)
    const offRestored = transport.listen("worktree:restored", maybeRefresh)
    const offHandoff = transport.listen("worktree:handoff", maybeRefresh)
    const offLagged = transport.listen("_lagged", scheduleRefresh)
    return () => {
      offCreated()
      offUpdated()
      offArchived()
      offRestored()
      offHandoff()
      offLagged()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [fetchWorktrees, incognito, sessionId])

  return { worktrees, loading, error, refresh: fetchWorktrees }
}
