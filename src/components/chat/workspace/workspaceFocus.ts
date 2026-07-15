export interface WorkspaceFocusSignal {
  sessionId: string
  nonce: number
}

export function shouldConsumeWorkspaceFocus(
  request: WorkspaceFocusSignal | null | undefined,
  currentSessionId: string | null | undefined,
  lastConsumedNonce: number,
): request is WorkspaceFocusSignal {
  return Boolean(
    request && request.sessionId === currentSessionId && request.nonce !== lastConsumedNonce,
  )
}
