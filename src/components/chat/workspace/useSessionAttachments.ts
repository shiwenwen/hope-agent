import { useCallback, useEffect, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { KbAttachment } from "@/types/knowledge"
import { workspaceKnowledgeErrorDetail } from "./workspaceKnowledgeFeedback"

interface SessionAttachmentsState {
  key: string | null
  attachments: KbAttachment[]
  loadErrorDetail: string | null
}

/**
 * Fetch the knowledge spaces attached to a session (owner-plane
 * `list_session_kbs_cmd`) and keep them fresh on the `knowledge:changed` event.
 * Shared by the composer `KnowledgePicker` and the Workspace knowledge section so
 * the fetch + invalidation contract lives in one place.
 *
 * The empty case (incognito / no session) is DERIVED, never written via setState
 * — incognito gets zero KB info (D10) and the backend is never called. `reload`
 * is exposed for callers that need an imperative refresh (e.g. after an
 * attach/detach mutation or when a popover opens).
 */
export function useSessionAttachments(
  sessionId: string | null | undefined,
  projectId: string | null | undefined,
  opts?: { incognito?: boolean },
): { attachments: KbAttachment[]; reload: () => void; loadErrorDetail: string | null } {
  const incognito = opts?.incognito ?? false
  const requestKey = !incognito && sessionId ? `${sessionId}::${projectId ?? ""}` : null
  const [state, setState] = useState<SessionAttachmentsState>({
    key: null,
    attachments: [],
    loadErrorDetail: null,
  })

  const reload = useCallback(() => {
    const key = !incognito && sessionId ? `${sessionId}::${projectId ?? ""}` : null
    if (!key) {
      setState({ key: null, attachments: [], loadErrorDetail: null })
      return
    }
    getTransport()
      .call<KbAttachment[]>("list_session_kbs_cmd", {
        sessionId,
        projectId: projectId ?? undefined,
      })
      .then((attachments) => setState({ key, attachments, loadErrorDetail: null }))
      .catch((e) => {
        logger.error("chat", "useSessionAttachments::reload", "load failed", e)
        const loadErrorDetail = workspaceKnowledgeErrorDetail(e)
        setState((prev) => ({
          key,
          attachments: prev.key === key ? prev.attachments : [],
          loadErrorDetail,
        }))
      })
  }, [sessionId, projectId, incognito])

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) reload()
    })
    return () => {
      cancelled = true
    }
  }, [reload])

  useEffect(() => getTransport().listen("knowledge:changed", reload), [reload])

  const current = state.key === requestKey ? state : null
  const attachments = current?.attachments ?? []
  const loadErrorDetail = current?.loadErrorDetail ?? null
  return { attachments, reload, loadErrorDetail }
}
