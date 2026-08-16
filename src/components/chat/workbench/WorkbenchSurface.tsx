import type { ReactNode } from "react"
import { UI_EASING, UI_MOTION } from "@/components/ui/motion"
import { cn } from "@/lib/utils"
import type { WorkbenchLayoutMode } from "./types"

interface WorkbenchSurfaceProps {
  width: number
  layoutMode: WorkbenchLayoutMode
  collapsed?: boolean
  resizing?: boolean
  maximized?: boolean
  children: ReactNode
}

export function WorkbenchSurface({
  width,
  layoutMode,
  collapsed = false,
  resizing = false,
  maximized = false,
  children,
}: WorkbenchSurfaceProps) {
  return (
    <section
      className={cn(
        // Same ground as the conversation column (and the shared title bar):
        // the workbench is one half of the same window, not a floating panel,
        // so it must not carry the cooler `surface-app` tint beside it.
        // `border-t` closes the tab strip against its content: with the tab row
        // and the panel sharing one ground, nothing else marks where the tab
        // ends and the panel begins.
        "relative flex h-full min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-l border-t border-border-soft bg-background",
        !resizing && "transition-[width,opacity,border-color] motion-reduce:transition-none",
        layoutMode === "stage" && "border-l-0",
        collapsed && "pointer-events-none border-l-transparent border-t-transparent opacity-0",
        maximized && "fixed inset-x-0 bottom-0 top-[72px] z-50 h-auto border-l-0",
      )}
      style={{
        width: maximized ? "100%" : collapsed ? 0 : width,
        transitionDuration: !resizing ? `${UI_MOTION.panelWidth}ms` : undefined,
        transitionTimingFunction: !resizing ? UI_EASING.emphasized : undefined,
      }}
      aria-hidden={collapsed ? true : undefined}
      inert={collapsed ? true : undefined}
    >
      <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden">{children}</div>
    </section>
  )
}
