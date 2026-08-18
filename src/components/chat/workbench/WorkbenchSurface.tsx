import type { ReactNode } from "react"
import { UI_EASING, UI_MOTION } from "@/components/ui/motion"
import { cn } from "@/lib/utils"
import type { WorkbenchLayoutMode } from "./types"
import { WORKBENCH_MAXIMIZED_BODY_TOP } from "./useWorkbenchSizing"

interface WorkbenchSurfaceProps {
  width: number
  layoutMode: WorkbenchLayoutMode
  collapsed?: boolean
  resizing?: boolean
  maximized?: boolean
  /** No panel is open. The surface stays mounted — panels that own their own
   *  open signal (Canvas listens for `canvas_show`) must keep listening — but
   *  takes no layout, not even the 1px column border. */
  empty?: boolean
  children: ReactNode
}

export function WorkbenchSurface({
  width,
  layoutMode,
  collapsed = false,
  resizing = false,
  maximized = false,
  empty = false,
  children,
}: WorkbenchSurfaceProps) {
  return (
    <section
      className={cn(
        // Same ground as the conversation column; `border-t` closes the tab
        // strip against its content.
        "relative flex h-full min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-l border-t border-border-soft bg-background",
        !resizing && "transition-[width,opacity,border-color] motion-reduce:transition-none",
        layoutMode === "stage" && "border-l-0",
        collapsed && "pointer-events-none border-l-transparent border-t-transparent opacity-0",
        maximized &&
          cn("fixed inset-x-0 bottom-0 z-50 h-auto border-l-0", WORKBENCH_MAXIMIZED_BODY_TOP),
        empty && "hidden",
      )}
      style={{
        width: maximized ? "100%" : collapsed ? 0 : width,
        transitionDuration: !resizing ? `${UI_MOTION.panelWidth}ms` : undefined,
        transitionTimingFunction: !resizing ? UI_EASING.emphasized : undefined,
      }}
      aria-hidden={collapsed || empty ? true : undefined}
      inert={collapsed || empty ? true : undefined}
    >
      <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden">{children}</div>
    </section>
  )
}
