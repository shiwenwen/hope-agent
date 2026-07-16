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

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react"
import { useTranslation } from "react-i18next"
import {
  Archive,
  ArchiveRestore,
  ArrowRight,
  Brain,
  FileCode2,
  FolderOpen,
  MessageSquare,
  Pencil,
  Plus,
  Sparkles,
  Trash2,
} from "lucide-react"

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
import { formatBytes } from "@/lib/format"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { Project, ProjectMeta, ProjectOverviewSummary } from "@/types/project"
import type { SessionMeta } from "@/types/chat"

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
  onOpenSession?: (sessionId: string) => void
  onOpenStructuredMemory?: (projectId: string) => void
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
  onOpenSession,
  onOpenStructuredMemory,
}: ProjectOverviewDialogProps) {
  const { t, i18n } = useTranslation()
  const [tab, setTab] = useState("overview")
  const [viewportWidth, setViewportWidth] = useState(getViewportWidth)
  const [sheetWidth, setSheetWidth] = useState(readStoredSheetWidth)
  const [resizing, setResizing] = useState(false)
  const [overview, setOverview] = useState<ProjectOverviewSummary | null>(null)
  const [overviewLoading, setOverviewLoading] = useState(false)
  const [overviewError, setOverviewError] = useState(false)
  const sheetWidthRef = useRef(sheetWidth)
  const recentSessionsRef = useRef<HTMLDivElement>(null)
  const overviewRequestRef = useRef(0)
  const overviewReloadTimerRef = useRef<number | null>(null)
  const dragRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null)

  const renderedSheetWidth =
    viewportWidth < 640 ? viewportWidth : clampSheetWidth(sheetWidth, viewportWidth)
  const wideOverview = renderedSheetWidth >= 760

  const loadOverview = useCallback(async () => {
    if (!open || !project) return
    const request = ++overviewRequestRef.current
    setOverviewLoading(true)
    try {
      const result = await getTransport().call<ProjectOverviewSummary>(
        "get_project_overview_cmd",
        { id: project.id },
      )
      if (request !== overviewRequestRef.current) return
      setOverview(result)
      setOverviewError(false)
    } catch (error) {
      if (request !== overviewRequestRef.current) return
      logger.error("project", "ProjectOverviewDialog::loadOverview", "load failed", error)
      setOverviewError(true)
    } finally {
      if (request === overviewRequestRef.current) setOverviewLoading(false)
    }
  }, [open, project])

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
    setOverview(null)
    setOverviewError(false)
    void loadOverview()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  useEffect(() => {
    if (!open || !project) return
    const transport = getTransport()
    const scheduleReload = () => {
      if (overviewReloadTimerRef.current !== null) {
        window.clearTimeout(overviewReloadTimerRef.current)
      }
      overviewReloadTimerRef.current = window.setTimeout(() => {
        overviewReloadTimerRef.current = null
        void loadOverview()
      }, 120)
    }
    const reloadProjectMemory = (payload: unknown) => {
      const event = payload as { projectId?: string } | null
      if (event?.projectId === project.id) scheduleReload()
    }
    const reloadClaim = (payload: unknown) => {
      const event = payload as { scopeType?: string; scopeId?: string } | null
      if (event?.scopeType === "project" && event.scopeId && event.scopeId !== project.id) return
      scheduleReload()
    }
    const reloadProject = (payload: unknown) => {
      const event = payload as { projectId?: string } | null
      if (event?.projectId === project.id) scheduleReload()
    }
    const reloadProjectFile = (payload: unknown) => {
      const event = payload as {
        scope?: string
        scopeId?: string
        projectId?: string
      } | null
      if (!event) return
      const directProjectChange = event.scope === "project" && event.scopeId === project.id
      const projectSessionChange = event.scope === "session" && event.projectId === project.id
      if (directProjectChange || projectSessionChange) scheduleReload()
    }
    const unsubs = [
      transport.listen("project_memory:changed", reloadProjectMemory),
      transport.listen("memory:claim_changed", reloadClaim),
      transport.listen("project:updated", reloadProject),
      transport.listen("project:fs_changed", reloadProjectFile),
    ]
    return () => {
      unsubs.forEach((unsubscribe) => unsubscribe())
      if (overviewReloadTimerRef.current !== null) {
        window.clearTimeout(overviewReloadTimerRef.current)
        overviewReloadTimerRef.current = null
      }
    }
  }, [loadOverview, open, project])

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
          data-ha-title-tip={t("project.resizeSettingsSheet")}
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
          <TabsContent value="overview" className="flex-1 overflow-y-auto px-5 py-4">
            <div className="space-y-5">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-base font-semibold">{t("project.overview.homeTitle")}</h3>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    {t("project.overview.homeDescription")}
                  </p>
                </div>
                {!project.archived && (
                  <Button
                    size="sm"
                    onClick={() => {
                      onNewSessionInProject(project.id, project.defaultAgentId)
                      onOpenChange(false)
                    }}
                  >
                    <Plus className="mr-1.5 h-4 w-4" />
                    {t("project.newChatInProject")}
                  </Button>
                )}
              </div>

              {overviewLoading && !overview ? (
                <OverviewSkeleton wide={wideOverview} />
              ) : (
                <>
                  {overviewError && !overview && (
                    <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                      {t("project.overview.loadFailed")}
                    </div>
                  )}
                  <div className={`grid gap-3 ${wideOverview ? "grid-cols-4" : "grid-cols-2"}`}>
                    <StatCard
                      icon={MessageSquare}
                      label={t("project.overview.totalSessions")}
                      value={overview?.sessionCount}
                      onClick={() =>
                        recentSessionsRef.current?.scrollIntoView({
                          behavior: "smooth",
                          block: "start",
                        })
                      }
                    />
                    <StatCard
                      icon={Sparkles}
                      label={t("project.overview.autoMemoryTopics")}
                      value={overview?.autoMemoryTopicCount}
                      onClick={() => setTab("auto-memory")}
                    />
                    <StatCard
                      icon={Brain}
                      label={t("project.overview.activeClaims")}
                      value={overview?.activeClaimCount}
                      onClick={() => onOpenStructuredMemory?.(project.id)}
                    />
                    <StatCard
                      icon={FileCode2}
                      label={t("project.overview.agentsLines")}
                      value={overview?.instructions?.lineCount}
                      suffix={t("project.overview.linesUnit")}
                      onClick={() => setTab("instructions")}
                    />
                  </div>
                </>
              )}

              <div
                className={`grid items-start gap-4 ${wideOverview ? "grid-cols-[minmax(0,1.45fr)_minmax(260px,0.8fr)]" : "grid-cols-1"}`}
              >
                <div ref={recentSessionsRef} className="scroll-mt-4 rounded-xl border border-border/70">
                  <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
                    <div>
                      <h4 className="text-sm font-semibold">{t("project.overview.recentSessions")}</h4>
                      <p className="mt-0.5 text-[11px] text-muted-foreground">
                        {t("project.overview.recentSessionsHint")}
                      </p>
                    </div>
                    {overviewLoading && overview && (
                      <span className="h-2 w-2 animate-pulse rounded-full bg-primary" />
                    )}
                  </div>
                  <RecentSessions
                    sessions={overview?.recentSessions ?? []}
                    loading={overviewLoading && !overview}
                    error={overviewError && !overview}
                    archived={project.archived}
                    locale={i18n.language}
                    onCreate={() => {
                      onNewSessionInProject(project.id, project.defaultAgentId)
                      onOpenChange(false)
                    }}
                    onOpen={(sessionId) => {
                      if (!onOpenSession) return
                      onOpenSession?.(sessionId)
                      onOpenChange(false)
                    }}
                    t={t}
                  />
                </div>

                <ProjectContextCard
                  loading={overviewLoading && !overview}
                  project={project}
                  overview={overview}
                  onOpenFiles={() => setTab("files")}
                  onOpenInstructions={() => setTab("instructions")}
                  onOpenAutoMemory={() => setTab("auto-memory")}
                  t={t}
                />
              </div>
            </div>
          </TabsContent>

          {/* Files */}
          <TabsContent value="files" className="flex-1 overflow-hidden p-0">
            <FileBrowserView
              scope="project"
              scopeId={project.id}
              rootPath={project.workingDir ?? project.id}
              editable={!project.archived}
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
            <ProjectInstructionsEditor projectId={project.id} readOnly={project.archived} />
          </TabsContent>

          {/* Project auto memory: bounded index + on-demand topic files. */}
          <TabsContent value="auto-memory" className="flex-1 overflow-hidden p-0">
            <ProjectMemorySection projectId={project.id} readOnly={project.archived} />
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

