import { useCallback, useEffect, useRef, useState } from "react"
import { ResizeHandleGlow } from "@/components/ui/resize-handle-glow"
import { WORKBENCH_MIN } from "./useWorkbenchSizing"

interface WorkbenchResizeHandleProps {
  width: number
  availableWidth: number
  resizeLabel: string
  onWidthChange: (width: number) => void
  onWidthCommit: (width: number) => void
  onResetWidth: () => void
  onResizingChange?: (resizing: boolean) => void
}

/** Full-height divider shared by the title bar and docked workbench body. */
export function WorkbenchResizeHandle({
  width,
  availableWidth,
  resizeLabel,
  onWidthChange,
  onWidthCommit,
  onResetWidth,
  onResizingChange,
}: WorkbenchResizeHandleProps) {
  const cleanupRef = useRef<(() => void) | null>(null)
  const frameRef = useRef<number | null>(null)
  const nextWidthRef = useRef(width)
  const [resizing, setResizing] = useState(false)

  useEffect(
    () => () => {
      cleanupRef.current?.()
      if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current)
      onResizingChange?.(false)
    },
    [onResizingChange],
  )

  const beginResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault()
      cleanupRef.current?.()
      setResizing(true)
      onResizingChange?.(true)
      const startX = event.clientX
      const startWidth = width
      nextWidthRef.current = startWidth
      let moved = false
      const iframes = document.querySelectorAll("iframe")
      iframes.forEach((frame) => ((frame as HTMLElement).style.pointerEvents = "none"))

      const flushWidth = () => {
        frameRef.current = null
        onWidthChange(nextWidthRef.current)
      }
      const handleMove = (moveEvent: PointerEvent) => {
        moved = true
        nextWidthRef.current = startWidth - (moveEvent.clientX - startX)
        if (frameRef.current === null) frameRef.current = window.requestAnimationFrame(flushWidth)
      }
      let cleaned = false
      const cleanup = () => {
        if (cleaned) return
        cleaned = true
        window.removeEventListener("pointermove", handleMove)
        window.removeEventListener("pointerup", finish)
        window.removeEventListener("pointercancel", finish)
        window.removeEventListener("blur", finish)
        document.body.style.cursor = ""
        document.body.style.userSelect = ""
        iframes.forEach((frame) => ((frame as HTMLElement).style.pointerEvents = ""))
        if (cleanupRef.current === cleanup) cleanupRef.current = null
      }
      const finish = () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
        cleanup()
        setResizing(false)
        onResizingChange?.(false)
        if (moved) onWidthCommit(nextWidthRef.current)
      }
      cleanupRef.current = cleanup
      window.addEventListener("pointermove", handleMove)
      window.addEventListener("pointerup", finish)
      window.addEventListener("pointercancel", finish)
      window.addEventListener("blur", finish)
      document.body.style.cursor = "col-resize"
      document.body.style.userSelect = "none"
    },
    [onResizingChange, onWidthChange, onWidthCommit, width],
  )

  const handleSeparatorKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const step = event.shiftKey ? 48 : 16
      let next: number | null = null
      if (event.key === "ArrowLeft") next = width + step
      if (event.key === "ArrowRight") next = width - step
      if (event.key === "Home") next = WORKBENCH_MIN
      if (event.key === "End") next = availableWidth
      if (next === null) return
      event.preventDefault()
      onWidthCommit(next)
    },
    [availableWidth, onWidthCommit, width],
  )

  return (
    <div
      className="group absolute inset-y-0 z-40 w-[10px] translate-x-1/2 cursor-col-resize touch-none"
      style={{ right: width }}
      role="separator"
      tabIndex={0}
      aria-label={resizeLabel}
      aria-orientation="vertical"
      aria-valuemin={WORKBENCH_MIN}
      aria-valuemax={availableWidth}
      aria-valuenow={Math.round(width)}
      onPointerDown={beginResize}
      onDoubleClick={onResetWidth}
      onKeyDown={handleSeparatorKeyDown}
    >
      <ResizeHandleGlow active={resizing} className="inset-y-0 left-1/2 w-px -translate-x-1/2" />
    </div>
  )
}
