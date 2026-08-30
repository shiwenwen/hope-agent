export function resolveSideChatSurfaceSessionId(
  parentSessionId: string | null,
  sideSessionId: string | null,
  panelOpen: boolean,
): string | null {
  return panelOpen && sideSessionId ? sideSessionId : parentSessionId
}

export type SideChatQuoteOwner = "main" | "side" | null

export function resolveSideChatQuoteOwner(
  ownerSessionId: string | null | undefined,
  parentSessionId: string | null,
  sideSessionId: string | null,
  panelOpen: boolean,
): SideChatQuoteOwner {
  if (ownerSessionId === parentSessionId) return "main"
  if (panelOpen && sideSessionId && ownerSessionId === sideSessionId) return "side"
  return null
}
