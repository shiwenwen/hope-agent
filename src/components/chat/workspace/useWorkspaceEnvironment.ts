import { useCallback, useEffect, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { WorkspaceEnvironmentSnapshot } from "@/lib/transport"

export interface WorkspaceEnvironmentState {
  snapshot: WorkspaceEnvironmentSnapshot | null
  loading: boolean
  error: string | null
}

/**
 * Read-only session environment for the workspace panel. Fetches on mount /
 * session switch / workspace scope change and refreshes when a turn finishes
 * so git status reflects files the agent just touched.
 */
export function useWorkspaceEnvironment(
  sessionId: string | null | undefined,
  opts: { turnActive?: boolean; refreshKey?: string | null } = {},
): WorkspaceEnvironmentState {
  const { turnActive = false, refreshKey = "" } = opts
  const [state, setState] = useState<WorkspaceEnvironmentState>({
    snapshot: null,
    loading: false,
    error: null,
  })
  const reqRef = useRef(0)

  const fetchInto = useCallback((sid: string, fetchOpts: { clear?: boolean } = {}) => {
    const req = ++reqRef.current
    setState((prev) => ({
      snapshot: fetchOpts.clear ? null : prev.snapshot,
      loading: true,
      error: null,
    }))
    getTransport()
      .loadSessionEnvironment(sid)
      .then((snapshot) => {
        if (reqRef.current !== req) return
        setState({ snapshot, loading: false, error: null })
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useWorkspaceEnvironment", "Failed to load session environment", e)
        setState((prev) => ({ ...prev, loading: false, error: message }))
      })
  }, [])

  useEffect(() => {
    let cancelled = false
    if (!sessionId) {
      reqRef.current += 1
      queueMicrotask(() => {
        if (!cancelled) {
          setState({ snapshot: null, loading: false, error: null })
        }
      })
      return () => {
        cancelled = true
      }
    }
    queueMicrotask(() => {
      if (!cancelled) {
        fetchInto(sessionId, { clear: true })
      }
    })
    return () => {
      cancelled = true
    }
  }, [sessionId, refreshKey, fetchInto])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive && sessionId) {
      queueMicrotask(() => {
        if (!cancelled) {
          fetchInto(sessionId)
        }
      })
    }
    return () => {
      cancelled = true
    }
  }, [turnActive, sessionId, fetchInto])

  return state
}
