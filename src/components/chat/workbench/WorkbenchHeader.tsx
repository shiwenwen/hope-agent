import { useEffect, useMemo, useRef, type DragEvent, type KeyboardEvent } from "react"
import { Maximize2, Minimize2, PanelRightClose, PanelRightOpen, Plus, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { IconTip } from "@/components/ui/tooltip"
import { UI_EASING, UI_MOTION } from "@/components/ui/motion"
import { WindowModeIcon } from "@/components/common/WindowModeIcon"
import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import { cn } from "@/lib/utils"
import { TITLE_BAR_ICON_BUTTON } from "../titleBarStyles"
import type { WorkbenchLayoutMode, WorkbenchPanelId, WorkbenchTabItem } from "./types"

interface WorkbenchHeaderProps {
  width: number
  layoutMode: WorkbenchLayoutMode
  collapsed?: boolean
  resizing?: boolean
  maximized?: boolean
  tabs: WorkbenchTabItem[]
  launchItems: WorkbenchTabItem[]
  activeId: string | null
  onSelect: (id: string) => void
  onOpen: (id: WorkbenchPanelId) => void
  onCloseTab: (id: string) => void
  onReorder: (source: string, target: string) => void
  onToggleTabWindow?: (id: string) => void
  onToggleMaximize?: () => void
  onCloseWorkbench: () => void
}

function badgeClass(tone: "attention" | "running" | "neutral"): string {
  if (tone === "attention") return "bg-amber-500 text-white"
  if (tone === "running") return "bg-blue-500 text-white"
  return "bg-muted-foreground text-background"
}

export function WorkbenchHeader({
  width,
  layoutMode,
  collapsed = false,
  resizing = false,
  maximized = false,
  tabs,
  launchItems,
  activeId,
  onSelect,
  onOpen,
  onCloseTab,
  onReorder,
  onToggleTabWindow,
  onToggleMaximize,
  onCloseWorkbench,
}: WorkbenchHeaderProps) {
  const { t } = useTranslation()
  const draggedIdRef = useRef<string | null>(null)
  const tabIds = useMemo(() => tabs.map((tab) => tab.id), [tabs])

  useEffect(() => {
    if (!activeId || collapsed) return
    const activeTab = document.querySelector<HTMLElement>(
      `[data-workbench-tab="${CSS.escape(activeId)}"]`,
    )
    activeTab?.scrollIntoView?.({ block: "nearest", inline: "nearest" })
  }, [activeId, collapsed])

  const handleTabKeyDown = (event: KeyboardEvent<HTMLDivElement>, id: string) => {
    const index = tabIds.indexOf(id)
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault()
      onSelect(id)
      return
    }
    if (event.key === "Delete") {
      event.preventDefault()
      onCloseTab(id)
      return
    }
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return
    event.preventDefault()
    const delta = event.key === "ArrowLeft" ? -1 : 1
    const nextId = tabIds[(index + delta + tabIds.length) % tabIds.length]
    if (event.altKey && event.shiftKey) {
      onReorder(id, nextId)
      return
    }
    const next = document.querySelector<HTMLElement>(`[data-workbench-tab="${CSS.escape(nextId)}"]`)
    next?.focus()
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>, target: string) => {
    event.preventDefault()
    const source = draggedIdRef.current
    draggedIdRef.current = null
    if (source && source !== target) onReorder(source, target)
  }

  return (
    <div
      className={cn(
        "flex h-10 min-w-0 shrink-0 items-end overflow-hidden border-l border-border-soft bg-background",
        !resizing && "transition-[width,opacity,border-color] motion-reduce:transition-none",
        layoutMode === "stage" && "border-l-0",
        collapsed && "pointer-events-none border-l-transparent opacity-0",
        maximized &&
          "fixed inset-x-0 top-0 z-[60] h-[72px] border-l-0 bg-background pt-7 shadow-sm",
      )}
      style={{
        width: maximized ? "100%" : collapsed ? 0 : width,
        transitionDuration: !resizing ? `${UI_MOTION.panelWidth}ms` : undefined,
        transitionTimingFunction: !resizing ? UI_EASING.emphasized : undefined,
      }}
      aria-hidden={collapsed ? true : undefined}
      inert={collapsed ? true : undefined}
      data-tauri-drag-region
    >
      <div
        className="flex h-9 min-w-0 flex-1 items-end gap-0.5 overflow-x-auto px-1 pb-1"
        role="tablist"
        aria-label={t("chat.rightPanel.dock")}
        // The strip fills the title-bar row, so it needs its own drag region.
        data-tauri-drag-region
      >
        {tabs.map((tab) => {
          const Icon = tab.icon
          const active = tab.id === activeId
          return (
            <div
              key={tab.id}
              data-workbench-tab={tab.id}
              role="tab"
              tabIndex={active ? 0 : -1}
              aria-selected={active}
              draggable
              onDragStart={() => {
                draggedIdRef.current = tab.id
              }}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => handleDrop(event, tab.id)}
              onClick={() => onSelect(tab.id)}
              onAuxClick={(event) => {
                if (event.button === 1) onCloseTab(tab.id)
              }}
              onKeyDown={(event) => handleTabKeyDown(event, tab.id)}
              className={cn(
                "group flex h-7 max-w-[180px] shrink-0 cursor-default items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground transition-colors",
                active
                  ? "bg-secondary text-foreground"
                  : "hover:bg-secondary/40 hover:text-foreground",
              )}
            >
              {tab.fileIcon ? (
                <FileTypeIcon
                  name={tab.fileIcon.name}
                  mime={tab.fileIcon.mime}
                  className="h-3.5 w-3.5 shrink-0"
                />
              ) : (
                <Icon className="h-3.5 w-3.5 shrink-0" />
              )}
              <span className="min-w-0 flex-1 truncate">
                {tab.label ?? (tab.labelKey ? t(tab.labelKey) : tab.id)}
              </span>
              {tab.badge && tab.badge.count > 0 && (
                <span
                  className={cn(
                    "flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-1 text-[9px] font-semibold leading-none tabular-nums",
                    badgeClass(tab.badge.tone),
                  )}
                  aria-label={t(tab.badge.labelKey, { count: tab.badge.count })}
                >
                  {tab.badge.count > 99 ? "99+" : tab.badge.count}
                </span>
              )}
              {tab.windowMode && onToggleTabWindow && (
                <button
                  type="button"
                  tabIndex={-1}
                  className={cn(
                    "flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground",
                    !active && "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
                  )}
                  aria-label={
                    tab.windowMode === "docked"
                      ? t("chat.controlPanel.floatWindow")
                      : t("fileBrowser.reattach", "Reattach")
                  }
                  onClick={(event) => {
                    event.stopPropagation()
                    onToggleTabWindow(tab.id)
                  }}
                >
                  <WindowModeIcon
                    action={tab.windowMode === "docked" ? "detach" : "reattach"}
                    className="h-3 w-3"
                  />
                </button>
              )}
              <button
                type="button"
                tabIndex={-1}
                className={cn(
                  "-mr-1 flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground",
                  !active && "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
                )}
                aria-label={t("common.close")}
                onClick={(event) => {
                  event.stopPropagation()
                  onCloseTab(tab.id)
                }}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          )
        })}
      </div>

      <div className="flex h-9 shrink-0 items-center gap-0.5 px-1 pb-0.5">
        <DropdownMenu>
          <IconTip label={t("chat.rightPanel.dock")}>
            <DropdownMenuTrigger asChild>
              <Button type="button" variant="ghost" size="icon" className="h-7 w-7">
                <Plus className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
          </IconTip>
          <DropdownMenuContent align="end" variant="floating" className="min-w-[200px]">
            {launchItems.map((item) => {
              const Icon = item.icon
              return (
                <DropdownMenuItem key={item.id} onSelect={() => onOpen(item.panelId)}>
                  <Icon className="mr-2 h-4 w-4" />
                  <span className="min-w-0 flex-1 truncate">
                    {item.label ?? (item.labelKey ? t(item.labelKey) : item.id)}
                  </span>
                </DropdownMenuItem>
              )
            })}
          </DropdownMenuContent>
        </DropdownMenu>
        {onToggleMaximize && (
          <IconTip
            label={
              maximized
                ? t("fileBrowser.minimize", "Restore")
                : t("fileBrowser.maximize", "Maximize")
            }
          >
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={onToggleMaximize}
              aria-label={
                maximized
                  ? t("fileBrowser.minimize", "Restore")
                  : t("fileBrowser.maximize", "Maximize")
              }
            >
              {maximized ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}
            </Button>
          </IconTip>
        )}
        <IconTip label={t("chat.rightPanel.collapse")}>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={onCloseWorkbench}
            aria-label={t("chat.rightPanel.collapse")}
          >
            <PanelRightClose className="h-4 w-4" />
          </Button>
        </IconTip>
      </div>
    </div>
  )
}

