import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ArrowLeft,
  CheckCircle2,
  Download,
  ListChecks,
  Loader2,
  RefreshCw,
  RotateCcw,
  Trash2,
  XCircle,
} from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import {
  formatLocalModelJobLogLine,
  isLocalModelJobActive,
  LOCAL_MODEL_JOB_EVENTS,
  phaseTranslationKey,
  type LocalModelJobLogEntry,
  type LocalModelJobSnapshot,
  type LocalModelJobStatus,
} from "@/types/local-model-jobs"

function statusTone(status: LocalModelJobStatus): string {
  switch (status) {
    case "completed":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "failed":
    case "interrupted":
      return "border-destructive/30 bg-destructive/10 text-destructive"
    case "cancelled":
    case "paused":
      return "border-muted-foreground/25 bg-muted text-muted-foreground"
    case "cancelling":
      return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "border-primary/25 bg-primary/10 text-primary"
  }
}

function formatTime(seconds: number | null | undefined): string {
  if (!seconds) return ""
  return new Date(seconds * 1000).toLocaleString()
}

export default function TaskCenterView({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation()
  const [jobs, setJobs] = useState<LocalModelJobSnapshot[]>([])
  const [loading, setLoading] = useState(false)
  const [expandedLogs, setExpandedLogs] = useState<Record<string, boolean>>({})
  const [logs, setLogs] = useState<Record<string, LocalModelJobLogEntry[]>>({})
  const [actioning, setActioning] = useState<Record<string, boolean>>({})

  const expandedLogsRef = useRef(expandedLogs)
  useEffect(() => {
    expandedLogsRef.current = expandedLogs
  }, [expandedLogs])

  const sortedJobs = useMemo(
    () => [...jobs].sort((a, b) => b.createdAt - a.createdAt),
    [jobs],
  )
  const activeCount = sortedJobs.filter(isLocalModelJobActive).length

  const phaseLabel = useCallback(
    (phase: string | undefined) => {
      const key = phaseTranslationKey(phase)
      return key ? t(key) : (phase ?? "")
    },
    [t],
  )

  const upsertJob = useCallback((job: LocalModelJobSnapshot) => {
    setJobs((prev) => {
      const idx = prev.findIndex((item) => item.jobId === job.jobId)
      if (idx === -1) return [job, ...prev]
      const next = [...prev]
      next[idx] = job
      return next
    })
  }, [])

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const list = await getTransport().call<LocalModelJobSnapshot[]>("local_model_job_list")
      setJobs(list)
    } catch (e) {
      logger.error("tasks", "TaskCenterView::refresh", "Failed to load model jobs", e)
      toast.error(t("localModelJobs.errors.loadFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void refresh()
  }, [refresh])

  useEffect(() => {
    const onSnapshot = (raw: unknown) => {
      upsertJob(parsePayload<LocalModelJobSnapshot>(raw))
    }
    const onLog = (raw: unknown) => {
      const entry = parsePayload<LocalModelJobLogEntry>(raw)
      if (!expandedLogsRef.current[entry.jobId]) return
      setLogs((prev) => {
        const current = prev[entry.jobId] ?? []
        return { ...prev, [entry.jobId]: [...current.slice(-499), entry] }
      })
    }
    const unlistenCreated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, onSnapshot)
    const unlistenUpdated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, onSnapshot)
    const unlistenCompleted = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.completed, onSnapshot)
    const unlistenLog = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.log, onLog)
    return () => {
      unlistenCreated()
      unlistenUpdated()
      unlistenCompleted()
      unlistenLog()
    }
  }, [upsertJob])

  const loadLogs = useCallback(async (jobId: string) => {
    const entries = await getTransport().call<LocalModelJobLogEntry[]>("local_model_job_logs", {
      jobId,
    })
    setLogs((prev) => ({ ...prev, [jobId]: entries }))
  }, [])

  const toggleLogs = useCallback(
    (jobId: string) => {
      setExpandedLogs((prev) => {
        const nextOpen = !prev[jobId]
        if (nextOpen) {
          void loadLogs(jobId).catch((e) => {
            logger.warn("tasks", "TaskCenterView::loadLogs", "Failed to load logs", e)
          })
        }
        return { ...prev, [jobId]: nextOpen }
      })
    },
    [loadLogs],
  )

  const runAction = useCallback(
    async (jobId: string, action: "cancel" | "retry" | "clear") => {
      setActioning((prev) => ({ ...prev, [jobId]: true }))
      try {
        if (action === "cancel") {
          upsertJob(
            await getTransport().call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId }),
          )
        } else if (action === "retry") {
          const nextJob = await getTransport().call<LocalModelJobSnapshot>("local_model_job_retry", {
            jobId,
          })
          upsertJob(nextJob)
          toast.success(t("localModelJobs.toast.retryStarted", { model: nextJob.displayName }))
        } else {
          await getTransport().call("local_model_job_clear", { jobId })
          setJobs((prev) => prev.filter((job) => job.jobId !== jobId))
          setLogs((prev) => {
            const next = { ...prev }
            delete next[jobId]
            return next
          })
          setExpandedLogs((prev) => {
            const next = { ...prev }
            delete next[jobId]
            return next
          })
          setActioning((prev) => {
            const next = { ...prev }
            delete next[jobId]
            return next
          })
          return
        }
      } catch (e) {
        toast.error(String(e))
      } finally {
        setActioning((prev) => ({ ...prev, [jobId]: false }))
      }
    },
    [t, upsertJob],
  )

  return (
    <div className="flex-1 min-w-0 overflow-y-auto bg-background">
      <div className="mx-auto w-full max-w-5xl px-6 py-6">
        <div className="mb-6 flex items-center justify-between gap-3">
          <div className="flex items-center gap-3 min-w-0">
            <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" onClick={onBack}>
              <ArrowLeft className="h-4 w-4" />
            </Button>
            <div className="min-w-0">
              <h1 className="text-xl font-semibold text-foreground">{t("localModelJobs.title")}</h1>
              <p className="mt-1 text-xs text-muted-foreground">
                {activeCount > 0
                  ? t("localModelJobs.activeSummary", { count: activeCount })
                  : t("localModelJobs.subtitle")}
              </p>
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={() => void refresh()} disabled={loading}>
            <RefreshCw className={cn("mr-2 h-3.5 w-3.5", loading && "animate-spin")} />
            {t("common.refresh")}
          </Button>
        </div>

        {sortedJobs.length === 0 ? (
          <div className="flex min-h-[260px] flex-col items-center justify-center rounded-lg border border-dashed border-border text-center">
            <ListChecks className="mb-3 h-8 w-8 text-muted-foreground" />
            <div className="text-sm font-medium">{t("localModelJobs.emptyTitle")}</div>
            <div className="mt-1 text-xs text-muted-foreground">{t("localModelJobs.emptyDescription")}</div>
          </div>
        ) : (
          <div className="space-y-3">
            {sortedJobs.map((job) => {
              const active = isLocalModelJobActive(job)
              const percent = job.percent ?? null
              const expanded = Boolean(expandedLogs[job.jobId])
              const jobLogs = logs[job.jobId] ?? []
              return (
                <div key={job.jobId} className="rounded-lg border border-border bg-card">
                  <div className="p-4">
                    <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-semibold text-foreground">
                            {job.displayName}
                          </span>
                          <span
                            className={cn(
                              "rounded border px-1.5 py-0.5 text-[10px] font-medium",
                              statusTone(job.status),
                            )}
                          >
                            {t(`localModelJobs.status.${job.status}`)}
                          </span>
                          <span className="rounded border border-border bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                            {t(`localModelJobs.kind.${job.kind}`)}
                          </span>
                        </div>
                        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <span className="font-mono">{job.modelId}</span>
                          <span>·</span>
                          <span>{phaseLabel(job.phase)}</span>
                          <span>·</span>
                          <span>{formatTime(job.updatedAt)}</span>
                        </div>
                        <div className="mt-3 flex items-center gap-3">
                          <Progress
                            value={percent}
                            indeterminate={active && percent == null}
                            className="h-2 flex-1"
                          />
                          {percent != null && (
                            <span className="w-10 text-right text-xs tabular-nums text-muted-foreground">
                              {Math.round(percent)}%
                            </span>
                          )}
                        </div>
                        {job.error && (
                          <p className="mt-2 whitespace-pre-wrap text-xs text-destructive">
                            {job.error}
                          </p>
                        )}
                      </div>

                      <div className="flex shrink-0 flex-wrap items-center gap-2">
                        <Button variant="outline" size="sm" onClick={() => toggleLogs(job.jobId)}>
                          {expanded
                            ? t("localModelJobs.actions.hideLogs")
                            : t("localModelJobs.actions.viewLogs")}
                        </Button>
                        {active && (
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={actioning[job.jobId]}
                            onClick={() => void runAction(job.jobId, "cancel")}
                          >
                            {actioning[job.jobId] ? (
                              <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <XCircle className="mr-2 h-3.5 w-3.5" />
                            )}
                            {t("localModelJobs.actions.cancel")}
                          </Button>
                        )}
                        {!active && job.status !== "completed" && (
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={actioning[job.jobId]}
                            onClick={() => void runAction(job.jobId, "retry")}
                          >
                            <RotateCcw className="mr-2 h-3.5 w-3.5" />
                            {t("localModelJobs.actions.retry")}
                          </Button>
                        )}
                        {!active && (
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={actioning[job.jobId]}
                            onClick={() => void runAction(job.jobId, "clear")}
                          >
                            {job.status === "completed" ? (
                              <CheckCircle2 className="mr-2 h-3.5 w-3.5" />
                            ) : (
                              <Trash2 className="mr-2 h-3.5 w-3.5" />
                            )}
                            {t("localModelJobs.actions.clear")}
                          </Button>
                        )}
                      </div>
                    </div>
                  </div>
                  {expanded && (
                    <div className="border-t border-border bg-muted/30 p-3">
                      {jobLogs.length === 0 ? (
                        <div className="flex items-center gap-2 text-xs text-muted-foreground">
                          <Download className="h-3.5 w-3.5" />
                          {t("localModelJobs.logs.empty")}
                        </div>
                      ) : (
                        <div className="max-h-56 overflow-y-auto rounded-md border border-border/60 bg-background p-2 font-mono text-[11px] leading-tight text-muted-foreground">
                          {jobLogs.map((entry) => (
                            <div key={`${entry.jobId}-${entry.seq}`} className="whitespace-pre-wrap break-all">
                              {formatLocalModelJobLogLine(entry.message, entry.createdAt)}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}
