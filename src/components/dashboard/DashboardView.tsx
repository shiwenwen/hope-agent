import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ArrowLeft, RefreshCw, Download, Play, Pause } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import DashboardFilter from "./DashboardFilter"
import OverviewCards from "./OverviewCards"
import type { CardAction } from "./OverviewCards"
import DetailListPanel from "./DetailListPanel"
import InsightsSection from "./InsightsSection"
import TokenUsageSection from "./TokenUsageSection"
import ToolUsageSection from "./ToolUsageSection"
import SessionSection from "./SessionSection"
import ErrorSection from "./ErrorSection"
import TaskSection from "./TaskSection"
import SystemMetricsSection from "./SystemMetricsSection"
import LocalModelsSection from "./LocalModelsSection"
import RecapTab from "./recap/RecapTab"
import DreamingTab from "./dreaming/DreamingTab"
import LearningTab from "./learning/LearningTab"
import ControlPlaneSection from "./ControlPlaneSection"
import { normalizeInitialTab, showsGlobalOverview } from "./dashboardTabs"
import type {
  DashboardFilter as DashboardFilterState,
  OverviewStatsWithDelta,
  DashboardTokenData,
  ToolUsageStats,
  DashboardSessionData,
  DashboardErrorData,
  DashboardTaskData,
  SystemMetrics,
  DashboardInsights,
  Granularity,
  DetailListType,
  AutoRefreshInterval,
  DashboardLocalModelUsage,
  ControlPlaneDashboard,
  AttentionItem,
} from "./types"
import { autoRefreshMs } from "./types"
import type { HardwareInfo, LocalOllamaModel, OllamaStatus } from "@/types/local-llm"
import type { LocalModelJobSnapshot } from "@/types/local-model-jobs"
import type { SettingsSection } from "@/components/settings/types"

function defaultFilter(): DashboardFilterState {
  const now = new Date()
  const thirtyDaysAgo = new Date(now)
  thirtyDaysAgo.setDate(thirtyDaysAgo.getDate() - 30)
  return {
    startDate: thirtyDaysAgo.toISOString(),
    endDate: now.toISOString(),
    agentId: null,
    providerId: null,
    modelId: null,
    usageKind: null,
    operation: null,
  }
}

/** Max 60 samples (~30 min at 30s refresh) kept in-memory for sparklines. */
const SYSTEM_HISTORY_LIMIT = 60

interface SystemHistoryPoint {
  t: number
  cpu: number
  mem: number
}

/** Escape CSV values. */
function csvEscape(v: unknown): string {
  if (v == null) return ""
  const s = String(v)
  if (s.includes(",") || s.includes("\n") || s.includes('"')) {
    return `"${s.replace(/"/g, '""')}"`
  }
  return s
}

/** Convert array of records to CSV and trigger download in the browser. */
function downloadCsv(filename: string, rows: Record<string, unknown>[]) {
  if (rows.length === 0) return
  const headers = Object.keys(rows[0])
  const lines = [
    headers.join(","),
    ...rows.map((r) => headers.map((h) => csvEscape(r[h])).join(",")),
  ]
  const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8;" })
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  URL.revokeObjectURL(url)
}

interface LocalModelsTabData {
  ollama: OllamaStatus | null
  ollamaVersion: string | null
  hardware: HardwareInfo | null
  models: LocalOllamaModel[] | null
  usage: DashboardLocalModelUsage | null
  jobs: LocalModelJobSnapshot[] | null
}

interface DashboardViewProps {
  onBack: () => void
  onOpenSettings?: (section?: SettingsSection) => void
  initialTab?: string
  initialRecapReportId?: string | null
  onOpenPlanHistory?: () => void
  onOpenControlItem?: (item: AttentionItem) => void
}