function StatCard({
  icon: Icon,
  label,
  value,
  suffix,
  onClick,
}: {
  icon: typeof MessageSquare
  label: string
  value?: number | null
  suffix?: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group rounded-xl border border-border/70 bg-accent/15 px-3 py-3 text-left transition-colors hover:bg-secondary/40"
    >
      <div className="flex items-start justify-between gap-2">
        <Icon className="h-4 w-4 text-muted-foreground" />
        <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/60" />
      </div>
      <div className="mt-3 text-2xl font-semibold tabular-nums">
        {value ?? "—"}
        {value !== undefined && value !== null && suffix && (
          <span className="ml-1 text-xs font-normal text-muted-foreground">{suffix}</span>
        )}
      </div>
      <div className="mt-0.5 text-xs text-muted-foreground">{label}</div>
    </button>
  )
}

function RecentSessions({
  sessions,
  loading,
  error,
  archived,
  locale,
  onCreate,
  onOpen,
  t,
}: {
  sessions: SessionMeta[]
  loading: boolean
  error: boolean
  archived: boolean
  locale: string
  onCreate: () => void
  onOpen: (sessionId: string) => void
  t: ReturnType<typeof useTranslation>["t"]
}) {
  if (loading) {
    return (
      <div className="space-y-2 p-3">
        {[0, 1, 2].map((item) => (
          <div key={item} className="h-14 animate-pulse rounded-lg bg-muted/60" />
        ))}
      </div>
    )
  }
  if (error) {
    return (
      <div className="flex min-h-32 items-center justify-center px-4 py-8 text-center text-xs text-muted-foreground">
        {t("project.overview.loadFailed")}
      </div>
    )
  }
  if (sessions.length === 0) {
    return (
      <div className="flex min-h-40 flex-col items-center justify-center px-4 py-8 text-center">
        <MessageSquare className="h-7 w-7 text-muted-foreground/50" />
        <p className="mt-3 text-sm font-medium">{t("project.overview.noRecentSessions")}</p>
        <p className="mt-1 text-xs text-muted-foreground">
          {t("project.overview.noRecentSessionsHint")}
        </p>
        {!archived && (
          <Button size="sm" variant="outline" className="mt-4" onClick={onCreate}>
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {t("project.newChatInProject")}
          </Button>
        )}
      </div>
    )
  }
  const formatter = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })
  return (
    <div className="p-2">
      {sessions.map((session) => (
        <button
          key={session.id}
          type="button"
          onClick={() => onOpen(session.id)}
          className="group flex w-full items-center gap-3 rounded-lg px-2.5 py-2.5 text-left hover:bg-accent"
        >
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground group-hover:text-primary">
            <MessageSquare className="h-4 w-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">
              {session.title || t("project.overview.untitledSession")}
            </span>
            <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">
              {safeFormatDate(session.updatedAt, formatter)} · {t("project.overview.messageCount", {
                count: session.messageCount,
              })}
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-1.5">
            {session.pendingInteractionCount > 0 && (
              <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] text-amber-700 dark:text-amber-300">
                {t("project.overview.awaitingResponse")}
              </span>
            )}
            {session.unreadCount > 0 && (
              <>
                <span aria-hidden="true" className="h-2.5 w-2.5 rounded-full bg-destructive" />
                <span className="sr-only">{t("chat.unreadStatus")}</span>
              </>
            )}
            <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/60 group-hover:text-primary" />
          </span>
        </button>
      ))}
    </div>
  )
}

