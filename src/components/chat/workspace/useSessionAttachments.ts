import { useCallback, useEffect, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import type { KbAttachment } from "@/types/knowledge"

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
): { attachments: KbAttachment[]; reload: () => void } {
  const incognito = opts?.incognito ?? false
  const [fetched, setFetched] = useState<KbAttachment[]>([])

  const reload = useCallback(() => {
    if (incognito || !sessionId) return
    getTransport()
      .call<KbAttachment[]>("list_session_kbs_cmd", {
        sessionId,
        projectId: projectId ?? undefined,
      })
      .then(setFetched)
      .catch(() => setFetched([]))
  }, [sessionId, projectId, incognito])

  useEffect(() => {
    reload()
  }, [reload])

  useEffect(() => getTransport().listen("knowledge:changed", reload), [reload])

  const attachments = incognito || !sessionId ? [] : fetched
  return { attachments, reload }
}
