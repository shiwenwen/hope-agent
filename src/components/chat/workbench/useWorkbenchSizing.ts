import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { WorkbenchLayoutMode, WorkbenchWidthMode } from "./types"

export const CHAT_INITIAL_RESERVE = 560
export const CHAT_HARD_MIN = 360
export const WORKBENCH_MIN = 420
export const WORKBENCH_MAX = 1280
export const WORKBENCH_INITIAL_RATIO = 0.68
export const WORKBENCH_MANUAL_RATIO = 0.78
export const WORKBENCH_STAGE_THRESHOLD = WORKBENCH_MIN + CHAT_HARD_MIN
export const WORKBENCH_LAYOUT_HYSTERESIS = 80

const WIDTH_MODE_KEY = "hope.chat.workbench.widthMode"
const MANUAL_WIDTH_KEY = "hope.chat.workbench.manualWidth"

function finitePositive(value: string | null): number | null {
  if (!value) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

export function automaticWorkbenchWidth(availableWidth: number): number {
  const preferred = Math.min(
    WORKBENCH_MAX,
    Math.floor(availableWidth * WORKBENCH_INITIAL_RATIO),
    availableWidth - CHAT_INITIAL_RESERVE,
  )
  return clamp(preferred, WORKBENCH_MIN, availableWidth - CHAT_HARD_MIN)
}

export function manualWorkbenchWidth(availableWidth: number, requestedWidth: number): number {
  const maximum = Math.min(
    WORKBENCH_MAX,
    Math.floor(availableWidth * WORKBENCH_MANUAL_RATIO),
    availableWidth - CHAT_HARD_MIN,
  )
  return clamp(requestedWidth, WORKBENCH_MIN, maximum)
}

export function nextWorkbenchLayoutMode(
  current: WorkbenchLayoutMode,
  open: boolean,
  availableWidth: number,
): WorkbenchLayoutMode {
  if (!open) return "docked"
  if (
    current === "stage" &&
    availableWidth >= WORKBENCH_STAGE_THRESHOLD + WORKBENCH_LAYOUT_HYSTERESIS
  ) {
    return "docked"
  }
  if (current === "docked" && availableWidth < WORKBENCH_STAGE_THRESHOLD) return "stage"
  return current
}

interface WorkbenchSizing {
  containerRef: React.RefObject<HTMLDivElement | null>
  availableWidth: number
  width: number
  widthMode: WorkbenchWidthMode
  layoutMode: WorkbenchLayoutMode
  setManualWidth: (width: number) => void
  commitManualWidth: (width: number) => void
  resetAutomaticWidth: () => void
}

export function useWorkbenchSizing(open: boolean): WorkbenchSizing {
  const containerRef = useRef<HTMLDivElement>(null)
  const [availableWidth, setAvailableWidth] = useState(() =>
    typeof window === "undefined" ? 1200 : window.innerWidth,
  )
  const [widthMode, setWidthMode] = useState<WorkbenchWidthMode>(() => {
    if (typeof window === "undefined") return "auto"
    return window.localStorage.getItem(WIDTH_MODE_KEY) === "manual" ? "manual" : "auto"
  })
  const [requestedManualWidth, setRequestedManualWidth] = useState(() => {
    if (typeof window === "undefined") return 720
    return finitePositive(window.localStorage.getItem(MANUAL_WIDTH_KEY)) ?? 720
  })

  useEffect(() => {
    const node = containerRef.current
    if (!node) return

    const update = () => {
      const next = Math.round(node.getBoundingClientRect().width)
      if (next > 0) setAvailableWidth((current) => (current === next ? current : next))
    }
    update()
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update)
      return () => window.removeEventListener("resize", update)
    }
    const observer = new ResizeObserver(update)
    observer.observe(node)
    return () => observer.disconnect()
  }, [])

  const layoutModeRef = useRef<WorkbenchLayoutMode>("docked")
  layoutModeRef.current = nextWorkbenchLayoutMode(layoutModeRef.current, open, availableWidth)
  const layoutMode = layoutModeRef.current
  const width = useMemo(() => {
    if (layoutMode === "stage") return availableWidth
    return widthMode === "manual"
      ? manualWorkbenchWidth(availableWidth, requestedManualWidth)
      : automaticWorkbenchWidth(availableWidth)
  }, [availableWidth, layoutMode, requestedManualWidth, widthMode])

  const setManualWidth = useCallback((nextWidth: number) => {
    setWidthMode("manual")
    setRequestedManualWidth(nextWidth)
  }, [])

  const commitManualWidth = useCallback(
    (nextWidth: number) => {
      const committedWidth = manualWorkbenchWidth(availableWidth, nextWidth)
      setWidthMode("manual")
      setRequestedManualWidth(committedWidth)
      if (typeof window !== "undefined") {
        window.localStorage.setItem(WIDTH_MODE_KEY, "manual")
        window.localStorage.setItem(MANUAL_WIDTH_KEY, String(Math.round(committedWidth)))
      }
    },
    [availableWidth],
  )

  const resetAutomaticWidth = useCallback(() => {
    setWidthMode("auto")
    if (typeof window !== "undefined") {
      window.localStorage.setItem(WIDTH_MODE_KEY, "auto")
    }
  }, [])

  return {
    containerRef,
    availableWidth,
    width,
    widthMode,
    layoutMode,
    setManualWidth,
    commitManualWidth,
    resetAutomaticWidth,
  }
}
