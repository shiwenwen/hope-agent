/**
 * Sidebar section listing projects.
 *
 * Each project row is a tree node: clicking the row toggles its expansion to
 * reveal the sessions belonging to that project. Hover surfaces "+" (new
 * chat in this project) and gear (open settings sheet) buttons; right-click
 * shows the same actions plus archive. Below the row, when expanded, the
 * project's sessions render with `SessionItem` indented one level.
 *
 * Each project paginates **independently** — `useProjectSessions` fetches that
 * project's own sessions on demand (`list_project_sessions_cmd`), starting at
 * `PROJECT_SESSION_PAGE_SIZE` with "show more" / "show less" controls — rather
 * than slicing the shared global session array (which only holds the most
 * recent page and would hide older project sessions). The global array is still
 * passed in as a cheap realtime change-signal for refetching.
 *
 * The mainline `SessionList` keeps showing **unassigned** sessions only —
 * see [src/components/chat/sidebar/ChatSidebar.tsx](sidebar/ChatSidebar.tsx).
 */

import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  DndContext,
  closestCenter,
  MouseSensor,
  TouchSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DraggableAttributes,
  type DraggableSyntheticListeners,
} from "@dnd-kit/core"
import {
  SortableContext,
  arrayMove,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import {
  ChevronRight,
  ChevronDown,
  ChevronUp,
  Folder,
  FolderOpen,
  Loader2,
  MessageSquarePlus,
  Plus,
  Settings,
  Archive,
  ArchiveRestore,
  CheckCheck,
} from "lucide-react"

import { IconTip } from "@/components/ui/tooltip"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { ProjectMeta } from "@/types/project"
import type { AgentSummaryForSidebar, SessionMeta, UnreadSessionTarget } from "@/types/chat"
import type { SidebarDisplayMode } from "../sidebar/types"
import SessionItem from "../sidebar/SessionItem"
import SidebarSectionHeader from "../sidebar/SidebarSectionHeader"
import ProjectIcon from "./ProjectIcon"
import { PROJECT_SESSION_PAGE_SIZE } from "../hooks/constants"
import { useProjectSessions } from "./hooks/useProjectSessions"
import { PROJECT_TEXT_COLOR_MAP } from "./colors"

interface ProjectSectionProps {
  projects: ProjectMeta[]
  /** Global session array (live overlay). No longer rendered directly under
   *  projects — each group fetches its own page via `useProjectSessions` — but
   *  still used for the `SessionItem` parent lookup and as a realtime
   *  change-signal that drives per-project refetches. */
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentSessionId: string | null
  readableSessionId: string | null
  loadingSessionIds: Set<string>
  expanded: boolean
  setExpanded: (v: boolean) => void
  onAddProject: () => void
  onOpenProjectSettings: (project: ProjectMeta) => void
  onNewChatInProject: (projectId: string, opts?: { incognito?: boolean }) => void
  onArchiveProject: (projectId: string, archived: boolean) => void
  onReorderProjects?: (projectIds: string[]) => void
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
  onArchiveSession: (sessionId: string, e: React.MouseEvent) => void
  onMarkAllRead?: () => void
  renamingSessionId: string | null
  renameValue: string
  renameInputRef: React.RefObject<HTMLInputElement | null>
  onStartRename: (sessionId: string, currentTitle: string) => void
  onRenameValueChange: (value: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  onMoveSessionToProject?: (sessionId: string, projectId: string | null) => void
  onToggleSessionPinned?: (sessionId: string, pinned: boolean) => void
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
  displayMode: SidebarDisplayMode
  motionDisabled?: boolean
  /** Whether the sticky Agent header occupies the 32px row above Projects. */
  hasAgentHeader?: boolean
  unreadFocusTarget?: (UnreadSessionTarget & { signal: number }) | null
}

const EXPANDED_STORAGE_KEY = "ha:project-expanded"
const ARCHIVED_EXPANDED_STORAGE_KEY = "ha:project-archived-expanded"

function readStoredBoolean(key: string, fallback: boolean): boolean {
  try {
    if (typeof window === "undefined") return fallback
    const raw = window.localStorage.getItem(key)
    if (raw === null) return fallback
    return raw === "true"
  } catch {
    return fallback
  }
}

function writeStoredBoolean(key: string, value: boolean) {
  try {
    window.localStorage.setItem(key, String(value))
  } catch {
    // localStorage may be unavailable in restricted browser modes.
  }
}

export default function ProjectSection(props: ProjectSectionProps) {
  const { t } = useTranslation()
  const {
    projects,
    sessions,
    expanded,
    setExpanded,
    onAddProject,
    onReorderProjects,
    motionDisabled = false,
    hasAgentHeader = false,
    unreadFocusTarget,
  } = props
  const visibleProjects = useMemo(() => projects.filter((p) => !p.archived), [projects])
  const archivedProjects = useMemo(() => projects.filter((p) => p.archived), [projects])
  const visibleProjectIds = useMemo(
    () => visibleProjects.map((project) => project.id),
    [visibleProjects],
  )
  const sensors = useSensors(
    useSensor(MouseSensor, { activationConstraint: { distance: 5 } }),
    useSensor(TouchSensor, {
      activationConstraint: { delay: 250, tolerance: 5 },
    }),
  )
  const [archivedExpanded, setArchivedExpandedState] = useState(() =>
    readStoredBoolean(ARCHIVED_EXPANDED_STORAGE_KEY, false),
  )
  const [projectSorting, setProjectSorting] = useState(false)

  const setArchivedExpanded = useCallback((expanded: boolean) => {
    setArchivedExpandedState(expanded)
    writeStoredBoolean(ARCHIVED_EXPANDED_STORAGE_KEY, expanded)
  }, [])

  // Single localStorage entry for all project expansion states. Loaded once,
  // persisted on toggle. Stale keys for deleted projects are harmless and
  // get rewritten naturally on the next toggle.
  const [expandedMap, setExpandedMap] = useState<Record<string, boolean>>(() => {
    try {
      const raw = localStorage.getItem(EXPANDED_STORAGE_KEY)
      return raw ? JSON.parse(raw) : {}
    } catch {
      return {}
    }
  })

  // A reveal target temporarily forces its containing tree path open without
  // mutating the user's persisted expansion preferences from an effect.
  const unreadProjectId = unreadFocusTarget?.projectId ?? null
  const projectsExpanded = expanded || unreadProjectId !== null
  const archivedProjectsExpanded =
    archivedExpanded ||
    (unreadProjectId !== null &&
      archivedProjects.some((project) => project.id === unreadProjectId))
  const projectIsExpanded = useCallback(
    (projectId: string) => (expandedMap[projectId] ?? false) || projectId === unreadProjectId,
    [expandedMap, unreadProjectId],
  )

  const toggleProjectExpanded = useCallback((projectId: string) => {
    setExpandedMap((prev) => {
      const next = { ...prev, [projectId]: !prev[projectId] }
      try {
        localStorage.setItem(EXPANDED_STORAGE_KEY, JSON.stringify(next))
      } catch {
        /* ignore */
      }
      return next
    })
  }, [])

  const collapseAllProjects = useCallback(() => {
    setExpandedMap((prev) => {
      if (Object.keys(prev).length === 0) return prev
      try {
        localStorage.setItem(EXPANDED_STORAGE_KEY, "{}")
      } catch {
        /* ignore */
      }
      return {}
    })
  }, [])

  const finishProjectSorting = useCallback(() => {
    setProjectSorting(false)
  }, [])

  const handleProjectDragEnd = useCallback(
    (event: DragEndEvent) => {
      finishProjectSorting()
      if (!onReorderProjects) return
      const { active, over } = event
      if (!over || active.id === over.id) return
      const oldIndex = visibleProjects.findIndex((project) => project.id === active.id)
      const newIndex = visibleProjects.findIndex((project) => project.id === over.id)
      if (oldIndex === -1 || newIndex === -1) return
      onReorderProjects(arrayMove(visibleProjects, oldIndex, newIndex).map((project) => project.id))
    },
    [finishProjectSorting, onReorderProjects, visibleProjects],
  )

  const handleProjectDragStart = useCallback(() => {
    setProjectSorting(true)
    collapseAllProjects()
  }, [collapseAllProjects])

  // Group sessions by projectId once per render so each ProjectGroup is O(1)
  // instead of re-scanning the full list (O(N×M) for N sessions × M projects).
  const sessionsByProject = useMemo(() => {
    const map = new Map<string, SessionMeta[]>()
    for (const s of sessions) {
      if (!s.projectId) continue
      const arr = map.get(s.projectId)
      if (arr) arr.push(s)
      else map.set(s.projectId, [s])
    }
    return map
  }, [sessions])

  return (
    <div className="contents">
      <SidebarSectionHeader
        title={t("project.projects")}
        count={visibleProjects.length > 0 ? visibleProjects.length : undefined}
        expanded={expanded}
        onToggle={() => setExpanded(!expanded)}
        className={cn(
          "sticky z-20 mb-0 flex h-8 items-center border-b border-border/50 bg-surface-panel px-3",
          hasAgentHeader ? "top-8" : "top-0",
        )}
        action={
          <IconTip label={t("project.newProject")}>
            <button
              onClick={onAddProject}
              className="text-muted-foreground/60 hover:text-foreground transition-colors"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        }
      />

      <AnimatedCollapse open={projectsExpanded} durationMs={motionDisabled ? 0 : undefined}>
        <div className="space-y-0.5 px-3 pb-1 pt-1">
          {visibleProjects.length === 0 && (
            <button
              onClick={onAddProject}
              className="w-full text-left text-xs text-muted-foreground/70 italic px-2 py-1.5 rounded-md hover:bg-accent/40"
            >
              {archivedProjects.length > 0
                ? t("project.newProject")
                : t("project.createFirstProject")}
            </button>
          )}
          {onReorderProjects ? (
            <DndContext
              sensors={sensors}
              collisionDetection={closestCenter}
              onDragStart={handleProjectDragStart}
              onDragCancel={finishProjectSorting}
              onDragEnd={handleProjectDragEnd}
            >
              <SortableContext items={visibleProjectIds} strategy={verticalListSortingStrategy}>
                {visibleProjects.map((project) => (
                  <SortableProjectGroup
                    key={project.id}
                    {...props}
                    project={project}
                    projectSessions={sessionsByProject.get(project.id) ?? []}
                    expanded={projectIsExpanded(project.id)}
                    onToggleExpanded={() => toggleProjectExpanded(project.id)}
                    canReorder
                    suppressChildren={projectSorting}
                  />
                ))}
              </SortableContext>
            </DndContext>
          ) : (
            visibleProjects.map((project) => (
              <ProjectGroup
                key={project.id}
                {...props}
                project={project}
                projectSessions={sessionsByProject.get(project.id) ?? []}
                expanded={projectIsExpanded(project.id)}
                onToggleExpanded={() => toggleProjectExpanded(project.id)}
              />
            ))
          )}
          {archivedProjects.length > 0 && (
            <div className="mt-2 border-t border-border/40 pt-2">
              <button
                onClick={() => setArchivedExpanded(!archivedExpanded)}
                className="flex w-full items-center gap-1.5 px-2 py-1 text-[11px] font-semibold tracking-normal text-muted-foreground/70 hover:text-foreground transition-colors"
              >
                <ChevronRight
                  className={cn(
                    "h-3 w-3 transition-transform duration-200",
                    archivedProjectsExpanded && "rotate-90",
                  )}
                />
                <Archive className="h-3 w-3" />
                <span className="truncate">{t("project.archivedProjects")}</span>
                <span className="ml-auto text-muted-foreground/60">{archivedProjects.length}</span>
              </button>
              <AnimatedCollapse
                open={archivedProjectsExpanded}
                durationMs={motionDisabled ? 0 : undefined}
              >
                <div className="mt-0.5 space-y-0.5">
                  {archivedProjects.map((project) => (
                    <ProjectGroup
                      key={project.id}
                      {...props}
                      project={project}
                      projectSessions={sessionsByProject.get(project.id) ?? []}
                      expanded={projectIsExpanded(project.id)}
                      onToggleExpanded={() => toggleProjectExpanded(project.id)}
                      archivedView
                    />
                  ))}
                </div>
              </AnimatedCollapse>
            </div>
          )}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

// ── Single project row + its session children ────────────────────

interface ProjectGroupProps extends Omit<ProjectSectionProps, "expanded" | "setExpanded"> {
  project: ProjectMeta
  projectSessions: SessionMeta[]
  expanded: boolean
  onToggleExpanded: () => void
  archivedView?: boolean
  canReorder?: boolean
  dragAttributes?: DraggableAttributes
  dragListeners?: DraggableSyntheticListeners
  setSortableNodeRef?: (node: HTMLElement | null) => void
  sortableStyle?: React.CSSProperties
  isDragging?: boolean
  suppressChildren?: boolean
}

function SortableProjectGroup(props: ProjectGroupProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: props.project.id,
    disabled: !props.canReorder,
  })
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.45 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  return (
    <ProjectGroup
      {...props}
      dragAttributes={attributes}
      dragListeners={listeners}
      setSortableNodeRef={setNodeRef}
      sortableStyle={style}
      isDragging={isDragging}
    />
  )
}

function ProjectGroup({
  project,
  projectSessions,
  expanded: groupExpanded,
  onToggleExpanded: handleToggleExpanded,
  sessions,
  currentSessionId,
  readableSessionId,
  loadingSessionIds,
  onOpenProjectSettings,
  onNewChatInProject,
  onArchiveProject,
  onSwitchSession,
  onArchiveSession,
  onMarkAllRead,
  renamingSessionId,
  renameValue,
  renameInputRef,
  onStartRename,
  onRenameValueChange,
  onCommitRename,
  onCancelRename,
  onMoveSessionToProject,
  onToggleSessionPinned,
  getAgentInfo,
  formatRelativeTime,
  projects,
  archivedView = false,
  displayMode,
  canReorder = false,
  dragAttributes,
  dragListeners,
  setSortableNodeRef,
  sortableStyle,
  isDragging = false,
  suppressChildren = false,
  motionDisabled = false,
  unreadFocusTarget,
}: ProjectGroupProps) {
  const { t } = useTranslation()
  // The actually-readable session is already excluded from
  // `project.unreadCount` by the backend rollup, so the badge uses the value
  // directly — no cross-source subtraction that could transiently go negative
  // / flicker as the two fetches settle.
  const projectUnreadCount = project.unreadCount

  // Fingerprint of the project's slice of the live global session array. Any
  // visible change (create / delete / rename / reorder / read / pin) flips it
  // and triggers the independent per-project refetch below.
  const changeSignal = useMemo(
    () =>
      projectSessions
        .map(
          (s) =>
            `${s.id}:${s.updatedAt}:${s.pinnedAt ?? ""}:${s.unreadCount}:${s.title ?? ""}:${s.pendingInteractionCount}`,
        )
        .join("|"),
    [projectSessions],
  )

  const {
    sessions: childSessions,
    total: childTotal,
    loading: childLoading,
    loadingMore: childLoadingMore,
    hasMore: childHasMore,
    canCollapse: childCanCollapse,
    showMore: childShowMore,
    showLess: childShowLess,
  } = useProjectSessions({
    projectId: project.id,
    expanded: groupExpanded,
    changeSignal,
    sessionCount: project.sessionCount,
    ensureSessionId:
      unreadFocusTarget?.projectId === project.id ? unreadFocusTarget.sessionId : null,
    ensureSessionOffset:
      unreadFocusTarget?.projectId === project.id ? unreadFocusTarget.listOffset : null,
  })
  const showPaginationFooter = childTotal > PROJECT_SESSION_PAGE_SIZE
  const ProjectToggleIcon = groupExpanded ? FolderOpen : Folder
  const projectFolderColorClass =
    (project.color && PROJECT_TEXT_COLOR_MAP[project.color]) || "text-muted-foreground/70"

  const handleMarkProjectRead = useCallback(async () => {
    if (project.unreadCount === 0) return
    try {
      await getTransport().call("mark_project_sessions_read_cmd", {
        projectId: project.id,
      })
      onMarkAllRead?.()
    } catch (err) {
      logger.error(
        "chat",
        "ProjectSection::markProjectRead",
        "Failed to mark project sessions as read",
        err,
      )
    }
  }, [project.id, project.unreadCount, onMarkAllRead])

  return (
    <div className={cn(isDragging && "relative z-40")}>
      <div ref={setSortableNodeRef} style={sortableStyle}>
        <ContextMenu>
          <ContextMenuTrigger asChild>
            <div
              {...dragAttributes}
              {...dragListeners}
              className={cn(
                "group/project relative flex min-h-10 items-center gap-2 overflow-hidden rounded-md bg-muted/20 py-1.5 pl-1 pr-2.5 text-left transition-colors hover:bg-accent/35",
                displayMode === "compact" && "min-h-8 gap-1.5 py-1 pl-1 pr-2",
                canReorder && !archivedView
                  ? "cursor-grab touch-pan-y active:cursor-grabbing"
                  : "cursor-pointer",
              )}
              onClick={handleToggleExpanded}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault()
                  handleToggleExpanded()
                }
              }}
            >
              <span className="relative shrink-0">
                <ProjectToggleIcon
                  className={cn(
                    "h-3.5 w-3.5 transition-colors duration-150",
                    projectFolderColorClass,
                  )}
                />
                {displayMode === "detailed" && !project.logo && projectUnreadCount > 0 && (
                  <span
                    aria-hidden="true"
                    className="absolute -top-1 -right-1.5 z-10 flex h-[16px] min-w-[16px] items-center justify-center rounded-full border border-background bg-destructive px-0.5 text-[9px] font-semibold leading-none text-destructive-foreground tabular-nums pointer-events-none"
                  >
                    {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
                  </span>
                )}
              </span>
              {displayMode === "detailed" && project.logo && (
                <div className="relative shrink-0">
                  <ProjectIcon project={project} size="sm" />
                  {projectUnreadCount > 0 && (
                    <span
                      aria-hidden="true"
                      className="absolute -top-1 -right-1.5 z-10 flex h-[16px] min-w-[16px] items-center justify-center rounded-full border border-background bg-destructive px-0.5 text-[9px] font-semibold leading-none text-destructive-foreground tabular-nums pointer-events-none"
                    >
                      {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
                    </span>
                  )}
                </div>
              )}

              <div className="min-w-0 flex-1 pr-12">
                <div
                  data-ha-title-tip={project.name}
                  className={cn(
                    "truncate font-semibold text-foreground",
                    displayMode === "compact" ? "text-[12.5px]" : "text-sm",
                  )}
                >
                  {project.name}
                  {projectUnreadCount > 0 && (
                    <span className="sr-only">
                      {" — "}
                      {t("chat.unreadConversationCount", { count: projectUnreadCount })}
                    </span>
                  )}
                </div>
              </div>
              {/* Hover-only action buttons. Match `AgentSection.tsx` styling so
                  the two sections feel consistent. */}
              <div
                className="absolute right-2 top-1/2 z-10 flex -translate-y-1/2 items-center gap-1 opacity-0 pointer-events-none transition-opacity group-hover/project:pointer-events-auto group-hover/project:opacity-100 group-focus-within/project:pointer-events-auto group-focus-within/project:opacity-100"
                onPointerDown={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
              >
                {!archivedView && (
                  <IconTip label={t("project.newChatInProject")}>
                    <button
                      className="rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-background/70 hover:text-primary"
                      onClick={(e) => {
                        e.stopPropagation()
                        onNewChatInProject(project.id)
                      }}
                    >
                      <MessageSquarePlus className="h-3.5 w-3.5" />
                    </button>
                  </IconTip>
                )}
                <IconTip label={t("project.openProjectSettings")}>
                  <button
                    className="rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-background/70 hover:text-primary"
                    onClick={(e) => {
                      e.stopPropagation()
                      onOpenProjectSettings(project)
                    }}
                  >
                    <Settings className="h-3.5 w-3.5" />
                  </button>
                </IconTip>
                {archivedView && (
                  <IconTip label={t("project.unarchiveProject")}>
                    <button
                      className="rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-background/70 hover:text-primary"
                      onClick={(e) => {
                        e.stopPropagation()
                        onArchiveProject(project.id, false)
                      }}
                    >
                      <ArchiveRestore className="h-3.5 w-3.5" />
                    </button>
                  </IconTip>
                )}
              </div>
              {displayMode === "compact" && projectUnreadCount > 0 && (
                <span className="pointer-events-none absolute right-3 top-1/2 inline-flex h-[15px] min-w-[15px] -translate-y-1/2 items-center justify-center rounded-full bg-destructive px-1 text-[9px] font-semibold leading-none text-destructive-foreground tabular-nums transition-opacity group-hover/project:opacity-0 group-focus-within/project:opacity-0">
                  {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
                </span>
              )}
              {project.sessionCount > 0 &&
                !(displayMode === "compact" && projectUnreadCount > 0) && (
                  <span
                    className={cn(
                      "pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[10px] tabular-nums transition-opacity",
                      "text-muted-foreground/70 group-hover/project:opacity-0 group-focus-within/project:opacity-0",
                    )}
                  >
                    {project.sessionCount}
                  </span>
                )}
            </div>
          </ContextMenuTrigger>
          <ContextMenuContent variant="floating">
            {!archivedView && (
              <ContextMenuItem onClick={() => onNewChatInProject(project.id)}>
                <MessageSquarePlus className="h-3 w-3 mr-2" />
                {t("project.newChatInProject")}
              </ContextMenuItem>
            )}
            <ContextMenuItem onClick={() => onOpenProjectSettings(project)}>
              <Settings className="h-3 w-3 mr-2" />
              {t("project.openProjectSettings")}
            </ContextMenuItem>
            <ContextMenuItem onClick={handleMarkProjectRead} disabled={project.unreadCount === 0}>
              <CheckCheck className="h-3 w-3 mr-2" />
              {t("chat.markAllRead")}
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              onClick={() => onArchiveProject(project.id, archivedView ? false : !project.archived)}
            >
              {archivedView ? (
                <ArchiveRestore className="h-3 w-3 mr-2" />
              ) : (
                <Archive className="h-3 w-3 mr-2" />
              )}
              {project.archived ? t("project.unarchiveProject") : t("project.archiveProject")}
            </ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
      </div>

      {!suppressChildren && (
        <AnimatedCollapse open={groupExpanded} durationMs={motionDisabled ? 0 : undefined}>
          <div
            className={cn(
              "pl-4 pr-1 mt-0.5",
              displayMode === "compact" ? "space-y-1" : "space-y-0.5",
            )}
          >
            {childLoading ? (
              <div className="flex justify-center py-3">
                <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
              </div>
            ) : childSessions.length === 0 ? (
              archivedView ? (
                <div className="px-2 py-1 text-[11px] text-muted-foreground/60">
                  {t("project.sessionsInProject", { count: 0 })}
                </div>
              ) : (
                <button
                  onClick={() => onNewChatInProject(project.id)}
                  className="w-full text-left text-[11px] text-muted-foreground/70 italic px-2 py-1 rounded-md hover:bg-accent/30"
                >
                  {t("project.noSessionsHint")}
                </button>
              )
            ) : (
              <>
                {childSessions.map((session) => (
                  <SessionItem
                    key={session.id}
                    session={session}
                    sessions={sessions}
                    agent={getAgentInfo(session.agentId)}
                    projects={projects}
                    isActive={session.id === currentSessionId}
                    isReadable={session.id === readableSessionId}
                    isLoading={loadingSessionIds.has(session.id)}
                    renamingSessionId={renamingSessionId}
                    renameValue={renameValue}
                    renameInputRef={renameInputRef}
                    onSwitchSession={onSwitchSession}
                    onArchiveClick={onArchiveSession}
                    onStartRename={onStartRename}
                    onRenameValueChange={onRenameValueChange}
                    onCommitRename={onCommitRename}
                    onCancelRename={onCancelRename}
                    onMarkAllRead={onMarkAllRead}
                    onMoveToProject={onMoveSessionToProject}
                    onTogglePinned={onToggleSessionPinned}
                    getAgentInfo={getAgentInfo}
                    formatRelativeTime={formatRelativeTime}
                    displayMode={displayMode}
                    revealSignal={
                      unreadFocusTarget?.sessionId === session.id
                        ? unreadFocusTarget.signal
                        : undefined
                    }
                  />
                ))}
                {showPaginationFooter && (
                  <div className="flex items-center justify-center gap-3 px-2 pt-0.5 pb-1">
                    <button
                      onClick={childShowMore}
                      disabled={!childHasMore || childLoadingMore}
                      className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/70 transition-colors hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
                    >
                      {childLoadingMore ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <ChevronDown className="h-3 w-3" />
                      )}
                      {t("project.showMore")}
                    </button>
                    <button
                      onClick={childShowLess}
                      disabled={!childCanCollapse}
                      className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/70 transition-colors hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
                    >
                      <ChevronUp className="h-3 w-3" />
                      {t("project.showLess")}
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
        </AnimatedCollapse>
      )}
    </div>
  )
}
