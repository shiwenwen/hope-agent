import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { WorkbenchLayoutMode, WorkbenchWidthMode } from "./types"

/** Ideal minimum for the conversation; the card's lane is added on top of it. */
export const CHAT_IDEAL_MIN = 560
export const CHAT_HARD_MIN = 360
/** Ideal minimum for the workbench — it only goes below this once the chat is
 *  already at its own ideal minimum. */
export const WORKBENCH_IDEAL_MIN = 560
export const WORKBENCH_MIN = 420
export const WORKBENCH_MAX = 1280
/** Even split, so narrowing takes from both columns instead of only the chat. */
export const WORKBENCH_DEFAULT_RATIO = 0.5
export const WORKBENCH_STAGE_THRESHOLD = WORKBENCH_MIN + CHAT_HARD_MIN
export const WORKBENCH_LAYOUT_HYSTERESIS = 80

const WIDTH_MODE_KEY = "hope.chat.workbench.widthMode"
const MANUAL_RATIO_KEY = "hope.chat.workbench.manualRatio"

function finiteRatio(value: string | null): number | null {
  if (!value) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 && parsed < 1 ? parsed : null
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

export interface WorkbenchLayoutInput {
  /** Content width right of the session sidebar. */
  available: number
  /** Share of `available` the workbench wants — the drag ratio, or the default. */
  ratio: number
  /** The conversation's ideal minimum, card lane included when it is open. */
  chatIdeal: number
}

export interface WorkbenchLayout {
  width: number
  /** No room left for the chat's ideal plus the workbench's hard minimum. */
  collapse: boolean
}

/**
 * Both columns shrink together while each is still above its ideal minimum.
 * Past that the workbench is the one that gives — first down to its own
 * minimum, then away entirely — so the conversation keeps its ideal width.
 */
export function resolveWorkbenchLayout({
  available,
  ratio,
  chatIdeal,
}: WorkbenchLayoutInput): WorkbenchLayout {
  if (available < chatIdeal + WORKBENCH_MIN) return { width: WORKBENCH_MIN, collapse: true }
  const upper = Math.min(WORKBENCH_MAX, available - chatIdeal)
  const lower = available >= chatIdeal + WORKBENCH_IDEAL_MIN ? WORKBENCH_IDEAL_MIN : WORKBENCH_MIN
  return {
    width: clamp(Math.round(available * ratio), Math.min(lower, upper), upper),
    collapse: false,
  }
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
  /** The chat's ideal minimum no longer fits beside a minimum workbench. */
  shouldCollapse: boolean
  /** Width `shouldCollapse` flips at, so callers can add their own hysteresis. */
  collapseThreshold: number
  /** There is still room for the environment card's lane. */
  cardFits: boolean
  setManualWidth: (width: number) => void
  commitManualWidth: (width: number) => void
  resetAutomaticWidth: () => void
}

export function useWorkbenchSizing(
  open: boolean,
  cardLane: number = 0,
  cardOpen: boolean = false,
): WorkbenchSizing {
  const containerRef = useRef<HTMLDivElement>(null)
  const [availableWidth, setAvailableWidth] = useState(() =>
    typeof window === "undefined" ? 1200 : window.innerWidth,
  )
  const [widthMode, setWidthMode] = useState<WorkbenchWidthMode>(() => {
    if (typeof window === "undefined") return "auto"
    return window.localStorage.getItem(WIDTH_MODE_KEY) === "manual" ? "manual" : "auto"
  })
  // Stored as a share of the container, not pixels: a remembered pixel width
  // would sit still while the window narrows, making the chat pay for all of it.
  const [manualRatio, setManualRatio] = useState(() => {
    if (typeof window === "undefined") return WORKBENCH_DEFAULT_RATIO
    return finiteRatio(window.localStorage.getItem(MANUAL_RATIO_KEY)) ?? WORKBENCH_DEFAULT_RATIO
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

  // Hysteresis makes the layout mode depend on its own previous value, so it is
  // state adjusted during render rather than a plain derivation.
  const [storedLayoutMode, setStoredLayoutMode] = useState<WorkbenchLayoutMode>("docked")
  const layoutMode = nextWorkbenchLayoutMode(storedLayoutMode, open, availableWidth)
  if (layoutMode !== storedLayoutMode) setStoredLayoutMode(layoutMode)
  // The card is given up before the workbench is: it only keeps its lane while
  // both columns can still hold their ideal minimums beside it. Deliberately
  // independent of `cardOpen` — otherwise opening the card in a narrow window
  // would flip this and close it again on the same click.
  const cardFits = availableWidth >= CHAT_IDEAL_MIN + cardLane + (open ? WORKBENCH_IDEAL_MIN : 0)
  const chatIdeal = CHAT_IDEAL_MIN + (cardOpen && cardFits ? cardLane : 0)
  const layout = useMemo(
    () =>
      resolveWorkbenchLayout({
        available: availableWidth,
        ratio: widthMode === "manual" ? manualRatio : WORKBENCH_DEFAULT_RATIO,
        chatIdeal,
      }),
    [availableWidth, chatIdeal, manualRatio, widthMode],
  )
  const width = layoutMode === "stage" ? availableWidth : layout.width

  const setManualWidth = useCallback(
    (nextWidth: number) => {
      if (availableWidth <= 0) return
      setWidthMode("manual")
      setManualRatio(nextWidth / availableWidth)
    },
    [availableWidth],
  )

  const commitManualWidth = useCallback(
    (nextWidth: number) => {
      if (availableWidth <= 0) return
      const committed = resolveWorkbenchLayout({
        available: availableWidth,
        ratio: nextWidth / availableWidth,
        chatIdeal,
      }).width
      const ratio = committed / availableWidth
      setWidthMode("manual")
      setManualRatio(ratio)
      if (typeof window !== "undefined") {
        window.localStorage.setItem(WIDTH_MODE_KEY, "manual")
        window.localStorage.setItem(MANUAL_RATIO_KEY, ratio.toFixed(4))
      }
    },
    [availableWidth, chatIdeal],
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
    shouldCollapse: layout.collapse,
    collapseThreshold: chatIdeal + WORKBENCH_MIN,
    cardFits,
    setManualWidth,
    commitManualWidth,
    resetAutomaticWidth,
  }
}
