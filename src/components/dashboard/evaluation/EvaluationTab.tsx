import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock3,
  Download,
  FlaskConical,
  Play,
  RefreshCw,
  ShieldCheck,
  Square,
  Upload,
  XCircle,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NumberInput } from "@/components/ui/number-input"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

import type {
  EvalAppRunRequest,
  EvalAppPlan,
  EvalAnnotationRecord,
  EvalBaselineRecord,
  EvalCatalog,
  EvalCompareResult,
  EvalCompatibilityMetric,
  EvalExperimentDetail,
  EvalExperimentRecord,
  EvalImportResult,
  EvalLocalExportResult,
  EvalModelOption,
  EvalPreview,
  EvalTrialDetail,
  EvalTrialRecord,
  EvalTrendMetric,
  EvalTrendPoint,
} from "./types"

const ACTIVE_STATUSES = new Set(["queued", "planning", "running", "cancelling"])

interface EvaluationChangedEvent {
  experimentId?: string
  change?: string
  phase?: string
  campaignId?: string
  trialId?: string
  completed?: number
  total?: number
  outcome?: string
  wallMs?: number
  modelCalls?: number
  toolCalls?: number
  inputTokens?: number
  outputTokens?: number
  costUsd?: number
  loopIterations?: number
  spawnedAgents?: number
  asyncJobs?: number
  activeChildren?: number
  attribution?: string
  lastEvent?: string
  lastEventStatus?: string
  dimension?: string
  observed?: number
  limit?: number
  ratio?: number
}

interface LiveEvalProgress {
  phase?: string
  currentTrial?: string
  completed?: number
  total?: number
  tokens: number
  costUsd: number
  warning?: string
}

interface LiveTrialProgress {
  campaignId: string
  trialId: string
  status: "running" | "completed"
  outcome?: string
  durationMs?: number
  modelCalls?: number
  toolCalls?: number
  inputTokens?: number
  outputTokens?: number
  costUsd?: number
  loopIterations?: number
  spawnedAgents?: number
  asyncJobs?: number
  activeChildren?: number
  attribution?: string
  lastEvent?: string
  lastEventStatus?: string
}

type MonitorTrialStatus = "queued" | "running" | "completed" | "aborted" | "not_run"

interface MonitorTrialRow {
  campaignId: string
  trialId: string
  status: MonitorTrialStatus
  outcome?: string
  durationMs?: number
  modelCalls?: number
  toolCalls?: number
  inputTokens?: number
  outputTokens?: number
  costUsd?: number
  suiteId?: string
  caseId?: string
  arm?: string
  loopIterations?: number
  spawnedAgents?: number
  asyncJobs?: number
  activeChildren?: number
  attribution?: string
  lastEvent?: string
  lastEventStatus?: string
  persisted: boolean
}

