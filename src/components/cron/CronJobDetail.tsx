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
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { CronJob, CronRunLog } from "./CronJobForm.types"
import { statusColor, statusLabel, formatSchedule, deliveryTargetLabel } from "./cronHelpers"
import type { ProjectMeta } from "@/types/project"
import type { AgentSummaryForSidebar } from "@/types/chat"
import CronSessionViewer from "./CronSessionViewer"

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
  const [logsOffset, setLogsOffset] = useState(0)
  const [logsHasMore, setLogsHasMore] = useState(false)
  const [loadingMoreLogs, setLoadingMoreLogs] = useState(false)

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
    fetchData()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId])

  // Default to the most recent run that has a session, so opening the detail
  // immediately shows its conversation; never overrides an explicit selection.
  useEffect(() => {
    if (selectedSessionId) return
    const first = logs.find((l) => l.sessionId)
    if (first?.sessionId) setSelectedSessionId(first.sessionId)
  }, [logs, selectedSessionId])

  async function handleToggle() {
    if (!job) return
    const enabled = job.status !== "active"
    await getTransport().call("cron_toggle_job", { id: job.id, enabled })
    fetchData()
    onRefresh()
  }

  async function handleRunNow() {
    if (!job) return
    await getTransport().call("cron_run_now", { id: job.id })
    // Refresh after a short delay to pick up the run log
    setTimeout(fetchData, 2000)
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

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-4 border-b border-border/60">
        {!embedded && (
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onBack}>
            <ArrowLeft className="h-4 w-4" />
          </Button>
        )}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <IconTip label={statusLabel(job.status, t)}>
              <span className={`inline-block w-2 h-2 rounded-full ${statusColor(job.status)}`} />
            </IconTip>
            <h3 className="text-sm font-medium truncate">{job.name}</h3>
          </div>
          {job.description && (
            <p className="text-xs text-muted-foreground truncate mt-0.5">{job.description}</p>
          )}
        </div>
        <div className="flex gap-1">
          <IconTip label={t("cron.runNow")}>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={handleRunNow}>
              <Zap className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          {job.runningAt && (
            <IconTip label={t("common.cancel")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 text-red-500 hover:text-red-600"
                onClick={handleCancelRun}
                disabled={cancelling}
              >
                <XCircle className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}
          <IconTip label={t("common.edit")}>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => onEdit(job)}>
              <Pencil className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={job.status === "active" ? t("cron.pause") : t("cron.resume")}>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={handleToggle}>
              {job.status === "active" ? (
                <Pause className="h-3.5 w-3.5" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
            </Button>
          </IconTip>
          <IconTip label={t("common.delete")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 text-red-500 hover:text-red-600"
              onClick={() => onDelete(job)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      </div>

      {/* Body: left column (info + run history) · right read-only conversation */}
      <div className="flex flex-1 min-h-0">
        <div className="flex w-96 shrink-0 flex-col overflow-y-auto border-r border-border/60 bg-muted/20">
          {/* Info */}
          <div className="px-5 py-4 text-xs space-y-1.5">
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("cron.schedule")}</span>
              <span>{formatSchedule(job.schedule, t)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("cron.nextRun")}</span>
              <span>{job.nextRunAt ? new Date(job.nextRunAt).toLocaleString() : "-"}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("cron.lastRun")}</span>
              <span>{job.lastRunAt ? new Date(job.lastRunAt).toLocaleString() : "-"}</span>
            </div>
            <div className="flex justify-between gap-3">
              <span className="text-muted-foreground">{t("cron.project")}</span>
              <span className="flex min-w-0 items-center gap-1 text-right">
                <FolderOpen className="h-3 w-3 shrink-0 text-muted-foreground" />
                <span className="truncate">
                  {job.projectId
                    ? project
                      ? project.name
                      : t("cron.missingProject")
                    : t("cron.noProject")}
                </span>
              </span>
            </div>
            {job.runningAt && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("subagent.status.running")}</span>
                <span>{new Date(job.runningAt).toLocaleString()}</span>
              </div>
            )}
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("cron.failures")}</span>
              <span>
                {job.consecutiveFailures} / {job.maxFailures}
              </span>
            </div>
            <div className="flex justify-between gap-3">
              <span className="text-muted-foreground">{t("cron.agent")}</span>
              <span className="flex min-w-0 items-center gap-1 text-right">
                <Bot className="h-3 w-3 shrink-0 text-muted-foreground" />
                <span className="truncate">{agentLabel}</span>
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("cron.jobTimeoutOverride")}</span>
              <span>
                {job.jobTimeoutSecs != null
                  ? `${job.jobTimeoutSecs}s`
                  : t("cron.timeoutGlobalDefault")}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">{t("notification.cronNotify")}</span>
              {job.notifyOnComplete ? (
                <Check className="h-3.5 w-3.5 text-emerald-500" />
              ) : (
                <Minus className="h-3.5 w-3.5 text-muted-foreground" />
              )}
            </div>
            {job.deliveryTargets.length > 0 && (
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">{t("cron.prefixDeliveryWithName")}</span>
                {job.prefixDeliveryWithName ? (
                  <Check className="h-3.5 w-3.5 text-emerald-500" />
                ) : (
                  <Minus className="h-3.5 w-3.5 text-muted-foreground" />
                )}
              </div>
            )}
            <div>
              <span className="text-muted-foreground">{t("cron.deliveryTargets")}</span>
              {job.deliveryTargets.length === 0 ? (
                <p className="mt-1 text-muted-foreground/80">{t("cron.noDeliveryTargets")}</p>
              ) : (
                <div className="mt-1 flex flex-col gap-1">
                  {job.deliveryTargets.map((tg, i) => (
                    <div
                      key={i}
                      className={cn(
                        "flex items-center gap-1.5 rounded bg-secondary/30 px-2 py-1",
                        tg.stale && "text-red-500",
                      )}
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
            <div>
              <span className="text-muted-foreground">{t("cron.message")}</span>
              <p className="mt-1 whitespace-pre-wrap break-words bg-secondary/30 rounded px-2 py-1.5">
                {job.payload.prompt}
              </p>
            </div>
          </div>

          {/* Run History */}
          <div className="px-5 py-3">
            <h4 className="text-xs font-medium text-muted-foreground mb-2">
              {t("cron.runHistory")}
            </h4>
            {logs.length === 0 ? (
              <p className="text-xs text-muted-foreground py-4 text-center">{t("cron.noRuns")}</p>
            ) : (
              <div className="space-y-2">
                {logs.map((log) => (
                  <div
                    key={log.id}
                    className={cn(
                      "rounded-lg border border-transparent p-3 text-xs transition-colors",
                      log.sessionId && "cursor-pointer",
                      log.sessionId && selectedSessionId === log.sessionId
                        ? "bg-primary/5 ring-1 ring-inset ring-primary/40"
                        : "bg-card hover:bg-secondary/60",
                    )}
                    onClick={() => log.sessionId && setSelectedSessionId(log.sessionId)}
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
                      <div className="mt-1.5 text-red-500 bg-red-500/5 rounded px-2 py-1 break-words">
                        {log.error}
                      </div>
                    )}
                    {log.resultPreview && (
                      <div className="mt-1.5 text-muted-foreground bg-secondary/30 rounded px-2 py-1 line-clamp-3 break-words">
                        {log.resultPreview}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
            {logsHasMore && (
              <div className="pt-2">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 w-full text-xs"
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
          </div>
        </div>

        {/* Right — read-only conversation of the selected run */}
        <div className="flex flex-1 min-w-0 flex-col">
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