export default function DashboardView({
  onBack,
  onOpenSettings,
  initialTab,
  initialRecapReportId,
  onOpenPlanHistory,
  onOpenControlItem,
}: DashboardViewProps) {
  const { t } = useTranslation()
  const [filter, setFilter] = useState<DashboardFilterState>(defaultFilter)
  const [activeTab, setActiveTab] = useState(() => normalizeInitialTab(initialTab))
  const [activeList, setActiveList] = useState<DetailListType | null>(null)
  const [loading, setLoading] = useState(true)
  const [overview, setOverview] = useState<OverviewStatsWithDelta | null>(null)
  const [insightsData, setInsightsData] = useState<DashboardInsights | null>(null)
  const [tokenData, setTokenData] = useState<DashboardTokenData | null>(null)
  const [toolData, setToolData] = useState<ToolUsageStats[] | null>(null)
  const [sessionData, setSessionData] = useState<DashboardSessionData | null>(null)
  const [errorData, setErrorData] = useState<DashboardErrorData | null>(null)
  const [taskData, setTaskData] = useState<DashboardTaskData | null>(null)
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null)
  const [systemHistory, setSystemHistory] = useState<SystemHistoryPoint[]>([])
  const [controlPlaneData, setControlPlaneData] = useState<ControlPlaneDashboard | null>(null)
  const [controlProjectId, setControlProjectId] = useState<string | null>(null)
  const [localModelsData, setLocalModelsData] = useState<LocalModelsTabData>({
    ollama: null,
    ollamaVersion: null,
    hardware: null,
    models: null,
    usage: null,
    jobs: null,
  })
  const [granularity, setGranularity] = useState<Granularity>("day")
  const [autoRefresh, setAutoRefresh] = useState<AutoRefreshInterval>("off")
  const [lastRefreshAt, setLastRefreshAt] = useState<Date | null>(null)
  const [agents, setAgents] = useState<{ id: string; name: string; emoji?: string | null }[]>([])
  const tabsRef = useRef<HTMLDivElement>(null)

  const agentNameMap = useMemo(() => {
    const map: Record<string, string> = {}
    for (const a of agents) {
      map[a.id] = a.emoji ? `${a.emoji} ${a.name}` : a.name
    }
    return map
  }, [agents])

  const loadOverview = useCallback(async () => {
    try {
      const data = await getTransport().call<OverviewStatsWithDelta>("dashboard_overview_delta", {
        filter,
      })
      setOverview(data)
    } catch (e) {
      logger.error("dashboard", "loadOverview", `Failed: ${e}`)
    }
  }, [filter])

  // Load agent names once on mount
  useEffect(() => {
    getTransport()
      .call<{ id: string; name: string; emoji?: string | null }[]>("list_agents")
      .then(setAgents)
      .catch(() => {})
  }, [])

  const loadTabData = useCallback(
    async (tab: string) => {
      try {
        switch (tab) {
          case "insights": {
            const d = await getTransport().call<DashboardInsights>("dashboard_insights", { filter })
            setInsightsData(d)
            break
          }
          case "tokens": {
            const td = await getTransport().call<DashboardTokenData>("dashboard_token_usage", {
              filter,
            })
            setTokenData(td)
            break
          }
          case "tools": {
            const tld = await getTransport().call<ToolUsageStats[]>("dashboard_tool_usage", {
              filter,
            })
            setToolData(tld)
            break
          }
          case "sessions": {
            const sd = await getTransport().call<DashboardSessionData>("dashboard_sessions", {
              filter,
            })
            setSessionData(sd)
            break
          }
          case "errors": {
            const ed = await getTransport().call<DashboardErrorData>("dashboard_errors", { filter })
            setErrorData(ed)
            break
          }
          case "tasks": {
            const tkd = await getTransport().call<DashboardTaskData>("dashboard_tasks", { filter })
            setTaskData(tkd)
            break
          }
          case "control-plane": {
            const controlFilter = {
              startDate: filter.startDate,
              endDate: filter.endDate,
              agentId: filter.agentId,
              projectId: controlProjectId,
            }
            const result = await getTransport().call<ControlPlaneDashboard>(
              "dashboard_control_plane",
              { filter: controlFilter },
            )
            setControlPlaneData(result)
            break
          }
          case "system": {
            const sm = await getTransport().call<SystemMetrics>("dashboard_system_metrics")
            setSystemMetrics(sm)
            setSystemHistory((prev) => {
              const point: SystemHistoryPoint = {
                t: Date.now(),
                cpu: Math.min(sm.processCpuPercent, sm.cpuCount * 100),
                mem: sm.memory.rssPercent,
              }
              const next = [...prev, point]
              if (next.length > SYSTEM_HISTORY_LIMIT) {
                next.splice(0, next.length - SYSTEM_HISTORY_LIMIT)
              }
              return next
            })
            break
          }
          case "local-models": {
            // Tolerate per-probe failures (e.g. Ollama unreachable) without
            // breaking the whole tab — each settled result becomes null if
            // the call rejects.
            const settled = await Promise.allSettled([
              getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
              getTransport().call<HardwareInfo>("local_llm_detect_hardware"),
              getTransport().call<{ version: string | null } | string | null>(
                "local_llm_detect_ollama_version",
              ),
              getTransport().call<LocalOllamaModel[]>("local_llm_list_models"),
              getTransport().call<DashboardLocalModelUsage>("dashboard_local_model_usage", {
                filter,
              }),
              getTransport().call<LocalModelJobSnapshot[]>("local_model_job_list"),
            ])
            const pick = <T,>(r: PromiseSettledResult<T>): T | null =>
              r.status === "fulfilled" ? r.value : null
            const versionResp = pick(settled[2])
            const version =
              typeof versionResp === "string"
                ? versionResp
                : versionResp && typeof versionResp === "object"
                  ? (versionResp.version ?? null)
                  : null
            setLocalModelsData({
              ollama: pick(settled[0]),
              hardware: pick(settled[1]),
              ollamaVersion: version,
              models: pick(settled[3]),
              usage: pick(settled[4]),
              jobs: pick(settled[5]),
            })
            break
          }
        }
      } catch (e) {
        logger.error("dashboard", "loadTabData", `Failed loading ${tab}: ${e}`)
      }
    },
    [filter, controlProjectId],
  )

  // Initial load & filter change reload
  useEffect(() => {
    const timer = setTimeout(() => {
      setLoading(true)
      const overviewRequest = showsGlobalOverview(activeTab) ? loadOverview() : Promise.resolve()
      Promise.all([overviewRequest, loadTabData(activeTab)]).finally(() => {
        setLoading(false)
        setLastRefreshAt(new Date())
      })
    }, 0)
    return () => clearTimeout(timer)
  }, [filter, loadOverview, loadTabData, activeTab])

  // Tab switch reload (skip initial mount since above effect handles it)
  useEffect(() => {
    const timer = setTimeout(() => {
      loadTabData(activeTab)
    }, 0)
    return () => clearTimeout(timer)
  }, [activeTab, granularity, loadTabData])

  // Auto-refresh polling
  useEffect(() => {
    const ms = autoRefreshMs(autoRefresh)
    if (ms <= 0) return
    const id = window.setInterval(() => {
      const overviewRequest = showsGlobalOverview(activeTab) ? loadOverview() : Promise.resolve()
      Promise.all([overviewRequest, loadTabData(activeTab)]).finally(() => {
        setLastRefreshAt(new Date())
      })
    }, ms)
    return () => window.clearInterval(id)
  }, [autoRefresh, loadOverview, loadTabData, activeTab])

  const handleCardClick = useCallback((action: CardAction) => {
    if (action.type === "tab") {
      setActiveList(null)
      setActiveTab(action.tab)
      requestAnimationFrame(() => {
        tabsRef.current?.scrollIntoView({ behavior: "smooth", block: "start" })
      })
    } else {
      setActiveList((prev) => (prev === action.listType ? null : action.listType))
    }
  }, [])

  const handleTabChange = useCallback((tab: string) => {
    setActiveList(null)
    setActiveTab(tab)
  }, [])

  const handleRefresh = useCallback(() => {
    setLoading(true)
    setActiveList(null)
    const overviewRequest = showsGlobalOverview(activeTab) ? loadOverview() : Promise.resolve()
    Promise.all([overviewRequest, loadTabData(activeTab)]).finally(() => {
      setLoading(false)
      setLastRefreshAt(new Date())
    })
  }, [loadOverview, loadTabData, activeTab])

  /** Export the currently visible tab's data to CSV. */
  const handleExport = useCallback(() => {
    const ts = new Date().toISOString().replace(/[:.]/g, "-")
    switch (activeTab) {
      case "tokens": {
        if (!tokenData) return
        downloadCsv(
          `ha-tokens-${ts}.csv`,
          tokenData.byModel.map((m) => ({
            model: m.modelId,
            provider: m.providerName,
            input_tokens: m.inputTokens,
            output_tokens: m.outputTokens,
            estimated_cost_usd: m.estimatedCostUsd.toFixed(6),
            avg_ttft_ms: m.avgTtftMs ?? "",
          })),
        )
        break
      }
      case "tools": {
        if (!toolData) return
        downloadCsv(
          `ha-tools-${ts}.csv`,
          toolData.map((r) => ({
            tool: r.toolName,
            call_count: r.callCount,
            error_count: r.errorCount,
            avg_duration_ms: r.avgDurationMs.toFixed(2),
            total_duration_ms: r.totalDurationMs,
          })),
        )
        break
      }
      case "sessions": {
        if (!sessionData) return
        downloadCsv(
          `ha-sessions-${ts}.csv`,
          sessionData.byAgent.map((a) => ({
            agent_id: a.agentId,
            sessions: a.sessionCount,
            messages: a.messageCount,
            total_tokens: a.totalTokens,
          })),
        )
        break
      }
      case "errors": {
        if (!errorData) return
        downloadCsv(
          `ha-errors-${ts}.csv`,
          errorData.byCategory.map((c) => ({
            category: c.category,
            count: c.count,
          })),
        )
        break
      }
      case "insights": {
        if (!insightsData) return
        downloadCsv(
          `ha-insights-topsessions-${ts}.csv`,
          insightsData.topSessions.map((s) => ({
            id: s.id,
            title: s.title ?? "",
            agent_id: s.agentId,
            model_id: s.modelId ?? "",
            message_count: s.messageCount,
            total_tokens: s.totalTokens,
            estimated_cost_usd: s.estimatedCostUsd.toFixed(6),
            updated_at: s.updatedAt,
          })),
        )
        break
      }
    }
  }, [activeTab, tokenData, toolData, sessionData, errorData, insightsData])

  const canExport =
    (activeTab === "tokens" && !!tokenData) ||
    (activeTab === "tools" && !!toolData) ||
    (activeTab === "sessions" && !!sessionData) ||
    (activeTab === "errors" && !!errorData) ||
    (activeTab === "insights" && !!insightsData)

  const showGranularity =
    activeTab === "tokens" || activeTab === "sessions" || activeTab === "errors"

  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-background">
      {/* Header */}
      <div className="shrink-0 border-b px-6 py-3 flex items-center gap-3" data-tauri-drag-region>
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onBack}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <h1 className="text-lg font-semibold">{t("dashboard.title")}</h1>
        {lastRefreshAt && (
          <span className="text-[11px] text-muted-foreground hidden md:inline">
            {t("dashboard.lastRefresh")}: {lastRefreshAt.toLocaleTimeString()}
          </span>
        )}
        <div className="flex-1" />

        {/* Auto refresh selector */}
        <Select value={autoRefresh} onValueChange={(v) => setAutoRefresh(v as AutoRefreshInterval)}>
          <SelectTrigger className="h-8 w-[120px] text-xs">
            <div className="flex items-center gap-1.5">
              {autoRefresh === "off" ? (
                <Pause className="h-3 w-3" />
              ) : (
                <Play className="h-3 w-3 text-emerald-500" />
              )}
              <SelectValue />
            </div>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="off">{t("dashboard.autoRefresh.off")}</SelectItem>
            <SelectItem value="30s">{t("dashboard.autoRefresh.30s")}</SelectItem>
            <SelectItem value="1m">{t("dashboard.autoRefresh.1m")}</SelectItem>
            <SelectItem value="5m">{t("dashboard.autoRefresh.5m")}</SelectItem>
          </SelectContent>
        </Select>

        <IconTip label={t("dashboard.export") as string}>
          <span className="inline-flex">
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={handleExport}
              disabled={!canExport}
            >
              <Download className="h-4 w-4" />
            </Button>
          </span>
        </IconTip>

        <IconTip label={t("dashboard.refresh") as string}>
          <span className="inline-flex">
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={handleRefresh}
              disabled={loading}
            >
              <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
            </Button>
          </span>
        </IconTip>
      </div>

      {/* Filter bar */}
      <DashboardFilter
        filter={filter}
        onChange={setFilter}
        controlPlane={activeTab === "control-plane"}
      />

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* Overview cards with delta */}
        {showsGlobalOverview(activeTab) && (
          <>
            <OverviewCards
              data={overview}
              loading={loading}
              activeList={activeList}
              onCardClick={handleCardClick}
            />

            {/* Detail list panel (between cards and tabs) */}
            {activeList && (
              <DetailListPanel
                listType={activeList}
                filter={filter}
                agentNameMap={agentNameMap}
                onClose={() => setActiveList(null)}
              />
            )}
          </>
        )}

        {/* Tabs */}
        <Tabs ref={tabsRef} value={activeTab} onValueChange={handleTabChange}>
          <div className="flex items-center gap-3 flex-wrap">
            <TabsList>
              <TabsTrigger value="insights">{t("dashboard.tabs.insights")}</TabsTrigger>
              <TabsTrigger value="control-plane">{t("dashboard.controlPlane.title")}</TabsTrigger>
              <TabsTrigger value="tokens">{t("dashboard.tabs.tokens")}</TabsTrigger>
              <TabsTrigger value="tools">{t("dashboard.tabs.tools")}</TabsTrigger>
              <TabsTrigger value="sessions">{t("dashboard.tabs.sessions")}</TabsTrigger>
              <TabsTrigger value="errors">{t("dashboard.tabs.errors")}</TabsTrigger>
              <TabsTrigger value="tasks">{t("dashboard.controlPlane.automationTab")}</TabsTrigger>
              <TabsTrigger value="system">{t("dashboard.tabs.system")}</TabsTrigger>
              <TabsTrigger value="local-models">{t("dashboard.tabs.localModels")}</TabsTrigger>
              <TabsTrigger value="recap">{t("dashboard.tabs.recap")}</TabsTrigger>
              <TabsTrigger value="learning">{t("dashboard.tabs.learning")}</TabsTrigger>
              <TabsTrigger value="dreaming">{t("dashboard.tabs.dreaming")}</TabsTrigger>
            </TabsList>
            {showGranularity && (
              <div className="flex gap-1">
                {(["day", "week", "month"] as Granularity[]).map((g) => (
                  <Button
                    key={g}
                    variant={granularity === g ? "secondary" : "ghost"}
                    size="sm"
                    onClick={() => setGranularity(g)}
                    className="text-xs h-7"
                  >
                    {t(`dashboard.granularity.${g}`)}
                  </Button>
                ))}
              </div>
            )}
          </div>

          <TabsContent value="insights">
            <InsightsSection
              data={insightsData}
              loading={loading}
              onDrillDownModel={(modelId) => setFilter((f) => ({ ...f, modelId }))}
            />
          </TabsContent>
          <TabsContent value="control-plane">
            <ControlPlaneSection
              data={controlPlaneData}
              loading={loading}
              projectId={controlProjectId}
              onProjectChange={setControlProjectId}
              onOpenPlanHistory={onOpenPlanHistory}
              onOpenAttention={onOpenControlItem}
              initialSection={initialTab === "plans" ? "progress" : "overview"}
            />
          </TabsContent>
          <TabsContent value="tokens">
            <TokenUsageSection
              data={tokenData}
              loading={loading}
              onDrillDown={(modelId) => setFilter((f) => ({ ...f, modelId: modelId }))}
              onDrillDownOperation={(operation) => setFilter((f) => ({ ...f, operation }))}
            />
          </TabsContent>
          <TabsContent value="tools">
            <ToolUsageSection data={toolData} loading={loading} />
          </TabsContent>
          <TabsContent value="sessions">
            <SessionSection
              data={sessionData}
              loading={loading}
              agentNameMap={agentNameMap}
              onDrillDown={(agentId) => setFilter((f) => ({ ...f, agentId: agentId }))}
            />
          </TabsContent>
          <TabsContent value="errors">
            <ErrorSection data={errorData} loading={loading} />
          </TabsContent>
          <TabsContent value="tasks">
            <TaskSection data={taskData} loading={loading} />
          </TabsContent>
          <TabsContent value="system">
            <SystemMetricsSection data={systemMetrics} history={systemHistory} loading={loading} />
          </TabsContent>
          <TabsContent value="local-models">
            <LocalModelsSection
              loading={loading}
              ollama={localModelsData.ollama}
              ollamaVersion={localModelsData.ollamaVersion}
              hardware={localModelsData.hardware}
              models={localModelsData.models}
              usage={localModelsData.usage}
              jobs={localModelsData.jobs}
              onOpenSettings={onOpenSettings}
            />
          </TabsContent>
          <TabsContent value="recap">
            <RecapTab initialReportId={initialRecapReportId} />
          </TabsContent>
          <TabsContent value="learning">
            <LearningTab filter={filter} />
          </TabsContent>
          <TabsContent value="dreaming">
            <DreamingTab />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}
