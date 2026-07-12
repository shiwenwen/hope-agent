import { useState, useEffect, useMemo, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload, TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { logger } from "@/lib/logger"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"

export interface UseApprovalsReturn {
  approvalRequests: ApprovalRequest[]
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
}

/** Resolution sources that represent another interactive surface handling the
 * prompt — these warrant a "handled elsewhere" toast. Timeout / session-delete /
 * eviction resolutions dismiss silently (they aren't "another person answered"). */
const ELSEWHERE_TOAST_SOURCES = new Set(["gui", "http", "im"])

function isExpired(request: ApprovalRequest, now = Date.now()): boolean {
  return typeof request.local_timeout_at_ms === "number" && request.local_timeout_at_ms <= now
}

function withLocalDeadline(request: ApprovalRequest, existing?: ApprovalRequest): ApprovalRequest {
  if (typeof request.timeout_at_ms !== "number") {
    return { ...request, local_timeout_at_ms: null }
  }
  if (
    existing?.timeout_at_ms === request.timeout_at_ms &&
    typeof existing.local_timeout_at_ms === "number"
  ) {
    return { ...request, local_timeout_at_ms: existing.local_timeout_at_ms }
  }
  const serverNow = request.server_now_ms ?? Date.now()
  const remainingMs = Math.max(0, request.timeout_at_ms - serverNow)
  return { ...request, local_timeout_at_ms: Date.now() + remainingMs }
}

function normalizeRequests(requests: ApprovalRequest[]): ApprovalRequest[] {
  const byId = new Map<string, ApprovalRequest>()
  for (const request of requests) {
    if (!request?.request_id) continue
    const hydrated = withLocalDeadline(request, byId.get(request.request_id))
    if (isExpired(hydrated)) continue
    byId.set(request.request_id, hydrated)
  }
  return [...byId.values()].sort((a, b) => {
    const createdDiff = (a.created_at_ms ?? 0) - (b.created_at_ms ?? 0)
    return createdDiff || a.request_id.localeCompare(b.request_id)
  })
}

export function useApprovals(currentSessionId: string | null): UseApprovalsReturn {
  const { t } = useTranslation()
  const [allApprovalRequests, setAllApprovalRequests] = useState<ApprovalRequest[]>([])
  const allApprovalRequestsRef = useRef<ApprovalRequest[]>([])
  const mutationVersionRef = useRef(0)
  const reconcileSequenceRef = useRef(0)
  const appliedReconcileRef = useRef(0)
  // `approval_required` events that arrived after a recovery request began.
  // A snapshot may have been read just before such an event was registered, so
  // those requests must be merged into (not overwritten by) the response.
  const pendingEventVersionByIdRef = useRef<Map<string, number>>(new Map())
  // Terminal events can race an older in-flight snapshot. Request ids are
  // globally unique, so a bounded tombstone prevents resolved dialogs reviving.
  const terminalApprovalIdsRef = useRef<Set<string>>(new Set())
  // Request ids THIS surface just resolved (via handleApprovalResponse). Lets the
  // approval:resolved listener tell our own action apart from a resolution that
  // happened on another surface, without needing to know our own surface id.
  const locallyResolvedRef = useRef<Set<string>>(new Set())
  // Keep the latest `t` in a ref so the long-lived approval:resolved listener can
  // subscribe once ([] deps) instead of tearing down + re-subscribing on every
  // language change — that gap could drop an approval:resolved event and leave a
  // dialog stuck open.
  const tRef = useRef(t)
  useEffect(() => {
    tRef.current = t
  }, [t])

  const commitRequests = useCallback(
    (update: (previous: ApprovalRequest[]) => ApprovalRequest[]) => {
      const next = update(allApprovalRequestsRef.current)
      allApprovalRequestsRef.current = next
      setAllApprovalRequests(next)
    },
    [],
  )

  const rememberTerminalApproval = useCallback((requestId: string) => {
    pendingEventVersionByIdRef.current.delete(requestId)
    const terminal = terminalApprovalIdsRef.current
    terminal.add(requestId)
    while (terminal.size > 256) {
      const oldest = terminal.values().next().value
      if (typeof oldest !== "string") break
      terminal.delete(oldest)
    }
  }, [])

  const recordPendingEvent = useCallback(
    (request: ApprovalRequest) => {
      if (terminalApprovalIdsRef.current.has(request.request_id)) return
      const version = ++mutationVersionRef.current
      pendingEventVersionByIdRef.current.set(request.request_id, version)
      commitRequests((previous) => normalizeRequests([...previous, request]))
    },
    [commitRequests],
  )

  const mutateRequests = useCallback(
    (update: (previous: ApprovalRequest[]) => ApprovalRequest[]) => {
      mutationVersionRef.current += 1
      commitRequests(update)
    },
    [commitRequests],
  )

  const reconcilePendingApprovals = useCallback(async () => {
    const mutationVersion = mutationVersionRef.current
    const sequence = ++reconcileSequenceRef.current
    try {
      const snapshot = await getTransport().call<ApprovalRequest[]>("list_pending_approvals")
      if (sequence < appliedReconcileRef.current) return
      appliedReconcileRef.current = sequence
      const terminal = terminalApprovalIdsRef.current
      const normalizedSnapshot = normalizeRequests(Array.isArray(snapshot) ? snapshot : []).filter(
        (request) => !terminal.has(request.request_id),
      )
      const snapshotIds = new Set(normalizedSnapshot.map((request) => request.request_id))
      const concurrentEvents = allApprovalRequestsRef.current.filter((request) => {
        const eventVersion = pendingEventVersionByIdRef.current.get(request.request_id) ?? 0
        return eventVersion > mutationVersion && !terminal.has(request.request_id)
      })
      const concurrentIds = new Set(concurrentEvents.map((request) => request.request_id))
      for (const [requestId, eventVersion] of pendingEventVersionByIdRef.current) {
        if (
          terminal.has(requestId) ||
          snapshotIds.has(requestId) ||
          (eventVersion <= mutationVersion && !concurrentIds.has(requestId))
        ) {
          pendingEventVersionByIdRef.current.delete(requestId)
        }
      }
      commitRequests(() => normalizeRequests([...normalizedSnapshot, ...concurrentEvents]))
    } catch (e) {
      // Keep event-derived state on transient failures. A later reconnect/focus
      // retries, and local deadline guards still prevent expired approvals.
      logger.error("ui", "ChatScreen::approval", "Failed to reconcile approvals", e)
    }
  }, [commitRequests])
  const approvalRequests = useMemo(
    () =>
      allApprovalRequests.filter((request) => {
        if (!request.session_id) return true
        return request.session_id === currentSessionId
      }),
    [allApprovalRequests, currentSessionId],
  )

  // Listen for command approval events
  useEffect(() => {
    return getTransport().listen("approval_required", (raw) => {
      try {
        const req = parsePayload<ApprovalRequest>(raw)
        if (!req) return
        recordPendingEvent(req)
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
      }
    })
  }, [recordPendingEvent])

  // Backend-enforced timeouts remove the pending request from the global
  // registry; mirror that lifecycle locally so stale modals disappear.
  useEffect(() => {
    return getTransport().listen("approval_timed_out", (raw) => {
      try {
        const payload = parsePayload<{ request_id?: string; requestId?: string }>(raw)
        if (!payload) return
        const requestId = payload.request_id ?? payload.requestId
        if (!requestId) return
        rememberTerminalApproval(requestId)
        mutateRequests((prev) => prev.filter((r) => r.request_id !== requestId))
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval timeout", e)
      }
    })
  }, [mutateRequests, rememberTerminalApproval])

  // G6 (SURFACE-1): an approval resolved on ANY surface broadcasts
  // `approval:resolved`. Dismiss the matching dialog everywhere; if it was
  // resolved by another interactive surface (not our own click), surface a
  // toast so the user understands why the prompt vanished.
  useEffect(() => {
    return getTransport().listen("approval:resolved", (raw) => {
      try {
        const payload = parsePayload<{
          requestId?: string
          request_id?: string
          source?: string
        }>(raw)
        if (!payload) return
        const requestId = payload.requestId ?? payload.request_id
        if (!requestId) return
        const wasOurOwn = locallyResolvedRef.current.delete(requestId)
        const wasPresent = allApprovalRequestsRef.current.some((r) => r.request_id === requestId)
        rememberTerminalApproval(requestId)
        mutateRequests((prev) => prev.filter((r) => r.request_id !== requestId))
        if (!wasOurOwn && wasPresent && ELSEWHERE_TOAST_SOURCES.has(payload.source ?? "")) {
          toast.info(tRef.current("approval.resolvedElsewhere"))
        }
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval resolved", e)
      }
    })
  }, [mutateRequests, rememberTerminalApproval])

  // Events are the fast path, but they are not durable. Reconcile on mount and
  // every signal that indicates the renderer/transport may have missed events.
  useEffect(() => {
    void reconcilePendingApprovals()
    const offResync = getTransport().listen(
      TRANSPORT_EVENT_RESYNC_REQUIRED,
      reconcilePendingApprovals,
    )
    const onFocus = () => void reconcilePendingApprovals()
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") void reconcilePendingApprovals()
    }
    window.addEventListener("focus", onFocus)
    document.addEventListener("visibilitychange", onVisibilityChange)
    return () => {
      offResync()
      window.removeEventListener("focus", onFocus)
      document.removeEventListener("visibilitychange", onVisibilityChange)
    }
  }, [reconcilePendingApprovals])

  // The backend timer remains authoritative, but a lost timeout event must not
  // leave enabled buttons on an expired request. Remove at the nearest absolute
  // deadline, then reconcile to repair any clock/race discrepancy.
  useEffect(() => {
    const deadlines = allApprovalRequests
      .map((request) => request.local_timeout_at_ms)
      .filter((deadline): deadline is number => typeof deadline === "number")
    if (deadlines.length === 0) return
    const nearest = Math.min(...deadlines)
    const delay = Math.max(0, Math.min(nearest - Date.now() + 25, 2_147_483_647))
    const timer = window.setTimeout(() => {
      mutateRequests((prev) => prev.filter((request) => !isExpired(request)))
      void reconcilePendingApprovals()
    }, delay)
    return () => window.clearTimeout(timer)
  }, [allApprovalRequests, mutateRequests, reconcilePendingApprovals])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    // Mark as ours BEFORE the round-trip so the echoed approval:resolved doesn't
    // toast "handled elsewhere" for our own action.
    locallyResolvedRef.current.add(requestId)
    try {
      await getTransport().call("respond_to_approval", { requestId, response })
      locallyResolvedRef.current.delete(requestId)
      // Success must close locally even if this surface missed the echoed event.
      rememberTerminalApproval(requestId)
      mutateRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    } catch (e) {
      locallyResolvedRef.current.delete(requestId)
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
      toast.error(`${tRef.current("common.error")}: ${String(e)}`)
      // The request may have been accepted while its HTTP response was lost, or
      // it may still be pending. Only an authoritative snapshot can distinguish.
      await reconcilePendingApprovals()
    }
  }

  return { approvalRequests, handleApprovalResponse }
}
