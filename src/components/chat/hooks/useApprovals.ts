import { useState, useEffect, useMemo } from "react"
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

export function useApprovals(currentSessionId: string | null): UseApprovalsReturn {
  const [allApprovalRequests, setAllApprovalRequests] = useState<ApprovalRequest[]>([])
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
        setAllApprovalRequests((prev) => [...prev, req])
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
        if (!payload) return
        const requestId = payload.request_id ?? payload.requestId
        if (!requestId) return
        setAllApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval timeout", e)
      }
    })
  }, [])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    setAllApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    try {
      await getTransport().call("respond_to_approval", { requestId, response })
    } catch (e) {
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
    }
  }

  return { approvalRequests, handleApprovalResponse }
}
