import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefCallback,
  type ReactNode,
} from "react"
import { cn } from "@/lib/utils"
import { UI_MOTION } from "@/components/ui/motion"

interface RightPanelShellProps {
  width: number
  onWidthChange?: (width: number) => void
  resizeLabel: string
  children: ReactNode
  minWidth?: number
  maxWidth?: number
  maxViewportRatio?: number
  reservedMainWidth?: number
  maximized?: boolean
  collapsed?: boolean
  overlay?: boolean
  contentKey?: string | number | null
  surfaceClassName?: string
  bodyClassName?: string
  /** Root node used by the shared fullscreen FLIP transition hook. */
  fullscreenTransitionRef?: RefCallback<HTMLDivElement>
  /** Animate the rail from zero width when it is the first right panel to open. */
  animateOnMount?: boolean
}

export function RightPanelShell({
  width,
  onWidthChange,
  resizeLabel,
  children,
  minWidth = 360,
  maxWidth = 960,
  maxViewportRatio = 0.55,
  reservedMainWidth = 420,
  maximized = false,
  collapsed = false,
  overlay = false,
  contentKey,
  surfaceClassName,
  bodyClassName,
  fullscreenTransitionRef,
  animateOnMount = false,
}: RightPanelShellProps) {
  const shellRef = useRef<HTMLDivElement>(null)
  const resolvedContentKey = contentKey ?? "right-panel-content"
  const lastContentKeyRef = useRef<string | number>(resolvedContentKey)
  const transitionTimerRef = useRef<number | null>(null)
  const transitionFrameRef = useRef<number | null>(null)
  const entryFrameRef = useRef<number | null>(null)
  const dragCleanupRef = useRef<(() => void) | null>(null)
  const [transitionVeilVisible, setTransitionVeilVisible] = useState(false)
  const [isResizing, setIsResizing] = useState(false)
  const [entryVisible, setEntryVisible] = useState(!animateOnMount)
  const setShellRef = useCallback(
    (node: HTMLDivElement | null) => {
      shellRef.current = node
      fullscreenTransitionRef?.(node)
    },
    [fullscreenTransitionRef],
  )

  useLayoutEffect(() => {
    if (entryVisible) return
    entryFrameRef.current = window.requestAnimationFrame(() => {
      setEntryVisible(true)
      entryFrameRef.current = null
    })
    return () => {
      if (entryFrameRef.current !== null) {
        window.cancelAnimationFrame(entryFrameRef.current)
        entryFrameRef.current = null
      }
    }
  }, [entryVisible])

  useLayoutEffect(() => {
    if (Object.is(lastContentKeyRef.current, resolvedContentKey)) return
    lastContentKeyRef.current = resolvedContentKey
    if (transitionTimerRef.current !== null) window.clearTimeout(transitionTimerRef.current)
    if (transitionFrameRef.current !== null) window.cancelAnimationFrame(transitionFrameRef.current)
    transitionFrameRef.current = window.requestAnimationFrame(() => {
      setTransitionVeilVisible(true)
      transitionFrameRef.current = window.requestAnimationFrame(() => {
        setTransitionVeilVisible(false)
        transitionFrameRef.current = null
      })
    })
    transitionTimerRef.current = window.setTimeout(() => {
      setTransitionVeilVisible(false)
      transitionTimerRef.current = null
    }, UI_MOTION.panelContentEnter)
  }, [resolvedContentKey])

  useEffect(
    () => () => {
      if (transitionTimerRef.current !== null) {
        window.clearTimeout(transitionTimerRef.current)
        transitionTimerRef.current = null
      }
      if (transitionFrameRef.current !== null) {
        window.cancelAnimationFrame(transitionFrameRef.current)
        transitionFrameRef.current = null
      }
      if (entryFrameRef.current !== null) {
        window.cancelAnimationFrame(entryFrameRef.current)
        entryFrameRef.current = null
      }
      dragCleanupRef.current?.()
    },
    [],
  )

  const visuallyCollapsed = collapsed || !entryVisible

  const handleDragStart = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (!onWidthChange || visuallyCollapsed) return
      e.preventDefault()
      dragCleanupRef.current?.()
      setIsResizing(true)
      const startX = e.clientX
      const startWidth = width
      const containerWidth =
        shellRef.current?.parentElement?.getBoundingClientRect().width ?? window.innerWidth
      const availableWidth = Math.max(0, containerWidth - reservedMainWidth)
      const effectiveMinWidth = Math.min(minWidth, availableWidth)
      const effectiveMaxWidth = Math.max(
        effectiveMinWidth,
        Math.min(maxWidth, containerWidth * maxViewportRatio, availableWidth),
      )
      const onMouseMove = (ev: MouseEvent) => {
        const nextWidth = Math.min(
          effectiveMaxWidth,
          Math.max(effectiveMinWidth, startWidth - (ev.clientX - startX)),
        )
        onWidthChange(nextWidth)
      }
      const iframes = document.querySelectorAll("iframe")
      iframes.forEach((frame) => ((frame as HTMLElement).style.pointerEvents = "none"))
      let cleanedUp = false
      const cleanup = () => {
        if (cleanedUp) return
        cleanedUp = true
        document.removeEventListener("mousemove", onMouseMove)
        document.removeEventListener("mouseup", finishDrag)
        window.removeEventListener("blur", finishDrag)
        document.body.style.cursor = ""
        document.body.style.userSelect = ""
        iframes.forEach((frame) => ((frame as HTMLElement).style.pointerEvents = ""))
        if (dragCleanupRef.current === cleanup) dragCleanupRef.current = null
      }
      const finishDrag = () => {
        cleanup()
        setIsResizing(false)
      }
      dragCleanupRef.current = cleanup
      document.addEventListener("mousemove", onMouseMove)
      document.addEventListener("mouseup", finishDrag)
      window.addEventListener("blur", finishDrag)
      document.body.style.cursor = "col-resize"
      document.body.style.userSelect = "none"
    },
    [
      maxViewportRatio,
      maxWidth,
      minWidth,
      onWidthChange,
      reservedMainWidth,
      visuallyCollapsed,
      width,
    ],
  )

  const availableWidthCss = `max(0px, calc(100% - ${reservedMainWidth}px))`
  const panelStyle: CSSProperties = visuallyCollapsed
    ? { width: 0, minWidth: 0, maxWidth: 0 }
    : {
        width: `min(${width}px, ${availableWidthCss})`,
        minWidth: `min(${minWidth}px, ${availableWidthCss})`,
        maxWidth: `min(${maxWidth}px, ${maxViewportRatio * 100}%, ${availableWidthCss})`,
      }

  const fullscreenSurface = (maximized || overlay) && !collapsed

  return (
    <div
      ref={setShellRef}
      className={cn(
        "flex min-h-0 flex-col overflow-hidden bg-surface-app",
        fullscreenSurface
          ? "fixed inset-0 z-50"
          : "relative h-full shrink-0 bg-transparent",
        !fullscreenSurface &&
          !isResizing &&
          "transition-[width,min-width,max-width,padding] duration-[250ms] ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width] motion-reduce:transition-none",
        fullscreenSurface
          ? overlay && !maximized
            ? "p-2 sm:p-3"
            : "p-0"
          : visuallyCollapsed
            ? "min-w-0 max-w-0 p-0 pointer-events-none"
            : "p-3 pl-2",
        fullscreenSurface &&
          animateOnMount &&
          "animate-in fade-in-0 slide-in-from-right-2 duration-[250ms] motion-reduce:animate-none",
        surfaceClassName,
      )}
      style={fullscreenSurface ? undefined : panelStyle}
      aria-hidden={visuallyCollapsed ? true : undefined}
      inert={visuallyCollapsed ? true : undefined}
    >
      {!fullscreenSurface && (
        <div
          className={cn(
            "peer absolute left-0 top-3 bottom-3 z-10 w-4",
            onWidthChange && !visuallyCollapsed && "cursor-col-resize",
            visuallyCollapsed && "hidden",
          )}
          onMouseDown={handleDragStart}
          role="separator"
          aria-orientation="vertical"
          aria-label={resizeLabel}
        />
      )}
      <div
        className={cn(
          "flex h-full min-h-0 w-full flex-col overflow-hidden transition-[opacity,transform,border-color,border-radius,box-shadow] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] [contain:layout_paint] motion-reduce:transition-none",
          maximized
            ? "rounded-none border-0 bg-surface-app shadow-none"
            : "rounded-2xl border border-border-soft bg-surface-panel shadow-panel peer-hover:border-l-primary/35",
          isResizing && "border-l-primary/50",
          visuallyCollapsed ? "translate-x-4 opacity-0" : "translate-x-0 opacity-100",
          bodyClassName,
        )}
      >
        <div className="relative flex h-full min-h-0 w-full flex-col overflow-hidden">
          <div
            key={resolvedContentKey}
            className="relative z-10 flex h-full min-h-0 w-full flex-col animate-in fade-in-0 duration-200 motion-reduce:animate-none"
            style={{ animationDuration: `${UI_MOTION.panelContentEnter}ms` }}
          >
            {children}
          </div>
          <div
            className={cn(
              "pointer-events-none absolute inset-0 z-20 bg-surface-panel transition-opacity ease-out motion-reduce:hidden",
              transitionVeilVisible ? "opacity-100" : "opacity-0",
            )}
            style={{ transitionDuration: `${UI_MOTION.panelContentExit}ms` }}
            aria-hidden
          />
        </div>
      </div>
    </div>
  )
}
