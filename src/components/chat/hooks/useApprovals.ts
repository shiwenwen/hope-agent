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
        setAllApprovalRequests((prev) => [...prev, parsePayload<ApprovalRequest>(raw)])
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
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
