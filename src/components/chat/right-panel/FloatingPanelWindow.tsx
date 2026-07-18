import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { PanelRight, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { useFloatingWindow, type ResizeEdge } from "@/hooks/useFloatingWindow"

const RESIZE_EDGES: Array<{ edge: ResizeEdge; className: string }> = [
  { edge: "n", className: "left-2 right-2 -top-0.5 h-1.5 cursor-ns-resize" },
  { edge: "s", className: "left-2 right-2 -bottom-0.5 h-1.5 cursor-ns-resize" },
  { edge: "e", className: "top-2 bottom-2 -right-0.5 w-1.5 cursor-ew-resize" },
  { edge: "w", className: "top-2 bottom-2 -left-0.5 w-1.5 cursor-ew-resize" },
  { edge: "ne", className: "-right-0.5 -top-0.5 h-2.5 w-2.5 cursor-nesw-resize" },
  { edge: "nw", className: "-left-0.5 -top-0.5 h-2.5 w-2.5 cursor-nwse-resize" },
  { edge: "se", className: "-bottom-0.5 -right-0.5 h-2.5 w-2.5 cursor-nwse-resize" },
  { edge: "sw", className: "-bottom-0.5 -left-0.5 h-2.5 w-2.5 cursor-nesw-resize" },
]

interface FloatingPanelWindowProps {
  storageKey: string
  icon: ReactNode
  title: string
  zIndex: number
  onDock: () => void
  onClose: () => void
  onFocus: () => void
  children: ReactNode
}

/**
 * In-app floating window shell: draggable title bar, 8-way resize handles,
 * viewport clamping and per-panel rect persistence via `useFloatingWindow`.
 * Deliberately z-40..49 — dialogs and fullscreen overlays (z-50) cover it.
 */
export function FloatingPanelWindow({
  storageKey,
  icon,
  title,
  zIndex,
  onDock,
  onClose,
  onFocus,
  children,
}: FloatingPanelWindowProps) {
  const { t } = useTranslation()
  const { gesture, rootRef, rootStyle, handleTitlePointerDown, handleResizePointerDown } =
    useFloatingWindow({
      storageKey,
      defaultRect: () => ({
        x: window.innerWidth - 400 - 24,
        y: window.innerHeight - 320 - 24,
        width: 400,
        height: 320,
      }),
    })

  return (
    <div
      ref={rootRef}
      style={{ ...rootStyle, zIndex }}
      onPointerDownCapture={onFocus}
      className={cn(
        "flex flex-col overflow-hidden rounded-xl border border-border/70 bg-background shadow-2xl",
        !gesture && "animate-in fade-in-0 zoom-in-95 duration-200",
        gesture && "transition-none",
      )}
    >
      <div
        onPointerDown={handleTitlePointerDown}
        onDoubleClick={onDock}
        className="flex shrink-0 cursor-grab touch-none select-none items-center gap-2 border-b border-border/60 bg-muted/40 px-3 py-1.5 active:cursor-grabbing"
      >
        {icon}
        <div className="min-w-0 flex-1 truncate text-xs font-medium">{title}</div>
        <IconTip label={t("chat.controlPanel.dockPanel")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={onDock}
          >
            <PanelRight className="h-3 w-3" />
          </Button>
        </IconTip>
        <IconTip label={t("chat.browserPanel.close")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={onClose}
          >
            <X className="h-3 w-3" />
          </Button>
        </IconTip>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">{children}</div>

      {RESIZE_EDGES.map(({ edge, className }) => (
        <div
          key={edge}
          role="separator"
          aria-label={t("chat.controlPanel.resizeWindow")}
          onPointerDown={handleResizePointerDown(edge)}
          className={cn("absolute z-10 touch-none", className)}
        />
      ))}
    </div>
  )
}
