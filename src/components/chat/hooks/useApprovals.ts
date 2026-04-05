import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"

export interface UseApprovalsReturn {
  approvalRequests: ApprovalRequest[]
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
}

export function useApprovals(): UseApprovalsReturn {
  const [approvalRequests, setApprovalRequests] = useState<ApprovalRequest[]>([])

  // Listen for command approval events
  useEffect(() => {
    return getTransport().listen("approval_required", (raw) => {
      try {
        const request: ApprovalRequest = JSON.parse(raw as string)
        setApprovalRequests((prev) => [...prev, request])
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
      }
    })
  }, [])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    setApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    try {
      await getTransport().call("respond_to_approval", { requestId, response })
    } catch (e) {
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
    }
  }

  return { approvalRequests, handleApprovalResponse }
}
