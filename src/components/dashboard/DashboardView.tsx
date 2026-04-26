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
import RecapTab from "./recap/RecapTab"
import DreamingTab from "./dreaming/DreamingTab"
import LearningTab from "./learning/LearningTab"
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
} from "./types"
import { autoRefreshMs } from "./types"

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

export default function DashboardView({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation()
  const [filter, setFilter] = useState<DashboardFilterState>(defaultFilter)
  const [activeTab, setActiveTab] = useState("insights")
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
        }
      } catch (e) {
        logger.error("dashboard", "loadTabData", `Failed loading ${tab}: ${e}`)
      }
    },
    [filter],
  )

  // Initial load & filter change reload
  useEffect(() => {
    const timer = setTimeout(() => {
      setLoading(true)
      Promise.all([loadOverview(), loadTabData(activeTab)]).finally(() => {
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
      Promise.all([loadOverview(), loadTabData(activeTab)]).finally(() => {
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

  const handleRefresh = useCallback(() => {
    setLoading(true)
    setActiveList(null)
    Promise.all([loadOverview(), loadTabData(activeTab)]).finally(() => {
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
          `oc-tokens-${ts}.csv`,
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
          `oc-tools-${ts}.csv`,
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
          `oc-sessions-${ts}.csv`,
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
          `oc-errors-${ts}.csv`,
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
          `oc-insights-topsessions-${ts}.csv`,
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
      <DashboardFilter filter={filter} onChange={setFilter} />

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* Overview cards with delta */}
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

        {/* Tabs */}
        <Tabs ref={tabsRef} value={activeTab} onValueChange={setActiveTab}>
          <div className="flex items-center gap-3 flex-wrap">
            <TabsList>
              <TabsTrigger value="insights">{t("dashboard.tabs.insights")}</TabsTrigger>
              <TabsTrigger value="tokens">{t("dashboard.tabs.tokens")}</TabsTrigger>
              <TabsTrigger value="tools">{t("dashboard.tabs.tools")}</TabsTrigger>
              <TabsTrigger value="sessions">{t("dashboard.tabs.sessions")}</TabsTrigger>
              <TabsTrigger value="errors">{t("dashboard.tabs.errors")}</TabsTrigger>
              <TabsTrigger value="tasks">{t("dashboard.tabs.tasks")}</TabsTrigger>
              <TabsTrigger value="system">{t("dashboard.tabs.system")}</TabsTrigger>
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
          <TabsContent value="tokens">
            <TokenUsageSection
              data={tokenData}
              loading={loading}
              onDrillDown={(modelId) => setFilter((f) => ({ ...f, modelId: modelId }))}
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
          <TabsContent value="recap">
            <RecapTab />
          </TabsContent>
          <TabsContent value="learning">
            <LearningTab />
          </TabsContent>
          <TabsContent value="dreaming">
            <DreamingTab />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}
