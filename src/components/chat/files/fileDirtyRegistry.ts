const dirtyEditors = new Set<string>()
const discardHandlers = new Map<string, () => void>()
/** Editor id → the surface that owns it (a workbench file tab, say). Editors
 *  with no owner only answer to the process-wide guards. */
const editorOwners = new Map<string, string>()

export function setFileEditorDirty(id: string, dirty: boolean, owner?: string): void {
  if (dirty) {
    dirtyEditors.add(id)
    if (owner) editorOwners.set(id, owner)
  } else {
    dirtyEditors.delete(id)
    editorOwners.delete(id)
  }
}

export function clearFileEditorDirty(id: string): void {
  dirtyEditors.delete(id)
  editorOwners.delete(id)
}

export function registerFileEditorDiscard(id: string, discard: () => void): () => void {
  discardHandlers.set(id, discard)
  return () => {
    if (discardHandlers.get(id) === discard) discardHandlers.delete(id)
  }
}

export function hasDirtyFileEditors(owner?: string): boolean {
  if (!owner) return dirtyEditors.size > 0
  return [...dirtyEditors].some((id) => editorOwners.get(id) === owner)
}

/**
 * Single leave guard for every surface that can unmount or re-scope a file
 * editor (session navigation, new chat, and transport changes).
 *
 * `owner` narrows it to one surface: closing a single file tab must not revert
 * the unsaved buffers of the tabs that stay open. Omit it for the process-wide
 * guards, which really are leaving every editor behind.
 */
export function confirmDiscardDirtyFileEditors(message: string, owner?: string): boolean {
  if (!hasDirtyFileEditors(owner) || typeof window === "undefined") return true
  if (!window.confirm(message)) return false
  for (const id of [...dirtyEditors]) {
    if (owner && editorOwners.get(id) !== owner) continue
    discardHandlers.get(id)?.()
    dirtyEditors.delete(id)
    editorOwners.delete(id)
  }
  return true
}
