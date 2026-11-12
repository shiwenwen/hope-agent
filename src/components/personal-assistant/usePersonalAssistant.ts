import { useCallback, useEffect, useRef, useState } from "react"
import { getTransportRevision, useTransport, useTransportRevision } from "@/lib/transport-provider"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import type { SessionMeta } from "@/types/chat"

export interface AssistantProfile {
  name: string
  avatar: string | null
}

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
  return (
    (request.status === "running" || request.status === "cancellation_requested") &&
    (request.pendingInteractionCount ?? 0) > 0
  )
}

const emptyData = {
  requests: [] as AssistantRequest[],
  watches: [] as AssistantWatch[],
  profile: null as AssistantProfile | null,
}

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
  const watchedGoals = useRef<Map<string, string> | null>(null)

  const refresh = useCallback(async () => {
    const version = ++requestVersion.current
    enabled.current = true
    setLoading(true)
    const current = () => version === requestVersion.current && revision === getTransportRevision()
    try {
      const [requests, watches, profile] = await Promise.all([
        transport.call<AssistantRequest[]>("list_personal_assistant_requests"),
        transport.call<AssistantWatch[]>("personal_assistant_inbox"),
        transport.call<AssistantProfile>("get_personal_assistant_profile"),
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
      watchedGoals.current = new Map(watches.map((watch) => [watch.goalId, watch.sessionId]))
      setData({ requests, watches, profile, revision, authorized: true })
      setError(null)
    } catch (cause) {
      if (!current()) return
      failed.current = true
      runningTargets.current.clear()
      watchedGoals.current = null
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
    watchedGoals.current = null
    let eventTimer: ReturnType<typeof setTimeout> | undefined
    const onChange = () => {
      if (enabled.current && !failed.current && document.visibilityState === "visible")
        void refresh()
    }
    const onVerify = () => {
      if (enabled.current && document.visibilityState === "visible") void refresh()
    }
    const scheduleRefresh = () => {
      if (eventTimer !== undefined) return
      eventTimer = setTimeout(() => {
        eventTimer = undefined
        onChange()
      }, 200)
    }
    const onGoalChange = (payload: unknown) => {
      // Goal mutations update the inbox projection in SQL. Their own events
      // invalidate it even when no assistant request is currently running.
      const goal = payload as { id?: unknown; goalId?: unknown; sessionId?: unknown } | null
      const goalId = goal?.goalId ?? goal?.id
      const watched = watchedGoals.current
      if (
        watched &&
        !(typeof goalId === "string" && watched.has(goalId)) &&
        !(typeof goal?.sessionId === "string" && [...watched.values()].includes(goal.sessionId))
      )
        return
      scheduleRefresh()
    }
    const offChange = transport.listen("personal_assistant:changed", onChange)
    const offResync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, onVerify)
    const offGoalUpdated = transport.listen("goal:updated", onGoalChange)
    const offGoalEvent = transport.listen("goal:event", onGoalChange)
    const offGoalCreated = transport.listen("goal:created", onGoalChange)
    const offInteraction = transport.listen("session_pending_interactions_changed", (payload) => {
      const sessionId = (payload as { sessionId?: string } | null)?.sessionId
      if (
        runningTargets.current.size === 0 ||
        (sessionId && !runningTargets.current.has(sessionId))
      )
        return
      scheduleRefresh()
    })
    document.addEventListener("visibilitychange", onVerify)
    if (enabled.current) void refresh()
    return () => {
      // This is a request generation, deliberately advanced at teardown.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      requestVersion.current++
      offChange()
      offResync()
      offGoalUpdated()
      offGoalEvent()
      offGoalCreated()
      offInteraction()
      clearTimeout(eventTimer)
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

  const saveProfile = useCallback(
    async (profile: AssistantProfile) => {
      if (revision !== getTransportRevision()) throw new Error("Assistant connection changed")
      const saved = await transport.call<AssistantProfile>("update_personal_assistant_profile", {
        profile,
      })
      if (revision !== getTransportRevision()) return null
      await refresh()
      return saved
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
    saveProfile,
  }
}

export type PersonalAssistantState = ReturnType<typeof usePersonalAssistant>
