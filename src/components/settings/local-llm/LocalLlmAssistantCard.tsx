import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Cpu,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
} from "lucide-react"
import { Ollama } from "@lobehub/icons"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { formatBytesFromMb, formatGbFromMb } from "@/lib/format"
import { cn } from "@/lib/utils"
import { InstallProgressDialog } from "@/components/settings/local-llm/InstallProgressDialog"
import { IconTip } from "@/components/ui/tooltip"
import {
  formatLocalModelJobLogLine,
  isLocalModelJobActive,
  isLocalModelJobTerminal,
  LOCAL_MODEL_JOB_EVENTS,
  localModelJobToProgressFrame,
  phaseTranslationKey,
  type LocalModelJobLogEntry,
  type LocalModelJobSnapshot,
  type ProgressFrame,
} from "@/types/local-model-jobs"
import type { LocalOllamaModel } from "@/types/local-llm"

// ── Wire types (mirror ha_core::local_llm::types) ─────────────────

type BudgetSource = "unified-memory" | "dedicated-vram" | "system-memory"
type OllamaPhase = "not-installed" | "installed" | "running"
type RecommendationReason = "insufficient" | "unified-memory" | "dgpu" | "ram-fallback"

interface GpuInfo {
  name: string
  vramMb?: number | null
}

interface HardwareInfo {
  os: string
  totalMemoryMb: number
  availableMemoryMb: number
  gpu?: GpuInfo | null
  budgetSource: BudgetSource
  budgetMb: number
}

interface ModelCandidate {
  id: string
  displayName: string
  family: string
  sizeMb: number
  contextWindow: number
  reasoning: boolean
}

interface ModelRecommendation {
  hardware: HardwareInfo
  recommended: ModelCandidate | null
  alternatives: ModelCandidate[]
  reason: RecommendationReason
}

interface OllamaStatus {
  phase: OllamaPhase
  baseUrl: string
  installScriptSupported: boolean
}

// ── Helpers ───────────────────────────────────────────────────────

const MAX_DIALOG_LOG_LINES = 240

function reasonText(
  rec: ModelRecommendation,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const hw = rec.hardware
  switch (rec.reason) {
    case "insufficient":
      return t("settings.localLlm.hardware.insufficient")
    case "unified-memory":
      return t("settings.localLlm.hardware.macOs", {
        memory: formatGbFromMb(hw.totalMemoryMb),
        budget: formatGbFromMb(hw.budgetMb),
      })
    case "dgpu":
      return t("settings.localLlm.hardware.dgpu", {
        gpu: hw.gpu?.name ?? "GPU",
        vram: hw.gpu?.vramMb ? formatGbFromMb(hw.gpu.vramMb) : "?",
        budget: formatGbFromMb(hw.budgetMb),
      })
    default:
      return t("settings.localLlm.hardware.ramFallback", {
        memory: formatGbFromMb(hw.totalMemoryMb),
        budget: formatGbFromMb(hw.budgetMb),
      })
  }
}

// ── Component ─────────────────────────────────────────────────────

