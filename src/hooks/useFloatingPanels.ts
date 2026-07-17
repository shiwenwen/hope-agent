import { useCallback, useMemo, useState } from "react"

/** Panels that support the in-app floating window mode. */
export type FloatablePanel = "browser" | "mac-control"

/** Floating windows live below dialogs / fullscreen overlays (z-50). */
const FLOATING_Z_BASE = 40
const FLOATING_Z_MAX = 49

/**
 * Which control panels are currently floating, plus their stacking order.
 * State is a single array in stacking order (last = topmost) so all updaters
 * stay pure; rects are owned by each window's `useFloatingWindow`
 * (localStorage) — this hook only tracks membership and z-order so ChatScreen
 * stays thin.
 */
export function useFloatingPanels(): {
  floatingPanels: FloatablePanel[]
  isFloating: (panel: FloatablePanel) => boolean
  zIndexOf: (panel: FloatablePanel) => number
  float: (panel: FloatablePanel) => void
  dock: (panel: FloatablePanel) => void
  closeFloating: (panel: FloatablePanel) => void
  focusFloating: (panel: FloatablePanel) => void
} {
  const [stack, setStack] = useState<FloatablePanel[]>([])

  const float = useCallback((panel: FloatablePanel) => {
    setStack((prev) =>
      prev.includes(panel) ? prev : [...prev.filter((p) => p !== panel), panel],
    )
  }, [])

  const remove = useCallback((panel: FloatablePanel) => {
    setStack((prev) => (prev.includes(panel) ? prev.filter((p) => p !== panel) : prev))
  }, [])

  const focusFloating = useCallback((panel: FloatablePanel) => {
    setStack((prev) => {
      // Already topmost (or not floating) → no state change, so a click
      // inside the top window doesn't re-render the whole ChatScreen.
      if (!prev.includes(panel) || prev[prev.length - 1] === panel) return prev
      return [...prev.filter((p) => p !== panel), panel]
    })
  }, [])

  const isFloating = useCallback((panel: FloatablePanel) => stack.includes(panel), [stack])

  const zIndexOf = useCallback(
    (panel: FloatablePanel) =>
      Math.min(FLOATING_Z_BASE + Math.max(stack.indexOf(panel), 0), FLOATING_Z_MAX),
    [stack],
  )

  const floatingPanels = useMemo(() => stack, [stack])

  return {
    floatingPanels,
    isFloating,
    zIndexOf,
    float,
    dock: remove,
    closeFloating: remove,
    focusFloating,
  }
}
