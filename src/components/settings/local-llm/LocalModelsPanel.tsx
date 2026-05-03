import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertTriangle,
  Brain,
  Database,
  Download,
  Loader2,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  Square,
  Star,
  Trash2,
} from "lucide-react"
import { Ollama } from "@lobehub/icons"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Progress } from "@/components/ui/progress"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
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
import { InstallProgressDialog } from "@/components/settings/local-llm/InstallProgressDialog"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { formatBytes } from "@/lib/format"
import { formatJobTransferLine } from "@/lib/format-job-transfer"
import {
  formatLocalModelJobLogLine,
  isJobSuccessorOf,
  isLocalModelJobActive,
  isLocalModelJobResumable,
  isLocalModelJobTerminal,
  isLocalModelJobVisible,
  LOCAL_MODEL_JOB_EVENTS,
  localModelJobPercent,
  localModelJobToProgressFrame,
  phaseTranslationKey,
  type LocalModelJobLogEntry,
  type LocalModelJobSnapshot,
  type ProgressFrame,
} from "@/types/local-model-jobs"
import type {
  LocalOllamaModel,
  ModelCandidate,
  ModelRecommendation,
  OllamaLibraryModel,
  OllamaLibraryModelDetail,
  OllamaLibrarySearchResponse,
  OllamaPullRequest,
  OllamaStatus,
} from "@/types/local-llm"
import { openExternalUrl } from "@/lib/openExternalUrl"

const MAX_DIALOG_LOG_LINES = 240

const JOB_ACTION_COMMANDS = {
  pause: "local_model_job_pause",
  cancel: "local_model_job_cancel",
  resume: "local_model_job_retry",
} as const
const CAPABILITY_FILTERS = ["all", "embedding", "vision", "tools", "thinking"] as const
const ACTION_BUTTON_CLASS = "shrink-0 whitespace-nowrap"
const ACTION_ICON_CLASS = "mr-2 h-3.5 w-3.5 shrink-0"
const APPROX_Q4_MB_PER_BILLION_PARAMS = 560

function sizeLabel(bytes?: number | null): string {
  return bytes ? formatBytes(bytes, { maxUnit: "GB", trimTrailingZeros: true }) : "-"
}

function contextLabel(value?: number | null): string {
  if (!value) return "-"
  return value >= 1000 ? `${Math.round(value / 1000)}K` : String(value)
}

function candidateSizeLabel(sizeMb: number): string {
  return formatBytes(sizeMb * 1024 * 1024, { maxUnit: "GB", trimTrailingZeros: true })
}

function exceedsBudgetMb(sizeMb?: number | null, budgetMb?: number | null): boolean {
  return sizeMb != null && budgetMb != null && sizeMb > budgetMb
}

function exceedsBudgetBytes(sizeBytes?: number | null, budgetMb?: number | null): boolean {
  return sizeBytes != null && budgetMb != null && sizeBytes > budgetMb * 1024 * 1024
}

function approximateSizeMbFromLibraryLabel(label: string): number | null {
  const diskSize = label.match(/(\d+(?:\.\d+)?)\s*(tb|gb|mb)\b/i)
  if (diskSize) {
    const value = Number(diskSize[1])
    const unit = diskSize[2].toLowerCase()
    if (unit === "tb") return value * 1024 * 1024
    if (unit === "gb") return value * 1024
    return value
  }

  const parameterSize = label.match(/(\d+(?:\.\d+)?)\s*b\b/i)
  if (!parameterSize) return null
  return Number(parameterSize[1]) * APPROX_Q4_MB_PER_BILLION_PARAMS
}

function librarySizeExceedsBudget(label: string, budgetMb?: number | null): boolean {
  return exceedsBudgetMb(approximateSizeMbFromLibraryLabel(label), budgetMb)
}

function modelMatchesFilter(model: OllamaLibraryModel, filter: string): boolean {
  if (filter === "all") return true
  return model.capabilities.some((cap) => cap.toLowerCase() === filter)
}

function candidateMatchesFilter(model: ModelCandidate, filter: string): boolean {
  if (filter === "all") return true
  return filter === "thinking" && model.reasoning
}

function isCompletionCapable(model: LocalOllamaModel): boolean {
  if (model.capabilities.length === 0) return true
  const caps = model.capabilities.map((cap) => cap.toLowerCase())
  const hasCompletion = caps.some((cap) =>
    ["completion", "chat", "tools", "thinking", "vision"].includes(cap),
  )
  return hasCompletion && !caps.every((cap) => cap === "embedding")
}

function isEmbeddingCapable(model: LocalOllamaModel): boolean {
  return model.capabilities.some((cap) => cap.toLowerCase() === "embedding")
}

