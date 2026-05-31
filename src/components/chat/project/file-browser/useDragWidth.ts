import { useCallback, type MouseEvent as ReactMouseEvent } from "react"

/**
 * Mouse-drag width adjuster for a vertical splitter. Returns an `onMouseDown`
 * handler. `direction: "ltr"` (default) means dragging right grows the width
 * (handle on the right edge of the resized element); `"rtl"` means dragging
 * left grows it (handle on the left edge, as in RightPanelShell).
 *
 * Mirrors the proven drag logic in RightPanelShell: it suspends iframe pointer
 * events during the drag and restores the cursor/selection on mouse-up.
 */
export function useDragWidth(opts: {
  width: number
  min: number
  max: number
  onChange: (w: number) => void
  direction?: "ltr" | "rtl"
}) {
  const { width, min, max, onChange, direction = "ltr" } = opts
  return useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startWidth = width
      const sign = direction === "rtl" ? -1 : 1
      const onMove = (ev: MouseEvent) => {
        const next = Math.min(max, Math.max(min, startWidth + sign * (ev.clientX - startX)))
        onChange(next)
      }
      const iframes = document.querySelectorAll("iframe")
      iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = "none"))
      const onUp = () => {
        document.removeEventListener("mousemove", onMove)
        document.removeEventListener("mouseup", onUp)
        document.body.style.cursor = ""
        document.body.style.userSelect = ""
        iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = ""))
      }
      document.addEventListener("mousemove", onMove)
      document.addEventListener("mouseup", onUp)
      document.body.style.cursor = "col-resize"
      document.body.style.userSelect = "none"
    },
    [width, min, max, onChange, direction],
  )
}
