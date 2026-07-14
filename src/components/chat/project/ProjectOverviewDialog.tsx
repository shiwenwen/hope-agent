/**
 * Project settings sheet (formerly `ProjectOverviewDialog`).
 *
 * Slides in from the right as a non-modal-feeling drawer. Tabs:
 * Overview | Files | Instructions | Auto Memory. The old "Sessions" tab is gone — the
 * sidebar now renders project sessions inline as a nested tree node, so
 * having the same list inside this sheet is redundant.
 *
 * The component is exported under its original name so existing imports in
 * `ChatScreen.tsx` keep working without churn; rename to
 * `ProjectSettingsSheet` is left as a follow-up.
 */

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react"
import { useTranslation } from "react-i18next"
import { Pencil, Trash2, Archive, ArchiveRestore } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { Project, ProjectMeta } from "@/types/project"

import { FileBrowserView } from "./file-browser/FileBrowserView"
import ProjectIcon from "./ProjectIcon"
import { ProjectMemorySection } from "./ProjectMemorySection"
import ProjectInstructionsEditor from "./ProjectInstructionsEditor"

interface ProjectOverviewDialogProps {
  open: boolean
  project: ProjectMeta | null
  onOpenChange: (open: boolean) => void
  onEdit: (project: Project) => void
  onDelete: (project: Project) => void
  onArchive: (project: Project, archived: boolean) => void
  onNewSessionInProject: (projectId: string, defaultAgentId?: string | null) => void
  /**
   * Kept in the API for compatibility but no longer used — the Sessions tab
   * was removed because the sidebar now lists project sessions inline.
   */
  onOpenSession?: (sessionId: string) => void
}

const DEFAULT_SHEET_WIDTH = 860
const MIN_SHEET_WIDTH = 560
const SHEET_VIEWPORT_GUTTER = 48
const SHEET_WIDTH_STORAGE_KEY = "ha:project-settings-sheet-width"

