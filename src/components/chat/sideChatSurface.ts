export function resolveSideChatSurfaceSessionId(
  parentSessionId: string | null,
  sideSessionId: string | null,
  panelOpen: boolean,
): string | null {
  return panelOpen && sideSessionId ? sideSessionId : parentSessionId
}
