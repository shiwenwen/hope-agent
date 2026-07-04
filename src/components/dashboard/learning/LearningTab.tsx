import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Activity,
  Archive,
  CheckCircle2,
  Copy,
  FileCheck2,
  FileText,
  GitBranch,
  Layers3,
  Loader2,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Sparkles,
  Upload,
  XCircle,
} from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  CodingBenchmarkCampaign,
  CodingBenchmarkCenterReport,
  CodingBenchmarkCorpusHealthReport,
  CodingBenchmarkLeaderboardReport,
  CodingBenchmarkReport,
  CodingBenchmarkBacklogItem,
  CodingBenchmarkBacklogMaterializeResult,
  CodingBenchmarkTaskPack,
  CodingBenchmarkTaskPackManifest,
  CodingBenchmarkTaskPackValidationReport,
  CodingContinuousBenchmarkGateReport,
  CodingEvalReleaseGateReport,
  GenerateCodingImprovementProposalsResult,
  CodingLearningGeneralizationReport,
  DomainConnectorE2EGateReport,
  DomainEvalRunRecord,
  DomainEvalFixtureRunRecord,
  DomainEvalCampaign,
  DomainEvalCampaignLeaderboardReport,
  DomainEvalTask,
  DomainOperationalGateReport,
  DomainQualityGateReport,
  DomainReadinessGateReport,
  DomainSoakReport,
  DomainEvalCalibrationRecord,
} from "@/lib/transport"
import type { CodingImprovementDashboard, DashboardFilter, DomainQualityDashboard } from "../types"

interface LearningOverview {
  windowDays: number
  autoCreatedSkills: number
  userCreatedSkills: number
  skillsActivated: number
  skillsPatched: number
  skillsDiscarded: number
  skillsUsed: number
  recallHits: number
  recallSummaryUsed: number
  profileMemories: number
}

interface TimelinePoint {
  ts: number
  kind: string
  skillId?: string
  source?: string
}

interface SkillUsage {
  skillId: string
  usedCount: number
  lastUsedTs?: number
  createdSource?: string
}

interface RecallStats {
  hits: number
  summarized: number
  windowDays: number
}

interface BenchmarkProviderOption {
  id: string
  name: string
  enabled?: boolean
  models: { id: string; name: string }[]
}

interface BenchmarkModelOption {
  key: string
  providerId: string
  providerName: string
  modelId: string
  modelName: string
}

const WINDOW_OPTIONS = [7, 14, 30, 60, 90]
const DAY_MS = 24 * 60 * 60 * 1000

interface LearningTabProps {
  filter: DashboardFilter
}

function releaseGateWindowDays(filter: DashboardFilter, fallbackDays: number): number {
  if (!filter.startDate) return fallbackDays
  const start = Date.parse(filter.startDate)
  if (!Number.isFinite(start)) return fallbackDays
  return Math.max(1, Math.min(180, Math.ceil((Date.now() - start) / DAY_MS)))
}

