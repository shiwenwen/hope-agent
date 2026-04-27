import { useTranslation } from "react-i18next"
import {
  AlertCircle,
  CheckCircle2,
  Loader2,
  RotateCcw,
  StopCircle,
  X,
  Zap,
} from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  isLocalModelJobActive,
  isLocalModelJobTerminal,
  phaseTranslationKey,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface ReembedJobCardProps {
  data: MemoryData
}

/**
 * Always-visible reembed status panel shown at the bottom of the vector search
 * settings page. Three modes:
 *
 * - **Idle** (no tracked job): button to manually re-embed all memories under
 *   the active model. Disabled when vector search is off or the library is empty.
 * - **Active** (running / cancelling): progress bar driven by
 *   `bytesCompleted / bytesTotal` (we use those fields to ship done/total entry
 *   counts via the standard local-model-job channel) plus a Cancel button.
 * - **Terminal** (completed / cancelled / failed / interrupted): result row +
 *   Retry (failed/interrupted) + Dismiss to clear it from the DB.
 */
export default function ReembedJobCard({ data }: ReembedJobCardProps) {
  const { t } = useTranslation()
  const {
    reembedJob,
    dismissReembedJob,
    memoryEmbeddingState,
    totalCount,
    batchLoading,
    handleReembedAll,
  } = data

  const enabled = memoryEmbeddingState.selection.enabled
  const idleDisabled = !enabled || totalCount === 0 || batchLoading

  if (!reembedJob) {
    return (
      <div className="rounded-lg border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-3">
          <div>
            <div className="text-sm font-medium">
              {t("settings.embedding.reembedJob.title")}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {enabled
                ? t("settings.embedding.reembedJob.idle", { count: totalCount })
                : t("settings.embedding.reembedJob.idleDisabled")}
            </div>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={idleDisabled}
            onClick={handleReembedAll}
          >
            {batchLoading ? (
              <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            ) : (
              <Zap className="mr-1.5 h-3.5 w-3.5" />
            )}
            {t("settings.embedding.reembedJob.startNow")}
          </Button>
        </div>
      </div>
    )
  }

  const active = isLocalModelJobActive(reembedJob)
  const terminal = isLocalModelJobTerminal(reembedJob)
  const phaseKey = phaseTranslationKey(reembedJob.phase)
  const phaseLabel = phaseKey ? t(phaseKey) : reembedJob.phase

  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm font-medium">
            <StatusIcon status={reembedJob.status} />
            {t("settings.embedding.reembedJob.title")}
            <StatusBadge status={reembedJob.status} />
          </div>
          <div className="mt-1 text-xs text-muted-foreground">
            {reembedJob.displayName} · {phaseLabel}
          </div>
        </div>
        {active && (
          <CancelButton
            jobId={reembedJob.jobId}
            onError={(msg) => toast.error(msg)}
          />
        )}
      </div>

      {(active || terminal) && (
        <ProgressBar job={reembedJob} />
      )}

      {reembedJob.error && (
        <div className="mt-3 flex items-start gap-2 rounded border border-destructive/30 bg-destructive/5 px-2.5 py-2 text-xs text-destructive">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="break-words">{reembedJob.error}</span>
        </div>
      )}

      {terminal && (
        <div className="mt-3 flex flex-wrap justify-end gap-2 border-t border-border/60 pt-3">
          {(reembedJob.status === "failed" || reembedJob.status === "interrupted") && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                // The new job snapshot arrives via `local_model_job:created` —
                // useMemoryData's subscription replaces the terminal one.
                void getTransport()
                  .call("local_model_job_retry", { jobId: reembedJob.jobId })
                  .catch((e) => {
                    logger.error("settings", "ReembedJobCard::retry", "Failed to retry", e)
                    toast.error(String(e))
                  })
              }}
            >
              <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
              {t("settings.embedding.reembedJob.retry")}
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={dismissReembedJob}>
            <X className="mr-1.5 h-3.5 w-3.5" />
            {t("settings.embedding.reembedJob.dismiss")}
          </Button>
        </div>
      )}
    </div>
  )
}

function StatusIcon({ status }: { status: LocalModelJobSnapshot["status"] }) {
  switch (status) {
    case "running":
    case "cancelling":
      return <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
    case "completed":
      return <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
    case "failed":
    case "interrupted":
      return <AlertCircle className="h-3.5 w-3.5 text-destructive" />
    case "cancelled":
    case "paused":
      return <StopCircle className="h-3.5 w-3.5 text-muted-foreground" />
  }
}

function StatusBadge({ status }: { status: LocalModelJobSnapshot["status"] }) {
  const { t } = useTranslation()
  const cls: Record<LocalModelJobSnapshot["status"], string> = {
    running: "border-primary/30 bg-primary/10 text-primary",
    cancelling: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    completed:
      "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    failed: "border-destructive/30 bg-destructive/10 text-destructive",
    interrupted: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    cancelled: "border-border bg-secondary text-muted-foreground",
    paused: "border-border bg-secondary text-muted-foreground",
  }
  return (
    <span
      className={`rounded border px-1.5 py-0.5 text-[10px] font-medium ${cls[status]}`}
    >
      {t(`settings.embedding.reembedJob.status.${status}`)}
    </span>
  )
}

function CancelButton({
  jobId,
  onError,
}: {
  jobId: string
  onError: (msg: string) => void
}) {
  const { t } = useTranslation()
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={() => {
        void getTransport()
          .call("local_model_job_cancel", { jobId })
          .catch((e) => {
            logger.error("settings", "ReembedJobCard::cancel", "Failed to cancel", e)
            onError(String(e))
          })
      }}
    >
      <StopCircle className="mr-1.5 h-3.5 w-3.5" />
      {t("settings.embedding.reembedJob.cancel")}
    </Button>
  )
}

function ProgressBar({ job }: { job: LocalModelJobSnapshot }) {
  const { t } = useTranslation()
  // done/total entry counts ride on bytesCompleted/bytesTotal so the standard
  // local-model-job channel surfaces a real "N / M memories" indicator.
  const done = job.bytesCompleted ?? 0
  const total = job.bytesTotal ?? 0
  const percent =
    total > 0
      ? Math.min(100, Math.floor((Number(done) / Number(total)) * 100))
      : (job.percent ?? 0)
  return (
    <div className="mt-3">
      <Progress value={percent} />
      <div className="mt-1 flex items-center justify-between text-[11px] text-muted-foreground">
        <span>{t("settings.embedding.reembedJob.progress", { done, total })}</span>
        <span className="font-mono">{percent}%</span>
      </div>
    </div>
  )
}
