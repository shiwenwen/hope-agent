import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import type { AskUserQuestionGroup } from "./AskUserQuestionBlock"

/**
 * Owns the `pendingQuestionGroup` for a single chat surface: restores an
 * unanswered group on session switch, picks up live `ask_user_request` events
 * for the active session, and clears on `ask_user_timed_out`.
 *
 * Extracted from the ask_user portion of `usePlanMode` so non-plan surfaces
 * (the design-space per-project chat, knowledge chat, …) can render
 * `ask_user_question` without dragging in plan state. Per-session by design —
 * each surface's listener only reacts to questions whose `sessionId` matches
 * its own `currentSessionId`, so the main chat and a design thread never
 * cross-render each other's questions. `usePlanMode` keeps its own inline copy
 * (battle-tested main-chat path); this is the shared primitive for new
 * consumers.
 */
export function useAskUserPending(currentSessionId: string | null): {
  pendingQuestionGroup: AskUserQuestionGroup | null
  setPendingQuestionGroup: React.Dispatch<React.SetStateAction<AskUserQuestionGroup | null>>
} {
  const [pendingQuestionGroup, setPendingQuestionGroup] = useState<AskUserQuestionGroup | null>(null)

  // Restore any still-pending group on session switch; drop the previous one
  // first so a stale card never lingers across a thread change (incl. new-chat).
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setPendingQuestionGroup(null)
    if (!currentSessionId) return
    let cancelled = false
    getTransport()
      .call<AskUserQuestionGroup | null>("get_pending_ask_user_group", {
        sessionId: currentSessionId,
      })
      .then((group) => {
        if (cancelled) return
        if (group && group.sessionId === currentSessionId) setPendingQuestionGroup(group)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [currentSessionId])

  // Live request emitted by the ask_user_question tool for the active session.
  useEffect(() => {
    return getTransport().listen("ask_user_request", (raw) => {
      try {
        const group = parsePayload<AskUserQuestionGroup>(raw)
        if (!group || group.sessionId !== currentSessionId) return
        setPendingQuestionGroup(group)
      } catch {
        // ignore parse errors
      }
    })
  }, [currentSessionId])

  // Mirror backend timeout cleanup so an expired question stops accepting input.
  useEffect(() => {
    return getTransport().listen("ask_user_timed_out", (raw) => {
      try {
        const payload = parsePayload<{ requestId?: string; sessionId?: string }>(raw)
        if (!payload || payload.sessionId !== currentSessionId) return
        setPendingQuestionGroup((prev) => (prev?.requestId === payload.requestId ? null : prev))
      } catch {
        // ignore parse errors
      }
    })
  }, [currentSessionId])

  return { pendingQuestionGroup, setPendingQuestionGroup }
}