function ProjectContextCard({
  loading,
  project,
  overview,
  onOpenFiles,
  onOpenInstructions,
  onOpenAutoMemory,
  t,
}: {
  loading: boolean
  project: ProjectMeta
  overview: ProjectOverviewSummary | null
  onOpenFiles: () => void
  onOpenInstructions: () => void
  onOpenAutoMemory: () => void
  t: ReturnType<typeof useTranslation>["t"]
}) {
  if (loading) {
    return <div aria-hidden="true" className="h-80 animate-pulse rounded-xl bg-muted/60" />
  }
  const derivedRoot = overview?.instructions?.path.replace(/[/\\]AGENTS\.md$/, "")
  const workingDir = project.workingDir || derivedRoot || t("project.overview.defaultWorkspace")
  return (
    <div className="rounded-xl border border-border/70 p-4">
      <h4 className="text-sm font-semibold">{t("project.overview.contextTitle")}</h4>
      <p className="mt-0.5 text-[11px] text-muted-foreground">
        {t("project.overview.contextDescription")}
      </p>

      <button
        type="button"
        onClick={onOpenFiles}
        className="mt-4 flex w-full items-center gap-3 rounded-lg bg-muted/45 px-3 py-2.5 text-left hover:bg-muted"
      >
        <FolderOpen className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1">
          <span className="block text-xs font-medium">{t("project.overview.workingDirectory")}</span>
          <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
            {workingDir}
          </span>
        </span>
        <ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
      </button>

      <div className="mt-4 space-y-3">
        <ContextLink
          icon={FileCode2}
          title="AGENTS.md"
          description={
            overview?.instructions
              ? overview.instructions.empty
                ? t("project.overview.agentsEmpty")
                : t("project.overview.agentsStats", {
                    lines: overview.instructions.lineCount,
                    size: formatBytes(overview.instructions.sizeBytes, {
                      trimTrailingZeros: true,
                    }),
                  })
              : t("project.overview.unavailable")
          }
          hint={t("project.overview.agentsRole")}
          onClick={onOpenInstructions}
        />
        <ContextLink
          icon={Sparkles}
          title={t("project.tabAutoMemory")}
          description={t("project.overview.autoMemoryStats", {
            count: overview?.autoMemoryTopicCount ?? "—",
          })}
          hint={t("project.overview.autoMemoryRole")}
          onClick={onOpenAutoMemory}
        />
        <div className="flex gap-2.5 border-t border-border/60 pt-3">
          <Brain className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div>
            <p className="text-xs font-medium">{t("project.overview.structuredMemory")}</p>
            <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
              {t("project.overview.structuredMemoryRole")}
            </p>
          </div>
        </div>
      </div>
    </div>
  )
}

function ContextLink({
  icon: Icon,
  title,
  description,
  hint,
  onClick,
}: {
  icon: typeof FileCode2
  title: string
  description: string
  hint: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex w-full gap-2.5 text-left"
    >
      <Icon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground group-hover:text-primary" />
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2 text-xs font-medium">
          {title}
          <ArrowRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60 group-hover:text-primary" />
        </span>
        <span className="mt-0.5 block text-[11px] text-muted-foreground">{description}</span>
        <span className="mt-1 block text-[10px] leading-relaxed text-muted-foreground/75">
          {hint}
        </span>
      </span>
    </button>
  )
}

function OverviewSkeleton({ wide }: { wide: boolean }) {
  return (
    <div className={`grid gap-3 ${wide ? "grid-cols-4" : "grid-cols-2"}`}>
      {[0, 1, 2, 3].map((item) => (
        <div key={item} className="h-28 animate-pulse rounded-xl bg-muted/60" />
      ))}
    </div>
  )
}

function safeFormatDate(value: string, formatter: Intl.DateTimeFormat): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : formatter.format(date)
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
