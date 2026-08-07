import type { SessionMeta } from "@/types/chat"

export const SESSION_PIN_CHANGED_EVENT = "hope:session-pinned"

export type SessionPinChangePhase = "optimistic" | "rollback" | "refresh"

export interface SessionPinChangeDetail {
  session: SessionMeta
  phase: SessionPinChangePhase
}

export function dispatchSessionPinChange(session: SessionMeta, phase: SessionPinChangePhase) {
  window.dispatchEvent(
    new CustomEvent<SessionPinChangeDetail>(SESSION_PIN_CHANGED_EVENT, {
      detail: { session, phase },
    }),
  )
}

export function sessionWithPinnedState(
  session: SessionMeta,
  pinned: boolean,
  pinnedAt = new Date().toISOString(),
): SessionMeta {
  return {
    ...session,
    pinnedAt: pinned ? session.pinnedAt || pinnedAt : null,
  }
}

export function sortSessionsForSidebar(sessions: SessionMeta[]): SessionMeta[] {
  return sessions.slice().sort((a, b) => {
    const aPinned = a.pinnedAt ? Date.parse(a.pinnedAt) || 0 : 0
    const bPinned = b.pinnedAt ? Date.parse(b.pinnedAt) || 0 : 0
    if (aPinned !== bPinned) return bPinned - aPinned
    return (Date.parse(b.updatedAt) || 0) - (Date.parse(a.updatedAt) || 0)
  })
}
