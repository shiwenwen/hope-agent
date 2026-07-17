import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react"
import { suspendPageInteractions } from "@/lib/drag-interaction"

export interface FloatingRect {
  x: number
  y: number
  width: number
  height: number
}

export type ResizeEdge = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw"

interface UseFloatingWindowOptions {
  storageKey: string
  defaultRect: () => FloatingRect
  minWidth?: number
  minHeight?: number
  /** Max size as a fraction of the viewport. */
  maxViewportRatio?: number
}

/** Keep at least this much of the window reachable so it can't be dragged
 *  fully off-screen. */
const MIN_VISIBLE_PX = 96
/** Title bar height that must stay inside the viewport vertically. */
const MIN_VISIBLE_Y_PX = 40

function clampRect(rect: FloatingRect, minW: number, minH: number, ratio: number): FloatingRect {
  const vw = window.innerWidth
  const vh = window.innerHeight
  const width = Math.min(Math.max(rect.width, minW), Math.max(minW, vw * ratio))
  const height = Math.min(Math.max(rect.height, minH), Math.max(minH, vh * ratio))
  const x = Math.min(Math.max(rect.x, -(width - MIN_VISIBLE_PX)), vw - MIN_VISIBLE_PX)
  const y = Math.min(Math.max(rect.y, 0), vh - MIN_VISIBLE_Y_PX)
  return { x, y, width, height }
}

function loadRect(storageKey: string): FloatingRect | null {
  try {
    const raw = localStorage.getItem(storageKey)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<FloatingRect>
    if (
      typeof parsed.x !== "number" ||
      typeof parsed.y !== "number" ||
      typeof parsed.width !== "number" ||
      typeof parsed.height !== "number"
    ) {
      return null
    }
    return parsed as FloatingRect
  } catch {
    return null
  }
}

/**
 * Drag/resize state machine for an in-app floating window.
 *
 * Performance contract: during a gesture the rect lives in a ref and is
 * written straight to the DOM node (`transform` for moves; width/height only
 * while resizing) through rAF — React state is committed once on pointer-up,
 * so the frame `<img>` subtree never re-renders mid-drag.
 */
