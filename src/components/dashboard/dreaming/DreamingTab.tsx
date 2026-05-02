import { useEffect, useState, useEffectEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Loader2, Moon, Play, RefreshCw } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface DiaryEntry {
  filename: string
  modified: string
  sizeBytes: number
}

interface DreamReport {
  trigger: string
  candidatesScanned: number
  candidatesNominated: number
  promoted: Array<{ memoryId: number; score: number; title: string; rationale: string }>
  diaryPath?: string | null
  durationMs: number
  note?: string | null
}

export default function DreamingTab() {
  const { t } = useTranslation()
  const [diaries, setDiaries] = useState<DiaryEntry[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [content, setContent] = useState<string>("")
  const [loading, setLoading] = useState(false)
  const [running, setRunning] = useState(false)
  const [lastReport, setLastReport] = useState<DreamReport | null>(null)

  const loadDiaries = async () => {
    try {
      const list = await getTransport().call<DiaryEntry[]>("dreaming_list_diaries", { limit: 100 })
      setDiaries(list ?? [])
    } catch (e) {
      logger.error("dashboard", "DreamingTab::list", "Failed to list diaries", e)
    }
  }
  const loadDiariesEffectEvent = useEffectEvent(loadDiaries)

  const loadContent = async (filename: string) => {
    try {
      const res = await getTransport().call<{ filename: string; content: string } | string | null>(
        "dreaming_read_diary",
        { filename },
      )
      const text =
        typeof res === "string"
          ? res
          : res && typeof res === "object" && "content" in res
            ? res.content
            : ""
      setContent(text ?? "")
    } catch (e) {
      logger.error("dashboard", "DreamingTab::read", "Failed to read diary", e)
      setContent("")
    }
  }
  const loadContentEffectEvent = useEffectEvent(loadContent)

  const refreshStatus = async () => {
    try {
      const res = await getTransport().call<boolean | { running: boolean }>("dreaming_is_running")
      const v = typeof res === "boolean" ? res : (res?.running ?? false)
      setRunning(!!v)
    } catch {
      // Non-fatal.
    }
  }
  const refreshStatusEffectEvent = useEffectEvent(refreshStatus)

  const handleRunNow = async () => {
    if (running) return
    setRunning(true)
    setLoading(true)
    try {
      const report = await getTransport().call<DreamReport>("dreaming_run_now")
      setLastReport(report)
      await loadDiaries()
    } catch (e) {
      logger.error("dashboard", "DreamingTab::run", "Run-now failed", e)
    } finally {
      setRunning(false)
      setLoading(false)
    }
  }

  useEffect(() => {
    loadDiariesEffectEvent()
    refreshStatusEffectEvent()
    const unlisten = getTransport().listen("dreaming:cycle_complete", () => {
      loadDiariesEffectEvent()
      refreshStatusEffectEvent()
    })
    return unlisten
  }, [])

  // Auto-select the newest diary when the list first arrives or after a
  // refresh — without adding `selected` to loadDiaries' deps, which would
  // retrigger the listing every time the user picks a different entry.
  useEffect(() => {
    if (!selected && diaries.length > 0) {
      setSelected(diaries[0].filename)
    }
  }, [diaries, selected])

  useEffect(() => {
    if (selected) void loadContentEffectEvent(selected)
  }, [selected])

  return (
    <div className="flex flex-col gap-4 mt-4">
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <h3 className="text-sm font-semibold flex items-center gap-2">
            <Moon className="h-4 w-4 text-muted-foreground" />
            {t("dashboard.dreaming.title")}
          </h3>
          <p className="text-xs text-muted-foreground">{t("dashboard.dreaming.subtitle")}</p>
        </div>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              loadDiaries()
              refreshStatus()
            }}
            disabled={loading}
          >
            <RefreshCw className="h-3.5 w-3.5 mr-1" />
            {t("common.refresh")}
          </Button>
          <Button size="sm" onClick={handleRunNow} disabled={running}>
            {running ? (
              <>
                <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
                {t("dashboard.dreaming.running")}
              </>
            ) : (
              <>
                <Play className="h-3.5 w-3.5 mr-1" />
                {t("dashboard.dreaming.runNow")}
              </>
            )}
          </Button>
        </div>
      </div>

      {lastReport && (
        <div className="rounded-lg border border-border/60 bg-secondary/20 p-3 text-xs space-y-1">
          <div className="font-medium">
            {t("dashboard.dreaming.lastCycle")} (
            {t(`dashboard.dreaming.trigger.${lastReport.trigger}`)})
          </div>
          <div className="text-muted-foreground">
            {t("dashboard.dreaming.scanned", { count: lastReport.candidatesScanned })} ·{" "}
            {t("dashboard.dreaming.nominated", { count: lastReport.candidatesNominated })} ·{" "}
            {t("dashboard.dreaming.promoted", { count: lastReport.promoted.length })} ·{" "}
            {lastReport.durationMs}ms
          </div>
          {lastReport.note && <div className="text-muted-foreground italic">{lastReport.note}</div>}
        </div>
      )}

      <div className="grid grid-cols-[240px_1fr] gap-4 min-h-[400px]">
        <div className="border border-border/60 rounded-lg overflow-hidden">
          <div className="px-3 py-2 border-b border-border/60 bg-secondary/20 text-xs font-medium">
            {t("dashboard.dreaming.diaryList")} ({diaries.length})
          </div>
          <div className="max-h-[600px] overflow-y-auto">
            {diaries.length === 0 ? (
              <div className="px-3 py-6 text-xs text-muted-foreground text-center">
                {t("dashboard.dreaming.empty")}
              </div>
            ) : (
              diaries.map((entry) => (
                <button
                  key={entry.filename}
                  onClick={() => setSelected(entry.filename)}
                  className={`w-full text-left px-3 py-2 text-xs hover:bg-secondary/40 transition-colors border-b border-border/30 ${
                    selected === entry.filename ? "bg-secondary/60 font-medium" : ""
                  }`}
                >
                  <div className="truncate">{entry.filename.replace(/\.md$/, "")}</div>
                  <div className="text-[10px] text-muted-foreground">
                    {(entry.sizeBytes / 1024).toFixed(1)} KB
                  </div>
                </button>
              ))
            )}
          </div>
        </div>

        <div className="border border-border/60 rounded-lg p-4 overflow-y-auto max-h-[720px]">
          {content ? (
            <MarkdownRenderer content={content} />
          ) : (
            <div className="text-xs text-muted-foreground text-center py-12">
              {t("dashboard.dreaming.selectDiary")}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
