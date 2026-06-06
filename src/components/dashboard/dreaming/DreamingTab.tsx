import { useEffect, useState, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { FileText, Loader2, Moon, Play, RefreshCw } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface DiaryEntry {
  filename: string
  modified: string
  sizeBytes: number
}

// Result of a manual run-now. A skipped run (pre-run gate: disabled / overlap
// / lease contention) has runId=null and is NOT persisted to run history, so
// its note is surfaced ephemerally instead of silently dropped.
interface DreamReport {
  runId?: string | null
  trigger: string
  candidatesScanned: number
  candidatesNominated: number
  promoted: Array<{ memoryId: number; score: number; title: string; rationale: string }>
  diaryPath?: string | null
  durationMs: number
  note?: string | null
}

// Durable run record — mirrors ha-core `DreamingRunRecord` (camelCase).
// Survives restart, unlike the old in-process last-report snapshot.
interface DreamingRun {
  id: string
  trigger: string
  phase: string
  status: string
  startedAt: string
  finishedAt?: string | null
  durationMs: number
  candidatesScanned: number
  candidatesNominated: number
  promotedCount: number
  decisionCount: number
  diaryPath?: string | null
  note?: string | null
}

// Provenance pointer (Evidence Layer) — mirrors ha-core `EvidenceRef`.
interface EvidenceRef {
  sourceType: string
  memoryId?: number | null
  sessionId?: string | null
  messageId?: number | null
}

// Authorized, redacted excerpt — mirrors ha-core `EvidenceQuote`.
interface EvidenceQuote {
  sessionId: string
  messageId?: number | null
  role?: string | null
  quote: string
  truncated: boolean
  available: boolean
  reason?: string | null
}

interface DreamingDecision {
  id: string
  decisionType: string
  targetType: string
  targetId?: string | null
  score?: number | null
  rationale: string
  createdAt: string
  // Provenance lives in the decision's `afterJson` blob (Phase 1 keeps
  // evidence lightweight — no dedicated table). Parsed via `parseEvidence`.
  afterJson?: string | null
}

// Pull evidence refs out of a decision's `afterJson` blob, tolerating the
// pre-Evidence-Layer shape (`{pinned,title}` with no `evidence` key).
function parseEvidence(afterJson?: string | null): EvidenceRef[] {
  if (!afterJson) return []
  try {
    const parsed = JSON.parse(afterJson) as { evidence?: unknown }
    return Array.isArray(parsed.evidence) ? (parsed.evidence as EvidenceRef[]) : []
  } catch {
    return []
  }
}

interface DreamingRunDetail {
  run: DreamingRun
  decisions: DreamingDecision[]
}

const STATUS_DOT: Record<string, string> = {
  running: "bg-amber-500",
  completed: "bg-emerald-500",
  failed: "bg-red-500",
  skipped: "bg-muted-foreground/50",
}

// Evidence chips for one decision, with an authorized expand for session
// sources. The quote is resolved server-side (incognito-gated + redacted),
// so the control never reveals anything the backend wouldn't.
function DecisionEvidence({ refs }: { refs: EvidenceRef[] }) {
  const { t } = useTranslation()
  const [openIdx, setOpenIdx] = useState<number | null>(null)
  const [quote, setQuote] = useState<EvidenceQuote | null>(null)
  const [loadingQuote, setLoadingQuote] = useState(false)

  if (refs.length === 0) return null

  const expand = async (idx: number, sessionId: string, messageId?: number | null) => {
    if (openIdx === idx) {
      setOpenIdx(null)
      return
    }
    setOpenIdx(idx)
    setQuote(null)
    setLoadingQuote(true)
    try {
      const q = await getTransport().call<EvidenceQuote>("dreaming_evidence_quote", {
        sessionId,
        messageId: messageId ?? undefined,
      })
      setQuote(q ?? null)
    } catch (e) {
      logger.error("dashboard", "DreamingTab::evidence", "Failed to load evidence quote", e)
      setQuote(null)
    } finally {
      setLoadingQuote(false)
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-1 mt-1">
      <span className="text-[10px] text-muted-foreground">
        {t("dashboard.dreaming.runs.evidence")}:
      </span>
      {refs.map((r, idx) => {
        if (r.sourceType === "memory" && r.memoryId != null) {
          return (
            <span
              key={idx}
              className="rounded bg-secondary/60 px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground"
            >
              memory #{r.memoryId}
            </span>
          )
        }
        if (r.sourceType === "session_message" && r.sessionId) {
          const sid = r.sessionId
          // Only a precise message anchor can be expanded to the correct
          // source. Phase 1 session refs carry no messageId, so they render
          // as display-only chips; the expand path lights up automatically
          // once claim extraction supplies per-claim message anchors.
          if (r.messageId == null) {
            return (
              <span
                key={idx}
                className="rounded bg-secondary/60 px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground inline-flex items-center gap-1"
              >
                <FileText className="h-3 w-3" />
                session {sid.slice(0, 8)}…
              </span>
            )
          }
          const mid = r.messageId
          return (
            <button
              key={idx}
              onClick={() => void expand(idx, sid, mid)}
              className="rounded bg-secondary/60 px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground hover:bg-secondary inline-flex items-center gap-1"
            >
              <FileText className="h-3 w-3" />
              session {sid.slice(0, 8)}…
            </button>
          )
        }
        return null
      })}
      {openIdx !== null && (
        <div className="w-full mt-1 rounded border border-border/40 bg-muted/30 px-2 py-1.5 text-[11px]">
          {loadingQuote ? (
            <span className="text-muted-foreground inline-flex items-center gap-1">
              <Loader2 className="h-3 w-3 animate-spin" />
              {t("common.loading")}
            </span>
          ) : quote?.available ? (
            <div className="space-y-0.5">
              {quote.role && (
                <span className="font-mono text-[10px] uppercase text-muted-foreground">
                  {quote.role}
                </span>
              )}
              <div className="text-muted-foreground whitespace-pre-wrap break-words">
                {quote.quote}
              </div>
            </div>
          ) : (
            <span className="italic text-muted-foreground">
              {quote?.reason === "incognito"
                ? t("dashboard.dreaming.runs.evidenceIncognito")
                : t("dashboard.dreaming.runs.evidenceUnavailable")}
            </span>
          )}
        </div>
      )}
    </div>
  )
}

export default function DreamingTab() {
  const { t } = useTranslation()
  const [diaries, setDiaries] = useState<DiaryEntry[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [content, setContent] = useState<string>("")
  const [loading, setLoading] = useState(false)
  const [running, setRunning] = useState(false)
  const [runs, setRuns] = useState<DreamingRun[]>([])
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [runDetail, setRunDetail] = useState<DreamingRunDetail | null>(null)
  // Ephemeral note from the most recent manual run that was skipped before a
  // durable row existed (cleared when a real cycle completes).
  const [skipNotice, setSkipNotice] = useState<string | null>(null)
  // Mirror `dreaming.manualEnabled` so flipping the Settings toggle hides
  // the Run-now button instead of leaving it clickable but no-op.
  const [manualEnabled, setManualEnabled] = useState(true)

  const loadDiaries = useCallback(async () => {
    try {
      const list = await getTransport().call<DiaryEntry[]>(
        "dreaming_list_diaries",
        { limit: 100 },
      )
      setDiaries(list ?? [])
    } catch (e) {
      logger.error("dashboard", "DreamingTab::list", "Failed to list diaries", e)
    }
  }, [])

  // Durable run history — the source of truth for the status summary, so it
  // survives a restart (the old `dreaming_last_report` was process-local).
  const loadRuns = useCallback(async () => {
    try {
      const list = await getTransport().call<DreamingRun[]>("dreaming_list_runs", {
        limit: 20,
      })
      setRuns(list ?? [])
    } catch (e) {
      logger.error("dashboard", "DreamingTab::runs", "Failed to list runs", e)
    }
  }, [])

  const loadRunDetail = useCallback(async (runId: string) => {
    try {
      const detail = await getTransport().call<DreamingRunDetail | null>(
        "dreaming_get_run",
        { id: runId },
      )
      setRunDetail(detail ?? null)
    } catch (e) {
      logger.error("dashboard", "DreamingTab::runDetail", "Failed to load run", e)
      setRunDetail(null)
    }
  }, [])

  const loadContent = useCallback(async (filename: string) => {
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
  }, [])

  const refreshStatus = useCallback(async () => {
    try {
      const res = await getTransport().call<boolean | { running: boolean }>("dreaming_is_running")
      const v = typeof res === "boolean" ? res : res?.running ?? false
      setRunning(!!v)
    } catch {
      // Non-fatal.
    }
  }, [])

  const handleRunNow = async () => {
    if (running) return
    setRunning(true)
    setLoading(true)
    setSkipNotice(null)
    try {
      const report = await getTransport().call<DreamReport>("dreaming_run_now")
      // A real cycle gets a durable row (shown in history); a skipped run has
      // no runId — surface its note so the click isn't silent.
      setSkipNotice(report && !report.runId ? report.note ?? null : null)
      await Promise.all([loadDiaries(), loadRuns()])
    } catch (e) {
      logger.error("dashboard", "DreamingTab::run", "Run-now failed", e)
    } finally {
      setRunning(false)
      setLoading(false)
    }
  }

  useEffect(() => {
    loadDiaries()
    loadRuns()
    refreshStatus()
    const unlistenComplete = getTransport().listen("dreaming:cycle_complete", () => {
      setSkipNotice(null) // a real cycle ran — clear any stale skip notice
      loadDiaries()
      loadRuns()
      refreshStatus()
    })
    const unlistenStarted = getTransport().listen("dreaming:cycle_started", () => {
      loadRuns()
      refreshStatus()
    })
    return () => {
      unlistenComplete()
      unlistenStarted()
    }
  }, [loadDiaries, loadRuns, refreshStatus])

  useEffect(() => {
    const sync = async () => {
      try {
        const cfg = await getTransport().call<{ manualEnabled?: boolean }>(
          "get_dreaming_config",
        )
        setManualEnabled(cfg?.manualEnabled ?? true)
      } catch {
        // Non-fatal — keep button visible on read failure.
      }
    }
    void sync()
    return getTransport().listen("config:changed", () => {
      void sync()
    })
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
    if (selected) void loadContent(selected)
  }, [selected, loadContent])

  useEffect(() => {
    if (selectedRunId) void loadRunDetail(selectedRunId)
    else setRunDetail(null)
  }, [selectedRunId, loadRunDetail])

  const latest = runs[0] ?? null

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
              loadRuns()
              refreshStatus()
            }}
            disabled={loading}
          >
            <RefreshCw className="h-3.5 w-3.5 mr-1" />
            {t("common.refresh")}
          </Button>
          {manualEnabled && (
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
          )}
        </div>
      </div>

      {skipNotice && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-muted-foreground italic">
          {skipNotice}
        </div>
      )}

      {latest && (
        <div className="rounded-lg border border-border/60 bg-secondary/20 p-3 text-xs space-y-1">
          <div className="font-medium flex items-center gap-2">
            <span
              className={`h-2 w-2 rounded-full ${STATUS_DOT[latest.status] ?? "bg-muted-foreground/50"}`}
            />
            {t("dashboard.dreaming.lastCycle")} (
            {t(`dashboard.dreaming.trigger.${latest.trigger}`, latest.trigger)})
          </div>
          <div className="text-muted-foreground">
            {t("dashboard.dreaming.scanned", { count: latest.candidatesScanned })} ·{" "}
            {t("dashboard.dreaming.nominated", { count: latest.candidatesNominated })} ·{" "}
            {t("dashboard.dreaming.promoted", { count: latest.promotedCount })} ·{" "}
            {latest.durationMs}ms
          </div>
          {latest.note && <div className="text-muted-foreground italic">{latest.note}</div>}
        </div>
      )}

      <div className="grid grid-cols-[240px_1fr] gap-4">
        {/* Run history list */}
        <div className="border border-border/60 rounded-lg overflow-hidden">
          <div className="px-3 py-2 border-b border-border/60 bg-secondary/20 text-xs font-medium">
            {t("dashboard.dreaming.runs.title")} ({runs.length})
          </div>
          <div className="max-h-[260px] overflow-y-auto">
            {runs.length === 0 ? (
              <div className="px-3 py-6 text-xs text-muted-foreground text-center">
                {t("dashboard.dreaming.runs.empty")}
              </div>
            ) : (
              runs.map((run) => (
                <button
                  key={run.id}
                  onClick={() => setSelectedRunId(run.id)}
                  className={`w-full text-left px-3 py-2 text-xs hover:bg-secondary/40 transition-colors border-b border-border/30 ${
                    selectedRunId === run.id ? "bg-secondary/60 font-medium" : ""
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span
                      className={`h-2 w-2 rounded-full shrink-0 ${STATUS_DOT[run.status] ?? "bg-muted-foreground/50"}`}
                    />
                    <span className="truncate">
                      {t(`dashboard.dreaming.trigger.${run.trigger}`, run.trigger)} ·{" "}
                      {t(`dashboard.dreaming.runs.status.${run.status}`, run.status)}
                    </span>
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5">
                    {new Date(run.startedAt).toLocaleString()} ·{" "}
                    {t("dashboard.dreaming.promoted", { count: run.promotedCount })}
                  </div>
                </button>
              ))
            )}
          </div>
        </div>

        {/* Selected run detail (decisions) */}
        <div className="border border-border/60 rounded-lg p-3 overflow-y-auto max-h-[260px]">
          {runDetail ? (
            <div className="text-xs space-y-2">
              <div className="text-muted-foreground">
                {t("dashboard.dreaming.scanned", {
                  count: runDetail.run.candidatesScanned,
                })}{" "}
                ·{" "}
                {t("dashboard.dreaming.nominated", {
                  count: runDetail.run.candidatesNominated,
                })}{" "}
                ·{" "}
                {t("dashboard.dreaming.promoted", {
                  count: runDetail.run.promotedCount,
                })}{" "}
                · {runDetail.run.durationMs}ms
              </div>
              {runDetail.run.note && (
                <div className="text-muted-foreground italic">{runDetail.run.note}</div>
              )}
              <div className="font-medium pt-1">
                {t("dashboard.dreaming.runs.decisions")} ({runDetail.decisions.length})
              </div>
              {runDetail.decisions.length === 0 ? (
                <div className="text-muted-foreground">
                  {t("dashboard.dreaming.runs.noDecisions")}
                </div>
              ) : (
                <ul className="space-y-1.5">
                  {runDetail.decisions.map((d) => (
                    <li key={d.id} className="rounded border border-border/40 px-2 py-1.5">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[10px] text-muted-foreground">
                          {d.targetType}#{d.targetId ?? "?"}
                        </span>
                        {typeof d.score === "number" && (
                          <span className="text-[10px] text-muted-foreground">
                            {d.score.toFixed(2)}
                          </span>
                        )}
                      </div>
                      <div className="text-muted-foreground">{d.rationale}</div>
                      <DecisionEvidence refs={parseEvidence(d.afterJson)} />
                    </li>
                  ))}
                </ul>
              )}
            </div>
          ) : (
            <div className="text-xs text-muted-foreground text-center py-12">
              {t("dashboard.dreaming.runs.selectRun")}
            </div>
          )}
        </div>
      </div>

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