export function useFloatingWindow(options: UseFloatingWindowOptions): {
  rect: FloatingRect
  gesture: "move" | ResizeEdge | null
  rootRef: RefObject<HTMLDivElement | null>
  rootStyle: CSSProperties
  handleTitlePointerDown: (e: ReactPointerEvent) => void
  handleResizePointerDown: (edge: ResizeEdge) => (e: ReactPointerEvent) => void
} {
  const { storageKey, defaultRect, minWidth = 320, minHeight = 240, maxViewportRatio = 0.9 } = options
  const [rect, setRect] = useState<FloatingRect>(() =>
    clampRect(loadRect(storageKey) ?? defaultRect(), minWidth, minHeight, maxViewportRatio),
  )
  const [gesture, setGesture] = useState<"move" | ResizeEdge | null>(null)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const liveRect = useRef<FloatingRect>(rect)
  const rafId = useRef<number | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)

  const applyLiveRect = useCallback(() => {
    rafId.current = null
    const node = rootRef.current
    if (!node) return
    const r = liveRect.current
    node.style.transform = `translate3d(${r.x}px, ${r.y}px, 0)`
    node.style.width = `${r.width}px`
    node.style.height = `${r.height}px`
  }, [])

  const scheduleApply = useCallback(() => {
    if (rafId.current == null) {
      rafId.current = requestAnimationFrame(applyLiveRect)
    }
  }, [applyLiveRect])

  const commit = useCallback(() => {
    const next = clampRect(liveRect.current, minWidth, minHeight, maxViewportRatio)
    liveRect.current = next
    setRect(next)
    setGesture(null)
    try {
      localStorage.setItem(storageKey, JSON.stringify(next))
    } catch {
      // ignore quota errors
    }
    scheduleApply()
  }, [maxViewportRatio, minHeight, minWidth, scheduleApply, storageKey])

  const beginGesture = useCallback(
    (
      e: ReactPointerEvent,
      kind: "move" | ResizeEdge,
      cursor: string,
      onMove: (dx: number, dy: number, start: FloatingRect) => FloatingRect,
    ) => {
      if (e.button !== 0) return
      e.preventDefault()
      const target = e.currentTarget as HTMLElement
      try {
        target.setPointerCapture(e.pointerId)
      } catch {
        // ignore
      }
      const startX = e.clientX
      const startY = e.clientY
      const start = { ...liveRect.current }
      const restore = suspendPageInteractions(cursor)
      setGesture(kind)

      const handleMove = (ev: PointerEvent) => {
        liveRect.current = clampRect(
          onMove(ev.clientX - startX, ev.clientY - startY, start),
          minWidth,
          minHeight,
          maxViewportRatio,
        )
        scheduleApply()
      }
      const finish = () => {
        target.removeEventListener("pointermove", handleMove)
        target.removeEventListener("pointerup", finish)
        target.removeEventListener("lostpointercapture", finish)
        window.removeEventListener("blur", finish)
        try {
          target.releasePointerCapture(e.pointerId)
        } catch {
          // ignore
        }
        restore()
        cleanupRef.current = null
        commit()
      }
      cleanupRef.current = finish
      target.addEventListener("pointermove", handleMove)
      target.addEventListener("pointerup", finish)
      target.addEventListener("lostpointercapture", finish)
      window.addEventListener("blur", finish)
    },
    [commit, maxViewportRatio, minHeight, minWidth, scheduleApply],
  )

  const handleTitlePointerDown = useCallback(
    (e: ReactPointerEvent) => {
      beginGesture(e, "move", "grabbing", (dx, dy, start) => ({
        ...start,
        x: start.x + dx,
        y: start.y + dy,
      }))
    },
    [beginGesture],
  )

  const handleResizePointerDown = useCallback(
    (edge: ResizeEdge) => {
      const cursor =
        edge === "n" || edge === "s"
          ? "ns-resize"
          : edge === "e" || edge === "w"
            ? "ew-resize"
            : edge === "ne" || edge === "sw"
              ? "nesw-resize"
              : "nwse-resize"
      return (e: ReactPointerEvent) => {
        beginGesture(e, edge, cursor, (dx, dy, start) => {
          let { x, y, width, height } = start
          if (edge.includes("e")) width = start.width + dx
          if (edge.includes("s")) height = start.height + dy
          if (edge.includes("w")) {
            const w = Math.max(minWidth, start.width - dx)
            x = start.x + (start.width - w)
            width = w
          }
          if (edge.includes("n")) {
            const h = Math.max(minHeight, start.height - dy)
            y = start.y + (start.height - h)
            height = h
          }
          return { x, y, width, height }
        })
      }
    },
    [beginGesture, minHeight, minWidth],
  )

  // Re-clamp the committed rect when the viewport shrinks (window resize,
  // external display unplugged).
  useEffect(() => {
    const onResize = () => {
      setRect((prev) => {
        const next = clampRect(prev, minWidth, minHeight, maxViewportRatio)
        liveRect.current = next
        return next
      })
    }
    window.addEventListener("resize", onResize)
    return () => window.removeEventListener("resize", onResize)
  }, [maxViewportRatio, minHeight, minWidth])

  // Abort a live gesture on unmount.
  useEffect(() => {
    return () => {
      cleanupRef.current?.()
      if (rafId.current != null) cancelAnimationFrame(rafId.current)
    }
  }, [])

  const rootStyle: CSSProperties = {
    position: "fixed",
    left: 0,
    top: 0,
    width: rect.width,
    height: rect.height,
    transform: `translate3d(${rect.x}px, ${rect.y}px, 0)`,
    willChange: gesture ? "transform" : undefined,
  }

  return { rect, gesture, rootRef, rootStyle, handleTitlePointerDown, handleResizePointerDown }
}
