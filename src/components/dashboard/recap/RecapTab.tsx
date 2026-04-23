import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Loader2, Download, RefreshCw, Trash2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type { RecapReport, RecapReportSummary, RecapProgress, GenerateMode } from "../types"

interface Props {
  /** Optional report ID to preload (used when chat `/recap` deep-links here). */
  initialReportId?: string | null
}

export default function RecapTab({ initialReportId }: Props) {
  const { t } = useTranslation()
  const [reports, setReports] = useState<RecapReportSummary[]>([])
  const [selected, setSelected] = useState<string | null>(initialReportId ?? null)
  const [current, setCurrent] = useState<RecapReport | null>(null)
  const [generating, setGenerating] = useState(false)
  const [progress, setProgress] = useState<RecapProgress | null>(null)
  const [rangeDays, setRangeDays] = useState<"incremental" | "7" | "30" | "90">("incremental")
  const [confirmDeleteOpen, setConfirmDeleteOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const loadList = useCallback(async () => {
    try {
      const rows = await getTransport().call<RecapReportSummary[]>("recap_list_reports", {
        limit: 50,
      })
      setReports(rows)
      if (!selected && rows.length > 0) {
        setSelected(rows[0].id)
      }
    } catch (e) {
      logger.error("recap", "loadList", `Failed: ${e}`)
    }
  }, [selected])

  const loadReport = useCallback(async (id: string) => {
    setLoading(true)
    setError(null)
    try {
      const r = await getTransport().call<RecapReport | null>("recap_get_report", {
        id,
      })
      if (r) setCurrent(r)
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadList()
  }, [loadList])

  useEffect(() => {
    if (selected) loadReport(selected)
  }, [selected, loadReport])

  useEffect(() => {
    if (!generating) return
    const transport = getTransport()
    const unlisten = transport.listen("recap_progress", (payload) => {
      const obj = payload as { progress: RecapProgress }
      if (obj?.progress) {
        setProgress(obj.progress)
        if (obj.progress.phase === "done") {
          setGenerating(false)
          void loadList()
        } else if (obj.progress.phase === "failed") {
          setGenerating(false)
          setError(obj.progress.message ?? "generation failed")
        }
      }
    })
    return () => {
      unlisten()
    }
  }, [generating, loadList])

  const onGenerate = useCallback(async () => {
    setError(null)
    setGenerating(true)
    setProgress(null)
    const mode: GenerateMode =
      rangeDays === "incremental"
        ? { mode: "incremental" }
        : {
            mode: "full",
            filters: {
              startDate: new Date(Date.now() - parseInt(rangeDays) * 86400_000)
                .toISOString()
                .slice(0, 10),
              endDate: new Date().toISOString().slice(0, 10),
              agentId: null,
              providerId: null,
              modelId: null,
            },
          }
    try {
      const report = await getTransport().call<RecapReport>("recap_generate", {
        mode,
      })
      setCurrent(report)
      setSelected(report.meta.id)
      await loadList()
    } catch (e) {
      setError(String(e))
    } finally {
      setGenerating(false)
    }
  }, [rangeDays, loadList])

  const onExport = useCallback(async () => {
    if (!current) return
    try {
      const path = await getTransport().call<string>("recap_export_html", {
        id: current.meta.id,
        outputPath: null,
      })
      logger.info("recap", "export", `exported to ${path}`)
      alert(t("recap.exportedTo", { path }))
    } catch (e) {
      setError(String(e))
    }
  }, [current, t])

  const onDelete = useCallback(async () => {
    if (!selected) return
    const reportTitle = current?.meta.title || t("recap.delete")
    try {
      await getTransport().call("recap_delete_report", { id: selected })
      setSelected(null)
      setCurrent(null)
      await loadList()
      toast.success(t("common.deleted"), {
        description: reportTitle,
      })
    } catch (e) {
      setError(String(e))
      toast.error(t("common.deleteFailed"), {
        description: reportTitle,
      })
    }
    setConfirmDeleteOpen(false)
  }, [selected, current, loadList, t])

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <Select value={rangeDays} onValueChange={(v) => setRangeDays(v as typeof rangeDays)}>
          <SelectTrigger className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="incremental">{t("recap.range.incremental")}</SelectItem>
            <SelectItem value="7">{t("recap.range.days", { n: 7 })}</SelectItem>
            <SelectItem value="30">{t("recap.range.days", { n: 30 })}</SelectItem>
            <SelectItem value="90">{t("recap.range.days", { n: 90 })}</SelectItem>
          </SelectContent>
        </Select>
        <Button onClick={onGenerate} disabled={generating} size="sm">
          {generating ? (
            <Loader2 className="w-4 h-4 mr-2 animate-spin" />
          ) : (
            <RefreshCw className="w-4 h-4 mr-2" />
          )}
          {t("recap.generate")}
        </Button>

        {reports.length > 0 && (
          <Select value={selected ?? undefined} onValueChange={(v) => setSelected(v)}>
            <SelectTrigger className="w-72">
              <SelectValue placeholder={t("recap.history")} />
            </SelectTrigger>
            <SelectContent>
              {reports.map((r) => (
                <SelectItem key={r.id} value={r.id}>
                  {r.title} · {r.sessionCount}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}

        {current && (
          <>
            <Button variant="outline" size="sm" onClick={onExport}>
              <Download className="w-4 h-4 mr-2" />
              {t("recap.exportHtml")}
            </Button>
            <Button variant="outline" size="sm" onClick={() => setConfirmDeleteOpen(true)}>
              <Trash2 className="w-4 h-4 mr-2" />
              {t("recap.delete")}
            </Button>
          </>
        )}
      </div>

      {generating && progress && (
        <div className="text-xs text-muted-foreground flex items-center gap-2">
          <Loader2 className="w-3 h-3 animate-spin" />
          {formatProgress(progress, t)}
        </div>
      )}

      {error && <div className="text-sm text-red-500">{error}</div>}

      {!current && !generating && (
        <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
          {reports.length === 0 ? t("recap.noReports") : t("recap.selectReport")}
        </div>
      )}

      {loading && <Loader2 className="w-5 h-5 animate-spin" />}
      {current && !loading && <ReportView report={current} />}

      <AlertDialog open={confirmDeleteOpen} onOpenChange={setConfirmDeleteOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("recap.confirmDelete")}</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void onDelete()}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function formatProgress(p: RecapProgress, t: ReturnType<typeof useTranslation>["t"]): string {
  switch (p.phase) {
    case "started":
      return t("recap.progress.started", { n: p.totalSessions })
    case "extractingFacets":
      return t("recap.progress.extractingFacets", {
        done: p.completed,
        total: p.total,
      })
    case "aggregatingDashboard":
      return t("recap.progress.aggregating")
    case "generatingSections":
      return t("recap.progress.generatingSections", {
        done: p.completed,
        total: p.total,
      })
    case "persisting":
      return t("recap.progress.persisting")
    case "done":
      return t("recap.progress.done")
    case "failed":
      return p.message ?? "failed"
    default:
      return ""
  }
}

function ReportView({ report }: { report: RecapReport }) {
  const { meta, quantitative, facetSummary, sections } = report
  const cur = quantitative.overview.current
  const health = quantitative.health
  const healthClass = healthColor(health.status)

  return (
    <div className="flex flex-col gap-4">
      {/* Header */}
      <div className="rounded-lg border bg-card p-4">
        <div className="text-lg font-semibold">{meta.title}</div>
        <div className="text-xs text-muted-foreground mt-1">
          {meta.generatedAt} · <code>{meta.analysisModel}</code> · {meta.sessionCount} sessions
        </div>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-3">
        <Kpi label="Sessions" value={cur.totalSessions.toString()} />
        <Kpi label="Messages" value={cur.totalMessages.toString()} />
        <Kpi label="Tool calls" value={cur.totalToolCalls.toString()} />
        <Kpi label="Errors" value={cur.totalErrors.toString()} />
        <Kpi label="Cost" value={`$${cur.estimatedCostUsd.toFixed(2)}`} />
        <Kpi label="Avg TTFT" value={cur.avgTtftMs ? `${cur.avgTtftMs.toFixed(0)} ms` : "—"} />
      </div>

      <div className="rounded-lg border bg-card p-4">
        <div className="flex items-center gap-2">
          <div className="text-sm font-medium">Health score</div>
          <div className="text-xl font-semibold">{health.score}/100</div>
          <span className={cn("text-xs px-2 py-0.5 rounded-full", healthClass)}>
            {health.status}
          </span>
        </div>
      </div>

      {sections.map((s) => (
        <div
          key={s.key}
          className={cn(
            "rounded-lg border bg-card p-4",
            s.key === "at_a_glance" &&
              "bg-gradient-to-br from-indigo-50 to-sky-50 dark:from-indigo-950/40 dark:to-sky-950/30",
          )}
        >
          <div className="text-base font-semibold mb-2">{s.title}</div>
          <div className="text-sm whitespace-pre-wrap leading-relaxed">{s.markdown}</div>
        </div>
      ))}

      {facetSummary.totalFacets > 0 && (
        <div className="rounded-lg border bg-card p-4">
          <div className="text-base font-semibold mb-2">Facet breakdown</div>
          <FacetBars title="Top goals" items={facetSummary.goalHistogram} />
          <FacetBars title="Outcomes" items={facetSummary.outcomeDistribution} />
          <FacetBars title="Friction sources" items={facetSummary.frictionTop} />
        </div>
      )}
    </div>
  )
}

function Kpi({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border bg-card p-3">
      <div className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</div>
      <div className="text-xl font-semibold mt-1">{value}</div>
    </div>
  )
}

function FacetBars({ title, items }: { title: string; items: [string, number][] }) {
  if (!items || items.length === 0) return null
  const max = Math.max(...items.map(([, n]) => n), 1)
  return (
    <div className="mb-3 last:mb-0">
      <div className="text-xs font-medium text-muted-foreground mb-1">{title}</div>
      <div className="flex flex-col gap-1">
        {items.map(([k, n]) => (
          <div key={k} className="flex items-center gap-2 text-xs">
            <span className="w-32 truncate text-muted-foreground">{k}</span>
            <span className="flex-1 h-2 bg-muted rounded overflow-hidden">
              <span className="block h-full bg-primary" style={{ width: `${(n / max) * 100}%` }} />
            </span>
            <span className="w-8 text-right tabular-nums">{n}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function healthColor(status: string): string {
  switch (status) {
    case "excellent":
      return "bg-green-500/15 text-green-600 dark:text-green-400"
    case "good":
      return "bg-sky-500/15 text-sky-600 dark:text-sky-400"
    case "warning":
      return "bg-amber-500/15 text-amber-600 dark:text-amber-400"
    case "critical":
      return "bg-red-500/15 text-red-600 dark:text-red-400"
    default:
      return "bg-muted text-muted-foreground"
  }
}
