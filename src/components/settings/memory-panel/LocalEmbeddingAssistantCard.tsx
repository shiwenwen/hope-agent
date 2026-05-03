import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  CheckCircle2,
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
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { logger } from "@/lib/logger"
import { formatBytesFromMb } from "@/lib/format"
import { cn } from "@/lib/utils"
import { InstallProgressDialog } from "@/components/settings/local-llm/InstallProgressDialog"
import type { MemoryEmbeddingSetDefaultResult, OllamaEmbeddingModel } from "./types"
import type { MemoryEmbeddingState } from "@/types/embedding-models"
import {
  formatLocalModelJobLogLine,
  isJobSuccessorOf,
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

const MAX_DIALOG_LOG_LINES = 240

export default function LocalEmbeddingAssistantCard({
  onActivated,
}: {
  onActivated: (result: MemoryEmbeddingSetDefaultResult) => void
}) {
  const { t } = useTranslation()
  const [models, setModels] = useState<OllamaEmbeddingModel[]>([])
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [memoryEmbedding, setMemoryEmbedding] = useState<MemoryEmbeddingState | null>(null)
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
  const [pendingActivation, setPendingActivation] = useState<OllamaEmbeddingModel | null>(null)
  const handledCompletedJobs = useRef<Set<string>>(new Set())
  // 记录从本卡片启动 / 接力到的所有 jobId。`local_model_job:*` 监听器是全局的，
  // 同时 mount 期间可能有 chat model pull / preload / 手动 reembed 等无关任务
  // 终止；不归属本卡片的任务不应在这里 fire `setError("启用失败")` 误导用户。
  const ownedJobIdsRef = useRef<Set<string>>(new Set())
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
      const [nextModels, status, embeddingState] = await Promise.all([
        getTransport().call<OllamaEmbeddingModel[]>("local_embedding_list_models"),
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
        getTransport().call<MemoryEmbeddingState>("memory_embedding_get"),
      ])
      setModels(nextModels)
      setOllama(status)
      setMemoryEmbedding(embeddingState)
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
    openExternalUrl("https://ollama.com/download")
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
        ownedJobIdsRef.current.add(job.jobId)
        openJobDialog(job)
      } catch (e) {
        const msg = String(e)
        logger.error(
          "local-llm",
          "LocalEmbeddingAssistantCard::activateModel",
          "Failed to start embedding model job",
          {
            modelId: model.id,
            error: msg,
          },
        )
        setDialogError(msg)
        setError(t("settings.localEmbedding.error.activateFailed", { message: msg }))
      } finally {
        setSubmitting(false)
      }
    },
    [ollama, openDownloadPage, openJobDialog, t],
  )

  const handleTerminalJob = useCallback(
    (job: LocalModelJobSnapshot) => {
      if (!isLocalModelJobTerminal(job)) return
      if (!ownedJobIdsRef.current.has(job.jobId)) return
      if (handledCompletedJobs.current.has(job.jobId)) return
      handledCompletedJobs.current.add(job.jobId)
      ownedJobIdsRef.current.delete(job.jobId)
      if (job.status === "completed") {
        appendDialogLog(t("settings.localLlm.phases.done"), job.updatedAt)
        // 仅 embedding pull 任务的 resultJson 形如 `MemoryEmbeddingSetDefaultResult`；
        // 接力到 memory_reembed 后该任务的 resultJson 是 `{reembedded, mode}`，把它
        // 当成 SetDefaultResult 强转给父组件会让 `result.state` 为 undefined，下次
        // 渲染 `selection.enabled` 时崩溃。按 kind 守门只对 embedding_model 触发激活回调。
        if (job.kind === "embedding_model") {
          const result = job.resultJson as MemoryEmbeddingSetDefaultResult | null | undefined
          if (result) onActivated(result)
        }
        void refresh()
      } else if (job.error) {
        logger.error(
          "local-llm",
          "LocalEmbeddingAssistantCard::handleTerminalJob",
          "Embedding model job failed",
          {
            jobId: job.jobId,
            modelId: job.modelId,
            kind: job.kind,
            phase: job.phase,
            error: job.error,
          },
        )
        appendDialogLog(job.error, job.updatedAt)
        // 区分 embedding pull 失败 vs reembed 失败：pull 失败说明模型没下下来，
        // 默认模型也没切；reembed 失败时 pull 已经完成、默认模型已切，只是历史
        // 记忆向量没重建——按「激活失败」报警会误导。
        if (job.kind === "memory_reembed") {
          setError(
            t("settings.embedding.reembedJob.failed") + (job.error ? ` — ${job.error}` : ""),
          )
        } else {
          setError(t("settings.localEmbedding.error.activateFailed", { message: job.error }))
        }
      }
    },
    [appendDialogLog, onActivated, refresh, t],
  )

  useEffect(() => {
    const handleSnapshot = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      // 顶层 fire：接力把 currentJob 从 pull 切到 reembed 后，pull.completed 经
      // setCurrentJob 不匹配会丢 result。handleTerminalJob 内部 ownedJobIdsRef +
      // handledCompletedJobs 去重，重复调用安全。
      handleTerminalJob(job)
      setCurrentJob((current) => {
        if (current?.jobId === job.jobId) {
          setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
          setDialogDone(isLocalModelJobTerminal(job) && !job.error)
          setDialogError(job.error ?? null)
          return job
        }
        // 接力：embedding pull 任务派发的 MemoryReembed 任务通过 `successorForJobId`
        // 指回 pull 任务，dialog 自动跟到 reembed 上展示「重建记忆向量」实时进度。
        if (isJobSuccessorOf(job, current)) {
          ownedJobIdsRef.current.add(job.jobId)
          setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
          setDialogLogs([])
          setDialogDone(isLocalModelJobTerminal(job) && !job.error)
          setDialogError(job.error ?? null)
          setDialogTitle(t("settings.embedding.reembedJob.title"))
          setDialogSubtitle(job.modelId)
          void hydrateJobLogs(job.jobId)
          // 顶层 handleTerminalJob 在 successor 加入 ownedJobIdsRef 之前 early-return
          // 过；事件 WS 重连等场景下首次见到的可能就是 terminal 快照（completed /
          // failed），必须在这里补一次终态处理触发 onActivated / setError。
          handleTerminalJob(job)
          return job
        }
        return current
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

    const unlistenCreated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, handleSnapshot)
    const unlistenUpdated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, handleSnapshot)
    const unlistenCompleted = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.completed, handleSnapshot)
    const unlistenLog = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.log, handleLog)
    return () => {
      unlistenCreated()
      unlistenUpdated()
      unlistenCompleted()
      unlistenLog()
    }
  }, [appendDialogLog, handleTerminalJob, hydrateJobLogs, phaseLabel, t])

  const cancelCurrentJob = useCallback(() => {
    const job = currentJob
    if (!job) return
    void getTransport()
      .call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId: job.jobId })
      .catch((e) => {
        const msg = String(e)
        logger.error(
          "local-llm",
          "LocalEmbeddingAssistantCard::cancelCurrentJob",
          "Failed to cancel job",
          {
            jobId: job.jobId,
            error: msg,
          },
        )
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

  const ollamaStatus = () => {
    if (ollama?.phase !== "running") return null
    return (
      <span className="flex items-center gap-1 text-[11px] shrink-0 text-emerald-600 dark:text-emerald-400">
        <CheckCircle2 className="h-3.5 w-3.5" />
        {t(`settings.localModels.ollama.${ollama.phase}`)}
      </span>
    )
  }

  // 已下载 + ollama 在跑 + 已是默认 memory embedding 模型时显示 disabled「已启用」
  // 按钮；任一不满足都让用户能从这张卡修复（重启 ollama / 重拉外部删掉的 tag）。
  // 后端 ollama EmbeddingModelConfig.apiModel == OllamaEmbeddingModel.id 字面相等，
  // 见 F-054 followup（待后端补 is_active flag 后可去掉这层比对）。
  const recommendedIsActiveEmbedding =
    !!recommended?.installed
    && ollama?.phase === "running"
    && !!memoryEmbedding?.selection.enabled
    && memoryEmbedding?.currentModel?.source === "ollama"
    && memoryEmbedding?.currentModel?.apiModel === recommended.id

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

    if (recommendedIsActiveEmbedding) {
      return (
        <Button variant="secondary" size="sm" disabled>
          <CheckCircle2 className="h-3.5 w-3.5 mr-1.5 text-emerald-600 dark:text-emerald-400" />
          {t("settings.localEmbedding.buttons.alreadyActive", {
            model: recommended.displayName,
          })}
        </Button>
      )
    }

    const label =
      recommended.installed && ollama?.phase === "running"
        ? t("settings.localEmbedding.buttons.enable", { model: recommended.displayName })
        : t("settings.localEmbedding.buttons.activate", { model: recommended.displayName })

    return (
      <Button size="sm" onClick={() => setPendingActivation(recommended)} disabled={busy}>
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
            {ollamaStatus()}
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

        <div className="flex items-center justify-end">{primaryAction()}</div>

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

      <AlertDialog
        open={!!pendingActivation}
        onOpenChange={(open) => !open && setPendingActivation(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.embeddingModels.confirmSwitchTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.localEmbedding.confirmEnableDesc", {
                model: pendingActivation?.displayName ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const model = pendingActivation
                setPendingActivation(null)
                if (model) void activateModel(model)
              }}
            >
              {t("settings.embeddingModels.confirmSwitchAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
