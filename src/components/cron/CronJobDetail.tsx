import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
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
  ExternalLink,
} from "lucide-react"
import type { CronJob, CronRunLog } from "./CronJobForm"
import { statusColor, formatSchedule } from "./CronJobForm"

interface CronJobDetailProps {
  jobId: string
  onBack: () => void
  onEdit: (job: CronJob) => void
  onRefresh: () => void
  onViewSession?: (sessionId: string) => void
}

export default function CronJobDetail({ jobId, onBack, onEdit, onRefresh, onViewSession }: CronJobDetailProps) {
  const { t } = useTranslation()
  const [job, setJob] = useState<CronJob | null>(null)
  const [logs, setLogs] = useState<CronRunLog[]>([])
  const [loading, setLoading] = useState(true)

  async function fetchData() {
    try {
      const [j, l] = await Promise.all([
        invoke<CronJob | null>("cron_get_job", { id: jobId }),
        invoke<CronRunLog[]>("cron_get_run_logs", { jobId, limit: 50 }),
      ])
      setJob(j)
      setLogs(l)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { fetchData() }, [jobId])

  async function handleToggle() {
    if (!job) return
    const enabled = job.status !== "active"
    await invoke("cron_toggle_job", { id: job.id, enabled })
    fetchData()
    onRefresh()
  }

  async function handleDelete() {
    if (!job) return
    await invoke("cron_delete_job", { id: job.id })
    onBack()
    onRefresh()
  }

  async function handleRunNow() {
    if (!job) return
    await invoke("cron_run_now", { id: job.id })
    // Refresh after a short delay to pick up the run log
    setTimeout(fetchData, 2000)
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32">
        <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  if (!job) {
    return (
      <div className="p-6 text-center text-muted-foreground">
        {t("cron.jobNotFound")}
      </div>
    )
  }

  return (
    <TooltipProvider>
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-4 border-b border-border">
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onBack}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className={`inline-block w-2 h-2 rounded-full ${statusColor(job.status)}`} />
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
          <IconTip label={t("cron.edit")}>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => onEdit(job)}>
              <Pencil className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={job.status === "active" ? t("cron.pause") : t("cron.resume")}>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={handleToggle}>
              {job.status === "active" ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
            </Button>
          </IconTip>
          <IconTip label={t("cron.delete")}>
            <Button variant="ghost" size="icon" className="h-7 w-7 text-red-500 hover:text-red-600" onClick={handleDelete}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      </div>

      {/* Info */}
      <div className="px-5 py-3 border-b border-border text-xs space-y-1.5">
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
        <div className="flex justify-between">
          <span className="text-muted-foreground">{t("cron.failures")}</span>
          <span>{job.consecutiveFailures} / {job.maxFailures}</span>
        </div>
        <div>
          <span className="text-muted-foreground">{t("cron.message")}</span>
          <p className="mt-1 whitespace-pre-wrap break-words bg-secondary/30 rounded px-2 py-1.5">{job.payload.prompt}</p>
        </div>
      </div>

      {/* Run History */}
      <div className="flex-1 overflow-y-auto px-5 py-3">
        <h4 className="text-xs font-medium text-muted-foreground mb-2">{t("cron.runHistory")}</h4>
        {logs.length === 0 ? (
          <p className="text-xs text-muted-foreground py-4 text-center">{t("cron.noRuns")}</p>
        ) : (
          <div className="space-y-2">
            {logs.map((log) => (
              <div
                key={log.id}
                className={`border border-border rounded-lg p-3 text-xs ${onViewSession && log.sessionId ? "cursor-pointer hover:bg-secondary/50 transition-colors" : ""}`}
                onClick={() => onViewSession && log.sessionId && onViewSession(log.sessionId)}
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-1.5">
                    {log.status === "success" ? (
                      <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
                    ) : (
                      <XCircle className="h-3.5 w-3.5 text-red-500" />
                    )}
                    <span className="font-medium">{log.status === "success" ? t("cron.runStatusSuccess") : t("cron.runStatusError")}</span>
                  </div>
                  <div className="flex items-center gap-3">
                    <div className="flex items-center gap-1.5 text-muted-foreground">
                      <Clock className="h-3 w-3" />
                      <span>{log.durationMs ? `${(log.durationMs / 1000).toFixed(1)}s` : "-"}</span>
                    </div>
                    {onViewSession && log.sessionId && (
                      <ExternalLink className="h-3 w-3 text-muted-foreground" />
                    )}
                  </div>
                </div>
                <div className="text-muted-foreground mt-1">
                  {new Date(log.startedAt).toLocaleString()}
                </div>
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
      </div>
    </div>
    </TooltipProvider>
  )
}
