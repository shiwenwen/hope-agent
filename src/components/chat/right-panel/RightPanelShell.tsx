import { useCallback, useRef, type CSSProperties, type ReactNode } from "react"
import { cn } from "@/lib/utils"

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
  contentKey?: string | number | null
  surfaceClassName?: string
  bodyClassName?: string
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
  contentKey,
  surfaceClassName,
  bodyClassName,
}: RightPanelShellProps) {
  const shellRef = useRef<HTMLDivElement>(null)

  const handleDragStart = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (!onWidthChange || collapsed) return
      e.preventDefault()
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
      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove)
        document.removeEventListener("mouseup", onMouseUp)
        document.body.style.cursor = ""
        document.body.style.userSelect = ""
        iframes.forEach((frame) => ((frame as HTMLElement).style.pointerEvents = ""))
      }
      document.addEventListener("mousemove", onMouseMove)
      document.addEventListener("mouseup", onMouseUp)
      document.body.style.cursor = "col-resize"
      document.body.style.userSelect = "none"
    },
    [collapsed, maxViewportRatio, maxWidth, minWidth, onWidthChange, reservedMainWidth, width],
  )

  const availableWidthCss = `max(0px, calc(100% - ${reservedMainWidth}px))`
  const panelStyle: CSSProperties = collapsed
    ? { width: 0, minWidth: 0, maxWidth: 0 }
    : {
        width: `min(${width}px, ${availableWidthCss})`,
        minWidth: `min(${minWidth}px, ${availableWidthCss})`,
        maxWidth: `min(${maxWidth}px, ${maxViewportRatio * 100}%, ${availableWidthCss})`,
      }

  if (maximized && !collapsed) {
    return (
      <div
        className={cn(
          "fixed inset-0 z-50 flex min-h-0 flex-col overflow-hidden bg-surface-app",
          surfaceClassName,
        )}
      >
        {children}
      </div>
    )
  }

  return (
    <div
      ref={shellRef}
      className={cn(
        "relative flex h-full min-h-0 shrink-0 overflow-hidden transition-[width,min-width,max-width,padding,opacity,transform] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width,opacity,transform] motion-reduce:transition-none",
        collapsed
          ? "min-w-0 max-w-0 translate-x-2 p-0 opacity-0 pointer-events-none"
          : "translate-x-0 p-3 pl-2 opacity-100",
      )}
      style={panelStyle}
      aria-hidden={collapsed ? true : undefined}
      inert={collapsed ? true : undefined}
    >
      <div
        className={cn(
          "group absolute left-0 top-3 bottom-3 z-10 flex w-3 items-center justify-center",
          onWidthChange && !collapsed && "cursor-col-resize",
          collapsed && "hidden",
        )}
        onMouseDown={handleDragStart}
        role="separator"
        aria-orientation="vertical"
        aria-label={resizeLabel}
      >
        <div className="h-full w-px rounded-full bg-transparent transition-colors group-hover:bg-primary/35 group-active:bg-primary/50" />
      </div>
      <div
        className={cn(
          "flex h-full min-h-0 w-full flex-col overflow-hidden rounded-panel border border-border-soft bg-surface-panel shadow-panel",
          bodyClassName,
        )}
      >
        <div
          key={contentKey ?? "right-panel-content"}
          className="flex h-full min-h-0 w-full flex-col animate-in fade-in-0 slide-in-from-right-1 duration-150 motion-reduce:animate-none"
        >
          {children}
        </div>
      </div>
    </div>
  )
}
