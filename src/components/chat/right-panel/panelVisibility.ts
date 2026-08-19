import { createContext, useContext, useEffect, useRef } from "react"

/**
 * Whether the surrounding panel's content is actually on screen.
 *
 * Every workbench tab stays warm-mounted, so a hidden panel keeps running its
 * effects — including polls nobody can read. Panels rendered outside a shell
 * (floating windows, settings, dialogs) have no such notion and default to
 * visible, so this only ever narrows behaviour inside the workbench.
 */
export const PanelVisibilityContext = createContext(true)

export function usePanelVisible(): boolean {
  return useContext(PanelVisibilityContext)
}

/**
 * Run `onReveal` each time the panel comes back into view — never on mount.
 * For data with no event feed behind it, this is what replaces the polling the
 * panel skipped while it was hidden, without duplicating the initial load.
 */
export function usePanelRevealRefresh(onReveal: () => void): void {
  const visible = usePanelVisible()
  const wasHiddenRef = useRef(false)
  const callbackRef = useRef(onReveal)

  useEffect(() => {
    callbackRef.current = onReveal
  }, [onReveal])

  useEffect(() => {
    if (!visible) {
      wasHiddenRef.current = true
      return
    }
    if (!wasHiddenRef.current) return
    wasHiddenRef.current = false
    callbackRef.current()
  }, [visible])
}