export default function LearningTab({ filter }: LearningTabProps) {
  const { t } = useTranslation()
  const [windowDays, setWindowDays] = useState(30)
  const [loading, setLoading] = useState(false)
  const [overview, setOverview] = useState<LearningOverview | null>(null)
  const [timeline, setTimeline] = useState<TimelinePoint[]>([])
  const [topSkills, setTopSkills] = useState<SkillUsage[]>([])
  const [recall, setRecall] = useState<RecallStats | null>(null)
  const [coding, setCoding] = useState<CodingImprovementDashboard | null>(null)
  const [benchmark, setBenchmark] = useState<CodingBenchmarkCenterReport | null>(null)
  const [benchmarkCampaigns, setBenchmarkCampaigns] = useState<CodingBenchmarkCampaign[]>([])
  const [benchmarkLeaderboard, setBenchmarkLeaderboard] =
    useState<CodingBenchmarkLeaderboardReport | null>(null)
  const [benchmarkTaskPacks, setBenchmarkTaskPacks] = useState<CodingBenchmarkTaskPack[]>([])
  const [benchmarkCorpusHealth, setBenchmarkCorpusHealth] =
    useState<CodingBenchmarkCorpusHealthReport | null>(null)
  const [benchmarkReports, setBenchmarkReports] = useState<CodingBenchmarkReport[]>([])
  const [continuousGate, setContinuousGate] =
    useState<CodingContinuousBenchmarkGateReport | null>(null)
  const [benchmarkBacklog, setBenchmarkBacklog] = useState<CodingBenchmarkBacklogItem[]>([])
  const [benchmarkProviders, setBenchmarkProviders] = useState<BenchmarkProviderOption[]>([])
  const [selectedBenchmarkModels, setSelectedBenchmarkModels] = useState<string[]>([])
  const [benchmarkMaxTasks, setBenchmarkMaxTasks] = useState(3)
  const [benchmarkBudgetUsd, setBenchmarkBudgetUsd] = useState("")
  const [releaseGate, setReleaseGate] = useState<CodingEvalReleaseGateReport | null>(null)
  const [generalization, setGeneralization] =
    useState<CodingLearningGeneralizationReport | null>(null)
  const [domainQualityGate, setDomainQualityGate] =
    useState<DomainQualityGateReport | null>(null)
  const [domainReadinessGate, setDomainReadinessGate] =
    useState<DomainReadinessGateReport | null>(null)
  const [domainOperationalGate, setDomainOperationalGate] =
    useState<DomainOperationalGateReport | null>(null)
  const [domainSoakReport, setDomainSoakReport] = useState<DomainSoakReport | null>(null)
  const [domainConnectorE2EGate, setDomainConnectorE2EGate] =
    useState<DomainConnectorE2EGateReport | null>(null)
  const [domainEvalRuns, setDomainEvalRuns] = useState<DomainEvalRunRecord[]>([])
  const [domainFixtureRuns, setDomainFixtureRuns] = useState<DomainEvalFixtureRunRecord[]>([])
  const [domainEvalCampaigns, setDomainEvalCampaigns] = useState<DomainEvalCampaign[]>([])
  const [domainCampaignLeaderboard, setDomainCampaignLeaderboard] =
    useState<DomainEvalCampaignLeaderboardReport | null>(null)
  const [domainEvalTasks, setDomainEvalTasks] = useState<DomainEvalTask[]>([])
  const [selectedDomainModels, setSelectedDomainModels] = useState<string[]>([])
  const [domainCampaignMaxTasks, setDomainCampaignMaxTasks] = useState(3)
  const [domainCampaignBudgetUsd, setDomainCampaignBudgetUsd] = useState("")
  const [benchmarkRunning, setBenchmarkRunning] = useState(false)
  const [benchmarkError, setBenchmarkError] = useState<string | null>(null)
  const [domainCampaignError, setDomainCampaignError] = useState<string | null>(null)
  const [campaignActionId, setCampaignActionId] = useState<string | null>(null)
  const [domainCampaignActionId, setDomainCampaignActionId] = useState<string | null>(null)
  const [corpusActionId, setCorpusActionId] = useState<string | null>(null)
  const [reportActionId, setReportActionId] = useState<string | null>(null)
  const [gateActionId, setGateActionId] = useState<string | null>(null)
  const [domainCalibrationActionId, setDomainCalibrationActionId] = useState<string | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    setBenchmarkError(null)
    setDomainCampaignError(null)
    try {
      const [
        ov,
        tl,
        ts,
        rs,
        ci,
        bc,
        campaigns,
        leaderboard,
        taskPacks,
        corpusHealth,
        reports,
        gate,
        backlog,
        providers,
        rg,
        gen,
        drg,
        dog,
        dsr,
        dceg,
        dqg,
        der,
        dfr,
        dec,
        decl,
        det,
      ] = await Promise.all([
        getTransport().call<LearningOverview>("dashboard_learning_overview", {
          windowDays,
        }),
        getTransport().call<TimelinePoint[]>("dashboard_learning_timeline", {
          windowDays,
        }),
        getTransport().call<SkillUsage[]>("dashboard_top_skills", {
          windowDays,
          limit: 10,
        }),
        getTransport().call<RecallStats>("dashboard_recall_stats", {
          windowDays,
        }),
        getTransport().call<CodingImprovementDashboard>("dashboard_coding_improvement", {
          filter,
          limit: 8,
        }),
        getTransport().call<CodingBenchmarkCenterReport>("get_coding_benchmark_center", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 12,
          },
        }),
        getTransport().call<CodingBenchmarkCampaign[]>("list_coding_benchmark_campaigns", {
          input: {
            limit: 6,
          },
        }),
        getTransport().call<CodingBenchmarkLeaderboardReport>("get_benchmark_leaderboard", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 6,
            minItems: 1,
          },
        }),
        getTransport().call<CodingBenchmarkTaskPack[]>("list_benchmark_task_packs", {
          input: {
            limit: 8,
          },
        }),
        getTransport().call<CodingBenchmarkCorpusHealthReport>("get_benchmark_corpus_health", {
          input: {},
        }),
        getTransport().call<CodingBenchmarkReport[]>("list_benchmark_reports", {
          input: {
            limit: 6,
          },
        }),
        getTransport().call<CodingContinuousBenchmarkGateReport>("evaluate_continuous_benchmark_gate", {
          input: {
            triggerKind: "manual",
            windowDays: releaseGateWindowDays(filter, windowDays),
            maxEvidenceAgeDays: 14,
            requireReleaseReportEvidence: true,
            requireRecentCampaign: true,
            minCampaignItems: 1,
            minCasePassRate: 1,
            maxOpenBacklogItems: 0,
          },
        }),
        getTransport().call<CodingBenchmarkBacklogItem[]>("list_benchmark_backlog", {
          input: {
            status: "open",
            limit: 6,
          },
        }),
        getTransport()
          .call<BenchmarkProviderOption[]>("get_providers")
          .catch((error) => {
            logger.warn(
              "dashboard",
              "LearningTab::loadProviders",
              "Failed to load benchmark providers",
              error,
            )
            return []
          }),
        getTransport().call<CodingEvalReleaseGateReport>("evaluate_coding_eval_release_gate", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
          },
        }),
        getTransport().call<CodingLearningGeneralizationReport>(
          "evaluate_coding_learning_generalization",
          {
            input: {
              windowDays: releaseGateWindowDays(filter, windowDays),
            },
          },
        ),
        getTransport().call<DomainReadinessGateReport>("evaluate_domain_readiness_gate", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            minEvalRuns: 1,
            minQualityRuns: 1,
            minDomainCoverage: 1,
            minCampaignItems: 1,
            minLeaderboardRows: 1,
            maxFailedCampaignItems: 0,
            maxOpenLearningProposals: 0,
            requireApprovalSafety: true,
          },
        }),
        getTransport().call<DomainOperationalGateReport>("evaluate_domain_operational_gate", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            minWorkflowRuns: 1,
            maxFailedWorkflowRuns: 0,
            maxBlockedWorkflowRuns: 0,
            maxCancelledWorkflowRuns: 0,
            maxActiveWorkflowRuns: 0,
            minLoopRuns: 0,
            maxFailedLoopRuns: 0,
            maxActiveCampaigns: 0,
            maxFailedCampaignItems: 0,
          },
        }),
        getTransport().call<DomainSoakReport>("generate_domain_soak_report", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            maxItems: 12,
          },
        }),
        getTransport().call<DomainConnectorE2EGateReport>("evaluate_domain_connector_e2e_gate", {
          input: {
            requireConnectorInput: true,
            requireDraft: true,
            requireExplicitApproval: true,
            requireExecutionResult: true,
            requirePostActionVerification: true,
            requireRollbackPlan: true,
            requireExportGuardForDelivery: true,
          },
        }),
        getTransport().call<DomainQualityGateReport>("evaluate_domain_quality_gate", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            minEvalRuns: 1,
            minQualityRuns: 1,
            minDomainCoverage: 1,
            requireApprovalSafety: true,
          },
        }),
        getTransport().call<DomainEvalRunRecord[]>("list_domain_eval_runs", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 6,
          },
        }),
        getTransport().call<DomainEvalFixtureRunRecord[]>("list_domain_eval_fixture_runs", {
          input: {
            sourceType: "fixture",
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 6,
          },
        }),
        getTransport().call<DomainEvalCampaign[]>("list_domain_eval_campaigns", {
          input: {
            limit: 6,
          },
        }),
        getTransport().call<DomainEvalCampaignLeaderboardReport>("get_domain_eval_campaign_leaderboard", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 6,
          },
        }),
        getTransport().call<DomainEvalTask[]>("list_domain_eval_tasks", {
          input: {
            limit: 20,
          },
        }),
      ])
      setOverview(ov)
      setTimeline(tl ?? [])
      setTopSkills(ts ?? [])
      setRecall(rs)
      setCoding(ci)
      setBenchmark(bc)
      setBenchmarkCampaigns(campaigns ?? [])
      setBenchmarkLeaderboard(leaderboard)
      setBenchmarkTaskPacks(taskPacks ?? [])
      setBenchmarkCorpusHealth(corpusHealth)
      setBenchmarkReports(reports ?? [])
      setContinuousGate(gate)
      setBenchmarkBacklog(backlog ?? [])
      setBenchmarkProviders(providers ?? [])
      setReleaseGate(rg)
      setGeneralization(gen)
      setDomainReadinessGate(drg)
      setDomainOperationalGate(dog)
      setDomainSoakReport(dsr)
      setDomainConnectorE2EGate(dceg)
      setDomainQualityGate(dqg)
      setDomainEvalRuns(der ?? [])
      setDomainFixtureRuns(dfr ?? [])
      setDomainEvalCampaigns(dec ?? [])
      setDomainCampaignLeaderboard(decl)
      setDomainEvalTasks(det ?? [])
    } catch (e) {
      logger.error("dashboard", "LearningTab::load", "Failed to load learning data", e)
    } finally {
      setLoading(false)
    }
  }, [filter, windowDays])

  const runBenchmark = useCallback(async () => {
    setBenchmarkRunning(true)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkCampaign>("create_coding_benchmark_campaign", {
        input: {
          name: "Dashboard deterministic benchmark",
          runNow: true,
          goldTaskInput: {
            executionMode: "fixture_patch",
            baselineKind: "deterministic_mock",
            label: "Benchmark Campaign deterministic run",
            sourceType: "benchmark_campaign",
            sourceId: "dashboard",
            recordEvalRuns: true,
            recordPackRun: true,
            evaluateGoal: true,
          },
          models: [],
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::runBenchmark", "Failed to run benchmark pack", e)
    } finally {
      setBenchmarkRunning(false)
    }
  }, [reload])

  const benchmarkModelOptions = benchmarkProviders
    .filter((provider) => provider.enabled !== false)
    .flatMap((provider) =>
      provider.models.map((model) => ({
        key: `${provider.id}::${model.id}`,
        providerId: provider.id,
        providerName: provider.name,
        modelId: model.id,
        modelName: model.name,
      })),
    )

  const toggleBenchmarkModel = useCallback((key: string) => {
    setSelectedBenchmarkModels((current) =>
      current.includes(key)
        ? current.filter((item) => item !== key)
        : current.length >= 4
          ? [...current.slice(1), key]
          : [...current, key],
    )
  }, [])

  const toggleDomainModel = useCallback((key: string) => {
    setSelectedDomainModels((current) =>
      current.includes(key)
        ? current.filter((item) => item !== key)
        : current.length >= 4
          ? [...current.slice(1), key]
          : [...current, key],
    )
  }, [])

  const runExternalBenchmark = useCallback(async () => {
    const selected = benchmarkModelOptions.filter((option) =>
      selectedBenchmarkModels.includes(option.key),
    )
    if (!selected.length) {
      setBenchmarkError("Select at least one external model.")
      return
    }
    const providerIds = new Set(selected.map((option) => option.providerId))
    const providers = benchmarkProviders.filter((provider) => providerIds.has(provider.id))
    const parsedBudget = Number(benchmarkBudgetUsd)
    setBenchmarkRunning(true)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkCampaign>("create_coding_benchmark_campaign", {
        input: {
          name: "External model benchmark campaign",
          runNow: true,
          maxBudgetUsd:
            benchmarkBudgetUsd.trim() && Number.isFinite(parsedBudget) && parsedBudget > 0
              ? parsedBudget
              : null,
          goldTaskInput: {
            executionMode: "agent",
            baselineKind: "external_model",
            label: "External model benchmark campaign",
            sourceType: "benchmark_campaign",
            sourceId: "dashboard-external",
            recordEvalRuns: true,
            recordPackRun: true,
            evaluateGoal: true,
            autoApproveTools: true,
            maxTasks: Math.max(1, Math.min(20, benchmarkMaxTasks)),
            providers,
          },
          models: selected.map((option) => ({
            providerId: option.providerId,
            modelId: option.modelId,
            label: `${option.providerName}/${option.modelName}`,
          })),
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::runExternalBenchmark", "Failed to run external benchmark", e)
    } finally {
      setBenchmarkRunning(false)
    }
  }, [
    benchmarkBudgetUsd,
    benchmarkMaxTasks,
    benchmarkModelOptions,
    benchmarkProviders,
    reload,
    selectedBenchmarkModels,
  ])

  const cancelBenchmarkCampaign = useCallback(async (campaignId: string) => {
    setCampaignActionId(campaignId)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkCampaign | null>("cancel_coding_benchmark_campaign", {
        campaignId,
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::cancelBenchmarkCampaign", "Failed to cancel campaign", e)
    } finally {
      setCampaignActionId(null)
    }
  }, [reload])

  const retryBenchmarkCampaign = useCallback(async (campaignId: string) => {
    setCampaignActionId(campaignId)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkCampaign | null>("run_coding_benchmark_campaign", {
        input: {
          campaignId,
          retryFailedOnly: true,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::retryBenchmarkCampaign", "Failed to retry campaign", e)
    } finally {
      setCampaignActionId(null)
    }
  }, [reload])

  const runDomainEvalCampaign = useCallback(async () => {
    setDomainCampaignActionId("new")
    setDomainCampaignError(null)
    try {
      await getTransport().call<DomainEvalCampaign>("create_domain_eval_campaign", {
        input: {
          name: "Dashboard domain trace campaign",
          executionMode: "trace_fixture",
          maxTasks: 5,
          runNow: true,
          models: [],
        },
      })
      await reload()
    } catch (e) {
      setDomainCampaignError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::runDomainEvalCampaign", "Failed to run domain eval campaign", e)
    } finally {
      setDomainCampaignActionId(null)
    }
  }, [reload])

  const runExternalDomainEvalCampaign = useCallback(async () => {
    const selected = benchmarkModelOptions.filter((option) =>
      selectedDomainModels.includes(option.key),
    )
    if (!selected.length) {
      setDomainCampaignError("Select at least one external model.")
      return
    }
    const providerIds = new Set(selected.map((option) => option.providerId))
    const providers = benchmarkProviders.filter((provider) => providerIds.has(provider.id))
    const parsedBudget = Number(domainCampaignBudgetUsd)
    setDomainCampaignActionId("external")
    setDomainCampaignError(null)
    try {
      await getTransport().call<DomainEvalCampaign>("create_domain_eval_campaign", {
        input: {
          name: "External domain eval campaign",
          executionMode: "agent",
          maxTasks: Math.max(1, Math.min(15, domainCampaignMaxTasks)),
          runNow: true,
          maxBudgetUsd:
            domainCampaignBudgetUsd.trim() && Number.isFinite(parsedBudget) && parsedBudget > 0
              ? parsedBudget
              : null,
          providers,
          models: selected.map((option) => ({
            providerId: option.providerId,
            modelId: option.modelId,
            label: `${option.providerName}/${option.modelName}`,
          })),
        },
      })
      await reload()
    } catch (e) {
      setDomainCampaignError(e instanceof Error ? e.message : String(e))
      logger.error(
        "dashboard",
        "LearningTab::runExternalDomainEvalCampaign",
        "Failed to run external domain campaign",
        e,
      )
    } finally {
      setDomainCampaignActionId(null)
    }
  }, [
    benchmarkModelOptions,
    benchmarkProviders,
    domainCampaignBudgetUsd,
    domainCampaignMaxTasks,
    reload,
    selectedDomainModels,
  ])

  const cancelDomainEvalCampaign = useCallback(async (campaignId: string) => {
    setDomainCampaignActionId(campaignId)
    setDomainCampaignError(null)
    try {
      await getTransport().call<DomainEvalCampaign | null>("cancel_domain_eval_campaign", {
        campaignId,
      })
      await reload()
    } catch (e) {
      setDomainCampaignError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::cancelDomainEvalCampaign", "Failed to cancel domain campaign", e)
    } finally {
      setDomainCampaignActionId(null)
    }
  }, [reload])

  const retryDomainEvalCampaign = useCallback(async (campaignId: string) => {
    setDomainCampaignActionId(campaignId)
    setDomainCampaignError(null)
    try {
      await getTransport().call<DomainEvalCampaign | null>("run_domain_eval_campaign", {
        input: {
          campaignId,
          retryFailedOnly: true,
        },
      })
      await reload()
    } catch (e) {
      setDomainCampaignError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::retryDomainEvalCampaign", "Failed to retry domain campaign", e)
    } finally {
      setDomainCampaignActionId(null)
    }
  }, [reload])

  const generateDomainCampaignLearning = useCallback(async (campaign: DomainEvalCampaign) => {
    if (!campaign.sessionId) {
      setDomainCampaignError("This domain campaign has no session scope for learning proposals.")
      return
    }
    const campaignId = campaign.id
    const actionKey = `learn:${campaignId}`
    setDomainCampaignActionId(actionKey)
    setDomainCampaignError(null)
    try {
      await getTransport().call<GenerateCodingImprovementProposalsResult>(
        "generate_coding_improvement_proposals",
        {
          sessionId: campaign.sessionId,
          windowDays: releaseGateWindowDays(filter, windowDays),
          sourceType: "domain_eval_campaign",
          sourceId: campaignId,
          proposalKinds: ["domain_eval_case", "domain_guidance"],
        },
      )
      await reload()
    } catch (e) {
      setDomainCampaignError(e instanceof Error ? e.message : String(e))
      logger.error(
        "dashboard",
        "LearningTab::generateDomainCampaignLearning",
        "Failed to generate domain campaign learning proposals",
        e,
      )
    } finally {
      setDomainCampaignActionId(null)
    }
  }, [filter, reload, windowDays])

  const importSampleTaskPack = useCallback(async () => {
    setCorpusActionId("import")
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkTaskPack>("import_benchmark_task_pack", {
        input: {
          manifest: sampleBenchmarkTaskPackManifest(),
          explicitImportConsent: true,
          importedFrom: "dashboard_sample_manifest",
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::importSampleTaskPack", "Failed to import corpus pack", e)
    } finally {
      setCorpusActionId(null)
    }
  }, [reload])

  const updateTaskPackStatus = useCallback(async (pack: CodingBenchmarkTaskPack, status: string) => {
    const actionKey = `${pack.packId}@${pack.version}:${status}`
    setCorpusActionId(actionKey)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkTaskPack>("update_benchmark_task_pack_status", {
        input: {
          packId: pack.packId,
          version: pack.version,
          status,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::updateTaskPackStatus", "Failed to update task pack", e)
    } finally {
      setCorpusActionId(null)
    }
  }, [reload])

  const validateTaskPack = useCallback(async (pack: CodingBenchmarkTaskPack) => {
    const actionKey = `${pack.packId}@${pack.version}:validate`
    setCorpusActionId(actionKey)
    setBenchmarkError(null)
    try {
      const report = await getTransport().call<CodingBenchmarkTaskPackValidationReport>(
        "validate_benchmark_task_pack",
        {
          input: {
            packId: pack.packId,
            version: pack.version,
          },
        },
      )
      if (report.status !== "passed") {
        const failed = report.checks.find((check) => check.status !== "passed")
        setBenchmarkError(failed ? `${failed.name}: ${failed.actual}` : "Task pack validation failed.")
      }
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::validateTaskPack", "Failed to validate task pack", e)
    } finally {
      setCorpusActionId(null)
    }
  }, [reload])

  const generateBenchmarkReport = useCallback(async (reportType: string, campaignId?: string | null) => {
    const actionKey = campaignId ? `${reportType}:${campaignId}` : reportType
    setReportActionId(actionKey)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkReport>("generate_benchmark_report", {
        input: {
          reportType,
          campaignId: campaignId ?? null,
          campaignIds: campaignId ? [campaignId] : benchmarkCampaigns.slice(0, 6).map((campaign) => campaign.id),
          windowDays: releaseGateWindowDays(filter, windowDays),
          markReleaseEvidence: reportType === "release",
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::generateBenchmarkReport", "Failed to generate benchmark report", e)
    } finally {
      setReportActionId(null)
    }
  }, [benchmarkCampaigns, filter, reload, windowDays])

  const markBenchmarkReport = useCallback(async (report: CodingBenchmarkReport, releaseEvidence: boolean) => {
    setReportActionId(`mark:${report.id}`)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkReport>("mark_benchmark_report_release_evidence", {
        input: {
          reportId: report.id,
          releaseEvidence,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::markBenchmarkReport", "Failed to mark benchmark report", e)
    } finally {
      setReportActionId(null)
    }
  }, [reload])

  const copyBenchmarkReportPath = useCallback(async (path: string) => {
    try {
      await navigator.clipboard?.writeText(path)
    } catch (e) {
      logger.warn("dashboard", "LearningTab::copyBenchmarkReportPath", "Failed to copy report path", e)
    }
  }, [])

  const materializeBenchmarkBacklog = useCallback(async () => {
    setGateActionId("materialize")
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkBacklogMaterializeResult>("materialize_benchmark_backlog", {
        input: {
          windowDays: releaseGateWindowDays(filter, windowDays),
          limit: 20,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::materializeBenchmarkBacklog", "Failed to materialize benchmark backlog", e)
    } finally {
      setGateActionId(null)
    }
  }, [filter, reload, windowDays])

  const resolveBenchmarkBacklogItem = useCallback(async (item: CodingBenchmarkBacklogItem) => {
    setGateActionId(`resolve:${item.id}`)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingBenchmarkBacklogItem>("update_benchmark_backlog_status", {
        input: {
          itemId: item.id,
          status: "resolved",
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::resolveBenchmarkBacklogItem", "Failed to resolve benchmark backlog item", e)
    } finally {
      setGateActionId(null)
    }
  }, [reload])

  const recordDomainEvalCalibration = useCallback(async (run: DomainEvalRunRecord) => {
    setDomainCalibrationActionId(run.id)
    setBenchmarkError(null)
    try {
      await getTransport().call<DomainEvalCalibrationRecord>("record_domain_eval_calibration", {
        input: {
          taskId: run.taskId,
          taskVersion: run.taskVersion,
          projectId: run.projectId ?? null,
          reviewer: "dashboard",
          verdict: run.status === "passed" ? "approved" : "needs_revision",
          sourceRunId: run.id,
          note:
            run.status === "passed"
              ? `Human review accepted ${run.label} as calibration evidence.`
              : `Human review marked ${run.label} for calibration follow-up after ${run.status}.`,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error(
        "dashboard",
        "LearningTab::recordDomainEvalCalibration",
        "Failed to record domain eval calibration",
        e,
      )
    } finally {
      setDomainCalibrationActionId(null)
    }
  }, [reload])

  useEffect(() => {
    reload()
  }, [reload])

  const totalRecall = (recall?.hits ?? 0) + (recall?.summarized ?? 0)
  const summaryPct = totalRecall > 0 ? Math.round(((recall?.summarized ?? 0) / totalRecall) * 100) : 0

  return (
    <div className="flex flex-col gap-4 mt-4">
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <h3 className="text-sm font-semibold flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-muted-foreground" />
            {t("dashboard.learning.title")}
          </h3>
          <p className="text-xs text-muted-foreground">{t("dashboard.learning.subtitle")}</p>
        </div>
        <div className="flex gap-2 items-center">
          <div className="flex gap-1">
            {WINDOW_OPTIONS.map((d) => (
              <Button
                key={d}
                size="sm"
                variant={windowDays === d ? "secondary" : "ghost"}
                className="text-xs h-7 px-2"
                onClick={() => setWindowDays(d)}
              >
                {t("dashboard.learning.daysN", { n: d })}
              </Button>
            ))}
          </div>
          <Button size="sm" variant="outline" onClick={reload} disabled={loading}>
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </Button>
        </div>
      </div>

      <CodingImprovementSection
        coding={coding}
        benchmark={benchmark}
        benchmarkCampaigns={benchmarkCampaigns}
        benchmarkLeaderboard={benchmarkLeaderboard}
        benchmarkTaskPacks={benchmarkTaskPacks}
        benchmarkCorpusHealth={benchmarkCorpusHealth}
        benchmarkReports={benchmarkReports}
        continuousGate={continuousGate}
        benchmarkBacklog={benchmarkBacklog}
        benchmarkModelOptions={benchmarkModelOptions}
        selectedBenchmarkModels={selectedBenchmarkModels}
        benchmarkMaxTasks={benchmarkMaxTasks}
        benchmarkBudgetUsd={benchmarkBudgetUsd}
        releaseGate={releaseGate}
        generalization={generalization}
        domainReadinessGate={domainReadinessGate}
        domainOperationalGate={domainOperationalGate}
        domainSoakReport={domainSoakReport}
        domainConnectorE2EGate={domainConnectorE2EGate}
        domainQualityGate={domainQualityGate}
        domainEvalRuns={domainEvalRuns}
        domainFixtureRuns={domainFixtureRuns}
        domainEvalCampaigns={domainEvalCampaigns}
        domainCampaignLeaderboard={domainCampaignLeaderboard}
        domainEvalTasks={domainEvalTasks}
        domainModelOptions={benchmarkModelOptions}
        selectedDomainModels={selectedDomainModels}
        domainCampaignMaxTasks={domainCampaignMaxTasks}
        domainCampaignBudgetUsd={domainCampaignBudgetUsd}
        benchmarkRunning={benchmarkRunning}
        benchmarkError={benchmarkError}
        domainCampaignError={domainCampaignError}
        campaignActionId={campaignActionId}
        domainCampaignActionId={domainCampaignActionId}
        corpusActionId={corpusActionId}
        reportActionId={reportActionId}
        gateActionId={gateActionId}
        domainCalibrationActionId={domainCalibrationActionId}
        onRunBenchmark={runBenchmark}
        onRunExternalBenchmark={runExternalBenchmark}
        onToggleBenchmarkModel={toggleBenchmarkModel}
        onBenchmarkMaxTasksChange={setBenchmarkMaxTasks}
        onBenchmarkBudgetUsdChange={setBenchmarkBudgetUsd}
        onCancelBenchmarkCampaign={cancelBenchmarkCampaign}
        onRetryBenchmarkCampaign={retryBenchmarkCampaign}
        onRunDomainEvalCampaign={runDomainEvalCampaign}
        onRunExternalDomainEvalCampaign={runExternalDomainEvalCampaign}
        onGenerateDomainCampaignLearning={generateDomainCampaignLearning}
        onToggleDomainModel={toggleDomainModel}
        onDomainCampaignMaxTasksChange={setDomainCampaignMaxTasks}
        onDomainCampaignBudgetUsdChange={setDomainCampaignBudgetUsd}
        onCancelDomainEvalCampaign={cancelDomainEvalCampaign}
        onRetryDomainEvalCampaign={retryDomainEvalCampaign}
        onImportSampleTaskPack={importSampleTaskPack}
        onUpdateTaskPackStatus={updateTaskPackStatus}
        onValidateTaskPack={validateTaskPack}
        onGenerateBenchmarkReport={generateBenchmarkReport}
        onMarkBenchmarkReport={markBenchmarkReport}
        onCopyBenchmarkReportPath={copyBenchmarkReportPath}
        onMaterializeBenchmarkBacklog={materializeBenchmarkBacklog}
        onResolveBenchmarkBacklogItem={resolveBenchmarkBacklogItem}
        onRecordDomainEvalCalibration={recordDomainEvalCalibration}
      />

      {/* Overview cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <OverviewCard
          label={t("dashboard.learning.autoSkills")}
          value={overview?.autoCreatedSkills ?? 0}
          hint={t("dashboard.learning.userSkillsHint", {
            n: overview?.userCreatedSkills ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.activated")}
          value={overview?.skillsActivated ?? 0}
          hint={t("dashboard.learning.patchedHint", {
            n: overview?.skillsPatched ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.recallHits")}
          value={overview?.recallHits ?? 0}
          hint={t("dashboard.learning.recallSummaryHint", {
            n: overview?.recallSummaryUsed ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.profileMemories")}
          value={overview?.profileMemories ?? 0}
          hint={t("dashboard.learning.profileHint")}
        />
      </div>

      {/* Timeline */}
      <div className="border border-border/60 rounded-lg p-4">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
          {t("dashboard.learning.timeline")}
        </h4>
        {timeline.length === 0 ? (
          <div className="text-xs text-muted-foreground text-center py-6">
            {t("dashboard.learning.noEvents")}
          </div>
        ) : (
          <div className="space-y-1 max-h-[240px] overflow-y-auto">
            {timeline.slice().reverse().map((p, i) => (
              <div
                key={`${p.ts}-${i}`}
                className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
              >
                <span className="text-muted-foreground tabular-nums w-32 shrink-0">
                  {new Date(p.ts * 1000).toLocaleString()}
                </span>
                <span
                  className={`px-1.5 py-0.5 rounded text-[10px] font-medium shrink-0 ${kindColor(p.kind)}`}
                >
                  {t(`dashboard.learning.kind.${p.kind}`)}
                </span>
                {p.skillId && (
                  <span className="text-foreground font-medium truncate flex-1">
                    {p.skillId}
                  </span>
                )}
                {p.source && (
                  <span className="text-[10px] text-muted-foreground">{p.source}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Top skills */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.topSkills")}
          </h4>
          {topSkills.length === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noSkillUsage")}
            </div>
          ) : (
            <div className="space-y-1.5">
              {topSkills.map((s) => (
                <div
                  key={s.skillId}
                  className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
                >
                  <span className="flex-1 truncate font-medium">{s.skillId}</span>
                  <span className="text-muted-foreground tabular-nums">
                    {s.usedCount}× · {s.lastUsedTs ? new Date(s.lastUsedTs * 1000).toLocaleDateString() : "—"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Recall effectiveness */}
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.recallEffectiveness")}
          </h4>
          {totalRecall === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noRecall")}
            </div>
          ) : (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.recallHits")}</span>
                <span className="font-mono">{recall?.hits ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-emerald-500 transition-all"
                  style={{ width: `${100 - summaryPct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.summarized")}</span>
                <span className="font-mono">{recall?.summarized ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-sky-500 transition-all"
                  style={{ width: `${summaryPct}%` }}
                />
              </div>
              <div className="text-[10px] text-muted-foreground text-right pt-1">
                {t("dashboard.learning.summaryPct", { pct: summaryPct })}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function OverviewCard({
  label,
  value,
  hint,
}: {
  label: string
  value: number
  hint?: string
}) {
  return (
    <div className="border border-border/60 rounded-lg p-3 flex flex-col gap-1">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[10px] text-muted-foreground">{hint}</div>}
    </div>
  )
}

function CodingImprovementSection({
  coding,
  benchmark,
  benchmarkCampaigns,
  benchmarkLeaderboard,
  benchmarkTaskPacks,
  benchmarkCorpusHealth,
  benchmarkReports,
  continuousGate,
  benchmarkBacklog,
  benchmarkModelOptions,
  selectedBenchmarkModels,
  benchmarkMaxTasks,
  benchmarkBudgetUsd,
  releaseGate,
  generalization,
  domainReadinessGate,
  domainOperationalGate,
  domainSoakReport,
  domainConnectorE2EGate,
  domainQualityGate,
  domainEvalRuns,
  domainFixtureRuns,
  domainEvalCampaigns,
  domainCampaignLeaderboard,
  domainEvalTasks,
  domainModelOptions,
  selectedDomainModels,
  domainCampaignMaxTasks,
  domainCampaignBudgetUsd,
  benchmarkRunning,
  benchmarkError,
  domainCampaignError,
  campaignActionId,
  domainCampaignActionId,
  corpusActionId,
  reportActionId,
  gateActionId,
  domainCalibrationActionId,
  onRunBenchmark,
  onRunExternalBenchmark,
  onToggleBenchmarkModel,
  onBenchmarkMaxTasksChange,
  onBenchmarkBudgetUsdChange,
  onCancelBenchmarkCampaign,
  onRetryBenchmarkCampaign,
  onRunDomainEvalCampaign,
  onRunExternalDomainEvalCampaign,
  onGenerateDomainCampaignLearning,
  onToggleDomainModel,
  onDomainCampaignMaxTasksChange,
  onDomainCampaignBudgetUsdChange,
  onCancelDomainEvalCampaign,
  onRetryDomainEvalCampaign,
  onImportSampleTaskPack,
  onUpdateTaskPackStatus,
  onValidateTaskPack,
  onGenerateBenchmarkReport,
  onMarkBenchmarkReport,
  onCopyBenchmarkReportPath,
  onMaterializeBenchmarkBacklog,
  onResolveBenchmarkBacklogItem,
  onRecordDomainEvalCalibration,
}: {
  coding: CodingImprovementDashboard | null
  benchmark: CodingBenchmarkCenterReport | null
  benchmarkCampaigns: CodingBenchmarkCampaign[]
  benchmarkLeaderboard: CodingBenchmarkLeaderboardReport | null
  benchmarkTaskPacks: CodingBenchmarkTaskPack[]
  benchmarkCorpusHealth: CodingBenchmarkCorpusHealthReport | null
  benchmarkReports: CodingBenchmarkReport[]
  continuousGate: CodingContinuousBenchmarkGateReport | null
  benchmarkBacklog: CodingBenchmarkBacklogItem[]
  benchmarkModelOptions: BenchmarkModelOption[]
  selectedBenchmarkModels: string[]
  benchmarkMaxTasks: number
  benchmarkBudgetUsd: string
  releaseGate: CodingEvalReleaseGateReport | null
  generalization: CodingLearningGeneralizationReport | null
  domainReadinessGate: DomainReadinessGateReport | null
  domainOperationalGate: DomainOperationalGateReport | null
  domainSoakReport: DomainSoakReport | null
  domainConnectorE2EGate: DomainConnectorE2EGateReport | null
  domainQualityGate: DomainQualityGateReport | null
  domainEvalRuns: DomainEvalRunRecord[]
  domainFixtureRuns: DomainEvalFixtureRunRecord[]
  domainEvalCampaigns: DomainEvalCampaign[]
  domainCampaignLeaderboard: DomainEvalCampaignLeaderboardReport | null
  domainEvalTasks: DomainEvalTask[]
  domainModelOptions: BenchmarkModelOption[]
  selectedDomainModels: string[]
  domainCampaignMaxTasks: number
  domainCampaignBudgetUsd: string
  benchmarkRunning: boolean
  benchmarkError: string | null
  domainCampaignError: string | null
  campaignActionId: string | null
  domainCampaignActionId: string | null
  corpusActionId: string | null
  reportActionId: string | null
  gateActionId: string | null
  domainCalibrationActionId: string | null
  onRunBenchmark: () => void
  onRunExternalBenchmark: () => void
  onToggleBenchmarkModel: (key: string) => void
  onBenchmarkMaxTasksChange: (value: number) => void
  onBenchmarkBudgetUsdChange: (value: string) => void
  onCancelBenchmarkCampaign: (campaignId: string) => void
  onRetryBenchmarkCampaign: (campaignId: string) => void
  onRunDomainEvalCampaign: () => void
  onRunExternalDomainEvalCampaign: () => void
  onGenerateDomainCampaignLearning: (campaign: DomainEvalCampaign) => void
  onToggleDomainModel: (key: string) => void
  onDomainCampaignMaxTasksChange: (value: number) => void
  onDomainCampaignBudgetUsdChange: (value: string) => void
  onCancelDomainEvalCampaign: (campaignId: string) => void
  onRetryDomainEvalCampaign: (campaignId: string) => void
  onImportSampleTaskPack: () => void
  onUpdateTaskPackStatus: (pack: CodingBenchmarkTaskPack, status: string) => void
  onValidateTaskPack: (pack: CodingBenchmarkTaskPack) => void
  onGenerateBenchmarkReport: (reportType: string, campaignId?: string | null) => void
  onMarkBenchmarkReport: (report: CodingBenchmarkReport, releaseEvidence: boolean) => void
  onCopyBenchmarkReportPath: (path: string) => void
  onMaterializeBenchmarkBacklog: () => void
  onResolveBenchmarkBacklogItem: (item: CodingBenchmarkBacklogItem) => void
  onRecordDomainEvalCalibration: (run: DomainEvalRunRecord) => void
}) {
  const { t } = useTranslation()
  const overview = coding?.overview
  const domainQuality = coding?.domainQuality
  const recentTimeline = coding?.timeline.slice(-10).reverse() ?? []
  const failureModes = [...(coding?.topFailures ?? []), ...(coding?.toolCallFailures ?? [])]
  const maxTimelineValue = Math.max(
    1,
    ...recentTimeline.map(
      (p) =>
        p.completedWorkflows +
        p.blockedWorkflows +
        p.failedWorkflows +
        p.evalPassed +
        p.evalFailed +
        p.evalPackPassed +
        p.evalPackFailed +
        p.strategyImproved +
        p.strategyRegressed +
        p.strategyMixed +
        Math.abs(p.validationViolationDelta) +
        Math.abs(p.scopeCreepDelta) +
        p.proposalsCreated +
        p.proposalsApplied +
        p.proposalsPromoted +
        p.retroRecommendations,
    ),
  )

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.codingImprovement", {
            defaultValue: "Coding improvement",
          })}
        </h4>
        {coding?.generatedAt && (
          <span className="text-[10px] text-muted-foreground">
            {new Date(coding.generatedAt).toLocaleString()}
          </span>
        )}
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 xl:grid-cols-8 gap-3">
        <InsightCard
          icon={GitBranch}
          label={t("dashboard.learning.workflowHealth", {
            defaultValue: "Workflow",
          })}
          value={formatPct(overview?.workflowCompletionRate)}
          hint={`${overview?.completedWorkflows ?? 0}/${overview?.workflowRuns ?? 0}`}
        />
        <InsightCard
          icon={CheckCircle2}
          label={t("dashboard.learning.evalHealth", { defaultValue: "Eval" })}
          value={formatPct(overview?.evalSuccessRate)}
          hint={`${overview?.passedEvalRuns ?? 0}/${overview?.evalRuns ?? 0}`}
        />
        <InsightCard
          icon={Layers3}
          label={t("dashboard.learning.packHealth", { defaultValue: "Pack" })}
          value={formatPct(overview?.evalPackPassRate)}
          hint={`${overview?.passedEvalPackRuns ?? 0}/${overview?.evalPackRuns ?? 0}`}
        />
        <InsightCard
          icon={Activity}
          label={t("dashboard.learning.strategyEffects", {
            defaultValue: "Strategy",
          })}
          value={overview?.strategyEffectRuns ?? 0}
          hint={`+${overview?.improvedStrategyEffects ?? 0} / -${overview?.regressedStrategyEffects ?? 0}`}
        />
        <InsightCard
          icon={Sparkles}
          label={t("dashboard.learning.toolCalls", { defaultValue: "Tool calls" })}
          value={overview?.missingToolCallRuns ?? 0}
          hint={t("dashboard.learning.missingToolCalls", {
            defaultValue: "missing calls",
          })}
        />
        <InsightCard
          icon={ShieldAlert}
          label={t("dashboard.learning.blockers", { defaultValue: "Blockers" })}
          value={overview?.openReviewBlockers ?? 0}
          hint={t("dashboard.learning.verificationFailures", {
            defaultValue: "{{n}} verification",
            n: overview?.failedVerificationSteps ?? 0,
          })}
        />
        <InsightCard
          icon={Layers3}
          label={t("dashboard.learning.distillationQueue", {
            defaultValue: "Distillation",
          })}
          value={overview?.distillationCandidates ?? 0}
          hint={t("dashboard.learning.proposalHint", {
            defaultValue: "{{n}} drafts",
            n: overview?.draftProposals ?? 0,
          })}
        />
        <InsightCard
          icon={Activity}
          label={t("dashboard.learning.retros", { defaultValue: "Retros" })}
          value={overview?.retros ?? 0}
          hint={t("dashboard.learning.retroHint", {
            defaultValue: "{{n}} recommendations",
            n: overview?.retroRecommendations ?? 0,
          })}
        />
      </div>

      <BenchmarkCenterPanel
        report={benchmark}
        campaigns={benchmarkCampaigns}
        leaderboard={benchmarkLeaderboard}
        modelOptions={benchmarkModelOptions}
        selectedModelKeys={selectedBenchmarkModels}
        maxTasks={benchmarkMaxTasks}
        budgetUsd={benchmarkBudgetUsd}
        running={benchmarkRunning}
        error={benchmarkError}
        actionId={campaignActionId}
        onRun={onRunBenchmark}
        onRunExternal={onRunExternalBenchmark}
        onToggleModel={onToggleBenchmarkModel}
        onMaxTasksChange={onBenchmarkMaxTasksChange}
        onBudgetUsdChange={onBenchmarkBudgetUsdChange}
        onCancelCampaign={onCancelBenchmarkCampaign}
        onRetryCampaign={onRetryBenchmarkCampaign}
      />

      <BenchmarkCorpusPanel
        health={benchmarkCorpusHealth}
        packs={benchmarkTaskPacks}
        actionId={corpusActionId}
        onImportSample={onImportSampleTaskPack}
        onUpdateStatus={onUpdateTaskPackStatus}
        onValidate={onValidateTaskPack}
      />

      <BenchmarkReportPanel
        reports={benchmarkReports}
        campaigns={benchmarkCampaigns}
        actionId={reportActionId}
        onGenerate={onGenerateBenchmarkReport}
        onMarkReleaseEvidence={onMarkBenchmarkReport}
        onCopyPath={onCopyBenchmarkReportPath}
      />

      <ContinuousBenchmarkGatePanel
        gate={continuousGate}
        backlog={benchmarkBacklog}
        actionId={gateActionId}
        onMaterializeBacklog={onMaterializeBenchmarkBacklog}
        onResolveBacklogItem={onResolveBenchmarkBacklogItem}
      />

      <DomainQualityDashboardPanel dashboard={domainQuality ?? null} />

      <DomainReadinessGatePanel report={domainReadinessGate} />

      <DomainOperationalGatePanel report={domainOperationalGate} />

      <DomainSoakReportPanel report={domainSoakReport} />

      <DomainConnectorE2EGatePanel report={domainConnectorE2EGate} />

      <DomainQualityGatePanel
        report={domainQualityGate}
        runs={domainEvalRuns}
        taskCount={domainEvalTasks.length}
        calibratedTaskCount={
          domainEvalTasks.filter((task) =>
            task.calibration.some((record) => record.scope === "user" || record.scope === "project"),
          ).length
        }
        calibrationActionId={domainCalibrationActionId}
        onRecordCalibration={onRecordDomainEvalCalibration}
      />

      <DomainEvalCampaignPanel
        campaigns={domainEvalCampaigns}
        leaderboard={domainCampaignLeaderboard}
        modelOptions={domainModelOptions}
        selectedModelKeys={selectedDomainModels}
        maxTasks={domainCampaignMaxTasks}
        budgetUsd={domainCampaignBudgetUsd}
        actionId={domainCampaignActionId}
        error={domainCampaignError}
        onRun={onRunDomainEvalCampaign}
        onRunExternal={onRunExternalDomainEvalCampaign}
        onGenerateLearning={onGenerateDomainCampaignLearning}
        onToggleModel={onToggleDomainModel}
        onMaxTasksChange={onDomainCampaignMaxTasksChange}
        onBudgetUsdChange={onDomainCampaignBudgetUsdChange}
        onCancel={onCancelDomainEvalCampaign}
        onRetry={onRetryDomainEvalCampaign}
      />

      <DomainFixtureSmokePanel runs={domainFixtureRuns} />

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
        <ReleaseGatePanel report={releaseGate} />
        <GeneralizationPanel report={generalization} />
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-[1.3fr_1fr] gap-3">
        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <div className="flex items-center justify-between mb-3">
            <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("dashboard.learning.projectSignals", {
                defaultValue: "Project signals",
              })}
            </h4>
            <span className="text-[10px] text-muted-foreground tabular-nums">
              {coding?.byProject.length ?? 0}
            </span>
          </div>
          {coding?.byProject.length ? (
            <div className="space-y-2">
              {coding.byProject.map((project) => (
                <ProjectSignalRow
                  key={project.projectId ?? "__unassigned__"}
                  name={project.projectName ?? project.projectId ?? "Unassigned"}
                  projectId={project.projectId}
                  workflowRate={project.workflowCompletionRate}
                  evalRate={project.evalSuccessRate}
                  packRate={project.evalPackPassRate}
                  strategyRegressions={project.regressedStrategyEffects}
                  blockers={project.openReviewBlockers}
                  candidates={project.distillationCandidates}
                />
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noProjectSignals", {
              defaultValue: "No coding improvement signals",
            })} />
          )}
        </div>

        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.failureModes", { defaultValue: "Failure modes" })}
          </h4>
          {failureModes.length ? (
            <div className="space-y-2">
              {failureModes.map((failure) => (
                <div
                  key={failure.category}
                  className="flex items-center gap-2 text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <span className={`h-2 w-2 rounded-full ${severityDot(failure.severity)}`} />
                  <span className="font-medium truncate flex-1">{failure.label}</span>
                  <span className="text-muted-foreground tabular-nums">{failure.count}</span>
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noFailureModes", {
              defaultValue: "No failure modes",
            })} />
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-3 gap-3">
        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.improvementTimeline", {
              defaultValue: "Improvement timeline",
            })}
          </h4>
          {recentTimeline.length ? (
            <div className="space-y-2">
              {recentTimeline.map((point) => {
                const total =
                  point.completedWorkflows +
                  point.blockedWorkflows +
                  point.failedWorkflows +
                  point.evalPassed +
                  point.evalFailed +
                  point.evalPackPassed +
                  point.evalPackFailed +
                  point.strategyImproved +
                  point.strategyRegressed +
                  point.strategyMixed +
                  Math.abs(point.validationViolationDelta) +
                  Math.abs(point.scopeCreepDelta) +
                  point.proposalsCreated +
                  point.proposalsApplied +
                  point.proposalsPromoted +
                  point.retroRecommendations
                return (
                  <div key={point.date} className="flex items-center gap-3 text-xs">
                    <span className="w-20 text-muted-foreground tabular-nums">
                      {point.date}
                    </span>
                    <div className="h-2 flex-1 bg-secondary/40 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-emerald-500"
                        style={{ width: `${Math.max(4, (total / maxTimelineValue) * 100)}%` }}
                      />
                    </div>
                    <span className="w-8 text-right tabular-nums text-muted-foreground">
                      {total}
                    </span>
                  </div>
                )
              })}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noTimeline", {
              defaultValue: "No timeline data",
            })} />
          )}
        </div>

        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.latestStrategyEffects", {
              defaultValue: "Latest strategy effects",
            })}
          </h4>
          {coding?.latestStrategyEffects.length ? (
            <div className="space-y-2 max-h-[220px] overflow-y-auto">
              {coding.latestStrategyEffects.map((effect) => (
                <div
                  key={effect.id}
                  className="text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <div className="flex items-center gap-2 mb-1 min-w-0">
                    <span className={`px-1.5 py-0.5 rounded text-[10px] ${verdictTone(effect.verdict)}`}>
                      {effect.verdict}
                    </span>
                    <span className="font-medium truncate flex-1">{effect.strategyType}</span>
                    <span className="text-[10px] text-muted-foreground tabular-nums">
                      {new Date(effect.createdAt).toLocaleDateString()}
                    </span>
                  </div>
                  <p className="text-[10px] text-muted-foreground truncate">
                    {effect.baselineLabel} -&gt; {effect.candidateLabel}
                  </p>
                  <div className="mt-1 flex flex-wrap gap-1.5">
                    <MetricPill
                      label="P"
                      value={formatSignedPct(effect.passRateDelta)}
                      tone={deltaTone(effect.passRateDelta)}
                    />
                    <MetricPill
                      label="S"
                      value={formatSignedPct(effect.averageScoreDelta)}
                      tone={deltaTone(effect.averageScoreDelta)}
                    />
                    <MetricPill
                      label="V"
                      value={formatSignedCount(effect.validationViolationDelta)}
                      tone={inverseDeltaTone(effect.validationViolationDelta)}
                    />
                    <MetricPill
                      label="C"
                      value={formatSignedCount(effect.scopeCreepDelta)}
                      tone={inverseDeltaTone(effect.scopeCreepDelta)}
                    />
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noStrategyEffects", {
              defaultValue: "No strategy effects",
            })} />
          )}
        </div>

        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.latestRetros", { defaultValue: "Latest retros" })}
          </h4>
          {coding?.latestRetros.length ? (
            <div className="space-y-2 max-h-[220px] overflow-y-auto">
              {coding.latestRetros.map((retro) => (
                <div
                  key={retro.id}
                  className="text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <div className="flex items-center gap-2 mb-1">
                    <span className={`px-1.5 py-0.5 rounded text-[10px] ${stateTone(retro.runState)}`}>
                      {retro.runState}
                    </span>
                    <span className="text-muted-foreground tabular-nums">
                      {new Date(retro.updatedAt).toLocaleDateString()}
                    </span>
                  </div>
                  <p className="text-foreground line-clamp-2">{retro.summary}</p>
                  {retro.recommendations[0] && (
                    <p className="text-[10px] text-muted-foreground mt-1 truncate">
                      {retro.recommendations[0].title}
                    </p>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noRetros", {
              defaultValue: "No retros",
            })} />
          )}
        </div>
      </div>
    </section>
  )
}

function BenchmarkReportPanel({
  reports,
  campaigns,
  actionId,
  onGenerate,
  onMarkReleaseEvidence,
  onCopyPath,
}: {
  reports: CodingBenchmarkReport[]
  campaigns: CodingBenchmarkCampaign[]
  actionId: string | null
  onGenerate: (reportType: string, campaignId?: string | null) => void
  onMarkReleaseEvidence: (report: CodingBenchmarkReport, releaseEvidence: boolean) => void
  onCopyPath: (path: string) => void
}) {
  const { t } = useTranslation()
  const latestCampaign = campaigns[0]
  const generatingComparison = actionId === "comparison"
  const generatingRelease = actionId === "release"
  const generatingCampaign = latestCampaign ? actionId === `campaign:${latestCampaign.id}` : false

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.benchmarkReports", {
              defaultValue: "Benchmark reports",
            })}
          </h4>
          <span className="rounded bg-secondary/40 px-2 py-1 text-[10px] text-muted-foreground tabular-nums">
            {reports.length}
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5"
            onClick={() => onGenerate("comparison", null)}
            disabled={Boolean(actionId)}
            title={t("dashboard.learning.generateComparisonReport", {
              defaultValue: "Generate comparison report",
            })}
          >
            {generatingComparison ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FileText className="h-3.5 w-3.5" />}
            <span className="text-xs">
              {t("dashboard.learning.reportComparison", {
                defaultValue: "Comparison",
              })}
            </span>
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5"
            onClick={() => onGenerate("release", null)}
            disabled={Boolean(actionId)}
            title={t("dashboard.learning.generateReleaseReport", {
              defaultValue: "Generate release report",
            })}
          >
            {generatingRelease ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
            <span className="text-xs">
              {t("dashboard.learning.reportRelease", {
                defaultValue: "Release",
              })}
            </span>
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5"
            onClick={() => latestCampaign && onGenerate("campaign", latestCampaign.id)}
            disabled={Boolean(actionId) || !latestCampaign}
            title={t("dashboard.learning.generateCampaignReport", {
              defaultValue: "Generate campaign report",
            })}
          >
            {generatingCampaign ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Layers3 className="h-3.5 w-3.5" />}
            <span className="text-xs">
              {t("dashboard.learning.reportCampaign", {
                defaultValue: "Campaign",
              })}
            </span>
          </Button>
        </div>
      </div>

      {reports.length ? (
        <div className="space-y-2">
          {reports.slice(0, 6).map((report) => (
            <div key={report.id} className="rounded border border-border/40 p-2.5 text-xs">
              <div className="flex flex-wrap items-center gap-2">
                <span className={`rounded px-1.5 py-0.5 text-[10px] ${releaseGateTone(report.status)}`}>
                  {report.status}
                </span>
                <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {report.reportType}
                </span>
                <span className="min-w-0 max-w-[320px] truncate font-medium">{report.title}</span>
                <span className="text-[10px] text-muted-foreground tabular-nums">
                  {new Date(report.createdAt).toLocaleString()}
                </span>
                <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
                  {report.releaseEvidence && (
                    <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-600 dark:text-emerald-400">
                      release
                    </span>
                  )}
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 px-1.5"
                    onClick={() => onCopyPath(report.markdownPath)}
                    title={t("dashboard.learning.copyReportPath", {
                      defaultValue: "Copy report path",
                    })}
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 px-1.5"
                    onClick={() => onMarkReleaseEvidence(report, !report.releaseEvidence)}
                    disabled={Boolean(actionId)}
                    title={t("dashboard.learning.toggleReleaseEvidence", {
                      defaultValue: report.releaseEvidence
                        ? "Remove release evidence"
                        : "Mark as release evidence",
                    })}
                  >
                    {actionId === `mark:${report.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CheckCircle2 className="h-3.5 w-3.5" />}
                  </Button>
                </div>
              </div>
              <p className="mt-1.5 line-clamp-2 text-[10px] text-muted-foreground" title={report.summary}>
                {report.summary}
              </p>
              <div className="mt-1.5 truncate text-[10px] text-muted-foreground" title={report.markdownPath}>
                {report.markdownPath}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.noBenchmarkReports", {
            defaultValue: "No benchmark reports",
          })}
        />
      )}
    </div>
  )
}

function ContinuousBenchmarkGatePanel({
  gate,
  backlog,
  actionId,
  onMaterializeBacklog,
  onResolveBacklogItem,
}: {
  gate: CodingContinuousBenchmarkGateReport | null
  backlog: CodingBenchmarkBacklogItem[]
  actionId: string | null
  onMaterializeBacklog: () => void
  onResolveBacklogItem: (item: CodingBenchmarkBacklogItem) => void
}) {
  const { t } = useTranslation()
  const blockingChecks = gate?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []
  const recommendations = gate?.recommendedNextSteps.slice(0, 3) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.continuousBenchmarkGate", {
              defaultValue: "Continuous gate",
            })}
          </h4>
          <span className={`rounded px-2 py-1 text-[10px] font-medium ${releaseGateTone(gate?.status)}`}>
            {gate?.status ?? "loading"}
          </span>
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1.5"
          onClick={onMaterializeBacklog}
          disabled={Boolean(actionId) || !gate || gate.summary.pendingFailureItems === 0}
          title={t("dashboard.learning.materializeBenchmarkBacklog", {
            defaultValue: "Create backlog items from failed benchmark cases",
          })}
        >
          {actionId === "materialize" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FileCheck2 className="h-3.5 w-3.5" />}
          <span className="text-xs">
            {t("dashboard.learning.createBacklog", {
              defaultValue: "Create backlog",
            })}
          </span>
        </Button>
      </div>

      {gate ? (
        <div className="grid grid-cols-1 xl:grid-cols-[1.1fr_1fr] gap-3">
          <div className="space-y-3">
            <div className="grid grid-cols-2 md:grid-cols-4 gap-2 text-xs">
              <MetricPill label="RC" value={gate.summary.freshCampaigns} tone={gate.summary.freshCampaigns > 0 ? "accent" : "warn"} />
              <MetricPill label="CP" value={formatPct(gate.summary.casePassRate)} tone={gate.summary.casePassRate === 1 ? "accent" : "warn"} />
              <MetricPill label="BL" value={gate.summary.openBacklogItems} tone={gate.summary.openBacklogItems > 0 ? "warn" : "muted"} />
              <MetricPill label="PF" value={gate.summary.pendingFailureItems} tone={gate.summary.pendingFailureItems > 0 ? "warn" : "muted"} />
              <MetricPill label="INT" value={gate.reliability.interruptedCampaigns} tone={gate.reliability.interruptedCampaigns > 0 ? "warn" : "muted"} />
              <MetricPill label="PE" value={gate.reliability.providerErrorItems} tone={gate.reliability.providerErrorItems > 0 ? "warn" : "muted"} />
              <MetricPill label="RT" value={formatPct(gate.reliability.retrySuccessRate)} />
              <MetricPill label="RAW" value={`${gate.summary.rawArtifactRetentionDays}d`} />
            </div>

            {blockingChecks.length ? (
              <div className="space-y-1.5">
                {blockingChecks.map((check) => (
                  <div key={check.name} className="rounded border border-border/40 p-2 text-xs">
                    <div className="flex items-center gap-2">
                      <span className={`rounded px-1.5 py-0.5 text-[10px] ${releaseGateTone(check.status)}`}>
                        {check.status}
                      </span>
                      <span className="font-medium">{check.name}</span>
                      <span className="ml-auto text-[10px] text-muted-foreground truncate">{check.actual}</span>
                    </div>
                    <p className="mt-1 text-[10px] text-muted-foreground line-clamp-2">{check.detail}</p>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyLine
                label={t("dashboard.learning.noContinuousGateBlockers", {
                  defaultValue: "No blocking gate checks",
                })}
              />
            )}
          </div>

          <div className="space-y-3 min-w-0">
            <div className="rounded border border-border/40 p-2.5 text-xs">
              <div className="mb-2 font-medium text-muted-foreground">
                {t("dashboard.learning.nextSteps", {
                  defaultValue: "Next steps",
                })}
              </div>
              {recommendations.length ? (
                <div className="space-y-1">
                  {recommendations.map((step) => (
                    <div key={step} className="line-clamp-2 text-[10px] text-muted-foreground">
                      {step}
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyLine label="—" />
              )}
            </div>

            <div className="rounded border border-border/40 p-2.5 text-xs">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="font-medium text-muted-foreground">
                  {t("dashboard.learning.benchmarkBacklog", {
                    defaultValue: "Benchmark backlog",
                  })}
                </span>
                <span className="text-[10px] tabular-nums text-muted-foreground">{backlog.length}</span>
              </div>
              {backlog.length ? (
                <div className="space-y-1.5">
                  {backlog.slice(0, 4).map((item) => (
                    <div key={item.id} className="flex items-start gap-2 rounded bg-secondary/20 p-2">
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-medium">{item.title}</div>
                        <div className="truncate text-[10px] text-muted-foreground">
                          {item.failureCategory} / {item.label ?? item.modelId ?? item.baselineKind}
                        </div>
                      </div>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-6 px-1.5"
                        onClick={() => onResolveBacklogItem(item)}
                        disabled={Boolean(actionId)}
                        title={t("dashboard.learning.resolveBacklogItem", {
                          defaultValue: "Resolve backlog item",
                        })}
                      >
                        {actionId === `resolve:${item.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CheckCircle2 className="h-3.5 w-3.5" />}
                      </Button>
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noBenchmarkBacklog", {
                    defaultValue: "No open benchmark backlog",
                  })}
                />
              )}
            </div>
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.noContinuousGate", {
            defaultValue: "No continuous gate data",
          })}
        />
      )}
    </div>
  )
}

function BenchmarkCorpusPanel({
  health,
  packs,
  actionId,
  onImportSample,
  onUpdateStatus,
  onValidate,
}: {
  health: CodingBenchmarkCorpusHealthReport | null
  packs: CodingBenchmarkTaskPack[]
  actionId: string | null
  onImportSample: () => void
  onUpdateStatus: (pack: CodingBenchmarkTaskPack, status: string) => void
  onValidate: (pack: CodingBenchmarkTaskPack) => void
}) {
  const { t } = useTranslation()
  const attentionChecks =
    health?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []
  const visiblePacks = packs.slice(0, 6)

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.benchmarkCorpus", {
              defaultValue: "Task corpus",
            })}
          </h4>
          <span className={`rounded px-2 py-1 text-[10px] font-medium ${releaseGateTone(health?.status)}`}>
            {health?.status ?? "loading"}
          </span>
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1.5"
          onClick={onImportSample}
          disabled={actionId === "import"}
          title={t("dashboard.learning.importSampleTaskPack", {
            defaultValue: "Import sample task pack",
          })}
        >
          {actionId === "import" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
          <span className="text-xs">
            {t("dashboard.learning.importSample", {
              defaultValue: "Import sample",
            })}
          </span>
        </Button>
      </div>

      {health ? (
        <div className="space-y-3">
          <div className="flex flex-wrap gap-1.5">
            <MetricPill label="PK" value={`${health.activePacks}/${health.packs}`} tone={health.activePacks > 0 ? "accent" : "muted"} />
            <MetricPill label="TS" value={`${health.activeTasks}/${health.tasks}`} tone={health.activeTasks > 0 ? "accent" : "muted"} />
            <MetricPill label="DR" value={health.draftTasks} />
            <MetricPill label="ST" value={health.staleTasks.length} tone={health.staleTasks.length > 0 ? "warn" : "muted"} />
            <MetricPill label="DP" value={health.duplicateTasks.length} tone={health.duplicateTasks.length > 0 ? "warn" : "muted"} />
            <MetricPill label="RG" value={health.gamingRiskTasks.length} tone={health.gamingRiskTasks.length > 0 ? "warn" : "muted"} />
          </div>

          <div className="grid grid-cols-1 gap-2 xl:grid-cols-[minmax(0,1fr)_minmax(220px,0.75fr)]">
            <div className="space-y-2 min-w-0">
              {visiblePacks.length ? (
                visiblePacks.map((pack) => (
                  <BenchmarkTaskPackRow
                    key={`${pack.packId}@${pack.version}`}
                    pack={pack}
                    busyAction={actionId}
                    onUpdateStatus={onUpdateStatus}
                    onValidate={onValidate}
                  />
                ))
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noTaskPacks", {
                    defaultValue: "No task packs",
                  })}
                />
              )}
            </div>

            <div className="min-w-0 space-y-2">
              <div className="flex flex-wrap gap-1.5">
                {health.byTaskType.slice(0, 5).map((bucket) => (
                  <span
                    key={bucket.key}
                    className="max-w-full truncate rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground"
                    title={bucket.key}
                  >
                    {bucket.key}:{bucket.count}
                  </span>
                ))}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {attentionChecks.length ? (
                  attentionChecks.map((check) => (
                    <span
                      key={check.name}
                      className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                      title={`${check.expected} · ${check.actual}`}
                    >
                      {check.name}: {check.actual}
                    </span>
                  ))
                ) : (
                  <span className="text-[10px] text-muted-foreground">
                    {t("dashboard.learning.corpusClean", {
                      defaultValue: "Corpus checks passed",
                    })}
                  </span>
                )}
              </div>
            </div>
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.corpusLoading", {
            defaultValue: "Loading corpus",
          })}
        />
      )}
    </div>
  )
}

function BenchmarkTaskPackRow({
  pack,
  busyAction,
  onUpdateStatus,
  onValidate,
}: {
  pack: CodingBenchmarkTaskPack
  busyAction: string | null
  onUpdateStatus: (pack: CodingBenchmarkTaskPack, status: string) => void
  onValidate: (pack: CodingBenchmarkTaskPack) => void
}) {
  const { t } = useTranslation()
  const activeTasks = pack.tasks.filter((task) => task.status === "active").length
  const riskTasks = pack.tasks.filter((task) => task.riskFlags.length > 0).length
  const baseKey = `${pack.packId}@${pack.version}`
  const validating = busyAction === `${baseKey}:validate`
  const activating = busyAction === `${baseKey}:active`
  const archiving = busyAction === `${baseKey}:archived`

  return (
    <div className="rounded border border-border/40 p-2.5 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`rounded px-1.5 py-0.5 text-[10px] ${releaseGateTone(pack.status === "active" ? "passed" : pack.status === "archived" ? "failed" : "insufficient_data")}`}>
          {pack.status}
        </span>
        <span className="min-w-0 max-w-[280px] truncate font-medium">
          {pack.name}
        </span>
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {pack.packId}@{pack.version}
        </span>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
          <MetricPill label="TS" value={`${activeTasks}/${pack.tasks.length}`} tone={activeTasks > 0 ? "accent" : "muted"} />
          <MetricPill label="RG" value={riskTasks} tone={riskTasks > 0 ? "warn" : "muted"} />
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-1.5"
            onClick={() => onValidate(pack)}
            disabled={Boolean(busyAction)}
            title={t("dashboard.learning.validateTaskPack", {
              defaultValue: "Validate task pack",
            })}
          >
            {validating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FileCheck2 className="h-3.5 w-3.5" />}
          </Button>
          {pack.status !== "active" && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5"
              onClick={() => onUpdateStatus(pack, "active")}
              disabled={Boolean(busyAction)}
              title={t("dashboard.learning.activateTaskPack", {
                defaultValue: "Activate task pack",
              })}
            >
              {activating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CheckCircle2 className="h-3.5 w-3.5" />}
            </Button>
          )}
          {pack.status !== "archived" && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-muted-foreground hover:text-amber-700 dark:hover:text-amber-300"
              onClick={() => onUpdateStatus(pack, "archived")}
              disabled={Boolean(busyAction)}
              title={t("dashboard.learning.archiveTaskPack", {
                defaultValue: "Archive task pack",
              })}
            >
              {archiving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Archive className="h-3.5 w-3.5" />}
            </Button>
          )}
        </div>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
          {pack.sourceKind}
        </span>
        <span className="max-w-full truncate rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
          {pack.redactionStatus}
        </span>
        {pack.tasks.slice(0, 4).map((task) => (
          <span
            key={`${task.taskId}@${task.version}`}
            className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${
              task.status === "active"
                ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                : "bg-secondary/40 text-muted-foreground"
            }`}
            title={`${task.taskType} · ${task.difficulty}`}
          >
            {task.taskId}@{task.version}
          </span>
        ))}
        {pack.tasks.length > 4 && (
          <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            +{pack.tasks.length - 4}
          </span>
        )}
      </div>
    </div>
  )
}

function BenchmarkCenterPanel({
  report,
  campaigns,
  leaderboard,
  modelOptions,
  selectedModelKeys,
  maxTasks,
  budgetUsd,
  running,
  error,
  actionId,
  onRun,
  onRunExternal,
  onToggleModel,
  onMaxTasksChange,
  onBudgetUsdChange,
  onCancelCampaign,
  onRetryCampaign,
}: {
  report: CodingBenchmarkCenterReport | null
  campaigns: CodingBenchmarkCampaign[]
  leaderboard: CodingBenchmarkLeaderboardReport | null
  modelOptions: BenchmarkModelOption[]
  selectedModelKeys: string[]
  maxTasks: number
  budgetUsd: string
  running: boolean
  error: string | null
  actionId: string | null
  onRun: () => void
  onRunExternal: () => void
  onToggleModel: (key: string) => void
  onMaxTasksChange: (value: number) => void
  onBudgetUsdChange: (value: string) => void
  onCancelCampaign: (campaignId: string) => void
  onRetryCampaign: (campaignId: string) => void
}) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []
  const recentRuns = report?.runs.slice(0, 4) ?? []
  const activeCampaigns = campaigns.filter((campaign) =>
    ["queued", "running", "cancel_requested"].includes(campaign.status),
  ).length
  const visibleModels = modelOptions.slice(0, 10)

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.benchmarkCenter", {
              defaultValue: "Benchmark center",
            })}
          </h4>
          <span
            className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
          >
            {report?.status ?? "loading"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          {activeCampaigns > 0 && (
            <span className="text-[10px] text-muted-foreground tabular-nums">
              {activeCampaigns} active
            </span>
          )}
          <Button size="sm" variant="outline" className="h-7 gap-1.5" onClick={onRun} disabled={running}>
          {running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
          <span className="text-xs">
            {t("dashboard.learning.runBenchmark", { defaultValue: "Run" })}
          </span>
          </Button>
        </div>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1.15fr)_minmax(220px,0.85fr)] gap-3">
            <div className="flex flex-wrap gap-1.5 content-start">
              <MetricPill label="RN" value={report.summary.totalRuns} />
              <MetricPill
                label="PR"
                value={formatPct(report.summary.runPassRate)}
                tone={report.summary.failedRuns > 0 ? "warn" : "accent"}
              />
              <MetricPill
                label="CS"
                value={formatPct(report.summary.casePassRate)}
                tone={report.summary.failedCases > 0 ? "warn" : "accent"}
              />
              <MetricPill
                label="EM"
                value={report.summary.externalModelRuns}
                tone={report.summary.externalModelRuns > 0 ? "accent" : "muted"}
              />
            </div>
            <div className="min-w-0 space-y-2">
              {recentRuns.length ? (
                recentRuns.map((run) => (
                  <div
                    key={run.id}
                    className="flex flex-wrap items-center gap-2 text-xs border-b border-border/20 pb-1.5 last:border-0 last:pb-0"
                  >
                    <span className={`px-1.5 py-0.5 rounded text-[10px] ${releaseGateTone(run.status)}`}>
                      {run.status}
                    </span>
                    <span className="font-medium truncate max-w-48">
                      {run.label ?? run.baselineKind}
                    </span>
                    <span className="text-muted-foreground tabular-nums">
                      {run.passedCases}/{run.passedCases + run.failedCases}
                    </span>
                    <span className="text-[10px] text-muted-foreground">
                      {new Date(run.createdAt).toLocaleDateString()}
                    </span>
                    {run.failedCasesSummary[0] && (
                      <span className="text-[10px] text-muted-foreground truncate basis-full">
                        {run.failedCasesSummary[0]}
                      </span>
                    )}
                  </div>
                ))
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noBenchmarkRuns", {
                    defaultValue: "No benchmark runs",
                  })}
                />
              )}
            </div>
            <div className="min-w-0 space-y-2">
              <div className="flex flex-wrap gap-1.5">
                {report.baselines.slice(0, 3).map((baseline) => (
                  <MetricPill
                    key={baseline.baselineKind}
                    label={baseline.baselineKind === "external_model" ? "EX" : "DT"}
                    value={`${baseline.passedRuns}/${baseline.runs}`}
                    tone={baseline.failedRuns > 0 ? "warn" : "accent"}
                  />
                ))}
              </div>
              {attentionChecks.length ? (
                <div className="flex flex-wrap gap-1.5">
                  {attentionChecks.map((check) => (
                    <span
                      key={check.name}
                      className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                      title={`${check.expected} · ${check.actual}`}
                    >
                      {check.name}: {check.actual}
                    </span>
                  ))}
                </div>
              ) : (
                <span className="text-[10px] text-muted-foreground">
                  {t("dashboard.learning.benchmarkClean", {
                    defaultValue: "Benchmark checks passed",
                  })}
                </span>
              )}
            </div>
          </div>

          <div className="border-t border-border/40 pt-3">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.modelLeaderboard", {
                  defaultValue: "Model leaderboard",
                })}
              </span>
              <span className={`rounded px-1.5 py-0.5 text-[10px] ${releaseGateTone(leaderboard?.status)}`}>
                {leaderboard?.status ?? "loading"}
              </span>
            </div>
            {leaderboard?.rows.length ? (
              <div className="space-y-1.5">
                {leaderboard.rows.slice(0, 6).map((row) => (
                  <div
                    key={`${row.rank}-${row.taskPackId}-${row.providerId ?? "det"}-${row.modelId ?? "det"}`}
                    className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 text-xs"
                  >
                    <span className="w-6 text-muted-foreground tabular-nums">#{row.rank}</span>
                    <div className="min-w-0">
                      <div className="truncate font-medium">{row.label}</div>
                      <div className="truncate text-[10px] text-muted-foreground">
                        {row.baselineKind} · {row.executionMode} · {row.taskPackId}
                      </div>
                    </div>
                    <div className="flex flex-wrap justify-end gap-1.5">
                      <MetricPill
                        label="CS"
                        value={formatPct(row.casePassRate)}
                        tone={row.failedCases > 0 ? "warn" : "accent"}
                      />
                      <MetricPill
                        label="IT"
                        value={`${row.passedItems}/${row.items}`}
                        tone={row.failedItems > 0 ? "warn" : "accent"}
                      />
                      {row.warnings.length > 0 && (
                        <span
                          className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700 dark:text-amber-300"
                          title={row.warnings.join(", ")}
                        >
                          !
                        </span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyLine
                label={t("dashboard.learning.noBenchmarkLeaderboard", {
                  defaultValue: "No comparable model rows",
                })}
              />
            )}
          </div>

          <div className="border-t border-border/40 pt-3">
            <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.externalCampaign", {
                  defaultValue: "External campaign",
                })}
              </span>
              <div className="flex flex-wrap items-center gap-2">
                <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
                  <span>Tasks</span>
                  <input
                    className="h-6 w-14 rounded border border-border bg-background px-1.5 text-xs tabular-nums"
                    type="number"
                    min={1}
                    max={20}
                    value={maxTasks}
                    onChange={(event) =>
                      onMaxTasksChange(Math.max(1, Math.min(20, Number(event.target.value) || 1)))
                    }
                  />
                </label>
                <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
                  <span>USD</span>
                  <input
                    className="h-6 w-20 rounded border border-border bg-background px-1.5 text-xs tabular-nums"
                    type="number"
                    min={0}
                    step="0.01"
                    value={budgetUsd}
                    onChange={(event) => onBudgetUsdChange(event.target.value)}
                  />
                </label>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 gap-1.5"
                  onClick={onRunExternal}
                  disabled={running || selectedModelKeys.length === 0}
                >
                  {running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                  <span className="text-xs">
                    {t("dashboard.learning.runExternalBenchmark", {
                      defaultValue: "Run external",
                    })}
                  </span>
                </Button>
              </div>
            </div>
            {visibleModels.length ? (
              <div className="flex flex-wrap gap-1.5">
                {visibleModels.map((option) => {
                  const selected = selectedModelKeys.includes(option.key)
                  return (
                    <button
                      key={option.key}
                      type="button"
                      onClick={() => onToggleModel(option.key)}
                      className={`max-w-full truncate rounded border px-1.5 py-0.5 text-[10px] ${
                        selected
                          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                          : "border-border/50 bg-secondary/30 text-muted-foreground"
                      }`}
                      title={`${option.providerName}/${option.modelName}`}
                    >
                      {option.providerName}/{option.modelName}
                    </button>
                  )
                })}
                {modelOptions.length > visibleModels.length && (
                  <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                    +{modelOptions.length - visibleModels.length}
                  </span>
                )}
              </div>
            ) : (
              <EmptyLine
                label={t("dashboard.learning.noBenchmarkModels", {
                  defaultValue: "No enabled provider models",
                })}
              />
            )}
          </div>

          <div className="border-t border-border/40 pt-3">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.benchmarkCampaigns", {
                  defaultValue: "Campaigns",
                })}
              </span>
              <span className="text-[10px] text-muted-foreground tabular-nums">
                {campaigns.length}
              </span>
            </div>
            {campaigns.length ? (
              <div className="space-y-2">
                {campaigns.slice(0, 6).map((campaign) => (
                  <BenchmarkCampaignRow
                    key={campaign.id}
                    campaign={campaign}
                    busy={actionId === campaign.id}
                    onCancel={onCancelCampaign}
                    onRetry={onRetryCampaign}
                  />
                ))}
              </div>
            ) : (
              <EmptyLine
                label={t("dashboard.learning.noBenchmarkCampaigns", {
                  defaultValue: "No benchmark campaigns",
                })}
              />
            )}
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.benchmarkLoading", {
            defaultValue: "Loading benchmark center",
          })}
        />
      )}
      {error && (
        <p className="mt-2 text-[10px] text-destructive line-clamp-2" title={error}>
          {t("dashboard.learning.benchmarkRunFailed", {
            defaultValue: "Run failed: {{message}}",
            message: error,
          })}
        </p>
      )}
    </div>
  )
}

function BenchmarkCampaignRow({
  campaign,
  busy,
  onCancel,
  onRetry,
}: {
  campaign: CodingBenchmarkCampaign
  busy: boolean
  onCancel: (campaignId: string) => void
  onRetry: (campaignId: string) => void
}) {
  const { t } = useTranslation()
  const canCancel = ["queued", "running", "cancel_requested"].includes(campaign.status)
  const canRetry = ["failed", "partial", "cancelled", "interrupted"].includes(campaign.status)
  const primaryItem = campaign.items[0]
  const visibleItems = campaign.items.slice(0, 4)

  return (
    <div className="rounded border border-border/40 p-2.5 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`px-1.5 py-0.5 rounded text-[10px] ${benchmarkCampaignTone(campaign.status)}`}>
          {campaign.status}
        </span>
        <span className="font-medium truncate max-w-[260px]">{campaign.name}</span>
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {new Date(campaign.updatedAt).toLocaleString()}
        </span>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
          <MetricPill
            label="IT"
            value={`${campaign.summary.passedItems}/${campaign.summary.totalItems}`}
            tone={campaign.summary.failedItems > 0 ? "warn" : campaign.summary.passedItems > 0 ? "accent" : "muted"}
          />
          <MetricPill
            label="CS"
            value={formatPct(campaign.summary.casePassRate)}
            tone={campaign.summary.failedCases > 0 ? "warn" : campaign.summary.passedCases > 0 ? "accent" : "muted"}
          />
          <MetricPill
            label="CK"
            value={campaign.summary.totalChecks}
            tone={campaign.summary.totalChecks > 0 ? "accent" : "muted"}
          />
          {canRetry && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5"
              onClick={() => onRetry(campaign.id)}
              disabled={busy}
              title={t("dashboard.learning.retryBenchmarkCampaign", {
                defaultValue: "Retry failed campaign items",
              })}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
            </Button>
          )}
          {canCancel && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-muted-foreground hover:text-destructive"
              onClick={() => onCancel(campaign.id)}
              disabled={busy}
              title={t("dashboard.learning.cancelBenchmarkCampaign", {
                defaultValue: "Cancel campaign",
              })}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <XCircle className="h-3.5 w-3.5" />}
            </Button>
          )}
        </div>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {visibleItems.map((item) => (
          <span
            key={item.id}
            className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${benchmarkCampaignTone(item.status)}`}
            title={item.error ?? item.packRunId ?? item.id}
          >
            {formatCampaignItemTarget(item.providerId, item.modelId, item.label)} · {item.status}
            {item.packRunId ? ` · ${item.packRunId}` : ""}
          </span>
        ))}
        {campaign.items.length > visibleItems.length && (
          <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            +{campaign.items.length - visibleItems.length}
          </span>
        )}
      </div>
      {(campaign.error || primaryItem?.error) && (
        <p className="mt-1.5 line-clamp-2 text-[10px] text-destructive" title={campaign.error ?? primaryItem?.error ?? undefined}>
          {campaign.error ?? primaryItem?.error}
        </p>
      )}
    </div>
  )
}

function DomainQualityDashboardPanel({
  dashboard,
}: {
  dashboard: DomainQualityDashboard | null
}) {
  const { t } = useTranslation()
  const overview = dashboard?.overview
  const timeline = dashboard?.timeline.slice(-8).reverse() ?? []
  const maxTimelineValue = Math.max(
    1,
    ...timeline.map(
      (point) =>
        point.qualityRuns +
        point.evalPassed +
        point.evalFailed +
        point.approvalBlockers +
        point.proposalsCreated,
    ),
  )

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainQualityTrends", {
              defaultValue: "General domain trends",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {t("dashboard.learning.domainQualityTrendsHint", {
              defaultValue: "Quality runs, blockers, evals, and learning candidates",
            })}
          </p>
        </div>
        <span className="rounded bg-secondary/40 px-2 py-1 text-[10px] text-muted-foreground">
          {t("dashboard.learning.domainsCovered", {
            defaultValue: "{{n}} domains",
            n: overview?.domainsCovered ?? 0,
          })}
        </span>
      </div>

      {dashboard ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-5 gap-2">
            <InsightCard
              icon={FileCheck2}
              label={t("dashboard.learning.domainQualityRuns", {
                defaultValue: "Quality",
              })}
              value={formatPct(overview?.qualityCompletionRate)}
              hint={`${overview?.completedQualityRuns ?? 0}/${overview?.qualityRuns ?? 0}`}
            />
            <InsightCard
              icon={ShieldAlert}
              label={t("dashboard.learning.domainQualityBlockers", {
                defaultValue: "Blockers",
              })}
              value={
                (overview?.blockedQualityRuns ?? 0) +
                (overview?.failedQualityRuns ?? 0) +
                (overview?.needsUserQualityRuns ?? 0)
              }
              hint={t("dashboard.learning.approvalBlockers", {
                defaultValue: "{{n}} approval",
                n: overview?.approvalBlockers ?? 0,
              })}
            />
            <InsightCard
              icon={CheckCircle2}
              label={t("dashboard.learning.domainEval", { defaultValue: "Domain eval" })}
              value={formatPct(overview?.evalPassRate)}
              hint={`${overview?.passedEvalRuns ?? 0}/${overview?.evalRuns ?? 0}`}
            />
            <InsightCard
              icon={Activity}
              label={t("dashboard.learning.domainScore", { defaultValue: "Score" })}
              value={overview?.averageEvalScore?.toFixed(2) ?? "—"}
              hint={t("dashboard.learning.averageEvalScore", {
                defaultValue: "average eval",
              })}
            />
            <InsightCard
              icon={Layers3}
              label={t("dashboard.learning.domainLearning", {
                defaultValue: "Learning",
              })}
              value={overview?.draftDomainProposals ?? 0}
              hint={t("dashboard.learning.domainPromotedHint", {
                defaultValue: "{{n}} promoted",
                n: overview?.promotedDomainProposals ?? 0,
              })}
            />
          </div>

          <div className="grid grid-cols-1 xl:grid-cols-[1.2fr_1fr] gap-3">
            <div className="rounded border border-border/50 p-3 min-w-0">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  {t("dashboard.learning.domainQualityByDomain", {
                    defaultValue: "By domain",
                  })}
                </span>
                <span className="text-[10px] text-muted-foreground">
                  {dashboard.byDomain.length}
                </span>
              </div>
              {dashboard.byDomain.length ? (
                <div className="space-y-2">
                  {dashboard.byDomain.slice(0, 6).map((domain) => (
                    <div
                      key={domain.domain}
                      className="grid grid-cols-[minmax(0,1fr)] gap-2 text-xs sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
                    >
                      <div className="min-w-0">
                        <div className="truncate font-medium">{domain.domain}</div>
                        <div className="truncate text-[10px] text-muted-foreground">
                          {domain.completedQualityRuns}/{domain.qualityRuns} quality ·{" "}
                          {domain.evalRuns} eval
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-1.5 sm:justify-end">
                        <MetricPill label="QL" value={formatPct(domain.qualityCompletionRate)} />
                        <MetricPill label="EV" value={formatPct(domain.evalPassRate)} />
                        <MetricPill
                          label="AP"
                          value={domain.approvalBlockers}
                          tone={domain.approvalBlockers > 0 ? "warn" : "muted"}
                        />
                        <MetricPill
                          label="DR"
                          value={domain.draftProposals}
                          tone={domain.draftProposals > 0 ? "accent" : "muted"}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noDomainQualityDomains", {
                    defaultValue: "No domain quality history",
                  })}
                />
              )}
            </div>

            <div className="rounded border border-border/50 p-3 min-w-0">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.domainQualityBlockerReasons", {
                  defaultValue: "Top blockers",
                })}
              </span>
              {dashboard.topBlockers.length ? (
                <div className="mt-2 space-y-2">
                  {dashboard.topBlockers.slice(0, 5).map((blocker) => (
                    <div
                      key={blocker.category}
                      className="flex items-center gap-2 border-b border-border/20 pb-2 text-xs last:border-0 last:pb-0"
                    >
                      <span className={`h-2 w-2 rounded-full ${severityDot(blocker.severity)}`} />
                      <span className="min-w-0 flex-1 truncate">{blocker.label}</span>
                      <span className="tabular-nums text-muted-foreground">{blocker.count}</span>
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noDomainQualityBlockers", {
                    defaultValue: "No domain quality blockers",
                  })}
                />
              )}
            </div>
          </div>

          <div className="grid grid-cols-1 xl:grid-cols-[1fr_1.2fr] gap-3">
            <div className="rounded border border-border/50 p-3 min-w-0">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.domainQualityTimeline", {
                  defaultValue: "Trend",
                })}
              </span>
              {timeline.length ? (
                <div className="mt-2 space-y-2">
                  {timeline.map((point) => {
                    const total =
                      point.qualityRuns +
                      point.evalPassed +
                      point.evalFailed +
                      point.approvalBlockers +
                      point.proposalsCreated
                    return (
                      <div
                        key={point.date}
                        className="grid grid-cols-[5.5rem_minmax(0,1fr)_auto] items-center gap-2 text-[10px]"
                      >
                        <span className="text-muted-foreground">{point.date}</span>
                        <div className="h-2 overflow-hidden rounded bg-secondary">
                          <div
                            className="h-full rounded bg-emerald-500/70"
                            style={{ width: `${Math.max(4, (total / maxTimelineValue) * 100)}%` }}
                          />
                        </div>
                        <span className="tabular-nums text-muted-foreground">{total}</span>
                      </div>
                    )
                  })}
                </div>
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noDomainQualityTimeline", {
                    defaultValue: "No domain trend yet",
                  })}
                />
              )}
            </div>

            <div className="rounded border border-border/50 p-3 min-w-0">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                {t("dashboard.learning.recentDomainQualityRuns", {
                  defaultValue: "Recent runs",
                })}
              </span>
              {dashboard.recentRuns.length ? (
                <div className="mt-2 grid grid-cols-1 lg:grid-cols-2 gap-2">
                  {dashboard.recentRuns.slice(0, 4).map((run) => (
                    <div key={run.id} className="rounded border border-border/40 p-2 text-xs min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate font-medium">{run.domain}</span>
                        <span
                          className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] ${domainQualityRunTone(run.state)}`}
                        >
                          {run.state}
                        </span>
                      </div>
                      <p className="mt-1 line-clamp-2 text-[10px] text-muted-foreground">
                        {run.summary || run.id}
                      </p>
                      <div className="mt-1 flex flex-wrap gap-1.5">
                        <MetricPill
                          label="F"
                          value={run.failedChecks}
                          tone={run.failedChecks > 0 ? "warn" : "muted"}
                        />
                        <MetricPill
                          label="U"
                          value={run.needsUserChecks}
                          tone={run.needsUserChecks > 0 ? "warn" : "muted"}
                        />
                        <MetricPill
                          label="A"
                          value={run.approvalBlockers}
                          tone={run.approvalBlockers > 0 ? "warn" : "muted"}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyLine
                  label={t("dashboard.learning.noRecentDomainQualityRuns", {
                    defaultValue: "No recent quality runs",
                  })}
                />
              )}
            </div>
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainQualityTrendsLoading", {
            defaultValue: "Loading general-domain trends",
          })}
        />
      )}
    </div>
  )
}

function DomainOperationalGatePanel({ report }: { report: DomainOperationalGateReport | null }) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 5) ?? []
  const recommendations = report?.recommendedNextSteps.slice(0, 3) ?? []
  const workflowBad = report
    ? report.summary.failedWorkflowRuns +
      report.summary.blockedWorkflowRuns +
      report.summary.cancelledWorkflowRuns
    : 0
  const campaignBad = report
    ? report.summary.failedCampaignItems +
      report.summary.cancelledCampaignItems +
      report.summary.interruptedCampaignItems
    : 0

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainOperationalGate", {
              defaultValue: "Domain operations",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {report
              ? t("dashboard.learning.domainOperationalGateHint", {
                  defaultValue: "{{scope}} · since {{since}}",
                  scope: report.scope,
                  since: new Date(report.since).toLocaleDateString(),
                })
              : t("dashboard.learning.domainOperationalGateLoadingHint", {
                  defaultValue: "Loading operational evidence",
                })}
          </p>
        </div>
        <span className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}>
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-6 gap-2">
            <MetricPill
              label="WF"
              value={`${report.summary.completedWorkflowRuns}/${report.summary.workflowRuns}`}
              tone={workflowBad > 0 ? "warn" : report.summary.completedWorkflowRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="BAD"
              value={workflowBad}
              tone={workflowBad > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="ACT"
              value={report.summary.activeWorkflowRuns}
              tone={report.summary.activeWorkflowRuns > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="LP"
              value={`${report.summary.succeededLoopRuns}/${report.summary.loopRuns}`}
              tone={report.summary.failedLoopRuns > 0 ? "warn" : report.summary.loopRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="CP"
              value={`${report.summary.passedCampaignItems}/${report.summary.campaignItems}`}
              tone={campaignBad > 0 ? "warn" : report.summary.passedCampaignItems > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="RUN"
              value={report.summary.activeCampaigns}
              tone={report.summary.activeCampaigns > 0 ? "warn" : "muted"}
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground">
              {t("dashboard.learning.domainOperationalClean", {
                defaultValue: "Operational checks passed",
              })}
            </span>
          )}
          {recommendations.length > 0 && (
            <div className="space-y-1">
              {recommendations.map((step) => (
                <div key={step} className="flex items-start gap-1.5 text-[10px] text-muted-foreground">
                  <Activity className="mt-0.5 h-3 w-3 shrink-0" />
                  <span className="min-w-0">{step}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainOperationalLoading", {
            defaultValue: "Loading operational readiness",
          })}
        />
      )}
    </div>
  )
}

function DomainSoakReportPanel({ report }: { report: DomainSoakReport | null }) {
  const { t } = useTranslation()
  const incidents = report?.incidents.slice(0, 4) ?? []
  const timeline = report?.timeline.slice(0, 4) ?? []
  const recommendations = report?.recommendedNextSteps.slice(0, 3) ?? []
  const workflowBad = report
    ? report.summary.failedWorkflowRuns +
      report.summary.blockedWorkflowRuns +
      report.summary.cancelledWorkflowRuns
    : 0
  const campaignBad = report
    ? report.summary.failedCampaignItems +
      report.summary.cancelledCampaignItems +
      report.summary.interruptedCampaignItems
    : 0

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainSoakReport", {
              defaultValue: "Domain soak report",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {report
              ? t("dashboard.learning.domainSoakReportHint", {
                  defaultValue: "{{scope}} · {{days}}d · {{records}} records",
                  scope: report.scope,
                  days: report.windowDays,
                  records: report.summary.totalRecords,
                })
              : t("dashboard.learning.domainSoakReportLoadingHint", {
                  defaultValue: "Loading long-run evidence",
                })}
          </p>
        </div>
        <span className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}>
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-8 gap-2">
            <MetricPill
              label="WF"
              value={`${report.summary.completedWorkflowRuns}/${report.summary.workflowRuns}`}
              tone={workflowBad > 0 ? "warn" : report.summary.completedWorkflowRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="LP"
              value={`${report.summary.succeededLoopRuns}/${report.summary.loopRuns}`}
              tone={report.summary.failedLoopRuns > 0 ? "warn" : report.summary.loopRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="CP"
              value={`${report.summary.passedCampaignItems}/${report.summary.campaignItems}`}
              tone={campaignBad > 0 ? "warn" : report.summary.passedCampaignItems > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="CE"
              value={report.summary.connectorE2eEvidence}
              tone={report.summary.connectorVerificationEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="CR"
              value={report.summary.criticalIncidents}
              tone={report.summary.criticalIncidents > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="WN"
              value={report.summary.warningIncidents}
              tone={report.summary.warningIncidents > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="MAX"
              value={formatSecs(report.summary.maxWorkflowDrainSecs)}
              tone="muted"
            />
            <MetricPill
              label="REC"
              value={report.summary.totalRecords}
              tone={report.summary.totalRecords > 0 ? "accent" : "muted"}
            />
          </div>

          {incidents.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {incidents.map((incident) => (
                <span
                  key={`${incident.source}:${incident.id}`}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${
                    incident.severity === "critical"
                      ? releaseGateCheckTone("failed")
                      : releaseGateCheckTone("insufficient_data")
                  }`}
                  title={`${incident.reason} · ${incident.recommendation}`}
                >
                  {incident.source}: {incident.status}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground">
              {t("dashboard.learning.domainSoakClean", {
                defaultValue: "No soak incidents in this window",
              })}
            </span>
          )}

          {timeline.length > 0 && (
            <div className="grid gap-1">
              {timeline.map((item) => (
                <div
                  key={`${item.source}:${item.id}`}
                  className="flex items-center justify-between gap-2 text-[10px] text-muted-foreground"
                >
                  <span className="min-w-0 truncate">
                    {item.source} · {item.label}
                  </span>
                  <span className="shrink-0 tabular-nums">
                    {new Date(item.at).toLocaleString()}
                  </span>
                </div>
              ))}
            </div>
          )}

          {recommendations.length > 0 && (
            <div className="space-y-1">
              {recommendations.map((step) => (
                <div key={step} className="flex items-start gap-1.5 text-[10px] text-muted-foreground">
                  <FileCheck2 className="mt-0.5 h-3 w-3 shrink-0" />
                  <span className="min-w-0">{step}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainSoakLoading", {
            defaultValue: "Loading soak report",
          })}
        />
      )}
    </div>
  )
}

function DomainConnectorE2EGatePanel({ report }: { report: DomainConnectorE2EGateReport | null }) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 5) ?? []
  const recommendations = report?.recommendedNextSteps.slice(0, 3) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainConnectorE2EGate", {
              defaultValue: "Connector E2E",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {report
              ? t("dashboard.learning.domainConnectorE2EGateHint", {
                  defaultValue: "{{scope}} · {{connector}} · {{action}}",
                  scope: report.scope.scope,
                  connector: report.connector ?? "connector",
                  action: report.action ?? "action",
                })
              : t("dashboard.learning.domainConnectorE2EGateLoadingHint", {
                  defaultValue: "Loading connector evidence",
                })}
          </p>
        </div>
        <span className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}>
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-7 gap-2">
            <MetricPill
              label="IN"
              value={report.summary.connectorInputEvidence}
              tone={report.summary.connectorInputEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="DR"
              value={report.summary.draftEvidence}
              tone={report.summary.draftEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="OK"
              value={report.summary.approvalEvidence}
              tone={report.summary.approvalEvidence > 0 ? "accent" : "warn"}
            />
            <MetricPill
              label="EX"
              value={report.summary.executionEvidence}
              tone={report.summary.executionEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="VF"
              value={report.summary.verificationEvidence}
              tone={report.summary.verificationEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="RB"
              value={report.summary.rollbackEvidence}
              tone={report.summary.rollbackEvidence > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="GU"
              value={report.summary.connectorActionGuardStatus ?? "n/a"}
              tone={
                report.summary.connectorActionGuardStatus === "passed"
                  ? "accent"
                  : report.summary.connectorActionGuardStatus === "failed"
                    ? "warn"
                    : "muted"
              }
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground">
              {t("dashboard.learning.domainConnectorE2EClean", {
                defaultValue: "Connector E2E checks passed",
              })}
            </span>
          )}
          {recommendations.length > 0 && (
            <div className="space-y-1">
              {recommendations.map((step) => (
                <div key={step} className="flex items-start gap-1.5 text-[10px] text-muted-foreground">
                  <ShieldAlert className="mt-0.5 h-3 w-3 shrink-0" />
                  <span className="min-w-0">{step}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainConnectorE2ELoading", {
            defaultValue: "Loading connector E2E evidence",
          })}
        />
      )}
    </div>
  )
}

function DomainReadinessGatePanel({ report }: { report: DomainReadinessGateReport | null }) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 5) ?? []
  const recommendations = report?.recommendedNextSteps.slice(0, 3) ?? []
  const campaignFailures = report
    ? report.summary.failedCampaignItems +
      report.summary.cancelledCampaignItems +
      report.summary.interruptedCampaignItems
    : 0

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainReadinessGate", {
              defaultValue: "Domain readiness",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {report
              ? t("dashboard.learning.domainReadinessGateHint", {
                  defaultValue: "{{scope}} · since {{since}}",
                  scope: report.scope,
                  since: new Date(report.since).toLocaleDateString(),
                })
              : t("dashboard.learning.domainReadinessGateLoadingHint", {
                  defaultValue: "Loading readiness evidence",
                })}
          </p>
        </div>
        <span className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}>
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-6 gap-2">
            <MetricPill
              label="QG"
              value={report.summary.qualityStatus}
              tone={report.summary.qualityStatus === "passed" ? "accent" : "warn"}
            />
            <MetricPill label="EV" value={`${report.summary.evalRuns}/${report.summary.qualityRuns}`} />
            <MetricPill
              label="CP"
              value={`${report.summary.passedCampaignItems}/${report.summary.campaignItems}`}
              tone={campaignFailures > 0 ? "warn" : report.summary.passedCampaignItems > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="LD"
              value={report.summary.leaderboardRows}
              tone={report.summary.leaderboardStatus === "passed" ? "accent" : "muted"}
            />
            <MetricPill
              label="FL"
              value={campaignFailures}
              tone={campaignFailures > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="LP"
              value={report.summary.openLearningProposals + report.summary.pendingLearningCampaigns}
              tone={
                report.summary.openLearningProposals + report.summary.pendingLearningCampaigns > 0
                  ? "warn"
                  : "muted"
              }
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground">
              {t("dashboard.learning.domainReadinessClean", {
                defaultValue: "Domain readiness checks passed",
              })}
            </span>
          )}
          {recommendations.length > 0 && (
            <div className="space-y-1">
              {recommendations.map((step) => (
                <div key={step} className="flex items-start gap-1.5 text-[10px] text-muted-foreground">
                  <ShieldAlert className="mt-0.5 h-3 w-3 shrink-0" />
                  <span className="min-w-0">{step}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainReadinessLoading", {
            defaultValue: "Loading domain readiness",
          })}
        />
      )}
    </div>
  )
}

function DomainQualityGatePanel({
  report,
  runs,
  taskCount,
  calibratedTaskCount,
  calibrationActionId,
  onRecordCalibration,
}: {
  report: DomainQualityGateReport | null
  runs: DomainEvalRunRecord[]
  taskCount: number
  calibratedTaskCount: number
  calibrationActionId: string | null
  onRecordCalibration: (run: DomainEvalRunRecord) => void
}) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainQualityGate", {
              defaultValue: "General domain quality",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {t("dashboard.learning.domainQualityGateHint", {
              defaultValue: "{{tasks}} eval tasks · {{calibrated}} calibrated",
              tasks: taskCount,
              calibrated: calibratedTaskCount,
            })}
          </p>
        </div>
        <span
          className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
        >
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-5 gap-2">
            <MetricPill label="EV" value={`${report.summary.passedEvalRuns}/${report.summary.evalRuns}`} />
            <MetricPill label="PR" value={formatPct(report.summary.passRate)} />
            <MetricPill label="SC" value={report.summary.averageScore?.toFixed(2) ?? "n/a"} />
            <MetricPill
              label="QB"
              value={
                report.summary.blockedQualityRuns +
                report.summary.failedQualityRuns +
                report.summary.needsUserQualityRuns
              }
              tone={
                report.summary.blockedQualityRuns +
                  report.summary.failedQualityRuns +
                  report.summary.needsUserQualityRuns >
                0
                  ? "warn"
                  : "muted"
              }
            />
            <MetricPill label="DM" value={report.summary.domainsCovered} />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground">
              {t("dashboard.learning.domainQualityClean", {
                defaultValue: "All general-domain checks passed",
              })}
            </span>
          )}
          {runs.length ? (
            <div className="grid grid-cols-1 lg:grid-cols-3 gap-2">
              {runs.slice(0, 3).map((run) => (
                <div key={run.id} className="rounded border border-border/50 p-2 min-w-0">
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-xs font-medium">{run.label}</span>
                    <span
                      className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(run.status)}`}
                    >
                      {run.status}
                    </span>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                    <span>{run.domain}</span>
                    <span>{run.score.toFixed(2)}</span>
                    <span>{new Date(run.createdAt).toLocaleDateString()}</span>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    className="mt-2 h-7 w-full gap-1.5 text-[11px]"
                    onClick={() => onRecordCalibration(run)}
                    disabled={calibrationActionId === run.id}
                  >
                    {calibrationActionId === run.id ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <CheckCircle2 className="h-3.5 w-3.5" />
                    )}
                    {t("dashboard.learning.recordDomainCalibration", {
                      defaultValue: "Mark reviewed",
                    })}
                  </Button>
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine
              label={t("dashboard.learning.noDomainEvalRuns", {
                defaultValue: "No general-domain eval runs",
              })}
            />
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.domainQualityLoading", {
            defaultValue: "Loading general-domain quality gate",
          })}
        />
      )}
    </div>
  )
}

function DomainEvalCampaignPanel({
  campaigns,
  leaderboard,
  modelOptions,
  selectedModelKeys,
  maxTasks,
  budgetUsd,
  actionId,
  error,
  onRun,
  onRunExternal,
  onGenerateLearning,
  onToggleModel,
  onMaxTasksChange,
  onBudgetUsdChange,
  onCancel,
  onRetry,
}: {
  campaigns: DomainEvalCampaign[]
  leaderboard: DomainEvalCampaignLeaderboardReport | null
  modelOptions: BenchmarkModelOption[]
  selectedModelKeys: string[]
  maxTasks: number
  budgetUsd: string
  actionId: string | null
  error: string | null
  onRun: () => void
  onRunExternal: () => void
  onGenerateLearning: (campaign: DomainEvalCampaign) => void
  onToggleModel: (key: string) => void
  onMaxTasksChange: (value: number) => void
  onBudgetUsdChange: (value: string) => void
  onCancel: (campaignId: string) => void
  onRetry: (campaignId: string) => void
}) {
  const { t } = useTranslation()
  const activeCampaigns = campaigns.filter((campaign) =>
    ["queued", "running", "cancel_requested"].includes(campaign.status),
  ).length
  const latest = campaigns[0]
  const visibleModels = modelOptions.slice(0, 10)

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainCampaigns", {
              defaultValue: "Domain campaigns",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {t("dashboard.learning.domainCampaignsHint", {
              defaultValue: "Batch non-coding eval packs with durable status, retry and cancellation",
            })}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {activeCampaigns > 0 && (
            <span className="text-[10px] text-muted-foreground tabular-nums">
              {activeCampaigns} active
            </span>
          )}
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5"
            onClick={onRun}
            disabled={actionId === "new"}
          >
            {actionId === "new" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            <span className="text-xs">
              {t("dashboard.learning.runDomainCampaign", {
                defaultValue: "Run trace pack",
              })}
            </span>
          </Button>
        </div>
      </div>
      {latest ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-5 gap-2">
            <MetricPill label="CP" value={campaigns.length} />
            <MetricPill
              label="IT"
              value={`${latest.summary.passedItems}/${latest.summary.totalItems}`}
              tone={latest.summary.failedItems > 0 ? "warn" : "accent"}
            />
            <MetricPill
              label="PR"
              value={formatPct(latest.summary.itemPassRate)}
              tone={latest.summary.failedItems > 0 ? "warn" : "accent"}
            />
            <MetricPill label="SC" value={formatScore(latest.summary.averageScore)} />
            <MetricPill
              label="CK"
              value={latest.summary.totalChecks}
              tone={latest.summary.failedChecks > 0 ? "warn" : "accent"}
            />
          </div>
          <div className="space-y-2">
            {campaigns.slice(0, 6).map((campaign) => (
              <DomainEvalCampaignRow
                key={campaign.id}
                campaign={campaign}
                busy={actionId === campaign.id || actionId === `learn:${campaign.id}`}
                learningBusy={actionId === `learn:${campaign.id}`}
                onCancel={onCancel}
                onRetry={onRetry}
                onGenerateLearning={onGenerateLearning}
              />
            ))}
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.noDomainCampaigns", {
            defaultValue: "No domain eval campaigns",
          })}
        />
      )}
      <div className="mt-3 border-t border-border/40 pt-3">
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainExternalCampaign", {
              defaultValue: "External domain campaign",
            })}
          </span>
          <div className="flex flex-wrap items-center gap-2">
            <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
              <span>Tasks</span>
              <input
                className="h-6 w-14 rounded border border-border bg-background px-1.5 text-xs tabular-nums"
                type="number"
                min={1}
                max={15}
                value={maxTasks}
                onChange={(event) =>
                  onMaxTasksChange(Math.max(1, Math.min(15, Number(event.target.value) || 1)))
                }
              />
            </label>
            <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
              <span>USD</span>
              <input
                className="h-6 w-20 rounded border border-border bg-background px-1.5 text-xs tabular-nums"
                type="number"
                min={0}
                step="0.01"
                value={budgetUsd}
                onChange={(event) => onBudgetUsdChange(event.target.value)}
              />
            </label>
            <Button
              size="sm"
              variant="outline"
              className="h-7 gap-1.5"
              onClick={onRunExternal}
              disabled={actionId === "external" || selectedModelKeys.length === 0}
            >
              {actionId === "external" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
              <span className="text-xs">
                {t("dashboard.learning.runExternalDomainCampaign", {
                  defaultValue: "Run external",
                })}
              </span>
            </Button>
          </div>
        </div>
        {visibleModels.length ? (
          <div className="flex flex-wrap gap-1.5">
            {visibleModels.map((option) => {
              const selected = selectedModelKeys.includes(option.key)
              return (
                <button
                  key={option.key}
                  type="button"
                  onClick={() => onToggleModel(option.key)}
                  className={`max-w-full truncate rounded border px-1.5 py-0.5 text-[10px] ${
                    selected
                      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                      : "border-border/50 bg-secondary/30 text-muted-foreground"
                  }`}
                  title={`${option.providerName}/${option.modelName}`}
                >
                  {option.providerName}/{option.modelName}
                </button>
              )
            })}
            {modelOptions.length > visibleModels.length && (
              <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                +{modelOptions.length - visibleModels.length}
              </span>
            )}
          </div>
        ) : (
          <EmptyLine
            label={t("dashboard.learning.noDomainCampaignModels", {
              defaultValue: "No enabled provider models",
            })}
          />
        )}
      </div>
      <DomainCampaignLeaderboard leaderboard={leaderboard} />
      {error && (
        <p className="mt-2 line-clamp-2 text-[10px] text-destructive" title={error}>
          {t("dashboard.learning.domainCampaignFailed", {
            defaultValue: "Domain campaign failed: {{message}}",
            message: error,
          })}
        </p>
      )}
    </div>
  )
}

function DomainCampaignLeaderboard({
  leaderboard,
}: {
  leaderboard: DomainEvalCampaignLeaderboardReport | null
}) {
  const { t } = useTranslation()
  return (
    <div className="mt-3 border-t border-border/40 pt-3">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.domainModelLeaderboard", {
            defaultValue: "Domain model leaderboard",
          })}
        </span>
        <span className={`rounded px-1.5 py-0.5 text-[10px] ${releaseGateTone(leaderboard?.status)}`}>
          {leaderboard?.status ?? "loading"}
        </span>
      </div>
      {leaderboard?.rows.length ? (
        <div className="space-y-1.5">
          {leaderboard.rows.slice(0, 6).map((row) => (
            <div
              key={`${row.rank}-${row.executionMode}-${row.providerId ?? "trace"}-${row.modelId ?? "fixture"}`}
              className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 text-xs"
            >
              <span className="w-6 text-muted-foreground tabular-nums">#{row.rank}</span>
              <div className="min-w-0">
                <div className="truncate font-medium">{row.label}</div>
                <div className="truncate text-[10px] text-muted-foreground">
                  {row.executionMode} · {row.domains.slice(0, 3).join(", ") || "domain"} · {row.evidence.length} traces
                </div>
              </div>
              <div className="flex flex-wrap justify-end gap-1.5">
                <MetricPill
                  label="SC"
                  value={formatScore(row.averageScore)}
                  tone={row.failedItems > 0 ? "warn" : "accent"}
                />
                <MetricPill
                  label="IT"
                  value={`${row.passedItems}/${row.items}`}
                  tone={row.failedItems > 0 ? "warn" : "accent"}
                />
                {row.warnings.length > 0 && (
                  <span
                    className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700 dark:text-amber-300"
                    title={row.warnings.join(", ")}
                  >
                    !
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.noDomainLeaderboard", {
            defaultValue: "No comparable domain campaign rows",
          })}
        />
      )}
    </div>
  )
}

function DomainEvalCampaignRow({
  campaign,
  busy,
  learningBusy,
  onCancel,
  onRetry,
  onGenerateLearning,
}: {
  campaign: DomainEvalCampaign
  busy: boolean
  learningBusy: boolean
  onCancel: (campaignId: string) => void
  onRetry: (campaignId: string) => void
  onGenerateLearning: (campaign: DomainEvalCampaign) => void
}) {
  const { t } = useTranslation()
  const canCancel = ["queued", "running", "cancel_requested"].includes(campaign.status)
  const canRetry = ["failed", "partial", "cancelled", "interrupted"].includes(campaign.status)
  const canGenerateLearning =
    Boolean(campaign.sessionId) &&
    campaign.items.some((item) => ["failed", "cancelled", "interrupted"].includes(item.status))
  const visibleItems = campaign.items.slice(0, 5)
  const primaryItem = campaign.items[0]

  return (
    <div className="rounded border border-border/40 p-2.5 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`px-1.5 py-0.5 rounded text-[10px] ${benchmarkCampaignTone(campaign.status)}`}>
          {campaign.status}
        </span>
        <span className="font-medium truncate max-w-[260px]">{campaign.name}</span>
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {new Date(campaign.updatedAt).toLocaleString()}
        </span>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
          <MetricPill
            label="IT"
            value={`${campaign.summary.passedItems}/${campaign.summary.totalItems}`}
            tone={campaign.summary.failedItems > 0 ? "warn" : campaign.summary.passedItems > 0 ? "accent" : "muted"}
          />
          <MetricPill
            label="SC"
            value={formatScore(campaign.summary.averageScore)}
            tone={campaign.summary.failedItems > 0 ? "warn" : "accent"}
          />
          <MetricPill
            label="CK"
            value={campaign.summary.totalChecks}
            tone={campaign.summary.failedChecks > 0 ? "warn" : "accent"}
          />
          {canGenerateLearning && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5"
              onClick={() => onGenerateLearning(campaign)}
              disabled={busy}
              title={t("dashboard.learning.generateDomainCampaignLearning", {
                defaultValue: "Create learning drafts from this domain campaign",
              })}
            >
              {learningBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Sparkles className="h-3.5 w-3.5" />}
            </Button>
          )}
          {canRetry && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5"
              onClick={() => onRetry(campaign.id)}
              disabled={busy}
              title={t("dashboard.learning.retryDomainCampaign", {
                defaultValue: "Retry failed domain campaign items",
              })}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
            </Button>
          )}
          {canCancel && (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-muted-foreground hover:text-destructive"
              onClick={() => onCancel(campaign.id)}
              disabled={busy}
              title={t("dashboard.learning.cancelDomainCampaign", {
                defaultValue: "Cancel domain campaign",
              })}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <XCircle className="h-3.5 w-3.5" />}
            </Button>
          )}
        </div>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {visibleItems.map((item) => (
          <span
            key={item.id}
            className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${benchmarkCampaignTone(item.status)}`}
            title={item.error ?? item.fixtureRunId ?? item.evalRunId ?? item.id}
          >
            {item.taskId} · {formatCampaignItemTarget(item.providerId, item.modelId, item.label)} · {item.status}
            {typeof item.score === "number" ? ` · ${formatScore(item.score)}` : ""}
          </span>
        ))}
        {campaign.items.length > visibleItems.length && (
          <span className="rounded bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            +{campaign.items.length - visibleItems.length}
          </span>
        )}
      </div>
      {(campaign.error || primaryItem?.error) && (
        <p className="mt-1.5 line-clamp-2 text-[10px] text-destructive" title={campaign.error ?? primaryItem?.error ?? undefined}>
          {campaign.error ?? primaryItem?.error}
        </p>
      )}
    </div>
  )
}

function DomainFixtureSmokePanel({ runs }: { runs: DomainEvalFixtureRunRecord[] }) {
  const { t } = useTranslation()
  const total = runs.length
  const passed = runs.filter((run) => run.passed).length
  const failed = runs.filter((run) => !run.passed || run.status !== "passed").length
  const agentRuns = runs.filter((run) => run.executionMode === "agent").length
  const traceRuns = runs.filter((run) => run.executionMode === "trace_fixture").length
  const passRate = total > 0 ? passed / total : null

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.domainSmokeCenter", {
              defaultValue: "Domain smoke runs",
            })}
          </h4>
          <p className="text-[10px] text-muted-foreground">
            {t("dashboard.learning.domainSmokeCenterHint", {
              defaultValue: "Synthetic trace and agent fixture runs are isolated from the live quality gate",
            })}
          </p>
        </div>
        <span className={`px-2 py-1 rounded text-[10px] font-medium ${failed > 0 ? "bg-red-500/10 text-red-600" : "bg-emerald-500/10 text-emerald-600"}`}>
          {total ? `${passed}/${total}` : "none"}
        </span>
      </div>
      {runs.length ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 md:grid-cols-5 gap-2">
            <MetricPill label="SR" value={total} />
            <MetricPill label="PR" value={formatPct(passRate)} tone={failed > 0 ? "warn" : "accent"} />
            <MetricPill label="AG" value={agentRuns} />
            <MetricPill label="TR" value={traceRuns} />
            <MetricPill label="FL" value={failed} tone={failed > 0 ? "warn" : "muted"} />
          </div>
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-2">
            {runs.slice(0, 6).map((run) => (
              <div key={run.id} className="rounded border border-border/50 p-2 min-w-0">
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-xs font-medium">{run.name}</span>
                  <span
                    className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(run.status)}`}
                  >
                    {run.status}
                  </span>
                </div>
                <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                  <span>{domainFixtureModeLabel(run.executionMode)}</span>
                  <span>{run.sourceType}</span>
                  <span>{new Date(run.createdAt).toLocaleDateString()}</span>
                </div>
                <div className="mt-2 flex flex-wrap gap-1">
                  <TraceBadge active={Boolean(run.evalRunId)} label="eval" />
                  <TraceBadge active={Boolean(run.qualityRunId)} label="quality" />
                  <TraceBadge active={Boolean(run.workflowRunId)} label="workflow" />
                  <TraceBadge active={Boolean(run.report.execution?.turnId)} label="turn" />
                </div>
                {run.error ? (
                  <p className="mt-2 truncate text-[10px] text-red-600" title={run.error}>
                    {run.error}
                  </p>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.noDomainSmokeRuns", {
            defaultValue: "No synthetic domain smoke runs",
          })}
        />
      )}
    </div>
  )
}

function TraceBadge({ active, label }: { active: boolean; label: string }) {
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] ${
        active ? "bg-emerald-500/10 text-emerald-600" : "bg-muted text-muted-foreground"
      }`}
    >
      {label}
    </span>
  )
}

function domainFixtureModeLabel(mode: string): string {
  if (mode === "trace_fixture") return "trace"
  if (mode === "agent") return "agent"
  return mode
}

function ReleaseGatePanel({ report }: { report: CodingEvalReleaseGateReport | null }) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.releaseGate", { defaultValue: "Release gate" })}
        </h4>
        <span
          className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
        >
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1fr)] gap-3">
          <div className="flex flex-wrap gap-1.5">
            <MetricPill label="PK" value={formatPct(report.summary.packPassRate)} />
            <MetricPill
              label="ST"
              value={report.summary.regressedStrategyEffects}
              tone={report.summary.regressedStrategyEffects > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="TC"
              value={report.summary.missingToolCallRuns}
              tone={report.summary.missingToolCallRuns > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="EX"
              value={report.summary.externalModelPackRuns}
              tone={
                report.thresholds.requireExternalModelPack &&
                report.summary.externalModelPackRuns === 0
                  ? "warn"
                  : "muted"
              }
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5 min-w-0 xl:justify-end">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground xl:text-right">
              {t("dashboard.learning.releaseGateClean", { defaultValue: "All checks passed" })}
            </span>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.releaseGateLoading", {
            defaultValue: "Loading release gate",
          })}
        />
      )}
    </div>
  )
}

function GeneralizationPanel({
  report,
}: {
  report: CodingLearningGeneralizationReport | null
}) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.generalizationGate", {
            defaultValue: "Generalization gate",
          })}
        </h4>
        <span
          className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
        >
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1fr)] gap-3">
          <div className="flex flex-wrap gap-1.5">
            <MetricPill label="PR" value={`${report.summary.passedProjects}/${report.summary.projectsEvaluated}`} />
            <MetricPill
              label="LR"
              value={report.summary.totalPromotedLearning}
              tone={report.summary.totalPromotedLearning > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="PK"
              value={report.summary.totalPackRuns}
              tone={report.summary.projectsWithPackRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="RG"
              value={report.summary.regressedProjects}
              tone={report.summary.regressedProjects > 0 ? "warn" : "muted"}
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5 min-w-0 xl:justify-end">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground xl:text-right">
              {t("dashboard.learning.generalizationClean", {
                defaultValue: "Cross-project checks passed",
              })}
            </span>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.generalizationLoading", {
            defaultValue: "Loading generalization gate",
          })}
        />
      )}
    </div>
  )
}

function InsightCard({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: LucideIcon
  label: string
  value: string | number
  hint?: string
}) {
  return (
    <div className="border border-border/60 rounded-lg p-3 flex flex-col gap-2 min-w-0">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{label}</span>
      </div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[10px] text-muted-foreground truncate">{hint}</div>}
    </div>
  )
}

function ProjectSignalRow({
  name,
  projectId,
  workflowRate,
  evalRate,
  packRate,
  strategyRegressions,
  blockers,
  candidates,
}: {
  name: string
  projectId: string | null
  workflowRate: number | null
  evalRate: number | null
  packRate: number | null
  strategyRegressions: number
  blockers: number
  candidates: number
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2 text-xs sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
      <div className="min-w-0">
        <div className="font-medium truncate">{name}</div>
        {projectId && <div className="text-[10px] text-muted-foreground truncate">{projectId}</div>}
      </div>
      <div className="flex flex-wrap gap-1.5 sm:justify-end">
        <MetricPill label="WF" value={formatPct(workflowRate)} />
        <MetricPill label="EV" value={formatPct(evalRate)} />
        <MetricPill label="PK" value={formatPct(packRate)} />
        <MetricPill
          label="ST"
          value={strategyRegressions}
          tone={strategyRegressions > 0 ? "warn" : "muted"}
        />
        <MetricPill label="B" value={blockers} tone={blockers > 0 ? "warn" : "muted"} />
        <MetricPill label="Q" value={candidates} tone={candidates > 0 ? "accent" : "muted"} />
      </div>
    </div>
  )
}

function MetricPill({
  label,
  value,
  tone = "muted",
}: {
  label: string
  value: string | number
  tone?: "muted" | "warn" | "accent"
}) {
  const toneClass =
    tone === "warn"
      ? "bg-red-500/10 text-red-600 dark:text-red-400"
      : tone === "accent"
        ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
        : "bg-secondary/40 text-muted-foreground"
  return (
    <span className={`inline-flex min-w-12 justify-center rounded px-1.5 py-0.5 tabular-nums ${toneClass}`}>
      {label}:{value}
    </span>
  )
}

function EmptyLine({ label }: { label: string }) {
  return <div className="text-xs text-muted-foreground text-center py-6">{label}</div>
}

function formatPct(value: number | null | undefined): string {
  return typeof value === "number" ? `${Math.round(value * 100)}%` : "—"
}

function formatScore(value: number | null | undefined): string {
  return typeof value === "number" ? value.toFixed(3) : "—"
}

function formatSecs(value: number | null | undefined): string {
  if (typeof value !== "number") return "—"
  if (value < 60) return `${Math.round(value)}s`
  if (value < 3600) return `${Math.round(value / 60)}m`
  return `${(value / 3600).toFixed(1)}h`
}

function formatSignedPct(value: number): string {
  const pct = Math.round(value * 100)
  return `${pct > 0 ? "+" : ""}${pct}%`
}

function formatSignedCount(value: number): string {
  return `${value > 0 ? "+" : ""}${value}`
}

function deltaTone(value: number): "muted" | "warn" | "accent" {
  if (value > 0) return "accent"
  if (value < 0) return "warn"
  return "muted"
}

function inverseDeltaTone(value: number): "muted" | "warn" | "accent" {
  if (value < 0) return "accent"
  if (value > 0) return "warn"
  return "muted"
}

function releaseGateTone(status?: string | null): string {
  switch (status) {
    case "passed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "failed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    case "insufficient_data":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function sampleBenchmarkTaskPackManifest(): CodingBenchmarkTaskPackManifest {
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
  const calibratedAt = new Date().toISOString()
  return {
    packId: "sample-real-project-regression",
    version: `v${stamp}`,
    name: "Sample real project regression pack",
    description: "Curated sample manifest for benchmark corpus management.",
    status: "draft",
    sourceKind: "fixture_repo",
    sourceUri: "local://hope-agent/examples/benchmark-corpus/sample-real-project-regression",
    repoTemplate: "fixture://react-rust-desktop-app",
    licenseNote: "Synthetic sample manifest bundled for local corpus validation.",
    privacyNote: "No user repository files are read or uploaded by this import.",
    redactionStatus: "not_required",
    tasks: [
      {
        taskId: "SAMPLE-BUGFIX-001",
        version: "v1",
        title: "Repair stale async benchmark status rendering",
        status: "active",
        taskType: "bugfix",
        difficulty: "medium",
        language: "typescript",
        framework: "react",
        sourceUri: "local://hope-agent/examples/benchmark-corpus/sample-real-project-regression/issues/bugfix-001",
        repoTemplate: "fixture://react-rust-desktop-app",
        tags: ["dashboard", "async-state"],
        successCriteria: [
          "Running and completed states render without stale action buttons.",
          "The UI keeps campaign status, item counts and retry action in sync after reload.",
        ],
        validationCommands: ["pnpm typecheck"],
        allowedPaths: ["src/components/dashboard/**", "src/lib/**"],
        forbiddenPaths: ["src-tauri/**", "crates/**"],
        calibrationNotes: ["Calibrated from dashboard state-management regressions."],
        calibratedAt,
        licenseNote: "Synthetic local fixture.",
        privacyNote: "No private source content.",
        redactionStatus: "not_required",
      },
      {
        taskId: "SAMPLE-FEATURE-002",
        version: "v1",
        title: "Add a compact corpus health summary",
        status: "active",
        taskType: "feature",
        difficulty: "medium",
        language: "typescript",
        framework: "react",
        sourceUri: "local://hope-agent/examples/benchmark-corpus/sample-real-project-regression/issues/feature-002",
        repoTemplate: "fixture://react-rust-desktop-app",
        tags: ["benchmark", "corpus", "dashboard"],
        successCriteria: [
          "Corpus health shows pack count, active task count and stale/risk signals.",
          "The summary remains readable with empty, warning and passing states.",
        ],
        validationCommands: ["pnpm typecheck"],
        allowedPaths: ["src/components/dashboard/**", "src/lib/transport.ts"],
        forbiddenPaths: ["crates/ha-core/tests/**"],
        calibrationNotes: ["Covers product-facing benchmark corpus visibility."],
        calibratedAt,
        licenseNote: "Synthetic local fixture.",
        privacyNote: "No private source content.",
        redactionStatus: "not_required",
      },
      {
        taskId: "SAMPLE-REFACTOR-003",
        version: "v1",
        title: "Separate benchmark validation policy from runner state",
        status: "active",
        taskType: "refactor",
        difficulty: "hard",
        language: "rust",
        framework: "ha-core",
        sourceUri: "local://hope-agent/examples/benchmark-corpus/sample-real-project-regression/issues/refactor-003",
        repoTemplate: "fixture://react-rust-desktop-app",
        tags: ["rust", "benchmark", "validation"],
        successCriteria: [
          "Validation is deterministic and does not execute provider or project commands.",
          "Activation fails closed when active tasks lack source, success criteria or validation commands.",
        ],
        validationCommands: ["cargo check -p ha-core --locked"],
        allowedPaths: ["crates/ha-core/src/coding_improvement.rs"],
        forbiddenPaths: ["src-tauri/**", "src/**"],
        calibrationNotes: ["Calibrated around owner-plane benchmark registry invariants."],
        calibratedAt,
        licenseNote: "Synthetic local fixture.",
        privacyNote: "No private source content.",
        redactionStatus: "not_required",
      },
      {
        taskId: "SAMPLE-I18N-004",
        version: "v1",
        title: "Keep dashboard fallback labels deterministic",
        status: "active",
        taskType: "i18n",
        difficulty: "easy",
        language: "typescript",
        framework: "i18next",
        sourceUri: "local://hope-agent/examples/benchmark-corpus/sample-real-project-regression/issues/i18n-004",
        repoTemplate: "fixture://react-rust-desktop-app",
        tags: ["i18n", "dashboard"],
        successCriteria: [
          "New UI strings use i18n keys with stable default values.",
          "No visible label overflows compact dashboard controls.",
        ],
        validationCommands: ["pnpm typecheck"],
        allowedPaths: ["src/components/dashboard/**", "src/i18n/**"],
        forbiddenPaths: ["crates/**"],
        calibrationNotes: ["Covers non-code-facing product polish tasks."],
        calibratedAt,
        licenseNote: "Synthetic local fixture.",
        privacyNote: "No private source content.",
        redactionStatus: "not_required",
      },
    ],
  }
}

function benchmarkCampaignTone(status?: string | null): string {
  switch (status) {
    case "passed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "running":
    case "queued":
    case "cancel_requested":
      return "bg-sky-500/10 text-sky-600 dark:text-sky-400"
    case "failed":
    case "interrupted":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    case "partial":
    case "skipped":
    case "cancelled":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function formatCampaignItemTarget(
  providerId?: string | null,
  modelId?: string | null,
  label?: string | null,
): string {
  if (providerId && modelId) return label ? `${label} (${providerId}/${modelId})` : `${providerId}/${modelId}`
  return label?.trim() || "deterministic"
}

function releaseGateCheckTone(status: string): string {
  return status === "failed"
    ? "bg-red-500/10 text-red-600 dark:text-red-400"
    : "bg-amber-500/10 text-amber-700 dark:text-amber-300"
}

function domainQualityRunTone(status: string): string {
  switch (status) {
    case "completed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "failed":
    case "blocked":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    case "needs_user":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function severityDot(severity: string): string {
  switch (severity) {
    case "p0":
    case "p1":
    case "high":
      return "bg-red-500"
    case "p2":
    case "medium":
      return "bg-amber-500"
    default:
      return "bg-muted-foreground/40"
  }
}

function verdictTone(verdict: string): string {
  switch (verdict) {
    case "improved":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "regressed":
    case "mixed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function stateTone(state: string): string {
  switch (state) {
    case "completed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "blocked":
    case "failed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function kindColor(kind: string): string {
  switch (kind) {
    case "skill_created":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "skill_activated":
      return "bg-sky-500/10 text-sky-600 dark:text-sky-400"
    case "skill_patched":
      return "bg-amber-500/10 text-amber-600 dark:text-amber-400"
    case "skill_discarded":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}
