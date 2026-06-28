export const COMPLETED_TURN_COLLAPSE_EVENT = "hope:autoCollapseCompletedTurns"

export function normalizeCompletedTurnCollapsePreference(value: unknown): boolean {
  return value !== false
}

export function emitCompletedTurnCollapsePreference(enabled: boolean) {
  if (typeof window === "undefined") return
  window.dispatchEvent(
    new CustomEvent(COMPLETED_TURN_COLLAPSE_EVENT, {
      detail: { enabled },
    }),
  )
}
