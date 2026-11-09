import { useCallback, useEffect, useRef, useState } from "react"
import { getTransportRevision, useTransport, useTransportRevision } from "@/lib/transport-provider"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import type { SessionMeta } from "@/types/chat"

export interface AssistantRequest {
  id: string
  sourceTurnId: string
  sourceMessageId: number
  itemKey: string
  targetSessionId: string
  targetTitle?: string | null
  pendingInteractionCount?: number | null
  instruction: string
  status:
    | "queued"
    | "running"
    | "cancellation_requested"
    | "completed"
    | "interrupted"
    | "cancelled"
    | "failed"
  turnId?: string | null
  resultMessageId?: number | null
  resultPreview?: string | null
  error?: string | null
  createdAt: string
  updatedAt: string
}

export interface AssistantWatch {
  sessionId: string
  goalId: string
  objective: string
  state: string
  needsAttention: boolean
  reason?: "blocked" | "awaiting_acceptance" | null
  eventId?: number | null
}

export function requestNeedsResponse(request: AssistantRequest): boolean {
  return (request.status === "running" || request.status === "cancellation_requested") && (request.pendingInteractionCount ?? 0) > 0
}

const emptyData = { requests: [] as AssistantRequest[], watches: [] as AssistantWatch[] }

export function usePersonalAssistant() {
  const transport = useTransport()
  const revision = useTransportRevision()
  const [data, setData] = useState({ ...emptyData, revision, authorized: false })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const requestVersion = useRef(0)
  const enabled = useRef(true)
  const failed = useRef(false)
  const runningTargets = useRef(new Set<string>())

  const refresh = useCallback(async () => {
    const version = ++requestVersion.current
    enabled.current = true
    setLoading(true)
    const current = () => version === requestVersion.current && revision === getTransportRevision()
    try {
      const [requests, watches] = await Promise.all([
        transport.call<AssistantRequest[]>("list_personal_assistant_requests"),
        transport.call<AssistantWatch[]>("personal_assistant_inbox"),
      ])
      if (!current()) return
      failed.current = false
      runningTargets.current = new Set(
        requests
          .filter(
            (request) =>
              request.status === "running" || request.status === "cancellation_requested",
          )
          .map((request) => request.targetSessionId),
      )
      setData({ requests, watches, revision, authorized: true })
      setError(null)
    } catch (cause) {
      if (!current()) return
      failed.current = true
      runningTargets.current.clear()
      setData({ ...emptyData, revision, authorized: false })
      setError(String(cause))
    } finally {
      if (current()) setLoading(false)
    }
  }, [transport, revision])

  useEffect(() => {
    requestVersion.current++
    setData({ ...emptyData, revision, authorized: false })
    setError(null)
    setLoading(false)
    failed.current = false
    runningTargets.current.clear()
    let interactionTimer: ReturnType<typeof setTimeout> | undefined
    const onChange = () => {
      if (enabled.current && !failed.current && document.visibilityState === "visible")
        void refresh()
    }
    const onVerify = () => {
      if (enabled.current && document.visibilityState === "visible") void refresh()
    }
    const offChange = transport.listen("personal_assistant:changed", onChange)
    const offResync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, onVerify)
    const offInteraction = transport.listen("session_pending_interactions_changed", (payload) => {
      const sessionId = (payload as { sessionId?: string } | null)?.sessionId
      if (
        runningTargets.current.size === 0 ||
        (sessionId && !runningTargets.current.has(sessionId))
      )
        return
      clearTimeout(interactionTimer)
      interactionTimer = setTimeout(onChange, 200)
    })
    document.addEventListener("visibilitychange", onVerify)
    if (enabled.current) void refresh()
    return () => {
      // This is a request generation, deliberately advanced at teardown.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      requestVersion.current++
      offChange()
      offResync()
      offInteraction()
      clearTimeout(interactionTimer)
      document.removeEventListener("visibilitychange", onVerify)
    }
  }, [transport, revision, refresh])

  const open = useCallback(async () => {
    const home = await transport.call<SessionMeta>("ensure_personal_assistant_home")
    if (revision !== getTransportRevision()) return null
    if (home.kind !== "personal_assistant") throw new Error("Invalid personal assistant session")
    void refresh()
    return home
  }, [transport, revision, refresh])

  const watchGoal = useCallback(
    async (sessionId: string, goalId: string, watching: boolean) => {
      await transport.call(watching ? "unwatch_personal_goal" : "watch_personal_goal", {
        sessionId,
        goalId,
      })
      if (revision === getTransportRevision()) await refresh()
    },
    [transport, revision, refresh],
  )

  const cancel = useCallback(
    async (id: string) => {
      await transport.call<AssistantRequest>("cancel_personal_assistant_request", { id })
      if (revision === getTransportRevision()) await refresh()
    },
    [transport, revision, refresh],
  )

  return {
    ...(data.revision === revision ? data : { ...emptyData, authorized: false }),
    error,
    loading,
    refresh,
    open,
    watchGoal,
    cancel,
  }
}

export type PersonalAssistantState = ReturnType<typeof usePersonalAssistant>
