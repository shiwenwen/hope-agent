import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { BarChart3, CheckCircle2, Loader2, XCircle } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { RecapProgress, RecapReport } from "@/components/dashboard/types"

interface RecapProgressCardProps {
  reportId?: string | null
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
}

export default function RecapProgressCard({
  reportId,
  onOpenDashboardTab,
}: RecapProgressCardProps) {
  const { t } = useTranslation()
  const safeReportId = reportId?.trim() ?? ""
  const [progress, setProgress] = useState<RecapProgress | null>(null)
  const [reportTitle, setReportTitle] = useState<string | null>(null)

  useEffect(() => {
    if (!safeReportId) return
    let cancelled = false
    getTransport()
      .call<RecapReport | null>("recap_get_report", { id: safeReportId })
      .then((report) => {
        if (cancelled || !report) return
        setReportTitle(report.meta.title)
        setProgress({ phase: "done", reportId: safeReportId })
      })
      .catch((e) => {
        logger.debug("recap", "RecapProgressCard::hydrate", "No report yet", e)
      })

    const unlisten = getTransport().listen("recap_progress", (payload) => {
      const raw = payload as { reportId?: string; progress?: RecapProgress }
      const next = raw.progress
      if (!next) return
      const eventReportId =
        raw.reportId ?? ("reportId" in next ? (next as { reportId?: string }).reportId : undefined)
      if (eventReportId !== safeReportId) return

      setProgress(next)
      if (next.phase === "done") {
        getTransport()
          .call<RecapReport | null>("recap_get_report", { id: safeReportId })
          .then((report) => {
            if (report) setReportTitle(report.meta.title)
          })
          .catch(() => {})
      }
    })

    return () => {
      cancelled = true
      unlisten()
    }
  }, [safeReportId])

  if (!safeReportId) {
    return (
      <div className="w-full max-w-[520px] rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive">
        {t("recap.chatCard.missingId", { defaultValue: "Recap report id is missing." })}
      </div>
    )
  }

  const state = progressState(progress)
  const pct = progressPercent(progress)
  const title =
    reportTitle ??
    t("recap.chatCard.title", {
      defaultValue: "Recap report",
    })
  const statusText = progress
    ? formatRecapProgress(progress, t)
    : t("recap.chatCard.waiting", {
        defaultValue: "Waiting for recap progress...",
      })

  const StatusIcon = state === "done" ? CheckCircle2 : state === "failed" ? XCircle : Loader2

  return (
    <div className="w-full max-w-[520px] rounded-lg border border-border-soft bg-surface-panel p-3 text-sm shadow-sm">
      <div className="flex items-start gap-2.5">
        <div
          className={cn(
            "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
            state === "done"
              ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
              : state === "failed"
                ? "bg-destructive/10 text-destructive"
                : "bg-primary/10 text-primary",
          )}
        >
          {state === "running" ? (
            <StatusIcon className="h-4 w-4 animate-spin" />
          ) : (
            <StatusIcon className="h-4 w-4" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <BarChart3 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <div className="truncate font-medium text-foreground">{title}</div>
          </div>
          <div className="mt-1 text-xs text-muted-foreground">{statusText}</div>
          <div className="mt-2">
            <Progress value={pct} indeterminate={state === "running" && pct == null} />
          </div>
          <div className="mt-3 flex items-center justify-between gap-2">
            <code className="truncate text-[11px] text-muted-foreground">
              {safeReportId.slice(0, 8)}
            </code>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => onOpenDashboardTab?.("recap", safeReportId)}
            >
              {t("recap.chatCard.open", { defaultValue: "Open Recap" })}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}

function progressState(progress: RecapProgress | null): "running" | "done" | "failed" {
  if (progress?.phase === "done") return "done"
  if (progress?.phase === "failed") return "failed"
  return "running"
}

function progressPercent(progress: RecapProgress | null): number | null {
  if (!progress) return null
  switch (progress.phase) {
    case "started":
      return 5
    case "extractingFacets":
      return ratioPercent(progress.completed, progress.total, 10, 45)
    case "aggregatingDashboard":
      return 55
    case "generatingSections":
      return ratioPercent(progress.completed, progress.total, 60, 90)
    case "persisting":
      return 95
    case "done":
      return 100
    case "failed":
      return 100
  }
}

function ratioPercent(done: number, total: number, min: number, max: number): number {
  if (total <= 0) return min
  const pct = Math.max(0, Math.min(1, done / total))
  return Math.round(min + (max - min) * pct)
}

function formatRecapProgress(
  progress: RecapProgress,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (progress.phase) {
    case "started":
      return t("recap.progress.started", { n: progress.totalSessions })
    case "extractingFacets":
      return t("recap.progress.extractingFacets", {
        done: progress.completed,
        total: progress.total,
      })
    case "aggregatingDashboard":
      return t("recap.progress.aggregating")
    case "generatingSections":
      return t("recap.progress.generatingSections", {
        done: progress.completed,
        total: progress.total,
      })
    case "persisting":
      return t("recap.progress.persisting")
    case "done":
      return t("recap.progress.done")
    case "failed":
      return progress.message ?? t("recap.progress.failed")
  }
}
