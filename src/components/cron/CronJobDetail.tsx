import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
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
import { requestChatFocus } from "@/components/chat/chatFocus"
import {
  ArrowLeft,
  ArrowUpRight,
  Play,
  Pause,
  Trash2,
  Pencil,
  Zap,
  CheckCircle2,
  XCircle,
  Clock,
  FolderOpen,
  Send,
  Loader2,
  CircleSlash,
  Bot,
  Check,
  Minus,
  Square,
  Archive,
  FolderGit2,
  Hand,
  RotateCcw,
  Timer,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { markCronSessionRead } from "@/hooks/useCronUnreadStore"
import type {
  CronJob,
  CronPreflightReport,
  CronRunCancelResult,
  CronRunLog,
  CronRunNowResult,
  CronWorkspaceActionAvailability,
  CronWorkspaceActionResult,
  CronWorkspaceResource,
} from "./CronJobForm.types"
import {
  statusColor,
  statusLabel,
  formatSchedule,
  deliveryTargetLabel,
  cronDisplayTitle,
} from "./cronHelpers"
import type { ProjectMeta } from "@/types/project"
import type { AgentSummaryForSidebar } from "@/types/chat"
import type { LoopSnapshot, LoopState } from "@/components/chat/workspace/useLoopSchedules"
import CronLoopBadge from "./CronLoopBadge"
import CronPreflightDialog from "./CronPreflightDialog"

const LOG_PAGE = 50
type WorkspaceAction = "takeover" | "return" | "resume" | "discard" | "archive" | "restore"
const WORKSPACE_ACTION_KEYS = {
  takeover: "chat.browserPanel.takeOver",
  return: "cron.workspaceActionReturn",
  resume: "cron.workspaceActionResume",
  discard: "diffPanel.git.discard",
  archive: "workspace.worktree.archive",
  restore: "workspace.worktree.restore",
} as const
export function CronWorkspaceResourceCard({
  resource,
  onChanged,
}: {
  resource: CronWorkspaceResource
  onChanged: () => void
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState<string | null>(null)
  const [discardOpen, setDiscardOpen] = useState(false)
  const blockedLabel = (label: string, availability: CronWorkspaceActionAvailability) =>
    availability.allowed
      ? label
      : t("cron.workspaceActionBlocked", { reason: availability.reasonCode ?? "-" })
  const actionLabel = (action: WorkspaceAction) => t(WORKSPACE_ACTION_KEYS[action])
  const act = async (action: WorkspaceAction) => {
    if (busy) return
    setBusy(action)
    try {
      let command = "cron_workspace_takeover"
      let args: Record<string, unknown> = {
        jobId: resource.jobId,
        sessionId: resource.sessionId,
      }
      if (action === "return" || action === "resume") {
        command = "cron_workspace_return"
        args.resume = action === "resume"
      } else if (action === "discard") {
        if (resource.worktree.purpose === "scheduled_run") {
          command = "cron_workspace_discard_run"
          args = { runLogId: resource.runLogId, sessionId: resource.sessionId, confirm: true }
        } else {
          command = "cron_workspace_discard_task"
          args = { jobId: resource.jobId, confirm: true }
        }
      } else if (action === "archive" || action === "restore") {
        command = `${action}_managed_worktree`
        args = { worktreeId: resource.worktree.id }
      }
      const result = await getTransport().call<CronWorkspaceActionResult>(command, args)
      toast.success(t("cron.workspaceActionDone", { action: actionLabel(action) }))
      onChanged()
      if (action === "takeover") {
        const sessionId = result.resource?.sessionId ?? resource.sessionId
        if (sessionId) requestChatFocus({ sessionId })
      }
    } catch (error) {
      logger.error("cron", "CronWorkspaceResourceCard::act", `${action} failed`, error)
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(null)
      setDiscardOpen(false)
    }
  }
  const actionButton = (
    action: Exclude<WorkspaceAction, "discard">,
    availability: CronWorkspaceActionAvailability,
    icon: ReactNode,
  ) => (
    <IconTip label={blockedLabel(actionLabel(action), availability)}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-7 gap-1 px-2 text-[11px]"
        disabled={!availability.allowed || Boolean(busy)}
        onClick={() => void act(action)}
      >
        {busy === action ? <Loader2 className="h-3 w-3 animate-spin" /> : icon}
        {actionLabel(action)}
      </Button>
    </IconTip>
  )
  const summary = resource.worktree.dirtySnapshot
  return (
    <div className="rounded-xl border border-border/60 bg-card px-3.5 py-3 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <FolderGit2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium">
          {resource.mode === "project"
            ? t("cron.project")
            : t(
                resource.mode === "fresh"
                  ? "chat.projectRuntime.worktree"
                  : "cron.workspaceModePersistent",
              )}{" "}
          · {resource.workspaceStatus}
          {resource.worktree.baseSha ? ` · ${resource.worktree.baseSha.slice(0, 8)}` : ""}
        </span>
        {resource.sessionId && (
          <IconTip label={t("subagent.openSession")}>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => requestChatFocus({ sessionId: resource.sessionId! })}
            >
              <ArrowUpRight className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        )}
      </div>
      {summary && !summary.clean && (
        <p className="mt-1 text-muted-foreground">
          {t("workspace.worktree.changed", { count: summary.changedFiles })}
        </p>
      )}
      <div className="mt-2 flex flex-wrap justify-end gap-1">
        {resource.mode === "persistent" &&
          !resource.worktree.handoffSessionId &&
          actionButton("takeover", resource.actions.takeOver, <Hand className="h-3 w-3" />)}
        {resource.mode === "persistent" && resource.worktree.handoffSessionId && (
          <>
            {actionButton(
              "return",
              resource.actions.returnToTask,
              <RotateCcw className="h-3 w-3" />,
            )}
            {actionButton("resume", resource.actions.returnAndResume, <Play className="h-3 w-3" />)}
          </>
        )}
        {resource.worktree.state === "archived"
          ? actionButton("restore", resource.actions.restore, <Play className="h-3 w-3" />)
          : actionButton("archive", resource.actions.archive, <Archive className="h-3 w-3" />)}
        <IconTip label={blockedLabel(t("diffPanel.git.discard"), resource.actions.discard)}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-[11px] text-destructive"
            disabled={!resource.actions.discard.allowed || Boolean(busy)}
            onClick={() => setDiscardOpen(true)}
          >
            <Trash2 className="h-3 w-3" />
            {t("diffPanel.git.discard")}
          </Button>
        </IconTip>
      </div>
      <AlertDialog open={discardOpen} onOpenChange={setDiscardOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("diffPanel.git.discard")}</AlertDialogTitle>
            <AlertDialogDescription>{t("diffPanel.git.discardConfirm")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void act("discard")}
            >
              {t("diffPanel.git.discard")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function JobDetailRow({
  label,
  icon,
  children,
  className,
}: {
  label: ReactNode
  icon?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex min-h-11 items-center justify-between gap-4 border-b border-border/45 px-4 py-2.5 last:border-b-0",
        className,
      )}
    >
      <span className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className="min-w-0 text-right text-xs font-medium">{children}</span>
    </div>
  )
}

interface CronJobDetailProps {
  jobId: string
  /** Agent roster used to resolve the task's configured Agent label. */
  agents: AgentSummaryForSidebar[]
  /** App-level visibility; returning to the persistent view refreshes run data. */
  isViewVisible?: boolean
  onBack: () => void
  onEdit: (job: CronJob) => void
  onDelete: (job: CronJob) => void
  onRefresh: () => void
  /** Embedded in a master-detail pane (list view) — hides the back arrow since
   *  the job list stays visible alongside and selection switches in place. */
  embedded?: boolean
}

export default function CronJobDetail({
  jobId,
  agents,
  isViewVisible = true,
  onBack,
  onEdit,
  onDelete,
  onRefresh,
  embedded = false,
}: CronJobDetailProps) {
  const { t } = useTranslation()
  const [job, setJob] = useState<CronJob | null>(null)
  const [logs, setLogs] = useState<CronRunLog[]>([])
  const [workspaceResources, setWorkspaceResources] = useState<CronWorkspaceResource[] | null>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [cancelling, setCancelling] = useState(false)
  const [logsOffset, setLogsOffset] = useState(0)
  const [logsHasMore, setLogsHasMore] = useState(false)
  const [loadingMoreLogs, setLoadingMoreLogs] = useState(false)
  const [runNowBusy, setRunNowBusy] = useState(false)
  const [runPreflight, setRunPreflight] = useState<CronPreflightReport | null>(null)
  const [archivingSessionId, setArchivingSessionId] = useState<string | null>(null)
  const [loopState, setLoopState] = useState<LoopState | null>(null)
  const wasViewVisibleRef = useRef(isViewVisible)

  const openRunConversation = useCallback(
    (log: CronRunLog) => {
      if (!log.sessionId) return
      // A SessionTurn must retain its exact message anchor; never guess the
      // latest message in a shared conversation when an old row is incomplete.
      if (job?.payload.type === "sessionTurn" && log.targetMessageId == null) return
      void markCronSessionRead(log.sessionId, log.targetMessageId).catch((error) => {
        logger.warn(
          "cron",
          "CronJobDetail::openRunConversation",
          "Failed to mark cron run as read before navigation",
          error,
        )
      })
      requestChatFocus({
        sessionId: log.sessionId,
        targetMessageId: log.targetMessageId ?? undefined,
      })
    },
    [job?.payload.type],
  )

  const handleArchiveRun = useCallback(
    async (log: CronRunLog) => {
      if (archivingSessionId) return
      setArchivingSessionId(log.sessionId)
      try {
        await getTransport().call("set_session_archived_cmd", {
          sessionId: log.sessionId,
          archived: true,
        })
        const remaining = logs.filter((candidate) => candidate.sessionId !== log.sessionId)
        const removedCount = logs.length - remaining.length
        setLogs(remaining)
        setLogsOffset((current) => Math.max(0, current - removedCount))
        window.dispatchEvent(
          new CustomEvent("hope:session-archive-changed", {
            detail: { sessionId: log.sessionId, archived: true },
          }),
        )
        toast.success(t("chat.sessionArchived"), {
          description: log.resultPreview || job?.name,
        })
      } catch (error) {
        logger.error("cron", "CronJobDetail::archive", "Failed to archive cron conversation", error)
        toast.error(t("chat.archiveSessionFailed"), { description: job?.name })
      } finally {
        setArchivingSessionId(null)
      }
    },
    [archivingSessionId, job?.name, logs, t],
  )

  async function fetchData() {
    try {
      const [j, l, resources] = await Promise.all([
        getTransport().call<CronJob | null>("cron_get_job", { id: jobId }),
        getTransport().call<CronRunLog[]>("cron_get_run_logs", { jobId, limit: LOG_PAGE }),
        getTransport()
          .call<CronWorkspaceResource[]>("cron_workspace_resources", { jobId })
          .catch(() => null),
      ])
      setJob(j)
      setLogs(l)
      setWorkspaceResources(Array.isArray(resources) ? resources : null)
      setLogsOffset(l.length)
      setLogsHasMore(l.length === LOG_PAGE)
      if (j?.payload.type === "sessionLoop") {
        const snapshot = await getTransport()
          .call<LoopSnapshot | null>("get_loop_schedule", { loopId: j.payload.loopId })
          .catch(() => null)
        setLoopState(snapshot?.schedule.state ?? null)
      } else {
        setLoopState(null)
      }
      if (j?.projectId) {
        const list = await getTransport().call<ProjectMeta[]>("list_projects_cmd", {
          includeArchived: true,
        })
        setProjects(Array.isArray(list) ? list : [])
      } else {
        setProjects([])
      }
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    setLoopState(null)
    fetchData()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId])

  useEffect(() => {
    const becameVisible = isViewVisible && !wasViewVisibleRef.current
    wasViewVisibleRef.current = isViewVisible
    if (becameVisible) void fetchData()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isViewVisible])

  useEffect(() => {
    if (!isViewVisible) return
    const refresh = () => void fetchData()
    const unlisten = [
      getTransport().listen("chat:turn_queue_changed", refresh),
      getTransport().listen("chat:turn_started", refresh),
      getTransport().listen("cron:run_completed", refresh),
    ]
    return () => unlisten.forEach((stop) => stop())
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isViewVisible, jobId])

  async function handleToggle() {
    if (!job) return
    if (job.payload.type === "sessionLoop") {
      if (job.status === "completed" || loopState === "completed" || loopState === "cancelled")
        return
      await getTransport().call(
        loopState === "active" || (!loopState && job.status === "active")
          ? "pause_loop_schedule"
          : "resume_loop_schedule",
        { loopId: job.payload.loopId },
      )
    } else {
      const enabled = job.status !== "active"
      await getTransport().call("cron_toggle_job", { id: job.id, enabled })
    }
    fetchData()
    onRefresh()
  }

  async function handleRunNow() {
    if (!job) return
    if (job.payload.type === "sessionLoop") {
      if (job.status === "completed" || loopState === "completed" || loopState === "cancelled")
        return
      await getTransport().call("run_loop_schedule_now", { loopId: job.payload.loopId })
    } else {
      setRunNowBusy(true)
      try {
        const report = await getTransport().call<CronPreflightReport>("cron_preflight", {
          request: { operation: "runNow", jobId: job.id, expectedRevision: job.revision },
        })
        setRunPreflight(report)
      } catch (error) {
        logger.error("cron", "CronJobDetail::preflightRunNow", "Run-now preflight failed", error)
        toast.error(t("cron.preflightTitle"), { description: String(error) })
      } finally {
        setRunNowBusy(false)
      }
      return
    }
    // Refresh after a short delay to pick up the run log
    setTimeout(fetchData, 2000)
  }

  async function confirmRunNow() {
    if (!job || job.payload.type === "sessionLoop" || !runPreflight?.canProceed) return
    setRunNowBusy(true)
    try {
      const result = await getTransport().call<CronRunNowResult>("cron_run_now", {
        id: job.id,
        expectedRevision: job.revision,
      })
      if (result.status === "rejected") {
        setRunPreflight(result.report)
        return
      }
      setRunPreflight(null)
      await fetchData()
      onRefresh()
      setTimeout(fetchData, 1500)
    } catch (error) {
      logger.error("cron", "CronJobDetail::runNow", "Run-now start failed", error)
      toast.error(t("cron.preflightTitle"), { description: String(error) })
    } finally {
      setRunNowBusy(false)
    }
  }

  async function handleStopLoop() {
    if (
      !job ||
      job.payload.type !== "sessionLoop" ||
      job.status === "completed" ||
      loopState === "completed" ||
      loopState === "cancelled"
    )
      return
    await getTransport().call("stop_loop_schedule", { loopId: job.payload.loopId })
    await fetchData()
    onRefresh()
  }

  async function handleCancelRun() {
    const run = logs.find((log) => log.status === "queued" || log.status === "running")
    if (!job || job.payload.type === "sessionLoop" || !run || cancelling) return
    setCancelling(true)
    try {
      const result = await getTransport().call<CronRunCancelResult>("cron_cancel_run", {
        runLogId: run.id,
      })
      if (!result.terminal && !result.cancelRequested) {
        throw new Error(result.code ?? "exact cron run was not cancellable")
      }
      await fetchData()
      onRefresh()
    } catch (error) {
      logger.error("cron", "CronJobDetail::cancelRun", "Failed to cancel exact cron run", error)
      toast.error(t("cron.cancelRunFailed"))
    } finally {
      setCancelling(false)
    }
  }

  async function loadMoreLogs() {
    if (loadingMoreLogs || !logsHasMore) return
    setLoadingMoreLogs(true)
    try {
      const page = await getTransport().call<CronRunLog[]>("cron_get_run_logs", {
        jobId,
        limit: LOG_PAGE,
        offset: logsOffset,
      })
      setLogs((prev) => [...prev, ...page])
      setLogsOffset((prev) => prev + page.length)
      setLogsHasMore(page.length === LOG_PAGE)
    } finally {
      setLoadingMoreLogs(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32">
        <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  if (!job) {
    return <div className="p-6 text-center text-muted-foreground">{t("cron.jobNotFound")}</div>
  }
  const project = job.projectId ? projects.find((p) => p.id === job.projectId) : null
  const taskAgentId = job.payload.type === "sessionTurn" ? null : job.payload.agentId
  const agentInfo = taskAgentId ? agents.find((a) => a.id === taskAgentId) : null
  const agentLabel = taskAgentId
    ? agentInfo
      ? `${agentInfo.emoji ? `${agentInfo.emoji} ` : ""}${agentInfo.name}`
      : taskAgentId
    : t("cron.autoAgent")
  const isLoop = job.payload.type === "sessionLoop"
  const sessionTurnPayload = job.payload.type === "sessionTurn" ? job.payload : null
  const isSessionTurn = sessionTurnPayload !== null
  const cancellableRun = !isLoop
    ? logs.find((log) => log.status === "queued" || log.status === "running")
    : null
  const isTerminalLoop =
    isLoop && (job.status === "completed" || loopState === "completed" || loopState === "cancelled")
  const isLoopActive = isLoop && (loopState === "active" || (!loopState && job.status === "active"))
  const isEffectivelyActive = isLoop ? isLoopActive : job.status === "active"
  const displayStatus = isLoop
    ? loopState === "active"
      ? "active"
      : loopState === "completed" || loopState === "cancelled"
        ? "completed"
        : loopState === "blocked"
          ? "disabled"
          : loopState === "paused"
            ? "paused"
            : job.status
    : job.status
  const displayStatusLabel =
    isLoop && loopState
      ? loopState === "active"
        ? t("workspace.loop.stateActive", "运行中")
        : loopState === "paused"
          ? t("workspace.loop.statePaused", "已暂停")
          : loopState === "completed"
            ? t("workspace.loop.stateCompleted", "已完成")
            : loopState === "cancelled"
              ? t("workspace.loop.stateCancelled", "已停止")
              : t("workspace.loop.stateBlocked", "已阻塞")
      : statusLabel(job.status, t)
  const title = cronDisplayTitle(job.name, job.payload.type)
  const canOpenRunConversation = (log: CronRunLog) =>
    Boolean(log.sessionId) && (!isSessionTurn || log.targetMessageId != null)

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex shrink-0 items-center gap-3 px-4 pb-3 pt-3">
        {!embedded && (
          <Button variant="ghost" size="icon" className="h-8 w-8 rounded-lg" onClick={onBack}>
            <ArrowLeft className="h-4 w-4" />
          </Button>
        )}
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <IconTip label={displayStatusLabel}>
              <span className={`inline-block h-2 w-2 rounded-full ${statusColor(displayStatus)}`} />
            </IconTip>
            {isLoop && <CronLoopBadge />}
            <h3 className="truncate text-[15px] font-semibold tracking-tight">{title}</h3>
          </div>
          <p className="mt-0.5 truncate pl-4 text-xs text-muted-foreground">
            {displayStatusLabel}
            {job.description ? ` · ${job.description}` : ""}
          </p>
        </div>
        <div className="flex items-center gap-0.5">
          {(!isLoop || isLoopActive) && (
            <Button
              variant="ghost"
              size="sm"
              className="mr-1 h-8 gap-1.5 rounded-lg bg-primary/10 px-2.5 text-xs text-primary hover:bg-primary/15 hover:text-primary"
              onClick={handleRunNow}
              disabled={runNowBusy}
            >
              {runNowBusy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Zap className="h-3.5 w-3.5" />
              )}
              {t("cron.runNow")}
            </Button>
          )}
          {cancellableRun && (
            <IconTip label={t("common.cancel")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-red-500 hover:text-red-600"
                onClick={handleCancelRun}
                disabled={cancelling}
              >
                <XCircle className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}
          {!isLoop && (
            <IconTip label={t("common.edit")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg"
                onClick={() => onEdit(job)}
              >
                <Pencil className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}
          {!isTerminalLoop && (
            <IconTip label={isEffectivelyActive ? t("cron.pause") : t("cron.resume")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg"
                onClick={handleToggle}
              >
                {isEffectivelyActive ? (
                  <Pause className="h-3.5 w-3.5" />
                ) : (
                  <Play className="h-3.5 w-3.5" />
                )}
              </Button>
            </IconTip>
          )}
          {isLoop ? (
            !isTerminalLoop && (
              <IconTip label={t("chat.loopSlash.stop")}>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 rounded-lg text-muted-foreground hover:text-red-500"
                  onClick={handleStopLoop}
                >
                  <Square className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            )
          ) : (
            <IconTip label={t("common.delete")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-red-500"
                onClick={() => onDelete(job)}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-8">
        <div className="mx-auto w-full max-w-5xl">
          <section className="border-b border-border/55 pb-5 pt-1">
            {job.description && (
              <p className="mb-2 text-sm leading-6 text-muted-foreground">{job.description}</p>
            )}
            <div className="rounded-xl border border-border/60 bg-card px-4 py-3.5 shadow-sm shadow-black/[0.02]">
              <p className="whitespace-pre-wrap break-words text-sm leading-6">
                {job.payload.prompt}
              </p>
            </div>
          </section>

          <section className="mt-5">
            <h4 className="mb-2 text-xs font-semibold">{t("chat.details")}</h4>
            <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
              <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
                <JobDetailRow label={t("cron.schedule")} icon={<Clock className="h-3.5 w-3.5" />}>
                  {formatSchedule(job.schedule, t)}
                </JobDetailRow>
                <JobDetailRow label={t("cron.nextRun")}>
                  {job.nextRunAt ? new Date(job.nextRunAt).toLocaleString() : "-"}
                </JobDetailRow>
                <JobDetailRow label={t("cron.lastRun")}>
                  {job.lastRunAt ? new Date(job.lastRunAt).toLocaleString() : "-"}
                </JobDetailRow>
              </div>

              <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
                {isSessionTurn ? (
                  <>
                    <JobDetailRow
                      label={t("cron.sessionTurnTarget")}
                      icon={<Timer className="h-3.5 w-3.5" />}
                    >
                      <span className="break-all">{sessionTurnPayload?.sessionId}</span>
                    </JobDetailRow>
                    <div className="px-4 py-3 text-xs leading-5 text-muted-foreground">
                      {t("cron.sessionTurnLiveHint")}
                    </div>
                  </>
                ) : (
                  <>
                    <JobDetailRow
                      label={t("cron.project")}
                      icon={<FolderOpen className="h-3.5 w-3.5" />}
                    >
                      {job.projectId
                        ? project
                          ? project.name
                          : t("cron.missingProject")
                        : t("cron.noProject")}
                    </JobDetailRow>
                    <JobDetailRow label={t("cron.agent")} icon={<Bot className="h-3.5 w-3.5" />}>
                      {agentLabel}
                    </JobDetailRow>
                    <JobDetailRow
                      label={t("workspace.environment.worktree")}
                      icon={<FolderGit2 className="h-3.5 w-3.5" />}
                    >
                      {job.workspacePolicy.mode === "project"
                        ? t("cron.project")
                        : t(
                            job.workspacePolicy.mode === "fresh"
                              ? "chat.projectRuntime.worktree"
                              : "cron.workspaceModePersistent",
                          )}
                      {job.workspacePolicy.baseRef ? ` · ${job.workspacePolicy.baseRef}` : ""}
                    </JobDetailRow>
                  </>
                )}
              </div>
            </div>

            <div className="mt-3 grid overflow-hidden rounded-xl border border-border/60 bg-card md:grid-cols-2">
              {job.runningAt && (
                <JobDetailRow label={t("subagent.status.running")}>
                  {new Date(job.runningAt).toLocaleString()}
                </JobDetailRow>
              )}
              <JobDetailRow label={t("cron.failures")}>
                {job.consecutiveFailures} / {job.maxFailures}
              </JobDetailRow>
              <JobDetailRow label={t("cron.jobTimeoutOverride")}>
                {job.jobTimeoutSecs != null
                  ? `${job.jobTimeoutSecs}s`
                  : t("cron.timeoutGlobalDefault")}
              </JobDetailRow>
              <JobDetailRow label={t("notification.cronNotify")}>
                {job.notifyOnComplete ? (
                  <Check className="ml-auto h-3.5 w-3.5 text-emerald-500" />
                ) : (
                  <Minus className="ml-auto h-3.5 w-3.5 text-muted-foreground" />
                )}
              </JobDetailRow>
              <JobDetailRow label={t("cron.prefixDeliveryWithName")}>
                {job.prefixDeliveryWithName ? (
                  <Check className="ml-auto h-3.5 w-3.5 text-emerald-500" />
                ) : (
                  <Minus className="ml-auto h-3.5 w-3.5 text-muted-foreground" />
                )}
              </JobDetailRow>
              <JobDetailRow label={t("cron.deliveryTargets")} className="md:col-span-2">
                {job.deliveryTargets.length === 0 ? (
                  <span className="font-normal text-muted-foreground">
                    {t("cron.noDeliveryTargets")}
                  </span>
                ) : (
                  <span className="flex flex-wrap justify-end gap-1.5">
                    {job.deliveryTargets.map((target, index) => (
                      <span
                        key={index}
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md bg-muted/65 px-2 py-1 text-[11px] font-normal",
                          target.stale && "text-red-500",
                        )}
                      >
                        <Send className="h-3 w-3" />
                        {deliveryTargetLabel(target)}
                        {target.stale && ` · ${t("cron.deliveryTargetStale")}`}
                      </span>
                    ))}
                  </span>
                )}
              </JobDetailRow>
            </div>
          </section>

          {workspaceResources === null ? (
            <p className="mt-5 rounded-xl border border-destructive/25 bg-destructive/5 px-4 py-3 text-xs text-destructive">
              {t("workspace.environment.unavailable")}
            </p>
          ) : workspaceResources.length > 0 ? (
            <section className="mt-5 space-y-2">
              <h4 className="text-xs font-semibold">{t("workspace.goal.detailWorktrees")}</h4>
              {workspaceResources.map((resource) => (
                <CronWorkspaceResourceCard
                  key={resource.worktree.id}
                  resource={resource}
                  onChanged={() => void fetchData()}
                />
              ))}
            </section>
          ) : null}

          <section className="mt-6">
            <div className="mb-2 flex items-center justify-between">
              <h4 className="text-xs font-semibold">{t("cron.runHistory")}</h4>
              {logs.length > 0 && (
                <span className="text-[11px] tabular-nums text-muted-foreground">
                  {logs.length}
                </span>
              )}
            </div>
            {logs.length === 0 ? (
              <div className="py-8 text-center text-xs text-muted-foreground">
                {t("cron.noRuns")}
              </div>
            ) : (
              <div className="space-y-1">
                {logs.map((log) => {
                  const canOpen = canOpenRunConversation(log)
                  return (
                    <div key={log.id} className="group/row relative">
                      <button
                        type="button"
                        disabled={!canOpen}
                        className={cn(
                          "grid w-full grid-cols-[auto_minmax(0,1fr)_auto_auto] items-start gap-x-3 rounded-xl py-3 pr-12 text-left transition-colors",
                          canOpen
                            ? "cursor-pointer hover:bg-secondary/40"
                            : "cursor-default opacity-80",
                        )}
                        onClick={() => openRunConversation(log)}
                      >
                        <span className="mt-0.5 shrink-0">
                          {log.status === "success" ? (
                            <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                          ) : log.status === "queued" ? (
                            <Clock className="h-4 w-4 text-amber-500" />
                          ) : log.status === "running" || log.status === "completing" ? (
                            <Loader2 className="h-4 w-4 animate-spin text-blue-500" />
                          ) : log.status === "cancelling" ? (
                            <Loader2 className="h-4 w-4 animate-spin text-amber-500" />
                          ) : log.status === "empty" ? (
                            <CircleSlash className="h-4 w-4 text-muted-foreground" />
                          ) : log.status === "cancelled" ? (
                            <XCircle className="h-4 w-4 text-muted-foreground" />
                          ) : (
                            <XCircle className="h-4 w-4 text-red-500" />
                          )}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium">{title}</span>
                          <span className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
                            <span
                              className={cn(
                                "font-medium",
                                log.status === "success" &&
                                  "text-emerald-600 dark:text-emerald-400",
                                (log.status === "queued" || log.status === "cancelling") &&
                                  "text-amber-600 dark:text-amber-400",
                                (log.status === "running" || log.status === "completing") &&
                                  "text-blue-600 dark:text-blue-400",
                                log.status === "error" && "text-red-500",
                              )}
                            >
                              {log.status === "success"
                                ? t("cron.runStatusSuccess")
                                : log.status === "queued"
                                  ? t("common.statusValues.queued")
                                  : log.status === "running" || log.status === "completing"
                                    ? t("cron.runStatusRunning")
                                    : log.status === "cancelling"
                                      ? t("common.statusValues.cancelling")
                                      : log.status === "empty"
                                        ? t("cron.runStatusEmpty")
                                        : log.status === "cancelled"
                                          ? t("common.cancel")
                                          : t("cron.runStatusError")}
                            </span>
                            <span className="md:hidden">
                              {new Date(log.startedAt).toLocaleString()}
                            </span>
                            <span className="inline-flex items-center gap-1 md:hidden">
                              <Clock className="h-3 w-3" />
                              {log.durationMs ? `${(log.durationMs / 1000).toFixed(1)}s` : "-"}
                            </span>
                            {log.workspaceStatus && (
                              <span className="inline-flex items-center gap-1">
                                <FolderGit2 className="h-3 w-3" />
                                {log.workspaceStatus}
                              </span>
                            )}
                            {log.deliveryStatus && (
                              <span
                                className={cn(
                                  "inline-flex items-center gap-1",
                                  log.deliveryStatus === "delivered"
                                    ? "text-emerald-600 dark:text-emerald-400"
                                    : log.deliveryStatus === "partial"
                                      ? "text-amber-600 dark:text-amber-400"
                                      : "text-red-500",
                                )}
                              >
                                <Send className="h-3 w-3" />
                                {t(`cron.deliveryStatus.${log.deliveryStatus}`)}
                              </span>
                            )}
                            {log.error ? (
                              <span className="min-w-0 truncate text-red-500">{log.error}</span>
                            ) : log.resultPreview ? (
                              <span className="min-w-0 truncate">{log.resultPreview}</span>
                            ) : null}
                          </span>
                        </span>
                        <span className="hidden shrink-0 text-right text-[11px] text-muted-foreground md:block">
                          <span className="block">{new Date(log.startedAt).toLocaleString()}</span>
                          <span className="mt-1 inline-flex items-center justify-end gap-1">
                            <Clock className="h-3 w-3" />
                            {log.durationMs ? `${(log.durationMs / 1000).toFixed(1)}s` : "-"}
                          </span>
                        </span>
                        {canOpen ? (
                          <ArrowUpRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        ) : (
                          <span className="h-3.5 w-3.5" />
                        )}
                      </button>
                      {log.sessionId && (
                        <IconTip label={t("chat.archiveSession")}>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="absolute right-0 top-1/2 h-7 w-7 -translate-y-1/2 opacity-0 transition-opacity group-hover/row:opacity-100"
                            disabled={archivingSessionId === log.sessionId}
                            onClick={() => void handleArchiveRun(log)}
                          >
                            {archivingSessionId === log.sessionId ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Archive className="h-3.5 w-3.5" />
                            )}
                          </Button>
                        </IconTip>
                      )}
                    </div>
                  )
                })}
              </div>
            )}
            {logsHasMore && (
              <div className="pt-2">
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-8 w-full rounded-lg text-xs text-muted-foreground"
                  disabled={loadingMoreLogs}
                  onClick={() => void loadMoreLogs()}
                >
                  {loadingMoreLogs ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    t("cron.loadMore")
                  )}
                </Button>
              </div>
            )}
          </section>
        </div>
      </div>
      <CronPreflightDialog
        report={runPreflight}
        busy={runNowBusy}
        confirmLabel={t("cron.runNow")}
        onClose={() => setRunPreflight(null)}
        onConfirm={() => void confirmRunNow()}
        onRetry={() => void handleRunNow()}
      />
    </div>
  )
}
