import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
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
    let unlisten: UnlistenFn | undefined
    listen<string>("approval_required", (event) => {
      try {
        const request: ApprovalRequest = JSON.parse(event.payload)
        setApprovalRequests((prev) => [...prev, request])
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    setApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    try {
      await invoke("respond_to_approval", { requestId, response })
    } catch (e) {
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
    }
  }

  return { approvalRequests, handleApprovalResponse }
}
