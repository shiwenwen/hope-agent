/**
 * Sidebar section listing projects.
 *
 * Each project row is a tree node: clicking the row toggles its expansion to
 * reveal the sessions belonging to that project. Hover surfaces "+" (new
 * chat in this project) and gear (open settings sheet) buttons; right-click
 * shows the same actions plus archive. Below the row, when expanded, the
 * project's sessions render with `SessionItem` indented one level.
 *
 * The mainline `SessionList` keeps showing **unassigned** sessions only —
 * see [src/components/chat/sidebar/ChatSidebar.tsx](sidebar/ChatSidebar.tsx).
 */

import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  MessageSquarePlus,
  Plus,
  Settings,
  Archive,
  ArchiveRestore,
  CheckCheck,
} from "lucide-react"

import { IconTip } from "@/components/ui/tooltip"
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
import type {
  AgentSummaryForSidebar,
  SessionMeta,
} from "@/types/chat"
import type { SidebarDisplayMode } from "../sidebar/types"
import SessionItem from "../sidebar/SessionItem"
import SidebarSectionHeader from "../sidebar/SidebarSectionHeader"
import ProjectIcon from "./ProjectIcon"

interface ProjectSectionProps {
  projects: ProjectMeta[]
  /** Sessions list — used to render the children of each project group. */
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  expanded: boolean
  setExpanded: (v: boolean) => void
  onAddProject: () => void
  onOpenProjectSettings: (project: ProjectMeta) => void
  onNewChatInProject: (projectId: string, opts?: { incognito?: boolean }) => void
  onArchiveProject: (projectId: string, archived: boolean) => void
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
  onDeleteSession: (sessionId: string, e: React.MouseEvent) => void
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

function isUnreadChatSession(session: SessionMeta): boolean {
  return !session.channelInfo && !session.parentSessionId && session.unreadCount > 0
}

export default function ProjectSection(props: ProjectSectionProps) {
  const { t } = useTranslation()
  const {
    projects,
    sessions,
    expanded,
    setExpanded,
    onAddProject,
  } = props
  const visibleProjects = useMemo(() => projects.filter((p) => !p.archived), [projects])
  const archivedProjects = useMemo(() => projects.filter((p) => p.archived), [projects])
  const [archivedExpanded, setArchivedExpandedState] = useState(() =>
    readStoredBoolean(ARCHIVED_EXPANDED_STORAGE_KEY, false),
  )

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
        className="sticky top-8 z-20 mb-0 flex h-8 items-center border-b border-border/50 bg-surface-panel px-3"
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

      {expanded && (
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
          {visibleProjects.map((project) => (
            <ProjectGroup
              key={project.id}
              {...props}
              project={project}
              projectSessions={sessionsByProject.get(project.id) ?? []}
              expanded={expandedMap[project.id] ?? false}
              onToggleExpanded={() => toggleProjectExpanded(project.id)}
            />
          ))}
          {archivedProjects.length > 0 && (
            <div className="mt-2 border-t border-border/40 pt-2">
              <button
                onClick={() => setArchivedExpanded(!archivedExpanded)}
                className="flex w-full items-center gap-1.5 px-2 py-1 text-[11px] font-semibold tracking-normal text-muted-foreground/70 hover:text-foreground transition-colors"
              >
                <ChevronRight
                  className={cn(
                    "h-3 w-3 transition-transform duration-200",
                    archivedExpanded && "rotate-90",
                  )}
                />
                <Archive className="h-3 w-3" />
                <span className="truncate">{t("project.archivedProjects")}</span>
                <span className="ml-auto text-muted-foreground/60">
                  {archivedProjects.length}
                </span>
              </button>
              {archivedExpanded && (
                <div className="mt-0.5 space-y-0.5">
                  {archivedProjects.map((project) => (
                    <ProjectGroup
                      key={project.id}
                      {...props}
                      project={project}
                      projectSessions={sessionsByProject.get(project.id) ?? []}
                      expanded={expandedMap[project.id] ?? false}
                      onToggleExpanded={() => toggleProjectExpanded(project.id)}
                      archivedView
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
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
}

function ProjectGroup({
  project,
  projectSessions,
  expanded: groupExpanded,
  onToggleExpanded: handleToggleExpanded,
  sessions,
  currentSessionId,
  loadingSessionIds,
  onOpenProjectSettings,
  onNewChatInProject,
  onArchiveProject,
  onSwitchSession,
  onDeleteSession,
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
}: ProjectGroupProps) {
  const { t } = useTranslation()
  const currentSessionUnreadCount = useMemo(
    () =>
      projectSessions.find(
        (session) => session.id === currentSessionId && isUnreadChatSession(session),
      )?.unreadCount ?? 0,
    [projectSessions, currentSessionId],
  )
  const projectUnreadCount = Math.max(0, project.unreadCount - currentSessionUnreadCount)

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
    <div>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            className={cn(
              "group/project flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-accent/40 transition-colors text-left cursor-pointer",
              displayMode === "compact" && "gap-1.5 py-1",
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
            <ChevronRight
              className={cn(
                "h-3 w-3 shrink-0 text-muted-foreground/60 transition-transform duration-150",
                groupExpanded && "rotate-90",
              )}
            />
            {displayMode === "detailed" && (
              <div className="relative shrink-0">
                <ProjectIcon project={project} size="sm" withColorChip />
                {projectUnreadCount > 0 && (
                  <span
                    className="absolute -top-1 -right-1.5 z-10 flex h-[16px] min-w-[16px] items-center justify-center rounded-full border border-background bg-destructive px-0.5 text-[9px] font-semibold leading-none text-destructive-foreground tabular-nums pointer-events-none"
                  >
                    {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
                  </span>
                )}
              </div>
            )}

            <div className="flex-1 min-w-0">
              <div className={cn("truncate text-foreground/90", displayMode === "compact" ? "text-[12.5px]" : "text-sm")}>
                {project.name}
              </div>
            </div>
            {/* Hover-only action buttons. Match `AgentSection.tsx` styling so
                the two sections feel consistent. */}
            {!archivedView && (
              <IconTip label={t("project.newChatInProject")}>
                <button
                  className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/project:text-muted-foreground/60 hover:!text-primary transition-colors"
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
                className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/project:text-muted-foreground/60 hover:!text-primary transition-colors"
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
                  className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/project:text-muted-foreground/60 hover:!text-primary transition-colors"
                  onClick={(e) => {
                    e.stopPropagation()
                    onArchiveProject(project.id, false)
                  }}
                >
                  <ArchiveRestore className="h-3.5 w-3.5" />
                </button>
              </IconTip>
            )}
            {displayMode === "compact" && projectUnreadCount > 0 && (
              <span className="inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full bg-destructive px-1 text-[9px] font-semibold leading-none text-destructive-foreground tabular-nums">
                {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
              </span>
            )}
            {project.sessionCount > 0 && (
              <span
                className={cn(
                  "text-[10px] tabular-nums shrink-0",
                  // Hide the badge while hover buttons are showing to avoid clutter.
                  "text-muted-foreground/70 group-hover/project:hidden",
                )}
              >
                {project.sessionCount}
              </span>
            )}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent>
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
          <ContextMenuItem
            onClick={handleMarkProjectRead}
            disabled={project.unreadCount === 0}
          >
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

      {groupExpanded && (
        <div
          className={cn(
            "pl-3 pr-1 mt-0.5",
            displayMode === "compact" ? "space-y-1" : "space-y-0.5",
          )}
        >
          {projectSessions.length === 0 ? (
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
            projectSessions.map((session) => (
              <SessionItem
                key={session.id}
                session={session}
                sessions={sessions}
                agent={getAgentInfo(session.agentId)}
                projects={projects}
                isActive={session.id === currentSessionId}
                isLoading={loadingSessionIds.has(session.id)}
                renamingSessionId={renamingSessionId}
                renameValue={renameValue}
                renameInputRef={renameInputRef}
                onSwitchSession={onSwitchSession}
                onDeleteClick={onDeleteSession}
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
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}