export default function ProjectOverviewDialog({
  open,
  project,
  onOpenChange,
  onEdit,
  onDelete,
  onArchive,
  onNewSessionInProject,
}: ProjectOverviewDialogProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState("overview")
  const [viewportWidth, setViewportWidth] = useState(getViewportWidth)
  const [sheetWidth, setSheetWidth] = useState(readStoredSheetWidth)
  const [resizing, setResizing] = useState(false)
  const sheetWidthRef = useRef(sheetWidth)
  const dragRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null)

  const renderedSheetWidth =
    viewportWidth < 640 ? viewportWidth : clampSheetWidth(sheetWidth, viewportWidth)

  function applySheetWidth(nextWidth: number, persist = false) {
    const next = clampSheetWidth(nextWidth, viewportWidth)
    sheetWidthRef.current = next
    setSheetWidth(next)
    if (persist) storeSheetWidth(next)
  }

  function handleResizePointerDown(event: PointerEvent<HTMLDivElement>) {
    if (viewportWidth < 640) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: renderedSheetWidth,
    }
    setResizing(true)
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"
  }

  function handleResizePointerMove(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) return
    applySheetWidth(drag.startWidth + drag.startX - event.clientX)
  }

  function finishResize(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) return
    dragRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    setResizing(false)
    document.body.style.cursor = ""
    document.body.style.userSelect = ""
    storeSheetWidth(sheetWidthRef.current)
  }

  function handleResizeKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    let next: number | null = null
    if (event.key === "ArrowLeft") next = sheetWidthRef.current + 24
    if (event.key === "ArrowRight") next = sheetWidthRef.current - 24
    if (event.key === "Home") next = MIN_SHEET_WIDTH
    if (event.key === "End") next = viewportWidth - SHEET_VIEWPORT_GUTTER
    if (next === null) return
    event.preventDefault()
    applySheetWidth(next, true)
  }

  useEffect(() => {
    if (!open || !project) return
    setTab("overview")
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  useEffect(() => {
    const handleResize = () => setViewportWidth(getViewportWidth())
    window.addEventListener("resize", handleResize)
    return () => window.removeEventListener("resize", handleResize)
  }, [])

  useEffect(
    () => () => {
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
    },
    [],
  )

  if (!project) return null

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className={resizing ? "flex w-full select-none flex-col p-0" : "flex w-full flex-col p-0"}
        style={{ width: renderedSheetWidth, maxWidth: "none" }}
      >
        <div
          role="separator"
          aria-label={t("project.resizeSettingsSheet")}
          aria-orientation="vertical"
          aria-valuemin={MIN_SHEET_WIDTH}
          aria-valuemax={Math.max(MIN_SHEET_WIDTH, viewportWidth - SHEET_VIEWPORT_GUTTER)}
          aria-valuenow={Math.round(renderedSheetWidth)}
          data-dragging={resizing || undefined}
          tabIndex={0}
          title={t("project.resizeSettingsSheet")}
          onDoubleClick={() => applySheetWidth(DEFAULT_SHEET_WIDTH, true)}
          onKeyDown={handleResizeKeyDown}
          onPointerDown={handleResizePointerDown}
          onPointerMove={handleResizePointerMove}
          onPointerUp={finishResize}
          onPointerCancel={finishResize}
          className="group absolute inset-y-0 left-0 z-20 hidden w-3 -translate-x-1/2 cursor-col-resize touch-none items-center justify-center outline-none sm:flex"
        >
          <span className="h-full w-px bg-transparent transition-colors group-hover:bg-primary/50 group-focus-visible:bg-primary group-data-[dragging=true]:bg-primary" />
        </div>
        <SheetHeader className="px-5 pt-5 pb-3 border-b border-border">
          <div className="flex items-start gap-3">
            <ProjectIcon project={project} size="lg" />
            <div className="flex-1 min-w-0 pt-0.5">
              <SheetTitle className="truncate">{project.name}</SheetTitle>
              {project.description && (
                <SheetDescription className="line-clamp-2">{project.description}</SheetDescription>
              )}
            </div>
            <div className="flex items-center gap-0.5 mr-7">
              <IconTip label={t("common.edit")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onEdit(project)}
                  className="h-8 w-8 p-0"
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
              <IconTip
                label={
                  project.archived ? t("project.unarchiveProject") : t("project.archiveProject")
                }
              >
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onArchive(project, !project.archived)}
                  className="h-8 w-8 p-0 text-muted-foreground"
                >
                  {project.archived ? (
                    <ArchiveRestore className="h-3.5 w-3.5" />
                  ) : (
                    <Archive className="h-3.5 w-3.5" />
                  )}
                </Button>
              </IconTip>
              <IconTip label={t("common.delete")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onDelete(project)}
                  className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            </div>
          </div>
        </SheetHeader>

        <Tabs value={tab} onValueChange={setTab} className="flex-1 flex flex-col overflow-hidden">
          <TabsList className="shrink-0 mx-5 mt-3 self-start">
            <TabsTrigger value="overview">{t("project.tabOverview")}</TabsTrigger>
            <TabsTrigger value="files">{t("project.tabFiles")}</TabsTrigger>
            <TabsTrigger value="instructions">{t("project.tabInstructions")}</TabsTrigger>
            <TabsTrigger value="auto-memory">{t("project.tabAutoMemory")}</TabsTrigger>
          </TabsList>

          {/* Overview */}
          <TabsContent value="overview" className="flex-1 overflow-y-auto px-5 py-3 space-y-4">
            <div className="grid grid-cols-2 gap-3">
              <StatCard label={t("project.overview.totalSessions")} value={project.sessionCount} />
              <StatCard label={t("project.overview.totalMemories")} value={project.memoryCount} />
            </div>

            {!project.archived && (
              <Button
                onClick={() => {
                  onNewSessionInProject(project.id, project.defaultAgentId)
                  onOpenChange(false)
                }}
                className="w-full"
              >
                {t("project.newChatInProject")}
              </Button>
            )}
          </TabsContent>

          {/* Files */}
          <TabsContent value="files" className="flex-1 overflow-hidden p-0">
            <FileBrowserView
              scope="project"
              scopeId={project.id}
              rootPath={project.workingDir ?? project.id}
              editable
              layout="split"
              className="h-full"
            />
          </TabsContent>

          {/* Instructions */}
          <TabsContent
            value="instructions"
            forceMount
            className="min-h-0 flex-1 overflow-hidden px-5 py-3 data-[state=inactive]:hidden"
          >
            <ProjectInstructionsEditor projectId={project.id} />
          </TabsContent>

          {/* Project auto memory: bounded index + on-demand topic files. */}
          <TabsContent value="auto-memory" className="flex-1 overflow-hidden p-0">
            <ProjectMemorySection projectId={project.id} />
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

function StatCard({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border border-border/60 bg-accent/20 px-3 py-3">
      <div className="text-2xl font-semibold">{value}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  )
}

function getViewportWidth(): number {
  return typeof window === "undefined" ? DEFAULT_SHEET_WIDTH : window.innerWidth
}

function clampSheetWidth(width: number, viewportWidth: number): number {
  const max = Math.max(MIN_SHEET_WIDTH, viewportWidth - SHEET_VIEWPORT_GUTTER)
  return Math.min(Math.max(width, MIN_SHEET_WIDTH), max)
}

function readStoredSheetWidth(): number {
  if (typeof window === "undefined") return DEFAULT_SHEET_WIDTH
  try {
    const stored = Number(window.localStorage.getItem(SHEET_WIDTH_STORAGE_KEY))
    return Number.isFinite(stored) && stored > 0 ? stored : DEFAULT_SHEET_WIDTH
  } catch {
    return DEFAULT_SHEET_WIDTH
  }
}

function storeSheetWidth(width: number) {
  try {
    window.localStorage.setItem(SHEET_WIDTH_STORAGE_KEY, String(Math.round(width)))
  } catch {
    // Storage may be disabled; resizing still works for the current mount.
  }
}
