import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  Archive,
  ArchiveRestore,
  Bot,
  Folder,
  Loader2,
  MessageSquare,
  Search,
  Trash2,
} from "lucide-react"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { SessionMeta } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

type ConversationType = "all" | "regular" | "subagent" | "channel" | "cron" | "knowledge" | "design"

const ARCHIVE_PAGE_SIZE = 50

interface ArchiveGroup {
  id: string
  label: string
  projectId: string | null
  sessions: SessionMeta[]
}

function conversationType(session: SessionMeta): Exclude<ConversationType, "all"> {
  if (session.kind === "knowledge") return "knowledge"
  if (session.kind === "design") return "design"
  if (session.isCron) return "cron"
  if (session.channelInfo) return "channel"
  if (session.parentSessionId) return "subagent"
  return "regular"
}

export default function ArchivedConversationsPanel() {
  const { t, i18n } = useTranslation()
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [total, setTotal] = useState(0)
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [query, setQuery] = useState("")
  const [typeFilter, setTypeFilter] = useState<ConversationType>("all")
  const [projectFilter, setProjectFilter] = useState("all")
  const [workingId, setWorkingId] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<SessionMeta | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const [[archived, archivedTotal], projectList] = await Promise.all([
        getTransport().call<[SessionMeta[], number]>("list_archived_sessions_cmd", {
          limit: ARCHIVE_PAGE_SIZE,
          offset: 0,
        }),
        getTransport().call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true }),
      ])
      setSessions(archived)
      setTotal(archivedTotal)
      setProjects(projectList)
    } catch (error) {
      logger.error(
        "settings",
        "ArchivedConversationsPanel::load",
        "Failed to load archived conversations",
        error,
      )
      toast.error(t("settings.conversationArchive.loadFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void load()
  }, [load])

  const loadMore = useCallback(async () => {
    if (loadingMore || sessions.length >= total) return
    setLoadingMore(true)
    try {
      const [next, archivedTotal] = await getTransport().call<[SessionMeta[], number]>(
        "list_archived_sessions_cmd",
        {
          limit: ARCHIVE_PAGE_SIZE,
          offset: sessions.length,
        },
      )
      setSessions((current) => {
        const seen = new Set(current.map((session) => session.id))
        return [...current, ...next.filter((session) => !seen.has(session.id))]
      })
      setTotal(archivedTotal)
    } catch (error) {
      logger.error(
        "settings",
        "ArchivedConversationsPanel::loadMore",
        "Failed to load more archived conversations",
        error,
      )
      toast.error(t("settings.conversationArchive.loadFailed"))
    } finally {
      setLoadingMore(false)
    }
  }, [loadingMore, sessions.length, t, total])

  const projectById = useMemo(
    () => new Map(projects.map((project) => [project.id, project])),
    [projects],
  )
  const filtered = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase()
    return sessions.filter((session) => {
      const type = conversationType(session)
      if (typeFilter !== "all" && type !== typeFilter) return false
      if (projectFilter === "unassigned" && session.projectId) return false
      if (projectFilter !== "all" && projectFilter !== "unassigned") {
        if (session.projectId !== projectFilter) return false
      }
      if (!normalizedQuery) return true
      const projectName = session.projectId ? projectById.get(session.projectId)?.name : ""
      return [session.title, session.agentId, projectName]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(normalizedQuery))
    })
  }, [projectById, projectFilter, query, sessions, typeFilter])

  const groups = useMemo<ArchiveGroup[]>(() => {
    const byId = new Map<string, ArchiveGroup>()
    for (const session of filtered) {
      const type = conversationType(session)
      const project = session.projectId ? projectById.get(session.projectId) : undefined
      const id = project ? `project:${project.id}` : `space:${type}`
      const label = project?.name ?? t(`settings.conversationArchive.types.${type}`)
      const group = byId.get(id) ?? {
        id,
        label,
        projectId: project?.id ?? null,
        sessions: [],
      }
      group.sessions.push(session)
      byId.set(id, group)
    }
    return [...byId.values()].sort((left, right) => {
      const leftProject = left.projectId ? projectById.get(left.projectId) : undefined
      const rightProject = right.projectId ? projectById.get(right.projectId) : undefined
      if (leftProject && rightProject) return leftProject.sortOrder - rightProject.sortOrder
      if (leftProject) return -1
      if (rightProject) return 1
      return left.label.localeCompare(right.label)
    })
  }, [filtered, projectById, t])

  const formatDate = useCallback(
    (value: string | null | undefined) => {
      if (!value) return ""
      const date = new Date(value)
      if (Number.isNaN(date.getTime())) return ""
      return new Intl.DateTimeFormat(i18n.resolvedLanguage, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      }).format(date)
    },
    [i18n.resolvedLanguage],
  )

  const restore = useCallback(
    async (session: SessionMeta) => {
      setWorkingId(session.id)
      try {
        await getTransport().call("set_session_archived_cmd", {
          sessionId: session.id,
          archived: false,
        })
        setSessions((current) => current.filter((item) => item.id !== session.id))
        setTotal((current) => Math.max(0, current - 1))
        window.dispatchEvent(
          new CustomEvent("hope:session-archive-changed", {
            detail: { sessionId: session.id, archived: false },
          }),
        )
        toast.success(t("settings.conversationArchive.restored"), {
          description: session.title || t("chat.untitledSession"),
        })
      } catch (error) {
        logger.error(
          "settings",
          "ArchivedConversationsPanel::restore",
          "Failed to restore conversation",
          error,
        )
        toast.error(t("settings.conversationArchive.restoreFailed"))
      } finally {
        setWorkingId(null)
      }
    },
    [t],
  )

  const permanentlyDelete = useCallback(async () => {
    const target = deleteTarget
    if (!target) return
    setWorkingId(target.id)
    try {
      await getTransport().call("delete_session_cmd", { sessionId: target.id })
      setSessions((current) => current.filter((item) => item.id !== target.id))
      setTotal((current) => Math.max(0, current - 1))
      setDeleteTarget(null)
      toast.success(t("settings.conversationArchive.deleted"), {
        description: target.title || t("chat.untitledSession"),
      })
    } catch (error) {
      logger.error(
        "settings",
        "ArchivedConversationsPanel::delete",
        "Failed to permanently delete conversation",
        error,
      )
      toast.error(t("settings.conversationArchive.deleteFailed"))
    } finally {
      setWorkingId(null)
    }
  }, [deleteTarget, t])

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-5xl space-y-6 px-6 py-5">
        <div className="space-y-1">
          <h2 className="text-xl font-semibold text-foreground">
            {t("settings.conversationArchive.title")}
          </h2>
          <p className="text-sm text-muted-foreground">
            {t("settings.conversationArchive.description")}
          </p>
        </div>

        <div className="grid gap-2 md:grid-cols-[minmax(240px,1fr)_180px_220px]">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <SearchInput
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("settings.conversationArchive.searchPlaceholder")}
              className="h-9 pl-9"
            />
          </div>
          <Select
            value={typeFilter}
            onValueChange={(value) => setTypeFilter(value as ConversationType)}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(
                ["all", "regular", "subagent", "channel", "cron", "knowledge", "design"] as const
              ).map((type) => (
                <SelectItem key={type} value={type}>
                  {t(`settings.conversationArchive.types.${type}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={projectFilter} onValueChange={setProjectFilter}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("settings.conversationArchive.allProjects")}</SelectItem>
              <SelectItem value="unassigned">
                {t("settings.conversationArchive.noProject")}
              </SelectItem>
              {projects.map((project) => (
                <SelectItem key={project.id} value={project.id}>
                  {project.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {loading ? (
          <div className="flex min-h-48 items-center justify-center text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : groups.length === 0 ? (
          <div className="space-y-3">
            <div className="flex min-h-56 flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border-soft text-center">
              <Archive className="h-8 w-8 text-muted-foreground/40" />
              <p className="text-sm font-medium text-foreground">
                {t("settings.conversationArchive.empty")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("settings.conversationArchive.emptyHint")}
              </p>
            </div>
            {sessions.length < total && (
              <div className="flex justify-center">
                <Button variant="secondary" onClick={() => void loadMore()} disabled={loadingMore}>
                  {loadingMore && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                  {t("settings.conversationArchive.loadMore")}
                </Button>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-6">
            {groups.map((group) => (
              <section key={group.id} className="space-y-2">
                <div className="flex items-center gap-2 px-1 text-sm font-medium text-foreground">
                  {group.projectId ? (
                    <Folder className="h-4 w-4 text-muted-foreground" />
                  ) : (
                    <MessageSquare className="h-4 w-4 text-muted-foreground" />
                  )}
                  <span className="truncate">{group.label}</span>
                  <span className="ml-auto text-xs font-normal text-muted-foreground">
                    {t("settings.conversationArchive.count", { count: group.sessions.length })}
                  </span>
                </div>
                <div className="overflow-hidden rounded-xl border border-border-soft bg-surface-panel">
                  {group.sessions.map((session, index) => {
                    const type = conversationType(session)
                    const busy = workingId === session.id
                    return (
                      <div
                        key={session.id}
                        className={`flex items-center gap-3 px-4 py-3 ${index > 0 ? "border-t border-border-soft" : ""}`}
                      >
                        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground">
                          <Bot className="h-4 w-4" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span className="truncate text-sm font-medium text-foreground">
                              {session.title || t("chat.untitledSession")}
                            </span>
                            <span className="shrink-0 rounded-md bg-muted/60 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                              {t(`settings.conversationArchive.types.${type}`)}
                            </span>
                          </div>
                          <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                            <span className="truncate">{session.agentId}</span>
                            <span aria-hidden="true">·</span>
                            <span className="shrink-0">
                              {formatDate(session.archivedAt ?? session.updatedAt)}
                            </span>
                          </div>
                        </div>
                        <IconTip label={t("settings.conversationArchive.deletePermanently")}>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 shrink-0 text-muted-foreground hover:text-destructive"
                            onClick={() => setDeleteTarget(session)}
                            disabled={busy}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </IconTip>
                        <Button
                          variant="secondary"
                          size="sm"
                          className="shrink-0 gap-1.5"
                          onClick={() => void restore(session)}
                          disabled={busy}
                        >
                          {busy ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <ArchiveRestore className="h-3.5 w-3.5" />
                          )}
                          {t("settings.conversationArchive.restore")}
                        </Button>
                      </div>
                    )
                  })}
                </div>
              </section>
            ))}
            {sessions.length < total && (
              <div className="flex justify-center">
                <Button variant="secondary" onClick={() => void loadMore()} disabled={loadingMore}>
                  {loadingMore && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                  {t("settings.conversationArchive.loadMore")}
                </Button>
              </div>
            )}
          </div>
        )}
      </div>

      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.conversationArchive.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.conversationArchive.deleteWarning", {
                title: deleteTarget?.title || t("chat.untitledSession"),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={(event) => {
                event.preventDefault()
                void permanentlyDelete()
              }}
              disabled={deleteTarget ? workingId === deleteTarget.id : false}
            >
              {t("settings.conversationArchive.deletePermanently")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
