import { useState, useEffect, useMemo, useRef } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
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

export function useApprovals(currentSessionId: string | null): UseApprovalsReturn {
  const { t } = useTranslation()
  const [allApprovalRequests, setAllApprovalRequests] = useState<ApprovalRequest[]>([])
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
        setAllApprovalRequests((prev) => [...prev, parsePayload<ApprovalRequest>(raw)])
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
      }
    })
  }, [])

  // Backend-enforced timeouts remove the pending request from the global
  // registry; mirror that lifecycle locally so stale modals disappear.
  useEffect(() => {
    return getTransport().listen("approval_timed_out", (raw) => {
      try {
        const payload = parsePayload<{ request_id?: string; requestId?: string }>(raw)
        const requestId = payload.request_id ?? payload.requestId
        if (!requestId) return
        setAllApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval timeout", e)
      }
    })
  }, [])

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
        const requestId = payload.requestId ?? payload.request_id
        if (!requestId) return
        const wasOurOwn = locallyResolvedRef.current.delete(requestId)
        let wasPresent = false
        setAllApprovalRequests((prev) => {
          wasPresent = prev.some((r) => r.request_id === requestId)
          return prev.filter((r) => r.request_id !== requestId)
        })
        if (!wasOurOwn && wasPresent && ELSEWHERE_TOAST_SOURCES.has(payload.source ?? "")) {
          toast.info(tRef.current("approval.resolvedElsewhere"))
        }
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval resolved", e)
      }
    })
  }, [])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    // Mark as ours BEFORE the round-trip so the echoed approval:resolved doesn't
    // toast "handled elsewhere" for our own action.
    locallyResolvedRef.current.add(requestId)
    setAllApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    try {
      await getTransport().call("respond_to_approval", { requestId, response })
    } catch (e) {
      locallyResolvedRef.current.delete(requestId)
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
    }
  }

  return { approvalRequests, handleApprovalResponse }
}
