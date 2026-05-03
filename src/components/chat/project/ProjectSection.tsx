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
  ChevronDown,
  ChevronRight,
  MessageSquarePlus,
  Plus,
  Settings,
  Archive,
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
import SessionItem from "../sidebar/SessionItem"
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
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
}

const EXPANDED_STORAGE_KEY = "ha:project-expanded"

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
    <div className="px-3 pt-3 pb-1">
      <div className="flex items-center gap-1 mb-2">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80 hover:text-foreground transition-colors"
        >
          {expanded ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
          {t("project.projects")}
          {visibleProjects.length > 0 && (
            <span className="ml-1 text-muted-foreground/60">· {visibleProjects.length}</span>
          )}
        </button>
        <div className="ml-auto">
          <IconTip label={t("project.newProject")}>
            <button
              onClick={onAddProject}
              className="text-muted-foreground/60 hover:text-foreground transition-colors"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
      </div>

      {expanded && (
        <div className="space-y-0.5">
          {visibleProjects.length === 0 ? (
            <button
              onClick={onAddProject}
              className="w-full text-left text-xs text-muted-foreground/70 italic px-2 py-1.5 rounded-md hover:bg-accent/40"
            >
              {t("project.createFirstProject")}
            </button>
          ) : (
            visibleProjects.map((project) => (
              <ProjectGroup
                key={project.id}
                {...props}
                project={project}
                projectSessions={sessionsByProject.get(project.id) ?? []}
                expanded={expandedMap[project.id] ?? false}
                onToggleExpanded={() => toggleProjectExpanded(project.id)}
              />
            ))
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
  getAgentInfo,
  formatRelativeTime,
  projects,
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
            <div className="relative shrink-0">
              <ProjectIcon project={project} size="sm" withColorChip />
              {projectUnreadCount > 0 && (
                <span
                  className="absolute -top-1 -right-1.5 z-10 min-w-[16px] h-[16px] px-0.5 rounded-full text-white text-[9px] font-bold flex items-center justify-center border border-background pointer-events-none leading-none"
                  style={{
                    background:
                      "linear-gradient(135deg, #ff6b6b 0%, #ee3333 50%, #cc1111 100%)",
                    boxShadow:
                      "0 2px 6px rgba(220, 38, 38, 0.45), inset 0 1px 1px rgba(255, 255, 255, 0.25)",
                  }}
                >
                  {projectUnreadCount > 99 ? "99+" : projectUnreadCount}
                </span>
              )}
            </div>

            <div className="flex-1 min-w-0">
              <div className="text-sm truncate text-foreground/90">{project.name}</div>
            </div>
            {/* Hover-only action buttons. Match `AgentSection.tsx` styling so
                the two sections feel consistent. */}
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
          <ContextMenuItem onClick={() => onNewChatInProject(project.id)}>
            <MessageSquarePlus className="h-3 w-3 mr-2" />
            {t("project.newChatInProject")}
          </ContextMenuItem>
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
          <ContextMenuItem onClick={() => onArchiveProject(project.id, !project.archived)}>
            <Archive className="h-3 w-3 mr-2" />
            {project.archived ? t("project.unarchiveProject") : t("project.archiveProject")}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      {groupExpanded && (
        <div className="pl-3 pr-1 mt-0.5 space-y-0.5">
          {projectSessions.length === 0 ? (
            <button
              onClick={() => onNewChatInProject(project.id)}
              className="w-full text-left text-[11px] text-muted-foreground/70 italic px-2 py-1 rounded-md hover:bg-accent/30"
            >
              {t("project.noSessionsHint")}
            </button>
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
                getAgentInfo={getAgentInfo}
                formatRelativeTime={formatRelativeTime}
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}
