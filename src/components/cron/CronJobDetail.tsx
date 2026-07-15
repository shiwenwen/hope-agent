import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  ArrowLeft,
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
  MessagesSquare,
  Bot,
  Check,
  Minus,
  ChevronDown,
  Square,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { CronJob, CronRunLog } from "./CronJobForm.types"
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
import CronSessionViewer from "./CronSessionViewer"
import CronLoopBadge from "./CronLoopBadge"

const LOG_PAGE = 50

interface CronJobDetailProps {
  jobId: string
  /** Agent roster for message-bubble identities, fetched once by the parent
   *  (job-independent) so row-switch remounts don't refetch it. */
  agents: AgentSummaryForSidebar[]
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
  onBack,
  onEdit,
  onDelete,
  onRefresh,
  embedded = false,
}: CronJobDetailProps) {
  const { t } = useTranslation()
  const [job, setJob] = useState<CronJob | null>(null)
  const [logs, setLogs] = useState<CronRunLog[]>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [cancelling, setCancelling] = useState(false)
  // Run conversation shown read-only on the right (no jump to the main chat).
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  // Loop runs can share one session, so sessionId cannot identify the selected
  // history row. Keep the run-log id separately for a single visual selection.
  const [selectedLogId, setSelectedLogId] = useState<number | null>(null)
  const [logsOffset, setLogsOffset] = useState(0)
  const [logsHasMore, setLogsHasMore] = useState(false)
  const [loadingMoreLogs, setLoadingMoreLogs] = useState(false)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [loopState, setLoopState] = useState<LoopState | null>(null)

  async function fetchData() {
    try {
      const [j, l] = await Promise.all([
        getTransport().call<CronJob | null>("cron_get_job", { id: jobId }),
        getTransport().call<CronRunLog[]>("cron_get_run_logs", { jobId, limit: LOG_PAGE }),
      ])
      setJob(j)
      setLogs(l)
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
    setSelectedSessionId(null)
    setSelectedLogId(null)
    setDetailsOpen(false)
    setLoopState(null)
    fetchData()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId])

  // Default to the most recent run that has a session, so opening the detail
  // immediately shows its conversation; never overrides an explicit selection.
  useEffect(() => {
    if (selectedLogId !== null && logs.some((log) => log.id === selectedLogId)) return
    const first = logs.find((l) => l.sessionId)
    if (first?.sessionId) {
      setSelectedLogId(first.id)
      setSelectedSessionId(first.sessionId)
    }
  }, [logs, selectedLogId])

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
      await getTransport().call("cron_run_now", { id: job.id })
    }
    // Refresh after a short delay to pick up the run log
    setTimeout(fetchData, 2000)
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
    if (!job?.runningAt || cancelling) return
    setCancelling(true)
    try {
      await getTransport().call("cancel_runtime_task", { kind: "cron", id: job.id })
      await fetchData()
      onRefresh()
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
  const agentInfo = job.payload.agentId ? agents.find((a) => a.id === job.payload.agentId) : null
  const agentLabel = job.payload.agentId
    ? agentInfo
      ? `${agentInfo.emoji ? `${agentInfo.emoji} ` : ""}${agentInfo.name}`
      : job.payload.agentId
    : t("cron.autoAgent")
  const isLoop = job.payload.type === "sessionLoop"
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
          {job.description && (
            <p className="mt-0.5 truncate pl-4 text-xs text-muted-foreground">{job.description}</p>
          )}
        </div>
        <div className="flex items-center gap-0.5">
          {(!isLoop || isLoopActive) && (
            <Button
              variant="ghost"
              size="sm"
              className="mr-1 h-8 gap-1.5 rounded-lg bg-primary/10 px-2.5 text-xs text-primary hover:bg-primary/15 hover:text-primary"
              onClick={handleRunNow}
            >
              <Zap className="h-3.5 w-3.5" />
              {t("cron.runNow")}
            </Button>
          )}
          {job.runningAt && (
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

      {/* Body: left column (info + run history) · right read-only conversation */}
      <div className="flex min-h-0 flex-1 px-3 pb-3">
        <div className="flex w-[20rem] shrink-0 flex-col pr-3">
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            {/* The schedule is the primary fact; secondary configuration stays
                collapsed until requested so run history remains easy to scan. */}
            <section className="rounded-2xl bg-sky-500/[0.055] px-3.5 py-3">
              <div className="flex items-start gap-2.5">
                <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-sky-500/10 text-sky-700 dark:text-sky-300">
                  <Clock className="h-3.5 w-3.5" />
                </span>
                <div className="min-w-0">
                  <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("cron.schedule")}
                  </p>
                  <p className="mt-0.5 text-xs font-medium leading-5">
                    {formatSchedule(job.schedule, t)}
                  </p>
                </div>
              </div>
              <div className="mt-3 grid grid-cols-2 gap-3 pl-0.5 text-[11px]">
                <div className="min-w-0">
                  <p className="text-muted-foreground">{t("cron.nextRun")}</p>
                  <p className="mt-0.5 truncate font-medium">
                    {job.nextRunAt ? new Date(job.nextRunAt).toLocaleString() : "-"}
                  </p>
                </div>
                <div className="min-w-0">
                  <p className="text-muted-foreground">{t("cron.lastRun")}</p>
                  <p className="mt-0.5 truncate font-medium">
                    {job.lastRunAt ? new Date(job.lastRunAt).toLocaleString() : "-"}
                  </p>
                </div>
              </div>
            </section>

            <div className="mt-3 space-y-2 px-1 text-xs">
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">{t("cron.project")}</span>
                <span className="flex min-w-0 items-center gap-1.5 text-right">
                  <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">
                    {job.projectId
                      ? project
                        ? project.name
                        : t("cron.missingProject")
                      : t("cron.noProject")}
                  </span>
                </span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">{t("cron.agent")}</span>
                <span className="flex min-w-0 items-center gap-1.5 text-right">
                  <Bot className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">{agentLabel}</span>
                </span>
              </div>
            </div>

            <button
              type="button"
              aria-expanded={detailsOpen}
              onClick={() => setDetailsOpen((open) => !open)}
              className="mt-2 flex h-8 w-full items-center justify-between rounded-lg px-2 text-xs text-muted-foreground transition-colors hover:bg-muted/45 hover:text-foreground"
            >
              <span>{t("chat.details")}</span>
              <ChevronDown
                className={cn("h-3.5 w-3.5 transition-transform", detailsOpen && "rotate-180")}
              />
            </button>

            {detailsOpen && (
              <div className="space-y-2 px-2 pb-1 pt-1 text-xs">
                {job.runningAt && (
                  <div className="flex justify-between gap-3">
                    <span className="text-muted-foreground">{t("subagent.status.running")}</span>
                    <span className="text-right">{new Date(job.runningAt).toLocaleString()}</span>
                  </div>
                )}
                <div className="flex justify-between gap-3">
                  <span className="text-muted-foreground">{t("cron.failures")}</span>
                  <span>
                    {job.consecutiveFailures} / {job.maxFailures}
                  </span>
                </div>
                <div className="flex justify-between gap-3">
                  <span className="text-muted-foreground">{t("cron.jobTimeoutOverride")}</span>
                  <span>
                    {job.jobTimeoutSecs != null
                      ? `${job.jobTimeoutSecs}s`
                      : t("cron.timeoutGlobalDefault")}
                  </span>
                </div>
                <div className="flex items-center justify-between gap-3">
                  <span className="text-muted-foreground">{t("notification.cronNotify")}</span>
                  {job.notifyOnComplete ? (
                    <Check className="h-3.5 w-3.5 text-emerald-500" />
                  ) : (
                    <Minus className="h-3.5 w-3.5 text-muted-foreground" />
                  )}
                </div>
                {job.deliveryTargets.length > 0 && (
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">
                      {t("cron.prefixDeliveryWithName")}
                    </span>
                    {job.prefixDeliveryWithName ? (
                      <Check className="h-3.5 w-3.5 text-emerald-500" />
                    ) : (
                      <Minus className="h-3.5 w-3.5 text-muted-foreground" />
                    )}
                  </div>
                )}
                <div className="pt-1">
                  <span className="text-muted-foreground">{t("cron.deliveryTargets")}</span>
                  {job.deliveryTargets.length === 0 ? (
                    <p className="mt-1 text-muted-foreground/80">{t("cron.noDeliveryTargets")}</p>
                  ) : (
                    <div className="mt-1.5 flex flex-col gap-1.5">
                      {job.deliveryTargets.map((tg, i) => (
                        <div
                          key={i}
                          className={cn("flex items-center gap-1.5", tg.stale && "text-red-500")}
                        >
                          <Send className="h-3 w-3 shrink-0" />
                          <span className="truncate">{deliveryTargetLabel(tg)}</span>
                          {tg.stale && (
                            <span className="ml-auto shrink-0 text-[10px]">
                              {t("cron.deliveryTargetStale")}
                            </span>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
                <div className="pt-1">
                  <span className="text-muted-foreground">{t("cron.message")}</span>
                  <p className="mt-1 whitespace-pre-wrap break-words rounded-lg bg-muted/35 px-2.5 py-2 leading-5">
                    {job.payload.prompt}
                  </p>
                </div>
              </div>
            )}

            {/* Run History */}
            <section className="mt-4">
              <div className="mb-2 flex items-center justify-between px-1">
                <h4 className="text-xs font-medium">{t("cron.runHistory")}</h4>
                {logs.length > 0 && (
                  <span className="px-1 text-[10px] tabular-nums text-muted-foreground">
                    {logs.length}
                  </span>
                )}
              </div>
              {logs.length === 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">{t("cron.noRuns")}</p>
              ) : (
                <div className="grid auto-rows-max gap-1">
                  {logs.map((log) => (
                    <button
                      type="button"
                      key={log.id}
                      disabled={!log.sessionId}
                      className={cn(
                        "block h-auto w-full self-start rounded-lg px-2.5 py-2 text-left text-xs transition-colors disabled:cursor-default",
                        log.sessionId && "cursor-pointer",
                        log.sessionId && selectedLogId === log.id
                          ? "bg-sky-500/[0.075]"
                          : "hover:bg-sky-500/[0.04]",
                      )}
                      onClick={() => {
                        if (!log.sessionId) return
                        setSelectedLogId(log.id)
                        setSelectedSessionId(log.sessionId)
                      }}
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-1.5">
                          {log.status === "success" ? (
                            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
                          ) : log.status === "running" ? (
                            <Loader2 className="h-3.5 w-3.5 text-blue-500 animate-spin" />
                          ) : log.status === "empty" ? (
                            <CircleSlash className="h-3.5 w-3.5 text-muted-foreground" />
                          ) : log.status === "cancelled" ? (
                            <XCircle className="h-3.5 w-3.5 text-muted-foreground" />
                          ) : (
                            <XCircle className="h-3.5 w-3.5 text-red-500" />
                          )}
                          <span className="font-medium">
                            {log.status === "success"
                              ? t("cron.runStatusSuccess")
                              : log.status === "running"
                                ? t("cron.runStatusRunning")
                                : log.status === "empty"
                                  ? t("cron.runStatusEmpty")
                                  : log.status === "cancelled"
                                    ? t("common.cancel")
                                    : t("cron.runStatusError")}
                          </span>
                        </div>
                        <div className="flex items-center gap-3">
                          <div className="flex items-center gap-1.5 text-muted-foreground">
                            <Clock className="h-3 w-3" />
                            <span>
                              {log.durationMs ? `${(log.durationMs / 1000).toFixed(1)}s` : "-"}
                            </span>
                          </div>
                        </div>
                      </div>
                      <div className="text-muted-foreground mt-1">
                        {new Date(log.startedAt).toLocaleString()}
                      </div>
                      {log.deliveryStatus && (
                        <div className="mt-1 flex items-center gap-1">
                          <Send
                            className={`h-3 w-3 ${
                              log.deliveryStatus === "delivered"
                                ? "text-emerald-500"
                                : log.deliveryStatus === "partial"
                                  ? "text-amber-500"
                                  : "text-red-500"
                            }`}
                          />
                          <span
                            className={
                              log.deliveryStatus === "delivered"
                                ? "text-emerald-600 dark:text-emerald-400"
                                : log.deliveryStatus === "partial"
                                  ? "text-amber-600 dark:text-amber-400"
                                  : "text-red-500"
                            }
                          >
                            {t(`cron.deliveryStatus.${log.deliveryStatus}`)}
                          </span>
                        </div>
                      )}
                      {log.error && (
                        <p className="mt-1.5 line-clamp-2 break-words text-red-500">{log.error}</p>
                      )}
                      {log.resultPreview && (
                        <p className="mt-1.5 line-clamp-2 break-words text-muted-foreground">
                          {log.resultPreview}
                        </p>
                      )}
                    </button>
                  ))}
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

        {/* Right — read-only conversation of the selected run */}
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background">
          {selectedSessionId ? (
            <CronSessionViewer
              key={selectedSessionId}
              sessionId={selectedSessionId}
              agents={agents}
            />
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-muted-foreground">
              <MessagesSquare className="h-10 w-10 opacity-40" />
              <p className="text-sm">{t("cron.conversationsSelectHint")}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