export default function EvaluationTab() {
  const { t } = useTranslation()
  const [catalog, setCatalog] = useState<EvalCatalog | null>(null)
  const [history, setHistory] = useState<EvalExperimentRecord[]>([])
  const [detail, setDetail] = useState<EvalExperimentDetail | null>(null)
  const [annotations, setAnnotations] = useState<EvalAnnotationRecord[]>([])
  const [annotationText, setAnnotationText] = useState("")
  const [baselines, setBaselines] = useState<EvalBaselineRecord[]>([])
  const [compareBaselineId, setCompareBaselineId] = useState("")
  const [compareCandidateId, setCompareCandidateId] = useState("")
  const [comparison, setComparison] = useState<EvalCompareResult | null>(null)
  const [trendBaselineId, setTrendBaselineId] = useState("")
  const [trendMetric, setTrendMetric] = useState<EvalTrendMetric>("task_success")
  const [trends, setTrends] = useState<EvalTrendPoint[]>([])
  const [profileId, setProfileId] = useState("quick")
  const [selectedModels, setSelectedModels] = useState<string[]>([])
  const [selectedCredentials, setSelectedCredentials] = useState<Record<string, string>>({})
  const [selectedCases, setSelectedCases] = useState<string[]>([])
  const [selectedArms, setSelectedArms] = useState<string[]>(["control"])
  const [repetitions, setRepetitions] = useState(1)
  const [maxCost, setMaxCost] = useState(100)
  const [maxWallMinutes, setMaxWallMinutes] = useState(480)
  const [concurrency, setConcurrency] = useState(4)
  const [consentCosts, setConsentCosts] = useState(false)
  const [consentTools, setConsentTools] = useState(false)
  const [preview, setPreview] = useState<EvalPreview | null>(null)
  const [activeRunId, setActiveRunId] = useState<string | null>(null)
  const [focusedRunId, setFocusedRunId] = useState<string | null>(null)
  const [focusedRunPlan, setFocusedRunPlan] = useState<EvalAppPlan | null>(null)
  const [runDetail, setRunDetail] = useState<EvalExperimentDetail | null>(null)
  const [runDetailLoading, setRunDetailLoading] = useState(false)
  const [liveTrials, setLiveTrials] = useState<Record<string, LiveTrialProgress>>({})
  const [liveProgress, setLiveProgress] = useState<LiveEvalProgress>({ tokens: 0, costUsd: 0 })
  const [activeSection, setActiveSection] = useState("run")
  const [loading, setLoading] = useState(true)
  const [actionLoading, setActionLoading] = useState(false)
  const focusedRunIdRef = useRef<string | null>(null)

  const profile = useMemo(
    () => catalog?.profiles.find((candidate) => candidate.id === profileId) ?? null,
    [catalog, profileId],
  )
  const availableCases = useMemo(() => {
    if (!catalog || !profile) return []
    const selectedSuites = new Map(profile.suites.map((suite) => [suite.suiteId, suite.caseTags]))
    return catalog.suites.flatMap((suite) => {
      const tags = selectedSuites.get(suite.id)
      if (!tags) return []
      return suite.cases
        .filter((item) => tags.length === 0 || item.tags.some((tag) => tags.includes(tag)))
        .map((item) => ({ ...item, suiteId: suite.id }))
    })
  }, [catalog, profile])
  const selectedModelOptions = useMemo(
    () =>
      selectedModels
        .map((key) => catalog?.models.find((model) => modelKey(model) === key))
        .filter((model): model is EvalModelOption => Boolean(model)),
    [catalog, selectedModels],
  )
  const activeRun = history.find((item) => item.id === activeRunId) ?? null
  const focusedRun =
    runDetail?.experiment.id === focusedRunId
      ? runDetail.experiment
      : (history.find((item) => item.id === focusedRunId) ?? null)

  const applyHistorySnapshot = useCallback(
    (nextHistory: EvalExperimentRecord[], nextBaselines: EvalBaselineRecord[]) => {
      setHistory(nextHistory)
      setBaselines(nextBaselines)
      const active = nextHistory.find(
        (item) => item.kind === "hope_core" && ACTIVE_STATUSES.has(item.status),
      )
      setActiveRunId(active?.id ?? null)
      if (active && !focusedRunIdRef.current) {
        focusedRunIdRef.current = active.id
        setFocusedRunId(active.id)
        setActiveSection("run")
      }
      const comparable = nextHistory.filter(
        (item) => item.kind === "hope_core" && item.status === "completed",
      )
      setCompareBaselineId((current) => current || comparable[1]?.id || comparable[0]?.id || "")
      setCompareCandidateId((current) => current || comparable[0]?.id || "")
      setTrendBaselineId(
        (current) => current || nextBaselines[0]?.experimentId || comparable[0]?.id || "",
      )
    },
    [],
  )

  const refreshHistory = useCallback(async () => {
    const transport = getTransport()
    const [nextHistory, nextBaselines] = await Promise.all([
      transport.call<EvalExperimentRecord[]>("eval_list_history", {
        query: { limit: 100, offset: 0 },
      }),
      transport.call<EvalBaselineRecord[]>("eval_list_baselines", { tier: null }),
    ])
    applyHistorySnapshot(nextHistory, nextBaselines)
  }, [applyHistorySnapshot])

  const refreshRunDetail = useCallback(async (experimentId: string) => {
    const next = await getTransport().call<EvalExperimentDetail>("eval_get_experiment", {
      experimentId,
    })
    if (focusedRunIdRef.current === experimentId) setRunDetail(next)
  }, [])

  const load = useCallback(async () => {
    const transport = getTransport()
    const [nextCatalog, nextHistory, nextBaselines] = await Promise.all([
      transport.call<EvalCatalog>("eval_catalog"),
      transport.call<EvalExperimentRecord[]>("eval_list_history", {
        query: { limit: 100, offset: 0 },
      }),
      transport.call<EvalBaselineRecord[]>("eval_list_baselines", { tier: null }),
    ])
    setCatalog(nextCatalog)
    applyHistorySnapshot(nextHistory, nextBaselines)
    setProfileId((current) =>
      nextCatalog.profiles.some((item) => item.id === current)
        ? current
        : (nextCatalog.profiles[0]?.id ?? "quick"),
    )
  }, [applyHistorySnapshot])

  useEffect(() => {
    setLoading(true)
    load()
      .catch((error) => toast.error(String(error)))
      .finally(() => setLoading(false))
  }, [load])

  useEffect(() => {
    const unlisten = getTransport().listen("evaluation:changed", (raw) => {
      const event = parsePayload<EvaluationChangedEvent>(raw)
      if (!event?.experimentId) return
      if (event.experimentId === focusedRunIdRef.current) {
        if (event.change === "budget_warning") {
          const percent = Math.round((event.ratio ?? 0) * 100)
          toast.warning(t("dashboard.evaluation.budgetWarning", "评测预算接近上限"), {
            description: `${event.dimension ?? "budget"}: ${percent}% (${event.observed ?? 0}/${event.limit ?? 0})`,
          })
        }
        setLiveProgress((current) => {
          if (event.change === "trial_completed") {
            return {
              ...current,
              currentTrial: event.trialId,
              completed: event.completed,
              total: event.total,
              tokens: current.tokens + (event.inputTokens ?? 0) + (event.outputTokens ?? 0),
              costUsd: current.costUsd + (event.costUsd ?? 0),
            }
          }
          if (event.change === "trial_started") {
            return {
              ...current,
              currentTrial: event.trialId,
              completed: event.completed,
              total: event.total,
            }
          }
          if (event.change === "trial_progress") {
            return {
              ...current,
              currentTrial: event.trialId,
            }
          }
          if (event.change === "progress") {
            return {
              ...current,
              phase: event.phase,
              completed: event.completed,
              total: event.total,
            }
          }
          if (event.change === "budget_warning") {
            const percent = Math.round((event.ratio ?? 0) * 100)
            const warning = `${event.dimension ?? "budget"}: ${percent}% (${event.observed ?? 0}/${event.limit ?? 0})`
            return { ...current, warning }
          }
          return current
        })
        const campaignId = event.campaignId
        const trialId = event.trialId
        if (trialId && campaignId) {
          const key = trialProgressKey(campaignId, trialId)
          if (event.change === "trial_started") {
            setLiveTrials((current) => ({
              ...current,
              [key]: {
                campaignId,
                trialId,
                status: "running",
              },
            }))
          }
          if (event.change === "trial_progress") {
            setLiveTrials((current) => ({
              ...current,
              [key]: {
                ...current[key],
                campaignId,
                trialId,
                status: "running",
                durationMs: event.wallMs,
                modelCalls: event.modelCalls,
                toolCalls: event.toolCalls,
                inputTokens: event.inputTokens,
                outputTokens: event.outputTokens,
                costUsd: event.costUsd,
                loopIterations: event.loopIterations,
                spawnedAgents: event.spawnedAgents,
                asyncJobs: event.asyncJobs,
                activeChildren: event.activeChildren,
                attribution: event.attribution,
                lastEvent: event.lastEvent,
                lastEventStatus: event.lastEventStatus,
              },
            }))
          }
          if (event.change === "trial_completed") {
            setLiveTrials((current) => ({
              ...current,
              [key]: {
                ...current[key],
                campaignId,
                trialId,
                status: "completed",
                outcome: event.outcome,
                durationMs: event.wallMs,
                modelCalls: event.modelCalls,
                toolCalls: event.toolCalls,
                inputTokens: event.inputTokens,
                outputTokens: event.outputTokens,
                costUsd: event.costUsd,
              },
            }))
          }
        }
        if (
          [
            "trial_completed",
            "campaign_completed",
            "completed",
            "cancelled",
            "failed",
            "interrupted",
          ].includes(event.change ?? "")
        ) {
          void refreshRunDetail(event.experimentId).catch(() => undefined)
        }
      }
      if (
        ![
          "progress",
          "trial_started",
          "trial_progress",
          "trial_completed",
          "budget_warning",
          "artifact_written",
        ].includes(event.change ?? "")
      ) {
        void refreshHistory()
      }
    })
    return () => {
      unlisten()
    }
  }, [refreshHistory, refreshRunDetail, t])

  useEffect(() => {
    if (!activeRunId) return
    const timer = window.setInterval(() => {
      void refreshHistory()
      if (focusedRunIdRef.current === activeRunId) {
        void refreshRunDetail(activeRunId).catch(() => undefined)
      }
    }, 2500)
    return () => window.clearInterval(timer)
  }, [activeRunId, refreshHistory, refreshRunDetail])

  useEffect(() => {
    setLiveProgress({ tokens: 0, costUsd: 0 })
    setLiveTrials({})
    setRunDetail(null)
    if (!focusedRunId) return
    setRunDetailLoading(true)
    refreshRunDetail(focusedRunId)
      .catch((error) => toast.error(String(error)))
      .finally(() => setRunDetailLoading(false))
  }, [focusedRunId, refreshRunDetail])

  useEffect(() => {
    setPreview(null)
    if (!profile) return
    setMaxCost((current) => Math.min(Math.max(current, 0.01), profile.maxCostUsd))
    setConcurrency((current) => Math.min(Math.max(current, 1), profile.maxConcurrency))
    setSelectedArms(
      profile.armMode === "one_control_per_case"
        ? profile.allowedArms
            .filter((arm) => arm === "control" || arm.endsWith("_control"))
            .slice(0, 1)
        : profile.allowedArms,
    )
    setRepetitions(profile.defaultRepetitions ?? 1)
    setSelectedCases([])
  }, [profile])

  const buildRequest = useCallback((): EvalAppRunRequest => {
    if (!profile) throw new Error("No evaluation profile selected")
    const suiteSelections = profile.allowCustom
      ? profile.suites.map((suite) => ({
          suiteId: suite.suiteId,
          caseIds: selectedCases.filter((caseId) =>
            availableCases.some((item) => item.suiteId === suite.suiteId && item.id === caseId),
          ),
          arms: selectedArms,
          repetitions: profile.useSuiteRepetitions ? undefined : repetitions,
        }))
      : []
    return {
      schemaVersion: "eval-app-run-request.v1",
      profileId: profile.id,
      suiteSelections,
      models: selectedModelOptions.map((model) => ({
        providerId: model.providerId,
        modelId: model.modelId,
        credentialProfileRef:
          selectedCredentials[modelKey(model)] ?? model.credentialProfiles[0]?.credentialProfileRef,
      })),
      campaignBudget: {
        maxWallSeconds: Math.round(maxWallMinutes * 60),
        maxModelCalls: Math.max(20, profile.maxTrials * 20),
        maxInputTokens: Math.max(100_000, profile.maxTrials * 100_000),
        maxOutputTokens: Math.max(20_000, profile.maxTrials * 20_000),
        maxCostUsd: maxCost,
        maxToolCalls: Math.max(100, profile.maxTrials * 100),
        maxAgents: 16,
        maxConcurrency: concurrency,
      },
      debugRetention: "redacted",
      consent: { modelCosts: consentCosts, syntheticToolExecution: consentTools },
    }
  }, [
    availableCases,
    concurrency,
    consentCosts,
    consentTools,
    maxCost,
    maxWallMinutes,
    profile,
    repetitions,
    selectedArms,
    selectedCases,
    selectedCredentials,
    selectedModelOptions,
  ])

  async function handlePreview() {
    setActionLoading(true)
    try {
      const next = await getTransport().call<EvalPreview>("eval_preview", {
        request: buildRequest(),
      })
      setPreview(next)
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleStart() {
    setActionLoading(true)
    try {
      const plan = preview?.plan ?? null
      const experimentId = await getTransport().call<string>("eval_start", {
        request: buildRequest(),
        parentExperimentId: null,
        expectedPlanDigest: preview?.plan.planDigest,
      })
      focusedRunIdRef.current = experimentId
      setActiveRunId(experimentId)
      setFocusedRunId(experimentId)
      setFocusedRunPlan(plan)
      setActiveSection("run")
      toast.success(t("dashboard.evaluation.started", "评测已启动"))
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleCancel() {
    if (!activeRunId) return
    setActionLoading(true)
    try {
      await getTransport().call("eval_cancel", { experimentId: activeRunId })
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleRetry(experimentId: string) {
    setActionLoading(true)
    try {
      const nextId = await getTransport().call<string>("eval_retry", { experimentId })
      focusedRunIdRef.current = nextId
      setActiveRunId(nextId)
      setFocusedRunId(nextId)
      setFocusedRunPlan(null)
      setActiveSection("run")
      setDetail(null)
      toast.success(t("dashboard.evaluation.retryCreated"))
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  function handleNewRun() {
    if (activeRunId) return
    focusedRunIdRef.current = null
    setFocusedRunId(null)
    setFocusedRunPlan(null)
    setRunDetail(null)
    setLiveTrials({})
    setLiveProgress({ tokens: 0, costUsd: 0 })
    setPreview(null)
    setActiveSection("run")
  }

  async function handlePinned(experimentId: string, pinned: boolean) {
    try {
      await getTransport().call("eval_set_pinned", { experimentId, pinned })
      toast.success(t(pinned ? "dashboard.evaluation.pinned" : "dashboard.evaluation.unpinned"))
      await refreshHistory()
      await openDetail(experimentId)
    } catch (error) {
      toast.error(String(error))
    }
  }

  async function openDetail(experimentId: string) {
    try {
      const [nextDetail, nextAnnotations] = await Promise.all([
        getTransport().call<EvalExperimentDetail>("eval_get_experiment", { experimentId }),
        experimentId.startsWith("coding:") || experimentId.startsWith("domain:")
          ? Promise.resolve([] as EvalAnnotationRecord[])
          : getTransport().call<EvalAnnotationRecord[]>("eval_list_annotations", { experimentId }),
      ])
      setDetail(nextDetail)
      setAnnotations(nextAnnotations)
      setAnnotationText("")
    } catch (error) {
      toast.error(String(error))
    }
  }

  async function handleCreateAnnotation() {
    if (!detail || !annotationText.trim()) return
    try {
      await getTransport().call("eval_create_annotation", {
        experimentId: detail.experiment.id,
        campaignId: null,
        trialId: null,
        text: annotationText.trim(),
      })
      await openDetail(detail.experiment.id)
    } catch (error) {
      toast.error(String(error))
    }
  }

  async function handleCompare() {
    if (!compareBaselineId || !compareCandidateId) return
    setActionLoading(true)
    try {
      setComparison(
        await getTransport().call<EvalCompareResult>("eval_compare", {
          query: {
            baselineExperimentId: compareBaselineId,
            candidateExperimentId: compareCandidateId,
          },
        }),
      )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleLoadTrends() {
    if (!trendBaselineId) return
    setActionLoading(true)
    try {
      setTrends(
        await getTransport().call<EvalTrendPoint[]>("eval_trends", {
          query: { baselineExperimentId: trendBaselineId, metric: trendMetric, limit: 100 },
        }),
      )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleImportBundle() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Hope Evaluation Evidence", extensions: ["zip"] }],
      })
      if (!selected || Array.isArray(selected)) return
      setActionLoading(true)
      const result = await getTransport().call<EvalImportResult>("eval_import_bundle", {
        bundlePath: selected,
      })
      toast.success(
        t(
          result.alreadyImported
            ? "dashboard.evaluation.importAlready"
            : "dashboard.evaluation.importSuccess",
        ),
      )
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleImportUnverified() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Hope Model Evidence", extensions: ["json"] }],
      })
      if (!selected || Array.isArray(selected)) return
      setActionLoading(true)
      const result = await getTransport().call<EvalImportResult>("eval_import_unverified", {
        evidencePath: selected,
      })
      toast.warning(
        t(
          result.alreadyImported
            ? "dashboard.evaluation.importAlready"
            : "dashboard.evaluation.importUnverifiedSuccess",
        ),
      )
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleExportLocal(experimentId: string) {
    try {
      const { save } = await import("@tauri-apps/plugin-dialog")
      const outputPath = await save({
        defaultPath: `${experimentId}.hope-eval.zip`,
        filters: [{ name: "Hope Local Evaluation", extensions: ["zip"] }],
      })
      if (!outputPath) return
      setActionLoading(true)
      const result = await getTransport().call<EvalLocalExportResult>("eval_export_local_bundle", {
        experimentId,
        outputPath,
      })
      toast.success(t("dashboard.evaluation.exportLocalSuccess", { count: result.campaignCount }))
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleCreateBaseline(experimentId: string) {
    setActionLoading(true)
    try {
      const experiment = history.find((item) => item.id === experimentId)
      const tier = experiment?.profileId.startsWith("protected:")
        ? experiment.profileId.slice("protected:".length)
        : null
      if (!tier || !["nightly", "weekly", "release", "monthly"].includes(tier)) {
        throw new Error(t("dashboard.evaluation.baselineTierMissing"))
      }
      await getTransport().call("eval_create_baseline", {
        experimentId,
        tier,
        note: "Approved in Evaluation Center",
      })
      toast.success(t("dashboard.evaluation.baselineCreated"))
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setActionLoading(false)
    }
  }

  async function handleDeleteBaseline(baselineId: string) {
    try {
      await getTransport().call("eval_delete_baseline", { baselineId })
      await refreshHistory()
    } catch (error) {
      toast.error(String(error))
    }
  }

  function toggleModel(model: EvalModelOption) {
    setPreview(null)
    const key = modelKey(model)
    setSelectedModels((current) => {
      if (current.includes(key)) return current.filter((value) => value !== key)
      if (!profile || current.length >= profile.maxModels || current.length >= 4) return current
      if (model.credentialProfiles[0]) {
        setSelectedCredentials((values) => ({
          ...values,
          [key]: values[key] ?? model.credentialProfiles[0].credentialProfileRef,
        }))
      }
      return [...current, key]
    })
  }

  function toggleCase(caseId: string) {
    setPreview(null)
    setSelectedCases((current) =>
      current.includes(caseId) ? current.filter((value) => value !== caseId) : [...current, caseId],
    )
  }

  function toggleArm(arm: string) {
    setPreview(null)
    setSelectedArms((current) =>
      current.includes(arm) ? current.filter((value) => value !== arm) : [...current, arm],
    )
  }

  if (loading && !catalog) {
    return (
      <div className="py-12 text-center text-sm text-muted-foreground">{t("common.loading")}</div>
    )
  }

  return (
    <div className="space-y-5">
      <section className="rounded-xl bg-secondary/30 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="flex items-center gap-2">
              <FlaskConical className="h-5 w-5 text-primary" />
              <h2 className="text-base font-semibold">
                {t("dashboard.evaluation.title", "能力评测")}
              </h2>
            </div>
            <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
              {t(
                "dashboard.evaluation.subtitle",
                "在隔离进程中用已配置的真实模型运行 Goal、Workflow、异步任务和多 Agent 场景。",
              )}
            </p>
          </div>
          <ReadinessBadge catalog={catalog} />
        </div>
        <div className="mt-3 flex items-start gap-2 rounded-lg bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            {t(
              "dashboard.evaluation.localWarning",
              "本机运行会调用所选模型并可能产生费用，只使用合成任务；结果是本机诊断，永远不能替代受保护的发版证据。",
            )}
          </span>
        </div>
      </section>

      <Tabs value={activeSection} onValueChange={setActiveSection}>
        <TabsList>
          <TabsTrigger value="run" className="gap-2">
            {t("dashboard.evaluation.run", "运行")}
            {activeRun && (
              <span
                className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500"
                aria-label={t("dashboard.evaluation.running", "评测运行中")}
              />
            )}
          </TabsTrigger>
          <TabsTrigger value="history">{t("dashboard.evaluation.history", "历史")}</TabsTrigger>
          <TabsTrigger value="compare">{t("dashboard.evaluation.compare")}</TabsTrigger>
          <TabsTrigger value="trends">{t("dashboard.evaluation.trends")}</TabsTrigger>
          <TabsTrigger value="baselines">{t("dashboard.evaluation.baselines")}</TabsTrigger>
          <TabsTrigger value="metrics">{t("dashboard.evaluation.metrics", "指标概览")}</TabsTrigger>
        </TabsList>

        <TabsContent value="run" className="space-y-4">
          {focusedRunId ? (
            focusedRun ? (
              <RunMonitorPanel
                run={focusedRun}
                detail={runDetail}
                plan={focusedRunPlan}
                catalog={catalog}
                live={liveProgress}
                liveTrials={liveTrials}
                loading={runDetailLoading}
                onCancel={handleCancel}
                onNewRun={handleNewRun}
                busy={actionLoading}
              />
            ) : (
              <div className="flex items-center justify-center gap-2 rounded-xl bg-secondary/20 py-12 text-sm text-muted-foreground">
                <RefreshCw className="h-4 w-4 animate-spin" />
                {t("dashboard.evaluation.loadingRun", "正在加载运行详情…")}
              </div>
            )
          ) : (
            <>
              <WizardSection
                number="1"
                title={t("dashboard.evaluation.chooseProfile", "选择评测画像")}
              >
                <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
                  {catalog?.profiles.map((item) => {
                    const selected = profileId === item.id
                    return (
                      <button
                        type="button"
                        key={item.id}
                        aria-pressed={selected}
                        onClick={() => setProfileId(item.id)}
                        className={cn(
                          "rounded-lg p-3 text-left transition-colors",
                          selected ? "bg-secondary" : "bg-secondary/30 hover:bg-secondary/40",
                        )}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="text-sm font-medium">
                            {t(`dashboard.evaluation.profiles.${item.id}.title`, item.title)}
                          </div>
                          <SelectionMark selected={selected} kind="radio" />
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">
                          {t(
                            `dashboard.evaluation.profiles.${item.id}.description`,
                            item.description,
                          )}
                        </div>
                        <div className="mt-2 text-[11px] text-muted-foreground">
                          {t("dashboard.evaluation.profileLimits", {
                            trials: item.maxTrials,
                            models: item.maxModels,
                          })}
                        </div>
                      </button>
                    )
                  })}
                </div>
              </WizardSection>

              {profile?.allowCustom && (
                <WizardSection
                  number="2"
                  title={t("dashboard.evaluation.chooseCases", "选择场景与对照组")}
                >
                  <div className="grid gap-2 lg:grid-cols-2">
                    {availableCases.map((item) => (
                      <button
                        type="button"
                        key={item.id}
                        aria-pressed={selectedCases.includes(item.id)}
                        onClick={() => toggleCase(item.id)}
                        className={cn(
                          "rounded-lg p-3 text-left text-sm transition-colors",
                          selectedCases.includes(item.id)
                            ? "bg-secondary"
                            : "bg-secondary/30 hover:bg-secondary/40",
                        )}
                      >
                        <span className="flex items-start justify-between gap-3">
                          <span>
                            <span className="font-medium">{item.id}</span>
                            <span className="ml-2 text-muted-foreground">{item.title}</span>
                          </span>
                          <SelectionMark
                            selected={selectedCases.includes(item.id)}
                            kind="checkbox"
                          />
                        </span>
                      </button>
                    ))}
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    {profile.allowedArms.map((arm) => (
                      <Button
                        key={arm}
                        size="sm"
                        variant={selectedArms.includes(arm) ? "secondary" : "ghost"}
                        onClick={() => toggleArm(arm)}
                      >
                        {arm}
                      </Button>
                    ))}
                    {!profile.useSuiteRepetitions && (
                      <label className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
                        {t("dashboard.evaluation.repetitions", "重复")}
                        <NumberInput
                          className="h-8 w-20"
                          min={1}
                          max={5}
                          value={repetitions}
                          onChange={(event) => {
                            setPreview(null)
                            setRepetitions(
                              Math.min(5, Math.max(1, Number(event.target.value) || 1)),
                            )
                          }}
                        />
                      </label>
                    )}
                  </div>
                </WizardSection>
              )}

              <WizardSection
                number={profile?.allowCustom ? "3" : "2"}
                title={t("dashboard.evaluation.chooseModels", "选择真实模型")}
              >
                {catalog?.models.filter((model) => model.supportsIsolatedEval).length ? (
                  <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                    {catalog.models
                      .filter((model) => model.supportsIsolatedEval)
                      .map((model) => {
                        const selected = selectedModels.includes(modelKey(model))
                        const key = modelKey(model)
                        const codexLocalOnly = model.warnings.includes(
                          "codex_oauth_local_diagnostic_only",
                        )
                        return (
                          <div
                            key={key}
                            className={cn(
                              "rounded-lg p-3 transition-colors",
                              selected
                                ? "bg-secondary"
                                : "bg-secondary/30 hover:bg-secondary/40",
                            )}
                          >
                            <button
                              type="button"
                              aria-pressed={selected}
                              onClick={() => toggleModel(model)}
                              className="w-full text-left"
                            >
                              <div className="flex items-start justify-between gap-3">
                                <div className="text-sm font-medium">{model.label}</div>
                                <SelectionMark selected={selected} kind="checkbox" />
                              </div>
                              <div className="text-xs text-muted-foreground">
                                {model.providerLabel}
                              </div>
                              <div className="mt-2 text-[11px] text-muted-foreground">
                                {model.costKnown
                                  ? t("dashboard.evaluation.priceKnown", "价格已配置")
                                  : t("dashboard.evaluation.priceUnknown", "费用可能无法估算")}
                                {codexLocalOnly && (
                                  <span>
                                    {" · "}
                                    {t("dashboard.evaluation.diagnosticOnly", "仅诊断")}
                                  </span>
                                )}
                              </div>
                            </button>
                            {selected && model.credentialProfiles.length > 1 && (
                              <Select
                                value={
                                  selectedCredentials[key] ??
                                  model.credentialProfiles[0]?.credentialProfileRef
                                }
                                onValueChange={(value) => {
                                  setPreview(null)
                                  setSelectedCredentials((values) => ({ ...values, [key]: value }))
                                }}
                              >
                                <SelectTrigger className="mt-2 h-8 text-xs">
                                  <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                  {model.credentialProfiles.map((credential) => (
                                    <SelectItem
                                      key={credential.credentialProfileRef}
                                      value={credential.credentialProfileRef}
                                    >
                                      {credential.label}
                                    </SelectItem>
                                  ))}
                                </SelectContent>
                              </Select>
                            )}
                          </div>
                        )
                      })}
                  </div>
                ) : (
                  <div className="rounded-lg bg-secondary/30 p-4 text-sm text-muted-foreground">
                    {t(
                      "dashboard.evaluation.noModels",
                      "没有可用于隔离评测的 API Key、本地模型或已登录 Codex。请先在设置中配置 Provider。",
                    )}
                  </div>
                )}
              </WizardSection>

              <WizardSection
                number={profile?.allowCustom ? "4" : "3"}
                title={t("dashboard.evaluation.budget", "设置硬预算")}
              >
                <div className="grid gap-3 sm:grid-cols-3">
                  <BudgetField
                    label={t("dashboard.evaluation.maxCost", "最高费用（USD）")}
                    value={maxCost}
                    min={0.01}
                    max={profile?.maxCostUsd ?? 1_000_000}
                    step={0.5}
                    onChange={(value) => {
                      setPreview(null)
                      setMaxCost(value)
                    }}
                  />
                  <BudgetField
                    label={t("dashboard.evaluation.maxWall", "最长时间（分钟）")}
                    value={maxWallMinutes}
                    min={1}
                    onChange={(value) => {
                      setPreview(null)
                      setMaxWallMinutes(value)
                    }}
                  />
                  <BudgetField
                    label={t("dashboard.evaluation.concurrency", "并发数")}
                    value={concurrency}
                    min={1}
                    max={profile?.maxConcurrency ?? 500}
                    onChange={(value) => {
                      setPreview(null)
                      setConcurrency(value)
                    }}
                  />
                </div>
              </WizardSection>

              <WizardSection
                number={profile?.allowCustom ? "5" : "4"}
                title={t("dashboard.evaluation.confirm", "预览并确认")}
              >
                <div className="flex flex-wrap gap-2">
                  <ConsentButton
                    selected={consentCosts}
                    onClick={() => {
                      setPreview(null)
                      setConsentCosts((value) => !value)
                    }}
                    label={t("dashboard.evaluation.costConsent", "我确认会产生模型费用")}
                  />
                  <ConsentButton
                    selected={consentTools}
                    onClick={() => {
                      setPreview(null)
                      setConsentTools((value) => !value)
                    }}
                    label={t("dashboard.evaluation.toolConsent", "我确认会执行合成工具任务")}
                  />
                </div>
                {preview && (
                  <div className="mt-3 grid gap-2 rounded-lg bg-secondary/30 p-3 text-sm sm:grid-cols-2 xl:grid-cols-6">
                    <Metric
                      label={t("dashboard.evaluation.trials")}
                      value={String(preview.estimatedTrials)}
                    />
                    <Metric
                      label={t("dashboard.evaluation.models")}
                      value={String(preview.plan.campaigns.length)}
                    />
                    <Metric
                      label={t("dashboard.evaluation.maxCost")}
                      value={`$${preview.maxCostUsd?.toFixed(2) ?? "—"}`}
                    />
                    <Metric
                      label={t("dashboard.evaluation.maxWall", "最长时间（分钟）")}
                      value={
                        preview.maxWallSeconds == null
                          ? "—"
                          : formatLongDuration(preview.maxWallSeconds * 1_000)
                      }
                    />
                    <Metric
                      label={t(
                        "dashboard.evaluation.budgetDimensions.wall",
                        "单场景总时间",
                      )}
                      value={formatPlanTrialTimeouts(preview.plan)}
                    />
                    <Metric
                      label={t("dashboard.evaluation.environment")}
                      value={`${preview.plan.runtimeEnvironment.os}/${preview.plan.runtimeEnvironment.arch}`}
                    />
                  </div>
                )}
                <div className="mt-3 flex gap-2">
                  <Button
                    variant="secondary"
                    onClick={handlePreview}
                    disabled={
                      actionLoading || selectedModels.length === 0 || !consentCosts || !consentTools
                    }
                  >
                    <RefreshCw className={cn("mr-2 h-4 w-4", actionLoading && "animate-spin")} />
                    {t("dashboard.evaluation.preview", "生成计划")}
                  </Button>
                  <Button
                    onClick={handleStart}
                    disabled={!preview || actionLoading || Boolean(activeRunId)}
                  >
                    <Play className="mr-2 h-4 w-4" />
                    {t("dashboard.evaluation.start", "开始真实评测")}
                  </Button>
                </div>
              </WizardSection>
            </>
          )}
        </TabsContent>

        <TabsContent value="history">
          <HistoryTable rows={history} onOpen={openDetail} />
          {detail && (
            <ExperimentDetail
              detail={detail}
              catalog={catalog}
              annotations={annotations}
              annotationText={annotationText}
              onAnnotationTextChange={setAnnotationText}
              onCreateAnnotation={
                detail.experiment.kind === "hope_core" ? handleCreateAnnotation : undefined
              }
              onClose={() => setDetail(null)}
              onPinned={
                detail.experiment.kind === "hope_core" &&
                detail.experiment.integrity !== "protected_verified" &&
                detail.experiment.integrity !== "protected_unknown_assets"
                  ? (pinned) => handlePinned(detail.experiment.id, pinned)
                  : undefined
              }
              onRetry={
                detail.experiment.kind === "hope_core" &&
                detail.experiment.source === "local_app" &&
                !ACTIVE_STATUSES.has(detail.experiment.status) &&
                !activeRunId
                  ? () => handleRetry(detail.experiment.id)
                  : undefined
              }
              onExport={
                detail.experiment.source === "local_app" &&
                detail.experiment.integrity === "local_diagnostic"
                  ? () => handleExportLocal(detail.experiment.id)
                  : undefined
              }
              onCreateBaseline={
                detail.experiment.integrity === "protected_verified" &&
                (detail.experiment.signatureStatus === "verified" ||
                  detail.experiment.signatureStatus === "verified_retired")
                  ? () => handleCreateBaseline(detail.experiment.id)
                  : undefined
              }
            />
          )}
        </TabsContent>

        <TabsContent value="compare">
          <ComparePanel
            history={history}
            baselineId={compareBaselineId}
            candidateId={compareCandidateId}
            onBaselineChange={(value) => {
              setComparison(null)
              setCompareBaselineId(value)
            }}
            onCandidateChange={(value) => {
              setComparison(null)
              setCompareCandidateId(value)
            }}
            comparison={comparison}
            onCompare={handleCompare}
            busy={actionLoading}
          />
        </TabsContent>

        <TabsContent value="trends">
          <TrendsPanel
            history={history}
            baselineId={trendBaselineId}
            metric={trendMetric}
            trends={trends}
            onBaselineChange={(value) => {
              setTrends([])
              setTrendBaselineId(value)
            }}
            onMetricChange={(value) => {
              setTrends([])
              setTrendMetric(value)
            }}
            onLoad={handleLoadTrends}
            busy={actionLoading}
          />
        </TabsContent>

        <TabsContent value="baselines">
          <BaselinesPanel
            baselines={baselines}
            history={history}
            onImport={handleImportBundle}
            onImportUnverified={handleImportUnverified}
            onDelete={handleDeleteBaseline}
            busy={actionLoading}
            importAvailable={catalog?.readiness.signedImportAvailable ?? false}
            importIssues={catalog?.readiness.signedImportIssues ?? []}
          />
        </TabsContent>

        <TabsContent value="metrics">
          <MetricsOverview history={history} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function ReadinessBadge({ catalog }: { catalog: EvalCatalog | null }) {
  const { t } = useTranslation()
  const ready = catalog?.readiness.canRun
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-full px-3 py-1.5 text-xs",
        ready
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : "bg-destructive/10 text-destructive",
      )}
    >
      {ready ? <CheckCircle2 className="h-4 w-4" /> : <XCircle className="h-4 w-4" />}
      {ready
        ? t("dashboard.evaluation.sidecarReady")
        : (catalog?.readiness.issues[0] ?? t("dashboard.evaluation.sidecarUnavailable"))}
    </div>
  )
}

function WizardSection({
  number,
  title,
  children,
}: {
  number: string
  title: string
  children: ReactNode
}) {
  return (
    <section className="rounded-xl bg-secondary/20 p-4">
      <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs text-primary-foreground">
          {number}
        </span>
        {title}
      </h3>
      {children}
    </section>
  )
}

function SelectionMark({ selected, kind }: { selected: boolean; kind: "radio" | "checkbox" }) {
  if (kind === "radio") {
    return (
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border",
          selected ? "border-foreground" : "border-muted-foreground/50",
        )}
      >
        {selected && <span className="h-2 w-2 rounded-full bg-foreground" />}
      </span>
    )
  }

  return (
    <span
      aria-hidden="true"
      className={cn(
        "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] border",
        selected
          ? "border-foreground bg-foreground text-background"
          : "border-muted-foreground/50 bg-background/40",
      )}
    >
      {selected && <Check className="h-3 w-3" strokeWidth={3} />}
    </span>
  )
}

function BudgetField({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string
  value: number
  min: number
  max?: number
  step?: number
  onChange: (value: number) => void
}) {
  return (
    <label className="space-y-1 text-xs text-muted-foreground">
      <span>{label}</span>
      <NumberInput
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(event) => {
          const next = Math.max(min, Number(event.target.value) || min)
          onChange(max === undefined ? next : Math.min(max, next))
        }}
      />
    </label>
  )
}

function ConsentButton({
  selected,
  onClick,
  label,
}: {
  selected: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        "rounded-lg px-3 py-2 text-xs transition-colors",
        selected ? "bg-secondary" : "bg-secondary/30 hover:bg-secondary/40",
      )}
    >
      <span className="mr-2">{selected ? "✓" : "○"}</span>
      {label}
    </button>
  )
}

function RunMonitorPanel({
  run,
  detail,
  plan,
  catalog,
  live,
  liveTrials,
  loading,
  onCancel,
  onNewRun,
  busy,
}: {
  run: EvalExperimentRecord
  detail: EvalExperimentDetail | null
  plan: EvalAppPlan | null
  catalog: EvalCatalog | null
  live: LiveEvalProgress
  liveTrials: Record<string, LiveTrialProgress>
  loading: boolean
  onCancel: () => void
  onNewRun: () => void
  busy: boolean
}) {
  const { t } = useTranslation()
  const [trialDetail, setTrialDetail] = useState<EvalTrialDetail | null>(null)
  const [selectedLiveTrialKey, setSelectedLiveTrialKey] = useState<string | null>(null)
  const [trialLoading, setTrialLoading] = useState(false)
  const [now, setNow] = useState(Date.now())
  const active = ACTIVE_STATUSES.has(run.status)

  useEffect(() => {
    setNow(Date.now())
    if (!active) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [active])

  const persistedByKey = new Map(
    (detail?.trials ?? []).map((trial) => [trialProgressKey(trial.campaignId, trial.id), trial]),
  )
  const rowsByKey = new Map<string, MonitorTrialRow>()
  for (const campaign of plan?.campaigns ?? []) {
    for (const trial of campaign.resolvedPlan.trials) {
      const key = trialProgressKey(campaign.campaignId, trial.id)
      rowsByKey.set(key, {
        campaignId: campaign.campaignId,
        trialId: trial.id,
        suiteId: trial.suiteId,
        caseId: trial.caseId,
        arm: trial.arm,
        status: "queued",
        persisted: false,
      })
    }
  }
  for (const [key, trial] of Object.entries(liveTrials)) {
    rowsByKey.set(key, { ...rowsByKey.get(key), ...trial, persisted: false })
  }
  for (const [key, trial] of persistedByKey) {
    rowsByKey.set(key, monitorTrialFromRecord(trial))
  }
  const startedKeys = new Set(Object.keys(liveTrials))
  const persistedKeys = new Set(persistedByKey.keys())
  const trialRows = [...rowsByKey.entries()].map(([key, row]) => ({
    ...row,
    status:
      persistedKeys.has(key) || row.status === "completed"
        ? ("completed" as const)
        : active
          ? startedKeys.has(key)
            ? ("running" as const)
            : ("queued" as const)
          : startedKeys.has(key)
            ? ("aborted" as const)
            : ("not_run" as const),
  }))

  const completed = Math.max(
    run.completedTrials,
    live.completed ?? 0,
    trialRows.filter((trial) => trial.status === "completed").length,
  )
  const total = Math.max(run.totalTrials, live.total ?? 0)
  const progress = total ? (completed / total) * 100 : 0
  const passed = Math.max(
    run.passedTrials,
    trialRows.filter((trial) => trial.outcome === "passed").length,
  )
  const infraErrors = Math.max(
    run.infraErrorTrials,
    trialRows.filter((trial) => trial.outcome === "infra_error").length,
  )
  const failed = Math.max(
    run.failedTrials,
    trialRows.filter((trial) =>
      ["task_failed", "policy_failed", "budget_exhausted"].includes(trial.outcome ?? ""),
    ).length,
  )
  const tokens = Math.max(
    live.tokens,
    trialRows.reduce((sum, trial) => sum + (trial.inputTokens ?? 0) + (trial.outputTokens ?? 0), 0),
  )
  const toolCalls = trialRows.reduce((sum, trial) => sum + (trial.toolCalls ?? 0), 0)
  const modelCalls = trialRows.reduce((sum, trial) => sum + (trial.modelCalls ?? 0), 0)
  const observedCost = Math.max(
    run.observedCostUsd ?? 0,
    live.costUsd,
    trialRows.reduce((sum, trial) => sum + (trial.costUsd ?? 0), 0),
  )
  const startedAt = run.startedAt ? new Date(run.startedAt).getTime() : NaN
  const endedAt = run.completedAt ? new Date(run.completedAt).getTime() : now
  const elapsedMs = Number.isFinite(startedAt) ? Math.max(0, endedAt - startedAt) : 0
  const campaignPlanById = new Map(
    (plan?.campaigns ?? []).map((campaign) => [campaign.campaignId, campaign]),
  )
  const campaignIds = new Set([
    ...(plan?.campaigns ?? []).map((campaign) => campaign.campaignId),
    ...(detail?.campaigns ?? []).map((campaign) => campaign.id),
    ...trialRows.map((trial) => trial.campaignId),
  ])
  const selectedLiveTrial = selectedLiveTrialKey
    ? trialRows.find(
        (trial) => trialProgressKey(trial.campaignId, trial.trialId) === selectedLiveTrialKey,
      )
    : undefined

  async function openTrial(row: MonitorTrialRow) {
    const key = trialProgressKey(row.campaignId, row.trialId)
    if (!row.persisted) {
      setTrialDetail(null)
      setSelectedLiveTrialKey((current) => (current === key ? null : key))
      return
    }
    if (trialDetail?.record.id === row.trialId) {
      setTrialDetail(null)
      return
    }
    setSelectedLiveTrialKey(null)
    setTrialLoading(true)
    try {
      setTrialDetail(
        await getTransport().call<EvalTrialDetail>("eval_get_trial", {
          experimentId: run.id,
          campaignId: row.campaignId,
          trialId: row.trialId,
        }),
      )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setTrialLoading(false)
    }
  }

  return (
    <div className="space-y-4">
      <section className="rounded-xl bg-secondary/20 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              {active ? (
                <RefreshCw className="h-4 w-4 animate-spin text-emerald-600" />
              ) : run.status === "completed" ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              ) : (
                <XCircle className="h-4 w-4 text-destructive" />
              )}
              <h3 className="text-sm font-semibold">
                {active
                  ? t("dashboard.evaluation.runningDetail", "实时运行详情")
                  : t("dashboard.evaluation.resultDetail", "评测结果")}
              </h3>
              <StatusBadge status={run.status} />
              <IntegrityBadge integrity={run.integrity} />
            </div>
            <div className="mt-1 truncate text-xs text-muted-foreground">
              {t(`dashboard.evaluation.profiles.${run.profileId}.title`, run.profileId)} ·{" "}
              {active
                ? t(`dashboard.evaluation.phases.${live.phase ?? run.status}`, {
                    defaultValue: live.phase ?? run.status,
                  })
                : t(`dashboard.evaluation.statuses.${run.status}`, {
                    defaultValue: run.status,
                  })} · {run.id}
            </div>
          </div>
          {active ? (
            <Button
              size="sm"
              variant="secondary"
              onClick={onCancel}
              disabled={busy || run.status === "cancelling"}
            >
              <Square className="mr-2 h-3.5 w-3.5" />
              {t("common.cancel")}
            </Button>
          ) : (
            <Button size="sm" onClick={onNewRun} disabled={busy}>
              <Play className="mr-2 h-3.5 w-3.5" />
              {t("dashboard.evaluation.newRun", "开始新评测")}
            </Button>
          )}
        </div>
        <div className="mt-4 flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>{t("dashboard.evaluation.progressTrials", { completed, total })}</span>
          <span>{Math.round(progress)}%</span>
        </div>
        <Progress
          className="mt-2"
          value={progress}
          indeterminate={run.status === "planning" && run.totalTrials === 0}
        />
        {live.warning && (
          <div className="mt-3 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
            {live.warning}
          </div>
        )}
        {run.error && (
          <div className="mt-3 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {run.error}
          </div>
        )}
      </section>

      <section className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-8">
        <RunMetricCard
          label={t("dashboard.evaluation.completed", "已完成")}
          value={`${completed}/${total}`}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.passed", "通过")}
          value={String(passed)}
          tone="success"
        />
        <RunMetricCard
          label={t("dashboard.evaluation.failed", "失败")}
          value={String(failed)}
          tone={failed > 0 ? "danger" : "default"}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.infraErrors", "基础设施错误")}
          value={String(infraErrors)}
          tone={infraErrors > 0 ? "warning" : "default"}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.tokens", "Token 数")}
          value={tokens.toLocaleString()}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.toolCalls", "工具调用")}
          value={toolCalls.toLocaleString()}
          hint={`${modelCalls} ${t("dashboard.evaluation.modelCalls", "模型调用")}`}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.cost", "费用")}
          value={observedCost > 0 ? `$${observedCost.toFixed(4)}` : "—"}
        />
        <RunMetricCard
          label={t("dashboard.evaluation.duration", "耗时")}
          value={formatLongDuration(elapsedMs)}
        />
      </section>

      <section className="rounded-xl bg-secondary/20 p-4">
        <h3 className="text-sm font-semibold">
          {t("dashboard.evaluation.campaignStatus", "评测批次状态")}
        </h3>
        <div className="mt-3 grid gap-2 lg:grid-cols-2">
          {[...campaignIds].map((campaignId) => {
            const record = detail?.campaigns.find((campaign) => campaign.id === campaignId)
            const campaignPlan = campaignPlanById.get(campaignId)
            const campaignTrials = trialRows.filter((trial) => trial.campaignId === campaignId)
            const campaignCompleted = campaignTrials.filter(
              (trial) => trial.status === "completed",
            ).length
            const campaignTotal = Math.max(record?.totalTrials ?? 0, campaignTrials.length)
            const campaignActive = campaignTrials.some((trial) => trial.status === "running")
            const status: EvalExperimentRecord["status"] =
              record?.status === "completed"
                ? "completed"
                : !active
                  ? run.status
                  : campaignActive
                    ? "running"
                    : "queued"
            return (
              <div key={campaignId} className="rounded-lg bg-background/50 p-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">
                      {campaignPlan
                        ? `${campaignPlan.model.providerId} / ${campaignPlan.model.modelId}`
                        : campaignId}
                    </div>
                    <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                      {campaignId}
                    </div>
                  </div>
                  <StatusBadge status={status} />
                </div>
                <div className="mt-3 flex items-center justify-between text-[11px] text-muted-foreground">
                  <span>
                    {campaignCompleted}/{campaignTotal}
                  </span>
                  <span>{record?.costUsd == null ? "—" : `$${record.costUsd.toFixed(4)}`}</span>
                </div>
                <Progress
                  className="mt-1.5"
                  value={campaignTotal ? (campaignCompleted / campaignTotal) * 100 : 0}
                />
              </div>
            )
          })}
        </div>
      </section>

      <section className="overflow-hidden rounded-xl bg-secondary/20">
        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <h3 className="text-sm font-semibold">
            {t("dashboard.evaluation.trialStatus", "场景实时状态")}
          </h3>
          {loading && <RefreshCw className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
        </div>
        <div className="max-h-[34rem] overflow-auto">
          {trialRows.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-muted-foreground">
              {t("dashboard.evaluation.waitingForTrials", "正在准备评测场景…")}
            </div>
          ) : (
            <div className="min-w-[760px]">
              <div className="grid grid-cols-[minmax(280px,1fr)_120px_100px_90px_100px_100px_20px] items-center gap-3 px-4 py-2 text-[10px] font-medium text-muted-foreground">
                <span>{t("dashboard.evaluation.scenario", "场景")}</span>
                <span className="text-center">{t("dashboard.evaluation.statusLabel", "状态")}</span>
                <span className="text-center">{t("dashboard.evaluation.duration", "耗时")}</span>
                <span className="text-center">{t("dashboard.evaluation.toolCalls", "工具调用")}</span>
                <span className="text-center">{t("dashboard.evaluation.tokens", "Token 数")}</span>
                <span className="text-center">{t("dashboard.evaluation.cost", "费用")}</span>
                <span />
              </div>
              {trialRows.map((trial) => {
                const key = trialProgressKey(trial.campaignId, trial.trialId)
                const tokenCount = (trial.inputTokens ?? 0) + (trial.outputTokens ?? 0)
                const caseTitle = evaluationCaseTitle(catalog, trial.suiteId, trial.caseId)
                const selected =
                  selectedLiveTrialKey === key || trialDetail?.record.id === trial.trialId
                const hasLiveDetail =
                  trial.durationMs != null ||
                  trial.modelCalls != null ||
                  trial.toolCalls != null ||
                  trial.lastEvent != null
                return (
                  <button
                    key={key}
                    type="button"
                    aria-expanded={selected}
                    disabled={(!trial.persisted && !hasLiveDetail) || trialLoading}
                    onClick={() => openTrial(trial)}
                    className={cn(
                      "grid w-full grid-cols-[minmax(280px,1fr)_120px_100px_90px_100px_100px_20px] items-center gap-3 px-4 py-2.5 text-left text-xs transition-colors hover:bg-secondary/40 disabled:pointer-events-none",
                      selected && "bg-secondary",
                    )}
                  >
                    <span className="min-w-0">
                      <span className="block truncate font-medium">
                        {caseTitle ?? trial.caseId ?? trial.trialId}
                      </span>
                      <span className="block truncate text-[10px] text-muted-foreground">
                        {trial.caseId ?? trial.trialId} · {trial.suiteId ?? trial.campaignId} ·{" "}
                        {trial.arm ?? "—"}
                      </span>
                    </span>
                    <TrialStateBadge status={trial.status} outcome={trial.outcome} />
                    <span className="text-center tabular-nums">
                      {trial.durationMs == null ? "—" : formatDuration(trial.durationMs)}
                    </span>
                    <span className="text-center tabular-nums">{trial.toolCalls ?? "—"}</span>
                    <span className="text-center tabular-nums">{tokenCount || "—"}</span>
                    <span className="text-center tabular-nums">
                      {trial.costUsd == null ? "—" : `$${trial.costUsd.toFixed(4)}`}
                    </span>
                    {selected ? (
                      <ChevronDown className="h-4 w-4 text-muted-foreground" />
                    ) : (
                      <ChevronRight className="h-4 w-4 text-muted-foreground" />
                    )}
                  </button>
                )
              })}
            </div>
          )}
        </div>
      </section>

      {trialDetail &&
        (trialDetail.result ? (
          <TrialCausalDetail
            detail={trialDetail}
            caseTitle={evaluationCaseTitle(
              catalog,
              trialDetail.record.suiteId,
              trialDetail.record.caseId,
            )}
            onClose={() => setTrialDetail(null)}
          />
        ) : (
          <TrialRecordDetail
            detail={trialDetail}
            caseTitle={evaluationCaseTitle(
              catalog,
              trialDetail.record.suiteId,
              trialDetail.record.caseId,
            )}
            onClose={() => setTrialDetail(null)}
          />
        ))}
      {!trialDetail && selectedLiveTrialKey && (
        <LiveTrialDetail
          trial={selectedLiveTrial}
          caseTitle={evaluationCaseTitle(
            catalog,
            selectedLiveTrial?.suiteId,
            selectedLiveTrial?.caseId,
          )}
          onClose={() => setSelectedLiveTrialKey(null)}
        />
      )}
    </div>
  )
}

