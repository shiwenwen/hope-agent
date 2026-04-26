import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Cpu,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  Sparkles,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { formatBytesFromMb } from "@/lib/format"
import { cn } from "@/lib/utils"
import { InstallProgressDialog } from "@/components/settings/local-llm/InstallProgressDialog"
import type { EmbeddingConfig, OllamaEmbeddingModel } from "./types"
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

type OllamaPhase = "not-installed" | "installed" | "running"

interface OllamaStatus {
  phase: OllamaPhase
  baseUrl: string
  installScriptSupported: boolean
}

interface DesktopOpenResult {
  ok?: boolean
}

const MAX_DIALOG_LOG_LINES = 240

export default function LocalEmbeddingAssistantCard({
  onActivated,
}: {
  onActivated: (config: EmbeddingConfig) => void
}) {
  const { t } = useTranslation()
  const [models, setModels] = useState<OllamaEmbeddingModel[]>([])
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [chosen, setChosen] = useState<OllamaEmbeddingModel | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [showAlternatives, setShowAlternatives] = useState(false)
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
      const [nextModels, status] = await Promise.all([
        getTransport().call<OllamaEmbeddingModel[]>("local_embedding_list_models"),
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
      ])
      setModels(nextModels)
      setOllama(status)
      setChosen((current) =>
        current ? (nextModels.find((model) => model.id === current.id) ?? current) : current,
      )
    } catch (e) {
      logger.error("settings", "LocalEmbeddingAssistant::refresh", "Failed to refresh", e)
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

  const openDownloadPage = useCallback(() => {
    const url = "https://ollama.com/download"
    const openInBrowser = () => window.open(url, "_blank", "noopener")
    void getTransport()
      .call<DesktopOpenResult | void>("open_url", { url })
      .then((result) => {
        if (result && typeof result === "object" && result.ok === false) {
          openInBrowser()
        }
      })
      .catch(openInBrowser)
  }, [])

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
      logger.warn("settings", "LocalEmbeddingAssistant::hydrateJobLogs", "Failed to load logs", e)
    }
  }, [])

  const openJobDialog = useCallback(
    (job: LocalModelJobSnapshot) => {
      setCurrentJob(job)
      setDialogOpen(true)
      setDialogTitle(t("settings.localEmbedding.install.title"))
      setDialogSubtitle(job.modelId)
      setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
      setDialogLogs([])
      setDialogDone(job.status === "completed")
      setDialogError(job.error ?? null)
      void hydrateJobLogs(job.jobId)
    },
    [hydrateJobLogs, phaseLabel, t],
  )

  const activateModel = useCallback(
    async (model: OllamaEmbeddingModel) => {
      if (ollama?.phase === "not-installed" && !ollama.installScriptSupported) {
        openDownloadPage()
        return
      }

      setSubmitting(true)
      setError(null)
      try {
        const job = await getTransport().call<LocalModelJobSnapshot>(
          "local_model_job_start_embedding",
          { model },
        )
        openJobDialog(job)
      } catch (e) {
        const msg = String(e)
        setDialogError(msg)
        setError(t("settings.localEmbedding.error.activateFailed", { message: msg }))
      } finally {
        setSubmitting(false)
      }
    },
    [ollama, openDownloadPage, openJobDialog, t],
  )

  const handleTerminalJob = useCallback((job: LocalModelJobSnapshot) => {
    if (!isLocalModelJobTerminal(job)) return
    if (handledCompletedJobs.current.has(job.jobId)) return
    handledCompletedJobs.current.add(job.jobId)
    if (job.status === "completed") {
      appendDialogLog(t("settings.localLlm.phases.done"), job.updatedAt)
      const config = job.resultJson as EmbeddingConfig | null | undefined
      if (config) onActivated(config)
      void refresh()
    } else if (job.error) {
      appendDialogLog(job.error, job.updatedAt)
      setError(t("settings.localEmbedding.error.activateFailed", { message: job.error }))
    }
  }, [appendDialogLog, onActivated, refresh, t])

  useEffect(() => {
    const handleSnapshot = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      setCurrentJob((current) => {
        if (current?.jobId !== job.jobId) return current
        setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
        setDialogDone(job.status === "completed")
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
        setDialogError(msg)
        setError(msg)
      })
  }, [currentJob])

  const recommended = chosen ?? models.find((model) => model.recommended) ?? models[0] ?? null

  if (!recommended) {
    return (
      <div className="rounded-lg border border-dashed border-border bg-card/40 p-3">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.localEmbedding.detecting")}
        </div>
      </div>
    )
  }

  const primaryAction = () => {
    if (!ollama) return null

    if (ollama?.phase === "not-installed" && !ollama.installScriptSupported) {
      return (
        <Button variant="secondary" size="sm" onClick={openDownloadPage}>
          <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
          {t("settings.localEmbedding.buttons.downloadOllama")}
        </Button>
      )
    }

    const label =
      recommended.installed && ollama?.phase === "running"
        ? t("settings.localEmbedding.buttons.enable", { model: recommended.displayName })
        : t("settings.localEmbedding.buttons.activate", { model: recommended.displayName })

    return (
      <Button size="sm" onClick={() => void activateModel(recommended)} disabled={busy}>
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
        ) : (
          <Download className="h-3.5 w-3.5 mr-1.5" />
        )}
        {label}
      </Button>
    )
  }

  return (
    <>
      <div className="rounded-lg border border-primary/25 bg-primary/5 p-3 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-3 min-w-0">
            <div className="w-8 h-8 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0">
              <Sparkles className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-foreground">
                {t("settings.localEmbedding.title")}
              </div>
              <div className="text-[11px] text-muted-foreground mt-0.5">
                {t("settings.localEmbedding.subtitle")}
              </div>
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

        <div className="rounded-lg border border-border/60 bg-card p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-medium text-foreground">
                  {recommended.displayName}
                </span>
                {recommended.recommended && (
                  <span className="text-[10px] uppercase tracking-wide text-emerald-700 dark:text-emerald-300 bg-emerald-500/10 border border-emerald-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localEmbedding.recommended")}
                  </span>
                )}
                {recommended.installed && (
                  <span className="text-[10px] uppercase tracking-wide text-sky-700 dark:text-sky-300 bg-sky-500/10 border border-sky-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localEmbedding.installed")}
                  </span>
                )}
              </div>
              <div className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1.5 flex-wrap">
                <Cpu className="h-3 w-3" />
                <span>{formatBytesFromMb(recommended.sizeMb)}</span>
                <span>·</span>
                <span>
                  {t("settings.localEmbedding.dimensions", { n: recommended.dimensions })}
                </span>
                <span>·</span>
                <span>
                  {t("settings.localEmbedding.contextWindow", {
                    n: recommended.contextWindow.toLocaleString(),
                  })}
                </span>
                <span>·</span>
                <span>{recommended.languages.join(", ")}</span>
                {recommended.minOllamaVersion && (
                  <>
                    <span>·</span>
                    <span>Ollama {recommended.minOllamaVersion}+</span>
                  </>
                )}
              </div>
            </div>
            <div className="shrink-0">{primaryAction()}</div>
          </div>

          {models.length > 1 && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-2 h-7 px-2 text-[11px] text-muted-foreground"
              onClick={() => setShowAlternatives((v) => !v)}
            >
              {showAlternatives ? (
                <ChevronUp className="h-3 w-3 mr-1" />
              ) : (
                <ChevronDown className="h-3 w-3 mr-1" />
              )}
              {showAlternatives
                ? t("settings.localEmbedding.hideAlternatives")
                : t("settings.localEmbedding.showAlternatives")}
            </Button>
          )}

          {showAlternatives && (
            <div className="mt-2 space-y-1 border-t border-border/60 pt-2">
              {models.map((model) => {
                const active = model.id === recommended.id
                return (
                  <Button
                    key={model.id}
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setChosen(model)}
                    className={cn(
                      "w-full h-auto justify-between px-2 py-1.5 text-left text-[11px]",
                      active
                        ? "bg-primary/10 text-foreground"
                        : "text-muted-foreground hover:bg-secondary",
                    )}
                  >
                    <span className="truncate">{model.displayName}</span>
                    <span className="font-mono text-[10px] text-muted-foreground/80 shrink-0">
                      {formatBytesFromMb(model.sizeMb)} · {model.dimensions}d
                    </span>
                  </Button>
                )
              })}
            </div>
          )}
        </div>

        {error && <p className="text-[11px] text-destructive whitespace-pre-wrap">{error}</p>}
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