export default function LocalModelsPanel() {
  const { t } = useTranslation()
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [models, setModels] = useState<LocalOllamaModel[]>([])
  const [recommendation, setRecommendation] = useState<ModelRecommendation | null>(null)
  const [loading, setLoading] = useState(false)
  const [actioning, setActioning] = useState<Record<string, boolean>>({})
  const [activeJobs, setActiveJobs] = useState<LocalModelJobSnapshot[]>([])

  const [query, setQuery] = useState("")
  const [capabilityFilter, setCapabilityFilter] = useState<(typeof CAPABILITY_FILTERS)[number]>("all")
  const [searching, setSearching] = useState(false)
  const [searchResult, setSearchResult] = useState<OllamaLibrarySearchResponse | null>(null)
  const [selectedDetail, setSelectedDetail] = useState<OllamaLibraryModelDetail | null>(null)
  const [manualTag, setManualTag] = useState("")

  const [pendingDelete, setPendingDelete] = useState<LocalOllamaModel | null>(null)
  const [pendingCancelJob, setPendingCancelJob] = useState<LocalModelJobSnapshot | null>(null)
  const [pendingEmbeddingDefault, setPendingEmbeddingDefault] = useState<LocalOllamaModel | null>(null)

  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogTitle, setDialogTitle] = useState("")
  const [dialogSubtitle, setDialogSubtitle] = useState<string | undefined>(undefined)
  const [dialogFrame, setDialogFrame] = useState<ProgressFrame | null>(null)
  const [dialogLogs, setDialogLogs] = useState<string[]>([])
  const [dialogDone, setDialogDone] = useState(false)
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [currentJob, setCurrentJob] = useState<LocalModelJobSnapshot | null>(null)
  const latestJobByIdRef = useRef<Map<string, LocalModelJobSnapshot>>(new Map())
  const hiddenCancelledJobIdsRef = useRef<Set<string>>(new Set())
  const clearAfterCancelJobIdsRef = useRef<Set<string>>(new Set())
  const handledTerminalJobs = useRef<Set<string>>(new Set())
  const refreshedTerminalJobs = useRef<Set<string>>(new Set())
  const jobTransferRef = useRef<
    Map<string, { bytes: number; timestamp: number; speedBps?: number; etaSeconds?: number }>
  >(new Map())
  const [jobTransferStats, setJobTransferStats] = useState<
    Record<string, { speedBps?: number; etaSeconds?: number }>
  >({})
  const localBudgetMb = recommendation?.hardware?.budgetMb ?? null
  const localBudgetLabel = localBudgetMb ? candidateSizeLabel(localBudgetMb) : "-"

  const renderLargeWarning = useCallback(
    (className?: string) => (
      <span
        className={cn(
          "inline-flex items-center gap-1 rounded border border-destructive/30 bg-destructive/10 px-1.5 py-0.5 text-[10px] font-medium text-destructive",
          className,
        )}
      >
        <AlertTriangle className="h-3 w-3 shrink-0" />
        {t("settings.localModels.largeModelWarning", { budget: localBudgetLabel })}
      </span>
    ),
    [localBudgetLabel, t],
  )

  const filteredSearchModels = useMemo(
    () => (searchResult?.models ?? []).filter((model) => modelMatchesFilter(model, capabilityFilter)),
    [capabilityFilter, searchResult],
  )

  const recommendedModels = useMemo(() => {
    const byId = new Map<string, ModelCandidate>()
    if (recommendation?.recommended) {
      byId.set(recommendation.recommended.id, recommendation.recommended)
    }
    for (const candidate of recommendation?.alternatives ?? []) {
      byId.set(candidate.id, candidate)
    }
    return [...byId.values()].filter((model) => candidateMatchesFilter(model, capabilityFilter))
  }, [capabilityFilter, recommendation])

  const sortedActiveJobs = useMemo(
    () => [...activeJobs].sort((a, b) => b.createdAt - a.createdAt || a.jobId.localeCompare(b.jobId)),
    [activeJobs],
  )

  const phaseLabel = useCallback(
    (phase: string | undefined) => {
      const key = phaseTranslationKey(phase)
      return key ? t(key) : (phase ?? "")
    },
    [t],
  )

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
    setLoading(true)
    try {
      const [status, localModels, rec] = await Promise.all([
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
        getTransport().call<LocalOllamaModel[]>("local_llm_list_models"),
        getTransport().call<ModelRecommendation>("local_llm_recommend_model"),
      ])
      setOllama(status)
      setModels(localModels)
      setRecommendation(rec)
    } catch (e) {
      logger.error("local-llm", "LocalModelsPanel::refresh", "Failed to refresh", e)
      toast.error(t("settings.localModels.errors.refreshFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const upsertActiveJob = useCallback((job: LocalModelJobSnapshot) => {
    latestJobByIdRef.current.set(job.jobId, job)
    setActiveJobs((prev) => {
      if (hiddenCancelledJobIdsRef.current.has(job.jobId)) {
        return prev.filter((item) => item.jobId !== job.jobId)
      }
      if (!isLocalModelJobVisible(job)) {
        return prev.filter((item) => item.jobId !== job.jobId)
      }
      const idx = prev.findIndex((item) => item.jobId === job.jobId)
      if (idx === -1) return [job, ...prev]
      const next = [...prev]
      next[idx] = job
      return next
    })
  }, [])

  const refreshActiveJobs = useCallback(async () => {
    try {
      const jobs = await getTransport().call<LocalModelJobSnapshot[]>("local_model_job_list")
      setActiveJobs(
        jobs.filter(
          (job) => isLocalModelJobVisible(job) && !hiddenCancelledJobIdsRef.current.has(job.jobId),
        ),
      )
    } catch (e) {
      logger.warn("local-llm", "LocalModelsPanel::refreshActiveJobs", "Failed to load jobs", e)
    }
  }, [])

  useEffect(() => {
    void refreshActiveJobs()
  }, [refreshActiveJobs])

  useEffect(() => {
    const now = Date.now()
    const activeIds = new Set(activeJobs.map((job) => job.jobId))
    const nextStats: Record<string, { speedBps?: number; etaSeconds?: number }> = {}

    for (const job of activeJobs) {
      const completed = job.bytesCompleted ?? null
      if (completed == null) continue

      const previous = jobTransferRef.current.get(job.jobId)
      if (!previous || completed < previous.bytes) {
        jobTransferRef.current.set(job.jobId, { bytes: completed, timestamp: now })
        continue
      }

      if (completed > previous.bytes && now > previous.timestamp) {
        const elapsedSeconds = (now - previous.timestamp) / 1000
        const speedBps = (completed - previous.bytes) / elapsedSeconds
        const etaSeconds =
          job.bytesTotal != null && job.bytesTotal > completed && speedBps > 0
            ? (job.bytesTotal - completed) / speedBps
            : undefined
        jobTransferRef.current.set(job.jobId, {
          bytes: completed,
          timestamp: now,
          speedBps,
          etaSeconds,
        })
      }

      const current = jobTransferRef.current.get(job.jobId)
      if (current?.speedBps) {
        nextStats[job.jobId] = {
          speedBps: current.speedBps,
          etaSeconds: current.etaSeconds,
        }
      }
    }

    for (const jobId of Array.from(jobTransferRef.current.keys())) {
      if (!activeIds.has(jobId)) jobTransferRef.current.delete(jobId)
    }
    setJobTransferStats((prev) => {
      const prevKeys = Object.keys(prev)
      const nextKeys = Object.keys(nextStats)
      if (prevKeys.length === nextKeys.length) {
        const unchanged = nextKeys.every((k) => {
          const a = prev[k]
          const b = nextStats[k]
          return a && b && a.speedBps === b.speedBps && a.etaSeconds === b.etaSeconds
        })
        if (unchanged) return prev
      }
      return nextStats
    })
  }, [activeJobs])

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
      logger.warn("local-llm", "LocalModelsPanel::hydrateJobLogs", "Failed to load logs", e)
    }
  }, [])

  const removeVisibleJob = useCallback((jobId: string) => {
    latestJobByIdRef.current.delete(jobId)
    setActiveJobs((prev) => prev.filter((item) => item.jobId !== jobId))
  }, [])

  const clearJobRecord = useCallback(
    async (jobId: string) => {
      await getTransport().call("local_model_job_clear", { jobId })
      hiddenCancelledJobIdsRef.current.delete(jobId)
      clearAfterCancelJobIdsRef.current.delete(jobId)
      removeVisibleJob(jobId)
    },
    [removeVisibleJob],
  )

  const handleTerminalJob = useCallback(
    (job: LocalModelJobSnapshot) => {
      if (!isLocalModelJobTerminal(job)) return
      if (handledTerminalJobs.current.has(job.jobId)) return
      handledTerminalJobs.current.add(job.jobId)
      if (job.status === "completed") {
        appendDialogLog(t("settings.localLlm.phases.done"), job.updatedAt)
      } else if (job.error) {
        appendDialogLog(job.error, job.updatedAt)
      }
    },
    [appendDialogLog, t],
  )

  const refreshAfterTerminalJob = useCallback(
    (job: LocalModelJobSnapshot) => {
      if (!isLocalModelJobTerminal(job)) return
      if (refreshedTerminalJobs.current.has(job.jobId)) return
      refreshedTerminalJobs.current.add(job.jobId)
      void refresh()
    },
    [refresh],
  )

  const openJobDialog = useCallback(
    (job: LocalModelJobSnapshot) => {
      const latest = latestJobByIdRef.current.get(job.jobId) ?? job
      setCurrentJob(latest)
      setDialogOpen(true)
      setDialogTitle(t("settings.localModels.jobs.title", { model: latest.displayName }))
      setDialogSubtitle(latest.modelId)
      setDialogFrame(localModelJobToProgressFrame(latest, phaseLabel))
      setDialogLogs([])
      setDialogDone(isLocalModelJobTerminal(latest) && !latest.error)
      setDialogError(latest.error ?? null)
      if (isLocalModelJobTerminal(latest)) {
        handleTerminalJob(latest)
        refreshAfterTerminalJob(latest)
      }
      void hydrateJobLogs(latest.jobId)
    },
    [handleTerminalJob, hydrateJobLogs, phaseLabel, refreshAfterTerminalJob, t],
  )

  useEffect(() => {
    const handleSnapshot = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      if (clearAfterCancelJobIdsRef.current.has(job.jobId) && isLocalModelJobTerminal(job)) {
        void clearJobRecord(job.jobId).catch((e) => {
          logger.warn("local-llm", "LocalModelsPanel::clearCancelledJob", "Failed to clear cancelled job", {
            jobId: job.jobId,
            error: String(e),
          })
        })
        return
      }
      upsertActiveJob(job)
      refreshAfterTerminalJob(job)
      setCurrentJob((current) => {
        if (current?.jobId === job.jobId) {
          setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
          setDialogDone(isLocalModelJobTerminal(job) && !job.error)
          setDialogError(job.error ?? null)
          handleTerminalJob(job)
          return job
        }
        if (isJobSuccessorOf(job, current)) {
          setDialogFrame(localModelJobToProgressFrame(job, phaseLabel))
          setDialogLogs([])
          setDialogDone(isLocalModelJobTerminal(job) && !job.error)
          setDialogError(job.error ?? null)
          setDialogTitle(t("settings.embedding.reembedJob.title"))
          setDialogSubtitle(job.modelId)
          void hydrateJobLogs(job.jobId)
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
  }, [
    appendDialogLog,
    clearJobRecord,
    handleTerminalJob,
    hydrateJobLogs,
    phaseLabel,
    refreshAfterTerminalJob,
    t,
    upsertActiveJob,
  ])

  const runModelAction = useCallback(
    async (modelId: string, action: () => Promise<unknown>, successKey: string) => {
      setActioning((prev) => ({ ...prev, [modelId]: true }))
      try {
        await action()
        toast.success(t(successKey, { model: modelId }))
        await refresh()
      } catch (e) {
        const message = String(e)
        logger.error("local-llm", "LocalModelsPanel::runModelAction", "Local model action failed", {
          modelId,
          successKey,
          error: message,
        })
        toast.error(message)
      } finally {
        setActioning((prev) => ({ ...prev, [modelId]: false }))
      }
    },
    [refresh, t],
  )

  const startOllama = useCallback(async () => {
    await runModelAction(
      "__ollama__",
      () => getTransport().call("local_llm_start_ollama"),
      "settings.localModels.toast.ollamaStarted",
    )
  }, [runModelAction])

  const openOllamaDownloadPage = useCallback(() => {
    openExternalUrl("https://ollama.com/download")
  }, [])

  const installOrDownloadOllama = useCallback(async () => {
    if (!ollama) return
    if (!ollama.installScriptSupported) {
      openOllamaDownloadPage()
      return
    }

    setActioning((prev) => ({ ...prev, __ollama_install__: true }))
    try {
      const job = await getTransport().call<LocalModelJobSnapshot>(
        "local_model_job_start_ollama_install",
      )
      upsertActiveJob(job)
    } catch (e) {
      const message = String(e)
      logger.error(
        "local-llm",
        "LocalModelsPanel::installOrDownloadOllama",
        "Failed to start Ollama install job",
        { error: message },
      )
      toast.error(message)
    } finally {
      setActioning((prev) => ({ ...prev, __ollama_install__: false }))
    }
  }, [ollama, openOllamaDownloadPage, upsertActiveJob])

  const loadLibraryModel = useCallback(
    async (model: string, silent = false) => {
      if (!silent) setSearching(true)
      try {
        const detail = await getTransport().call<OllamaLibraryModelDetail>(
          "local_llm_get_library_model",
          { model },
        )
        setSelectedDetail(detail)
      } catch (e) {
        logger.error("local-llm", "LocalModelsPanel::loadLibraryModel", "Load detail failed", e)
        if (!silent) {
          toast.error(t("settings.localModels.errors.detailFailed"))
        }
      } finally {
        if (!silent) setSearching(false)
      }
    },
    [t],
  )

  const toggleLibraryModel = useCallback(
    async (model: string) => {
      if (selectedDetail?.model.name === model) {
        setSelectedDetail(null)
        return
      }
      await loadLibraryModel(model)
    },
    [loadLibraryModel, selectedDetail?.model.name],
  )

  const loadLibrary = useCallback(async (libraryQuery: string, silent = false) => {
    setSearching(true)
    try {
      const result = await getTransport().call<OllamaLibrarySearchResponse>(
        "local_llm_search_library",
        { query: libraryQuery },
      )
      setSearchResult(result)
      const firstModel = result.models[0]?.name
      if (firstModel) {
        await loadLibraryModel(firstModel, true)
      } else {
        setSelectedDetail(null)
      }
    } catch (e) {
      logger.error("local-llm", "LocalModelsPanel::searchLibrary", "Search failed", e)
      if (!silent) {
        toast.error(t("settings.localModels.errors.searchFailed"))
      }
    } finally {
      setSearching(false)
    }
  }, [loadLibraryModel, t])

  const searchLibrary = useCallback(
    async () => {
      await loadLibrary(query)
    },
    [loadLibrary, query],
  )

  useEffect(() => {
    void loadLibrary("", true)
  }, [loadLibrary])

  const refreshAll = useCallback(async () => {
    await Promise.all([refresh(), refreshActiveJobs(), loadLibrary(query, true)])
  }, [loadLibrary, query, refresh, refreshActiveJobs])

  const startPullJob = useCallback(
    async (request: OllamaPullRequest) => {
      try {
        const job = await getTransport().call<LocalModelJobSnapshot>(
          "local_model_job_start_ollama_pull",
          { request },
        )
        upsertActiveJob(job)
      } catch (e) {
        const message = String(e)
        logger.error("local-llm", "LocalModelsPanel::startPullJob", "Failed to start pull job", {
          request,
          error: message,
        })
        toast.error(message)
      }
    },
    [upsertActiveJob],
  )

  const startPreloadJob = useCallback(
    async (model: LocalOllamaModel) => {
      setActioning((prev) => ({ ...prev, [model.id]: true }))
      try {
        const job = await getTransport().call<LocalModelJobSnapshot>(
          "local_model_job_start_ollama_preload",
          { modelId: model.id, displayName: model.name || model.id },
        )
        upsertActiveJob(job)
        openJobDialog(job)
      } catch (e) {
        const message = String(e)
        logger.error("local-llm", "LocalModelsPanel::startPreloadJob", "Failed to start preload job", {
          modelId: model.id,
          error: message,
        })
        toast.error(message)
      } finally {
        setActioning((prev) => ({ ...prev, [model.id]: false }))
      }
    },
    [openJobDialog, upsertActiveJob],
  )

  const cancelCurrentJob = useCallback(() => {
    const job = currentJob
    if (!job) return
    void getTransport()
      .call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId: job.jobId })
      .then(upsertActiveJob)
      .catch((e) => {
        const message = String(e)
        logger.error("local-llm", "LocalModelsPanel::cancelCurrentJob", "Failed to cancel job", {
          jobId: job.jobId,
          error: message,
        })
        setDialogError(message)
      })
  }, [currentJob, upsertActiveJob])

  const runJobAction = useCallback(
    async (job: LocalModelJobSnapshot, action: "pause" | "resume") => {
      const actionKey = `${job.jobId}:${action}`
      setActioning((prev) => ({ ...prev, [actionKey]: true }))
      try {
        // "resume" maps to retry: Ollama's chunked layer cache lets the next
        // pull pick up where the cancelled one stopped, so a fresh job is OK.
        const command = JOB_ACTION_COMMANDS[action]
        const nextJob = await getTransport().call<LocalModelJobSnapshot>(command, {
          jobId: job.jobId,
        })
        if (action === "resume") {
          latestJobByIdRef.current.delete(job.jobId)
          setActiveJobs((prev) => prev.filter((item) => item.jobId !== job.jobId))
        }
        upsertActiveJob(nextJob)
      } catch (e) {
        const message = String(e)
        logger.error("local-llm", "LocalModelsPanel::runJobAction", "Local model job action failed", {
          jobId: job.jobId,
          action,
          error: message,
        })
        toast.error(message)
      } finally {
        setActioning((prev) => ({ ...prev, [actionKey]: false }))
      }
    },
    [upsertActiveJob],
  )

  const confirmCancelJob = useCallback(async () => {
    const job = pendingCancelJob
    if (!job) return
    const actionKey = `${job.jobId}:cancel`
    setActioning((prev) => ({ ...prev, [actionKey]: true }))
    try {
      if (isLocalModelJobActive(job)) {
        hiddenCancelledJobIdsRef.current.add(job.jobId)
        clearAfterCancelJobIdsRef.current.add(job.jobId)
        await getTransport().call<LocalModelJobSnapshot>("local_model_job_cancel", {
          jobId: job.jobId,
        })
        removeVisibleJob(job.jobId)
      } else {
        await clearJobRecord(job.jobId)
      }
      setPendingCancelJob(null)
    } catch (e) {
      const message = String(e)
      logger.error("local-llm", "LocalModelsPanel::confirmCancelJob", "Local model job cancel failed", {
        jobId: job.jobId,
        error: message,
      })
      toast.error(message)
    } finally {
      setActioning((prev) => ({ ...prev, [actionKey]: false }))
    }
  }, [clearJobRecord, pendingCancelJob, removeVisibleJob])

  const confirmDelete = useCallback(async () => {
    const model = pendingDelete
    if (!model) return
    await runModelAction(
      model.id,
      () => getTransport().call("local_llm_delete_model", { modelId: model.id }),
      "settings.localModels.toast.deleted",
    )
    setPendingDelete(null)
  }, [pendingDelete, runModelAction])

  const confirmEmbeddingDefault = useCallback(async () => {
    const model = pendingEmbeddingDefault
    const configId = model?.usage.embeddingConfigId
    if (!model || !configId) return
    await runModelAction(
      model.id,
      () => getTransport().call("memory_embedding_set_default", { modelConfigId: configId, mode: "keep_existing" }),
      "settings.localModels.toast.embeddingSet",
    )
    setPendingEmbeddingDefault(null)
  }, [pendingEmbeddingDefault, runModelAction])

  const deleteWarnings = pendingDelete
    ? [
        pendingDelete.usage.running && t("settings.localModels.delete.running"),
        pendingDelete.usage.activeModel && t("settings.localModels.delete.active"),
        pendingDelete.usage.fallbackModel && t("settings.localModels.delete.fallback"),
        pendingDelete.usage.providerModel && t("settings.localModels.delete.provider"),
        pendingDelete.usage.embeddingConfig && t("settings.localModels.delete.embeddingConfig"),
        pendingDelete.usage.embeddingModel && t("settings.localModels.delete.embedding"),
      ].filter(Boolean)
    : []

  const installedEmpty = !loading && models.length === 0
  const activeJobCount = sortedActiveJobs.filter(isLocalModelJobActive).length
  const pausedJobCount = sortedActiveJobs.filter((job) => job.status === "paused").length
  const actionableJobCount = sortedActiveJobs.filter(isLocalModelJobResumable).length
  const jobSummaryText =
    activeJobCount > 0
      ? t("localModelJobs.activeSummary", { count: activeJobCount })
      : actionableJobCount > 0 && pausedJobCount !== actionableJobCount
        ? t("localModelJobs.pendingSummary", { count: actionableJobCount })
        : t("localModelJobs.pausedSummary", { count: pausedJobCount })
  const jobTransferSummary = useCallback(
    (job: LocalModelJobSnapshot) => {
      const stats = jobTransferStats[job.jobId]
      return (
        formatJobTransferLine({
          unit: job.kind === "memory_reembed" ? "count" : "bytes",
          completed: job.bytesCompleted,
          total: job.bytesTotal,
          speedBps: stats?.speedBps ?? null,
          etaSeconds: stats?.etaSeconds ?? null,
          t,
        }) ?? ""
      )
    },
    [jobTransferStats, t],
  )

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mb-5 flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h2 className="text-lg font-semibold text-foreground">{t("settings.localModels.title")}</h2>
          <p className="mt-1 text-xs text-muted-foreground">{t("settings.localModels.subtitle")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {ollama?.phase === "not-installed" ? (
            <Button
              variant="outline"
              size="sm"
              className={cn(
                ACTION_BUTTON_CLASS,
                "border-amber-500/30 bg-amber-500/10 text-amber-700 hover:bg-amber-500/15 hover:text-amber-800 dark:text-amber-300 dark:hover:text-amber-200",
              )}
              onClick={() => void installOrDownloadOllama()}
              disabled={actioning.__ollama_install__}
            >
              {actioning.__ollama_install__ ? (
                <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
              ) : (
                <Ollama size={14} className="mr-2 h-3.5 w-3.5 shrink-0" />
              )}
              {ollama.installScriptSupported
                ? t("settings.localModels.actions.installOllama")
                : t("settings.localLlm.buttons.downloadOllama")}
            </Button>
          ) : (
            <span
              className={cn(
                ACTION_BUTTON_CLASS,
                "inline-flex h-8 items-center justify-center gap-1.5 rounded-md border px-3 text-xs font-medium",
                "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
              )}
            >
              <Ollama size={14} className="h-3.5 w-3.5" />
              {ollama ? t(`settings.localModels.ollama.${ollama.phase}`) : t("settings.localLlm.detecting")}
            </span>
          )}
          {ollama?.phase === "installed" && (
            <Button
              size="sm"
              className={ACTION_BUTTON_CLASS}
              onClick={() => void startOllama()}
              disabled={actioning.__ollama__}
            >
              {actioning.__ollama__ ? (
                <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
              ) : (
                <Play className={ACTION_ICON_CLASS} />
              )}
              {t("settings.localLlm.buttons.startOllama")}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            className={ACTION_BUTTON_CLASS}
            onClick={() => void refreshAll()}
            disabled={loading}
          >
            <RefreshCw className={cn(ACTION_ICON_CLASS, loading && "animate-spin")} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {sortedActiveJobs.length > 0 && (
        <div className="mb-4 rounded-lg border border-primary/20 bg-primary/5 p-4">
          <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex min-w-0 items-center gap-2 text-sm font-medium text-foreground">
              {activeJobCount > 0 ? (
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary" />
              ) : actionableJobCount > 0 && pausedJobCount !== actionableJobCount ? (
                <AlertTriangle className="h-4 w-4 shrink-0 text-primary" />
              ) : (
                <Pause className="h-4 w-4 shrink-0 text-primary" />
              )}
              <span className="truncate">{jobSummaryText}</span>
            </div>
            <span className="shrink-0 text-xs text-muted-foreground">{t("localModelJobs.subtitle")}</span>
          </div>
          <div className="grid items-stretch gap-3 lg:grid-cols-2">
            {sortedActiveJobs.map((job) => {
              const percent = localModelJobPercent(job)
              const transferSummary = jobTransferSummary(job)
              const active = isLocalModelJobActive(job)
              const resumable = isLocalModelJobResumable(job)
              const statusText = active
                ? phaseLabel(job.phase)
                : t(`localModelJobs.status.${job.status}`)
              return (
                <div
                  key={job.jobId}
                  className="flex h-full min-h-[156px] cursor-pointer flex-col rounded-md border border-border/70 bg-card p-4 text-left transition-colors hover:bg-secondary/60"
                  onClick={() => openJobDialog(job)}
                >
                  <div className="flex min-w-0 items-start justify-between gap-4">
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm font-medium text-foreground">
                        {job.displayName}
                      </div>
                      <div className="mt-2 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                        <span className="min-w-0 truncate font-mono">{job.modelId}</span>
                        <span>·</span>
                        <span className="shrink-0">{t(`localModelJobs.kind.${job.kind}`)}</span>
                        <span>·</span>
                        <span className="shrink-0">{statusText}</span>
                      </div>
                    </div>
                    {percent != null && (
                      <span className="w-10 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                        {Math.round(percent)}%
                      </span>
                    )}
                  </div>
                  <div className="mt-4 space-y-2">
                    <Progress
                      value={percent}
                      indeterminate={active && percent == null}
                      className="h-1.5"
                    />
                    <div className="min-h-4 truncate text-xs tabular-nums text-muted-foreground">
                      {transferSummary}
                    </div>
                  </div>
                  <div
                    className="mt-auto flex flex-wrap justify-end gap-2 pt-4"
                    onClick={(event) => event.stopPropagation()}
                  >
                    {active && (
                      <Button
                        variant="outline"
                        size="sm"
                        className={ACTION_BUTTON_CLASS}
                        disabled={actioning[`${job.jobId}:pause`] || actioning[`${job.jobId}:cancel`]}
                        onClick={() => void runJobAction(job, "pause")}
                      >
                        {actioning[`${job.jobId}:pause`] ? (
                          <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                        ) : (
                          <Pause className={ACTION_ICON_CLASS} />
                        )}
                        {t("localModelJobs.actions.pause")}
                      </Button>
                    )}
                    {resumable && (
                      <Button
                        variant="outline"
                        size="sm"
                        className={ACTION_BUTTON_CLASS}
                        disabled={actioning[`${job.jobId}:resume`] || actioning[`${job.jobId}:cancel`]}
                        onClick={() => void runJobAction(job, "resume")}
                      >
                        {actioning[`${job.jobId}:resume`] ? (
                          <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                        ) : (
                          <RotateCcw className={ACTION_ICON_CLASS} />
                        )}
                        {t("localModelJobs.actions.resume")}
                      </Button>
                    )}
                    {active && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className={cn(ACTION_BUTTON_CLASS, "text-destructive hover:text-destructive")}
                        disabled={actioning[`${job.jobId}:pause`] || actioning[`${job.jobId}:resume`] || actioning[`${job.jobId}:cancel`]}
                        onClick={() => setPendingCancelJob(job)}
                      >
                        {actioning[`${job.jobId}:cancel`] ? (
                          <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                        ) : (
                          <Square className={ACTION_ICON_CLASS} />
                        )}
                        {t("localModelJobs.actions.cancel")}
                      </Button>
                    )}
                    {resumable && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className={cn(ACTION_BUTTON_CLASS, "text-destructive hover:text-destructive")}
                        disabled={actioning[`${job.jobId}:resume`] || actioning[`${job.jobId}:cancel`]}
                        onClick={() => setPendingCancelJob(job)}
                      >
                        {actioning[`${job.jobId}:cancel`] ? (
                          <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                        ) : (
                          <Square className={ACTION_ICON_CLASS} />
                        )}
                        {t("localModelJobs.actions.cancel")}
                      </Button>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}

      <Tabs defaultValue="installed" className="space-y-4">
        <TabsList>
          <TabsTrigger value="installed">{t("settings.localModels.installedTab")}</TabsTrigger>
          <TabsTrigger value="library">{t("settings.localModels.libraryTab")}</TabsTrigger>
        </TabsList>

        <TabsContent value="installed" className="space-y-3">
          {loading ? (
            <div className="flex items-center justify-center py-12 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : installedEmpty ? (
            <div className="rounded-lg border border-dashed border-border p-8 text-center">
              <Database className="mx-auto mb-3 h-8 w-8 text-muted-foreground/50" />
              <div className="text-sm font-medium">{t("settings.localModels.emptyInstalled")}</div>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.localModels.emptyInstalledDesc")}
              </p>
            </div>
          ) : (
            models.map((model) => {
              const completionCapable = isCompletionCapable(model)
              const embeddingCapable = isEmbeddingCapable(model)
              const preloading = sortedActiveJobs.some(
                (job) => job.kind === "ollama_preload" && job.modelId === model.id && isLocalModelJobActive(job),
              )
              const rowBusy = actioning[model.id] || preloading
              const largeModel = exceedsBudgetBytes(model.sizeBytes, localBudgetMb)
              return (
                <div key={model.id} className="rounded-lg border border-border bg-card p-4">
                  <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-mono text-sm font-semibold text-foreground">{model.id}</span>
                        {completionCapable && (
                          <span className="rounded border border-blue-500/25 bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-700 dark:text-blue-300">
                            {t("settings.localModels.badges.llm")}
                          </span>
                        )}
                        {embeddingCapable && (
                          <span className="rounded border border-purple-500/25 bg-purple-500/10 px-1.5 py-0.5 text-[10px] text-purple-700 dark:text-purple-300">
                            {t("settings.localModels.badges.embedding")}
                          </span>
                        )}
                        {model.running && (
                          <span className="rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-700 dark:text-emerald-300">
                            {t("settings.localModels.badges.running")}
                          </span>
                        )}
                        {model.usage.providerModel && (
                          <span className="rounded border border-sky-500/25 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-700 dark:text-sky-300">
                            {t("settings.localModels.badges.provider")}
                          </span>
                        )}
                        {model.usage.embeddingConfig && !model.usage.embeddingModel && (
                          <span className="rounded border border-cyan-500/25 bg-cyan-500/10 px-1.5 py-0.5 text-[10px] text-cyan-700 dark:text-cyan-300">
                            {t("settings.localModels.badges.embeddingConfig")}
                          </span>
                        )}
                        {model.usage.activeModel && (
                          <span className="rounded border border-primary/25 bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
                            {t("settings.localModels.badges.default")}
                          </span>
                        )}
                        {model.usage.embeddingModel && (
                          <span className="rounded border border-fuchsia-500/25 bg-fuchsia-500/10 px-1.5 py-0.5 text-[10px] text-fuchsia-700 dark:text-fuchsia-300">
                            {t("settings.localModels.badges.memory")}
                          </span>
                        )}
                        {largeModel && renderLargeWarning()}
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                        <span className={cn(largeModel && "font-medium text-destructive")}>
                          {sizeLabel(model.sizeBytes)}
                        </span>
                        <span>·</span>
                        <span>{contextLabel(model.contextWindow)}</span>
                        {model.details?.family && (
                          <>
                            <span>·</span>
                            <span>{model.details.family}</span>
                          </>
                        )}
                        {model.details?.quantizationLevel && (
                          <>
                            <span>·</span>
                            <span>{model.details.quantizationLevel}</span>
                          </>
                        )}
                        {model.capabilities.length > 0 && (
                          <>
                            <span>·</span>
                            <span>{model.capabilities.join(", ")}</span>
                          </>
                        )}
                      </div>
                    </div>
                    <div className="flex shrink-0 flex-wrap items-center gap-2">
                      {model.running ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() =>
                            void runModelAction(
                              model.id,
                              () => getTransport().call("local_llm_stop_model", { modelId: model.id }),
                              "settings.localModels.toast.stopped",
                            )
                          }
                        >
                          <Square className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.stop")}
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() => void startPreloadJob(model)}
                        >
                          {rowBusy ? (
                            <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                          ) : (
                            <Play className={ACTION_ICON_CLASS} />
                          )}
                          {t("settings.localModels.actions.preload")}
                        </Button>
                      )}
                      {completionCapable && !model.usage.providerModel && (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() =>
                            void runModelAction(
                              model.id,
                              () => getTransport().call("local_llm_add_provider_model", { modelId: model.id }),
                              "settings.localModels.toast.providerAdded",
                            )
                          }
                        >
                          <Database className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.addProvider")}
                        </Button>
                      )}
                      {completionCapable && model.usage.providerModel && !model.usage.activeModel && (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() =>
                            void runModelAction(
                              model.id,
                              () => getTransport().call("local_llm_set_default_model", { modelId: model.id }),
                              "settings.localModels.toast.defaultSet",
                            )
                          }
                        >
                          <Star className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.setDefault")}
                        </Button>
                      )}
                      {embeddingCapable && !model.usage.embeddingConfig && (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() =>
                            void runModelAction(
                              model.id,
                              () => getTransport().call("local_llm_add_embedding_config", { modelId: model.id }),
                              "settings.localModels.toast.embeddingConfigAdded",
                            )
                          }
                        >
                          <Brain className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.addEmbeddingConfig")}
                        </Button>
                      )}
                      {embeddingCapable && model.usage.embeddingConfig && !model.usage.embeddingModel && (
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          disabled={rowBusy}
                          onClick={() => setPendingEmbeddingDefault(model)}
                        >
                          <Brain className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.setMemoryDefault")}
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={rowBusy}
                        className={cn(ACTION_BUTTON_CLASS, "text-destructive hover:text-destructive")}
                        onClick={() => setPendingDelete(model)}
                      >
                        <Trash2 className={ACTION_ICON_CLASS} />
                        {t("common.delete")}
                      </Button>
                    </div>
                  </div>
                </div>
              )
            })
          )}
        </TabsContent>

        <TabsContent value="library" className="space-y-4">
          <div className="rounded-lg border border-border bg-card p-4">
            <div className="flex flex-col gap-3 lg:flex-row">
              <div className="relative flex-1">
                <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void searchLibrary()
                  }}
                  placeholder={t("settings.localModels.searchPlaceholder")}
                  className="pl-9"
                />
              </div>
              <Button
                className={ACTION_BUTTON_CLASS}
                onClick={() => void searchLibrary()}
                disabled={searching}
              >
                {searching ? (
                  <Loader2 className={cn(ACTION_ICON_CLASS, "animate-spin")} />
                ) : (
                  <Search className={ACTION_ICON_CLASS} />
                )}
                {t("common.search")}
              </Button>
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              {CAPABILITY_FILTERS.map((filter) => (
                <Button
                  key={filter}
                  variant={capabilityFilter === filter ? "default" : "outline"}
                  size="sm"
                  className="h-7 text-xs"
                  onClick={() => setCapabilityFilter(filter)}
                >
                  {t(`settings.localModels.filters.${filter}`)}
                </Button>
              ))}
            </div>
            {searchResult?.stale && (
              <p className="mt-2 text-xs text-amber-600 dark:text-amber-400">
                {t("settings.localModels.cacheStale")}
              </p>
            )}
          </div>

          <div className="space-y-4">
            <div className="space-y-2">
              {recommendedModels.map((model) => {
                const isRecommended = recommendation?.recommended?.id === model.id
                const largeModel = exceedsBudgetMb(model.sizeMb, localBudgetMb)
                return (
                  <div key={model.id} className="rounded-lg border border-border bg-card p-3">
                    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-semibold text-foreground">
                            {model.displayName}
                          </span>
                          {isRecommended && (
                            <span className="rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-emerald-700 dark:text-emerald-300">
                              {t("settings.localLlm.recommended")}
                            </span>
                          )}
                          {model.reasoning && (
                            <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                              {t("settings.localLlm.reasoning")}
                            </span>
                          )}
                          {largeModel && renderLargeWarning()}
                        </div>
                        <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                          <span className="font-mono">{model.id}</span>
                          <span className={cn(largeModel && "font-medium text-destructive")}>
                            {candidateSizeLabel(model.sizeMb)}
                          </span>
                          <span>
                            {t("settings.localLlm.contextWindow", {
                              n: model.contextWindow.toLocaleString(),
                            })}
                          </span>
                        </div>
                      </div>
                      <div className="flex min-w-fit shrink-0 flex-wrap justify-end gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          onClick={() =>
                            void startPullJob({
                              modelId: model.id,
                              displayName: model.displayName,
                            })
                          }
                        >
                          <Download className={ACTION_ICON_CLASS} />
                          {t("settings.localModels.actions.download")}
                        </Button>
                      </div>
                    </div>
                  </div>
                )
              })}
              {filteredSearchModels.length === 0 ? (
                <div className="rounded-lg border border-dashed border-border p-8 text-center text-sm text-muted-foreground">
                  {searching && !searchResult ? (
                    <Loader2 className="mx-auto h-5 w-5 animate-spin" />
                  ) : searchResult ? (
                    t("settings.localModels.noSearchResults")
                  ) : (
                    t("settings.localModels.searchHint")
                  )}
                </div>
              ) : (
                filteredSearchModels.map((model) => {
                  const shownSizes = model.sizes.slice(0, 6)
                  const hasLargeSize = shownSizes.some((size) =>
                    librarySizeExceedsBudget(size, localBudgetMb),
                  )
                  const expanded = selectedDetail?.model.name === model.name
                  return (
                    <div
                      key={model.name}
                      className={cn(
                        "rounded-lg border bg-card p-3",
                        expanded ? "border-primary/50 bg-primary/5" : "border-border",
                      )}
                    >
                      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                        <div className="min-w-0">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="text-sm font-semibold text-foreground">{model.name}</span>
                            {model.capabilities.map((cap) => (
                              <span key={cap} className="rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                {cap}
                              </span>
                            ))}
                            {hasLargeSize && renderLargeWarning()}
                          </div>
                          <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">{model.description}</p>
                          <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                            {shownSizes.map((size) => {
                              const largeSize = librarySizeExceedsBudget(size, localBudgetMb)
                              return (
                                <span
                                  key={size}
                                  className={cn(
                                    largeSize &&
                                      "rounded border border-destructive/30 bg-destructive/10 px-1 text-destructive",
                                  )}
                                >
                                  {size}
                                </span>
                              )
                            })}
                            {model.pullCount && <span>{t("settings.localModels.downloads", { count: model.pullCount })}</span>}
                            {model.tagCount != null && <span>{t("settings.localModels.tags", { count: model.tagCount })}</span>}
                          </div>
                        </div>
                        <Button
                          variant={expanded ? "default" : "outline"}
                          size="sm"
                          className={ACTION_BUTTON_CLASS}
                          onClick={() => void toggleLibraryModel(model.name)}
                        >
                          {expanded
                            ? t("settings.localModels.actions.hideTags")
                            : t("settings.localModels.actions.viewTags")}
                        </Button>
                      </div>
                      {expanded && (
                        <div className="mt-3 border-t border-border/70 pt-3">
                          {selectedDetail.summary && (
                            <p className="mb-3 text-xs text-muted-foreground">{selectedDetail.summary}</p>
                          )}
                          <div className="space-y-2">
                            {selectedDetail.tags.map((tag) => {
                              const largeModel = exceedsBudgetBytes(tag.sizeBytes, localBudgetMb)
                              return (
                                <div
                                  key={tag.id}
                                  className={cn(
                                    "rounded-md border p-3",
                                    largeModel ? "border-destructive/40 bg-destructive/5" : "border-border/70",
                                  )}
                                >
                                  <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                                    <div className="min-w-0">
                                      <div className="flex flex-wrap items-center gap-2">
                                        <span className="font-mono text-xs font-medium text-foreground">{tag.id}</span>
                                        {largeModel && renderLargeWarning()}
                                      </div>
                                      <div className="mt-1 flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                                        <span className={cn(largeModel && "font-medium text-destructive")}>
                                          {tag.sizeLabel ?? "-"}
                                        </span>
                                        <span>{tag.contextLabel ?? "-"}</span>
                                        {tag.inputTypes.length > 0 && <span>{tag.inputTypes.join(", ")}</span>}
                                        {tag.cloudOnly && (
                                          <span className="text-amber-600 dark:text-amber-400">
                                            {t("settings.localModels.cloudOnly")}
                                          </span>
                                        )}
                                      </div>
                                    </div>
                                    <div className="flex min-w-fit shrink-0 flex-wrap justify-end gap-2">
                                      <Button
                                        variant="outline"
                                        size="sm"
                                        className={ACTION_BUTTON_CLASS}
                                        disabled={tag.cloudOnly}
                                        onClick={() =>
                                          void startPullJob({
                                            modelId: tag.id,
                                            displayName: tag.id,
                                          })
                                        }
                                      >
                                        <Download className={ACTION_ICON_CLASS} />
                                        {t("settings.localModels.actions.download")}
                                      </Button>
                                    </div>
                                  </div>
                                </div>
                              )
                            })}
                          </div>
                        </div>
                      )}
                    </div>
                  )
                })
              )}
            </div>
          </div>

          <div className="rounded-lg border border-border bg-card p-4">
            <div className="mb-2 text-sm font-medium">{t("settings.localModels.manualTitle")}</div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                value={manualTag}
                onChange={(e) => setManualTag(e.target.value)}
                placeholder="qwen3.6:27b"
              />
              <Button
                variant="outline"
                className={ACTION_BUTTON_CLASS}
                disabled={!manualTag.trim()}
                onClick={() =>
                  void startPullJob({
                    modelId: manualTag.trim(),
                    displayName: manualTag.trim(),
                  })
                }
              >
                <Download className={ACTION_ICON_CLASS} />
                {t("settings.localModels.actions.download")}
              </Button>
            </div>
          </div>
        </TabsContent>
      </Tabs>

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
        backgroundLabel={t("localModelJobs.actions.backgroundTask")}
        cancelLabel={t("localModelJobs.actions.cancelTask")}
        closeTitle={t("localModelJobs.close.taskTitle")}
        closeDescription={t("localModelJobs.close.taskDescription")}
      />

      <AlertDialog
        open={!!pendingEmbeddingDefault}
        onOpenChange={(open) => !open && setPendingEmbeddingDefault(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.embeddingModels.confirmSwitchTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.embeddingModels.confirmSwitchDesc", {
                model: pendingEmbeddingDefault?.id ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void confirmEmbeddingDefault()}>
              {t("settings.embeddingModels.confirmSwitchAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!pendingCancelJob} onOpenChange={(open) => !open && setPendingCancelJob(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("localModelJobs.cancelConfirm.title")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("localModelJobs.cancelConfirm.description", {
                model: pendingCancelJob?.displayName ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void confirmCancelJob()}
            >
              {t("localModelJobs.cancelConfirm.action")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!pendingDelete} onOpenChange={(open) => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.localModels.delete.title")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.localModels.delete.description", { model: pendingDelete?.id ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {deleteWarnings.length > 0 && (
            <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-amber-700 dark:text-amber-300">
              {deleteWarnings.map((warning) => (
                <div key={String(warning)}>{warning}</div>
              ))}
            </div>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void confirmDelete()}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