function RunMetricCard({
  label,
  value,
  hint,
  tone = "default",
}: {
  label: string
  value: string
  hint?: string
  tone?: "default" | "success" | "warning" | "danger"
}) {
  return (
    <div className="rounded-xl bg-secondary/20 p-3">
      <div
        className={cn(
          "text-lg font-semibold tabular-nums",
          tone === "success" && "text-emerald-600 dark:text-emerald-400",
          tone === "warning" && "text-amber-600 dark:text-amber-400",
          tone === "danger" && "text-destructive",
        )}
      >
        {value}
      </div>
      <div className="mt-0.5 text-[11px] text-muted-foreground">{label}</div>
      {hint && <div className="mt-1 text-[10px] text-muted-foreground">{hint}</div>}
    </div>
  )
}

function TrialStateBadge({
  status,
  outcome,
}: {
  status: MonitorTrialRow["status"]
  outcome?: string
}) {
  const { t } = useTranslation()
  const label =
    status === "not_run"
      ? t("dashboard.evaluation.trialNotRun", "未运行")
      : status === "aborted"
        ? t("dashboard.evaluation.trialAborted", "已中止")
        : status === "queued"
          ? t("dashboard.evaluation.trialQueued", "等待中")
          : status === "running"
            ? t("dashboard.evaluation.trialRunning", "运行中")
            : outcome === "passed"
              ? t("dashboard.evaluation.trialPassed", "通过")
              : outcome
                ? t(`dashboard.evaluation.outcomes.${outcome}`, { defaultValue: outcome })
                : t("dashboard.evaluation.trialCompleted", "已完成")
  return (
    <span
      className={cn(
        "inline-flex min-h-5 w-fit items-center justify-center justify-self-center whitespace-nowrap rounded-full px-2 py-0.5 text-[10px]",
        status === "not_run" && "bg-secondary text-muted-foreground",
        status === "aborted" && "bg-destructive/10 text-destructive",
        status === "queued" && "bg-secondary text-muted-foreground",
        status === "running" && "bg-blue-500/10 text-blue-600 dark:text-blue-300",
        status === "completed" &&
          outcome === "passed" &&
          "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
        status === "completed" &&
          outcome === "infra_error" &&
          "bg-amber-500/10 text-amber-700 dark:text-amber-300",
        status === "completed" &&
          outcome !== "passed" &&
          outcome !== "infra_error" &&
          "bg-destructive/10 text-destructive",
      )}
    >
      {label}
    </span>
  )
}