export function WorkbenchOpenButton({
  tabs,
  launchItems,
  onOpen,
  onOpenPanel,
}: {
  tabs: WorkbenchTabItem[]
  launchItems: WorkbenchTabItem[]
  onOpen: () => void
  onOpenPanel: (id: WorkbenchPanelId) => void
}) {
  const { t } = useTranslation()
  const badgeCount = tabs.reduce((sum, tab) => sum + (tab.badge?.count ?? 0), 0)
  if (tabs.length === 0) {
    return (
      <DropdownMenu>
        <IconTip label={t("chat.rightPanel.dock")}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className={cn("relative", TITLE_BAR_ICON_BUTTON)}
              aria-label={t("chat.rightPanel.dock")}
            >
              <PanelRightOpen className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
        </IconTip>
        <DropdownMenuContent align="end" variant="floating" className="min-w-[200px]">
          {launchItems.map((item) => {
            const Icon = item.icon
            return (
              <DropdownMenuItem key={item.id} onSelect={() => onOpenPanel(item.panelId)}>
                <Icon className="mr-2 h-4 w-4" />
                <span className="min-w-0 flex-1 truncate">
                  {item.label ?? (item.labelKey ? t(item.labelKey) : item.id)}
                </span>
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuContent>
      </DropdownMenu>
    )
  }
  return (
    <IconTip label={t("chat.rightPanel.expand")}>
      <button
        type="button"
        className={cn("relative", TITLE_BAR_ICON_BUTTON)}
        aria-label={t("chat.rightPanel.expand")}
        onClick={onOpen}
      >
        <PanelRightOpen className="h-4 w-4" />
        {badgeCount > 0 && (
          <span className="absolute -right-1 -top-1 flex h-[15px] min-w-[15px] items-center justify-center rounded-full bg-blue-500 px-0.5 text-[9px] font-semibold leading-none text-white">
            {badgeCount > 99 ? "99+" : badgeCount}
          </span>
        )}
      </button>
    </IconTip>
  )
}
