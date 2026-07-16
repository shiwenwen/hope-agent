const dirtyEditors = new Set<string>()
const discardHandlers = new Map<string, () => void>()

export function setFileEditorDirty(id: string, dirty: boolean): void {
  if (dirty) dirtyEditors.add(id)
  else dirtyEditors.delete(id)
}

export function clearFileEditorDirty(id: string): void {
  dirtyEditors.delete(id)
}

export function registerFileEditorDiscard(id: string, discard: () => void): () => void {
  discardHandlers.set(id, discard)
  return () => {
    if (discardHandlers.get(id) === discard) discardHandlers.delete(id)
  }
}

export function hasDirtyFileEditors(): boolean {
  return dirtyEditors.size > 0
}

/**
 * Single leave guard for every surface that can unmount or re-scope a file
 * editor (session navigation, new chat, and transport changes).
 */
export function confirmDiscardDirtyFileEditors(message: string): boolean {
  if (!hasDirtyFileEditors() || typeof window === "undefined") return true
  if (!window.confirm(message)) return false
  for (const id of [...dirtyEditors]) discardHandlers.get(id)?.()
  dirtyEditors.clear()
  return true
}
