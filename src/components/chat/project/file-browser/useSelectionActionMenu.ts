import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEventHandler,
  type RefObject,
} from "react"

import type { SelectionActionMenuPosition } from "@/components/common/SelectionActionMenu"

const VIEWPORT_GUTTER = 8
const MENU_ESTIMATED_WIDTH = 300
const MENU_ESTIMATED_HEIGHT = 38
const MENU_SELECTION_GAP = 8
const KEYBOARD_SELECTION_DEBOUNCE_MS = 100
const POINTER_FINISH_GRACE_MS = 100
const CONTEXT_SELECTION_SUPPRESS_MS = 250

export interface TextSelectionValue {
  text: string
}

export interface SelectionActionMenuState<T extends TextSelectionValue> {
  position: SelectionActionMenuPosition
  text: string
  value: T | null
  copyMode: "selection" | "all"
  trigger: "selection" | "context"
}

interface UseSelectionActionMenuOptions<T extends TextSelectionValue> {
  rootRef: RefObject<HTMLElement | null>
  readSelection: () => T | null
  getCopyAllText?: () => string
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

function positionAroundRect(rect: DOMRect): SelectionActionMenuPosition {
  const viewportWidth = window.innerWidth
  const viewportHeight = window.innerHeight
  const x = clamp(
    rect.left + rect.width / 2 - MENU_ESTIMATED_WIDTH / 2,
    VIEWPORT_GUTTER,
    viewportWidth - MENU_ESTIMATED_WIDTH - VIEWPORT_GUTTER,
  )
  const preferredY = rect.top - MENU_ESTIMATED_HEIGHT - MENU_SELECTION_GAP
  const y =
    preferredY >= VIEWPORT_GUTTER
      ? preferredY
      : clamp(
          rect.bottom + MENU_SELECTION_GAP,
          VIEWPORT_GUTTER,
          viewportHeight - MENU_ESTIMATED_HEIGHT - VIEWPORT_GUTTER,
        )
  return { x, y }
}

function currentSelectionPosition(): SelectionActionMenuPosition | null {
  const selection = window.getSelection()
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null
  const range = selection.getRangeAt(0)
  const rects =
    typeof range.getClientRects === "function"
      ? Array.from(range.getClientRects()).filter((rect) => rect.width > 0 || rect.height > 0)
      : []
  const rect =
    rects[0] ??
    (typeof range.getBoundingClientRect === "function" ? range.getBoundingClientRect() : null)
  if (!rect) return null
  if (!Number.isFinite(rect.left) || !Number.isFinite(rect.top)) return null
  return positionAroundRect(rect)
}

function positionAtPointer(clientX: number, clientY: number): SelectionActionMenuPosition {
  return {
    x: clamp(
      clientX,
      VIEWPORT_GUTTER,
      window.innerWidth - MENU_ESTIMATED_WIDTH - VIEWPORT_GUTTER,
    ),
    y: clamp(
      clientY,
      VIEWPORT_GUTTER,
      window.innerHeight - MENU_ESTIMATED_HEIGHT - VIEWPORT_GUTTER,
    ),
  }
}

/**
 * Tracks a DOM selection owned by one preview surface. `selectionchange` is
 * used instead of mouse-up alone so keyboard selection and mobile selection
 * handles receive the same automatic toolbar. Right-click remains available,
 * including the legacy copy-all action when no text is selected.
 */
export function useSelectionActionMenu<T extends TextSelectionValue>({
  rootRef,
  readSelection,
  getCopyAllText,
}: UseSelectionActionMenuOptions<T>) {
  const [menu, setMenu] = useState<SelectionActionMenuState<T> | null>(null)
  const selectionTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const primaryPointerActiveRef = useRef(false)
  const pendingPointerSelectionRef = useRef(false)
  const pointerFinishedUntilRef = useRef(0)
  const suppressAutomaticUntilRef = useRef(0)

  const closeMenu = useCallback(() => setMenu(null), [])

  const syncSelection = useCallback(() => {
    const root = rootRef.current
    if (!root || !root.isConnected) {
      setMenu((current) => (current?.trigger === "selection" ? null : current))
      return
    }
    const value = readSelection()
    const position = value ? currentSelectionPosition() : null
    if (!value || !position) {
      // A collapsed selection should close an automatically opened toolbar,
      // but must not erase a deliberate right-click copy-all menu.
      setMenu((current) => (current?.trigger === "selection" ? null : current))
      return
    }
    setMenu({
      position,
      text: value.text,
      value,
      copyMode: "selection",
      trigger: "selection",
    })
  }, [readSelection, rootRef])

  const scheduleSelectionSync = useCallback(
    (delay: number) => {
      if (selectionTimerRef.current) clearTimeout(selectionTimerRef.current)
      selectionTimerRef.current = setTimeout(() => {
        selectionTimerRef.current = null
        if (Date.now() < suppressAutomaticUntilRef.current) return
        syncSelection()
      }, delay)
    },
    [syncSelection],
  )

  useEffect(() => {
    const targetBelongsToRoot = (target: EventTarget | null) =>
      target instanceof Node && !!rootRef.current?.contains(target)

    const startPrimaryPointer = (target: EventTarget | null) => {
      if (!targetBelongsToRoot(target)) return
      primaryPointerActiveRef.current = true
      pendingPointerSelectionRef.current = false
      if (selectionTimerRef.current) {
        clearTimeout(selectionTimerRef.current)
        selectionTimerRef.current = null
      }
    }

    const finishPrimaryPointer = () => {
      if (!primaryPointerActiveRef.current) return
      primaryPointerActiveRef.current = false
      pointerFinishedUntilRef.current = Date.now() + POINTER_FINISH_GRACE_MS
      const hadPendingSelection = pendingPointerSelectionRef.current
      pendingPointerSelectionRef.current = false
      const selection = window.getSelection()
      if (!hadPendingSelection && (!selection || selection.isCollapsed)) return
      // The final Selection is committed at pointer/touch end. Defer one task
      // so WebKit can publish the endpoint before it is read.
      scheduleSelectionSync(0)
    }

    const onPointerDown = (event: PointerEvent) => {
      if (!event.isPrimary || event.button !== 0) return
      startPrimaryPointer(event.target)
    }
    const onPointerUp = (event: PointerEvent) => {
      if (!event.isPrimary) return
      finishPrimaryPointer()
    }
    const onPointerCancel = (event: PointerEvent) => {
      if (!event.isPrimary) return
      primaryPointerActiveRef.current = false
      pendingPointerSelectionRef.current = false
    }
    const onPointerOut = (event: PointerEvent) => {
      if (
        event.isPrimary &&
        primaryPointerActiveRef.current &&
        (event.pointerType === "mouse" || !event.pointerType) &&
        !event.relatedTarget
      ) {
        finishPrimaryPointer()
      }
    }
    const onWindowBlur = () => {
      primaryPointerActiveRef.current = false
      pendingPointerSelectionRef.current = false
      if (selectionTimerRef.current) {
        clearTimeout(selectionTimerRef.current)
        selectionTimerRef.current = null
      }
    }
    // Touch listeners are a fallback for older WebViews that expose DOM
    // selection handles without dispatching the matching PointerEvent pair.
    const onTouchStart = (event: TouchEvent) => {
      if (event.touches.length === 1) startPrimaryPointer(event.target)
    }
    const onTouchEnd = () => finishPrimaryPointer()
    const onTouchCancel = () => {
      primaryPointerActiveRef.current = false
      pendingPointerSelectionRef.current = false
    }
    const onSelectionChange = () => {
      if (Date.now() < suppressAutomaticUntilRef.current) return
      if (primaryPointerActiveRef.current) {
        pendingPointerSelectionRef.current = true
        return
      }
      if (Date.now() < pointerFinishedUntilRef.current) {
        scheduleSelectionSync(0)
        return
      }
      // Keyboard extensions (Shift+Arrow, Select All) have no pointer boundary,
      // so use a short debounce to avoid flickering through intermediate ranges.
      scheduleSelectionSync(KEYBOARD_SELECTION_DEBOUNCE_MS)
    }

    document.addEventListener("pointerdown", onPointerDown, true)
    document.addEventListener("pointerup", onPointerUp, true)
    document.addEventListener("pointercancel", onPointerCancel, true)
    document.addEventListener("pointerout", onPointerOut, true)
    document.addEventListener("touchstart", onTouchStart, true)
    document.addEventListener("touchend", onTouchEnd, true)
    document.addEventListener("touchcancel", onTouchCancel, true)
    document.addEventListener("selectionchange", onSelectionChange)
    window.addEventListener("blur", onWindowBlur)
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true)
      document.removeEventListener("pointerup", onPointerUp, true)
      document.removeEventListener("pointercancel", onPointerCancel, true)
      document.removeEventListener("pointerout", onPointerOut, true)
      document.removeEventListener("touchstart", onTouchStart, true)
      document.removeEventListener("touchend", onTouchEnd, true)
      document.removeEventListener("touchcancel", onTouchCancel, true)
      document.removeEventListener("selectionchange", onSelectionChange)
      window.removeEventListener("blur", onWindowBlur)
      if (selectionTimerRef.current) clearTimeout(selectionTimerRef.current)
    }
  }, [rootRef, scheduleSelectionSync])

  useEffect(() => {
    if (!menu) return
    const onPointerDown = () => setMenu(null)
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null)
    }
    const close = () => setMenu(null)
    window.addEventListener("pointerdown", onPointerDown)
    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("resize", close)
    window.addEventListener("blur", close)
    window.addEventListener("scroll", close, true)
    return () => {
      window.removeEventListener("pointerdown", onPointerDown)
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("resize", close)
      window.removeEventListener("blur", close)
      window.removeEventListener("scroll", close, true)
    }
  }, [menu])

  const onContextMenuCapture = useCallback<MouseEventHandler<HTMLElement>>(
    (event) => {
      suppressAutomaticUntilRef.current = Date.now() + CONTEXT_SELECTION_SUPPRESS_MS
      pendingPointerSelectionRef.current = false
      if (selectionTimerRef.current) {
        clearTimeout(selectionTimerRef.current)
        selectionTimerRef.current = null
      }
      const value = readSelection()
      const text = value?.text ?? getCopyAllText?.() ?? ""
      if (!text.trim()) return
      event.preventDefault()
      setMenu({
        position: positionAtPointer(event.clientX, event.clientY),
        text,
        value,
        copyMode: value ? "selection" : "all",
        trigger: "context",
      })
    },
    [getCopyAllText, readSelection],
  )

  return { menu, closeMenu, onContextMenuCapture }
}

/** Read a non-empty Selection whose endpoints both belong to `root`. */
export function readDomTextSelection(root: HTMLElement | null): TextSelectionValue | null {
  const selection = window.getSelection()
  const text = selection?.toString() ?? ""
  if (!root || !selection || selection.isCollapsed || !text.trim()) return null
  if (!root.contains(selection.anchorNode) || !root.contains(selection.focusNode)) return null
  return { text }
}
