import { useCallback, useEffect, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import type { SessionGitControlSnapshot } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

export interface SessionGitControlState {
  snapshot: SessionGitControlSnapshot | null
  loading: boolean
  error: string | null
  progress?: { operation: string; stage: string; message?: string | null } | null
  refresh: () => void
}

export function useSessionGitControl(
  sessionId: string | null | undefined,
  turnActive = false,
): SessionGitControlState {
  const [state, setState] = useState<Omit<SessionGitControlState, "refresh">>({
    snapshot: null,
    loading: false,
    error: null,
    progress: null,
  })
  const requestRef = useRef(0)

  const refresh = useCallback(() => {
    if (!sessionId) return
    const request = ++requestRef.current
    setState((previous) => ({ ...previous, loading: true, error: null }))
    getTransport()
      .call<SessionGitControlSnapshot>("load_session_git_control_cmd", { sessionId })
      .then((snapshot) => {
        if (requestRef.current === request) {
          setState((previous) => ({ ...previous, snapshot, loading: false, error: null }))
        }
      })
      .catch((error) => {
        if (requestRef.current !== request) return
        const message = error instanceof Error ? error.message : String(error)
        logger.error("ui", "useSessionGitControl", "Failed to load Git state", error)
        setState((previous) => ({ ...previous, snapshot: null, loading: false, error: message }))
      })
  }, [sessionId])

  useEffect(() => {
    if (!sessionId) {
      requestRef.current += 1
      queueMicrotask(() => setState({ snapshot: null, loading: false, error: null, progress: null }))
      return
    }
    queueMicrotask(refresh)
  }, [refresh, sessionId])

  useEffect(() => {
    if (!sessionId) return
    const transport = getTransport()
    const off = transport.listen("session:git_changed", (payload) => {
      const detail = payload as { sessionId?: string }
      if (detail?.sessionId === sessionId) refresh()
    })
    const offProgress = transport.listen("session:git_progress", (payload) => {
      const detail = payload as {
        sessionId?: string
        operation?: string
        status?: string
        stage?: string
        message?: string | null
      }
      if (detail?.sessionId !== sessionId || !detail.operation || !detail.stage) return
      if (detail.status && detail.status !== "running") {
        setState((previous) => ({ ...previous, progress: null }))
        return
      }
      setState((previous) => ({
        ...previous,
        progress: {
          operation: detail.operation as string,
          stage: detail.stage as string,
          message: detail.message,
        },
      }))
    })
    const offCompleted = transport.listen("session:git_completed", (payload) => {
      const detail = payload as { sessionId?: string }
      if (detail?.sessionId === sessionId) {
        setState((previous) => ({ ...previous, progress: null }))
        refresh()
      }
    })
    return () => {
      off()
      offProgress()
      offCompleted()
    }
  }, [refresh, sessionId])

  const previousTurnActive = useRef(turnActive)
  useEffect(() => {
    const wasActive = previousTurnActive.current
    previousTurnActive.current = turnActive
    if (wasActive && !turnActive) queueMicrotask(refresh)
  }, [refresh, turnActive])

  return { ...state, refresh }
}
