export const AUTO_SEND_PENDING_EVENT = "hope:autoSendPending"

export function normalizeAutoSendPendingPreference(value: unknown): boolean {
  return value !== false
}

export function emitAutoSendPendingPreference(enabled: boolean): void {
  if (typeof window === "undefined") return
  window.dispatchEvent(
    new CustomEvent(AUTO_SEND_PENDING_EVENT, {
      detail: { enabled },
    }),
  )
}
