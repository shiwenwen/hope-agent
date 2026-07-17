import { useCallback, useMemo, useRef, useState } from "react"

/** Panels that support the in-app floating window mode. */
export type FloatablePanel = "browser" | "mac-control"

/** Floating windows live below dialogs / fullscreen overlays (z-50). */
const FLOATING_Z_BASE = 40
const FLOATING_Z_MAX = 49

/**
 * Which control panels are currently floating, plus their stacking order.
 * Rects are owned by each window's `useFloatingWindow` (localStorage); this
 * hook only tracks membership and z-order so ChatScreen stays thin.
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
  const [floating, setFloating] = useState<Partial<Record<FloatablePanel, number>>>({})
  const zCounter = useRef(0)

  const float = useCallback((panel: FloatablePanel) => {
    zCounter.current += 1
    const z = zCounter.current
    setFloating((prev) => ({ ...prev, [panel]: z }))
  }, [])

  const remove = useCallback((panel: FloatablePanel) => {
    setFloating((prev) => {
      if (!(panel in prev)) return prev
      const next = { ...prev }
      delete next[panel]
      return next
    })
  }, [])

  const focusFloating = useCallback((panel: FloatablePanel) => {
    zCounter.current += 1
    const z = zCounter.current
    setFloating((prev) => (panel in prev ? { ...prev, [panel]: z } : prev))
  }, [])

  const floatingPanels = useMemo(
    () =>
      (Object.entries(floating) as Array<[FloatablePanel, number]>)
        .sort((a, b) => a[1] - b[1])
        .map(([panel]) => panel),
    [floating],
  )

  const isFloating = useCallback((panel: FloatablePanel) => panel in floating, [floating])

  const zIndexOf = useCallback(
    (panel: FloatablePanel) => {
      const order = floatingPanels.indexOf(panel)
      return Math.min(FLOATING_Z_BASE + Math.max(order, 0), FLOATING_Z_MAX)
    },
    [floatingPanels],
  )

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