function monitorTrialFromRecord(trial: EvalTrialRecord): MonitorTrialRow {
  return {
    campaignId: trial.campaignId,
    trialId: trial.id,
    suiteId: trial.suiteId,
    caseId: trial.caseId,
    arm: trial.arm,
    status: "completed",
    outcome: trial.outcome,
    durationMs: trial.durationMs,
    modelCalls: trial.modelCalls,
    toolCalls: trial.toolCalls,
    inputTokens: trial.inputTokens,
    outputTokens: trial.outputTokens,
    costUsd: trial.costUsd,
    persisted: true,
  }
}

function evaluationCaseTitle(
  catalog: EvalCatalog | null,
  suiteId?: string,
  caseId?: string,
): string | undefined {
  if (!caseId) return undefined
  const suites = suiteId ? catalog?.suites.filter((suite) => suite.id === suiteId) : catalog?.suites
  return suites?.flatMap((suite) => suite.cases).find((item) => item.id === caseId)?.title
}

function HistoryTable({
  rows,
  onOpen,
}: {
  rows: EvalExperimentRecord[]
  onOpen: (id: string) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="overflow-hidden rounded-xl bg-secondary/20">
      {rows.length === 0 ? (
        <div className="p-8 text-center text-sm text-muted-foreground">
          {t("dashboard.evaluation.emptyHistory")}
        </div>
      ) : (
        rows.map((row) => (
          <button
            key={row.id}
            type="button"
            onClick={() => onOpen(row.id)}
            className="grid w-full grid-cols-[1fr_auto] gap-3 border-b border-border/40 p-3 text-left last:border-0 hover:bg-secondary/40"
          >
            <div>
              <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
                {row.profileId}
                <IntegrityBadge integrity={row.integrity} />
                {row.signatureStatus && <SignatureBadge status={row.signatureStatus} />}
                <StatusBadge status={row.status} />
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {new Date(row.createdAt).toLocaleString()} · {row.kind} ·{" "}
                {row.reference.slice(0, 8)} · {row.source}
              </div>
            </div>
            <div className="text-right text-xs text-muted-foreground">
              <div>
                {row.passedTrials}/{row.totalTrials} {t("dashboard.evaluation.passed")}
              </div>
              <div>
                {row.observedCostUsd == null
                  ? t("dashboard.evaluation.costUnknown")
                  : `$${row.observedCostUsd.toFixed(3)}`}
              </div>
            </div>
          </button>
        ))
      )}
    </div>
  )
}