export default function LocalLlmAssistantCard({
  onProviderInstalled,
  compact = false,
}: {
  onProviderInstalled: () => void
  compact?: boolean
}) {
  const { t } = useTranslation()
  const [recommendation, setRecommendation] = useState<ModelRecommendation | null>(null)
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [installedModels, setInstalledModels] = useState<LocalOllamaModel[]>([])
  const [refreshing, setRefreshing] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [showAlternatives, setShowAlternatives] = useState(false)
  // `null` = follow recommendation; non-null = user explicitly picked.
  const [chosen, setChosen] = useState<ModelCandidate | null>(null)
  const [error, setError] = useState<string | null>(null)

  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogTitle, setDialogTitle] = useState("")
  const [dialogSubtitle, setDialogSubtitle] = useState<string | undefined>(undefined)
  const [dialogFrame, setDialogFrame] = useState<ProgressFrame | null>(null)
  const [dialogLogs, setDialogLogs] = useState<string[]>([])
  const [dialogDone, setDialogDone] = useState(false)
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [currentJob, setCurrentJob] = useState<LocalModelJobSnapshot | null>(null)
  const handledCompletedJobs = useRef<Set<string>>(new Set())
  const recommended = chosen ?? recommendation?.recommended ?? null
  const jobActive = currentJob ? isLocalModelJobActive(currentJob) : false
  const busy = submitting || jobActive

  const appendDialogLog = useCallback((message: string, createdAt?: number) => {
    const trimmed = message.trim()
    if (!trimmed) return
    const line = formatLocalModelJobLogLine(trimmed, createdAt)
    setDialogLogs((prev) => {
      if (prev[prev.length - 1] === line) return prev
      return [...prev.slice(-(MAX_DIALOG_LOG_LINES - 1)), line]
    })
  }, [])

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      // 多拉一次 list_models 是为了：(1) 识别「active_model 仍指向但模型已被
      // 外部 `ollama rm` 删除」的状态，让用户能从这张卡重新拉取修复；(2) 拿到
      // 按 `(provider_id, model_id)` 双键校验过的 `usage.activeModel` flag，
      // 跨 provider 同名 modelId 不会误判。
      const [rec, status, installed] = await Promise.all([
        getTransport().call<ModelRecommendation>("local_llm_recommend_model"),
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
        getTransport()
          .call<LocalOllamaModel[]>("local_llm_list_models")
          .catch(() => [] as LocalOllamaModel[]),
      ])
      setRecommendation(rec)
      setOllama(status)
      setInstalledModels(installed)
    } catch (e) {
      logger.error("local-llm", "refresh", "Failed to detect hardware/ollama", e)
      setError(String(e))
    } finally {
      setRefreshing(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const phaseLabel = useCallback(
    (phase: string | undefined) => {
      const key = phaseTranslationKey(phase)
      return key ? t(key) : (phase ?? "")
    },
    [t],
  )

  const hydrateJobLogs = useCallback(async (jobId: string) => {
    try {
      const entries = await getTransport().call<LocalModelJobLogEntry[]>("local_model_job_logs", {
        jobId,
      })
      setDialogLogs(
        entries
          .slice(-MAX_DIALOG_LOG_LINES)
          .map((entry) => formatLocalModelJobLogLine(entry.message, entry.createdAt)),
      )
    } catch (e) {
      logger.warn("local-llm", "hydrateJobLogs", "Failed to load local model job logs", e)
    }
  }, [])

  const openJobDialog = useCallback(
    (job: LocalModelJobSnapshot) => {
      setCurrentJob(job)
      setDialogOpen(true)
      setDialogTitle(t("settings.localLlm.buttons.installModel", { model: job.displayName }))
      setDialogSubtitle(job.modelId)
      setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
      setDialogLogs([])
      setDialogDone(isLocalModelJobTerminal(job) && !job.error)
      setDialogError(job.error ?? null)
      void hydrateJobLogs(job.jobId)
    },
    [hydrateJobLogs, phaseLabel, t],
  )

  const startModelJob = useCallback(async (model: ModelCandidate) => {
    setSubmitting(true)
    setError(null)
    try {
      const job = await getTransport().call<LocalModelJobSnapshot>(
        "local_model_job_start_chat_model",
        { model },
      )
      openJobDialog(job)
    } catch (e) {
      const msg = String(e)
      logger.error("local-llm", "LocalLlmAssistantCard::startModelJob", "Failed to start chat model job", {
        modelId: model.id,
        error: msg,
      })
      setDialogError(msg)
      setError(t("settings.localLlm.error.pullFailed", { message: msg }))
    } finally {
      setSubmitting(false)
    }
  }, [openJobDialog, t])

  const handleTerminalJob = useCallback((job: LocalModelJobSnapshot) => {
    if (!isLocalModelJobTerminal(job)) return
    if (handledCompletedJobs.current.has(job.jobId)) return
    handledCompletedJobs.current.add(job.jobId)
    if (job.status === "completed") {
      appendDialogLog(t("settings.localLlm.phases.done"), job.updatedAt)
      void refresh()
      onProviderInstalled()
    } else if (job.error) {
      logger.error("local-llm", "LocalLlmAssistantCard::handleTerminalJob", "Chat model job failed", {
        jobId: job.jobId,
        modelId: job.modelId,
        phase: job.phase,
        error: job.error,
      })
      appendDialogLog(job.error, job.updatedAt)
      setError(t("settings.localLlm.error.pullFailed", { message: job.error }))
    }
  }, [appendDialogLog, onProviderInstalled, refresh, t])

  useEffect(() => {
    const handleSnapshot = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      setCurrentJob((current) => {
        if (current?.jobId !== job.jobId) return current
        setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
        setDialogDone(isLocalModelJobTerminal(job) && !job.error)
        setDialogError(job.error ?? null)
        handleTerminalJob(job)
        return job
      })
    }

    const handleLog = (raw: unknown) => {
      const entry = parsePayload<LocalModelJobLogEntry>(raw)
      setCurrentJob((current) => {
        if (current?.jobId !== entry.jobId) return current
        appendDialogLog(entry.message, entry.createdAt)
        return current
      })
    }

    const unlistenUpdated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, handleSnapshot)
    const unlistenCompleted = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.completed, handleSnapshot)
    const unlistenLog = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.log, handleLog)
    return () => {
      unlistenUpdated()
      unlistenCompleted()
      unlistenLog()
    }
  }, [appendDialogLog, handleTerminalJob, phaseLabel])

  const cancelCurrentJob = useCallback(() => {
    const job = currentJob
    if (!job) return
    void getTransport()
      .call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId: job.jobId })
      .catch((e) => {
        const msg = String(e)
        logger.error("local-llm", "LocalLlmAssistantCard::cancelCurrentJob", "Failed to cancel job", {
          jobId: job.jobId,
          error: msg,
        })
        setDialogError(msg)
        setError(msg)
      })
  }, [currentJob])

  const startRecommendedJob = useCallback(() => {
    if (!recommended) return
    void startModelJob(recommended)
  }, [recommended, startModelJob])

  const openDownloadPage = useCallback(() => {
    openExternalUrl("https://ollama.com/download")
  }, [])

  if (!recommendation) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-card/40 p-4">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.localLlm.detecting")}
        </div>
      </div>
    )
  }

  const insufficient = !recommended
  const actionButtonClassName = compact
    ? "h-auto min-h-8 px-2.5 py-1.5 text-xs whitespace-normal"
    : undefined
  // 后端 `UsageIndex::usage_for` 已经按 `(provider_id, model_id)` 双键算好
  // `usage.activeModel` flag，跨 provider 同名 modelId 不会误判；同时 list_models
  // 隐含 installed 状态——单数组 find 一次性满足两条断言。
  const recommendedIsActive = !!recommended
    && (installedModels.find((m) => m.id === recommended.id)?.usage.activeModel ?? false)

  // Decide which primary action is exposed.
  const renderAction = () => {
    if (insufficient || !ollama) return null

    if (ollama.phase === "running" && recommendedIsActive) {
      return (
        <Button variant="secondary" size="sm" className={actionButtonClassName} disabled>
          <CheckCircle2 className="h-3.5 w-3.5 mr-1.5 text-emerald-600 dark:text-emerald-400" />
          {t("settings.localLlm.buttons.alreadyActive", {
            model: recommended?.displayName ?? "",
          })}
        </Button>
      )
    }

    if (ollama.phase === "not-installed") {
      if (!ollama.installScriptSupported) {
        return (
          <Button
            variant="default"
            size="sm"
            className={actionButtonClassName}
            onClick={openDownloadPage}
          >
            <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
            {t("settings.localLlm.buttons.downloadOllama")}
          </Button>
        )
      }
      return (
        <Button
          variant="default"
          size="sm"
          className={actionButtonClassName}
          onClick={startRecommendedJob}
          disabled={busy}
        >
          {busy ? (
            <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
          ) : (
            <Ollama size={14} className="h-3.5 w-3.5 mr-1.5 shrink-0" />
          )}
          {t("settings.localLlm.buttons.installOllama")}
        </Button>
      )
    }

    if (ollama.phase === "installed") {
      return (
        <Button
          variant="default"
          size="sm"
          className={actionButtonClassName}
          onClick={startRecommendedJob}
          disabled={busy}
        >
          {busy ? (
            <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
          ) : (
            <Ollama size={14} className="h-3.5 w-3.5 mr-1.5 shrink-0" />
          )}
          {t("settings.localLlm.buttons.installOllama")}
        </Button>
      )
    }

    return (
      <Button
        variant="default"
        size="sm"
        className={actionButtonClassName}
        onClick={startRecommendedJob}
        disabled={busy || !recommended}
      >
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
        ) : (
          <Download className="h-3.5 w-3.5 mr-1.5" />
        )}
        {t("settings.localLlm.buttons.installModel", {
          model: recommended?.displayName ?? "",
        })}
      </Button>
    )
  }

  return (
    <>
      <div
        className={cn(
          "rounded-xl border border-primary/30 bg-gradient-to-br from-primary/5 to-card",
          compact ? "p-3 space-y-2" : "p-4 space-y-3",
        )}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-3 min-w-0">
            <div
              className={cn(
                "rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0",
                compact ? "w-8 h-8" : "w-9 h-9",
              )}
            >
              <Ollama size={16} className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-foreground">
                {t("settings.localLlm.title")}
              </div>
              {compact ? (
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground mt-0.5 min-w-0">
                  <Cpu className="h-3 w-3 shrink-0" />
                  <span className="truncate">{reasonText(recommendation, t)}</span>
                </div>
              ) : (
                <div className="text-[11px] text-muted-foreground mt-0.5">
                  {t("settings.localLlm.subtitle")}
                </div>
              )}
            </div>
          </div>
          <IconTip label={t("common.refresh")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0"
              onClick={() => void refresh()}
              disabled={refreshing}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${refreshing ? "animate-spin" : ""}`} />
            </Button>
          </IconTip>
        </div>

        {!compact && (
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <Cpu className="h-3 w-3" />
            <span className="truncate">{reasonText(recommendation, t)}</span>
          </div>
        )}

        {recommended ? (
          <div
            className={cn("rounded-lg border border-border/60 bg-card", compact ? "p-2.5" : "p-3")}
          >
            <div
              className={cn(
                "gap-3",
                compact
                  ? "flex flex-col sm:flex-row sm:items-center sm:justify-between"
                  : "flex items-start justify-between",
              )}
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-foreground">
                    {recommended.displayName}
                  </span>
                  <span className="text-[10px] uppercase tracking-wide text-emerald-700 dark:text-emerald-300 bg-emerald-500/10 border border-emerald-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localLlm.recommended")}
                  </span>
                </div>
                <div className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1.5 flex-wrap">
                  <span>{formatBytesFromMb(recommended.sizeMb)}</span>
                  <span>·</span>
                  <span>
                    {t("settings.localLlm.contextWindow", {
                      n: recommended.contextWindow.toLocaleString(),
                    })}
                  </span>
                  {recommended.reasoning && (
                    <>
                      <span>·</span>
                      <span className="text-amber-600 dark:text-amber-400">
                        {t("settings.localLlm.reasoning")}
                      </span>
                    </>
                  )}
                  <span>·</span>
                  <span
                    className={cn(
                      "font-mono text-[10px] text-muted-foreground/70",
                      compact && "hidden sm:inline",
                    )}
                  >
                    {recommended.id}
                  </span>
                </div>
              </div>
              {compact ? (
                <div className="shrink-0 self-start sm:self-auto">{renderAction()}</div>
              ) : (
                ollama?.phase === "running" && (
                  <span className="text-emerald-600 dark:text-emerald-400 flex items-center gap-1 text-[11px] shrink-0">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    {t("settings.localModels.ollama.running")}
                  </span>
                )
              )}
            </div>

            {recommendation.alternatives.length > 1 && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-2 h-auto -ml-2 gap-1 px-2 py-1 text-[11px] font-normal text-muted-foreground hover:bg-transparent hover:text-foreground"
                onClick={() => setShowAlternatives((v) => !v)}
              >
                {showAlternatives ? (
                  <ChevronUp className="h-3 w-3" />
                ) : (
                  <ChevronDown className="h-3 w-3" />
                )}
                {showAlternatives
                  ? t("settings.localLlm.hideAlternatives")
                  : t("settings.localLlm.showAlternatives")}
              </Button>
            )}
            {showAlternatives && (
              <div className="mt-2 space-y-1 border-t border-border/60 pt-2">
                {recommendation.alternatives.map((c) => {
                  const isChosen = recommended?.id === c.id
                  return (
                    <Button
                      key={c.id}
                      type="button"
                      variant="ghost"
                      onClick={() => setChosen(c)}
                      className={cn(
                        "h-auto w-full justify-between gap-2 rounded-md px-2 py-1.5 text-[11px] font-normal",
                        isChosen
                          ? "bg-primary/10 text-foreground hover:bg-primary/15 hover:text-foreground"
                          : "text-muted-foreground hover:bg-secondary",
                      )}
                    >
                      <span className="truncate">{c.displayName}</span>
                      <span className="font-mono text-[10px] text-muted-foreground/80 shrink-0">
                        {formatBytesFromMb(c.sizeMb)}
                      </span>
                    </Button>
                  )
                })}
              </div>
            )}
          </div>
        ) : (
          <div
            className={cn(
              "rounded-lg border border-dashed border-amber-500/30 bg-amber-500/10 text-[11px] text-amber-700 dark:text-amber-300",
              compact ? "p-2.5" : "p-3",
            )}
          >
            {t("settings.localLlm.hardware.insufficient")}
          </div>
        )}

        {!compact && <div className="flex items-center justify-end">{renderAction()}</div>}

        {error && (
          <div className="space-y-1.5">
            <p className="text-[11px] text-destructive whitespace-pre-wrap">{error}</p>
            {ollama?.phase === "not-installed" && (
              <Button
                variant="outline"
                size="sm"
                className="h-auto px-2.5 py-1 text-xs"
                onClick={openDownloadPage}
              >
                <ExternalLink className="h-3 w-3 mr-1.5" />
                {t("settings.localLlm.buttons.downloadOllama")}
              </Button>
            )}
          </div>
        )}
      </div>

      <InstallProgressDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        title={dialogTitle}
        subtitle={dialogSubtitle}
        frame={dialogFrame}
        logs={dialogLogs}
        done={dialogDone}
        error={dialogError}
        cancellable={false}
        onBackground={() => setDialogOpen(false)}
        onCancelTask={currentJob && isLocalModelJobActive(currentJob) ? cancelCurrentJob : undefined}
      />
    </>
  )
}
