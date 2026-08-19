import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { UI_MOTION } from "@/components/ui/motion"
import { PanelVisibilityContext } from "./panelVisibility"

/**
 * Compatibility host for one workbench tab. The workbench frame owns width, the
 * column border, the resize affordance and maximize, so all this still does is
 * stack its content absolutely, cross-fade tab switches, and take the inactive
 * tabs out of the accessibility tree.
 */
interface RightPanelShellProps {
  children: ReactNode
  collapsed?: boolean
  contentKey?: string | number | null
  surfaceClassName?: string
  bodyClassName?: string
  /** Fade the content in when this is the first panel to open. */
  animateOnMount?: boolean
}

export function RightPanelShell({
  children,
  collapsed = false,
  contentKey,
  surfaceClassName,
  bodyClassName,
  animateOnMount = false,
}: RightPanelShellProps) {
  const resolvedContentKey = contentKey ?? "right-panel-content"
  const lastContentKeyRef = useRef<string | number>(resolvedContentKey)
  const transitionTimerRef = useRef<number | null>(null)
  const transitionFrameRef = useRef<number | null>(null)
  const entryFrameRef = useRef<number | null>(null)
  const [transitionVeilVisible, setTransitionVeilVisible] = useState(false)
  const [entryVisible, setEntryVisible] = useState(!animateOnMount)

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
    },
    [],
  )

  const visuallyCollapsed = collapsed || !entryVisible

  return (
    <div
      className={cn(
        "absolute inset-0 flex h-full min-h-0 w-full min-w-0 flex-col overflow-hidden bg-background p-0",
        surfaceClassName,
        visuallyCollapsed && "pointer-events-none",
        // Shells stack absolutely: a collapsed one would otherwise paint over
        // the active panel. Must stay last so `cn` keeps it.
        visuallyCollapsed && "bg-transparent",
      )}
      aria-hidden={visuallyCollapsed ? true : undefined}
      inert={visuallyCollapsed ? true : undefined}
    >
      <div
        className={cn(
          "flex h-full min-h-0 w-full flex-col overflow-hidden rounded-none border-0 bg-background shadow-none transition-[opacity,transform] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] [contain:layout_paint] motion-reduce:transition-none",
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
            {/* Warm-mounted tabs keep their effects; tell them whether anyone
                can actually see the result so pollers can stand down. */}
            <PanelVisibilityContext.Provider value={!visuallyCollapsed}>
              {children}
            </PanelVisibilityContext.Provider>
          </div>
          <div
            className={cn(
              "pointer-events-none absolute inset-0 z-20 bg-background transition-opacity ease-out motion-reduce:hidden",
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