function ExperimentDetail({
  detail,
  catalog,
  annotations,
  annotationText,
  onAnnotationTextChange,
  onCreateAnnotation,
  onClose,
  onCreateBaseline,
  onRetry,
  onPinned,
  onExport,
}: {
  detail: EvalExperimentDetail
  catalog: EvalCatalog | null
  annotations: EvalAnnotationRecord[]
  annotationText: string
  onAnnotationTextChange: (value: string) => void
  onCreateAnnotation?: () => void
  onClose: () => void
  onCreateBaseline?: () => void
  onRetry?: () => void
  onPinned?: (pinned: boolean) => void
  onExport?: () => void
}) {
  const { t } = useTranslation()
  const [trialDetail, setTrialDetail] = useState<EvalTrialDetail | null>(null)
  const [trialLoading, setTrialLoading] = useState(false)

  async function openTrial(campaignId: string, trialId: string) {
    if (detail.experiment.kind !== "hope_core") return
    if (trialDetail?.record.id === trialId) {
      setTrialDetail(null)
      return
    }
    setTrialLoading(true)
    try {
      setTrialDetail(
        await getTransport().call<EvalTrialDetail>("eval_get_trial", {
          experimentId: detail.experiment.id,
          campaignId,
          trialId,
        }),
      )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setTrialLoading(false)
    }
  }

  return (
    <section className="mt-4 rounded-xl bg-secondary/20 p-4">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <h3 className="min-w-0 truncate font-semibold">{detail.experiment.id}</h3>
          {detail.experiment.signatureStatus && (
            <SignatureBadge status={detail.experiment.signatureStatus} />
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          {onRetry && (
            <Button size="sm" variant="secondary" onClick={onRetry}>
              {t("dashboard.evaluation.retryAsNew")}
            </Button>
          )}
          {onExport && (
            <Button size="sm" variant="secondary" onClick={onExport}>
              <Download className="mr-2 h-3.5 w-3.5" />
              {t("dashboard.evaluation.exportLocal")}
            </Button>
          )}
          {onPinned && (
            <Button
              size="sm"
              variant="secondary"
              onClick={() => onPinned(!detail.experiment.pinned)}
            >
              {t(
                detail.experiment.pinned
                  ? "dashboard.evaluation.unpin"
                  : "dashboard.evaluation.pin",
              )}
            </Button>
          )}
          {onCreateBaseline && (
            <Button size="sm" variant="secondary" onClick={onCreateBaseline}>
              {t("dashboard.evaluation.setBaseline")}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.close")}
          </Button>
        </div>
      </div>
      <div className="mt-3 grid gap-2 sm:grid-cols-4">
        <Metric
          label={t("dashboard.evaluation.passed")}
          value={String(detail.experiment.passedTrials)}
        />
        <Metric
          label={t("dashboard.evaluation.failed")}
          value={String(detail.experiment.failedTrials)}
        />
        <Metric
          label={t("dashboard.evaluation.infraErrors")}
          value={String(detail.experiment.infraErrorTrials)}
        />
        <Metric
          label={t("dashboard.evaluation.cost")}
          value={
            detail.experiment.observedCostUsd == null
              ? "—"
              : `$${detail.experiment.observedCostUsd.toFixed(3)}`
          }
        />
      </div>
      {detail.experiment.kind === "hope_core" && (
        <div className="mt-4 rounded-lg bg-blue-500/10 px-3 py-2 text-xs text-blue-700 dark:text-blue-200">
          {t(
            "dashboard.evaluation.historyDetailPrivacy",
            "点击任一场景可查看脱敏的实际运行详情；历史不会保存 Prompt、模型正文或工具参数与输出。",
          )}
        </div>
      )}
      <div className="mt-4 space-y-1 overflow-x-auto">
        {detail.trials.map((trial) => {
          const selected = trialDetail?.record.id === trial.id
          const caseTitle = evaluationCaseTitle(catalog, trial.suiteId, trial.caseId)
          return (
            <div key={`${trial.campaignId}-${trial.id}`} className="min-w-[760px]">
              <button
                type="button"
                aria-expanded={selected}
                onClick={() => openTrial(trial.campaignId, trial.id)}
                disabled={detail.experiment.kind !== "hope_core" || trialLoading}
                className={cn(
                  "grid w-full grid-cols-[minmax(260px,1fr)_auto_auto_120px] items-center gap-3 rounded-lg bg-background/50 px-3 py-2 text-left text-xs hover:bg-secondary/40 disabled:pointer-events-none",
                  selected && "bg-secondary",
                )}
              >
                <div className="min-w-0">
                  <div className="truncate font-medium">{caseTitle ?? trial.caseId}</div>
                  <div className="truncate text-[10px] text-muted-foreground">
                    {trial.caseId} · {trial.suiteId} · {trial.arm}
                  </div>
                </div>
                <TrialStateBadge status="completed" outcome={trial.outcome} />
                <div className="text-muted-foreground">
                  {formatDuration(trial.durationMs)} ·{" "}
                  {t("dashboard.evaluation.toolCallsShort", { count: trial.toolCalls })} ·{" "}
                  {t("dashboard.evaluation.tokensShort", {
                    count: (trial.inputTokens ?? 0) + (trial.outputTokens ?? 0),
                  })}
                </div>
                <span className="flex items-center justify-end gap-1 text-muted-foreground">
                  {t(
                    selected
                      ? "dashboard.evaluation.hideScenarioDetails"
                      : "dashboard.evaluation.viewScenarioDetails",
                    selected ? "收起运行详情" : "查看运行详情",
                  )}
                  {selected ? (
                    <ChevronDown className="h-4 w-4" />
                  ) : (
                    <ChevronRight className="h-4 w-4" />
                  )}
                </span>
              </button>
              {selected &&
                trialDetail &&
                (trialDetail.result ? (
                  <TrialCausalDetail
                    detail={trialDetail}
                    caseTitle={caseTitle}
                    onClose={() => setTrialDetail(null)}
                  />
                ) : (
                  <TrialRecordDetail
                    detail={trialDetail}
                    caseTitle={caseTitle}
                    onClose={() => setTrialDetail(null)}
                  />
                ))}
            </div>
          )
        })}
      </div>
      {onCreateAnnotation && (
        <div className="mt-4 space-y-2">
          <form
            className="flex flex-col gap-2 sm:flex-row sm:items-center"
            onSubmit={(event) => {
              event.preventDefault()
              if (annotationText.trim()) onCreateAnnotation()
            }}
          >
            <Input
              className="min-w-0 flex-1"
              value={annotationText}
              maxLength={4000}
              placeholder={t("dashboard.evaluation.annotationPlaceholder")}
              onChange={(event) => onAnnotationTextChange(event.target.value)}
            />
            <Button
              type="submit"
              variant="secondary"
              className="shrink-0 whitespace-nowrap sm:min-w-24"
              disabled={!annotationText.trim()}
            >
              {t("dashboard.evaluation.addAnnotation")}
            </Button>
          </form>
          {annotations.map((annotation) => (
            <div key={annotation.id} className="rounded bg-background/50 px-3 py-2 text-xs">
              <div>{annotation.text}</div>
              <div className="mt-1 text-[10px] text-muted-foreground">
                {new Date(annotation.createdAt).toLocaleString()}
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

function TrialOutcomeExplanation({ detail }: { detail: EvalTrialDetail }) {
  const { t, i18n } = useTranslation()
  const result = detail.result
  const outcome = result?.outcome ?? detail.record.outcome
  const failureClass = result?.failureClass ?? detail.record.failureClass
  const wallMs = result?.timings.wallMs ?? detail.record.durationMs
  const isWallTimeout = failureClass === "trial_wall_timeout"
  const missingSignals = result?.warnings
    .find((warning) => warning.startsWith("missing required signals:"))
    ?.slice("missing required signals:".length)
    .trim()
  const signalLabels: Record<string, string> = {
    model: t("dashboard.evaluation.signals.model", "模型调用"),
    goal: t("dashboard.evaluation.signals.goal", "Goal 目标"),
    loop: t("dashboard.evaluation.signals.loop", "持续推进循环"),
    workflow: t("dashboard.evaluation.signals.workflow", "工作流"),
    async_jobs: t("dashboard.evaluation.signals.asyncJobs", "异步任务"),
    subagent: t("dashboard.evaluation.signals.subagent", "子 Agent"),
    team: t("dashboard.evaluation.signals.team", "Agent 团队"),
    tool: t("dashboard.evaluation.signals.tool", "工具调用"),
    fault: t("dashboard.evaluation.signals.fault", "故障注入"),
  }
  const missingSignalLabels = missingSignals
    ?.split(",")
    .map((signal) => signal.trim())
    .filter(Boolean)
    .map((signal) => signalLabels[signal] ?? signal)
  const readableMissingSignals = missingSignalLabels?.length
    ? new Intl.ListFormat(i18n.resolvedLanguage ?? i18n.language, {
        style: "long",
        type: "conjunction",
      }).format(missingSignalLabels)
    : undefined

  const exhaustionReasons = new Set<string>()
  for (const warning of result?.warnings ?? []) {
    const match = warning.match(
      /^(?:runtime stopped at immutable trial budget|exceeded trial budgets):\s*(.+)$/,
    )
    if (!match) continue
    for (const value of match[1].split(",")) {
      const reason = value.trim()
      exhaustionReasons.add(
        reason === "wall_time" ? "wall" : reason === "cost_unknown" ? "cost" : reason,
      )
    }
  }
  if (isWallTimeout) {
    exhaustionReasons.add("wall")
    exhaustionReasons.add("wall_time")
  }

  const budget = detail.budget
  const limitRows: Array<{
    key: string
    label: string
    actual: string
    limit: string
    triggered: boolean
  }> = []
  const addLimit = (
    key: string,
    label: string,
    actual: number | undefined,
    limit: number | undefined,
    format: (value: number) => string = (value) => value.toLocaleString(),
  ) => {
    if (limit == null) return
    limitRows.push({
      key,
      label,
      actual: actual == null ? "—" : format(actual),
      limit: format(limit),
      triggered: exhaustionReasons.has(key) || (actual != null && actual >= limit),
    })
  }
  const wallLimitSeconds = detail.timeoutSeconds ?? budget?.maxWallSeconds
  addLimit(
    "wall",
    t("dashboard.evaluation.budgetDimensions.wall", "单场景总时间"),
    wallMs,
    wallLimitSeconds == null ? undefined : wallLimitSeconds * 1_000,
    formatDuration,
  )
  addLimit(
    "model_calls",
    t("dashboard.evaluation.modelCalls", "模型调用"),
    result?.orchestration.modelCalls,
    budget?.maxModelCalls,
  )
  addLimit(
    "input_tokens",
    t("dashboard.evaluation.budgetDimensions.inputTokens", "输入 Token"),
    result?.tokens.input,
    budget?.maxInputTokens,
  )
  addLimit(
    "output_tokens",
    t("dashboard.evaluation.budgetDimensions.outputTokens", "输出 Token"),
    result?.tokens.output,
    budget?.maxOutputTokens,
  )
  addLimit(
    "cost",
    t("dashboard.evaluation.cost", "费用"),
    result?.cost.totalUsd,
    budget?.maxCostUsd,
    (value) => `$${value.toFixed(4)}`,
  )
  addLimit(
    "tool_calls",
    t("dashboard.evaluation.toolCalls", "工具调用"),
    result?.tools.attempted,
    budget?.maxToolCalls,
  )
  addLimit(
    "agents",
    t("dashboard.evaluation.spawnedAgents", "已创建 Agent"),
    result?.orchestration.spawnedAgents,
    budget?.maxAgents,
  )
  addLimit(
    "concurrency",
    t("dashboard.evaluation.budgetDimensions.concurrency", "最大并发"),
    result?.orchestration.maxConcurrency,
    budget?.maxConcurrency,
  )
  if (isWallTimeout) {
    const wallRow = limitRows.find((row) => row.key === "wall")
    if (wallRow) wallRow.triggered = true
  }

  let title = t("dashboard.evaluation.genericFailedTitle", "场景未通过")
  let summary = t(
    "dashboard.evaluation.genericFailedSummary",
    "场景没有得到通过结果，请结合验收结果和技术轨迹定位原因。",
  )
  let tone: "success" | "warning" | "danger" = "danger"

  if (outcome === "passed") {
    title = t("dashboard.evaluation.passExplanationTitle", "场景通过")
    summary = t(
      "dashboard.evaluation.passExplanationSummary",
      "任务已完成，并且所有强制验收项均通过。",
    )
    tone = "success"
  } else if (isWallTimeout) {
    title = t("dashboard.evaluation.wallTimeoutTitle", "单场景时间耗尽")
    summary = t("dashboard.evaluation.wallTimeoutSummary", {
      duration: formatDuration(wallMs),
      defaultValue:
        "该场景运行到 {{duration}} 后被停止。耗尽的是允许的运行时间，不是 Token、费用或模型调用额度。",
    })
    tone = "warning"
  } else if (outcome === "budget_exhausted") {
    title = t("dashboard.evaluation.resourceBudgetTitle", "单场景资源预算耗尽")
    summary = t(
      "dashboard.evaluation.resourceBudgetSummary",
      "场景触及了计划中的模型调用、Token、费用、工具、Agent 或并发上限。",
    )
    tone = "warning"
  } else if (outcome === "task_failed") {
    title = t("dashboard.evaluation.taskFailedTitle", "任务验收未通过")
    summary = t(
      "dashboard.evaluation.taskFailedSummary",
      "场景已经执行结束，但至少一个强制里程碑或验收条件未通过。",
    )
  } else if (outcome === "policy_failed") {
    title = t("dashboard.evaluation.policyFailedTitle", "安全或策略检查未通过")
    summary = t(
      "dashboard.evaluation.policyFailedSummary",
      "运行被安全策略阻止，或产物不符合评测的安全约束。",
    )
  } else if (outcome === "infra_error") {
    title = t("dashboard.evaluation.infraFailedTitle", "模型或评测服务异常")
    summary = t(
      "dashboard.evaluation.infraFailedSummary",
      "Provider、Hope Server 或隔离运行环境发生异常，结果不代表模型能力。",
    )
    tone = "warning"
  }

  return (
    <section
      className={cn(
        "mt-3 rounded-lg px-3 py-3",
        tone === "success" && "bg-emerald-500/10 text-emerald-800 dark:text-emerald-200",
        tone === "warning" && "bg-amber-500/10 text-amber-900 dark:text-amber-100",
        tone === "danger" && "bg-destructive/10 text-destructive",
      )}
    >
      <div className="text-[11px] font-medium opacity-75">
        {t("dashboard.evaluation.resultExplanation", "结果说明")}
      </div>
      <div className="mt-0.5 text-sm font-semibold">{title}</div>
      <p className="mt-1 leading-relaxed opacity-90">{summary}</p>
      {isWallTimeout && result && (
        <p className="mt-1 leading-relaxed opacity-90">
          {t("dashboard.evaluation.wallTimeoutBreakdown", {
            modelDuration: formatDuration(result.timings.modelActiveMs),
            wallDuration: formatDuration(result.timings.wallMs),
            defaultValue:
              "总耗时 {{wallDuration}}，其中模型活跃时间仅 {{modelDuration}}；其余时间位于 Agent 编排、等待或尚未归因的阶段。",
          })}
        </p>
      )}
      {readableMissingSignals && (
        <p className="mt-1 leading-relaxed opacity-90">
          {t("dashboard.evaluation.missingExpectedSignals", {
            signals: readableMissingSignals,
            defaultValue: "结束前没有检测到场景要求的行为：{{signals}}。",
          })}
        </p>
      )}
      {outcome === "budget_exhausted" && limitRows.length > 0 && (
        <div className="mt-3 rounded-md bg-background/45 p-2.5 text-foreground">
          <div className="text-[11px] font-medium text-muted-foreground">
            {t("dashboard.evaluation.budgetLimits", "本次单场景预算")}
          </div>
          <div className="mt-2 grid gap-1.5 sm:grid-cols-2 lg:grid-cols-4">
            {limitRows.map((row) => (
              <div
                key={row.key}
                className={cn(
                  "rounded bg-secondary/40 px-2 py-1.5",
                  row.triggered && "bg-amber-500/15 text-amber-900 dark:text-amber-100",
                )}
              >
                <div className="text-[10px] opacity-70">{row.label}</div>
                <div className="mt-0.5 font-medium tabular-nums">
                  {t("dashboard.evaluation.budgetLimitComparison", {
                    actual: row.actual,
                    limit: row.limit,
                    defaultValue: "实际 {{actual}} / 上限 {{limit}}",
                  })}
                </div>
                {row.triggered && (
                  <div className="mt-0.5 text-[10px] font-medium">
                    {t("dashboard.evaluation.budgetTriggered", "本次触发项")}
                  </div>
                )}
              </div>
            ))}
          </div>
          {isWallTimeout && wallLimitSeconds != null && (
            <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">
              {t("dashboard.evaluation.wallTimeoutLimitNote", {
                limit: formatDuration(wallLimitSeconds * 1_000),
                defaultValue:
                  "配置的单场景总时间上限为 {{limit}}；主执行会提前停止，为取消、证据落盘和进程清理保留尾部时间。",
              })}
            </p>
          )}
        </div>
      )}
      {isWallTimeout && (
        <p className="mt-2 border-t border-current/15 pt-2 leading-relaxed opacity-90">
          <span className="font-medium">{t("common.nextStep", "下一步")}：</span>
          {t(
            "dashboard.evaluation.wallTimeoutNextStep",
            "先检查模型调用结束后为何没有继续推进；如果任务本身确实需要更久，再使用自定义画像提高单场景和实验总时长。",
          )}
        </p>
      )}
    </section>
  )
}

function TrialCausalDetail({
  detail,
  caseTitle,
  onClose,
}: {
  detail: EvalTrialDetail
  caseTitle?: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  const result = detail.result!
  const checks = [...result.milestones, ...result.invariants, ...result.judgeChecks]
  return (
    <div className="mt-4 rounded-lg bg-background/60 p-3 text-xs">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate font-semibold">{caseTitle ?? detail.record.caseId}</div>
          <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
            {detail.record.caseId} · {detail.record.arm} · {result.trialId}
          </div>
        </div>
        <Button size="sm" variant="ghost" onClick={onClose}>
          {t("common.close")}
        </Button>
      </div>
      <TrialOutcomeExplanation detail={detail} />
      <h4 className="mb-2 mt-4 font-semibold">
        {t("dashboard.evaluation.actualUsage", "实际消耗")}
      </h4>
      <div className="mt-2 grid gap-2 sm:grid-cols-4">
        <Metric
          label={t("dashboard.evaluation.trialMetrics.outcome")}
          value={t(`dashboard.evaluation.outcomes.${result.outcome}`, {
            defaultValue: result.outcome,
          })}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.spans")}
          value={String(result.trace.spanCount)}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.closed")}
          value={t(result.trace.closed ? "common.yes" : "common.no")}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.orphans")}
          value={String(result.trace.orphanSpanCount)}
        />
      </div>
      <div className="mt-3 grid gap-2 rounded bg-secondary/20 p-2 sm:grid-cols-3 lg:grid-cols-6">
        <Metric
          label={t("dashboard.evaluation.trialMetrics.wallCritical")}
          value={`${formatDuration(result.timings.wallMs)} / ${formatDuration(result.timings.criticalPathMs)}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.modelToolActive")}
          value={`${formatDuration(result.timings.modelActiveMs)} / ${formatDuration(result.timings.toolActiveMs)}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.queueEnvironment")}
          value={`${formatDuration(result.timings.queueWaitMs)} / ${formatDuration(result.timings.environmentWaitMs)}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.inputOutput")}
          value={`${result.tokens.input ?? "—"} / ${result.tokens.output ?? "—"}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.cacheReasoning")}
          value={`${result.tokens.cacheRead ?? "—"} / ${result.tokens.reasoning ?? "—"}`}
        />
        <Metric
          label={t("dashboard.evaluation.cost")}
          value={result.cost.totalUsd == null ? "—" : `$${result.cost.totalUsd.toFixed(4)}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.toolsAttemptedLogical")}
          value={`${result.tools.attempted} / ${result.tools.logicalCalls}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.toolsEffectiveRetries")}
          value={`${result.tools.effective} / ${result.tools.retries}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.modelCallsRetries")}
          value={`${result.orchestration.modelCalls} / ${result.orchestration.modelRetries}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.loopsFailovers")}
          value={`${result.orchestration.loopIterations} / ${result.orchestration.failovers}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.agentsConcurrency")}
          value={`${result.orchestration.spawnedAgents} / ${result.orchestration.maxConcurrency}`}
        />
        <Metric
          label={t("dashboard.evaluation.trialMetrics.asyncHandoffs")}
          value={`${result.orchestration.asyncJobs} / ${result.orchestration.handoffs}`}
        />
      </div>
      {checks.length > 0 && (
        <div className="mt-3 space-y-1">
          <h4 className="pb-1 font-semibold">
            {t("dashboard.evaluation.verificationResults", "验收结果")}
          </h4>
          {checks.map((check) => (
            <div
              key={check.id}
              className="flex items-start justify-between gap-3 rounded bg-secondary/30 px-2 py-1.5"
            >
              <div>
                <span className="font-medium">{check.id}</span>
                <span className="ml-2 text-muted-foreground">{check.detail}</span>
              </div>
              <span className={check.passed ? "text-emerald-600" : "text-destructive"}>
                {t(
                  check.passed
                    ? "dashboard.evaluation.checkPass"
                    : "dashboard.evaluation.checkFail",
                )}
                {check.blocking ? ` · ${t("dashboard.evaluation.blocking")}` : ""}
              </span>
            </div>
          ))}
        </div>
      )}
      {result.traceEvents.length > 0 && (
        <div className="mt-3 space-y-1">
          <h4 className="pb-1 font-semibold">
            {t("dashboard.evaluation.executionTrace", "执行轨迹")}
          </h4>
          {result.traceEvents.map((event) => (
            <div
              key={event.seq}
              className="grid grid-cols-[50px_1fr_auto] gap-2 rounded bg-secondary/20 px-2 py-1.5"
            >
              <span className="tabular-nums text-muted-foreground">#{event.seq}</span>
              <span>
                {event.event}
                {event.key ? ` · ${event.key}` : ""}
              </span>
              <span>
                {event.status} · {formatDuration(event.durationMs)}
              </span>
            </div>
          ))}
        </div>
      )}
      {(result.error || result.failureClass || result.warnings.length > 0) && (
        <details className="mt-3 rounded bg-secondary/20 p-2 text-muted-foreground">
          <summary className="cursor-pointer font-medium">
            {t("dashboard.evaluation.technicalDetails", "技术诊断信息")}
          </summary>
          <div className="mt-2 break-words font-mono text-[10px] leading-relaxed">
            {[result.failureClass, result.error, ...result.warnings].filter(Boolean).join(" · ")}
          </div>
        </details>
      )}
    </div>
  )
}

function TrialRecordDetail({
  detail,
  caseTitle,
  onClose,
}: {
  detail: EvalTrialDetail
  caseTitle?: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  const record = detail.record
  const tokens = (record.inputTokens ?? 0) + (record.outputTokens ?? 0)
  return (
    <div className="mt-4 rounded-lg bg-background/60 p-3 text-xs">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate font-semibold">{caseTitle ?? record.caseId}</div>
          <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
            {record.caseId} · {record.arm} · {record.id}
          </div>
        </div>
        <Button size="sm" variant="ghost" onClick={onClose}>
          {t("common.close")}
        </Button>
      </div>
      <TrialOutcomeExplanation detail={detail} />
      <div className="mt-3 rounded-lg bg-secondary/20 px-3 py-2 text-muted-foreground">
        {t(
          "dashboard.evaluation.partialDetailNotice",
          "这次运行没有生成完整 evidence，下面显示的是退出前已经持久化的运行摘要。",
        )}
      </div>
      <h4 className="mb-2 mt-4 font-semibold">
        {t("dashboard.evaluation.actualUsage", "实际消耗")}
      </h4>
      <div className="mt-2 grid gap-2 sm:grid-cols-3 lg:grid-cols-6">
        <Metric
          label={t("dashboard.evaluation.trialMetrics.outcome")}
          value={t(`dashboard.evaluation.outcomes.${record.outcome}`, {
            defaultValue: record.outcome,
          })}
        />
        <Metric
          label={t("dashboard.evaluation.duration")}
          value={formatDuration(record.durationMs)}
        />
        <Metric
          label={t("dashboard.evaluation.modelCalls")}
          value={String(record.modelCalls)}
        />
        <Metric
          label={t("dashboard.evaluation.toolCalls")}
          value={String(record.toolCalls)}
        />
        <Metric label={t("dashboard.evaluation.tokens")} value={tokens ? String(tokens) : "—"} />
        <Metric
          label={t("dashboard.evaluation.cost")}
          value={record.costUsd == null ? "—" : `$${record.costUsd.toFixed(4)}`}
        />
      </div>
      {record.failureClass && (
        <div className="mt-3 rounded bg-destructive/5 p-2 text-muted-foreground">
          {record.failureClass}
        </div>
      )}
    </div>
  )
}

function LiveTrialDetail({
  trial,
  caseTitle,
  onClose,
}: {
  trial?: MonitorTrialRow
  caseTitle?: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  if (!trial) return null
  const tokens = (trial.inputTokens ?? 0) + (trial.outputTokens ?? 0)
  const activityKey = trial.lastEvent?.startsWith("model.")
    ? "activityModel"
    : trial.lastEvent?.startsWith("tool.")
      ? "activityTool"
      : trial.lastEvent?.startsWith("goal.")
        ? "activityGoal"
        : trial.lastEvent?.startsWith("workflow.")
          ? "activityWorkflow"
          : trial.lastEvent?.startsWith("agent.") || trial.lastEvent?.startsWith("team.")
            ? "activityAgent"
            : trial.lastEvent?.startsWith("budget.")
              ? "activityBudget"
              : trial.lastEvent
                ? "activityRunning"
                : "activityWaitingModel"
  const activity = t(`dashboard.evaluation.${activityKey}`)
  const attribution = trial.attribution
    ? t(`dashboard.evaluation.attributions.${trial.attribution}`, {
        defaultValue: trial.attribution,
      })
    : "—"
  return (
    <section className="rounded-xl bg-secondary/20 p-4 text-xs">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-semibold">
            {caseTitle ?? trial.caseId ?? trial.trialId}
          </div>
          <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
            {trial.caseId ?? trial.trialId} · {trial.arm ?? "—"} ·{" "}
            {t("dashboard.evaluation.liveScenarioDetail", "场景运行详情")}
          </div>
          <div className="mt-1 truncate text-muted-foreground">
            {activity} · {t("dashboard.evaluation.attribution", "归因完整度")}: {attribution}
          </div>
        </div>
        <Button size="sm" variant="ghost" onClick={onClose}>
          {t("common.close")}
        </Button>
      </div>
      <div className="mt-3 grid gap-2 sm:grid-cols-4 lg:grid-cols-8">
        <Metric
          label={t("dashboard.evaluation.duration")}
          value={trial.durationMs == null ? "—" : formatDuration(trial.durationMs)}
        />
        <Metric
          label={t("dashboard.evaluation.modelCalls")}
          value={String(trial.modelCalls ?? 0)}
        />
        <Metric
          label={t("dashboard.evaluation.toolCalls")}
          value={String(trial.toolCalls ?? 0)}
        />
        <Metric label={t("dashboard.evaluation.tokens")} value={tokens ? String(tokens) : "—"} />
        <Metric
          label={t("dashboard.evaluation.cost")}
          value={trial.costUsd == null ? "—" : `$${trial.costUsd.toFixed(4)}`}
        />
        <Metric
          label={t("dashboard.evaluation.loopIterations", "循环次数")}
          value={String(trial.loopIterations ?? 0)}
        />
        <Metric
          label={t("dashboard.evaluation.spawnedAgents", "已创建 Agent")}
          value={String(trial.spawnedAgents ?? 0)}
        />
        <Metric
          label={t("dashboard.evaluation.asyncJobs", "异步任务")}
          value={String(trial.asyncJobs ?? 0)}
        />
      </div>
      {(trial.activeChildren ?? 0) > 0 && (
        <div className="mt-3 rounded-lg bg-blue-500/10 px-3 py-2 text-blue-600 dark:text-blue-300">
          {t("dashboard.evaluation.activeChildren", {
            count: trial.activeChildren,
            defaultValue: "{{count}} 个子任务仍在运行",
          })}
        </div>
      )}
    </section>
  )
}

function ComparePanel({
  history,
  baselineId,
  candidateId,
  onBaselineChange,
  onCandidateChange,
  comparison,
  onCompare,
  busy,
}: {
  history: EvalExperimentRecord[]
  baselineId: string
  candidateId: string
  onBaselineChange: (value: string) => void
  onCandidateChange: (value: string) => void
  comparison: EvalCompareResult | null
  onCompare: () => void
  busy: boolean
}) {
  const { t } = useTranslation()
  const options = history.filter((item) => item.kind === "hope_core" && item.status === "completed")
  return (
    <section className="space-y-4 rounded-xl bg-secondary/20 p-4">
      <div>
        <h3 className="text-sm font-semibold">{t("dashboard.evaluation.compareTitle")}</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          {t("dashboard.evaluation.compareHint")}
        </p>
      </div>
      <div className="grid gap-3 md:grid-cols-[1fr_1fr_auto]">
        <ExperimentSelect
          label={t("dashboard.evaluation.baseline")}
          value={baselineId}
          options={options}
          onChange={onBaselineChange}
        />
        <ExperimentSelect
          label={t("dashboard.evaluation.candidate")}
          value={candidateId}
          options={options}
          onChange={onCandidateChange}
        />
        <Button
          className="self-end"
          onClick={onCompare}
          disabled={busy || !baselineId || !candidateId}
        >
          {t("dashboard.evaluation.calculateCompare")}
        </Button>
      </div>
      {comparison && (
        <div className="space-y-3">
          {comparison.comparisons.map((group) => (
            <div
              key={`${group.baselineCampaignId}:${group.candidateCampaignId}`}
              className="overflow-hidden rounded-lg bg-background/50"
            >
              <div className="bg-secondary/30 px-3 py-2 text-[10px] text-muted-foreground">
                {group.baselineCampaignId} → {group.candidateCampaignId} ·{" "}
                {group.baselineModelDigest.slice(0, 8)} → {group.candidateModelDigest.slice(0, 8)}
              </div>
              {group.metrics.map((metric) => {
                const comparable =
                  metric.compatibility.compatibility === "exact" ||
                  metric.compatibility.compatibility === "functional"
                const improved =
                  metric.deltaPercent == null
                    ? null
                    : metricDeltaImproved(metric.metric, metric.deltaPercent)
                return (
                  <div
                    key={metric.metric}
                    className="grid grid-cols-[1fr_auto_auto_auto] gap-3 border-b border-border/40 px-3 py-2 text-xs last:border-0"
                  >
                    <div>
                      <div className="font-medium">{metric.metric}</div>
                      <div className="text-[10px] text-muted-foreground">
                        {metric.compatibility.reasons.join(", ") ||
                          t("dashboard.evaluation.fullyComparable")}
                      </div>
                    </div>
                    <CompatibilityBadge value={metric.compatibility.compatibility} />
                    <div className="text-right tabular-nums">
                      {formatMetric(metric.baselineValue)} → {formatMetric(metric.candidateValue)}
                    </div>
                    <div
                      className={cn(
                        "w-20 text-right tabular-nums",
                        comparable &&
                          improved != null &&
                          (improved ? "text-emerald-600" : "text-amber-600"),
                      )}
                    >
                      {comparable && metric.deltaPercent != null
                        ? `${metric.deltaPercent >= 0 ? "+" : ""}${metric.deltaPercent.toFixed(1)}%`
                        : t("dashboard.evaluation.diagnosticOnly")}
                    </div>
                  </div>
                )
              })}
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

function TrendsPanel({
  history,
  baselineId,
  metric,
  trends,
  onBaselineChange,
  onMetricChange,
  onLoad,
  busy,
}: {
  history: EvalExperimentRecord[]
  baselineId: string
  metric: EvalTrendMetric
  trends: EvalTrendPoint[]
  onBaselineChange: (value: string) => void
  onMetricChange: (value: EvalTrendMetric) => void
  onLoad: () => void
  busy: boolean
}) {
  const { t } = useTranslation()
  const options = history.filter((item) => item.kind === "hope_core" && item.status === "completed")
  return (
    <section className="space-y-4 rounded-xl bg-secondary/20 p-4">
      <div className="grid gap-3 md:grid-cols-[1fr_220px_auto]">
        <ExperimentSelect
          label={t("dashboard.evaluation.trendAnchor")}
          value={baselineId}
          options={options}
          onChange={onBaselineChange}
        />
        <label className="space-y-1 text-xs text-muted-foreground">
          <span>{t("dashboard.evaluation.comparableMetric")}</span>
          <Select
            value={metric}
            onValueChange={(value) => onMetricChange(value as EvalTrendMetric)}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(
                [
                  "task_success",
                  "end_to_end_yield",
                  "any_pass_at_k",
                  "all_pass_at_k",
                  "infra_error",
                  "policy_failure",
                  "budget_exhausted",
                  "false_completion",
                  "wall_time",
                  "tool_calls",
                  "tokens",
                  "usd_cost",
                  "multi_agent_uplift",
                ] as EvalTrendMetric[]
              ).map((value) => (
                <SelectItem key={value} value={value}>
                  {t(`dashboard.evaluation.trendMetrics.${value}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
        <Button className="self-end" onClick={onLoad} disabled={busy || !baselineId}>
          {t("dashboard.evaluation.loadTrends")}
        </Button>
      </div>
      {trends.length === 0 ? (
        <div className="py-8 text-center text-sm text-muted-foreground">
          {t("dashboard.evaluation.trendsEmpty")}
        </div>
      ) : (
        <div className="space-y-2">
          {trends.map((point) => (
            <div
              key={`${point.experimentId}:${point.campaignId}`}
              className="grid gap-2 rounded-lg bg-background/50 p-3 text-xs md:grid-cols-[160px_1fr_auto_auto] md:items-center"
            >
              <div>
                <div className="font-medium">
                  {new Date(point.completedAt).toLocaleDateString()}
                </div>
                <div className="text-[10px] text-muted-foreground">
                  {point.reference.slice(0, 8)} · {point.modelDigest.slice(0, 8)}
                </div>
              </div>
              {isTrendRateMetric(point.metric) ? (
                <div className="h-2 overflow-hidden rounded-full bg-secondary">
                  <div
                    className="h-full bg-primary"
                    style={{
                      width: `${Math.max(0, Math.min(100, (point.metricValue ?? 0) * 100))}%`,
                    }}
                  />
                </div>
              ) : (
                <div className="text-[10px] text-muted-foreground">
                  {t("dashboard.evaluation.success")} {(point.successRate * 100).toFixed(1)}% ·{" "}
                  {t("dashboard.evaluation.trendMetrics.end_to_end_yield")}{" "}
                  {(point.endToEndYield * 100).toFixed(1)}% · Infra{" "}
                  {(point.infraErrorRate * 100).toFixed(1)}%
                </div>
              )}
              <div className="text-right tabular-nums">
                <div className="font-medium">
                  {t(`dashboard.evaluation.trendMetrics.${point.metric}`)}{" "}
                  {formatTrendValue(point.metric, point.metricValue)}
                </div>
                <div className="text-[10px] text-muted-foreground">
                  {t("dashboard.evaluation.trendMetrics.policy_failure")}{" "}
                  {(point.policyFailureRate * 100).toFixed(1)}% ·{" "}
                  {t("dashboard.evaluation.trendMetrics.false_completion")}{" "}
                  {(point.falseCompletionRate * 100).toFixed(1)}%
                </div>
              </div>
              <CompatibilityBadge value={point.compatibility.compatibility} />
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

function BaselinesPanel({
  baselines,
  history,
  onImport,
  onImportUnverified,
  onDelete,
  busy,
  importAvailable,
  importIssues,
}: {
  baselines: EvalBaselineRecord[]
  history: EvalExperimentRecord[]
  onImport: () => void
  onImportUnverified: () => void
  onDelete: (id: string) => void
  busy: boolean
  importAvailable: boolean
  importIssues: string[]
}) {
  const { t } = useTranslation()
  const byId = new Map(history.map((item) => [item.id, item]))
  return (
    <section className="space-y-4 rounded-xl bg-secondary/20 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">{t("dashboard.evaluation.baselineTitle")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("dashboard.evaluation.baselineHint")}
          </p>
          {!importAvailable && (
            <p className="mt-2 text-xs text-amber-600">
              {t("dashboard.evaluation.importUnavailable")}:{" "}
              {importIssues.join("; ") || t("dashboard.evaluation.trustRegistryMissing")}
            </p>
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="ghost" onClick={onImportUnverified} disabled={busy}>
            {t("dashboard.evaluation.importUnverifiedJson")}
          </Button>
          <Button variant="secondary" onClick={onImport} disabled={busy || !importAvailable}>
            <Upload className="mr-2 h-4 w-4" />
            {t("dashboard.evaluation.importSignedBundle")}
          </Button>
        </div>
      </div>
      {baselines.length === 0 ? (
        <div className="py-8 text-center text-sm text-muted-foreground">
          {t("dashboard.evaluation.baselinesEmpty")}
        </div>
      ) : (
        baselines.map((baseline) => {
          const experiment = byId.get(baseline.experimentId)
          return (
            <div
              key={baseline.id}
              className="flex items-center justify-between gap-3 rounded-lg bg-background/50 p-3 text-xs"
            >
              <div>
                <div className="font-medium">
                  {baseline.tier} · {experiment?.reference.slice(0, 8) ?? baseline.experimentId}
                </div>
                <div className="text-muted-foreground">
                  {new Date(baseline.approvedAt).toLocaleString()} · {baseline.approvedBy} ·{" "}
                  {baseline.note ?? t("dashboard.evaluation.noNote")}
                </div>
              </div>
              <Button size="sm" variant="ghost" onClick={() => onDelete(baseline.id)}>
                {t("dashboard.evaluation.removeAnchor")}
              </Button>
            </div>
          )
        })
      )}
    </section>
  )
}

function ExperimentSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: string
  options: EvalExperimentRecord[]
  onChange: (value: string) => void
}) {
  const { t } = useTranslation()
  return (
    <label className="space-y-1 text-xs text-muted-foreground">
      <span>{label}</span>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue placeholder={t("dashboard.evaluation.chooseExperiment")} />
        </SelectTrigger>
        <SelectContent>
          {options.map((item) => (
            <SelectItem key={item.id} value={item.id}>
              {item.profileId} · {item.reference.slice(0, 8)} · {item.integrity}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  )
}

function CompatibilityBadge({ value }: { value: string }) {
  return (
    <span
      className={cn(
        "w-fit rounded-full px-2 py-0.5 text-[10px]",
        value === "exact"
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : value === "functional"
            ? "bg-blue-500/10 text-blue-700 dark:text-blue-300"
            : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
      )}
    >
      {value}
    </span>
  )
}

function MetricsOverview({ history }: { history: EvalExperimentRecord[] }) {
  const { t } = useTranslation()
  const complete = history.filter((row) => row.status === "completed")
  const totals = complete.reduce(
    (acc, row) => ({
      total: acc.total + row.totalTrials,
      infra: acc.infra + row.infraErrorTrials,
      cost: acc.cost + (row.observedCostUsd ?? 0),
    }),
    { total: 0, infra: 0, cost: 0 },
  )
  return (
    <div className="grid gap-3 md:grid-cols-4">
      <MetricCard
        icon={ShieldCheck}
        label={t("dashboard.evaluation.totalTrials")}
        value={String(totals.total)}
      />
      <MetricCard
        icon={AlertTriangle}
        label={t("dashboard.evaluation.infraErrors")}
        value={String(totals.infra)}
      />
      <MetricCard
        icon={Clock3}
        label={t("dashboard.evaluation.completedCampaigns")}
        value={String(complete.length)}
      />
      <MetricCard
        icon={FlaskConical}
        label={t("dashboard.evaluation.knownCostTotal")}
        value={`$${totals.cost.toFixed(2)}`}
      />
    </div>
  )
}

function MetricCard({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Clock3
  label: string
  value: string
}) {
  return (
    <div className="rounded-xl bg-secondary/20 p-4">
      <Icon className="h-4 w-4 text-primary" />
      <div className="mt-3 text-2xl font-semibold">{value}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  )
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="font-medium">{value}</div>
    </div>
  )
}

function IntegrityBadge({ integrity }: { integrity: EvalExperimentRecord["integrity"] }) {
  const { t } = useTranslation()
  const labels: Record<EvalExperimentRecord["integrity"], string> = {
    protected_verified: t("dashboard.evaluation.integrity.protectedVerified"),
    protected_unknown_assets: t("dashboard.evaluation.integrity.protectedUnknownAssets"),
    local_diagnostic: t("dashboard.evaluation.integrity.localDiagnostic"),
    unverified_import: t("dashboard.evaluation.integrity.unverifiedImport"),
    legacy_local: t("dashboard.evaluation.integrity.legacyLocal"),
  }
  return (
    <span
      className={cn(
        "rounded-full px-2 py-0.5 text-[10px]",
        integrity === "protected_verified"
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : integrity === "unverified_import"
            ? "bg-destructive/10 text-destructive"
            : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
      )}
    >
      {labels[integrity]}
    </span>
  )
}

function SignatureBadge({ status }: { status: string }) {
  const { t } = useTranslation()
  const labels: Record<string, string> = {
    verified: t("dashboard.evaluation.signature.verified"),
    verified_retired: t("dashboard.evaluation.signature.retired"),
    verified_now_revoked: t("dashboard.evaluation.signature.nowRevoked"),
    verified_key_missing: t("dashboard.evaluation.signature.keyMissing"),
    unsigned: t("dashboard.evaluation.signature.unsigned"),
  }
  const trusted = status === "verified" || status === "verified_retired"
  return (
    <span
      className={cn(
        "rounded-full px-2 py-0.5 text-[10px]",
        trusted
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : "bg-destructive/10 text-destructive",
      )}
    >
      {labels[status] ?? status}
    </span>
  )
}

function StatusBadge({ status }: { status: EvalExperimentRecord["status"] }) {
  const { t } = useTranslation()
  return (
    <span className="inline-flex min-h-5 items-center justify-center whitespace-nowrap rounded-full bg-secondary px-2 py-0.5 text-[10px] text-muted-foreground">
      {t(`dashboard.evaluation.statuses.${status}`, { defaultValue: status })}
    </span>
  )
}

function modelKey(model: EvalModelOption) {
  return `${model.providerId}::${model.modelId}`
}
function trialProgressKey(campaignId: string, trialId: string) {
  return `${campaignId}::${trialId}`
}
function formatDuration(ms: number) {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`
}
function formatLongDuration(ms: number) {
  if (ms < 60_000) return formatDuration(ms)
  const totalSeconds = Math.floor(ms / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  return hours > 0
    ? `${hours}h ${String(minutes).padStart(2, "0")}m ${String(seconds).padStart(2, "0")}s`
    : `${minutes}m ${String(seconds).padStart(2, "0")}s`
}
function formatPlanTrialTimeouts(plan: EvalAppPlan) {
  const values = plan.campaigns.flatMap((campaign) =>
    campaign.resolvedPlan.suites.flatMap((suite) =>
      suite.cases.map((plannedCase) => plannedCase.timeoutSeconds),
    ),
  )
  if (values.length === 0) return "—"
  const minimum = Math.min(...values)
  const maximum = Math.max(...values)
  const minimumLabel = formatLongDuration(minimum * 1_000)
  return minimum === maximum
    ? minimumLabel
    : `${minimumLabel} – ${formatLongDuration(maximum * 1_000)}`
}
function formatMetric(value?: number) {
  return value == null ? "—" : Math.abs(value) < 10 ? value.toFixed(3) : value.toFixed(1)
}
function isTrendRateMetric(metric: EvalTrendMetric) {
  return [
    "task_success",
    "end_to_end_yield",
    "any_pass_at_k",
    "all_pass_at_k",
    "infra_error",
    "policy_failure",
    "budget_exhausted",
    "false_completion",
  ].includes(metric)
}
function formatTrendValue(metric: EvalTrendMetric, value?: number) {
  if (value == null) return "—"
  if (isTrendRateMetric(metric)) return `${(value * 100).toFixed(1)}%`
  if (metric === "wall_time") return formatDuration(value)
  if (metric === "usd_cost") return `$${value.toFixed(4)}`
  if (metric === "multi_agent_uplift") return `${value >= 0 ? "+" : ""}${value.toFixed(1)}pp`
  return value.toFixed(1)
}
function metricDeltaImproved(metric: EvalCompatibilityMetric, deltaPercent: number) {
  if (deltaPercent === 0) return true
  return metric === "functional" || metric === "multi_agent" ? deltaPercent > 0 : deltaPercent < 0
}
