import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ArrowLeft, RefreshCw } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import DashboardFilter from "./DashboardFilter"
import OverviewCards from "./OverviewCards"
import type { CardAction } from "./OverviewCards"
import DetailListPanel from "./DetailListPanel"
import TokenUsageSection from "./TokenUsageSection"
import ToolUsageSection from "./ToolUsageSection"
import SessionSection from "./SessionSection"
import ErrorSection from "./ErrorSection"
import TaskSection from "./TaskSection"
import SystemMetricsSection from "./SystemMetricsSection"
import type {
  DashboardFilter as DashboardFilterState,
  OverviewStats,
  DashboardTokenData,
  ToolUsageStats,
  DashboardSessionData,
  DashboardErrorData,
  DashboardTaskData,
  SystemMetrics,
  Granularity,
  DetailListType,
} from "./types"

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

export default function DashboardView({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation()
  const [filter, setFilter] = useState<DashboardFilterState>(defaultFilter)
  const [activeTab, setActiveTab] = useState("tokens")
  const [activeList, setActiveList] = useState<DetailListType | null>(null)
  const [loading, setLoading] = useState(true)
  const [overview, setOverview] = useState<OverviewStats | null>(null)
  const [tokenData, setTokenData] = useState<DashboardTokenData | null>(null)
  const [toolData, setToolData] = useState<ToolUsageStats[] | null>(null)
  const [sessionData, setSessionData] = useState<DashboardSessionData | null>(null)
  const [errorData, setErrorData] = useState<DashboardErrorData | null>(null)
  const [taskData, setTaskData] = useState<DashboardTaskData | null>(null)
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null)
  const [granularity, setGranularity] = useState<Granularity>("day")
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
      const data = await invoke<OverviewStats>("dashboard_overview", { filter })
      setOverview(data)
    } catch (e) {
      logger.error("dashboard", "loadOverview", `Failed: ${e}`)
    }
  }, [filter])

  // Load agent names once on mount
  useEffect(() => {
    invoke<{ id: string; name: string; emoji?: string | null }[]>("list_agents")
      .then(setAgents)
      .catch(() => {})
  }, [])

  const loadTabData = useCallback(
    async (tab: string) => {
      try {
        switch (tab) {
          case "tokens": {
            const td = await invoke<DashboardTokenData>("dashboard_token_usage", {
              filter,
            })
            setTokenData(td)
            break
          }
          case "tools": {
            const tld = await invoke<ToolUsageStats[]>("dashboard_tool_usage", { filter })
            setToolData(tld)
            break
          }
          case "sessions": {
            const sd = await invoke<DashboardSessionData>("dashboard_sessions", {
              filter,
            })
            setSessionData(sd)
            break
          }
          case "errors": {
            const ed = await invoke<DashboardErrorData>("dashboard_errors", {
              filter,
            })
            setErrorData(ed)
            break
          }
          case "tasks": {
            const tkd = await invoke<DashboardTaskData>("dashboard_tasks", { filter })
            setTaskData(tkd)
            break
          }
          case "system": {
            const sm = await invoke<SystemMetrics>("dashboard_system_metrics")
            setSystemMetrics(sm)
            break
          }
        }
      } catch (e) {
        logger.error("dashboard", "loadTabData", `Failed loading ${tab}: ${e}`)
      }
    },
    [filter, granularity],
  )

  useEffect(() => {
    setLoading(true)
    Promise.all([loadOverview(), loadTabData(activeTab)]).finally(() => setLoading(false))
  }, [filter]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    loadTabData(activeTab)
  }, [activeTab, granularity]) // eslint-disable-line react-hooks/exhaustive-deps

  const handleCardClick = useCallback((action: CardAction) => {
    if (action.type === "tab") {
      setActiveList(null)
      setActiveTab(action.tab)
      requestAnimationFrame(() => {
        tabsRef.current?.scrollIntoView({ behavior: "smooth", block: "start" })
      })
    } else {
      // Toggle list: click same card again to close
      setActiveList((prev) => (prev === action.listType ? null : action.listType))
    }
  }, [])

  const handleRefresh = useCallback(() => {
    setLoading(true)
    // Close detail list on refresh so it reloads when reopened
    setActiveList(null)
    Promise.all([loadOverview(), loadTabData(activeTab)]).finally(() => setLoading(false))
  }, [loadOverview, loadTabData, activeTab])

  const showGranularity =
    activeTab === "tokens" || activeTab === "sessions" || activeTab === "errors"

  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-background">
      {/* Header */}
      <div
        className="shrink-0 border-b px-6 py-3 flex items-center gap-3"
        data-tauri-drag-region
      >
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onBack}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <h1 className="text-lg font-semibold">{t("dashboard.title")}</h1>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={handleRefresh}
          disabled={loading}
        >
          <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
        </Button>
      </div>

      {/* Filter bar */}
      <DashboardFilter filter={filter} onChange={setFilter} />

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* Overview cards */}
        <OverviewCards data={overview} loading={loading} activeList={activeList} onCardClick={handleCardClick} />

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
              <TabsTrigger value="tokens">{t("dashboard.tabs.tokens")}</TabsTrigger>
              <TabsTrigger value="tools">{t("dashboard.tabs.tools")}</TabsTrigger>
              <TabsTrigger value="sessions">{t("dashboard.tabs.sessions")}</TabsTrigger>
              <TabsTrigger value="errors">{t("dashboard.tabs.errors")}</TabsTrigger>
              <TabsTrigger value="tasks">{t("dashboard.tabs.tasks")}</TabsTrigger>
              <TabsTrigger value="system">{t("dashboard.tabs.system")}</TabsTrigger>
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

          <TabsContent value="tokens">
            <TokenUsageSection
              data={tokenData}
              loading={loading}
              onDrillDown={(modelId) =>
                setFilter((f) => ({ ...f, modelId: modelId }))
              }
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
              onDrillDown={(agentId) =>
                setFilter((f) => ({ ...f, agentId: agentId }))
              }
            />
          </TabsContent>
          <TabsContent value="errors">
            <ErrorSection data={errorData} loading={loading} />
          </TabsContent>
          <TabsContent value="tasks">
            <TaskSection data={taskData} loading={loading} />
          </TabsContent>
          <TabsContent value="system">
            <SystemMetricsSection data={systemMetrics} loading={loading} />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}
